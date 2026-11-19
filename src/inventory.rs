//! The avatar's inventory: how commands name an item or folder, the
//! requests that read folders and items over HTTP and change the Current
//! Outfit folder, a worker thread that makes them so the main loop never
//! waits, and the text of a listing. Nothing read here is kept: every job
//! asks the grid anew.

use crate::caps::InventoryUrls;
use crate::escape::escape;
use crate::net::Event;
use crate::xml::{self, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the grid has to answer everything one command asks of the
/// inventory, counted from when the command arrived.
pub const LIMIT: Duration = Duration::from_secs(50);
/// Folders named in one `FetchInventoryDescendents2` request. The viewer
/// asks for no more than this at a time
/// (llinventorymodelbackgroundfetch.cpp, `max_batch_size`).
const FOLDER_BATCH: usize = 10;
/// Items named in one `FetchInventory2` request.
const ITEM_BATCH: usize = 50;

// Asset types (llassettype.h), as inventory items carry them.
pub const ASSET_TEXTURE: i32 = 0;
pub const ASSET_SOUND: i32 = 1;
pub const ASSET_CALLING_CARD: i32 = 2;
pub const ASSET_LANDMARK: i32 = 3;
pub const ASSET_CLOTHING: i32 = 5;
pub const ASSET_OBJECT: i32 = 6;
pub const ASSET_NOTECARD: i32 = 7;
pub const ASSET_FOLDER: i32 = 8;
pub const ASSET_SCRIPT: i32 = 10;
pub const ASSET_BODY_PART: i32 = 13;
pub const ASSET_ANIMATION: i32 = 20;
pub const ASSET_GESTURE: i32 = 21;
pub const ASSET_LINK: i32 = 24;
pub const ASSET_LINK_FOLDER: i32 = 25;
// Inventory types (llinventorytype.h) of the items the tests make.
#[cfg(test)]
const INV_OBJECT: i32 = 6;
#[cfg(test)]
const INV_WEARABLE: i32 = 18;
/// The folder type of the Current Outfit folder (llfoldertype.h).
const FOLDER_CURRENT_OUTFIT: i32 = 46;

// ------------------------------------------------------------------ naming

/// An inventory item or folder as a command names it.
#[derive(Debug, Clone, PartialEq)]
pub enum Ref {
    Id(Uuid),
    /// The names from the top of My Inventory; none for My Inventory itself.
    Path(Vec<String>),
}

/// The UUID `text` is, when it has the form of one: 36 characters with the
/// hyphens in place.
pub fn parse_uuid(text: &str) -> Option<Uuid> {
    let text = text.trim();
    let hyphens = text
        .bytes()
        .enumerate()
        .all(|(index, byte)| (byte == b'-') == matches!(index, 8 | 13 | 18 | 23));
    (text.len() == 36 && hyphens).then(|| Uuid::try_parse(text).ok()).flatten()
}

/// Splits a path into its names. A `/` at the start or the end is ignored;
/// `\/` is a `/` inside a name and `\\` a backslash, and `\t`, `\n` and
/// `\r` are read as a listing writes them. Nothing left means My Inventory
/// itself.
pub fn parse_path(path: &str) -> Result<Vec<String>, String> {
    let mut names = vec![String::new()];
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        let name = names.last_mut().expect("never empty");
        match c {
            '/' => names.push(String::new()),
            '\\' => match chars.next() {
                Some('/') => name.push('/'),
                Some('\\') | None => name.push('\\'),
                Some('t') => name.push('\t'),
                Some('n') => name.push('\n'),
                Some('r') => name.push('\r'),
                Some(other) => {
                    name.push('\\');
                    name.push(other);
                }
            },
            other => name.push(other),
        }
    }
    if names.len() > 1 && names[0].is_empty() {
        names.remove(0);
    }
    if names.last().is_some_and(String::is_empty) {
        names.pop();
    }
    // A slash at each end and nothing between them.
    if names.len() == 1 && names[0].is_empty() {
        names.clear();
    }
    if names.iter().any(String::is_empty) {
        return Err(format!("the path \"{path}\" has an empty name in it"));
    }
    Ok(names)
}

/// Reads what a command gave for an item or folder: a UUID when it has the
/// form of one, a path otherwise.
pub fn parse_ref(given: &str) -> Result<Ref, String> {
    match parse_uuid(given) {
        Some(id) => Ok(Ref::Id(id)),
        None => parse_path(given).map(Ref::Path),
    }
}

/// A name as a path shows it: the line escapes, and `\/` for a `/`.
pub fn escape_name(name: &str) -> String {
    escape(name).replace('/', "\\/")
}

/// A path as a listing and the log show it.
pub fn show_path(names: &[String]) -> String {
    names.iter().map(|name| escape_name(name)).collect::<Vec<_>>().join("/")
}

impl Ref {
    /// For messages: the UUID, or the path in quotes; My Inventory by name.
    pub fn show(&self) -> String {
        match self {
            Ref::Id(id) => id.to_string(),
            Ref::Path(names) if names.is_empty() => "My Inventory".to_string(),
            Ref::Path(names) => format!("\"{}\"", show_path(names)),
        }
    }
}

/// The arguments of the `inventory` command.
#[derive(Debug, Clone, PartialEq)]
pub struct ListArgs {
    pub folder: Ref,
    /// Only the items attached to the avatar now.
    pub attached: bool,
    /// Everything below the folder, not just its direct children.
    pub recursive: bool,
}

/// Reads `[-a] [-n] [<folder>]`, the flags in any position.
pub fn parse_list_args(args: &[String]) -> Result<ListArgs, String> {
    let mut parsed = ListArgs {
        folder: Ref::Path(Vec::new()),
        attached: false,
        recursive: true,
    };
    let mut folder = None;
    for arg in args {
        match arg.as_str() {
            "-a" | "--attached" => parsed.attached = true,
            "-n" | "--no-recurse" => parsed.recursive = false,
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "`inventory` has no option `{flag}`; it takes -a (--attached) and -n (--no-recurse), \
                     and a folder that starts with `-` is written with a `/` in front"
                ));
            }
            given => {
                if let Some(first) = folder.replace(given) {
                    return Err(format!(
                        "`inventory` takes one folder, not \"{first}\" and \"{given}\"; a path with spaces has to be quoted"
                    ));
                }
            }
        }
    }
    if let Some(given) = folder {
        parsed.folder = parse_ref(given)?;
    }
    Ok(parsed)
}

/// The attachment points by number, as viewers name them
/// (avatar_lad.xml), with `HUD ` in front of the heads-up display's.
const POINTS: [&str; 55] = [
    "Chest",
    "Skull",
    "Left Shoulder",
    "Right Shoulder",
    "Left Hand",
    "Right Hand",
    "Left Foot",
    "Right Foot",
    "Spine",
    "Pelvis",
    "Mouth",
    "Chin",
    "Left Ear",
    "Right Ear",
    "Left Eyeball",
    "Right Eyeball",
    "Nose",
    "R Upper Arm",
    "R Forearm",
    "L Upper Arm",
    "L Forearm",
    "Right Hip",
    "R Upper Leg",
    "R Lower Leg",
    "Left Hip",
    "L Upper Leg",
    "L Lower Leg",
    "Stomach",
    "Left Pec",
    "Right Pec",
    "HUD Center 2",
    "HUD Top Right",
    "HUD Top",
    "HUD Top Left",
    "HUD Center",
    "HUD Bottom Left",
    "HUD Bottom",
    "HUD Bottom Right",
    "Neck",
    "Avatar Center",
    "Left Ring Finger",
    "Right Ring Finger",
    "Tail Base",
    "Tail Tip",
    "Left Wing",
    "Right Wing",
    "Jaw",
    "Alt Left Ear",
    "Alt Right Ear",
    "Alt Left Eye",
    "Alt Right Eye",
    "Tongue",
    "Groin",
    "Left Hind Foot",
    "Right Hind Foot",
];
/// The highest attachment point a simulator message can carry beside the
/// add bit.
const MAX_POINT: u8 = 127;

/// The name of an attachment point, or its number when it has none.
pub fn point_name(point: u8) -> String {
    match usize::from(point).checked_sub(1).and_then(|index| POINTS.get(index)) {
        Some(name) => name.to_string(),
        None => point.to_string(),
    }
}

/// `text` without the spaces around it and with single spaces inside.
fn spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The attachment point `text` names, in any letter case, or gives as a
/// number from 1 to 127.
pub fn parse_point(text: &str) -> Option<u8> {
    let text = spaced(text);
    if let Ok(number) = text.parse::<u8>() {
        return (1..=MAX_POINT).contains(&number).then_some(number);
    }
    let found = POINTS.iter().position(|name| name.eq_ignore_ascii_case(&text))?;
    Some(found as u8 + 1)
}

/// The kinds of body part and clothing by wearable type
/// (llwearabletype.h). The first four are the body parts.
const KINDS: [&str; 17] = [
    "shape",
    "skin",
    "hair",
    "eyes",
    "shirt",
    "pants",
    "shoes",
    "socks",
    "jacket",
    "gloves",
    "undershirt",
    "underpants",
    "skirt",
    "alpha",
    "tattoo",
    "physics",
    "universal",
];
const BODY_PART_KINDS: u8 = 4;

/// The name of a wearable type, or its number when it has none.
pub fn kind_name(kind: u8) -> String {
    match KINDS.get(usize::from(kind)) {
        Some(name) => name.to_string(),
        None => kind.to_string(),
    }
}

/// The wearable type `text` names, in any letter case.
pub fn parse_kind(text: &str) -> Option<u8> {
    let text = spaced(text);
    KINDS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(&text))
        .map(|kind| kind as u8)
}

/// Whether a wearable type is one the avatar cannot be without.
pub fn is_body_part(kind: u8) -> bool {
    kind < BODY_PART_KINDS
}

/// What `detach` (or `unwear`) is to take off.
#[derive(Debug, Clone, PartialEq)]
pub enum Removal {
    Item(Ref),
    /// Everything attached to this point.
    Point(u8),
    /// Every piece of clothing of this kind.
    Kind(u8),
}

/// Reads the argument of `detach`: a UUID when it has the form of one, an
/// attachment point or a kind of clothing when it names one, and a path
/// otherwise.
pub fn parse_removal(given: &str) -> Result<Removal, String> {
    if let Some(id) = parse_uuid(given) {
        return Ok(Removal::Item(Ref::Id(id)));
    }
    if let Some(point) = parse_point(given) {
        return Ok(Removal::Point(point));
    }
    if let Some(kind) = parse_kind(given) {
        return Ok(Removal::Kind(kind));
    }
    parse_path(given).map(|names| Removal::Item(Ref::Path(names)))
}

// ------------------------------------------------------------- the records

/// A folder as its parent's contents describe it.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    /// What the folder is for (llfoldertype.h); -1 for an ordinary one.
    pub folder_type: i32,
}

/// An inventory item as the grid describes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub id: Uuid,
    /// The folder it is in.
    pub parent: Uuid,
    pub name: String,
    pub description: String,
    pub asset_type: i32,
    pub inv_type: i32,
    pub flags: u32,
    /// The asset; for a link, the item or folder it points to.
    pub asset_id: Uuid,
    pub owner: Uuid,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub next_owner_mask: u32,
}

impl Item {
    /// The low byte of the flags: the attachment point an object was last
    /// on, the wearable type of clothing and body parts
    /// (llinventorydefines.h:83).
    pub fn subtype(&self) -> u8 {
        (self.flags & 0xFF) as u8
    }

    /// For messages: the name in quotes and the id.
    pub fn show(&self) -> String {
        format!("\"{}\" <{}>", self.name, self.id)
    }
}

/// What the listing calls an asset type.
pub fn type_name(asset_type: i32) -> &'static str {
    match asset_type {
        ASSET_TEXTURE => "texture",
        ASSET_SOUND => "sound",
        ASSET_CALLING_CARD => "callingcard",
        ASSET_LANDMARK => "landmark",
        ASSET_CLOTHING => "clothing",
        ASSET_OBJECT => "object",
        ASSET_NOTECARD => "notecard",
        ASSET_FOLDER => "folder",
        ASSET_SCRIPT => "script",
        ASSET_BODY_PART => "bodypart",
        ASSET_ANIMATION => "animation",
        ASSET_GESTURE => "gesture",
        ASSET_LINK | ASSET_LINK_FOLDER => "link",
        _ => "other",
    }
}

/// What an error calls an item of this type: "is a notecard".
pub fn type_phrase(asset_type: i32) -> String {
    let name = match asset_type {
        ASSET_CALLING_CARD => "calling card",
        ASSET_CLOTHING => "piece of clothing",
        ASSET_BODY_PART => "body part",
        ASSET_LINK | ASSET_LINK_FOLDER => "link",
        other => type_name(other),
    };
    let article = if name.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    match name {
        "other" => format!("of a type the bot has no name for ({asset_type})"),
        name => format!("{article} {name}"),
    }
}

/// The contents of one folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Contents {
    pub folder: Uuid,
    /// Counts the folder's changes; what the outfit service is told.
    pub version: i64,
    pub folders: Vec<Folder>,
    pub items: Vec<Item>,
}

// -------------------------------------------------- requests and replies

/// The names Second Life may give for an asset type where OpenSim gives
/// the number (llassettype.cpp:74-104).
const ASSET_TYPE_NAMES: [(&str, i32); 24] = [
    ("texture", 0),
    ("sound", 1),
    ("callcard", 2),
    ("landmark", 3),
    ("script", 4),
    ("clothing", 5),
    ("object", 6),
    ("notecard", 7),
    ("category", 8),
    ("lsltext", 10),
    ("lslbyte", 11),
    ("txtr_tga", 12),
    ("bodypart", 13),
    ("snd_wav", 17),
    ("img_tga", 18),
    ("jpeg", 19),
    ("animatn", 20),
    ("gesture", 21),
    ("simstate", 22),
    ("link", 24),
    ("link_f", 25),
    ("mesh", 49),
    ("settings", 56),
    ("material", 57),
];
/// Likewise for inventory types (llinventorytype.cpp:72-92).
const INV_TYPE_NAMES: [(&str, i32); 17] = [
    ("texture", 0),
    ("sound", 1),
    ("callcard", 2),
    ("landmark", 3),
    ("object", 6),
    ("notecard", 7),
    ("category", 8),
    ("root", 9),
    ("script", 10),
    ("snapshot", 15),
    ("attach", 17),
    ("wearable", 18),
    ("animation", 19),
    ("gesture", 20),
    ("mesh", 22),
    ("settings", 25),
    ("material", 26),
];

/// A type given as a number or as one of `names`; -1 for anything else.
fn type_number(value: Option<&Value>, names: &[(&str, i32)]) -> i32 {
    match value {
        Some(Value::Int(number)) => i32::try_from(*number).unwrap_or(-1),
        Some(Value::Str(text)) => {
            let text = text.trim();
            let named = names.iter().find(|(name, _)| *name == text).map(|(_, number)| *number);
            named.or_else(|| text.parse().ok()).unwrap_or(-1)
        }
        _ => -1,
    }
}

/// Flags and permission masks are 32 bits that arrive as a signed integer,
/// or as four bytes.
fn bits(value: Option<&Value>) -> u32 {
    match value {
        Some(Value::Binary(bytes)) => <[u8; 4]>::try_from(bytes.as_slice()).map_or(0, u32::from_be_bytes),
        Some(other) => other.as_i64().unwrap_or(0) as u32,
        None => 0,
    }
}

fn uuid_field(map: &Value, key: &str) -> Uuid {
    map.get(key).and_then(Value::as_uuid).unwrap_or_default()
}

fn item_from(map: &Value) -> Option<Item> {
    let id = map.get("item_id").and_then(Value::as_uuid)?;
    let text = |key| map.str_field(key).unwrap_or("").to_string();
    let permissions = map.get("permissions");
    let mask = |key| bits(permissions.and_then(|permissions| permissions.get(key)));
    // Some replies name a link's target separately.
    let linked = map.get("linked_id").and_then(Value::as_uuid).filter(|id| !id.is_nil());
    Some(Item {
        id,
        parent: uuid_field(map, "parent_id"),
        name: text("name"),
        description: text("desc"),
        asset_type: type_number(map.get("type"), &ASSET_TYPE_NAMES),
        inv_type: type_number(map.get("inv_type"), &INV_TYPE_NAMES),
        flags: bits(map.get("flags")),
        asset_id: linked.unwrap_or_else(|| uuid_field(map, "asset_id")),
        owner: permissions
            .map(|permissions| uuid_field(permissions, "owner_id"))
            .unwrap_or_default(),
        group_mask: mask("group_mask"),
        everyone_mask: mask("everyone_mask"),
        next_owner_mask: mask("next_owner_mask"),
    })
}

fn request_map(entries: &[(&str, Value)]) -> Value {
    Value::Map(entries.iter().map(|(key, value)| (key.to_string(), value.clone())).collect())
}

/// The `FetchInventoryDescendents2` request for the contents of `folders`.
pub fn descendents_request(owner: Uuid, folders: &[Uuid], want_folders: bool, want_items: bool) -> String {
    let folders = folders.iter().map(|folder| {
        request_map(&[
            ("folder_id", Value::Uuid(*folder)),
            ("owner_id", Value::Uuid(owner)),
            ("sort_order", Value::Int(0)),
            ("fetch_folders", Value::Bool(want_folders)),
            ("fetch_items", Value::Bool(want_items)),
        ])
    });
    xml::write_llsd(&request_map(&[("folders", Value::Array(folders.collect()))]))
}

/// Parses a `FetchInventoryDescendents2` reply into the folders it
/// describes and the ids of those it calls bad. Only the items that are
/// in a folder count as its contents: OpenSim adds the items its links
/// point to. A folder asked for and not described is unknown to the grid.
pub fn parse_descendents(body: &str) -> Result<(Vec<Contents>, Vec<Uuid>), String> {
    let reply = xml::parse_llsd(body).map_err(|e| e.to_string())?;
    if !matches!(reply, Value::Map(_)) {
        return Err("the inventory reply is not a map".to_string());
    }
    fn list<'a>(map: &'a Value, key: &str) -> &'a [Value] {
        map.get(key).and_then(Value::as_array).unwrap_or(&[])
    }
    let mut described = Vec::new();
    for folder in list(&reply, "folders") {
        let Some(id) = folder.get("folder_id").and_then(Value::as_uuid) else {
            continue;
        };
        let folders = list(folder, "categories").iter().filter_map(|category| {
            Some(Folder {
                id: category.get("category_id").and_then(Value::as_uuid)?,
                name: category.str_field("name").unwrap_or("").to_string(),
                folder_type: type_number(category.get("type_default").or(category.get("type")), &[]),
            })
        });
        let items = list(folder, "items").iter().filter_map(item_from).filter(|item| item.parent == id);
        described.push(Contents {
            folder: id,
            version: folder.get("version").and_then(Value::as_i64).unwrap_or(-1),
            folders: folders.collect(),
            items: items.collect(),
        });
    }
    let bad = list(&reply, "bad_folders")
        .iter()
        .filter_map(|folder| folder.get("folder_id").and_then(Value::as_uuid));
    Ok((described, bad.collect()))
}

/// The `FetchInventory2` request for `items` by id.
pub fn items_request(owner: Uuid, items: &[Uuid]) -> String {
    let items = items
        .iter()
        .map(|item| request_map(&[("owner_id", Value::Uuid(owner)), ("item_id", Value::Uuid(*item))]));
    xml::write_llsd(&request_map(&[
        ("agent_id", Value::Uuid(owner)),
        ("items", Value::Array(items.collect())),
    ]))
}

/// Parses a `FetchInventory2` reply into the items it has; those the grid
/// does not know are simply not in it.
pub fn parse_items(body: &str) -> Result<Vec<Item>, String> {
    let reply = xml::parse_llsd(body).map_err(|e| e.to_string())?;
    if !matches!(reply, Value::Map(_)) {
        return Err("the inventory reply is not a map".to_string());
    }
    let items = reply.get("items").and_then(Value::as_array).unwrap_or(&[]);
    Ok(items.iter().filter_map(item_from).collect())
}

/// Where `InventoryAPIv3` takes new links for `folder`. `tid` marks the
/// request as one transaction (llaisapi.cpp:99-137).
pub fn link_url(api: &str, folder: Uuid, tid: Uuid) -> String {
    format!("{}/category/{folder}?tid={tid}", api.trim_end_matches('/'))
}

/// The body that makes a link to `item`: what `link_inventory_array`
/// sends (llviewerinventory.cpp:1483-1501).
pub fn link_request(item: &Item) -> String {
    let link = request_map(&[
        ("linked_id", Value::Uuid(item.id)),
        ("type", Value::Int(i64::from(ASSET_LINK))),
        ("inv_type", Value::Int(i64::from(item.inv_type))),
        ("name", Value::Str(item.name.clone())),
        ("desc", Value::Str(item.description.clone())),
    ]);
    xml::write_llsd(&request_map(&[("links", Value::Array(vec![link]))]))
}

/// What became of making a link: the id of the link made, when the reply
/// says. Any 2xx answer with a map is success (llaisapi.cpp:877-881).
pub fn parse_link_reply(status: u16, body: &str) -> Result<Option<Uuid>, String> {
    let reply = xml::parse_llsd(body).ok().filter(|reply| matches!(reply, Value::Map(_)));
    match reply {
        Some(reply) if (200..300).contains(&status) => {
            let created = reply.get("_created_items").and_then(Value::as_array).unwrap_or(&[]);
            Ok(created.first().and_then(Value::as_uuid))
        }
        reply => Err(refusal(status, reply.as_ref())),
    }
}

/// Where `InventoryAPIv3` takes the removal of `item` (llaisapi.cpp:219-247).
pub fn remove_url(api: &str, item: Uuid) -> String {
    format!("{}/item/{item}", api.trim_end_matches('/'))
}

/// What became of removing an item. One that is gone already is as good
/// as removed.
pub fn parse_remove_reply(status: u16, body: &str) -> Result<(), String> {
    match status {
        200..=299 | 404 | 410 => Ok(()),
        _ => Err(refusal(status, xml::parse_llsd(body).ok().as_ref())),
    }
}

/// The reason in a refusal: what the grid wrote, or the HTTP status.
fn refusal(status: u16, reply: Option<&Value>) -> String {
    let text = |key| reply.and_then(|reply| reply.str_field(key)).filter(|text| !text.trim().is_empty());
    match text("error_description").or(text("message")).or(text("error")) {
        Some(reason) => format!("{} (HTTP {status})", reason.trim()),
        None => format!("HTTP {status}"),
    }
}

/// The `UpdateAvatarAppearance` request: make the avatar's appearance
/// from the Current Outfit folder at this version (llappearancemgr.cpp:3997).
pub fn appearance_request(version: i64) -> String {
    xml::write_llsd(&request_map(&[("cof_version", Value::Int(version))]))
}

/// What became of the appearance request: accepted, or the grid's reason,
/// with the folder version it expected when it says.
pub fn parse_appearance_reply(status: u16, body: &str) -> Result<(), String> {
    let reply = xml::parse_llsd(body).ok();
    let reply = reply.as_ref();
    let flag = reply.and_then(|reply| reply.get("success"));
    let accepted = matches!(flag, Some(Value::Bool(true))) || flag.and_then(Value::as_i64) == Some(1);
    if accepted && (200..300).contains(&status) {
        return Ok(());
    }
    let mut reason = refusal(status, reply);
    if let Some(expected) = reply.and_then(|reply| reply.get("expected")).and_then(Value::as_i64) {
        reason.push_str(&format!("; the grid expected version {expected} of the Current Outfit folder"));
    }
    Err(reason)
}

// ----------------------------------------------------------------- the jobs

/// Why a job came to nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Failed {
    /// The grid had not answered everything by the job's deadline.
    TimedOut,
    Reason(String),
}

impl From<String> for Failed {
    fn from(reason: String) -> Failed {
        Failed::Reason(reason)
    }
}

/// What `detach` takes off through the outfit service.
#[derive(Debug, Clone, PartialEq)]
pub enum Clothes {
    Item(Item),
    /// Every piece of clothing of this wearable type.
    Kind(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub enum What {
    /// The entries under a folder: all of them, or its direct children.
    List {
        folder: Ref,
        recursive: bool,
    },
    /// The item a command names, links followed.
    Find(Ref),
    /// These items by id; those the grid does not know are left out.
    Items(Vec<Uuid>),
    /// Make sure the Current Outfit folder has a link to this item.
    Link(Item),
    /// Remove the Current Outfit folder's links to this item.
    Unlink(Uuid),
    /// The objects the Current Outfit folder has links to: what should be
    /// attached. Answered with `Outcome::Items`.
    OutfitObjects,
    /// Wear this piece of clothing or body part.
    Wear(Item),
    TakeOff(Clothes),
    /// The folder an item of this asset type is filed in when it is given
    /// to the avatar. Answered with `Outcome::Folder`.
    SystemFolder(i32),
}

/// One command's work for the worker.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: u64,
    /// The circuit the URLs belong to.
    pub generation: u64,
    pub urls: InventoryUrls,
    /// The avatar.
    pub owner: Uuid,
    /// The folder at the top of My Inventory.
    pub root: Uuid,
    /// When the command that needs this gives up.
    pub deadline: Instant,
    pub what: What,
}

/// One line of a listing, before it is known what is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: Uuid,
    /// `ASSET_FOLDER` for a folder.
    pub asset_type: i32,
    pub flags: u32,
    /// The path from the top of My Inventory as it is printed, with a
    /// `/` at the end for a folder.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Listed(Vec<Entry>),
    Found(Item),
    Items(Vec<Item>),
    /// The Current Outfit folder was changed as asked; for `Wear` and
    /// `TakeOff`, the items put on or taken off.
    Outfit(Vec<Item>),
    Folder(Uuid),
}

const LLSD: &str = "application/llsd+xml";
/// The most the worker reads of one reply.
const MAX_REPLY: u64 = 256 * 1024 * 1024;
/// Extra time for putting the Current Outfit folder back after a failure.
const UNDO_GRACE: Duration = Duration::from_secs(10);

/// The grid's inventory as one job sees it.
struct Service<'a> {
    agent: &'a ureq::Agent,
    urls: &'a InventoryUrls,
    owner: Uuid,
    root: Uuid,
    deadline: Instant,
}

fn failed(error: ureq::Error) -> Failed {
    use std::io::ErrorKind::{TimedOut, WouldBlock};
    match error {
        ureq::Error::Timeout(_) => Failed::TimedOut,
        ureq::Error::Io(e) if matches!(e.kind(), TimedOut | WouldBlock) => Failed::TimedOut,
        other => Failed::Reason(format!("the inventory request failed: {other}")),
    }
}

/// A fresh id for one `InventoryAPIv3` transaction.
fn transaction_id() -> Uuid {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // Each `RandomState` is keyed at random by the standard library.
    let random = || RandomState::new().build_hasher().finish().to_le_bytes();
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&random());
    bytes[8..].copy_from_slice(&random());
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

impl Service<'_> {
    /// One request, with whatever time the job has left.
    fn exchange(&self, url: &str, body: Option<&str>) -> Result<(u16, String), Failed> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Failed::TimedOut);
        }
        let answer = match body {
            Some(body) => {
                let request = self.agent.post(url).config().timeout_global(Some(left)).build();
                request.content_type(LLSD).header("Accept", LLSD).send(body)
            }
            None => {
                let request = self.agent.delete(url).config().timeout_global(Some(left)).build();
                request.header("Accept", LLSD).call()
            }
        };
        let mut response = answer.map_err(failed)?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_REPLY)
            .read_to_string()
            .map_err(failed)?;
        Ok((status, body))
    }

    /// A request to one of the two services that read.
    fn read(&self, url: Option<&String>, body: &str) -> Result<String, Failed> {
        let url = url.ok_or_else(|| Failed::Reason(NO_SERVICE.to_string()))?;
        match self.exchange(url, Some(body))? {
            (200, body) => Ok(body),
            (status, _) => Err(Failed::Reason(format!("the inventory service answered HTTP {status}"))),
        }
    }

    /// The contents of `folders`, in one request; those the grid does not
    /// know are left out.
    fn folders(&self, folders: &[Uuid], want_folders: bool, want_items: bool) -> Result<Vec<Contents>, Failed> {
        crate::debug!("inventory: asking for the contents of {} folder(s)", folders.len());
        let request = descendents_request(self.owner, folders, want_folders, want_items);
        let (described, bad) = parse_descendents(&self.read(self.urls.descendents.as_ref(), &request)?)?;
        Ok(described
            .into_iter()
            .filter(|contents| folders.contains(&contents.folder) && !bad.contains(&contents.folder))
            .collect())
    }

    fn folder(&self, folder: Uuid, want_folders: bool, want_items: bool) -> Result<Contents, Failed> {
        let found = self.folders(&[folder], want_folders, want_items)?.pop();
        found.ok_or_else(|| Failed::Reason(format!("the grid knows no inventory folder {folder}")))
    }

    /// The avatar's own items among `ids`.
    fn items(&self, ids: &[Uuid]) -> Result<Vec<Item>, Failed> {
        let mut found = Vec::new();
        for batch in ids.chunks(ITEM_BATCH) {
            crate::debug!("inventory: asking for {} item(s)", batch.len());
            let reply = self.read(self.urls.items.as_ref(), &items_request(self.owner, batch))?;
            let items = parse_items(&reply)?;
            found.extend(
                items
                    .into_iter()
                    .filter(|item| batch.contains(&item.id) && (item.owner.is_nil() || item.owner == self.owner)),
            );
        }
        Ok(found)
    }

    /// The folder a path names, a folder per name from the top.
    fn resolve_folder(&self, names: &[String]) -> Result<Uuid, Failed> {
        let mut folder = self.root;
        for (depth, name) in names.iter().enumerate() {
            let contents = self.folder(folder, true, false)?;
            let found: Vec<Uuid> = contents.folders.iter().filter(|sub| sub.name == *name).map(|sub| sub.id).collect();
            folder = one_of(&found, "folder", name, &names[..depth])?;
        }
        Ok(folder)
    }

    /// The item a command names. A link stands for what it points to.
    fn find(&self, item: &Ref) -> Result<Item, Failed> {
        let found = match item {
            Ref::Id(id) => {
                let items = self.items(&[*id])?;
                let found = items.into_iter().next();
                found.ok_or_else(|| Failed::Reason(format!("there is no item {id} in the avatar's inventory")))?
            }
            Ref::Path(names) => {
                let Some((name, folders)) = names.split_last() else {
                    return Err(Failed::Reason("My Inventory itself is a folder, not an item".to_string()));
                };
                let contents = self.folder(self.resolve_folder(folders)?, false, true)?;
                let found: Vec<Uuid> = contents
                    .items
                    .iter()
                    .filter(|item| item.name == *name)
                    .map(|item| item.id)
                    .collect();
                let id = one_of(&found, "item", name, folders)?;
                contents.items.into_iter().find(|item| item.id == id).expect("picked from these")
            }
        };
        match found.asset_type {
            ASSET_LINK => {
                let target = self.items(&[found.asset_id])?.into_iter().next();
                let target = target.filter(|target| !matches!(target.asset_type, ASSET_LINK | ASSET_LINK_FOLDER));
                target.ok_or_else(|| Failed::Reason(format!("the link {} points to nothing in the avatar's inventory", found.show())))
            }
            ASSET_LINK_FOLDER => Err(Failed::Reason(format!("{} is a link to a folder, not an item", found.show()))),
            _ => Ok(found),
        }
    }

    /// The path of the folder `wanted` from the top of My Inventory,
    /// found by looking through the folders from there.
    fn locate_folder(&self, wanted: Uuid) -> Result<Vec<String>, Failed> {
        if wanted == self.root {
            return Ok(Vec::new());
        }
        let mut seen = HashSet::from([self.root]);
        let mut queue = vec![(self.root, Vec::new())];
        while !queue.is_empty() {
            let batch: Vec<(Uuid, Vec<String>)> = queue.drain(..queue.len().min(FOLDER_BATCH)).collect();
            let ids: Vec<Uuid> = batch.iter().map(|(id, _)| *id).collect();
            for contents in self.folders(&ids, true, false)? {
                let Some((_, path)) = batch.iter().find(|(id, _)| *id == contents.folder) else {
                    continue;
                };
                for sub in contents.folders {
                    let path = [path.as_slice(), &[sub.name]].concat();
                    if sub.id == wanted {
                        return Ok(path);
                    }
                    if seen.insert(sub.id) {
                        queue.push((sub.id, path));
                    }
                }
            }
        }
        Err(Failed::Reason(format!("there is no folder {wanted} in My Inventory")))
    }

    /// The entries under a folder: breadth first, several folders to a
    /// request.
    fn list(&self, folder: &Ref, recursive: bool) -> Result<Vec<Entry>, Failed> {
        let (top, names) = match folder {
            Ref::Path(names) => (self.resolve_folder(names)?, names.clone()),
            Ref::Id(id) => (*id, self.locate_folder(*id)?),
        };
        let prefix: String = names.iter().map(|name| escape_name(name) + "/").collect();
        let mut entries = Vec::new();
        let mut seen = HashSet::from([top]);
        let mut queue = vec![(top, prefix)];
        while !queue.is_empty() {
            let batch: Vec<(Uuid, String)> = queue.drain(..queue.len().min(FOLDER_BATCH)).collect();
            let ids: Vec<Uuid> = batch.iter().map(|(id, _)| *id).collect();
            let described = self.folders(&ids, true, true)?;
            if !described.iter().any(|contents| contents.folder == top) && ids.contains(&top) {
                return Err(Failed::Reason(format!("the grid knows no inventory folder {top}")));
            }
            for contents in described {
                let Some((_, prefix)) = batch.iter().find(|(id, _)| *id == contents.folder) else {
                    continue;
                };
                for sub in contents.folders {
                    let path = format!("{prefix}{}/", escape_name(&sub.name));
                    if recursive && seen.insert(sub.id) {
                        queue.push((sub.id, path.clone()));
                    }
                    entries.push(Entry {
                        id: sub.id,
                        asset_type: ASSET_FOLDER,
                        flags: 0,
                        path,
                    });
                }
                for item in contents.items {
                    entries.push(Entry {
                        id: item.id,
                        asset_type: item.asset_type,
                        flags: item.flags,
                        path: format!("{prefix}{}", escape_name(&item.name)),
                    });
                }
            }
        }
        Ok(entries)
    }

    // --------------------------------------------- the Current Outfit folder

    fn api(&self) -> Result<&str, Failed> {
        self.urls
            .api
            .as_deref()
            .ok_or_else(|| Failed::Reason(NO_OUTFIT_SERVICE.to_string()))
    }

    /// The Current Outfit folder: the one of that type at the top of My
    /// Inventory.
    fn outfit_folder(&self) -> Result<Uuid, Failed> {
        let top = self.folder(self.root, true, false)?;
        let found = top.folders.iter().find(|folder| folder.folder_type == FOLDER_CURRENT_OUTFIT);
        found
            .map(|folder| folder.id)
            .ok_or_else(|| Failed::Reason("the inventory has no Current Outfit folder".to_string()))
    }

    /// Where a viewer files an item of this asset type that is given to
    /// the avatar: the folder of that type at the top of My Inventory
    /// (Objects, Notecards, ...), or the top itself where there is none,
    /// as for a folder. Reads the top level and nothing below it.
    fn system_folder(&self, asset_type: i32) -> Result<Uuid, Failed> {
        if asset_type == ASSET_FOLDER || asset_type < 0 {
            return Ok(self.root);
        }
        let top = self.folder(self.root, true, false)?;
        let found = top.folders.iter().find(|folder| folder.folder_type == asset_type);
        Ok(found.map_or(self.root, |folder| folder.id))
    }

    /// The links in the Current Outfit folder.
    fn outfit_links(&self, folder: Uuid) -> Result<Vec<Item>, Failed> {
        let contents = self.folder(folder, false, true)?;
        Ok(contents.items.into_iter().filter(|item| item.asset_type == ASSET_LINK).collect())
    }

    /// Adds a link to `item` to the folder. Returns the link's id.
    fn add_link(&self, folder: Uuid, item: &Item) -> Result<Uuid, Failed> {
        let url = link_url(self.api()?, folder, transaction_id());
        crate::debug!("inventory: adding a link to {} to the Current Outfit folder", item.show());
        let (status, body) = self.exchange(&url, Some(&link_request(item)))?;
        let refused = |reason| {
            Failed::Reason(format!(
                "the grid refused to add {} to the Current Outfit folder: {reason}",
                item.show()
            ))
        };
        if let Some(link) = parse_link_reply(status, &body).map_err(refused)? {
            return Ok(link);
        }
        // The reply did not say which link it made: look for it.
        let links = self.outfit_links(folder)?;
        let made = links.iter().find(|link| link.asset_id == item.id);
        made.map(|link| link.id)
            .ok_or_else(|| Failed::Reason(format!("the grid did not add {} to the Current Outfit folder", item.show())))
    }

    fn remove_link(&self, link: &Item) -> Result<(), Failed> {
        crate::debug!("inventory: removing the link \"{}\" from the Current Outfit folder", link.name);
        let (status, body) = self.exchange(&remove_url(self.api()?, link.id), None)?;
        let refused = |reason| {
            Failed::Reason(format!(
                "the grid refused to remove \"{}\" from the Current Outfit folder: {reason}",
                link.name
            ))
        };
        parse_remove_reply(status, &body).map_err(refused)
    }

    /// Asks the grid to make the avatar's appearance from the folder as
    /// it is now.
    fn update_appearance(&self, folder: Uuid) -> Result<(), Failed> {
        let version = self.folder(folder, false, true)?.version;
        let url = self
            .urls
            .appearance
            .as_ref()
            .ok_or_else(|| Failed::Reason(NO_OUTFIT_SERVICE.to_string()))?;
        crate::debug!("inventory: asking for the avatar's appearance from version {version} of the Current Outfit folder");
        let (status, body) = self.exchange(url, Some(&appearance_request(version)))?;
        let refused = |reason| Failed::Reason(format!("the grid refused the new outfit: {reason}"));
        parse_appearance_reply(status, &body).map_err(refused)
    }

    /// Carries out `change` on the folder and asks for the new appearance;
    /// when either fails, puts back what was changed.
    fn change_outfit(&self, folder: Uuid, change: impl FnOnce(&mut Vec<Undo>) -> Result<(), Failed>) -> Result<(), Failed> {
        let mut done = Vec::new();
        let failure = match change(&mut done).and_then(|()| self.update_appearance(folder)) {
            Ok(()) => return Ok(()),
            Err(failure) => failure,
        };
        if done.is_empty() {
            return Err(failure);
        }
        // Whatever time is left may be none: the folder is worth a little more.
        let patient = Service {
            deadline: self.deadline.max(Instant::now() + UNDO_GRACE),
            ..*self
        };
        let mut undone = Ok(());
        for step in done.into_iter().rev() {
            let result = match &step {
                Undo::Remove(link) => patient.remove_link(link),
                Undo::Restore(link) => patient.add_link(folder, link).map(|_| ()),
            };
            undone = undone.and(result);
        }
        let reason = match failure {
            Failed::TimedOut => "the grid did not answer in time".to_string(),
            Failed::Reason(reason) => reason,
        };
        Err(Failed::Reason(match undone {
            Ok(()) => format!("{reason}; the Current Outfit folder was put back as it was"),
            Err(_) => format!("{reason}; putting the Current Outfit folder back failed too, so it now differs from what the avatar wears"),
        }))
    }

    /// The links of the folder and the items they point to.
    fn outfit(&self, folder: Uuid) -> Result<(Vec<Item>, Vec<Item>), Failed> {
        let links = self.outfit_links(folder)?;
        let ids: Vec<Uuid> = links.iter().map(|link| link.asset_id).collect();
        let targets = self.items(&ids)?;
        Ok((links, targets))
    }

    /// The objects the Current Outfit folder links to. Links to clothing
    /// and body parts are left out, and so are links to items the grid no
    /// longer has, which it does not return.
    fn outfit_objects(&self) -> Result<Vec<Item>, Failed> {
        let folder = self.outfit_folder()?;
        let (_, targets) = self.outfit(folder)?;
        let mut objects: Vec<Item> = Vec::new();
        for target in targets.into_iter().filter(|target| target.asset_type == ASSET_OBJECT) {
            if !objects.iter().any(|known| known.id == target.id) {
                objects.push(target);
            }
        }
        Ok(objects)
    }

    fn wear(&self, item: &Item) -> Result<Vec<Item>, Failed> {
        let folder = self.outfit_folder()?;
        let (links, targets) = self.outfit(folder)?;
        if links.iter().any(|link| link.asset_id == item.id) {
            return Err(Failed::Reason(format!("{} is worn already", item.show())));
        }
        // The avatar wears one body part of each kind: the new one takes
        // the place of the old.
        let same_kind = |link: &&Item| {
            let target = targets.iter().find(|target| target.id == link.asset_id);
            target.is_some_and(|target| target.asset_type == ASSET_BODY_PART && target.subtype() == item.subtype())
        };
        let replaced: Vec<&Item> = links
            .iter()
            .filter(same_kind)
            .filter(|_| item.asset_type == ASSET_BODY_PART)
            .collect();
        self.change_outfit(folder, |done| {
            for link in replaced {
                self.remove_link(link)?;
                done.push(Undo::Restore(as_target(link)));
            }
            let id = self.add_link(folder, item)?;
            done.push(Undo::Remove(Item { id, ..item.clone() }));
            Ok(())
        })?;
        Ok(vec![item.clone()])
    }

    fn take_off(&self, clothes: &Clothes) -> Result<Vec<Item>, Failed> {
        let folder = self.outfit_folder()?;
        let (links, targets) = self.outfit(folder)?;
        let target = |link: &Item| targets.iter().find(|target| target.id == link.asset_id);
        let chosen: Vec<&Item> = match clothes {
            Clothes::Item(item) => links.iter().filter(|link| link.asset_id == item.id).collect(),
            Clothes::Kind(kind) => {
                let of_kind =
                    |link: &&Item| target(link).is_some_and(|target| target.asset_type == ASSET_CLOTHING && target.subtype() == *kind);
                links.iter().filter(of_kind).collect()
            }
        };
        if chosen.is_empty() {
            return Err(Failed::Reason(match clothes {
                Clothes::Item(item) => format!("{} is not worn", item.show()),
                Clothes::Kind(kind) => format!("the avatar wears no {}", kind_name(*kind)),
            }));
        }
        self.change_outfit(folder, |done| {
            for link in &chosen {
                self.remove_link(link)?;
                done.push(Undo::Restore(as_target(link)));
            }
            Ok(())
        })?;
        // Two links to one item are one item taken off.
        let mut taken: Vec<Item> = Vec::new();
        for link in chosen {
            let item = target(link).cloned().unwrap_or_else(|| as_target(link));
            if !taken.iter().any(|known| known.id == item.id) {
                taken.push(item);
            }
        }
        Ok(taken)
    }

    fn link(&self, item: &Item) -> Result<(), Failed> {
        let folder = self.outfit_folder()?;
        if self.outfit_links(folder)?.iter().any(|link| link.asset_id == item.id) {
            return Ok(());
        }
        self.add_link(folder, item).map(|_| ())
    }

    fn unlink(&self, item: Uuid) -> Result<(), Failed> {
        let folder = self.outfit_folder()?;
        for link in self.outfit_links(folder)?.iter().filter(|link| link.asset_id == item) {
            self.remove_link(link)?;
        }
        Ok(())
    }

    fn carry_out(&self, what: &What) -> Result<Outcome, Failed> {
        Ok(match what {
            What::List { folder, recursive } => Outcome::Listed(self.list(folder, *recursive)?),
            What::Find(item) => Outcome::Found(self.find(item)?),
            What::Items(ids) => Outcome::Items(self.items(ids)?),
            What::Link(item) => {
                self.link(item)?;
                Outcome::Outfit(Vec::new())
            }
            What::Unlink(item) => {
                self.unlink(*item)?;
                Outcome::Outfit(Vec::new())
            }
            What::OutfitObjects => Outcome::Items(self.outfit_objects()?),
            What::Wear(item) => Outcome::Outfit(self.wear(item)?),
            What::TakeOff(clothes) => Outcome::Outfit(self.take_off(clothes)?),
            What::SystemFolder(asset_type) => Outcome::Folder(self.system_folder(*asset_type)?),
        })
    }
}

/// A change to the Current Outfit folder to take back.
enum Undo {
    /// Remove the link with this id that was added.
    Remove(Item),
    /// Add a link to this item again.
    Restore(Item),
}

/// What a link says of the item it points to: enough to make the link
/// again.
fn as_target(link: &Item) -> Item {
    Item {
        id: link.asset_id,
        ..link.clone()
    }
}

/// Why inventory commands are refused where the simulator has no
/// inventory service, or the login gave no root folder.
pub const NO_SERVICE: &str = "this simulator offers no inventory service";
/// Why clothing is refused where the grid leaves the avatar's appearance
/// to the viewer.
pub const NO_OUTFIT_SERVICE: &str = "this grid makes the avatar's appearance in the viewer, which the bot cannot do; \
                                     clothing and body parts can only be changed where the grid makes it (Second Life)";

/// The one entry among `found` that a name of a path stands for.
fn one_of(found: &[Uuid], kind: &str, name: &str, within: &[String]) -> Result<Uuid, Failed> {
    let place = Ref::Path(within.to_vec()).show();
    match found {
        [one] => Ok(*one),
        [] => Err(Failed::Reason(format!(
            "no {kind} named \"{}\" was found in {place}",
            escape_name(name)
        ))),
        several => {
            let ids: Vec<String> = several.iter().map(Uuid::to_string).collect();
            Err(Failed::Reason(format!(
                "{place} holds {} {kind}s named \"{}\": {}; name one by its UUID",
                several.len(),
                escape_name(name),
                ids.join(", ")
            )))
        }
    }
}

/// Carries out one job against the grid.
fn carry_out(agent: &ureq::Agent, job: &Job) -> Result<Outcome, Failed> {
    let service = Service {
        agent,
        urls: &job.urls,
        owner: job.owner,
        root: job.root,
        deadline: job.deadline,
    };
    service.carry_out(&job.what)
}

/// Worker thread body: carries the jobs out one after another and answers
/// each with `Event::Inventory`, until either channel closes. A job whose
/// deadline passed while it waited is answered as timed out.
fn run(jobs: Receiver<Job>, events: Sender<Event>) {
    let agent = crate::caps::http_agent(LIMIT);
    for job in jobs {
        let result = carry_out(&agent, &job);
        if let Err(Failed::Reason(reason)) = &result {
            crate::debug!("inventory job {}: {reason}", job.id);
        }
        let answer = Event::Inventory {
            id: job.id,
            generation: job.generation,
            result,
        };
        if events.send(answer).is_err() {
            return;
        }
    }
}

/// Starts the worker; jobs sent to the returned channel are answered on
/// `events`.
pub fn spawn(events: Sender<Event>) -> Sender<Job> {
    let (sender, jobs) = mpsc::channel();
    thread::spawn(move || run(jobs, events));
    sender
}

// -------------------------------------------------------------- the listing

/// The text of a listing: a line per entry with its UUID, type, attach or
/// wear point, `attached` or `-`, and path, separated by tabs and in the
/// order of the paths. `attached` says which items are on the avatar now
/// and where; with `attached_only`, nothing else is listed. Returns the
/// text and the number of lines.
pub fn listing(entries: &[Entry], attached: &HashMap<Uuid, u8>, attached_only: bool) -> (String, usize) {
    let mut lines = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        // A link has an id of its own and is never what is attached.
        let on = Some(entry)
            .filter(|entry| entry.asset_type == ASSET_OBJECT)
            .and_then(|entry| attached.get(&entry.id));
        if attached_only && on.is_none() {
            continue;
        }
        let subtype = (entry.flags & 0xFF) as u8;
        let point = match (entry.asset_type, on) {
            (ASSET_OBJECT, Some(point)) => point_name(*point),
            (ASSET_OBJECT, None) if subtype != 0 => point_name(subtype),
            (ASSET_CLOTHING | ASSET_BODY_PART, _) => kind_name(subtype),
            _ => "-".to_string(),
        };
        let mark = if on.is_some() { "attached" } else { "-" };
        let line = format!("{}\t{}\t{point}\t{mark}\t{}", entry.id, type_name(entry.asset_type), entry.path);
        lines.insert((entry.path.as_bytes(), index), line);
    }
    let count = lines.len();
    (lines.into_values().collect::<Vec<_>>().join("\n"), count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Result<Vec<String>, String> {
        Ok(names.iter().map(|name| name.to_string()).collect())
    }

    fn path(parts: &[&str]) -> Ref {
        Ref::Path(names(parts).unwrap())
    }

    const RADIO: &str = "11111111-2222-3333-4444-555555555555";

    #[test]
    fn paths_are_names_separated_by_slashes() {
        assert_eq!(parse_path("Objects/Bot gear/Radio"), names(&["Objects", "Bot gear", "Radio"]));
        assert_eq!(parse_path("Radio"), names(&["Radio"]));
        // A slash at either end is ignored.
        assert_eq!(parse_path("/Objects/Radio"), names(&["Objects", "Radio"]));
        assert_eq!(parse_path("Objects/Bot gear/"), names(&["Objects", "Bot gear"]));
        assert_eq!(parse_path("/Chest"), names(&["Chest"]));
        assert_eq!(parse_path("/-n/"), names(&["-n"]));
        // Names are taken as they are, spaces and case included.
        assert_eq!(parse_path(" objects /Radio "), names(&[" objects ", "Radio "]));
    }

    #[test]
    fn escapes_put_slashes_and_backslashes_into_names() {
        assert_eq!(parse_path("Objects/Radio\\/TV"), names(&["Objects", "Radio/TV"]));
        assert_eq!(parse_path("a\\\\b/c"), names(&["a\\b", "c"]));
        // An escaped slash at the end belongs to the name.
        assert_eq!(parse_path("Objects/Radio\\/"), names(&["Objects", "Radio/"]));
        assert_eq!(parse_path("\\/x"), names(&["/x"]));
        // What a listing writes for a tab or a line break is read back.
        assert_eq!(parse_path("a\\tb\\nc\\rd"), names(&["a\tb\nc\rd"]));
        // Any other backslash stands for itself.
        assert_eq!(parse_path("a\\xb"), names(&["a\\xb"]));
        assert_eq!(parse_path("dangling\\"), names(&["dangling\\"]));
    }

    #[test]
    fn the_empty_path_is_my_inventory() {
        assert_eq!(parse_path(""), names(&[]));
        assert_eq!(parse_path("/"), names(&[]));
        assert_eq!(parse_path("//"), names(&[]));
        assert!(parse_path("a//b").unwrap_err().contains("empty name"));
        assert!(parse_path("///").is_err());
    }

    #[test]
    fn names_are_escaped_the_way_paths_read_them() {
        for name in ["Radio/TV", "back\\slash", "tab\there", "line\nbreak\r", "plain", "\\/"] {
            let shown = escape_name(name);
            assert!(!shown.contains(['\t', '\n', '\r']), "{shown:?}");
            assert_eq!(parse_path(&shown), names(&[name]), "{shown:?}");
        }
        assert_eq!(escape_name("Radio/TV"), "Radio\\/TV");
        assert_eq!(show_path(&names(&["Objects", "Radio/TV"]).unwrap()), "Objects/Radio\\/TV");
    }

    #[test]
    fn a_uuid_is_read_as_one_and_anything_else_as_a_path() {
        let id = Uuid::parse_str(RADIO).unwrap();
        assert_eq!(parse_ref(RADIO), Ok(Ref::Id(id)));
        assert_eq!(parse_ref(&RADIO.to_uppercase()), Ok(Ref::Id(id)));
        assert_eq!(parse_ref("Objects/Radio"), Ok(path(&["Objects", "Radio"])));
        // With a slash in front it is a name that looks like a UUID.
        assert_eq!(parse_ref(&format!("/{RADIO}")), Ok(path(&[RADIO])));
        // Only the usual form counts.
        let bare = RADIO.replace('-', "");
        assert_eq!(parse_ref(&bare), Ok(path(&[&bare])));
        assert_eq!(parse_ref(&format!("{{{RADIO}}}")), Ok(path(&[&format!("{{{RADIO}}}")])));
        assert_eq!(parse_ref(""), Ok(path(&[])));
        assert_eq!(Ref::Id(id).show(), RADIO);
        assert_eq!(path(&["Objects", "Radio"]).show(), "\"Objects/Radio\"");
        assert_eq!(path(&[]).show(), "My Inventory");
    }

    fn list(args: &[&str]) -> Result<ListArgs, String> {
        parse_list_args(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    fn listing(folder: Ref, attached: bool, recursive: bool) -> Result<ListArgs, String> {
        Ok(ListArgs {
            folder,
            attached,
            recursive,
        })
    }

    #[test]
    fn inventory_arguments_are_flags_and_one_folder() {
        assert_eq!(list(&[]), listing(path(&[]), false, true));
        assert_eq!(list(&["Objects/Bot gear"]), listing(path(&["Objects", "Bot gear"]), false, true));
        assert_eq!(list(&["-n", "Objects"]), listing(path(&["Objects"]), false, false));
        assert_eq!(list(&["Objects", "-n"]), listing(path(&["Objects"]), false, false));
        assert_eq!(list(&["-a"]), listing(path(&[]), true, true));
        assert_eq!(list(&["-a", "Clothing", "-n"]), listing(path(&["Clothing"]), true, false));
        assert_eq!(list(&["-n", "-a"]), listing(path(&[]), true, false));
        // Both spellings.
        assert_eq!(list(&["--attached", "--no-recurse"]), list(&["-a", "-n"]));
        // A folder by UUID, and one that would otherwise be a flag.
        assert_eq!(list(&[RADIO]), listing(Ref::Id(Uuid::parse_str(RADIO).unwrap()), false, true));
        assert_eq!(list(&["/-n"]), listing(path(&["-n"]), false, true));
        assert_eq!(list(&["/"]), listing(path(&[]), false, true));
    }

    #[test]
    fn inventory_refuses_unknown_flags_and_two_folders() {
        assert!(list(&["-x"]).unwrap_err().contains("`-x`"));
        assert!(list(&["Objects", "--all"]).unwrap_err().contains("`--all`"));
        assert!(list(&["-an"]).unwrap_err().contains("`-an`"));
        let reason = list(&["Objects/Bot", "gear"]).unwrap_err();
        assert!(
            reason.contains("\"Objects/Bot\"") && reason.contains("\"gear\"") && reason.contains("quoted"),
            "{reason}"
        );
        assert!(list(&["a//b"]).unwrap_err().contains("empty name"));
    }

    #[test]
    fn attachment_points_have_names_and_numbers() {
        assert_eq!(parse_point("Left Hand"), Some(5));
        assert_eq!(parse_point("left hand"), Some(5));
        assert_eq!(parse_point("  LEFT   hand "), Some(5));
        assert_eq!(parse_point("5"), Some(5));
        assert_eq!(parse_point("Chest"), Some(1));
        assert_eq!(parse_point("r upper arm"), Some(18));
        assert_eq!(parse_point("Right Hind Foot"), Some(55));
        // The heads-up display's points carry `HUD`; the body has no `Center`.
        assert_eq!(parse_point("HUD Center"), Some(35));
        assert_eq!(parse_point("hud center 2"), Some(31));
        assert_eq!(parse_point("Center"), None);
        assert_eq!(parse_point("Avatar Center"), Some(40));
        // Numbers beyond the names are points all the same, up to 127.
        assert_eq!(parse_point("56"), Some(56));
        assert_eq!(parse_point("127"), Some(127));
        for not in ["0", "128", "300", "-1", "elbow", "", "5.0", "Left"] {
            assert_eq!(parse_point(not), None, "{not:?}");
        }
        assert_eq!(point_name(5), "Left Hand");
        assert_eq!(point_name(1), "Chest");
        assert_eq!(point_name(38), "HUD Bottom Right");
        assert_eq!(point_name(55), "Right Hind Foot");
        assert_eq!(point_name(56), "56");
        assert_eq!(point_name(0), "0");
        // Every name leads back to its number.
        for point in 1..=55 {
            assert_eq!(parse_point(&point_name(point)), Some(point));
        }
    }

    #[test]
    fn kinds_of_clothing_have_names() {
        assert_eq!(parse_kind("shirt"), Some(4));
        assert_eq!(parse_kind("Shirt "), Some(4));
        assert_eq!(parse_kind("shape"), Some(0));
        assert_eq!(parse_kind("eyes"), Some(3));
        assert_eq!(parse_kind("universal"), Some(16));
        assert_eq!(parse_kind("hat"), None);
        assert_eq!(parse_kind("4"), None);
        assert_eq!(kind_name(4), "shirt");
        assert_eq!(kind_name(1), "skin");
        assert_eq!(kind_name(15), "physics");
        assert_eq!(kind_name(40), "40");
        assert!(is_body_part(0) && is_body_part(1) && is_body_part(2) && is_body_part(3));
        assert!(!is_body_part(4) && !is_body_part(16));
        for kind in 0..17 {
            assert_eq!(parse_kind(&kind_name(kind)), Some(kind));
        }
    }

    #[test]
    fn detach_reads_a_uuid_then_a_point_or_kind_then_a_path() {
        let id = Uuid::parse_str(RADIO).unwrap();
        assert_eq!(parse_removal(RADIO), Ok(Removal::Item(Ref::Id(id))));
        assert_eq!(parse_removal("left hand"), Ok(Removal::Point(5)));
        assert_eq!(parse_removal("Chest"), Ok(Removal::Point(1)));
        assert_eq!(parse_removal("5"), Ok(Removal::Point(5)));
        assert_eq!(parse_removal("shirt"), Ok(Removal::Kind(4)));
        assert_eq!(parse_removal("Skin"), Ok(Removal::Kind(1)));
        // A slash in front makes a path of what would be a point.
        assert_eq!(parse_removal("/Chest"), Ok(Removal::Item(path(&["Chest"]))));
        assert_eq!(parse_removal("/shirt"), Ok(Removal::Item(path(&["shirt"]))));
        assert_eq!(parse_removal("Objects/Radio"), Ok(Removal::Item(path(&["Objects", "Radio"]))));
        // Neither a point nor a kind: a path of one name.
        assert_eq!(parse_removal("left elbow"), Ok(Removal::Item(path(&["left elbow"]))));
        assert_eq!(parse_removal("0"), Ok(Removal::Item(path(&["0"]))));
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    const AGENT: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const OBJECTS: &str = "0b1ec750-0000-0000-0000-000000000002";
    const RADIO: &str = "11111111-2222-3333-4444-555555555555";

    fn uuid(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    #[test]
    fn descendents_request_names_the_folders() {
        let request = descendents_request(uuid(AGENT), &[uuid(OBJECTS), Uuid::nil()], true, false);
        let folder = |id: &str| {
            format!(
                "<map><key>fetch_folders</key><boolean>1</boolean><key>fetch_items</key><boolean>0</boolean>\
                 <key>folder_id</key><uuid>{id}</uuid><key>owner_id</key><uuid>{AGENT}</uuid>\
                 <key>sort_order</key><integer>0</integer></map>"
            )
        };
        let expected = format!(
            "<llsd><map><key>folders</key><array>{}{}</array></map></llsd>",
            folder(OBJECTS),
            folder(&Uuid::nil().to_string())
        );
        assert!(request.ends_with(&expected), "{request}");
        assert!(descendents_request(uuid(AGENT), &[uuid(OBJECTS)], false, true)
            .contains("<key>fetch_folders</key><boolean>0</boolean><key>fetch_items</key><boolean>1</boolean>"));
    }

    /// A reply as the viewer reads one from Second Life: types by name,
    /// flags and masks as signed integers, a link naming its target, and a
    /// folder the grid does not know.
    #[test]
    fn parses_a_reply_with_the_viewers_field_names() {
        let body = format!(
            "<llsd><map><key>folders</key><array><map>\
               <key>folder_id</key><uuid>{OBJECTS}</uuid><key>owner_id</key><uuid>{AGENT}</uuid>\
               <key>agent_id</key><uuid>{AGENT}</uuid><key>version</key><integer>17</integer>\
               <key>descendents</key><integer>3</integer>\
               <key>categories</key><array><map>\
                 <key>category_id</key><uuid>0b1ec750-0000-0000-0000-000000000003</uuid>\
                 <key>parent_id</key><uuid>{OBJECTS}</uuid><key>name</key><string>Bot gear</string>\
                 <key>type_default</key><integer>-1</integer><key>version</key><integer>4</integer>\
               </map></array>\
               <key>items</key><array><map>\
                 <key>item_id</key><uuid>{RADIO}</uuid><key>parent_id</key><uuid>{OBJECTS}</uuid>\
                 <key>asset_id</key><uuid>a55e7000-0000-0000-0000-000000000004</uuid>\
                 <key>name</key><string>Radio</string><key>desc</key><string>plays &amp; hums</string>\
                 <key>type</key><string>object</string><key>inv_type</key><string>attach</string>\
                 <key>flags</key><integer>261</integer><key>created_at</key><integer>1700000000</integer>\
                 <key>permissions</key><map>\
                   <key>creator_id</key><uuid>{AGENT}</uuid><key>owner_id</key><uuid>{AGENT}</uuid>\
                   <key>last_owner_id</key><uuid>{AGENT}</uuid><key>group_id</key><uuid>{nil}</uuid>\
                   <key>is_owner_group</key><boolean>0</boolean>\
                   <key>base_mask</key><integer>2147483647</integer><key>owner_mask</key><integer>2147483647</integer>\
                   <key>group_mask</key><integer>0</integer><key>everyone_mask</key><integer>32768</integer>\
                   <key>next_owner_mask</key><integer>-1</integer>\
                 </map>\
                 <key>sale_info</key><map><key>sale_type</key><string>not</string><key>sale_price</key><integer>0</integer></map>\
               </map><map>\
                 <key>item_id</key><uuid>11111111-2222-3333-4444-000000000009</uuid><key>parent_id</key><uuid>{OBJECTS}</uuid>\
                 <key>linked_id</key><uuid>{RADIO}</uuid><key>name</key><string>Radio</string><key>desc</key><string></string>\
                 <key>type</key><string>link</string><key>inv_type</key><string>object</string><key>flags</key><integer>0</integer>\
               </map><map>\
                 <key>item_id</key><uuid>11111111-2222-3333-4444-00000000000a</uuid><key>parent_id</key><uuid>{OBJECTS}</uuid>\
                 <key>asset_id</key><uuid>a55e7000-0000-0000-0000-000000000005</uuid>\
                 <key>name</key><string>Dance</string><key>desc</key><string></string>\
                 <key>type</key><string>animatn</string><key>inv_type</key><string>animation</string>\
                 <key>flags</key><binary>AAAAAA==</binary>\
               </map></array>\
             </map></array>\
             <key>bad_folders</key><array><map>\
               <key>folder_id</key><uuid>{nil}</uuid><key>error</key><string>Unknown</string>\
             </map></array></map></llsd>",
            nil = Uuid::nil()
        );
        let (described, bad) = parse_descendents(&body).unwrap();
        assert_eq!(bad, [Uuid::nil()]);
        assert_eq!(described.len(), 1);
        let contents = &described[0];
        assert_eq!((contents.folder, contents.version), (uuid(OBJECTS), 17));
        assert_eq!(
            contents.folders,
            [Folder {
                id: uuid("0b1ec750-0000-0000-0000-000000000003"),
                name: "Bot gear".into(),
                folder_type: -1,
            }]
        );
        assert_eq!(
            contents.items[0],
            Item {
                id: uuid(RADIO),
                parent: uuid(OBJECTS),
                name: "Radio".into(),
                description: "plays & hums".into(),
                asset_type: ASSET_OBJECT,
                inv_type: 17,
                flags: 0x105,
                asset_id: uuid("a55e7000-0000-0000-0000-000000000004"),
                owner: uuid(AGENT),
                group_mask: 0,
                everyone_mask: 0x8000,
                next_owner_mask: 0xFFFF_FFFF,
            }
        );
        assert_eq!(contents.items[0].subtype(), 5);
        // The link points at the radio.
        assert_eq!(
            (contents.items[1].asset_type, contents.items[1].asset_id),
            (ASSET_LINK, uuid(RADIO))
        );
        assert_eq!(
            (contents.items[2].asset_type, contents.items[2].inv_type, contents.items[2].flags),
            (ASSET_ANIMATION, 19, 0)
        );
    }

    /// A reply as OpenSim's source writes one (FetchInvDescHandler.cs):
    /// numbers for the types, the item a link points to added to the same
    /// list, and a second map for bad folders that an LLSD reader never
    /// sees.
    #[test]
    fn parses_a_reply_as_opensim_writes_it() {
        let item = |id: &str, parent: &str, asset: &str, kind: i32, name: &str| {
            format!(
                "<map><key>parent_id</key><uuid>{parent}</uuid><key>asset_id</key><uuid>{asset}</uuid>\
                 <key>item_id</key><uuid>{id}</uuid>\
                 <key>permissions</key><map><key>creator_id</key><uuid>{AGENT}</uuid><key>owner_id</key><uuid>{AGENT}</uuid>\
                 <key>group_id</key><uuid>{nil}</uuid><key>base_mask</key><integer>647168</integer>\
                 <key>owner_mask</key><integer>647168</integer><key>group_mask</key><integer>0</integer>\
                 <key>everyone_mask</key><integer>0</integer><key>next_owner_mask</key><integer>581632</integer>\
                 <key>is_owner_group</key><boolean>0</boolean></map>\
                 <key>type</key><integer>{kind}</integer><key>inv_type</key><integer>6</integer><key>flags</key><integer>2</integer>\
                 <key>sale_info</key><map><key>sale_price</key><integer>0</integer><key>sale_type</key><integer>0</integer></map>\
                 <key>name</key><string>{name}</string><key>desc</key><string></string><key>created_at</key><integer>1700000000</integer></map>",
                nil = Uuid::nil()
            )
        };
        let outfit = "0b1ec750-0000-0000-0000-00000000002e";
        let body = format!(
            "<llsd><map><key>folders</key><array><map>\
               <key>folder_id</key><uuid>{outfit}</uuid><key>agent_id</key><uuid>{AGENT}</uuid>\
               <key>owner_id</key><uuid>{AGENT}</uuid><key>descendents</key><integer>1</integer>\
               <key>version</key><integer>9</integer><key>categories</key><array /><key>items</key><array>{}{}</array>\
             </map></array></map>\
             <map><key>bad_folders</key><array><map><key>folder_id</key><uuid>{OBJECTS}</uuid>\
             <key>error</key><string>Unknown</string></map></array></map></llsd>",
            // The target first, in its own folder, then the link.
            item(RADIO, OBJECTS, "a55e7000-0000-0000-0000-000000000004", 6, "Hat"),
            item("11111111-2222-3333-4444-000000000009", outfit, RADIO, 24, "Hat"),
        );
        let (described, bad) = parse_descendents(&body).unwrap();
        // The second map is out of an LLSD reader's sight.
        assert!(bad.is_empty());
        assert_eq!(described.len(), 1);
        let contents = &described[0];
        assert_eq!((contents.folder, contents.version, contents.folders.len()), (uuid(outfit), 9, 0));
        // Only the link is in the folder.
        assert_eq!(contents.items.len(), 1);
        let link = &contents.items[0];
        assert_eq!(
            (link.asset_type, link.asset_id, link.inv_type, link.flags),
            (ASSET_LINK, uuid(RADIO), 6, 2)
        );
        assert_eq!((link.owner, link.next_owner_mask), (uuid(AGENT), 581_632));
    }

    /// A reply recorded from the local OpenSim rig: the estate owner's
    /// Current Outfit folder, asked for together with a folder that does
    /// not exist. OpenSim describes the six items the links point to in the
    /// same list, each with the folder it is really in, and describes the
    /// unknown folder as an empty one, with an empty version.
    #[test]
    fn parses_a_recorded_opensim_reply() {
        let body = include_str!("testdata/opensim_descendents_reply.xml");
        assert_eq!(body.matches("<key>item_id</key>").count(), 12);
        let (described, bad) = parse_descendents(body).unwrap();
        assert!(bad.is_empty());
        assert_eq!(described.len(), 2);
        let outfit = &described[0];
        assert_eq!(outfit.folder, uuid("3c5d2925-32c4-4b0e-bddb-3d5c6bd42364"));
        assert_eq!((outfit.version, outfit.folders.len()), (9, 0));
        // Only the six links are the folder's own.
        assert_eq!(outfit.items.len(), 6);
        assert!(outfit
            .items
            .iter()
            .all(|item| item.asset_type == ASSET_LINK && item.parent == outfit.folder));
        assert!(outfit
            .items
            .iter()
            .all(|item| item.owner == uuid("7e570000-0000-4000-8000-0000000000aa")));
        let mut names: Vec<&str> = outfit.items.iter().map(|item| item.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "Default Eyes",
                "Default Hair",
                "Default Pants",
                "Default Shape",
                "Default Shirt",
                "Default Skin"
            ]
        );
        // A link says what it points to, and its flags carry the kind.
        let shirt = outfit.items.iter().find(|item| item.name == "Default Shirt").unwrap();
        assert!(shirt.asset_id.to_string().starts_with("2587be7a"));
        assert_eq!((shirt.subtype(), shirt.inv_type), (4, INV_WEARABLE));
        let unknown = &described[1];
        assert_eq!(unknown.folder, uuid("0b1ec750-dead-4000-8000-000000000000"));
        assert_eq!((unknown.version, unknown.folders.len(), unknown.items.len()), (0, 0, 0));
    }

    #[test]
    fn replies_that_make_no_sense_are_refused() {
        assert!(parse_descendents("not xml").is_err());
        assert!(parse_descendents("<llsd><string>no</string></llsd>").is_err());
        assert!(parse_items("<llsd><array /></llsd>").is_err());
        // Nothing in it is nothing found.
        assert_eq!(parse_descendents("<llsd><map></map></llsd>"), Ok((vec![], vec![])));
        assert_eq!(parse_items("<llsd><map></map></llsd>"), Ok(vec![]));
        // Entries without an id are passed over.
        let odd = "<llsd><map><key>folders</key><array><map><key>version</key><integer>1</integer></map></array>\
                   <key>items</key><array><map><key>name</key><string>x</string></map></array></map></llsd>";
        assert_eq!(parse_descendents(odd), Ok((vec![], vec![])));
        assert_eq!(parse_items(odd), Ok(vec![]));
    }

    #[test]
    fn items_request_and_reply() {
        let request = items_request(uuid(AGENT), &[uuid(RADIO)]);
        assert!(
            request.ends_with(&format!(
                "<llsd><map><key>agent_id</key><uuid>{AGENT}</uuid><key>items</key><array><map>\
                 <key>item_id</key><uuid>{RADIO}</uuid><key>owner_id</key><uuid>{AGENT}</uuid></map></array></map></llsd>"
            )),
            "{request}"
        );
        let reply = format!(
            "<llsd><map><key>agent_id</key><uuid>{AGENT}</uuid><key>items</key><array><map>\
             <key>item_id</key><uuid>{RADIO}</uuid><key>parent_id</key><uuid>{OBJECTS}</uuid>\
             <key>asset_id</key><uuid>a55e7000-0000-0000-0000-000000000004</uuid>\
             <key>type</key><integer>5</integer><key>inv_type</key><integer>18</integer><key>flags</key><integer>4</integer>\
             <key>name</key><string>Blue shirt</string><key>desc</key><string>cotton</string>\
             <key>permissions</key><map><key>owner_id</key><uuid>{AGENT}</uuid></map></map></array></map></llsd>"
        );
        let items = parse_items(&reply).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            (items[0].id, items[0].name.as_str(), items[0].description.as_str()),
            (uuid(RADIO), "Blue shirt", "cotton")
        );
        assert_eq!(
            (items[0].asset_type, items[0].inv_type, items[0].subtype()),
            (ASSET_CLOTHING, 18, 4)
        );
        assert_eq!(items[0].owner, uuid(AGENT));
    }

    #[test]
    fn types_have_names_for_the_listing_and_for_errors() {
        let names: Vec<&str> = [0, 1, 2, 3, 5, 6, 7, 8, 10, 13, 20, 21, 24, 25, 49, -1]
            .iter()
            .map(|asset_type| type_name(*asset_type))
            .collect();
        assert_eq!(
            names,
            [
                "texture",
                "sound",
                "callingcard",
                "landmark",
                "clothing",
                "object",
                "notecard",
                "folder",
                "script",
                "bodypart",
                "animation",
                "gesture",
                "link",
                "link",
                "other",
                "other"
            ]
        );
        assert_eq!(type_phrase(ASSET_OBJECT), "an object");
        assert_eq!(type_phrase(ASSET_ANIMATION), "an animation");
        assert_eq!(type_phrase(ASSET_NOTECARD), "a notecard");
        assert_eq!(type_phrase(ASSET_SOUND), "a sound");
        assert_eq!(type_phrase(ASSET_CLOTHING), "a piece of clothing");
        assert_eq!(type_phrase(ASSET_BODY_PART), "a body part");
        assert!(type_phrase(49).contains("49"));
    }

    // ------------------------------------------------- the outfit service

    #[test]
    fn a_link_is_made_in_the_folder_through_the_inventory_api() {
        let (folder, tid) = (uuid(OBJECTS), uuid(RADIO));
        assert_eq!(
            link_url("https://sim/cap/abc", folder, tid),
            format!("https://sim/cap/abc/category/{OBJECTS}?tid={RADIO}")
        );
        assert_eq!(
            link_url("https://sim/cap/abc/", folder, tid),
            format!("https://sim/cap/abc/category/{OBJECTS}?tid={RADIO}")
        );
        let shirt = Item {
            id: uuid(RADIO),
            name: "Blue <shirt>".into(),
            description: "cotton".into(),
            asset_type: ASSET_CLOTHING,
            inv_type: INV_WEARABLE,
            flags: 4,
            ..Item::default()
        };
        let request = link_request(&shirt);
        assert!(
            request.ends_with(&format!(
                "<llsd><map><key>links</key><array><map><key>desc</key><string>cotton</string>\
                 <key>inv_type</key><integer>18</integer><key>linked_id</key><uuid>{RADIO}</uuid>\
                 <key>name</key><string>Blue &lt;shirt&gt;</string><key>type</key><integer>24</integer>\
                 </map></array></map></llsd>"
            )),
            "{request}"
        );
        // Two transactions are two ids.
        assert_ne!(transaction_id(), transaction_id());
        assert_eq!(transaction_id().get_version_num(), 4);
    }

    #[test]
    fn link_replies_say_what_was_made_or_why_not() {
        let made = format!(
            "<llsd><map><key>_created_items</key><array><uuid>{RADIO}</uuid></array>\
             <key>_updated_category_versions</key><map><key>{OBJECTS}</key><integer>12</integer></map></map></llsd>"
        );
        assert_eq!(parse_link_reply(201, &made), Ok(Some(uuid(RADIO))));
        assert_eq!(parse_link_reply(200, &made), Ok(Some(uuid(RADIO))));
        // Success that does not say which link it made.
        assert_eq!(parse_link_reply(200, "<llsd><map></map></llsd>"), Ok(None));
        // A 2xx without a map is no success (llaisapi.cpp:877).
        assert_eq!(parse_link_reply(200, ""), Err("HTTP 200".to_string()));
        assert_eq!(
            parse_link_reply(200, "<llsd><string>ok</string></llsd>"),
            Err("HTTP 200".to_string())
        );
        let refused = "<llsd><map><key>error_code</key><string>INVENTORY_LIMIT</string>\
                       <key>error_description</key><string>Too many items in folder </string></map></llsd>";
        assert_eq!(
            parse_link_reply(403, refused),
            Err("Too many items in folder (HTTP 403)".to_string())
        );
        assert_eq!(parse_link_reply(500, "<html>oops</html>"), Err("HTTP 500".to_string()));
    }

    #[test]
    fn an_item_is_removed_through_the_inventory_api() {
        assert_eq!(
            remove_url("https://sim/cap/abc/", uuid(RADIO)),
            format!("https://sim/cap/abc/item/{RADIO}")
        );
        assert_eq!(parse_remove_reply(200, "<llsd><map></map></llsd>"), Ok(()));
        assert_eq!(parse_remove_reply(204, ""), Ok(()));
        // Gone already is gone.
        assert_eq!(parse_remove_reply(410, ""), Ok(()));
        assert_eq!(parse_remove_reply(404, ""), Ok(()));
        let refused = "<llsd><map><key>message</key><string>Not yours</string></map></llsd>";
        assert_eq!(parse_remove_reply(403, refused), Err("Not yours (HTTP 403)".to_string()));
        assert_eq!(parse_remove_reply(503, ""), Err("HTTP 503".to_string()));
    }

    #[test]
    fn the_appearance_request_carries_the_folder_version() {
        assert!(appearance_request(42).ends_with("<llsd><map><key>cof_version</key><integer>42</integer></map></llsd>"));
        let accepted = "<llsd><map><key>success</key><boolean>1</boolean><key>agent_id</key><uuid>aaaaaaaa-0000-0000-0000-000000000001</uuid></map></llsd>";
        assert_eq!(parse_appearance_reply(200, accepted), Ok(()));
        // The refusals the viewer knows (llappearancemgr.cpp:4012-4029).
        let stale = "<llsd><map><key>success</key><boolean>0</boolean><key>error</key><string>Cof Version Mismatch</string>\
                     <key>expected</key><integer>43</integer></map></llsd>";
        let reason = parse_appearance_reply(200, stale).unwrap_err();
        assert!(reason.contains("Cof Version Mismatch") && reason.contains("version 43"), "{reason}");
        let reason = parse_appearance_reply(400, stale).unwrap_err();
        assert!(reason.contains("HTTP 400") && reason.contains("version 43"), "{reason}");
        assert_eq!(parse_appearance_reply(503, "busy"), Err("HTTP 503".to_string()));
        assert_eq!(parse_appearance_reply(200, "<llsd><map></map></llsd>"), Err("HTTP 200".to_string()));
        // A success flag on an error status is not believed.
        assert!(parse_appearance_reply(500, accepted).is_err());
    }

    // ------------------------------------------------------------ listing

    fn entry(id: u128, asset_type: i32, flags: u32, path: &str) -> Entry {
        Entry {
            id: Uuid::from_u128(id),
            asset_type,
            flags,
            path: path.to_string(),
        }
    }

    fn lines(entries: &[Entry], attached: &[(u128, u8)], attached_only: bool) -> Vec<String> {
        let attached: HashMap<Uuid, u8> = attached.iter().map(|(id, point)| (Uuid::from_u128(*id), *point)).collect();
        let (text, count) = listing(entries, &attached, attached_only);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        assert_eq!(lines.len(), count);
        lines
    }

    fn line(id: u128, fields: &str) -> String {
        format!("{}\t{fields}", Uuid::from_u128(id))
    }

    #[test]
    fn a_listing_line_has_five_fields() {
        let entries = [
            entry(1, ASSET_OBJECT, 2, "Objects/Radio"),
            entry(2, ASSET_OBJECT, 2, "Objects/Hat"),
            entry(3, ASSET_OBJECT, 0, "Objects/Crate"),
            entry(4, ASSET_CLOTHING, 4, "Clothing/Blue shirt"),
            entry(5, ASSET_BODY_PART, 1, "Body Parts/Tanned skin"),
            entry(6, ASSET_FOLDER, 0, "Objects/"),
            entry(7, ASSET_ANIMATION, 0, "Animations/Dance"),
            entry(8, ASSET_OBJECT, 200, "Objects/Odd"),
            entry(9, ASSET_CLOTHING, 99, "Clothing/Odd"),
        ];
        let listed = lines(&entries, &[(1, 5)], false);
        let has = |expected: String| assert!(listed.contains(&expected), "{expected:?} not in {listed:#?}");
        // Attached: where it is now, whatever the inventory recorded.
        has(line(1, "object\tLeft Hand\tattached\tObjects/Radio"));
        // Not attached: where it was last, or nothing recorded.
        has(line(2, "object\tSkull\t-\tObjects/Hat"));
        has(line(3, "object\t-\t-\tObjects/Crate"));
        has(line(4, "clothing\tshirt\t-\tClothing/Blue shirt"));
        has(line(5, "bodypart\tskin\t-\tBody Parts/Tanned skin"));
        has(line(6, "folder\t-\t-\tObjects/"));
        has(line(7, "animation\t-\t-\tAnimations/Dance"));
        // A point or a kind without a name is its number.
        has(line(8, "object\t200\t-\tObjects/Odd"));
        has(line(9, "clothing\t99\t-\tClothing/Odd"));
        assert_eq!(listed.len(), 9);
        assert!(listed.iter().all(|line| line.split('\t').count() == 5));
    }

    #[test]
    fn a_listing_is_in_the_order_of_its_paths_and_has_no_header() {
        let entries = [
            entry(1, ASSET_OBJECT, 0, "Objects/Radio"),
            entry(2, ASSET_OBJECT, 0, "Objects/Bot gear/Antenna"),
            entry(3, ASSET_FOLDER, 0, "Objects/"),
            entry(4, ASSET_FOLDER, 0, "Objects/Bot gear/"),
        ];
        let paths: Vec<String> = lines(&entries, &[], false)
            .iter()
            .map(|line| line.rsplit('\t').next().unwrap().to_string())
            .collect();
        assert_eq!(
            paths,
            ["Objects/", "Objects/Bot gear/", "Objects/Bot gear/Antenna", "Objects/Radio"]
        );
        // Two entries with one path both stay.
        let twins = [entry(1, ASSET_OBJECT, 0, "Objects/Twin"), entry(2, ASSET_OBJECT, 0, "Objects/Twin")];
        assert_eq!(lines(&twins, &[], false).len(), 2);
        // Nothing to list is no text at all.
        assert_eq!(listing(&[], &HashMap::new(), false), (String::new(), 0));
    }

    #[test]
    fn only_attached_items_are_listed_when_asked() {
        let entries = [
            entry(1, ASSET_OBJECT, 2, "Objects/Radio"),
            entry(2, ASSET_OBJECT, 2, "Objects/Hat"),
            entry(3, ASSET_FOLDER, 0, "Objects/"),
            // A link to the radio: its own id, and never attached, even
            // if that id were in the table.
            entry(4, ASSET_LINK, 0, "Outfits/Work/Radio"),
            entry(5, ASSET_CLOTHING, 4, "Clothing/Blue shirt"),
        ];
        assert_eq!(
            lines(&entries, &[(1, 5), (4, 6), (5, 1)], true),
            [line(1, "object\tLeft Hand\tattached\tObjects/Radio")]
        );
        assert_eq!(lines(&entries, &[], true), Vec::<String>::new());
        assert_eq!(listing(&entries, &HashMap::new(), true), (String::new(), 0));
        let all = lines(&entries, &[(1, 5), (4, 6)], false);
        assert!(all.contains(&line(4, "link\t-\t-\tOutfits/Work/Radio")), "{all:#?}");
    }

    #[test]
    fn names_in_a_listing_are_escaped() {
        // The worker builds paths with `escape_name`: a slash, a tab and a
        // backslash inside names stay inside one field.
        let path = format!("{}/{}", escape_name("Objects"), escape_name("Radio/TV\tloud\\quiet\n"));
        assert_eq!(path, "Objects/Radio\\/TV\\tloud\\\\quiet\\n");
        let listed = lines(&[entry(1, ASSET_OBJECT, 0, &path)], &[], false);
        assert_eq!(listed, [line(1, &format!("object\t-\t-\t{path}"))]);
        // And the printed path names the same entry again.
        assert_eq!(
            parse_path(&path),
            Ok(vec!["Objects".to_string(), "Radio/TV\tloud\\quiet\n".to_string()])
        );
    }
}

#[cfg(test)]
mod grid_tests {
    //! The jobs against a small grid of its own behind a local HTTP
    //! listener, as `names.rs` tests its worker.
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const OWNER: Uuid = Uuid::from_u128(0xA1);
    const ROOT: Uuid = Uuid::from_u128(0x100);
    const OUTFIT: Uuid = Uuid::from_u128(0x146);

    /// The grid's side: folders, items, and what it was asked.
    #[derive(Default)]
    struct Grid {
        /// (id, parent, name, type).
        folders: Vec<(Uuid, Uuid, String, i32)>,
        items: Vec<Item>,
        /// The Current Outfit folder's version.
        version: i64,
        next_id: u128,
        /// One line per request.
        asked: Vec<String>,
        /// Adds the items links point to, as OpenSim does, and keeps
        /// unknown folders out of sight.
        opensim: bool,
        refuse_appearance: bool,
        refuse_links: bool,
        refuse_removals: bool,
        /// Link replies do not say which link was made.
        quiet_links: bool,
        fail_reads: bool,
    }

    fn id_of(name: &str) -> Uuid {
        // Stable ids from names keep the tests readable.
        let mut value: u128 = 0x1000;
        for byte in name.bytes() {
            value = value.wrapping_mul(131).wrapping_add(u128::from(byte));
        }
        Uuid::from_u128(value)
    }

    impl Grid {
        fn folder(&mut self, parent: Uuid, name: &str) -> Uuid {
            let id = id_of(&format!("folder {parent} {name}"));
            self.folders.push((id, parent, name.to_string(), -1));
            id
        }

        fn item(&mut self, parent: Uuid, name: &str, asset_type: i32, flags: u32) -> Uuid {
            self.next_id += 1;
            let id = Uuid::from_u128(0x5000 + self.next_id);
            let inv_type = match asset_type {
                ASSET_CLOTHING | ASSET_BODY_PART => INV_WEARABLE,
                ASSET_OBJECT => INV_OBJECT,
                _ => asset_type,
            };
            self.items.push(Item {
                id,
                parent,
                name: name.to_string(),
                description: format!("{name}, described"),
                asset_type,
                inv_type,
                flags,
                asset_id: Uuid::from_u128(0xA55E7000 + self.next_id),
                owner: OWNER,
                group_mask: 0,
                everyone_mask: 0x8000,
                next_owner_mask: 0x0008_2000,
            });
            id
        }

        fn link(&mut self, parent: Uuid, target: Uuid, asset_type: i32) -> Uuid {
            let name = self
                .items
                .iter()
                .find(|item| item.id == target)
                .map_or("Gear".to_string(), |item| item.name.clone());
            let id = self.item(parent, &name, asset_type, 0);
            self.items.last_mut().unwrap().asset_id = target;
            id
        }

        fn named(&self, name: &str) -> Item {
            let found = self.items.iter().find(|item| item.name == name && item.asset_type != ASSET_LINK);
            found.unwrap_or_else(|| panic!("no item {name}")).clone()
        }

        /// The names of what the Current Outfit folder links to.
        fn worn(&self) -> Vec<String> {
            let mut names: Vec<String> = self
                .items
                .iter()
                .filter(|item| item.parent == OUTFIT)
                .map(|item| item.name.clone())
                .collect();
            names.sort();
            names
        }

        /// The inventory the tests look at:
        ///
        /// ```text
        /// Objects/      Radio (left hand), Radio/TV, Twin, Twin, Hat (skull)
        ///   Bot gear/   Antenna
        ///   Radio/      Knob
        /// Clothing/     Blue shirt, White shirt, Jeans
        /// Body Parts/   Tanned skin, Pale skin, Round shape
        /// Animations/   Dance
        /// Empty/
        /// Outfits/Work/ a link to Radio, a link to the folder Bot gear
        /// Current Outfit/  links to White shirt, Pale skin, Round shape
        /// ```
        fn stocked() -> Grid {
            let mut grid = Grid {
                version: 7,
                ..Grid::default()
            };
            let objects = grid.folder(ROOT, "Objects");
            let radio = grid.item(objects, "Radio", ASSET_OBJECT, 5);
            grid.item(objects, "Radio/TV", ASSET_OBJECT, 0);
            grid.item(objects, "Twin", ASSET_OBJECT, 0);
            grid.item(objects, "Twin", ASSET_OBJECT, 0);
            grid.item(objects, "Hat", ASSET_OBJECT, 2);
            let gear = grid.folder(objects, "Bot gear");
            grid.item(gear, "Antenna", ASSET_OBJECT, 0);
            let radio_folder = grid.folder(objects, "Radio");
            grid.item(radio_folder, "Knob", ASSET_OBJECT, 0);
            let clothing = grid.folder(ROOT, "Clothing");
            grid.item(clothing, "Blue shirt", ASSET_CLOTHING, 4);
            let white = grid.item(clothing, "White shirt", ASSET_CLOTHING, 4);
            grid.item(clothing, "Jeans", ASSET_CLOTHING, 5);
            let parts = grid.folder(ROOT, "Body Parts");
            grid.item(parts, "Tanned skin", ASSET_BODY_PART, 1);
            let pale = grid.item(parts, "Pale skin", ASSET_BODY_PART, 1);
            let shape = grid.item(parts, "Round shape", ASSET_BODY_PART, 0);
            let animations = grid.folder(ROOT, "Animations");
            grid.item(animations, "Dance", ASSET_ANIMATION, 0);
            grid.folder(ROOT, "Empty");
            let outfits = grid.folder(ROOT, "Outfits");
            let work = grid.folder(outfits, "Work");
            grid.link(work, radio, ASSET_LINK);
            grid.link(work, gear, ASSET_LINK_FOLDER);
            grid.folders
                .push((OUTFIT, ROOT, "Current Outfit".to_string(), FOLDER_CURRENT_OUTFIT));
            for worn in [white, pale, shape] {
                grid.link(OUTFIT, worn, ASSET_LINK);
            }
            grid
        }

        fn item_value(item: &Item) -> Value {
            let int = |bits: u32| Value::Int(i64::from(bits as i32));
            request_map(&[
                ("item_id", Value::Uuid(item.id)),
                ("parent_id", Value::Uuid(item.parent)),
                ("asset_id", Value::Uuid(item.asset_id)),
                ("name", Value::Str(item.name.clone())),
                ("desc", Value::Str(item.description.clone())),
                ("type", Value::Int(i64::from(item.asset_type))),
                ("inv_type", Value::Int(i64::from(item.inv_type))),
                ("flags", int(item.flags)),
                (
                    "permissions",
                    request_map(&[
                        ("owner_id", Value::Uuid(item.owner)),
                        ("group_mask", int(item.group_mask)),
                        ("everyone_mask", int(item.everyone_mask)),
                        ("next_owner_mask", int(item.next_owner_mask)),
                    ]),
                ),
            ])
        }

        fn descendents(&mut self, request: &Value) -> (u16, Value) {
            let (mut described, mut bad, mut names) = (Vec::new(), Vec::new(), Vec::new());
            for wanted in request.get("folders").and_then(Value::as_array).unwrap() {
                let id = wanted.get("folder_id").and_then(Value::as_uuid).unwrap();
                assert_eq!(wanted.get("owner_id").and_then(Value::as_uuid), Some(OWNER));
                let flag = |key| wanted.get(key) == Some(&Value::Bool(true));
                let known = self.folders.iter().find(|folder| folder.0 == id).map(|folder| folder.2.clone());
                let name = if id == ROOT { Some("My Inventory".to_string()) } else { known };
                let Some(name) = name else {
                    names.push("?".to_string());
                    bad.push(request_map(&[
                        ("folder_id", Value::Uuid(id)),
                        ("error", Value::Str("Unknown".into())),
                    ]));
                    continue;
                };
                let kinds = match (flag("fetch_folders"), flag("fetch_items")) {
                    (true, true) => "",
                    (true, false) => " folders",
                    (false, true) => " items",
                    (false, false) => " nothing",
                };
                names.push(format!("{name}{kinds}"));
                let categories = self
                    .folders
                    .iter()
                    .filter(|folder| folder.1 == id && flag("fetch_folders"))
                    .map(|folder| {
                        request_map(&[
                            ("category_id", Value::Uuid(folder.0)),
                            ("parent_id", Value::Uuid(folder.1)),
                            ("name", Value::Str(folder.2.clone())),
                            ("type_default", Value::Int(i64::from(folder.3))),
                        ])
                    });
                let own: Vec<&Item> = self.items.iter().filter(|item| item.parent == id && flag("fetch_items")).collect();
                let mut items = Vec::new();
                if self.opensim {
                    // The link targets first, with their own parents.
                    let targets = own.iter().filter(|item| item.asset_type == ASSET_LINK);
                    let targets = targets.filter_map(|link| self.items.iter().find(|item| item.id == link.asset_id));
                    items.extend(targets.map(Grid::item_value));
                }
                items.extend(own.into_iter().map(Grid::item_value));
                described.push(request_map(&[
                    ("folder_id", Value::Uuid(id)),
                    ("owner_id", Value::Uuid(OWNER)),
                    ("version", Value::Int(if id == OUTFIT { self.version } else { 1 })),
                    ("categories", Value::Array(categories.collect())),
                    ("items", Value::Array(items)),
                ]));
            }
            self.asked.push(format!("folders: {}", names.join(", ")));
            let mut reply = vec![("folders", Value::Array(described))];
            if !self.opensim {
                reply.push(("bad_folders", Value::Array(bad)));
            }
            (200, request_map(&reply))
        }

        fn fetch_items(&mut self, request: &Value) -> (u16, Value) {
            assert_eq!(request.get("agent_id").and_then(Value::as_uuid), Some(OWNER));
            let wanted = request.get("items").and_then(Value::as_array).unwrap();
            let ids: Vec<Uuid> = wanted
                .iter()
                .map(|item| item.get("item_id").and_then(Value::as_uuid).unwrap())
                .collect();
            let found: Vec<&Item> = self.items.iter().filter(|item| ids.contains(&item.id)).collect();
            let names: Vec<&str> = found.iter().map(|item| item.name.as_str()).collect();
            let unknown = ids.len() - found.len();
            self.asked.push(format!(
                "items: {}{}",
                names.join(", "),
                if unknown > 0 { " and unknown ones" } else { "" }
            ));
            let items = found.into_iter().map(Grid::item_value).collect();
            (
                200,
                request_map(&[("agent_id", Value::Uuid(OWNER)), ("items", Value::Array(items))]),
            )
        }

        fn refused(reason: &str) -> (u16, Value) {
            (403, request_map(&[("error_description", Value::Str(reason.to_string()))]))
        }

        fn make_link(&mut self, path: &str, request: &Value) -> (u16, Value) {
            let folder = path.trim_start_matches("/api/category/").split('?').next().unwrap();
            assert!(
                path.contains("?tid=") && Uuid::parse_str(path.split("?tid=").nth(1).unwrap()).is_ok(),
                "{path}"
            );
            assert_eq!(Uuid::parse_str(folder), Ok(OUTFIT));
            let link = &request.get("links").and_then(Value::as_array).unwrap()[0];
            assert_eq!(link.get("type"), Some(&Value::Int(24)));
            let name = link.str_field("name").unwrap().to_string();
            self.asked.push(format!("link: {name}"));
            if self.refuse_links {
                return Grid::refused("Outfit folder is full");
            }
            self.next_id += 1;
            let id = Uuid::from_u128(0x7000 + self.next_id);
            self.items.push(Item {
                id,
                parent: OUTFIT,
                name,
                description: link.str_field("desc").unwrap().to_string(),
                asset_type: ASSET_LINK,
                inv_type: link.get("inv_type").and_then(Value::as_i64).unwrap() as i32,
                asset_id: link.get("linked_id").and_then(Value::as_uuid).unwrap(),
                owner: OWNER,
                ..Item::default()
            });
            self.version += 1;
            let created = if self.quiet_links { vec![] } else { vec![Value::Uuid(id)] };
            (201, request_map(&[("_created_items", Value::Array(created))]))
        }

        fn remove(&mut self, path: &str) -> (u16, Value) {
            let id = Uuid::parse_str(path.trim_start_matches("/api/item/")).unwrap();
            let name = self.items.iter().find(|item| item.id == id).map(|item| item.name.clone());
            self.asked.push(format!("remove: {}", name.as_deref().unwrap_or("?")));
            if self.refuse_removals {
                return Grid::refused("Not now");
            }
            if name.is_none() {
                return (404, request_map(&[]));
            }
            self.items.retain(|item| item.id != id);
            self.version += 1;
            (200, request_map(&[]))
        }

        fn appearance(&mut self, request: &Value) -> (u16, Value) {
            let version = request.get("cof_version").and_then(Value::as_i64).unwrap();
            self.asked.push(format!("appearance: version {version}"));
            if self.refuse_appearance || version != self.version {
                let refusal = [
                    ("success", Value::Bool(false)),
                    ("error", Value::Str("Outfit not acceptable".into())),
                    ("expected", Value::Int(self.version)),
                ];
                return (400, request_map(&refusal));
            }
            (200, request_map(&[("success", Value::Bool(true))]))
        }

        fn answer(&mut self, method: &str, path: &str, body: &str) -> (u16, String) {
            if self.fail_reads {
                self.asked.push(format!("{method} {path}"));
                return (500, String::new());
            }
            let request = xml::parse_llsd(body).unwrap_or(Value::Undef);
            let (status, reply) = match (method, path) {
                ("POST", "/descendents") => self.descendents(&request),
                ("POST", "/items") => self.fetch_items(&request),
                ("POST", "/appearance") => self.appearance(&request),
                ("POST", path) if path.starts_with("/api/category/") => self.make_link(path, &request),
                ("DELETE", path) if path.starts_with("/api/item/") => self.remove(path),
                other => panic!("unexpected request {other:?}"),
            };
            (status, xml::write_llsd(&reply))
        }
    }

    /// Reads one HTTP request: method, path and body.
    fn read_request(stream: &std::net::TcpStream) -> Option<(String, String, String)> {
        let mut reader = BufReader::new(stream.try_clone().ok()?);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let mut words = line.split_whitespace();
        let (method, path) = (words.next()?.to_string(), words.next()?.to_string());
        let mut length = 0;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? <= 2 {
                break;
            }
            if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().ok()?;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        Some((method, path, String::from_utf8(body).ok()?))
    }

    /// Serves `grid` on a local port until the test process ends. Returns
    /// the URLs of its four services.
    fn serve(grid: &Arc<Mutex<Grid>>) -> InventoryUrls {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let grid = grid.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let Some((method, path, body)) = read_request(&stream) else {
                    continue;
                };
                let (status, reply) = grid.lock().unwrap().answer(&method, &path, &body);
                let head = format!(
                    "HTTP/1.1 {status} Whatever\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.len()
                );
                let _ = stream.write_all(format!("{head}{reply}").as_bytes());
            }
        });
        InventoryUrls {
            descendents: Some(format!("{base}/descendents")),
            items: Some(format!("{base}/items")),
            api: Some(format!("{base}/api")),
            appearance: Some(format!("{base}/appearance")),
        }
    }

    struct Rig {
        grid: Arc<Mutex<Grid>>,
        urls: InventoryUrls,
        agent: ureq::Agent,
    }

    impl Rig {
        fn new() -> Rig {
            Rig::with(Grid::stocked())
        }

        fn with(grid: Grid) -> Rig {
            let grid = Arc::new(Mutex::new(grid));
            Rig {
                urls: serve(&grid),
                grid,
                agent: crate::caps::http_agent(LIMIT),
            }
        }

        fn job(&self, what: What) -> Job {
            Job {
                id: 1,
                generation: 1,
                urls: self.urls.clone(),
                owner: OWNER,
                root: ROOT,
                deadline: Instant::now() + Duration::from_secs(20),
                what,
            }
        }

        fn run(&self, what: What) -> Result<Outcome, Failed> {
            carry_out(&self.agent, &self.job(what))
        }

        /// The grid, for a look or a change. Never held across a job:
        /// the listener needs it to answer.
        fn grid(&self) -> std::sync::MutexGuard<'_, Grid> {
            self.grid.lock().unwrap()
        }

        fn named(&self, name: &str) -> Item {
            self.grid().named(name)
        }

        fn worn(&self) -> Vec<String> {
            self.grid().worn()
        }

        /// What the grid was asked since the last call.
        fn asked(&self) -> Vec<String> {
            std::mem::take(&mut self.grid().asked)
        }

        fn find(&self, item: &str) -> Result<Item, String> {
            match self.run(What::Find(parse_ref(item).unwrap())) {
                Ok(Outcome::Found(item)) => Ok(item),
                Err(Failed::Reason(reason)) => Err(reason),
                other => panic!("unexpected {other:?}"),
            }
        }

        /// The paths a listing has, in the worker's order.
        fn list(&self, folder: &str, recursive: bool) -> Result<Vec<String>, String> {
            let folder = parse_ref(folder).unwrap();
            match self.run(What::List { folder, recursive }) {
                Ok(Outcome::Listed(entries)) => Ok(entries.into_iter().map(|entry| entry.path).collect()),
                Err(Failed::Reason(reason)) => Err(reason),
                other => panic!("unexpected {other:?}"),
            }
        }

        fn sorted(&self, folder: &str, recursive: bool) -> Vec<String> {
            let mut paths = self.list(folder, recursive).unwrap();
            paths.sort();
            paths
        }

        fn outfit(&self, what: What) -> Result<Vec<String>, String> {
            match self.run(what) {
                Ok(Outcome::Outfit(items)) => Ok(items.into_iter().map(|item| item.name).collect()),
                Err(Failed::Reason(reason)) => Err(reason),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    // -------------------------------------------------------- finding

    #[test]
    fn a_path_is_resolved_a_folder_per_name() {
        let rig = Rig::new();
        let antenna = rig.find("Objects/Bot gear/Antenna").unwrap();
        assert_eq!((antenna.name.as_str(), antenna.asset_type), ("Antenna", ASSET_OBJECT));
        let expected = rig.named("Antenna");
        assert_eq!(antenna, expected);
        // Folders only on the way, items only at the end.
        assert_eq!(
            rig.asked(),
            [
                "folders: My Inventory folders",
                "folders: Objects folders",
                "folders: Bot gear items"
            ]
        );
        // An item at the top takes one request; the record is complete.
        let radio = rig.find("/Objects/Radio").unwrap();
        assert_eq!((radio.subtype(), radio.inv_type, radio.owner), (5, INV_OBJECT, OWNER));
        assert_eq!((radio.everyone_mask, radio.next_owner_mask), (0x8000, 0x0008_2000));
        assert_eq!(radio.description, "Radio, described");
        assert_eq!(rig.find("Objects/Radio\\/TV").unwrap().name, "Radio/TV");
    }

    #[test]
    fn what_a_path_does_not_find_is_named() {
        let rig = Rig::new();
        // Names match exactly, case included.
        let reason = rig.find("objects/Radio").unwrap_err();
        assert!(
            reason.contains("no folder named \"objects\"") && reason.contains("My Inventory"),
            "{reason}"
        );
        let reason = rig.find("Objects/Bot gear/Dish").unwrap_err();
        assert!(
            reason.contains("no item named \"Dish\"") && reason.contains("\"Objects/Bot gear\""),
            "{reason}"
        );
        let reason = rig.find("Objects/Nowhere/Dish").unwrap_err();
        assert!(
            reason.contains("no folder named \"Nowhere\"") && reason.contains("\"Objects\""),
            "{reason}"
        );
        // A folder is no item, and My Inventory is none either.
        assert!(rig.find("Objects/Bot gear").unwrap_err().contains("no item named \"Bot gear\""));
        assert!(rig.find("Empty").unwrap_err().contains("no item named \"Empty\""));
        assert!(rig.find("/").unwrap_err().contains("My Inventory itself"));
        // Nothing is asked about a path once a part of it is missing.
        rig.asked();
        rig.find("Nowhere/a/b/c").unwrap_err();
        assert_eq!(rig.asked(), ["folders: My Inventory folders"]);
    }

    #[test]
    fn two_entries_with_one_name_are_refused_with_their_ids() {
        let rig = Rig::new();
        let twins: Vec<String> = rig
            .grid()
            .items
            .iter()
            .filter(|item| item.name == "Twin")
            .map(|item| item.id.to_string())
            .collect();
        let reason = rig.find("Objects/Twin").unwrap_err();
        assert!(reason.contains("2 items named \"Twin\""), "{reason}");
        assert!(twins.iter().all(|id| reason.contains(id)), "{reason}");
        // Either is reachable by its id.
        assert_eq!(rig.find(&twins[1]).unwrap().id.to_string(), twins[1]);
        // Two folders with one name on the way.
        let mut grid = Grid::stocked();
        let objects = grid.folders[0].0;
        let other = grid.folder(objects, "Bot gear");
        grid.folders.last_mut().unwrap().0 = id_of("another");
        let _ = other;
        let rig = Rig::with(grid);
        let reason = rig.find("Objects/Bot gear/Antenna").unwrap_err();
        assert!(
            reason.contains("2 folders named \"Bot gear\"") && reason.contains(&id_of("another").to_string()),
            "{reason}"
        );
    }

    #[test]
    fn the_last_name_is_an_item_for_items_and_a_folder_for_folders() {
        let rig = Rig::new();
        // `Objects` holds an item and a folder named `Radio`.
        assert_eq!(rig.find("Objects/Radio").unwrap().asset_type, ASSET_OBJECT);
        assert_eq!(rig.list("Objects/Radio", true), Ok(vec!["Objects/Radio/Knob".to_string()]));
    }

    #[test]
    fn a_link_stands_for_the_item_it_points_to() {
        let rig = Rig::new();
        let radio = rig.named("Radio");
        assert_eq!(rig.find("Outfits/Work/Radio"), Ok(radio.clone()));
        let link = rig
            .grid()
            .items
            .iter()
            .find(|item| item.asset_type == ASSET_LINK && item.asset_id == radio.id)
            .map(|item| item.id);
        let link = link.unwrap();
        rig.asked();
        assert_eq!(rig.find(&link.to_string()), Ok(radio.clone()));
        assert_eq!(rig.asked(), ["items: Radio", "items: Radio"]);
        // A link to a folder is no item; a link to nothing is said to be one.
        assert!(rig.find("Outfits/Work/Gear").unwrap_err().contains("link to a folder"));
        rig.grid().items.retain(|item| item.id != radio.id);
        assert!(rig.find("Outfits/Work/Radio").unwrap_err().contains("points to nothing"));
    }

    #[test]
    fn an_item_is_found_by_its_id() {
        let rig = Rig::new();
        let hat = rig.named("Hat");
        assert_eq!(rig.find(&hat.id.to_string()), Ok(hat.clone()));
        assert_eq!(rig.asked(), ["items: Hat"]);
        let stranger = Uuid::from_u128(0xDEAD);
        let reason = rig.find(&stranger.to_string()).unwrap_err();
        assert!(
            reason.contains("there is no item") && reason.contains(&stranger.to_string()),
            "{reason}"
        );
        // Somebody else's item is not in the avatar's inventory.
        rig.grid().items.iter_mut().find(|item| item.id == hat.id).unwrap().owner = Uuid::from_u128(0xB0B);
        assert!(rig.find(&hat.id.to_string()).unwrap_err().contains("there is no item"));
        // Several at once, the unknown left out.
        let dance = rig.named("Dance");
        let found = rig.run(What::Items(vec![dance.id, stranger]));
        assert_eq!(found, Ok(Outcome::Items(vec![dance])));
    }

    // -------------------------------------------------------- listing

    #[test]
    fn a_listing_walks_everything_below_the_folder() {
        let rig = Rig::new();
        assert_eq!(
            rig.sorted("Objects", true),
            [
                "Objects/Bot gear/",
                "Objects/Bot gear/Antenna",
                "Objects/Hat",
                "Objects/Radio",
                "Objects/Radio/",
                "Objects/Radio/Knob",
                "Objects/Radio\\/TV",
                "Objects/Twin",
                "Objects/Twin",
            ]
        );
        // The folder itself is not listed; its sub-folders go in one request.
        assert_eq!(
            rig.asked(),
            ["folders: My Inventory folders", "folders: Objects", "folders: Bot gear, Radio"]
        );
        // Direct children only: one request for the contents.
        let children = rig.sorted("Objects", false);
        assert_eq!(children.len(), 7);
        assert!(children.contains(&"Objects/Bot gear/".to_string()) && !children.contains(&"Objects/Bot gear/Antenna".to_string()));
        assert_eq!(rig.asked(), ["folders: My Inventory folders", "folders: Objects"]);
        assert_eq!(rig.list("Empty", true), Ok(vec![]));
        // The entries carry what the listing's other fields need.
        let Ok(Outcome::Listed(entries)) = rig.run(What::List {
            folder: parse_ref("Clothing").unwrap(),
            recursive: true,
        }) else {
            panic!("not listed")
        };
        let shirt = entries.iter().find(|entry| entry.path == "Clothing/Blue shirt").unwrap();
        let blue = rig.named("Blue shirt").id;
        assert_eq!((shirt.id, shirt.asset_type, shirt.flags), (blue, ASSET_CLOTHING, 4));
    }

    #[test]
    fn everything_in_my_inventory_is_listed_from_the_top() {
        let rig = Rig::new();
        let all = rig.sorted("", true);
        let folders = rig.grid().folders.len();
        let items = rig.grid().items.len();
        assert_eq!(all.len(), folders + items);
        for expected in [
            "Objects/",
            "Objects/Radio",
            "Current Outfit/",
            "Current Outfit/White shirt",
            "Outfits/Work/Gear",
            "Empty/",
        ] {
            assert!(all.contains(&expected.to_string()), "{expected} not in {all:#?}");
        }
        // No request to find the top: the login said where it is.
        assert_eq!(rig.asked()[0], "folders: My Inventory");
    }

    #[test]
    fn a_big_folder_is_walked_in_batches() {
        let mut grid = Grid::stocked();
        let many = grid.folder(ROOT, "Many");
        for index in 0..25 {
            let sub = grid.folder(many, &format!("Box {index:02}"));
            grid.item(sub, &format!("Thing {index:02}"), ASSET_OBJECT, 0);
        }
        let rig = Rig::with(grid);
        let listed = rig.sorted("Many", true);
        assert_eq!(listed.len(), 50);
        assert_eq!((listed[0].as_str(), listed[49].as_str()), ("Many/Box 00/", "Many/Box 24/Thing 24"));
        // The top folder, then its 25 sub-folders ten to a request.
        let asked = rig.asked();
        assert_eq!(asked.len(), 2 + 3, "{asked:#?}");
        assert_eq!(asked[2].matches("Box").count(), 10);
        assert_eq!(asked[4].matches("Box").count(), 5);
    }

    #[test]
    fn a_folder_named_by_id_is_listed_with_its_whole_path() {
        let rig = Rig::new();
        let gear = rig
            .grid()
            .folders
            .iter()
            .find(|folder| folder.2 == "Bot gear")
            .map(|folder| folder.0);
        let gear = gear.unwrap();
        assert_eq!(rig.list(&gear.to_string(), true), Ok(vec!["Objects/Bot gear/Antenna".to_string()]));
        // Looked for from the top, folders only, then listed.
        let asked = rig.asked();
        assert_eq!(asked[0], "folders: My Inventory folders");
        assert!(asked[..asked.len() - 1].iter().all(|line| line.ends_with(" folders")), "{asked:#?}");
        assert_eq!(asked.last().unwrap(), "folders: Bot gear");
        // The top by its id is the top.
        assert_eq!(rig.sorted(&ROOT.to_string(), false).len(), 7);
        let stranger = Uuid::from_u128(0xDEAD);
        let reason = rig.list(&stranger.to_string(), true).unwrap_err();
        assert!(
            reason.contains("there is no folder") && reason.contains(&stranger.to_string()),
            "{reason}"
        );
    }

    #[test]
    fn a_folder_that_does_not_exist_is_refused() {
        let rig = Rig::new();
        let reason = rig.list("Nowhere", true).unwrap_err();
        assert!(
            reason.contains("no folder named \"Nowhere\"") && reason.contains("My Inventory"),
            "{reason}"
        );
        let reason = rig.list("Objects/Bot gear/Deeper", false).unwrap_err();
        assert!(reason.contains("\"Deeper\"") && reason.contains("\"Objects/Bot gear\""), "{reason}");
        // An item is no folder.
        assert!(rig.list("Objects/Hat", true).unwrap_err().contains("no folder named \"Hat\""));
        // A top folder the grid does not know, whichever way it says so.
        for opensim in [false, true] {
            let rig = Rig::new();
            rig.grid().opensim = opensim;
            let mut job = rig.job(What::List {
                folder: Ref::Path(vec![]),
                recursive: true,
            });
            job.root = Uuid::from_u128(0xDEAD);
            let Err(Failed::Reason(reason)) = carry_out(&rig.agent, &job) else {
                panic!("listed an unknown folder")
            };
            assert!(reason.contains("knows no inventory folder"), "{reason}");
        }
    }

    #[test]
    fn the_items_links_point_to_are_not_taken_for_the_folders_own() {
        // OpenSim adds them to the folder's items.
        let rig = Rig::new();
        rig.grid().opensim = true;
        assert_eq!(rig.sorted("Outfits/Work", true), ["Outfits/Work/Gear", "Outfits/Work/Radio"]);
        assert_eq!(
            rig.sorted("Current Outfit", true),
            [
                "Current Outfit/Pale skin",
                "Current Outfit/Round shape",
                "Current Outfit/White shirt"
            ]
        );
        let radio = rig.named("Radio");
        assert_eq!(rig.find("Outfits/Work/Radio"), Ok(radio));
    }

    // ------------------------------------------------- failures and time

    #[test]
    fn a_grid_that_answers_with_an_error_fails_the_job() {
        let rig = Rig::new();
        rig.grid().fail_reads = true;
        assert!(rig.find("Objects/Radio").unwrap_err().contains("HTTP 500"));
        assert!(rig.list("", true).unwrap_err().contains("HTTP 500"));
        // Without the services there is nothing to ask.
        let mut job = rig.job(What::Find(parse_ref("Objects/Radio").unwrap()));
        job.urls = InventoryUrls::default();
        assert_eq!(carry_out(&rig.agent, &job), Err(Failed::Reason(NO_SERVICE.to_string())));
    }

    #[test]
    fn a_grid_that_never_answers_times_the_job_out() {
        // A listener that takes the connection and says nothing.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/descendents", listener.local_addr().unwrap());
        let silent = thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(1).collect();
            thread::sleep(Duration::from_secs(3));
            drop(held);
        });
        let rig = Rig::new();
        let mut job = rig.job(What::List {
            folder: Ref::Path(vec![]),
            recursive: true,
        });
        job.urls.descendents = Some(url);
        job.deadline = Instant::now() + Duration::from_millis(700);
        let started = Instant::now();
        assert_eq!(carry_out(&rig.agent, &job), Err(Failed::TimedOut));
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(600) && took < Duration::from_secs(3), "{took:?}");
        silent.join().unwrap();
    }

    #[test]
    fn the_worker_answers_jobs_in_turn_and_times_out_the_ones_that_waited_too_long() {
        let rig = Rig::new();
        let (events, incoming) = mpsc::channel();
        let jobs = spawn(events);
        let mut late = rig.job(What::Find(parse_ref("Objects/Hat").unwrap()));
        (late.id, late.generation) = (7, 3);
        // Its deadline passed while it was queued.
        late.deadline = Instant::now() - Duration::from_millis(1);
        let mut timely = rig.job(What::Find(parse_ref("Objects/Hat").unwrap()));
        timely.id = 8;
        jobs.send(late).unwrap();
        jobs.send(timely).unwrap();
        let mut answers = Vec::new();
        for _ in 0..2 {
            match incoming.recv_timeout(Duration::from_secs(10)) {
                Ok(Event::Inventory { id, generation, result }) => answers.push((id, generation, result)),
                _ => panic!("no answer from the worker"),
            }
        }
        assert_eq!(answers[0], (7, 3, Err(Failed::TimedOut)));
        let hat = rig.named("Hat");
        assert_eq!(answers[1], (8, 1, Ok(Outcome::Found(hat))));
        // The late job asked the grid nothing.
        assert_eq!(rig.asked(), ["folders: My Inventory folders", "folders: Objects items"]);
    }

    // ---------------------------------------------- the Current Outfit

    #[test]
    fn wearing_clothing_adds_a_link_and_asks_for_the_appearance() {
        let rig = Rig::new();
        let shirt = rig.named("Blue shirt");
        assert_eq!(rig.outfit(What::Wear(shirt.clone())), Ok(vec!["Blue shirt".to_string()]));
        // Nothing is taken off: the white shirt stays under the blue one.
        assert_eq!(rig.worn(), ["Blue shirt", "Pale skin", "Round shape", "White shirt"]);
        assert_eq!(
            rig.asked(),
            [
                "folders: My Inventory folders",
                "folders: Current Outfit items",
                "items: White shirt, Pale skin, Round shape",
                "link: Blue shirt",
                "folders: Current Outfit items",
                // The folder's version after the change.
                "appearance: version 8",
            ]
        );
        let link = rig.grid().items.last().unwrap().clone();
        assert_eq!(
            (link.asset_id, link.inv_type, link.description.as_str()),
            (shirt.id, INV_WEARABLE, "Blue shirt, described")
        );
        // Worn already: refused, and nothing changes.
        let reason = rig.outfit(What::Wear(shirt)).unwrap_err();
        assert!(reason.contains("\"Blue shirt\"") && reason.contains("worn already"), "{reason}");
        assert!(!rig
            .asked()
            .iter()
            .any(|line| line.starts_with("link") || line.starts_with("appearance")));
    }

    #[test]
    fn wearing_a_body_part_replaces_the_one_of_its_kind() {
        let rig = Rig::new();
        let skin = rig.named("Tanned skin");
        assert_eq!(rig.outfit(What::Wear(skin)), Ok(vec!["Tanned skin".to_string()]));
        // The other skin went; the shape, another kind, and the shirt stay.
        assert_eq!(rig.worn(), ["Round shape", "Tanned skin", "White shirt"]);
        let asked = rig.asked();
        assert_eq!(
            asked[3..],
            [
                "remove: Pale skin",
                "link: Tanned skin",
                "folders: Current Outfit items",
                "appearance: version 9"
            ]
        );
    }

    #[test]
    fn a_refused_outfit_is_put_back() {
        let rig = Rig::new();
        rig.grid().refuse_appearance = true;
        let before = rig.worn();
        let skin = rig.named("Tanned skin");
        let reason = rig.outfit(What::Wear(skin)).unwrap_err();
        assert!(reason.contains("Outfit not acceptable"), "{reason}");
        assert!(reason.contains("put back as it was"), "{reason}");
        assert_eq!(rig.worn(), before);
        let asked = rig.asked();
        // Undone in reverse: the new link goes, the old one comes back.
        assert_eq!(
            asked[asked.len() - 3..],
            ["appearance: version 9", "remove: Tanned skin", "link: Pale skin"]
        );
        // The same when the grid does not say which link it made.
        rig.grid().quiet_links = true;
        let shirt = rig.named("Blue shirt");
        let reason = rig.outfit(What::Wear(shirt)).unwrap_err();
        assert!(reason.contains("put back as it was"), "{reason}");
        assert_eq!(rig.worn(), before);
    }

    #[test]
    fn a_grid_that_refuses_the_link_changes_nothing() {
        let rig = Rig::new();
        rig.grid().refuse_links = true;
        let before = rig.worn();
        let shirt = rig.named("Blue shirt");
        let reason = rig.outfit(What::Wear(shirt)).unwrap_err();
        assert!(
            reason.contains("refused to add \"Blue shirt\"") && reason.contains("Outfit folder is full"),
            "{reason}"
        );
        // Nothing had been changed: nothing to put back, no appearance asked.
        assert!(!reason.contains("put back"), "{reason}");
        assert_eq!(rig.worn(), before);
        assert!(!rig.asked().iter().any(|line| line.starts_with("appearance")));
    }

    #[test]
    fn a_failing_undo_is_said_in_the_error() {
        // The skin's old link is removed, the new one is refused, and so
        // is putting the old one back.
        let rig = Rig::new();
        rig.grid().refuse_links = true;
        let skin = rig.named("Tanned skin");
        let reason = rig.outfit(What::Wear(skin)).unwrap_err();
        assert!(reason.contains("refused to add \"Tanned skin\""), "{reason}");
        assert!(reason.contains("putting the Current Outfit folder back failed too"), "{reason}");
        assert_eq!(rig.worn(), ["Round shape", "White shirt"]);
        // Taking off: the appearance is refused, and then the link too.
        let rig = Rig::new();
        rig.grid().refuse_appearance = true;
        rig.grid().refuse_links = true;
        let reason = rig.outfit(What::TakeOff(Clothes::Kind(4))).unwrap_err();
        assert!(
            reason.contains("Outfit not acceptable") && reason.contains("failed too"),
            "{reason}"
        );
    }

    #[test]
    fn clothing_is_taken_off_by_item_and_by_kind() {
        let rig = Rig::new();
        let white = rig.named("White shirt");
        assert_eq!(
            rig.outfit(What::TakeOff(Clothes::Item(white.clone()))),
            Ok(vec!["White shirt".to_string()])
        );
        assert_eq!(rig.worn(), ["Pale skin", "Round shape"]);
        let asked = rig.asked();
        assert_eq!(
            asked[3..],
            ["remove: White shirt", "folders: Current Outfit items", "appearance: version 8"]
        );
        // Not worn any more.
        let reason = rig.outfit(What::TakeOff(Clothes::Item(white))).unwrap_err();
        assert!(reason.contains("\"White shirt\"") && reason.contains("is not worn"), "{reason}");

        // Two shirts and a pair of jeans: `shirt` takes off both shirts.
        let rig = Rig::new();
        for name in ["Blue shirt", "Jeans"] {
            let item = rig.named(name);
            rig.grid().link(OUTFIT, item.id, ASSET_LINK);
        }
        let mut taken = rig.outfit(What::TakeOff(Clothes::Kind(4))).unwrap();
        taken.sort();
        assert_eq!(taken, ["Blue shirt", "White shirt"]);
        assert_eq!(rig.worn(), ["Jeans", "Pale skin", "Round shape"]);
        // Nothing of the kind is worn; a body part of that number is not clothing.
        assert!(rig.outfit(What::TakeOff(Clothes::Kind(6))).unwrap_err().contains("wears no shoes"));
        assert!(rig.outfit(What::TakeOff(Clothes::Kind(1))).unwrap_err().contains("wears no skin"));
        // A refused outfit puts the links back.
        rig.grid().refuse_appearance = true;
        let reason = rig.outfit(What::TakeOff(Clothes::Kind(5))).unwrap_err();
        assert!(reason.contains("put back as it was"), "{reason}");
        assert_eq!(rig.worn(), ["Jeans", "Pale skin", "Round shape"]);
    }

    #[test]
    fn attachments_are_noted_in_the_outfit_folder_without_an_appearance_request() {
        let rig = Rig::new();
        let radio = rig.named("Radio");
        assert_eq!(rig.outfit(What::Link(radio.clone())), Ok(vec![]));
        assert!(rig.worn().contains(&"Radio".to_string()));
        assert_eq!(
            rig.asked(),
            ["folders: My Inventory folders", "folders: Current Outfit items", "link: Radio"]
        );
        // A link that is there is not added again.
        assert_eq!(rig.outfit(What::Link(radio.clone())), Ok(vec![]));
        assert_eq!(rig.asked(), ["folders: My Inventory folders", "folders: Current Outfit items"]);
        // Every link to the item goes.
        rig.grid().link(OUTFIT, radio.id, ASSET_LINK);
        assert_eq!(rig.outfit(What::Unlink(radio.id)), Ok(vec![]));
        assert!(!rig.worn().contains(&"Radio".to_string()));
        assert_eq!(rig.asked()[2..], ["remove: Radio", "remove: Radio"]);
        // Nothing to remove is fine too.
        assert_eq!(rig.outfit(What::Unlink(radio.id)), Ok(vec![]));
        // A refusal is the job's failure, for the log.
        rig.grid().refuse_links = true;
        assert!(rig.outfit(What::Link(radio)).unwrap_err().contains("Outfit folder is full"));
    }

    #[test]
    fn the_objects_the_outfit_folder_lists_are_what_should_be_attached() {
        let rig = Rig::new();
        let objects = |rig: &Rig| match rig.run(What::OutfitObjects) {
            Ok(Outcome::Items(items)) => Ok(items),
            Err(Failed::Reason(reason)) => Err(reason),
            other => panic!("unexpected {other:?}"),
        };
        // Clothing and body parts only: nothing to attach, and the folder
        // is not changed by the asking.
        assert_eq!(objects(&rig), Ok(vec![]));
        assert_eq!(
            rig.asked(),
            [
                "folders: My Inventory folders",
                "folders: Current Outfit items",
                "items: White shirt, Pale skin, Round shape"
            ]
        );
        // Links to two objects, one of them twice, and to an object that
        // is no longer in the inventory.
        let (radio, antenna) = (rig.named("Radio"), rig.named("Antenna"));
        for target in [radio.id, antenna.id, radio.id, Uuid::from_u128(0xDEAD)] {
            rig.grid().link(OUTFIT, target, ASSET_LINK);
        }
        assert_eq!(objects(&rig), Ok(vec![radio, antenna]));
        assert_eq!(rig.worn().len(), 7);
        // An empty folder.
        rig.grid().items.retain(|item| item.parent != OUTFIT);
        assert_eq!(objects(&rig), Ok(vec![]));
        // No folder: the reason the other outfit jobs give.
        rig.grid().folders.retain(|folder| folder.0 != OUTFIT);
        assert!(objects(&rig).unwrap_err().contains("no Current Outfit folder"));
    }

    #[test]
    fn an_offered_item_goes_into_the_system_folder_for_its_type() {
        let rig = Rig::new();
        let notecards = Uuid::from_u128(0x707);
        rig.grid().folders.push((notecards, ROOT, "Notecards".to_string(), ASSET_NOTECARD));
        // One of that type further down is not what is looked for.
        let objects = rig.grid().folders.iter().find(|folder| folder.2 == "Objects").unwrap().0;
        rig.grid()
            .folders
            .push((Uuid::from_u128(0x708), objects, "Old sounds".to_string(), ASSET_SOUND));
        let folder = |asset_type| match rig.run(What::SystemFolder(asset_type)) {
            Ok(Outcome::Folder(folder)) => folder,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(folder(ASSET_NOTECARD), notecards);
        // The top level is read, and nothing below it.
        assert_eq!(rig.asked(), ["folders: My Inventory folders"]);
        // No system folder for the type: the top of My Inventory.
        assert_eq!(folder(ASSET_SOUND), ROOT);
        assert_eq!(rig.asked(), ["folders: My Inventory folders"]);
        // A folder that is given goes to the top without a look.
        assert_eq!(folder(ASSET_FOLDER), ROOT);
        assert_eq!(rig.asked(), [] as [&str; 0]);
        // A grid that does not answer is the job's failure.
        rig.grid().fail_reads = true;
        assert!(rig.run(What::SystemFolder(ASSET_NOTECARD)).is_err());
    }

    #[test]
    fn an_inventory_without_the_outfit_folder_or_service_is_refused() {
        let rig = Rig::new();
        rig.grid().folders.retain(|folder| folder.0 != OUTFIT);
        let shirt = rig.named("Blue shirt");
        assert!(rig
            .outfit(What::Wear(shirt.clone()))
            .unwrap_err()
            .contains("no Current Outfit folder"));
        assert!(rig
            .outfit(What::Link(shirt.clone()))
            .unwrap_err()
            .contains("no Current Outfit folder"));
        // The two reading services alone, as on OpenSimulator.
        let rig = Rig::new();
        let mut job = rig.job(What::Wear(shirt));
        (job.urls.api, job.urls.appearance) = (None, None);
        assert_eq!(carry_out(&rig.agent, &job), Err(Failed::Reason(NO_OUTFIT_SERVICE.to_string())));
        assert_eq!(rig.worn(), ["Pale skin", "Round shape", "White shirt"]);
    }
}
