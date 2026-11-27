//! rslbot: logs an avatar in to a Second Life or OpenSimulator grid, gets it
//! to a target region, sits it on a target object and keeps it there.

mod autoreply;
mod avatars;
mod backoff;
mod caps;
mod commands;
mod config;
mod control;
mod escape;
mod friends;
mod grouplog;
mod groups;
mod inventory;
mod log;
mod login;
mod msglog;
mod names;
mod net;
mod payments;
mod perms;
mod persist;
mod profile;
mod reconciler;
mod rlv;
mod scan;
mod session;
mod startup;
mod txlog;
mod web;
mod udp {
    pub mod circuit;
    pub mod messages;
    pub mod packet;
}
mod xml;
mod xmpp;

use autoreply::Autoreply;
use commands::{Commands, Grid, SentIm};
use config::{Config, Persist, PersistOverride, Place, RlvPersist, Sit, Source};
use control::{AutoreplyRequest, GroupRequest, PayTo, Request, Response, RlvRequest};
use login::Outcome;
use msglog::{Channel, Logs, Recorded, Recorder};
use names::{Lookup, Search};
use net::{Event, RealNet};
use payments::{Interface, Ledger};
use persist::{Book, Location, LocationArg, Placement, SitArg, SitState, World};
use reconciler::{Action, Decision, Goal, Reconciler, SeatGoal};
use rlv::{After, Check, Command, Follow, Issuer, Limits, OwnStep, Restrictions, Rlv, Sitting, Step, Verdict, Who};
use session::{Seat, Session, SessionEvent, Touch};
use std::io::{BufRead, IsTerminal};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use udp::messages::Vector3;
use uuid::Uuid;

/// What RLV viewers answer an instant message `recvim` hides with
/// (RLVa's `blocked_recvim_remote`).
const RLV_RECVIM_REPLY: &str =
    "The Resident you messaged is currently prevented from reading your instant messages at the moment, please try again later.";

const EXIT_OK: i32 = 0;
const EXIT_FATAL: i32 = 1;
const EXIT_CONFIG: i32 = 2;

const LOGIN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on one wait in the main loop when nothing is scheduled.
const IDLE_WAIT: Duration = Duration::from_secs(60);
/// How long the config's `persist` waits for the simulator to say where
/// the seat is, so that the position it takes is the seated one.
const SEAT_POSITION_WAIT: Duration = Duration::from_secs(10);

const USAGE: &str = "\
Usage: rslbot [--verbose] [CONFIG]
       rslbot hash
       rslbot ctl COMMAND [ARGUMENT...]

Runs the bot from CONFIG (default: russelbot.toml).
`rslbot hash` reads a password from standard input and prints the value to
put in the config as `password_hash`.
`rslbot ctl` sends a command to a running bot; see `rslbot ctl --help`.

Options:
  -v, --verbose   also log individual simulator packets
  -h, --help      show this help";

fn main() {
    std::process::exit(run_cli(std::env::args().skip(1).collect()));
}

/// The arguments of a `ctl` invocation. Everything after `ctl` belongs to
/// the command, whatever it looks like.
fn ctl_arguments(args: &[String]) -> Option<&[String]> {
    match args.split_first() {
        Some((first, rest)) if first == "ctl" => Some(rest),
        _ => None,
    }
}

fn run_cli(args: Vec<String>) -> i32 {
    if let Some(command) = ctl_arguments(&args) {
        return control::client(command, control::address(), &mut std::io::stdout(), &mut std::io::stderr());
    }
    let mut positional = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "-v" | "--verbose" => log::set_verbose(true),
            "-h" | "--help" => {
                println!("{USAGE}");
                return EXIT_OK;
            }
            option if option.starts_with('-') => {
                eprintln!("rslbot: unknown option {option}\n\n{USAGE}");
                return EXIT_CONFIG;
            }
            _ => positional.push(arg.as_str()),
        }
    }
    match positional[..] {
        [] => run_bot(Path::new(config::DEFAULT_CONFIG_PATH)),
        ["hash"] => hash_command(),
        [path] => run_bot(Path::new(path)),
        _ => {
            eprintln!("rslbot: too many arguments\n\n{USAGE}");
            EXIT_CONFIG
        }
    }
}

/// Reads a password from standard input and prints its hash.
fn hash_command() -> i32 {
    let stdin = std::io::stdin();
    let password = if stdin.is_terminal() {
        rpassword::prompt_password("Password: ")
    } else {
        let mut line = String::new();
        stdin.lock().read_line(&mut line).map(|_| line)
    };
    let password = match password {
        Ok(password) => password,
        Err(e) => {
            eprintln!("rslbot: cannot read the password: {e}");
            return EXIT_FATAL;
        }
    };
    let password = password.strip_suffix('\n').unwrap_or(&password);
    let password = password.strip_suffix('\r').unwrap_or(password);
    if password.is_empty() {
        eprintln!("rslbot: the password is empty");
        return EXIT_FATAL;
    }
    println!("{}", config::hash_password(password));
    EXIT_OK
}

fn install_signal_handler(events: Sender<Event>) {
    let seen = AtomicBool::new(false);
    let result = ctrlc::set_handler(move || {
        // A second signal during shutdown exits at once.
        if seen.swap(true, Ordering::SeqCst) || events.send(Event::Signal).is_err() {
            std::process::exit(EXIT_OK);
        }
    });
    if let Err(e) = result {
        info!("cannot install the signal handler: {e}");
    }
}

fn seconds(delay: Duration) -> String {
    format!("{} s", delay.as_secs())
}

struct Bot {
    config: Config,
    reconciler: Reconciler,
    session: Option<Session<RealNet>>,
    events: Sender<Event>,
    http: ureq::Agent,
    /// Circuit generations are unique for the life of the process, so events
    /// from a dropped session can never be mistaken for the current one's.
    next_generation: u64,
    stopping: bool,
    /// Chat and instant messages waiting for their names.
    recorder: Recorder,
    logs: Logs,
    name_lookups: Sender<Lookup>,
    /// The current simulator's display-name service: not heard of yet
    /// (`None`), absent (`Some(None)`) or its URL.
    names_url: Option<Option<String>>,
    /// Commands from the control socket.
    commands: Commands,
    /// The avatar search and profile services.
    lookups: Lookups,
    /// The placement the bot holds, and what `persist` remembers.
    book: Book,
    /// The config's `persist`, until it has been applied or the operator
    /// has taken over.
    startup: Option<Persist>,
    /// Since when the config placement has been reached but for the
    /// position of the seat.
    startup_wait: Option<Instant>,
    /// The startup script, and how far it has got.
    script: startup::Script,
    /// What inventory commands need of the grid.
    inventory: Inventory,
    /// RLV: whether it is on, and what a forced movement does to `book`.
    rlv: Rlv,
    /// A followed forced teleport that has not ended.
    follow: Follow,
    /// `startup` is armed for a followed forced sit, not by the config:
    /// the seat is held already, and its position is persisted once the
    /// avatar sits there.
    follow_seat: bool,
    /// Who sent the followed forced sit that made the held seat what it
    /// is: its own restrictions do not hold the bot's steps towards it.
    seat_sender: Option<Who>,
    /// Followed forced sits on objects the simulator has not described,
    /// waiting for them to be looked for: the sender, the command and the
    /// object.
    seats_sought: Vec<(Issuer, Command, Uuid)>,
    /// The held placement was set by a `persist` or `home` that override
    /// exempted and has not been reached yet: no restriction holds the
    /// bot's steps towards it.
    exempt_placement: bool,
    /// The held sit state asks for the avatar to be off its seat, and it
    /// has not been told to stand yet.
    stand_owed: bool,
    /// The step of the bot's own that a restriction holds back, and the
    /// stand likewise: what it is and why, as the log has it.
    held_step: Option<(String, String)>,
    held_stand: Option<String>,
    /// The XMPP gateway, when the config has one.
    xmpp: Option<xmpp::gateway::Gateway>,
    /// What the pages of the web interface are sent, when the config has
    /// one: the messages kept for them, and the pages that are open.
    web: Option<web::hub::Hub>,
    /// Whether and with what the bot answers instant messages by itself.
    autoreply: Autoreply,
    /// The balance has been asked for since the last login.
    balance_asked: bool,
    /// Stands in for a session's view of the world, which tests have none of.
    #[cfg(test)]
    world: Option<Box<dyn World>>,
    /// Stands in for the session where commands use the grid, in tests.
    #[cfg(test)]
    grid: Option<commands::tests::FakeGrid>,
}

/// The way to the avatar's inventory: the worker, and what the login and
/// the current simulator said about where it is.
struct Inventory {
    jobs: Sender<inventory::Job>,
    /// The current simulator's inventory services: not heard of yet
    /// (`None`), or as far as it has them.
    urls: Option<caps::InventoryUrls>,
    /// The folder at the top of My Inventory, from the login.
    root: Option<Uuid>,
    /// The avatar, whose inventory it is.
    agent: Uuid,
    /// The circuit `urls` belong to.
    generation: Option<u64>,
}

/// The workers that ask the grid about avatars over HTTP, and where the
/// current simulator answers.
struct Lookups {
    avatar_searches: Sender<Search>,
    /// The current simulator's avatar search service, if it has one.
    search_url: Option<String>,
    profiles: Sender<profile::Fetch>,
    /// The current simulator's profile service, if it has one.
    profile_url: Option<String>,
    /// The worker that takes chat sessions up.
    take_up: Sender<caps::TakeUp>,
    /// The current simulator's chat-session service, if it has one.
    chat_sessions_url: Option<String>,
}

/// The session as commands see it. A simulator with an avatar search or
/// profile service is asked there, as viewers do; the older simulator
/// messages are for the ones without.
struct Live<'a> {
    session: &'a mut Session<RealNet>,
    lookups: &'a Lookups,
    inventory: &'a Inventory,
    rlv: &'a Rlv,
}

/// Whether, and on what, the avatar sits, as far as the session knows: a
/// ground sit only when the bot sent it.
fn sitting(session: &Session<RealNet>) -> Sitting {
    match session.seat() {
        Seat::Object(_) | Seat::Other => Sitting::Object,
        _ if session.on_ground() => Sitting::Ground,
        _ => Sitting::No,
    }
}

impl Grid for Live<'_> {
    fn activate_group(&mut self, group: Uuid, now: Instant) -> Result<(), String> {
        self.session.activate_group(group, now)
    }
    fn invite_to_group(&mut self, group: Uuid, avatar: Uuid, now: Instant) -> Result<(), String> {
        self.session.invite_to_group(group, avatar, now)
    }
    fn ask_group_profile(&mut self, group: Uuid, now: Instant) -> Result<(), String> {
        self.session.ask_group_profile(group, now)
    }
    fn join_group(&mut self, group: Uuid, now: Instant) -> Result<(), String> {
        self.session.join_group(group, now)
    }
    fn leave_group(&mut self, group: Uuid, now: Instant) -> Result<(), String> {
        self.session.leave_group(group, now)
    }
    fn ask_groups(&mut self, now: Instant) -> Result<(), String> {
        self.session.ask_groups(now)
    }
    fn answer_invitation(&mut self, group: Uuid, id: Uuid, accept: bool, now: Instant) -> Result<(), String> {
        self.session.answer_invitation(group, id, accept, now)
    }
    fn start_group_session(&mut self, group: Uuid, now: Instant) -> Result<(), String> {
        self.session.start_group_session(group, now)
    }
    fn group_chat(&mut self, group: Uuid, text: &str, now: Instant) -> Result<(), String> {
        self.session.group_chat(group, text, now)
    }
    fn take_up_session(&mut self, group: Uuid) -> bool {
        match &self.lookups.chat_sessions_url {
            Some(url) => {
                let take_up = caps::TakeUp {
                    url: url.clone(),
                    session: group,
                };
                // The worker lives as long as the bot.
                let _ = self.lookups.take_up.send(take_up);
                true
            }
            None => false,
        }
    }
    fn pay(&mut self, dest: Uuid, amount: i32, kind: i32, description: &str, now: Instant) -> Result<(), String> {
        self.session.pay(dest, amount, kind, description, now)
    }
    fn request_pay_price(&mut self, object: Uuid, now: Instant) -> Result<(), String> {
        self.session.request_pay_price(object, now)
    }
    fn money_target(&self, object: Uuid) -> Option<(Uuid, bool)> {
        self.session.money_target(object)
    }
    fn reach(&mut self, distance: Option<f32>, now: Instant) -> Result<f32, String> {
        self.session.reach(distance, now)
    }
    fn present(&self) -> Result<(), String> {
        self.session.present()
    }
    fn own_id(&self) -> Uuid {
        self.session.agent_id()
    }
    fn avatars(&self) -> Vec<avatars::Seen> {
        self.session.avatars()
    }
    fn avatars_known(&self) -> bool {
        self.session.avatars_known()
    }
    fn friends(&self) -> &friends::Friends {
        self.session.friends()
    }
    fn ask_profile(&mut self, avatar: Uuid, now: Instant) -> Result<(), String> {
        let Some(url) = &self.lookups.profile_url else {
            return self.session.request_profile(avatar, now);
        };
        self.session.present()?;
        let fetch = profile::Fetch {
            url: url.clone(),
            id: avatar,
        };
        self.lookups
            .profiles
            .send(fetch)
            .map_err(|_| "the profile service is not running".to_string())
    }
    fn sightings(&self) -> Vec<scan::Sighting> {
        self.session.sightings()
    }
    fn ask_properties(&mut self, local_ids: &[u32], now: Instant) {
        self.session.ask_properties(local_ids, now);
    }
    fn settled_at(&self) -> Instant {
        self.session.settled_at()
    }
    fn restrictions(&self) -> Option<&Restrictions> {
        self.rlv.in_force()
    }
    fn sitting(&self) -> Sitting {
        sitting(self.session)
    }
    fn thing(&self, object: Uuid) -> session::Thing {
        self.session.thing(object)
    }
    fn attachment_prims(&self, item: Uuid) -> Vec<Uuid> {
        self.session.attachment_prims(item)
    }
    fn look_for(&mut self, object: Uuid, now: Instant) -> Result<session::Look, String> {
        self.session.look_for(object, now)
    }
    fn unknown_object(&self, object: Uuid) -> String {
        self.session.unknown_object(object)
    }
    fn say(&mut self, channel: i32, chat_type: u8, text: &str, now: Instant) -> Result<(), String> {
        self.session.say(channel, chat_type, text, now)
    }
    fn instant_message(&mut self, to: Uuid, text: &str, now: Instant) -> Result<(), String> {
        self.session.instant_message(to, text, now)
    }
    fn find_avatar(&mut self, query: Uuid, name: &str, now: Instant) -> Result<(), String> {
        let Some(url) = &self.lookups.search_url else {
            return self.session.find_avatar(query, name, now);
        };
        self.session.present()?;
        let search = Search {
            url: url.to_string(),
            query,
            name: name.to_string(),
        };
        self.lookups
            .avatar_searches
            .send(search)
            .map_err(|_| "the avatar search is not running".to_string())
    }
    fn stand(&mut self, now: Instant) -> Result<(), String> {
        self.session.stand(now)
    }
    fn sit_ground(&mut self, now: Instant) -> Result<(), String> {
        self.session.sit_ground(now)
    }
    fn sit(&mut self, object: Uuid, now: Instant) -> Result<(), String> {
        self.session.present()?;
        self.session.start_sit(object, now);
        Ok(())
    }
    fn teleport(&mut self, region: &str, position: Vector3, now: Instant) -> Result<(), String> {
        self.session.present()?;
        self.session.start_teleport(region, position, now);
        Ok(())
    }
    fn teleport_to(&mut self, handle: u64, position: Vector3, now: Instant) -> Result<(), String> {
        self.session.present()?;
        self.session.start_teleport_to(handle, position, now);
        Ok(())
    }
    fn region(&self) -> Option<String> {
        self.session.region().map(str::to_string)
    }
    fn position(&self) -> Option<Vector3> {
        self.session.position()
    }
    fn request_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String> {
        self.session.request_teleport(to, message, now)
    }
    fn offer_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String> {
        self.session.offer_teleport(to, message, now)
    }
    fn accept_teleport(&mut self, lure: Uuid, now: Instant) -> Result<(), String> {
        self.session.accept_teleport(lure, now)
    }
    fn touch(&mut self, object: Uuid, now: Instant) -> Result<Touch, String> {
        self.session.touch(object, now)
    }
    fn inventory(&mut self, id: u64, deadline: Instant, what: inventory::What) -> Result<(), String> {
        self.session.present()?;
        let Some(urls) = &self.inventory.urls else {
            return Err("the simulator has not said yet whether it has an inventory service; try again in a moment".to_string());
        };
        let (Some(root), true) = (self.inventory.root, urls.descendents.is_some() && urls.items.is_some()) else {
            return Err(inventory::NO_SERVICE.to_string());
        };
        let job = inventory::Job {
            id,
            generation: self.session.generation(),
            urls: urls.clone(),
            owner: self.inventory.agent,
            root,
            deadline,
            what,
        };
        self.inventory
            .jobs
            .send(job)
            .map_err(|_| "the inventory worker is not running".to_string())
    }
    fn outfit_service(&self) -> bool {
        self.inventory.urls.as_ref().is_some_and(caps::InventoryUrls::outfit_service)
    }
    fn ready(&self) -> bool {
        self.session.present().is_ok() && self.inventory.urls.is_some()
    }
    fn attached(&self) -> Vec<(Uuid, u8)> {
        self.session.attached()
    }
    fn attached_item(&self, object: Uuid) -> Option<Uuid> {
        self.session.attached_item(object)
    }
    fn attach(&mut self, item: &inventory::Item, point: u8, now: Instant) -> Result<(), String> {
        let item = udp::messages::AttachItem {
            item_id: item.id,
            owner_id: if item.owner.is_nil() { self.inventory.agent } else { item.owner },
            flags: item.flags,
            group_mask: item.group_mask,
            everyone_mask: item.everyone_mask,
            next_owner_mask: item.next_owner_mask,
            name: item.name.clone(),
            description: item.description.clone(),
        };
        self.session.attach(&item, point, now)
    }
    fn detach(&mut self, item: Uuid, now: Instant) -> Result<(), String> {
        self.session.detach(item, now)
    }
    fn animate(&mut self, animation: Uuid, start: bool, now: Instant) -> Result<(), String> {
        self.session.animate(animation, start, now)
    }
    fn play_sound(&mut self, sound: Uuid, now: Instant) -> Result<(), String> {
        self.session.play_sound(sound, now)
    }
    fn object_owner(&mut self, object: Uuid, now: Instant) -> Result<(), String> {
        self.session.request_object_owner(object, now)
    }
    fn grant_permissions(&mut self, object: Uuid, item: Uuid, permissions: i32, now: Instant) -> Result<(), String> {
        self.session.grant_permissions(object, item, permissions, now)
    }
    fn accept_inventory(&mut self, to: Uuid, from_object: bool, transaction: Uuid, folder: Uuid, now: Instant) -> Result<(), String> {
        self.session.accept_inventory(to, from_object, transaction, folder, now)
    }
}

/// The session, if there is one, for commands to act on.
fn live<'a>(session: &'a mut Option<Session<RealNet>>, lookups: &'a Lookups, inventory: &'a Inventory, rlv: &'a Rlv) -> Option<Live<'a>> {
    let session = session.as_mut()?;
    Some(Live {
        session,
        lookups,
        inventory,
        rlv,
    })
}

/// What the main loop should do after handling something.
enum Flow {
    Continue,
    Exit(i32),
}

impl Bot {
    /// A bot that holds the config placement and has not logged in yet.
    fn new(config: Config, logs: Logs, events: Sender<Event>) -> Bot {
        let book = Book::new(config_placement(&config));
        let mut commands = Commands::new(config.requesttp_timeout);
        commands.set_restore(config.restore_attachments);
        commands.set_rules(config.perms.clone());
        // The limit as far as the transactions file says comes with
        // `start_payments`.
        let ledger = config.payments.as_ref().map_or(Ledger::new(None, None), Ledger::from_config);
        commands.set_payments(config.payments.clone(), ledger);
        Bot {
            rlv: Rlv::new(config.rlv, config.rlv_persist, config.persist_override),
            follow: Follow::default(),
            follow_seat: false,
            seat_sender: None,
            seats_sought: Vec::new(),
            exempt_placement: false,
            stand_owed: false,
            held_step: None,
            held_stand: None,
            #[cfg(test)]
            world: None,
            #[cfg(test)]
            grid: None,
            // Started once the head of the startup script has run.
            xmpp: None,
            web: config.web.as_ref().map(|_| {
                let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                web::hub::Hub::new(started)
            }),
            balance_asked: false,
            autoreply: match &config.autoreply {
                Some(auto) => Autoreply::new(Some(auto.message.clone()), auto.pause),
                None => Autoreply::new(None, autoreply::DEFAULT_PAUSE),
            },
            reconciler: Reconciler::new(goal(book.held(), config_start(&config))),
            book,
            startup: Some(config.persist),
            startup_wait: None,
            script: startup::Script::new(config.startup.clone()),
            recorder: Recorder::default(),
            logs,
            name_lookups: names::spawn(events.clone()),
            names_url: None,
            commands,
            lookups: Lookups {
                avatar_searches: names::spawn_search(events.clone()),
                search_url: None,
                profiles: profile::spawn(events.clone()),
                profile_url: None,
                take_up: caps::spawn_take_up(),
                chat_sessions_url: None,
            },
            inventory: Inventory {
                jobs: inventory::spawn(events.clone()),
                urls: None,
                root: None,
                agent: Uuid::nil(),
                generation: None,
            },
            config,
            session: None,
            events,
            http: caps::http_agent(LOGIN_HTTP_TIMEOUT),
            next_generation: 1,
            stopping: false,
        }
    }

    /// Restores the limit from the transactions file read at startup,
    /// says how payments stand, and gives the pages the newest rows when
    /// the web interface may pay.
    fn start_payments(&mut self, back: &txlog::ReadBack, now: u64) {
        let (ledger, lines) = payments_start(&self.config, back, now);
        for line in lines {
            info!("{line}");
        }
        self.commands.set_payments(self.config.payments.clone(), ledger);
        let web_pays = self.config.payments.as_ref().is_some_and(|p| p.may(Interface::Web));
        if let (Some(hub), true) = (&mut self.web, web_pays) {
            hub.allow_money(back.newest());
        }
    }

    /// Connects the XMPP gateway, when the config has one: before the
    /// first login, which may take a while, and after the head of the
    /// startup script.
    fn start_gateway(&mut self) {
        let events = self.events.clone();
        self.xmpp = self.config.xmpp.as_ref().map(|xmpp| xmpp::gateway::Gateway::start(xmpp, events));
    }

    /// Carries out the setting commands at the head of the startup
    /// script, before the bot contacts any server. They answer at once.
    /// One that is refused is a mistake in the config.
    fn startup_head(&mut self, now: Instant) -> Result<(), startup::ScriptError> {
        while let Some(line) = self.script.next_head() {
            info!("startup: line {}: {}", line.number, escape::escape(&line.text));
            let (reply, answer) = mpsc::channel();
            // No setting command stops the bot.
            let _ = self.control(line.request, reply, now);
            let fail = |reason: String| startup::ScriptError { line: line.number, reason };
            match answer.try_recv() {
                Ok(Response::Ok(text)) => Bot::script_answered(line.number, text),
                Ok(Response::Error(reason)) => return Err(fail(reason)),
                Err(_) => return Err(fail("the command got no answer".to_string())),
            }
        }
        Ok(())
    }

    /// Writes the answer of a startup script command to the log.
    fn script_answered(line: usize, answer: Option<String>) {
        match answer {
            Some(text) => info!("startup: line {line} answered: {}", text.replace('\n', "; ")),
            None => info!("startup: line {line} done"),
        }
    }

    /// Goes on with the rest of the startup script once the config
    /// placement has been reached: takes the answer of the command in
    /// flight, and hands over the next one when there is none. A command
    /// that is refused ends the script. Returns whether anything happened.
    fn startup_rest(&mut self) -> Result<bool, i32> {
        let mut acted = false;
        loop {
            if let Some((line, outcome)) = self.script.answered() {
                acted = true;
                match outcome {
                    Ok(answer) => Bot::script_answered(line.number, answer),
                    Err(reason) => {
                        let after = match self.script.stop() {
                            0 => "it was the last command of the startup script".to_string(),
                            left => format!(
                                "the startup script stops here: {} after it will not be carried out",
                                startup::commands(left)
                            ),
                        };
                        info!("startup: line {} refused: {reason}; {after}", line.number);
                        return Ok(acted);
                    }
                }
            }
            if self.stopping {
                // A `quit` of its own, or the operator's.
                if self.script.running() {
                    self.script.stop();
                }
                return Ok(acted);
            }
            let was_running = self.script.running();
            let Some(line) = self.script.next() else {
                if was_running && !self.script.running() {
                    info!("startup: the script is done");
                }
                return Ok(acted);
            };
            acted = true;
            info!("startup: line {}: {}", line.number, escape::escape(&line.text));
            let (reply, answer) = mpsc::channel();
            let request = line.request.clone();
            self.script.sent(line, answer);
            if let Flow::Exit(code) = self.control(request, reply, Instant::now()) {
                return Err(code);
            }
        }
    }

    /// Gives up the rest of the startup script when it still waits for
    /// the config placement: the operator, or what moved the avatar, has
    /// taken over, as with the config's `persist`.
    fn give_up_script(&mut self, why: &str) {
        if let Some((first, left)) = self.script.give_up() {
            info!(
                "startup: {why}; {} of the startup script, from line {first} on, will not be carried out",
                startup::commands(left)
            );
        }
    }

    fn login(&mut self, start: login::Start) {
        let cfg = &self.config;
        let held = self.book.held().location.as_ref();
        info!(
            "logging in as {} {} at {} (start: {})",
            cfg.first_name,
            cfg.last_name,
            cfg.login_url,
            login::start_location(start, held)
        );
        let outcome = login::login(&self.http, cfg, start, held);
        let now = Instant::now();
        match outcome {
            Outcome::Success(info) => {
                info!(
                    "login accepted; connecting to simulator {} with circuit code {}",
                    info.sim_addr, info.circuit_code
                );
                self.reconciler.login_accepted();
                self.inventory.root = info.inventory_root;
                self.inventory.agent = info.agent_id;
                if info.inventory_root.is_none() {
                    debug!("the login gave no inventory root folder");
                }
                let net = RealNet::new(self.events.clone());
                let name = format!("{} {}", cfg.first_name, cfg.last_name);
                let generation = self.next_generation;
                let session = Session::new(net, &info, &name, cfg.scan_distance, cfg.fetch_offline_im, generation, now);
                self.session = Some(session);
                self.balance_asked = false;
                self.commands.groups_session_began();
            }
            Outcome::NeedsOperator(failure) => {
                info!("login needs the operator ({}): {}", failure.reason, failure.message);
                self.reconciler.login_needs_operator(&failure);
            }
            Outcome::Refused(failure) => {
                info!("login refused ({}): {}", failure.reason, failure.message);
                if let Some(delay) = self.reconciler.login_refused(&failure, now) {
                    info!("next login in {}, asking for the last location", seconds(delay));
                }
            }
            Outcome::Temporary(failure) => {
                info!("login failed ({}): {}", failure.reason, failure.message);
                let delay = self.reconciler.login_failed(now);
                info!("next login in {}", seconds(delay));
            }
        }
    }

    fn drop_session(&mut self) {
        if let Some(session) = self.session.take() {
            self.next_generation = session.generation() + 1;
        }
        self.names_url = None;
        self.lookups.search_url = None;
        self.lookups.profile_url = None;
        self.lookups.chat_sessions_url = None;
        self.inventory.urls = None;
        self.inventory.generation = None;
        self.commands.session_lost();
        self.rlv.forget_waiting();
        // What they waited to see described was in this session.
        self.seats_sought.clear();
        // The objects of the next session are other objects.
        self.rlv.session_ended();
        // And the avatar of the next one sits on nothing.
        self.stand_owed = false;
        self.held_stand = None;
        self.held_step = None;
    }

    /// Reads the Current Outfit folder when it is time to compare it with
    /// what is attached (see `Commands::entered_region`).
    fn restore_attachments(&mut self, now: Instant) {
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.restore_due(live.as_mut(), now);
    }

    /// Decides the requests that waited too long for the grid to say who
    /// is behind them (see `Commands::requests_due`).
    fn decide_requests(&mut self, now: Instant) {
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.requests_due(live.as_mut(), now);
        self.follow_commands();
    }

    /// Looks at the `scan` commands that wait for the simulator (see
    /// `Commands::scans_due`).
    fn scan_objects(&mut self, now: Instant) {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            return self.commands.scans_due(Some(grid), now);
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.scans_due(live.as_mut(), now);
    }

    /// Notices a hand-over to another simulator: the inventory services
    /// the bot knows are the old one's, and so is whatever a command is
    /// asking them.
    fn follow_simulator(&mut self) {
        let generation = self.session.as_ref().map(Session::generation);
        if generation != self.inventory.generation {
            if self.inventory.generation.is_some() && generation.is_some() {
                self.inventory.urls = None;
                self.commands.simulator_changed();
            }
            self.inventory.generation = generation;
        }
    }

    /// The grid answered an avatar search: the commands waiting for it go on.
    fn avatars_found(&mut self, query: Uuid, avatars: &[(Uuid, String)], now: Instant) {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            for sent in self.commands.avatars_found(query, avatars, Some(grid), now) {
                self.record_sent(sent, now);
            }
            self.follow_commands();
            return;
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        for sent in self.commands.avatars_found(query, avatars, live.as_mut(), now) {
            self.record_sent(sent, now);
        }
        self.follow_commands();
    }

    /// Stops putting the avatar back once a command has moved it. The
    /// operator has taken over: the config's `persist` no longer applies.
    fn follow_commands(&mut self) {
        if self.go_manual() {
            // And a forced movement that was to be followed is not.
            self.follow.give_up();
        }
    }

    /// Goes to manual mode if a command has moved the avatar. Returns
    /// whether one has.
    fn go_manual(&mut self) -> bool {
        let moved = self.commands.take_manual();
        if moved {
            self.startup = None;
            self.give_up_script("the avatar was moved before it was in place");
            self.follow_seat = false;
            self.seat_sender = None;
            self.set_manual(true);
        }
        moved
    }

    /// Switches between holding the placement and leaving the avatar where
    /// the operator put it. Switching to holding always lets the bot act
    /// at once, also when it was holding already.
    fn set_manual(&mut self, manual: bool) {
        let was_manual = self.reconciler.is_manual();
        self.reconciler.set_manual(manual);
        if manual {
            // Nothing is the bot's to do any more, held back or exempt.
            self.exempt_placement = false;
            self.stand_owed = false;
        }
        if !manual {
            info!("placement: holding {}", self.book.held());
        } else if !was_manual {
            info!("placement: manual; the avatar stays where commands put it until `persist` or `home`");
        }
    }

    /// Hands the held placement to the reconciler as its goal.
    fn sync_goal(&mut self, now: Instant) {
        let held = self.book.held();
        let seated = self.session.as_ref().is_some_and(|session| on_target(held.sit, session.seat()));
        self.reconciler.set_goal(goal(held, config_start(&self.config)), seated, now);
    }

    /// Carries out a persist: makes what the two arguments stand for the
    /// held placement and holds it. Returns the placement as text, or why
    /// nothing was changed.
    fn persist(&mut self, location: &LocationArg, sit: SitArg, exempt: bool, now: Instant) -> Result<String, String> {
        // `cur` needs the avatar in a region and not on its way out of it.
        let world = self.session.as_ref().filter(|session| session.present().is_ok());
        let world = world.map(|session| session as &dyn World);
        #[cfg(test)]
        let world = world.or(self.world.as_deref());
        let placement = self
            .book
            .resolve(location, sit, world, self.commands.last_tp(), self.commands.last_sit())?;
        self.book.commit(placement.clone());
        self.startup = None;
        self.startup_wait = None;
        self.give_up_script("a placement was persisted before the avatar was in place");
        // What was to follow a forced movement is overtaken by this.
        self.follow_seat = false;
        self.seat_sender = None;
        self.follow.give_up();
        // An answer to an earlier `requesttp` must not take the avatar away.
        self.commands.forget_requests();
        self.sync_goal(now);
        self.set_manual(false);
        // The steps towards it are held to the RLV restrictions, unless
        // the command that asked for it was exempt.
        self.exempt_placement = exempt;
        // Owed only for a seat the avatar is on now: the bot does not keep
        // an avatar standing that sits down later.
        self.stand_owed = self
            .session
            .as_ref()
            .filter(|session| session.present().is_ok())
            .is_some_and(|session| needs_stand(placement.sit, session.seat(), session.on_ground()));
        self.owed_stand(now);
        Ok(placement.to_string())
    }

    /// The restrictions the bot's own steps towards the held placement
    /// are held to: none for a placement an exempt command set, and not
    /// those of the sender of a followed forced sit.
    fn own_limits<'a>(rlv: &'a Rlv, exempt: bool, seat_sender: Option<Who>) -> Limits<'a> {
        match exempt {
            true => Limits::none(),
            false => Limits::new(rlv.in_force(), seat_sender),
        }
    }

    /// Stands the avatar up when the held sit state calls for it and no
    /// restriction holds the stand back. One that is held back stays
    /// owed, and is looked at again with every pass of the main loop.
    fn owed_stand(&mut self, now: Instant) {
        if !self.stand_owed {
            return;
        }
        let held = self.book.held().sit;
        let Some(session) = self.session.as_mut().filter(|session| session.present().is_ok()) else {
            return;
        };
        if !needs_stand(held, session.seat(), session.on_ground()) {
            self.stand_owed = false;
            if self.held_stand.take().is_some() {
                info!("rlv: no longer holding back the stand");
            }
            return;
        }
        let limits = Bot::own_limits(&self.rlv, self.exempt_placement, self.seat_sender);
        match rlv::own_step(limits, sitting(session), OwnStep::Stand, self.rlv.overridden()) {
            Verdict::HeldBack(refusal) => {
                let why = refusal.short();
                if self.held_stand.as_ref() != Some(&why) {
                    info!("rlv: holding back the stand: {why}");
                    self.held_stand = Some(why);
                }
                return;
            }
            Verdict::LetThrough(refusal) => {
                info!("rlv: standing up although {}: persist override sit", refusal.short());
            }
            Verdict::Go => {}
        }
        if self.held_stand.take().is_some() {
            info!("rlv: no longer holding back the stand");
        }
        self.stand_owed = false;
        match session.stand(now) {
            Ok(()) => {
                info!("standing up");
                self.commands.stood_up();
            }
            Err(reason) => info!("cannot stand up: {reason}"),
        }
    }

    /// A `persist` or `home` command: reads its arguments, carries it out
    /// and answers.
    fn persist_command(&mut self, what: &str, location: &str, sit: &str, exempt: bool, reply: &Sender<Response>, now: Instant) {
        let outcome = persist::parse_location(location)
            .and_then(|location| Ok((location, persist::parse_sit_state(sit)?)))
            .and_then(|(location, sit)| {
                info!("control: {what}");
                self.persist(&location, sit, exempt, now)
            });
        // The client may have gone away; nothing depends on it.
        let _ = reply.send(match outcome {
            Ok(placement) => Response::Ok(Some(placement)),
            Err(reason) => {
                info!("control: {what} refused: {reason}");
                Response::Error(reason)
            }
        });
    }

    /// Carries out the config's `persist` once the avatar is where the
    /// config puts it. Returns when to look again while it waits for the
    /// seat's position.
    fn apply_startup(&mut self, now: Instant) -> Option<Instant> {
        let setting = self.startup?;
        let session = self.session.as_ref()?;
        let arrival = Arrival {
            present: session.present().is_ok(),
            in_target_region: self.reconciler.in_target_region(),
            seated_on_target: self.reconciler.seated_on_target(),
            ground_sat: self.reconciler.ground_sat(),
            seat_frame_known: session.seat_frame_known(),
        };
        // Until it has been applied, the config's sit state is the held
        // one; for a followed forced sit the held one is what counts.
        // The rest of the startup script may ask where the avatar sits.
        let rest = self.script.waiting();
        let settled = match startup_due(self.book.held().sit, setting, rest, arrival, self.startup_wait, now) {
            Startup::NotYet => {
                self.startup_wait = None;
                return None;
            }
            Startup::Wait(until) => {
                self.startup_wait.get_or_insert(now);
                return Some(until);
            }
            Startup::Now { settled } => settled,
        };
        self.startup_reached(setting, settled, now);
        None
    }

    /// The avatar is where the config, or a followed forced sit, puts it:
    /// applies `setting`, and lets the rest of the startup script run when
    /// it waited for this. `settled` is false when the seat's position was
    /// waited for in vain.
    fn startup_reached(&mut self, setting: Persist, settled: bool, now: Instant) {
        self.startup = None;
        self.startup_wait = None;
        let for_rlv = std::mem::take(&mut self.follow_seat);
        // Before anything is persisted: a persist gives up a script that
        // still waits.
        if self.script.release() {
            info!("startup: the avatar is in place; carrying out the rest of the startup script");
        }
        let Persist::On { position, sit } = setting else {
            info!("the avatar is in place and `persist` is off");
            self.set_manual(true);
            return;
        };
        if !settled {
            info!(
                "the simulator has not said where the seat is within {} s; \
                 persisting the position from before the sit",
                SEAT_POSITION_WAIT.as_secs()
            );
        }
        let word = |source| if source == Source::Cur { "cur" } else { "cfg" };
        let whose = if for_rlv { "rlv: following" } else { "config" };
        let what = format!("{whose}: persist {} {}", word(position), word(sit));
        info!("{what}");
        let location = if position == Source::Cur {
            LocationArg::Cur
        } else {
            LocationArg::Cfg
        };
        let sit = if sit == Source::Cur { SitArg::Cur } else { SitArg::Cfg };
        // The seat is still the one the forced sit made the held one.
        let seat_sender = self.seat_sender.filter(|_| for_rlv);
        if let Err(reason) = self.persist(&location, sit, false, now) {
            info!("{what} refused: {reason}");
        }
        self.seat_sender = seat_sender;
    }

    /// The answer to the `status` command as `via` gets it: with the
    /// money lines for an interface that may pay.
    fn status(&self, via: Interface) -> String {
        let avatar = format!("{} {}", self.config.first_name, self.config.last_name);
        let money = self.config.payments.as_ref().filter(|payments| payments.may(via)).map(|_| {
            let blocked = commands::descriptions_blocked(self.rlv.in_force());
            (self.commands.balance(), self.commands.limit_text(), blocked)
        });
        status_answer(&avatar, &self.place_status(), money)
    }

    /// The lines of `status` about where the avatar is and what is held.
    fn place_status(&self) -> String {
        let here = self.session.as_ref().and_then(|session| {
            let region = session.region()?;
            Some(Whereabouts {
                region,
                position: session.position(),
                seat: session.seat(),
                seat_name: session.seat_name(),
            })
        });
        status_text(here, self.book.held(), self.reconciler.is_manual())
    }

    /// Carries out a command from the control socket or the startup
    /// script.
    fn control(&mut self, request: Request, reply: Sender<Response>, now: Instant) -> Flow {
        self.control_as(request, reply, false, Interface::Ctl, now)
    }

    /// Carries out a command. `own` for one the page of the web interface
    /// sent by itself: it is never the exempt command of an RLV override,
    /// which it leaves in place, and the operator log gets no line for it.
    /// `via` is the interface it came through, which decides whether it
    /// may pay and what `status` shows.
    fn control_as(&mut self, request: Request, reply: Sender<Response>, own: bool, via: Interface, now: Instant) -> Flow {
        // While RLV is in override, the next command but these seven is
        // exempt from the restrictions, and RLV is on again with it. No
        // restriction applies to `friends` and `profile`, so they have no
        // use for one; `scan` is held to some, and uses it up.
        let never = matches!(
            request,
            Request::Rlv(_)
                | Request::Status
                | Request::Perms(_)
                | Request::Autoreply(_)
                | Request::Friends
                | Request::Profile(_)
                | Request::Quit
                | Request::Pay { .. }
                | Request::Prices(_)
                | Request::Payees
                | Request::Group(GroupRequest::List | GroupRequest::Info(_) | GroupRequest::Invitations)
        );
        let exempt = !never && !own && self.rlv.take_override(request.name());
        match request {
            Request::Status => {
                // The client may have gone away; nothing depends on it.
                let _ = reply.send(Response::Ok(Some(self.status(via))));
            }
            request @ (Request::Pay { .. } | Request::Prices(_) | Request::Payees) => {
                self.money_command(request, reply, own, via, now);
            }
            Request::Persist { location, sit } => {
                let (location, sit) = (location.as_deref().unwrap_or("cur"), sit.as_deref().unwrap_or("cur"));
                self.persist_command(&format!("persist {location} {sit}"), location, sit, exempt, &reply, now);
            }
            Request::Home => self.persist_command("home", "last", "last", exempt, &reply, now),
            Request::Rlv(RlvRequest::Mode(mode)) => {
                let _ = reply.send(match self.rlv.set_mode(mode) {
                    Ok(line) => Response::Ok(Some(line)),
                    Err(reason) => {
                        info!("control: rlv mode refused: {reason}");
                        Response::Error(reason)
                    }
                });
            }
            Request::Rlv(RlvRequest::PersistOverride { sit, tp }) => {
                let line = self.rlv.set_overridden(PersistOverride { sit, tp });
                let _ = reply.send(Response::Ok(Some(line)));
            }
            Request::Rlv(RlvRequest::Reset) => {
                let line = self.rlv.reset();
                let _ = reply.send(Response::Ok(Some(line)));
            }
            Request::Rlv(RlvRequest::Persist(follow)) => {
                let setting = if follow { RlvPersist::Follow } else { RlvPersist::Drop };
                let line = self.rlv.set_persist(setting);
                let _ = reply.send(Response::Ok(Some(line)));
            }
            Request::Rlv(RlvRequest::Status) => {
                let _ = reply.send(Response::Ok(Some(self.rlv.status())));
            }
            Request::Rlv(RlvRequest::Cmd(text)) => self.rlv_command(&text, &reply, now),
            Request::Autoreply(request) => {
                let _ = reply.send(match self.autoreply_command(request, now) {
                    Ok(answer) => Response::Ok(Some(answer)),
                    Err(reason) => {
                        info!("control: autoreply refused: {reason}");
                        Response::Error(reason)
                    }
                });
            }
            Request::Quit => {
                info!("control: quit");
                let _ = reply.send(Response::Ok(None));
                return self.stop();
            }
            request => {
                let origin = if own {
                    commands::Origin::Page
                } else if exempt {
                    commands::Origin::Exempt
                } else {
                    commands::Origin::Operator
                };
                // A join or an acceptance may pay a fee: a row the limit
                // needs is written first, if it can be now.
                let pays = matches!(
                    &request,
                    Request::Group(GroupRequest::Join { pay: Some(_), .. }) | Request::Accept { pay: Some(_), .. }
                );
                if pays {
                    let unwritten = self.logs.retry_transactions();
                    self.commands.set_unwritten(unwritten);
                }
                #[cfg(test)]
                if let Some(grid) = self.grid.as_mut() {
                    if let Some(sent) = self.commands.handle_through(origin, via, request, reply, Some(grid), now) {
                        self.record_sent(sent, now);
                    }
                    self.follow_commands();
                    self.take_transactions(now);
                    return Flow::Continue;
                }
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                if let Some(sent) = self.commands.handle_through(origin, via, request, reply, live.as_mut(), now) {
                    self.record_sent(sent, now);
                }
                self.follow_commands();
                self.take_transactions(now);
            }
        }
        Flow::Continue
    }

    /// `pay`, `prices` and `payees`, for an interface that may pay. They
    /// never move the avatar and no RLV restriction applies to them.
    fn money_command(&mut self, request: Request, reply: Sender<Response>, own: bool, via: Interface, now: Instant) {
        let command = request.name();
        let refusal = match &self.config.payments {
            None => Some("payments are not enabled: the config has no `[payments]` table".to_string()),
            Some(payments) if !payments.may(via) => Some(format!(
                "payments are not enabled for `{via}`: `perms` in the config's `[payments]` table does not list it"
            )),
            Some(_) => None,
        };
        if let Some(reason) = refusal {
            let what = match &request {
                Request::Pay { amount, payee, .. } => format!("pay L${amount} to {}", payee_words(payee)),
                _ => command.to_string(),
            };
            if !own {
                info!("{via}: {what} refused: {reason}");
            }
            let _ = reply.send(Response::Error(reason));
            return;
        }
        if matches!(request, Request::Pay { .. }) {
            // A row the limit needs is written first, if it can be now.
            let unwritten = self.logs.retry_transactions();
            self.commands.set_unwritten(unwritten);
        }
        let origin = if own { commands::Origin::Page } else { commands::Origin::Operator };
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            self.commands.money_command(origin, via, request, reply, Some(grid), now);
            return;
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.money_command(origin, via, request, reply, live.as_mut(), now);
    }

    /// Asks the grid for the avatar's balance, once after each login, when
    /// payments are switched on.
    fn ask_balance(&mut self, now: Instant) {
        if self.config.payments.is_none() || self.balance_asked {
            return;
        }
        if let Some(session) = &mut self.session {
            if session.request_balance(now).is_ok() {
                self.balance_asked = true;
            }
        }
    }

    /// Gives up on the payment commands that waited too long.
    fn pay_due(&mut self, now: Instant) {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            return self.commands.money_due(Some(grid), now);
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.money_due(live.as_mut(), now);
    }

    // -------------------------------------------------------------- RLV

    /// `rlv cmd`: RLV commands of the operator's, dealt with at once as
    /// if an object had sent them, with the avatar itself as the sender.
    fn rlv_command(&mut self, text: &str, reply: &Sender<Response>, now: Instant) {
        let refuse = |reason: String| {
            info!("control: rlv cmd refused: {reason}");
            let _ = reply.send(Response::Error(reason));
        };
        if !self.rlv.is_on() {
            return refuse("RLV is off; `rslbot ctl rlv mode on` switches it on".to_string());
        }
        let issuer = Issuer::operator(self.inventory.agent);
        let outcomes = self.rlv_message(&issuer, text, now);
        if outcomes.is_empty() {
            return refuse(format!("there is no RLV command in \"{text}\""));
        }
        let carried_out = outcomes.iter().all(|(_, outcome)| outcome.is_ok());
        let lines: Vec<String> = outcomes
            .into_iter()
            .map(|(command, outcome)| match outcome {
                Ok(Some(said)) => format!("{command}: {said}"),
                Ok(None) => format!("{command}: ok"),
                Err(reason) => format!("{command}: {reason}"),
            })
            .collect();
        let answer = lines.join("\n");
        // The client may have gone away; the commands are dealt with.
        let _ = reply.send(match carried_out {
            true => Response::Ok(Some(answer)),
            false => Response::Error(answer),
        });
    }

    /// An object said RLV commands to its owner: now if the avatar can
    /// act, and otherwise once it can, after what waits already.
    fn rlv_heard(&mut self, issuer: Issuer, text: String, now: Instant) {
        if self.can_act() && !self.rlv.has_waiting() {
            self.rlv_message(&issuer, &text, now);
        } else {
            self.rlv.keep(issuer, text);
        }
    }

    /// Whether the avatar is in a region and not on its way out of it.
    fn can_act(&self) -> bool {
        self.session.as_ref().is_some_and(|session| session.present().is_ok())
    }

    /// Deals with the RLV messages that waited for the avatar, for as
    /// long as it can act: one of them may send it off again.
    fn rlv_waiting(&mut self, now: Instant) {
        while self.can_act() {
            let Some((issuer, text)) = self.rlv.next_waiting() else {
                return;
            };
            self.rlv_message(&issuer, &text, now);
        }
    }

    /// Deals with the RLV commands of one message, in order. Returns each
    /// with what became of it: what was said for a query, nothing more
    /// for a command that was carried out or begun, or why it was not.
    fn rlv_message(&mut self, issuer: &Issuer, text: &str, now: Instant) -> Vec<(String, Result<Option<String>, String>)> {
        let mut outcomes = Vec::new();
        for piece in rlv::parse(text) {
            let command = match piece {
                Ok(command) => command,
                Err(malformed) => {
                    info!("rlv: {issuer}: {}: malformed: {}", malformed.text, malformed.reason);
                    outcomes.push((malformed.text, Err(format!("malformed: {}", malformed.reason))));
                    continue;
                }
            };
            // A restriction and a clear are the store's alone.
            if let Some(step) = self.rlv.restrict(issuer, &command) {
                outcomes.push((
                    command.text,
                    match step {
                        Step::Failed(reason) => Err(reason),
                        _ => Ok(None),
                    },
                ));
                continue;
            }
            let outcome = self.rlv_force(issuer, &command, now);
            outcomes.push((command.text, outcome));
        }
        outcomes
    }

    /// Deals with one RLV command that is neither a restriction nor a
    /// clear, and with what becomes of it.
    fn rlv_force(&mut self, issuer: &Issuer, command: &Command, now: Instant) -> Result<Option<String>, String> {
        let follows = self.rlv.follows();
        let was_manual = self.reconciler.is_manual();
        let step = {
            let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
            rlv::deal(command, issuer, follows, &mut self.commands, live.as_mut(), now)
        };
        match step {
            Step::Said(said) => Ok(Some(said)),
            Step::Done => Ok(None),
            Step::Moved(movement) => {
                let after = self.follow.moved(movement, follows, was_manual);
                self.go_manual();
                self.after_forced(after, "", now);
                Ok(None)
            }
            Step::HoldSeat(object) => self.hold_seat(issuer, &command.text, object, now).map(|()| None),
            // Begun: nothing is persisted until the seat has been looked for.
            Step::Sought(object) => {
                self.seats_sought.push((issuer.clone(), command.clone(), object));
                Ok(None)
            }
            Step::Failed(reason) => Err(reason),
        }
    }

    /// `object` has been described, or the time to look for it is over:
    /// the commands that waited for it are dealt with again, by the
    /// restrictions in force now.
    fn looked_for(&mut self, object: Uuid, described: bool, now: Instant) {
        {
            let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
            match described {
                true => self.commands.described(object, live.as_mut(), now),
                false => self.commands.not_described(object, live.as_mut(), now),
            }
        }
        // A forced sit carried out now moves the avatar as a command does.
        self.follow_commands();
        let (found, sought) = std::mem::take(&mut self.seats_sought)
            .into_iter()
            .partition(|(_, _, sought)| *sought == object);
        self.seats_sought = sought;
        for (issuer, command, _) in found {
            // The outcome is in the log; nobody waits for it.
            let _ = self.rlv_force(&issuer, &command, now);
        }
    }

    /// A forced sit that is followed: the object becomes the held seat at
    /// once, with the place the avatar is at, and the bot seats the avatar
    /// as on any held seat. When it sits there, its seated position is
    /// persisted the way the config's setting is applied.
    fn hold_seat(&mut self, issuer: &Issuer, command: &str, object: Uuid, now: Instant) -> Result<(), String> {
        info!("rlv: {issuer}: {command}: persist cur {object}");
        match self.persist(&LocationArg::Cur, SitArg::Is(SitState::Object(object)), false, now) {
            Ok(_) => {
                self.startup = Some(Persist::On {
                    position: Source::Cur,
                    sit: Source::Cur,
                });
                self.follow_seat = true;
                self.seat_sender = Some(issuer.who());
                Ok(())
            }
            Err(reason) => {
                info!("rlv: {issuer}: {command}: refused: {reason}");
                Err(reason)
            }
        }
    }

    /// What follows a step of a forced movement: a persist of its result,
    /// or the placement mode from before it. `why` says what went wrong.
    fn after_forced(&mut self, after: After, why: &str, now: Instant) {
        match after {
            After::Nothing => {}
            After::Persist(sit) => {
                let what = format!("rlv: following: persist cur {sit}");
                info!("{what}");
                if let Err(reason) = self.persist(&LocationArg::Cur, SitArg::Is(sit), false, now) {
                    info!("{what} refused: {reason}");
                }
            }
            After::Restore { manual } => {
                let mode = if manual { "manual mode" } else { "holding" };
                info!("rlv: not following: {why}; back to {mode} as before");
                self.set_manual(manual);
            }
        }
    }

    /// Carries out an `autoreply` command; returns its answer.
    fn autoreply_command(&mut self, request: AutoreplyRequest, now: Instant) -> Result<String, String> {
        let auto = &mut self.autoreply;
        let answer = match request {
            AutoreplyRequest::Status => return Ok(auto.status(now)),
            AutoreplyRequest::Message(text) => {
                autoreply::check_message(&text)?;
                let answer = format!("autoreply: on, message: {text}");
                auto.set_text(text);
                answer
            }
            AutoreplyRequest::Pause(text) => {
                let pause = autoreply::parse_duration(&text)?;
                auto.set_pause(pause);
                format!("autoreply pause: {}", autoreply::format_duration(pause))
            }
            AutoreplyRequest::Refresh => match auto.refresh(now) {
                1 => "autoreply: forgot the pause of 1 avatar".to_string(),
                count => format!("autoreply: forgot the pauses of {count} avatars"),
            },
            AutoreplyRequest::On | AutoreplyRequest::Off => {
                let on = matches!(request, AutoreplyRequest::On);
                let state = if on { "on" } else { "off" };
                match auto.switch(on)? {
                    true => format!("autoreply: {state}"),
                    false => format!("autoreply: already {state}; nothing changed"),
                }
            }
        };
        info!("control: {answer}");
        Ok(answer)
    }

    /// An instant message arrived: RLV may hide it, and the bot may answer
    /// it by itself.
    fn im_received(&mut self, mut message: session::Message, now: Instant) {
        let session::Source::Avatar { id: from, name } = &message.source else {
            return self.record(Channel::Im, message, now);
        };
        let (from, name) = (*from, name.clone());
        self.commands.im_received(from);
        // A message stored while the avatar was offline gets no answer.
        let live = message.sent_at.is_none();
        let limits = Bot::own_limits(&self.rlv, false, None);
        let (hidden, barred) = (limits.recvim(from), limits.im(from, true));
        if let Err(refusal) = hidden {
            message.text = format!("(blocked by RLV: {})", refusal.rule);
            self.record(Channel::Im, message, now);
            if live {
                self.answer(from, name, RLV_RECVIM_REPLY.to_string(), now);
            }
            return;
        }
        self.record(Channel::Im, message, now);
        if !live {
            return;
        }
        let Some(text) = self.autoreply.answer(from, now).map(str::to_string) else {
            return;
        };
        if let Err(refusal) = barred {
            return info!(
                "autoreply: not sent to {}: {} is in force, held by {}",
                commands::whom(from, &name),
                refusal.rule,
                refusal.holder
            );
        }
        if self.answer(from, name, text, now) {
            self.autoreply.sent(from, now);
        }
    }

    /// Sends `to` an automatic answer and records it; returns whether it
    /// went out.
    fn answer(&mut self, to: Uuid, name: String, text: String, now: Instant) -> bool {
        if let Err(why) = self.send_autoresponse(to, &text, now) {
            info!("autoreply: not sent to {}: {why}", commands::whom(to, &name));
            return false;
        }
        let message = session::Message {
            kind: "im_auto".to_string(),
            source: session::Source::Avatar { id: to, name },
            sent_at: None,
            text,
        };
        self.record(Channel::Im, message, now);
        true
    }

    fn send_autoresponse(&mut self, to: Uuid, text: &str, now: Instant) -> Result<(), String> {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            return grid.autoresponse(to, text);
        }
        match self.session.as_mut() {
            Some(session) => session.autoresponse(to, text, now),
            None => Err(session::NOT_CONNECTED.to_string()),
        }
    }

    /// Puts an instant message the bot sent into the message log. The
    /// avatar it went to is in its autoreply pause from now on.
    fn record_sent(&mut self, sent: SentIm, now: Instant) {
        self.autoreply.sent(sent.to, now);
        let source = session::Source::Avatar {
            id: sent.to,
            name: sent.name,
        };
        let message = session::Message {
            kind: "im_out".to_string(),
            source,
            sent_at: None,
            text: sent.text,
        };
        self.record(Channel::Im, message, now);
    }

    fn record(&mut self, channel: Channel, message: session::Message, now: Instant) {
        let received = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.recorder.push(channel, message, received, now);
    }

    /// Sends pending name lookups on their way and writes the messages
    /// whose names are settled.
    fn service_messages(&mut self) {
        self.take_transactions(Instant::now());
        // The simulator has not said yet whether it knows names: the
        // messages wait, and so do the requests, which give up by themselves.
        if let Some(url) = self.names_url.clone() {
            // The names the message log wants, and those of the owners of
            // objects that offered inventory.
            let mut ids = self.recorder.take_lookups();
            ids.extend(self.commands.take_name_lookups());
            let unanswered = match url {
                _ if ids.is_empty() => Vec::new(),
                None => ids,
                Some(url) => match self.name_lookups.send(Lookup { url, ids }) {
                    Ok(()) => Vec::new(),
                    Err(unsent) => unsent.0.ids,
                },
            };
            if !unanswered.is_empty() {
                self.names_failed(&unanswered, Instant::now());
            }
        }
        let now = Instant::now();
        let recorded = self.recorder.flush(now);
        self.write_recorded(recorded, now);
    }

    /// Hands what the recorder released to the logs, the XMPP gateway and
    /// the pages.
    fn write_recorded(&mut self, recorded: Vec<Recorded>, now: Instant) {
        for recorded in recorded {
            match recorded {
                Recorded::Message(record) => {
                    self.logs.write(&record);
                    if let Some(xmpp) = &mut self.xmpp {
                        xmpp.record(&record, now);
                    }
                    if let Some(hub) = &mut self.web {
                        hub.record(&record);
                    }
                }
                Recorded::Transaction(transaction) => {
                    self.logs.write_transaction(&transaction);
                    if let Some(hub) = &mut self.web {
                        hub.transaction(&transaction);
                    }
                }
                // Not for the owners of the XMPP gateway.
                Recorded::Group(message) => {
                    self.logs.write_group(&message);
                    if let Some(hub) = &mut self.web {
                        hub.group(&message);
                    }
                }
            }
        }
    }

    /// Queues the transactions the payment commands recorded, and the
    /// group messages.
    fn take_transactions(&mut self, now: Instant) {
        for transaction in self.commands.take_transactions() {
            self.recorder.push_transaction(transaction, now);
        }
        for mut message in self.commands.take_group_records() {
            // A line the bot said carries the avatar's own name.
            if message.kind == grouplog::Kind::ChatOut && message.avatar_name.is_empty() {
                message.avatar_name = format!("{} {}", self.config.first_name, self.config.last_name);
            }
            self.recorder.push_group(message, now);
        }
    }

    /// The grid named these avatars and does not know those: the message
    /// log and the requests that waited for a name go on.
    fn names_resolved(&mut self, found: Vec<(Uuid, names::Names)>, missing: Vec<Uuid>, now: Instant) {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            self.commands.names_resolved(&found, &missing, Some(grid), now);
            self.recorder.names_resolved(found, missing, now);
            return;
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.names_resolved(&found, &missing, live.as_mut(), now);
        self.recorder.names_resolved(found, missing, now);
    }

    /// A name lookup came to nothing.
    fn names_failed(&mut self, ids: &[Uuid], now: Instant) {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            self.commands.names_failed(ids, Some(grid), now);
            self.recorder.lookup_failed(ids);
            return;
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        self.commands.names_failed(ids, live.as_mut(), now);
        self.recorder.lookup_failed(ids);
    }

    /// Writes every message still waiting for names; for use on the way out.
    fn flush_messages(&mut self) {
        let now = Instant::now();
        // A payment that waits for the grid is recorded as unconfirmed.
        self.commands.money_stop();
        self.take_transactions(now);
        let recorded = self.recorder.flush_all(now);
        self.write_recorded(recorded, now);
    }

    // ------------------------------------------------------------- XMPP

    /// The presence of the gateway's addresses for the bot's state now.
    fn xmpp_wanted(&self) -> xmpp::presence::Presence {
        let region = self.session.as_ref().and_then(|session| session.region());
        xmpp::presence::presence(region, self.stopping)
    }

    /// Sends the owners what changed of the gateway's presence.
    fn xmpp_presence(&mut self) {
        let wanted = self.xmpp_wanted();
        if let Some(xmpp) = &mut self.xmpp {
            xmpp.update_presence(&wanted);
        }
    }

    /// Something from the component connection; an owner's request is
    /// carried out as the control socket's are.
    fn xmpp_event(&mut self, event: xmpp::conn::XmppEvent, now: Instant) -> Flow {
        let wanted = self.xmpp_wanted();
        let Some(xmpp) = &mut self.xmpp else {
            return Flow::Continue;
        };
        let Some(command) = xmpp.handle(event, &wanted) else {
            return Flow::Continue;
        };
        let xmpp::gateway::Command { request, reply, waiting } = command;
        let flow = self.control_as(request, reply, false, Interface::Xmpp, now);
        if let Some(xmpp) = &mut self.xmpp {
            xmpp.waited(waiting);
        }
        flow
    }

    /// The bot is done: the gateway's addresses go offline.
    fn xmpp_stop(&mut self) {
        if let Some(xmpp) = &mut self.xmpp {
            xmpp.stop();
        }
    }

    // -------------------------------------------------------------- web

    /// Something from the web interface: a page's command is carried out
    /// as the control socket's are, and a page that opens its stream gets
    /// what it missed and the status.
    fn web_event(&mut self, event: web::WebEvent, now: Instant) -> Flow {
        match event {
            web::WebEvent::Command { request, reply, own } => self.control_as(request, reply, own, Interface::Web, now),
            web::WebEvent::Subscribe { last, page } => {
                let status = self.status(Interface::Web);
                if let Some(hub) = &mut self.web {
                    hub.subscribe(last.as_deref(), page, &status);
                }
                Flow::Continue
            }
        }
    }

    /// Sends the open pages the status when it has changed, and the word
    /// that the groups changed. Nothing is put together while no page is
    /// open.
    fn web_status(&mut self) {
        if self.commands.take_groups_changed() {
            if let Some(hub) = &mut self.web {
                hub.groups_changed();
            }
        }
        if self.web.as_ref().is_some_and(web::hub::Hub::watched) {
            let status = self.status(Interface::Web);
            if let Some(hub) = &mut self.web {
                hub.status(&status);
            }
        }
    }

    /// The bot is done: the pages' streams end.
    fn web_stop(&mut self) {
        if let Some(hub) = &mut self.web {
            hub.close();
        }
    }

    fn apply(&mut self, event: SessionEvent, now: Instant) -> Flow {
        let manual = self.reconciler.is_manual();
        match event {
            SessionEvent::RegionEntered(name) => {
                self.reconciler.region_entered(&name, now);
                // Without a position in the config, where the first login
                // put the avatar is the location to hold.
                let position = self.session.as_ref().and_then(Session::position).unwrap_or_default();
                if self.book.first_location(Location {
                    region: name.clone(),
                    position,
                }) {
                    self.sync_goal(now);
                    info!("placement: holding {}", self.book.held());
                }
                match (&self.reconciler.goal().region, self.reconciler.in_target_region()) {
                    (None, _) => info!("connected, in region \"{name}\""),
                    (Some(_), true) => info!("connected, in region \"{name}\" (the target region)"),
                    (Some(_), false) => info!("connected, in region \"{name}\" (not the target region)"),
                }
                self.commands.arrived(&name);
                self.commands.entered_region(now);
                self.ask_balance(now);
                let after = self.follow.arrived();
                self.after_forced(after, "", now);
            }
            SessionEvent::Lost(reason) => {
                let after = self.follow.failed();
                self.after_forced(after, "the session was lost", now);
                let delay = self.reconciler.session_lost(now);
                self.drop_session();
                if self.stopping {
                    info!("session lost while logging out: {reason}");
                    return Flow::Exit(EXIT_OK);
                }
                info!("session lost: {reason}; next login in {}", seconds(delay));
            }
            SessionEvent::Seated(sat) => {
                // The prim the simulator named is the seat from here on.
                let object = sat.prim;
                if let Some(region) = self.session.as_ref().and_then(Session::region) {
                    // For `persist`: this object is in this region.
                    self.book.seated(object, region);
                    if let Some(requested) = sat.requested {
                        self.book.seated(requested, region);
                    }
                }
                let held = self.book.held().sit;
                if on_target(held, Seat::Object(sat)) {
                    self.reconciler.seated(now);
                    info!("seated on {object}");
                } else {
                    // Not the seat the bot holds: as good as standing to it.
                    self.reconciler.seated_elsewhere(now);
                    match held {
                        SitState::Object(_) => info!("seated on {object}, which is not the target"),
                        _ => info!("seated on {object}"),
                    }
                }
                self.commands.seated(object);
            }
            SessionEvent::Standing => match self.reconciler.standing(now) {
                Some(_) if manual => info!("unseated"),
                Some(delay) => info!("unseated; sitting again in {}", seconds(delay)),
                None if matches!(self.book.held().sit, SitState::Object(_)) => info!("not seated on the target"),
                None => info!("not seated on an object"),
            },
            SessionEvent::SitFailed(reason) => {
                // A sit of the operator's is answered to the operator; only
                // the bot's own count towards its backoff.
                if self.commands.sit_failed(&reason) || manual {
                    info!("sit failed: {reason}");
                } else {
                    let delay = self.reconciler.sit_failed(now);
                    info!("sit failed: {reason}; next attempt in {}", seconds(delay));
                }
            }
            SessionEvent::TeleportArrived => {
                self.reconciler.teleport_arrived();
                info!("teleport arrived");
                let region = self.session.as_ref().and_then(Session::region).unwrap_or_default().to_string();
                self.commands.arrived(&region);
                let after = self.follow.arrived();
                self.after_forced(after, "", now);
            }
            SessionEvent::TeleportFailed(reason) => {
                if self.commands.teleport_failed(&reason) || manual {
                    info!("teleport failed: {reason}");
                } else {
                    let delay = self.reconciler.teleport_failed(now);
                    info!("teleport failed: {reason}; next attempt in {}", seconds(delay));
                }
                let after = self.follow.failed();
                self.after_forced(after, "the teleport failed", now);
            }
            SessionEvent::TeleportOffer { from, name, lure } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands.teleport_offer(from, &name, lure, live.as_mut(), now);
                // One accepted by rule moves the avatar as a command does.
                self.follow_commands();
            }
            SessionEvent::InventoryOffer(offer) => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands.inventory_offer(offer, live.as_mut(), now);
            }
            SessionEvent::PermissionRequest {
                object,
                item,
                object_name,
                owner_name,
                permissions,
            } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands
                    .permission_request(object, item, &object_name, &owner_name, permissions, live.as_mut(), now);
            }
            SessionEvent::ObjectOwner {
                object,
                owner,
                group,
                name,
            } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands.object_owner(object, owner, group, live.as_mut(), now);
                self.commands.pay_owner(object, owner, group, name.as_deref(), live.as_mut(), now);
            }
            SessionEvent::PayPrice { object, default, buttons } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands.pay_price(object, default, &buttons, live.as_mut(), now);
            }
            SessionEvent::Money {
                balance,
                success,
                description,
                info,
            } => {
                let own = self.inventory.agent;
                self.commands.money_report(balance, success, &description, info.as_ref(), own);
                self.take_transactions(now);
            }
            SessionEvent::TeleportRequested { from, name, text } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands.teleport_requested(from, &name, &text, live.as_mut(), now);
            }
            SessionEvent::Touched(object) => self.commands.touched(object),
            // What an object held ends with the object, in every mode.
            SessionEvent::ObjectGone(object) => self.rlv.object_gone(object),
            SessionEvent::RegionLeft { worn } => self.rlv.region_left(&worn),
            SessionEvent::Described(object) => self.looked_for(object, true, now),
            SessionEvent::NotDescribed(object) => self.looked_for(object, false, now),
            SessionEvent::TouchFailed(object) => {
                let reason = match &self.session {
                    Some(session) => session.unknown_object(object),
                    None => "the session was lost".to_string(),
                };
                self.commands.touch_failed(object, &reason);
            }
            SessionEvent::Alert(message) => {
                info!("simulator alert: {message}");
                self.commands.alert(&message);
            }
            SessionEvent::InventoryUrls(urls) => {
                if urls.descendents.is_none() || urls.items.is_none() {
                    debug!("the simulator has no inventory service");
                } else if !urls.outfit_service() {
                    debug!("the simulator has no outfit service: clothing cannot be changed here");
                }
                self.inventory.urls = Some(urls);
            }
            SessionEvent::Attached { item, point } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                if !self.commands.attached(item, point, live.as_mut(), now) {
                    info!("attachment: item {item} is on {}", inventory::point_name(point));
                }
            }
            SessionEvent::Detached { item, point } => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                if !self.commands.detached(item, live.as_mut(), now) {
                    info!("attachment: item {item} is gone from {}", inventory::point_name(point));
                }
            }
            SessionEvent::AttachFailed(item) => self.commands.attach_failed(item),
            SessionEvent::DetachFailed(item) => self.commands.detach_failed(item),
            SessionEvent::Chat(message) => {
                // What an object says to its owner may be RLV commands; it
                // is recorded like any other chat all the same.
                let rlv = rlv::sender(&message).filter(|_| self.rlv.is_on());
                let text = message.text.clone();
                self.record(Channel::Chat, message, now);
                if let Some(issuer) = rlv {
                    self.rlv_heard(issuer, text, now);
                }
            }
            SessionEvent::InstantMessage(message) => self.im_received(message, now),
            SessionEvent::DisplayNamesUrl(url) => {
                if url.is_none() {
                    debug!("the simulator has no display-name service");
                }
                self.names_url = Some(url);
            }
            SessionEvent::AvatarSearchUrl(url) => {
                if url.is_none() {
                    debug!("the simulator has no avatar search service");
                }
                self.lookups.search_url = url;
            }
            SessionEvent::AvatarsFound { query, avatars } => self.avatars_found(query, &avatars, now),
            SessionEvent::ProfilesUrl(url) => {
                if url.is_none() {
                    debug!("the simulator has no profile service");
                }
                self.lookups.profile_url = url;
            }
            SessionEvent::Profile(profile) => self.commands.profile_received(profile),
            event @ (SessionEvent::Groups(_)
            | SessionEvent::GroupDropped(_)
            | SessionEvent::ActiveGroup { .. }
            | SessionEvent::GroupProfile(_)
            | SessionEvent::Joined { .. }
            | SessionEvent::Left { .. }
            | SessionEvent::GroupInvitation { .. }
            | SessionEvent::GroupSessionOffered(_)
            | SessionEvent::GroupChat { .. }
            | SessionEvent::GroupNotice { .. }
            | SessionEvent::GroupSession { .. }) => self.group_event(event, now),
            SessionEvent::ChatSessionsUrl(url) => {
                if url.is_none() {
                    debug!("the simulator has no service for taking chat sessions up");
                }
                self.lookups.chat_sessions_url = url;
            }
            SessionEvent::FriendshipEnded(id) => info!("{id} ended the friendship with the avatar"),
            SessionEvent::FriendshipBegun(id) => info!("{id} accepted the avatar's offer of friendship"),
            SessionEvent::LoggedOut => {
                info!("logged out");
                return Flow::Exit(EXIT_OK);
            }
        }
        Flow::Continue
    }

    /// What the grid says about the avatar's groups, group chat, notices
    /// and invitations. Group messages are recorded; none is answered.
    fn group_event(&mut self, event: SessionEvent, now: Instant) {
        #[cfg(test)]
        if let Some(grid) = self.grid.as_mut() {
            Bot::group_event_on(&mut self.commands, Some(grid), event, now);
            return self.take_transactions(now);
        }
        let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
        Bot::group_event_on(&mut self.commands, live.as_mut(), event, now);
        self.take_transactions(now);
    }

    fn group_event_on<G: Grid>(commands: &mut Commands, grid: Option<&mut G>, event: SessionEvent, now: Instant) {
        match event {
            SessionEvent::Groups(list) => commands.groups_listed(list, grid, now),
            SessionEvent::GroupDropped(group) => commands.group_dropped(group, now),
            SessionEvent::ActiveGroup { group, title, name } => commands.active_group(group, &title, &name),
            SessionEvent::GroupProfile(profile) => commands.group_profile(profile, grid, now),
            SessionEvent::Joined { group, success } => commands.joined(group, success),
            SessionEvent::Left { group, success } => commands.left(group, success, now),
            SessionEvent::GroupInvitation {
                group,
                id,
                inviter,
                fee,
                text,
            } => {
                debug!("invitation from {inviter} into the group {group}: {text}");
                commands.group_invitation(group, id, &inviter, fee, grid, now);
            }
            SessionEvent::GroupSessionOffered(session) => commands.session_offered(session, grid),
            SessionEvent::GroupChat { session, from, name, text } => commands.group_line(session, from, &name, &text, grid),
            SessionEvent::GroupNotice {
                group,
                sender,
                name,
                text,
                bucket,
                sent_at,
            } => commands.group_notice(group, sender, &name, &text, &bucket, sent_at),
            SessionEvent::GroupSession { group, ok, error } => commands.group_session(group, ok, &error, grid, now),
            _ => {}
        }
    }

    /// Runs timers and applies whatever the session has to report.
    fn service_session(&mut self) -> Flow {
        let now = Instant::now();
        self.follow_simulator();
        let Some(session) = &mut self.session else {
            return Flow::Continue;
        };
        session.tick(now);
        for event in session.take_events() {
            if let Flow::Exit(code) = self.apply(event, now) {
                return Flow::Exit(code);
            }
        }
        self.rlv_waiting(now);
        Flow::Continue
    }

    /// Starts the next action, if any. Returns when to look again.
    fn reconcile(&mut self) -> Result<Option<Instant>, i32> {
        loop {
            let now = Instant::now();
            self.owed_stand(now);
            let action = match self.reconciler.decide(now) {
                Decision::Wait(until) => {
                    self.not_held_back();
                    if self.exempt_placement && !self.stand_owed && self.in_place() {
                        // Reached: from here on the restrictions hold again.
                        self.exempt_placement = false;
                    }
                    return Ok(until);
                }
                Decision::Exit { code, message } => {
                    info!("giving up: {message}");
                    return Err(code);
                }
                Decision::Start(action) => action,
            };
            if self.held_back(action) {
                // Not started, so not a failed attempt: the next pass of
                // the main loop looks again.
                return Ok(None);
            }
            self.reconciler.started(action);
            match (action, &mut self.session) {
                (Action::Login(start), _) => self.login(start),
                (Action::Teleport, Some(session)) => {
                    // The reconciler asks for this only while a location is held.
                    let Some(Location { region, position }) = &self.book.held().location else {
                        return Ok(None);
                    };
                    let [x, y, z] = position.map(persist::coordinate);
                    info!("teleporting to \"{region}\" at {x}, {y}, {z}");
                    session.start_teleport(region, *position, now);
                }
                (Action::Sit, Some(session)) => {
                    // Likewise only while an object is held as the seat.
                    let SitState::Object(object) = self.book.held().sit else {
                        return Ok(None);
                    };
                    info!("sitting on {object}");
                    session.start_sit(object, now);
                }
                (Action::SitGround, Some(session)) => {
                    // The simulator does not answer this one; sent is done.
                    match session.sit_ground(now) {
                        Ok(()) => info!("sitting on the ground"),
                        Err(reason) => info!("cannot sit on the ground: {reason}"),
                    }
                }
                // The reconciler only asks for these while connected.
                (Action::Teleport | Action::Sit | Action::SitGround, None) => return Ok(None),
            }
        }
    }

    /// Whether the avatar is where the bot holds it, as far as the bot's
    /// own steps go.
    fn in_place(&self) -> bool {
        let seated = match self.reconciler.goal().seat {
            Some(reconciler::SeatGoal::Object) => self.reconciler.seated_on_target(),
            Some(reconciler::SeatGoal::Ground) => self.reconciler.ground_sat(),
            None => true,
        };
        self.reconciler.in_target_region() && seated
    }

    /// No step of the bot's own is held back any more.
    fn not_held_back(&mut self) {
        if let Some((what, _)) = self.held_step.take() {
            info!("rlv: no longer holding back {what}");
        }
    }

    /// Whether an RLV restriction holds back the step the bot is about to
    /// start. A login never is. The log says so once, and names the
    /// restriction that the persist override setting lets a step past.
    fn held_back(&mut self, action: Action) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let held = self.book.held();
        let (what, step) = match (action, &held.location, held.sit) {
            (Action::Teleport, Some(Location { region, position }), _) => {
                let here = session.region().is_some_and(|here| reconciler::same_region(here, region));
                let apart = |from: Vector3| [0, 1, 2].map(|i| (position[i] - from[i]).powi(2)).iter().sum::<f32>().sqrt();
                let local = here.then(|| session.position().map_or(f32::INFINITY, apart));
                (format!("the teleport to \"{region}\""), OwnStep::Teleport { local })
            }
            (Action::Sit, _, SitState::Object(object)) => {
                let distance = session.thing(object).distance;
                (format!("the sit on {object}"), OwnStep::Sit { distance })
            }
            (Action::SitGround, _, _) => ("the sit on the ground".to_string(), OwnStep::SitGround),
            _ => return false,
        };
        let limits = Bot::own_limits(&self.rlv, self.exempt_placement, self.seat_sender);
        // A seat the simulator has not described may be near enough once
        // it is: the scan is to find it. The step waits as long as it takes.
        let look = match held.sit {
            SitState::Object(object) if matches!(action, Action::Sit) => {
                let thing = session.thing(object);
                matches!(limits.sit_on(sitting(session), &thing), Check::LookFirst(_))
            }
            _ => false,
        };
        let verdict = rlv::own_step(limits, sitting(session), step, self.rlv.overridden());
        if look && matches!(verdict, Verdict::HeldBack(_)) {
            if let Some(session) = self.session.as_mut() {
                session.scan(Instant::now());
            }
        }
        match verdict {
            Verdict::Go => {
                self.not_held_back();
                false
            }
            Verdict::LetThrough(refusal) => {
                self.not_held_back();
                info!(
                    "rlv: {what} goes ahead although {}: persist override {}",
                    refusal.short(),
                    step.sort()
                );
                false
            }
            Verdict::HeldBack(refusal) => {
                let why = refusal.short();
                if self.held_step.as_ref() != Some(&(what.clone(), why.clone())) {
                    self.not_held_back();
                    info!("rlv: holding back {what}: {why}");
                    self.held_step = Some((what, why));
                }
                true
            }
        }
    }

    fn stop(&mut self) -> Flow {
        if self.stopping {
            return Flow::Exit(EXIT_OK);
        }
        self.stopping = true;
        // A payment that waits for the grid is answered, and recorded as
        // unconfirmed, while whoever sent it can still be told.
        self.commands.money_stop();
        self.take_transactions(Instant::now());
        match &mut self.session {
            Some(session) => {
                info!("stop requested; logging out");
                session.start_logout(Instant::now());
                self.rlv.forget_waiting();
                self.rlv.session_ended();
                Flow::Continue
            }
            None => {
                info!("stop requested");
                Flow::Exit(EXIT_OK)
            }
        }
    }

    fn handle(&mut self, event: Event) -> Flow {
        let now = Instant::now();
        match (event, &mut self.session) {
            (Event::Signal, _) => return self.stop(),
            (Event::Control { request, reply }, _) => return self.control(request, reply, now),
            (Event::Xmpp(event), _) => return self.xmpp_event(event, now),
            (Event::Web(event), _) => return self.web_event(event, now),
            (Event::AvatarsFound { query, avatars }, _) => self.avatars_found(query, &avatars, now),
            (Event::AvatarSearchFailed { query, reason }, _) => self.commands.lookup_failed(query, &reason),
            (Event::Datagram { generation, data }, Some(session)) => session.handle_datagram(generation, &data, now),
            (Event::Caps { generation, event }, Some(session)) => session.handle_caps(generation, event, now),
            (Event::Names { found, missing }, _) => self.names_resolved(found, missing, now),
            (Event::NamesFailed(ids), _) => self.names_failed(&ids, now),
            (Event::Profile { result: Ok(profile), .. }, _) => self.commands.profile_received(profile),
            (Event::Profile { id, result: Err(reason) }, _) => self.commands.profile_failed(id, &reason),
            // An answer about another simulator's inventory service is of
            // no use: the commands that asked were told so.
            (Event::Inventory { generation, .. }, Some(session)) if generation != session.generation() => {}
            (Event::Inventory { id, result, .. }, _) => {
                let mut live = live(&mut self.session, &self.lookups, &self.inventory, &self.rlv);
                self.commands.inventory_answer(id, result, live.as_mut(), now);
            }
            // Left over from a session that is already gone.
            (_, None) => {}
        }
        Flow::Continue
    }
}

/// The placement the config describes: what the bot holds when it starts.
fn config_placement(config: &Config) -> Placement {
    let location = match &config.place {
        Place::Region { name, position } => Some(Location {
            region: name.clone(),
            position: *position,
        }),
        Place::Home | Place::Latest => None,
    };
    let sit = match config.sit {
        Some(Sit::Object(object)) => SitState::Object(object),
        Some(Sit::Ground) => SitState::Ground,
        None => SitState::Stand,
    };
    Placement { location, sit }
}

/// Where a login asks to be placed while no location is held.
fn config_start(config: &Config) -> login::Start {
    match config.place {
        Place::Home => login::Start::Home,
        Place::Region { .. } | Place::Latest => login::Start::Last,
    }
}

/// What the reconciler needs to know of the held placement. `start` is
/// where a login goes while no location is held.
fn goal(held: &Placement, start: login::Start) -> Goal {
    let seat = match held.sit {
        SitState::Object(_) => Some(SeatGoal::Object),
        SitState::Ground => Some(SeatGoal::Ground),
        SitState::Stand | SitState::Unchanged => None,
    };
    match &held.location {
        Some(location) => Goal {
            region: Some(location.region.clone()),
            start: login::Start::Target,
            seat,
        },
        None => Goal { region: None, start, seat },
    }
}

/// Whether the avatar is seated on the object `sit` names: the simulator
/// named that object, or answered the bot's request for it.
fn on_target(sit: SitState, seat: Seat) -> bool {
    matches!((sit, seat), (SitState::Object(target), Seat::Object(sat)) if sat.is_on(target))
}

/// Whether a persist with this sit state has to stand the avatar up:
/// `stand` when it sits, on an object or on the ground by the bot's doing,
/// and `ground` when it is on an object, so that the ground sit can follow.
fn needs_stand(sit: SitState, seat: Seat, on_ground: bool) -> bool {
    let on_object = matches!(seat, Seat::Object(_) | Seat::Other);
    match sit {
        SitState::Stand => on_object || on_ground,
        SitState::Ground => on_object,
        SitState::Object(_) | SitState::Unchanged => false,
    }
}

/// Where the avatar is, for `status`.
struct Whereabouts<'a> {
    region: &'a str,
    position: Option<Vector3>,
    seat: Seat,
    /// What the object the avatar sits on is called, once the simulator
    /// has said.
    seat_name: Option<&'a str>,
}

/// The text of the `status` answer. `here` is missing while the avatar is
/// not in a region.
fn status_text(here: Option<Whereabouts>, held: &Placement, manual: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("connected: {}", if here.is_some() { "yes" } else { "no" }));
    if let Some(Whereabouts {
        region,
        position,
        seat,
        seat_name,
    }) = here
    {
        lines.push(format!("region: {region}"));
        if let Some(position) = position {
            let [x, y, z] = position.map(persist::coordinate);
            lines.push(format!("position: {x}, {y}, {z}"));
        }
        let on_object = matches!(seat, Seat::Object(_) | Seat::Other);
        let seat = match seat {
            Seat::Unknown => "unknown".to_string(),
            Seat::None => "none".to_string(),
            Seat::Object(_) if on_target(held.sit, seat) => "target".to_string(),
            Seat::Object(sat) => format!("object {}", sat.prim),
            Seat::Other => "other".to_string(),
        };
        lines.push(format!("seat: {seat}"));
        if let Some(name) = seat_name.filter(|_| on_object) {
            lines.push(format!("seat name: {}", escape::escape(name)));
        }
    }
    lines.push(format!("persist: {held}"));
    lines.push(format!("mode: {}", if manual { "manual" } else { "holding" }));
    lines.join("\n")
}

/// The answer to `status`: `connected`, then the avatar's name, then the
/// rest of `place`, and the money lines when given: the balance as far as
/// known, the `limit` line's value, and whether RLV holds back the
/// descriptions of payments.
fn status_answer(avatar: &str, place: &str, money: Option<(Option<i64>, String, bool)>) -> String {
    let (connected, rest) = place.split_once('\n').unwrap_or((place, ""));
    let mut lines = vec![connected.to_string(), format!("avatar: {avatar}")];
    lines.extend(rest.lines().map(String::from));
    if let Some((balance, limit, blocked)) = money {
        let balance = balance.map_or("unknown".to_string(), |balance| format!("L${balance}"));
        lines.push(format!("balance: {balance}"));
        lines.push(format!("limit: {limit}"));
        if blocked {
            lines.push("pay description: blocked by RLV".to_string());
        }
    }
    lines.join("\n")
}

/// The payee of a `pay` as the log names it before it is resolved.
fn payee_words(payee: &PayTo) -> String {
    match payee {
        PayTo::Avatar(given) => format!("\"{given}\""),
        PayTo::Object(object) => format!("the object {object}"),
    }
}

/// What the bot sees of the avatar when it asks whether the config
/// placement is reached.
#[derive(Debug, Clone, Copy)]
struct Arrival {
    /// In a region and not on the way out of it.
    present: bool,
    in_target_region: bool,
    seated_on_target: bool,
    /// The ground sit has been handed to the simulator.
    ground_sat: bool,
    /// The seated avatar's position has been worked out from its seat.
    seat_frame_known: bool,
}

#[derive(Debug, PartialEq)]
enum Startup {
    /// The config placement is not reached.
    NotYet,
    /// It is, but for the seat's position; look again then.
    Wait(Instant),
    /// Apply the setting now. `settled` is false when the seat's position
    /// was waited for in vain.
    Now { settled: bool },
}

/// Whether the config's `persist` is due. `sit` is the config's sit state;
/// `rest` says that the rest of the startup script waits for this too;
/// `since` is when the wait for the seat's position began, if it has.
fn startup_due(sit: SitState, setting: Persist, rest: bool, arrival: Arrival, since: Option<Instant>, now: Instant) -> Startup {
    if !arrival.present || !arrival.in_target_region {
        return Startup::NotYet;
    }
    let reached = match sit {
        SitState::Object(_) => arrival.seated_on_target,
        SitState::Ground => arrival.ground_sat,
        SitState::Stand | SitState::Unchanged => true,
    };
    if !reached {
        return Startup::NotYet;
    }
    // Only a `cur` location is about where the avatar sits, and a script
    // may ask for one.
    let wants_position = rest || matches!(setting, Persist::On { position: Source::Cur, .. });
    if wants_position && matches!(sit, SitState::Object(_)) && !arrival.seat_frame_known {
        let deadline = since.unwrap_or(now) + SEAT_POSITION_WAIT;
        if now < deadline {
            return Startup::Wait(deadline);
        }
        return Startup::Now { settled: false };
    }
    Startup::Now { settled: true }
}

/// Seconds since the Unix epoch.
fn wall_clock() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Opens the log files and reads the transactions file back. An error
/// names the key the config gave the file under.
fn open_logs(config: &Config) -> Result<(Logs, txlog::ReadBack), String> {
    let back = match &config.logs.transactions {
        Some(log) => txlog::read(&log.path).map_err(|e| format!("`{}`: {e}", log.key))?,
        None => txlog::ReadBack::default(),
    };
    let logs = Logs::open(&config.logs).map_err(|e| format!("`{}`: {}", e.key, e.message))?;
    if back.skipped > 0 {
        if let Some(log) = &config.logs.transactions {
            info!("{}: skipped {} row(s) that could not be read", log.path.display(), back.skipped);
        }
    }
    Ok((logs, back))
}

/// The limit as the transactions file leaves it at `now`, and the lines
/// the bot writes about payments when it starts.
fn payments_start(config: &Config, back: &txlog::ReadBack, now: u64) -> (Ledger, Vec<String>) {
    let Some(payments) = &config.payments else {
        return (Ledger::new(None, None), Vec::new());
    };
    let mut ledger = Ledger::from_config(payments);
    ledger.replay(back.counted(), now);
    let interfaces: Vec<String> = payments.interfaces().iter().map(|i| format!("`{i}`")).collect();
    let who = match interfaces.len() {
        0 => "no interface may pay".to_string(),
        1 => format!("{} may pay", interfaces[0]),
        n => format!("{} and {} may pay", interfaces[..n - 1].join(", "), interfaces[n - 1]),
    };
    let limit = match (payments.limit, payments.renew) {
        (None, _) => "no limit".to_string(),
        (Some(limit), Some(renew)) => format!(
            "a limit of L${limit} every {}, {} left",
            autoreply::format_duration(renew),
            payments::limit_text(&ledger, now)
        ),
        (Some(limit), None) => format!("a limit of L${limit}, {} left", payments::limit_text(&ledger, now)),
    };
    let mut lines = vec![format!("payments: {who}, with {limit}")];
    let open_web = config.web.as_ref().is_some_and(|web| web.password.is_none());
    if payments.may(Interface::Web) && open_web {
        lines.push(
            "warning: `web` may pay and `[web]` has no `password`: whoever reaches the web interface \
             can pay; set `web.password`"
                .to_string(),
        );
    }
    (ledger, lines)
}

fn run_bot(config_path: &Path) -> i32 {
    let config = match Config::load(config_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("rslbot: {e}");
            return EXIT_CONFIG;
        }
    };
    // Before anything touches the network: a log that cannot be written,
    // or a transactions file that is something else, is a config error.
    let (logs, back) = match open_logs(&config) {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("rslbot: {}: {e}", config_path.display());
            return EXIT_CONFIG;
        }
    };
    let (events, incoming) = mpsc::channel();
    // Held until the bot is done: dropping it removes the socket file.
    let _control = if config.socket {
        match control::address().and_then(|address| control::listen(&address, events.clone())) {
            Ok(server) => Some(server),
            Err(e) => {
                eprintln!("rslbot: {}: `socket`: {e}", config_path.display());
                return EXIT_CONFIG;
            }
        }
    } else {
        None
    };
    // Likewise held until the bot is done, and likewise a config error
    // when its address cannot be listened on.
    let web = match &config.web {
        Some(web) => match web::listen(web, events.clone()) {
            Ok(server) => Some(server),
            Err(e) => {
                eprintln!("rslbot: {}: `web.listen`: {e}", config_path.display());
                return EXIT_CONFIG;
            }
        },
        None => None,
    };
    install_signal_handler(events.clone());
    for note in &config.deprecated {
        info!("{}: {note}", config_path.display());
    }
    let mut bot = Bot::new(config, logs, events);
    bot.start_payments(&back, wall_clock());
    // Still before anything touches the network: a setting command of the
    // startup script that is refused is a config error.
    if let Err(e) = bot.startup_head(Instant::now()) {
        let key = bot.config.startup_key();
        eprintln!("rslbot: {}: `{key}`: line {}: {}", config_path.display(), e.line, e.reason);
        return EXIT_CONFIG;
    }
    bot.start_gateway();
    let code = run_loop(&mut bot, &incoming);
    bot.flush_messages();
    bot.xmpp_stop();
    bot.web_stop();
    if let Some(server) = &web {
        server.stop();
    }
    code
}

fn run_loop(bot: &mut Bot, incoming: &Receiver<Event>) -> i32 {
    loop {
        if let Flow::Exit(code) = bot.service_session() {
            return code;
        }
        bot.commands.tick(Instant::now());
        bot.decide_requests(Instant::now());
        bot.scan_objects(Instant::now());
        bot.restore_attachments(Instant::now());
        bot.pay_due(Instant::now());
        bot.service_messages();
        bot.xmpp_presence();
        bot.web_status();
        let mut wake = Instant::now() + IDLE_WAIT;
        if !bot.stopping {
            match bot.reconcile() {
                Ok(Some(until)) => wake = wake.min(until),
                Ok(None) => {}
                Err(code) => return code,
            }
            let pending = bot.startup.is_some();
            if let Some(until) = bot.apply_startup(Instant::now()) {
                wake = wake.min(until);
            }
            if pending && bot.startup.is_none() {
                // What the bot holds may have changed: decide again.
                continue;
            }
            match bot.startup_rest() {
                // A command of the script may have changed it as well.
                Ok(true) => continue,
                Ok(false) => {}
                Err(code) => return code,
            }
        }
        if let Some(session) = &bot.session {
            wake = wake.min(session.next_deadline());
        }
        if let Some(deadline) = bot.recorder.next_deadline() {
            wake = wake.min(deadline);
        }
        if let Some(deadline) = bot.commands.next_deadline() {
            wake = wake.min(deadline);
        }
        match incoming.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(event) => {
                if let Flow::Exit(code) = bot.handle(event) {
                    return code;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Cannot happen while `bot` holds a sender.
            Err(RecvTimeoutError::Disconnected) => return EXIT_FATAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn everything_after_ctl_belongs_to_the_command() {
        let args = strings(&["ctl", "say", "-v", "/1", "x", "--help"]);
        assert_eq!(ctl_arguments(&args), Some(&args[1..]));
        assert_eq!(ctl_arguments(&strings(&["ctl"])), Some(&[][..]));
        // Only as the first argument.
        assert_eq!(ctl_arguments(&strings(&["-v", "ctl", "say", "x"])), None);
        assert_eq!(ctl_arguments(&strings(&["bot.toml"])), None);
        assert_eq!(ctl_arguments(&[]), None);
    }

    #[test]
    fn ctl_without_a_command_is_a_usage_error() {
        assert_eq!(run_cli(strings(&["ctl"])), EXIT_CONFIG);
    }

    // ------------------------------------------------------- placement

    const OBJECT: &str = "11111111-2222-3333-4444-555555555555";
    const ACCOUNT: &str = "user = \"Sit Bot\"\npassword = \"secret\"\n";

    fn object() -> Uuid {
        Uuid::parse_str(OBJECT).unwrap()
    }

    /// A config with `region`, `position` and an object as `sit`, plus `more`.
    fn full(more: &str) -> Config {
        let text = format!("{ACCOUNT}region = \"Target\"\nposition = [128, 128, 25]\nsit = \"{OBJECT}\"\n{more}");
        Config::parse(&text).unwrap()
    }

    fn at(region: &str, x: f32, y: f32, z: f32) -> Location {
        Location {
            region: region.to_string(),
            position: [x, y, z],
        }
    }

    fn sat(prim: Uuid, requested: Option<Uuid>) -> Seat {
        Seat::Object(session::Sat { prim, requested })
    }

    #[test]
    fn the_config_describes_the_placement_to_start_with() {
        let placement = config_placement(&full(""));
        assert_eq!(
            placement,
            Placement {
                location: Some(at("Target", 128.0, 128.0, 25.0)),
                sit: SitState::Object(object())
            }
        );
        let ground = Config::parse(&format!("{ACCOUNT}sit = \"ground\"\nstart = \"home\"\n")).unwrap();
        assert_eq!(
            config_placement(&ground),
            Placement {
                location: None,
                sit: SitState::Ground
            }
        );
        assert_eq!(config_start(&ground), login::Start::Home);
        let bare = Config::parse(ACCOUNT).unwrap();
        assert_eq!(
            config_placement(&bare),
            Placement {
                location: None,
                sit: SitState::Stand
            }
        );
        assert_eq!(config_start(&bare), login::Start::Last);
    }

    #[test]
    fn the_goal_follows_the_held_placement() {
        let held = |location: Option<Location>, sit| Placement { location, sit };
        let here = Some(at("Blue Lagoon", 1.0, 2.0, 3.0));
        let seats = [
            (SitState::Object(object()), Some(SeatGoal::Object)),
            (SitState::Ground, Some(SeatGoal::Ground)),
            (SitState::Stand, None),
            (SitState::Unchanged, None),
        ];
        for (sit, seat) in seats {
            // With a location: its region, and logins go there whatever the config says.
            let with = goal(&held(here.clone(), sit), login::Start::Home);
            assert_eq!(
                with,
                Goal {
                    region: Some("Blue Lagoon".to_string()),
                    start: login::Start::Target,
                    seat
                }
            );
            // Without one: no region, and logins go where the config's `start` says.
            let without = goal(&held(None, sit), login::Start::Home);
            assert_eq!(
                without,
                Goal {
                    region: None,
                    start: login::Start::Home,
                    seat
                }
            );
        }
    }

    #[test]
    fn the_first_location_becomes_the_goal_of_a_config_without_one() {
        let bare = Config::parse(ACCOUNT).unwrap();
        let mut book = Book::new(config_placement(&bare));
        assert_eq!(goal(book.held(), config_start(&bare)).region, None);
        assert!(book.first_location(at("Home", 60.0, 70.0, 22.0)));
        let goal = goal(book.held(), config_start(&bare));
        assert_eq!((goal.region.as_deref(), goal.start), (Some("Home"), login::Start::Target));
    }

    #[test]
    fn the_target_is_the_object_asked_for_or_the_prim_named() {
        let (child, root, other) = (object(), Uuid::from_u128(2), Uuid::from_u128(3));
        // The config names a child prim; the simulator seats the avatar on the root.
        assert!(on_target(SitState::Object(child), sat(root, Some(child))));
        // `cur` has made the root the held sit state.
        assert!(on_target(SitState::Object(root), sat(root, Some(child))));
        assert!(on_target(SitState::Object(root), sat(root, None)));
        // A sit nobody asked for counts only by the prim named.
        assert!(!on_target(SitState::Object(child), sat(root, None)));
        assert!(!on_target(SitState::Object(other), sat(root, Some(child))));
        for sit in [SitState::Ground, SitState::Stand, SitState::Unchanged] {
            assert!(!on_target(sit, sat(root, Some(child))));
        }
        assert!(!on_target(SitState::Object(child), Seat::None));
        assert!(!on_target(SitState::Object(child), Seat::Other));
    }

    #[test]
    fn a_persist_stands_the_avatar_up_only_when_its_sit_state_needs_it() {
        let seated = sat(object(), None);
        // `stand`: from an object, from something unnamed, from the ground.
        assert!(needs_stand(SitState::Stand, seated, false));
        assert!(needs_stand(SitState::Stand, Seat::Other, false));
        assert!(needs_stand(SitState::Stand, Seat::None, true));
        assert!(!needs_stand(SitState::Stand, Seat::None, false));
        assert!(!needs_stand(SitState::Stand, Seat::Unknown, false));
        // `ground`: only off an object.
        assert!(needs_stand(SitState::Ground, seated, false));
        assert!(!needs_stand(SitState::Ground, Seat::None, true));
        assert!(!needs_stand(SitState::Ground, Seat::None, false));
        // An object and `unchanged`: never.
        for sit in [SitState::Unchanged, SitState::Object(Uuid::from_u128(9))] {
            assert!(!needs_stand(sit, seated, false));
            assert!(!needs_stand(sit, Seat::None, true));
        }
    }

    #[test]
    fn status_shows_the_place_the_seat_and_the_held_placement() {
        let held = Placement {
            location: Some(at("Target", 128.0, 128.5, 25.0)),
            sit: SitState::Object(object()),
        };
        let here = Whereabouts {
            region: "Target",
            position: Some([131.004, 64.5, 23.0]),
            seat: sat(object(), None),
            seat_name: None,
        };
        assert_eq!(
            status_text(Some(here), &held, false),
            format!(
                "connected: yes\nregion: Target\nposition: 131, 64.5, 23\nseat: target\n\
                 persist: Target/128/128.5/25 {OBJECT}\nmode: holding"
            )
        );
        // On another object, in manual mode: the prim the simulator named.
        let other = Uuid::from_u128(7);
        let here = Whereabouts {
            region: "Home",
            position: None,
            seat: sat(other, Some(Uuid::from_u128(8))),
            seat_name: None,
        };
        assert_eq!(
            status_text(Some(here), &held, true),
            format!("connected: yes\nregion: Home\nseat: object {other}\npersist: Target/128/128.5/25 {OBJECT}\nmode: manual")
        );
        for (seat, word) in [(Seat::None, "none"), (Seat::Unknown, "unknown"), (Seat::Other, "other")] {
            let here = Whereabouts {
                region: "Home",
                position: None,
                seat,
                seat_name: None,
            };
            assert!(status_text(Some(here), &held, false).contains(&format!("\nseat: {word}\n")));
        }
    }

    #[test]
    fn status_names_what_the_avatar_sits_on() {
        let held = Placement {
            location: Some(at("Target", 128.0, 128.5, 25.0)),
            sit: SitState::Object(object()),
        };
        let seated = |seat, seat_name| {
            let here = Whereabouts {
                region: "Target",
                position: None,
                seat,
                seat_name,
            };
            status_text(Some(here), &held, false)
        };
        // On the held object: the name follows the seat.
        assert_eq!(
            seated(sat(object(), None), Some("Park Bench")),
            format!(
                "connected: yes\nregion: Target\nseat: target\nseat name: Park Bench\npersist: Target/128/128.5/25 {OBJECT}\nmode: holding"
            )
        );
        // On a child prim of another object: the prim the simulator
        // named, and the name of the linkset's root.
        let cushion = Uuid::from_u128(7);
        let text = seated(sat(cushion, None), Some("Sofa"));
        assert!(text.contains(&format!("\nseat: object {cushion}\nseat name: Sofa\n")), "{text}");
        assert!(seated(Seat::Other, Some("Sofa")).contains("\nseat: other\nseat name: Sofa\n"));
        // Not told yet: answered without the line.
        assert!(!seated(sat(object(), None), None).contains("seat name"));
        // Standing, on the ground, or not known: no seat to name.
        for seat in [Seat::None, Seat::Unknown] {
            assert!(!seated(seat, Some("Sofa")).contains("seat name"));
        }
        // A name stays on its line.
        let text = seated(sat(object(), None), Some("Two\nlines\tand a \\"));
        assert!(text.contains("\nseat name: Two\\nlines\\tand a \\\\\npersist:"), "{text}");
    }

    #[test]
    fn status_between_logins_shows_what_is_held() {
        let held = Placement {
            location: Some(at("Target", 128.0, 128.0, 25.0)),
            sit: SitState::Ground,
        };
        assert_eq!(
            status_text(None, &held, true),
            "connected: no\npersist: Target/128/128/25 ground\nmode: manual"
        );
        // Before the first login of a config without a position.
        let held = Placement {
            location: None,
            sit: SitState::Stand,
        };
        assert_eq!(
            status_text(None, &held, false),
            "connected: no\npersist: unknown stand\nmode: holding"
        );
    }

    // --------------------------------------------- the config's persist

    const CUR: Persist = Persist::On {
        position: Source::Cur,
        sit: Source::Cur,
    };
    const CFG: Persist = Persist::On {
        position: Source::Cfg,
        sit: Source::Cfg,
    };

    /// In the target region, nothing more.
    fn arrived() -> Arrival {
        Arrival {
            present: true,
            in_target_region: true,
            seated_on_target: false,
            ground_sat: false,
            seat_frame_known: false,
        }
    }

    #[test]
    fn the_config_placement_is_reached_according_to_its_sit() {
        let now = Instant::now();
        let due = |sit, arrival| startup_due(sit, CFG, false, arrival, None, now);
        // No seat: being there is enough.
        assert_eq!(due(SitState::Stand, arrived()), Startup::Now { settled: true });
        // An object: seated on it.
        assert_eq!(due(SitState::Object(object()), arrived()), Startup::NotYet);
        let seated = Arrival {
            seated_on_target: true,
            ..arrived()
        };
        assert_eq!(due(SitState::Object(object()), seated), Startup::Now { settled: true });
        // The ground: the ground sit sent.
        assert_eq!(due(SitState::Ground, arrived()), Startup::NotYet);
        assert_eq!(
            due(
                SitState::Ground,
                Arrival {
                    ground_sat: true,
                    ..arrived()
                }
            ),
            Startup::Now { settled: true }
        );
        // Never before the avatar is in the target region and staying.
        for sit in [SitState::Stand, SitState::Ground, SitState::Object(object())] {
            let reached = Arrival {
                seated_on_target: true,
                ground_sat: true,
                seat_frame_known: true,
                ..arrived()
            };
            assert_eq!(
                due(
                    sit,
                    Arrival {
                        in_target_region: false,
                        ..reached
                    }
                ),
                Startup::NotYet
            );
            assert_eq!(due(sit, Arrival { present: false, ..reached }), Startup::NotYet);
        }
    }

    #[test]
    fn a_current_location_waits_ten_seconds_for_the_seat() {
        let now = Instant::now();
        let seated = Arrival {
            seated_on_target: true,
            ..arrived()
        };
        let sit = SitState::Object(object());
        // The seat's position is known: at once.
        let known = Arrival {
            seat_frame_known: true,
            ..seated
        };
        assert_eq!(startup_due(sit, CUR, false, known, None, now), Startup::Now { settled: true });
        // Not known: wait, from when the wait began.
        assert_eq!(
            startup_due(sit, CUR, false, seated, None, now),
            Startup::Wait(now + SEAT_POSITION_WAIT)
        );
        let later = now + Duration::from_secs(4);
        assert_eq!(
            startup_due(sit, CUR, false, seated, Some(now), later),
            Startup::Wait(now + SEAT_POSITION_WAIT)
        );
        assert_eq!(
            startup_due(sit, CUR, false, known, Some(now), later),
            Startup::Now { settled: true }
        );
        // After ten seconds it goes ahead with what it has.
        let late = now + SEAT_POSITION_WAIT;
        assert_eq!(
            startup_due(sit, CUR, false, seated, Some(now), late),
            Startup::Now { settled: false }
        );
        // Only a current location is about the seat's position.
        let seat_only = Persist::On {
            position: Source::Cfg,
            sit: Source::Cur,
        };
        for setting in [CFG, seat_only, Persist::Off] {
            assert_eq!(startup_due(sit, setting, false, seated, None, now), Startup::Now { settled: true });
        }
        // And only a seat on an object has one.
        let on_ground = Arrival {
            ground_sat: true,
            ..arrived()
        };
        assert_eq!(
            startup_due(SitState::Ground, CUR, false, on_ground, None, now),
            Startup::Now { settled: true }
        );
        assert_eq!(
            startup_due(SitState::Stand, CUR, false, arrived(), None, now),
            Startup::Now { settled: true }
        );
    }

    // ------------------------------------------ persist without a session

    /// A bot that has not logged in, and the control answers it gives.
    fn bot(config: Config) -> Bot {
        let (events, _incoming) = mpsc::channel();
        Bot::new(config, Logs::none(), events)
    }

    fn command(bot: &mut Bot, line: &str) -> Response {
        let (reply, answer) = mpsc::channel();
        let request = Request::decode(line.as_bytes()).unwrap();
        assert!(matches!(bot.control(request, reply, Instant::now()), Flow::Continue));
        answer.try_recv().unwrap()
    }

    fn next_login(bot: &Bot) -> Decision {
        bot.reconciler.decide(Instant::now())
    }

    #[test]
    fn persist_while_disconnected_takes_values_that_need_no_simulator() {
        let mut bot = bot(full(""));
        let answer = command(&mut bot, "persist\tBlue Lagoon/128/128/25\tground");
        assert_eq!(answer, Response::Ok(Some("Blue Lagoon/128/128/25 ground".to_string())));
        assert_eq!(bot.book.held().to_string(), "Blue Lagoon/128/128/25 ground");
        assert_eq!(bot.reconciler.goal().region.as_deref(), Some("Blue Lagoon"));
        assert_eq!(bot.reconciler.goal().seat, Some(SeatGoal::Ground));
        assert!(bot
            .status(Interface::Ctl)
            .ends_with("persist: Blue Lagoon/128/128/25 ground\nmode: holding"));
        // Back to the config.
        let answer = command(&mut bot, "persist\tcfg\tcfg");
        assert_eq!(answer, Response::Ok(Some(format!("Target/128/128/25 {OBJECT}"))));
        assert_eq!(bot.reconciler.goal().region.as_deref(), Some("Target"));
        assert_eq!(next_login(&bot), Decision::Start(Action::Login(login::Start::Target)));
    }

    #[test]
    fn persist_of_current_values_is_refused_while_disconnected() {
        let mut bot = bot(full(""));
        bot.set_manual(true);
        for line in ["persist", "persist\tcur", "persist\tcfg\tcur", "persist\tcur\tstand"] {
            assert_eq!(
                command(&mut bot, line),
                Response::Error(session::NOT_CONNECTED.to_string()),
                "{line}"
            );
        }
        // Nothing changed: not the placement, not the mode, not the pending setting.
        assert_eq!(bot.book.held(), &config_placement(&full("")));
        assert!(bot.reconciler.is_manual());
        assert!(bot.startup.is_some());
    }

    #[test]
    fn malformed_persist_arguments_change_nothing() {
        let mut bot = bot(full(""));
        bot.set_manual(true);
        let Response::Error(reason) = command(&mut bot, "persist\tTarget/128/128\tstand") else {
            panic!("accepted")
        };
        assert!(
            reason.contains("Target/128/128") && reason.contains("<region>/<x>/<y>/<z>"),
            "{reason}"
        );
        let Response::Error(reason) = command(&mut bot, "persist\tcfg\tchair") else {
            panic!("accepted")
        };
        assert!(reason.contains("chair") && reason.contains("unchanged"), "{reason}");
        // Too many arguments do not get as far as the bot.
        assert!(Request::decode(b"persist\tTarget/128/128/25\tstand\tnow")
            .unwrap_err()
            .contains("`persist`"));
        assert_eq!(bot.book.held(), &config_placement(&full("")));
        assert!(bot.reconciler.is_manual());
    }

    #[test]
    fn home_returns_to_the_last_persist_or_else_to_the_config() {
        let mut bot = bot(full(""));
        bot.set_manual(true);
        // No persist yet: the config placement.
        assert_eq!(command(&mut bot, "home"), Response::Ok(Some(format!("Target/128/128/25 {OBJECT}"))));
        assert!(!bot.reconciler.is_manual());
        // After a persist: that one, also from manual mode.
        command(&mut bot, "persist\tBlue Lagoon/10/20/30\tunchanged");
        bot.set_manual(true);
        assert_eq!(
            command(&mut bot, "home"),
            Response::Ok(Some("Blue Lagoon/10/20/30 unchanged".to_string()))
        );
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.reconciler.goal().seat, None);
        assert_eq!(bot.reconciler.goal().region.as_deref(), Some("Blue Lagoon"));
    }

    #[test]
    fn a_persist_or_home_command_replaces_the_configs_persist() {
        for line in ["home", "persist\tcfg\tcfg"] {
            let mut bot = bot(full("persist = \"off\"\n"));
            assert_eq!(bot.startup, Some(Persist::Off));
            assert!(matches!(command(&mut bot, line), Response::Ok(_)));
            assert_eq!(bot.startup, None, "{line}");
            assert_eq!(bot.apply_startup(Instant::now()), None);
            assert!(!bot.reconciler.is_manual());
        }
        // A refused one does not.
        let mut bot = bot(full("persist = \"cur\"\n"));
        assert!(matches!(command(&mut bot, "persist"), Response::Error(_)));
        assert_eq!(bot.startup, Some(CUR));
    }

    #[test]
    fn a_config_without_a_position_holds_no_location_before_the_first_login() {
        let mut bot = bot(Config::parse(ACCOUNT).unwrap());
        assert_eq!(
            bot.status(Interface::Ctl),
            "connected: no\navatar: Sit Bot\npersist: unknown stand\nmode: holding"
        );
        assert_eq!(next_login(&bot), Decision::Start(Action::Login(login::Start::Last)));
        // `cfg` is accepted all the same, and still waits for that login.
        assert_eq!(
            command(&mut bot, "persist\tcfg\tcfg"),
            Response::Ok(Some("unknown stand".to_string()))
        );
        assert_eq!(next_login(&bot), Decision::Start(Action::Login(login::Start::Last)));
    }

    // ------------------------------------------------------------------ RLV

    fn ok(bot: &mut Bot, line: &str) -> String {
        match command(bot, line) {
            Response::Ok(Some(text)) => text,
            other => panic!("{line}: expected an answer with a text, got {other:?}"),
        }
    }

    fn refused(bot: &mut Bot, line: &str) -> String {
        match command(bot, line) {
            Response::Error(reason) => reason,
            other => panic!("{line}: expected an error, got {other:?}"),
        }
    }

    /// What an object named `Collar` says to its owner.
    fn owner_say(text: &str) -> SessionEvent {
        SessionEvent::Chat(session::Message {
            kind: "owner".to_string(),
            source: session::Source::Object {
                id: Some(Uuid::from_bytes([7; 16])),
                name: "Collar".to_string(),
                owner: Uuid::from_bytes([1; 16]),
            },
            sent_at: None,
            text: text.to_string(),
        })
    }

    fn hear(bot: &mut Bot, event: SessionEvent) {
        assert!(matches!(bot.apply(event, Instant::now()), Flow::Continue));
    }

    #[test]
    fn rlv_starts_as_the_config_says() {
        let mut plain = bot(full(""));
        assert_eq!(
            ok(&mut plain, "rlv\tstatus"),
            "rlv: off\npersist: drop\npersist override: none\nrestrictions: none"
        );
        let mut on = bot(full("rlv = true\n"));
        assert_eq!(
            ok(&mut on, "rlv\tstatus"),
            "rlv: on\npersist: drop\npersist override: none\nrestrictions: none"
        );
        let mut following = bot(full("[persist]\nrlv = \"follow\"\n"));
        assert_eq!(
            ok(&mut following, "rlv\tstatus"),
            "rlv: off\npersist: follow\npersist override: none\nrestrictions: none"
        );
        // That table holds nothing at the start, whatever RLV does later.
        assert_eq!(following.startup, Some(Persist::Off));
    }

    #[test]
    fn rlv_mode_and_persist_are_switched_by_command_in_any_state() {
        // Between logins, in holding mode.
        let mut bot = bot(full(""));
        let held = bot.book.held().clone();
        assert_eq!(ok(&mut bot, "rlv\tmode\ton"), "rlv: on");
        assert_eq!(ok(&mut bot, "rlv\tmode\ton"), "rlv: on");
        assert_eq!(
            ok(&mut bot, "rlv\tstatus"),
            "rlv: on\npersist: drop\npersist override: none\nrestrictions: none"
        );
        // The persist setting does not care whether RLV is on.
        assert_eq!(ok(&mut bot, "rlv\tmode\toff"), "rlv: off");
        assert_eq!(ok(&mut bot, "rlv\tpersist\tfollow"), "rlv persist: follow");
        assert_eq!(ok(&mut bot, "rlv\tpersist\tfollow"), "rlv persist: follow");
        assert_eq!(
            ok(&mut bot, "rlv\tstatus"),
            "rlv: off\npersist: follow\npersist override: none\nrestrictions: none"
        );
        assert_eq!(ok(&mut bot, "rlv\tmode\ton"), "rlv: on");
        assert_eq!(
            ok(&mut bot, "rlv\tstatus"),
            "rlv: on\npersist: follow\npersist override: none\nrestrictions: none"
        );
        assert_eq!(ok(&mut bot, "rlv\tpersist\tdrop"), "rlv persist: drop");
        // None of it touches the placement.
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.book.held(), &held);
        assert_eq!(bot.startup, Some(Persist::Off));
    }

    #[test]
    fn rlv_cmd_is_refused_while_rlv_is_off() {
        let mut bot = bot(full(""));
        let reason = refused(&mut bot, "rlv\tcmd\t@unsit=force");
        assert!(reason.contains("RLV is off") && reason.contains("rlv mode on"), "{reason}");
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn rlv_cmd_answers_a_line_for_each_command() {
        let mut bot = bot(full("rlv = true\n"));
        // Between logins nothing can be said or done; each says so.
        assert_eq!(
            refused(&mut bot, "rlv\tcmd\t@versionnum=5"),
            format!("versionnum=5: {}", session::NOT_CONNECTED)
        );
        assert_eq!(
            refused(&mut bot, "rlv\tcmd\tunsit=force"),
            format!("unsit=force: {}", session::NOT_CONNECTED)
        );
        assert_eq!(
            refused(&mut bot, "rlv\tcmd\t@versionnum=5,notify:5=add,nonsense,"),
            format!(
                "versionnum=5: {}\nnotify:5=add: not supported\nnonsense: malformed: it has no `=`",
                session::NOT_CONNECTED
            )
        );
        let reason = refused(&mut bot, "rlv\tcmd\t@");
        assert!(reason.contains("no RLV command"), "{reason}");
        // Nothing was carried out, so nothing changed.
        assert!(!bot.reconciler.is_manual());
        assert!(!bot.rlv.has_waiting());
    }

    #[test]
    fn a_followed_sit_that_cannot_be_persisted_changes_nothing() {
        let mut bot = bot(full("rlv = true\n[persist]\nrlv = \"follow\"\n"));
        let held = bot.book.held().clone();
        // `cur` needs the avatar in a region.
        let reason = refused(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", Uuid::from_bytes([9; 16])));
        assert!(reason.contains("not connected"), "{reason}");
        assert_eq!(bot.book.held(), &held);
        assert!(!bot.reconciler.is_manual());
        // The config's setting is still to be applied, and not for RLV.
        assert_eq!(bot.startup, Some(Persist::Off));
        assert!(!bot.follow_seat);
    }

    /// Where the avatar is said to be while a test has no session.
    struct Somewhere(Location, persist::Sitting);

    impl World for Somewhere {
        fn here(&self) -> Location {
            self.0.clone()
        }
        fn sitting(&self) -> persist::Sitting {
            self.1
        }
        fn region_of(&self, _: Uuid) -> Option<String> {
            None
        }
    }

    fn bench() -> Uuid {
        Uuid::from_bytes([9; 16])
    }

    /// A bot with RLV on and following, holding the config placement, whose
    /// avatar stands at 40, 50, 22 in `Home` as far as `persist` can tell.
    fn follower() -> Bot {
        let mut bot = bot(full("rlv = true\npersist = \"cfg\"\n"));
        ok(&mut bot, "rlv\tpersist\tfollow");
        bot.world = Some(Box::new(Somewhere(at("Home", 40.0, 50.0, 22.0), persist::Sitting::Standing)));
        bot
    }

    // ------------------------------------------------------- restrictions

    fn collar() -> Issuer {
        Issuer {
            id: Uuid::from_bytes([7; 16]),
            name: "Collar".to_string(),
            operator: false,
        }
    }

    fn cage() -> Issuer {
        Issuer {
            id: Uuid::from_bytes([8; 16]),
            name: "Cage".to_string(),
            operator: false,
        }
    }

    /// What an object says to its owner, dealt with as if the avatar
    /// could act: a bot under test has no session.
    fn says(bot: &mut Bot, issuer: &Issuer, text: &str) {
        bot.rlv_message(issuer, text, Instant::now());
    }

    fn restriction_lines(bot: &mut Bot) -> Vec<String> {
        ok(bot, "rlv\tstatus").lines().skip(3).map(str::to_string).collect()
    }

    #[test]
    fn override_is_a_mode_between_on_and_off() {
        let mut bot = bot(full(""));
        // Not to be had while RLV is off.
        let reason = refused(&mut bot, "rlv\tmode\toverride");
        assert!(reason.contains("RLV is off") && reason.contains("rlv mode on"), "{reason}");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: off\n"));
        assert_eq!(ok(&mut bot, "rlv\tmode\ton"), "rlv: on");
        assert_eq!(ok(&mut bot, "rlv\tmode\toverride"), "rlv: override");
        assert_eq!(ok(&mut bot, "rlv\tmode\toverride"), "rlv: override");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: override\n"));
        assert!(bot.rlv.is_on());
        // It lasts across a session loss, and is taken back by `on`.
        hear(&mut bot, SessionEvent::Lost("kicked".to_string()));
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: override\n"));
        assert_eq!(ok(&mut bot, "rlv\tmode\ton"), "rlv: on");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
        // From override straight to off.
        ok(&mut bot, "rlv\tmode\toverride");
        assert_eq!(ok(&mut bot, "rlv\tmode\toff"), "rlv: off");
        assert!(!bot.rlv.is_on());
    }

    #[test]
    fn override_is_used_up_by_the_next_command_but_four() {
        let mut bot = bot(full("rlv = true\n"));
        let mode = |bot: &mut Bot| ok(bot, "rlv\tstatus").lines().next().unwrap().to_string();
        ok(&mut bot, "rlv\tmode\toverride");
        // These never use it up: `status`, `perms` and every `rlv` command.
        ok(&mut bot, "status");
        ok(&mut bot, "perms\tstatus");
        ok(&mut bot, "rlv\tpersist\tdrop");
        ok(&mut bot, "rlv\tcmd\t@sendim=n");
        refused(&mut bot, "rlv\tcmd\t@unsit=force");
        assert_eq!(mode(&mut bot), "rlv: override");
        // A command that fails for a reason of its own uses it up.
        assert_eq!(refused(&mut bot, "stand"), session::NOT_CONNECTED);
        assert_eq!(mode(&mut bot), "rlv: on");
        // So does one that no restriction concerns.
        ok(&mut bot, "rlv\tmode\toverride");
        refused(&mut bot, "sendtp\tJane Doe");
        assert_eq!(mode(&mut bot), "rlv: on");
        // And one that is malformed inside.
        ok(&mut bot, "rlv\tmode\toverride");
        refused(&mut bot, "sit\tnowhere");
        assert_eq!(mode(&mut bot), "rlv: on");
    }

    #[test]
    fn a_scan_reads_only() {
        let mut bot = bot(full("rlv = true\n"));
        let mut grid = commands::tests::FakeGrid::default();
        let sign = Uuid::from_bytes([0x51; 16]);
        grid.sightings = vec![scan::Sighting {
            object: sign,
            local_id: 700,
            distance: 3.0,
            position: [0.0; 3],
            flags: 0,
            attachment: false,
            child: false,
            properties: Some(udp::messages::Properties {
                object: sign,
                owner: Uuid::nil(),
                group: Uuid::from_bytes([0x6C; 16]),
                name: "Sign".to_string(),
                description: String::new(),
            }),
        }];
        bot.grid = Some(grid);
        ok(&mut bot, "rlv\tmode\toverride");
        let held = bot.book.held().to_string();

        // The answer comes once the waiting scans are looked at.
        let (reply, answer) = mpsc::channel();
        let request = Request::decode(b"scan\t20\t-n\tsign").unwrap();
        assert!(matches!(bot.control(request, reply, Instant::now()), Flow::Continue));
        assert!(answer.try_recv().is_err());
        bot.scan_objects(Instant::now());
        let Ok(Response::Ok(Some(line))) = answer.try_recv() else {
            panic!("no listing");
        };
        assert!(
            line.starts_with(&format!("{sign}\t3.0\t-\t")) && line.ends_with("\t(group)\tSign\t"),
            "{line}"
        );

        // It was the command the override let through, as any other is.
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
        assert!(refused(&mut bot, "scan\t-x").contains("`-x`"));
        // Nor did it move the avatar or change what the bot holds.
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.book.held().to_string(), held);
        assert!(ok(&mut bot, "status").ends_with("mode: holding"));
        let calls = &bot.grid.as_ref().unwrap().calls;
        assert_eq!(calls[..], [commands::tests::Call::Reach(Some(20.0))]);
    }

    /// A bot with RLV on and a grid where Jane stands 5 metres from the
    /// avatar and a sign of a group 3 metres away, the collar holding
    /// `held` there.
    fn among_others(held: &[&str]) -> Bot {
        let mut bot = bot(full("rlv = true\n"));
        let sign = Uuid::from_bytes([0x51; 16]);
        let mut grid = commands::tests::FakeGrid {
            position: Some([100.0, 100.0, 25.0]),
            avatars: vec![avatars::Seen {
                id: Uuid::from_bytes([1; 16]),
                position: [105.0, 100.0],
                height: Some(25.0),
                hidden: false,
            }],
            friends: friends::Friends::new(&[(Uuid::from_bytes([1; 16]), friends::SEE_ONLINE)]),
            ..commands::tests::FakeGrid::default()
        };
        grid.sightings = vec![scan::Sighting {
            object: sign,
            local_id: 700,
            distance: 3.0,
            position: [0.0; 3],
            flags: 0,
            attachment: false,
            child: false,
            properties: Some(udp::messages::Properties {
                object: sign,
                owner: Uuid::from_bytes([1; 16]),
                group: Uuid::nil(),
                name: "Sign".to_string(),
                description: String::new(),
            }),
        }];
        let collar = rlv::Who::Object(Uuid::from_bytes([0xC0; 16]));
        for text in held {
            let (behaviour, option) = match text.split_once(':') {
                Some((behaviour, option)) => (behaviour, Some(option)),
                None => (*text, None),
            };
            grid.kept.set(collar, "Collar", behaviour, option);
        }
        bot.grid = Some(grid);
        bot
    }

    /// Carries out `line` and answers it after the waits it has: the text,
    /// or why it failed.
    fn waited(bot: &mut Bot, line: &str) -> Result<Option<String>, String> {
        let (reply, answer) = mpsc::channel();
        let request = Request::decode(line.as_bytes()).unwrap();
        assert!(matches!(bot.control(request, reply, Instant::now()), Flow::Continue));
        if let Ok(response) = answer.try_recv() {
            return match response {
                Response::Ok(text) => Ok(text),
                Response::Error(reason) => Err(reason),
            };
        }
        // The profiles asked for come at once; names are never given.
        let calls = std::mem::take(&mut bot.grid.as_mut().unwrap().calls);
        for call in calls {
            if let commands::tests::Call::Profile(id) = call {
                let profile = profile::Profile {
                    id,
                    ..profile::tests::full()
                };
                bot.commands.profile_received(profile);
            }
        }
        let later = Instant::now() + commands::SCAN_LIMIT + Duration::from_secs(1);
        bot.scan_objects(later);
        match answer.try_recv() {
            Ok(Response::Ok(text)) => Ok(text),
            Ok(Response::Error(reason)) => Err(reason),
            Err(e) => panic!("{line}: no answer: {e:?}"),
        }
    }

    #[test]
    fn scan_avatars_reads_only() {
        let mut bot = among_others(&[]);
        bot.grid.as_mut().unwrap().sitting = Sitting::Object;
        let held = bot.book.held().to_string();
        let listed = waited(&mut bot, "scan\tavatars").unwrap().unwrap();
        assert!(
            listed.starts_with(&format!("{}\t5.0\t105,100,25\t", Uuid::from_bytes([1; 16]))),
            "{listed}"
        );
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.book.held().to_string(), held);
        assert!(ok(&mut bot, "status").ends_with("mode: holding"));
        // No sit, no teleport, no object scan: nothing asked of the grid.
        assert_eq!(bot.grid.as_ref().unwrap().calls, []);
    }

    #[test]
    fn a_scan_under_an_override_is_held_to_nothing_and_uses_it_up() {
        // Restrictions on scans.
        let mut bot = among_others(&["shownames", "camdrawmax:10"]);
        let grid = bot.grid.as_mut().unwrap();
        let far = scan::Sighting {
            distance: 20.0,
            ..grid.sightings[0].clone()
        };
        grid.sightings.push(far);
        ok(&mut bot, "rlv\tmode\toverride");
        let listed = waited(&mut bot, "scan").unwrap().unwrap();
        // The owner, and out to the object scan's distance.
        assert!(listed.contains(&format!("\t{}\t?\tSign\t", Uuid::from_bytes([1; 16]))), "{listed}");
        assert_eq!(listed.lines().count(), 2);
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
        // The next one is held to them.
        let listed = waited(&mut bot, "scan").unwrap().unwrap();
        assert!(listed.contains("\t?\t?\tSign\t"), "{listed}");
        assert_eq!(listed.lines().count(), 1);
        // None on scans: used up all the same.
        let mut bot = among_others(&["sendchat"]);
        ok(&mut bot, "rlv\tmode\toverride");
        waited(&mut bot, "scan").unwrap();
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
    }

    #[test]
    fn scan_avatars_under_an_override_lets_one_through() {
        let mut bot = among_others(&["shownearby", "shownames"]);
        ok(&mut bot, "rlv\tmode\toverride");
        let listed = waited(&mut bot, "scan\tavatars").unwrap().unwrap();
        assert!(listed.starts_with(&Uuid::from_bytes([1; 16]).to_string()), "{listed}");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
        // Only one.
        let reason = waited(&mut bot, "scan\tavatars").unwrap_err();
        assert!(reason.contains("shownearby") && reason.contains("rlv mode override"), "{reason}");
    }

    #[test]
    fn friends_and_profile_leave_an_override_alone() {
        let mut bot = among_others(&["shownames", "shownearby", "showminimap"]);
        ok(&mut bot, "rlv\tmode\toverride");
        let listed = waited(&mut bot, "friends").unwrap().unwrap();
        assert_eq!(listed, format!("{}\toffline\t?\t?", Uuid::from_bytes([1; 16])));
        let profile = waited(&mut bot, &format!("profile\t{}", Uuid::from_bytes([1; 16])))
            .unwrap()
            .unwrap();
        assert!(
            profile.starts_with(&format!("id: {}\nonline: yes\nfriend: yes\n", Uuid::from_bytes([1; 16]))),
            "{profile}"
        );
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: override\n"));
        assert!(ok(&mut bot, "status").ends_with("mode: holding"));
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn restrictions_on_scans_change_nothing_else() {
        let mut bot = bot(full("rlv = true\n"));
        let status = ok(&mut bot, "status");
        ok(&mut bot, "rlv\tcmd\t@shownames=n,shownearby=n,showminimap=n");
        assert_eq!(ok(&mut bot, "status"), status);
        let jane = Uuid::from_bytes([2; 16]);
        let said = session::Message {
            kind: "say".to_string(),
            source: session::Source::Avatar {
                id: jane,
                name: "Jane Doe".to_string(),
            },
            sent_at: None,
            text: "hello".to_string(),
        };
        hear(&mut bot, SessionEvent::Chat(said));
        let records = bot.recorder.flush_all(Instant::now());
        assert_eq!(records.len(), 1);
        let record = records[0].message();
        assert_eq!((record.avatar_name.as_str(), record.avatar_uuid), ("Jane Doe", Some(jane)));
    }

    #[test]
    fn a_scan_between_logins_is_refused() {
        let mut bot = bot(full(""));
        assert_eq!(refused(&mut bot, "scan"), session::NOT_CONNECTED);
        assert_eq!(refused(&mut bot, "scan\t96\t-s"), session::NOT_CONNECTED);
    }

    #[test]
    fn an_exempt_persist_exempts_the_way_to_what_it_holds() {
        let mut bot = bot(full("rlv = true\n"));
        ok(&mut bot, "rlv\tmode\toverride");
        ok(&mut bot, "persist\tcfg\tcfg");
        assert!(bot.exempt_placement);
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
        // The next persist is an ordinary one again.
        ok(&mut bot, "persist\tcfg\tcfg");
        assert!(!bot.exempt_placement);
        // `home` likewise, and manual mode ends it.
        ok(&mut bot, "rlv\tmode\toverride");
        ok(&mut bot, "home");
        assert!(bot.exempt_placement);
        bot.set_manual(true);
        assert!(!bot.exempt_placement);
    }

    #[test]
    fn rlv_status_lists_the_restrictions_in_every_mode() {
        let mut bot = bot(full("rlv = true\n"));
        assert_eq!(restriction_lines(&mut bot), ["restrictions: none"]);
        says(&mut bot, &collar(), "@detach=n,showloc=n");
        assert_eq!(ok(&mut bot, "rlv\tcmd\t@tploc=n"), "tploc=n: ok");
        let id = collar().id;
        let lines = [
            "restrictions: 3".to_string(),
            format!("Collar <{id}>: detach"),
            format!("Collar <{id}>: showloc (no effect)"),
            "operator: tploc".to_string(),
        ];
        assert_eq!(restriction_lines(&mut bot), lines);
        ok(&mut bot, "rlv\tmode\toff");
        assert_eq!(restriction_lines(&mut bot), lines);
        // Off keeps them, and they are there when RLV is on again.
        ok(&mut bot, "rlv\tmode\ton");
        assert_eq!(restriction_lines(&mut bot), lines);
    }

    #[test]
    fn rlv_cmd_holds_its_restrictions_apart_from_every_object() {
        let mut bot = bot(full("rlv = true\n"));
        says(&mut bot, &collar(), "@detach=n");
        assert_eq!(ok(&mut bot, "rlv\tcmd\t@sendim=n,tploc=add"), "sendim=n: ok\ntploc=add: ok");
        // Setting again and lifting what is not set are carried out too.
        assert_eq!(ok(&mut bot, "rlv\tcmd\tsendim=n,unsit=y"), "sendim=n: ok\nunsit=y: ok");
        // An object's clear leaves the operator's alone.
        says(&mut bot, &collar(), "@clear");
        assert_eq!(
            restriction_lines(&mut bot),
            ["restrictions: 2", "operator: sendim", "operator: tploc"]
        );
        // And the operator's clear an object's.
        says(&mut bot, &collar(), "@detach=n");
        assert_eq!(ok(&mut bot, "rlv\tcmd\t@clear=tp"), "clear=tp: ok");
        assert_eq!(ok(&mut bot, "rlv\tcmd\t@clear"), "clear: ok");
        assert_eq!(
            restriction_lines(&mut bot),
            ["restrictions: 1".to_string(), format!("Collar <{}>: detach", collar().id)]
        );
        // What the bot does not keep is said so, and fails the command.
        assert_eq!(
            refused(&mut bot, "rlv\tcmd\t@fly=n,notify:5=add,frobnicate=n"),
            "fly=n: ok\nnotify:5=add: not supported\nfrobnicate=n: not supported"
        );
        assert_eq!(restriction_lines(&mut bot).len(), 3);
    }

    #[test]
    fn rlv_reset_clears_everything_in_any_mode() {
        let mut bot = bot(full("rlv = true\n"));
        says(&mut bot, &collar(), "@detach=n,sendim=n");
        ok(&mut bot, "rlv\tcmd\t@tploc=n");
        let held = bot.book.held().clone();
        assert_eq!(ok(&mut bot, "rlv\treset"), "rlv reset: 3 restrictions cleared");
        assert_eq!(restriction_lines(&mut bot), ["restrictions: none"]);
        assert_eq!(ok(&mut bot, "rlv\treset"), "rlv reset: 0 restrictions cleared");
        // Nothing else changed, and an object may set a restriction again.
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\npersist: drop\n"));
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.book.held(), &held);
        says(&mut bot, &collar(), "@detach=n");
        assert_eq!(restriction_lines(&mut bot).len(), 2);
        // While RLV is off as well, and it stays off.
        ok(&mut bot, "rlv\tmode\toff");
        assert_eq!(ok(&mut bot, "rlv\treset"), "rlv reset: 1 restrictions cleared");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: off\n"));
        ok(&mut bot, "rlv\tmode\ton");
        assert_eq!(restriction_lines(&mut bot), ["restrictions: none"]);
    }

    #[test]
    fn an_objects_restrictions_end_with_the_object() {
        let mut bot = bot(full("rlv = true\n"));
        let operator = "operator: tploc".to_string();
        let set = |bot: &mut Bot| {
            ok(bot, "rlv\treset");
            says(bot, &collar(), "@sendim=n,unsit=n");
            says(bot, &cage(), "@sittp=n");
            ok(bot, "rlv\tcmd\t@tploc=n");
        };
        // The simulator says the collar is gone.
        set(&mut bot);
        hear(&mut bot, SessionEvent::ObjectGone(collar().id));
        assert_eq!(
            restriction_lines(&mut bot),
            [
                "restrictions: 2".to_string(),
                format!("Cage <{}>: sittp", cage().id),
                operator.clone()
            ]
        );
        // One that held nothing is nothing.
        hear(&mut bot, SessionEvent::ObjectGone(Uuid::from_bytes([9; 16])));
        assert_eq!(restriction_lines(&mut bot).len(), 3);
        // The avatar leaves the region wearing the collar: the cage stays behind.
        set(&mut bot);
        hear(&mut bot, SessionEvent::RegionLeft { worn: vec![collar().id] });
        let lines = restriction_lines(&mut bot);
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(!lines.iter().any(|line| line.starts_with("Cage")), "{lines:?}");
        // The session is lost: the operator's are all that is left.
        set(&mut bot);
        hear(&mut bot, SessionEvent::Lost("kicked".to_string()));
        assert_eq!(restriction_lines(&mut bot), ["restrictions: 1".to_string(), operator.clone()]);
        // All of it while RLV is off, too.
        set(&mut bot);
        ok(&mut bot, "rlv\tmode\toff");
        hear(&mut bot, SessionEvent::ObjectGone(cage().id));
        hear(&mut bot, SessionEvent::RegionLeft { worn: vec![] });
        ok(&mut bot, "rlv\tmode\ton");
        assert_eq!(restriction_lines(&mut bot), ["restrictions: 1".to_string(), operator]);
    }

    #[test]
    fn the_persist_override_setting_starts_from_the_config_and_is_switched_by_command() {
        let line = |bot: &mut Bot| ok(bot, "rlv\tstatus").lines().nth(2).unwrap().to_string();
        let mut plain = bot(full(""));
        assert_eq!(line(&mut plain), "persist override: none");
        let mut both = bot(full("[persist]\noverride = [\"tp\", \"sit\"]\n"));
        assert_eq!(line(&mut both), "persist override: sit, tp");
        // By command, in any mode, also when nothing changes.
        let held = plain.book.held().clone();
        assert_eq!(ok(&mut plain, "rlv\tpersist\toverride\ttp"), "rlv persist override: tp");
        assert_eq!(ok(&mut plain, "rlv\tpersist\toverride\ttp"), "rlv persist override: tp");
        assert_eq!(line(&mut plain), "persist override: tp");
        assert_eq!(ok(&mut plain, "rlv\tpersist\toverride\ttp\tsit"), "rlv persist override: sit, tp");
        assert_eq!(ok(&mut plain, "rlv\tpersist\toverride\tsit"), "rlv persist override: sit");
        assert_eq!(ok(&mut plain, "rlv\tpersist\toverride\tnone"), "rlv persist override: none");
        // The other settings, the mode and the placement stay.
        ok(&mut plain, "rlv\tpersist\tfollow");
        ok(&mut plain, "rlv\tpersist\toverride\ttp");
        assert_eq!(
            ok(&mut plain, "rlv\tstatus"),
            "rlv: off\npersist: follow\npersist override: tp\nrestrictions: none"
        );
        assert!(!plain.reconciler.is_manual());
        assert_eq!(plain.book.held(), &held);
        // It lasts across a session loss.
        hear(&mut plain, SessionEvent::Lost("kicked".to_string()));
        assert_eq!(line(&mut plain), "persist override: tp");
    }

    #[test]
    fn a_followed_sit_remembers_who_sent_it() {
        let mut bot = follower();
        says(&mut bot, &collar(), &format!("@sit:{}=force", bench()));
        assert_eq!(bot.seat_sender, Some(Who::Object(collar().id)));
        // The second persist, once the avatar sits there, keeps it.
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        assert_eq!(bot.seat_sender, Some(Who::Operator));
        // Another persist, and the operator taking over, end it.
        ok(&mut bot, "persist\tcfg\tcfg");
        assert_eq!(bot.seat_sender, None);
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        moved(&mut bot, commands::Origin::Operator, Request::Stand);
        bot.follow_commands();
        assert_eq!(bot.seat_sender, None);
    }

    #[test]
    fn a_followed_sit_that_waits_persists_nothing_until_its_seat_is_described() {
        let mut bot = follower();
        let held = bot.book.held().clone();
        // What the bot keeps when `deal` answers `Step::Sought`.
        let sought = || {
            let command = rlv::parse(&format!("@sit:{}=force", bench())).remove(0).unwrap();
            (collar(), command, bench())
        };
        bot.seats_sought.push(sought());
        // Another object described changes nothing.
        hear(&mut bot, SessionEvent::Described(Uuid::from_bytes([0x55; 16])));
        assert_eq!(bot.book.held(), &held);
        assert!(!bot.follow_seat && bot.seat_sender.is_none());
        assert_eq!(bot.seats_sought.len(), 1);
        // The bench described: dealt with again, and held as a followed sit.
        hear(&mut bot, SessionEvent::Described(bench()));
        assert!(bot.seats_sought.is_empty());
        assert_eq!(bot.book.held().to_string(), format!("Home/40/50/22 {}", bench()));
        assert!(bot.follow_seat);
        assert_eq!(bot.seat_sender, Some(Who::Object(collar().id)));
        assert!(!bot.reconciler.is_manual());
        // A session lost forgets what waited, and persists nothing.
        let mut bot = follower();
        bot.seats_sought.push(sought());
        hear(&mut bot, SessionEvent::Lost("kicked".to_string()));
        assert!(bot.seats_sought.is_empty());
        assert_eq!(bot.book.held(), &held);
    }

    #[test]
    fn a_followed_sit_holds_the_seat_at_once() {
        let mut bot = follower();
        let sit = format!("sit:{}=force", bench());
        assert_eq!(ok(&mut bot, &format!("rlv\tcmd\t@{sit}")), format!("{sit}: ok"));
        // The seat is held with the place the avatar is at, and the bot
        // is the one to sit: it holds, it was not sent off by hand.
        assert_eq!(bot.book.held().to_string(), format!("Home/40/50/22 {}", bench()));
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.reconciler.goal().seat, Some(SeatGoal::Object));
        // The seated position is persisted when the avatar sits there, the
        // way the config's setting would be.
        assert_eq!(
            bot.startup,
            Some(Persist::On {
                position: Source::Cur,
                sit: Source::Cur
            })
        );
        assert!(bot.follow_seat);
        // It is no `sit` command of the operator's.
        assert_eq!(bot.commands.last_sit(), None);
        // `home` returns to it: it was a persist.
        assert_eq!(ok(&mut bot, "home"), format!("Home/40/50/22 {}", bench()));
    }

    #[test]
    fn a_followed_sit_also_holds_for_a_bot_that_held_nothing() {
        let mut bot = follower();
        bot.set_manual(true);
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.book.held().sit, SitState::Object(bench()));
    }

    #[test]
    fn the_second_persist_of_a_followed_sit_is_given_up_for_the_operator() {
        for line in ["persist\tcfg\tcfg", "home"] {
            let mut bot = follower();
            ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
            ok(&mut bot, line);
            assert_eq!((bot.startup, bot.follow_seat), (None, false), "{line}");
        }
        // Switching the setting afterwards does not undo what was accepted.
        let mut bot = follower();
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        ok(&mut bot, "rlv\tpersist\tdrop");
        assert!(bot.startup.is_some() && bot.follow_seat);
        // A later forced sit takes its place.
        let other = Uuid::from_bytes([8; 16]);
        ok(&mut bot, "rlv\tpersist\tfollow");
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{other}=force"));
        assert_eq!(bot.book.held().sit, SitState::Object(other));
        assert!(bot.startup.is_some() && bot.follow_seat);
    }

    #[test]
    fn a_followed_sit_on_an_object_elsewhere_is_refused() {
        let mut bot = follower();
        // The avatar sat on the bench in `Target` earlier, and is in `Home`.
        bot.book.seated(bench(), "Target");
        let held = bot.book.held().clone();
        let reason = refused(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        assert!(reason.contains("Home") && reason.contains("Target"), "{reason}");
        assert_eq!(bot.book.held(), &held);
        assert!(!bot.reconciler.is_manual() && !bot.follow_seat);
    }

    #[test]
    fn a_followed_sit_outlasts_a_lost_session() {
        let mut bot = follower();
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        hear(&mut bot, SessionEvent::Lost("kicked".to_string()));
        // The seat is held, so the next login goes there and sits; the
        // seated position is still to be persisted then.
        assert_eq!(bot.book.held().sit, SitState::Object(bench()));
        assert!(!bot.reconciler.is_manual());
        assert!(bot.startup.is_some() && bot.follow_seat);
    }

    /// Carries a movement out on a simulator that takes everything, as
    /// the operator's command or as one for RLV.
    fn moved(bot: &mut Bot, origin: commands::Origin, request: Request) {
        let (reply, _answer) = mpsc::channel();
        let mut grid = commands::tests::FakeGrid::default();
        bot.commands.handle_from(origin, request, reply, Some(&mut grid), Instant::now());
    }

    #[test]
    fn the_operator_ends_a_followed_sit_that_has_not_seated_the_avatar() {
        let mut bot = follower();
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        moved(&mut bot, commands::Origin::Operator, Request::Stand);
        bot.follow_commands();
        // Manual mode, by the operator's command; nothing more follows the
        // forced sit, and the bench stays what `home` returns to.
        assert!(bot.reconciler.is_manual());
        assert_eq!((bot.startup, bot.follow_seat), (None, false));
        assert_eq!(bot.book.held().to_string(), format!("Home/40/50/22 {}", bench()));
    }

    #[test]
    fn a_forced_stand_ends_a_followed_sit_that_has_not_seated_the_avatar() {
        let mut bot = follower();
        ok(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        // What `rlv_message` does once the commands have taken the stand.
        let was_manual = bot.reconciler.is_manual();
        moved(&mut bot, commands::Origin::Rlv(Who::Operator), Request::Stand);
        let after = bot.follow.moved(rlv::Movement::Stand, true, was_manual);
        assert!(bot.go_manual());
        bot.after_forced(after, "", Instant::now());
        // Standing is what is held now, and no seat is waited for.
        assert_eq!(bot.book.held().to_string(), "Home/40/50/22 stand");
        assert!(!bot.reconciler.is_manual());
        assert_eq!((bot.startup, bot.follow_seat), (None, false));
    }

    #[test]
    fn a_dropped_movement_goes_manual_and_keeps_what_was_held() {
        let mut bot = follower();
        ok(&mut bot, "rlv\tpersist\tdrop");
        let held = bot.book.held().clone();
        let was_manual = bot.reconciler.is_manual();
        moved(&mut bot, commands::Origin::Rlv(Who::Operator), Request::Stand);
        let after = bot.follow.moved(rlv::Movement::Stand, false, was_manual);
        assert!(bot.go_manual());
        bot.after_forced(after, "", Instant::now());
        assert!(bot.reconciler.is_manual());
        assert_eq!(bot.book.held(), &held);
        // `home` returns to it.
        assert_eq!(ok(&mut bot, "home"), held.to_string());
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn a_dropped_sit_is_a_sit_command_and_holds_nothing() {
        let mut bot = follower();
        ok(&mut bot, "rlv\tpersist\tdrop");
        let held = bot.book.held().clone();
        // Between logins the `sit` it is carried out as cannot be sent.
        let reason = refused(&mut bot, &format!("rlv\tcmd\t@sit:{}=force", bench()));
        assert!(reason.ends_with(session::NOT_CONNECTED), "{reason}");
        assert_eq!(bot.book.held(), &held);
        assert!(!bot.reconciler.is_manual() && !bot.follow_seat);
    }

    #[test]
    fn the_seated_position_of_a_followed_sit_is_waited_for() {
        // What `apply_startup` asks once the followed seat is held.
        let cur = Persist::On {
            position: Source::Cur,
            sit: Source::Cur,
        };
        let held = SitState::Object(bench());
        let now = Instant::now();
        assert_eq!(startup_due(held, cur, false, arrived(), None, now), Startup::NotYet);
        let seated = Arrival {
            seated_on_target: true,
            ..arrived()
        };
        assert_eq!(
            startup_due(held, cur, false, seated, None, now),
            Startup::Wait(now + SEAT_POSITION_WAIT)
        );
        let later = now + SEAT_POSITION_WAIT;
        assert_eq!(
            startup_due(held, cur, false, seated, Some(now), later),
            Startup::Now { settled: false }
        );
        let described = Arrival {
            seat_frame_known: true,
            ..seated
        };
        assert_eq!(
            startup_due(held, cur, false, described, Some(now), now),
            Startup::Now { settled: true }
        );
    }

    #[test]
    fn a_followed_teleport_is_persisted_when_the_avatar_has_arrived() {
        let mut bot = follower();
        // As the bot is after it has accepted a forced teleport while holding.
        bot.follow.moved(rlv::Movement::Teleport, true, false);
        bot.set_manual(true);
        hear(&mut bot, SessionEvent::RegionEntered("Home".to_string()));
        assert_eq!(bot.book.held().to_string(), "Home/40/50/22 stand");
        assert!(!bot.reconciler.is_manual());
        // Once: the next arrival is not that teleport's.
        bot.world = Some(Box::new(Somewhere(at("Target", 1.0, 2.0, 3.0), persist::Sitting::Standing)));
        hear(&mut bot, SessionEvent::TeleportArrived);
        assert_eq!(bot.book.held().to_string(), "Home/40/50/22 stand");
    }

    #[test]
    fn a_followed_teleport_that_fails_leaves_everything_as_it_was() {
        for was_manual in [false, true] {
            for event in [
                SessionEvent::TeleportFailed("region \"Nowhere\" not found".to_string()),
                SessionEvent::Lost("kicked".to_string()),
            ] {
                let mut bot = follower();
                bot.set_manual(was_manual);
                let held = bot.book.held().clone();
                bot.follow.moved(rlv::Movement::Teleport, true, was_manual);
                bot.set_manual(true);
                hear(&mut bot, event.clone());
                assert_eq!(bot.reconciler.is_manual(), was_manual, "{event:?}");
                assert_eq!(bot.book.held(), &held, "{event:?}");
                // And an arrival later on is not followed.
                hear(&mut bot, SessionEvent::RegionEntered("Home".to_string()));
                assert_eq!(bot.book.held(), &held, "{event:?}");
            }
        }
    }

    #[test]
    fn a_followed_teleport_is_given_up_when_something_else_is_persisted() {
        let mut bot = follower();
        bot.follow.moved(rlv::Movement::Teleport, true, false);
        bot.set_manual(true);
        ok(&mut bot, "persist\tcfg\tcfg");
        let held = bot.book.held().clone();
        hear(&mut bot, SessionEvent::RegionEntered("Home".to_string()));
        assert_eq!(bot.book.held(), &held);
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn a_followed_stand_or_ground_sit_is_persisted_at_once() {
        for (movement, sit) in [(rlv::Movement::Stand, "stand"), (rlv::Movement::Ground, "ground")] {
            let mut bot = follower();
            // As after the command was handed to the simulator connection.
            let after = bot.follow.moved(movement, true, false);
            bot.set_manual(true);
            bot.after_forced(after, "", Instant::now());
            assert_eq!(bot.book.held().to_string(), format!("Home/40/50/22 {sit}"));
            assert!(!bot.reconciler.is_manual());
        }
        // A persist that cannot be carried out leaves the bot in manual mode.
        let mut bot = follower();
        bot.world = None;
        let held = bot.book.held().clone();
        let after = bot.follow.moved(rlv::Movement::Stand, true, false);
        bot.set_manual(true);
        bot.after_forced(after, "", Instant::now());
        assert!(bot.reconciler.is_manual());
        assert_eq!(bot.book.held(), &held);
    }

    #[test]
    fn owner_chat_is_read_as_rlv_only_while_rlv_is_on() {
        let now = Instant::now();
        // Off: the line is chat and nothing else.
        let mut off = bot(full(""));
        hear(&mut off, owner_say("@versionnum=5"));
        assert!(!off.rlv.has_waiting());
        assert_eq!(off.recorder.flush_all(now).len(), 1);
        // On: it is recorded all the same, and, with the avatar not in a
        // region yet, kept until it is.
        let mut on = bot(full("rlv = true\n"));
        hear(&mut on, owner_say("@versionnum=5"));
        hear(&mut on, owner_say("@unsit=force"));
        assert!(on.rlv.has_waiting());
        assert_eq!(on.recorder.flush_all(now).len(), 2);
        // What waits goes with the session, and with RLV.
        on.drop_session();
        assert!(!on.rlv.has_waiting());
        hear(&mut on, owner_say("@versionnum=5"));
        assert_eq!(ok(&mut on, "rlv\tmode\toff"), "rlv: off");
        assert!(!on.rlv.has_waiting());
        // `rlv cmd` is answered, not kept.
        assert_eq!(ok(&mut on, "rlv\tmode\ton"), "rlv: on");
        refused(&mut on, "rlv\tcmd\t@versionnum=5");
        assert!(!on.rlv.has_waiting());
    }

    #[test]
    fn other_chat_is_never_read_as_rlv() {
        let mut bot = bot(full("rlv = true\n"));
        let say = |kind: &str, source: session::Source, text: &str| {
            SessionEvent::Chat(session::Message {
                kind: kind.to_string(),
                source,
                sent_at: None,
                text: text.to_string(),
            })
        };
        let object = || session::Source::Object {
            id: Some(Uuid::from_bytes([7; 16])),
            name: "Collar".to_string(),
            owner: Uuid::from_bytes([1; 16]),
        };
        let avatar = || session::Source::Avatar {
            id: Uuid::from_bytes([2; 16]),
            name: "Jane Doe".to_string(),
        };
        hear(&mut bot, say("say", object(), "@unsit=force"));
        hear(&mut bot, say("owner", object(), "Battery low"));
        hear(&mut bot, say("say", avatar(), "@unsit=force"));
        hear(&mut bot, say("owner", avatar(), "@unsit=force"));
        let message = session::Message {
            kind: "im".to_string(),
            source: avatar(),
            sent_at: None,
            text: "@version".to_string(),
        };
        hear(&mut bot, SessionEvent::InstantMessage(message));
        assert!(!bot.rlv.has_waiting());
        // All of it is recorded as what it is.
        assert_eq!(bot.recorder.flush_all(Instant::now()).len(), 5);
    }
    // ------------------------------------------------- startup script

    const JANE_OBJECT: &str = "01234567-9abc-defe-dcba-987654321000";

    /// A bot with `script` as its startup script and `more` after it in
    /// the config, the head of the script carried out.
    fn scripted(more: &str, script: &str) -> Bot {
        let mut bot = bot(full(&format!("startup = '''\n{script}'''\n{more}")));
        bot.startup_head(Instant::now()).unwrap();
        bot
    }

    /// The same on a simulator that takes everything, with the avatar
    /// where the config puts it: seated on the configured object.
    fn in_place(more: &str, script: &str) -> Bot {
        let mut bot = scripted(more, script);
        bot.grid = Some(commands::tests::FakeGrid::default());
        bot.world = Some(Box::new(Somewhere(
            at("Target", 129.33, 127.6, 26.45),
            persist::Sitting::Object(object()),
        )));
        let setting = bot.startup.expect("the config's persist is still to be applied");
        bot.startup_reached(setting, true, Instant::now());
        bot
    }

    /// What the simulator was asked for since the last look.
    fn asked(bot: &mut Bot) -> Vec<commands::tests::Call> {
        std::mem::take(&mut bot.grid.as_mut().unwrap().calls)
    }

    /// Hands the bot a command whose answer may come later.
    fn send(bot: &mut Bot, line: &str) -> Receiver<Response> {
        let (reply, answer) = mpsc::channel();
        let request = Request::decode(line.as_bytes()).unwrap();
        assert!(matches!(bot.control(request, reply, Instant::now()), Flow::Continue));
        answer
    }

    fn said(text: &str) -> commands::tests::Call {
        commands::tests::Call::Say(0, 1, text.to_string())
    }

    fn script_is_over(bot: &Bot) -> bool {
        !bot.script.waiting() && !bot.script.running()
    }

    #[test]
    fn the_head_of_the_script_sets_what_the_deprecated_keys_set() {
        let mut scripted = scripted(
            "",
            &format!(
                "# as the tables below\nrlv mode on\nrlv persist follow\nrlv persist override sit tp\n\
                 perms autoaccept tp \"Jane Doe\"\nperms autoaccept anim all\nperms revoke anim {JANE_OBJECT}\n\
                 autoreply message Away\nautoreply pause 2h\n"
            ),
        );
        let mut keyed = bot(full(&format!(
            "rlv = true\n[persist]\nrlv = \"follow\"\noverride = [\"sit\", \"tp\"]\n\
             [perms.autoaccept]\ntp = [\"Jane Doe\"]\nanim = \"all\"\n[perms.except]\nanim = [\"{JANE_OBJECT}\"]\n\
             [autoreply]\nmessage = \"Away\"\npause = \"2h\"\n"
        )));
        for status in ["rlv\tstatus", "perms\tstatus", "autoreply\tstatus"] {
            assert_eq!(ok(&mut scripted, status), ok(&mut keyed, status), "{status}");
        }
        assert!(ok(&mut scripted, "rlv\tstatus").starts_with("rlv: on\npersist: follow\npersist override: sit, tp"));
        // None of it touched the placement, and nothing is left to run.
        assert!(!scripted.reconciler.is_manual());
        assert_eq!(scripted.startup, Some(Persist::Off));
        assert!(script_is_over(&scripted));
    }

    #[test]
    fn the_head_comes_after_the_deprecated_keys() {
        let mut bot = scripted("rlv = true\n", "rlv mode off\n");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: off"));
        let mut bot = scripted("[autoreply]\nmessage = \"Away\"\npause = \"2h\"\n", "autoreply pause 30m\n");
        assert_eq!(
            ok(&mut bot, "autoreply\tstatus"),
            "autoreply: on\nmessage: Away\npause: 30m\navatars in their pause: 0"
        );
    }

    #[test]
    fn the_head_ends_at_the_first_command_that_is_no_setting() {
        let mut bot = scripted("", "rlv mode on\nsay hello\nrlv persist follow\n");
        let status = ok(&mut bot, "rlv\tstatus");
        assert!(status.starts_with("rlv: on\npersist: drop"), "{status}");
        assert!(bot.script.waiting());
        // A script that starts with another command has no head.
        let mut bot = scripted("", "say hello\nrlv mode on\n");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: off"));
        assert!(bot.script.waiting());
    }

    #[test]
    fn a_setting_command_refused_at_the_head_is_reported_with_its_line() {
        let head = |script: &str| {
            let mut bot = bot(full(&format!("startup = '''\n{script}'''\n")));
            bot.startup_head(Instant::now()).unwrap_err()
        };
        let error = head("rlv mode on\nperms autoaccept money all\nsay hello\n");
        assert_eq!(error.line, 2);
        assert!(
            error.reason.contains("`tp`") && error.reason.contains("`camera`"),
            "{}",
            error.reason
        );
        let error = head("# one\n\nautoreply pause soon\n");
        assert_eq!(error.line, 3);
        let error = head("rlv mode override\n");
        assert!(error.reason.contains("off"), "{}", error.reason);
        let error = head("autoreply on\n");
        assert!(error.reason.contains("autoreply message"), "{}", error.reason);
    }

    #[test]
    fn a_waiting_script_waits_for_the_seats_position() {
        let now = Instant::now();
        let seated = Arrival {
            seated_on_target: true,
            ..arrived()
        };
        let known = Arrival {
            seat_frame_known: true,
            ..seated
        };
        let sit = SitState::Object(object());
        for setting in [CFG, Persist::Off] {
            // With a rest of the script: as a current location waits.
            assert_eq!(
                startup_due(sit, setting, true, seated, None, now),
                Startup::Wait(now + SEAT_POSITION_WAIT),
                "{setting:?}"
            );
            assert_eq!(
                startup_due(sit, setting, true, seated, Some(now), now + SEAT_POSITION_WAIT),
                Startup::Now { settled: false }
            );
            assert_eq!(startup_due(sit, setting, true, known, None, now), Startup::Now { settled: true });
            // Without one: as before.
            assert_eq!(startup_due(sit, setting, false, seated, None, now), Startup::Now { settled: true });
        }
        // Nothing is waited for when the avatar is not to sit on an object,
        // and nothing is due before the placement is reached.
        assert_eq!(
            startup_due(SitState::Stand, Persist::Off, true, arrived(), None, now),
            Startup::Now { settled: true }
        );
        assert_eq!(startup_due(sit, Persist::Off, true, arrived(), None, now), Startup::NotYet);
    }

    #[test]
    fn the_rest_starts_when_the_config_placement_is_reached() {
        let mut bot = scripted("", "rlv mode on\nsay hello\n");
        bot.grid = Some(commands::tests::FakeGrid::default());
        // Not before: the avatar is not in place.
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
        assert!(bot.script.waiting());
        bot.startup_reached(Persist::Off, true, Instant::now());
        // Without `persist` the bot goes to manual mode, and the rest runs.
        assert!(bot.reconciler.is_manual());
        assert!(bot.script.running());
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
        assert!(script_is_over(&bot));
        assert_eq!(bot.startup_rest(), Ok(false));
    }

    #[test]
    fn persist_in_the_script_holds_what_the_deprecated_setting_held() {
        // `persist`: where the avatar is and what it sits on.
        let mut scripted = in_place("", "persist\n");
        assert!(scripted.reconciler.is_manual());
        assert_eq!(scripted.startup_rest(), Ok(true));
        let mut keyed = in_place("persist = \"cur\"\n", "");
        assert_eq!(scripted.book.held(), keyed.book.held());
        assert_eq!(scripted.book.held().location, Some(at("Target", 129.33, 127.6, 26.45)));
        assert!(!scripted.reconciler.is_manual() && !keyed.reconciler.is_manual());
        assert_eq!(ok(&mut scripted, "status"), ok(&mut keyed, "status"));
        // `persist cfg cfg`: the config placement.
        let mut scripted = in_place("", "persist cfg cfg\n");
        assert_eq!(scripted.startup_rest(), Ok(true));
        let mut keyed = in_place("persist = \"cfg\"\n", "");
        assert_eq!(scripted.book.held(), keyed.book.held());
        assert_eq!(scripted.book.held().location, Some(at("Target", 128.0, 128.0, 25.0)));
        assert_eq!(ok(&mut scripted, "status"), ok(&mut keyed, "status"));
        // Neither asked the simulator for anything.
        assert!(asked(&mut scripted).is_empty());
    }

    #[test]
    fn the_deprecated_persist_is_applied_before_the_rest_runs() {
        let mut bot = in_place("persist = \"cfg\"\n", "say hello\n");
        assert!(!bot.reconciler.is_manual());
        assert_eq!(bot.book.held().location, Some(at("Target", 128.0, 128.0, 25.0)));
        // Applying it did not give the script up.
        assert!(bot.script.running());
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
    }

    #[test]
    fn the_rest_runs_one_command_at_a_time() {
        let seat = Uuid::from_bytes([9; 16]);
        let mut bot = in_place("", &format!("tp 128 128 25 Home\nsit {seat}\nsay hello\n"));
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(
            asked(&mut bot),
            [commands::tests::Call::Teleport("Home".to_string(), [128.0, 128.0, 25.0])]
        );
        // The sit waits for the teleport's answer.
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
        bot.commands.arrived("Home");
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [commands::tests::Call::Sit(seat)]);
        assert_eq!(bot.startup_rest(), Ok(false));
        bot.commands.seated(seat);
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
        assert!(script_is_over(&bot));
        // The script's commands are the operator's: they moved the avatar.
        assert!(bot.reconciler.is_manual());
    }

    #[test]
    fn a_refused_command_stops_the_script_and_not_the_bot() {
        let mut bot = in_place("", "sit chair\nsay hello\nsay again\n");
        assert_eq!(bot.startup_rest(), Ok(true));
        assert!(asked(&mut bot).is_empty());
        assert!(script_is_over(&bot));
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
        // The bot goes on taking commands.
        assert!(ok(&mut bot, "status").contains("mode: manual"));
        // A command that fails later stops it as well.
        let seat = Uuid::from_bytes([9; 16]);
        let mut bot = in_place("", &format!("sit {seat}\npersist\n"));
        assert_eq!(bot.startup_rest(), Ok(true));
        assert!(bot.commands.sit_failed("there is no such object"));
        assert_eq!(bot.startup_rest(), Ok(true));
        assert!(script_is_over(&bot));
        assert!(bot.reconciler.is_manual());
    }

    #[test]
    fn a_setting_command_in_the_rest_that_is_refused_stops_the_script_only() {
        let mut bot = in_place("", "say hello\nautoreply pause soon\nsay again\n");
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
        assert!(script_is_over(&bot));
    }

    #[test]
    fn quit_in_the_script_stops_the_bot() {
        let mut bot = in_place("", "say done\nquit\nsay never\n");
        assert_eq!(bot.startup_rest(), Err(EXIT_OK));
        assert_eq!(asked(&mut bot), [said("done")]);
        assert!(bot.stopping);
        // Nothing of the script comes after it: only its answer is taken.
        assert_eq!(bot.startup_rest(), Ok(true));
        assert!(script_is_over(&bot));
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
    }

    #[test]
    fn the_operator_acting_first_gives_the_rest_up_and_keeps_the_settings() {
        for line in ["tp\t128\t128\t25\tHome", "persist\tcfg\tcfg", "home"] {
            let mut bot = scripted("", "rlv mode on\nsay hello\npersist\n");
            bot.grid = Some(commands::tests::FakeGrid::default());
            assert!(bot.script.waiting(), "{line}");
            let _answer = send(&mut bot, line);
            asked(&mut bot);
            assert!(script_is_over(&bot), "{line}");
            assert_eq!(bot.startup, None, "{line}");
            assert_eq!(bot.startup_rest(), Ok(false), "{line}");
            assert!(asked(&mut bot).is_empty(), "{line}");
            assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on"), "{line}");
        }
        // A command that moves nothing, or is refused, leaves it waiting.
        let mut bot = scripted("", "rlv mode on\nsay hello\npersist\n");
        bot.grid = Some(commands::tests::FakeGrid::default());
        for line in ["status", "say\thi", "sit\tchair", "persist\tcur\tchair", "rlv\tmode\toff"] {
            let _answer = send(&mut bot, line);
            assert!(bot.script.waiting(), "{line}");
        }
    }

    #[test]
    fn a_running_script_is_not_stopped_by_the_operator_or_by_its_own_persist() {
        let mut bot = in_place("", "tp 128 128 25 Home\npersist cfg cfg\nsay hello\n");
        assert_eq!(bot.startup_rest(), Ok(true));
        asked(&mut bot);
        // The operator's commands while the teleport is on its way.
        assert!(ok(&mut bot, "status").starts_with("connected"));
        assert!(matches!(command(&mut bot, "say\thi"), Response::Ok(_)));
        assert!(matches!(command(&mut bot, "persist\tcfg\tcfg"), Response::Ok(_)));
        assert_eq!(asked(&mut bot), [said("hi")]);
        assert!(bot.script.running());
        assert_eq!(bot.startup_rest(), Ok(false));
        // The script goes on where it was, through its own persist.
        bot.commands.arrived("Home");
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
        assert!(script_is_over(&bot));
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn a_followed_forced_sit_neither_starts_nor_stops_the_script() {
        // While the script runs.
        let mut bot = in_place("", "tp 128 128 25 Home\nsay hello\n");
        assert_eq!(bot.startup_rest(), Ok(true));
        asked(&mut bot);
        bot.startup = Some(CUR);
        bot.follow_seat = true;
        bot.startup_reached(CUR, true, Instant::now());
        assert!(bot.script.running());
        bot.commands.arrived("Home");
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
        // And when it is done.
        assert!(script_is_over(&bot));
        bot.startup = Some(CUR);
        bot.follow_seat = true;
        bot.startup_reached(CUR, true, Instant::now());
        assert!(script_is_over(&bot));
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
    }

    #[test]
    fn the_script_is_carried_out_once_in_a_run() {
        // Done, then the session is lost and the bot logs in again.
        let mut bot = in_place("", "say hello\n");
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
        bot.drop_session();
        assert_eq!(bot.startup, None);
        assert_eq!(bot.apply_startup(Instant::now()), None);
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
        assert!(script_is_over(&bot));
        // Stopped by a session loss in the middle: not taken up again.
        let mut bot = in_place("", "tp 128 128 25 Home\nsit ground\n");
        assert_eq!(bot.startup_rest(), Ok(true));
        asked(&mut bot);
        bot.drop_session();
        assert_eq!(bot.startup_rest(), Ok(true));
        assert!(script_is_over(&bot));
        assert_eq!(bot.startup_rest(), Ok(false));
        assert!(asked(&mut bot).is_empty());
    }

    #[test]
    fn a_session_lost_before_the_placement_is_reached_keeps_the_rest_waiting() {
        let mut bot = scripted("", "rlv mode on\nsay hello\n");
        bot.grid = Some(commands::tests::FakeGrid::default());
        bot.drop_session();
        assert!(bot.script.waiting());
        assert_eq!(bot.startup, Some(Persist::Off));
        assert_eq!(bot.startup_rest(), Ok(false));
        // The next login gets the avatar in place.
        bot.startup_reached(Persist::Off, true, Instant::now());
        assert_eq!(bot.startup_rest(), Ok(true));
        assert_eq!(asked(&mut bot), [said("hello")]);
    }

    // ------------------------------------------------------ autoreply

    fn answering(more: &str) -> Bot {
        let mut bot = bot(full(&format!("{more}[autoreply]\nmessage = \"Away\"\npause = \"2h\"\n")));
        bot.grid = Some(commands::tests::FakeGrid::default());
        bot
    }

    fn jane_id() -> Uuid {
        Uuid::from_bytes([0x1a; 16])
    }

    fn im_from(id: Uuid, name: &str, text: &str) -> SessionEvent {
        SessionEvent::InstantMessage(session::Message {
            kind: "im".to_string(),
            source: session::Source::Avatar {
                id,
                name: name.to_string(),
            },
            sent_at: None,
            text: text.to_string(),
        })
    }

    fn janes(text: &str) -> SessionEvent {
        im_from(jane_id(), "Jane Doe", text)
    }

    /// The automatic answers sent since the last call.
    fn answers(bot: &mut Bot) -> Vec<(Uuid, String)> {
        let calls = std::mem::take(&mut bot.grid.as_mut().unwrap().calls);
        calls
            .into_iter()
            .filter_map(|call| match call {
                commands::tests::Call::AutoIm(to, text) => Some((to, text)),
                _ => None,
            })
            .collect()
    }

    /// The instant messages recorded since the last call: kind and text.
    fn recorded(bot: &mut Bot) -> Vec<(String, String)> {
        let records = bot.recorder.flush_all(Instant::now());
        records
            .into_iter()
            .map(|record| record.message().clone())
            .map(|record| (record.kind, record.text))
            .collect()
    }

    fn pair(kind: &str, text: &str) -> (String, String) {
        (kind.to_string(), text.to_string())
    }

    fn in_builders(bot: &mut Bot) -> Uuid {
        let builders = crate::groups::tests::builders();
        let groups = vec![crate::groups::tests::member(builders, "Builders", 0, true)];
        hear(bot, SessionEvent::Groups(groups));
        builders
    }

    fn group_line(group: Uuid, text: &str) -> SessionEvent {
        SessionEvent::GroupChat {
            session: group,
            from: jane_id(),
            name: "Jane Doe".into(),
            text: text.into(),
        }
    }

    /// The group messages recorded since the last call: kind and text.
    fn group_records(bot: &mut Bot) -> Vec<(String, String)> {
        bot.take_transactions(Instant::now());
        bot.recorder
            .flush_all(Instant::now())
            .into_iter()
            .filter_map(|recorded| match recorded {
                Recorded::Group(message) => Some((message.kind.name().to_string(), message.text)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_line_of_group_chat_gets_no_autoreply_and_changes_no_pause() {
        let mut bot = answering("");
        let builders = in_builders(&mut bot);
        hear(&mut bot, group_line(builders, "say hello"));
        assert_eq!(answers(&mut bot), []);
        assert_eq!(group_records(&mut bot), [pair("chat", "say hello")]);
        // Her pause has not begun: her instant message is answered.
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), "Away".to_string())]);
    }

    #[test]
    fn a_line_the_bot_says_carries_its_own_name() {
        let mut bot = answering("");
        let builders = in_builders(&mut bot);
        hear(&mut bot, group_line(builders, "first"));
        let answer = send(&mut bot, "group\tchat\tBuilders\thi");
        hear(
            &mut bot,
            SessionEvent::GroupSession {
                group: builders,
                ok: true,
                error: String::new(),
            },
        );
        assert_eq!(
            answer.try_recv().unwrap(),
            Response::Ok(Some(format!("said in \"Builders\" <{builders}>")))
        );
        bot.take_transactions(Instant::now());
        let records = bot.recorder.flush_all(Instant::now());
        let Some(Recorded::Group(said)) = records.last() else {
            panic!("{records:?}")
        };
        assert_eq!(said.kind, grouplog::Kind::ChatOut);
        assert_eq!(said.avatar_name, format!("{} {}", bot.config.first_name, bot.config.last_name));
    }

    #[test]
    fn group_list_info_and_invitations_never_use_up_an_override() {
        let mut bot = answering("rlv = true\n");
        in_builders(&mut bot);
        ok(&mut bot, "rlv\tmode\toverride");
        for line in ["group\tlist", "group\tinvitations"] {
            command(&mut bot, line);
            assert!(ok(&mut bot, "rlv\tstatus").contains("rlv: override"), "{line}");
        }
        send(&mut bot, "group\tinfo\tBuilders");
        assert!(ok(&mut bot, "rlv\tstatus").contains("rlv: override"));
        send(&mut bot, "group\tactivate\tBuilders");
        assert!(ok(&mut bot, "rlv\tstatus").contains("rlv: on"));
    }

    #[test]
    fn an_instant_message_is_answered_once_within_the_pause() {
        let mut bot = answering("");
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), "Away".to_string())]);
        assert_eq!(recorded(&mut bot), [pair("im", "hi"), pair("im_auto", "Away")]);
        hear(&mut bot, janes("hello?"));
        assert_eq!(answers(&mut bot), []);
        // Another avatar has a pause of its own.
        hear(&mut bot, im_from(Uuid::from_bytes([0x2b; 16]), "John Roe", "hi"));
        assert_eq!(answers(&mut bot).len(), 1);
    }

    #[test]
    fn objects_the_grid_and_stored_messages_are_not_answered() {
        let mut bot = answering("");
        let object = session::Message {
            kind: "object".to_string(),
            source: session::Source::Object {
                id: Some(Uuid::from_bytes([7; 16])),
                name: "Collar".to_string(),
                owner: jane_id(),
            },
            sent_at: None,
            text: "locked".to_string(),
        };
        hear(&mut bot, SessionEvent::InstantMessage(object));
        let grid = session::Message {
            kind: "im".to_string(),
            source: session::Source::System {
                name: "Second Life".to_string(),
            },
            sent_at: None,
            text: "maintenance".to_string(),
        };
        hear(&mut bot, SessionEvent::InstantMessage(grid));
        let SessionEvent::InstantMessage(mut stored) = janes("earlier") else {
            unreachable!()
        };
        stored.sent_at = Some(1_700_000_000);
        hear(&mut bot, SessionEvent::InstantMessage(stored));
        assert_eq!(answers(&mut bot), []);
        assert_eq!(recorded(&mut bot).len(), 3);
        // The stored message started no pause.
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot).len(), 1);
    }

    #[test]
    fn nothing_is_answered_without_an_autoreply_table() {
        let mut bot = bot(full(""));
        bot.grid = Some(commands::tests::FakeGrid::default());
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), []);
        assert_eq!(recorded(&mut bot), [pair("im", "hi")]);
    }

    #[test]
    fn an_answer_that_cannot_be_sent_is_not_recorded_and_starts_no_pause() {
        let mut bot = answering("");
        bot.grid.as_mut().unwrap().offline = true;
        hear(&mut bot, janes("hi"));
        assert_eq!(recorded(&mut bot), [pair("im", "hi")]);
        bot.grid.as_mut().unwrap().offline = false;
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot).len(), 1);
    }

    #[test]
    fn an_instant_message_on_command_starts_the_pause() {
        let mut bot = answering("");
        ok(&mut bot, &format!("im\t{}\tone moment", jane_id()));
        bot.names_resolved(vec![], vec![jane_id()], Instant::now());
        assert!(bot
            .grid
            .as_mut()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, commands::tests::Call::Im(..))));
        hear(&mut bot, janes("ok"));
        assert_eq!(answers(&mut bot), []);
    }

    #[test]
    fn sendim_holds_an_autoreply_back_and_startim_does_not() {
        let mut bot = answering("rlv = true\n");
        ok(&mut bot, "rlv\tcmd\t@sendim=n");
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), []);
        assert_eq!(recorded(&mut bot), [pair("im", "hi")]);
        // Held back, the answer started no pause.
        ok(&mut bot, "rlv\tcmd\t@sendim=y,startim=n");
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), "Away".to_string())]);
    }

    #[test]
    fn recvim_hides_a_message_and_rlv_answers_it() {
        let mut bot = answering("rlv = true\n");
        ok(&mut bot, "rlv\tcmd\t@recvim=n");
        hear(&mut bot, janes("are you there?"));
        assert_eq!(answers(&mut bot), [(jane_id(), RLV_RECVIM_REPLY.to_string())]);
        assert_eq!(
            recorded(&mut bot),
            [pair("im", "(blocked by RLV: recvim)"), pair("im_auto", RLV_RECVIM_REPLY)]
        );
        // Every blocked message is answered, and none starts a pause.
        hear(&mut bot, janes("hello?"));
        assert_eq!(answers(&mut bot), [(jane_id(), RLV_RECVIM_REPLY.to_string())]);
        ok(&mut bot, "rlv\tcmd\t@recvim=y");
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), "Away".to_string())]);
    }

    #[test]
    fn recvim_answers_whether_autoreply_is_on_or_not() {
        let mut bot = bot(full("rlv = true\n"));
        bot.grid = Some(commands::tests::FakeGrid::default());
        ok(&mut bot, &format!("rlv\tcmd\t@recvimfrom:{}=n", jane_id()));
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), RLV_RECVIM_REPLY.to_string())]);
        hear(&mut bot, im_from(Uuid::from_bytes([0x2b; 16]), "John Roe", "hi"));
        assert_eq!(answers(&mut bot), []);
        let rule = format!("recvimfrom:{}", jane_id());
        assert_eq!(
            recorded(&mut bot),
            [
                pair("im", &format!("(blocked by RLV: {rule})")),
                pair("im_auto", RLV_RECVIM_REPLY),
                pair("im", "hi")
            ]
        );
    }

    #[test]
    fn recvim_exceptions_rlv_off_objects_and_stored_messages() {
        let mut bot = answering("rlv = true\n");
        ok(&mut bot, &format!("rlv\tcmd\t@recvim=n,recvim:{}=add", jane_id()));
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), "Away".to_string())]);
        recorded(&mut bot);
        // A stored message is hidden, and not answered.
        let SessionEvent::InstantMessage(mut stored) = im_from(Uuid::from_bytes([0x2b; 16]), "John Roe", "earlier") else {
            unreachable!()
        };
        stored.sent_at = Some(1_700_000_000);
        hear(&mut bot, SessionEvent::InstantMessage(stored));
        assert_eq!(answers(&mut bot), []);
        assert_eq!(recorded(&mut bot), [pair("im", "(blocked by RLV: recvim)")]);
        // An object's message is not blocked.
        let object = session::Message {
            kind: "object".to_string(),
            source: session::Source::Object {
                id: Some(Uuid::from_bytes([7; 16])),
                name: "Collar".to_string(),
                owner: jane_id(),
            },
            sent_at: None,
            text: "locked".to_string(),
        };
        hear(&mut bot, SessionEvent::InstantMessage(object));
        assert_eq!(recorded(&mut bot), [pair("object", "locked")]);
        // With RLV off nothing is blocked.
        ok(&mut bot, "rlv\tmode\toff");
        hear(&mut bot, im_from(Uuid::from_bytes([0x2b; 16]), "John Roe", "hi"));
        assert_eq!(answers(&mut bot), [(Uuid::from_bytes([0x2b; 16]), "Away".to_string())]);
        assert_eq!(recorded(&mut bot), [pair("im", "hi"), pair("im_auto", "Away")]);
    }

    #[test]
    fn autoreply_command_changes_the_setting() {
        let mut bot = answering("");
        assert_eq!(
            ok(&mut bot, "autoreply\tstatus"),
            "autoreply: on\nmessage: Away\npause: 2h\navatars in their pause: 0"
        );
        assert_eq!(
            ok(&mut bot, "autoreply\tmessage\tNew autoreply from the bot"),
            "autoreply: on, message: New autoreply from the bot"
        );
        assert_eq!(ok(&mut bot, "autoreply\tpause\t30m"), "autoreply pause: 30m");
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), [(jane_id(), "New autoreply from the bot".to_string())]);
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot), []);
        assert!(ok(&mut bot, "autoreply\tstatus").ends_with("pause: 30m\navatars in their pause: 1"));
        assert_eq!(ok(&mut bot, "autoreply\trefresh"), "autoreply: forgot the pause of 1 avatar");
        hear(&mut bot, janes("hi"));
        assert_eq!(answers(&mut bot).len(), 1);
        assert_eq!(ok(&mut bot, "autoreply\toff"), "autoreply: off");
        assert_eq!(ok(&mut bot, "autoreply\toff"), "autoreply: already off; nothing changed");
        hear(&mut bot, im_from(Uuid::from_bytes([0x2b; 16]), "John Roe", "hi"));
        assert_eq!(answers(&mut bot), []);
        assert_eq!(ok(&mut bot, "autoreply\ton"), "autoreply: on");
        hear(&mut bot, im_from(Uuid::from_bytes([0x2b; 16]), "John Roe", "hi"));
        assert_eq!(
            answers(&mut bot),
            [(Uuid::from_bytes([0x2b; 16]), "New autoreply from the bot".to_string())]
        );
        assert_eq!(ok(&mut bot, "autoreply\trefresh"), "autoreply: forgot the pauses of 2 avatars");
    }

    #[test]
    fn autoreply_command_refusals_change_nothing() {
        let mut plain = bot(full(""));
        assert!(refused(&mut plain, "autoreply\ton").contains("`autoreply message TEXT`"));
        assert!(ok(&mut plain, "autoreply\tstatus").starts_with("autoreply: off\nmessage: (none)\npause: 1h"));
        let mut bot = answering("");
        let reason = refused(&mut bot, "autoreply\tpause\tsoon");
        assert!(
            reason.contains("`soon` is not a duration") && reason.contains("`1h30m`"),
            "{reason}"
        );
        assert!(refused(&mut bot, "autoreply\tmessage\t   ").contains("blank"));
        assert!(refused(&mut bot, &format!("autoreply\tmessage\t{}", "x".repeat(1024))).contains("1023"));
        assert_eq!(
            ok(&mut bot, "autoreply\tstatus"),
            "autoreply: on\nmessage: Away\npause: 2h\navatars in their pause: 0"
        );
    }

    #[test]
    fn autoreply_command_is_never_refused_by_rlv_and_leaves_override_alone() {
        let mut bot = bot(full("rlv = true\n"));
        ok(&mut bot, "rlv\tcmd\t@sendim=n,recvim=n,sendchat=n");
        ok(&mut bot, "rlv\tmode\toverride");
        assert_eq!(ok(&mut bot, "autoreply\tpause\t10m"), "autoreply pause: 10m");
        assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: override"));
    }

    // ------------------------------------------------ permission requests

    fn holder(perms: &str) -> Bot {
        bot(full(&format!("persist = \"cfg\"\n{perms}")))
    }

    fn janes_offer(byte: u8) -> SessionEvent {
        SessionEvent::TeleportOffer {
            from: Uuid::from_bytes([2; 16]),
            name: "Jane Doe".to_string(),
            lure: Uuid::from_bytes([byte; 16]),
        }
    }

    #[test]
    fn the_configs_perms_are_the_rules_the_bot_starts_with() {
        let mut bot = holder(
            "[perms.autoaccept]\ntp = [\"Jane Doe\"]\nanim = \"all\"\n\
             [perms.except]\nanim = [\"01234567-9abc-defe-dcba-987654321000\"]\n",
        );
        assert_eq!(
            ok(&mut bot, "perms\tstatus"),
            "tp: Jane Doe\ninventory: none\nanim: all except 01234567-9abc-defe-dcba-987654321000\n\
             attach: none\ncontrols: none\ncamera: none\ngroup: none"
        );
        // Without the table nothing is accepted by itself.
        let mut bot = holder("");
        assert!(ok(&mut bot, "perms\tstatus").starts_with("tp: none\ninventory: none\n"));
    }

    #[test]
    fn perms_commands_work_between_logins_and_leave_the_placement_mode() {
        let mut bot = holder("");
        assert_eq!(ok(&mut bot, "perms\tautoaccept\ttp\tJane Doe"), "tp: Jane Doe is auto-accepted");
        assert_eq!(
            ok(&mut bot, "perms\tautoaccept\ttp\tjane.doe"),
            "warning: jane.doe was auto-accepted for tp already"
        );
        assert_eq!(ok(&mut bot, "perms\trevoke\ttp\tall"), "tp: nothing is auto-accepted any more");
        assert!(refused(&mut bot, "perms\tautoaccept\tmoney\tall").contains("no kind of request"));
        assert!(!bot.reconciler.is_manual());
        // `accept` needs the avatar in a region, under either name.
        for line in ["accept\ttp", "accepttp", "accept\tanim", "accepttp\tJane Doe"] {
            assert_eq!(refused(&mut bot, line), session::NOT_CONNECTED, "{line}");
        }
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn a_teleport_offer_that_waits_leaves_the_bot_holding() {
        let mut bot = holder("");
        assert!(matches!(bot.apply(janes_offer(1), Instant::now()), Flow::Continue));
        assert!(!bot.reconciler.is_manual());
        assert_eq!(
            ok(&mut bot, "perms\tstatus").lines().last(),
            Some("waiting: tp from Jane Doe <02020202-0202-0202-0202-020202020202>")
        );
        // The session's end takes the offer with it.
        bot.drop_session();
        assert_eq!(ok(&mut bot, "perms\tstatus").lines().count(), 7);
    }

    #[test]
    fn a_teleport_offer_accepted_by_rule_puts_a_holding_bot_in_manual_mode() {
        let mut bot = holder("[perms.autoaccept]\ntp = [\"Jane Doe\"]\n");
        let held = bot.book.held().clone();
        assert!(!bot.reconciler.is_manual());
        // On a simulator that takes everything, as in `apply`.
        let mut grid = commands::tests::FakeGrid::default();
        let (jane, lure) = (Uuid::from_bytes([2; 16]), Uuid::from_bytes([1; 16]));
        bot.commands.teleport_offer(jane, "Jane Doe", lure, Some(&mut grid), Instant::now());
        bot.follow_commands();
        assert_eq!(grid.calls, [commands::tests::Call::AcceptTp(lure)]);
        assert!(bot.reconciler.is_manual());
        // What was held is what `home` returns to.
        assert_eq!(bot.book.held(), &held);
        assert_eq!(ok(&mut bot, "home"), held.to_string());
        assert!(!bot.reconciler.is_manual());

        // An avatar that is not listed changes nothing.
        let mut grid = commands::tests::FakeGrid::default();
        bot.commands
            .teleport_offer(Uuid::from_bytes([3; 16]), "John Roe", lure, Some(&mut grid), Instant::now());
        bot.follow_commands();
        assert!(grid.calls.is_empty());
        assert!(!bot.reconciler.is_manual());
    }

    #[test]
    fn granting_a_script_or_accepting_inventory_leaves_the_bot_holding() {
        let mut bot = holder("[perms.autoaccept]\nanim = \"all\"\ninventory = \"all\"\n");
        let mut grid = commands::tests::FakeGrid::default();
        let (ball, script, jane) = (Uuid::from_bytes([7; 16]), Uuid::from_bytes([8; 16]), Uuid::from_bytes([2; 16]));
        let now = Instant::now();
        bot.commands
            .permission_request(ball, script, "Dance Ball", "Jane Doe", 0x10, Some(&mut grid), now);
        bot.commands.object_owner(ball, jane, Uuid::nil(), Some(&mut grid), now);
        let offer = session::InventoryOffer {
            from: jane,
            name: "Jane Doe".to_string(),
            from_group: false,
            from_object: false,
            transaction: Uuid::from_bytes([9; 16]),
            asset_type: inventory::ASSET_NOTECARD,
            text: "Rules".to_string(),
        };
        bot.commands.inventory_offer(offer, Some(&mut grid), now);
        bot.follow_commands();
        assert!(grid.calls.contains(&commands::tests::Call::Grant(ball, script, 0x10)));
        assert_eq!(grid.jobs.len(), 1, "the folder for the notecard is being looked for");
        assert!(!bot.reconciler.is_manual());
    }

    // ------------------------------------------------------------- XMPP

    mod gateway {
        use super::*;
        use crate::xmpp::conn::tests::{Fake, SECRET};
        use crate::xmpp::stanza::escape_text;
        use commands::tests::{Call, FakeGrid};

        const OWNER: &str = "owner@rig.test";
        const DOMAIN: &str = "bot.rig.test";

        fn jane() -> Uuid {
            Uuid::from_bytes([0x1a; 16])
        }

        fn collar() -> Uuid {
            Uuid::from_bytes([0xc0; 16])
        }

        fn janes_names() -> (Uuid, names::Names) {
            let names = names::Names {
                legacy: "Jane Doe".into(),
                display: "Janey".into(),
            };
            (jane(), names)
        }

        /// A bot with a gateway whose server is `fake`, and on a simulator
        /// that takes everything when `grid`.
        struct Rig {
            bot: Bot,
            incoming: Receiver<Event>,
            fake: Fake,
        }

        impl Rig {
            fn new(grid: bool) -> Rig {
                Rig::with("", grid)
            }

            /// The same with `more` after the `[xmpp]` table.
            fn with(more: &str, grid: bool) -> Rig {
                let fake = Fake::new();
                let config = full(&format!(
                    "[xmpp]\ncomponent = \"{DOMAIN}\"\nserver = \"{}\"\nsecret = \"{SECRET}\"\nowners = [\"{OWNER}\"]\n{more}",
                    fake.address()
                ));
                let (events, incoming) = mpsc::channel();
                let mut bot = Bot::new(config, Logs::none(), events);
                bot.start_gateway();
                if grid {
                    bot.grid = Some(FakeGrid::default());
                }
                Rig { bot, incoming, fake }
            }

            fn connected(grid: bool) -> Rig {
                Rig::connected_with("", grid)
            }

            fn connected_with(more: &str, grid: bool) -> Rig {
                let mut rig = Rig::with(more, grid);
                rig.connect();
                rig.fake.expect(&format!("from='local@{DOMAIN}' to='{OWNER}'/>"));
                rig
            }

            fn connect(&mut self) {
                self.fake.accept();
                assert!(matches!(self.next(), Flow::Continue));
            }

            /// Handles the next event, as the main loop does.
            fn next(&mut self) -> Flow {
                let event = self.incoming.recv_timeout(Duration::from_secs(5)).expect("an event");
                let flow = self.bot.handle(event);
                self.bot.service_messages();
                self.bot.xmpp_presence();
                flow
            }

            fn from(&mut self, from: &str, to: &str, body: &str) -> Flow {
                self.fake.send(&format!(
                    "<message type='chat' from='{from}' to='{to}' id='m1'><body>{}</body></message>",
                    escape_text(body)
                ));
                self.next()
            }

            fn owner_sends(&mut self, to: &str, body: &str) -> Flow {
                self.from(&format!("{OWNER}/phone"), to, body)
            }

            fn calls(&mut self) -> Vec<Call> {
                std::mem::take(&mut self.bot.grid.as_mut().unwrap().calls)
            }

            /// The name lookup the last command started.
            fn lookup(&mut self) -> Uuid {
                match &self.calls()[..] {
                    [Call::Find(query, _)] => *query,
                    other => panic!("expected a lookup, got {other:?}"),
                }
            }

            /// Records a message whose names are known, as the session
            /// hands it over.
            fn heard(&mut self, channel: Channel, message: session::Message, names: Vec<(Uuid, names::Names)>) {
                let now = Instant::now();
                self.bot.names_resolved(names, vec![], now);
                self.bot.record(channel, message, now);
                self.bot.service_messages();
            }

            fn quiet(&mut self) -> String {
                self.fake.rest(Duration::from_millis(150))
            }
        }

        fn message(kind: &str, source: session::Source, text: &str) -> session::Message {
            session::Message {
                kind: kind.into(),
                source,
                sent_at: None,
                text: text.into(),
            }
        }

        fn from_jane(kind: &str, text: &str) -> session::Message {
            message(
                kind,
                session::Source::Avatar {
                    id: jane(),
                    name: "Jane Doe".into(),
                },
                text,
            )
        }

        #[test]
        fn on_connect_presence_comes_first_then_what_waited_then_live_messages() {
            let mut rig = Rig::new(false);
            // Recorded before the server accepted the component.
            rig.heard(Channel::Chat, from_jane("say", "early"), vec![janes_names()]);
            rig.connect();
            rig.heard(Channel::Chat, from_jane("say", "live"), vec![janes_names()]);
            let written = rig.fake.expect("<body>Janey (Jane Doe): live</body></message>");
            let domain = written.find(&format!(
                "<presence from='{DOMAIN}' to='{OWNER}'><show>away</show><status>not in a region</status></presence>"
            ));
            let local = written.find(&format!("<presence type='unavailable' from='local@{DOMAIN}' to='{OWNER}'/>"));
            let early = written.find("<body>Janey (Jane Doe): early</body><delay xmlns='urn:xmpp:delay'");
            let live = written.find(&format!(
                "<message type='chat' from='local@{DOMAIN}' to='{OWNER}'><body>Janey (Jane Doe): live</body></message>"
            ));
            assert!(
                domain.is_some() && local.is_some() && early.is_some() && live.is_some(),
                "{written}"
            );
            assert!(domain < local && local < early && early < live, "{written}");
        }

        #[test]
        fn strangers_are_refused_and_discovery_and_ping_answer_anyone() {
            let mut rig = Rig::connected(true);
            let stranger = "someone@elsewhere.org/x";
            assert!(matches!(rig.from(stranger, &format!("jane.doe@{DOMAIN}"), "hello"), Flow::Continue));
            let error = rig.fake.expect("</message>");
            assert!(
                error.contains("type='error'") && error.contains("<forbidden ") && error.contains(&format!("to='{stranger}'")),
                "{error}"
            );
            assert!(matches!(rig.from(stranger, DOMAIN, "quit"), Flow::Continue));
            assert!(rig.fake.expect("</message>").contains("<forbidden "));
            assert!(!rig.bot.stopping);
            assert_eq!(rig.calls(), []);

            rig.fake
                .send(&format!("<presence type='subscribe' from='someone@elsewhere.org' to='{DOMAIN}'/>"));
            rig.next();
            assert_eq!(
                rig.fake.expect("/>"),
                format!("<presence type='unsubscribed' from='{DOMAIN}' to='someone@elsewhere.org'/>")
            );

            rig.fake.send(&format!(
                "<iq type='get' id='d1' from='{stranger}' to='{DOMAIN}'><query xmlns='http://jabber.org/protocol/disco#info'/></iq>"
            ));
            rig.next();
            let disco = rig.fake.expect("</iq>");
            assert!(
                disco.contains("type='result'") && disco.contains("category='gateway'") && disco.contains("urn:xmpp:ping"),
                "{disco}"
            );
            rig.fake.send(&format!(
                "<iq type='get' id='p1' from='{stranger}' to='local@{DOMAIN}'><ping xmlns='urn:xmpp:ping'/></iq>"
            ));
            rig.next();
            assert_eq!(
                rig.fake.expect("/>"),
                format!("<iq type='result' from='local@{DOMAIN}' to='{stranger}' id='p1'/>")
            );
            rig.fake.send(&format!(
                "<iq type='get' id='v1' from='{OWNER}/phone' to='jane.doe@{DOMAIN}'><vCard xmlns='vcard-temp'/></iq>"
            ));
            rig.next();
            assert!(rig.fake.expect("</iq>").contains("<service-unavailable "));
            // Error stanzas are never answered.
            rig.fake.send(&format!(
                "<message type='error' from='{stranger}' to='{DOMAIN}'><error type='cancel'/></message>"
            ));
            rig.next();
            assert_eq!(rig.quiet(), "");
        }

        /// The body of the next message from the control contact.
        fn answer(rig: &mut Rig) -> String {
            let message = rig.fake.expect("</message>");
            assert!(
                message.starts_with(&format!("<message type='chat' from='{DOMAIN}' to='{OWNER}/phone'>")),
                "{message}"
            );
            let body = message.split_once("<body>").unwrap().1.split_once("</body>").unwrap().0;
            body.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
        }

        #[test]
        fn the_control_contact_is_the_interface_xmpp() {
            for (perms, shown) in [("[\"xmpp\"]", true), ("[\"ctl\"]", false)] {
                let mut rig = Rig::connected_with(&format!("[payments]\nlimit = \"unlimited\"\nperms = {perms}\n"), true);
                rig.owner_sends(DOMAIN, "status");
                let status = answer(&mut rig);
                assert_eq!(status.ends_with("balance: unknown\nlimit: unlimited"), shown, "{perms}: {status}");
                rig.owner_sends(DOMAIN, &format!("pay 10 avatar {}", jane()));
                if shown {
                    assert_eq!(rig.calls(), []);
                    let (id, names) = janes_names();
                    rig.bot.names_resolved(vec![(id, names)], Vec::new(), Instant::now());
                    assert_eq!(rig.calls(), [Call::Pay(jane(), 10, udp::messages::TRANSACTION_GIFT, String::new())]);
                } else {
                    assert!(answer(&mut rig).contains("`xmpp`"));
                }
            }
        }

        #[test]
        fn commands_are_answered_as_the_control_socket_answers_them() {
            let mut rig = Rig::connected(false);
            rig.owner_sends(DOMAIN, "status");
            assert_eq!(answer(&mut rig), rig.bot.status(Interface::Xmpp));
            rig.owner_sends(DOMAIN, "sit ground");
            let reason = refused(&mut rig.bot, "sit\tground");
            assert_eq!(answer(&mut rig), format!("error: {reason}"));
            rig.owner_sends(DOMAIN, "im \"Jane Doe hi");
            let unclosed = answer(&mut rig);
            assert!(unclosed.starts_with("error: ") && unclosed.contains("quote"), "{unclosed}");
            rig.owner_sends(DOMAIN, "dance");
            assert!(answer(&mut rig).starts_with("error: "));
            rig.owner_sends(DOMAIN, "");
            assert!(answer(&mut rig).contains("help"));
            rig.owner_sends(DOMAIN, "help");
            assert!(answer(&mut rig).contains("\nim AVATAR MESSAGE...\n"));
            rig.owner_sends(DOMAIN, &"x".repeat(8193));
            assert!(answer(&mut rig).contains("8192"));
            rig.owner_sends(DOMAIN, "autoreply message back soon");
            assert_eq!(answer(&mut rig), "autoreply: on, message: back soon");
            rig.owner_sends(DOMAIN, "autoreply off");
            assert_eq!(answer(&mut rig), "autoreply: off");
        }

        #[test]
        fn owners_messages_become_commands_ims_and_chat() {
            let mut rig = Rig::connected(true);
            // Name with a space: the answer comes once the name is found.
            rig.owner_sends(DOMAIN, "im \"Jane Doe\" see you later");
            let query = rig.lookup();
            rig.bot.avatars_found(query, &[(jane(), "Jane Doe".into())], Instant::now());
            assert_eq!(rig.calls(), [Call::Im(jane(), "see you later".into())]);
            rig.next();
            assert_eq!(answer(&mut rig), format!("sent to Jane Doe <{}>", jane()));
            // Jane is in her autoreply pause now: her answer gets none.
            ok(&mut rig.bot, "autoreply\tmessage\tAway");
            let message = session::Message {
                kind: "im".to_string(),
                source: session::Source::Avatar {
                    id: jane(),
                    name: "Jane Doe".to_string(),
                },
                sent_at: None,
                text: "ok".to_string(),
            };
            assert!(matches!(
                rig.bot.apply(SessionEvent::InstantMessage(message), Instant::now()),
                Flow::Continue
            ));
            assert!(!rig.calls().iter().any(|call| matches!(call, Call::AutoIm(..))));
            // The sent IM is recorded, and not passed to the owners.
            rig.bot.names_resolved(vec![janes_names()], vec![], Instant::now());
            rig.bot.service_messages();
            assert_eq!(rig.quiet(), "");

            // Reply to an avatar: the address stands for the UUID now.
            rig.owner_sends(&format!("jane.doe@{DOMAIN}"), "see you");
            assert_eq!(rig.calls(), [Call::Im(jane(), "see you".into())]);
            assert_eq!(rig.quiet(), "");

            // Unknown name.
            rig.owner_sends(&format!("no.body@{DOMAIN}"), "hi");
            let query = rig.lookup();
            rig.bot.avatars_found(query, &[], Instant::now());
            rig.next();
            let error = rig.fake.expect("</message>");
            assert!(
                error.contains("type='error'") && error.contains("<item-not-found ") && error.contains("no.body"),
                "{error}"
            );
            assert!(
                error.contains(&format!("from='no.body@{DOMAIN}' to='{OWNER}/phone' id='m1'")),
                "{error}"
            );
            assert_eq!(rig.calls(), []);

            // Reply to an object.
            rig.owner_sends(&format!("obj-{}@{DOMAIN}", collar()), "hi");
            assert!(rig.fake.expect("</message>").contains("<feature-not-implemented "));
            assert_eq!(rig.calls(), []);

            // Typing notification.
            rig.fake.send(&format!(
                "<message type='chat' from='{OWNER}/phone' to='jane.doe@{DOMAIN}'><composing xmlns='http://jabber.org/protocol/chatstates'/></message>"
            ));
            rig.next();
            assert_eq!(rig.calls(), []);
            assert_eq!(rig.quiet(), "");

            // Owner speaks in local chat, with the chat-bar forms.
            rig.owner_sends(&format!("local@{DOMAIN}"), "hello everyone");
            rig.owner_sends(&format!("local@{DOMAIN}"), "/shout come here");
            rig.owner_sends(&format!("local@{DOMAIN}"), "/7 open");
            assert_eq!(
                rig.calls(),
                [
                    Call::Say(0, 1, "hello everyone".into()),
                    Call::Say(0, 2, "come here".into()),
                    Call::Say(7, 1, "open".into())
                ]
            );
            assert_eq!(rig.quiet(), "");

            // Too long.
            rig.owner_sends(&format!("local@{DOMAIN}"), &"x".repeat(2000));
            assert!(rig.fake.expect("</message>").contains("<not-acceptable "));
            assert_eq!(rig.calls(), []);
        }

        #[test]
        fn a_teleport_does_not_hold_back_other_contacts() {
            let mut rig = Rig::connected(true);
            rig.bot.names_resolved(vec![janes_names()], vec![], Instant::now());
            rig.heard(Channel::Im, from_jane("im", "hello"), vec![janes_names()]);
            rig.fake.expect("</message>");
            rig.owner_sends(DOMAIN, "tp 128 128 25 Home");
            assert!(matches!(&rig.calls()[..], [Call::Teleport(region, _)] if region == "Home"));
            rig.owner_sends(&format!("jane.doe@{DOMAIN}"), "hi");
            assert_eq!(rig.calls(), [Call::Im(jane(), "hi".into())]);
            assert_eq!(rig.quiet(), "", "no answer to the teleport yet");
            rig.bot.commands.arrived("Home");
            rig.next();
            assert!(answer(&mut rig).contains("Home"));
        }

        #[test]
        fn a_seventeenth_waiting_command_is_answered_as_busy() {
            let mut rig = Rig::connected(true);
            for n in 0..16 {
                rig.owner_sends(DOMAIN, &format!("im \"Name{n} Doe\" hi"));
                rig.lookup();
            }
            assert_eq!(rig.bot.xmpp.as_ref().unwrap().waiting(), 16);
            rig.owner_sends(DOMAIN, "im \"Name16 Doe\" hi");
            assert!(answer(&mut rig).starts_with("error: the bot is busy"));
            assert_eq!(rig.calls(), []);
        }

        #[test]
        fn instant_messages_and_chat_reach_the_owners() {
            let mut rig = Rig::connected(true);
            // IM from an avatar, with its display name as nickname.
            rig.heard(Channel::Im, from_jane("im", "hello"), vec![janes_names()]);
            assert_eq!(
                rig.fake.expect("</message>"),
                format!(
                    "<message type='chat' from='jane.doe@{DOMAIN}' to='{OWNER}'><body>hello</body>\
                     <nick xmlns='http://jabber.org/protocol/nick'>Janey</nick></message>"
                )
            );
            // IM from an object.
            let object = session::Source::Object {
                id: Some(collar()),
                name: "Collar".into(),
                owner: jane(),
            };
            rig.heard(Channel::Im, message("object", object, "locked"), vec![janes_names()]);
            let im = rig.fake.expect("</message>");
            assert!(
                im.starts_with(&format!("<message type='chat' from='obj-{}@{DOMAIN}'", collar())),
                "{im}"
            );
            assert!(im.contains("<body>locked</body>") && im.contains(">Collar</nick>"), "{im}");
            // Offline IM, with the time it was sent.
            let mut stored = from_jane("im", "while you were out");
            stored.sent_at = Some(1_795_021_463);
            rig.heard(Channel::Im, stored, vec![janes_names()]);
            assert!(rig.fake.expect("</message>").contains("stamp='2026-11-18T17:04:23Z'"));
            // From the grid itself: the control contact.
            let grid = session::Source::System {
                name: "Second Life".into(),
            };
            rig.heard(Channel::Im, message("im", grid, "maintenance"), vec![]);
            assert!(rig
                .fake
                .expect("</message>")
                .contains(&format!("from='{DOMAIN}' to='{OWNER}'><body>Second Life: maintenance</body>")));

            // Chat heard, shout and emote, an object to its owner.
            rig.heard(Channel::Chat, from_jane("say", "hi all"), vec![janes_names()]);
            assert!(rig
                .fake
                .expect("</message>")
                .contains(&format!("from='local@{DOMAIN}' to='{OWNER}'><body>Janey (Jane Doe): hi all</body>")));
            let bob = Uuid::from_bytes([0xb0; 16]);
            let bobs = names::Names {
                legacy: "bob Resident".into(),
                display: String::new(),
            };
            let by_bob = |kind: &str, text: &str| {
                message(
                    kind,
                    session::Source::Avatar {
                        id: bob,
                        name: "bob Resident".into(),
                    },
                    text,
                )
            };
            rig.heard(Channel::Chat, by_bob("shout", "HEY"), vec![(bob, bobs.clone())]);
            rig.heard(Channel::Chat, by_bob("say", "/me waves"), vec![(bob, bobs)]);
            assert!(rig.fake.expect("</message>").contains("<body>bob Resident shouts: HEY</body>"));
            assert!(rig.fake.expect("</message>").contains("<body>bob Resident waves</body>"));
            let owner_say = message(
                "owner",
                session::Source::Object {
                    id: Some(collar()),
                    name: "Collar".into(),
                    owner: jane(),
                },
                "ready",
            );
            rig.heard(Channel::Chat, owner_say, vec![janes_names()]);
            assert!(rig.fake.expect("</message>").contains("<body>Collar [owner]: ready</body>"));

            // An IM sent through the control socket is not passed on.
            ok(&mut rig.bot, &format!("im\t{}\thi", jane()));
            rig.bot.names_resolved(vec![janes_names()], vec![], Instant::now());
            rig.bot.service_messages();
            assert_eq!(rig.quiet(), "");
            rig.calls();

            // Message that looks like a command, with the gateway on.
            rig.heard(Channel::Im, from_jane("im", "say hello"), vec![janes_names()]);
            assert!(rig
                .fake
                .expect("</message>")
                .contains(&format!("from='jane.doe@{DOMAIN}' to='{OWNER}'><body>say hello</body>")));
            assert_eq!(rig.calls(), []);
        }

        #[test]
        fn group_messages_are_not_passed_on() {
            let mut rig = Rig::connected(true);
            let builders = crate::groups::tests::builders();
            let groups = vec![crate::groups::tests::member(builders, "Builders", 0, true)];
            hear(&mut rig.bot, SessionEvent::Groups(groups));
            rig.bot.names_resolved(vec![janes_names()], vec![], Instant::now());
            hear(
                &mut rig.bot,
                SessionEvent::GroupChat {
                    session: builders,
                    from: jane(),
                    name: "Jane Doe".into(),
                    text: "hello all".into(),
                },
            );
            hear(
                &mut rig.bot,
                SessionEvent::GroupNotice {
                    group: builders,
                    sender: None,
                    name: "Jane Doe".into(),
                    text: "Meeting|Tonight".into(),
                    bucket: Vec::new(),
                    sent_at: None,
                },
            );
            rig.bot.service_messages();
            rig.bot.flush_messages();
            assert_eq!(rig.quiet(), "");
            assert_eq!(rig.calls(), []);
        }

        #[test]
        fn bot_stopped() {
            let mut rig = Rig::connected(false);
            rig.bot.flush_messages();
            rig.bot.xmpp_stop();
            let written = rig.fake.expect("</stream:stream>");
            assert_eq!(
                written,
                format!(
                    "<presence type='unavailable' from='{DOMAIN}' to='{OWNER}'/>\
                     <presence type='unavailable' from='local@{DOMAIN}' to='{OWNER}'/></stream:stream>"
                )
            );
        }

        #[test]
        fn quit_is_answered_before_the_addresses_go_offline() {
            let mut rig = Rig::connected(false);
            assert!(matches!(rig.owner_sends(DOMAIN, "quit"), Flow::Exit(EXIT_OK)));
            // As the main loop would before it ends.
            rig.bot.xmpp_presence();
            rig.bot.flush_messages();
            rig.bot.xmpp_stop();
            let written = rig.fake.expect("</stream:stream>");
            let ok = written.find("<body>ok</body>").expect("ok");
            let offline = written.find("type='unavailable'").expect("offline");
            assert!(ok < offline, "{written}");
            // Not again on the way out (`local@` was offline already).
            assert_eq!(written.matches("type='unavailable'").count(), 1, "{written}");
        }

        #[test]
        fn no_xmpp_table_no_gateway() {
            assert!(bot(full("")).xmpp.is_none());
        }
    }

    // -------------------------------------------------------------- web

    mod payments {
        use super::*;
        use commands::tests::{Call, FakeGrid};
        use udp::messages::{TransactionInfo, TRANSACTION_GIFT};

        const JANE: Uuid = Uuid::from_bytes([0x71; 16]);

        fn janes_names() -> (Uuid, names::Names) {
            let names = names::Names {
                legacy: "Jane Doe".into(),
                display: "JD".into(),
            };
            (JANE, names)
        }

        /// A temporary directory of this test's own.
        fn temp_dir(name: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir().join(format!("rslbot-pay-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        /// A bot with `table` as its `[payments]` table, unlimited, on a
        /// simulator that takes everything.
        fn paying(table: &str) -> Bot {
            let mut bot = bot(full(&format!("[payments]\nlimit = \"unlimited\"\n{table}\n")));
            bot.start_payments(&txlog::ReadBack::default(), wall_clock());
            bot.grid = Some(FakeGrid::default());
            bot
        }

        fn calls(bot: &mut Bot) -> Vec<Call> {
            std::mem::take(&mut bot.grid.as_mut().unwrap().calls)
        }

        /// Hands the bot a command through `via`; the receiver of its answer.
        fn through(bot: &mut Bot, via: Interface, own: bool, line: &str) -> Receiver<Response> {
            let (reply, answer) = mpsc::channel();
            let request = Request::decode(line.as_bytes()).unwrap();
            assert!(matches!(bot.control_as(request, reply, own, via, Instant::now()), Flow::Continue));
            answer
        }

        fn pay_jane(bot: &mut Bot, via: Interface) -> Receiver<Response> {
            let answer = through(bot, via, false, &format!("pay\t10\tavatar\t{JANE}"));
            let (id, names) = janes_names();
            bot.names_resolved(vec![(id, names)], Vec::new(), Instant::now());
            answer
        }

        fn paid(bot: &mut Bot) -> bool {
            calls(bot).iter().any(|call| matches!(call, Call::Pay(..)))
        }

        fn error(answer: &Receiver<Response>) -> String {
            match answer.try_recv() {
                Ok(Response::Error(reason)) => reason,
                other => panic!("expected an error, got {other:?}"),
            }
        }

        fn report(bot: &mut Bot, balance: i32, info: Option<TransactionInfo>) {
            hear(
                bot,
                SessionEvent::Money {
                    balance,
                    success: true,
                    description: String::new(),
                    info,
                },
            );
        }

        // ------------------------------------------------------ the gate

        #[test]
        fn without_the_table_nothing_is_paid() {
            let mut bot = bot(full(""));
            bot.grid = Some(FakeGrid::default());
            for line in ["pay\t10\tavatar\tJane Doe", "prices\tx", "payees"] {
                for via in Interface::ALL {
                    let reason = error(&through(&mut bot, via, false, line));
                    assert!(reason.contains("[payments]"), "{line} via {via}: {reason}");
                }
            }
            assert_eq!(calls(&mut bot), []);
            assert!(!bot.status(Interface::Ctl).contains("balance"));
        }

        #[test]
        fn only_the_interfaces_perms_lists_pay() {
            let mut bot = paying("perms = [\"ctl\"]");
            let reason = error(&through(&mut bot, Interface::Web, false, "pay\t10\tavatar\tJane Doe"));
            assert!(reason.contains("`web`"), "{reason}");
            let reason = error(&through(&mut bot, Interface::Xmpp, false, "payees"));
            assert!(reason.contains("`xmpp`"), "{reason}");
            assert_eq!(calls(&mut bot), []);
            pay_jane(&mut bot, Interface::Ctl);
            assert_eq!(calls(&mut bot), [Call::Pay(JANE, 10, TRANSACTION_GIFT, String::new())]);
        }

        #[test]
        fn paying_over_xmpp_only() {
            let mut bot = paying("perms = [\"xmpp\"]");
            pay_jane(&mut bot, Interface::Xmpp);
            assert!(paid(&mut bot));
            report(&mut bot, 990, Some(gift(10)));
            assert!(error(&through(&mut bot, Interface::Ctl, false, "pay\t10\tavatar\tJane Doe")).contains("`ctl`"));
        }

        #[test]
        fn nobody_listed_is_as_without_the_table() {
            let mut bot = paying("perms = []");
            for via in Interface::ALL {
                assert!(error(&through(&mut bot, via, false, "pay\t10\tavatar\tJane Doe")).contains("not enabled"));
            }
            assert!(!bot.status(Interface::Ctl).contains("limit"));
        }

        #[test]
        fn the_startup_script_pays_as_ctl() {
            let script = format!("pay 10 avatar {JANE}\n");
            let mut ctl = scripted("[payments]\nlimit = \"unlimited\"\nperms = [\"ctl\"]\n", &script);
            ctl.grid = Some(FakeGrid::default());
            ctl.startup_reached(Persist::Off, true, Instant::now());
            assert_eq!(ctl.startup_rest(), Ok(true));
            let (id, names) = janes_names();
            ctl.names_resolved(vec![(id, names)], Vec::new(), Instant::now());
            assert!(paid(&mut ctl));
            // With only `web` in `perms` the line is refused, and the
            // script stops there.
            let mut web = scripted("[payments]\nlimit = \"unlimited\"\nperms = [\"web\"]\n", &script);
            web.grid = Some(FakeGrid::default());
            web.startup_reached(Persist::Off, true, Instant::now());
            assert_eq!(web.startup_rest(), Ok(true));
            assert!(!paid(&mut web));
            assert!(script_is_over(&web));
        }

        #[test]
        fn an_override_is_not_used_up_by_payments() {
            let mut bot = paying("perms = [\"ctl\"]");
            ok(&mut bot, "rlv\tmode\ton");
            ok(&mut bot, "rlv\tmode\toverride");
            through(&mut bot, Interface::Ctl, false, "prices\t11111111-2222-3333-4444-555555555555");
            through(&mut bot, Interface::Ctl, false, "payees");
            assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: override"));
        }

        #[test]
        fn the_interface_travels_with_the_command() {
            // A probe: `status` shows the money lines only to an
            // interface that may pay, and each way in is its interface.
            let mut bot = paying("perms = [\"web\"]");
            let status = |bot: &mut Bot, via| match through(bot, via, false, "status").try_recv() {
                Ok(Response::Ok(Some(text))) => text,
                other => panic!("{other:?}"),
            };
            assert!(!status(&mut bot, Interface::Ctl).contains("limit"));
            assert!(!status(&mut bot, Interface::Xmpp).contains("limit"));
            assert!(status(&mut bot, Interface::Web).ends_with("balance: unknown\nlimit: unlimited"));
            // The control socket and the script use `control`, which is
            // `ctl`.
            assert!(!ok(&mut bot, "status").contains("limit"));
        }

        // --------------------------------------------------------- status

        fn gift(amount: i32) -> TransactionInfo {
            TransactionInfo {
                kind: TRANSACTION_GIFT,
                source: Uuid::nil(),
                source_group: false,
                dest: JANE,
                dest_group: false,
                amount,
                item: String::new(),
            }
        }

        #[test]
        fn status_lines() {
            let mut bot = bot(full(&format!(
                "[logs]\ntransactions = \"{}\"\n[payments]\nlimit = 1000\nperms = [\"ctl\"]\n",
                temp_dir("status").join("tx.tsv").display()
            )));
            bot.start_payments(&txlog::ReadBack::default(), wall_clock());
            let status = ok(&mut bot, "status");
            assert_eq!(status.lines().nth(1), Some("avatar: Sit Bot"));
            assert!(status.ends_with("mode: holding\nbalance: unknown\nlimit: L$1000"), "{status}");
            report(&mut bot, 1200, None);
            assert!(ok(&mut bot, "status").ends_with("balance: L$1200\nlimit: L$1000"));
            // A payment received changes it without asking.
            let received = TransactionInfo {
                source: JANE,
                dest: Uuid::nil(),
                ..gift(50)
            };
            report(&mut bot, 1250, Some(received));
            assert!(ok(&mut bot, "status").contains("balance: L$1250"));
        }

        #[test]
        fn status_says_when_rlv_holds_back_descriptions() {
            let mut bot = paying("perms = [\"ctl\"]");
            assert!(!ok(&mut bot, "status").contains("pay description"));
            ok(&mut bot, "rlv\tmode\ton");
            ok(&mut bot, "rlv\tcmd\t@sendim=n");
            assert!(ok(&mut bot, "status").ends_with("limit: unlimited\npay description: blocked by RLV"));
            // Not for an interface that may not pay, and not while RLV is off.
            assert!(!bot.status(Interface::Web).contains("pay description"));
            ok(&mut bot, "rlv\tmode\toff");
            assert!(!ok(&mut bot, "status").contains("pay description"));
        }

        #[test]
        fn a_single_word_user_is_named_as_resident() {
            let mut bot = bot(Config::parse("user = \"sitbot\"\npassword = \"x\"\n").unwrap());
            assert_eq!(ok(&mut bot, "status").lines().nth(1), Some("avatar: sitbot Resident"));
        }

        #[test]
        fn the_readmes_status_example_is_what_the_bot_prints() {
            let held = Placement {
                location: Some(at("Target", 130.0, 128.0, 25.9)),
                sit: SitState::Object(object()),
            };
            let here = Whereabouts {
                region: "Target",
                position: Some([130.0, 128.0, 25.9]),
                seat: sat(object(), None),
                seat_name: Some("Park Bench"),
            };
            let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(7 * 86_400)));
            let paid = 1_796_126_400; // 2026-12-01T12:00:00Z
            ledger.reserve(250, paid).unwrap();
            let money = (Some(1200), crate::payments::limit_text(&ledger, paid + 60), false);
            let text = status_answer("Sit Bot", &status_text(Some(here), &held, false), Some(money));
            assert!(include_str!("../README.md").contains(&format!("```\n{text}\n```")), "{text}");
        }

        #[test]
        fn status_answer_puts_the_avatar_second_and_money_last() {
            assert_eq!(
                status_answer("Sit Bot", "connected: no\npersist: unknown stand\nmode: holding", None),
                "connected: no\navatar: Sit Bot\npersist: unknown stand\nmode: holding"
            );
            assert_eq!(
                status_answer(
                    "Sit Bot",
                    "connected: no\nmode: manual",
                    Some((Some(5), "L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)".into(), false))
                ),
                "connected: no\navatar: Sit Bot\nmode: manual\nbalance: L$5\nlimit: L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)"
            );
        }

        #[test]
        fn the_balance_is_asked_for_once_after_each_login_with_the_table_only() {
            let mut with = paying("perms = []");
            assert!(!with.balance_asked);
            // Without a session there is nothing to ask.
            with.ask_balance(Instant::now());
            assert!(!with.balance_asked);
            let mut without = bot(full(""));
            without.ask_balance(Instant::now());
            assert!(!without.balance_asked);
        }

        #[test]
        fn balance_changes_reach_the_pages_status() {
            let mut bot = paying("perms = [\"web\"]");
            let before = bot.status(Interface::Web);
            report(&mut bot, 1250, None);
            let after = bot.status(Interface::Web);
            assert_ne!(before, after);
            assert!(after.ends_with("balance: L$1250\nlimit: unlimited"));
        }

        // ------------------------------------------------ the limit at startup

        fn row(time: u64, kind: txlog::Kind, amount: u64, counted: u64) -> txlog::Transaction {
            txlog::Transaction {
                time,
                kind,
                amount,
                balance: None,
                counted,
                code: TRANSACTION_GIFT,
                party: Some(JANE),
                party_group: false,
                avatar_name: "Jane Doe".into(),
                display_name: String::new(),
                object_name: String::new(),
                object: None,
                text: String::new(),
            }
        }

        fn limited(extra: &str, dir: &Path) -> Config {
            let path = dir.join("tx.tsv");
            full(&format!(
                "[logs]\ntransactions = \"{}\"\n[payments]\nlimit = 1000\nperms = [\"ctl\", \"web\"]\n{extra}",
                path.display()
            ))
        }

        #[test]
        fn the_limit_is_restored_from_the_file() {
            let dir = temp_dir("restore");
            let now = wall_clock();
            let rows = [
                row(now - 3600, txlog::Kind::Paid, 900, 900),
                row(now - 60, txlog::Kind::Received, 50, 0),
                row(now - 50, txlog::Kind::Paid, 40, 0),
                row(now - 40, txlog::Kind::Failed, 70, 0),
                row(now - 30, txlog::Kind::Spent, 80, 0),
            ];
            let text: String = std::iter::once(txlog::header()).chain(rows.iter().map(|r| r.row())).collect();
            let config = limited("", &dir);
            std::fs::write(&config.logs.transactions.as_ref().unwrap().path, text).unwrap();
            let (_, back) = open_logs(&config).unwrap();
            assert_eq!(back.rows.len(), 5);
            let mut bot = bot(config);
            bot.start_payments(&back, now);
            assert!(ok(&mut bot, "status").ends_with("limit: L$100"));
            bot.grid = Some(FakeGrid::default());
            report(&mut bot, 10_000, None);
            let answer = through(&mut bot, Interface::Ctl, false, &format!("pay\t101\tavatar\t{JANE}"));
            let (id, names) = janes_names();
            bot.names_resolved(vec![(id, names)], Vec::new(), Instant::now());
            assert!(error(&answer).contains("L$100"));
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn a_restart_keeps_what_was_paid() {
            let dir = temp_dir("restart");
            let config = limited("", &dir);
            let (logs, back) = open_logs(&config).unwrap();
            let (events, _incoming) = mpsc::channel();
            let mut first = Bot::new(config.clone(), logs, events);
            first.start_payments(&back, wall_clock());
            first.grid = Some(FakeGrid::default());
            let answer = through(&mut first, Interface::Ctl, false, &format!("pay\t900\tavatar\t{JANE}"));
            let (id, names) = janes_names();
            first.names_resolved(vec![(id, names)], Vec::new(), Instant::now());
            let own = first.inventory.agent;
            report(&mut first, 100, Some(TransactionInfo { source: own, ..gift(900) }));
            assert!(matches!(answer.try_recv(), Ok(Response::Ok(_))));
            // The record waits a moment for its names, and is written on the
            // way out at the latest.
            first.flush_messages();
            drop(first);
            let (logs, back) = open_logs(&config).unwrap();
            assert_eq!(back.rows.len(), 1);
            let (events, _incoming) = mpsc::channel();
            let mut second = Bot::new(config, logs, events);
            second.start_payments(&back, wall_clock());
            assert!(ok(&mut second, "status").ends_with("limit: L$100"));
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn start_line_and_the_warning_for_an_open_web_interface() {
            let dir = temp_dir("start");
            let now = 1_796_126_400; // 2026-12-01T12:00:00Z
            let config = limited("limit_renew = \"7d\"\n[web]\nlisten = \"127.0.0.1:8080\"\n", &dir);
            let back = txlog::ReadBack {
                rows: vec![row(now - 86_400, txlog::Kind::Paid, 250, 250)],
                skipped: 0,
            };
            let (ledger, lines) = payments_start(&config, &back, now);
            assert_eq!(ledger.left(now), Some(750));
            assert_eq!(
                lines[0],
                "payments: `ctl` and `web` may pay, with a limit of L$1000 every 7d, \
                 L$750 (renews to L$1000 on 2026-12-07T12:00:00Z) left"
            );
            assert!(lines[1].contains("web.password"), "{lines:?}");
            let locked = limited("[web]\nlisten = \"127.0.0.1:8080\"\npassword = \"p\"\n", &dir);
            assert_eq!(payments_start(&locked, &back, now).1.len(), 1);
            assert_eq!(payments_start(&full(""), &back, now).1, Vec::<String>::new());
            std::fs::remove_dir_all(dir).unwrap();
        }

        // ---------------------------------------------- the record of the limit

        fn read_only(path: &Path, on: bool) {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(on);
            std::fs::set_permissions(path, permissions).unwrap();
        }

        #[test]
        fn disk_full_and_room_again() {
            let dir = temp_dir("full");
            let config = limited("", &dir);
            let path = config.logs.transactions.as_ref().unwrap().path.clone();
            let (logs, back) = open_logs(&config).unwrap();
            let (events, _incoming) = mpsc::channel();
            let mut bot = Bot::new(config, logs, events);
            bot.start_payments(&back, wall_clock());
            bot.grid = Some(FakeGrid::default());
            report(&mut bot, 10_000, None);
            read_only(&path, true);
            if std::fs::OpenOptions::new().append(true).open(&path).is_ok() {
                // Root writes whatever the permissions say.
                read_only(&path, false);
                return;
            }
            let answer = pay_jane(&mut bot, Interface::Ctl);
            assert!(paid(&mut bot));
            let own = bot.inventory.agent;
            report(&mut bot, 9990, Some(TransactionInfo { source: own, ..gift(10) }));
            assert!(matches!(answer.try_recv(), Ok(Response::Ok(_))));
            bot.flush_messages();
            // The row could not be written: the next payment that counts is
            // refused, naming the file.
            let reason = error(&pay_jane(&mut bot, Interface::Ctl));
            assert!(reason.contains(&path.display().to_string()), "{reason}");
            assert!(!paid(&mut bot));
            // Room again: the waiting row goes first, then the payment.
            read_only(&path, false);
            pay_jane(&mut bot, Interface::Ctl);
            assert!(paid(&mut bot));
            assert_eq!(txlog::read(&path).unwrap().rows.len(), 1);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn a_payment_that_waits_is_recorded_when_the_bot_stops() {
            let dir = temp_dir("stop");
            let config = limited("", &dir);
            let path = config.logs.transactions.as_ref().unwrap().path.clone();
            let (logs, back) = open_logs(&config).unwrap();
            let (events, _incoming) = mpsc::channel();
            let mut bot = Bot::new(config, logs, events);
            bot.start_payments(&back, wall_clock());
            bot.grid = Some(FakeGrid::default());
            pay_jane(&mut bot, Interface::Ctl);
            assert!(paid(&mut bot));
            bot.flush_messages();
            let rows = txlog::read(&path).unwrap().rows;
            assert_eq!((rows.len(), rows[0].kind, rows[0].counted), (1, txlog::Kind::Unconfirmed, 10));
            std::fs::remove_dir_all(dir).unwrap();
        }

        // ------------------------------------------------- config errors at start

        fn exit_of(name: &str, config: &str) -> i32 {
            let dir = temp_dir(name);
            let path = dir.join("russelbot.toml");
            std::fs::write(&path, config).unwrap();
            let code = run_bot(&path);
            std::fs::remove_dir_all(dir).unwrap();
            code
        }

        #[test]
        fn a_log_that_cannot_be_opened_stops_the_bot_before_the_network() {
            let missing = std::env::temp_dir().join(format!("rslbot-no-such-dir-{}", std::process::id()));
            let new = format!("{ACCOUNT}[logs]\nim = \"{}/im.tsv\"\n", missing.display());
            let old = format!("{ACCOUNT}log_im = \"{}/im.tsv\"\n", missing.display());
            assert_eq!(exit_of("new-key", &new), EXIT_CONFIG);
            assert_eq!(exit_of("old-key", &old), EXIT_CONFIG);
            let error = |text: &str| open_logs(&Config::parse(text).unwrap()).err().unwrap();
            assert!(error(&new).starts_with("`logs.im`: cannot open"), "{}", error(&new));
            assert!(error(&old).starts_with("`log_im`: cannot open"), "{}", error(&old));
        }

        #[test]
        fn a_transactions_file_that_is_something_else_stops_the_bot() {
            let dir = temp_dir("not-tx");
            let path = dir.join("chat.tsv");
            std::fs::write(&path, msglog::header()).unwrap();
            let text = format!("{ACCOUNT}[logs]\ntransactions = \"{}\"\n", path.display());
            let error = open_logs(&Config::parse(&text).unwrap()).err().unwrap();
            assert!(error.starts_with("`logs.transactions`:") && error.contains("chat.tsv"), "{error}");
            assert_eq!(exit_of("not-tx-run", &text), EXIT_CONFIG);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    mod web_interface {
        use super::*;
        use commands::tests::{Call, FakeGrid};
        use std::net::SocketAddr;
        use std::thread;
        use web::tests::{body, listen_quick, post_to, Stream};

        fn jane() -> Uuid {
            Uuid::from_bytes([0x1a; 16])
        }

        fn janes_names() -> (Uuid, names::Names) {
            let names = names::Names {
                legacy: "Jane Doe".into(),
                display: "Janey".into(),
            };
            (jane(), names)
        }

        fn from_jane(kind: &str, text: &str) -> session::Message {
            session::Message {
                kind: kind.into(),
                source: session::Source::Avatar {
                    id: jane(),
                    name: "Jane Doe".into(),
                },
                sent_at: None,
                text: text.into(),
            }
        }

        /// A bot with a web interface on a port of its own, and on a
        /// simulator that takes everything when `grid`.
        struct Rig {
            bot: Bot,
            incoming: Receiver<Event>,
            server: web::Server,
            /// The bot asked to be stopped.
            exited: bool,
        }

        impl Rig {
            fn new(grid: bool) -> Rig {
                Rig::with("", grid)
            }

            /// The same with `more` after the `[web]` table.
            fn with(more: &str, grid: bool) -> Rig {
                let mut config = full(&format!("[web]\nlisten = \"127.0.0.1:8080\"\n{more}"));
                // Any free port, which a config cannot ask for.
                config.web.as_mut().unwrap().listen.set_port(0);
                let (events, incoming) = mpsc::channel();
                let server = listen_quick(config.web.as_ref().unwrap(), events.clone());
                let mut bot = Bot::new(config, Logs::none(), events);
                if grid {
                    bot.grid = Some(FakeGrid::default());
                }
                Rig {
                    bot,
                    incoming,
                    server,
                    exited: false,
                }
            }

            fn address(&self) -> SocketAddr {
                self.server.address()
            }

            /// Handles the next event, as the main loop does. Whether
            /// there was one.
            fn next(&mut self, wait: Duration) -> bool {
                let Ok(event) = self.incoming.recv_timeout(wait) else {
                    return false;
                };
                if let Flow::Exit(_) = self.bot.handle(event) {
                    self.exited = true;
                }
                self.bot.service_messages();
                self.bot.web_status();
                true
            }

            /// Sends what the page sends, without waiting for the answer,
            /// which comes on what is returned.
            fn begin(&mut self, path: &'static str, line: &str, own: bool) -> Receiver<String> {
                let (address, line) = (self.address(), line.to_string());
                let (done, answer) = mpsc::channel();
                thread::spawn(move || {
                    let headers: &[&str] = if own { &["X-Rslbot-Own: 1"] } else { &[] };
                    let _ = done.send(body(&post_to(address, path, headers, &line)).to_string());
                });
                // The command has reached the bot once it has been handled.
                assert!(self.next(Duration::from_secs(5)), "the command did not arrive");
                answer
            }

            /// Sends what the page sends and gives the answer line, with
            /// the waits a listing has run out.
            fn send(&mut self, path: &'static str, line: &str, own: bool) -> String {
                let answer = self.begin(path, line, own);
                let later = Instant::now() + commands::SCAN_LIMIT + Duration::from_secs(1);
                self.bot.scan_objects(later);
                answer.recv_timeout(Duration::from_secs(5)).expect("an answer")
            }

            fn page(&mut self, line: &str) -> String {
                self.send("/api/command", line, false)
            }

            /// Sends what the page sends where the answer is the web
            /// interface's own and nothing reaches the bot.
            fn unread(&self, path: &str, line: &str) -> String {
                let answer = body(&post_to(self.address(), path, &[], line)).to_string();
                assert!(self.incoming.try_recv().is_err(), "{line} reached the bot");
                answer
            }

            fn console(&mut self, line: &str) -> String {
                self.send("/api/console", line, false)
            }

            /// A page opens its event stream; what it is sent first has
            /// been read.
            fn open(&mut self, path: &str) -> (Stream, Vec<String>) {
                let mut stream = self.server.stream(path, &[]);
                assert!(self.next(Duration::from_secs(5)), "the page did not arrive");
                let kept = stream.opening();
                (stream, kept)
            }

            /// Records a message whose names are known, as the session
            /// hands it over.
            fn heard(&mut self, channel: Channel, message: session::Message) {
                let now = Instant::now();
                self.bot.names_resolved(vec![janes_names()], vec![], now);
                self.bot.record(channel, message, now);
                self.bot.service_messages();
            }

            fn calls(&mut self) -> Vec<Call> {
                std::mem::take(&mut self.bot.grid.as_mut().unwrap().calls)
            }
        }

        /// The answer line the control socket writes for `line`.
        fn by_socket(bot: &mut Bot, line: &str) -> String {
            command(bot, line).encode()
        }

        #[test]
        fn the_pages_commands_are_answered_as_the_control_sockets() {
            // Between logins.
            let mut rig = Rig::new(false);
            let status = rig.page("status");
            assert!(status.starts_with("ok\tconnected: no\\navatar: Sit Bot\\npersist: "), "{status}");
            assert_eq!(status, by_socket(&mut rig.bot, "status"));
            let refused = rig.page("sit\tground");
            assert!(refused.starts_with("error\t"), "{refused}");
            assert_eq!(refused, by_socket(&mut rig.bot, "sit\tground"));
            assert_eq!(rig.console("sit ground"), refused);
            assert_eq!(rig.console("status"), status);
            // Unknown, and with the wrong arguments: an error, and nothing more.
            assert!(rig.unread("/api/command", "dance").contains("dance"));
            assert!(rig.unread("/api/command", "im\tjane").starts_with("error\t"));
            assert!(rig.unread("/api/console", "dance").contains("dance"));
        }

        #[test]
        fn an_argument_is_taken_as_it_is() {
            let mut rig = Rig::new(true);
            let region = "Joe's \"Blue\" Lagoon";
            let answer = rig.begin("/api/command", &format!("tp\t128\t128\t25\t{region}"), false);
            assert_eq!(rig.calls(), [Call::Teleport(region.to_string(), [128.0, 128.0, 25.0])]);
            assert!(answer.try_recv().is_err(), "answered before the avatar arrived");
            rig.bot.commands.arrived(region);
            assert_eq!(
                answer.recv_timeout(Duration::from_secs(5)).unwrap(),
                format!("ok\tarrived in {region}\n")
            );
        }

        #[test]
        fn a_teleport_does_not_hold_back_other_commands_or_pages() {
            let mut rig = Rig::new(true);
            let (mut page, _) = rig.open("/api/events");
            let teleport = rig.begin("/api/command", "tp\t128\t128\t25\tHome", false);
            assert!(matches!(&rig.calls()[..], [Call::Teleport(region, _)] if region == "Home"));
            // Chat is said at once, and heard chat reaches the page.
            assert_eq!(rig.page("say\thi"), "ok\n");
            assert_eq!(rig.calls(), [Call::Say(0, 1, "hi".into())]);
            rig.heard(Channel::Chat, from_jane("say", "going somewhere?"));
            // The command put the bot in manual mode, which the page was told.
            assert!(page.event().ends_with("mode: manual\n"));
            assert_eq!(page.chat(), "going somewhere?");
            assert!(teleport.try_recv().is_err(), "no answer to the teleport yet");
            rig.bot.commands.arrived("Home");
            assert_eq!(teleport.recv_timeout(Duration::from_secs(5)).unwrap(), "ok\tarrived in Home\n");
        }

        #[test]
        fn quit_is_answered_and_stops_the_bot() {
            let mut rig = Rig::new(false);
            let (mut page, _) = rig.open("/api/events");
            assert_eq!(rig.page("quit"), "ok\n");
            assert!(rig.exited && rig.bot.stopping);
            // As the main loop does on its way out: the page's stream ends,
            // and nothing is left for the server to wait for.
            rig.bot.flush_messages();
            rig.bot.web_stop();
            assert_eq!(page.event(), "");
            let started = Instant::now();
            rig.server.stop();
            assert!(started.elapsed() < Duration::from_secs(1));
        }

        #[test]
        fn console_lines_are_split_as_the_control_contact_splits_them() {
            let mut rig = Rig::new(true);
            // Name with a space: the answer comes once the name is found.
            let answer = rig.begin("/api/console", "im \"Jane Doe\" see you later", false);
            let query = match &rig.calls()[..] {
                [Call::Find(query, _)] => *query,
                other => panic!("expected a lookup, got {other:?}"),
            };
            rig.bot.avatars_found(query, &[(jane(), "Jane Doe".into())], Instant::now());
            assert_eq!(rig.calls(), [Call::Im(jane(), "see you later".into())]);
            assert_eq!(
                answer.recv_timeout(Duration::from_secs(5)).unwrap(),
                format!("ok\tsent to Jane Doe <{}>\n", jane())
            );
            // Unclosed quote: an error, and nothing is sent to the grid.
            let unclosed = rig.unread("/api/console", "im \"Jane Doe hi");
            assert!(unclosed.starts_with("error\t") && unclosed.contains("double quotes"), "{unclosed}");
            assert_eq!(rig.calls(), []);
            // Help, with no `[xmpp]` table in the config.
            assert!(rig.bot.xmpp.is_none());
            let help = rig.unread("/api/console", "help");
            assert!(
                help.starts_with("ok\tCommands") && help.contains("\\nscan avatars [DISTANCE]\\n"),
                "{help}"
            );
        }

        /// Carries out `line` as the page's own or as the operator's and
        /// answers it after the waits it has.
        fn listed(bot: &mut Bot, line: &str, own: bool) -> Result<Option<String>, String> {
            let (reply, answer) = mpsc::channel();
            let request = Request::decode(line.as_bytes()).unwrap();
            let event = Event::Web(web::WebEvent::Command { request, reply, own });
            assert!(matches!(bot.handle(event), Flow::Continue));
            let at_once = answer.try_recv();
            let response = at_once.or_else(|_| {
                let later = Instant::now() + commands::SCAN_LIMIT + Duration::from_secs(1);
                bot.scan_objects(later);
                answer.try_recv()
            });
            match response {
                Ok(Response::Ok(text)) => Ok(text),
                Ok(Response::Error(reason)) => Err(reason),
                Err(e) => panic!("{line}: no answer: {e:?}"),
            }
        }

        /// What the operator log got of commands since this was last asked.
        fn logged(bot: &mut Bot) -> Vec<String> {
            let mut lines = bot.commands.take_told();
            lines.extend(commands::take_refusals());
            lines
        }

        #[test]
        fn the_pages_own_refresh_leaves_an_override_in_place() {
            let mut bot = among_others(&["shownearby"]);
            ok(&mut bot, "rlv\tmode\toverride");
            logged(&mut bot);
            // The page's own: held to the restriction, and not logged.
            let reason = listed(&mut bot, "scan\tavatars\tchat", true).unwrap_err();
            assert!(reason.contains("shownearby"), "{reason}");
            assert_eq!(logged(&mut bot), [] as [&str; 0]);
            assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: override\n"));
            // The next command the operator gives is the exempt one: the
            // page's refresh button, here.
            let listing = listed(&mut bot, "scan\tavatars\tchat", false).unwrap().unwrap();
            assert!(listing.starts_with(&Uuid::from_bytes([1; 16]).to_string()), "{listing}");
            assert!(ok(&mut bot, "rlv\tstatus").starts_with("rlv: on\n"));
            assert_eq!(
                logged(&mut bot),
                ["control: scan avatars chat: 1 listed; 1 without a name after 10 s"]
            );
            // And only that one.
            let reason = listed(&mut bot, "scan\tavatars\tchat", false).unwrap_err();
            assert!(reason.contains("shownearby"), "{reason}");
            assert_eq!(logged(&mut bot).len(), 1);
        }

        #[test]
        fn what_the_page_sends_by_itself_is_not_logged() {
            let mut bot = among_others(&[]);
            logged(&mut bot);
            let by_page = (
                listed(&mut bot, "scan\tavatars\tchat", true).unwrap(),
                listed(&mut bot, "friends", true).unwrap(),
            );
            assert_eq!(logged(&mut bot), [] as [&str; 0]);
            // The same commands of the operator's are answered the same,
            // and logged.
            let by_operator = (
                listed(&mut bot, "scan\tavatars\tchat", false).unwrap(),
                listed(&mut bot, "friends", false).unwrap(),
            );
            assert_eq!(by_page, by_operator);
            assert!(by_page.0.is_some() && by_page.1.is_some());
            let lines = logged(&mut bot);
            assert_eq!(lines.len(), 2, "{lines:?}");
            assert!(lines[0].starts_with("control: scan avatars chat: 1 listed"), "{lines:?}");
            assert!(lines[1].starts_with("control: friends: 1 listed, 0 online"), "{lines:?}");
            // Refused, between logins: answered, and still not logged.
            let mut offline = bot_between_logins();
            for line in ["scan\tavatars\tchat", "friends"] {
                let reason = listed(&mut offline, line, true).unwrap_err();
                assert!(reason.contains("not connected"), "{reason}");
            }
            assert_eq!(logged(&mut offline), [] as [&str; 0]);
            let reason = listed(&mut offline, "friends", false).unwrap_err();
            assert!(reason.contains("not connected"), "{reason}");
            assert_eq!(logged(&mut offline).len(), 1);
            // It moves nothing and changes no mode.
            assert!(!bot.reconciler.is_manual());
        }

        fn bot_between_logins() -> Bot {
            let mut bot = bot(full(""));
            bot.grid = Some(FakeGrid {
                offline: true,
                ..FakeGrid::default()
            });
            bot
        }

        #[test]
        fn the_own_mark_comes_through_from_the_page() {
            // Over HTTP, from the header to the origin of the command.
            let mut rig = Rig::new(false);
            rig.bot.grid = among_others(&["shownearby"]).grid.take();
            rig.bot.rlv = Rlv::new(true, RlvPersist::Drop, PersistOverride::default());
            assert_eq!(rig.page("rlv\tmode\toverride"), "ok\trlv: override\n");
            logged(&mut rig.bot);
            let refused = rig.send("/api/command", "scan\tavatars\tchat", true);
            assert!(refused.starts_with("error\t") && refused.contains("shownearby"), "{refused}");
            assert_eq!(logged(&mut rig.bot), [] as [&str; 0]);
            assert!(rig.page("rlv\tstatus").starts_with("ok\trlv: override\\n"));
        }

        #[test]
        fn pages_get_what_is_recorded_and_the_status_when_it_changes() {
            let mut rig = Rig::new(true);
            // Recorded before any page was open: kept.
            rig.heard(Channel::Im, from_jane("im", "are you there?"));
            let (mut page, kept) = rig.open("/api/events");
            assert_eq!(kept, ["are you there?"]);
            let (mut other, kept) = rig.open("/api/events");
            assert_eq!(kept, ["are you there?"]);
            // Chat heard: with the names and the avatar's UUID.
            rig.heard(Channel::Chat, from_jane("say", "hi all"));
            let event = page.event();
            let fields: Vec<&str> = event.lines().nth(2).unwrap().strip_prefix("data: ").unwrap().split('\t').collect();
            assert_eq!(
                fields[1..],
                ["say", &jane().to_string(), "Jane Doe", "Janey", "", "", "0", "hi all"]
            );
            assert!(event.starts_with("event: chat\nid: "), "{event}");
            assert_eq!(other.chat(), "hi all");
            // An IM sent from elsewhere, here the control socket.
            assert_eq!(ok(&mut rig.bot, &format!("im\t{}\tsee you", jane())), format!("sent to {}", jane()));
            rig.bot.service_messages();
            for stream in [&mut page, &mut other] {
                let event = stream.event();
                assert!(event.starts_with("event: im\n"), "{event}");
                assert!(
                    event.contains(&format!("\tim_out\t{}\tJane Doe\tJaney\t\t\t0\tsee you\n", jane())),
                    "{event}"
                );
            }
            // The status, when it changes and not otherwise.
            rig.bot.web_status();
            rig.bot.web_status();
            assert!(by_socket(&mut rig.bot, "status").ends_with("mode: holding\n"));
            assert!(matches!(command(&mut rig.bot, "stand"), Response::Ok(None)));
            rig.bot.web_status();
            rig.heard(Channel::Chat, from_jane("say", "after"));
            let event = page.event();
            assert!(
                event.starts_with("event: status\ndata: connected: no\\n") && event.ends_with("mode: manual\n"),
                "{event}"
            );
            assert_eq!(page.chat(), "after");
            // A page that opens now gets all four, in the order of recording.
            let (_, kept) = rig.open("/api/events");
            assert_eq!(kept, ["are you there?", "hi all", "see you", "after"]);
            // One that lost its stream goes on after the event it got last.
            let newest = event_id(&mut rig);
            let (start, number) = newest.split_once('-').unwrap();
            assert_eq!(number, "4");
            let mut resumed = rig.server.stream(&format!("/api/events?last={start}-2"), &[]);
            assert!(rig.next(Duration::from_secs(5)));
            assert_eq!(
                (resumed.chat_or_im(), resumed.chat_or_im()),
                ("see you".to_string(), "after".to_string())
            );
            assert!(resumed.event().starts_with("event: status\n"));
        }

        /// The id of the newest event, from a page opened for it.
        fn event_id(rig: &mut Rig) -> String {
            let mut stream = rig.server.stream("/api/events", &[]);
            assert!(rig.next(Duration::from_secs(5)));
            let mut newest = String::new();
            loop {
                let event = stream.event();
                if let Some(id) = event.lines().find_map(|line| line.strip_prefix("id: ")) {
                    newest = id.to_string();
                }
                if event.starts_with("event: status\n") {
                    return newest;
                }
            }
        }

        #[test]
        fn pages_get_transactions_while_web_may_pay() {
            let mut rig = Rig::with("[payments]\nlimit = \"unlimited\"\nperms = [\"ctl\", \"web\"]\n", true);
            rig.bot.start_payments(&txlog::ReadBack::default(), wall_clock());
            let (mut page, _) = rig.open("/api/events");
            // A payment made through the control socket.
            let (reply, answer) = mpsc::channel();
            let request = Request::decode(format!("pay\t25\tavatar\t{}", jane()).as_bytes()).unwrap();
            rig.bot.control(request, reply, Instant::now());
            let (id, names) = janes_names();
            rig.bot.names_resolved(vec![(id, names.clone())], Vec::new(), Instant::now());
            let own = rig.bot.inventory.agent;
            let info = udp::messages::TransactionInfo {
                kind: udp::messages::TRANSACTION_GIFT,
                source: own,
                source_group: false,
                dest: jane(),
                dest_group: false,
                amount: 25,
                item: String::new(),
            };
            hear(
                &mut rig.bot,
                SessionEvent::Money {
                    balance: 975,
                    success: true,
                    description: "You paid Jane Doe L$25.".into(),
                    info: Some(info),
                },
            );
            assert!(matches!(answer.try_recv(), Ok(Response::Ok(_))));
            // Its names are known: it is recorded at once.
            rig.bot.service_messages();
            let event = page.event();
            assert!(event.starts_with("event: money\nid: "), "{event}");
            let data = event.lines().nth(2).unwrap().strip_prefix("data: ").unwrap();
            let fields: Vec<&str> = data.split('\t').collect();
            assert_eq!(fields.len(), 12);
            assert_eq!(
                fields[1..9],
                ["paid", "25", "975", "25", "5001", &jane().to_string(), "Jane Doe", "Janey"]
            );
            // The status the page gets has the money lines, and changed.
            rig.bot.web_status();
            let status = page.event();
            assert!(
                status.starts_with("event: status\n") && status.contains("balance: L$975"),
                "{status}"
            );
        }

        #[test]
        fn the_page_and_its_console_are_the_interface_web() {
            let mut rig = Rig::with("[payments]\nlimit = \"unlimited\"\nperms = [\"web\"]\n", true);
            assert!(rig.page("status").ends_with("balance: unknown\\nlimit: unlimited\n"));
            assert!(rig.console("status").ends_with("balance: unknown\\nlimit: unlimited\n"));
            assert!(!ok(&mut rig.bot, "status").contains("limit"), "the control socket is `ctl`");
            let mut ctl = Rig::with("[payments]\nlimit = \"unlimited\"\nperms = [\"ctl\"]\n", true);
            let refused = ctl.console(&format!("pay 10 avatar \"{}\"", jane()));
            assert!(refused.starts_with("error\t") && refused.contains("`web`"), "{refused}");
            assert_eq!(ctl.calls(), []);
        }

        #[test]
        fn pages_get_no_transactions_while_web_may_not_pay() {
            let mut rig = Rig::with("[payments]\nlimit = \"unlimited\"\nperms = [\"ctl\"]\n", true);
            rig.bot.start_payments(&txlog::ReadBack::default(), wall_clock());
            let info = udp::messages::TransactionInfo {
                kind: udp::messages::TRANSACTION_GIFT,
                source: jane(),
                source_group: false,
                dest: rig.bot.inventory.agent,
                dest_group: false,
                amount: 50,
                item: String::new(),
            };
            hear(
                &mut rig.bot,
                SessionEvent::Money {
                    balance: 1050,
                    success: true,
                    description: String::new(),
                    info: Some(info),
                },
            );
            rig.bot.flush_messages();
            let (_, kept) = rig.open("/api/events");
            assert_eq!(kept, [] as [&str; 0]);
            assert!(!rig.bot.status(Interface::Web).contains("balance"));
        }

        #[test]
        fn payees_may_be_the_pages_own_and_say_may_not() {
            let mut rig = Rig::with(
                "[payments]\nlimit = \"unlimited\"\nperms = [\"web\"]\nwhitelist = [\"Jane Doe\"]\n",
                true,
            );
            logged(&mut rig.bot);
            assert_eq!(rig.send("/api/command", "payees", true), "ok\t?\\tJane Doe\\twhitelist\n");
            assert_eq!(logged(&mut rig.bot), [] as [&str; 0]);
            let refused = body(&post_to(rig.address(), "/api/command", &["X-Rslbot-Own: 1"], "say\thello")).to_string();
            assert!(refused.contains("cannot be sent as the page's own"), "{refused}");
            assert!(rig.incoming.try_recv().is_err(), "say reached the bot");
        }

        #[test]
        fn a_message_that_looks_like_a_command_is_shown_and_not_carried_out() {
            let mut rig = Rig::new(true);
            let (mut page, _) = rig.open("/api/events");
            rig.heard(Channel::Im, from_jane("im", "say hello"));
            let event = page.event();
            assert!(event.starts_with("event: im\n") && event.ends_with("\tsay hello\n"), "{event}");
            assert_eq!(rig.calls(), []);
        }

        #[test]
        fn the_streams_end_when_the_bot_stops() {
            let mut rig = Rig::new(false);
            let (mut page, _) = rig.open("/api/events");
            assert!(matches!(rig.bot.handle(Event::Signal), Flow::Exit(EXIT_OK)));
            rig.bot.flush_messages();
            rig.bot.web_stop();
            let started = Instant::now();
            assert_eq!(page.event(), "");
            rig.server.stop();
            assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        }

        #[test]
        fn no_web_table_nothing_kept() {
            let mut bot = bot(full(""));
            assert!(bot.web.is_none());
            let now = Instant::now();
            bot.names_resolved(vec![janes_names()], vec![], now);
            bot.record(Channel::Chat, from_jane("say", "hi"), now);
            bot.service_messages();
            bot.web_status();
            assert!(bot.web.is_none());
            // With the table, the bot keeps what it records from the start.
            assert!(Rig::new(false).bot.web.is_some());
        }
    }
}
