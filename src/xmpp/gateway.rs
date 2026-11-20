//! What the main loop keeps of the gateway: the connection, the owners,
//! the addresses used, the presence sent and the messages that wait. It
//! turns owners' stanzas into control requests, recorded messages into
//! stanzas, and answers into messages or errors.

use super::address::{self, AddressBook, Contact, Recipient};
use super::chat;
use super::command;
use super::conn::{self, Settings, Stop, Writer, XmppEvent};
use super::presence::{self, Presence};
use super::queue::{Outgoing, WaitQueue};
use super::stanza::{self, Condition, Shown, Stanza, NS_DISCO_INFO, NS_PING};
use super::stream::Element;
use crate::config::XmppConfig;
use crate::control::{request_line, Request, Response};
use crate::msglog::{Channel, Record};
use crate::net::Event;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long an owner's request waits for the bot's answer, as a control
/// connection does.
const ANSWER_LIMIT: Duration = Duration::from_secs(75);
/// At most this many requests wait for their answers at once.
pub const MAX_WAITING: usize = 16;
/// How long the writer has to write the last stanzas when the bot stops.
const CLOSE_LIMIT: Duration = Duration::from_secs(2);

const FORBIDDEN: &str = "only the owners of this bot may use it";
const OBJECTS_CANNOT: &str = "objects cannot receive instant messages";

fn epoch_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where the answer to an owner's request goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    /// The owner's full address, as the request came from it.
    owner: String,
    /// The address of the component the request was sent to.
    contact: String,
    id: Option<String>,
    /// The control contact answers every command with a message; the others
    /// answer only an error, with an error stanza.
    control: bool,
}

/// The bot's answer to an owner's request.
#[derive(Debug)]
pub struct Answer {
    pending: Pending,
    response: Response,
}

/// An owner's request for the bot, to be carried out as the control
/// socket's are, with `reply` for its answer; `waiting` then goes to
/// `Gateway::waited`.
pub struct Command {
    pub request: Request,
    pub reply: Sender<Response>,
    pub waiting: Waiting,
}

/// Where the answer to a request handed to the bot is awaited.
pub struct Waiting {
    answer: Receiver<Response>,
    pending: Pending,
}

/// The stanza error condition for the error text of `im` or `say`.
pub fn condition(text: &str) -> Condition {
    if text.contains("knows no avatar") || text.contains("is not an avatar name") {
        Condition::ItemNotFound
    } else if text.contains("there is no text") || text.contains("bytes long; the limit") {
        Condition::NotAcceptable
    } else {
        Condition::ServiceUnavailable
    }
}

pub struct Gateway {
    domain: String,
    owners: Vec<String>,
    events: Sender<Event>,
    /// The connection the main loop uses, by its generation.
    connection: Option<(u64, Writer)>,
    queue: WaitQueue,
    book: AddressBook,
    /// The presence last sent to the owners on this connection.
    sent: Option<Presence>,
    /// Requests waiting for their answers.
    waiting: Arc<AtomicUsize>,
    stop: Option<Stop>,
}

impl Gateway {
    /// Starts connecting the component.
    pub fn start(config: &XmppConfig, events: Sender<Event>) -> Gateway {
        Gateway::start_with(config, events, Settings::default())
    }

    pub fn start_with(config: &XmppConfig, events: Sender<Event>, settings: Settings) -> Gateway {
        let stop = conn::spawn(
            config.component.clone(),
            config.server.clone(),
            config.secret.clone(),
            settings,
            events.clone(),
        );
        Gateway {
            domain: config.component.clone(),
            owners: config.owners.clone(),
            events,
            connection: None,
            queue: WaitQueue::default(),
            book: AddressBook::default(),
            sent: None,
            waiting: Arc::new(AtomicUsize::new(0)),
            stop: Some(stop),
        }
    }

    fn address(&self, local: Option<&str>) -> String {
        match local {
            Some(local) => format!("{local}@{}", self.domain),
            None => self.domain.clone(),
        }
    }

    #[cfg(test)]
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Writes a stanza now, or not at all.
    fn write(&mut self, stanza: String) -> bool {
        let Some((_, writer)) = &self.connection else {
            return false;
        };
        if writer.try_send(stanza).is_ok() {
            return true;
        }
        crate::info!("xmpp: the server does not take what the component writes; giving the connection up");
        if let Some((_, writer)) = self.connection.take() {
            writer.give_up();
        }
        self.sent = None;
        false
    }

    /// Sends a message for the owners now, or keeps it until the component
    /// is connected.
    fn send(&mut self, outgoing: Outgoing) {
        if self.connection.is_some() {
            let stanzas = outgoing.stanzas(&self.owners, false);
            if stanzas.into_iter().all(|stanza| self.write(stanza)) {
                return;
            }
        }
        self.queue.push(outgoing);
    }

    /// Sends what waited, oldest first, each with its time.
    fn drain(&mut self) {
        while let Some(outgoing) = self.queue.pop() {
            let stanzas = outgoing.stanzas(&self.owners, true);
            if !stanzas.into_iter().all(|stanza| self.write(stanza)) {
                self.queue.push_front(outgoing);
                return;
            }
        }
    }

    /// Sends the owners what changed of the presence of the bare domain
    /// and `local@`.
    pub fn update_presence(&mut self, wanted: &Presence) {
        if self.connection.is_none() {
            return;
        }
        for (local, shown) in presence::changes(self.sent.as_ref(), wanted) {
            for owner in self.owners.clone() {
                let from = self.address(local);
                if !self.write(stanza::presence(&from, &owner, &shown)) {
                    return;
                }
            }
        }
        self.sent = Some(wanted.clone());
    }

    /// Hands a message written to the logs to the owners.
    pub fn record(&mut self, record: &Record, now: Instant) {
        if record.channel == Channel::Im {
            if let Some(id) = record.avatar_uuid {
                self.book.learn(&address::avatar_local(&record.avatar_name, id), id, now);
            }
        }
        let Some(delivery) = chat::delivery(record) else {
            return;
        };
        let from = self.address(delivery.from.as_deref());
        self.send(Outgoing::Chat {
            from,
            to: None,
            body: delivery.body,
            nick: delivery.nick,
            time: record.time,
            offline: record.offline,
        });
    }

    /// Deals with what the connection reports. `wanted` is the presence
    /// for the bot's state now. A request from an owner is returned, to be
    /// carried out and then passed to `waited`.
    pub fn handle(&mut self, event: XmppEvent, wanted: &Presence) -> Option<Command> {
        match event {
            XmppEvent::Connected { generation, writer } => {
                if self.stop.is_none() {
                    writer.close(CLOSE_LIMIT);
                    return None;
                }
                if let Some((_, old)) = self.connection.replace((generation, writer)) {
                    old.give_up();
                }
                self.sent = None;
                let dropped = self.queue.take_dropped();
                if dropped > 0 {
                    crate::info!("xmpp: {dropped} messages for the owners were dropped while the component was not connected");
                }
                self.update_presence(wanted);
                self.drain();
                None
            }
            XmppEvent::Disconnected { generation, reason } => {
                if self.connection.as_ref().is_some_and(|(current, _)| *current == generation) {
                    crate::debug!("xmpp: messages for the owners wait from now on ({reason})");
                    self.connection = None;
                    self.sent = None;
                }
                None
            }
            XmppEvent::Stanza { generation, element } => {
                if self.connection.as_ref().is_some_and(|(current, _)| *current == generation) {
                    self.stanza(&element, wanted)
                } else {
                    None
                }
            }
            XmppEvent::Answer(Answer { pending, response }) => {
                self.answer(&pending, response);
                None
            }
        }
    }

    fn stanza(&mut self, element: &Element, wanted: &Presence) -> Option<Command> {
        match stanza::classify(element) {
            Stanza::Error | Stanza::Other(_) => None,
            Stanza::Iq {
                kind,
                id,
                from: Some(from),
                to,
                child,
            } if kind == "get" || kind == "set" => {
                let id = id.unwrap_or_default();
                let contact = address::contact(&to, &self.domain);
                let answer = match (child.as_ref().map(|(ns, name)| (ns.as_str(), name.as_str())), &contact) {
                    (Some((NS_DISCO_INFO, "query")), Some(Contact::Control)) if kind == "get" => stanza::disco_info_result(&to, &from, &id),
                    (Some((NS_PING, "ping")), Some(_)) if kind == "get" => stanza::iq_result(&to, &from, &id),
                    _ => stanza::error_reply("iq", &to, &from, Some(&id), Condition::ServiceUnavailable, ""),
                };
                self.write(answer);
                None
            }
            Stanza::Iq { .. } => None,
            Stanza::Presence { kind, from, to } => {
                self.presence_stanza(kind.as_deref(), from.as_deref(), &to, wanted);
                None
            }
            Stanza::Message { kind, from, to, id, body } => {
                if kind == "groupchat" || kind == "headline" {
                    return None;
                }
                let from = from?;
                if !address::is_owner(&self.owners, Some(&from)) {
                    crate::info!("xmpp: refused a message from {from} to {to}: not an owner");
                    self.write(stanza::error_reply(
                        "message",
                        &to,
                        &from,
                        id.as_deref(),
                        Condition::Forbidden,
                        FORBIDDEN,
                    ));
                    return None;
                }
                let contact = address::contact(&to, &self.domain)?;
                // Chat states and the like.
                let body = body?;
                let bare_to = to.split_once('/').map_or(to.as_str(), |(bare, _)| bare).to_lowercase();
                let pending = Pending {
                    owner: from,
                    contact: bare_to,
                    id,
                    control: contact == Contact::Control,
                };
                self.message(contact, body, pending)
            }
        }
    }

    fn presence_stanza(&mut self, kind: Option<&str>, from: Option<&str>, to: &str, wanted: &Presence) {
        let Some(from) = from else {
            return;
        };
        let owner = address::is_owner(&self.owners, Some(from));
        let Some(contact) = address::contact(to, &self.domain) else {
            return;
        };
        let bare_to = to.split_once('/').map_or(to, |(bare, _)| bare).to_lowercase();
        let shown = match contact {
            Contact::Control => wanted.domain.clone(),
            Contact::Local => wanted.local.clone(),
            Contact::Avatar(_) | Contact::Object => Shown::Unavailable,
        };
        match (kind, owner) {
            (Some("subscribe"), true) => {
                crate::info!("xmpp: {from} subscribed to {bare_to}");
                self.write(stanza::subscription(&bare_to, from, true));
                self.write(stanza::presence(&bare_to, from, &shown));
            }
            (Some("subscribe"), false) => {
                crate::info!("xmpp: refused a subscription of {from} to {bare_to}: not an owner");
                self.write(stanza::subscription(&bare_to, from, false));
            }
            (Some("unsubscribe"), true) => {
                self.write(stanza::subscription(&bare_to, from, false));
            }
            (Some("probe"), true) => {
                self.write(stanza::presence(&bare_to, from, &shown));
            }
            _ => {}
        }
    }

    /// An owner's message with a body, to `contact`.
    fn message(&mut self, contact: Contact, body: String, pending: Pending) -> Option<Command> {
        let busy = self.waiting.load(Ordering::SeqCst) >= MAX_WAITING;
        let request = match contact {
            Contact::Control => {
                let args = match command::split(&body) {
                    Ok(args) => args,
                    Err(reason) => return self.refuse(&pending, reason),
                };
                match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
                    [] => return self.refuse(&pending, "there is no command; send `help` for the list".to_string()),
                    ["help"] => {
                        self.answer(&pending, Response::Ok(Some(command::help())));
                        return None;
                    }
                    _ => {}
                }
                match Request::decode(request_line(&args).as_bytes()) {
                    Ok(request) => request,
                    Err(reason) => {
                        crate::info!("xmpp: bad command from {}: {reason}", pending.owner);
                        return self.refuse(&pending, reason);
                    }
                }
            }
            Contact::Local => Request::Say(body),
            Contact::Avatar(local) => {
                let to = match self.book.recipient(&local) {
                    Recipient::Id(id) => id.to_string(),
                    Recipient::Name(name) => name,
                };
                Request::Im { to, text: body }
            }
            Contact::Object => {
                let error = stanza::error_reply(
                    "message",
                    &pending.contact,
                    &pending.owner,
                    pending.id.as_deref(),
                    Condition::FeatureNotImplemented,
                    OBJECTS_CANNOT,
                );
                self.send(Outgoing::Raw(error));
                return None;
            }
        };
        if busy {
            let reason = format!("the bot is busy: {MAX_WAITING} commands wait for their answers; try again");
            return self.refuse(&pending, reason);
        }
        let (reply, answer) = mpsc::channel();
        Some(Command {
            request,
            reply,
            waiting: Waiting { answer, pending },
        })
    }

    fn refuse(&mut self, pending: &Pending, reason: String) -> Option<Command> {
        self.answer(pending, Response::Error(reason));
        None
    }

    /// The request of `command` has been handed to the bot: its answer goes
    /// to the owner now if there is one, or else once it comes.
    pub fn waited(&mut self, waiting: Waiting) {
        let Waiting { answer, pending } = waiting;
        match answer.try_recv() {
            Ok(response) => self.answer(&pending, response),
            Err(mpsc::TryRecvError::Disconnected) => self.answer(&pending, Response::Error("the bot did not answer".to_string())),
            Err(mpsc::TryRecvError::Empty) => {
                let (events, waiting) = (self.events.clone(), self.waiting.clone());
                waiting.fetch_add(1, Ordering::SeqCst);
                thread::spawn(move || {
                    let response = answer
                        .recv_timeout(ANSWER_LIMIT)
                        .unwrap_or_else(|_| Response::Error("the bot did not answer".to_string()));
                    waiting.fetch_sub(1, Ordering::SeqCst);
                    let _ = events.send(Event::Xmpp(XmppEvent::Answer(Answer { pending, response })));
                });
            }
        }
    }

    fn answer(&mut self, pending: &Pending, response: Response) {
        if pending.control {
            let body = match response {
                Response::Ok(None) => "ok".to_string(),
                Response::Ok(Some(text)) => text,
                Response::Error(text) => format!("error: {text}"),
            };
            self.send(Outgoing::Chat {
                from: self.domain.clone(),
                to: Some(pending.owner.clone()),
                body,
                nick: None,
                time: epoch_now(),
                offline: false,
            });
        } else if let Response::Error(text) = response {
            let error = stanza::error_reply(
                "message",
                &pending.contact,
                &pending.owner,
                pending.id.as_deref(),
                condition(&text),
                &text,
            );
            self.send(Outgoing::Raw(error));
        }
    }

    /// The bot stops: the owners see its addresses go offline, and the
    /// stream is closed.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.stop();
        }
        // Both addresses, unless both have gone offline already.
        let stopping = presence::presence(None, true);
        if self.sent.as_ref() != Some(&stopping) {
            self.sent = None;
            self.update_presence(&stopping);
        }
        if let Some((_, writer)) = self.connection.take() {
            writer.close(CLOSE_LIMIT);
        }
    }

    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;
    use crate::xmpp::conn::tests::{Fake, SECRET};

    const OWNER: &str = "owner@rig.test";

    fn connected() -> (Gateway, Receiver<Event>, Fake) {
        let mut fake = Fake::new();
        let config = XmppConfig {
            component: "bot.rig.test".into(),
            server: fake.address(),
            secret: Secret::new(SECRET),
            owners: vec![OWNER.into()],
        };
        let (events, incoming) = mpsc::channel();
        let settings = Settings {
            first_delay: Duration::from_millis(50),
            ..Settings::default()
        };
        let mut gateway = Gateway::start_with(&config, events, settings);
        fake.accept();
        let home = presence::presence(Some("Home"), false);
        next(&mut gateway, &incoming, &home);
        assert!(gateway.is_connected());
        fake.expect("from='local@bot.rig.test' to='owner@rig.test'/>");
        (gateway, incoming, fake)
    }

    fn next(gateway: &mut Gateway, incoming: &Receiver<Event>, wanted: &Presence) -> Option<Command> {
        match incoming.recv_timeout(Duration::from_secs(5)).expect("an event") {
            Event::Xmpp(event) => gateway.handle(event, wanted),
            _ => panic!("not from the gateway"),
        }
    }

    #[test]
    fn adding_the_bot() {
        let (mut gateway, incoming, mut fake) = connected();
        let home = presence::presence(Some("Home"), false);
        fake.send("<presence type='subscribe' from='owner@rig.test' to='bot.rig.test'/>");
        assert!(next(&mut gateway, &incoming, &home).is_none());
        assert_eq!(
            fake.expect("/>"),
            "<presence type='subscribed' from='bot.rig.test' to='owner@rig.test'/>"
        );
        assert_eq!(
            fake.expect("</presence>"),
            "<presence from='bot.rig.test' to='owner@rig.test'><status>Home</status></presence>"
        );
        fake.send("<presence type='subscribe' from='owner@rig.test' to='local@bot.rig.test'/>");
        next(&mut gateway, &incoming, &home);
        fake.expect("type='subscribed'");
        assert_eq!(fake.expect("/>"), "from='local@bot.rig.test' to='owner@rig.test'/>");
        // Probes are answered from the same state; avatars are offline.
        fake.send("<presence type='probe' from='owner@rig.test' to='bot.rig.test'/>");
        next(&mut gateway, &incoming, &home);
        assert!(fake.expect("</presence>").contains("<status>Home</status>"));
        fake.send("<presence type='probe' from='owner@rig.test' to='jane.doe@bot.rig.test'/>");
        next(&mut gateway, &incoming, &home);
        assert_eq!(
            fake.expect("/>"),
            "<presence type='unavailable' from='jane.doe@bot.rig.test' to='owner@rig.test'/>"
        );
        fake.send("<presence type='unsubscribe' from='owner@rig.test' to='bot.rig.test'/>");
        next(&mut gateway, &incoming, &home);
        assert_eq!(
            fake.expect("/>"),
            "<presence type='unsubscribed' from='bot.rig.test' to='owner@rig.test'/>"
        );
        // Strangers' probes are not answered.
        fake.send("<presence type='probe' from='x@elsewhere.org' to='bot.rig.test'/>");
        next(&mut gateway, &incoming, &home);
        assert_eq!(fake.rest(Duration::from_millis(150)), "");
    }

    #[test]
    fn teleport_and_grid_lost() {
        let (mut gateway, _incoming, mut fake) = connected();
        gateway.update_presence(&presence::presence(Some("Home"), false));
        assert_eq!(fake.rest(Duration::from_millis(100)), "", "nothing changed");
        gateway.update_presence(&presence::presence(Some("Away"), false));
        assert_eq!(
            fake.rest(Duration::from_millis(150)),
            "<presence from='bot.rig.test' to='owner@rig.test'><status>Away</status></presence>"
        );
        gateway.update_presence(&presence::presence(None, false));
        assert_eq!(
            fake.rest(Duration::from_millis(150)),
            "<presence from='bot.rig.test' to='owner@rig.test'><show>away</show><status>not in a region</status></presence>\
             <presence type='unavailable' from='local@bot.rig.test' to='owner@rig.test'/>"
        );
    }

    #[test]
    fn messages_wait_while_disconnected_and_go_out_with_their_time() {
        let (mut gateway, incoming, mut fake) = connected();
        let home = presence::presence(Some("Home"), false);
        let jane = uuid::Uuid::from_bytes([1; 16]);
        let im = |text: &str, time| Record {
            channel: Channel::Im,
            time,
            kind: "im".into(),
            avatar_name: "Jane Doe".into(),
            display_name: String::new(),
            object_name: String::new(),
            object_uuid: None,
            text: text.into(),
            avatar_uuid: Some(jane),
            offline: false,
        };
        // The server goes away.
        fake.send("</stream:stream>");
        next(&mut gateway, &incoming, &home);
        assert!(!gateway.is_connected());
        gateway.record(&im("one", 1_795_021_463), Instant::now());
        gateway.record(&im("two", 1_795_021_493), Instant::now());
        // And comes back.
        fake.accept();
        next(&mut gateway, &incoming, &home);
        let written = fake.expect("<body>two</body>");
        let presence = written.find("<status>Home</status>").unwrap();
        let one = written.find("<body>one</body><nick xmlns='http://jabber.org/protocol/nick'>Jane Doe</nick><delay xmlns='urn:xmpp:delay' from='bot.rig.test' stamp='2026-11-18T17:04:23Z'/>").unwrap();
        assert!(presence < one, "{written}");
        assert!(fake.expect("</message>").contains("stamp='2026-11-18T17:04:53Z'"));
    }

    #[test]
    fn error_conditions_follow_the_commands_answers() {
        assert_eq!(condition("the grid knows no avatar named \"no.body\""), Condition::ItemNotFound);
        assert_eq!(
            condition("\"x y z\" is not an avatar name (`First Last`, ...) or a UUID"),
            Condition::ItemNotFound
        );
        assert_eq!(condition("there is no text to send"), Condition::NotAcceptable);
        assert_eq!(
            condition("the text is 2000 bytes long; the limit is 1023"),
            Condition::NotAcceptable
        );
        assert_eq!(condition("not connected to a region"), Condition::ServiceUnavailable);
    }
}
