//! What the bot does with a command from the control socket: the chat-bar
//! parsing of `say`, the avatars commands name and their lookup on the
//! grid, the commands that move the avatar, and the ones waiting for a
//! lookup or for the simulator to say how they ended.

use crate::config::DEFAULT_REQUESTTP_TIMEOUT;
use crate::control::{Request, Response};
use crate::session::{Touch, NOT_CONNECTED};
use crate::udp::messages::{Vector3, MAX_TEXT};
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the grid has to answer a name lookup.
pub const LOOKUP_LIMIT: Duration = Duration::from_secs(10);

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
    let chat = |channel, chat_type, text: &str| Chat { channel, chat_type, text: text.to_string() };
    let digits = message.strip_prefix('/').map_or(0, |rest| rest.bytes().take_while(u8::is_ascii_digit).count());
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
    Ok(Destination { x, y, z, region: (!region.is_empty()).then_some(region) })
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
        _ => Err(format!("\"{given}\" is not an avatar name (`First Last`, `first.last` or one word) or a UUID")),
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
    /// The region the avatar is in.
    fn region(&self) -> Option<String>;
    /// Where the simulator last said the avatar is.
    fn position(&self) -> Option<Vector3>;
    fn request_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String>;
    fn offer_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String>;
    /// Ends like `teleport`.
    fn accept_teleport(&mut self, lure: Uuid, now: Instant) -> Result<(), String>;
    fn touch(&mut self, object: Uuid, now: Instant) -> Result<Touch, String>;
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
    pending_sit: Option<Sender<Response>>,
    /// The `tp` or `accepttp` command waiting for the teleport to end.
    pending_teleport: Option<Sender<Response>>,
    /// `touch` commands waiting for their object to turn up.
    touches: Vec<(Uuid, Sender<Response>)>,
    /// A command moved the avatar since `take_manual` was last asked.
    went_manual: bool,
}

impl Default for Commands {
    fn default() -> Commands {
        Commands::new(DEFAULT_REQUESTTP_TIMEOUT)
    }
}

fn refuse(what: &str, reply: &Sender<Response>, reason: String) {
    crate::info!("control: {what} refused: {reason}");
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
            went_manual: false,
        }
    }

    /// Carries out `request` on `grid` and answers on `reply`: now, or once
    /// the grid has resolved a name or the simulator has said how it ended.
    /// Returns the instant message sent. `home`, `status` and `quit` are the
    /// bot's own business and never get here.
    pub fn handle<G: Grid>(
        &mut self,
        request: Request,
        reply: Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Option<SentIm> {
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
            Request::Home => "home",
            Request::Status => "status",
            Request::Quit => "quit",
        };
        let done = match request {
            Request::Say(message) => self.say(&message, grid, now),
            Request::Shout(message) => self.say_plainly(SHOUT, &message, grid, now),
            Request::Whisper(message) => self.say_plainly(WHISPER, &message, grid, now),
            Request::Im { to, text } => {
                check_text(&text).and_then(|()| self.with_avatar(&to, After::Im(text), &reply, grid, now))
            }
            Request::Stand => self.stand(grid, now),
            Request::Sit(target) => self.sit(&target, &reply, grid, now),
            Request::Tp(args) => self.tp(&args, &reply, grid, now),
            Request::AskTp { to, message } => check_length(&message, MAX_TEXT)
                .and_then(|()| self.with_avatar(&to, After::RequestTp(message), &reply, grid, now)),
            Request::AcceptTp(to) => self.with_avatar(&to, After::AcceptTp, &reply, grid, now),
            Request::SendTp { to, message } => check_length(&message, MAX_OFFER_TEXT)
                .and_then(|()| self.with_avatar(&to, After::SendTp(message), &reply, grid, now)),
            Request::Touch(object) => self.touch(&object, &reply, grid, now),
            Request::Home | Request::Status | Request::Quit => Err("not a command for the grid".to_string()),
        };
        finish(command, done, &reply)
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
    fn say_plainly<G: Grid>(
        &mut self,
        chat_type: u8,
        message: &str,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        check_text(message)?;
        grid.ok_or(NOT_CONNECTED)?.say(0, chat_type, message, now)?;
        let how = if chat_type == SHOUT { "shouted" } else { "whispered" };
        crate::info!("control: {how} on channel 0: {message}");
        Ok(Done::Answer(None))
    }

    fn stand<G: Grid>(&mut self, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        grid.ok_or(NOT_CONNECTED)?.stand(now)?;
        self.went_manual = true;
        crate::info!("control: standing up");
        if let Some(sit) = self.pending_sit.take() {
            refuse("sit", &sit, "the avatar was told to stand up".to_string());
        }
        Ok(Done::Answer(None))
    }

    fn sit<G: Grid>(
        &mut self,
        target: &str,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let target = parse_sit(target)?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let object = match target {
            SitTarget::Ground => {
                grid.sit_ground(now)?;
                self.went_manual = true;
                crate::info!("control: sitting on the ground");
                return Ok(Done::Answer(None));
            }
            SitTarget::Object(object) => object,
        };
        grid.sit(object, now)?;
        self.went_manual = true;
        crate::info!("control: sitting on {object}");
        if let Some(earlier) = self.pending_sit.replace(reply.clone()) {
            refuse("sit", &earlier, "superseded by a newer sit command".to_string());
        }
        Ok(Done::Parked)
    }

    fn tp<G: Grid>(
        &mut self,
        args: &[String],
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
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
        crate::info!("control: teleporting to \"{region}\" at {}, {}, {z}", to.x, to.y);
        self.park_teleport(reply);
        Ok(Done::Parked)
    }

    fn park_teleport(&mut self, reply: &Sender<Response>) {
        if let Some(earlier) = self.pending_teleport.replace(reply.clone()) {
            refuse("tp", &earlier, "superseded by a newer teleport".to_string());
        }
    }

    fn touch<G: Grid>(
        &mut self,
        object: &str,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let object = Uuid::parse_str(object.trim())
            .map_err(|_| format!("\"{}\" is not an object's UUID", object.trim()))?;
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
        self.waiting.push(Waiting { query, name, given, then, reply: reply.clone(), deadline });
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
            let Waiting { name, given, then, reply, .. } = command;
            let what = then.command();
            let done = match (exact_match(&name, avatars), grid.as_deref_mut()) {
                (None, _) => Err(format!("the grid knows no avatar named \"{given}\"")),
                (Some(_), None) => Err(NOT_CONNECTED.to_string()),
                (Some((id, proper)), Some(grid)) => {
                    self.known.insert(name, (*id, proper.clone()));
                    self.act(grid, *id, proper.clone(), then, &reply, now)
                }
            };
            sent.extend(finish(what, done, &reply));
        }
        sent
    }

    /// A lookup came to nothing: fails the commands that waited for it.
    pub fn lookup_failed(&mut self, query: Uuid, reason: &str) {
        let (failed, waiting) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.query == query);
        self.waiting = waiting;
        for command in failed {
            let Waiting { given, then, reply, .. } = command;
            refuse(then.command(), &reply, format!("the lookup of \"{given}\" failed: {reason}"));
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
            refuse(then.command(), &reply, reason);
        }
    }

    /// The session is gone, and with it any answer the commands waited for
    /// and every teleport offer made to it.
    pub fn session_lost(&mut self) {
        for command in std::mem::take(&mut self.waiting) {
            refuse(command.then.command(), &command.reply, NOT_CONNECTED.to_string());
        }
        if let Some(reply) = self.pending_sit.take() {
            refuse("sit", &reply, "the session was lost".to_string());
        }
        if let Some(reply) = self.pending_teleport.take() {
            refuse("tp", &reply, "the session was lost".to_string());
        }
        for (_, reply) in std::mem::take(&mut self.touches) {
            refuse("touch", &reply, "the session was lost".to_string());
        }
        self.offers.clear();
    }

    /// When `tick` next has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.waiting.iter().map(|w| w.deadline).min()
    }

    // ------------------------------------------- what the session reports

    /// The avatar is seated on `object`.
    pub fn seated(&mut self, object: Uuid) {
        if let Some(reply) = self.pending_sit.take() {
            let _ = reply.send(Response::Ok(Some(format!("seated on {object}"))));
        }
    }

    /// A sit came to nothing. Returns whether a command was waiting for it.
    pub fn sit_failed(&mut self, reason: &str) -> bool {
        match self.pending_sit.take() {
            Some(reply) => {
                refuse("sit", &reply, reason.to_string());
                true
            }
            None => false,
        }
    }

    /// A teleport ended with the avatar in `region`.
    pub fn arrived(&mut self, region: &str) {
        if let Some(reply) = self.pending_teleport.take() {
            let _ = reply.send(Response::Ok(Some(format!("arrived in {region}"))));
        }
    }

    /// A teleport came to nothing. Returns whether a command was waiting
    /// for it.
    pub fn teleport_failed(&mut self, reason: &str) -> bool {
        match self.pending_teleport.take() {
            Some(reply) => {
                refuse("tp", &reply, reason.to_string());
                true
            }
            None => false,
        }
    }

    /// A touch that waited for its object went out.
    pub fn touched(&mut self, object: Uuid) {
        let (done, touches) = std::mem::take(&mut self.touches).into_iter().partition(|(wanted, _)| *wanted == object);
        self.touches = touches;
        for (_, reply) in done {
            crate::info!("control: touched {object}");
            let _ = reply.send(Response::Ok(None));
        }
    }

    /// A touch gave up waiting for its object.
    pub fn touch_failed(&mut self, object: Uuid, reason: &str) {
        let (failed, touches) =
            std::mem::take(&mut self.touches).into_iter().partition(|(wanted, _)| *wanted == object);
        self.touches = touches;
        for (_, reply) in failed {
            refuse("touch", &reply, reason.to_string());
        }
    }

    /// An avatar offers the bot a teleport. One the bot asked for recently
    /// is accepted; any other is kept for `accepttp`.
    pub fn teleport_offer<G: Grid>(
        &mut self,
        from: Uuid,
        name: &str,
        lure: Uuid,
        grid: Option<&mut G>,
        now: Instant,
    ) {
        let who = whom(from, name);
        let full_name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if !full_name.is_empty() {
            // The offer says who it is from: no lookup needed to answer it.
            self.known.insert(full_name.to_lowercase(), (from, full_name));
        }
        let asked = self.asked.remove(&from).is_some_and(|at| now.duration_since(at) <= self.offer_timeout);
        if asked {
            match grid.ok_or(NOT_CONNECTED.to_string()).and_then(|grid| grid.accept_teleport(lure, now)) {
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

fn check_length(text: &str, limit: usize) -> Result<(), String> {
    if text.len() > limit {
        return Err(format!("the text is {} bytes long; the limit is {limit}", text.len()));
    }
    Ok(())
}

/// Answers a command that has come to an end, and hands back the instant
/// message it sent, if it did.
fn finish(command: &str, done: Result<Done, String>, reply: &Sender<Response>) -> Option<SentIm> {
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
            refuse(command, reply, reason);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    fn chat(channel: i32, chat_type: u8, text: &str) -> Result<Chat, String> {
        Ok(Chat { channel, chat_type, text: text.to_string() })
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
    enum Call {
        Say(i32, u8, String),
        Im(Uuid, String),
        Find(Uuid, String),
        Stand,
        SitGround,
        Sit(Uuid),
        Teleport(String, Vector3),
        RequestTp(Uuid, String),
        OfferTp(Uuid, String),
        AcceptTp(Uuid),
        Touch(Uuid),
    }

    struct FakeGrid {
        calls: Vec<Call>,
        offline: bool,
        position: Option<Vector3>,
        /// What `touch` answers.
        touch: Result<Touch, String>,
    }

    impl Default for FakeGrid {
        fn default() -> FakeGrid {
            let position = Some([10.0, 20.0, 30.0]);
            FakeGrid { calls: Vec::new(), offline: false, position, touch: Ok(Touch::Sent) }
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
        vec![(Uuid::from_bytes([2; 16]), "Jane Doester".to_string()), (jane(), "Jane Doe".to_string())]
    }

    fn sent_to_jane(text: &str) -> SentIm {
        SentIm { to: jane(), name: "Jane Doe".to_string(), text: text.to_string() }
    }

    fn error(answer: &Receiver<Response>) -> String {
        match answer.try_recv() {
            Ok(Response::Error(reason)) => reason,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    impl Rig {
        fn new() -> Rig {
            Rig { commands: Commands::default(), grid: FakeGrid::default(), now: Instant::now() }
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
            self.run(Request::Im { to: to.to_string(), text: text.to_string() })
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
        let sent = rig.commands.handle(Request::Say("hello".into()), reply, None::<&mut FakeGrid>, rig.now);
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
        assert_eq!(sent, Some(SentIm { to: jane(), name: String::new(), text: "hello".into() }));
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
        assert_eq!(rig.found(query, &found), [SentIm { to: id, name: "sitbot Resident".into(), text: "hi".into() }]);
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
        Ok(Destination { x, y, z, region: region.map(str::to_string) })
    }

    #[test]
    fn tp_reads_coordinates_then_an_optional_height_and_region() {
        assert_eq!(destination(&["100", "120"]), to(100.0, 120.0, None, None));
        assert_eq!(destination(&["100.5", "120", "40"]), to(100.5, 120.0, Some(40.0), None));
        assert_eq!(destination(&["128", "128", "25", "Home"]), to(128.0, 128.0, Some(25.0), Some("Home")));
        assert_eq!(destination(&["128", "128", "Blue", "Lagoon"]), to(128.0, 128.0, None, Some("Blue Lagoon")));
        assert_eq!(destination(&["128", "128", "25", "Blue", "Lagoon"]), to(128.0, 128.0, Some(25.0), Some("Blue Lagoon")));
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
            Request::AskTp { to: jane().to_string(), message: String::new() },
            Request::SendTp { to: jane().to_string(), message: String::new() },
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
        let answer = rig.command(Request::SendTp { to: "Jane Doe".into(), message: "come and see".into() });
        let query = rig.lookup("jane doe");
        assert_eq!(rig.found(query, &jane_found()), []);
        assert_eq!(ok(&answer), Some(format!("offered Jane Doe <{}> a teleport", jane())));
        // Without a message, the one viewers send.
        rig.command(Request::SendTp { to: "jane.doe".into(), message: String::new() });
        assert_eq!(
            rig.calls(),
            [Call::OfferTp(jane(), "come and see".into()), Call::OfferTp(jane(), "Join me in Home".into())]
        );
        assert!(!rig.commands.take_manual());
        let long = Request::SendTp { to: "jane.doe".into(), message: "x".repeat(255) };
        assert!(error(&rig.command(long)).contains("254"));
    }

    // ---------------------------------------------------- teleport offers

    fn lure(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    #[test]
    fn requested_teleport_offer_is_accepted_once() {
        let mut rig = Rig::new();
        let answer = rig.command(Request::AskTp { to: jane().to_string(), message: "may I?".into() });
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
        let answer = rig.command(Request::AskTp { to: "Jane Doe".into(), message: String::new() });
        let query = rig.lookup("jane doe");
        assert!(!rig.commands.take_manual(), "nothing has been asked yet");
        rig.found(query, &jane_found());
        assert_eq!(ok(&answer), Some(format!("asked Jane Doe <{}> for a teleport", jane())));
        assert!(rig.commands.take_manual());
        assert_eq!(rig.calls(), [Call::RequestTp(jane(), String::new())]);

        let answer = rig.command(Request::AskTp { to: "Nobody Here".into(), message: String::new() });
        let query = rig.lookup("nobody here");
        rig.found(query, &jane_found());
        assert!(error(&answer).contains("\"Nobody Here\""));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn late_and_unsolicited_offers_wait_for_accepttp() {
        let mut rig = Rig::new();
        rig.command(Request::AskTp { to: jane().to_string(), message: String::new() });
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
        rig.command(Request::AskTp { to: jane().to_string(), message: String::new() });
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
        rig.command(Request::AskTp { to: jane().to_string(), message: String::new() });
        rig.calls();
        rig.commands.forget_requests();
        rig.offer(jane(), "Jane Doe", lure(3));
        assert_eq!(rig.calls(), []);
    }

    #[test]
    fn an_offer_that_cannot_be_accepted_now_is_kept() {
        let mut rig = Rig::new();
        rig.command(Request::AskTp { to: jane().to_string(), message: String::new() });
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
}
