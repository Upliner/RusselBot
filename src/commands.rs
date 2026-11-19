//! What the bot does with a command from the control socket: the chat-bar
//! parsing of `say`, the avatars commands name and their lookup on the
//! grid, the commands that move the avatar, the ones that use its
//! inventory, and the ones waiting for a lookup, for the inventory or for
//! the simulator to say how they ended.

use crate::config::DEFAULT_REQUESTTP_TIMEOUT;
use crate::control::{Request, Response};
use crate::inventory::{self, Clothes, Entry, Failed, Item, ListArgs, Outcome, Removal, What};
use crate::persist::{Location, SitState};
use crate::session::{Touch, ATTACH_LIMIT, DETACH_LIMIT, NOT_CONNECTED};
use crate::udp::messages::{Vector3, MAX_TEXT};
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the grid has to answer a name lookup.
pub const LOOKUP_LIMIT: Duration = Duration::from_secs(10);

/// How long after its deadline an inventory job is given up on without an
/// answer from the worker, which keeps the deadline itself.
const JOB_GRACE: Duration = Duration::from_secs(3);

/// Teleport offers kept for `accepttp`, one per avatar.
const MAX_OFFERS: usize = 100;
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
    let spaced = given.replace('.', " ").to_lowercase();
    let words: Vec<&str> = spaced.split_whitespace().collect();
    match words[..] {
        [first] => Ok(Recipient::Name(format!("{first} resident"))),
        [first, last] => Ok(Recipient::Name(format!("{first} {last}"))),
        _ => Err(format!(
            "\"{given}\" is not an avatar name (`First Last`, `first.last` or one word) or a UUID"
        )),
    }
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

/// Who asks for a command: the operator through the control socket, or an
/// object the avatar owns through an RLV command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    Operator,
    Rlv,
}

/// How the operator log marks the operator's commands.
const CONTROL: &str = "control";

impl Origin {
    /// How the operator log marks a command of this origin.
    fn tag(self) -> &'static str {
        match self {
            Origin::Operator => CONTROL,
            Origin::Rlv => "rlv",
        }
    }
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
    AcceptTp,
    SendTp(String),
}

impl After {
    fn command(&self) -> &'static str {
        match self {
            After::Im(_) => "im",
            After::RequestTp(_) => "requesttp",
            After::AcceptTp => "accepttp",
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
    /// The avatars asked for a teleport, and when.
    asked: HashMap<Uuid, Instant>,
    /// The latest teleport offer of each avatar that was not accepted.
    offers: HashMap<Uuid, Uuid>,
    /// The `sit` command waiting for the simulator's verdict.
    pending_sit: Option<(Sender<Response>, Origin)>,
    /// The `tp` or `accepttp` command waiting for the teleport to end.
    pending_teleport: Option<(Sender<Response>, Origin)>,
    /// `touch` commands waiting for their object to turn up.
    touches: Vec<(Uuid, Sender<Response>)>,
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

fn whom(id: Uuid, name: &str) -> String {
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
            offers: HashMap::new(),
            pending_sit: None,
            pending_teleport: None,
            touches: Vec::new(),
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

    /// Carries out `request` on `grid` and answers on `reply`: now, or once
    /// the grid has resolved a name or the simulator has said how it ended.
    /// Returns the instant message sent. `persist`, `home`, `status` and
    /// `quit` are the bot's own business and never get here.
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
        let command = match &request {
            Request::Say(_) => "say",
            Request::Shout(_) => "shout",
            Request::Whisper(_) => "whisper",
            Request::Im { .. } => "im",
            Request::Stand => "stand",
            Request::Sit(_) => "sit",
            Request::Tp(_) => "tp",
            Request::AskTp { .. } => "requesttp",
            Request::AcceptTp(_) => "accepttp",
            Request::SendTp { .. } => "sendtp",
            Request::Touch(_) => "touch",
            Request::Persist { .. } => "persist",
            Request::Home => "home",
            Request::Status => "status",
            Request::Rlv(_) => "rlv",
            Request::Quit => "quit",
            Request::Inventory(_) => "inventory",
            Request::Attach { .. } => "attach",
            Request::Wear(_) => "wear",
            Request::Detach(_) => "detach",
            Request::Anim { stop: false, .. } => "anim play",
            Request::Anim { stop: true, .. } => "anim stop",
            Request::Sound(_) => "sound",
        };
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
            Request::AcceptTp(to) => self.with_avatar(&to, After::AcceptTp, &reply, grid, now),
            Request::SendTp { to, message } => {
                check_length(&message, MAX_OFFER_TEXT).and_then(|()| self.with_avatar(&to, After::SendTp(message), &reply, grid, now))
            }
            Request::Touch(object) => self.touch(&object, &reply, grid, now),
            Request::Persist { .. } | Request::Home | Request::Status | Request::Rlv(_) | Request::Quit => {
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
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.say(chat.channel, chat.chat_type, &chat.text, now)?;
        self.last_channel = chat.channel;
        let how = match chat.chat_type {
            WHISPER => "whispered",
            SHOUT => "shouted",
            _ => "said",
        };
        crate::info!("control: {how} on channel {}: {}", chat.channel, chat.text);
        Ok(Done::Answer(None))
    }

    /// `shout` and `whisper`: the message as it is, on channel 0.
    fn say_plainly<G: Grid>(&mut self, chat_type: u8, message: &str, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        check_text(message)?;
        grid.ok_or(NOT_CONNECTED)?.say(0, chat_type, message, now)?;
        let how = if chat_type == SHOUT { "shouted" } else { "whispered" };
        crate::info!("control: {how} on channel 0: {message}");
        Ok(Done::Answer(None))
    }

    fn stand<G: Grid>(&mut self, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        grid.ok_or(NOT_CONNECTED)?.stand(now)?;
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
        let object = match target {
            SitTarget::Ground => {
                grid.sit_ground(now)?;
                self.went_manual = true;
                self.remember_sit(SitState::Ground);
                crate::info!("{}: sitting on the ground", self.origin.tag());
                return Ok(Done::Answer(None));
            }
            SitTarget::Object(object) => object,
        };
        grid.sit(object, now)?;
        self.went_manual = true;
        self.remember_sit(SitState::Object(object));
        crate::info!("{}: sitting on {object}", self.origin.tag());
        if let Some((earlier, origin)) = self.pending_sit.replace((reply.clone(), self.origin)) {
            refuse(origin.tag(), "sit", &earlier, "superseded by a newer sit command".to_string());
        }
        Ok(Done::Parked)
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
        grid.teleport(&region, [to.x, to.y, z], now)?;
        self.went_manual = true;
        crate::info!("{}: teleporting to \"{region}\" at {}, {}, {z}", self.origin.tag(), to.x, to.y);
        if self.origin == Origin::Operator {
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
        if self.origin == Origin::Operator {
            self.last_sit = Some(sit);
        }
    }

    /// A teleport for RLV to a point given in the grid's own coordinates:
    /// `position` in the region with this handle. `ctl tp` has no such
    /// form; it ends as a `tp` does, with nobody waiting for the answer.
    pub fn tp_global<G: Grid>(&mut self, handle: u64, position: Vector3, grid: Option<&mut G>, now: Instant) -> Result<(), String> {
        grid.ok_or(NOT_CONNECTED)?.teleport_to(handle, position, now)?;
        self.went_manual = true;
        let [x, y, z] = position;
        let (east, north) = (handle >> 32, handle & 0xffff_ffff);
        crate::info!("rlv: teleporting to {x}, {y}, {z} in the region at {east}, {north}");
        // Nobody reads the answer; the slot says whose teleport is under way.
        let (reply, _) = std::sync::mpsc::channel();
        self.origin = Origin::Rlv;
        self.park_teleport(&reply);
        self.origin = Origin::Operator;
        Ok(())
    }

    fn touch<G: Grid>(&mut self, object: &str, reply: &Sender<Response>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let object = Uuid::parse_str(object.trim()).map_err(|_| format!("\"{}\" is not an object's UUID", object.trim()))?;
        match grid.ok_or(NOT_CONNECTED)?.touch(object, now)? {
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
        self.start(
            grid.ok_or(NOT_CONNECTED)?,
            what,
            Then::List(args),
            "inventory",
            reply,
            now + inventory::LIMIT,
        )
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
                    if !grid.attached().iter().any(|(attached, _)| *attached == item.id) {
                        return Err(format!("{} is not attached", item.show()));
                    }
                    grid.detach(item.id, now)?;
                    crate::info!("{}: detaching {}", self.origin.tag(), item.show());
                    self.detaching.push(Detaching {
                        origin: self.origin,
                        reply: reply.clone(),
                        waiting: vec![item],
                        gone: Vec::new(),
                        stuck: Vec::new(),
                    });
                    Ok(Done::Parked)
                }
                inventory::ASSET_BODY_PART => Err(format!("{} is a body part: {}", item.show(), keeps_body_part(item.subtype()))),
                inventory::ASSET_CLOTHING => {
                    if !grid.outfit_service() {
                        return Err(inventory::NO_OUTFIT_SERVICE.to_string());
                    }
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
                let mut waiting = Vec::new();
                for id in still {
                    grid.detach(id, now)?;
                    // An item the inventory did not describe goes by its id.
                    let described = found.iter().find(|item| item.id == id).cloned();
                    waiting.push(described.unwrap_or(Item { id, ..Item::default() }));
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
            if detach.stuck.is_empty() {
                let _ = detach.reply.send(Response::Ok(Some(format!("detached {}", names(&detach.gone)))));
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
    }

    fn fail_jobs(&mut self, reason: &str) {
        for (_, pending) in std::mem::take(&mut self.jobs) {
            refuse(pending.origin.tag(), pending.command, &pending.reply, reason.to_string());
        }
        self.notes.clear();
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
            Recipient::Id(id) => return self.act(grid, id, String::new(), then, reply, now),
            Recipient::Name(name) => name,
        };
        if let Some((id, proper)) = self.known.get(&name).cloned() {
            return self.act(grid, id, proper, then, reply, now);
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
            reply: reply.clone(),
            deadline,
        });
        Ok(Done::Parked)
    }

    /// What a command does once its avatar is known to be `id`.
    fn act<G: Grid>(
        &mut self,
        grid: &mut G,
        id: Uuid,
        name: String,
        then: After,
        reply: &Sender<Response>,
        now: Instant,
    ) -> Result<Done, String> {
        let who = whom(id, &name);
        match then {
            After::Im(text) => {
                grid.instant_message(id, &text, now)?;
                Ok(Done::Sent(format!("sent to {who}"), SentIm { to: id, name, text }))
            }
            After::RequestTp(message) => {
                grid.request_teleport(id, &message, now)?;
                self.asked.insert(id, now);
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
            After::AcceptTp => {
                let Some(lure) = self.offers.get(&id).copied() else {
                    return Err(format!("there is no teleport offer from {who}"));
                };
                grid.accept_teleport(lure, now)?;
                self.offers.remove(&id);
                self.went_manual = true;
                crate::info!("control: accepting the teleport offer from {who}");
                self.park_teleport(reply);
                Ok(Done::Parked)
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
                name, given, then, reply, ..
            } = command;
            let what = then.command();
            let done = match (exact_match(&name, avatars), grid.as_deref_mut()) {
                (None, _) => Err(format!("the grid knows no avatar named \"{given}\"")),
                (Some(_), None) => Err(NOT_CONNECTED.to_string()),
                (Some((id, proper)), Some(grid)) => {
                    self.known.insert(name, (*id, proper.clone()));
                    self.act(grid, *id, proper.clone(), then, &reply, now)
                }
            };
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
        self.fail_jobs("the session was lost");
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
        self.offers.clear();
    }

    /// When `tick` next has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        let lookups = self.waiting.iter().map(|w| w.deadline);
        let jobs = self.jobs.values().map(|job| job.deadline + JOB_GRACE);
        let restore = match self.restore {
            Restore::Due(at) => Some(at),
            _ => None,
        };
        lookups.chain(jobs).chain(restore).min()
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

    /// An avatar offers the bot a teleport. One the bot asked for recently
    /// is accepted; any other is kept for `accepttp`.
    pub fn teleport_offer<G: Grid>(&mut self, from: Uuid, name: &str, lure: Uuid, grid: Option<&mut G>, now: Instant) {
        let who = whom(from, name);
        let full_name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if !full_name.is_empty() {
            // The offer says who it is from: no lookup needed to answer it.
            self.known.insert(full_name.to_lowercase(), (from, full_name));
        }
        let asked = self
            .asked
            .remove(&from)
            .is_some_and(|at| now.duration_since(at) <= self.offer_timeout);
        if asked {
            match grid
                .ok_or(NOT_CONNECTED.to_string())
                .and_then(|grid| grid.accept_teleport(lure, now))
            {
                Ok(()) => return crate::info!("accepting the teleport offer from {who}, as requested"),
                Err(reason) => crate::info!("cannot accept the teleport offer from {who} now: {reason}"),
            }
        }
        if self.offers.len() < MAX_OFFERS || self.offers.contains_key(&from) {
            self.offers.insert(from, lure);
            crate::info!("teleport offer from {who} not accepted; `rslbot ctl accepttp` accepts it");
        } else {
            crate::info!("teleport offer from {who} ignored: too many offers are waiting");
        }
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
        /// The attachments on the avatar by their root prim, with the item
        /// each was made from.
        pub(crate) worn: Vec<(Uuid, Uuid)>,
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
            }
        }
    }

    impl FakeGrid {
        fn check(&self) -> Result<(), String> {
            if self.offline {
                Err(NOT_CONNECTED.to_string())
            } else {
                Ok(())
            }
        }
    }

    impl Grid for FakeGrid {
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
        fn play_sound(&mut self, sound: Uuid, _: Instant) -> Result<(), String> {
            self.check()?;
            self.calls.push(Call::Sound(sound));
            Ok(())
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
        rig.command(Request::AcceptTp(jane().to_string()));
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
        let answer = rig.command(Request::AcceptTp("john.roe".into()));
        assert_eq!(rig.calls(), [Call::AcceptTp(lure(5))]);
        assert!(rig.commands.take_manual());
        assert!(answer.try_recv().is_err());
        rig.commands.arrived("Elsewhere");
        assert_eq!(ok(&answer), Some("arrived in Elsewhere".to_string()));

        // Used once.
        let again = rig.command(Request::AcceptTp("john.roe".into()));
        assert!(error(&again).contains("no teleport offer from John Roe"));
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
        assert!(error(&rig.command(Request::AcceptTp(jane().to_string()))).contains("no teleport offer"));
        rig.offer(jane(), "Jane Doe", lure(1));
        rig.offer(jane(), "Jane Doe", lure(2));
        let answer = rig.command(Request::AcceptTp("Jane Doe".into()));
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
        assert!(error(&rig.command(Request::AcceptTp("Jane Doe".into()))).contains("no teleport offer"));

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
        rig.command(Request::AcceptTp("Jane Doe".into()));
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
}
