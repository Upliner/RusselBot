//! What the bot does with a command from the control socket: the chat-bar
//! parsing of `say`, the avatars commands name and their lookup on the
//! grid, the commands that move the avatar, the ones that use its
//! inventory, and the ones waiting for a lookup, for the inventory or for
//! the simulator to say how they ended.

use crate::config::DEFAULT_REQUESTTP_TIMEOUT;
use crate::control::{PermsRequest, Request, Response};
use crate::inventory::{self, Clothes, Entry, Failed, Item, ListArgs, Outcome, Removal, What};
use crate::names::Names;
use crate::perms::{self, Ask, Kind, Owner, Rules};
use crate::persist::{Location, SitState};
use crate::reconciler::same_region;
use crate::rlv::{Check, Limits, Refusal, Restrictions, Sitting, Who};
use crate::scan::{self, Query, Sighting};
use crate::session::{InventoryOffer, Look, Thing, Touch, ATTACH_LIMIT, DETACH_LIMIT, NOT_CONNECTED};
use crate::udp::messages::{Vector3, MAX_TEXT};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the grid has to answer a name lookup.
pub const LOOKUP_LIMIT: Duration = Duration::from_secs(10);

/// How long after its deadline an inventory job is given up on without an
/// answer from the worker, which keeps the deadline itself.
const JOB_GRACE: Duration = Duration::from_secs(3);

/// A teleport offer's message travels in a one-byte-length field.
const MAX_OFFER_TEXT: usize = 254;

const WHISPER: u8 = 0;
const SAY: u8 = 1;
const SHOUT: u8 = 2;

/// A `say` message after chat-bar parsing.
#[derive(Debug, PartialEq)]
pub struct Chat {
    pub channel: i32,
    /// 0 whisper, 1 say, 2 shout.
    pub chat_type: u8,
    pub text: String,
}

fn check_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("there is no text to send".to_string());
    }
    if text.len() > MAX_TEXT {
        return Err(format!("the text is {} bytes long; the limit is {MAX_TEXT}", text.len()));
    }
    Ok(())
}

/// Reads the start of `message` the way a viewer's chat bar does: `/N` names
/// a channel, `//` the channel used last, `/shout ` and `/whisper ` the
/// volume. Everything else, `/me` included, is the text itself.
pub fn parse_chat(message: &str, last_channel: i32) -> Result<Chat, String> {
    let chat = |channel, chat_type, text: &str| Chat {
        channel,
        chat_type,
        text: text.to_string(),
    };
    let digits = message
        .strip_prefix('/')
        .map_or(0, |rest| rest.bytes().take_while(u8::is_ascii_digit).count());
    let parsed = if let Some(rest) = message.strip_prefix("//") {
        chat(last_channel, SAY, rest)
    } else if digits > 0 {
        let (number, rest) = message[1..].split_at(digits);
        let channel = number.parse().map_err(|_| format!("channel {number} is out of range"))?;
        chat(channel, SAY, rest.trim_start())
    } else if let Some(rest) = message.strip_prefix("/shout ") {
        chat(0, SHOUT, rest)
    } else if let Some(rest) = message.strip_prefix("/whisper ") {
        chat(0, WHISPER, rest)
    } else {
        chat(0, SAY, message)
    };
    check_text(&parsed.text)?;
    Ok(parsed)
}

/// What a `sit` command names.
#[derive(Debug, PartialEq)]
pub enum SitTarget {
    Ground,
    Object(Uuid),
}

pub fn parse_sit(target: &str) -> Result<SitTarget, String> {
    let target = target.trim();
    if target.eq_ignore_ascii_case("ground") {
        return Ok(SitTarget::Ground);
    }
    match Uuid::parse_str(target) {
        Ok(object) => Ok(SitTarget::Object(object)),
        Err(_) => Err(format!("\"{target}\" is neither an object's UUID nor `ground`")),
    }
}

/// Where a `tp` command goes; what is `None` stays as it is.
#[derive(Debug, PartialEq)]
pub struct Destination {
    pub x: f32,
    pub y: f32,
    pub z: Option<f32>,
    pub region: Option<String>,
}

fn coordinate(text: &str, name: &str) -> Result<f32, String> {
    match text.parse::<f32>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(format!("`{name}` must be a number that is not negative, not \"{text}\"")),
    }
}

/// Reads `x y [z] [region words]`. A third argument that is a number is
/// the height; anything else starts the region's name.
pub fn parse_tp(args: &[String]) -> Result<Destination, String> {
    let [x, y, rest @ ..] = args else {
        return Err("`tp` needs at least x and y".to_string());
    };
    let (x, y) = (coordinate(x, "x")?, coordinate(y, "y")?);
    let is_number = |text: &String| text.parse::<f32>().is_ok_and(f32::is_finite);
    let (z, region) = match rest.split_first() {
        Some((first, words)) if is_number(first) => (Some(coordinate(first, "z")?), words),
        _ => (None, rest),
    };
    let region = region.join(" ").trim().to_string();
    Ok(Destination {
        x,
        y,
        z,
        region: (!region.is_empty()).then_some(region),
    })
}

/// Who an `im` command is for.
#[derive(Debug, PartialEq)]
pub enum Recipient {
    Id(Uuid),
    /// A full name in lower case, `first last`.
    Name(String),
}

/// Reads a recipient: a UUID, `First Last`, `first.last`, or a single word,
/// which stands for the last name `Resident`.
pub fn parse_recipient(given: &str) -> Result<Recipient, String> {
    let given = given.trim();
    if let Ok(id) = Uuid::parse_str(given) {
        return Ok(Recipient::Id(id));
    }
    crate::names::full_name(given)
        .map(Recipient::Name)
        .ok_or_else(|| format!("\"{given}\" is not an avatar name (`First Last`, `first.last` or one word) or a UUID"))
}

/// What to ask the grid's avatar search for. The search matches the start
/// of a name, and an avatar without a last name of its own is found by its
/// first name.
fn search_text(name: &str) -> &str {
    name.strip_suffix(" resident").unwrap_or(name)
}

/// The avatar among `avatars` whose full name is exactly `name` (lower
/// case), whatever the case of its letters.
pub fn exact_match<'a>(name: &str, avatars: &'a [(Uuid, String)]) -> Option<&'a (Uuid, String)> {
    avatars.iter().find(|(_, full)| full.to_lowercase() == name)
}

/// Who asks for something: the operator through the control socket, a
/// sender of RLV commands, or the bot itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    Operator,
    /// The operator's one command that `rlv mode override` exempts from
    /// the restrictions.
    Exempt,
    /// A force command of this sender: an object the avatar owns, or the
    /// operator through `rlv cmd`.
    Rlv(Who),
    /// What the bot does by itself: accepting offers and requests,
    /// restoring attachments.
    Bot,
}

/// How the operator log marks the operator's commands.
const CONTROL: &str = "control";

impl Origin {
    /// How the operator log marks a command of this origin.
    fn tag(self) -> &'static str {
        match self {
            Origin::Operator | Origin::Exempt => CONTROL,
            Origin::Rlv(_) => "rlv",
            Origin::Bot => "bot",
        }
    }

    /// Whether this is a command of the operator's own.
    fn is_operator(self) -> bool {
        matches!(self, Origin::Operator | Origin::Exempt)
    }

    /// The restrictions that are in force for what this origin does: none
    /// for the exempt command, and for a sender of RLV commands all but
    /// its own.
    pub fn limits(self, kept: Option<&Restrictions>) -> Limits<'_> {
        match self {
            Origin::Exempt => Limits::none(),
            Origin::Rlv(who) => Limits::new(kept, Some(who)),
            Origin::Operator | Origin::Bot => Limits::new(kept, None),
        }
    }

    /// A refusal as this origin is told it: the operator with the way
    /// past it.
    fn refused(self, refusal: Refusal) -> String {
        match self.is_operator() {
            true => refusal.answer(),
            false => refusal.short(),
        }
    }
}

/// How far it is from one place in a region to another, in metres.
fn distance(from: Vector3, to: Vector3) -> f32 {
    [0, 1, 2].map(|i| (to[i] - from[i]).powi(2)).iter().sum::<f32>().sqrt()
}

/// The part of a session the commands use. Everything that sends refuses
/// while the avatar is not in a region.
pub trait Grid {
    fn say(&mut self, channel: i32, chat_type: u8, text: &str, now: Instant) -> Result<(), String>;
    fn instant_message(&mut self, to: Uuid, text: &str, now: Instant) -> Result<(), String>;
    fn find_avatar(&mut self, query: Uuid, name: &str, now: Instant) -> Result<(), String>;
    fn stand(&mut self, now: Instant) -> Result<(), String>;
    fn sit_ground(&mut self, now: Instant) -> Result<(), String>;
    /// Ends in the session's `Seated` or `SitFailed`.
    fn sit(&mut self, object: Uuid, now: Instant) -> Result<(), String>;
    /// Ends in `RegionEntered`, `TeleportArrived` or `TeleportFailed`.
    fn teleport(&mut self, region: &str, position: Vector3, now: Instant) -> Result<(), String>;
    /// Teleports to `position` in the region with this handle, without
    /// looking anything up. Ends as `teleport` does.
    fn teleport_to(&mut self, handle: u64, position: Vector3, now: Instant) -> Result<(), String>;
    /// The region the avatar is in.
    fn region(&self) -> Option<String>;
    /// Where the simulator last said the avatar is.
    fn position(&self) -> Option<Vector3>;
    fn request_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String>;
    fn offer_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String>;
    /// Ends like `teleport`.
    fn accept_teleport(&mut self, lure: Uuid, now: Instant) -> Result<(), String>;
    fn touch(&mut self, object: Uuid, now: Instant) -> Result<Touch, String>;
    /// Hands the inventory worker a job, to be answered through
    /// `Commands::inventory_answer` with the same `id`. Refuses where the
    /// simulator has no inventory service.
    fn inventory(&mut self, id: u64, deadline: Instant, what: What) -> Result<(), String>;
    /// Whether the grid makes the avatar's appearance from its Current
    /// Outfit folder, which is what clothing needs.
    fn outfit_service(&self) -> bool;
    /// Whether the avatar is in a region whose simulator has said which
    /// services it offers.
    fn ready(&self) -> bool;
    /// The inventory items attached to the avatar now, and their points.
    fn attached(&self) -> Vec<(Uuid, u8)>;
    /// The inventory item of the attachment on the avatar whose root prim
    /// is `object`.
    fn attached_item(&self, object: Uuid) -> Option<Uuid>;
    /// Ends in the session's `Attached` or `AttachFailed`. `point` 0
    /// leaves the choice to the simulator.
    fn attach(&mut self, item: &Item, point: u8, now: Instant) -> Result<(), String>;
    /// Ends in the session's `Detached` or `DetachFailed`.
    fn detach(&mut self, item: Uuid, now: Instant) -> Result<(), String>;
    fn animate(&mut self, animation: Uuid, start: bool, now: Instant) -> Result<(), String>;
    fn play_sound(&mut self, sound: Uuid, now: Instant) -> Result<(), String>;
    /// Asks the simulator who owns `object`, to be answered through
    /// `Commands::object_owner`.
    fn object_owner(&mut self, object: Uuid, now: Instant) -> Result<(), String>;
    /// Grants the script `item` in `object` the permissions it asked for.
    fn grant_permissions(&mut self, object: Uuid, item: Uuid, permissions: i32, now: Instant) -> Result<(), String>;
    /// Accepts the inventory offer `transaction` of `to`, an avatar or the
    /// owner of the giving object, and has the item filed in `folder`.
    fn accept_inventory(&mut self, to: Uuid, from_object: bool, transaction: Uuid, folder: Uuid, now: Instant) -> Result<(), String>;
    /// The RLV restrictions that are in force: none while RLV is off.
    fn restrictions(&self) -> Option<&Restrictions>;
    /// Whether, and on what, the avatar sits, as far as is known.
    fn sitting(&self) -> Sitting;
    /// What `object` is, which prims belong to it and how far it is, as
    /// far as the simulator has described it.
    fn thing(&self, object: Uuid) -> Thing;
    /// The prims of the attachment on the avatar made from `item`.
    fn attachment_prims(&self, item: Uuid) -> Vec<Uuid>;
    /// Looks for an object the simulator has not described, with the
    /// object scan. A wait ends in the session's `Described` or
    /// `NotDescribed`.
    fn look_for(&mut self, object: Uuid, now: Instant) -> Result<Look, String>;
    /// Why `object` cannot be touched when the scan did not turn it up.
    fn unknown_object(&self, object: Uuid) -> String;
    /// Makes ready for a `scan` command that reaches `distance` metres,
    /// or as far as the object scan does: starts that scan, and widens it
    /// when the distance is more. Returns how far the command reaches.
    fn reach(&mut self, distance: Option<f32>, now: Instant) -> Result<f32, String>;
    /// The prims the simulator has described, with their distance.
    fn sightings(&self) -> Vec<Sighting>;
    /// Asks the simulator what the objects with these region-local ids
    /// are called and whose they are, as far as it has not said and was
    /// not asked a moment ago.
    fn ask_properties(&mut self, local_ids: &[u32], now: Instant);
    /// When the objects around the avatar count as all described.
    fn settled_at(&self) -> Instant;
}

/// A `scan` command waiting for the simulator to describe and name the
/// objects around the avatar.
struct Scanning {
    query: Query,
    /// How far it lists, in metres.
    distance: f32,
    /// Its arguments, for the operator log.
    args: String,
    reply: Sender<Response>,
    /// When it is answered with what is known.
    deadline: Instant,
}

/// How long a `scan` waits for the simulator before it lists what it knows.
pub const SCAN_LIMIT: Duration = Duration::from_secs(30);
/// How often the waiting scans are looked at.
const SCAN_POLL: Duration = Duration::from_millis(250);
/// The most owners' names kept from one scan to the next.
const MAX_OWNER_NAMES: usize = 10_000;

/// A command that a restriction refuses for an object the simulator has
/// not described, waiting for it to be: it is then carried out again from
/// the start, as whoever gave it.
struct Looking {
    object: Uuid,
    origin: Origin,
    request: Request,
    reply: Sender<Response>,
}

/// An inventory offer that is being accepted: the grid is being asked
/// which folder it goes into.
struct Filing {
    request: perms::Request,
    /// The `accept` command that waits for it; nobody for an offer the
    /// bot accepts by itself.
    reply: Option<Sender<Response>>,
    deadline: Instant,
}

/// What a request still needs to be known before the rules can decide it.
#[derive(Debug, PartialEq)]
enum Needs {
    /// Who owns this object.
    Owner(Uuid),
    /// The name of this avatar.
    Name(Uuid),
}

/// A request that waits for the grid to say who is behind it.
struct Undecided {
    request: perms::Request,
    needs: Needs,
    /// When it is decided with what is known.
    deadline: Instant,
}

fn parse_kind(kind: &str) -> Result<Kind, String> {
    Kind::parse(kind).ok_or_else(|| format!("`{kind}` is no kind of request; the kinds are {}", perms::KINDS))
}

/// What a command does once the inventory has answered.
#[derive(Debug, Clone, PartialEq)]
enum Then {
    /// Print the listing.
    List(ListArgs),
    /// Attach the item found; `given` is the item as the command named it.
    Attach {
        given: String,
        point: Option<u8>,
    },
    Wear {
        given: String,
    },
    Detach {
        given: String,
    },
    Anim {
        given: String,
        start: bool,
    },
    Sound {
        given: String,
    },
    /// Detach what is on the point, now that the items' names are known.
    DetachPoint {
        point: u8,
        items: Vec<Uuid>,
    },
    /// Say what was put on, or taken off.
    Outfit {
        put_on: bool,
    },
}

impl Then {
    /// What to tell the operator when the job came to nothing.
    fn failure(&self, failed: Failed) -> String {
        let limit = inventory::LIMIT.as_secs();
        match (failed, self) {
            (Failed::Reason(reason), _) => reason,
            (Failed::TimedOut, Then::List(_)) => {
                format!("the listing took longer than {limit} s; name a folder to list less of the inventory")
            }
            (Failed::TimedOut, Then::DetachPoint { point, .. }) => format!(
                "the grid did not say within {limit} s what is attached to {}",
                inventory::point_name(*point)
            ),
            (Failed::TimedOut, Then::Outfit { .. }) => {
                format!("the grid did not answer within {limit} s; the avatar's outfit may or may not have changed")
            }
            (
                Failed::TimedOut,
                Then::Attach { given, .. }
                | Then::Wear { given }
                | Then::Detach { given }
                | Then::Anim { given, .. }
                | Then::Sound { given },
            ) => format!("the grid did not answer the lookup of \"{given}\" within {limit} s"),
        }
    }
}

/// A command waiting for an inventory job.
struct Pending {
    command: &'static str,
    origin: Origin,
    reply: Sender<Response>,
    /// When the command gives up on the inventory: `LIMIT` after it
    /// arrived, whatever jobs it takes.
    deadline: Instant,
    then: Then,
}

/// An `attach` or `wear` waiting to see its object on the avatar.
struct Attaching {
    command: &'static str,
    item: Item,
    reply: Sender<Response>,
    /// What the simulator said meanwhile, which may be why not.
    alert: Option<String>,
}

/// A `detach` waiting to see its objects gone from the avatar.
struct Detaching {
    origin: Origin,
    reply: Sender<Response>,
    waiting: Vec<Item>,
    gone: Vec<Item>,
    /// Those the simulator did not take off in time.
    stuck: Vec<Item>,
    /// What a restriction kept on the point, and which.
    kept: Option<String>,
}

/// An instant message that went out, for the message log.
#[derive(Debug, Clone, PartialEq)]
pub struct SentIm {
    pub to: Uuid,
    /// `First Last`; empty when the recipient was given by UUID.
    pub name: String,
    pub text: String,
}

/// What a command does with an avatar once it is known who that is.
#[derive(Debug, Clone, PartialEq)]
enum After {
    Im(String),
    RequestTp(String),
    SendTp(String),
}

impl After {
    fn command(&self) -> &'static str {
        match self {
            After::Im(_) => "im",
            After::RequestTp(_) => "requesttp",
            After::SendTp(_) => "sendtp",
        }
    }
}

/// A command waiting for its avatar's name to be resolved.
struct Waiting {
    query: Uuid,
    /// The full name in lower case.
    name: String,
    /// The avatar as the command gave it.
    given: String,
    then: After,
    /// Whose command it is: the operator's, exempt or not.
    origin: Origin,
    reply: Sender<Response>,
    deadline: Instant,
}

/// How far a command got.
enum Done {
    /// Carried out; this is the answer's text.
    Answer(Option<String>),
    /// An instant message went out; the answer's text and the message.
    Sent(String, SentIm),
    /// The answer comes later: the reply has been put aside.
    Parked,
}

/// How long the simulator of a region the avatar has entered is given to
/// show what is attached before the Current Outfit folder is compared with
/// it. Second Life has shown everything within four seconds.
pub const RESTORE_DELAY: Duration = Duration::from_secs(20);
/// How soon to look again when the avatar is between regions at that time.
const RESTORE_RETRY: Duration = Duration::from_secs(1);
/// The most attaches remembered as given up on.
const MAX_GAVE_UP: usize = 64;

/// How far the restoring of attachments has got since the login.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Restore {
    /// No region has been entered yet.
    Idle,
    /// Compare the Current Outfit folder with the avatar at this time.
    Due(Instant),
    /// The folder is being read, by the job with this id.
    Reading(u64),
    /// Compared; nothing more until the next login.
    Done,
}

/// Carries out commands. Lives as long as the bot, across sessions.
pub struct Commands {
    /// The channel `//` stands for.
    last_channel: i32,
    /// Resolved names: lower-case full name to id and the name as the grid
    /// spells it.
    known: HashMap<String, (Uuid, String)>,
    waiting: Vec<Waiting>,
    queries: u128,
    /// How long an asked avatar's teleport offer is accepted unprompted.
    offer_timeout: Duration,
    /// The avatars asked for a teleport, when, and whether the command
    /// that asked was exempt from the RLV restrictions.
    asked: HashMap<Uuid, (Instant, bool)>,
    /// The avatars the bot has exchanged instant messages with in this
    /// session: a message to one of them starts no conversation.
    talking: HashSet<Uuid>,
    /// Which requests are accepted without the operator.
    rules: Rules,
    /// The requests that were not, and can still be accepted on command.
    requests: perms::Waiting,
    /// Requests the rules cannot decide before the grid has said who is
    /// behind them.
    undecided: Vec<Undecided>,
    /// Inventory offers being accepted, by the id of the inventory job
    /// that finds their folder.
    filing: HashMap<u64, Filing>,
    /// Avatars whose names `undecided` requests wait for, not asked yet.
    wanted_names: Vec<Uuid>,
    /// The `sit` command waiting for the simulator's verdict.
    pending_sit: Option<(Sender<Response>, Origin)>,
    /// The `tp` or `accept tp` command waiting for the teleport to end.
    pending_teleport: Option<(Sender<Response>, Origin)>,
    /// `touch` commands waiting for their object to turn up.
    touches: Vec<(Uuid, Sender<Response>)>,
    /// `sit` and `touch` commands waiting for their object to be described
    /// before the restrictions decide them.
    looking: Vec<Looking>,
    /// `scan` commands waiting for the simulator, and when they are next
    /// looked at.
    scanning: Vec<Scanning>,
    scan_poll: Option<Instant>,
    /// The names of the owners of scanned objects: `None` for one the
    /// name service did not name. Kept from scan to scan when known.
    owners: HashMap<Uuid, Option<String>>,
    /// The owners whose names the name service has been or will be asked.
    owner_lookups: HashSet<Uuid>,
    /// Whose command is being carried out.
    origin: Origin,
    /// A command moved the avatar since `take_manual` was last asked.
    went_manual: bool,
    /// Where the last `tp` command that was carried out went.
    last_tp: Option<Location>,
    /// What the last `sit` command that was carried out named.
    last_sit: Option<SitState>,
    /// Inventory jobs under way, by their id.
    jobs: HashMap<u64, Pending>,
    job_ids: u64,
    /// Changes to the Current Outfit folder that follow an attach or a
    /// detach and that nobody waits for: what each is, by job id.
    notes: HashMap<u64, String>,
    attaching: Vec<Attaching>,
    detaching: Vec<Detaching>,
    /// The config option `restore_attachments`.
    restore_on: bool,
    restore: Restore,
    /// Objects the simulator was asked to attach because the Current
    /// Outfit folder lists them, and has not shown yet. Nobody waits.
    restoring: Vec<Item>,
    /// Items whose `attach` was answered as not carried out in this
    /// session: one that arrives after all still gets its link.
    gave_up: Vec<Item>,
    /// The lines `tell` wrote.
    #[cfg(test)]
    told: Vec<String>,
}

impl Default for Commands {
    fn default() -> Commands {
        Commands::new(DEFAULT_REQUESTTP_TIMEOUT)
    }
}

fn refuse(tag: &str, what: &str, reply: &Sender<Response>, reason: String) {
    crate::info!("{tag}: {what} refused: {reason}");
    // The client may have gone away; the command is settled either way.
    let _ = reply.send(Response::Error(reason));
}

pub(crate) fn whom(id: Uuid, name: &str) -> String {
    if name.is_empty() {
        id.to_string()
    } else {
        format!("{name} <{id}>")
    }
}

impl Commands {
    pub fn new(offer_timeout: Duration) -> Commands {
        Commands {
            last_channel: 0,
            known: HashMap::new(),
            waiting: Vec::new(),
            queries: 0,
            offer_timeout,
            asked: HashMap::new(),
            talking: HashSet::new(),
            rules: Rules::default(),
            requests: perms::Waiting::default(),
            undecided: Vec::new(),
            filing: HashMap::new(),
            wanted_names: Vec::new(),
            pending_sit: None,
            pending_teleport: None,
            touches: Vec::new(),
            looking: Vec::new(),
            scanning: Vec::new(),
            scan_poll: None,
            owners: HashMap::new(),
            owner_lookups: HashSet::new(),
            origin: Origin::Operator,
            went_manual: false,
            last_tp: None,
            last_sit: None,
            jobs: HashMap::new(),
            job_ids: 0,
            notes: HashMap::new(),
            attaching: Vec::new(),
            detaching: Vec::new(),
            restore_on: true,
            restore: Restore::Idle,
            restoring: Vec::new(),
            gave_up: Vec::new(),
            #[cfg(test)]
            told: Vec::new(),
        }
    }

    /// Whether attachments are restored after a login: the config option
    /// `restore_attachments`.
    pub fn set_restore(&mut self, on: bool) {
        self.restore_on = on;
    }

    /// Writes a line of the bot's own doing to the operator log.
    fn tell(&mut self, line: String) {
        crate::info!("{line}");
        #[cfg(test)]
        self.told.push(line);
    }

    /// The restrictions in force for the command that is being carried out.
    fn limits<'a, G: Grid>(&self, grid: &'a G) -> Limits<'a> {
        self.origin.limits(grid.restrictions())
    }

    /// A refusal as whoever gave that command is told it.
    fn refused(&self, refusal: Refusal) -> String {
        self.origin.refused(refusal)
    }

    /// An avatar sent the bot an instant message: the two are talking.
    pub fn im_received(&mut self, from: Uuid) {
        self.talking.insert(from);
    }

    /// Carries out `request` on `grid` and answers on `reply`: now, or once
    /// the grid has resolved a name or the simulator has said how it ended.
    /// Returns the instant message sent. `persist`, `home`, `status` and
    /// `quit` are the bot's own business and never get here.
    #[cfg(test)]
    pub fn handle<G: Grid>(&mut self, request: Request, reply: Sender<Response>, grid: Option<&mut G>, now: Instant) -> Option<SentIm> {
        self.handle_from(Origin::Operator, request, reply, grid, now)
    }

    /// `handle` for a command of `origin`. One carried out for RLV is
    /// marked so in the log and is not what `last_cmd` stands for.
    pub fn handle_from<G: Grid>(
        &mut self,
        origin: Origin,
        request: Request,
        reply: Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Option<SentIm> {
        self.origin = origin;
        let command = request.name();
        let done = match request {
            Request::Say(message) => self.say(&message, grid, now),
            Request::Shout(message) => self.say_plainly(SHOUT, &message, grid, now),
            Request::Whisper(message) => self.say_plainly(WHISPER, &message, grid, now),
            Request::Im { to, text } => check_text(&text).and_then(|()| self.with_avatar(&to, After::Im(text), &reply, grid, now)),
            Request::Stand => self.stand(grid, now),
            Request::Sit(target) => self.sit(&target, &reply, grid, now),
            Request::Tp(args) => self.tp(&args, &reply, grid, now),
            Request::AskTp { to, message } => {
                check_length(&message, MAX_TEXT).and_then(|()| self.with_avatar(&to, After::RequestTp(message), &reply, grid, now))
            }
            Request::Accept { kind, who } => self.accept_command(&kind, who.as_deref(), &reply, grid, now),
            Request::Perms(request) => self.perms(request),
            Request::SendTp { to, message } => {
                check_length(&message, MAX_OFFER_TEXT).and_then(|()| self.with_avatar(&to, After::SendTp(message), &reply, grid, now))
            }
            Request::Touch(object) => self.touch(&object, &reply, grid, now),
            Request::Scan(args) => self.scan(&args, &reply, grid, now),
            Request::Persist { .. } | Request::Home | Request::Status | Request::Rlv(_) | Request::Autoreply(_) | Request::Quit => {
                Err("not a command for the grid".to_string())
            }
            Request::Inventory(args) => self.inventory(&args, &reply, grid, now),
            Request::Attach { item, part } => self.find_for_attach(&item, part.as_deref(), &reply, grid, now),
            Request::Wear(item) => {
                let then = Then::Wear { given: item.clone() };
                self.find(&item, then, command, &reply, grid, now)
            }
            Request::Detach(what) => self.detach(&what, &reply, grid, now),
            Request::Anim { stop, item } => {
                let then = Then::Anim {
                    given: item.clone(),
                    start: !stop,
                };
                self.find(&item, then, command, &reply, grid, now)
            }
            Request::Sound(item) => {
                let then = Then::Sound { given: item.clone() };
                self.find(&item, then, command, &reply, grid, now)
            }
        };
        let sent = finish(origin.tag(), command, done, &reply);
        self.origin = Origin::Operator;
        sent
    }

    /// The operator sent the bot home: an answer to an earlier `requesttp`
    /// must not take it away again.
    pub fn forget_requests(&mut self) {
        self.asked.clear();
    }

    /// Whether a command has moved the avatar since this was last asked, so
    /// that the bot should stop putting it back.
    pub fn take_manual(&mut self) -> bool {
        std::mem::take(&mut self.went_manual)
    }

    /// Where the last `tp` command went that was handed to the simulator,
    /// whatever became of it: what `last_cmd` stands for as a location.
    pub fn last_tp(&self) -> Option<&Location> {
        self.last_tp.as_ref()
    }

    /// What the last `sit` command named that was handed to the simulator,
    /// an object or the ground: what `last_cmd` stands for as a sit state.
    pub fn last_sit(&self) -> Option<SitState> {
        self.last_sit
    }

    fn say<G: Grid>(&mut self, message: &str, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let chat = parse_chat(message, self.last_channel)?;
        let done = self.speak(&chat, grid.ok_or(NOT_CONNECTED)?, now)?;
        self.last_channel = chat.channel;
        Ok(done)
    }

    /// Says what the RLV restrictions leave of `chat`: on other channels,
    /// at another volume or shortened, which the answer then says.
    fn speak<G: Grid>(&mut self, chat: &Chat, grid: &mut G, now: Instant) -> Result<Done, String> {
        let spoken = self.limits(&*grid).chat(chat.channel, chat.chat_type, &chat.text);
        let spoken = spoken.map_err(|refusal| self.refused(refusal))?;
        for said in &spoken.utterances {
            grid.say(said.channel, said.volume, &said.text, now)?;
            let how = match said.volume {
                WHISPER => "whispered",
                SHOUT => "shouted",
                _ => "said",
            };
            crate::info!("control: {how} on channel {}: {}", said.channel, said.text);
        }
        for change in &spoken.changes {
            crate::info!("rlv: say: {change}");
        }
        Ok(Done::Answer((!spoken.changes.is_empty()).then(|| spoken.changes.join("; "))))
    }

    /// `shout` and `whisper`: the message as it is, on channel 0.
    fn say_plainly<G: Grid>(&mut self, chat_type: u8, message: &str, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        check_text(message)?;
        let chat = Chat {
            channel: 0,
            chat_type,
            text: message.to_string(),
        };
        self.speak(&chat, grid.ok_or(NOT_CONNECTED)?, now)
    }

    fn stand<G: Grid>(&mut self, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let allowed = self.limits(&*grid).stand(grid.sitting());
        allowed.map_err(|refusal| self.refused(refusal))?;
        grid.stand(now)?;
        self.went_manual = true;
        crate::info!("{}: standing up", self.origin.tag());
        self.stood_up();
        Ok(Done::Answer(None))
    }

    /// The avatar was told to stand up, by a command here or by the bot: a
    /// `sit` command still waiting for its seat will not get it.
    pub fn stood_up(&mut self) {
        if let Some((sit, origin)) = self.pending_sit.take() {
            refuse(origin.tag(), "sit", &sit, "the avatar was told to stand up".to_string());
        }
    }

    fn sit<G: Grid>(&mut self, target: &str, reply: &Sender<Response>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let target = parse_sit(target)?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let sitting = grid.sitting();
        let object = match target {
            SitTarget::Ground => {
                // A forced ground sit is held by `unsit` on an object only.
                let leaving = match (self.origin, sitting) {
                    (Origin::Rlv(_), Sitting::Ground) => Sitting::No,
                    _ => sitting,
                };
                let allowed = self.limits(&*grid).sit_ground(leaving);
                allowed.map_err(|refusal| self.refused(refusal))?;
                grid.sit_ground(now)?;
                self.went_manual = true;
                self.remember_sit(SitState::Ground);
                crate::info!("{}: sitting on the ground", self.origin.tag());
                return Ok(Done::Answer(None));
            }
            SitTarget::Object(object) => object,
        };
        let check = self.limits(&*grid).sit_on(sitting, &grid.thing(object));
        match check {
            Check::Allowed => {}
            Check::Refused(refusal) => return Err(self.refused(refusal)),
            Check::LookFirst(why) => match grid.look_for(object, now)? {
                Look::Waiting => return Ok(self.look(object, Request::Sit(object.to_string()), why, reply)),
                // Decided by what is known now: too far when it was not found.
                Look::Known | Look::GaveUp => {
                    let allowed = self.limits(&*grid).sit(sitting, grid.thing(object).distance);
                    allowed.map_err(|refusal| self.refused(refusal))?;
                }
            },
        }
        grid.sit(object, now)?;
        self.went_manual = true;
        self.remember_sit(SitState::Object(object));
        crate::info!("{}: sitting on {object}", self.origin.tag());
        if let Some((earlier, origin)) = self.pending_sit.replace((reply.clone(), self.origin)) {
            refuse(origin.tag(), "sit", &earlier, "superseded by a newer sit command".to_string());
        }
        Ok(Done::Parked)
    }

    /// Whether the restrictions let the command that is being carried
    /// out teleport the avatar to `position` in `region`. A forced
    /// teleport is not held by `tplocal`.
    fn may_teleport<G: Grid>(&self, grid: &G, region: &str, position: Vector3) -> Result<(), String> {
        let limits = self.limits(grid);
        let sitting = grid.sitting();
        let allowed = match self.origin {
            Origin::Rlv(_) => limits.forced_teleport(sitting),
            _ => {
                let here = grid.region().is_some_and(|here| same_region(&here, region));
                // From a place that is not known, any place is far.
                let local = here.then(|| grid.position().map_or(f32::INFINITY, |from| distance(from, position)));
                limits.teleport(sitting, local)
            }
        };
        allowed.map_err(|refusal| self.refused(refusal))
    }

    fn tp<G: Grid>(&mut self, args: &[String], reply: &Sender<Response>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let to = parse_tp(args)?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let region = match to.region {
            Some(region) => region,
            None => grid.region().ok_or(NOT_CONNECTED)?,
        };
        let z = match to.z.or(grid.position().map(|position| position[2])) {
            Some(z) => z,
            None => return Err("the avatar's height is not known; give z".to_string()),
        };
        self.may_teleport(&*grid, &region, [to.x, to.y, z])?;
        grid.teleport(&region, [to.x, to.y, z], now)?;
        self.went_manual = true;
        crate::info!("{}: teleporting to \"{region}\" at {}, {}, {z}", self.origin.tag(), to.x, to.y);
        if self.origin.is_operator() {
            self.last_tp = Some(Location {
                region,
                position: [to.x, to.y, z],
            });
        }
        self.park_teleport(reply);
        Ok(Done::Parked)
    }

    fn park_teleport(&mut self, reply: &Sender<Response>) {
        if let Some((earlier, origin)) = self.pending_teleport.replace((reply.clone(), self.origin)) {
            refuse(origin.tag(), "tp", &earlier, "superseded by a newer teleport".to_string());
        }
    }

    /// What a `sit` command named, for `last_cmd`: the operator's only.
    fn remember_sit(&mut self, sit: SitState) {
        if self.origin.is_operator() {
            self.last_sit = Some(sit);
        }
    }

    /// A teleport for RLV to a point given in the grid's own coordinates:
    /// `position` in the region with this handle. `ctl tp` has no such
    /// form; it ends as a `tp` does, with nobody waiting for the answer.
    pub fn tp_global<G: Grid>(
        &mut self,
        handle: u64,
        position: Vector3,
        who: Who,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<(), String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let origin = Origin::Rlv(who);
        let allowed = origin.limits(grid.restrictions()).forced_teleport(grid.sitting());
        allowed.map_err(|refusal| refusal.short())?;
        grid.teleport_to(handle, position, now)?;
        self.went_manual = true;
        let [x, y, z] = position;
        let (east, north) = (handle >> 32, handle & 0xffff_ffff);
        crate::info!("rlv: teleporting to {x}, {y}, {z} in the region at {east}, {north}");
        // Nobody reads the answer; the slot says whose teleport is under way.
        let (reply, _) = std::sync::mpsc::channel();
        self.origin = origin;
        self.park_teleport(&reply);
        self.origin = Origin::Operator;
        Ok(())
    }

    fn touch<G: Grid>(&mut self, object: &str, reply: &Sender<Response>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let object = Uuid::parse_str(object.trim()).map_err(|_| format!("\"{}\" is not an object's UUID", object.trim()))?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let check = self.limits(&*grid).touch_on(&grid.thing(object));
        match check {
            Check::Allowed => {}
            Check::Refused(refusal) => return Err(self.refused(refusal)),
            // Not the session's own wait, which would touch the object as
            // soon as it is described, whatever it turns out to be.
            Check::LookFirst(why) => match grid.look_for(object, now)? {
                Look::Waiting => return Ok(self.look(object, Request::Touch(object.to_string()), why, reply)),
                Look::GaveUp => return Err(grid.unknown_object(object)),
                Look::Known => {
                    let allowed = self.limits(&*grid).touch(&grid.thing(object));
                    allowed.map_err(|refusal| self.refused(refusal))?;
                }
            },
        }
        match grid.touch(object, now)? {
            Touch::Sent => {
                crate::info!("control: touched {object}");
                Ok(Done::Answer(None))
            }
            Touch::Waiting => {
                crate::info!("control: looking for {object} to touch it");
                self.touches.push((object, reply.clone()));
                Ok(Done::Parked)
            }
        }
    }

    /// `scan`: lists the objects around the avatar once the simulator
    /// has described and named them (see `scans_due`).
    fn scan<G: Grid>(&mut self, args: &[String], reply: &Sender<Response>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let query = Query::parse(args)?;
        let distance = grid.ok_or(NOT_CONNECTED)?.reach(query.distance, now)?;
        self.scanning.push(Scanning {
            query,
            distance,
            args: args.join(" "),
            reply: reply.clone(),
            deadline: now + SCAN_LIMIT,
        });
        self.scan_poll = Some(now);
        Ok(Done::Parked)
    }

    /// Looks at the `scan` commands that wait: asks the simulator for the
    /// properties their listings lack and the name service for the owners,
    /// and answers those whose surroundings have settled and are all named,
    /// or whose time is up, with what is known.
    pub fn scans_due<G: Grid>(&mut self, grid: Option<&mut G>, now: Instant) {
        // Without a session `session_lost` has answered them.
        let (Some(grid), Some(at)) = (grid, self.scan_poll) else {
            return;
        };
        if now < at {
            return;
        }
        let sightings = grid.sightings();
        let settled = now >= grid.settled_at();
        for scan in std::mem::take(&mut self.scanning) {
            let reachable: Vec<&Sighting> = sightings.iter().filter(|s| scan.query.reachable(s, scan.distance)).collect();
            let unnamed: Vec<u32> = reachable.iter().filter(|s| s.properties.is_none()).map(|s| s.local_id).collect();
            grid.ask_properties(&unnamed, now);
            let mut nameless = 0;
            for owner in reachable.iter().filter_map(|s| scan.query.owner_wanted(s)) {
                if !self.owners.contains_key(&owner) {
                    nameless += 1;
                    if self.owner_lookups.insert(owner) {
                        self.wanted_names.push(owner);
                    }
                }
            }
            let complete = settled && unnamed.is_empty() && nameless == 0;
            if !complete && now < scan.deadline {
                self.scanning.push(scan);
                continue;
            }
            let listed: Vec<Sighting> = reachable
                .into_iter()
                .filter(|s| scan.query.matches(s, &self.owners))
                .cloned()
                .collect();
            let count = listed.len();
            let short = match complete {
                true => String::new(),
                false => format!(
                    "; the simulator did not finish in {} s: {} without properties, {nameless} without an owner's name",
                    SCAN_LIMIT.as_secs(),
                    unnamed.len()
                ),
            };
            let given = if scan.args.is_empty() {
                String::new()
            } else {
                format!(" {}", scan.args)
            };
            crate::info!("{CONTROL}: scan{given}: {count} listed within {} m{short}", scan.distance);
            // The client may have gone away; the scan is done either way.
            let _ = scan.reply.send(Response::Ok(scan::listing(listed, &self.owners)));
        }
        self.scan_poll = (!self.scanning.is_empty()).then(|| now + SCAN_POLL);
        if self.scanning.is_empty() {
            // An owner the name service did not name is asked for again
            // by the next scan.
            self.owners.retain(|_, name| name.is_some());
            if self.owners.len() > MAX_OWNER_NAMES {
                self.owners.clear();
            }
        }
    }

    /// Answers the `scan` commands that wait with an error.
    fn fail_scans(&mut self, reason: &str) {
        for scan in std::mem::take(&mut self.scanning) {
            refuse(CONTROL, "scan", &scan.reply, reason.to_string());
        }
        self.scan_poll = None;
    }

    /// The name service answered for an owner of scanned objects, with a
    /// name or without one.
    fn owner_named(&mut self, id: Uuid, name: Option<&str>) {
        if self.owner_lookups.remove(&id) {
            self.owners.insert(id, name.map(str::to_string));
        }
    }

    /// Parks the command that is being carried out until `object` has
    /// been looked for.
    fn look(&mut self, object: Uuid, request: Request, why: Option<Refusal>, reply: &Sender<Response>) -> Done {
        let tag = self.origin.tag();
        let why = why.map(|refusal| format!(": {}", refusal.short())).unwrap_or_default();
        crate::info!("{tag}: {}: looking for {object} first{why}", request.name());
        self.looking.push(Looking {
            object,
            origin: self.origin,
            request,
            reply: reply.clone(),
        });
        Done::Parked
    }

    /// The simulator has described `object`: the commands that waited for
    /// it are carried out again, by the restrictions in force now.
    pub fn described<G: Grid>(&mut self, object: Uuid, grid: Option<&mut G>, now: Instant) {
        self.look_again(object, grid, now);
    }

    /// The time to look for `object` has passed, or the region or the
    /// session is gone: the commands that waited for it are carried out
    /// again, and refused as for an object that is not known.
    pub fn not_described<G: Grid>(&mut self, object: Uuid, grid: Option<&mut G>, now: Instant) {
        self.look_again(object, grid, now);
    }

    fn look_again<G: Grid>(&mut self, object: Uuid, mut grid: Option<&mut G>, now: Instant) {
        let (found, looking) = std::mem::take(&mut self.looking)
            .into_iter()
            .partition(|looking| looking.object == object);
        self.looking = looking;
        for Looking {
            origin, request, reply, ..
        } in found
        {
            self.handle_from(origin, request, reply, grid.as_deref_mut(), now);
        }
    }

    // ----------------------------------------------------- the inventory

    /// Hands the worker a job and parks the command until it answers.
    fn start<G: Grid>(
        &mut self,
        grid: &mut G,
        what: What,
        then: Then,
        command: &'static str,
        reply: &Sender<Response>,
        deadline: Instant,
    ) -> Result<Done, String> {
        self.job_ids += 1;
        grid.inventory(self.job_ids, deadline, what)?;
        let pending = Pending {
            command,
            origin: self.origin,
            reply: reply.clone(),
            deadline,
            then,
        };
        self.jobs.insert(self.job_ids, pending);
        Ok(Done::Parked)
    }

    /// `inventory [-a] [-n] [<folder>]`.
    fn inventory<G: Grid>(
        &mut self,
        args: &[String],
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let args = inventory::parse_list_args(args)?;
        let what = What::List {
            folder: args.folder.clone(),
            recursive: args.recursive,
        };
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let allowed = self.limits(&*grid).inventory();
        allowed.map_err(|refusal| self.refused(refusal))?;
        self.start(grid, what, Then::List(args), "inventory", reply, now + inventory::LIMIT)
    }

    /// Looks the item a command names up, for `then` to go on with.
    fn find<G: Grid>(
        &mut self,
        item: &str,
        then: Then,
        command: &'static str,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let item = inventory::parse_ref(item)?;
        self.start(
            grid.ok_or(NOT_CONNECTED)?,
            What::Find(item),
            then,
            command,
            reply,
            now + inventory::LIMIT,
        )
    }

    /// `attach <item> [<body part>]`: the body part is read before the
    /// grid is asked anything.
    fn find_for_attach<G: Grid>(
        &mut self,
        item: &str,
        part: Option<&str>,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let point = match part {
            Some(part) => Some(inventory::parse_point(part).ok_or_else(|| {
                format!("\"{part}\" is not an attachment point; give a name such as `Left Hand` or `HUD Center`, or a number from 1 to 127")
            })?),
            None => None,
        };
        let then = Then::Attach {
            given: item.to_string(),
            point,
        };
        self.find(item, then, "attach", reply, grid, now)
    }

    /// `detach <what>`, or `unwear`: an item, an attachment point, or a
    /// kind of clothing.
    fn detach<G: Grid>(&mut self, what: &str, reply: &Sender<Response>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let deadline = now + inventory::LIMIT;
        match inventory::parse_removal(what)? {
            Removal::Item(item) => {
                let then = Then::Detach { given: what.to_string() };
                self.start(grid.ok_or(NOT_CONNECTED)?, What::Find(item), then, "detach", reply, deadline)
            }
            Removal::Point(point) => {
                let grid = grid.ok_or(NOT_CONNECTED)?;
                let items: Vec<Uuid> = grid
                    .attached()
                    .iter()
                    .filter(|(_, on)| *on == point)
                    .map(|(item, _)| *item)
                    .collect();
                if items.is_empty() {
                    return Err(format!("nothing is attached to {}", inventory::point_name(point)));
                }
                // The items' names, for the answer and the log.
                let then = Then::DetachPoint {
                    point,
                    items: items.clone(),
                };
                self.start(grid, What::Items(items), then, "detach", reply, deadline)
            }
            Removal::Kind(kind) if inventory::is_body_part(kind) => Err(keeps_body_part(kind)),
            Removal::Kind(kind) => {
                let grid = grid.ok_or(NOT_CONNECTED)?;
                if !grid.outfit_service() {
                    return Err(inventory::NO_OUTFIT_SERVICE.to_string());
                }
                let allowed = self.limits(&*grid).unwear(kind);
                allowed.map_err(|refusal| self.refused(refusal))?;
                crate::info!(
                    "{}: taking off every {} the avatar wears",
                    self.origin.tag(),
                    inventory::kind_name(kind)
                );
                let what = What::TakeOff(Clothes::Kind(kind));
                self.start(grid, what, Then::Outfit { put_on: false }, "detach", reply, deadline)
            }
        }
    }

    /// The inventory worker answered job `id`: the command that waited for
    /// it goes on. An answer nobody waits for any more is ignored.
    pub fn inventory_answer<G: Grid>(&mut self, id: u64, result: Result<Outcome, Failed>, grid: Option<&mut G>, now: Instant) {
        if self.restore == Restore::Reading(id) {
            self.restore_missing(result, grid, now);
            return;
        }
        if let Some(filing) = self.filing.remove(&id) {
            let sent = match (result, grid) {
                (Ok(Outcome::Folder(folder)), Some(grid)) => match &filing.request.ask {
                    Ask::Inventory {
                        to,
                        from_object,
                        transaction,
                        ..
                    } => grid.accept_inventory(*to, *from_object, *transaction, folder, now),
                    _ => Err("not an inventory offer".to_string()),
                },
                (Ok(Outcome::Folder(_)), None) => Err(NOT_CONNECTED.to_string()),
                (Ok(_), _) => Err("the inventory answered something else than a folder".to_string()),
                (Err(failed), _) => Err(no_folder(failed)),
            };
            return self.filed(filing, sent);
        }
        if let Some(what) = self.notes.remove(&id) {
            match result {
                Ok(_) => crate::debug!("Current Outfit folder: {what}"),
                Err(Failed::Reason(reason)) => crate::info!("Current Outfit folder not updated ({what}): {reason}"),
                Err(Failed::TimedOut) => crate::info!("Current Outfit folder not updated ({what}): the grid did not answer in time"),
            }
            return;
        }
        let Some(pending) = self.jobs.remove(&id) else {
            return;
        };
        let Pending {
            command,
            origin,
            reply,
            deadline,
            then,
        } = pending;
        // What follows is still that command, whoever asked for it.
        self.origin = origin;
        let done = match result {
            Err(failed) => Err(then.failure(failed)),
            Ok(outcome) => self.go_on(then, outcome, command, &reply, deadline, grid, now),
        };
        self.origin = Origin::Operator;
        finish(origin.tag(), command, done, &reply);
    }

    /// What a command does with the inventory's answer.
    #[allow(clippy::too_many_arguments)]
    fn go_on<G: Grid>(
        &mut self,
        then: Then,
        outcome: Outcome,
        command: &'static str,
        reply: &Sender<Response>,
        deadline: Instant,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        match (then, outcome) {
            (Then::List(args), Outcome::Listed(entries)) => Ok(Done::Answer(list(&args, &entries, grid))),
            (Then::Attach { point, .. }, Outcome::Found(item)) => self.attach(item, point, command, reply, grid, now),
            (Then::Wear { .. }, Outcome::Found(item)) => match item.asset_type {
                inventory::ASSET_OBJECT => self.attach(item, None, command, reply, grid, now),
                inventory::ASSET_CLOTHING | inventory::ASSET_BODY_PART => {
                    if !grid.outfit_service() {
                        return Err(inventory::NO_OUTFIT_SERVICE.to_string());
                    }
                    let replaces = item.asset_type == inventory::ASSET_BODY_PART;
                    let allowed = self.limits(&*grid).wear(item.subtype(), replaces);
                    allowed.map_err(|refusal| self.refused(refusal))?;
                    crate::info!("control: wearing {}", item.show());
                    self.start(grid, What::Wear(item), Then::Outfit { put_on: true }, command, reply, deadline)
                }
                other => Err(format!(
                    "{} is {}, which cannot be worn",
                    item.show(),
                    inventory::type_phrase(other)
                )),
            },
            (Then::Detach { .. }, Outcome::Found(item)) => match item.asset_type {
                inventory::ASSET_OBJECT => {
                    let Some((_, point)) = grid.attached().into_iter().find(|(attached, _)| *attached == item.id) else {
                        return Err(format!("{} is not attached", item.show()));
                    };
                    let allowed = self.limits(&*grid).detach(point, &grid.attachment_prims(item.id));
                    allowed.map_err(|refusal| self.refused(refusal))?;
                    grid.detach(item.id, now)?;
                    crate::info!("{}: detaching {}", self.origin.tag(), item.show());
                    self.detaching.push(Detaching {
                        origin: self.origin,
                        reply: reply.clone(),
                        waiting: vec![item],
                        gone: Vec::new(),
                        stuck: Vec::new(),
                        kept: None,
                    });
                    Ok(Done::Parked)
                }
                inventory::ASSET_BODY_PART => Err(format!("{} is a body part: {}", item.show(), keeps_body_part(item.subtype()))),
                inventory::ASSET_CLOTHING => {
                    if !grid.outfit_service() {
                        return Err(inventory::NO_OUTFIT_SERVICE.to_string());
                    }
                    let allowed = self.limits(&*grid).unwear(item.subtype());
                    allowed.map_err(|refusal| self.refused(refusal))?;
                    crate::info!("{}: taking off {}", self.origin.tag(), item.show());
                    let what = What::TakeOff(Clothes::Item(item));
                    self.start(grid, what, Then::Outfit { put_on: false }, command, reply, deadline)
                }
                other => Err(format!(
                    "{} is {}, which is neither attached nor worn",
                    item.show(),
                    inventory::type_phrase(other)
                )),
            },
            (Then::DetachPoint { point, items }, Outcome::Items(found)) => {
                // What is on the point may have changed while the grid was asked.
                let attached = grid.attached();
                let still: Vec<Uuid> = items.into_iter().filter(|item| attached.contains(&(*item, point))).collect();
                if still.is_empty() {
                    return Err(format!("nothing is attached to {}", inventory::point_name(point)));
                }
                // An item the inventory did not describe goes by its id.
                let described = |id: Uuid| {
                    let described = found.iter().find(|item| item.id == id).cloned();
                    described.unwrap_or(Item { id, ..Item::default() })
                };
                // What a restriction holds on the point stays there.
                let (mut free, mut held) = (Vec::new(), Vec::new());
                for id in still {
                    match self.limits(&*grid).detach(point, &grid.attachment_prims(id)) {
                        Ok(()) => free.push(id),
                        Err(refusal) => held.push((described(id), refusal)),
                    }
                }
                let kept = held.first().map(|(_, refusal)| {
                    let stayed: Vec<Item> = held.iter().map(|(item, _)| item.clone()).collect();
                    format!("{} stayed on: {}", names(&stayed), self.refused(refusal.clone()))
                });
                if free.is_empty() {
                    return Err(kept.unwrap_or_default());
                }
                let mut waiting = Vec::new();
                for id in free {
                    grid.detach(id, now)?;
                    waiting.push(described(id));
                }
                crate::info!(
                    "{}: detaching {} from {}",
                    self.origin.tag(),
                    names(&waiting),
                    inventory::point_name(point)
                );
                self.detaching.push(Detaching {
                    origin: self.origin,
                    reply: reply.clone(),
                    waiting,
                    gone: Vec::new(),
                    stuck: Vec::new(),
                    kept,
                });
                Ok(Done::Parked)
            }
            (Then::Anim { start, .. }, Outcome::Found(item)) => {
                let animation = asset_of(&item, inventory::ASSET_ANIMATION, "an animation")?;
                grid.animate(animation, start, now)?;
                let text = match start {
                    true => format!("playing {}", item.show()),
                    false => format!("stopped {}", item.show()),
                };
                crate::info!("control: {text}");
                Ok(Done::Answer(Some(text)))
            }
            (Then::Sound { .. }, Outcome::Found(item)) => {
                let sound = asset_of(&item, inventory::ASSET_SOUND, "a sound")?;
                grid.play_sound(sound, now)?;
                let text = format!("played {}", item.show());
                crate::info!("control: {text}");
                Ok(Done::Answer(Some(text)))
            }
            (Then::Outfit { put_on }, Outcome::Outfit(items)) => {
                let text = match put_on {
                    true => {
                        let kinds: Vec<String> = items.iter().map(|item| inventory::kind_name(item.subtype())).collect();
                        format!("wearing {} ({})", names(&items), kinds.join(", "))
                    }
                    false => format!("took off {}", names(&items)),
                };
                crate::info!("{}: {text}", self.origin.tag());
                Ok(Done::Answer(Some(text)))
            }
            (then, outcome) => Err(format!("the inventory answered {outcome:?} where {then:?} was expected")),
        }
    }

    /// Attaches an item the inventory has described: to `point`, or where
    /// the simulator puts it, which is where it was last.
    fn attach<G: Grid>(
        &mut self,
        item: Item,
        point: Option<u8>,
        command: &'static str,
        reply: &Sender<Response>,
        grid: &mut G,
        now: Instant,
    ) -> Result<Done, String> {
        if item.asset_type != inventory::ASSET_OBJECT {
            return Err(format!(
                "{} is {}, not an object",
                item.show(),
                inventory::type_phrase(item.asset_type)
            ));
        }
        if let Some((_, on)) = grid.attached().iter().find(|(attached, _)| *attached == item.id) {
            return Err(format!("{} is attached to {} already", item.show(), inventory::point_name(*on)));
        }
        // Without a point named, the object goes where it was last.
        let goes_to = point.or(Some(item.subtype()).filter(|last| *last != 0));
        let allowed = self.limits(&*grid).attach(goes_to);
        allowed.map_err(|refusal| self.refused(refusal))?;
        grid.attach(&item, point.unwrap_or(0), now)?;
        let place = point.map_or("the point it was last on".to_string(), inventory::point_name);
        crate::info!("control: attaching {} to {place}", item.show());
        // The command's answer is now what this item's arrival is for.
        self.restoring.retain(|restoring| restoring.id != item.id);
        self.gave_up.retain(|gave_up| gave_up.id != item.id);
        let (same, others) = std::mem::take(&mut self.attaching)
            .into_iter()
            .partition(|earlier| earlier.item.id == item.id);
        self.attaching = others;
        for earlier in same {
            refuse(
                CONTROL,
                earlier.command,
                &earlier.reply,
                "superseded by a newer attach of the same item".to_string(),
            );
        }
        self.attaching.push(Attaching {
            command,
            item,
            reply: reply.clone(),
            alert: None,
        });
        Ok(Done::Parked)
    }

    /// Asks for a change to the Current Outfit folder that nobody waits
    /// for: where the grid keeps that folder, viewers expect it to say
    /// what is attached.
    fn note<G: Grid>(&mut self, grid: Option<&mut G>, what: What, text: String, now: Instant) {
        let Some(grid) = grid.filter(|grid| grid.outfit_service()) else {
            return;
        };
        self.job_ids += 1;
        match grid.inventory(self.job_ids, now + inventory::LIMIT, what) {
            Ok(()) => {
                self.notes.insert(self.job_ids, text);
            }
            Err(reason) => crate::info!("Current Outfit folder not updated ({text}): {reason}"),
        }
    }

    /// The simulator shows the object of `item` on the avatar. Returns
    /// whether the bot asked for it, by a command or to restore it; the
    /// log then has its line already.
    pub fn attached<G: Grid>(&mut self, item: Uuid, point: u8, grid: Option<&mut G>, now: Instant) -> bool {
        let (done, attaching): (Vec<_>, Vec<_>) = std::mem::take(&mut self.attaching).into_iter().partition(|a| a.item.id == item);
        self.attaching = attaching;
        let Some(attach) = done.into_iter().next() else {
            let on = inventory::point_name(point);
            if let Some(index) = self.restoring.iter().position(|restoring| restoring.id == item) {
                // The link is what asked for it: nothing to add.
                let restored = self.restoring.remove(index);
                self.tell(format!("attachment: restored {} to {on}", restored.show()));
                return true;
            }
            if let Some(index) = self.gave_up.iter().position(|gave_up| gave_up.id == item) {
                let late = self.gave_up.remove(index);
                self.tell(format!("attachment: {} arrived late, on {on}", late.show()));
                let what = format!("adding {}", late.show());
                self.note(grid, What::Link(late), what, now);
                return true;
            }
            return false;
        };
        let text = format!("{} is attached to {}", attach.item.show(), inventory::point_name(point));
        crate::info!("control: {text}");
        let _ = attach.reply.send(Response::Ok(Some(text)));
        let what = format!("adding {}", attach.item.show());
        self.note(grid, What::Link(attach.item), what, now);
        true
    }

    /// The simulator did not show the object in time.
    pub fn attach_failed(&mut self, item: Uuid) {
        let (failed, attaching): (Vec<_>, Vec<_>) = std::mem::take(&mut self.attaching).into_iter().partition(|a| a.item.id == item);
        self.attaching = attaching;
        if let Some(index) = self.restoring.iter().position(|restoring| restoring.id == item) {
            let lost = self.restoring.remove(index);
            let limit = ATTACH_LIMIT.as_secs();
            self.tell(format!(
                "attachment: the simulator did not restore {} within {limit} s",
                lost.show()
            ));
        }
        if let Some(attach) = failed.first() {
            // It may still come: its link is owed then.
            self.gave_up.retain(|gave_up| gave_up.id != item);
            if self.gave_up.len() == MAX_GAVE_UP {
                self.gave_up.remove(0);
            }
            self.gave_up.push(attach.item.clone());
        }
        for attach in failed {
            let mut reason = format!(
                "the simulator did not attach {} within {} s",
                attach.item.show(),
                ATTACH_LIMIT.as_secs()
            );
            if let Some(alert) = attach.alert {
                reason.push_str(&format!("; it said: {alert}"));
            }
            refuse(CONTROL, attach.command, &attach.reply, reason);
        }
    }

    /// The simulator sent text for the user to read: it may be why an
    /// attach that is waiting will not happen.
    pub fn alert(&mut self, message: &str) {
        for attach in &mut self.attaching {
            attach.alert = Some(message.to_string());
        }
    }

    /// The object of `item` is gone from the avatar. Returns whether a
    /// command of the bot's took it off.
    pub fn detached<G: Grid>(&mut self, item: Uuid, grid: Option<&mut G>, now: Instant) -> bool {
        let mut ours = None;
        for detach in &mut self.detaching {
            if let Some(index) = detach.waiting.iter().position(|waiting| waiting.id == item) {
                let gone = detach.waiting.remove(index);
                crate::info!("{}: {} is detached", detach.origin.tag(), gone.show());
                ours = Some(gone.show());
                detach.gone.push(gone);
            }
        }
        self.settle_detaches();
        match ours {
            Some(shown) => {
                self.note(grid, What::Unlink(item), format!("removing {shown}"), now);
                true
            }
            None => false,
        }
    }

    /// The simulator did not take the object off in time.
    pub fn detach_failed(&mut self, item: Uuid) {
        for detach in &mut self.detaching {
            if let Some(index) = detach.waiting.iter().position(|waiting| waiting.id == item) {
                detach.stuck.push(detach.waiting.remove(index));
            }
        }
        self.settle_detaches();
    }

    /// Answers the `detach` commands that have nothing left to wait for.
    fn settle_detaches(&mut self) {
        let (settled, detaching): (Vec<_>, Vec<_>) = std::mem::take(&mut self.detaching).into_iter().partition(|d| d.waiting.is_empty());
        self.detaching = detaching;
        for detach in settled {
            // What a restriction kept on is the operator's to know; a
            // force command has done what it could.
            let kept = detach.kept.filter(|_| detach.origin.is_operator());
            if detach.stuck.is_empty() {
                match kept {
                    Some(kept) => {
                        let reason = format!("detached {}; {kept}", names(&detach.gone));
                        refuse(detach.origin.tag(), "detach", &detach.reply, reason);
                    }
                    None => {
                        let _ = detach.reply.send(Response::Ok(Some(format!("detached {}", names(&detach.gone)))));
                    }
                }
                continue;
            }
            let mut reason = format!(
                "the simulator did not detach {} within {} s",
                names(&detach.stuck),
                DETACH_LIMIT.as_secs()
            );
            if !detach.gone.is_empty() {
                reason.push_str(&format!("; detached {}", names(&detach.gone)));
            }
            if let Some(kept) = kept {
                reason.push_str(&format!("; {kept}"));
            }
            refuse(detach.origin.tag(), "detach", &detach.reply, reason);
        }
    }

    // ------------------------------------- restoring attachments after login

    /// The avatar has entered a region. Until the Current Outfit folder
    /// has been compared with the avatar once since the login, that is
    /// due `RESTORE_DELAY` from now: the new simulator has yet to show
    /// what is attached, also when the folder is being read already.
    pub fn entered_region(&mut self, now: Instant) {
        if self.restore_on && self.restore != Restore::Done {
            self.restore = Restore::Due(now + RESTORE_DELAY);
        }
    }

    /// Asks for the Current Outfit folder when the time has come. For the
    /// main loop, beside `tick`.
    pub fn restore_due<G: Grid>(&mut self, grid: Option<&mut G>, now: Instant) {
        let Restore::Due(at) = self.restore else {
            return;
        };
        if now < at {
            return;
        }
        let Some(grid) = grid.filter(|grid| grid.ready()) else {
            // Between regions: the next region entry sets the time anew.
            self.restore = Restore::Due(now + RESTORE_RETRY);
            return;
        };
        self.restore = Restore::Done;
        if !grid.outfit_service() {
            // The folder is not kept here, so it says nothing.
            return;
        }
        self.job_ids += 1;
        match grid.inventory(self.job_ids, now + inventory::LIMIT, What::OutfitObjects) {
            Ok(()) => self.restore = Restore::Reading(self.job_ids),
            Err(reason) => self.tell(format!("attachments not restored: {reason}")),
        }
    }

    /// The Current Outfit folder's objects are known: asks the simulator
    /// for those that are not on the avatar.
    fn restore_missing<G: Grid>(&mut self, result: Result<Outcome, Failed>, grid: Option<&mut G>, now: Instant) {
        self.restore = Restore::Done;
        let listed = match result {
            Ok(Outcome::Items(listed)) => listed,
            Ok(_) => return,
            Err(Failed::Reason(reason)) => return self.tell(format!("attachments not restored: {reason}")),
            Err(Failed::TimedOut) => return self.tell("attachments not restored: the grid did not answer in time".to_string()),
        };
        let Some(grid) = grid.filter(|grid| grid.ready()) else {
            // Left the region meanwhile: what is attached is not known.
            self.restore = Restore::Due(now + RESTORE_RETRY);
            return;
        };
        let attached = grid.attached();
        let missing: Vec<Item> = listed
            .iter()
            .filter(|item| !attached.iter().any(|(on, _)| *on == item.id))
            .filter(|item| !self.attaching.iter().any(|attach| attach.item.id == item.id))
            .cloned()
            .collect();
        if missing.is_empty() {
            crate::debug!(
                "attachments: all {} objects of the Current Outfit folder are on the avatar",
                listed.len()
            );
            return;
        }
        for item in missing {
            // What an `attach` would be refused for is left off.
            let goes_to = Some(item.subtype()).filter(|last| *last != 0);
            if let Err(refusal) = Origin::Bot.limits(grid.restrictions()).attach(goes_to) {
                self.tell(format!("attachment: {} not restored: {}", item.show(), refusal.short()));
                continue;
            }
            match grid.attach(&item, 0, now) {
                Ok(()) => {
                    self.tell(format!(
                        "attachment: restoring {}, listed in the Current Outfit folder and not on the avatar",
                        item.show()
                    ));
                    self.gave_up.retain(|gave_up| gave_up.id != item.id);
                    self.restoring.push(item);
                }
                Err(reason) => self.tell(format!("attachment: {} not restored: {reason}", item.show())),
            }
        }
    }

    /// The avatar is on another simulator: what was asked of the old
    /// one's inventory service will not be used.
    pub fn simulator_changed(&mut self) {
        self.fail_jobs("the avatar changed simulator while the inventory was being read");
        // The objects a scan waited for stayed behind too.
        self.fail_scans("the avatar left the region");
        // The scripts that asked for permissions stayed behind.
        self.requests.clear_scripts();
        self.undecided.retain(|waiting| !matches!(waiting.needs, Needs::Owner(_)));
    }

    fn fail_jobs(&mut self, reason: &str) {
        for (_, pending) in std::mem::take(&mut self.jobs) {
            refuse(pending.origin.tag(), pending.command, &pending.reply, reason.to_string());
        }
        self.notes.clear();
        // An inventory offer on its way to being accepted waits again.
        for (_, filing) in std::mem::take(&mut self.filing) {
            self.filed(filing, Err(reason.to_string()));
        }
    }

    /// Does `then` with the avatar `to` names if it is known, and otherwise
    /// starts (or joins) a lookup and parks the command.
    fn with_avatar<G: Grid>(
        &mut self,
        to: &str,
        then: After,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let recipient = parse_recipient(to)?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let name = match recipient {
            Recipient::Id(id) => return self.act(grid, id, String::new(), then, now),
            Recipient::Name(name) => name,
        };
        if let Some((id, proper)) = self.known.get(&name).cloned() {
            return self.act(grid, id, proper, then, now);
        }
        let query = match self.waiting.iter().find(|w| w.name == name) {
            Some(same) => same.query,
            None => {
                self.queries += 1;
                let query = Uuid::from_u128(self.queries);
                grid.find_avatar(query, search_text(&name), now)?;
                query
            }
        };
        let deadline = now + LOOKUP_LIMIT;
        let given = to.trim().to_string();
        self.waiting.push(Waiting {
            query,
            name,
            given,
            then,
            origin: self.origin,
            reply: reply.clone(),
            deadline,
        });
        Ok(Done::Parked)
    }

    /// What a command does once its avatar is known to be `id`.
    fn act<G: Grid>(&mut self, grid: &mut G, id: Uuid, name: String, then: After, now: Instant) -> Result<Done, String> {
        let who = whom(id, &name);
        match then {
            After::Im(text) => {
                let allowed = self.limits(&*grid).im(id, self.talking.contains(&id));
                allowed.map_err(|refusal| self.refused(refusal))?;
                grid.instant_message(id, &text, now)?;
                self.talking.insert(id);
                Ok(Done::Sent(format!("sent to {who}"), SentIm { to: id, name, text }))
            }
            After::RequestTp(message) => {
                grid.request_teleport(id, &message, now)?;
                self.asked.insert(id, (now, self.origin == Origin::Exempt));
                self.went_manual = true;
                crate::info!("control: asked {who} for a teleport");
                Ok(Done::Answer(Some(format!("asked {who} for a teleport"))))
            }
            After::SendTp(message) => {
                let message = match message.is_empty() {
                    true => format!("Join me in {}", grid.region().ok_or(NOT_CONNECTED)?),
                    false => message,
                };
                grid.offer_teleport(id, &message, now)?;
                crate::info!("control: offered {who} a teleport");
                Ok(Done::Answer(Some(format!("offered {who} a teleport"))))
            }
        }
    }

    /// The grid answered a lookup: the commands that waited for it go on.
    /// Returns the instant messages sent.
    pub fn avatars_found<G: Grid>(
        &mut self,
        query: Uuid,
        avatars: &[(Uuid, String)],
        mut grid: Option<&mut G>,
        now: Instant,
    ) -> Vec<SentIm> {
        let mut sent = Vec::new();
        let (answered, waiting) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.query == query);
        self.waiting = waiting;
        for command in answered {
            let Waiting {
                name,
                given,
                then,
                origin,
                reply,
                ..
            } = command;
            let what = then.command();
            // What follows is still that command, exempt or not.
            self.origin = origin;
            let done = match (exact_match(&name, avatars), grid.as_deref_mut()) {
                (None, _) => Err(format!("the grid knows no avatar named \"{given}\"")),
                (Some(_), None) => Err(NOT_CONNECTED.to_string()),
                (Some((id, proper)), Some(grid)) => {
                    self.known.insert(name, (*id, proper.clone()));
                    self.act(grid, *id, proper.clone(), then, now)
                }
            };
            self.origin = Origin::Operator;
            sent.extend(finish(CONTROL, what, done, &reply));
        }
        sent
    }

    /// A lookup came to nothing: fails the commands that waited for it.
    pub fn lookup_failed(&mut self, query: Uuid, reason: &str) {
        let (failed, waiting) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.query == query);
        self.waiting = waiting;
        for command in failed {
            let Waiting { given, then, reply, .. } = command;
            refuse(
                CONTROL,
                then.command(),
                &reply,
                format!("the lookup of \"{given}\" failed: {reason}"),
            );
        }
    }

    /// Fails the commands whose lookup has gone unanswered for too long.
    pub fn tick(&mut self, now: Instant) {
        let (late, waiting) = std::mem::take(&mut self.waiting).into_iter().partition(|w| now >= w.deadline);
        self.waiting = waiting;
        for command in late {
            let Waiting { given, then, reply, .. } = command;
            let limit = LOOKUP_LIMIT.as_secs();
            let reason = format!("the grid did not answer the lookup of \"{given}\" within {limit} s");
            refuse(CONTROL, then.command(), &reply, reason);
        }
        // The worker answers by the deadline; this is for one that does not.
        let late: Vec<u64> = self
            .jobs
            .iter()
            .filter(|(_, job)| now >= job.deadline + JOB_GRACE)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            if let Some(job) = self.jobs.remove(&id) {
                refuse(job.origin.tag(), job.command, &job.reply, job.then.failure(Failed::TimedOut));
            }
        }
        let late: Vec<u64> = self
            .filing
            .iter()
            .filter(|(_, filing)| now >= filing.deadline + JOB_GRACE)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            if let Some(filing) = self.filing.remove(&id) {
                self.filed(filing, Err(no_folder(Failed::TimedOut)));
            }
        }
    }

    /// The session is gone, and with it any answer the commands waited for
    /// and every teleport offer made to it.
    pub fn session_lost(&mut self) {
        for command in std::mem::take(&mut self.waiting) {
            refuse(CONTROL, command.then.command(), &command.reply, NOT_CONNECTED.to_string());
        }
        if let Some((reply, origin)) = self.pending_sit.take() {
            refuse(origin.tag(), "sit", &reply, "the session was lost".to_string());
        }
        if let Some((reply, origin)) = self.pending_teleport.take() {
            refuse(origin.tag(), "tp", &reply, "the session was lost".to_string());
        }
        for (_, reply) in std::mem::take(&mut self.touches) {
            refuse(CONTROL, "touch", &reply, "the session was lost".to_string());
        }
        for looking in std::mem::take(&mut self.looking) {
            refuse(
                looking.origin.tag(),
                looking.request.name(),
                &looking.reply,
                "the session was lost".to_string(),
            );
        }
        self.fail_jobs("the session was lost");
        self.fail_scans("the session was lost");
        self.owner_lookups.clear();
        for attach in std::mem::take(&mut self.attaching) {
            refuse(CONTROL, attach.command, &attach.reply, "the session was lost".to_string());
        }
        for detach in std::mem::take(&mut self.detaching) {
            refuse(detach.origin.tag(), "detach", &detach.reply, "the session was lost".to_string());
        }
        // The next login compares anew, and owes this one's attaches nothing.
        self.restore = Restore::Idle;
        self.restoring.clear();
        self.gave_up.clear();
        // Nothing that waited can be answered through another session.
        for (_, filing) in std::mem::take(&mut self.filing) {
            if let Some(reply) = filing.reply {
                refuse(CONTROL, "accept", &reply, "the session was lost".to_string());
            }
        }
        self.requests.clear();
        self.undecided.clear();
        self.wanted_names.clear();
        // The conversations were this session's.
        self.talking.clear();
    }

    /// When `tick` next has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        let lookups = self.waiting.iter().map(|w| w.deadline);
        let jobs = self.jobs.values().map(|job| job.deadline + JOB_GRACE);
        let filing = self.filing.values().map(|filing| filing.deadline + JOB_GRACE);
        let undecided = self.undecided.iter().map(|waiting| waiting.deadline);
        let restore = match self.restore {
            Restore::Due(at) => Some(at),
            _ => None,
        };
        let scans = self.scan_poll;
        lookups.chain(jobs).chain(filing).chain(undecided).chain(restore).chain(scans).min()
    }

    // ------------------------------------------- what the session reports

    /// The avatar is seated on `object`.
    pub fn seated(&mut self, object: Uuid) {
        if let Some((reply, _)) = self.pending_sit.take() {
            let _ = reply.send(Response::Ok(Some(format!("seated on {object}"))));
        }
    }

    /// A sit came to nothing. Returns whether a command was waiting for it.
    pub fn sit_failed(&mut self, reason: &str) -> bool {
        match self.pending_sit.take() {
            Some((reply, origin)) => {
                refuse(origin.tag(), "sit", &reply, reason.to_string());
                true
            }
            None => false,
        }
    }

    /// A teleport ended with the avatar in `region`.
    pub fn arrived(&mut self, region: &str) {
        if let Some((reply, _)) = self.pending_teleport.take() {
            let _ = reply.send(Response::Ok(Some(format!("arrived in {region}"))));
        }
    }

    /// A teleport came to nothing. Returns whether a command was waiting
    /// for it.
    pub fn teleport_failed(&mut self, reason: &str) -> bool {
        match self.pending_teleport.take() {
            Some((reply, origin)) => {
                refuse(origin.tag(), "tp", &reply, reason.to_string());
                true
            }
            None => false,
        }
    }

    /// A touch that waited for its object went out.
    pub fn touched(&mut self, object: Uuid) {
        let (done, touches) = std::mem::take(&mut self.touches)
            .into_iter()
            .partition(|(wanted, _)| *wanted == object);
        self.touches = touches;
        for (_, reply) in done {
            crate::info!("control: touched {object}");
            let _ = reply.send(Response::Ok(None));
        }
    }

    /// A touch gave up waiting for its object.
    pub fn touch_failed(&mut self, object: Uuid, reason: &str) {
        let (failed, touches) = std::mem::take(&mut self.touches)
            .into_iter()
            .partition(|(wanted, _)| *wanted == object);
        self.touches = touches;
        for (_, reply) in failed {
            refuse(CONTROL, "touch", &reply, reason.to_string());
        }
    }

    // ------------------------------------------- requests and their rules

    /// The auto-accept rules the bot starts with: the config's.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// `perms autoaccept`, `perms revoke` and `perms status`. None of them
    /// needs the grid.
    fn perms(&mut self, request: PermsRequest) -> Result<Done, String> {
        let (kind, who, add) = match request {
            PermsRequest::Status => {
                let mut lines = vec![self.rules.status()];
                lines.extend(self.requests.status());
                return Ok(Done::Answer(Some(lines.join("\n"))));
            }
            PermsRequest::Autoaccept { kind, who } => (kind, who, true),
            PermsRequest::Revoke { kind, who } => (kind, who, false),
        };
        let kind = parse_kind(&kind)?;
        let is_all = |who: &String| who.trim().eq_ignore_ascii_case("all");
        let changes = match &who[..] {
            [] => return Err("say whom: `all`, or avatars' names and UUIDs".to_string()),
            [all] if is_all(all) => vec![match add {
                true => self.rules.autoaccept_all(kind),
                false => self.rules.revoke_all(kind),
            }],
            who if who.iter().any(is_all) => {
                return Err("`all` stands alone: it cannot be given together with a name or a UUID".to_string())
            }
            who => {
                // All of them are read before any of them changes a rule.
                let entries = who.iter().map(|who| perms::Entry::parse(who)).collect::<Result<Vec<_>, _>>()?;
                let change = |entry| match add {
                    true => self.rules.autoaccept(kind, entry),
                    false => self.rules.revoke(kind, entry),
                };
                entries.into_iter().map(change).collect()
            }
        };
        let lines: Vec<String> = changes.into_iter().map(|change| change.line).collect();
        for line in &lines {
            self.tell(format!("control: perms: {line}"));
        }
        Ok(Done::Answer(Some(lines.join("\n"))))
    }

    /// `accept KIND [WHO]`: accepts the request of that kind that came
    /// last, among those of `who` when one is named.
    fn accept_command<G: Grid>(
        &mut self,
        kind: &str,
        who: Option<&str>,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let kind = parse_kind(kind)?;
        let who = who.map(perms::Entry::parse).transpose()?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let Some(request) = self.requests.take(kind, who.as_ref()) else {
            return Err(match (kind, &who) {
                (Kind::Tp, None) => "no teleport offer waits".to_string(),
                (Kind::Tp, Some(who)) => format!("there is no teleport offer from {who}"),
                (_, None) => format!("no request of kind `{kind}` waits"),
                (_, Some(who)) => format!("no request of kind `{kind}` from {who} waits"),
            });
        };
        // A restriction leaves the request waiting, as any other reason does.
        let barred = Commands::barred(&request, self.limits(&*grid), grid.sitting());
        let done = match barred {
            Some(refusal) => Err(self.refused(refusal)),
            None => self.accept(&request, Some(reply), grid, now),
        };
        match &done {
            // It keeps its place among the waiting ones.
            Err(_) => {
                self.requests.push(request);
            }
            // Said when the answer has gone out.
            Ok(Done::Parked) if kind == Kind::Inventory => {}
            Ok(_) => self.tell(format!("control: accept {kind}: {}: accepted", request.describe())),
        }
        done
    }

    /// The restriction that keeps `request` from being accepted, if one
    /// does: `tplure` and a seated `unsit` for a teleport offer,
    /// `denypermission` for a script that asks to attach or to take
    /// controls.
    fn barred(request: &perms::Request, limits: Limits, sitting: Sitting) -> Option<Refusal> {
        match (&request.ask, &request.sender) {
            (Ask::Tp { .. }, perms::Sender::Avatar { id, .. }) => limits.lure(*id).err().or(limits.stand(sitting).err()),
            (Ask::Tp { .. }, _) => limits.stand(sitting).err(),
            (Ask::Script { .. }, _) if request.kinds().iter().any(|kind| matches!(kind, Kind::Attach | Kind::Controls)) => {
                limits.denies_permissions()
            }
            _ => None,
        }
    }

    /// Does what a viewer does when its user accepts the dialog. A
    /// teleport offer and an inventory offer end later: the command that
    /// accepts one is parked.
    fn accept<G: Grid>(
        &mut self,
        request: &perms::Request,
        reply: Option<&Sender<Response>>,
        grid: &mut G,
        now: Instant,
    ) -> Result<Done, String> {
        match &request.ask {
            Ask::Tp { lure } => {
                grid.accept_teleport(*lure, now)?;
                self.went_manual = true;
                match reply {
                    Some(reply) => {
                        self.park_teleport(reply);
                        Ok(Done::Parked)
                    }
                    None => Ok(Done::Answer(None)),
                }
            }
            Ask::Script { object, item, mask } => {
                grid.grant_permissions(*object, *item, *mask, now)?;
                let granted = perms::permission_names(*mask);
                Ok(Done::Answer(Some(format!("granted {}: {granted}", request.sender))))
            }
            Ask::Inventory { asset_type, .. } => {
                self.job_ids += 1;
                let deadline = now + inventory::LIMIT;
                grid.inventory(self.job_ids, deadline, What::SystemFolder(*asset_type))?;
                let filing = Filing {
                    request: request.clone(),
                    reply: reply.cloned(),
                    deadline,
                };
                self.filing.insert(self.job_ids, filing);
                Ok(Done::Parked)
            }
        }
    }

    /// An inventory offer's answer went out, or could not: the command
    /// that accepted it is answered, and an offer that could not be
    /// accepted waits again.
    fn filed(&mut self, filing: Filing, sent: Result<(), String>) {
        let Filing { request, reply, .. } = filing;
        let what = request.describe();
        match (sent, reply) {
            (Ok(()), Some(reply)) => {
                self.tell(format!("control: accept inventory: {what}: accepted"));
                let offered = request.offered().unwrap_or_default();
                let _ = reply.send(Response::Ok(Some(format!("accepted {offered} from {}", request.sender))));
            }
            (Ok(()), None) => self.tell(format!("{what}: accepted by itself")),
            (Err(reason), Some(reply)) => {
                self.requests.push(request);
                refuse(CONTROL, "accept", &reply, reason);
            }
            (Err(reason), None) => self.wait(request, &format!("cannot be accepted now: {reason}")),
        }
    }

    /// Accepts a request if the rules say so, and otherwise leaves it to
    /// the operator.
    fn decide<G: Grid>(&mut self, request: perms::Request, grid: Option<&mut G>, now: Instant) {
        let mut kinds = request.kinds();
        // RLV comes before the rules: it may drop the request, hold the
        // avatar on its seat, or grant what the rules would not.
        let (barred, granted) = match grid.as_deref() {
            Some(grid) => {
                let limits = Origin::Bot.limits(grid.restrictions());
                (Commands::barred(&request, limits, grid.sitting()), limits.grants_permissions())
            }
            None => (None, false),
        };
        let script = matches!(request.ask, Ask::Script { .. });
        if let Some(refusal) = barred.as_ref().filter(|_| script) {
            return self.tell(format!("{}: dropped: {}", request.describe(), refusal.short()));
        }
        if granted && script {
            kinds.retain(|kind| !matches!(kind, Kind::Attach | Kind::Controls));
        }
        if let Err(kind) = self.rules.decide(&kinds, &request.sender) {
            let why = match kinds.len() {
                1 => "waits".to_string(),
                _ => format!("waits, as {kind} is not auto-accepted for it"),
            };
            return self.wait(request, &why);
        }
        let done = match (grid, barred) {
            (_, Some(refusal)) => Err(refusal.short()),
            (Some(grid), None) => self.accept(&request, None, grid, now),
            (None, None) => Err(NOT_CONNECTED.to_string()),
        };
        match done {
            // Said when the answer has gone out.
            Ok(Done::Parked) => {}
            Ok(_) => self.tell(format!("{}: accepted by itself", request.describe())),
            Err(reason) => self.wait(request, &format!("cannot be accepted now: {reason}")),
        }
    }

    /// Keeps a request for `accept`, and says so.
    fn wait(&mut self, request: perms::Request, why: &str) {
        self.tell(format!("{}: {why}; `{}` accepts it", request.describe(), request.accept_command()));
        if let Some(oldest) = self.requests.push(request) {
            self.tell(format!("{}: forgotten, too many requests wait", oldest.describe()));
        }
    }

    /// An avatar offers the bot a teleport. One the bot asked for recently
    /// is accepted; any other is for the rules to decide.
    pub fn teleport_offer<G: Grid>(&mut self, from: Uuid, name: &str, lure: Uuid, grid: Option<&mut G>, now: Instant) {
        let full_name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if !full_name.is_empty() {
            // The offer says who it is from: no lookup needed to answer it.
            self.known.insert(full_name.to_lowercase(), (from, full_name.clone()));
        }
        let sender = perms::Sender::Avatar { id: from, name: full_name };
        let request = perms::Request::new(sender, Ask::Tp { lure });
        let asked = self
            .asked
            .remove(&from)
            .filter(|(at, _)| now.duration_since(*at) <= self.offer_timeout);
        // The offer that answers an exempt `requesttp` is exempt with it.
        let origin = match asked {
            Some((_, true)) => Origin::Exempt,
            _ => Origin::Bot,
        };
        let (dropped, seated, wanted) = match grid.as_deref() {
            Some(grid) => {
                let limits = origin.limits(grid.restrictions());
                (limits.lure(from).err(), limits.stand(grid.sitting()).err(), limits.accepts_tp(from))
            }
            None => (None, None, false),
        };
        if let Some(refusal) = dropped {
            return self.tell(format!("{}: dropped: {}", request.describe(), refusal.short()));
        }
        if asked.is_none() && !wanted {
            return self.decide(request, grid, now);
        }
        let accepted = match (seated, grid) {
            (Some(refusal), _) => Err(refusal.short()),
            (None, None) => Err(NOT_CONNECTED.to_string()),
            (None, Some(grid)) => grid.accept_teleport(lure, now),
        };
        match (accepted, asked) {
            (Ok(()), Some(_)) => self.tell(format!("{}: accepted, as requested", request.describe())),
            (Ok(()), None) => {
                // Nobody asked for this one: the bot stops holding its place.
                self.went_manual = true;
                self.tell(format!("{}: accepted for RLV, which holds `accepttp`", request.describe()));
            }
            (Err(reason), _) => self.wait(request, &format!("cannot be accepted now: {reason}")),
        }
    }

    /// An avatar asks to be teleported to the bot's avatar. RLV may drop
    /// the request, or have the bot answer it with an offer; otherwise it
    /// is the operator's to answer.
    pub fn teleport_requested<G: Grid>(&mut self, from: Uuid, name: &str, text: &str, grid: Option<&mut G>, now: Instant) {
        let who = whom(from, name.trim());
        let Some(grid) = grid else {
            return self.tell(format!("{who} asks to be teleported here ({text}); `rslbot ctl sendtp` offers it"));
        };
        let limits = Origin::Bot.limits(grid.restrictions());
        let (dropped, answers) = (limits.tp_request(from).err(), limits.answers_tp_request(from));
        if let Some(refusal) = dropped {
            return self.tell(format!("request of {who} to be teleported here: dropped: {}", refusal.short()));
        }
        if !answers {
            return self.tell(format!("{who} asks to be teleported here ({text}); `rslbot ctl sendtp` offers it"));
        }
        let offered = grid.region().ok_or(NOT_CONNECTED.to_string()).and_then(|region| {
            let message = format!("Join me in {region}");
            grid.offer_teleport(from, &message, now)
        });
        match offered {
            Ok(()) => self.tell(format!(
                "offered {who} a teleport, as asked, for RLV, which holds `accepttprequest`"
            )),
            Err(reason) => self.tell(format!("{who} asks to be teleported here; no offer could be sent: {reason}")),
        }
    }

    /// An avatar or an object offers the avatar an inventory item.
    pub fn inventory_offer<G: Grid>(&mut self, offer: InventoryOffer, grid: Option<&mut G>, now: Instant) {
        let name = offer.name.trim().to_string();
        let sender = match (offer.from_object, offer.from_group) {
            (false, _) => perms::Sender::Avatar { id: offer.from, name },
            (true, true) => perms::Sender::Object {
                id: None,
                name,
                owner: Owner::Group,
            },
            (true, false) => perms::Sender::Object {
                id: None,
                name,
                owner: Owner::Avatar {
                    id: Some(offer.from),
                    name: None,
                },
            },
        };
        let ask = Ask::Inventory {
            to: offer.from,
            from_object: offer.from_object,
            transaction: offer.transaction,
            asset_type: offer.asset_type,
            text: offer.text.trim().to_string(),
        };
        let request = perms::Request::new(sender, ask);
        // An object's offer names its owner by UUID only. The name is
        // looked up first: a rule may go by it, and so may the operator
        // who accepts an offer that waits.
        if offer.from_object && !offer.from_group {
            self.wanted_names.push(offer.from);
            self.undecided.push(Undecided {
                request,
                needs: Needs::Name(offer.from),
                deadline: now + LOOKUP_LIMIT,
            });
        } else {
            self.decide(request, grid, now);
        }
    }

    /// A script asks the avatar for permissions. The request names its
    /// owner as text only, which may as well be a group's name: before
    /// the rules decide, the simulator is asked who owns the object.
    #[allow(clippy::too_many_arguments)]
    pub fn permission_request<G: Grid>(
        &mut self,
        object: Uuid,
        item: Uuid,
        object_name: &str,
        owner_name: &str,
        permissions: i32,
        grid: Option<&mut G>,
        now: Instant,
    ) {
        let sender = perms::Sender::Object {
            id: Some(object),
            name: object_name.trim().to_string(),
            owner: Owner::Unconfirmed(owner_name.trim().to_string()),
        };
        // What the script asked before is closed by its asking again.
        self.requests.forget_script(object, item);
        self.undecided
            .retain(|waiting| !matches!(waiting.request.ask, Ask::Script { object: o, item: i, .. } if o == object && i == item));
        if permissions == 0 {
            return crate::debug!("permission request from {sender} asks for nothing");
        }
        if let Err(what) = perms::script_kinds(permissions) {
            return self.tell(format!(
                "permission request from {sender} cannot be accepted: it asks for {what}, which the bot never grants"
            ));
        }
        let ask = Ask::Script {
            object,
            item,
            mask: permissions,
        };
        let request = perms::Request::new(sender, ask);
        let Some(grid) = grid else {
            return self.decide(request, None::<&mut G>, now);
        };
        match grid.object_owner(object, now) {
            Ok(()) => self.undecided.push(Undecided {
                request,
                needs: Needs::Owner(object),
                deadline: now + LOOKUP_LIMIT,
            }),
            Err(_) => self.decide(request, Some(grid), now),
        }
    }

    /// Decides the requests that waited for `needs`, after `learn` has
    /// put into each what the grid said.
    fn settle<G: Grid>(&mut self, needs: &Needs, learn: impl Fn(&mut perms::Sender), mut grid: Option<&mut G>, now: Instant) {
        let (ready, undecided) = std::mem::take(&mut self.undecided)
            .into_iter()
            .partition(|waiting| waiting.needs == *needs);
        self.undecided = undecided;
        for mut waiting in ready {
            learn(&mut waiting.request.sender);
            self.decide(waiting.request, grid.as_deref_mut(), now);
        }
    }

    /// The simulator said who owns `object`: an avatar, or (no owner, or
    /// the group itself as the owner) a group.
    pub fn object_owner<G: Grid>(&mut self, object: Uuid, owner: Uuid, group: Uuid, grid: Option<&mut G>, now: Instant) {
        let learn = |sender: &mut perms::Sender| {
            let perms::Sender::Object { owner: slot, .. } = sender else {
                return;
            };
            let claimed = match slot {
                Owner::Unconfirmed(name) if !name.is_empty() => Some(name.clone()),
                _ => None,
            };
            *slot = match owner.is_nil() || owner == group {
                true => Owner::Group,
                false => Owner::Avatar {
                    id: Some(owner),
                    name: claimed,
                },
            };
        };
        self.settle(&Needs::Owner(object), learn, grid, now);
    }

    /// The avatars whose names requests wait for, to be looked up; each is
    /// handed out once.
    pub fn take_name_lookups(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.wanted_names)
    }

    /// The grid answered a name lookup: the requests that waited for one
    /// of these names are decided, those of avatars it does not know
    /// without a name.
    pub fn names_resolved<G: Grid>(&mut self, found: &[(Uuid, Names)], missing: &[Uuid], mut grid: Option<&mut G>, now: Instant) {
        for (id, names) in found {
            self.owner_named(*id, Some(&names.legacy));
            let learn = |sender: &mut perms::Sender| {
                if let perms::Sender::Object {
                    owner: Owner::Avatar { name, .. },
                    ..
                } = sender
                {
                    *name = Some(names.legacy.clone());
                }
            };
            self.settle(&Needs::Name(*id), learn, grid.as_deref_mut(), now);
        }
        self.names_failed(missing, grid, now);
    }

    /// A name lookup came to nothing: the requests that waited for it are
    /// decided without the name.
    pub fn names_failed<G: Grid>(&mut self, ids: &[Uuid], mut grid: Option<&mut G>, now: Instant) {
        for id in ids {
            self.owner_named(*id, None);
            self.settle(&Needs::Name(*id), |_| {}, grid.as_deref_mut(), now);
        }
    }

    /// Decides the requests the grid has not said in time who is behind,
    /// with what is known.
    pub fn requests_due<G: Grid>(&mut self, mut grid: Option<&mut G>, now: Instant) {
        let (late, undecided) = std::mem::take(&mut self.undecided)
            .into_iter()
            .partition(|waiting| now >= waiting.deadline);
        self.undecided = undecided;
        for waiting in late {
            self.decide(waiting.request, grid.as_deref_mut(), now);
        }
    }
}

/// Why an inventory offer's folder was not found.
fn no_folder(failed: Failed) -> String {
    match failed {
        Failed::Reason(reason) => reason,
        Failed::TimedOut => format!(
            "the grid did not say within {} s which folder the offer goes into",
            inventory::LIMIT.as_secs()
        ),
    }
}

/// The answer to `inventory`: the listing, or nothing when it is empty.
fn list<G: Grid>(args: &ListArgs, entries: &[Entry], grid: &G) -> Option<String> {
    let attached: HashMap<Uuid, u8> = grid.attached().into_iter().collect();
    let (text, count) = inventory::listing(entries, &attached, args.attached);
    let mut filters = String::new();
    if args.attached {
        filters.push_str(", attached only");
    }
    if !args.recursive {
        filters.push_str(", direct children only");
    }
    let entries = if count == 1 { "entry" } else { "entries" };
    crate::info!("control: inventory: listed {count} {entries} under {}{filters}", args.folder.show());
    (count > 0).then_some(text)
}

/// The asset an `anim` or `sound` command plays: the item has to be of
/// the right type, and the inventory has to give the asset's id.
fn asset_of(item: &Item, asset_type: i32, wanted: &str) -> Result<Uuid, String> {
    if item.asset_type != asset_type {
        return Err(format!(
            "{} is {}, not {wanted}",
            item.show(),
            inventory::type_phrase(item.asset_type)
        ));
    }
    if item.asset_id.is_nil() {
        let what = wanted.rsplit(' ').next().unwrap_or(wanted);
        return Err(format!("the inventory does not give the bot the {what} itself for {}", item.show()));
    }
    Ok(item.asset_id)
}

/// Why a body part is not taken off.
fn keeps_body_part(kind: u8) -> String {
    let kind = inventory::kind_name(kind);
    format!("the avatar cannot be without {kind}; {kind} can only be replaced, with `wear`")
}

/// Items for an answer or the log: each by name and id; one the inventory
/// did not describe by its id alone.
fn names(items: &[Item]) -> String {
    let shown: Vec<String> = items
        .iter()
        .map(|item| if item.name.is_empty() { item.id.to_string() } else { item.show() })
        .collect();
    shown.join(", ")
}

fn check_length(text: &str, limit: usize) -> Result<(), String> {
    if text.len() > limit {
        return Err(format!("the text is {} bytes long; the limit is {limit}", text.len()));
    }
    Ok(())
}

/// Answers a command that has come to an end, and hands back the instant
/// message it sent, if it did.
fn finish(tag: &str, command: &str, done: Result<Done, String>, reply: &Sender<Response>) -> Option<SentIm> {
    match done {
        Ok(Done::Answer(text)) => {
            let _ = reply.send(Response::Ok(text));
            None
        }
        Ok(Done::Sent(text, sent)) => {
            let _ = reply.send(Response::Ok(Some(text)));
            Some(sent)
        }
        Ok(Done::Parked) => None,
        Err(reason) => {
            refuse(tag, command, reply, reason);
            None
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    fn chat(channel: i32, chat_type: u8, text: &str) -> Result<Chat, String> {
        Ok(Chat {
            channel,
            chat_type,
            text: text.to_string(),
        })
    }

    #[test]
    fn plain_text_is_said_on_channel_zero() {
        assert_eq!(parse_chat("hello there", 7), chat(0, SAY, "hello there"));
        assert_eq!(parse_chat("/me waves", 7), chat(0, SAY, "/me waves"));
        assert_eq!(parse_chat("/dance now", 0), chat(0, SAY, "/dance now"));
        assert_eq!(parse_chat("/-5 negative", 0), chat(0, SAY, "/-5 negative"));
        assert_eq!(parse_chat("/", 0), chat(0, SAY, "/"));
    }

    #[test]
    fn a_number_after_the_slash_is_the_channel() {
        assert_eq!(parse_chat("/1 test message", 0), chat(1, SAY, "test message"));
        assert_eq!(parse_chat("/42open", 0), chat(42, SAY, "open"));
        assert_eq!(parse_chat("/0   spaced", 9), chat(0, SAY, "spaced"));
        assert_eq!(parse_chat("/2147483647 top", 0), chat(i32::MAX, SAY, "top"));
        assert!(parse_chat("/2147483648 over", 0).unwrap_err().contains("2147483648"));
        assert!(parse_chat("/99999999999999999999 x", 0).is_err());
    }

    #[test]
    fn two_slashes_repeat_the_last_channel() {
        assert_eq!(parse_chat("//second", 7), chat(7, SAY, "second"));
        assert_eq!(parse_chat("//second", 0), chat(0, SAY, "second"));
    }

    #[test]
    fn shout_and_whisper_change_the_volume() {
        assert_eq!(parse_chat("/shout over here", 5), chat(0, SHOUT, "over here"));
        assert_eq!(parse_chat("/whisper psst", 5), chat(0, WHISPER, "psst"));
        // Without the space they are ordinary words.
        assert_eq!(parse_chat("/shouting", 5), chat(0, SAY, "/shouting"));
        assert_eq!(parse_chat("/shout", 5), chat(0, SAY, "/shout"));
    }

    #[test]
    fn empty_and_over_long_text_is_refused() {
        for empty in ["", "   ", "/5", "/5  ", "//", "/shout  "] {
            assert!(parse_chat(empty, 0).unwrap_err().contains("no text"), "{empty:?}");
        }
        assert!(parse_chat(&"x".repeat(1023), 0).is_ok());
        assert!(parse_chat(&format!("/1 {}", "x".repeat(1023)), 0).is_ok());
        assert!(parse_chat(&"x".repeat(1024), 0).unwrap_err().contains("1023"));
        // The limit is in bytes.
        assert!(parse_chat(&"é".repeat(512), 0).is_err());
    }

    #[test]
    fn recipients_are_names_or_uuids() {
        let name = |text: &str| Ok(Recipient::Name(text.to_string()));
        assert_eq!(parse_recipient("Jane Doe"), name("jane doe"));
        assert_eq!(parse_recipient("jane.doe"), name("jane doe"));
        assert_eq!(parse_recipient("  JANE   DOE "), name("jane doe"));
        assert_eq!(parse_recipient("sitbot"), name("sitbot resident"));
        assert_eq!(parse_recipient("SitBot Resident"), name("sitbot resident"));
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(parse_recipient(id), Ok(Recipient::Id(Uuid::parse_str(id).unwrap())));
        for bad in ["", "  ", "a b c", "a.b.c"] {
            assert!(parse_recipient(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_the_exact_name_matches() {
        let (jane, other) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let found = vec![(other, "Jane Doester".to_string()), (jane, "JANE Doe".to_string())];
        assert_eq!(exact_match("jane doe", &found), Some(&found[1]));
        assert_eq!(exact_match("jane", &found), None);
        assert_eq!(exact_match("jane doe", &found[..1]), None);
        assert_eq!(search_text("sitbot resident"), "sitbot");
        assert_eq!(search_text("jane doe"), "jane doe");
    }

    // ------------------------------------------------------------ commands

    #[derive(Debug, PartialEq)]
    pub(crate) enum Call {
        Say(i32, u8, String),
        Im(Uuid, String),
        /// An automatic answer: an autoreply or RLV's.
        AutoIm(Uuid, String),
        Find(Uuid, String),
        Stand,
        SitGround,
        Sit(Uuid),
        Teleport(String, Vector3),
        TeleportTo(u64, Vector3),
        RequestTp(Uuid, String),
        OfferTp(Uuid, String),
        AcceptTp(Uuid),
        Touch(Uuid),
        /// An attach of this item to this point; 0 is the simulator's choice.
        Attach(Item, u8),
        Detach(Uuid),
        Animate(Uuid, bool),
        Sound(Uuid),
        /// A question about who owns this object.
        OwnerOf(Uuid),
        /// A grant of these permission bits to this object's script.
        Grant(Uuid, Uuid, i32),
        /// An accepted inventory offer: whom it is answered to, whether an
        /// object gave, the transaction and the folder.
        AcceptInventory(Uuid, bool, Uuid, Uuid),
        /// A look for an object the simulator has not described.
        LookFor(Uuid),
        /// A `scan` made ready, with the distance it named.
        Reach(Option<f32>),
        /// A question about the properties of these objects.
        Properties(Vec<u32>),
    }

    pub(crate) struct FakeGrid {
        pub(crate) calls: Vec<Call>,
        pub(crate) offline: bool,
        pub(crate) position: Option<Vector3>,
        /// What `touch` answers.
        pub(crate) touch: Result<Touch, String>,
        /// The inventory jobs handed over: id, deadline and what.
        pub(crate) jobs: Vec<(u64, Instant, What)>,
        /// The simulator has no inventory service.
        pub(crate) no_inventory: bool,
        /// The grid makes the avatar's appearance.
        pub(crate) outfit: bool,
        /// What is attached to the avatar.
        pub(crate) attached: Vec<(Uuid, u8)>,
        /// The simulator has not said yet which services it has.
        pub(crate) settling: bool,
        /// The attachments on the avatar by their prims, with the item
        /// each was made from.
        pub(crate) worn: Vec<(Uuid, Uuid)>,
        /// The RLV restrictions that are kept.
        pub(crate) kept: Restrictions,
        /// RLV is off: none of them is in force.
        pub(crate) rlv_off: bool,
        pub(crate) sitting: Sitting,
        /// What the simulator has described, object by object.
        pub(crate) things: Vec<(Uuid, Thing)>,
        /// What `look_for` answers for an object not in `things`.
        pub(crate) look: Result<Look, String>,
        /// How far the object scan reaches.
        pub(crate) reach: f32,
        /// The prims the simulator has described.
        pub(crate) sightings: Vec<Sighting>,
        /// When the surroundings count as all described; long ago when
        /// left out.
        pub(crate) settled_at: Option<Instant>,
    }

    impl Default for FakeGrid {
        fn default() -> FakeGrid {
            let position = Some([10.0, 20.0, 30.0]);
            FakeGrid {
                calls: Vec::new(),
                offline: false,
                position,
                touch: Ok(Touch::Sent),
                jobs: Vec::new(),
                no_inventory: false,
                outfit: false,
                attached: Vec::new(),
                settling: false,
                worn: Vec::new(),
                kept: Restrictions::default(),
                rlv_off: false,
                sitting: Sitting::No,
                things: Vec::new(),
                look: Ok(Look::Waiting),
                reach: 32.0,
                sightings: Vec::new(),
                settled_at: None,
            }
        }
    }

    impl FakeGrid {
        /// What `Session::autoresponse` does, which is no part of `Grid`.
        pub(crate) fn autoresponse(&mut self, to: Uuid, text: &str) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::AutoIm(to, text.to_string()));
            Ok(())
        }

        fn check(&self) -> Result<(), String> {
            if self.offline {
                Err(NOT_CONNECTED.to_string())
            } else {
                Ok(())
            }
        }
    }

    impl Grid for FakeGrid {
        fn reach(&mut self, distance: Option<f32>, _: Instant) -> Result<f32, String> {
            self.check()?;
            self.calls.push(Call::Reach(distance));
            if let Some(distance) = distance {
                self.reach = self.reach.max(distance);
            }
            Ok(distance.unwrap_or(self.reach))
        }
        fn sightings(&self) -> Vec<Sighting> {
            self.sightings.clone()
        }
        fn ask_properties(&mut self, local_ids: &[u32], _: Instant) {
            if !local_ids.is_empty() {
                self.calls.push(Call::Properties(local_ids.to_vec()));
            }
        }
        fn settled_at(&self) -> Instant {
            self.settled_at.unwrap_or_else(|| Instant::now() - Duration::from_secs(60))
        }
        fn say(&mut self, channel: i32, chat_type: u8, text: &str, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Say(channel, chat_type, text.to_string()));
            Ok(())
        }
        fn instant_message(&mut self, to: Uuid, text: &str, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Im(to, text.to_string()));
            Ok(())
        }
        fn find_avatar(&mut self, query: Uuid, name: &str, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Find(query, name.to_string()));
            Ok(())
        }
        fn stand(&mut self, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Stand);
            Ok(())
        }
        fn sit_ground(&mut self, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::SitGround);
            Ok(())
        }
        fn sit(&mut self, object: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Sit(object));
            Ok(())
        }
        fn teleport(&mut self, region: &str, position: Vector3, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Teleport(region.to_string(), position));
            Ok(())
        }
        fn teleport_to(&mut self, handle: u64, position: Vector3, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::TeleportTo(handle, position));
            Ok(())
        }
        fn region(&self) -> Option<String> {
            (!self.offline).then(|| "Home".to_string())
        }
        fn position(&self) -> Option<Vector3> {
            self.position
        }
        fn request_teleport(&mut self, to: Uuid, message: &str, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::RequestTp(to, message.to_string()));
            Ok(())
        }
        fn offer_teleport(&mut self, to: Uuid, message: &str, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::OfferTp(to, message.to_string()));
            Ok(())
        }
        fn accept_teleport(&mut self, lure: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::AcceptTp(lure));
            Ok(())
        }
        fn touch(&mut self, object: Uuid, _: Instant) -> Result<Touch, String> {
            self.check()?;
            self.calls.push(Call::Touch(object));
            self.touch.clone()
        }
        fn inventory(&mut self, id: u64, deadline: Instant, what: What) -> Result<(), String> {
            self.check()?;
            if self.no_inventory {
                return Err(inventory::NO_SERVICE.to_string());
            }
            self.jobs.push((id, deadline, what));
            Ok(())
        }
        fn outfit_service(&self) -> bool {
            self.outfit
        }
        fn ready(&self) -> bool {
            !self.offline && !self.settling
        }
        fn attached(&self) -> Vec<(Uuid, u8)> {
            self.attached.clone()
        }
        fn attached_item(&self, object: Uuid) -> Option<Uuid> {
            self.worn.iter().find(|(prim, _)| *prim == object).map(|(_, item)| *item)
        }
        fn attach(&mut self, item: &Item, point: u8, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Attach(item.clone(), point));
            Ok(())
        }
        fn detach(&mut self, item: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Detach(item));
            Ok(())
        }
        fn animate(&mut self, animation: Uuid, start: bool, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Animate(animation, start));
            Ok(())
        }
        fn object_owner(&mut self, object: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::OwnerOf(object));
            Ok(())
        }
        fn grant_permissions(&mut self, object: Uuid, item: Uuid, permissions: i32, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Grant(object, item, permissions));
            Ok(())
        }
        fn accept_inventory(&mut self, to: Uuid, from_object: bool, transaction: Uuid, folder: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::AcceptInventory(to, from_object, transaction, folder));
            Ok(())
        }
        fn play_sound(&mut self, sound: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Sound(sound));
            Ok(())
        }
        fn restrictions(&self) -> Option<&Restrictions> {
            (!self.rlv_off).then_some(&self.kept)
        }
        fn sitting(&self) -> Sitting {
            self.sitting
        }
        fn thing(&self, object: Uuid) -> Thing {
            let described = self.things.iter().find(|(id, _)| *id == object);
            described.map_or(Thing::unknown(object), |(_, thing)| thing.clone())
        }
        fn attachment_prims(&self, item: Uuid) -> Vec<Uuid> {
            self.worn.iter().filter(|(_, of)| *of == item).map(|(prim, _)| *prim).collect()
        }
        fn look_for(&mut self, object: Uuid, _: Instant) -> Result<Look, String> {
            self.check()?;
            self.calls.push(Call::LookFor(object));
            match self.things.iter().any(|(id, _)| *id == object) {
                true => Ok(Look::Known),
                false => self.look.clone(),
            }
        }
        fn unknown_object(&self, object: Uuid) -> String {
            format!("object {object} is not known to the bot")
        }
    }

    struct Rig {
        commands: Commands,
        grid: FakeGrid,
        now: Instant,
    }

    fn jane() -> Uuid {
        Uuid::from_bytes([1; 16])
    }

    fn jane_found() -> Vec<(Uuid, String)> {
        vec![
            (Uuid::from_bytes([2; 16]), "Jane Doester".to_string()),
            (jane(), "Jane Doe".to_string()),
        ]
    }

    fn sent_to_jane(text: &str) -> SentIm {
        SentIm {
            to: jane(),
            name: "Jane Doe".to_string(),
            text: text.to_string(),
        }
    }

    fn error(answer: &Receiver<Response>) -> String {
        match answer.try_recv() {
            Ok(Response::Error(reason)) => reason,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                commands: Commands::default(),
                grid: FakeGrid::default(),
                now: Instant::now(),
            }
        }

        fn run(&mut self, request: Request) -> (Receiver<Response>, Option<SentIm>) {
            let (reply, answer) = mpsc::channel();
            let sent = self.commands.handle(request, reply, Some(&mut self.grid), self.now);
            (answer, sent)
        }

        fn say(&mut self, message: &str) -> Receiver<Response> {
            self.run(Request::Say(message.to_string())).0
        }

        fn im(&mut self, to: &str, text: &str) -> (Receiver<Response>, Option<SentIm>) {
            self.run(Request::Im {
                to: to.to_string(),
                text: text.to_string(),
            })
        }

        fn found(&mut self, query: Uuid, avatars: &[(Uuid, String)]) -> Vec<SentIm> {
            self.commands.avatars_found(query, avatars, Some(&mut self.grid), self.now)
        }

        fn calls(&mut self) -> Vec<Call> {
            std::mem::take(&mut self.grid.calls)
        }

        /// The query id of the one lookup sent since the last call.
        fn lookup(&mut self, name: &str) -> Uuid {
            match &self.calls()[..] {
                [Call::Find(query, asked)] if asked == name => *query,
                other => panic!("expected one lookup of {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn say_sends_chat_and_remembers_the_channel() {
        let mut rig = Rig::new();
        assert_eq!(rig.say("hello").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.say("//still zero").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.say("/7 first").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.say("//second").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.say("/shout loud").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(
            rig.calls(),
            [
                Call::Say(0, SAY, "hello".into()),
                Call::Say(0, SAY, "still zero".into()),
                Call::Say(7, SAY, "first".into()),
                Call::Say(7, SAY, "second".into()),
                Call::Say(0, SHOUT, "loud".into()),
            ]
        );
    }

    #[test]
    fn say_is_refused_when_it_cannot_be_sent() {
        let mut rig = Rig::new();
        assert!(error(&rig.say("/5")).contains("no text"));
        assert!(error(&rig.say(&"x".repeat(2000))).contains("1023"));
        rig.grid.offline = true;
        assert_eq!(error(&rig.say("/3 hello")), NOT_CONNECTED);
        // No session at all.
        let (reply, answer) = mpsc::channel();
        let sent = rig
            .commands
            .handle(Request::Say("hello".into()), reply, None::<&mut FakeGrid>, rig.now);
        assert_eq!((error(&answer).as_str(), sent), (NOT_CONNECTED, None));
        assert_eq!(rig.calls(), []);
        // A refused message does not change what `//` means.
        rig.grid.offline = false;
        rig.say("//after");
        assert_eq!(rig.calls(), [Call::Say(0, SAY, "after".into())]);
    }

    #[test]
    fn im_to_a_uuid_goes_out_at_once() {
        let mut rig = Rig::new();
        let (answer, sent) = rig.im(&jane().to_string(), "hello");
        assert_eq!(
            sent,
            Some(SentIm {
                to: jane(),
                name: String::new(),
                text: "hello".into()
            })
        );
        assert_eq!(answer.try_recv(), Ok(Response::Ok(Some(format!("sent to {}", jane())))));
        assert_eq!(rig.calls(), [Call::Im(jane(), "hello".into())]);
    }

    #[test]
    fn im_to_a_name_waits_for_the_lookup() {
        let mut rig = Rig::new();
        let (answer, sent) = rig.im("jane.doe", "are you there?");
        assert_eq!(sent, None);
        assert!(answer.try_recv().is_err());
        let query = rig.lookup("jane doe");
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + LOOKUP_LIMIT));

        // An answer to some other query changes nothing.
        assert_eq!(rig.found(Uuid::from_bytes([9; 16]), &jane_found()), []);
        assert_eq!(rig.found(query, &jane_found()), [sent_to_jane("are you there?")]);
        assert_eq!(answer.try_recv(), Ok(Response::Ok(Some(format!("sent to Jane Doe <{}>", jane())))));
        assert_eq!(rig.calls(), [Call::Im(jane(), "are you there?".into())]);
        assert_eq!(rig.commands.next_deadline(), None);

        // The name is known from now on, however it is written.
        let (answer, sent) = rig.im("JANE DOE", "again");
        assert_eq!(sent, Some(sent_to_jane("again")));
        assert!(matches!(answer.try_recv(), Ok(Response::Ok(Some(_)))));
        assert_eq!(rig.calls(), [Call::Im(jane(), "again".into())]);
    }

    #[test]
    fn two_commands_for_one_name_share_a_lookup() {
        let mut rig = Rig::new();
        let (first, _) = rig.im("Jane Doe", "one");
        let (second, _) = rig.im("jane.doe", "two");
        let query = rig.lookup("jane doe");
        assert_eq!(rig.found(query, &jane_found()), [sent_to_jane("one"), sent_to_jane("two")]);
        assert!(matches!(first.try_recv(), Ok(Response::Ok(Some(_)))));
        assert!(matches!(second.try_recv(), Ok(Response::Ok(Some(_)))));
    }

    #[test]
    fn single_word_names_are_searched_by_the_word() {
        let mut rig = Rig::new();
        let (answer, _) = rig.im("sitbot", "hi");
        let query = rig.lookup("sitbot");
        let id = Uuid::from_bytes([5; 16]);
        let found = [(id, "sitbot Resident".to_string())];
        assert_eq!(
            rig.found(query, &found),
            [SentIm {
                to: id,
                name: "sitbot Resident".into(),
                text: "hi".into()
            }]
        );
        assert!(matches!(answer.try_recv(), Ok(Response::Ok(Some(_)))));
    }

    #[test]
    fn unknown_names_are_refused() {
        let mut rig = Rig::new();
        let (answer, _) = rig.im("Nobody Here", "hello");
        let query = rig.lookup("nobody here");
        assert_eq!(rig.found(query, &jane_found()), []);
        assert!(error(&answer).contains("\"Nobody Here\""));
        let (answer, _) = rig.im("Jane", "hello");
        let query = rig.lookup("jane");
        assert_eq!(rig.found(query, &[]), []);
        assert!(error(&answer).contains("\"Jane\""));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn an_unanswered_lookup_times_out() {
        let mut rig = Rig::new();
        let (answer, _) = rig.im("Jane Doe", "hello");
        let query = rig.lookup("jane doe");
        rig.commands.tick(rig.now + LOOKUP_LIMIT - Duration::from_millis(1));
        assert!(answer.try_recv().is_err());
        rig.commands.tick(rig.now + LOOKUP_LIMIT);
        assert!(error(&answer).contains("10 s"));
        assert_eq!(rig.commands.next_deadline(), None);
        // A late answer sends nothing.
        assert_eq!(rig.found(query, &jane_found()), []);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_failed_lookup_fails_its_commands_only() {
        let mut rig = Rig::new();
        let (jane, _) = rig.im("Jane Doe", "hello");
        let failed = rig.lookup("jane doe");
        let (john, _) = rig.im("John Roe", "hello");
        rig.calls();
        rig.commands.lookup_failed(failed, "HTTP 500");
        let reason = error(&jane);
        assert!(reason.contains("\"Jane Doe\"") && reason.contains("HTTP 500"), "{reason}");
        assert!(john.try_recv().is_err());
        assert!(rig.commands.next_deadline().is_some());
    }

    #[test]
    fn waiting_commands_fail_with_the_session() {
        let mut rig = Rig::new();
        let (lost, _) = rig.im("Jane Doe", "hello");
        rig.commands.session_lost();
        assert_eq!(error(&lost), NOT_CONNECTED);
        assert_eq!(rig.commands.next_deadline(), None);

        // The name resolves after the avatar has left the region.
        let (gone, _) = rig.im("Jane Doe", "hello");
        rig.calls();
        let query = Uuid::from_u128(2);
        rig.grid.offline = true;
        assert_eq!(rig.found(query, &jane_found()), []);
        assert_eq!(error(&gone), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn im_is_refused_before_anything_is_sent() {
        let mut rig = Rig::new();
        // A refused command leaves nothing for the message log either.
        assert_eq!(rig.im("Jane Doe", "").1, None);
        assert_eq!(rig.im("a b c", "hello").1, None);
        assert!(error(&rig.im("Jane Doe", "").0).contains("no text"));
        assert!(error(&rig.im("Jane Doe", &"x".repeat(1024)).0).contains("1023"));
        assert!(error(&rig.im("a b c", "hello").0).contains("a b c"));
        rig.grid.offline = true;
        assert_eq!(error(&rig.im("Jane Doe", "hello").0), NOT_CONNECTED);
        assert_eq!(error(&rig.im(&jane().to_string(), "hello").0), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.commands.next_deadline(), None);
    }

    #[test]
    fn a_client_that_went_away_does_not_stop_the_message() {
        let mut rig = Rig::new();
        let (answer, _) = rig.im("Jane Doe", "hello");
        drop(answer);
        let query = rig.lookup("jane doe");
        assert_eq!(rig.found(query, &jane_found()), [sent_to_jane("hello")]);
    }

    // ------------------------------------------------------------ parsing

    #[test]
    fn sit_takes_a_uuid_or_the_ground() {
        assert_eq!(parse_sit("ground"), Ok(SitTarget::Ground));
        assert_eq!(parse_sit(" Ground "), Ok(SitTarget::Ground));
        assert_eq!(parse_sit(&jane().to_string()), Ok(SitTarget::Object(jane())));
        assert!(parse_sit("chair").unwrap_err().contains("\"chair\""));
        assert!(parse_sit("").is_err());
    }

    fn destination(args: &[&str]) -> Result<Destination, String> {
        parse_tp(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    fn to(x: f32, y: f32, z: Option<f32>, region: Option<&str>) -> Result<Destination, String> {
        Ok(Destination {
            x,
            y,
            z,
            region: region.map(str::to_string),
        })
    }

    #[test]
    fn tp_reads_coordinates_then_an_optional_height_and_region() {
        assert_eq!(destination(&["100", "120"]), to(100.0, 120.0, None, None));
        assert_eq!(destination(&["100.5", "120", "40"]), to(100.5, 120.0, Some(40.0), None));
        assert_eq!(
            destination(&["128", "128", "25", "Home"]),
            to(128.0, 128.0, Some(25.0), Some("Home"))
        );
        assert_eq!(
            destination(&["128", "128", "Blue", "Lagoon"]),
            to(128.0, 128.0, None, Some("Blue Lagoon"))
        );
        assert_eq!(
            destination(&["128", "128", "25", "Blue", "Lagoon"]),
            to(128.0, 128.0, Some(25.0), Some("Blue Lagoon"))
        );
        // A region whose name starts with a number needs the height given.
        assert_eq!(destination(&["1", "2", "3", "7", "Seas"]), to(1.0, 2.0, Some(3.0), Some("7 Seas")));
        // Words that only look like numbers to a parser are names.
        assert_eq!(destination(&["1", "2", "Infinity"]), to(1.0, 2.0, None, Some("Infinity")));
        assert_eq!(destination(&["0", "0", "0"]), to(0.0, 0.0, Some(0.0), None));
    }

    #[test]
    fn tp_refuses_bad_coordinates() {
        assert!(destination(&["north", "128"]).unwrap_err().contains("`x`"));
        assert!(destination(&["128", "south"]).unwrap_err().contains("`y`"));
        assert!(destination(&["128"]).is_err());
        assert!(destination(&[]).is_err());
        assert!(destination(&["-1", "128"]).unwrap_err().contains("`x`"));
        assert!(destination(&["128", "128", "-5"]).unwrap_err().contains("`z`"));
        assert!(destination(&["inf", "128"]).is_err());
        assert!(destination(&["nan", "128"]).is_err());
    }

    // --------------------------------------------------- moving the avatar

    fn ok(answer: &Receiver<Response>) -> Option<String> {
        match answer.try_recv() {
            Ok(Response::Ok(text)) => text,
            other => panic!("expected success, got {other:?}"),
        }
    }

    fn other_object() -> Uuid {
        Uuid::from_bytes([0x0C; 16])
    }

    impl Rig {
        fn command(&mut self, request: Request) -> Receiver<Response> {
            self.run(request).0
        }

        fn words(words: &[&str]) -> Vec<String> {
            words.iter().map(|word| word.to_string()).collect()
        }

        fn offer(&mut self, from: Uuid, name: &str, lure: Uuid) {
            self.commands.teleport_offer(from, name, lure, Some(&mut self.grid), self.now);
        }

        fn accept(&mut self, kind: &str, who: Option<&str>) -> Receiver<Response> {
            self.command(Request::Accept {
                kind: kind.to_string(),
                who: who.map(str::to_string),
            })
        }
    }

    #[test]
    fn stand_and_ground_sit_answer_at_once_and_go_manual() {
        let mut rig = Rig::new();
        assert!(!rig.commands.take_manual());
        assert_eq!(ok(&rig.command(Request::Stand)), None);
        assert!(rig.commands.take_manual());
        assert!(!rig.commands.take_manual(), "asked once, told once");
        assert_eq!(ok(&rig.command(Request::Sit("ground".into()))), None);
        assert!(rig.commands.take_manual());
        assert_eq!(rig.calls(), [Call::Stand, Call::SitGround]);
    }

    #[test]
    fn refused_commands_do_not_go_manual() {
        let mut rig = Rig::new();
        assert!(error(&rig.command(Request::Sit("chair".into()))).contains("chair"));
        assert!(error(&rig.command(Request::Tp(Rig::words(&["north", "1"])))).contains("`x`"));
        assert!(error(&rig.command(Request::Touch("thing".into()))).contains("thing"));
        rig.grid.offline = true;
        let requests = [
            Request::Stand,
            Request::Sit("ground".into()),
            Request::Sit(other_object().to_string()),
            Request::Tp(Rig::words(&["1", "2", "3"])),
            Request::AskTp {
                to: jane().to_string(),
                message: String::new(),
            },
            Request::SendTp {
                to: jane().to_string(),
                message: String::new(),
            },
            Request::Touch(other_object().to_string()),
            Request::Shout("hello".into()),
        ];
        for request in requests {
            assert_eq!(error(&rig.command(request.clone())), NOT_CONNECTED, "{request:?}");
        }
        // And with no session at all.
        let (reply, answer) = mpsc::channel();
        rig.commands.handle(Request::Stand, reply, None::<&mut FakeGrid>, rig.now);
        assert_eq!(error(&answer), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn sit_on_an_object_answers_with_the_simulators_verdict() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::Sit(other_object().to_string()));
        assert_eq!(rig.calls(), [Call::Sit(other_object())]);
        assert!(rig.commands.take_manual());
        assert!(answer.try_recv().is_err(), "no answer before the simulator's");
        rig.commands.seated(other_object());
        assert_eq!(ok(&answer), Some(format!("seated on {}", other_object())));

        let answer = rig.command(Request::Sit(other_object().to_string()));
        assert!(rig.commands.sit_failed("no room to sit here"));
        assert_eq!(error(&answer), "no room to sit here");
        // Nobody waiting: the failure was the bot's own.
        assert!(!rig.commands.sit_failed("the simulator did not confirm"));
    }

    #[test]
    fn a_newer_sit_or_a_stand_ends_the_wait_of_an_older_sit() {
        let mut rig = Rig::new();
        let first = rig.command(Request::Sit(other_object().to_string()));
        let second = rig.command(Request::Sit(jane().to_string()));
        assert!(error(&first).contains("superseded"));
        assert_eq!(ok(&rig.command(Request::Stand)), None);
        assert!(error(&second).contains("stand"));
        rig.commands.seated(jane());
    }

    #[test]
    fn tp_fills_in_the_region_and_height_and_waits_for_the_arrival() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::Tp(Rig::words(&["100", "120"])));
        assert_eq!(rig.calls(), [Call::Teleport("Home".into(), [100.0, 120.0, 30.0])]);
        assert!(rig.commands.take_manual());
        assert!(answer.try_recv().is_err());
        rig.commands.arrived("Home");
        assert_eq!(ok(&answer), Some("arrived in Home".to_string()));

        let answer = rig.command(Request::Tp(Rig::words(&["128", "128", "25", "Blue", "Lagoon"])));
        assert_eq!(rig.calls(), [Call::Teleport("Blue Lagoon".into(), [128.0, 128.0, 25.0])]);
        assert!(rig.commands.teleport_failed("region \"Blue Lagoon\" not found"));
        assert!(error(&answer).contains("not found"));
        assert!(!rig.commands.teleport_failed("no answer within 60 s"));
    }

    #[test]
    fn tp_without_a_height_needs_one_to_be_known() {
        let mut rig = Rig::new();
        rig.grid.position = None;
        assert!(error(&rig.command(Request::Tp(Rig::words(&["100", "120"])))).contains("give z"));
        assert!(!rig.commands.take_manual());
        rig.command(Request::Tp(Rig::words(&["100", "120", "40"])));
        assert_eq!(rig.calls(), [Call::Teleport("Home".into(), [100.0, 120.0, 40.0])]);
    }

    #[test]
    fn the_last_tp_and_sit_are_remembered_for_persist() {
        let mut rig = Rig::new();
        assert_eq!((rig.commands.last_tp(), rig.commands.last_sit()), (None, None));

        // The region and the height are the ones the command resolved to.
        rig.command(Request::Tp(Rig::words(&["100", "120"])));
        let here = Location {
            region: "Home".to_string(),
            position: [100.0, 120.0, 30.0],
        };
        assert_eq!(rig.commands.last_tp(), Some(&here));
        // The outcome does not matter.
        rig.command(Request::Tp(Rig::words(&["128", "128", "25", "Blue", "Lagoon"])));
        assert!(rig.commands.teleport_failed("region \"Blue Lagoon\" not found"));
        let lagoon = Location {
            region: "Blue Lagoon".to_string(),
            position: [128.0, 128.0, 25.0],
        };
        assert_eq!(rig.commands.last_tp(), Some(&lagoon));

        rig.command(Request::Sit(other_object().to_string()));
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(other_object())));
        assert!(rig.commands.sit_failed("no room to sit here"));
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(other_object())));
        rig.command(Request::Sit("ground".into()));
        assert_eq!(rig.commands.last_sit(), Some(SitState::Ground));

        // Neither is a `tp` or a `sit`.
        rig.command(Request::Stand);
        rig.offer(jane(), "Jane Doe", Uuid::from_bytes([0x44; 16]));
        rig.accept("tp", Some(&jane().to_string()));
        rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        });
        assert_eq!(rig.commands.last_tp(), Some(&lagoon));
        assert_eq!(rig.commands.last_sit(), Some(SitState::Ground));

        // They outlive the session.
        rig.commands.session_lost();
        assert_eq!(rig.commands.last_tp(), Some(&lagoon));
        assert_eq!(rig.commands.last_sit(), Some(SitState::Ground));
    }

    #[test]
    fn refused_tp_and_sit_commands_are_not_remembered() {
        let mut rig = Rig::new();
        rig.command(Request::Sit("chair".into()));
        rig.command(Request::Tp(Rig::words(&["north", "1"])));
        rig.grid.offline = true;
        rig.command(Request::Sit(other_object().to_string()));
        rig.command(Request::Sit("ground".into()));
        rig.command(Request::Tp(Rig::words(&["1", "2", "3"])));
        assert_eq!((rig.commands.last_tp(), rig.commands.last_sit()), (None, None));
    }

    #[test]
    fn persist_is_not_a_command_for_the_grid() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::Persist { location: None, sit: None });
        assert!(error(&answer).contains("not a command for the grid"));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn shout_and_whisper_send_the_message_as_it_is() {
        let mut rig = Rig::new();
        rig.say("/7 on seven");
        assert_eq!(ok(&rig.command(Request::Shout("/1 hello".into()))), None);
        assert_eq!(ok(&rig.command(Request::Whisper("psst".into()))), None);
        // The channel `//` stands for is still the one `say` used.
        rig.say("//again");
        assert_eq!(
            rig.calls(),
            [
                Call::Say(7, SAY, "on seven".into()),
                Call::Say(0, SHOUT, "/1 hello".into()),
                Call::Say(0, WHISPER, "psst".into()),
                Call::Say(7, SAY, "again".into()),
            ]
        );
        assert!(error(&rig.command(Request::Shout("  ".into()))).contains("no text"));
        assert!(error(&rig.command(Request::Whisper("x".repeat(1024)))).contains("1023"));
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn sendtp_offers_a_teleport_and_leaves_the_bot_where_it_is() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::SendTp {
            to: "Jane Doe".into(),
            message: "come and see".into(),
        });
        let query = rig.lookup("jane doe");
        assert_eq!(rig.found(query, &jane_found()), []);
        assert_eq!(ok(&answer), Some(format!("offered Jane Doe <{}> a teleport", jane())));
        // Without a message, the one viewers send.
        rig.command(Request::SendTp {
            to: "jane.doe".into(),
            message: String::new(),
        });
        assert_eq!(
            rig.calls(),
            [
                Call::OfferTp(jane(), "come and see".into()),
                Call::OfferTp(jane(), "Join me in Home".into())
            ]
        );
        assert!(!rig.commands.take_manual());
        let long = Request::SendTp {
            to: "jane.doe".into(),
            message: "x".repeat(255),
        };
        assert!(error(&rig.command(long)).contains("254"));
    }

    // ---------------------------------------------------- teleport offers

    fn lure(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    #[test]
    fn requested_teleport_offer_is_accepted_once() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::AskTp {
            to: jane().to_string(),
            message: "may I?".into(),
        });
        assert_eq!(ok(&answer), Some(format!("asked {} for a teleport", jane())));
        assert!(rig.commands.take_manual());
        assert_eq!(rig.calls(), [Call::RequestTp(jane(), "may I?".into())]);

        rig.now += Duration::from_secs(9 * 60);
        rig.offer(jane(), "Jane Doe", lure(1));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
        // One request, one acceptance: the next offer waits for the operator.
        rig.offer(jane(), "Jane Doe", lure(2));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn requesttp_by_name_waits_for_the_lookup() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::AskTp {
            to: "Jane Doe".into(),
            message: String::new(),
        });
        let query = rig.lookup("jane doe");
        assert!(!rig.commands.take_manual(), "nothing has been asked yet");
        rig.found(query, &jane_found());
        assert_eq!(ok(&answer), Some(format!("asked Jane Doe <{}> for a teleport", jane())));
        assert!(rig.commands.take_manual());
        assert_eq!(rig.calls(), [Call::RequestTp(jane(), String::new())]);

        let answer = rig.command(Request::AskTp {
            to: "Nobody Here".into(),
            message: String::new(),
        });
        let query = rig.lookup("nobody here");
        rig.found(query, &jane_found());
        assert!(error(&answer).contains("\"Nobody Here\""));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn late_and_unsolicited_offers_wait_for_accepttp() {
        let mut rig = Rig::new();
        rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        });
        rig.calls();
        rig.commands.take_manual();
        rig.now += Duration::from_secs(15 * 60);
        rig.offer(jane(), "Jane Doe", lure(1));
        let stranger = Uuid::from_bytes([9; 16]);
        rig.offer(stranger, "John Roe", lure(5));
        assert_eq!(rig.calls(), [], "neither is accepted by itself");

        // The operator accepts by name; the offer said who it was from.
        let answer = rig.accept("tp", Some("john.roe"));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(5))]);
        assert!(rig.commands.take_manual());
        assert!(answer.try_recv().is_err());
        rig.commands.arrived("Elsewhere");
        assert_eq!(ok(&answer), Some("arrived in Elsewhere".to_string()));

        // Used once.
        let again = rig.accept("tp", Some("John Roe"));
        assert_eq!(error(&again), "there is no teleport offer from John Roe");
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn a_longer_timeout_accepts_a_later_offer() {
        let mut rig = Rig::new();
        rig.commands = Commands::new(Duration::from_secs(30 * 60));
        rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        });
        rig.calls();
        rig.now += Duration::from_secs(15 * 60);
        rig.offer(jane(), "Jane Doe", lure(1));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
    }

    #[test]
    fn accepttp_takes_the_latest_offer_and_reports_a_refusal() {
        let mut rig = Rig::new();
        assert!(error(&rig.accept("tp", Some(&jane().to_string()))).contains("no teleport offer"));
        rig.offer(jane(), "Jane Doe", lure(1));
        rig.offer(jane(), "Jane Doe", lure(2));
        let answer = rig.accept("tp", Some("Jane Doe"));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(2))]);
        assert!(rig.commands.teleport_failed("the offer has expired"));
        assert_eq!(error(&answer), "the offer has expired");
    }

    #[test]
    fn offers_are_forgotten_with_the_session_and_requests_with_home() {
        let mut rig = Rig::new();
        rig.offer(jane(), "Jane Doe", lure(1));
        let waiting = rig.command(Request::Tp(Rig::words(&["1", "2", "3"])));
        let sitting = rig.command(Request::Sit(other_object().to_string()));
        rig.calls();
        rig.commands.session_lost();
        assert!(error(&waiting).contains("session was lost"));
        assert!(error(&sitting).contains("session was lost"));
        assert!(error(&rig.accept("tp", Some("Jane Doe"))).contains("no teleport offer"));

        // Sent home after asking: the answer must not take the bot away.
        rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        });
        rig.calls();
        rig.commands.forget_requests();
        rig.offer(jane(), "Jane Doe", lure(3));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn an_offer_that_cannot_be_accepted_now_is_kept() {
        let mut rig = Rig::new();
        rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        });
        rig.calls();
        rig.grid.offline = true;
        rig.offer(jane(), "Jane Doe", lure(1));
        rig.grid.offline = false;
        rig.accept("tp", Some("Jane Doe"));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
    }

    // ------------------------------------------- requests and their rules

    const ANIM: i32 = 0x10;
    const ATTACH: i32 = 0x20;
    const CONTROLS: i32 = 0x4;

    fn ball() -> Uuid {
        Uuid::from_bytes([0xBA; 16])
    }

    fn other_ball() -> Uuid {
        Uuid::from_bytes([0xBB; 16])
    }

    fn john() -> Uuid {
        Uuid::from_bytes([9; 16])
    }

    fn group() -> Uuid {
        Uuid::from_bytes([0x66; 16])
    }

    /// The script in an object: one per object here.
    fn script_of(object: Uuid) -> Uuid {
        Uuid::from_u128(object.as_u128() ^ 0xFF)
    }

    fn transaction(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn notecards() -> Uuid {
        Uuid::from_bytes([0x70; 16])
    }

    impl Rig {
        fn perms(&mut self, command: &str, kind: &str, who: &[&str]) -> Receiver<Response> {
            let (kind, who) = (kind.to_string(), Rig::words(who));
            self.command(Request::Perms(match command {
                "autoaccept" => PermsRequest::Autoaccept { kind, who },
                _ => PermsRequest::Revoke { kind, who },
            }))
        }

        fn allow(&mut self, kind: &str, who: &[&str]) {
            ok(&self.perms("autoaccept", kind, who));
            self.told();
        }

        fn perms_status(&mut self) -> Vec<String> {
            let status = ok(&self.command(Request::Perms(PermsRequest::Status))).unwrap();
            status.lines().map(str::to_string).collect()
        }

        /// The requests that wait, as `perms status` shows them.
        fn waiting(&mut self) -> Vec<String> {
            self.perms_status()
                .into_iter()
                .filter(|line| line.starts_with("waiting:"))
                .collect()
        }

        /// A script in `object`, which says Jane Doe owns it, asks.
        fn asks(&mut self, object: Uuid, permissions: i32) {
            let item = script_of(object);
            self.commands
                .permission_request(object, item, "Dance Ball", "Jane Doe", permissions, Some(&mut self.grid), self.now);
        }

        /// The simulator says who owns `object`.
        fn owned(&mut self, object: Uuid, owner: Uuid, group: Uuid) {
            self.commands.object_owner(object, owner, group, Some(&mut self.grid), self.now);
        }

        /// A script in an object of Jane Doe's asks, and the simulator
        /// confirms the owner.
        fn janes_asks(&mut self, object: Uuid, permissions: i32) {
            self.asks(object, permissions);
            self.owned(object, jane(), Uuid::nil());
            self.grid.calls.retain(|call| !matches!(call, Call::OwnerOf(_)));
        }

        fn gives(&mut self, from: Uuid, name: &str, from_object: bool, from_group: bool, byte: u8) {
            let offer = InventoryOffer {
                from,
                name: name.to_string(),
                from_group,
                from_object,
                transaction: transaction(byte),
                asset_type: inventory::ASSET_NOTECARD,
                text: "Rules".to_string(),
            };
            self.commands.inventory_offer(offer, Some(&mut self.grid), self.now);
        }

        /// The inventory worker says where notecards go.
        fn files(&mut self) {
            let (id, what) = self.job();
            assert_eq!(what, What::SystemFolder(inventory::ASSET_NOTECARD));
            self.answer(id, Ok(Outcome::Folder(notecards())));
        }

        fn janes_name(&mut self) {
            let names = Names {
                legacy: "Jane Doe".to_string(),
                display: "Jane".to_string(),
            };
            self.commands
                .names_resolved(&[(jane(), names)], &[], Some(&mut self.grid), self.now);
        }

        fn decide_late(&mut self) {
            self.commands.requests_due(Some(&mut self.grid), self.now);
        }
    }

    #[test]
    fn a_teleport_offer_from_a_listed_avatar_is_accepted_by_itself() {
        let mut rig = Rig::new();
        rig.allow("tp", &["Jane Doe"]);
        rig.offer(jane(), "Jane Doe", lure(1));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
        assert!(rig.commands.take_manual(), "as a command that moves the avatar");
        assert_eq!(
            rig.told(),
            [format!("teleport offer from Jane Doe <{}>: accepted by itself", jane())]
        );
        assert_eq!(rig.waiting(), [] as [&str; 0]);

        // Anybody else's waits, and the bot stays as it is.
        rig.offer(john(), "John Roe", lure(5));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(
            rig.told(),
            [format!(
                "teleport offer from John Roe <{}>: waits; `rslbot ctl accept tp` accepts it",
                john()
            )]
        );
        assert_eq!(rig.waiting(), [format!("waiting: tp from John Roe <{}>", john())]);

        // A rule made later does not accept what waits already.
        rig.allow("tp", &["John Roe"]);
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(rig.waiting().len(), 1);
        rig.accept("tp", None);
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(5))]);
    }

    #[test]
    fn accept_tp_takes_the_latest_offer_or_the_named_avatars() {
        let mut rig = Rig::new();
        assert_eq!(error(&rig.accept("tp", None)), "no teleport offer waits");
        rig.offer(john(), "John Roe", lure(5));
        rig.offer(jane(), "Jane Doe", lure(1));
        rig.told();

        // A name the grid may not know is compared, not looked up.
        assert_eq!(
            error(&rig.accept("tp", Some("Nobody Here"))),
            "there is no teleport offer from Nobody Here"
        );
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());

        // Without a name: the one that came last. The other still waits.
        let answer = rig.accept("tp", None);
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
        assert!(rig.commands.take_manual());
        assert_eq!(
            rig.told(),
            [format!("control: accept tp: teleport offer from Jane Doe <{}>: accepted", jane())]
        );
        rig.commands.arrived("Elsewhere");
        assert_eq!(ok(&answer), Some("arrived in Elsewhere".to_string()));
        assert_eq!(rig.waiting(), [format!("waiting: tp from John Roe <{}>", john())]);

        // By UUID.
        rig.accept("tp", Some(&john().to_string()));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(5))]);
        assert_eq!(error(&rig.accept("tp", None)), "no teleport offer waits");
    }

    #[test]
    fn an_exception_does_not_stop_an_offer_the_bot_asked_for() {
        let mut rig = Rig::new();
        rig.allow("tp", &["all"]);
        ok(&rig.perms("revoke", "tp", &["Jane Doe"]));
        rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        });
        rig.calls();
        rig.told();
        rig.offer(jane(), "Jane Doe", lure(1));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
        assert_eq!(
            rig.told(),
            [format!("teleport offer from Jane Doe <{}>: accepted, as requested", jane())]
        );
        // Unasked, the exception holds, and everyone else is accepted.
        rig.offer(jane(), "Jane Doe", lure(2));
        assert_eq!(rig.calls(), []);
        rig.offer(john(), "John Roe", lure(5));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(5))]);
    }

    #[test]
    fn an_offer_the_rules_accept_waits_when_it_cannot_be_accepted_now() {
        let mut rig = Rig::new();
        rig.allow("tp", &["all"]);
        rig.grid.offline = true;
        rig.offer(jane(), "Jane Doe", lure(1));
        assert!(!rig.commands.take_manual());
        let told = rig.told();
        assert!(
            told[0].contains("cannot be accepted now") && told[0].contains("`rslbot ctl accept tp`"),
            "{told:?}"
        );
        rig.grid.offline = false;
        rig.accept("tp", None);
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
    }

    #[test]
    fn a_permission_request_is_decided_once_the_owner_is_known() {
        let mut rig = Rig::new();
        rig.allow("anim", &["Jane Doe"]);
        rig.asks(ball(), ANIM);
        // First the simulator is asked who owns the object.
        assert_eq!(rig.calls(), [Call::OwnerOf(ball())]);
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + LOOKUP_LIMIT));
        assert_eq!(rig.told(), [] as [&str; 0]);
        rig.owned(ball(), jane(), Uuid::nil());
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ANIM)]);
        assert_eq!(
            rig.told(),
            [format!(
                "permission request (anim) from Dance Ball <{}> of Jane Doe <{}>: accepted by itself",
                ball(),
                jane()
            )]
        );
        assert!(!rig.commands.take_manual(), "nothing moved the avatar");
        assert_eq!(rig.commands.next_deadline(), None);

        // The owner's UUID does as well as the name, for another kind.
        rig.allow("attach", &[&jane().to_string()]);
        rig.janes_asks(other_ball(), ATTACH);
        assert_eq!(rig.calls(), [Call::Grant(other_ball(), script_of(other_ball()), ATTACH)]);
        // And the object's own UUID, whoever owns it.
        rig.allow("controls", &[&ball().to_string()]);
        rig.asks(ball(), CONTROLS);
        rig.owned(ball(), john(), Uuid::nil());
        assert!(rig.calls().contains(&Call::Grant(ball(), script_of(ball()), CONTROLS)));
    }

    #[test]
    fn a_group_with_an_avatars_name_is_not_the_avatar() {
        let mut rig = Rig::new();
        rig.allow("anim", &["Jane Doe", &jane().to_string()]);
        // Deeded: no owner, or the group as the owner.
        for owner in [Uuid::nil(), group()] {
            rig.asks(ball(), ANIM);
            rig.owned(ball(), owner, group());
            assert_eq!(rig.calls(), [Call::OwnerOf(ball())], "nothing is granted");
            let told = rig.told();
            assert!(told[0].contains(&format!("Dance Ball <{}> of a group: waits", ball())), "{told:?}");
        }
        // Set to a group and owned by an avatar is the avatar's.
        rig.asks(other_ball(), ANIM);
        rig.owned(other_ball(), jane(), group());
        assert!(rig.calls().contains(&Call::Grant(other_ball(), script_of(other_ball()), ANIM)));
    }

    #[test]
    fn without_the_simulators_word_only_the_object_itself_matches() {
        let mut rig = Rig::new();
        rig.allow("anim", &["Jane Doe"]);
        rig.asks(ball(), ANIM);
        rig.calls();
        rig.now += LOOKUP_LIMIT - Duration::from_millis(1);
        rig.decide_late();
        assert_eq!(rig.waiting(), [] as [&str; 0], "still undecided");
        rig.now += Duration::from_millis(1);
        rig.decide_late();
        assert_eq!(rig.calls(), []);
        let told = rig.told();
        assert!(told[0].contains("said to be of Jane Doe (not confirmed): waits"), "{told:?}");
        assert_eq!(rig.waiting().len(), 1);
        // An answer that comes after all changes nothing.
        rig.owned(ball(), jane(), Uuid::nil());
        assert_eq!(rig.calls(), []);

        // The object's UUID needs no owner.
        rig.allow("anim", &[&other_ball().to_string()]);
        rig.asks(other_ball(), ANIM);
        rig.calls();
        rig.now += LOOKUP_LIMIT;
        rig.decide_late();
        assert_eq!(rig.calls(), [Call::Grant(other_ball(), script_of(other_ball()), ANIM)]);

        // A simulator that cannot be asked: decided at once, the same way.
        rig.grid.offline = true;
        rig.asks(other_ball(), ANIM);
        assert_eq!(rig.commands.next_deadline(), None);
        assert!(rig.told().last().unwrap().contains("cannot be accepted now"));
    }

    #[test]
    fn a_request_with_two_kinds_is_accepted_whole_or_waits_whole() {
        let mut rig = Rig::new();
        rig.allow("anim", &["all"]);
        rig.janes_asks(ball(), ANIM | CONTROLS);
        assert_eq!(rig.calls(), [], "neither permission is granted");
        let told = rig.told();
        assert!(
            told[0].contains("permission request (anim, controls)")
                && told[0].contains("waits, as controls is not auto-accepted for it; `rslbot ctl accept anim` accepts it"),
            "{told:?}"
        );
        assert_eq!(
            rig.waiting(),
            [format!(
                "waiting: anim, controls from Dance Ball <{}> of Jane Doe <{}>",
                ball(),
                jane()
            )]
        );

        // Found under either kind; accepting grants everything it asked.
        let answer = rig.accept("controls", None);
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ANIM | CONTROLS)]);
        assert_eq!(
            ok(&answer),
            Some(format!(
                "granted Dance Ball <{}> of Jane Doe <{}>: take controls, trigger animations",
                ball(),
                jane()
            ))
        );
        assert!(!rig.commands.take_manual());
        assert_eq!(error(&rig.accept("anim", None)), "no request of kind `anim` waits");

        // With both kinds auto-accepted the request is.
        rig.allow("controls", &["all"]);
        rig.janes_asks(ball(), ANIM | CONTROLS);
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ANIM | CONTROLS)]);
    }

    #[test]
    fn an_exception_is_not_accepted_under_all_or_under_its_owner() {
        let mut rig = Rig::new();
        rig.allow("anim", &["all"]);
        ok(&rig.perms("revoke", "anim", &[&ball().to_string()]));
        rig.janes_asks(ball(), ANIM);
        assert_eq!(rig.calls(), []);
        rig.janes_asks(other_ball(), ANIM);
        assert_eq!(rig.calls(), [Call::Grant(other_ball(), script_of(other_ball()), ANIM)]);
        // Taken off the exceptions, it is accepted again.
        rig.allow("anim", &[&ball().to_string()]);
        rig.janes_asks(ball(), ANIM);
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ANIM)]);
    }

    #[test]
    fn a_request_for_money_is_never_accepted() {
        let mut rig = Rig::new();
        for kind in ["anim", "attach", "controls", "camera"] {
            rig.allow(kind, &["all"]);
        }
        rig.asks(ball(), 0x2 | ANIM);
        assert_eq!(rig.calls(), [], "not even the owner is asked for");
        let told = rig.told();
        assert!(
            told[0].contains("cannot be accepted: it asks for permission to take money from the avatar"),
            "{told:?}"
        );
        assert_eq!(rig.waiting(), [] as [&str; 0]);
        assert_eq!(error(&rig.accept("anim", None)), "no request of kind `anim` waits");
        // Nor a request for nothing.
        rig.asks(ball(), 0);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_script_that_asks_again_has_one_request_waiting() {
        let mut rig = Rig::new();
        // Twice before the owner is known, and once more after.
        rig.asks(ball(), ANIM);
        rig.asks(ball(), ANIM);
        rig.owned(ball(), jane(), Uuid::nil());
        assert_eq!(rig.waiting().len(), 1);
        rig.janes_asks(ball(), ANIM | CONTROLS);
        assert_eq!(
            rig.waiting(),
            [format!(
                "waiting: anim, controls from Dance Ball <{}> of Jane Doe <{}>",
                ball(),
                jane()
            )]
        );
        // Another object's is another request.
        rig.janes_asks(other_ball(), ANIM);
        assert_eq!(rig.waiting().len(), 2);
        // A newer request that is accepted closes the one that waited, and
        // so does one that can never be.
        rig.allow("attach", &["all"]);
        rig.janes_asks(ball(), ATTACH);
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ATTACH)]);
        assert_eq!(rig.waiting().len(), 1);
        rig.asks(other_ball(), 0x2);
        assert_eq!(rig.waiting(), [] as [&str; 0]);
    }

    #[test]
    fn accept_finds_a_request_by_its_object_or_its_owner() {
        let mut rig = Rig::new();
        rig.janes_asks(ball(), ANIM);
        rig.janes_asks(other_ball(), ANIM);
        // And an object of John Roe's.
        let johns = Uuid::from_bytes([0xBC; 16]);
        rig.commands
            .permission_request(johns, script_of(johns), "Lamp", "John Roe", ATTACH, Some(&mut rig.grid), rig.now);
        rig.owned(johns, john(), Uuid::nil());
        rig.calls();
        rig.told();

        // The object that asked first, though another asked later.
        ok(&rig.accept("anim", Some(&ball().to_string())));
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ANIM)]);
        assert_eq!(rig.waiting().len(), 2);
        // An owner by name; not one who owns nothing that asked for it.
        assert_eq!(
            error(&rig.accept("attach", Some("Jane Doe"))),
            "no request of kind `attach` from Jane Doe waits"
        );
        ok(&rig.accept("anim", Some("jane.doe")));
        assert_eq!(rig.calls(), [Call::Grant(other_ball(), script_of(other_ball()), ANIM)]);
        // Used once.
        assert_eq!(error(&rig.accept("anim", None)), "no request of kind `anim` waits");
        let told = rig.told();
        assert!(
            told[0].starts_with("control: accept anim: permission request (anim) from Dance Ball"),
            "{told:?}"
        );
    }

    #[test]
    fn accept_is_refused_before_anything_is_looked_for() {
        let mut rig = Rig::new();
        rig.janes_asks(ball(), ANIM);
        let unknown = error(&rig.accept("money", None));
        assert!(
            unknown.contains("`money` is no kind of request") && unknown.contains("`camera`"),
            "{unknown}"
        );
        assert!(error(&rig.accept("anim", Some("Jane van Doe"))).contains("neither a UUID nor"));
        // Not connected: the request still waits afterwards.
        let (reply, answer) = mpsc::channel();
        let request = Request::Accept {
            kind: "anim".to_string(),
            who: None,
        };
        rig.commands.handle(request, reply, None::<&mut FakeGrid>, rig.now);
        assert_eq!(error(&answer), NOT_CONNECTED);
        rig.grid.offline = true;
        assert_eq!(error(&rig.accept("anim", None)), NOT_CONNECTED);
        rig.grid.offline = false;
        assert_eq!(rig.waiting().len(), 1);
        ok(&rig.accept("anim", None));
    }

    #[test]
    fn an_inventory_offer_is_accepted_into_the_folder_for_its_type() {
        let mut rig = Rig::new();
        rig.allow("inventory", &["all"]);
        rig.gives(jane(), "Jane Doe", false, false, 1);
        // Nothing is answered before the folder is known.
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.told(), [] as [&str; 0]);
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(jane(), false, transaction(1), notecards())]);
        assert_eq!(
            rig.told(),
            [format!(
                "inventory offer from Jane Doe <{}> (notecard Rules): accepted by itself",
                jane()
            )]
        );
        assert!(!rig.commands.take_manual());
        assert_eq!(rig.waiting(), [] as [&str; 0]);
    }

    #[test]
    fn an_inventory_offer_that_waits_asks_the_grid_nothing_until_it_is_accepted() {
        let mut rig = Rig::new();
        for byte in 1..=3 {
            rig.gives(jane(), "Jane Doe", false, false, byte);
        }
        rig.gives(john(), "John Roe", false, false, 9);
        assert!(rig.grid.jobs.is_empty(), "the inventory is not read for an offer that waits");
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.waiting().len(), 4, "every offer waits");
        assert_eq!(
            rig.told()[0],
            format!(
                "inventory offer from Jane Doe <{}> (notecard Rules): waits; `rslbot ctl accept inventory` accepts it",
                jane()
            )
        );

        // The latest of the named avatar.
        let answer = rig.accept("inventory", Some("Jane Doe"));
        assert!(answer.try_recv().is_err(), "not answered before the answer has gone out");
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(jane(), false, transaction(3), notecards())]);
        assert_eq!(ok(&answer), Some(format!("accepted notecard Rules from Jane Doe <{}>", jane())));
        assert_eq!(rig.waiting().len(), 3);
        assert!(rig.told()[0].starts_with("control: accept inventory: inventory offer from Jane Doe"));
        // The latest of all.
        rig.accept("inventory", None);
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(john(), false, transaction(9), notecards())]);
        assert_eq!(
            error(&rig.accept("inventory", Some("John Roe"))),
            "no request of kind `inventory` from John Roe waits"
        );
    }

    #[test]
    fn an_offer_whose_folder_cannot_be_found_keeps_waiting() {
        let mut rig = Rig::new();
        rig.allow("inventory", &["all"]);
        // No inventory service: nothing is answered, and the log says why.
        rig.grid.no_inventory = true;
        rig.gives(jane(), "Jane Doe", false, false, 1);
        assert_eq!(rig.calls(), []);
        let told = rig.told();
        assert!(
            told[0].contains(&format!("cannot be accepted now: {}", inventory::NO_SERVICE))
                && told[0].contains("`rslbot ctl accept inventory` accepts it"),
            "{told:?}"
        );
        assert_eq!(error(&rig.accept("inventory", None)), inventory::NO_SERVICE);
        assert_eq!(rig.waiting().len(), 1);

        // The grid does not answer the command's question.
        rig.grid.no_inventory = false;
        let answer = rig.accept("inventory", None);
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::TimedOut));
        assert!(error(&answer).contains("did not say within 50 s which folder"));
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.waiting().len(), 1);

        // Nor the bot's own, for an offer it was about to accept.
        rig.gives(john(), "John Roe", false, false, 2);
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::Reason("the inventory request failed".to_string())));
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.waiting().len(), 2);
        // A worker that never answers is given up on.
        let answer = rig.accept("inventory", None);
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + inventory::LIMIT + JOB_GRACE));
        rig.commands.tick(rig.now + inventory::LIMIT + JOB_GRACE);
        assert!(error(&answer).contains("which folder"));
        assert_eq!(rig.waiting().len(), 2);
        // A change of simulator while the folder is looked for.
        rig.gives(john(), "John Roe", false, false, 3);
        rig.commands.simulator_changed();
        assert_eq!(rig.waiting().len(), 3);
        // And the end of the session, which also ends the waiting.
        let answer = rig.accept("inventory", None);
        rig.commands.session_lost();
        assert!(error(&answer).contains("session was lost"));
        assert_eq!(rig.waiting(), [] as [&str; 0]);
    }

    #[test]
    fn an_objects_offer_is_matched_by_its_owner() {
        let mut rig = Rig::new();
        rig.allow("inventory", &["Jane Doe"]);
        // The offer names the owner by UUID: the name is looked up first.
        rig.gives(jane(), "Vendor", true, false, 1);
        assert_eq!(rig.commands.take_name_lookups(), [jane()]);
        assert_eq!(rig.commands.take_name_lookups(), Vec::<Uuid>::new(), "asked for once");
        assert!(rig.grid.jobs.is_empty());
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + LOOKUP_LIMIT));
        rig.janes_name();
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(jane(), true, transaction(1), notecards())]);
        assert_eq!(
            rig.told(),
            [format!(
                "inventory offer from the object \"Vendor\" of Jane Doe <{}> (notecard Rules): accepted by itself",
                jane()
            )]
        );

        // A name the grid does not give in time: the offer waits.
        rig.gives(jane(), "Vendor", true, false, 2);
        rig.now += LOOKUP_LIMIT;
        rig.decide_late();
        assert!(rig.grid.jobs.is_empty());
        assert_eq!(
            rig.waiting(),
            [format!(
                "waiting: inventory from the object \"Vendor\" of the avatar {}: notecard Rules",
                jane()
            )]
        );
        // Nor one the grid says it does not know, or cannot look up.
        rig.gives(jane(), "Vendor", true, false, 3);
        rig.commands.names_resolved(&[], &[jane()], Some(&mut rig.grid), rig.now);
        rig.gives(jane(), "Vendor", true, false, 4);
        rig.commands.names_failed(&[jane()], Some(&mut rig.grid), rig.now);
        assert_eq!(rig.waiting().len(), 3);
        assert!(rig.grid.jobs.is_empty());
        // The operator can still name the owner by UUID.
        rig.accept("inventory", Some(&jane().to_string()));
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(jane(), true, transaction(4), notecards())]);
    }

    #[test]
    fn an_objects_offer_is_also_matched_by_its_owners_uuid() {
        let mut rig = Rig::new();
        rig.allow("inventory", &[&jane().to_string(), &ball().to_string()]);
        // The name is asked for all the same; the UUID does without it.
        rig.gives(jane(), "Vendor", true, false, 1);
        assert_eq!(rig.commands.take_name_lookups(), [jane()]);
        rig.commands.names_failed(&[jane()], Some(&mut rig.grid), rig.now);
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(jane(), true, transaction(1), notecards())]);
        // The UUID of an object stands for nobody who gives inventory.
        rig.gives(john(), "Dance Ball", true, false, 2);
        rig.commands.take_name_lookups();
        rig.commands.names_failed(&[john()], Some(&mut rig.grid), rig.now);
        assert!(rig.grid.jobs.is_empty());
        // A group's object has no avatar to look up, and is matched by
        // nothing but `all`.
        rig.gives(jane(), "Vendor", true, true, 3);
        assert_eq!(rig.commands.take_name_lookups(), Vec::<Uuid>::new());
        assert!(rig.grid.jobs.is_empty());
        assert_eq!(rig.waiting().len(), 2);
        rig.allow("inventory", &["all"]);
        rig.gives(group(), "Vendor", true, true, 4);
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(group(), true, transaction(4), notecards())]);
    }

    #[test]
    fn an_objects_offer_that_waits_can_be_accepted_by_its_owners_name() {
        let mut rig = Rig::new();
        rig.gives(jane(), "Vendor", true, false, 1);
        assert_eq!(rig.commands.take_name_lookups(), [jane()]);
        rig.janes_name();
        assert_eq!(
            rig.waiting(),
            [format!(
                "waiting: inventory from the object \"Vendor\" of Jane Doe <{}>: notecard Rules",
                jane()
            )]
        );
        assert_eq!(
            error(&rig.accept("inventory", Some("John Roe"))),
            "no request of kind `inventory` from John Roe waits"
        );
        rig.accept("inventory", Some("Jane Doe"));
        rig.files();
        assert_eq!(rig.calls(), [Call::AcceptInventory(jane(), true, transaction(1), notecards())]);
    }

    #[test]
    fn waiting_requests_end_with_the_session_and_scripts_with_the_region() {
        let mut rig = Rig::new();
        rig.offer(jane(), "Jane Doe", lure(1));
        rig.gives(jane(), "Jane Doe", false, false, 1);
        rig.janes_asks(ball(), ANIM);
        rig.asks(other_ball(), ANIM);
        assert_eq!(rig.waiting().len(), 3);
        // Another simulator: the scripts stayed behind, decided or not.
        rig.commands.simulator_changed();
        assert_eq!(rig.waiting().len(), 2);
        rig.owned(other_ball(), jane(), Uuid::nil());
        assert_eq!(rig.waiting().len(), 2);
        assert_eq!(rig.commands.next_deadline(), None);
        rig.commands.session_lost();
        assert_eq!(rig.waiting(), [] as [&str; 0]);
    }

    #[test]
    fn perms_commands_change_the_rules_and_warn_when_nothing_changes() {
        let mut rig = Rig::new();
        let object = "01234567-9abc-defe-dcba-987654321000";
        let added = rig.perms("autoaccept", "inventory", &[object, "Avatar1"]);
        assert_eq!(
            ok(&added),
            Some(format!("inventory: {object} is auto-accepted\ninventory: Avatar1 is auto-accepted"))
        );
        assert_eq!(
            rig.told(),
            [
                format!("control: perms: inventory: {object} is auto-accepted"),
                "control: perms: inventory: Avatar1 is auto-accepted".to_string()
            ]
        );
        assert_eq!(rig.perms_status()[1], format!("inventory: {object}, Avatar1"));
        // Already there: a warning, and still a success.
        assert_eq!(
            ok(&rig.perms("autoaccept", "inventory", &["Avatar1"])),
            Some("warning: Avatar1 was auto-accepted for inventory already".to_string())
        );
        assert_eq!(rig.perms_status()[1], format!("inventory: {object}, Avatar1"));

        // Revoking one, then one that is not there, then everything.
        assert_eq!(
            ok(&rig.perms("revoke", "inventory", &[object])),
            Some(format!("inventory: {object} is not auto-accepted any more"))
        );
        assert_eq!(
            ok(&rig.perms("revoke", "inventory", &[object])),
            Some(format!("warning: {object} was not auto-accepted for inventory already"))
        );
        assert_eq!(
            ok(&rig.perms("revoke", "inventory", &["all"])),
            Some("inventory: nothing is auto-accepted any more".to_string())
        );
        assert_eq!(rig.perms_status()[1], "inventory: none");
        rig.told();

        // Everyone, and then an exception to it.
        assert_eq!(
            ok(&rig.perms("autoaccept", "anim", &["all"])),
            Some("anim: everyone is auto-accepted".to_string())
        );
        ok(&rig.perms("revoke", "anim", &[object]));
        assert_eq!(rig.perms_status()[2], format!("anim: all except {object}"));
        ok(&rig.perms("autoaccept", "anim", &[object]));
        assert_eq!(rig.perms_status()[2], "anim: all");
        ok(&rig.perms("revoke", "anim", &["all"]));
        assert_eq!(
            rig.told().last().unwrap(),
            "control: perms: anim: nothing is auto-accepted any more"
        );
    }

    #[test]
    fn malformed_perms_commands_change_nothing() {
        let mut rig = Rig::new();
        let unknown = error(&rig.perms("autoaccept", "money", &["all"]));
        assert!(
            unknown.contains("`money` is no kind of request") && unknown.contains("`tp`, `inventory`"),
            "{unknown}"
        );
        assert!(error(&rig.perms("autoaccept", "anim", &["all", "Avatar1"])).contains("`all` stands alone"));
        assert!(error(&rig.perms("revoke", "anim", &["Avatar1", "All"])).contains("`all` stands alone"));
        // One bad name: the good one before it is not added either.
        assert!(error(&rig.perms("autoaccept", "anim", &["Avatar1", "Jane van Doe"])).contains("neither a UUID nor"));
        assert!(error(&rig.perms("autoaccept", "anim", &[])).contains("say whom"));
        assert_eq!(
            rig.perms_status(),
            [
                "tp: none",
                "inventory: none",
                "anim: none",
                "attach: none",
                "controls: none",
                "camera: none"
            ]
        );
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn perms_commands_need_no_session() {
        let mut commands = Commands::default();
        let now = Instant::now();
        let run = |commands: &mut Commands, request| {
            let (reply, answer) = mpsc::channel();
            commands.handle(request, reply, None::<&mut FakeGrid>, now);
            ok(&answer).unwrap()
        };
        let added = run(
            &mut commands,
            Request::Perms(PermsRequest::Autoaccept {
                kind: "tp".to_string(),
                who: vec!["Jane Doe".to_string()],
            }),
        );
        assert_eq!(added, "tp: Jane Doe is auto-accepted");
        let status = run(&mut commands, Request::Perms(PermsRequest::Status));
        assert!(status.starts_with("tp: Jane Doe\ninventory: none\n"), "{status}");
        assert_eq!(status.lines().count(), 6);
        // The rules outlive a session.
        commands.session_lost();
        assert!(run(&mut commands, Request::Perms(PermsRequest::Status)).starts_with("tp: Jane Doe\n"));
    }

    #[test]
    fn the_configs_rules_are_what_the_bot_starts_with() {
        let mut rig = Rig::new();
        let mut rules = Rules::default();
        rules.set_rule(Kind::Tp, perms::Rule::List(vec![perms::Entry::parse("Jane Doe").unwrap()]));
        rules.set_rule(Kind::Anim, perms::Rule::All);
        rules.set_exceptions(
            Kind::Anim,
            vec![perms::Entry::parse("01234567-9abc-defe-dcba-987654321000").unwrap()],
        );
        rig.commands.set_rules(rules);
        assert_eq!(
            rig.perms_status(),
            [
                "tp: Jane Doe",
                "inventory: none",
                "anim: all except 01234567-9abc-defe-dcba-987654321000",
                "attach: none",
                "controls: none",
                "camera: none"
            ]
        );
        rig.offer(jane(), "Jane Doe", lure(1));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(1))]);
    }

    // --------------------------------------------------------------- touch

    #[test]
    fn touch_answers_at_once_for_a_known_object() {
        let mut rig = Rig::new();
        assert_eq!(ok(&rig.command(Request::Touch(other_object().to_string()))), None);
        assert_eq!(rig.calls(), [Call::Touch(other_object())]);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn touch_waits_for_a_scan_and_reports_how_it_ended() {
        let mut rig = Rig::new();
        rig.grid.touch = Ok(Touch::Waiting);
        let found = rig.command(Request::Touch(other_object().to_string()));
        let missing = rig.command(Request::Touch(jane().to_string()));
        assert!(found.try_recv().is_err() && missing.try_recv().is_err());
        rig.commands.touched(other_object());
        assert_eq!(ok(&found), None);
        assert!(missing.try_recv().is_err());
        rig.commands.touch_failed(jane(), "object is not known to the bot");
        assert!(error(&missing).contains("not known"));

        let lost = rig.command(Request::Touch(jane().to_string()));
        rig.commands.session_lost();
        assert!(error(&lost).contains("session was lost"));
    }

    #[test]
    fn touch_is_refused_when_the_object_is_unknown() {
        let mut rig = Rig::new();
        rig.grid.touch = Err("object is not known to the bot".to_string());
        assert!(error(&rig.command(Request::Touch(other_object().to_string()))).contains("not known"));
    }

    // ------------------------------------------------------- the inventory

    use crate::inventory::{Ref, ASSET_ANIMATION, ASSET_BODY_PART, ASSET_CLOTHING, ASSET_NOTECARD, ASSET_OBJECT, ASSET_SOUND};

    /// An inventory item of the tests: its id and its asset's follow from
    /// its name.
    fn item(name: &str, asset_type: i32, flags: u32) -> Item {
        let seed = name.bytes().fold(7u128, |sum, byte| sum * 131 + u128::from(byte));
        Item {
            id: Uuid::from_u128(seed),
            parent: Uuid::from_u128(1),
            name: name.to_string(),
            description: format!("{name}, described"),
            asset_type,
            inv_type: asset_type,
            flags,
            asset_id: Uuid::from_u128(seed + 1),
            owner: Uuid::from_u128(0xA1),
            group_mask: 0,
            everyone_mask: 0x8000,
            next_owner_mask: 0x0008_2000,
        }
    }

    fn radio() -> Item {
        item("Radio", ASSET_OBJECT, 5)
    }

    fn hat() -> Item {
        item("Hat", ASSET_OBJECT, 2)
    }

    fn watch() -> Item {
        item("Watch", ASSET_OBJECT, 5)
    }

    fn dance() -> Item {
        item("Dance", ASSET_ANIMATION, 0)
    }

    fn bell() -> Item {
        item("Bell", ASSET_SOUND, 0)
    }

    fn shirt() -> Item {
        item("Blue shirt", ASSET_CLOTHING, 4)
    }

    fn path(names: &[&str]) -> Ref {
        Ref::Path(names.iter().map(|name| name.to_string()).collect())
    }

    fn entry(item: &Item, path: &str) -> Entry {
        Entry {
            id: item.id,
            asset_type: item.asset_type,
            flags: item.flags,
            path: path.to_string(),
        }
    }

    impl Rig {
        /// The one inventory job handed over since the last call.
        fn job(&mut self) -> (u64, What) {
            match &std::mem::take(&mut self.grid.jobs)[..] {
                [(id, _, what)] => (*id, what.clone()),
                other => panic!("expected one inventory job, got {other:?}"),
            }
        }

        fn answer(&mut self, id: u64, result: Result<Outcome, Failed>) {
            self.commands.inventory_answer(id, result, Some(&mut self.grid), self.now);
        }

        /// Runs a command that names an item and answers its lookup with
        /// `found`.
        fn with_item(&mut self, request: Request, found: &Item) -> Receiver<Response> {
            let answer = self.command(request);
            let (id, what) = self.job();
            assert!(matches!(what, What::Find(_)), "{what:?}");
            self.answer(id, Ok(Outcome::Found(found.clone())));
            answer
        }

        fn attach(&mut self, item: &str, part: Option<&str>) -> Request {
            let _ = self;
            Request::Attach {
                item: item.to_string(),
                part: part.map(str::to_string),
            }
        }

        fn sim_attached(&mut self, item: &Item, point: u8) -> bool {
            self.grid.attached.push((item.id, point));
            self.commands.attached(item.id, point, Some(&mut self.grid), self.now)
        }

        fn sim_detached(&mut self, item: &Item) -> bool {
            self.grid.attached.retain(|(id, _)| *id != item.id);
            self.commands.detached(item.id, Some(&mut self.grid), self.now)
        }

        fn inventory(&mut self, args: &[&str]) -> Receiver<Response> {
            self.command(Request::Inventory(Rig::words(args)))
        }
    }

    fn not_answered(answer: &Receiver<Response>) {
        assert!(answer.try_recv().is_err(), "answered too early");
    }

    #[test]
    fn inventory_is_listed_when_the_worker_has_answered() {
        let mut rig = Rig::new();
        let answer = rig.inventory(&[]);
        not_answered(&answer);
        // The whole of My Inventory, with 50 s for the grid.
        assert_eq!(rig.grid.jobs[0].1, rig.now + inventory::LIMIT);
        let (id, what) = rig.job();
        assert_eq!(
            what,
            What::List {
                folder: path(&[]),
                recursive: true
            }
        );
        rig.grid.attached = vec![(radio().id, 6)];
        let entries = vec![entry(&radio(), "Objects/Radio"), entry(&hat(), "Objects/Hat")];
        rig.answer(id, Ok(Outcome::Listed(entries)));
        let text = ok(&answer).unwrap();
        // In the order of the paths; the attached one where it is now.
        assert_eq!(
            text,
            format!(
                "{}\tobject\tSkull\t-\tObjects/Hat\n{}\tobject\tRight Hand\tattached\tObjects/Radio",
                hat().id,
                radio().id
            )
        );
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(rig.commands.next_deadline(), None);
    }

    #[test]
    fn inventory_passes_its_folder_and_filters_on() {
        let mut rig = Rig::new();
        let answer = rig.inventory(&["-n", "Objects/Bot gear", "-a"]);
        let (id, what) = rig.job();
        assert_eq!(
            what,
            What::List {
                folder: path(&["Objects", "Bot gear"]),
                recursive: false
            }
        );
        rig.grid.attached = vec![(radio().id, 5)];
        let entries = vec![entry(&radio(), "Objects/Bot gear/Radio"), entry(&hat(), "Objects/Bot gear/Hat")];
        rig.answer(id, Ok(Outcome::Listed(entries.clone())));
        // Only what is attached now.
        let text = ok(&answer).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with("\tattached\tObjects/Bot gear/Radio"), "{text}");
        // Nothing to list is success without text.
        let answer = rig.inventory(&["-a", "Clothing"]);
        let (id, _) = rig.job();
        rig.answer(id, Ok(Outcome::Listed(vec![entry(&shirt(), "Clothing/Blue shirt")])));
        assert_eq!(ok(&answer), None);
        let answer = rig.inventory(&["Empty"]);
        let (id, _) = rig.job();
        rig.answer(id, Ok(Outcome::Listed(vec![])));
        assert_eq!(ok(&answer), None);
        // A folder by its id.
        rig.inventory(&[&hat().id.to_string()]);
        assert!(matches!(rig.job().1, What::List { folder: Ref::Id(id), recursive: true } if id == hat().id));
    }

    #[test]
    fn inventory_is_refused_before_the_grid_is_asked() {
        let mut rig = Rig::new();
        assert!(error(&rig.inventory(&["-x"])).contains("`-x`"));
        assert!(error(&rig.inventory(&["Objects", "Clothing"])).contains("one folder"));
        assert_eq!(rig.grid.jobs.len(), 0);
        // A simulator without the service.
        rig.grid.no_inventory = true;
        assert_eq!(error(&rig.inventory(&[])), inventory::NO_SERVICE);
        assert_eq!(error(&rig.command(Request::Sound("Sounds/Bell".into()))), inventory::NO_SERVICE);
        rig.grid.no_inventory = false;
        // Not in a region: teleporting, logging out, or between logins.
        rig.grid.offline = true;
        assert_eq!(error(&rig.inventory(&[])), NOT_CONNECTED);
        let (reply, answer) = mpsc::channel();
        let request = Request::Inventory(vec![]);
        rig.commands.handle(request, reply, None::<&mut FakeGrid>, rig.now);
        assert_eq!(error(&answer), NOT_CONNECTED);
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.commands.next_deadline(), None);
    }

    #[test]
    fn inventory_reports_what_the_worker_could_not_do() {
        let mut rig = Rig::new();
        let answer = rig.inventory(&["Nowhere"]);
        let (id, _) = rig.job();
        let reason = "no folder named \"Nowhere\" was found in My Inventory";
        rig.answer(id, Err(Failed::Reason(reason.to_string())));
        assert_eq!(error(&answer), reason);
        // Too much to list in time: the error says what to do about it.
        let answer = rig.inventory(&[]);
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::TimedOut));
        let reason = error(&answer);
        assert!(reason.contains("longer than 50 s") && reason.contains("name a folder"), "{reason}");
        // A lookup that took too long names what was looked up.
        let answer = rig.command(Request::Sound("Sounds/Bell".into()));
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::TimedOut));
        let reason = error(&answer);
        assert!(reason.contains("\"Sounds/Bell\"") && reason.contains("50 s"), "{reason}");
    }

    #[test]
    fn a_listing_under_way_fails_with_the_session_or_the_simulator() {
        let mut rig = Rig::new();
        let lost = rig.inventory(&[]);
        let (lost_id, _) = rig.job();
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + inventory::LIMIT + JOB_GRACE));
        rig.commands.session_lost();
        assert!(error(&lost).contains("session was lost"));
        let moved = rig.inventory(&[]);
        let (moved_id, _) = rig.job();
        rig.commands.simulator_changed();
        assert!(error(&moved).contains("changed simulator"));
        assert_eq!(rig.commands.next_deadline(), None);
        // The worker's answers for the dropped jobs come to nothing.
        rig.answer(lost_id, Ok(Outcome::Listed(vec![])));
        rig.answer(moved_id, Ok(Outcome::Found(radio())));
        rig.answer(999, Err(Failed::TimedOut));
        assert!(lost.try_recv().is_err() && moved.try_recv().is_err());
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_worker_that_never_answers_is_given_up_on() {
        let mut rig = Rig::new();
        let answer = rig.inventory(&[]);
        let (id, _) = rig.job();
        rig.commands.tick(rig.now + inventory::LIMIT);
        not_answered(&answer);
        rig.commands.tick(rig.now + inventory::LIMIT + JOB_GRACE);
        assert!(error(&answer).contains("name a folder"));
        rig.answer(id, Ok(Outcome::Listed(vec![])));
        assert!(answer.try_recv().is_err());
    }

    // -------------------------------------------------------- attachments

    #[test]
    fn attach_puts_an_object_on_the_named_point() {
        let mut rig = Rig::new();
        let request = rig.attach("Objects/Radio", Some("Left Hand"));
        let answer = rig.command(request);
        let (id, what) = rig.job();
        assert_eq!(what, What::Find(path(&["Objects", "Radio"])));
        assert_eq!(rig.calls(), []);
        rig.answer(id, Ok(Outcome::Found(radio())));
        // The item as the inventory described it, for the simulator.
        assert_eq!(rig.calls(), [Call::Attach(radio(), 5)]);
        not_answered(&answer);
        // Something else turning up is not the answer.
        assert!(!rig.sim_attached(&hat(), 2));
        not_answered(&answer);
        assert!(rig.sim_attached(&radio(), 5));
        let text = ok(&answer).unwrap();
        assert!(
            text.contains("\"Radio\"") && text.contains(&radio().id.to_string()) && text.ends_with("Left Hand"),
            "{text}"
        );
        // Neither the command nor its outcome is a movement.
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn body_parts_are_read_by_name_and_number() {
        let mut rig = Rig::new();
        for (part, point) in [("left hand", 5), ("5", 5), ("HUD Center", 35), ("r upper arm", 18), ("127", 127)] {
            let request = rig.attach("Objects/Radio", Some(part));
            rig.with_item(request, &radio());
            assert_eq!(rig.calls(), [Call::Attach(radio(), point)], "{part}");
        }
        // What is no attachment point is refused before the grid is asked.
        for part in ["elbow", "Center", "0", "128"] {
            let request = rig.attach("Objects/Radio", Some(part));
            let reason = error(&rig.command(request));
            assert!(
                reason.contains(&format!("\"{part}\"")) && reason.contains("not an attachment point"),
                "{reason}"
            );
        }
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn attach_without_a_body_part_leaves_the_point_to_the_simulator() {
        let mut rig = Rig::new();
        let request = rig.attach("Objects/Hat", None);
        let answer = rig.with_item(request, &hat());
        assert_eq!(rig.calls(), [Call::Attach(hat(), 0)]);
        // The answer names the point the simulator shows it on.
        assert!(rig.sim_attached(&hat(), 2));
        assert!(ok(&answer).unwrap().ends_with("is attached to Skull"));
        // By UUID.
        let request = rig.attach(&radio().id.to_string(), None);
        rig.command(request);
        assert_eq!(rig.job().1, What::Find(Ref::Id(radio().id)));
    }

    #[test]
    fn attach_adds_to_what_is_on_the_point() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(watch().id, 5)];
        let request = rig.attach("Objects/Radio", Some("Left Hand"));
        let answer = rig.with_item(request, &radio());
        // The watch is not taken off to make room.
        assert_eq!(rig.calls(), [Call::Attach(radio(), 5)]);
        assert!(rig.sim_attached(&radio(), 5));
        ok(&answer);
        assert_eq!(rig.grid.attached, [(watch().id, 5), (radio().id, 5)]);
    }

    #[test]
    fn attach_refuses_what_is_no_object_or_attached_already() {
        let mut rig = Rig::new();
        let request = rig.attach("Animations/Dance", None);
        let reason = error(&rig.with_item(request, &dance()));
        assert!(
            reason.contains("\"Dance\"") && reason.contains("is an animation, not an object"),
            "{reason}"
        );
        rig.grid.attached = vec![(radio().id, 5)];
        let request = rig.attach("Objects/Radio", Some("Chest"));
        let reason = error(&rig.with_item(request, &radio()));
        assert!(
            reason.contains("\"Radio\"") && reason.contains("attached to Left Hand already"),
            "{reason}"
        );
        // An item that cannot be found.
        let request = rig.attach("Objects/Dish", None);
        let answer = rig.command(request);
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::Reason("no item named \"Dish\" was found in \"Objects\"".into())));
        assert!(error(&answer).contains("\"Dish\""));
        // Nothing went to the simulator for any of them.
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.grid.attached, [(radio().id, 5)]);
    }

    #[test]
    fn an_attach_the_simulator_does_not_carry_out_fails() {
        let mut rig = Rig::new();
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        rig.commands.attach_failed(hat().id);
        not_answered(&answer);
        rig.commands.attach_failed(radio().id);
        let reason = error(&answer);
        assert!(reason.contains("did not attach \"Radio\"") && reason.contains("30 s"), "{reason}");
        assert!(!reason.contains("it said"), "{reason}");
        // What the simulator said meanwhile is likely the reason.
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        rig.commands.alert("Too many attachments");
        rig.commands.attach_failed(radio().id);
        assert!(error(&answer).ends_with("; it said: Too many attachments"));
        // An alert while nothing is awaited is nobody's.
        rig.commands.alert("Region restarting");
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        rig.commands.attach_failed(radio().id);
        assert!(!error(&answer).contains("Region restarting"));
        // The avatar left the region while the inventory was asked.
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.command(request);
        let (id, _) = rig.job();
        rig.grid.offline = true;
        rig.answer(id, Ok(Outcome::Found(radio())));
        assert_eq!(error(&answer), NOT_CONNECTED);
        // And with the session gone, the one waiting for the simulator.
        rig.grid.offline = false;
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        rig.commands.session_lost();
        assert!(error(&answer).contains("session was lost"));
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn wear_attaches_an_object_like_attach_without_a_body_part() {
        let mut rig = Rig::new();
        let answer = rig.with_item(Request::Wear("Objects/Hat".into()), &hat());
        assert_eq!(rig.calls(), [Call::Attach(hat(), 0)]);
        assert!(rig.sim_attached(&hat(), 2));
        assert!(ok(&answer).unwrap().ends_with("is attached to Skull"));
        // Also where the grid has no outfit service.
        assert!(!rig.grid.outfit);
        let reason = error(&rig.with_item(Request::Wear("Notecards/Readme".into()), &item("Readme", ASSET_NOTECARD, 0)));
        assert!(reason.contains("\"Readme\"") && reason.contains("is a notecard"), "{reason}");
        rig.grid.attached = vec![(hat().id, 2)];
        assert!(error(&rig.with_item(Request::Wear("Objects/Hat".into()), &hat())).contains("attached to Skull already"));
    }

    #[test]
    fn detach_takes_an_attached_item_off() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(radio().id, 5), (hat().id, 2)];
        let answer = rig.with_item(Request::Detach("Objects/Radio".into()), &radio());
        assert_eq!(rig.calls(), [Call::Detach(radio().id)]);
        not_answered(&answer);
        assert!(rig.sim_detached(&radio()));
        let text = ok(&answer).unwrap();
        assert!(
            text.starts_with("detached \"Radio\"") && text.contains(&radio().id.to_string()),
            "{text}"
        );
        // Something going by itself is nobody's command.
        assert!(!rig.sim_detached(&hat()));
        // An object that is not attached.
        let reason = error(&rig.with_item(Request::Detach("Objects/Hat".into()), &hat()));
        assert!(reason.contains("\"Hat\"") && reason.contains("is not attached"), "{reason}");
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn detach_reads_a_uuid_then_a_point_then_a_path() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(radio().id, 1)];
        rig.command(Request::Detach(radio().id.to_string()));
        assert_eq!(rig.job().1, What::Find(Ref::Id(radio().id)));
        // `Chest` is the point; `/Chest` an item of that name.
        rig.command(Request::Detach("Chest".into()));
        assert_eq!(rig.job().1, What::Items(vec![radio().id]));
        rig.command(Request::Detach("/Chest".into()));
        assert_eq!(rig.job().1, What::Find(path(&["Chest"])));
        rig.command(Request::Detach("Objects/Radio".into()));
        assert_eq!(rig.job().1, What::Find(path(&["Objects", "Radio"])));
    }

    #[test]
    fn detach_takes_everything_off_a_point_and_answers_once() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(watch().id, 5), (hat().id, 2), (radio().id, 5)];
        let answer = rig.command(Request::Detach("left hand".into()));
        let (id, what) = rig.job();
        assert_eq!(what, What::Items(vec![watch().id, radio().id]));
        assert_eq!(rig.calls(), []);
        rig.answer(id, Ok(Outcome::Items(vec![radio(), watch()])));
        assert_eq!(rig.calls(), [Call::Detach(watch().id), Call::Detach(radio().id)]);
        assert!(rig.sim_detached(&watch()));
        not_answered(&answer);
        assert!(rig.sim_detached(&radio()));
        let text = ok(&answer).unwrap();
        assert!(
            text.contains("\"Watch\"") && text.contains("\"Radio\"") && !text.contains("Hat"),
            "{text}"
        );
        assert_eq!(rig.grid.attached, [(hat().id, 2)]);

        // Nothing on the point: refused, and nothing is asked or sent.
        let reason = error(&rig.command(Request::Detach("Chest".into())));
        assert_eq!(reason, "nothing is attached to Chest");
        assert_eq!((rig.grid.jobs.len(), rig.calls().len()), (0, 0));

        // One of two stays on: an error that says which went.
        rig.grid.attached = vec![(watch().id, 5), (radio().id, 5)];
        let answer = rig.command(Request::Detach("5".into()));
        let (id, _) = rig.job();
        // An item the inventory did not describe goes by its id.
        rig.answer(id, Ok(Outcome::Items(vec![watch()])));
        assert!(rig.sim_detached(&watch()));
        rig.commands.detach_failed(radio().id);
        let reason = error(&answer);
        assert!(
            reason.contains(&format!("did not detach {}", radio().id)) && reason.contains("; detached \"Watch\""),
            "{reason}"
        );

        // It went by itself while the grid was asked for its name.
        rig.grid.attached = vec![(hat().id, 2)];
        let answer = rig.command(Request::Detach("Skull".into()));
        let (id, _) = rig.job();
        rig.grid.attached.clear();
        rig.answer(id, Ok(Outcome::Items(vec![hat()])));
        assert_eq!(error(&answer), "nothing is attached to Skull");
        rig.calls();
        // The names could not be had in time.
        rig.grid.attached = vec![(hat().id, 2)];
        let answer = rig.command(Request::Detach("Skull".into()));
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::TimedOut));
        assert!(error(&answer).contains("what is attached to Skull"));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_detach_the_simulator_does_not_carry_out_fails() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(radio().id, 5)];
        let answer = rig.with_item(Request::Detach("Objects/Radio".into()), &radio());
        rig.commands.detach_failed(radio().id);
        let reason = error(&answer);
        assert!(reason.contains("did not detach \"Radio\"") && reason.contains("15 s"), "{reason}");
        let answer = rig.with_item(Request::Detach("Objects/Radio".into()), &radio());
        rig.commands.session_lost();
        assert!(error(&answer).contains("session was lost"));
    }

    #[test]
    fn outfit_commands_need_the_avatar_in_a_region() {
        let mut rig = Rig::new();
        rig.grid.offline = true;
        rig.grid.outfit = true;
        rig.grid.attached = vec![(radio().id, 5)];
        let attach = rig.attach("Objects/Radio", None);
        let requests = [
            attach,
            Request::Wear("Objects/Radio".into()),
            Request::Detach("Objects/Radio".into()),
            Request::Detach("Left Hand".into()),
            Request::Detach("shirt".into()),
            Request::Anim {
                stop: false,
                item: "Animations/Dance".into(),
            },
            Request::Sound("Sounds/Bell".into()),
        ];
        for request in requests {
            // With a session that is on its way out, and with none.
            assert_eq!(error(&rig.command(request.clone())), NOT_CONNECTED, "{request:?}");
            let (reply, answer) = mpsc::channel();
            rig.commands.handle(request, reply, None::<&mut FakeGrid>, rig.now);
            assert_eq!(error(&answer), NOT_CONNECTED);
        }
        assert_eq!((rig.grid.jobs.len(), rig.calls().len()), (0, 0));
        assert!(!rig.commands.take_manual());
    }

    // ------------------------------------------------ animations and sounds

    fn anim(stop: bool, item: &str) -> Request {
        Request::Anim {
            stop,
            item: item.to_string(),
        }
    }

    #[test]
    fn anim_starts_and_stops_an_animation_by_its_asset() {
        let mut rig = Rig::new();
        let answer = rig.command(anim(false, "Animations/Dance"));
        let (id, what) = rig.job();
        assert_eq!(what, What::Find(path(&["Animations", "Dance"])));
        rig.answer(id, Ok(Outcome::Found(dance())));
        // The asset, not the inventory item; answered on hand-over.
        assert_eq!(rig.calls(), [Call::Animate(dance().asset_id, true)]);
        let text = ok(&answer).unwrap();
        assert!(
            text.starts_with("playing \"Dance\"") && text.contains(&dance().id.to_string()),
            "{text}"
        );
        // Stopping answers the same whether or not it was playing.
        let answer = rig.with_item(anim(true, &dance().id.to_string()), &dance());
        assert_eq!(rig.calls(), [Call::Animate(dance().asset_id, false)]);
        assert!(ok(&answer).unwrap().starts_with("stopped \"Dance\""));
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn anim_refuses_what_is_no_animation() {
        let mut rig = Rig::new();
        for stop in [false, true] {
            let reason = error(&rig.with_item(anim(stop, "Objects/Radio"), &radio()));
            assert!(
                reason.contains("\"Radio\"") && reason.contains("is an object, not an animation"),
                "{reason}"
            );
            // An item whose animation the inventory does not give.
            let hidden = Item {
                asset_id: Uuid::nil(),
                ..dance()
            };
            let reason = error(&rig.with_item(anim(stop, "Animations/Dance"), &hidden));
            assert!(reason.contains("does not give the bot the animation itself"), "{reason}");
        }
        let answer = rig.command(anim(false, "Animations/Jig"));
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::Reason("no item named \"Jig\" was found in \"Animations\"".into())));
        assert!(error(&answer).contains("\"Jig\""));
        assert_eq!(rig.calls(), []);
        // The avatar left the region while the inventory was asked.
        let answer = rig.command(anim(false, "Animations/Dance"));
        let (id, _) = rig.job();
        rig.grid.offline = true;
        rig.answer(id, Ok(Outcome::Found(dance())));
        assert_eq!(error(&answer), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn sound_plays_a_sound_by_its_asset() {
        let mut rig = Rig::new();
        let answer = rig.with_item(Request::Sound("Sounds/Bell".into()), &bell());
        assert_eq!(rig.calls(), [Call::Sound(bell().asset_id)]);
        assert!(ok(&answer).unwrap().starts_with("played \"Bell\""));
        let reason = error(&rig.with_item(Request::Sound("Animations/Dance".into()), &dance()));
        assert!(
            reason.contains("\"Dance\"") && reason.contains("is an animation, not a sound"),
            "{reason}"
        );
        let hidden = Item {
            asset_id: Uuid::nil(),
            ..bell()
        };
        let reason = error(&rig.with_item(Request::Sound("Sounds/Bell".into()), &hidden));
        assert!(reason.contains("does not give the bot the sound itself"), "{reason}");
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    // ----------------------------------------------------------- clothing

    #[test]
    fn wear_puts_clothing_on_through_the_outfit_service() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        let answer = rig.with_item(Request::Wear("Clothing/Blue shirt".into()), &shirt());
        // A second job for the same command, with the same deadline.
        assert_eq!(rig.grid.jobs[0].1, rig.now + inventory::LIMIT);
        let (id, what) = rig.job();
        assert_eq!(what, What::Wear(shirt()));
        not_answered(&answer);
        rig.answer(id, Ok(Outcome::Outfit(vec![shirt()])));
        let text = ok(&answer).unwrap();
        assert!(text.starts_with("wearing \"Blue shirt\"") && text.ends_with("(shirt)"), "{text}");
        // A body part goes the same way; the worker replaces the old one.
        let skin = item("Tanned skin", ASSET_BODY_PART, 1);
        let answer = rig.with_item(Request::Wear("Body Parts/Tanned skin".into()), &skin);
        let (id, what) = rig.job();
        assert_eq!(what, What::Wear(skin.clone()));
        rig.answer(id, Ok(Outcome::Outfit(vec![skin])));
        assert!(ok(&answer).unwrap().ends_with("(skin)"));
        // The grid's refusal is the answer.
        let answer = rig.with_item(Request::Wear("Clothing/Blue shirt".into()), &shirt());
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::Reason("\"Blue shirt\" is worn already".into())));
        assert!(error(&answer).contains("worn already"));
        let answer = rig.with_item(Request::Wear("Clothing/Blue shirt".into()), &shirt());
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::TimedOut));
        assert!(error(&answer).contains("may or may not have changed"));
        // Nothing of this goes to the simulator.
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn clothing_is_refused_where_the_grid_leaves_the_appearance_to_the_viewer() {
        let mut rig = Rig::new();
        assert!(!rig.grid.outfit);
        let skin = item("Tanned skin", ASSET_BODY_PART, 1);
        for (request, found) in [
            (Request::Wear("Clothing/Blue shirt".into()), shirt()),
            (Request::Wear("Body Parts/Tanned skin".into()), skin),
            (Request::Detach("Clothing/Blue shirt".into()), shirt()),
        ] {
            let reason = error(&rig.with_item(request, &found));
            assert_eq!(reason, inventory::NO_OUTFIT_SERVICE);
            // No job follows the lookup: nothing in the inventory changes.
            assert_eq!(rig.grid.jobs.len(), 0);
        }
        assert_eq!(error(&rig.command(Request::Detach("shirt".into()))), inventory::NO_OUTFIT_SERVICE);
        assert_eq!((rig.grid.jobs.len(), rig.calls().len()), (0, 0));
    }

    #[test]
    fn unwear_takes_clothing_off_by_item_and_by_kind() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        let answer = rig.with_item(Request::Detach("Clothing/Blue shirt".into()), &shirt());
        let (id, what) = rig.job();
        assert_eq!(what, What::TakeOff(Clothes::Item(shirt())));
        rig.answer(id, Ok(Outcome::Outfit(vec![shirt()])));
        assert!(ok(&answer).unwrap().starts_with("took off \"Blue shirt\""));
        // A kind of clothing: no lookup, the worker picks the pieces.
        let answer = rig.command(Request::Detach("Shirt".into()));
        let (id, what) = rig.job();
        assert_eq!(what, What::TakeOff(Clothes::Kind(4)));
        let white = item("White shirt", ASSET_CLOTHING, 4);
        rig.answer(id, Ok(Outcome::Outfit(vec![shirt(), white])));
        let text = ok(&answer).unwrap();
        assert!(text.contains("\"Blue shirt\"") && text.contains("\"White shirt\""), "{text}");
        // Nothing of the kind is worn.
        let answer = rig.command(Request::Detach("pants".into()));
        let (id, _) = rig.job();
        rig.answer(id, Err(Failed::Reason("the avatar wears no pants".into())));
        assert_eq!(error(&answer), "the avatar wears no pants");
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn body_parts_are_not_taken_off() {
        // On any grid, and without asking it.
        for outfit in [true, false] {
            let mut rig = Rig::new();
            rig.grid.outfit = outfit;
            for kind in ["skin", "shape", "Hair", "eyes"] {
                let reason = error(&rig.command(Request::Detach(kind.into())));
                let kind = kind.to_lowercase();
                assert!(reason.contains(&format!("{kind} can only be replaced, with `wear`")), "{reason}");
            }
            assert_eq!(rig.grid.jobs.len(), 0);
            // A body part named as an item.
            let skin = item("Tanned skin", ASSET_BODY_PART, 1);
            let reason = error(&rig.with_item(Request::Detach("Body Parts/Tanned skin".into()), &skin));
            assert!(
                reason.contains("\"Tanned skin\"") && reason.contains("skin can only be replaced"),
                "{reason}"
            );
            // Something that is neither attached nor worn.
            let reason = error(&rig.with_item(Request::Detach("Animations/Dance".into()), &dance()));
            assert!(reason.contains("is an animation, which is neither attached nor worn"), "{reason}");
            assert_eq!((rig.grid.jobs.len(), rig.calls().len()), (0, 0));
        }
    }

    #[test]
    fn the_outfit_folder_follows_the_bots_own_attachments() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        assert!(rig.sim_attached(&radio(), 5));
        // Answered at once; the folder is brought up to date afterwards.
        ok(&answer);
        let (link, what) = rig.job();
        assert_eq!(what, What::Link(radio()));
        // The grid refuses the change: logged, and the answer stands.
        rig.answer(link, Err(Failed::Reason("the grid refused to add \"Radio\"".into())));
        assert!(answer.try_recv().is_err());
        // The same after a detach.
        let answer = rig.with_item(Request::Detach("Objects/Radio".into()), &radio());
        assert!(rig.sim_detached(&radio()));
        ok(&answer);
        let (unlink, what) = rig.job();
        assert_eq!(what, What::Unlink(radio().id));
        rig.answer(unlink, Ok(Outcome::Outfit(vec![])));
        // What comes and goes without a command leaves the folder alone.
        assert!(!rig.sim_attached(&hat(), 2));
        assert!(!rig.sim_detached(&hat()));
        assert_eq!(rig.grid.jobs.len(), 0);
        // Nobody waits for the bookkeeping.
        assert_eq!(rig.commands.next_deadline(), None);
    }

    #[test]
    fn the_outfit_folder_is_left_alone_without_the_outfit_service() {
        let mut rig = Rig::new();
        assert!(!rig.grid.outfit);
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        assert!(rig.sim_attached(&radio(), 5));
        ok(&answer);
        let answer = rig.with_item(Request::Detach("Objects/Radio".into()), &radio());
        assert!(rig.sim_detached(&radio()));
        ok(&answer);
        assert_eq!(rig.grid.jobs.len(), 0);
    }
    // ---------------------------------------------- attaches that come late

    impl Rig {
        fn told(&mut self) -> Vec<String> {
            std::mem::take(&mut self.commands.told)
        }

        /// An `attach` of the radio that the simulator did not carry out
        /// in time.
        fn give_up_on_radio(&mut self) {
            let request = self.attach("Objects/Radio", None);
            let answer = self.with_item(request, &radio());
            self.commands.attach_failed(radio().id);
            assert!(error(&answer).contains("did not attach"));
            self.calls();
        }
    }

    #[test]
    fn an_attach_that_arrives_after_the_answer_still_gets_its_link() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.give_up_on_radio();
        assert_eq!(rig.grid.jobs.len(), 0);
        // Six seconds later, as on Second Life.
        assert!(rig.sim_attached(&radio(), 5));
        assert_eq!(
            rig.told(),
            ["attachment: \"Radio\" <".to_string() + &radio().id.to_string() + "> arrived late, on Left Hand"]
        );
        assert_eq!(rig.job().1, What::Link(radio()));
        // Once: the same object shown again is nobody's.
        rig.grid.attached.clear();
        assert!(!rig.sim_attached(&radio(), 5));
        assert_eq!(rig.grid.jobs.len(), 0);
    }

    #[test]
    fn a_late_attach_is_only_noted_where_the_folder_is_not_kept() {
        let mut rig = Rig::new();
        rig.give_up_on_radio();
        assert!(rig.sim_attached(&radio(), 5));
        assert_eq!(rig.told().len(), 1);
        assert_eq!(rig.grid.jobs.len(), 0);
    }

    #[test]
    fn a_second_attach_answers_for_an_object_that_was_given_up_on() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.give_up_on_radio();
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        assert!(rig.sim_attached(&radio(), 5));
        assert!(ok(&answer).unwrap().contains("attached to Left Hand"));
        // One link, and it did not come late for the command that waited.
        assert_eq!(rig.job().1, What::Link(radio()));
        assert_eq!(rig.told(), [] as [String; 0]);
    }

    #[test]
    fn what_was_given_up_on_is_forgotten_with_the_session() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.give_up_on_radio();
        rig.commands.session_lost();
        assert!(!rig.sim_attached(&radio(), 5));
        assert_eq!(rig.grid.jobs.len(), 0);
        // Only so many are remembered.
        for n in 0..(MAX_GAVE_UP as u128 + 1) {
            let item = Item {
                id: Uuid::from_u128(0xF00 + n),
                ..radio()
            };
            let request = rig.attach(&item.id.to_string(), None);
            let _ = rig.with_item(request, &item);
            rig.commands.attach_failed(item.id);
        }
        assert_eq!(rig.commands.gave_up.len(), MAX_GAVE_UP);
        assert_eq!(rig.commands.gave_up[0].id, Uuid::from_u128(0xF01));
    }

    // ------------------------------------- restoring attachments after login

    impl Rig {
        /// A login on a grid that keeps the Current Outfit folder: the
        /// region is entered and the time to compare has come. Returns
        /// the id of the job that reads the folder.
        fn compare(&mut self) -> u64 {
            self.grid.outfit = true;
            self.commands.entered_region(self.now);
            self.due();
            assert_eq!(self.grid.jobs.len(), 0, "asked before the simulator had its time");
            assert_eq!(self.commands.next_deadline(), Some(self.now + RESTORE_DELAY));
            self.now += RESTORE_DELAY;
            self.due();
            let (id, what) = self.job();
            assert_eq!(what, What::OutfitObjects);
            id
        }

        fn due(&mut self) {
            self.commands.restore_due(Some(&mut self.grid), self.now);
        }

        fn listed(&mut self, id: u64, items: &[Item]) {
            self.answer(id, Ok(Outcome::Items(items.to_vec())));
        }
    }

    #[test]
    fn an_object_the_outfit_folder_lists_and_the_simulator_did_not_bring_is_attached_again() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(hat().id, 2)];
        let id = rig.compare();
        rig.listed(id, &[radio(), hat()]);
        // To the point it was last on, and nothing is taken off.
        assert_eq!(rig.calls(), [Call::Attach(radio(), 0)]);
        let told = rig.told();
        assert_eq!(told.len(), 1);
        assert!(
            told[0].starts_with("attachment: restoring \"Radio\" <") && told[0].contains("Current Outfit folder"),
            "{told:?}"
        );
        // The simulator shows it: said so, and no link is added.
        assert!(rig.sim_attached(&radio(), 5));
        let told = rig.told();
        assert!(
            told.len() == 1 && told[0].starts_with("attachment: restored \"Radio\" <") && told[0].ends_with("to Left Hand"),
            "{told:?}"
        );
        assert_eq!(rig.grid.jobs.len(), 0);
        // It is attached like any other.
        let request = rig.attach("Objects/Radio", None);
        assert!(error(&rig.with_item(request, &radio())).contains("attached to Left Hand already"));
        // Not again in this login, whatever happens.
        rig.grid.attached.clear();
        assert_eq!(rig.commands.next_deadline(), None);
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY * 3;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn nothing_is_asked_of_the_simulator_when_nothing_is_missing() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(hat().id, 2), (radio().id, 5), (watch().id, 5)];
        let id = rig.compare();
        // The watch is worn and not listed: it stays, and so does the folder.
        rig.listed(id, &[radio(), hat()]);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.told(), [] as [String; 0]);
        // An empty folder takes nothing off either.
        let mut rig = Rig::new();
        rig.grid.attached = vec![(watch().id, 5)];
        let id = rig.compare();
        rig.listed(id, &[]);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.grid.attached, [(watch().id, 5)]);
    }

    #[test]
    fn nothing_is_restored_when_switched_off_or_without_the_outfit_service() {
        // Switched off: the folder is not asked about.
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.commands.set_restore(false);
        rig.commands.entered_region(rig.now);
        assert_eq!(rig.commands.next_deadline(), None);
        rig.now += RESTORE_DELAY;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        // A grid that does not keep the folder, as OpenSimulator.
        let mut rig = Rig::new();
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.commands.next_deadline(), None);
        // And not later in that login either.
        rig.grid.outfit = true;
        rig.now += RESTORE_DELAY;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.told(), [] as [String; 0]);
    }

    #[test]
    fn the_folder_is_compared_once_per_login() {
        let mut rig = Rig::new();
        let id = rig.compare();
        rig.listed(id, &[]);
        // A teleport to another region afterwards.
        rig.commands.simulator_changed();
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        // The session is lost and the bot logs in again.
        rig.commands.session_lost();
        rig.due();
        assert_eq!(rig.commands.next_deadline(), None);
        let id = rig.compare();
        rig.listed(id, &[radio()]);
        assert_eq!(rig.calls(), [Call::Attach(radio(), 0)]);
    }

    #[test]
    fn a_command_given_meanwhile_is_not_doubled_by_the_restore() {
        // The object is on its way for a command when the folder is read.
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.commands.entered_region(rig.now);
        let request = rig.attach("Objects/Radio", None);
        let answer = rig.with_item(request, &radio());
        assert_eq!(rig.calls(), [Call::Attach(radio(), 0)]);
        rig.now += RESTORE_DELAY;
        rig.due();
        let (id, _) = rig.job();
        rig.listed(id, &[radio()]);
        assert_eq!(rig.calls(), []);
        assert!(rig.sim_attached(&radio(), 5));
        assert!(ok(&answer).unwrap().contains("attached to Left Hand"));
        assert_eq!(rig.told(), [] as [String; 0]);
        // A command for an object that is being restored takes it over.
        let mut rig = Rig::new();
        let id = rig.compare();
        rig.listed(id, &[radio()]);
        rig.told();
        let request = rig.attach("Objects/Radio", Some("Chest"));
        let answer = rig.with_item(request, &radio());
        assert_eq!(rig.calls(), [Call::Attach(radio(), 0), Call::Attach(radio(), 1)]);
        assert!(rig.sim_attached(&radio(), 1));
        assert!(ok(&answer).unwrap().contains("attached to Chest"));
        assert_eq!(rig.told(), [] as [String; 0]);
        // The command's attach is noted in the folder as always.
        assert_eq!(rig.job().1, What::Link(radio()));
    }

    #[test]
    fn the_comparison_waits_for_a_region_that_has_shown_its_attachments() {
        // Between regions when the time comes.
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY;
        rig.grid.offline = true;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + RESTORE_RETRY));
        // The new region gets its twenty seconds too: its simulator has
        // not shown anything yet.
        rig.grid.offline = false;
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_RETRY;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        rig.now += RESTORE_DELAY;
        rig.due();
        let (id, _) = rig.job();
        // The same when the region changes while the folder is read: the
        // answer is not compared with an avatar nobody has described.
        rig.commands.simulator_changed();
        rig.commands.entered_region(rig.now);
        rig.listed(id, &[radio()]);
        assert_eq!(rig.calls(), []);
        rig.now += RESTORE_DELAY;
        rig.due();
        let (id, _) = rig.job();
        // And when the avatar leaves just as the answer comes.
        rig.grid.offline = true;
        rig.listed(id, &[radio()]);
        assert_eq!(rig.calls(), []);
        rig.grid.offline = false;
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY;
        rig.due();
        let (id, _) = rig.job();
        rig.listed(id, &[radio()]);
        assert_eq!(rig.calls(), [Call::Attach(radio(), 0)]);
        // A simulator that has not said yet what it offers is waited for.
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.grid.settling = true;
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        rig.grid.settling = false;
        rig.now += RESTORE_RETRY;
        rig.due();
        assert_eq!(rig.job().1, What::OutfitObjects);
    }

    #[test]
    fn a_restore_that_comes_to_nothing_is_said_once_and_not_tried_again() {
        // The simulator does not bring the object.
        let mut rig = Rig::new();
        let id = rig.compare();
        rig.listed(id, &[radio(), hat()]);
        rig.told();
        rig.commands.attach_failed(radio().id);
        let told = rig.told();
        assert!(
            told.len() == 1 && told[0].contains("did not restore \"Radio\"") && told[0].ends_with("within 30 s"),
            "{told:?}"
        );
        // The other one still counts when it comes.
        assert!(rig.sim_attached(&hat(), 2));
        assert_eq!(rig.told().len(), 1);
        // One that comes after that is an attachment like any other.
        assert!(!rig.sim_attached(&radio(), 5));
        rig.now += RESTORE_DELAY * 3;
        rig.due();
        assert_eq!(rig.grid.jobs.len(), 0);
        assert_eq!(rig.calls().len(), 2);

        // The folder cannot be read.
        for (failure, reason) in [
            (
                Failed::Reason("the inventory has no Current Outfit folder".into()),
                "no Current Outfit folder",
            ),
            (Failed::TimedOut, "did not answer in time"),
        ] {
            let mut rig = Rig::new();
            let id = rig.compare();
            rig.answer(id, Err(failure));
            let told = rig.told();
            assert!(
                told.len() == 1 && told[0].starts_with("attachments not restored: ") && told[0].contains(reason),
                "{told:?}"
            );
            rig.now += RESTORE_DELAY * 3;
            rig.due();
            assert_eq!(rig.grid.jobs.len(), 0);
            assert_eq!(rig.calls(), []);
        }
        // Or not even asked for.
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.grid.no_inventory = true;
        rig.commands.entered_region(rig.now);
        rig.now += RESTORE_DELAY;
        rig.due();
        let told = rig.told();
        assert!(told.len() == 1 && told[0].starts_with("attachments not restored: "), "{told:?}");
        assert_eq!(rig.commands.next_deadline(), None);
    }

    // ------------------------------------------------- RLV restrictions

    use crate::session::Placement;

    /// The attached collar that holds most of the restrictions here.
    fn collar() -> Uuid {
        Uuid::from_bytes([0xC0; 16])
    }

    impl Rig {
        fn hold_as(&mut self, who: Who, name: &str, text: &str) {
            let (behaviour, option) = match text.split_once(':') {
                Some((behaviour, option)) => (behaviour, Some(option)),
                None => (text, None),
            };
            self.grid.kept.set(who, name, behaviour, option);
        }

        /// The collar sets these restrictions.
        fn hold(&mut self, texts: &[&str]) {
            for text in texts {
                self.hold_as(Who::Object(collar()), "Collar", text);
            }
        }

        fn lift_all(&mut self) {
            self.grid.kept.reset();
        }

        /// The simulator has described `object` as this.
        fn described(&mut self, object: Uuid, placement: Placement, distance: f32) {
            let thing = Thing {
                placement,
                prims: vec![object],
                distance: Some(distance),
            };
            self.grid.things.push((object, thing));
        }

        fn exempt(&mut self, request: Request) -> Receiver<Response> {
            let (reply, answer) = mpsc::channel();
            self.commands
                .handle_from(Origin::Exempt, request, reply, Some(&mut self.grid), self.now);
            answer
        }

        fn tp(&mut self, words: &[&str]) -> Receiver<Response> {
            self.command(Request::Tp(Rig::words(words)))
        }
    }

    /// The error of a command a restriction refused: it names the
    /// restriction and the way past it.
    fn held_back(answer: &Receiver<Response>, rule: &str) -> String {
        let reason = error(answer);
        assert!(
            reason.starts_with(&format!("refused by RLV: {rule}, held by ")) && reason.contains("`rslbot ctl rlv mode override`"),
            "{reason}"
        );
        reason
    }

    #[test]
    fn a_refused_command_names_the_restriction_its_holder_and_override() {
        let mut rig = Rig::new();
        rig.hold(&["unsit"]);
        rig.grid.sitting = Sitting::Object;
        let reason = held_back(&rig.command(Request::Stand), "unsit");
        assert!(reason.contains(&format!("Collar <{}>", collar())), "{reason}");
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        // The operator's own restriction, set with `rlv cmd`.
        rig.lift_all();
        rig.hold_as(Who::Operator, "", "tploc");
        let reason = held_back(&rig.tp(&["128", "128", "25", "Elsewhere"]), "tploc");
        assert!(reason.contains("held by the operator"), "{reason}");
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(rig.commands.last_tp(), None);
    }

    #[test]
    fn no_restriction_is_in_force_while_rlv_is_off_or_for_the_exempt_command() {
        let mut rig = Rig::new();
        rig.hold(&["unsit", "sendchat", "tploc"]);
        rig.grid.sitting = Sitting::Object;
        held_back(&rig.command(Request::Stand), "unsit");
        // The command override exempts is carried out as if nothing were kept.
        assert_eq!(rig.exempt(Request::Stand).try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.exempt(Request::Say("hello".into())).try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.calls(), [Call::Stand, Call::Say(0, SAY, "hello".into())]);
        // It counts as the operator's command where that matters.
        let answer = rig.exempt(Request::Tp(Rig::words(&["128", "128", "25", "Elsewhere"])));
        not_answered(&answer);
        assert!(rig.commands.last_tp().is_some());
        rig.calls();
        // And the next one is held to them again.
        held_back(&rig.command(Request::Stand), "unsit");
        // With RLV off everything is as it is without restrictions.
        rig.grid.rlv_off = true;
        assert_eq!(rig.command(Request::Stand).try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.say("hello").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.calls(), [Call::Stand, Call::Say(0, SAY, "hello".into())]);
    }

    #[test]
    fn chat_is_refused_changed_or_redirected_as_the_restrictions_say() {
        let mut rig = Rig::new();
        rig.hold(&["sendchat"]);
        held_back(&rig.say("hello there"), "sendchat");
        assert_eq!(rig.calls(), []);
        // An emote goes through shortened, and the answer says so.
        let text = ok(&rig.say("/me nods slowly. Then she speaks at length")).unwrap();
        assert!(text.contains("shortened") && text.contains("sendchat"), "{text}");
        assert_eq!(rig.calls(), [Call::Say(0, SAY, "/me nods slowly".into())]);
        held_back(&rig.say("/me says \"hello\""), "sendchat");
        // A shout becomes normal chat, by the command and by the chat bar.
        rig.lift_all();
        rig.hold(&["chatshout"]);
        let text = ok(&rig.command(Request::Shout("over here".into()))).unwrap();
        assert!(text.contains("chatshout"), "{text}");
        assert!(ok(&rig.say("/shout over here")).is_some());
        assert_eq!(
            rig.calls(),
            [Call::Say(0, SAY, "over here".into()), Call::Say(0, SAY, "over here".into())]
        );
        // Everything is whispered.
        rig.lift_all();
        rig.hold(&["chatnormal"]);
        rig.say("hello");
        rig.command(Request::Whisper("psst".into()));
        assert_eq!(
            rig.calls(),
            [Call::Say(0, WHISPER, "hello".into()), Call::Say(0, WHISPER, "psst".into())]
        );
        // Redirected chat leaves channel 0, and `sendchat` has nothing to discard.
        rig.lift_all();
        rig.hold(&["redirchat:5", "sendchat"]);
        let text = ok(&rig.say("hello")).unwrap();
        assert!(text.contains("channel 5") && text.contains("redirchat:5"), "{text}");
        assert_eq!(rig.calls(), [Call::Say(5, SAY, "hello".into())]);
        // Private channels, but for the excepted one; `//` still means the
        // channel that was named last.
        rig.lift_all();
        rig.hold(&["sendchannel", "sendchannel:7"]);
        held_back(&rig.say("/8 open"), "sendchannel");
        assert_eq!(rig.say("/7 open").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.say("//again").try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.calls(), [Call::Say(7, SAY, "open".into()), Call::Say(7, SAY, "again".into())]);
    }

    #[test]
    fn instant_messages_are_held_by_sendim_and_startim() {
        let mut rig = Rig::new();
        rig.hold(&["sendim"]);
        let (answer, sent) = rig.im(&jane().to_string(), "hello");
        held_back(&answer, "sendim");
        assert_eq!((sent, rig.calls()), (None, vec![]));
        // Another reason comes first: nobody by that name.
        let (answer, _) = rig.im("Nobody Here", "hello");
        let query = rig.lookup("nobody here");
        rig.found(query, &[]);
        assert!(error(&answer).contains("knows no avatar"));
        // The restriction is looked at once the avatar is known.
        let (answer, _) = rig.im("Jane Doe", "hello");
        let query = rig.lookup("jane doe");
        assert_eq!(rig.found(query, &jane_found()), []);
        held_back(&answer, "sendim");
        // One avatar barred, the others not.
        rig.lift_all();
        rig.hold(&[&format!("sendimto:{}", jane())]);
        held_back(&rig.im(&jane().to_string(), "hello").0, &format!("sendimto:{}", jane()));
        assert!(rig.im(&john().to_string(), "hello").1.is_some());
        rig.calls();
        // No new conversations: Jane has written since the login, John has not.
        let mut rig = Rig::new();
        rig.hold(&["startim"]);
        rig.commands.im_received(jane());
        assert!(rig.im(&jane().to_string(), "hi").1.is_some());
        held_back(&rig.im(&john().to_string(), "hi").0, "startim");
        assert_eq!(rig.calls(), [Call::Im(jane(), "hi".into())]);
        // One the bot wrote to before the restriction is a conversation too.
        rig.lift_all();
        assert!(rig.im(&john().to_string(), "hi").1.is_some());
        rig.hold(&["startim"]);
        assert!(rig.im(&john().to_string(), "again").1.is_some());
        // They end with the session.
        rig.commands.session_lost();
        held_back(&rig.im(&john().to_string(), "hi").0, "startim");
    }

    #[test]
    fn sitting_and_standing_are_held_by_unsit_sit_and_sittp() {
        let (chair, bench, stool) = (other_object(), Uuid::from_bytes([0x61; 16]), Uuid::from_bytes([0x62; 16]));
        let sit = |object: Uuid| Request::Sit(object.to_string());
        let mut rig = Rig::new();
        rig.hold(&["unsit"]);
        // Standing: sitting down is no standing up.
        not_answered(&rig.command(sit(chair)));
        assert_eq!(rig.calls(), [Call::Sit(chair)]);
        // Seated: no standing up, and no changing seats.
        rig.grid.sitting = Sitting::Object;
        held_back(&rig.command(Request::Stand), "unsit");
        held_back(&rig.command(sit(bench)), "unsit");
        held_back(&rig.command(Request::Sit("ground".into())), "unsit");
        rig.grid.sitting = Sitting::Ground;
        held_back(&rig.command(Request::Stand), "unsit");
        assert_eq!(rig.calls(), []);
        assert!(rig.commands.take_manual(), "the first sit went manual");
        assert!(!rig.commands.take_manual(), "a refused command does not");
        // No sitting at all.
        let mut rig = Rig::new();
        rig.hold(&["sit"]);
        held_back(&rig.command(sit(chair)), "sit");
        held_back(&rig.command(Request::Sit("ground".into())), "sit");
        rig.lift_all();
        rig.hold(&["interact"]);
        held_back(&rig.command(sit(chair)), "interact");
        // From afar: 10 m is too far, 1 m is not, and an object of unknown
        // place counts as too far once the scan has had its time.
        rig.lift_all();
        rig.hold(&["sittp"]);
        rig.described(chair, Placement::InWorld, 10.0);
        rig.described(stool, Placement::InWorld, 1.0);
        held_back(&rig.command(sit(chair)), "sittp");
        not_answered(&rig.command(sit(stool)));
        rig.grid.look = Ok(Look::GaveUp);
        let reason = held_back(&rig.command(sit(bench)), "sittp");
        assert!(reason.contains("does not know how far"), "{reason}");
        assert_eq!(rig.calls(), [Call::Sit(stool), Call::LookFor(bench)]);
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(stool)));
    }

    #[test]
    fn teleports_are_held_by_tploc_tplocal_and_a_seated_unsit() {
        let mut rig = Rig::new();
        rig.hold(&["tploc"]);
        held_back(&rig.tp(&["128", "128", "25", "Elsewhere"]), "tploc");
        held_back(&rig.tp(&["12", "20", "30"]), "tploc");
        // Inside the region only; the avatar is in `Home` at 10, 20, 30.
        rig.lift_all();
        rig.hold(&["tplocal"]);
        held_back(&rig.tp(&["200", "200", "25"]), "tplocal");
        held_back(&rig.tp(&["200", "200", "25", "home"]), "tplocal");
        not_answered(&rig.tp(&["128", "128", "25", "Elsewhere"]));
        assert_eq!(rig.calls(), [Call::Teleport("Elsewhere".into(), [128.0, 128.0, 25.0])]);
        // Within the distance the holder gives.
        rig.lift_all();
        rig.hold(&["tplocal:10"]);
        not_answered(&rig.tp(&["14", "23", "30"]));
        held_back(&rig.tp(&["30", "20", "30"]), "tplocal:10");
        assert_eq!(rig.calls(), [Call::Teleport("Home".into(), [14.0, 23.0, 30.0])]);
        // Seated and held.
        rig.lift_all();
        rig.hold(&["unsit"]);
        rig.grid.sitting = Sitting::Object;
        held_back(&rig.tp(&["128", "128", "25", "Elsewhere"]), "unsit");
        rig.offer(jane(), "Jane Doe", Uuid::from_bytes([0x71; 16]));
        held_back(&rig.accept("tp", None), "unsit");
        assert_eq!(rig.calls(), []);
        // The offer still waits, and is accepted once the avatar stands.
        rig.grid.sitting = Sitting::No;
        not_answered(&rig.accept("tp", None));
        assert_eq!(rig.calls(), [Call::AcceptTp(Uuid::from_bytes([0x71; 16]))]);
    }

    #[test]
    fn touching_is_held_by_what_the_object_is() {
        let (door, lever, button) = (other_object(), Uuid::from_bytes([0x61; 16]), Uuid::from_bytes([0x62; 16]));
        let touch = |object: Uuid| Request::Touch(object.to_string());
        let mut rig = Rig::new();
        rig.described(door, Placement::InWorld, 1.0);
        rig.described(lever, Placement::InWorld, 1.0);
        rig.described(button, Placement::InWorld, 6.0);
        rig.described(collar(), Placement::Worn, 0.0);
        rig.hold(&["touchworld", &format!("touchworld:{lever}")]);
        held_back(&rig.command(touch(door)), "touchworld");
        assert_eq!(rig.command(touch(lever)).try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.command(touch(collar())).try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.calls(), [Call::Touch(lever), Call::Touch(collar())]);
        rig.lift_all();
        rig.hold(&["touchattachself", "fartouch"]);
        held_back(&rig.command(touch(collar())), "touchattachself");
        held_back(&rig.command(touch(button)), "fartouch");
        assert_eq!(rig.command(touch(door)).try_recv(), Ok(Response::Ok(None)));
        // An object that holds `touchme` can be touched, but not under `interact`.
        rig.lift_all();
        rig.hold(&["touchall"]);
        rig.hold_as(Who::Object(lever), "Lever", "touchme");
        held_back(&rig.command(touch(door)), "touchall");
        assert_eq!(rig.command(touch(lever)).try_recv(), Ok(Response::Ok(None)));
        rig.hold(&["interact"]);
        held_back(&rig.command(touch(lever)), "interact");
        rig.calls();
        // An object the simulator has not described is touched as before,
        // with the session's own wait, when no row of the table is held.
        rig.lift_all();
        rig.hold(&["sittp", "touchme"]);
        rig.grid.touch = Ok(Touch::Waiting);
        let unknown = Uuid::from_bytes([0x63; 16]);
        let answer = rig.command(touch(unknown));
        not_answered(&answer);
        assert_eq!(rig.calls(), [Call::Touch(unknown)]);
        rig.commands.touched(unknown);
        assert_eq!(answer.try_recv(), Ok(Response::Ok(None)));
        // Under `interact` nothing it may turn out to be is touched: refused
        // at once, without looking.
        rig.hold(&["interact"]);
        held_back(&rig.command(touch(unknown)), "interact");
        assert_eq!(rig.calls(), []);
    }

    // --------------------------------------- objects the bot does not know

    /// The answer when the bot has not described an object to the fake grid.
    fn not_known(answer: &Receiver<Response>, object: Uuid) {
        assert_eq!(error(answer), format!("object {object} is not known to the bot"));
    }

    impl Rig {
        /// The simulator has described `object` now, as the session reports.
        fn now_described(&mut self, object: Uuid, placement: Placement, distance: f32) {
            self.described(object, placement, distance);
            self.commands.described(object, Some(&mut self.grid), self.now);
        }

        /// The time to look for `object` has passed.
        fn never_described(&mut self, object: Uuid) {
            self.grid.look = Ok(Look::GaveUp);
            self.commands.not_described(object, Some(&mut self.grid), self.now);
        }
    }

    #[test]
    fn a_sit_waits_for_its_seat_under_sittp() {
        let chair = Uuid::from_bytes([0x61; 16]);
        let sit = Request::Sit(chair.to_string());
        let mut rig = Rig::new();
        rig.hold(&["sittp:20"]);
        // Sit that waits for its seat: looked for, nothing sent, not manual.
        let answer = rig.command(sit.clone());
        not_answered(&answer);
        assert_eq!(rig.calls(), [Call::LookFor(chair)]);
        assert!(!rig.commands.take_manual());
        assert_eq!(rig.commands.last_sit(), None);
        // Described 3 m away: the sit goes out, and is answered as without the wait.
        rig.now_described(chair, Placement::InWorld, 3.0);
        assert_eq!(rig.calls(), [Call::Sit(chair)]);
        assert!(rig.commands.take_manual());
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(chair)));
        not_answered(&answer);
        rig.commands.seated(chair);
        assert!(matches!(answer.try_recv(), Ok(Response::Ok(_))));
        // Something else described is nobody's business.
        rig.commands.described(Uuid::from_bytes([0x69; 16]), Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_seat_that_turns_out_too_far_is_refused_with_its_distance() {
        let chair = Uuid::from_bytes([0x61; 16]);
        let mut rig = Rig::new();
        rig.hold(&["sittp"]);
        let answer = rig.command(Request::Sit(chair.to_string()));
        not_answered(&answer);
        rig.calls();
        rig.now_described(chair, Placement::InWorld, 10.0);
        let reason = held_back(&answer, "sittp");
        assert!(reason.contains("10.0 m away"), "{reason}");
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual(), "a refused command does not go manual");
    }

    #[test]
    fn a_seat_never_described_is_refused_and_later_ones_at_once() {
        let ghost = Uuid::from_bytes([0x61; 16]);
        let sit = Request::Sit(ghost.to_string());
        let mut rig = Rig::new();
        rig.hold(&["sittp"]);
        let answer = rig.command(sit.clone());
        not_answered(&answer);
        rig.calls();
        rig.never_described(ghost);
        let reason = held_back(&answer, "sittp");
        assert!(reason.contains("does not know how far"), "{reason}");
        // After the scan has had its time: refused at once.
        let reason = held_back(&rig.command(sit), "sittp");
        assert!(reason.contains("does not know how far"), "{reason}");
        // Asked again at the deadline, which is how the bot learns that the
        // scan gave up, and once more for the second command.
        assert_eq!(rig.calls(), [Call::LookFor(ghost), Call::LookFor(ghost)]);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn without_a_restriction_that_needs_it_nothing_is_looked_for() {
        let chair = Uuid::from_bytes([0x61; 16]);
        let mut rig = Rig::new();
        // No restriction, no wait.
        not_answered(&rig.command(Request::Sit(chair.to_string())));
        assert_eq!(rig.calls(), [Call::Sit(chair)]);
        rig.commands.seated(chair);
        // A restriction that refuses whatever the distance: refused at once.
        rig.hold(&["sit", "sittp"]);
        held_back(&rig.command(Request::Sit(chair.to_string())), "sit");
        assert_eq!(rig.calls(), []);
        // The exempt command never waits for a restriction's sake.
        not_answered(&rig.exempt(Request::Sit(chair.to_string())));
        assert_eq!(rig.calls(), [Call::Sit(chair)]);
    }

    #[test]
    fn a_wait_ends_with_the_region_or_the_session() {
        let (chair, door) = (Uuid::from_bytes([0x61; 16]), Uuid::from_bytes([0x62; 16]));
        let mut rig = Rig::new();
        rig.hold(&["sittp", "touchworld"]);
        // Region left while waiting: looked at again, and not in a region.
        let answer = rig.command(Request::Sit(chair.to_string()));
        rig.grid.offline = true;
        rig.commands.not_described(chair, Some(&mut rig.grid), rig.now);
        assert_eq!(error(&answer), NOT_CONNECTED);
        // The session lost: every wait is answered.
        rig.grid.offline = false;
        let sit = rig.command(Request::Sit(chair.to_string()));
        let touch = rig.command(Request::Touch(door.to_string()));
        rig.commands.session_lost();
        assert_eq!(error(&sit), "the session was lost");
        assert_eq!(error(&touch), "the session was lost");
        rig.commands.described(chair, Some(&mut rig.grid), rig.now);
        rig.calls();
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_touch_under_a_restriction_looks_first_and_touches_what_it_finds() {
        let (door, hat, ghost) = (
            Uuid::from_bytes([0x61; 16]),
            Uuid::from_bytes([0x62; 16]),
            Uuid::from_bytes([0x63; 16]),
        );
        let touch = |object: Uuid| Request::Touch(object.to_string());
        let mut rig = Rig::new();
        rig.hold(&["touchattach"]);
        // Touch under a restriction: looked for, not touched with the
        // session's own wait.
        let answer = rig.command(touch(door));
        not_answered(&answer);
        assert_eq!(rig.calls(), [Call::LookFor(door)]);
        // A door rezzed in the region: touched once it is described.
        rig.now_described(door, Placement::InWorld, 3.0);
        assert_eq!(answer.try_recv(), Ok(Response::Ok(None)));
        assert_eq!(rig.calls(), [Call::Touch(door)]);
        // Another's attachment: refused once described.
        let answer = rig.command(touch(hat));
        rig.calls();
        rig.now_described(hat, Placement::WornBy(jane()), 2.0);
        held_back(&answer, "touchattach");
        assert_eq!(rig.calls(), []);
        // Never described: refused as an unknown object is.
        let answer = rig.command(touch(ghost));
        rig.never_described(ghost);
        not_known(&answer, ghost);
        // And at once after the scan has had its time.
        not_known(&rig.command(touch(ghost)), ghost);
        assert_eq!(rig.calls(), [Call::LookFor(ghost), Call::LookFor(ghost), Call::LookFor(ghost)]);
        // `fartouch` wants a distance, which only a description gives.
        rig.lift_all();
        rig.hold(&["fartouch"]);
        rig.grid.look = Ok(Look::Waiting);
        let far = Uuid::from_bytes([0x64; 16]);
        let answer = rig.command(touch(far));
        rig.now_described(far, Placement::InWorld, 6.0);
        held_back(&answer, "fartouch");
    }

    fn cuff() -> Item {
        item("Cuff", ASSET_OBJECT, 5)
    }

    fn badge() -> Item {
        item("Badge", ASSET_OBJECT, 0)
    }

    #[test]
    fn a_locked_attachment_stays_on() {
        let mut rig = Rig::new();
        // The collar is the object of `Cuff`, and a child prim of it holds the lock.
        let child = Uuid::from_bytes([0xC1; 16]);
        rig.grid.attached = vec![(cuff().id, 5), (watch().id, 5)];
        rig.grid.worn = vec![(collar(), cuff().id), (child, cuff().id)];
        rig.hold_as(Who::Object(child), "Cuff", "detach");
        let answer = rig.with_item(Request::Detach("Objects/Cuff".into()), &cuff());
        let reason = held_back(&answer, "detach");
        assert!(reason.contains(&format!("Cuff <{child}>")), "{reason}");
        assert_eq!(rig.calls(), []);
        // What is not locked comes off.
        let answer = rig.with_item(Request::Detach("Objects/Watch".into()), &watch());
        not_answered(&answer);
        assert_eq!(rig.calls(), [Call::Detach(watch().id)]);
        rig.sim_detached(&watch());
        // The whole point: the watch goes, the cuff stays, and the answer says so.
        rig.grid.attached = vec![(cuff().id, 5), (watch().id, 5)];
        let answer = rig.command(Request::Detach("left hand".into()));
        let (id, _) = rig.job();
        rig.answer(id, Ok(Outcome::Items(vec![cuff(), watch()])));
        assert_eq!(rig.calls(), [Call::Detach(watch().id)]);
        assert!(rig.sim_detached(&watch()));
        let reason = error(&answer);
        assert!(
            reason.starts_with("detached \"Watch\"")
                && reason.contains("\"Cuff\"")
                && reason.contains("stayed on: refused by RLV: detach, held by Cuff"),
            "{reason}"
        );
        // Nothing on the point can go: nothing is sent.
        let answer = rig.command(Request::Detach("left hand".into()));
        let (id, _) = rig.job();
        rig.answer(id, Ok(Outcome::Items(vec![cuff()])));
        let reason = error(&answer);
        assert!(reason.contains("\"Cuff\"") && reason.contains("stayed on"), "{reason}");
        assert_eq!(rig.calls(), []);
        // Unlocked, it comes off.
        rig.lift_all();
        not_answered(&rig.with_item(Request::Detach("Objects/Cuff".into()), &cuff()));
        assert_eq!(rig.calls(), [Call::Detach(cuff().id)]);
    }

    #[test]
    fn attachment_points_are_locked_full_or_empty() {
        let mut rig = Rig::new();
        rig.grid.attached = vec![(watch().id, 5)];
        rig.hold(&["detach:left hand"]);
        held_back(
            &rig.with_item(Request::Detach("Objects/Watch".into()), &watch()),
            "detach:left hand",
        );
        let attach = rig.attach("Objects/Radio", Some("left hand"));
        held_back(&rig.with_item(attach, &radio()), "detach:left hand");
        // The radio was last on the left hand: without a point it would go there.
        let attach = rig.attach("Objects/Radio", None);
        held_back(&rig.with_item(attach, &radio()), "detach:left hand");
        // Elsewhere it may go.
        let attach = rig.attach("Objects/Radio", Some("chest"));
        not_answered(&rig.with_item(attach, &radio()));
        assert_eq!(rig.calls(), [Call::Attach(radio(), 1)]);
        // Nothing may be attached anywhere.
        let mut rig = Rig::new();
        rig.hold(&["addattach"]);
        let attach = rig.attach("Objects/Radio", None);
        held_back(&rig.with_item(attach, &radio()), "addattach");
        // `wear` of an object is an attach.
        held_back(&rig.with_item(Request::Wear("Objects/Radio".into()), &radio()), "addattach");
        // A point that is not known beforehand may be the closed one.
        rig.lift_all();
        rig.hold(&["addattach:chest"]);
        let attach = rig.attach("Objects/Badge", None);
        let reason = held_back(&rig.with_item(attach, &badge()), "addattach:chest");
        assert!(reason.contains("not known"), "{reason}");
        let attach = rig.attach("Objects/Badge", Some("right hand"));
        not_answered(&rig.with_item(attach, &badge()));
        // One whose last point is known and open goes there.
        let attach = rig.attach("Objects/Radio", None);
        not_answered(&rig.with_item(attach, &radio()));
        assert_eq!(rig.calls(), [Call::Attach(badge(), 6), Call::Attach(radio(), 0)]);
        // Nothing may be detached.
        rig.lift_all();
        rig.hold(&["remattach"]);
        rig.grid.attached = vec![(watch().id, 5)];
        held_back(&rig.with_item(Request::Detach("Objects/Watch".into()), &watch()), "remattach");
    }

    #[test]
    fn clothing_is_held_by_addoutfit_and_remoutfit() {
        let mut rig = Rig::new();
        rig.grid.outfit = true;
        rig.hold(&["remoutfit:shirt"]);
        held_back(&rig.command(Request::Detach("shirt".into())), "remoutfit:shirt");
        held_back(
            &rig.with_item(Request::Detach("Clothing/Blue shirt".into()), &shirt()),
            "remoutfit:shirt",
        );
        assert_eq!(rig.grid.jobs.len(), 0);
        not_answered(&rig.command(Request::Detach("pants".into())));
        assert!(matches!(rig.job().1, What::TakeOff(_)));
        // Putting it on is another matter.
        not_answered(&rig.with_item(Request::Wear("Clothing/Blue shirt".into()), &shirt()));
        assert!(matches!(rig.job().1, What::Wear(_)));
        rig.lift_all();
        rig.hold(&["addoutfit"]);
        held_back(&rig.with_item(Request::Wear("Clothing/Blue shirt".into()), &shirt()), "addoutfit");
        assert_eq!(rig.grid.jobs.len(), 0);
    }

    #[test]
    fn the_inventory_is_closed_by_showinv_but_items_can_still_be_named() {
        let mut rig = Rig::new();
        rig.hold(&["showinv"]);
        held_back(&rig.inventory(&["Objects"]), "showinv");
        assert_eq!(rig.grid.jobs.len(), 0);
        let attach = rig.attach("Objects/Radio", None);
        not_answered(&rig.with_item(attach, &radio()));
        assert_eq!(rig.calls(), [Call::Attach(radio(), 0)]);
    }

    #[test]
    fn restoring_leaves_out_what_an_attach_would_be_refused_for() {
        let mut rig = Rig::new();
        rig.hold_as(Who::Operator, "", "addattach:chest");
        let chest_badge = item("Badge", ASSET_OBJECT, 1);
        let id = rig.compare();
        rig.listed(id, &[chest_badge.clone(), watch()]);
        assert_eq!(rig.calls(), [Call::Attach(watch(), 0)]);
        let told = rig.told();
        let left_out: Vec<&String> = told.iter().filter(|line| line.contains("not restored")).collect();
        assert_eq!(left_out.len(), 1, "{told:?}");
        assert!(
            left_out[0].contains("\"Badge\"") && left_out[0].contains("addattach:chest, held by the operator"),
            "{told:?}"
        );
    }

    // ------------------------------------- offers and requests under RLV

    #[test]
    fn tplure_drops_the_offers_it_forbids() {
        let mut rig = Rig::new();
        rig.commands.set_rules(Rules::default());
        rig.hold(&["tplure"]);
        rig.offer(jane(), "Jane Doe", lure(0x71));
        assert_eq!(rig.calls(), []);
        assert!(error(&rig.accept("tp", None)).contains("no teleport offer waits"));
        let told = rig.told();
        assert!(
            told.iter()
                .any(|line| line.contains("Jane Doe") && line.contains("dropped: tplure, held by Collar")),
            "{told:?}"
        );
        // Also one the bot asked for.
        ok(&rig.command(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        }));
        rig.calls();
        rig.offer(jane(), "Jane Doe", lure(0x72));
        assert_eq!(rig.calls(), []);
        assert!(error(&rig.accept("tp", None)).contains("no teleport offer waits"));
        // An excepted avatar's offer is decided as without `tplure`: it waits.
        rig.hold(&[&format!("tplure:{}", jane())]);
        rig.offer(jane(), "Jane Doe", lure(0x73));
        not_answered(&rig.accept("tp", None));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(0x73))]);
    }

    #[test]
    fn an_offer_that_waits_is_not_accepted_while_tplure_forbids_it() {
        let mut rig = Rig::new();
        rig.offer(jane(), "Jane Doe", lure(0x71));
        rig.hold(&["tplure"]);
        held_back(&rig.accept("tp", Some("Jane Doe")), "tplure");
        assert_eq!(rig.calls(), []);
        // It still waits.
        rig.lift_all();
        not_answered(&rig.accept("tp", Some("Jane Doe")));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(0x71))]);
    }

    #[test]
    fn the_offer_that_answers_an_exempt_requesttp_is_exempt_too() {
        let mut rig = Rig::new();
        rig.hold(&["tplure"]);
        rig.grid.sitting = Sitting::Object;
        rig.hold(&["unsit"]);
        ok(&rig.exempt(Request::AskTp {
            to: jane().to_string(),
            message: String::new(),
        }));
        rig.calls();
        rig.offer(jane(), "Jane Doe", lure(0x71));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(0x71))]);
        // One offer per request: the next is dropped.
        rig.offer(jane(), "Jane Doe", lure(0x72));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn accepttp_has_the_bot_accept_offers_by_itself() {
        let mut rig = Rig::new();
        rig.hold(&[&format!("accepttp:{}", jane())]);
        rig.offer(jane(), "Jane Doe", lure(0x71));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(0x71))]);
        assert!(rig.commands.take_manual(), "accepting moves the avatar");
        // Not from anyone else.
        rig.offer(john(), "John Roe", lure(0x72));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        // Without an option, from anyone.
        rig.hold(&["accepttp"]);
        rig.offer(john(), "John Roe", lure(0x73));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(0x73))]);
        rig.commands.take_manual();
        // `tplure` comes first.
        rig.hold(&["tplure"]);
        rig.offer(john(), "John Roe", lure(0x74));
        assert_eq!(rig.calls(), []);
        // A seated `unsit` makes the offer wait for the operator.
        rig.lift_all();
        rig.hold(&["accepttp", "unsit"]);
        rig.grid.sitting = Sitting::Object;
        rig.offer(john(), "John Roe", lure(0x75));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        rig.grid.sitting = Sitting::No;
        not_answered(&rig.accept("tp", None));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(0x75))]);
    }

    #[test]
    fn a_rule_does_not_take_a_seated_avatar_off_a_seat_unsit_holds() {
        let mut rig = Rig::new();
        ok(&rig.command(Request::Perms(PermsRequest::Autoaccept {
            kind: "tp".into(),
            who: vec!["all".into()],
        })));
        rig.hold(&["unsit"]);
        rig.grid.sitting = Sitting::Object;
        rig.offer(jane(), "Jane Doe", lure(0x71));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(rig.waiting().len(), 1);
        // And under `tplure` the rule accepts nothing: the offer is dropped.
        rig.hold(&["tplure"]);
        rig.offer(john(), "John Roe", lure(0x72));
        assert_eq!(rig.waiting().len(), 1);
    }

    #[test]
    fn requests_to_be_teleported_here_are_answered_or_dropped_for_rlv() {
        let mut rig = Rig::new();
        // Without RLV's say they are the operator's to answer.
        rig.commands
            .teleport_requested(jane(), "Jane Doe", "please", Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), []);
        let told = rig.told();
        assert!(
            told.len() == 1 && told[0].contains("asks to be teleported here") && told[0].contains("sendtp"),
            "{told:?}"
        );
        rig.hold(&["accepttprequest"]);
        rig.commands
            .teleport_requested(jane(), "Jane Doe", "please", Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), [Call::OfferTp(jane(), "Join me in Home".into())]);
        assert!(!rig.commands.take_manual());
        // `tprequest` comes first.
        rig.hold(&["tprequest"]);
        rig.told();
        rig.commands
            .teleport_requested(jane(), "Jane Doe", "please", Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), []);
        let told = rig.told();
        assert!(told.len() == 1 && told[0].contains("dropped: tprequest"), "{told:?}");
        // Only the avatar that is named.
        rig.lift_all();
        rig.hold(&[&format!("accepttprequest:{}", jane())]);
        rig.commands
            .teleport_requested(john(), "John Roe", "", Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn acceptpermission_grants_attach_and_controls_whatever_the_rules() {
        let mut rig = Rig::new();
        rig.hold(&["acceptpermission"]);
        rig.janes_asks(ball(), ATTACH | CONTROLS);
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), ATTACH | CONTROLS)]);
        // The other kinds still follow their rules.
        rig.janes_asks(other_ball(), CONTROLS | ANIM);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.waiting().len(), 1);
        // The rules themselves are what they were.
        assert!(
            rig.perms_status().iter().any(|line| line.starts_with("controls: none")),
            "{:?}",
            rig.perms_status()
        );
    }

    #[test]
    fn denypermission_drops_requests_for_attach_and_controls() {
        let mut rig = Rig::new();
        ok(&rig.command(Request::Perms(PermsRequest::Autoaccept {
            kind: "attach".into(),
            who: vec!["all".into()],
        })));
        rig.hold(&["denypermission", "acceptpermission"]);
        rig.janes_asks(ball(), ATTACH);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.waiting().len(), 0);
        assert!(error(&rig.accept("attach", None)).contains("no request"));
        // Animations are not its business.
        rig.janes_asks(ball(), ANIM);
        assert_eq!(rig.waiting().len(), 1);
    }

    #[test]
    fn a_request_that_waits_is_not_decided_again_when_a_restriction_is_set() {
        let mut rig = Rig::new();
        rig.janes_asks(ball(), CONTROLS);
        assert_eq!(rig.waiting().len(), 1);
        rig.hold(&["acceptpermission"]);
        assert_eq!((rig.calls(), rig.waiting().len()), (vec![], 1));
        // The same script's next request is granted.
        rig.janes_asks(ball(), CONTROLS);
        assert_eq!(rig.calls(), [Call::Grant(ball(), script_of(ball()), CONTROLS)]);
        // Under `denypermission` one that waits cannot be accepted, and waits on.
        rig.janes_asks(other_ball(), ANIM | CONTROLS);
        rig.lift_all();
        rig.hold(&["denypermission"]);
        held_back(&rig.accept("controls", None), "denypermission");
        assert_eq!(rig.waiting().len(), 1);
    }

    // ------------------------------------------------------------- scan

    use crate::udp::messages::{Properties, FLAG_SCRIPTED};

    /// A prim at this distance; with a name it is named, as its owner's.
    fn seen(n: u8, distance: f32, named: Option<(&str, Uuid)>) -> Sighting {
        let object = Uuid::from_bytes([n; 16]);
        Sighting {
            object,
            local_id: u32::from(n),
            distance,
            flags: 0,
            attachment: false,
            child: false,
            properties: named.map(|(name, owner)| Properties {
                object,
                owner,
                group: Uuid::nil(),
                name: name.to_string(),
                description: String::new(),
            }),
        }
    }

    fn names_of(id: Uuid, legacy: &str) -> (Uuid, Names) {
        let names = Names {
            legacy: legacy.to_string(),
            display: legacy.to_string(),
        };
        (id, names)
    }

    /// The names in the listing's sixth field, in order.
    fn listed(answer: &Receiver<Response>) -> Vec<String> {
        match answer.try_recv() {
            Ok(Response::Ok(Some(text))) => text.lines().map(|line| line.split('\t').nth(5).unwrap().to_string()).collect(),
            Ok(Response::Ok(None)) => Vec::new(),
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    impl Rig {
        fn scan_for(&mut self, args: &[&str]) -> Receiver<Response> {
            let args = args.iter().map(|arg| arg.to_string()).collect();
            self.run(Request::Scan(args)).0
        }

        /// The waiting scans are looked at.
        fn look(&mut self) {
            self.commands.scans_due(Some(&mut self.grid), self.now);
        }

        /// The simulator names the object with this local id.
        fn name_object(&mut self, local_id: u32, name: &str, owner: Uuid) {
            let sighting = self.grid.sightings.iter_mut().find(|s| s.local_id == local_id).unwrap();
            sighting.properties = seen(0, 0.0, Some((name, owner))).properties;
        }

        fn named_by_the_grid(&mut self, found: &[(Uuid, Names)], missing: &[Uuid]) {
            self.commands.names_resolved(found, missing, Some(&mut self.grid), self.now);
        }

        /// A bench of Jane's 3 metres away and a lamp of John's 10 metres
        /// away, both named, and both owners known to the name service.
        fn among_objects() -> Rig {
            let mut rig = Rig::new();
            rig.grid.sightings = vec![seen(2, 10.0, Some(("Lamp", john()))), seen(1, 3.0, Some(("Bench", jane())))];
            let answer = rig.scan_for(&[]);
            rig.look();
            assert_eq!(rig.commands.take_name_lookups().len(), 2);
            rig.named_by_the_grid(&[names_of(jane(), "Jane Doe"), names_of(john(), "John Roe")], &[]);
            rig.now += Duration::from_millis(250);
            rig.look();
            assert_eq!(listed(&answer), ["Bench", "Lamp"]);
            rig.calls();
            rig
        }
    }

    #[test]
    fn scan_lists_what_is_around_once_it_is_named() {
        let mut rig = Rig::new();
        rig.grid.sightings = vec![seen(2, 10.0, None), seen(1, 3.0, Some(("Bench", jane())))];
        let answer = rig.scan_for(&[]);
        assert_eq!(rig.calls(), [Call::Reach(None)]);
        assert!(answer.try_recv().is_err(), "it waits for the simulator");

        // The lamp has no name yet, and nobody has named Jane.
        rig.look();
        assert_eq!(rig.calls(), [Call::Properties(vec![2])]);
        assert_eq!(rig.commands.take_name_lookups(), [jane()]);
        assert!(answer.try_recv().is_err());
        assert_eq!(rig.commands.next_deadline(), Some(rig.now + SCAN_POLL));

        // Looked at four times a second, and not in between.
        rig.name_object(2, "Lamp", jane());
        rig.named_by_the_grid(&[names_of(jane(), "Jane Doe")], &[]);
        rig.now += Duration::from_millis(100);
        rig.look();
        assert!(answer.try_recv().is_err());
        rig.now += Duration::from_millis(150);
        rig.look();
        let Ok(Response::Ok(Some(text))) = answer.try_recv() else {
            panic!("no listing");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let bench = format!("{}\t3.0\t-\t{}\tJane Doe\tBench\t", Uuid::from_bytes([1; 16]), jane());
        assert_eq!(lines[0], bench);
        assert!(lines[1].ends_with("\tJane Doe\tLamp\t") && lines[1].contains("\t10.0\t"));
        // Nothing waits any more.
        assert_eq!((rig.calls(), rig.commands.next_deadline()), (vec![], None));
        assert_eq!(rig.commands.take_name_lookups(), Vec::<Uuid>::new());
    }

    #[test]
    fn scan_with_nothing_within_the_distance_prints_nothing() {
        let mut rig = Rig::new();
        rig.grid.sightings = vec![seen(1, 10.0, Some(("Bench", jane())))];
        let answer = rig.scan_for(&["5"]);
        assert_eq!(rig.calls(), [Call::Reach(Some(5.0))]);
        rig.look();
        assert_eq!(answer.try_recv(), Ok(Response::Ok(None)));
        // Nothing was asked for the sake of what is out of reach.
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.commands.take_name_lookups(), Vec::<Uuid>::new());
    }

    #[test]
    fn scan_needs_the_avatar_in_a_region() {
        let mut rig = Rig::new();
        let (reply, answer) = mpsc::channel();
        rig.commands.handle(Request::Scan(vec![]), reply, None::<&mut FakeGrid>, rig.now);
        assert_eq!(error(&answer), NOT_CONNECTED);
        rig.grid.offline = true;
        assert_eq!(error(&rig.scan_for(&["96"])), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.commands.next_deadline(), None);
    }

    #[test]
    fn scan_with_wrong_arguments_asks_the_simulator_nothing() {
        let mut rig = Rig::new();
        assert!(error(&rig.scan_for(&["-x"])).contains("`-x`"));
        assert!(error(&rig.scan_for(&["-n"])).contains("`-n` takes a text"));
        assert!(error(&rig.scan_for(&["2000"])).contains("from 1 to 1024"));
        assert!(error(&rig.scan_for(&["10", "20"])).contains("one distance"));
        assert!(error(&rig.scan_for(&["-o", "a b c"])).contains("\"a b c\""));
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.grid.reach, 32.0);
    }

    #[test]
    fn scan_waits_for_the_surroundings_to_settle() {
        let mut rig = Rig::among_objects();
        // The object scan has only just started.
        rig.grid.settled_at = Some(rig.now + Duration::from_secs(3));
        let answer = rig.scan_for(&[]);
        for _ in 0..11 {
            rig.look();
            assert!(answer.try_recv().is_err());
            rig.now += SCAN_POLL;
        }
        rig.now += SCAN_POLL;
        rig.look();
        assert_eq!(listed(&answer), ["Bench", "Lamp"]);
    }

    #[test]
    fn scan_gives_up_on_the_simulator_after_thirty_seconds() {
        let mut rig = Rig::among_objects();
        rig.grid.sightings.push(seen(3, 5.0, None));
        let plain = rig.scan_for(&[]);
        let by_name = rig.scan_for(&["-n", "e"]);
        let by_owner = rig.scan_for(&["-o", &jane().to_string()]);
        rig.look();
        rig.now += SCAN_LIMIT - SCAN_POLL;
        rig.look();
        assert!(plain.try_recv().is_err() && by_name.try_recv().is_err() && by_owner.try_recv().is_err());
        rig.now += SCAN_POLL;
        rig.look();
        // What the simulator did not name is listed as unknown, and meets
        // no filter on what it is called or whose it is.
        let Ok(Response::Ok(Some(text))) = plain.try_recv() else {
            panic!("no listing");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], format!("{}\t5.0\t-\t?\t?\t?\t?", Uuid::from_bytes([3; 16])));
        assert_eq!(listed(&by_name), ["Bench"]);
        assert_eq!(listed(&by_owner), ["Bench"]);
        assert_eq!(rig.commands.next_deadline(), None);
    }

    #[test]
    fn scan_asks_only_for_what_it_reaches() {
        let mut rig = Rig::new();
        let scripted = Sighting {
            flags: FLAG_SCRIPTED,
            ..seen(3, 4.0, None)
        };
        let hat = Sighting {
            attachment: true,
            ..seen(4, 2.0, None)
        };
        let cushion = Sighting {
            child: true,
            ..seen(5, 3.0, None)
        };
        rig.grid.sightings = vec![seen(1, 3.0, None), seen(2, 50.0, None), scripted, hat, cushion];
        let asked = |rig: &mut Rig, args: &[&str]| -> Vec<u32> {
            let _answer = rig.scan_for(args);
            rig.look();
            rig.commands.fail_scans("done");
            match &rig.calls()[..] {
                [Call::Reach(_), Call::Properties(ids)] => ids.clone(),
                [Call::Reach(_)] => Vec::new(),
                other => panic!("unexpected calls: {other:?}"),
            }
        };
        assert_eq!(asked(&mut rig, &["10"]), [1, 3]);
        assert_eq!(asked(&mut rig, &["100"]), [1, 2, 3]);
        assert_eq!(asked(&mut rig, &["10", "-s"]), [3]);
        assert_eq!(asked(&mut rig, &["10", "-a"]), [1, 3, 4]);
        assert_eq!(asked(&mut rig, &["10", "-c", "-n", "cushion"]), [1, 3, 5]);
        assert_eq!(asked(&mut rig, &["1"]), Vec::<u32>::new());
    }

    #[test]
    fn a_second_scan_of_the_same_objects_asks_nothing() {
        let mut rig = Rig::among_objects();
        let answer = rig.scan_for(&["20"]);
        rig.look();
        assert_eq!(listed(&answer), ["Bench", "Lamp"]);
        assert_eq!(rig.calls(), [Call::Reach(Some(20.0))]);
        assert_eq!(rig.commands.take_name_lookups(), Vec::<Uuid>::new());
    }

    #[test]
    fn scan_asks_for_the_names_of_the_owners_it_lists() {
        let mut rig = Rig::new();
        rig.grid.sightings = vec![
            seen(1, 3.0, Some(("Bench", jane()))),
            seen(2, 4.0, Some(("Table", jane()))),
            seen(3, 10.0, Some(("Lamp", john()))),
        ];
        // Only the lamp is listed: Jane's name is not needed.
        let answer = rig.scan_for(&["-n", "lamp"]);
        rig.look();
        assert_eq!(rig.commands.take_name_lookups(), [john()]);
        // The name service does not know John.
        rig.named_by_the_grid(&[], &[john()]);
        rig.now += SCAN_POLL;
        rig.look();
        let Ok(Response::Ok(Some(text))) = answer.try_recv() else {
            panic!("no listing");
        };
        assert!(text.ends_with(&format!("\t{}\t?\tLamp\t", john())), "{text}");

        // Each owner once, and John again: he may have a name by now.
        let answer = rig.scan_for(&[]);
        rig.look();
        let mut wanted = rig.commands.take_name_lookups();
        wanted.sort();
        assert_eq!(wanted, [jane(), john()]);
        rig.named_by_the_grid(&[names_of(jane(), "Jane Doe"), names_of(john(), "John Roe")], &[]);
        rig.now += SCAN_POLL;
        rig.look();
        assert_eq!(listed(&answer), ["Bench", "Table", "Lamp"]);
    }

    #[test]
    fn scan_by_an_owners_name_waits_for_the_names() {
        let mut rig = Rig::new();
        rig.grid.sightings = vec![seen(1, 3.0, Some(("Bench", jane()))), seen(2, 10.0, Some(("Lamp", john())))];
        let janes = rig.scan_for(&["-o", "jane doe"]);
        let nobodys = rig.scan_for(&["-o", "Nobody Here"]);
        rig.look();
        // Two scans, one lookup of each owner.
        let mut wanted = rig.commands.take_name_lookups();
        wanted.sort();
        assert_eq!(wanted, [jane(), john()]);
        assert!(janes.try_recv().is_err() && nobodys.try_recv().is_err());
        rig.named_by_the_grid(&[names_of(jane(), "Jane Doe"), names_of(john(), "John Roe")], &[]);
        rig.now += SCAN_POLL;
        rig.look();
        assert_eq!(listed(&janes), ["Bench"]);
        assert_eq!(nobodys.try_recv(), Ok(Response::Ok(None)));
    }

    #[test]
    fn a_waiting_scan_fails_when_the_avatar_leaves() {
        let mut rig = Rig::new();
        rig.grid.sightings = vec![seen(1, 3.0, None)];
        let answer = rig.scan_for(&["96"]);
        rig.look();
        rig.commands.simulator_changed();
        assert_eq!(error(&answer), "the avatar left the region");
        assert_eq!(rig.commands.next_deadline(), None);

        let answer = rig.scan_for(&[]);
        rig.commands.session_lost();
        assert_eq!(error(&answer), "the session was lost");
        // Nothing is left to look at, with or without a session.
        rig.look();
        rig.commands.scans_due(None::<&mut FakeGrid>, rig.now);
        assert!(answer.try_recv().is_err());
    }

    #[test]
    fn scan_leaves_the_avatar_and_the_placement_alone() {
        let mut rig = Rig::among_objects();
        let answer = rig.scan_for(&["96", "-a", "-c"]);
        rig.look();
        assert_eq!(listed(&answer), ["Bench", "Lamp"]);
        assert_eq!(rig.calls(), [Call::Reach(Some(96.0))]);
        assert!(!rig.commands.take_manual());
        assert_eq!((rig.commands.last_tp(), rig.commands.last_sit()), (None, None));
    }
}
