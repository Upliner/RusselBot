//! RLV, the RestrainedLove API: scripted objects say `@` commands to their
//! owner with `llOwnerSay`, and a viewer that implements the API carries
//! them out. This is the part of it the bot has: reading the commands,
//! answering the version queries, and carrying out the force commands that
//! have a control-socket command, keeping the restrictions the objects set,
//! and answering what is asked about them. `Limits` is where the rest of
//! the bot asks whether a restriction stands in the way of what it is about
//! to do. Much of the API is still missing, which the version the bot
//! reports says.

use crate::commands::{Commands, Grid, Origin};
use crate::config::{PersistOverride, RlvPersist};
pub use crate::control::RlvMode as Mode;
use crate::control::{Request, Response};
use crate::inventory;
use crate::persist::SitState;
use crate::session::{Look, Message, Placement, Source, Thing, NOT_CONNECTED};
use crate::udp::messages::Vector3;
use std::collections::VecDeque;
use std::fmt;
use std::sync::mpsc;
use std::time::Instant;
use uuid::Uuid;

/// The version of the RLV API the bot implements. Far below any viewer's,
/// so that a script that looks at it sees an incomplete implementation.
pub const VERSION: &str = "0.1.0";
/// `VERSION` as `@versionnum` gives it: X·10⁶ + Y·10⁴ + Z·10² + P.
pub const VERSION_NUM: u32 = 10_000;
/// How many messages are kept until the avatar can act.
pub const MAX_WAITING: usize = 64;
/// The chat type of the answer to a query: said at normal volume.
const SAY: u8 = 1;
/// A region's side in metres, for global coordinates.
const REGION_SIDE: f64 = 256.0;
const NOT_SUPPORTED: &str = "not supported";

/// The names RLV has for attachment points where the bot's own names
/// differ: the points of the heads-up display without `HUD`, and the root.
const RLV_POINTS: [(&str, u8); 9] = [
    ("center 2", 31),
    ("top right", 32),
    ("top", 33),
    ("top left", 34),
    ("center", 35),
    ("bottom left", 36),
    ("bottom", 37),
    ("bottom right", 38),
    ("root", 40),
];

// ---------------------------------------------------------------- parsing

/// What a command asks for, by its param.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    /// Do it once, now.
    Force,
    /// Answer on this chat channel; `None` for a number no channel can be.
    Channel(Option<i32>),
    /// Set (`n`, `add`) or lift (`y`, `rem`) a restriction.
    Restrict(bool),
    /// None of these: `@clear` comes alone, or with a filter after its
    /// `=`.
    Absent,
}

/// One RLV command: `behaviour[:option]=param`.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// The command as it was written, for the log and for answers.
    pub text: String,
    /// In lower case.
    pub behaviour: String,
    /// As it was written: it carries names and UUIDs.
    pub option: Option<String>,
    pub param: Param,
}

/// A piece of a message that is no RLV command.
#[derive(Debug, Clone, PartialEq)]
pub struct Malformed {
    pub text: String,
    pub reason: &'static str,
}

/// Reads the commands of a message, with or without its leading `@`: the
/// pieces between commas, in order. Nothing is trimmed; RLV commands are
/// sensitive to spaces.
pub fn parse(text: &str) -> Vec<Result<Command, Malformed>> {
    let text = text.strip_prefix('@').unwrap_or(text);
    text.split(',').filter(|piece| !piece.is_empty()).map(parse_one).collect()
}

fn parse_one(piece: &str) -> Result<Command, Malformed> {
    let malformed = |reason| Malformed {
        text: piece.to_string(),
        reason,
    };
    let Some((left, param)) = piece.split_once('=') else {
        // `@clear` alone lifts everything its sender has set.
        if !piece.eq_ignore_ascii_case("clear") {
            return Err(malformed("it has no `=`"));
        }
        return Ok(Command {
            text: piece.to_string(),
            behaviour: "clear".to_string(),
            option: None,
            param: Param::Absent,
        });
    };
    let (behaviour, option) = match left.split_once(':') {
        Some((behaviour, option)) => (behaviour, Some(option.to_string())),
        None => (left, None),
    };
    if behaviour.is_empty() {
        return Err(malformed("it names no behaviour"));
    }
    let lowered = param.to_ascii_lowercase();
    let filter = behaviour.eq_ignore_ascii_case("clear") && option.is_none();
    let digits = lowered.strip_prefix(['-', '+']).unwrap_or(&lowered);
    let param = match lowered.as_str() {
        "force" => Param::Force,
        _ if filter => Param::Absent,
        "n" | "add" => Param::Restrict(true),
        "y" | "rem" => Param::Restrict(false),
        _ if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => Param::Channel(lowered.parse().ok()),
        _ => return Err(malformed("its param is not `force`, a channel number, `n`, `add`, `y` or `rem`")),
    };
    Ok(Command {
        text: piece.to_string(),
        behaviour: behaviour.to_ascii_lowercase(),
        option,
        param,
    })
}

// ----------------------------------------------------------------- senders

/// Who sent RLV commands: an object the avatar owns, or the operator
/// through `rlv cmd`, who stands in for the avatar itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Issuer {
    pub id: Uuid,
    pub name: String,
    pub operator: bool,
}

impl Issuer {
    /// The operator, with the avatar's own id as a viewer's console has it.
    pub fn operator(avatar: Uuid) -> Issuer {
        Issuer {
            id: avatar,
            name: String::new(),
            operator: true,
        }
    }
}

impl Issuer {
    /// The sender as a holder of restrictions.
    pub fn who(&self) -> Who {
        match self.operator {
            true => Who::Operator,
            false => Who::Object(self.id),
        }
    }
}

impl fmt::Display for Issuer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.operator {
            true => f.write_str("operator"),
            false => write!(f, "{} <{}>", self.name, self.id),
        }
    }
}

/// The sender of the RLV commands in a chat message, if it carries any: one
/// an object said to its owner and that starts with `@`.
pub fn sender(message: &Message) -> Option<Issuer> {
    let Source::Object { id: Some(id), name, .. } = &message.source else {
        return None;
    };
    (message.kind == "owner" && message.text.starts_with('@')).then(|| Issuer {
        id: *id,
        name: name.clone(),
        operator: false,
    })
}

// ------------------------------------------------------------ restrictions

/// The most restrictions kept, over all holders.
pub const MAX_RESTRICTIONS: usize = 1024;
/// How far `sittp`, `fartouch` and `touchfar` let the avatar reach when
/// they give no distance, in metres.
const DEFAULT_REACH: f32 = 1.5;
/// The longest reply to a status query, in bytes: what a chat line holds.
const MAX_REPLY: usize = 1023;
const WHISPER: u8 = 0;
const SHOUT: u8 = 2;
/// The signs that make `sendchat` discard an emote or a slash word.
const SPECIAL_SIGNS: [char; 8] = ['(', ')', '"', '-', '*', '=', '_', '^'];
/// How much of an emote, and of other text that starts with a slash,
/// `sendchat` lets through, in characters.
const EMOTE_LENGTH: usize = 30;
const SLASH_LENGTH: usize = 15;

/// The restrictions that change what the bot does.
const ENFORCED: &[&str] = &[
    "sendchat",
    "emote",
    "chatshout",
    "chatnormal",
    "chatwhisper",
    "redirchat",
    "rediremote",
    "sendchannel",
    "sendchannel_sec",
    "sendchannel_except",
    "sendim",
    "sendim_sec",
    "sendimto",
    "startim",
    "startimto",
    "unsit",
    "sit",
    "sittp",
    "interact",
    "tploc",
    "tplocal",
    "tplure",
    "tplure_sec",
    "tprequest",
    "tprequest_sec",
    "accepttp",
    "accepttprequest",
    "touchall",
    "touchworld",
    "touchthis",
    "touchme",
    "touchattach",
    "touchattachself",
    "touchattachother",
    "touchhud",
    "fartouch",
    "touchfar",
    "detach",
    "addattach",
    "remattach",
    "addoutfit",
    "remoutfit",
    "acceptpermission",
    "denypermission",
    "showinv",
    "permissive",
];

/// The restrictions that are kept and reported and change nothing: they
/// are about what a viewer shows its user, or about things the bot has not.
const KEPT: &[&str] = &[
    "fly",
    "temprun",
    "alwaysrun",
    "camzoommax",
    "camzoommin",
    "setcam_fovmin",
    "setcam_fovmax",
    "camdistmax",
    "setcam_avdistmax",
    "camdistmin",
    "setcam_avdistmin",
    "camdrawmin",
    "camdrawmax",
    "camdrawalphamin",
    "camdrawalphamax",
    "camdrawcolor",
    "camunlock",
    "setcam_unlock",
    "camavdist",
    "camtextures",
    "setcam_textures",
    "recvchat",
    "recvchat_sec",
    "recvchatfrom",
    "recvemote",
    "recvemote_sec",
    "recvemotefrom",
    "recvim",
    "recvim_sec",
    "recvimfrom",
    "sendgesture",
    "tplm",
    "standtp",
    "viewnote",
    "viewscript",
    "viewtexture",
    "edit",
    "editobj",
    "editworld",
    "editattach",
    "rez",
    "share",
    "share_sec",
    "defaultwear",
    "unsharedwear",
    "unsharedunwear",
    "sharedwear",
    "sharedunwear",
    "detachthis",
    "detachallthis",
    "attachthis",
    "attachallthis",
    "detachthis_except",
    "detachallthis_except",
    "attachthis_except",
    "attachallthis_except",
    "showworldmap",
    "showminimap",
    "showloc",
    "shownames",
    "shownames_sec",
    "shownametags",
    "shownearby",
    "showhovertextall",
    "showhovertext",
    "showhovertexthud",
    "showhovertextworld",
    "setgroup",
    "setdebug",
    "setenv",
    "allowidle",
];

/// What a restriction the bot keeps does to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    Enforced,
    Kept,
}

/// Whether the bot keeps restrictions of this behaviour, and to what end.
pub fn effect(behaviour: &str) -> Option<Effect> {
    if ENFORCED.contains(&behaviour) {
        Some(Effect::Enforced)
    } else if KEPT.contains(&behaviour) {
        Some(Effect::Kept)
    } else {
        None
    }
}

/// Who holds a restriction: the operator, through `rlv cmd`, or an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Operator,
    Object(Uuid),
}

/// One restriction or exception, as its holder set it.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    /// In lower case.
    behaviour: String,
    /// As it was written.
    option: Option<String>,
}

impl Rule {
    fn is(&self, behaviour: &str, option: Option<&str>) -> bool {
        self.behaviour == behaviour
            && match (&self.option, option) {
                (None, None) => true,
                (Some(own), Some(other)) => own.to_lowercase() == other.to_lowercase(),
                _ => false,
            }
    }

    /// The restriction itself, without an option.
    fn plain(&self, behaviour: &str) -> bool {
        self.behaviour == behaviour && self.option.is_none()
    }

    fn text(&self) -> String {
        match &self.option {
            Some(option) => format!("{}:{option}", self.behaviour),
            None => self.behaviour.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Holder {
    who: Who,
    /// The object's name; empty for the operator.
    name: String,
    rules: Vec<Rule>,
}

impl Holder {
    /// The holder at the start of a line of `rlv status`.
    fn label(&self) -> String {
        match self.who {
            Who::Operator => "operator".to_string(),
            Who::Object(id) => format!("{} <{id}>", self.name),
        }
    }

    /// The holder in a sentence.
    fn named(&self) -> String {
        match self.who {
            Who::Operator => "the operator".to_string(),
            Who::Object(_) => self.label(),
        }
    }
}

/// What became of setting a restriction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Set {
    New,
    Already,
    /// Not kept: `MAX_RESTRICTIONS` are kept already.
    Full,
}

/// The restrictions the bot keeps: for every holder, in the order the
/// holders first set one, what it has set, in the order it set them.
#[derive(Debug, Default)]
pub struct Restrictions {
    holders: Vec<Holder>,
}

impl Restrictions {
    pub fn count(&self) -> usize {
        self.holders.iter().map(|holder| holder.rules.len()).sum()
    }

    fn holder(&mut self, who: Who) -> Option<&mut Holder> {
        self.holders.iter_mut().find(|holder| holder.who == who)
    }

    /// Forgets the holders that hold nothing any more.
    fn tidy(&mut self) {
        self.holders.retain(|holder| !holder.rules.is_empty());
    }

    /// Sets a restriction for `who`, known by `name`.
    pub fn set(&mut self, who: Who, name: &str, behaviour: &str, option: Option<&str>) -> Set {
        let held = |holder: &Holder| holder.rules.iter().any(|rule| rule.is(behaviour, option));
        if self.holders.iter().any(|holder| holder.who == who && held(holder)) {
            return Set::Already;
        }
        if self.count() >= MAX_RESTRICTIONS {
            return Set::Full;
        }
        let rule = Rule {
            behaviour: behaviour.to_string(),
            option: option.map(str::to_string),
        };
        match self.holder(who) {
            Some(holder) => {
                // An object may have been renamed since.
                holder.name = name.to_string();
                holder.rules.push(rule);
            }
            None => self.holders.push(Holder {
                who,
                name: name.to_string(),
                rules: vec![rule],
            }),
        }
        Set::New
    }

    /// Lifts a restriction of `who`. Returns whether it was held.
    pub fn lift(&mut self, who: Who, behaviour: &str, option: Option<&str>) -> bool {
        let Some(holder) = self.holder(who) else {
            return false;
        };
        let before = holder.rules.len();
        holder.rules.retain(|rule| !rule.is(behaviour, option));
        let lifted = holder.rules.len() < before;
        if lifted {
            self.tidy();
        }
        lifted
    }

    /// Lifts the restrictions of `who` whose text contains `part`, all of
    /// them without one. Returns how many there were.
    pub fn clear(&mut self, who: Who, part: Option<&str>) -> usize {
        let Some(holder) = self.holder(who) else {
            return 0;
        };
        let part = part.unwrap_or_default().to_lowercase();
        let before = holder.rules.len();
        holder.rules.retain(|rule| !rule.text().to_lowercase().contains(&part));
        let lifted = before - holder.rules.len();
        if lifted > 0 {
            self.tidy();
        }
        lifted
    }

    /// Drops what the object `id` holds. Returns its label and how many
    /// restrictions it held, if it held any.
    pub fn drop_holder(&mut self, id: Uuid) -> Option<(String, usize)> {
        let index = self.holders.iter().position(|holder| holder.who == Who::Object(id))?;
        let holder = self.holders.remove(index);
        Some((holder.label(), holder.rules.len()))
    }

    /// Drops what objects other than `worn` hold. Returns each such
    /// holder's label and how many restrictions it held.
    pub fn keep_only(&mut self, worn: &[Uuid]) -> Vec<(String, usize)> {
        let stays = |holder: &Holder| match holder.who {
            Who::Operator => true,
            Who::Object(id) => worn.contains(&id),
        };
        let (kept, dropped): (Vec<Holder>, Vec<Holder>) = std::mem::take(&mut self.holders).into_iter().partition(stays);
        self.holders = kept;
        dropped.iter().map(|holder| (holder.label(), holder.rules.len())).collect()
    }

    /// Drops what every object holds. Returns how many objects and how
    /// many restrictions that were.
    pub fn drop_objects(&mut self) -> (usize, usize) {
        let dropped = self.keep_only(&[]);
        (dropped.len(), dropped.iter().map(|(_, count)| count).sum())
    }

    /// Drops everything, the operator's too. Returns how many there were.
    pub fn reset(&mut self) -> usize {
        let count = self.count();
        if count > 0 {
            self.holders.clear();
        }
        count
    }

    /// Sets or lifts a restriction of `issuer`, or clears what it holds,
    /// if that is what `command` asks for and the bot keeps restrictions
    /// of that behaviour.
    pub fn apply(&mut self, issuer: &Issuer, command: &Command) -> Option<Step> {
        let who = issuer.who();
        let text = &command.text;
        let option = command.option.as_deref();
        let outcome = match (command.param, command.behaviour.as_str()) {
            (Param::Absent, "clear") => {
                let part = text.split_once('=').map(|(_, part)| part);
                format!("lifted {}", counted(self.clear(who, part)))
            }
            (Param::Restrict(_), behaviour) if effect(behaviour).is_none() => return None,
            (Param::Restrict(true), behaviour) => match self.set(who, &issuer.name, behaviour, option) {
                Set::New => "set".to_string(),
                Set::Already => "set already".to_string(),
                Set::Full => {
                    let reason = format!("{MAX_RESTRICTIONS} restrictions are kept already");
                    crate::info!("rlv: {issuer}: {text}: refused: {reason}");
                    return Some(Step::Failed(reason));
                }
            },
            (Param::Restrict(false), behaviour) => match self.lift(who, behaviour, option) {
                true => "lifted".to_string(),
                false => "was not set".to_string(),
            },
            _ => return None,
        };
        crate::info!("rlv: {issuer}: {text}: {outcome}");
        Some(Step::Done)
    }

    /// The restriction lines of `rlv status`.
    pub fn lines(&self) -> Vec<String> {
        let count = self.count();
        if count == 0 {
            return vec!["restrictions: none".to_string()];
        }
        let mut lines = vec![format!("restrictions: {count}")];
        for holder in &self.holders {
            for rule in &holder.rules {
                let note = match effect(&rule.behaviour) {
                    Some(Effect::Enforced) => "",
                    _ => " (no effect)",
                };
                lines.push(format!("{}: {}{note}", holder.label(), rule.text()));
            }
        }
        lines
    }

    /// The reply to `@getstatus` (the restrictions of `who`) or to
    /// `@getstatusall` (everyone's): each restriction whose text contains
    /// `part`, with `separator` in front of it.
    pub fn status(&self, who: Option<Who>, part: &str, separator: &str) -> String {
        let separator = if separator.is_empty() { "/" } else { separator };
        let part = part.to_lowercase();
        let mut reply = String::new();
        let holders = self.holders.iter().filter(|holder| who.is_none_or(|who| holder.who == who));
        for rule in holders.flat_map(|holder| &holder.rules) {
            let text = rule.text();
            if text.to_lowercase().contains(&part) {
                reply.push_str(separator);
                reply.push_str(&text);
            }
        }
        let mut end = reply.len().min(MAX_REPLY);
        while !reply.is_char_boundary(end) {
            end -= 1;
        }
        reply.truncate(end);
        reply
    }
}

// --------------------------------------------------------------- questions

/// Why something is not done: a restriction, and one holder of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    /// The restriction's text.
    pub rule: String,
    pub holder: String,
    /// What the restriction alone does not explain.
    pub note: Option<String>,
}

impl Refusal {
    /// For the log, and for whoever cannot be let through.
    pub fn short(&self) -> String {
        match &self.note {
            Some(note) => format!("{}, held by {} ({note})", self.rule, self.holder),
            None => format!("{}, held by {}", self.rule, self.holder),
        }
    }

    /// For the operator, whose next command override would let through.
    pub fn answer(&self) -> String {
        format!(
            "refused by RLV: {}; `rslbot ctl rlv mode override` lets one command through",
            self.short()
        )
    }

    fn noted(mut self, note: impl Into<String>) -> Refusal {
        self.note = Some(note.into());
        self
    }
}

/// The behaviours of the rows of "Touching": held, they concern a touch.
const TOUCH_ROWS: [&str; 11] = [
    "interact",
    "touchall",
    "touchworld",
    "touchworld_sec",
    "touchthis",
    "touchattach",
    "touchattachself",
    "touchattachother",
    "touchhud",
    "fartouch",
    "touchfar",
];

/// What the restrictions say of something done to an object, which, when
/// the simulator has not described it, may turn out to be near or far,
/// in-world or worn.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    /// Allowed whatever the object turns out to be.
    Allowed,
    /// Allowed for some of what the object may turn out to be: it is to
    /// be looked for first, and decided by what it then is. The refusal
    /// is why it would not be done as things stand, where that is known.
    LookFirst(Option<Refusal>),
    /// Refused whatever the object turns out to be.
    Refused(Refusal),
}

impl From<Result<(), Refusal>> for Check {
    fn from(result: Result<(), Refusal>) -> Check {
        match result {
            Ok(()) => Check::Allowed,
            Err(refusal) => Check::Refused(refusal),
        }
    }
}

/// What an option of a restriction may name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Id(Uuid),
    Channel(i32),
    Point(u8),
    Kind(u8),
}

impl Target {
    /// Whether `option` names this. One that cannot be read names nothing.
    fn named_by(self, option: &str) -> bool {
        match self {
            Target::Id(id) => Uuid::parse_str(option.trim()) == Ok(id),
            Target::Channel(channel) => option.trim().parse() == Ok(channel),
            Target::Point(wanted) => point(option) == Some(wanted),
            Target::Kind(kind) => inventory::parse_kind(option) == Some(kind),
        }
    }
}

/// Whether, and on what, the avatar sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sitting {
    No,
    Ground,
    Object,
}

impl Sitting {
    pub fn seated(self) -> bool {
        self != Sitting::No
    }
}

/// One thing to say: the outcome of `Limits::chat`.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub channel: i32,
    /// 0 whisper, 1 say, 2 shout.
    pub volume: u8,
    pub text: String,
}

/// What a chat message became under the restrictions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spoken {
    pub utterances: Vec<Utterance>,
    /// What was changed, and for which restriction.
    pub changes: Vec<String>,
}

/// The restrictions that are in force for someone who is about to act:
/// everything outside this module asks its questions here.
#[derive(Debug, Clone, Copy)]
pub struct Limits<'a> {
    store: Option<&'a Restrictions>,
    /// A holder whose own restrictions do not count: the sender of a
    /// force command.
    except: Option<Who>,
}

impl<'a> Limits<'a> {
    /// No restriction is in force: RLV is off, or the command is exempt.
    pub fn none() -> Limits<'static> {
        Limits { store: None, except: None }
    }

    pub fn new(store: Option<&'a Restrictions>, except: Option<Who>) -> Limits<'a> {
        Limits { store, except }
    }

    /// Every holder: exceptions count whoever holds them.
    fn all(&self) -> impl Iterator<Item = &'a Holder> {
        self.store.into_iter().flat_map(|store| &store.holders)
    }

    /// The holders whose restrictions count.
    fn holders(&self) -> impl Iterator<Item = &'a Holder> {
        let except = self.except;
        self.all().filter(move |holder| Some(holder.who) != except)
    }

    fn refusal(holder: &Holder, rule: &Rule) -> Refusal {
        Refusal {
            rule: rule.text(),
            holder: holder.named(),
            note: None,
        }
    }

    /// The first rule the holders that count have that `wanted` accepts.
    fn find(&self, wanted: impl Fn(&Holder, &Rule) -> bool) -> Option<Refusal> {
        self.holders().find_map(|holder| {
            holder
                .rules
                .iter()
                .find(|rule| wanted(holder, rule))
                .map(|rule| Limits::refusal(holder, rule))
        })
    }

    /// Whether anyone at all holds `behaviour` without an option.
    fn anyone(&self, behaviour: &str) -> bool {
        self.all().any(|holder| holder.rules.iter().any(|rule| rule.plain(behaviour)))
    }

    /// `behaviour`, held without an option.
    pub fn in_force(&self, behaviour: &str) -> Option<Refusal> {
        self.find(|_, rule| rule.plain(behaviour))
    }

    /// `behaviour`, or its secure form, held without an option by a holder
    /// against which no exception for `targets` counts. An exception is
    /// `behaviour` with one of the targets as its option: anyone's, or for
    /// the secure form and under `permissive` the same holder's only.
    fn against_any(&self, behaviour: &str, targets: &[Target]) -> Option<Refusal> {
        let secure = format!("{behaviour}_sec");
        let permissive = self.anyone("permissive");
        let excepts = |holder: &Holder| {
            let names = |rule: &Rule| {
                rule.behaviour == behaviour
                    && rule
                        .option
                        .as_deref()
                        .is_some_and(|option| targets.iter().any(|target| target.named_by(option)))
            };
            holder.rules.iter().any(names)
        };
        let anyone_excepts = self.all().any(excepts);
        self.find(|holder, rule| {
            let strict = rule.plain(&secure) || (permissive && rule.plain(behaviour));
            let loose = rule.plain(behaviour) && !permissive;
            (strict && !excepts(holder)) || (loose && !anyone_excepts)
        })
    }

    pub fn against(&self, behaviour: &str, target: Target) -> Option<Refusal> {
        self.against_any(behaviour, &[target])
    }

    /// `behaviour` held with `target` as its option.
    pub fn names(&self, behaviour: &str, target: Target) -> Option<Refusal> {
        self.find(|_, rule| rule.behaviour == behaviour && rule.option.as_deref().is_some_and(|option| target.named_by(option)))
    }

    /// The smallest distance the holders of these behaviours give, with a
    /// holder of it: `DEFAULT_REACH` for one that gives none.
    fn reach(&self, behaviours: &[&str]) -> Option<(f32, Refusal)> {
        let mut smallest: Option<(f32, Refusal)> = None;
        for holder in self.holders() {
            for rule in holder.rules.iter().filter(|rule| behaviours.contains(&rule.behaviour.as_str())) {
                let given = rule.option.as_deref().and_then(|option| option.trim().parse::<f32>().ok());
                let reach = given.filter(|reach| reach.is_finite() && *reach >= 0.0).unwrap_or(DEFAULT_REACH);
                if smallest.as_ref().is_none_or(|(least, _)| reach < *least) {
                    smallest = Some((reach, Limits::refusal(holder, rule)));
                }
            }
        }
        smallest
    }

    /// Whether something at `distance` is out of the reach these
    /// behaviours leave.
    fn out_of_reach(&self, behaviours: &[&str], distance: Option<f32>) -> Option<Refusal> {
        let (reach, refusal) = self.reach(behaviours)?;
        match distance {
            None => Some(refusal.noted("the bot does not know how far the object is")),
            Some(distance) if distance > reach => Some(refusal.noted(format!("the object is {distance:.1} m away"))),
            Some(_) => None,
        }
    }

    fn ok(refusal: Option<Refusal>) -> Result<(), Refusal> {
        match refusal {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    // -------------------------------------------------- what is being done

    /// Leaving a seat: standing up, or anything else that ends the sit.
    pub fn stand(&self, sitting: Sitting) -> Result<(), Refusal> {
        if !sitting.seated() {
            return Ok(());
        }
        Limits::ok(self.in_force("unsit"))
    }

    /// Sitting on an object at `distance`.
    pub fn sit(&self, sitting: Sitting, distance: Option<f32>) -> Result<(), Refusal> {
        Limits::ok(self.in_force("sit"))?;
        Limits::ok(self.in_force("interact"))?;
        self.stand(sitting)?;
        Limits::ok(self.out_of_reach(&["sittp"], distance))
    }

    /// Sitting on an object the simulator has not described, whose
    /// distance is therefore not known.
    pub fn sit_undescribed(&self, sitting: Sitting) -> Check {
        match self.sit(sitting, None) {
            Ok(()) => Check::Allowed,
            // Near enough, it would be allowed: worth looking for.
            Err(refusal) if self.sit(sitting, Some(0.0)).is_ok() => Check::LookFirst(Some(refusal)),
            Err(refusal) => Check::Refused(refusal),
        }
    }

    /// Sitting on `thing`, described or not.
    pub fn sit_on(&self, sitting: Sitting, thing: &Thing) -> Check {
        match thing.placement {
            Placement::Unknown => self.sit_undescribed(sitting),
            _ => Check::from(self.sit(sitting, thing.distance)),
        }
    }

    pub fn sit_ground(&self, sitting: Sitting) -> Result<(), Refusal> {
        Limits::ok(self.in_force("sit"))?;
        self.stand(sitting)
    }

    /// A teleport by coordinates. `local` is how far the destination is
    /// when it is in the region the avatar is in.
    pub fn teleport(&self, sitting: Sitting, local: Option<f32>) -> Result<(), Refusal> {
        self.forced_teleport(sitting)?;
        match local {
            Some(distance) => self.local_teleport(distance),
            None => Ok(()),
        }
    }

    /// What `@tpto` is held to: not `tplocal`.
    pub fn forced_teleport(&self, sitting: Sitting) -> Result<(), Refusal> {
        Limits::ok(self.in_force("tploc"))?;
        self.stand(sitting)
    }

    /// A teleport over `distance` inside the region: every holder of
    /// `tplocal` has to give a distance, and the smallest has to do.
    fn local_teleport(&self, distance: f32) -> Result<(), Refusal> {
        let mut widest: Option<Refusal> = None;
        for holder in self.holders() {
            for rule in holder.rules.iter().filter(|rule| rule.behaviour == "tplocal") {
                let given = rule.option.as_deref().and_then(|option| option.trim().parse::<f32>().ok());
                match given.filter(|reach| reach.is_finite()) {
                    Some(reach) if distance <= reach => {}
                    _ => widest = widest.or(Some(Limits::refusal(holder, rule))),
                }
            }
        }
        Limits::ok(widest)
    }

    /// Accepting a teleport offer of `from`.
    pub fn lure(&self, from: Uuid) -> Result<(), Refusal> {
        Limits::ok(self.against("tplure", Target::Id(from)))
    }

    /// Whether RLV has the bot accept the offers of `from` by itself.
    pub fn accepts_tp(&self, from: Uuid) -> bool {
        self.anyone("accepttp") || self.names("accepttp", Target::Id(from)).is_some()
    }

    /// Reading a request of `from` to be teleported to the avatar.
    pub fn tp_request(&self, from: Uuid) -> Result<(), Refusal> {
        Limits::ok(self.against("tprequest", Target::Id(from)))
    }

    /// Whether RLV has the bot answer such a request with an offer.
    pub fn answers_tp_request(&self, from: Uuid) -> bool {
        self.anyone("accepttprequest") || self.names("accepttprequest", Target::Id(from)).is_some()
    }

    /// Whether attach and control permissions are refused by RLV.
    pub fn denies_permissions(&self) -> Option<Refusal> {
        self.in_force("denypermission")
    }

    /// Whether attach and control permissions are granted by RLV.
    pub fn grants_permissions(&self) -> bool {
        self.anyone("acceptpermission")
    }

    /// An instant message to `to`. `talking` is whether the bot has a
    /// conversation with that avatar.
    pub fn im(&self, to: Uuid, talking: bool) -> Result<(), Refusal> {
        let to = Target::Id(to);
        Limits::ok(self.against("sendim", to))?;
        Limits::ok(self.names("sendimto", to))?;
        if talking {
            return Ok(());
        }
        Limits::ok(self.against("startim", to))?;
        Limits::ok(self.names("startimto", to))
    }

    pub fn inventory(&self) -> Result<(), Refusal> {
        Limits::ok(self.in_force("showinv"))
    }

    /// Touching `thing`.
    pub fn touch(&self, thing: &Thing) -> Result<(), Refusal> {
        Limits::ok(self.in_force("interact"))?;
        let prims = &thing.prims;
        let part = |who: Who| matches!(who, Who::Object(id) if prims.contains(&id));
        if self
            .all()
            .any(|holder| part(holder.who) && holder.rules.iter().any(|rule| rule.plain("touchme")))
        {
            return Ok(());
        }
        let ids: Vec<Target> = prims.iter().map(|prim| Target::Id(*prim)).collect();
        let names = |behaviour: &str| ids.iter().find_map(|id| self.names(behaviour, *id));
        Limits::ok(names("touchthis"))?;
        let (world, own, others) = match thing.placement {
            Placement::Hud => {
                Limits::ok(self.in_force("touchhud"))?;
                return Limits::ok(names("touchhud"));
            }
            Placement::Worn => (false, true, None),
            Placement::WornBy(avatar) => (false, false, Some(Some(avatar))),
            Placement::InWorld => (true, false, None),
            // It may be anything but the bot's own: all of it applies.
            Placement::Unknown => (true, false, Some(None)),
        };
        Limits::ok(self.in_force("touchall"))?;
        if world {
            Limits::ok(self.against_any("touchworld", &ids))?;
        }
        if own || others.is_some() {
            Limits::ok(self.in_force("touchattach"))?;
        }
        if own {
            Limits::ok(self.in_force("touchattachself"))?;
        }
        if let Some(avatar) = others {
            Limits::ok(self.in_force("touchattachother"))?;
            if let Some(avatar) = avatar {
                Limits::ok(self.names("touchattachother", Target::Id(avatar)))?;
            }
        }
        let distance = if own { Some(0.0) } else { thing.distance };
        Limits::ok(self.out_of_reach(&["fartouch", "touchfar"], distance))
    }

    /// Touching `object`, which the simulator has not described. Allowed
    /// when no row of the touch restrictions is held; refused when every
    /// kind of thing it may turn out to be, near or far, would be; to be
    /// looked for otherwise.
    pub fn touch_undescribed(&self, object: Uuid) -> Check {
        let row = |rule: &Rule| match rule.behaviour.as_str() {
            // With an option these are exceptions.
            "touchworld" | "touchworld_sec" => rule.option.is_none(),
            "interact" => rule.option.is_none(),
            behaviour => TOUCH_ROWS.contains(&behaviour),
        };
        if !self.holders().any(|holder| holder.rules.iter().any(row)) {
            return Check::Allowed;
        }
        let thing = |placement, distance| Thing {
            placement,
            prims: vec![object],
            distance: Some(distance),
        };
        let mut refusal = self.touch(&Thing::unknown(object)).err();
        let mut allowed = refusal.is_none();
        for placement in [Placement::Hud, Placement::Worn, Placement::WornBy(Uuid::nil()), Placement::InWorld] {
            for distance in [0.0, f32::INFINITY] {
                match self.touch(&thing(placement, distance)) {
                    Ok(()) => allowed = true,
                    Err(found) => {
                        refusal.get_or_insert(found);
                    }
                }
            }
        }
        match (allowed, refusal) {
            (false, Some(refusal)) => Check::Refused(refusal),
            (_, refusal) => Check::LookFirst(refusal),
        }
    }

    /// Touching `thing`, described or not.
    pub fn touch_on(&self, thing: &Thing) -> Check {
        match thing.placement {
            Placement::Unknown => self.touch_undescribed(thing.prims[0]),
            _ => Check::from(self.touch(thing)),
        }
    }

    /// Attaching something to `point`; `None` when the simulator chooses.
    pub fn attach(&self, point: Option<u8>) -> Result<(), Refusal> {
        Limits::ok(self.in_force("addattach"))?;
        let closes = |rule: &Rule| matches!(rule.behaviour.as_str(), "addattach" | "detach");
        match point {
            Some(point) => Limits::ok(
                self.find(|_, rule| closes(rule) && rule.option.as_deref().is_some_and(|option| Target::Point(point).named_by(option))),
            ),
            None => {
                let closed = self.find(|_, rule| closes(rule) && rule.option.as_deref().and_then(self::point).is_some());
                Limits::ok(closed.map(|refusal| refusal.noted("the point the object would go to is not known; name one")))
            }
        }
    }

    /// Detaching the attachment on `point` whose prims are `prims`.
    pub fn detach(&self, point: u8, prims: &[Uuid]) -> Result<(), Refusal> {
        let locked = |holder: &Holder, rule: &Rule| rule.plain("detach") && matches!(holder.who, Who::Object(id) if prims.contains(&id));
        Limits::ok(self.find(locked))?;
        Limits::ok(self.in_force("remattach"))?;
        Limits::ok(self.names("detach", Target::Point(point)))?;
        Limits::ok(self.names("remattach", Target::Point(point)))
    }

    /// Putting on clothing or a body part of `kind`; a body part replaces
    /// the one that is worn.
    pub fn wear(&self, kind: u8, replaces: bool) -> Result<(), Refusal> {
        Limits::ok(self.in_force("addoutfit"))?;
        Limits::ok(self.names("addoutfit", Target::Kind(kind)))?;
        if replaces {
            self.unwear(kind)?;
        }
        Ok(())
    }

    /// Taking off clothing of `kind`.
    pub fn unwear(&self, kind: u8) -> Result<(), Refusal> {
        Limits::ok(self.in_force("remoutfit"))?;
        Limits::ok(self.names("remoutfit", Target::Kind(kind)))
    }

    /// Whether the avatar may chat on a channel other than 0.
    fn private(&self, channel: i32) -> Result<(), Refusal> {
        let channel = Target::Channel(channel);
        Limits::ok(self.against("sendchannel", channel))?;
        Limits::ok(self.names("sendchannel_except", channel))
    }

    /// The channels `behaviour` redirects to, each with a holder of it.
    fn redirections(&self, behaviour: &str) -> Vec<(i32, Refusal)> {
        let mut channels: Vec<(i32, Refusal)> = Vec::new();
        for holder in self.holders() {
            for rule in holder.rules.iter().filter(|rule| rule.behaviour == behaviour) {
                let channel = rule.option.as_deref().and_then(|option| option.trim().parse::<i32>().ok());
                if let Some(channel) = channel.filter(|channel| !channels.iter().any(|(known, _)| known == channel)) {
                    channels.push((channel, Limits::refusal(holder, rule)));
                }
            }
        }
        channels
    }

    /// What becomes of `text`, to be said on `channel` at `volume`, after
    /// the chat bar has read it.
    pub fn chat(&self, channel: i32, volume: u8, text: &str) -> Result<Spoken, Refusal> {
        let mut spoken = Spoken::default();
        if channel != 0 {
            self.private(channel)?;
            spoken.utterances.push(Utterance {
                channel,
                volume,
                text: text.to_string(),
            });
            return Ok(spoken);
        }
        // The volume: whispers and shouts to normal, then normal to a whisper.
        let mut volume = volume;
        let loudness = |volume: u8| match volume {
            WHISPER => "whispered",
            SHOUT => "shouted",
            _ => "said at normal volume",
        };
        let mut turn = |volume: &mut u8, to: u8, refusal: Option<Refusal>| {
            if let Some(refusal) = refusal {
                let change = format!("{} instead of {}, because of {}", loudness(to), loudness(*volume), refusal.short());
                spoken.changes.push(change);
                *volume = to;
            }
        };
        match volume {
            WHISPER => turn(&mut volume, SAY, self.in_force("chatwhisper")),
            SHOUT => turn(&mut volume, SAY, self.in_force("chatshout")),
            _ => {}
        }
        if volume != WHISPER {
            turn(&mut volume, WHISPER, self.in_force("chatnormal"));
        }
        // Redirection, which takes the text off channel 0.
        let emote = text.starts_with("/me ") || text.starts_with("/me'");
        let redirected = self.redirections(if emote { "rediremote" } else { "redirchat" });
        if !redirected.is_empty() {
            let mut closed = None;
            for (to, refusal) in redirected {
                if to != 0 {
                    if let Err(private) = self.private(to) {
                        closed = closed.or(Some(private));
                        continue;
                    }
                }
                spoken
                    .changes
                    .push(format!("said on channel {to} instead of channel 0, because of {}", refusal.short()));
                spoken.utterances.push(Utterance {
                    channel: to,
                    volume: SAY,
                    text: text.to_string(),
                });
            }
            return match (spoken.utterances.is_empty(), closed) {
                (true, Some(closed)) => Err(closed.noted("the channel the text is redirected to")),
                _ => Ok(spoken),
            };
        }
        let mut text = text.to_string();
        if let Some(refusal) = self.in_force("sendchat") {
            if !text.starts_with('/') {
                return Err(refusal);
            }
            if text.contains(SPECIAL_SIGNS) {
                return Err(refusal.noted("the text has one of the signs ()\"-*=_^"));
            }
            if !(emote && self.anyone("emote")) {
                let whole = text.clone();
                if let Some(end) = text.find('.') {
                    text.truncate(end);
                }
                let length = if emote { EMOTE_LENGTH } else { SLASH_LENGTH };
                text = text.chars().take(length).collect::<String>().trim_end().to_string();
                if text != whole {
                    spoken
                        .changes
                        .push(format!("shortened to \"{text}\", because of {}", refusal.short()));
                }
            }
        }
        spoken.utterances.push(Utterance { channel: 0, volume, text });
        Ok(spoken)
    }
}

// ---------------------------------------------------------- the bot's own

/// What the bot is about to do by itself towards the placement it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OwnStep {
    /// To the held location; `local` is how far that is when it is in
    /// the region the avatar is in.
    Teleport {
        local: Option<f32>,
    },
    /// On the held object, which is this far away.
    Sit {
        distance: Option<f32>,
    },
    SitGround,
    /// Off the seat, for a held sit state that is no seat.
    Stand,
}

impl OwnStep {
    /// The word of the persist override setting this step goes by.
    pub fn sort(self) -> &'static str {
        match self {
            OwnStep::Teleport { .. } => "tp",
            _ => "sit",
        }
    }
}

/// Whether a step of the bot's own is started.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Go,
    /// A restriction stands in its way, and the persist override setting
    /// lets it through.
    LetThrough(Refusal),
    HeldBack(Refusal),
}

/// Holds a step of the bot's own to the restrictions, as the operator's
/// command for it would be, unless the persist override setting names
/// its sort.
pub fn own_step(limits: Limits, sitting: Sitting, step: OwnStep, overridden: PersistOverride) -> Verdict {
    let allowed = match step {
        OwnStep::Teleport { local } => limits.teleport(sitting, local),
        OwnStep::Sit { distance } => limits.sit(sitting, distance),
        OwnStep::SitGround => limits.sit_ground(sitting),
        OwnStep::Stand => limits.stand(sitting),
    };
    let exempt = match step {
        OwnStep::Teleport { .. } => overridden.tp,
        _ => overridden.sit,
    };
    match (allowed, exempt) {
        (Ok(()), _) => Verdict::Go,
        (Err(refusal), true) => Verdict::LetThrough(refusal),
        (Err(refusal), false) => Verdict::HeldBack(refusal),
    }
}

// ------------------------------------------------------------ carrying out

/// A forced movement the commands have carried out or begun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Movement {
    Sit,
    Ground,
    Stand,
    Teleport,
}

/// What became of a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A query: this was said on its channel.
    Said(String),
    /// Carried out or begun; the placement is not concerned.
    Done,
    /// A forced movement, carried out or begun as the control-socket
    /// command is: the commands went manual.
    Moved(Movement),
    /// A forced sit that is to be followed: nothing was done yet. The bot
    /// persists this object as its seat, which seats the avatar.
    HoldSeat(Uuid),
    /// A forced sit that is to be followed, on this object, which the
    /// simulator has not described: it is being looked for, and the
    /// command is to be dealt with again once it is described or the
    /// time to look has passed.
    Sought(Uuid),
    /// Not carried out, and why.
    Failed(String),
}

/// The reply to a version query.
fn version_reply(behaviour: &str) -> Option<String> {
    let bot = env!("CARGO_PKG_VERSION");
    match behaviour {
        // The old name is what the command has always answered with.
        "version" => Some(format!("RestrainedLife viewer v{VERSION} (RusselBot {bot})")),
        "versionnew" => Some(format!("RestrainedLove viewer v{VERSION} (RusselBot {bot})")),
        "versionnum" => Some(VERSION_NUM.to_string()),
        _ => None,
    }
}

/// The attachment point an RLV command names. A number is none: RLV has
/// no numbers here, and a folder may be called `5`.
fn point(option: &str) -> Option<u8> {
    if option.trim().parse::<u64>().is_ok() {
        return None;
    }
    let rlv = || RLV_POINTS.iter().find(|(name, _)| name.eq_ignore_ascii_case(option));
    inventory::parse_point(option).or_else(|| rlv().map(|(_, point)| *point))
}

/// Where a `@tpto` goes.
#[derive(Debug, PartialEq)]
enum Destination {
    /// The arguments of a `tp` command: x, y, z and the region.
    Named(Vec<String>),
    /// The handle of the region the point is in, and the point inside it.
    Global(u64, Vector3),
}

/// Reads the option of `@tpto`: `region/x/y/z`, or `X/Y/Z` in metres from
/// the grid's origin. What follows a `;` is a direction to look in.
fn destination(option: &str) -> Result<Destination, String> {
    let place = option.split(';').next().unwrap_or_default();
    let number = |text: &str, name: &str| match text.parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(format!("`{name}` must be a number that is not negative, not \"{text}\"")),
    };
    match place.split('/').collect::<Vec<_>>()[..] {
        [region, x, y, z] => {
            if region.trim().is_empty() {
                return Err("the destination names no region".to_string());
            }
            for (text, name) in [(x, "x"), (y, "y"), (z, "z")] {
                number(text, name)?;
            }
            Ok(Destination::Named([x, y, z, region].map(str::to_string).to_vec()))
        }
        [x, y, z] => {
            let (x, y, z) = (number(x, "X")?, number(y, "Y")?, number(z, "Z")?);
            let corner = |value: f64| (value / REGION_SIDE).floor() * REGION_SIDE;
            if x.max(y) >= f64::from(u32::MAX) {
                return Err("the destination is outside any grid".to_string());
            }
            let handle = ((corner(x) as u64) << 32) | corner(y) as u64;
            // No grid has a region there, and one asked for it never
            // answers. Most likely these are coordinates inside a region.
            if handle == 0 {
                return Err(format!(
                    "\"{place}\" are global coordinates in the region at 0, 0, where no grid has one; \
                     for a place inside a region write `region/x/y/z`"
                ));
            }
            Ok(Destination::Global(
                handle,
                [(x - corner(x)) as f32, (y - corner(y)) as f32, z as f32],
            ))
        }
        _ => Err(format!("\"{place}\" is neither `region/x/y/z` nor global `X/Y/Z`")),
    }
}

/// Carries out `request` as the control-socket command would be, for RLV.
/// Nobody waits for an answer that comes later; the log has it.
fn carry<G: Grid>(request: Request, who: Who, commands: &mut Commands, grid: Option<&mut G>, now: Instant) -> Result<(), String> {
    let (reply, answer) = mpsc::channel();
    commands.handle_from(Origin::Rlv(who), request, reply, grid, now);
    match answer.try_recv() {
        Ok(Response::Error(reason)) => Err(reason),
        // Answered, or put aside until the simulator has spoken.
        _ => Ok(()),
    }
}

/// Deals with one command of `issuer`. `follow` is the RLV persist
/// setting: a forced sit is then left to the bot (`Step::HoldSeat`).
pub fn deal<G: Grid>(
    command: &Command,
    issuer: &Issuer,
    follow: bool,
    commands: &mut Commands,
    grid: Option<&mut G>,
    now: Instant,
) -> Step {
    let text = &command.text;
    let refused = |reason: String| {
        crate::info!("rlv: {issuer}: {text}: refused: {reason}");
        Step::Failed(reason)
    };
    let unsupported = || {
        crate::info!("rlv: {issuer}: {text}: {NOT_SUPPORTED}");
        Step::Failed(NOT_SUPPORTED.to_string())
    };
    // Hands a force command to the control-socket command it stands for.
    let mut as_command = |shown: String, request: Request, done: Step, grid: Option<&mut G>| {
        crate::info!("rlv: {issuer}: {text}: as `{shown}`");
        match carry(request, issuer.who(), commands, grid, now) {
            Ok(()) => done,
            // The commands have logged why.
            Err(reason) => Step::Failed(reason),
        }
    };
    let option = command.option.as_deref();
    match (command.param, command.behaviour.as_str()) {
        (Param::Channel(channel), behaviour @ ("version" | "versionnew" | "versionnum" | "getstatus" | "getstatusall")) => {
            let channel = match channel {
                Some(0) => return refused("channel 0 cannot be answered on".to_string()),
                Some(channel) => channel,
                None => return refused("the channel is out of range".to_string()),
            };
            let Some(grid) = grid else {
                return refused(NOT_CONNECTED.to_string());
            };
            let reply = match version_reply(behaviour) {
                Some(reply) => reply,
                // What the sender holds, or what everyone does.
                None => {
                    let (part, separator) = option
                        .unwrap_or_default()
                        .split_once(';')
                        .unwrap_or((option.unwrap_or_default(), ""));
                    let who = (behaviour == "getstatus").then(|| issuer.who());
                    grid.restrictions()
                        .map(|kept| kept.status(who, part, separator))
                        .unwrap_or_default()
                }
            };
            match grid.say(channel, SAY, &reply, now) {
                Ok(()) => {
                    crate::info!("rlv: {issuer}: {text}: said \"{reply}\" on channel {channel}");
                    Step::Said(reply)
                }
                Err(reason) => refused(reason),
            }
        }
        (Param::Force, "sit") => {
            let given = option.unwrap_or_default();
            let Ok(object) = Uuid::parse_str(given) else {
                return refused(format!("\"{given}\" is not an object's UUID"));
            };
            if follow {
                // Left to the bot, which is held to the restrictions as
                // the sit would be: all but the sender's own.
                if let Some(grid) = grid {
                    let check = Limits::new(grid.restrictions(), Some(issuer.who())).sit_on(grid.sitting(), &grid.thing(object));
                    match check {
                        Check::Allowed => {}
                        Check::Refused(refusal) => return refused(refusal.short()),
                        Check::LookFirst(why) => match grid.look_for(object, now) {
                            Ok(Look::Waiting) => {
                                let why = why.map(|refusal| format!(": {}", refusal.short())).unwrap_or_default();
                                crate::info!("rlv: {issuer}: {text}: looking for {object} first{why}");
                                return Step::Sought(object);
                            }
                            // Decided by what is known now: too far when
                            // it was not found.
                            Ok(Look::Known | Look::GaveUp) => {
                                let limits = Limits::new(grid.restrictions(), Some(issuer.who()));
                                if let Err(refusal) = limits.sit(grid.sitting(), grid.thing(object).distance) {
                                    return refused(refusal.short());
                                }
                            }
                            Err(reason) => return refused(reason),
                        },
                    }
                }
                return Step::HoldSeat(object);
            }
            as_command(
                format!("sit {object}"),
                Request::Sit(object.to_string()),
                Step::Moved(Movement::Sit),
                grid,
            )
        }
        (Param::Force, "sitground") => {
            let request = Request::Sit("ground".to_string());
            as_command("sit ground".to_string(), request, Step::Moved(Movement::Ground), grid)
        }
        (Param::Force, "unsit") => as_command("stand".to_string(), Request::Stand, Step::Moved(Movement::Stand), grid),
        (Param::Force, "tpto") => match destination(option.unwrap_or_default()) {
            Err(reason) => refused(reason),
            Ok(Destination::Named(args)) => {
                let shown = format!("tp {}", args.join(" "));
                as_command(shown, Request::Tp(args), Step::Moved(Movement::Teleport), grid)
            }
            Ok(Destination::Global(handle, position)) => {
                crate::info!("rlv: {issuer}: {text}: as a teleport by global coordinates");
                match commands.tp_global(handle, position, issuer.who(), grid, now) {
                    Ok(()) => Step::Moved(Movement::Teleport),
                    Err(reason) => refused(reason),
                }
            }
        },
        // Without an option these take everything off, which no command of
        // the bot does; with one that is neither a point nor a UUID they
        // name a folder.
        (Param::Force, "detach" | "remattach") => {
            if let Some(point) = option.and_then(point) {
                let name = inventory::point_name(point);
                return as_command(format!("detach {name}"), Request::Detach(name), Step::Done, grid);
            }
            let Some(given) = option.and_then(|option| Uuid::parse_str(option).ok()) else {
                return unsupported();
            };
            // The UUID of an attachment, as a script knows it: its object.
            // One that is no such object is taken for the inventory item,
            // which is what the bot's own commands show.
            let attached = grid.as_ref().and_then(|grid| grid.attached_item(given));
            let item = attached.unwrap_or(given);
            as_command(format!("detach {item}"), Request::Detach(item.to_string()), Step::Done, grid)
        }
        (Param::Force, "remoutfit") => match option.map(|kind| (kind, inventory::parse_kind(kind))) {
            Some((_, Some(kind))) => {
                let name = inventory::kind_name(kind);
                as_command(format!("unwear {name}"), Request::Detach(name), Step::Done, grid)
            }
            Some((given, None)) => refused(format!("\"{given}\" is not a kind of clothing")),
            None => unsupported(),
        },
        (Param::Force, "detachme") => {
            let Some(grid) = grid else {
                return refused(NOT_CONNECTED.to_string());
            };
            let attached = if issuer.operator { None } else { grid.attached_item(issuer.id) };
            match attached {
                Some(item) => as_command(format!("detach {item}"), Request::Detach(item.to_string()), Step::Done, Some(grid)),
                None => refused("the sender is not attached".to_string()),
            }
        }
        _ => unsupported(),
    }
}

// ------------------------------------------------------------------- state

fn counted(count: usize) -> String {
    match count {
        1 => "1 restriction".to_string(),
        count => format!("{count} restrictions"),
    }
}

/// RLV as the bot runs it: its mode, what a forced movement does to the
/// held placement, which of the bot's own steps restrictions do not hold
/// back, the restrictions that are kept, and the messages that wait for
/// the avatar.
pub struct Rlv {
    mode: Mode,
    persist: RlvPersist,
    overridden: PersistOverride,
    store: Restrictions,
    /// Messages that arrived while the avatar could not act, in order.
    waiting: VecDeque<(Issuer, String)>,
}

impl Rlv {
    pub fn new(on: bool, persist: RlvPersist, overridden: PersistOverride) -> Rlv {
        if on {
            crate::info!("rlv: on");
        }
        if overridden != PersistOverride::default() {
            crate::info!("rlv: persist override: {}", overridden.words());
        }
        Rlv {
            mode: if on { Mode::On } else { Mode::Off },
            persist,
            overridden,
            store: Restrictions::default(),
            waiting: VecDeque::new(),
        }
    }

    /// Whether RLV commands are read: in override as well as on.
    pub fn is_on(&self) -> bool {
        self.mode != Mode::Off
    }

    fn mode_word(&self) -> &'static str {
        match self.mode {
            Mode::Off => "off",
            Mode::On => "on",
            Mode::Override => "override",
        }
    }

    /// Switches the mode. Off forgets what waits, and keeps the
    /// restrictions; override is on with one command exempt, and so not
    /// to be had while RLV is off. Returns the line that says which mode
    /// it is now.
    pub fn set_mode(&mut self, mode: Mode) -> Result<String, String> {
        if mode == Mode::Override && self.mode == Mode::Off {
            return Err("RLV is off, and no restriction is in force; `rslbot ctl rlv mode on` switches it on".to_string());
        }
        let changed = self.mode != mode;
        self.mode = mode;
        let line = format!("rlv: {}", self.mode_word());
        if changed {
            crate::info!("{line}");
        }
        if mode == Mode::Off {
            self.forget_waiting();
        }
        Ok(line)
    }

    /// The operator gives `command`: whether it is the one override
    /// exempts. RLV is on again then, whatever becomes of the command.
    pub fn take_override(&mut self, command: &str) -> bool {
        if self.mode != Mode::Override {
            return false;
        }
        self.mode = Mode::On;
        crate::info!("rlv: on again; `{command}` was exempt");
        true
    }

    /// The restrictions that are in force: none while RLV is off.
    pub fn in_force(&self) -> Option<&Restrictions> {
        self.is_on().then_some(&self.store)
    }

    /// Whether a forced movement is followed by a persist.
    pub fn follows(&self) -> bool {
        self.persist == RlvPersist::Follow
    }

    fn persist_word(&self) -> &'static str {
        match self.persist {
            RlvPersist::Drop => "drop",
            RlvPersist::Follow => "follow",
        }
    }

    /// Sets the RLV persist setting. Returns the line that says what it is.
    pub fn set_persist(&mut self, persist: RlvPersist) -> String {
        if self.persist != persist {
            self.persist = persist;
            crate::info!("rlv: persist {}", self.persist_word());
        }
        format!("rlv persist: {}", self.persist_word())
    }

    /// The persist override setting.
    pub fn overridden(&self) -> PersistOverride {
        self.overridden
    }

    /// Sets the persist override setting. Returns the line that says what
    /// it is.
    pub fn set_overridden(&mut self, overridden: PersistOverride) -> String {
        if self.overridden != overridden {
            self.overridden = overridden;
            crate::info!("rlv: persist override: {}", overridden.words());
        }
        format!("rlv persist override: {}", overridden.words())
    }

    /// The answer to `rlv status`.
    pub fn status(&self) -> String {
        let mut lines = vec![
            format!("rlv: {}", self.mode_word()),
            format!("persist: {}", self.persist_word()),
            format!("persist override: {}", self.overridden.words()),
        ];
        lines.extend(self.store.lines());
        lines.join("\n")
    }

    /// `rlv reset`: drops every restriction. Returns the answer.
    pub fn reset(&mut self) -> String {
        let count = self.store.reset();
        crate::info!("rlv: reset: {} cleared", counted(count));
        format!("rlv reset: {count} restrictions cleared")
    }

    /// Sets or lifts a restriction of `issuer`, or clears what it holds,
    /// if that is what `command` asks for and the bot keeps restrictions
    /// of that behaviour.
    pub fn restrict(&mut self, issuer: &Issuer, command: &Command) -> Option<Step> {
        self.store.apply(issuer, command)
    }

    /// The simulator says the object `id` is gone: so is what it held.
    pub fn object_gone(&mut self, id: Uuid) {
        if let Some((holder, count)) = self.store.drop_holder(id) {
            crate::info!("rlv: dropped {} of {holder}: the object is gone", counted(count));
        }
    }

    /// The avatar has left a region with the objects `worn` on it: what
    /// every other object held stays behind.
    pub fn region_left(&mut self, worn: &[Uuid]) {
        for (holder, count) in self.store.keep_only(worn) {
            crate::info!("rlv: dropped {} of {holder}: the avatar left its region", counted(count));
        }
    }

    /// The session is over: the objects of the next one are other objects.
    pub fn session_ended(&mut self) {
        let (objects, count) = self.store.drop_objects();
        if count > 0 {
            let objects = if objects == 1 {
                "1 object".to_string()
            } else {
                format!("{objects} objects")
            };
            crate::info!("rlv: dropped {} of {objects}: the session ended", counted(count));
        }
    }

    /// Keeps a message until the avatar can act. A full queue drops it.
    pub fn keep(&mut self, issuer: Issuer, text: String) {
        if self.waiting.len() >= MAX_WAITING {
            crate::info!("rlv: {issuer}: dropped, {MAX_WAITING} messages are waiting already: {text}");
            return;
        }
        self.waiting.push_back((issuer, text));
        let count = self.waiting.len();
        let messages = if count == 1 { "message" } else { "messages" };
        crate::info!("rlv: keeping {count} {messages} until the avatar is in a region");
    }

    pub fn has_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// The message that has waited longest.
    pub fn next_waiting(&mut self) -> Option<(Issuer, String)> {
        self.waiting.pop_front()
    }

    pub fn forget_waiting(&mut self) {
        if !self.waiting.is_empty() {
            crate::info!("rlv: dropped {} waiting messages", self.waiting.len());
            self.waiting.clear();
        }
    }
}

// --------------------------------------------------------------- following

/// What the bot does about its placement after a step of a forced movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum After {
    Nothing,
    /// Carry out `persist cur <sit state>`.
    Persist(SitState),
    /// The movement came to nothing: back to the placement mode from
    /// before it.
    Restore {
        manual: bool,
    },
}

/// Follows forced movements that end later than they are accepted: a
/// teleport is persisted when the avatar has arrived. (A followed sit is a
/// persist from the start, and is the bot's own business.)
#[derive(Debug, Default)]
pub struct Follow {
    /// A followed teleport is under way; whether the bot was in manual
    /// mode before it.
    teleport: Option<bool>,
}

impl Follow {
    /// The commands accepted a forced movement while the bot was in the
    /// placement mode `was_manual`. An earlier teleport under way is given
    /// up; its mode is the one to go back to.
    pub fn moved(&mut self, movement: Movement, follow: bool, was_manual: bool) -> After {
        let before = self.teleport.take().unwrap_or(was_manual);
        match (follow, movement) {
            (false, _) | (true, Movement::Sit) => After::Nothing,
            (true, Movement::Stand) => After::Persist(SitState::Stand),
            (true, Movement::Ground) => After::Persist(SitState::Ground),
            (true, Movement::Teleport) => {
                self.teleport = Some(before);
                After::Nothing
            }
        }
    }

    /// The avatar has arrived somewhere.
    pub fn arrived(&mut self) -> After {
        match self.teleport.take() {
            Some(_) => After::Persist(SitState::Stand),
            None => After::Nothing,
        }
    }

    /// A teleport failed, or the session was lost.
    pub fn failed(&mut self) -> After {
        match self.teleport.take() {
            Some(manual) => After::Restore { manual },
            None => After::Nothing,
        }
    }

    /// The operator has taken over, or something else was persisted.
    pub fn give_up(&mut self) {
        self.teleport = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tests::{Call, FakeGrid};

    fn command(text: &str) -> Command {
        match &parse(text)[..] {
            [Ok(command)] => command.clone(),
            other => panic!("expected one command in {text}, got {other:?}"),
        }
    }

    fn parts(text: &str) -> (String, Option<String>, Param) {
        let command = command(text);
        (command.behaviour, command.option, command.param)
    }

    fn reason(text: &str) -> &'static str {
        match &parse(text)[..] {
            [Err(malformed)] => malformed.reason,
            other => panic!("expected one malformed piece in {text}, got {other:?}"),
        }
    }

    #[test]
    fn a_message_is_split_into_its_commands() {
        let texts = |message: &str| -> Vec<String> {
            parse(message)
                .into_iter()
                .map(|piece| match piece {
                    Ok(command) => command.text,
                    Err(malformed) => format!("!{}", malformed.text),
                })
                .collect()
        };
        assert_eq!(texts("@versionnum=5,unsit=force"), ["versionnum=5", "unsit=force"]);
        // The at-sign is the message's, and `rlv cmd` may leave it out.
        assert_eq!(texts("unsit=force"), ["unsit=force"]);
        // Empty pieces are nothing, a trailing comma included.
        assert_eq!(texts("@unsit=force,"), ["unsit=force"]);
        assert_eq!(texts("@,,a=n,,"), ["a=n"]);
        assert_eq!(texts("@"), Vec::<String>::new());
        // A bad piece does not spoil the others.
        assert_eq!(texts("@nonsense,versionnum=5"), ["!nonsense", "versionnum=5"]);
    }

    #[test]
    fn a_command_is_behaviour_option_and_param() {
        assert_eq!(parts("@unsit=force"), ("unsit".to_string(), None, Param::Force));
        assert_eq!(
            parts("@versionnum=2222"),
            ("versionnum".to_string(), None, Param::Channel(Some(2222)))
        );
        assert_eq!(
            parts("@versionnum=-873246"),
            ("versionnum".to_string(), None, Param::Channel(Some(-873246)))
        );
        assert_eq!(parts("@version=0"), ("version".to_string(), None, Param::Channel(Some(0))));
        assert_eq!(parts("@version=99999999999"), ("version".to_string(), None, Param::Channel(None)));
        for (restriction, sets) in [("n", true), ("add", true), ("y", false), ("rem", false)] {
            assert_eq!(parts(&format!("@detach={restriction}")).2, Param::Restrict(sets), "{restriction}");
        }
        let (behaviour, option, param) = parts("@sit:11111111-2222-3333-4444-555555555555=force");
        assert_eq!(behaviour, "sit");
        assert_eq!(option.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
        assert_eq!(param, Param::Force);
        // An empty option is still an option.
        assert_eq!(parts("@detach:=force").1.as_deref(), Some(""));
        // `@clear` is the one command that may come without a param.
        assert_eq!(parts("@clear"), ("clear".to_string(), None, Param::Absent));
        assert_eq!(parts("@CLEAR"), ("clear".to_string(), None, Param::Absent));
        // And what follows its `=` is a filter, not one of the params.
        assert_eq!(parts("@clear=tp").2, Param::Absent);
        assert_eq!(parts("@clear=tplure:").2, Param::Absent);
    }

    #[test]
    fn behaviour_and_param_ignore_case_and_the_option_keeps_it() {
        assert_eq!(parts("@UnSit=FORCE"), ("unsit".to_string(), None, Param::Force));
        assert_eq!(parts("@Detach=N").2, Param::Restrict(true));
        let (behaviour, option, _) = parts("@TpTo:Blue Lagoon/1/2/3=Force");
        assert_eq!((behaviour.as_str(), option.as_deref()), ("tpto", Some("Blue Lagoon/1/2/3")));
    }

    #[test]
    fn the_first_equals_sign_and_the_first_colon_split() {
        let (behaviour, option, param) = parts("@tpto:A:B/1/2/3=force");
        assert_eq!(
            (behaviour.as_str(), option.as_deref(), param),
            ("tpto", Some("A:B/1/2/3"), Param::Force)
        );
        // What follows the first `=` is the param, whatever it holds.
        assert_eq!(
            reason("@notify:5;x=y=add"),
            "its param is not `force`, a channel number, `n`, `add`, `y` or `rem`"
        );
    }

    #[test]
    fn malformed_pieces_say_why() {
        assert_eq!(reason("@nonsense"), "it has no `=`");
        assert_eq!(reason("@clearance"), "it has no `=`");
        assert_eq!(reason("@=force"), "it names no behaviour");
        assert_eq!(reason("@:chest=force"), "it names no behaviour");
        assert!(reason("@unsit=maybe").starts_with("its param"));
        assert!(reason("@unsit=").starts_with("its param"));
        assert!(reason("@version=5.5").starts_with("its param"));
    }

    #[test]
    fn spaces_are_part_of_what_they_are_in() {
        // `@unsit = force`: the behaviour is `unsit ` and the param ` force`.
        assert!(reason("@unsit = force").starts_with("its param"));
        assert_eq!(parts("@unsit =force").0, "unsit ");
        // A second at-sign makes another behaviour of it.
        assert_eq!(command("@versionnum=5,@unsit=force".split(',').nth(1).unwrap()).behaviour, "unsit");
        match &parse("@versionnum=5,@unsit=force")[..] {
            [Ok(first), Ok(second)] => assert_eq!((first.behaviour.as_str(), second.behaviour.as_str()), ("versionnum", "@unsit")),
            other => panic!("{other:?}"),
        }
    }

    // ------------------------------------------------------------ dealing

    fn collar() -> Issuer {
        Issuer {
            id: Uuid::from_bytes([7; 16]),
            name: "Collar".to_string(),
            operator: false,
        }
    }

    fn chair() -> Uuid {
        Uuid::from_bytes([9; 16])
    }

    struct Rig {
        commands: Commands,
        grid: FakeGrid,
        follow: bool,
        now: Instant,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                commands: Commands::default(),
                grid: FakeGrid::default(),
                follow: false,
                now: Instant::now(),
            }
        }

        /// Deals with a command as the bot does: a restriction is the
        /// store's, everything else goes to `deal`.
        fn deal_from(&mut self, issuer: &Issuer, text: &str) -> Step {
            let command = command(text);
            if let Some(step) = self.grid.kept.apply(issuer, &command) {
                return step;
            }
            deal(&command, issuer, self.follow, &mut self.commands, Some(&mut self.grid), self.now)
        }

        fn deal(&mut self, text: &str) -> Step {
            self.deal_from(&collar(), text)
        }

        fn calls(&mut self) -> Vec<Call> {
            std::mem::take(&mut self.grid.calls)
        }
    }

    fn failed(step: Step) -> String {
        match step {
            Step::Failed(reason) => reason,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn version_queries_are_answered_on_their_channel() {
        let mut rig = Rig::new();
        let bot = env!("CARGO_PKG_VERSION");
        let old = format!("RestrainedLife viewer v0.1.0 (RusselBot {bot})");
        let new = format!("RestrainedLove viewer v0.1.0 (RusselBot {bot})");
        assert_eq!(rig.deal("@version=2222"), Step::Said(old.clone()));
        assert_eq!(rig.deal("@versionnew=2222"), Step::Said(new.clone()));
        assert_eq!(rig.deal("@versionnum=2222"), Step::Said("10000".to_string()));
        assert_eq!(rig.deal("@versionnum=-873246"), Step::Said("10000".to_string()));
        // An option changes nothing.
        assert_eq!(rig.deal("@versionnum:whatever=7"), Step::Said("10000".to_string()));
        assert_eq!(
            rig.calls(),
            [
                Call::Say(2222, 1, old),
                Call::Say(2222, 1, new),
                Call::Say(2222, 1, "10000".to_string()),
                Call::Say(-873246, 1, "10000".to_string()),
                Call::Say(7, 1, "10000".to_string()),
            ]
        );
        // Answering moves nothing.
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn the_version_number_follows_the_version() {
        let parts: Vec<u32> = VERSION.split('.').map(|part| part.parse().unwrap()).collect();
        assert_eq!(VERSION_NUM, parts[0] * 1_000_000 + parts[1] * 10_000 + parts[2] * 100);
    }

    #[test]
    fn a_query_that_cannot_be_answered_is_refused() {
        let mut rig = Rig::new();
        assert_eq!(failed(rig.deal("@versionnew=0")), "channel 0 cannot be answered on");
        assert_eq!(failed(rig.deal("@versionnew=99999999999")), "the channel is out of range");
        assert_eq!(rig.calls(), []);
        let mut commands = Commands::default();
        let step = deal(
            &command("@versionnum=5"),
            &collar(),
            false,
            &mut commands,
            None::<&mut FakeGrid>,
            rig.now,
        );
        assert_eq!(failed(step), NOT_CONNECTED);
    }

    #[test]
    fn what_the_bot_does_not_have_is_not_supported() {
        let mut rig = Rig::new();
        for text in [
            "@frobnicate=n",
            "@frobnicate=rem",
            "@clear=force",
            "@notify:5;detach=add",
            "@notify:5=rem",
            "@getstatus=force",
            "@getsitid=2222",
            "@versionnumbl=2222",
            "@getblacklist=2222",
            "@version=force",
            "@versionnew=n",
            "@attach:Outfits/Beach=force",
            "@setrot:1.5=force",
            "@unsit=5",
            "@frobnicate=force",
            // Everything, and what names a folder.
            "@detach=force",
            "@remattach=force",
            "@remoutfit=force",
            "@detach:Outfits/Beach=force",
            "@detach:5=force",
        ] {
            assert_eq!(failed(rig.deal(text)), "not supported", "{text}");
        }
        assert_eq!(rig.calls(), []);
        assert!(rig.grid.jobs.is_empty());
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn forced_sits_and_stands_go_through_the_commands() {
        let mut rig = Rig::new();
        let sit = format!("@sit:{}=force", chair());
        assert_eq!(rig.deal(&sit), Step::Moved(Movement::Sit));
        assert_eq!(rig.calls(), [Call::Sit(chair())]);
        assert!(rig.commands.take_manual());
        assert_eq!(rig.deal("@sitground=force"), Step::Moved(Movement::Ground));
        assert_eq!(rig.deal("@unsit=force"), Step::Moved(Movement::Stand));
        // An option on these two changes nothing.
        assert_eq!(rig.deal("@unsit:now=force"), Step::Moved(Movement::Stand));
        assert_eq!(rig.calls(), [Call::SitGround, Call::Stand, Call::Stand]);
        assert!(rig.commands.take_manual());
        // None of it is the operator's last command.
        assert_eq!(rig.commands.last_sit(), None);
    }

    #[test]
    fn a_forced_sit_needs_a_uuid_and_a_region() {
        let mut rig = Rig::new();
        assert_eq!(failed(rig.deal("@sit:chair=force")), "\"chair\" is not an object's UUID");
        assert_eq!(failed(rig.deal("@sit=force")), "\"\" is not an object's UUID");
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal(&format!("@sit:{}=force", chair()))), NOT_CONNECTED);
        assert_eq!(failed(rig.deal("@unsit=force")), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn a_followed_sit_is_left_to_the_bot() {
        let mut rig = Rig::new();
        rig.follow = true;
        assert_eq!(rig.deal(&format!("@sit:{}=force", chair())), Step::HoldSeat(chair()));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(failed(rig.deal("@sit:chair=force")), "\"chair\" is not an object's UUID");
        // The others are carried out all the same.
        assert_eq!(rig.deal("@unsit=force"), Step::Moved(Movement::Stand));
        assert_eq!(rig.calls(), [Call::Stand]);
    }

    #[test]
    fn a_forced_sit_takes_the_place_of_a_waiting_one() {
        let mut rig = Rig::new();
        let (reply, answer) = mpsc::channel();
        let other = Uuid::from_bytes([8; 16]);
        rig.commands
            .handle(Request::Sit(other.to_string()), reply, Some(&mut rig.grid), rig.now);
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(other)));
        assert_eq!(rig.deal(&format!("@sit:{}=force", chair())), Step::Moved(Movement::Sit));
        assert_eq!(
            answer.try_recv(),
            Ok(Response::Error("superseded by a newer sit command".to_string()))
        );
        // The operator's sit stays what `last_cmd` stands for.
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(other)));
        // The simulator's verdict on the forced one goes to nobody.
        assert!(rig.commands.sit_failed("no such object"));
    }

    #[test]
    fn a_forced_teleport_names_a_region_or_a_point_of_the_grid() {
        let mut rig = Rig::new();
        assert_eq!(rig.deal("@tpto:Blue Lagoon/128/100/25=force"), Step::Moved(Movement::Teleport));
        assert_eq!(rig.deal("@tpto:Home/128/100.5/25;1.57=force"), Step::Moved(Movement::Teleport));
        // A region may be called by a number.
        assert_eq!(rig.deal("@tpto:42/1/2/3=force"), Step::Moved(Movement::Teleport));
        assert_eq!(
            rig.calls(),
            [
                Call::Teleport("Blue Lagoon".to_string(), [128.0, 100.0, 25.0]),
                Call::Teleport("Home".to_string(), [128.0, 100.5, 25.0]),
                Call::Teleport("42".to_string(), [1.0, 2.0, 3.0]),
            ]
        );
        assert!(rig.commands.take_manual());
        // Region 1000, 1000 has its corner at 256000, 256000.
        assert_eq!(rig.deal("@tpto:256128/256100/25=force"), Step::Moved(Movement::Teleport));
        assert_eq!(rig.deal("@tpto:256511.5/512000/30;0=force"), Step::Moved(Movement::Teleport));
        // One of the two below 256 is still a region of some grid.
        assert_eq!(rig.deal("@tpto:300/10/20=force"), Step::Moved(Movement::Teleport));
        let handle = |x: u64, y: u64| (x << 32) | y;
        assert_eq!(
            rig.calls(),
            [
                Call::TeleportTo(handle(256000, 256000), [128.0, 100.0, 25.0]),
                Call::TeleportTo(handle(256256, 512000), [255.5, 0.0, 30.0]),
                Call::TeleportTo(handle(256, 0), [44.0, 10.0, 20.0]),
            ]
        );
        assert!(rig.commands.take_manual());
        // Where RLV went is not where the operator's last `tp` went.
        assert_eq!(rig.commands.last_tp(), None);
        // The outcome is nobody's to read, but it is the commands' teleport.
        assert!(rig.commands.teleport_failed("no such region"));
    }

    #[test]
    fn a_malformed_destination_is_refused() {
        let mut rig = Rig::new();
        for (text, part) in [
            ("@tpto:north/128=force", "neither"),
            ("@tpto:128=force", "neither"),
            ("@tpto=force", "neither"),
            ("@tpto:Home/1/2/3/4=force", "neither"),
            ("@tpto:Home/128/-5/25=force", "`y`"),
            ("@tpto:Home/128/5/up=force", "`z`"),
            ("@tpto:Home/inf/5/25=force", "`x`"),
            ("@tpto:/128/5/25=force", "no region"),
            ("@tpto:-1/2/3=force", "`X`"),
            ("@tpto:1/nan/3=force", "`Y`"),
            ("@tpto:1e12/2/3=force", "outside any grid"),
            // Coordinates inside a region, given without its name.
            ("@tpto:219/102/611=force", "`region/x/y/z`"),
            ("@tpto:0/0/0=force", "region at 0, 0"),
            ("@tpto:255.9/255.9/4000;1.57=force", "region at 0, 0"),
        ] {
            let reason = failed(rig.deal(text));
            assert!(reason.contains(part), "{text}: {reason}");
        }
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal("@tpto:256/2/3=force")), NOT_CONNECTED);
        assert_eq!(failed(rig.deal("@tpto:Home/1/2/3=force")), NOT_CONNECTED);
    }

    #[test]
    fn rlv_names_for_attachment_points_are_read() {
        for (name, number) in RLV_POINTS {
            assert_eq!(point(name), Some(number), "{name}");
            assert_eq!(point(&name.to_uppercase()), Some(number), "{name}");
        }
        // They are the bot's HUD points and its `Avatar Center`.
        for (name, number) in RLV_POINTS {
            let own = inventory::point_name(number);
            let expected = if name == "root" {
                "Avatar Center".to_string()
            } else {
                format!("HUD {name}")
            };
            assert!(own.eq_ignore_ascii_case(&expected), "{name}: {own}");
        }
        assert_eq!(point("left hand"), Some(5));
        assert_eq!(point("Chest"), Some(1));
        assert_eq!(point("HUD Center"), Some(35));
        assert_eq!(point("avatar center"), Some(40));
        assert_eq!(point("5"), None);
        assert_eq!(point("Outfits/Beach"), None);
        assert_eq!(point(""), None);
    }

    #[test]
    fn a_forced_detach_takes_everything_off_a_point() {
        let mut rig = Rig::new();
        let (watch, meter) = (Uuid::from_bytes([3; 16]), Uuid::from_bytes([4; 16]));
        rig.grid.attached = vec![(watch, 5), (meter, 35)];
        assert_eq!(rig.deal("@detach:left hand=force"), Step::Done);
        assert_eq!(rig.deal("@remattach:center=force"), Step::Done);
        // The inventory is asked for the items' names first, as for `detach`.
        let asked: Vec<&What> = rig.grid.jobs.iter().map(|(_, _, what)| what).collect();
        assert_eq!(asked, [&What::Items(vec![watch]), &What::Items(vec![meter])]);
        // Nothing on the point: the reason `detach` gives.
        assert_eq!(failed(rig.deal("@detach:chest=force")), "nothing is attached to Chest");
        assert_eq!(rig.grid.jobs.len(), 2);
        // Taking things off moves nothing.
        assert!(!rig.commands.take_manual());
    }

    use crate::inventory::{Clothes, What};

    #[test]
    fn a_forced_detach_takes_an_attachment_off_by_its_uuid() {
        let mut rig = Rig::new();
        let (object, item) = (Uuid::from_bytes([3; 16]), Uuid::from_bytes([4; 16]));
        rig.grid.worn = vec![(object, item)];
        let found = |rig: &Rig| match rig.grid.jobs.last() {
            Some((_, _, What::Find(inventory::Ref::Id(id)))) => *id,
            other => panic!("expected a lookup of an item, got {other:?}"),
        };
        // By the attachment's own UUID, which is what a script has: the item
        // it was made from is detached, as `detach <item>` does it.
        assert_eq!(rig.deal(&format!("@remattach:{object}=force")), Step::Done);
        assert_eq!(found(&rig), item);
        assert_eq!(
            rig.deal(&format!("@detach:{}=force", object.to_string().to_uppercase())),
            Step::Done
        );
        assert_eq!(found(&rig), item);
        // A UUID that is no attachment's object is taken for the item.
        assert_eq!(rig.deal(&format!("@remattach:{item}=force")), Step::Done);
        assert_eq!(found(&rig), item);
        let other = Uuid::from_bytes([5; 16]);
        assert_eq!(rig.deal(&format!("@remattach:{other}=force")), Step::Done);
        assert_eq!(found(&rig), other);
        assert_eq!(rig.grid.jobs.len(), 4);
        // The operator has UUIDs of items only, and may give one.
        let operator = Issuer::operator(collar().id);
        assert_eq!(rig.deal_from(&operator, &format!("@remattach:{item}=force")), Step::Done);
        assert!(!rig.commands.take_manual());
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal(&format!("@remattach:{object}=force"))), NOT_CONNECTED);
    }

    #[test]
    fn a_forced_remoutfit_takes_a_kind_of_clothing_off() {
        let mut rig = Rig::new();
        // Without the grid's outfit service clothing cannot be changed.
        assert_eq!(failed(rig.deal("@remoutfit:shirt=force")), inventory::NO_OUTFIT_SERVICE);
        rig.grid.outfit = true;
        assert_eq!(rig.deal("@remoutfit:shirt=force"), Step::Done);
        let shirt = inventory::parse_kind("shirt").unwrap();
        assert!(matches!(&rig.grid.jobs[..], [(_, _, What::TakeOff(Clothes::Kind(kind)))] if *kind == shirt));
        // A body part stays, for the reason `unwear` gives.
        let reason = failed(rig.deal("@remoutfit:skin=force"));
        assert!(reason.contains("skin"), "{reason}");
        assert_eq!(failed(rig.deal("@remoutfit:hat=force")), "\"hat\" is not a kind of clothing");
        assert_eq!(rig.grid.jobs.len(), 1);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn an_attachment_detaches_itself() {
        let mut rig = Rig::new();
        let item = Uuid::from_bytes([5; 16]);
        rig.grid.worn = vec![(collar().id, item)];
        assert_eq!(rig.deal("@detachme=force"), Step::Done);
        assert!(matches!(&rig.grid.jobs[..], [(_, _, What::Find(found))] if *found == inventory::Ref::Id(item)));
        // An object that is not worn, and the operator, have nothing to detach.
        let rezzed = Issuer {
            id: Uuid::from_bytes([6; 16]),
            ..collar()
        };
        assert_eq!(failed(rig.deal_from(&rezzed, "@detachme=force")), "the sender is not attached");
        let operator = Issuer::operator(collar().id);
        assert_eq!(failed(rig.deal_from(&operator, "@detachme=force")), "the sender is not attached");
        assert_eq!(rig.grid.jobs.len(), 1);
    }

    // -------------------------------------------------------------- senders

    fn heard(kind: &str, source: Source, text: &str) -> Message {
        Message {
            kind: kind.to_string(),
            source,
            sent_at: None,
            text: text.to_string(),
        }
    }

    fn object() -> Source {
        Source::Object {
            id: Some(collar().id),
            name: "Collar".to_string(),
            owner: Uuid::from_bytes([1; 16]),
        }
    }

    #[test]
    fn only_what_an_object_says_to_its_owner_carries_commands() {
        assert_eq!(sender(&heard("owner", object(), "@unsit=force")), Some(collar()));
        // Not what it says for all to hear, and not an ordinary message.
        assert_eq!(sender(&heard("say", object(), "@unsit=force")), None);
        assert_eq!(sender(&heard("owner", object(), "Battery low")), None);
        assert_eq!(sender(&heard("owner", object(), " @unsit=force")), None);
        // Not an avatar or the grid, whatever kind the message claims.
        let avatar = Source::Avatar {
            id: Uuid::from_bytes([2; 16]),
            name: "Jane Doe".to_string(),
        };
        assert_eq!(sender(&heard("say", avatar.clone(), "@unsit=force")), None);
        assert_eq!(sender(&heard("owner", avatar.clone(), "@unsit=force")), None);
        assert_eq!(sender(&heard("im", avatar, "@version")), None);
        let grid = Source::System { name: "Grid".to_string() };
        assert_eq!(sender(&heard("owner", grid, "@unsit=force")), None);
        assert_eq!(collar().to_string(), format!("Collar <{}>", collar().id));
        assert_eq!(Issuer::operator(collar().id).to_string(), "operator");
    }

    // ---------------------------------------------------------------- state

    #[test]
    fn the_mode_and_the_persist_setting_are_switched_and_shown() {
        let none = PersistOverride::default();
        let rest = "persist override: none\nrestrictions: none";
        let mut rlv = Rlv::new(false, RlvPersist::Drop, none);
        assert!(!rlv.is_on() && !rlv.follows());
        assert_eq!(rlv.status(), format!("rlv: off\npersist: drop\n{rest}"));
        assert_eq!(rlv.set_mode(Mode::On), Ok("rlv: on".to_string()));
        assert_eq!(rlv.set_mode(Mode::On), Ok("rlv: on".to_string()));
        assert_eq!(rlv.set_persist(RlvPersist::Follow), "rlv persist: follow");
        assert_eq!(rlv.set_persist(RlvPersist::Follow), "rlv persist: follow");
        assert!(rlv.is_on() && rlv.follows());
        assert_eq!(rlv.status(), format!("rlv: on\npersist: follow\n{rest}"));
        assert_eq!(rlv.set_mode(Mode::Off), Ok("rlv: off".to_string()));
        assert_eq!(rlv.set_persist(RlvPersist::Drop), "rlv persist: drop");
        assert_eq!(rlv.status(), format!("rlv: off\npersist: drop\n{rest}"));
        let rlv = Rlv::new(true, RlvPersist::Follow, none);
        assert_eq!(rlv.status(), format!("rlv: on\npersist: follow\n{rest}"));
    }

    #[test]
    fn waiting_messages_keep_their_order_up_to_the_limit() {
        let mut rlv = Rlv::new(true, RlvPersist::Drop, PersistOverride::default());
        assert!(!rlv.has_waiting());
        for number in 0..MAX_WAITING + 1 {
            rlv.keep(collar(), format!("@versionnum={}", number + 1));
        }
        // The 65th was dropped.
        let mut texts = Vec::new();
        while let Some((issuer, text)) = rlv.next_waiting() {
            assert_eq!(issuer, collar());
            texts.push(text);
        }
        assert_eq!(texts.len(), MAX_WAITING);
        assert_eq!(texts[0], "@versionnum=1");
        assert_eq!(texts[MAX_WAITING - 1], format!("@versionnum={MAX_WAITING}"));
        // Switching off forgets what waits; so does being told to.
        rlv.keep(collar(), "@unsit=force".to_string());
        rlv.set_mode(Mode::Off).unwrap();
        assert!(!rlv.has_waiting());
        rlv.keep(collar(), "@unsit=force".to_string());
        rlv.forget_waiting();
        assert!(!rlv.has_waiting());
    }

    // ------------------------------------------------------------ following

    #[test]
    fn dropped_movements_are_followed_by_nothing() {
        let mut follow = Follow::default();
        for movement in [Movement::Sit, Movement::Ground, Movement::Stand, Movement::Teleport] {
            assert_eq!(follow.moved(movement, false, false), After::Nothing);
        }
        assert_eq!(follow.arrived(), After::Nothing);
        assert_eq!(follow.failed(), After::Nothing);
    }

    #[test]
    fn a_followed_stand_and_ground_sit_are_persisted_at_once() {
        let mut follow = Follow::default();
        assert_eq!(follow.moved(Movement::Stand, true, false), After::Persist(SitState::Stand));
        assert_eq!(follow.moved(Movement::Ground, true, true), After::Persist(SitState::Ground));
        // Nothing is left under way.
        assert_eq!(follow.arrived(), After::Nothing);
        assert_eq!(follow.failed(), After::Nothing);
    }

    #[test]
    fn a_followed_teleport_is_persisted_on_arrival() {
        let mut follow = Follow::default();
        assert_eq!(follow.moved(Movement::Teleport, true, false), After::Nothing);
        assert_eq!(follow.arrived(), After::Persist(SitState::Stand));
        // Once: a later arrival is not this teleport's.
        assert_eq!(follow.arrived(), After::Nothing);
    }

    #[test]
    fn a_followed_teleport_that_fails_gives_the_earlier_mode_back() {
        for was_manual in [false, true] {
            let mut follow = Follow::default();
            follow.moved(Movement::Teleport, true, was_manual);
            assert_eq!(follow.failed(), After::Restore { manual: was_manual });
            assert_eq!(follow.failed(), After::Nothing);
        }
    }

    #[test]
    fn a_followed_teleport_is_given_up_for_what_comes_next() {
        // The operator steps in.
        let mut follow = Follow::default();
        follow.moved(Movement::Teleport, true, false);
        follow.give_up();
        assert_eq!(follow.arrived(), After::Nothing);
        // A forced stand during it is persisted by itself.
        follow.moved(Movement::Teleport, true, false);
        assert_eq!(follow.moved(Movement::Stand, true, true), After::Persist(SitState::Stand));
        assert_eq!(follow.arrived(), After::Nothing);
        // A second teleport keeps the mode from before the first: the bot
        // was holding then, and is in manual mode only because of the first.
        follow.moved(Movement::Teleport, true, false);
        follow.moved(Movement::Teleport, true, true);
        assert_eq!(follow.failed(), After::Restore { manual: false });
        // A movement that is dropped ends a teleport that was followed.
        follow.moved(Movement::Teleport, true, false);
        assert_eq!(follow.moved(Movement::Teleport, false, true), After::Nothing);
        assert_eq!(follow.arrived(), After::Nothing);
    }

    // --------------------------------------------------------- restrictions

    fn holder(number: u8) -> Who {
        Who::Object(Uuid::from_bytes([number; 16]))
    }

    /// The collar of these tests, and a second attachment.
    const COLLAR: u8 = 7;
    const CUFF: u8 = 8;

    fn id(number: u8) -> Uuid {
        Uuid::from_bytes([number; 16])
    }

    fn split(text: &str) -> (&str, Option<&str>) {
        match text.split_once(':') {
            Some((behaviour, option)) => (behaviour, Some(option)),
            None => (text, None),
        }
    }

    fn hold(store: &mut Restrictions, who: Who, text: &str) -> Set {
        let name = match who {
            Who::Operator => "",
            Who::Object(id) if id == Uuid::from_bytes([COLLAR; 16]) => "Collar",
            Who::Object(id) if id == Uuid::from_bytes([CUFF; 16]) => "Cuff",
            Who::Object(_) => "Thing",
        };
        let (behaviour, option) = split(text);
        store.set(who, name, behaviour, option)
    }

    fn holding(rules: &[(Who, &str)]) -> Restrictions {
        let mut store = Restrictions::default();
        for (who, text) in rules {
            assert_eq!(hold(&mut store, *who, text), Set::New, "{text}");
        }
        store
    }

    /// What one collar holds.
    fn collared(rules: &[&str]) -> Restrictions {
        let rules: Vec<(Who, &str)> = rules.iter().map(|text| (holder(COLLAR), *text)).collect();
        holding(&rules)
    }

    fn limits(store: &Restrictions) -> Limits<'_> {
        Limits::new(Some(store), None)
    }

    fn rule(refused: Result<(), Refusal>) -> String {
        match refused {
            Err(refusal) => refusal.rule,
            Ok(()) => panic!("expected a refusal"),
        }
    }

    #[test]
    fn the_known_behaviours_are_two_lists_without_overlap() {
        let mut all: Vec<&str> = ENFORCED.iter().chain(KEPT).copied().collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count, "a behaviour is listed twice");
        assert_eq!((ENFORCED.len(), KEPT.len()), (46, 71));
        for behaviour in ["sendchat", "unsit", "detach", "tplure_sec", "permissive", "touchfar"] {
            assert_eq!(effect(behaviour), Some(Effect::Enforced), "{behaviour}");
        }
        for behaviour in ["showloc", "recvim", "standtp", "fly", "allowidle", "tplm"] {
            assert_eq!(effect(behaviour), Some(Effect::Kept), "{behaviour}");
        }
        for behaviour in ["notify", "frobnicate", "clear", "getstatus", "Detach", ""] {
            assert_eq!(effect(behaviour), None, "{behaviour}");
        }
    }

    #[test]
    fn a_restriction_is_set_and_lifted_for_its_holder() {
        let mut store = Restrictions::default();
        let collar = holder(COLLAR);
        assert_eq!(hold(&mut store, collar, "sendim"), Set::New);
        assert_eq!(
            store.lines(),
            ["restrictions: 1".to_string(), format!("Collar <{}>: sendim", id(COLLAR))]
        );
        assert!(store.lift(collar, "sendim", None));
        assert_eq!(store.count(), 0);
        // An exception is kept the same way.
        let jane = id(1).to_string();
        assert_eq!(store.set(collar, "Collar", "sendim", Some(&jane)), Set::New);
        assert!(store.lift(collar, "sendim", Some(&jane)));
        assert_eq!(store.count(), 0);
        // Lifting what is not held changes nothing.
        assert!(!store.lift(collar, "sendim", None));
    }

    #[test]
    fn two_holders_of_one_restriction_both_have_to_lift_it() {
        let mut store = holding(&[(holder(COLLAR), "sendchat"), (holder(CUFF), "sendchat")]);
        assert!(store.lift(holder(COLLAR), "sendchat", None));
        assert_eq!(rule(limits(&store).chat(0, SAY, "hello").map(|_| ())), "sendchat");
        assert!(store.lift(holder(CUFF), "sendchat", None));
        assert!(limits(&store).chat(0, SAY, "hello").is_ok());
        // The operator is a holder apart from every object.
        let mut store = holding(&[(Who::Operator, "tploc"), (holder(COLLAR), "tploc")]);
        assert_eq!(store.clear(holder(COLLAR), None), 1);
        assert_eq!(store.lines()[1], "operator: tploc");
    }

    #[test]
    fn setting_twice_keeps_one() {
        let mut store = collared(&["unsit"]);
        assert_eq!(hold(&mut store, holder(COLLAR), "unsit"), Set::Already);
        assert_eq!(store.count(), 1);
        assert!(store.lift(holder(COLLAR), "unsit", None));
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn the_option_keeps_its_case_and_is_compared_without_it() {
        let mut store = Restrictions::default();
        assert_eq!(store.set(holder(COLLAR), "Collar", "detach", Some("Chest")), Set::New);
        assert_eq!(store.lines()[1], format!("Collar <{}>: detach:Chest", id(COLLAR)));
        assert_eq!(store.set(holder(COLLAR), "Collar", "detach", Some("CHEST")), Set::Already);
        // With and without an option are two restrictions.
        assert_eq!(store.set(holder(COLLAR), "Collar", "detach", None), Set::New);
        assert!(store.lift(holder(COLLAR), "detach", Some("chest")));
        assert_eq!(store.lines()[1], format!("Collar <{}>: detach", id(COLLAR)));
    }

    #[test]
    fn clear_lifts_what_its_text_is_part_of() {
        let jane = format!("tplure:{}", id(1));
        let all = ["tploc", "tplure", &jane, "sittp"];
        let mut store = collared(&all);
        assert_eq!(store.clear(holder(COLLAR), Some("tpl")), 3);
        assert_eq!(store.status(None, "", ""), "/sittp");
        let mut store = collared(&all);
        assert_eq!(store.clear(holder(COLLAR), Some("TPLURE:")), 1);
        assert_eq!(store.status(None, "", ""), "/tploc/tplure/sittp");
        // Without a text, and with an empty one, everything goes.
        assert_eq!(store.clear(holder(COLLAR), Some("")), 3);
        let mut store = collared(&all);
        assert_eq!(store.clear(holder(COLLAR), None), 4);
        assert_eq!(store.clear(holder(COLLAR), None), 0);
        // Never another holder's.
        let mut store = holding(&[(holder(COLLAR), "unsit"), (holder(CUFF), "unsit")]);
        assert_eq!(store.clear(holder(CUFF), None), 1);
        assert_eq!(store.status(Some(holder(COLLAR)), "", ""), "/unsit");
    }

    #[test]
    fn holders_are_dropped_with_their_objects() {
        let rules = [
            (holder(COLLAR), "detach"),
            (holder(COLLAR), "sendim"),
            (holder(CUFF), "unsit"),
            (Who::Operator, "tploc"),
        ];
        let mut store = holding(&rules);
        assert_eq!(store.drop_holder(id(COLLAR)), Some((format!("Collar <{}>", id(COLLAR)), 2)));
        assert_eq!(store.drop_holder(id(COLLAR)), None);
        assert_eq!(store.count(), 2);
        // Leaving a region keeps what is worn, and the operator's.
        let mut store = holding(&rules);
        assert_eq!(store.keep_only(&[id(CUFF)]), [(format!("Collar <{}>", id(COLLAR)), 2)]);
        assert_eq!(store.status(None, "", ""), "/unsit/tploc");
        assert_eq!(store.keep_only(&[id(CUFF)]), []);
        // The end of a session keeps the operator's only.
        let mut store = holding(&rules);
        assert_eq!(store.drop_objects(), (2, 3));
        assert_eq!(store.lines(), ["restrictions: 1", "operator: tploc"]);
        // A reset keeps nothing.
        let mut store = holding(&rules);
        assert_eq!(store.reset(), 4);
        assert_eq!(store.lines(), ["restrictions: none"]);
        assert_eq!(store.reset(), 0);
    }

    #[test]
    fn no_more_than_the_limit_is_kept() {
        let mut store = Restrictions::default();
        for number in 0..MAX_RESTRICTIONS {
            assert_eq!(
                store.set(holder(COLLAR), "Collar", "sendchannel", Some(&number.to_string())),
                Set::New
            );
        }
        assert_eq!(hold(&mut store, holder(CUFF), "unsit"), Set::Full);
        assert_eq!(hold(&mut store, holder(COLLAR), "sendchannel:5"), Set::Already);
        assert_eq!(store.count(), MAX_RESTRICTIONS);
        assert!(store.lift(holder(COLLAR), "sendchannel", Some("5")));
        assert_eq!(hold(&mut store, holder(CUFF), "unsit"), Set::New);
    }

    #[test]
    fn the_status_lines_name_holder_restriction_and_effect() {
        let store = holding(&[(holder(COLLAR), "detach"), (holder(COLLAR), "showloc"), (Who::Operator, "tploc")]);
        let collar = id(COLLAR);
        assert_eq!(
            store.lines(),
            [
                "restrictions: 3".to_string(),
                format!("Collar <{collar}>: detach"),
                format!("Collar <{collar}>: showloc (no effect)"),
                "operator: tploc".to_string(),
            ]
        );
        assert_eq!(Restrictions::default().lines(), ["restrictions: none"]);
    }

    #[test]
    fn status_queries_list_a_holders_restrictions_or_everyones() {
        let mut rules: Vec<(Who, &str)> = ["tploc", "tplure", "tplm", "sittp"]
            .iter()
            .map(|text| (holder(COLLAR), *text))
            .collect();
        rules.push((holder(CUFF), "detach"));
        rules.push((Who::Operator, "tploc"));
        let store = holding(&rules);
        let collar = Some(holder(COLLAR));
        assert_eq!(store.status(collar, "", ""), "/tploc/tplure/tplm/sittp");
        assert_eq!(store.status(collar, "sittp", ""), "/sittp");
        assert_eq!(store.status(collar, "TPL", ""), "/tploc/tplure/tplm");
        assert_eq!(store.status(collar, "tpl", "#"), "#tploc#tplure#tplm");
        assert_eq!(store.status(collar, "", "#"), "#tploc#tplure#tplm#sittp");
        assert_eq!(store.status(Some(holder(CUFF)), "", ""), "/detach");
        assert_eq!(store.status(Some(Who::Operator), "", ""), "/tploc");
        // Everyone's: what two hold appears twice.
        assert_eq!(store.status(None, "", ""), "/tploc/tplure/tplm/sittp/detach/tploc");
        assert_eq!(store.status(Some(holder(9)), "", ""), "");
        // Cut at what a chat line holds, between two characters.
        let mut long = Restrictions::default();
        for number in 0..200 {
            long.set(holder(COLLAR), "Collar", "sendimto", Some(&format!("é{number:04}")));
        }
        let reply = long.status(None, "", "");
        assert!(reply.len() <= MAX_REPLY && reply.len() > MAX_REPLY - 4, "{}", reply.len());
        assert!(reply.starts_with("/sendimto:é0000/"));
    }

    // ------------------------------------------------------------ questions

    #[test]
    fn nothing_is_in_force_without_a_store() {
        let none = Limits::none();
        assert!(none.in_force("unsit").is_none());
        assert!(none.stand(Sitting::Object).is_ok());
        assert!(none.chat(0, SAY, "hello").is_ok());
        assert!(!none.accepts_tp(id(1)));
    }

    #[test]
    fn exceptions_count_from_anyone_unless_the_restriction_is_secure() {
        let jane = Target::Id(id(1));
        let excepted = format!("sendim:{}", id(1));
        // Anyone's exception counts against a plain restriction.
        let store = holding(&[(holder(COLLAR), "sendim"), (holder(CUFF), &excepted)]);
        assert!(limits(&store).against("sendim", jane).is_none());
        assert!(limits(&store).against("sendim", Target::Id(id(2))).is_some());
        // Only the holder's own against the secure form.
        let mut store = holding(&[(holder(COLLAR), "sendim_sec"), (holder(CUFF), &excepted)]);
        assert_eq!(limits(&store).against("sendim", jane).unwrap().rule, "sendim_sec");
        hold(&mut store, holder(COLLAR), &excepted);
        assert!(limits(&store).against("sendim", jane).is_none());
        // And against every restriction while `permissive` is in force.
        let mut store = holding(&[
            (holder(COLLAR), "sendim"),
            (holder(COLLAR), "permissive"),
            (holder(CUFF), &excepted),
        ]);
        assert!(limits(&store).against("sendim", jane).is_some());
        hold(&mut store, holder(COLLAR), &excepted);
        assert!(limits(&store).against("sendim", jane).is_none());
        // Each holder of the restriction needs its exception then.
        hold(&mut store, holder(CUFF), "sendim");
        assert!(limits(&store).against("sendim", jane).is_none());
        let store = holding(&[
            (holder(COLLAR), "sendim_sec"),
            (holder(CUFF), "sendim_sec"),
            (holder(CUFF), &excepted),
        ]);
        assert_eq!(
            limits(&store).against("sendim", jane).unwrap().holder,
            format!("Collar <{}>", id(COLLAR))
        );
    }

    #[test]
    fn an_option_that_cannot_be_read_names_nothing() {
        let store = collared(&[
            "sendim",
            "sendim:jane",
            "sendimto:nobody",
            "sendchannel",
            "sendchannel:seven",
            "sittp:near",
        ]);
        let limits = limits(&store);
        assert!(limits.against("sendim", Target::Id(id(1))).is_some());
        assert!(limits.names("sendimto", Target::Id(id(1))).is_none());
        assert!(limits.against("sendchannel", Target::Channel(7)).is_some());
        // A distance that is none is the default one.
        assert!(limits.sit(Sitting::No, Some(1.4)).is_ok());
        assert_eq!(rule(limits.sit(Sitting::No, Some(1.6))), "sittp:near");
    }

    #[test]
    fn a_holder_can_be_left_out() {
        let store = holding(&[(holder(COLLAR), "unsit"), (holder(CUFF), "sittp")]);
        let without = |who| Limits::new(Some(&store), Some(who));
        assert!(without(holder(COLLAR)).stand(Sitting::Object).is_ok());
        assert_eq!(rule(without(holder(CUFF)).stand(Sitting::Object)), "unsit");
        assert!(without(holder(CUFF)).sit(Sitting::No, Some(20.0)).is_ok());
        assert_eq!(rule(without(Who::Operator).sit(Sitting::No, Some(20.0))), "sittp");
        // The operator's own do not hold `rlv cmd`.
        let store = holding(&[(Who::Operator, "unsit")]);
        assert!(Limits::new(Some(&store), Some(Who::Operator)).stand(Sitting::Object).is_ok());
        assert_eq!(limits(&store).stand(Sitting::Object).unwrap_err().holder, "the operator");
    }

    #[test]
    fn a_refusal_names_the_restriction_and_its_holder() {
        let store = collared(&["unsit"]);
        let refusal = limits(&store).stand(Sitting::Object).unwrap_err();
        let collar = id(COLLAR);
        assert_eq!(refusal.short(), format!("unsit, held by Collar <{collar}>"));
        assert_eq!(
            refusal.answer(),
            format!("refused by RLV: unsit, held by Collar <{collar}>; `rslbot ctl rlv mode override` lets one command through")
        );
        let store = holding(&[(Who::Operator, "tploc")]);
        let refusal = limits(&store).teleport(Sitting::No, None).unwrap_err();
        assert_eq!(refusal.short(), "tploc, held by the operator");
        let store = collared(&["sittp"]);
        let refusal = limits(&store).sit(Sitting::No, None).unwrap_err();
        assert!(refusal.answer().contains("does not know how far"), "{}", refusal.answer());
    }

    fn said(spoken: Result<Spoken, Refusal>) -> Vec<(i32, u8, String)> {
        let spoken = spoken.unwrap_or_else(|refusal| panic!("refused: {}", refusal.short()));
        spoken
            .utterances
            .into_iter()
            .map(|said| (said.channel, said.volume, said.text))
            .collect()
    }

    #[test]
    fn sendchat_lets_emotes_and_slash_words_through_shortened() {
        let store = collared(&["sendchat"]);
        let limits = limits(&store);
        assert_eq!(rule(limits.chat(0, SAY, "hello there").map(|_| ())), "sendchat");
        let spoken = limits.chat(0, SAY, "/me nods slowly. Then she speaks at length").unwrap();
        assert_eq!(spoken.utterances[0].text, "/me nods slowly");
        assert!(
            spoken.changes[0].contains("shortened") && spoken.changes[0].contains("sendchat"),
            "{:?}",
            spoken.changes
        );
        // Thirty characters of an emote, fifteen of another slash word.
        assert_eq!(
            said(limits.chat(0, SAY, "/me walks all the way across the room")),
            [(0, SAY, "/me walks all the way across t".to_string())]
        );
        assert_eq!(
            said(limits.chat(0, SAY, "/dance with everybody here")),
            [(0, SAY, "/dance with eve".to_string())]
        );
        // A short one goes out as it is, with nothing to report.
        assert_eq!(limits.chat(0, SAY, "/me waves").unwrap().changes, Vec::<String>::new());
        // Special signs discard it.
        for text in ["/me says \"hello\"", "/me half-smiles", "/me (quietly) nods", "/a=b"] {
            assert_eq!(rule(limits.chat(0, SAY, text).map(|_| ())), "sendchat", "{text}");
        }
        // What is cut off takes the space before it along.
        assert_eq!(said(limits.chat(0, SAY, "/me . and more")), [(0, SAY, "/me".to_string())]);
        // Other channels are not its business.
        assert_eq!(said(limits.chat(5, SAY, "hello there")), [(5, SAY, "hello there".to_string())]);
        // With the emote exception emotes are whole; the signs still count.
        let store = holding(&[(holder(COLLAR), "sendchat"), (holder(CUFF), "emote")]);
        let whole = "/me walks all the way across the room and sits down. Then sleeps";
        assert_eq!(said(self::limits(&store).chat(0, SAY, whole)), [(0, SAY, whole.to_string())]);
        assert!(self::limits(&store).chat(0, SAY, "/me says \"hello\"").is_err());
        assert_eq!(
            said(self::limits(&store).chat(0, SAY, "/dance with everybody here")),
            [(0, SAY, "/dance with eve".to_string())]
        );
    }

    #[test]
    fn the_volume_restrictions_change_the_range_only() {
        let store = collared(&["chatshout"]);
        let spoken = limits(&store).chat(0, SHOUT, "over here").unwrap();
        assert_eq!(
            spoken.utterances,
            [Utterance {
                channel: 0,
                volume: SAY,
                text: "over here".to_string()
            }]
        );
        assert!(spoken.changes[0].contains("chatshout"), "{:?}", spoken.changes);
        assert_eq!(said(limits(&store).chat(0, WHISPER, "psst")), [(0, WHISPER, "psst".to_string())]);
        let store = collared(&["chatwhisper"]);
        assert_eq!(said(limits(&store).chat(0, WHISPER, "psst")), [(0, SAY, "psst".to_string())]);
        let store = collared(&["chatnormal"]);
        assert_eq!(said(limits(&store).chat(0, SAY, "hello")), [(0, WHISPER, "hello".to_string())]);
        assert_eq!(said(limits(&store).chat(0, SHOUT, "hello")), [(0, WHISPER, "hello".to_string())]);
        // All three: everything is whispered.
        let store = collared(&["chatnormal", "chatshout", "chatwhisper"]);
        for volume in [WHISPER, SAY, SHOUT] {
            assert_eq!(said(limits(&store).chat(0, volume, "hello")), [(0, WHISPER, "hello".to_string())]);
        }
        // Not on other channels.
        assert_eq!(said(limits(&store).chat(3, SHOUT, "hello")), [(3, SHOUT, "hello".to_string())]);
    }

    #[test]
    fn redirected_chat_leaves_channel_zero() {
        let store = collared(&["redirchat:5", "sendchat"]);
        let spoken = limits(&store).chat(0, SAY, "hello").unwrap();
        assert_eq!(
            spoken.utterances,
            [Utterance {
                channel: 5,
                volume: SAY,
                text: "hello".to_string()
            }]
        );
        assert!(spoken.changes[0].contains("redirchat:5"), "{:?}", spoken.changes);
        // Emotes are not chat: `sendchat` deals with this one.
        assert_eq!(said(limits(&store).chat(0, SAY, "/me waves")), [(0, SAY, "/me waves".to_string())]);
        // Several redirections, and one for emotes.
        let store = holding(&[
            (holder(COLLAR), "redirchat:5"),
            (holder(CUFF), "redirchat:6"),
            (holder(CUFF), "redirchat:5"),
            (holder(CUFF), "rediremote:9"),
        ]);
        assert_eq!(
            said(limits(&store).chat(0, SAY, "hello")),
            [(5, SAY, "hello".to_string()), (6, SAY, "hello".to_string())]
        );
        assert_eq!(said(limits(&store).chat(0, SAY, "/me waves")), [(9, SAY, "/me waves".to_string())]);
        // A redirection to a closed channel is left out.
        let store = collared(&["redirchat:5", "redirchat:6", "sendchannel", "sendchannel:6"]);
        assert_eq!(said(limits(&store).chat(0, SAY, "hello")), [(6, SAY, "hello".to_string())]);
        let store = collared(&["redirchat:5", "sendchannel"]);
        assert_eq!(rule(limits(&store).chat(0, SAY, "hello").map(|_| ())), "sendchannel");
    }

    #[test]
    fn private_channels_are_closed_but_for_exceptions() {
        let store = collared(&["sendchannel", "sendchannel:7"]);
        assert_eq!(rule(limits(&store).chat(8, SAY, "open").map(|_| ())), "sendchannel");
        assert_eq!(said(limits(&store).chat(7, SAY, "open")), [(7, SAY, "open".to_string())]);
        assert_eq!(said(limits(&store).chat(0, SAY, "open")), [(0, SAY, "open".to_string())]);
        let store = collared(&["sendchannel_except:8"]);
        assert_eq!(rule(limits(&store).chat(8, SAY, "open").map(|_| ())), "sendchannel_except:8");
        assert!(limits(&store).chat(7, SAY, "open").is_ok());
        let store = holding(&[(holder(COLLAR), "sendchannel_sec"), (holder(CUFF), "sendchannel:7")]);
        assert_eq!(rule(limits(&store).chat(7, SAY, "open").map(|_| ())), "sendchannel_sec");
    }

    #[test]
    fn instant_messages_are_held_by_sendim_and_startim() {
        let (jane, john) = (id(1), id(2));
        let store = collared(&["sendim"]);
        assert_eq!(rule(limits(&store).im(jane, true)), "sendim");
        let barred = format!("sendimto:{jane}");
        let store = collared(&[&barred]);
        assert_eq!(rule(limits(&store).im(jane, true)), barred);
        assert!(limits(&store).im(john, false).is_ok());
        // No new conversations; open ones go on.
        let store = collared(&["startim"]);
        assert!(limits(&store).im(jane, true).is_ok());
        assert_eq!(rule(limits(&store).im(john, false)), "startim");
        let excepted = format!("startim:{john}");
        let store = collared(&["startim", &excepted]);
        assert!(limits(&store).im(john, false).is_ok());
        let barred = format!("startimto:{john}");
        let store = collared(&[&barred]);
        assert_eq!(rule(limits(&store).im(john, false)), barred);
        assert!(limits(&store).im(john, true).is_ok());
        // A group's name in the option names no avatar.
        let store = collared(&["sendimto:allgroups"]);
        assert!(limits(&store).im(jane, false).is_ok());
    }

    #[test]
    fn sitting_and_standing_follow_their_table() {
        let store = collared(&["unsit"]);
        let limits = limits(&store);
        assert_eq!(rule(limits.stand(Sitting::Object)), "unsit");
        assert_eq!(rule(limits.stand(Sitting::Ground)), "unsit");
        assert!(limits.stand(Sitting::No).is_ok());
        // Sitting down is no standing up; changing seats is.
        assert!(limits.sit(Sitting::No, Some(3.0)).is_ok());
        assert_eq!(rule(limits.sit(Sitting::Object, Some(3.0))), "unsit");
        assert_eq!(rule(limits.sit_ground(Sitting::Object)), "unsit");
        assert!(limits.sit_ground(Sitting::No).is_ok());
        let store = collared(&["sit"]);
        assert_eq!(rule(self::limits(&store).sit(Sitting::No, Some(0.5))), "sit");
        assert_eq!(rule(self::limits(&store).sit_ground(Sitting::No)), "sit");
        let store = collared(&["interact"]);
        assert_eq!(rule(self::limits(&store).sit(Sitting::No, Some(0.5))), "interact");
        assert!(self::limits(&store).sit_ground(Sitting::No).is_ok());
        // From afar.
        let store = collared(&["sittp"]);
        assert_eq!(rule(self::limits(&store).sit(Sitting::No, Some(10.0))), "sittp");
        assert!(self::limits(&store).sit(Sitting::No, Some(1.0)).is_ok());
        assert_eq!(rule(self::limits(&store).sit(Sitting::No, None)), "sittp");
        assert!(self::limits(&store).sit_ground(Sitting::No).is_ok());
        // The smallest distance of all.
        let store = holding(&[(holder(COLLAR), "sittp:5"), (holder(CUFF), "sittp:3")]);
        assert_eq!(rule(self::limits(&store).sit(Sitting::No, Some(4.0))), "sittp:3");
        assert!(self::limits(&store).sit(Sitting::No, Some(2.5)).is_ok());
    }

    #[test]
    fn teleports_follow_their_table() {
        let store = collared(&["tploc"]);
        assert_eq!(rule(limits(&store).teleport(Sitting::No, None)), "tploc");
        assert_eq!(rule(limits(&store).teleport(Sitting::No, Some(1.0))), "tploc");
        assert_eq!(rule(limits(&store).forced_teleport(Sitting::No)), "tploc");
        // Inside the region only.
        let store = collared(&["tplocal"]);
        assert_eq!(rule(limits(&store).teleport(Sitting::No, Some(1.0))), "tplocal");
        assert!(limits(&store).teleport(Sitting::No, None).is_ok());
        assert!(limits(&store).forced_teleport(Sitting::No).is_ok());
        let store = collared(&["tplocal:10"]);
        assert!(limits(&store).teleport(Sitting::No, Some(5.0)).is_ok());
        assert_eq!(rule(limits(&store).teleport(Sitting::No, Some(12.0))), "tplocal:10");
        // One holder without a distance closes it for all.
        let store = holding(&[(holder(COLLAR), "tplocal:10"), (holder(CUFF), "tplocal")]);
        assert_eq!(rule(limits(&store).teleport(Sitting::No, Some(5.0))), "tplocal");
        // Seated and held.
        let store = collared(&["unsit"]);
        assert_eq!(rule(limits(&store).teleport(Sitting::Object, None)), "unsit");
        assert_eq!(rule(limits(&store).forced_teleport(Sitting::Ground)), "unsit");
        assert!(limits(&store).teleport(Sitting::No, None).is_ok());
    }

    #[test]
    fn offers_and_requests_follow_tplure_and_tprequest() {
        let (jane, john) = (id(1), id(2));
        let excepted = format!("tplure:{jane}");
        let store = collared(&["tplure", &excepted]);
        assert!(limits(&store).lure(jane).is_ok());
        assert_eq!(rule(limits(&store).lure(john)), "tplure");
        let accepted = format!("accepttp:{jane}");
        let store = collared(&[&accepted]);
        assert!(limits(&store).accepts_tp(jane) && !limits(&store).accepts_tp(john));
        let store = collared(&["accepttp"]);
        assert!(limits(&store).accepts_tp(john));
        let store = collared(&["tprequest", "accepttprequest"]);
        assert_eq!(rule(limits(&store).tp_request(jane)), "tprequest");
        assert!(limits(&store).answers_tp_request(jane));
        let answered = format!("accepttprequest:{jane}");
        let store = collared(&[&answered]);
        assert!(limits(&store).tp_request(jane).is_ok());
        assert!(limits(&store).answers_tp_request(jane) && !limits(&store).answers_tp_request(john));
        let store = collared(&["acceptpermission", "denypermission", "showinv"]);
        assert!(limits(&store).grants_permissions());
        assert_eq!(limits(&store).denies_permissions().unwrap().rule, "denypermission");
        assert_eq!(rule(limits(&store).inventory()), "showinv");
    }

    fn thing(placement: Placement, object: u8, distance: f32) -> Thing {
        Thing {
            placement,
            prims: vec![id(object)],
            distance: Some(distance),
        }
    }

    #[test]
    fn touching_follows_its_table() {
        let door = thing(Placement::InWorld, 20, 1.0);
        let own = thing(Placement::Worn, 21, 0.0);
        let others = thing(Placement::WornBy(id(2)), 22, 1.0);
        let hud = thing(Placement::Hud, 23, 0.0);
        let all = [&door, &own, &others, &hud];
        let refused = |rules: &[&str]| -> Vec<bool> {
            let store = collared(rules);
            all.iter().map(|thing| limits(&store).touch(thing).is_err()).collect()
        };
        assert_eq!(refused(&[]), [false, false, false, false]);
        assert_eq!(refused(&["interact"]), [true, true, true, true]);
        assert_eq!(refused(&["touchall"]), [true, true, true, false]);
        assert_eq!(refused(&["touchworld"]), [true, false, false, false]);
        assert_eq!(refused(&["touchattach"]), [false, true, true, false]);
        assert_eq!(refused(&["touchattachself"]), [false, true, false, false]);
        assert_eq!(refused(&["touchattachother"]), [false, false, true, false]);
        assert_eq!(refused(&[&format!("touchattachother:{}", id(2))]), [false, false, true, false]);
        assert_eq!(refused(&[&format!("touchattachother:{}", id(3))]), [false, false, false, false]);
        assert_eq!(refused(&["touchhud"]), [false, false, false, true]);
        assert_eq!(refused(&[&format!("touchhud:{}", id(23))]), [false, false, false, true]);
        assert_eq!(refused(&[&format!("touchhud:{}", id(24))]), [false, false, false, false]);
        assert_eq!(refused(&[&format!("touchthis:{}", id(20))]), [true, false, false, false]);
        // An excepted object.
        assert_eq!(
            refused(&["touchworld", &format!("touchworld:{}", id(20))]),
            [false, false, false, false]
        );
        // Too far: what is on the avatar is at no distance.
        let far = thing(Placement::InWorld, 25, 6.0);
        let store = collared(&["fartouch"]);
        assert_eq!(rule(limits(&store).touch(&far)), "fartouch");
        assert!(limits(&store).touch(&door).is_ok());
        assert!(limits(&store)
            .touch(&Thing {
                distance: None,
                ..own.clone()
            })
            .is_ok());
        let store = collared(&["touchfar:10", "fartouch:8"]);
        assert!(limits(&store).touch(&far).is_ok());
        assert_eq!(rule(limits(&store).touch(&thing(Placement::InWorld, 25, 9.0))), "fartouch:8");
        // An object the simulator has not described may be anything.
        let unknown = Thing::unknown(id(30));
        for rules in [
            &["touchworld"][..],
            &["touchattach"],
            &["touchattachother"],
            &["fartouch"],
            &["touchall"],
        ] {
            assert!(limits(&collared(rules)).touch(&unknown).is_err(), "{rules:?}");
        }
        assert!(limits(&collared(&["touchattachself"])).touch(&unknown).is_ok());
        assert!(limits(&collared(&["touchhud"])).touch(&unknown).is_ok());
    }

    #[test]
    fn touchme_lets_its_object_be_touched() {
        let lever = Thing {
            placement: Placement::InWorld,
            // The touched prim and its root.
            prims: vec![id(31), id(30)],
            distance: Some(8.0),
        };
        let rules = [(holder(COLLAR), "touchall"), (holder(COLLAR), "fartouch"), (holder(30), "touchme")];
        let store = holding(&rules);
        assert!(limits(&store).touch(&lever).is_ok());
        // Another object's `touchme` is not this one's.
        assert!(limits(&store).touch(&thing(Placement::InWorld, 32, 1.0)).is_err());
        // Not against `interact`.
        let mut store = holding(&rules);
        hold(&mut store, holder(CUFF), "interact");
        assert_eq!(rule(limits(&store).touch(&lever)), "interact");
        // A restriction that names the root holds the touched prim too.
        let store = collared(&[&format!("touchthis:{}", id(30))]);
        assert!(limits(&store).touch(&lever).is_err());
        let store = collared(&["touchworld", &format!("touchworld:{}", id(30))]);
        assert!(limits(&store).touch(&lever).is_ok());
    }

    #[test]
    fn attachments_follow_their_table() {
        const CHEST: u8 = 1;
        const LEFT_HAND: u8 = 5;
        // A lock is its holder's: the attachment it is, or is part of.
        let store = collared(&["detach"]);
        assert_eq!(rule(limits(&store).detach(CHEST, &[id(COLLAR)])), "detach");
        assert_eq!(rule(limits(&store).detach(CHEST, &[id(40), id(COLLAR)])), "detach");
        assert!(limits(&store).detach(CHEST, &[id(41)]).is_ok());
        assert!(limits(&store).attach(Some(CHEST)).is_ok());
        // The operator's, and an object's that is not worn, lock nothing.
        let store = holding(&[(Who::Operator, "detach")]);
        assert!(limits(&store).detach(CHEST, &[id(COLLAR)]).is_ok());
        // A locked point: nothing off it, nothing onto it.
        let store = collared(&["detach:left hand"]);
        assert_eq!(rule(limits(&store).detach(LEFT_HAND, &[id(41)])), "detach:left hand");
        assert_eq!(rule(limits(&store).attach(Some(LEFT_HAND))), "detach:left hand");
        assert!(limits(&store).detach(CHEST, &[id(41)]).is_ok());
        assert!(limits(&store).attach(Some(CHEST)).is_ok());
        let store = collared(&["remattach"]);
        assert_eq!(rule(limits(&store).detach(CHEST, &[id(41)])), "remattach");
        assert!(limits(&store).attach(Some(CHEST)).is_ok());
        let store = collared(&["remattach:Chest"]);
        assert_eq!(rule(limits(&store).detach(CHEST, &[id(41)])), "remattach:Chest");
        assert!(limits(&store).detach(LEFT_HAND, &[id(41)]).is_ok());
        let store = collared(&["addattach"]);
        assert_eq!(rule(limits(&store).attach(Some(CHEST))), "addattach");
        assert_eq!(rule(limits(&store).attach(None)), "addattach");
        assert!(limits(&store).detach(CHEST, &[id(41)]).is_ok());
        // A point by its RLV name.
        let store = collared(&["addattach:center"]);
        assert_eq!(rule(limits(&store).attach(Some(35))), "addattach:center");
        assert!(limits(&store).attach(Some(CHEST)).is_ok());
        // A point the simulator would choose may be the closed one.
        let refusal = limits(&store).attach(None).unwrap_err();
        assert_eq!(refusal.rule, "addattach:center");
        assert!(refusal.short().contains("not known"), "{}", refusal.short());
        // A folder's name closes no point.
        let store = collared(&["addattach:Outfits/Beach"]);
        assert!(limits(&store).attach(None).is_ok());
    }

    #[test]
    fn clothing_follows_its_table() {
        let kind = |name: &str| inventory::parse_kind(name).unwrap();
        let store = collared(&["remoutfit:shirt"]);
        assert_eq!(rule(limits(&store).unwear(kind("shirt"))), "remoutfit:shirt");
        assert!(limits(&store).unwear(kind("pants")).is_ok());
        assert!(limits(&store).wear(kind("shirt"), false).is_ok());
        let store = collared(&["remoutfit"]);
        assert_eq!(rule(limits(&store).unwear(kind("pants"))), "remoutfit");
        // A body part replaces the one that is worn.
        assert_eq!(rule(limits(&store).wear(kind("skin"), true)), "remoutfit");
        assert!(limits(&store).wear(kind("shirt"), false).is_ok());
        let store = collared(&["addoutfit"]);
        assert_eq!(rule(limits(&store).wear(kind("jacket"), false)), "addoutfit");
        assert!(limits(&store).unwear(kind("jacket")).is_ok());
        let store = collared(&["addoutfit:jacket"]);
        assert_eq!(rule(limits(&store).wear(kind("jacket"), false)), "addoutfit:jacket");
        assert!(limits(&store).wear(kind("shirt"), false).is_ok());
    }

    // --------------------------------------------- restrictions as commands

    fn cuff() -> Issuer {
        Issuer {
            id: Uuid::from_bytes([CUFF; 16]),
            name: "Cuff".to_string(),
            operator: false,
        }
    }

    fn operator() -> Issuer {
        Issuer::operator(Uuid::from_bytes([1; 16]))
    }

    #[test]
    fn restrictions_are_kept_for_their_sender() {
        let mut rig = Rig::new();
        assert_eq!(rig.deal("@sendim=n"), Step::Done);
        assert_eq!(rig.deal("@Detach:Chest=ADD"), Step::Done);
        assert_eq!(rig.deal("@showloc=n"), Step::Done);
        // Set already, and lifting what is not set, are carried out as well.
        assert_eq!(rig.deal("@sendim=add"), Step::Done);
        assert_eq!(rig.deal("@unsit=y"), Step::Done);
        let collar = collar().id;
        assert_eq!(
            rig.grid.kept.lines(),
            [
                "restrictions: 3".to_string(),
                format!("Collar <{collar}>: sendim"),
                format!("Collar <{collar}>: detach:Chest"),
                format!("Collar <{collar}>: showloc (no effect)"),
            ]
        );
        assert_eq!(rig.deal("@sendim=y"), Step::Done);
        assert_eq!(rig.deal("@detach:chest=rem"), Step::Done);
        assert_eq!(rig.grid.kept.count(), 1);
        // What the bot does not know is not kept.
        assert_eq!(failed(rig.deal("@frobnicate=n")), "not supported");
        assert_eq!(failed(rig.deal("@notify:5=add")), "not supported");
        assert_eq!(rig.grid.kept.count(), 1);
        // The operator is a holder apart: neither clears the other's.
        assert_eq!(rig.deal_from(&operator(), "@tploc=n"), Step::Done);
        assert_eq!(rig.deal_from(&operator(), "@tplure=n"), Step::Done);
        assert_eq!(rig.deal("@clear"), Step::Done);
        assert_eq!(rig.grid.kept.lines(), ["restrictions: 2", "operator: tploc", "operator: tplure"]);
        assert_eq!(rig.deal("@showloc=n"), Step::Done);
        assert_eq!(rig.deal_from(&operator(), "@clear=tplu"), Step::Done);
        assert_eq!(rig.deal_from(&operator(), "@clear"), Step::Done);
        assert_eq!(rig.grid.kept.lines().len(), 2);
        // None of it sends anything or moves the avatar.
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn status_queries_say_what_the_sender_holds_or_what_everyone_does() {
        let mut rig = Rig::new();
        rig.deal("@tploc=n");
        rig.deal("@tplure=n");
        rig.deal("@tplm=n");
        rig.deal("@sittp=n");
        rig.deal_from(&cuff(), "@detach=n");
        let said = |text: &str| Step::Said(text.to_string());
        assert_eq!(rig.deal("@getstatus=2222"), said("/tploc/tplure/tplm/sittp"));
        assert_eq!(rig.deal("@getstatus:sittp=2222"), said("/sittp"));
        assert_eq!(rig.deal("@getstatus:tpl=2222"), said("/tploc/tplure/tplm"));
        assert_eq!(rig.deal("@getstatus:tpl;#=2222"), said("#tploc#tplure#tplm"));
        assert_eq!(rig.deal("@getstatus:;#=2222"), said("#tploc#tplure#tplm#sittp"));
        assert_eq!(rig.deal("@getstatus:;=2222"), said("/tploc/tplure/tplm/sittp"));
        assert_eq!(rig.deal_from(&cuff(), "@getstatus=2222"), said("/detach"));
        assert_eq!(rig.deal_from(&cuff(), "@getstatusall=-7"), said("/tploc/tplure/tplm/sittp/detach"));
        // The operator's console has its own, and is part of everyone.
        rig.deal_from(&operator(), "@tploc=n");
        assert_eq!(rig.deal_from(&operator(), "@getstatus=5"), said("/tploc"));
        assert_eq!(rig.deal("@getstatusall:tploc=5"), said("/tploc/tploc"));
        let calls = rig.calls();
        assert_eq!(calls.len(), 10);
        assert_eq!(calls[0], Call::Say(2222, 1, "/tploc/tplure/tplm/sittp".to_string()));
        assert_eq!(calls[7], Call::Say(-7, 1, "/tploc/tplure/tplm/sittp/detach".to_string()));
        // After a clear the reply is empty, and is said all the same.
        rig.deal("@clear");
        assert_eq!(rig.deal("@getstatus=2222"), said(""));
        assert_eq!(rig.calls(), [Call::Say(2222, 1, String::new())]);
        // As for the version queries, channel 0 cannot be answered on.
        assert_eq!(failed(rig.deal("@getstatus=0")), "channel 0 cannot be answered on");
        // A private channel that is closed to the avatar is open to replies.
        rig.deal("@sendchannel=n");
        assert_eq!(rig.deal("@getstatus=2222"), said("/sendchannel"));
    }

    fn far_chair(rig: &mut Rig) {
        let thing = Thing {
            placement: Placement::InWorld,
            prims: vec![chair()],
            distance: Some(20.0),
        };
        rig.grid.things.push((chair(), thing));
    }

    #[test]
    fn a_force_command_is_held_by_the_restrictions_of_others() {
        let mut rig = Rig::new();
        far_chair(&mut rig);
        rig.grid.sitting = Sitting::Object;
        rig.deal_from(&cuff(), "@unsit=n");
        let sit = format!("@sit:{}=force", chair());
        let held = format!("unsit, held by Cuff <{}>", cuff().id);
        assert_eq!(failed(rig.deal(&sit)), held);
        assert_eq!(failed(rig.deal("@unsit=force")), held);
        assert_eq!(failed(rig.deal("@tpto:Home/128/100/25=force")), held);
        assert_eq!(failed(rig.deal("@tpto:256128/256100/25=force")), held);
        // The console is a sender like any other.
        assert_eq!(failed(rig.deal_from(&operator(), "@unsit=force")), held);
        rig.grid.sitting = Sitting::No;
        rig.deal_from(&cuff(), "@clear");
        // Each of the restrictions the specification names.
        for (restriction, command) in [
            ("sit", sit.as_str()),
            ("interact", sit.as_str()),
            ("sittp", sit.as_str()),
            ("sit", "@sitground=force"),
            ("tploc", "@tpto:Home/128/100/25=force"),
            ("tploc", "@tpto:256128/256100/25=force"),
        ] {
            rig.deal_from(&cuff(), &format!("@{restriction}=n"));
            let reason = failed(rig.deal(command));
            assert!(reason.starts_with(&format!("{restriction}, held by Cuff")), "{command}: {reason}");
            rig.deal_from(&cuff(), "@clear");
        }
        // A forced teleport is not a local one, and a forced ground sit
        // leaves no seat when the avatar sits on the ground already.
        rig.deal_from(&cuff(), "@tplocal=n");
        assert_eq!(rig.deal("@tpto:Home/128/100/25=force"), Step::Moved(Movement::Teleport));
        rig.deal_from(&cuff(), "@unsit=n");
        rig.grid.sitting = Sitting::Ground;
        assert_eq!(rig.deal("@sitground=force"), Step::Moved(Movement::Ground));
        assert_eq!(
            rig.calls(),
            [Call::Teleport("Home".to_string(), [128.0, 100.0, 25.0]), Call::SitGround]
        );
        assert!(rig.commands.take_manual());
        // A refused one moves nothing.
        rig.grid.sitting = Sitting::Object;
        failed(rig.deal("@unsit=force"));
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn a_senders_own_restrictions_do_not_hold_its_force_commands() {
        let mut rig = Rig::new();
        far_chair(&mut rig);
        rig.grid.sitting = Sitting::Object;
        for restriction in ["unsit", "sittp", "tploc", "sit", "interact"] {
            rig.deal(&format!("@{restriction}=n"));
        }
        assert_eq!(rig.deal(&format!("@sit:{}=force", chair())), Step::Moved(Movement::Sit));
        assert_eq!(rig.deal("@unsit=force"), Step::Moved(Movement::Stand));
        assert_eq!(rig.deal("@sitground=force"), Step::Moved(Movement::Ground));
        assert_eq!(rig.deal("@tpto:Home/128/100/25=force"), Step::Moved(Movement::Teleport));
        assert_eq!(rig.deal("@tpto:256128/256100/25=force"), Step::Moved(Movement::Teleport));
        assert_eq!(rig.calls().len(), 5);
        // They do hold everyone else's, the operator's console included.
        assert!(failed(rig.deal_from(&cuff(), "@unsit=force")).starts_with("unsit, held by Collar"));
        assert!(failed(rig.deal_from(&operator(), "@unsit=force")).starts_with("unsit, held by Collar"));
        // And the console's own do not hold the console.
        let mut rig = Rig::new();
        rig.grid.sitting = Sitting::Object;
        rig.deal_from(&operator(), "@unsit=n");
        assert_eq!(rig.deal_from(&operator(), "@unsit=force"), Step::Moved(Movement::Stand));
        assert_eq!(failed(rig.deal("@unsit=force")), "unsit, held by the operator");
    }

    #[test]
    fn a_followed_sit_is_held_to_the_restrictions_before_the_seat_is_held() {
        let mut rig = Rig::new();
        rig.follow = true;
        far_chair(&mut rig);
        let sit = format!("@sit:{}=force", chair());
        // The cage's own `sittp` does not keep the cage from seating the avatar.
        rig.deal("@sittp=n");
        assert_eq!(rig.deal(&sit), Step::HoldSeat(chair()));
        // Another object's does.
        assert!(failed(rig.deal_from(&cuff(), &sit)).starts_with("sittp, held by Collar"));
        assert_eq!(rig.calls(), []);
    }

    /// The simulator has described the chair now, this far away.
    fn chair_described(rig: &mut Rig, distance: f32) {
        let thing = Thing {
            placement: Placement::InWorld,
            prims: vec![chair()],
            distance: Some(distance),
        };
        rig.grid.things.push((chair(), thing));
    }

    #[test]
    fn a_forced_sit_waits_for_its_seat_to_be_described() {
        let mut rig = Rig::new();
        rig.deal_from(&cuff(), "@sittp:20=n");
        let sit = format!("@sit:{}=force", chair());
        // Begun: the bench is looked for, and nothing moves yet.
        assert_eq!(rig.deal(&sit), Step::Moved(Movement::Sit));
        assert_eq!(rig.calls(), [Call::LookFor(chair())]);
        assert!(!rig.commands.take_manual());
        // Described 3 m away: the avatar sits, and the bot goes manual.
        chair_described(&mut rig, 3.0);
        rig.commands.described(chair(), Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), [Call::Sit(chair())]);
        assert!(rig.commands.take_manual());
        // By hand, as the operator's console, the same.
        let mut rig = Rig::new();
        rig.deal_from(&cuff(), "@sittp:20=n");
        assert_eq!(rig.deal_from(&operator(), &sit), Step::Moved(Movement::Sit));
        assert_eq!(rig.calls(), [Call::LookFor(chair())]);
    }

    #[test]
    fn a_followed_sit_waits_for_its_seat_to_be_described() {
        let mut rig = Rig::new();
        rig.follow = true;
        rig.deal_from(&cuff(), "@sittp:20=n");
        let sit = format!("@sit:{}=force", chair());
        // Followed sit that waits: looked for, and not held yet.
        assert_eq!(rig.deal(&sit), Step::Sought(chair()));
        assert_eq!(rig.deal_from(&operator(), &sit), Step::Sought(chair()));
        assert_eq!(rig.calls(), [Call::LookFor(chair()), Call::LookFor(chair())]);
        // Dealt with again once described: within 20 m it is held.
        chair_described(&mut rig, 3.0);
        assert_eq!(rig.deal(&sit), Step::HoldSeat(chair()));
        // Its own `sittp` would not have held it either: nothing to look for.
        let mut rig = Rig::new();
        rig.follow = true;
        rig.deal("@sittp=n");
        assert_eq!(rig.deal(&sit), Step::HoldSeat(chair()));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn a_followed_sit_is_refused_after_the_wait() {
        let mut rig = Rig::new();
        rig.follow = true;
        rig.deal_from(&cuff(), "@sittp=n");
        let sit = format!("@sit:{}=force", chair());
        assert_eq!(rig.deal(&sit), Step::Sought(chair()));
        // Described 10 m away: refused, naming the restriction.
        chair_described(&mut rig, 10.0);
        let reason = failed(rig.deal(&sit));
        assert!(
            reason.starts_with("sittp, held by Cuff") && reason.contains("10.0 m away"),
            "{reason}"
        );
        // Never described: refused once the scan has had its time.
        let mut rig = Rig::new();
        rig.follow = true;
        rig.deal_from(&cuff(), "@sittp=n");
        assert_eq!(rig.deal(&sit), Step::Sought(chair()));
        rig.grid.look = Ok(Look::GaveUp);
        let reason = failed(rig.deal(&sit));
        assert!(reason.contains("does not know how far"), "{reason}");
        // Not in a region: refused as well.
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal(&sit)), NOT_CONNECTED);
    }

    #[test]
    fn what_an_undescribed_object_may_turn_out_to_be_decides_whether_to_look() {
        let mut store = Restrictions::default();
        let collar = Who::Object(Uuid::from_bytes([0xC0; 16]));
        let unknown = Uuid::from_bytes([0x63; 16]);
        let check = |store: &Restrictions| {
            let limits = Limits::new(Some(store), None);
            (limits.sit_undescribed(Sitting::No), limits.touch_undescribed(unknown))
        };
        let looks = |check: &Check| matches!(check, Check::LookFirst(_));
        let refused = |check: &Check| matches!(check, Check::Refused(_));
        // Nothing held, or nothing about sitting or touching: no look.
        assert_eq!(check(&store), (Check::Allowed, Check::Allowed));
        store.set(collar, "Collar", "sendim", None);
        store.set(collar, "Collar", "touchworld", Some(&unknown.to_string()));
        assert_eq!(check(&store), (Check::Allowed, Check::Allowed));
        // A distance, or what the object is, makes it worth looking.
        for (behaviour, option) in [
            ("touchhud", None),
            ("fartouch", Some("3")),
            ("touchattachself", None),
            ("touchworld", None),
        ] {
            let mut store = Restrictions::default();
            store.set(collar, "Collar", behaviour, option);
            assert!(looks(&check(&store).1), "{behaviour}");
        }
        // A `touchthis` may name the root of the prim, and a
        // `touchattachother` the avatar wearing it.
        for (behaviour, option) in [
            ("touchthis", Uuid::from_bytes([1; 16])),
            ("touchattachother", Uuid::from_bytes([2; 16])),
        ] {
            let mut store = Restrictions::default();
            store.set(collar, "Collar", behaviour, Some(&option.to_string()));
            assert!(looks(&check(&store).1), "{behaviour}");
        }
        store.set(collar, "Collar", "sittp", None);
        assert!(looks(&check(&store).0));
        // What refuses whatever the object is refuses at once.
        store.set(collar, "Collar", "interact", None);
        let (sit, touch) = check(&store);
        assert!(refused(&sit) && refused(&touch), "{sit:?} {touch:?}");
    }

    #[test]
    fn a_forced_detach_leaves_what_is_locked() {
        let mut rig = Rig::new();
        let (watch, band) = (Uuid::from_bytes([3; 16]), Uuid::from_bytes([4; 16]));
        let described = |id: Uuid, name: &str| inventory::Item {
            id,
            name: name.to_string(),
            asset_type: inventory::ASSET_OBJECT,
            ..inventory::Item::default()
        };
        // The cuff is the object of `band`, and holds its own lock.
        rig.grid.attached = vec![(watch, 5), (band, 5)];
        rig.grid.worn = vec![(cuff().id, band)];
        rig.deal_from(&cuff(), "@detach=n");
        // A relay takes what it can off the point.
        assert_eq!(rig.deal("@detach:left hand=force"), Step::Done);
        let (id, _, _) = rig.grid.jobs.pop().unwrap();
        let found = Ok(inventory::Outcome::Items(vec![described(watch, "Watch"), described(band, "Band")]));
        rig.commands.inventory_answer(id, found, Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), [Call::Detach(watch)]);
        // By its UUID the locked one stays on, for anyone but itself.
        assert_eq!(rig.deal(&format!("@remattach:{}=force", cuff().id)), Step::Done);
        let (id, _, _) = rig.grid.jobs.pop().unwrap();
        let found = Ok(inventory::Outcome::Found(described(band, "Band")));
        rig.commands.inventory_answer(id, found.clone(), Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.deal_from(&cuff(), "@detachme=force"), Step::Done);
        let (id, _, _) = rig.grid.jobs.pop().unwrap();
        rig.commands.inventory_answer(id, found, Some(&mut rig.grid), rig.now);
        assert_eq!(rig.calls(), [Call::Detach(band)]);
        // Clothing another object holds on stays on.
        rig.grid.outfit = true;
        rig.deal_from(&cuff(), "@remoutfit:shirt=n");
        assert!(failed(rig.deal("@remoutfit:shirt=force")).starts_with("remoutfit:shirt, held by Cuff"));
        assert_eq!(rig.deal_from(&cuff(), "@remoutfit:shirt=force"), Step::Done);
    }

    // -------------------------------------------------------- the bot's own

    #[test]
    fn the_bots_own_steps_are_held_back_as_the_commands_would_be() {
        let none = PersistOverride::default();
        let store = collared(&["tploc", "sittp", "unsit"]);
        let limits = limits(&store);
        let held = |verdict: Verdict| match verdict {
            Verdict::HeldBack(refusal) => refusal.rule,
            other => panic!("expected a step that is held back, got {other:?}"),
        };
        // Caged: no teleport back, in the region or to another.
        assert_eq!(
            held(own_step(limits, Sitting::No, OwnStep::Teleport { local: None }, none)),
            "tploc"
        );
        assert_eq!(
            held(own_step(limits, Sitting::No, OwnStep::Teleport { local: Some(3.0) }, none)),
            "tploc"
        );
        // No sit from afar, no change of seat, no stand.
        assert_eq!(
            held(own_step(limits, Sitting::No, OwnStep::Sit { distance: Some(9.0) }, none)),
            "sittp"
        );
        assert_eq!(
            held(own_step(limits, Sitting::Object, OwnStep::Sit { distance: Some(1.0) }, none)),
            "unsit"
        );
        assert_eq!(held(own_step(limits, Sitting::Object, OwnStep::Stand, none)), "unsit");
        assert_eq!(held(own_step(limits, Sitting::Object, OwnStep::SitGround, none)), "unsit");
        // What nothing forbids goes ahead.
        assert_eq!(
            own_step(limits, Sitting::No, OwnStep::Sit { distance: Some(1.0) }, none),
            Verdict::Go
        );
        assert_eq!(own_step(limits, Sitting::No, OwnStep::SitGround, none), Verdict::Go);
        assert_eq!(own_step(limits, Sitting::No, OwnStep::Stand, none), Verdict::Go);
        // Released, or with RLV off, or for an exempt placement: everything does.
        for free in [Limits::new(Some(&Restrictions::default()), None), Limits::none()] {
            assert_eq!(
                own_step(free, Sitting::Object, OwnStep::Teleport { local: None }, none),
                Verdict::Go
            );
            assert_eq!(own_step(free, Sitting::Object, OwnStep::Stand, none), Verdict::Go);
        }
        // The sender of a followed sit does not hold the way to its seat.
        let store = holding(&[(holder(COLLAR), "sittp"), (holder(CUFF), "sit")]);
        let without = |who| Limits::new(Some(&store), Some(who));
        let step = OwnStep::Sit { distance: Some(20.0) };
        assert_eq!(held(own_step(without(holder(CUFF)), Sitting::No, step, none)), "sittp");
        assert_eq!(held(own_step(without(holder(COLLAR)), Sitting::No, step, none)), "sit");
    }

    #[test]
    fn the_persist_override_setting_lets_a_sort_of_step_through() {
        let store = collared(&["tploc", "sittp", "unsit", "sit"]);
        let limits = limits(&store);
        let (tp, sit) = (PersistOverride { sit: false, tp: true }, PersistOverride { sit: true, tp: false });
        let both = PersistOverride { sit: true, tp: true };
        let through = |verdict: Verdict| match verdict {
            Verdict::LetThrough(refusal) => refusal.rule,
            other => panic!("expected a step that is let through, got {other:?}"),
        };
        let teleport = OwnStep::Teleport { local: None };
        let seat = OwnStep::Sit { distance: Some(20.0) };
        // Back from the hub: the teleport and the sit, each naming what it passed.
        assert_eq!(through(own_step(limits, Sitting::No, teleport, both)), "tploc");
        assert_eq!(through(own_step(limits, Sitting::No, seat, both)), "sit");
        // Seated elsewhere under `unsit`, the teleport still goes.
        let seated = collared(&["unsit"]);
        assert_eq!(through(own_step(self::limits(&seated), Sitting::Object, teleport, tp)), "unsit");
        // Teleports only: the sit is held back, and the stand.
        assert_eq!(through(own_step(limits, Sitting::No, teleport, tp)), "tploc");
        assert!(matches!(own_step(limits, Sitting::No, seat, tp), Verdict::HeldBack(_)));
        assert!(matches!(
            own_step(limits, Sitting::Object, OwnStep::Stand, tp),
            Verdict::HeldBack(_)
        ));
        // Sits only: the sit, the ground sit and the stand go, the teleport waits.
        assert_eq!(through(own_step(limits, Sitting::No, seat, sit)), "sit");
        assert_eq!(through(own_step(limits, Sitting::No, OwnStep::SitGround, sit)), "sit");
        assert_eq!(through(own_step(limits, Sitting::Object, OwnStep::Stand, sit)), "unsit");
        assert!(matches!(own_step(limits, Sitting::No, teleport, sit), Verdict::HeldBack(_)));
        // What nothing forbids is not "let through".
        assert_eq!(own_step(Limits::none(), Sitting::No, teleport, both), Verdict::Go);
        assert_eq!((teleport.sort(), seat.sort(), OwnStep::Stand.sort()), ("tp", "sit", "sit"));
    }
}
