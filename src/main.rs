//! rslbot: logs an avatar in to a Second Life or OpenSimulator grid, gets it
//! to a target region, sits it on a target object and keeps it there.

mod backoff;
mod caps;
mod commands;
mod config;
mod control;
mod escape;
mod log;
mod login;
mod msglog;
mod names;
mod net;
mod reconciler;
mod session;
mod udp {
    pub mod circuit;
    pub mod messages;
    pub mod packet;
}
mod xml;

use commands::{Commands, Grid, SentIm};
use config::Config;
use login::Outcome;
use msglog::{Channel, Logs, Recorder};
use names::{Lookup, Search};
use net::{Event, RealNet};
use reconciler::{Action, Decision, Reconciler};
use control::{Request, Response};
use session::{Seat, Session, SessionEvent, Touch};
use udp::messages::Vector3;
use std::io::{BufRead, IsTerminal};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const EXIT_OK: i32 = 0;
const EXIT_FATAL: i32 = 1;
const EXIT_CONFIG: i32 = 2;

const LOGIN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on one wait in the main loop when nothing is scheduled.
const IDLE_WAIT: Duration = Duration::from_secs(60);

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
}

/// The session as commands see it. A simulator with an avatar search
/// service is asked there, as viewers do; the older simulator message is
/// for the ones without.
struct Live<'a> {
    session: &'a mut Session<RealNet>,
    search_url: Option<&'a str>,
    avatar_searches: &'a Sender<Search>,
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
        let search = Search { url: url.to_string(), query, name: name.to_string() };
        self.avatar_searches.send(search).map_err(|_| "the avatar search is not running".to_string())
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
}

/// The session, if there is one, for commands to act on.
fn live<'a>(
    session: &'a mut Option<Session<RealNet>>,
    search_url: &'a Option<String>,
    avatar_searches: &'a Sender<Search>,
) -> Option<Live<'a>> {
    let session = session.as_mut()?;
    Some(Live { session, search_url: search_url.as_deref(), avatar_searches })
}

/// What the main loop should do after handling something.
enum Flow {
    Continue,
    Exit(i32),
}

impl Bot {
    fn login(&mut self, start: login::Start) {
        let cfg = &self.config;
        info!(
            "logging in as {} {} at {} (start: {})",
            cfg.first_name,
            cfg.last_name,
            cfg.login_url,
            login::start_location(cfg, start)
        );
        let outcome = login::login(&self.http, cfg, start);
        let now = Instant::now();
        match outcome {
            Outcome::Success(info) => {
                info!(
                    "login accepted; connecting to simulator {} with circuit code {}",
                    info.sim_addr, info.circuit_code
                );
                self.reconciler.login_accepted();
                let net = RealNet::new(self.events.clone());
                let name = format!("{} {}", cfg.first_name, cfg.last_name);
                let generation = self.next_generation;
                let session =
                    Session::new(net, &info, &name, cfg.scan_distance, cfg.fetch_offline_im, generation, now);
                self.session = Some(session);
            }
            Outcome::NeedsOperator(failure) => {
                info!(
                    "login needs the operator ({}): {}",
                    failure.reason, failure.message
                );
                self.reconciler.login_needs_operator(&failure);
            }
            Outcome::Refused(failure) => {
                info!("login refused ({}): {}", failure.reason, failure.message);
                if let Some(delay) = self.reconciler.login_refused(&failure, now) {
                    info!(
                        "next login in {}, asking for the last location",
                        seconds(delay)
                    );
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
        self.commands.session_lost();
    }

    /// The grid answered an avatar search: the commands waiting for it go on.
    fn avatars_found(&mut self, query: Uuid, avatars: &[(Uuid, String)], now: Instant) {
        let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches);
        for sent in self.commands.avatars_found(query, avatars, live.as_mut(), now) {
            self.record_sent(sent, now);
        }
        self.follow_commands();
    }

    /// Stops putting the avatar back once a command has moved it.
    fn follow_commands(&mut self) {
        if self.commands.take_manual() {
            self.set_manual(true);
        }
    }

    /// Switches between holding the configured seat and leaving the avatar
    /// where the operator put it.
    fn set_manual(&mut self, manual: bool) {
        if self.reconciler.is_manual() == manual {
            return;
        }
        self.reconciler.set_manual(manual);
        if manual {
            info!("placement: manual; the avatar stays where commands put it until `home`");
        } else {
            info!("placement: holding; returning to the configured region and seat");
        }
    }

    /// The answer to the `status` command.
    fn status(&self) -> String {
        let mut lines = Vec::new();
        let region = self.session.as_ref().and_then(|session| session.region().map(|name| (session, name)));
        lines.push(format!("connected: {}", if region.is_some() { "yes" } else { "no" }));
        if let Some((session, name)) = region {
            lines.push(format!("region: {name}"));
            if let Some([x, y, z]) = session.position() {
                lines.push(format!("position: {x}, {y}, {z}"));
            }
            let seat = match session.seat() {
                Seat::Unknown => "unknown".to_string(),
                Seat::None => "none".to_string(),
                Seat::Object(object) if object == self.config.object => "target".to_string(),
                Seat::Object(object) => format!("object {object}"),
                Seat::Other => "other".to_string(),
            };
            lines.push(format!("seat: {seat}"));
        }
        lines.push(format!("mode: {}", if self.reconciler.is_manual() { "manual" } else { "holding" }));
        lines.join("\n")
    }

    /// Carries out a command from the control socket.
    fn control(&mut self, request: Request, reply: Sender<Response>, now: Instant) -> Flow {
        match request {
            Request::Status => {
                // The client may have gone away; nothing depends on it.
                let _ = reply.send(Response::Ok(Some(self.status())));
            }
            Request::Home => {
                info!("control: home");
                self.commands.forget_requests();
                self.set_manual(false);
                let _ = reply.send(Response::Ok(None));
            }
            Request::Quit => {
                info!("control: quit");
                let _ = reply.send(Response::Ok(None));
                return self.stop();
            }
            request => {
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches);
                if let Some(sent) = self.commands.handle(request, reply, live.as_mut(), now) {
                    self.record_sent(sent, now);
                }
                self.follow_commands();
            }
        }
        Flow::Continue
    }

    /// Puts an instant message the bot sent into the message log.
    fn record_sent(&mut self, sent: SentIm, now: Instant) {
        let source = session::Source::Avatar { id: sent.to, name: sent.name };
        let message = session::Message { kind: "im_out".to_string(), source, sent_at: None, text: sent.text };
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
                    let lookup = Lookup { url: url.clone(), ids: self.recorder.take_lookups() };
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
                let target = if self.reconciler.in_target_region() {
                    "the target region"
                } else {
                    "not the target region"
                };
                info!("connected, in region \"{name}\" ({target})");
                self.commands.arrived(&name);
            }
            SessionEvent::Lost(reason) => {
                let delay = self.reconciler.session_lost(now);
                self.drop_session();
                if self.stopping {
                    info!("session lost while logging out: {reason}");
                    return Flow::Exit(EXIT_OK);
                }
                info!("session lost: {reason}; next login in {}", seconds(delay));
            }
            SessionEvent::Seated(object) => {
                if object == self.config.object {
                    self.reconciler.seated(now);
                    info!("seated on {object}");
                } else {
                    // Not the seat the bot holds: as good as standing to it.
                    self.reconciler.standing(now);
                    info!("seated on {object}, which is not the target");
                }
                self.commands.seated(object);
            }
            SessionEvent::Standing => match self.reconciler.standing(now) {
                Some(_) if manual => info!("unseated"),
                Some(delay) => info!("unseated; sitting again in {}", seconds(delay)),
                None => info!("not seated on the target"),
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
            }
            SessionEvent::TeleportFailed(reason) => {
                if self.commands.teleport_failed(&reason) || manual {
                    info!("teleport failed: {reason}");
                } else {
                    let delay = self.reconciler.teleport_failed(now);
                    info!("teleport failed: {reason}; next attempt in {}", seconds(delay));
                }
            }
            SessionEvent::TeleportOffer { from, name, lure } => {
                let mut live = live(&mut self.session, &self.search_url, &self.avatar_searches);
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
            SessionEvent::Alert(message) => info!("simulator alert: {message}"),
            SessionEvent::Chat(message) => self.record(Channel::Chat, message, now),
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
        let Some(session) = &mut self.session else {
            return Flow::Continue;
        };
        session.tick(now);
        for event in session.take_events() {
            if let Flow::Exit(code) = self.apply(event, now) {
                return Flow::Exit(code);
            }
        }
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
                    let cfg = &self.config;
                    let [x, y, z] = cfg.position;
                    info!("teleporting to \"{}\" at {x}, {y}, {z}", cfg.region);
                    session.start_teleport(&cfg.region, cfg.position, now);
                }
                (Action::Sit, Some(session)) => {
                    info!("sitting on {}", self.config.object);
                    session.start_sit(self.config.object, now);
                }
                // The reconciler only asks for these while connected.
                (Action::Teleport | Action::Sit, None) => return Ok(None),
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
            (Event::Datagram { generation, data }, Some(session)) => {
                session.handle_datagram(generation, &data, now)
            }
            (Event::Caps { generation, event }, Some(session)) => {
                session.handle_caps(generation, event, now)
            }
            (Event::Names { found, missing }, _) => self.recorder.names_resolved(found, missing, now),
            (Event::NamesFailed(ids), _) => self.recorder.lookup_failed(&ids),
            // Left over from a session that is already gone.
            (_, None) => {}
        }
        Flow::Continue
    }
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
    let mut bot = Bot {
        recorder: Recorder::default(),
        logs,
        name_lookups: names::spawn(events.clone()),
        names_url: None,
        commands: Commands::new(config.requesttp_timeout),
        avatar_searches: names::spawn_search(events.clone()),
        search_url: None,
        reconciler: Reconciler::new(&config.region),
        config,
        session: None,
        events,
        http: caps::http_agent(LOGIN_HTTP_TIMEOUT),
        next_generation: 1,
        stopping: false,
    };
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
        bot.service_messages();
        let mut wake = Instant::now() + IDLE_WAIT;
        if !bot.stopping {
            match bot.reconcile() {
                Ok(Some(until)) => wake = wake.min(until),
                Ok(None) => {}
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
}
