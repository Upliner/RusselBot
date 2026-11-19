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

use commands::{Commands, Grid, SentIm};
use config::{Config, Persist, Place, RlvPersist, Sit, Source};
use control::{Request, Response, RlvRequest};
use login::Outcome;
use msglog::{Channel, Logs, Recorder};
use names::{Lookup, Search};
use net::{Event, RealNet};
use persist::{Book, Location, LocationArg, Placement, SitArg, SitState, World};
use reconciler::{Action, Decision, Goal, Reconciler, SeatGoal};
use rlv::{After, Follow, Issuer, Rlv, Step};
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
    /// Stands in for a session's view of the world, which tests have none of.
    #[cfg(test)]
    world: Option<Box<dyn World>>,
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
}

impl Grid for Live<'_> {
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
}

/// The session, if there is one, for commands to act on.
fn live<'a>(
    session: &'a mut Option<Session<RealNet>>,
    search_url: &'a Option<String>,
    avatar_searches: &'a Sender<Search>,
    inventory: &'a Inventory,
) -> Option<Live<'a>> {
    let session = session.as_mut()?;
    Some(Live {
        session,
        search_url: search_url.as_deref(),
        avatar_searches,
        inventory,
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
        Bot {
            rlv: Rlv::new(config.rlv, config.rlv_persist),
            follow: Follow::default(),
            follow_seat: false,
            #[cfg(test)]
            world: None,
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
    }

    /// Reads the Current Outfit folder when it is time to compare it with
    /// what is attached (see `Commands::entered_region`).
    fn restore_attachments(&mut self, now: Instant) {
        let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
        self.commands.restore_due(live.as_mut(), now);
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
        let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
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
    fn persist(&mut self, location: &LocationArg, sit: SitArg, now: Instant) -> Result<String, String> {
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
        self.follow.give_up();
        // An answer to an earlier `requesttp` must not take the avatar away.
        self.commands.forget_requests();
        self.sync_goal(now);
        if let Some(session) = self.session.as_mut().filter(|session| session.present().is_ok()) {
            if needs_stand(placement.sit, session.seat(), session.on_ground()) {
                match session.stand(now) {
                    Ok(()) => {
                        info!("standing up");
                        self.commands.stood_up();
                    }
                    Err(reason) => info!("cannot stand up: {reason}"),
                }
            }
        }
        self.set_manual(false);
        Ok(placement.to_string())
    }

    /// A `persist` or `home` command: reads its arguments, carries it out
    /// and answers.
    fn persist_command(&mut self, what: &str, location: &str, sit: &str, reply: &Sender<Response>, now: Instant) {
        let outcome = persist::parse_location(location)
            .and_then(|location| Ok((location, persist::parse_sit_state(sit)?)))
            .and_then(|(location, sit)| {
                info!("control: {what}");
                self.persist(&location, sit, now)
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
        if let Err(reason) = self.persist(&location, sit, now) {
            info!("{what} refused: {reason}");
        }
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
        match request {
            Request::Status => {
                // The client may have gone away; nothing depends on it.
                let _ = reply.send(Response::Ok(Some(self.status())));
            }
            Request::Persist { location, sit } => {
                let (location, sit) = (location.as_deref().unwrap_or("cur"), sit.as_deref().unwrap_or("cur"));
                self.persist_command(&format!("persist {location} {sit}"), location, sit, &reply, now);
            }
            Request::Home => self.persist_command("home", "last", "last", &reply, now),
            Request::Rlv(RlvRequest::Mode(on)) => {
                let line = self.rlv.set_on(on);
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
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
                if let Some(sent) = self.commands.handle(request, reply, live.as_mut(), now) {
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
            let follows = self.rlv.follows();
            let was_manual = self.reconciler.is_manual();
            let step = {
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
                rlv::deal(&command, issuer, follows, &mut self.commands, live.as_mut(), now)
            };
            let outcome = match step {
                Step::Said(said) => Ok(Some(said)),
                Step::Done => Ok(None),
                Step::Moved(movement) => {
                    let after = self.follow.moved(movement, follows, was_manual);
                    self.go_manual();
                    self.after_forced(after, "", now);
                    Ok(None)
                }
                Step::HoldSeat(object) => self.hold_seat(issuer, &command.text, object, now).map(|()| None),
                Step::Failed(reason) => Err(reason),
            };
            outcomes.push((command.text, outcome));
        }
        outcomes
    }

    /// A forced sit that is followed: the object becomes the held seat at
    /// once, with the place the avatar is at, and the bot seats the avatar
    /// as on any held seat. When it sits there, its seated position is
    /// persisted the way the config's setting is applied.
    fn hold_seat(&mut self, issuer: &Issuer, command: &str, object: Uuid, now: Instant) -> Result<(), String> {
        info!("rlv: {issuer}: {command}: persist cur {object}");
        match self.persist(&LocationArg::Cur, SitArg::Is(SitState::Object(object)), now) {
            Ok(_) => {
                self.startup = Some(Persist::On {
                    position: Source::Cur,
                    sit: Source::Cur,
                });
                self.follow_seat = true;
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
                if let Err(reason) = self.persist(&LocationArg::Cur, SitArg::Is(sit), now) {
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
        if self.recorder.has_lookups() {
            match &self.names_url {
                // The simulator has not said yet; the messages wait.
                None => {}
                Some(None) => {
                    let ids = self.recorder.take_lookups();
                    self.recorder.lookup_failed(&ids);
                }
                Some(Some(url)) => {
                    let lookup = Lookup {
                        url: url.clone(),
                        ids: self.recorder.take_lookups(),
                    };
                    if let Err(unsent) = self.name_lookups.send(lookup) {
                        self.recorder.lookup_failed(&unsent.0.ids);
                    }
                }
            }
        }
        for record in self.recorder.flush(Instant::now()) {
            self.logs.write(&record);
        }
    }

    /// Writes every message still waiting for names; for use on the way out.
    fn flush_messages(&mut self) {
        for record in self.recorder.flush_all(Instant::now()) {
            self.logs.write(&record);
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
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
                self.commands.teleport_offer(from, &name, lure, live.as_mut(), now);
            }
            SessionEvent::TeleportRequested { from, name, text } => {
                info!("{name} <{from}> asks to be teleported here ({text}); `rslbot ctl sendtp` offers it");
            }
            SessionEvent::Touched(object) => self.commands.touched(object),
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
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
                if !self.commands.attached(item, point, live.as_mut(), now) {
                    info!("attachment: item {item} is on {}", inventory::point_name(point));
                }
            }
            SessionEvent::Detached { item, point } => {
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
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
            SessionEvent::InstantMessage(message) => self.record(Channel::Im, message, now),
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
            let action = match self.reconciler.decide(now) {
                Decision::Wait(until) => return Ok(until),
                Decision::Exit { code, message } => {
                    info!("giving up: {message}");
                    return Err(code);
                }
                Decision::Start(action) => action,
            };
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
            (Event::AvatarsFound { query, avatars }, _) => self.avatars_found(query, &avatars, now),
            (Event::AvatarSearchFailed { query, reason }, _) => self.commands.lookup_failed(query, &reason),
            (Event::Datagram { generation, data }, Some(session)) => session.handle_datagram(generation, &data, now),
            (Event::Caps { generation, event }, Some(session)) => session.handle_caps(generation, event, now),
            (Event::Names { found, missing }, _) => self.recorder.names_resolved(found, missing, now),
            (Event::NamesFailed(ids), _) => self.recorder.lookup_failed(&ids),
            // An answer about another simulator's inventory service is of
            // no use: the commands that asked were told so.
            (Event::Inventory { generation, .. }, Some(session)) if generation != session.generation() => {}
            (Event::Inventory { id, result, .. }, _) => {
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches, &self.inventory);
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
    code
}

fn run_loop(bot: &mut Bot, incoming: &Receiver<Event>) -> i32 {
    loop {
        if let Flow::Exit(code) = bot.service_session() {
            return code;
        }
        bot.commands.tick(Instant::now());
        bot.restore_attachments(Instant::now());
        bot.service_messages();
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
        assert_eq!(ok(&mut plain, "rlv\tstatus"), "rlv: off\npersist: drop");
        let mut on = bot(full("rlv = true\n"));
        assert_eq!(ok(&mut on, "rlv\tstatus"), "rlv: on\npersist: drop");
        let mut following = bot(full("[persist]\nrlv = \"follow\"\n"));
        assert_eq!(ok(&mut following, "rlv\tstatus"), "rlv: off\npersist: follow");
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
        assert_eq!(ok(&mut bot, "rlv\tstatus"), "rlv: on\npersist: drop");
        // The persist setting does not care whether RLV is on.
        assert_eq!(ok(&mut bot, "rlv\tmode\toff"), "rlv: off");
        assert_eq!(ok(&mut bot, "rlv\tpersist\tfollow"), "rlv persist: follow");
        assert_eq!(ok(&mut bot, "rlv\tpersist\tfollow"), "rlv persist: follow");
        assert_eq!(ok(&mut bot, "rlv\tstatus"), "rlv: off\npersist: follow");
        assert_eq!(ok(&mut bot, "rlv\tmode\ton"), "rlv: on");
        assert_eq!(ok(&mut bot, "rlv\tstatus"), "rlv: on\npersist: follow");
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
            refused(&mut bot, "rlv\tcmd\t@versionnum=5,detach=n,nonsense,"),
            format!(
                "versionnum=5: {}\ndetach=n: not supported\nnonsense: malformed: it has no `=`",
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
        moved(&mut bot, commands::Origin::Rlv, Request::Stand);
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
        moved(&mut bot, commands::Origin::Rlv, Request::Stand);
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
}
