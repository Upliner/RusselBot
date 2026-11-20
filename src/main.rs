//! rslbot: logs an avatar in to a Second Life or OpenSimulator grid, gets it
//! to a target region, sits it on a target object and keeps it there.

mod backoff;
mod caps;
mod commands;
mod config;
mod control;
mod escape;
mod inventory;
mod log;
mod login;
mod msglog;
mod names;
mod net;
mod perms;
mod persist;
mod reconciler;
mod rlv;
mod session;
mod udp {
    pub mod circuit;
    pub mod messages;
    pub mod packet;
}
mod xml;
mod xmpp;

use commands::{Commands, Grid, SentIm};
use config::{Config, Persist, PersistOverride, Place, RlvPersist, Sit, Source};
use control::{Request, Response, RlvRequest};
use login::Outcome;
use msglog::{Channel, Logs, Recorder};
use names::{Lookup, Search};
use net::{Event, RealNet};
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
    avatar_searches: Sender<Search>,
    /// The current simulator's avatar search service, if it has one.
    search_url: Option<String>,
    /// The placement the bot holds, and what `persist` remembers.
    book: Book,
    /// The config's `persist`, until it has been applied or the operator
    /// has taken over.
    startup: Option<Persist>,
    /// Since when the config placement has been reached but for the
    /// position of the seat.
    startup_wait: Option<Instant>,
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

/// The session as commands see it. A simulator with an avatar search
/// service is asked there, as viewers do; the older simulator message is
/// for the ones without.
struct Live<'a> {
    session: &'a mut Session<RealNet>,
    search_url: Option<&'a str>,
    avatar_searches: &'a Sender<Search>,
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
        let Some(url) = self.search_url else {
            return self.session.find_avatar(query, name, now);
        };
        self.session.present()?;
        let search = Search {
            url: url.to_string(),
            query,
            name: name.to_string(),
        };
        self.avatar_searches
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
fn live<'a>(
    session: &'a mut Option<Session<RealNet>>,
    search_url: &'a Option<String>,
    avatar_searches: &'a Sender<Search>,
    inventory: &'a Inventory,
    rlv: &'a Rlv,
) -> Option<Live<'a>> {
    let session = session.as_mut()?;
    Some(Live {
        session,
        search_url: search_url.as_deref(),
        avatar_searches,
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
            // Before the first login, which may take a while.
            xmpp: config.xmpp.as_ref().map(|xmpp| xmpp::gateway::Gateway::start(xmpp, events.clone())),
            reconciler: Reconciler::new(goal(book.held(), config_start(&config))),
            book,
            startup: Some(config.persist),
            startup_wait: None,
            recorder: Recorder::default(),
            logs,
            name_lookups: names::spawn(events.clone()),
            names_url: None,
            commands,
            avatar_searches: names::spawn_search(events.clone()),
            search_url: None,
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
        self.search_url = None;
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
        let mut live = live(
            &mut self.session,
            &self.search_url,
            &self.avatar_searches,
            &self.inventory,
            &self.rlv,
        );
        self.commands.restore_due(live.as_mut(), now);
    }

    /// Decides the requests that waited too long for the grid to say who
    /// is behind them (see `Commands::requests_due`).
    fn decide_requests(&mut self, now: Instant) {
        let mut live = live(
            &mut self.session,
            &self.search_url,
            &self.avatar_searches,
            &self.inventory,
            &self.rlv,
        );
        self.commands.requests_due(live.as_mut(), now);
        self.follow_commands();
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
        let mut live = live(
            &mut self.session,
            &self.search_url,
            &self.avatar_searches,
            &self.inventory,
            &self.rlv,
        );
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
        let settled = match startup_due(self.book.held().sit, setting, arrival, self.startup_wait, now) {
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
        self.startup = None;
        self.startup_wait = None;
        let for_rlv = std::mem::take(&mut self.follow_seat);
        let Persist::On { position, sit } = setting else {
            info!("the avatar is in place and `persist` is off");
            self.set_manual(true);
            return None;
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
        None
    }

    /// The answer to the `status` command.
    fn status(&self) -> String {
        let here = self.session.as_ref().and_then(|session| {
            let region = session.region()?;
            Some(Whereabouts {
                region,
                position: session.position(),
                seat: session.seat(),
            })
        });
        status_text(here, self.book.held(), self.reconciler.is_manual())
    }

    /// Carries out a command from the control socket.
    fn control(&mut self, request: Request, reply: Sender<Response>, now: Instant) -> Flow {
        // While RLV is in override, the next command but these four is
        // exempt from the restrictions, and RLV is on again with it.
        let never = matches!(request, Request::Rlv(_) | Request::Status | Request::Perms(_) | Request::Quit);
        let exempt = !never && self.rlv.take_override(request.name());
        match request {
            Request::Status => {
                // The client may have gone away; nothing depends on it.
                let _ = reply.send(Response::Ok(Some(self.status())));
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
            Request::Quit => {
                info!("control: quit");
                let _ = reply.send(Response::Ok(None));
                return self.stop();
            }
            request => {
                let origin = if exempt {
                    commands::Origin::Exempt
                } else {
                    commands::Origin::Operator
                };
                #[cfg(test)]
                if let Some(grid) = self.grid.as_mut() {
                    if let Some(sent) = self.commands.handle_from(origin, request, reply, Some(grid), now) {
                        self.record_sent(sent, now);
                    }
                    self.follow_commands();
                    return Flow::Continue;
                }
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
                if let Some(sent) = self.commands.handle_from(origin, request, reply, live.as_mut(), now) {
                    self.record_sent(sent, now);
                }
                self.follow_commands();
            }
        }
        Flow::Continue
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
            let mut live = live(
                &mut self.session,
                &self.search_url,
                &self.avatar_searches,
                &self.inventory,
                &self.rlv,
            );
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
            let mut live = live(
                &mut self.session,
                &self.search_url,
                &self.avatar_searches,
                &self.inventory,
                &self.rlv,
            );
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

    /// Puts an instant message the bot sent into the message log.
    fn record_sent(&mut self, sent: SentIm, now: Instant) {
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
        for record in self.recorder.flush(now) {
            self.logs.write(&record);
            if let Some(xmpp) = &mut self.xmpp {
                xmpp.record(&record, now);
            }
        }
    }

    /// The grid named these avatars and does not know those: the message
    /// log and the requests that waited for a name go on.
    fn names_resolved(&mut self, found: Vec<(Uuid, names::Names)>, missing: Vec<Uuid>, now: Instant) {
        let mut live = live(
            &mut self.session,
            &self.search_url,
            &self.avatar_searches,
            &self.inventory,
            &self.rlv,
        );
        self.commands.names_resolved(&found, &missing, live.as_mut(), now);
        self.recorder.names_resolved(found, missing, now);
    }

    /// A name lookup came to nothing.
    fn names_failed(&mut self, ids: &[Uuid], now: Instant) {
        let mut live = live(
            &mut self.session,
            &self.search_url,
            &self.avatar_searches,
            &self.inventory,
            &self.rlv,
        );
        self.commands.names_failed(ids, live.as_mut(), now);
        self.recorder.lookup_failed(ids);
    }

    /// Writes every message still waiting for names; for use on the way out.
    fn flush_messages(&mut self) {
        let now = Instant::now();
        for record in self.recorder.flush_all(now) {
            self.logs.write(&record);
            if let Some(xmpp) = &mut self.xmpp {
                xmpp.record(&record, now);
            }
        }
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
        let flow = self.control(request, reply, now);
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
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
                self.commands.teleport_offer(from, &name, lure, live.as_mut(), now);
                // One accepted by rule moves the avatar as a command does.
                self.follow_commands();
            }
            SessionEvent::InventoryOffer(offer) => {
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
                self.commands.inventory_offer(offer, live.as_mut(), now);
            }
            SessionEvent::PermissionRequest {
                object,
                item,
                object_name,
                owner_name,
                permissions,
            } => {
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
                self.commands
                    .permission_request(object, item, &object_name, &owner_name, permissions, live.as_mut(), now);
            }
            SessionEvent::ObjectOwner { object, owner, group } => {
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
                self.commands.object_owner(object, owner, group, live.as_mut(), now);
            }
            SessionEvent::TeleportRequested { from, name, text } => {
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
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
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
                if !self.commands.attached(item, point, live.as_mut(), now) {
                    info!("attachment: item {item} is on {}", inventory::point_name(point));
                }
            }
            SessionEvent::Detached { item, point } => {
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
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
            SessionEvent::InstantMessage(message) => {
                if let session::Source::Avatar { id, .. } = &message.source {
                    self.commands.im_received(*id);
                }
                self.record(Channel::Im, message, now)
            }
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
                self.search_url = url;
            }
            SessionEvent::AvatarsFound { query, avatars } => self.avatars_found(query, &avatars, now),
            SessionEvent::LoggedOut => {
                info!("logged out");
                return Flow::Exit(EXIT_OK);
            }
        }
        Flow::Continue
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
            (Event::AvatarsFound { query, avatars }, _) => self.avatars_found(query, &avatars, now),
            (Event::AvatarSearchFailed { query, reason }, _) => self.commands.lookup_failed(query, &reason),
            (Event::Datagram { generation, data }, Some(session)) => session.handle_datagram(generation, &data, now),
            (Event::Caps { generation, event }, Some(session)) => session.handle_caps(generation, event, now),
            (Event::Names { found, missing }, _) => self.names_resolved(found, missing, now),
            (Event::NamesFailed(ids), _) => self.names_failed(&ids, now),
            // An answer about another simulator's inventory service is of
            // no use: the commands that asked were told so.
            (Event::Inventory { generation, .. }, Some(session)) if generation != session.generation() => {}
            (Event::Inventory { id, result, .. }, _) => {
                let mut live = live(
                    &mut self.session,
                    &self.search_url,
                    &self.avatar_searches,
                    &self.inventory,
                    &self.rlv,
                );
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
}

/// The text of the `status` answer. `here` is missing while the avatar is
/// not in a region.
fn status_text(here: Option<Whereabouts>, held: &Placement, manual: bool) -> String {
    let mut lines = Vec::new();
    lines.push(format!("connected: {}", if here.is_some() { "yes" } else { "no" }));
    if let Some(Whereabouts { region, position, seat }) = here {
        lines.push(format!("region: {region}"));
        if let Some(position) = position {
            let [x, y, z] = position.map(persist::coordinate);
            lines.push(format!("position: {x}, {y}, {z}"));
        }
        let seat = match seat {
            Seat::Unknown => "unknown".to_string(),
            Seat::None => "none".to_string(),
            Seat::Object(_) if on_target(held.sit, seat) => "target".to_string(),
            Seat::Object(sat) => format!("object {}", sat.prim),
            Seat::Other => "other".to_string(),
        };
        lines.push(format!("seat: {seat}"));
    }
    lines.push(format!("persist: {held}"));
    lines.push(format!("mode: {}", if manual { "manual" } else { "holding" }));
    lines.join("\n")
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
/// `since` is when the wait for the seat's position began, if it has.
fn startup_due(sit: SitState, setting: Persist, arrival: Arrival, since: Option<Instant>, now: Instant) -> Startup {
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
    // Only a `cur` location is about where the avatar sits.
    let wants_position = matches!(setting, Persist::On { position: Source::Cur, .. });
    if wants_position && matches!(sit, SitState::Object(_)) && !arrival.seat_frame_known {
        let deadline = since.unwrap_or(now) + SEAT_POSITION_WAIT;
        if now < deadline {
            return Startup::Wait(deadline);
        }
        return Startup::Now { settled: false };
    }
    Startup::Now { settled: true }
}

fn run_bot(config_path: &Path) -> i32 {
    let config = match Config::load(config_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("rslbot: {e}");
            return EXIT_CONFIG;
        }
    };
    // Before anything touches the network: a log that cannot be written is
    // a config error.
    let logs = match Logs::open(config.log_chat.as_deref(), config.log_im.as_deref()) {
        Ok(logs) => logs,
        Err(e) => {
            eprintln!("rslbot: {}: `{}`: {}", config_path.display(), e.key, e.message);
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
    install_signal_handler(events.clone());
    if config.deprecated_object {
        info!("{}: `object` is deprecated; write the same value as `sit`", config_path.display());
    }
    let mut bot = Bot::new(config, logs, events);
    let code = run_loop(&mut bot, &incoming);
    bot.flush_messages();
    bot.xmpp_stop();
    code
}

fn run_loop(bot: &mut Bot, incoming: &Receiver<Event>) -> i32 {
    loop {
        if let Flow::Exit(code) = bot.service_session() {
            return code;
        }
        bot.commands.tick(Instant::now());
        bot.decide_requests(Instant::now());
        bot.restore_attachments(Instant::now());
        bot.service_messages();
        bot.xmpp_presence();
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
            };
            assert!(status_text(Some(here), &held, false).contains(&format!("\nseat: {word}\n")));
        }
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
        let due = |sit, arrival| startup_due(sit, CFG, arrival, None, now);
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
        assert_eq!(startup_due(sit, CUR, known, None, now), Startup::Now { settled: true });
        // Not known: wait, from when the wait began.
        assert_eq!(startup_due(sit, CUR, seated, None, now), Startup::Wait(now + SEAT_POSITION_WAIT));
        let later = now + Duration::from_secs(4);
        assert_eq!(
            startup_due(sit, CUR, seated, Some(now), later),
            Startup::Wait(now + SEAT_POSITION_WAIT)
        );
        assert_eq!(startup_due(sit, CUR, known, Some(now), later), Startup::Now { settled: true });
        // After ten seconds it goes ahead with what it has.
        let late = now + SEAT_POSITION_WAIT;
        assert_eq!(startup_due(sit, CUR, seated, Some(now), late), Startup::Now { settled: false });
        // Only a current location is about the seat's position.
        let seat_only = Persist::On {
            position: Source::Cfg,
            sit: Source::Cur,
        };
        for setting in [CFG, seat_only, Persist::Off] {
            assert_eq!(startup_due(sit, setting, seated, None, now), Startup::Now { settled: true });
        }
        // And only a seat on an object has one.
        let on_ground = Arrival {
            ground_sat: true,
            ..arrived()
        };
        assert_eq!(
            startup_due(SitState::Ground, CUR, on_ground, None, now),
            Startup::Now { settled: true }
        );
        assert_eq!(
            startup_due(SitState::Stand, CUR, arrived(), None, now),
            Startup::Now { settled: true }
        );
    }

    // ------------------------------------------ persist without a session

    /// A bot that has not logged in, and the control answers it gives.
    fn bot(config: Config) -> Bot {
        let (events, _incoming) = mpsc::channel();
        Bot::new(config, Logs::open(None, None).ok().unwrap(), events)
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
        assert!(bot.status().ends_with("persist: Blue Lagoon/128/128/25 ground\nmode: holding"));
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
        assert_eq!(bot.status(), "connected: no\npersist: unknown stand\nmode: holding");
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
        assert_eq!(startup_due(held, cur, arrived(), None, now), Startup::NotYet);
        let seated = Arrival {
            seated_on_target: true,
            ..arrived()
        };
        assert_eq!(startup_due(held, cur, seated, None, now), Startup::Wait(now + SEAT_POSITION_WAIT));
        let later = now + SEAT_POSITION_WAIT;
        assert_eq!(startup_due(held, cur, seated, Some(now), later), Startup::Now { settled: false });
        let described = Arrival {
            seat_frame_known: true,
            ..seated
        };
        assert_eq!(startup_due(held, cur, described, Some(now), now), Startup::Now { settled: true });
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
             attach: none\ncontrols: none\ncamera: none"
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
        assert_eq!(ok(&mut bot, "perms\tstatus").lines().count(), 6);
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
                let fake = Fake::new();
                let config = full(&format!(
                    "[xmpp]\ncomponent = \"{DOMAIN}\"\nserver = \"{}\"\nsecret = \"{SECRET}\"\nowners = [\"{OWNER}\"]\n",
                    fake.address()
                ));
                let (events, incoming) = mpsc::channel();
                let mut bot = Bot::new(config, Logs::open(None, None).ok().unwrap(), events);
                if grid {
                    bot.grid = Some(FakeGrid::default());
                }
                Rig { bot, incoming, fake }
            }

            fn connected(grid: bool) -> Rig {
                let mut rig = Rig::new(grid);
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

            fn from_owner(&mut self, to: &str, body: &str) -> Flow {
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
        fn commands_are_answered_as_the_control_socket_answers_them() {
            let mut rig = Rig::connected(false);
            rig.from_owner(DOMAIN, "status");
            assert_eq!(answer(&mut rig), rig.bot.status());
            rig.from_owner(DOMAIN, "sit ground");
            let reason = refused(&mut rig.bot, "sit\tground");
            assert_eq!(answer(&mut rig), format!("error: {reason}"));
            rig.from_owner(DOMAIN, "im \"Jane Doe hi");
            let unclosed = answer(&mut rig);
            assert!(unclosed.starts_with("error: ") && unclosed.contains("quote"), "{unclosed}");
            rig.from_owner(DOMAIN, "dance");
            assert!(answer(&mut rig).starts_with("error: "));
            rig.from_owner(DOMAIN, "");
            assert!(answer(&mut rig).contains("help"));
            rig.from_owner(DOMAIN, "help");
            assert!(answer(&mut rig).contains("\nim AVATAR MESSAGE...\n"));
            rig.from_owner(DOMAIN, &"x".repeat(8193));
            assert!(answer(&mut rig).contains("8192"));
        }

        #[test]
        fn owners_messages_become_commands_ims_and_chat() {
            let mut rig = Rig::connected(true);
            // Name with a space: the answer comes once the name is found.
            rig.from_owner(DOMAIN, "im \"Jane Doe\" see you later");
            let query = rig.lookup();
            rig.bot.avatars_found(query, &[(jane(), "Jane Doe".into())], Instant::now());
            assert_eq!(rig.calls(), [Call::Im(jane(), "see you later".into())]);
            rig.next();
            assert_eq!(answer(&mut rig), format!("sent to Jane Doe <{}>", jane()));
            // The sent IM is recorded, and not passed to the owners.
            rig.bot.names_resolved(vec![janes_names()], vec![], Instant::now());
            rig.bot.service_messages();
            assert_eq!(rig.quiet(), "");

            // Reply to an avatar: the address stands for the UUID now.
            rig.from_owner(&format!("jane.doe@{DOMAIN}"), "see you");
            assert_eq!(rig.calls(), [Call::Im(jane(), "see you".into())]);
            assert_eq!(rig.quiet(), "");

            // Unknown name.
            rig.from_owner(&format!("no.body@{DOMAIN}"), "hi");
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
            rig.from_owner(&format!("obj-{}@{DOMAIN}", collar()), "hi");
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
            rig.from_owner(&format!("local@{DOMAIN}"), "hello everyone");
            rig.from_owner(&format!("local@{DOMAIN}"), "/shout come here");
            rig.from_owner(&format!("local@{DOMAIN}"), "/7 open");
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
            rig.from_owner(&format!("local@{DOMAIN}"), &"x".repeat(2000));
            assert!(rig.fake.expect("</message>").contains("<not-acceptable "));
            assert_eq!(rig.calls(), []);
        }

        #[test]
        fn a_teleport_does_not_hold_back_other_contacts() {
            let mut rig = Rig::connected(true);
            rig.bot.names_resolved(vec![janes_names()], vec![], Instant::now());
            rig.heard(Channel::Im, from_jane("im", "hello"), vec![janes_names()]);
            rig.fake.expect("</message>");
            rig.from_owner(DOMAIN, "tp 128 128 25 Home");
            assert!(matches!(&rig.calls()[..], [Call::Teleport(region, _)] if region == "Home"));
            rig.from_owner(&format!("jane.doe@{DOMAIN}"), "hi");
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
                rig.from_owner(DOMAIN, &format!("im \"Name{n} Doe\" hi"));
                rig.lookup();
            }
            assert_eq!(rig.bot.xmpp.as_ref().unwrap().waiting(), 16);
            rig.from_owner(DOMAIN, "im \"Name16 Doe\" hi");
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
            assert!(matches!(rig.from_owner(DOMAIN, "quit"), Flow::Exit(EXIT_OK)));
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
}
