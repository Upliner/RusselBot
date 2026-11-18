//! What the bot does with a command from the control socket: the chat-bar
//! parsing of `say`, the recipient of `im` and its lookup on the grid, and
//! the commands waiting for that lookup.

use crate::control::{Request, Response};
use crate::session::NOT_CONNECTED;
use crate::udp::messages::MAX_TEXT;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the grid has to answer a name lookup.
pub const LOOKUP_LIMIT: Duration = Duration::from_secs(10);

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

/// The part of a session the commands use.
pub trait Grid {
    fn say(&mut self, channel: i32, chat_type: u8, text: &str, now: Instant) -> Result<(), String>;
    fn instant_message(&mut self, to: Uuid, text: &str, now: Instant) -> Result<(), String>;
    fn find_avatar(&mut self, query: Uuid, name: &str, now: Instant) -> Result<(), String>;
}

/// An instant message that went out, for the message log.
#[derive(Debug, Clone, PartialEq)]
pub struct SentIm {
    pub to: Uuid,
    /// `First Last`; empty when the recipient was given by UUID.
    pub name: String,
    pub text: String,
}

/// An `im` command waiting for its recipient's name to be resolved.
struct Waiting {
    query: Uuid,
    /// The full name in lower case.
    name: String,
    /// The recipient as the command gave it.
    given: String,
    text: String,
    reply: Sender<Response>,
    deadline: Instant,
}

/// Carries out commands. Lives as long as the bot, across sessions.
#[derive(Default)]
pub struct Commands {
    /// The channel `//` stands for.
    last_channel: i32,
    /// Resolved names: lower-case full name to id and the name as the grid
    /// spells it.
    known: HashMap<String, (Uuid, String)>,
    waiting: Vec<Waiting>,
    queries: u128,
}

fn refuse(what: &str, reply: &Sender<Response>, reason: String) {
    crate::info!("control: {what} refused: {reason}");
    // The client may have gone away; the command is settled either way.
    let _ = reply.send(Response::Error(reason));
}

impl Commands {
    /// Carries out `request` on `grid` and answers on `reply`, now or once
    /// the grid has resolved a name. Returns the instant message sent.
    pub fn handle<G: Grid>(
        &mut self,
        request: Request,
        reply: Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Option<SentIm> {
        match request {
            Request::Say(message) => {
                if let Err(reason) = self.say(&message, grid, now) {
                    refuse("say", &reply, reason);
                } else {
                    let _ = reply.send(Response::Ok(None));
                }
                None
            }
            Request::Im { to, text } => match self.im(&to, text, &reply, grid, now) {
                Ok(sent) => sent,
                Err(reason) => {
                    refuse("im", &reply, reason);
                    None
                }
            },
        }
    }

    fn say<G: Grid>(&mut self, message: &str, grid: Option<&mut G>, now: Instant) -> Result<(), String> {
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
        Ok(())
    }

    /// Sends the message if the recipient is known, and otherwise starts
    /// (or joins) a lookup and parks the command.
    fn im<G: Grid>(
        &mut self,
        to: &str,
        text: String,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Option<SentIm>, String> {
        check_text(&text)?;
        let recipient = parse_recipient(to)?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        let name = match recipient {
            Recipient::Id(id) => return deliver(grid, id, String::new(), text, reply, now).map(Some),
            Recipient::Name(name) => name,
        };
        if let Some((id, proper)) = self.known.get(&name).cloned() {
            return deliver(grid, id, proper, text, reply, now).map(Some);
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
        self.waiting.push(Waiting { query, name, given: to.trim().to_string(), text, reply: reply.clone(), deadline });
        Ok(None)
    }

    /// The grid answered a lookup: sends the messages that waited for it.
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
            let Waiting { name, given, text, reply, .. } = command;
            let outcome = match (exact_match(&name, avatars), grid.as_deref_mut()) {
                (None, _) => Err(format!("the grid knows no avatar named \"{given}\"")),
                (Some(_), None) => Err(NOT_CONNECTED.to_string()),
                (Some((id, proper)), Some(grid)) => {
                    self.known.insert(name, (*id, proper.clone()));
                    deliver(grid, *id, proper.clone(), text, &reply, now)
                }
            };
            match outcome {
                Ok(im) => sent.push(im),
                Err(reason) => refuse("im", &reply, reason),
            }
        }
        sent
    }

    /// A lookup came to nothing: fails the commands that waited for it.
    pub fn lookup_failed(&mut self, query: Uuid, reason: &str) {
        let (failed, waiting) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.query == query);
        self.waiting = waiting;
        for command in failed {
            let Waiting { given, reply, .. } = command;
            refuse("im", &reply, format!("the lookup of \"{given}\" failed: {reason}"));
        }
    }

    /// Fails the commands whose lookup has gone unanswered for too long.
    pub fn tick(&mut self, now: Instant) {
        let (late, waiting) = std::mem::take(&mut self.waiting).into_iter().partition(|w| now >= w.deadline);
        self.waiting = waiting;
        for command in late {
            let Waiting { given, reply, .. } = command;
            let limit = LOOKUP_LIMIT.as_secs();
            refuse("im", &reply, format!("the grid did not answer the lookup of \"{given}\" within {limit} s"));
        }
    }

    /// The session is gone, and with it any answer to a lookup.
    pub fn session_lost(&mut self) {
        for command in std::mem::take(&mut self.waiting) {
            refuse("im", &command.reply, NOT_CONNECTED.to_string());
        }
    }

    /// When `tick` next has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.waiting.iter().map(|w| w.deadline).min()
    }
}

/// Sends the instant message and answers the command.
fn deliver<G: Grid>(
    grid: &mut G,
    to: Uuid,
    name: String,
    text: String,
    reply: &Sender<Response>,
    now: Instant,
) -> Result<SentIm, String> {
    grid.instant_message(to, &text, now)?;
    let whom = if name.is_empty() { to.to_string() } else { format!("{name} <{to}>") };
    let _ = reply.send(Response::Ok(Some(format!("sent to {whom}"))));
    Ok(SentIm { to, name, text })
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
    }

    #[derive(Default)]
    struct FakeGrid {
        calls: Vec<Call>,
        offline: bool,
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
}
