//! slbot: logs an avatar in to a Second Life or OpenSimulator grid, gets it
//! to a target region, sits it on a target object and keeps it there.

mod backoff;
mod caps;
mod config;
mod log;
mod login;
mod net;
mod reconciler;
mod session;
mod udp {
    pub mod circuit;
    pub mod messages;
    pub mod packet;
}
mod xml;

use config::Config;
use login::Outcome;
use net::{Event, RealNet};
use reconciler::{Action, Decision, Reconciler};
use session::{Session, SessionEvent};
use std::io::{BufRead, IsTerminal};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const EXIT_OK: i32 = 0;
const EXIT_FATAL: i32 = 1;
const EXIT_CONFIG: i32 = 2;

const LOGIN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on one wait in the main loop when nothing is scheduled.
const IDLE_WAIT: Duration = Duration::from_secs(60);

const USAGE: &str = "\
Usage: slbot [--verbose] [CONFIG]
       slbot hash

Runs the bot from CONFIG (default: slbot.toml).
`slbot hash` reads a password from standard input and prints the value to
put in the config as `password_hash`.

Options:
  -v, --verbose   also log individual simulator packets
  -h, --help      show this help";

fn main() {
    std::process::exit(run_cli(std::env::args().skip(1).collect()));
}

fn run_cli(args: Vec<String>) -> i32 {
    let mut positional = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "-v" | "--verbose" => log::set_verbose(true),
            "-h" | "--help" => {
                println!("{USAGE}");
                return EXIT_OK;
            }
            option if option.starts_with('-') => {
                eprintln!("slbot: unknown option {option}\n\n{USAGE}");
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
            eprintln!("slbot: too many arguments\n\n{USAGE}");
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
            eprintln!("slbot: cannot read the password: {e}");
            return EXIT_FATAL;
        }
    };
    let password = password.strip_suffix('\n').unwrap_or(&password);
    let password = password.strip_suffix('\r').unwrap_or(password);
    if password.is_empty() {
        eprintln!("slbot: the password is empty");
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
                let session = Session::new(net, &info, cfg.object, self.next_generation, now);
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
    }

    fn apply(&mut self, event: SessionEvent, now: Instant) -> Flow {
        let rec = &mut self.reconciler;
        match event {
            SessionEvent::RegionEntered(name) => {
                rec.region_entered(&name, now);
                let target = if rec.in_target_region() { "the target region" } else { "not the target region" };
                info!("connected, in region \"{name}\" ({target})");
            }
            SessionEvent::Lost(reason) => {
                let delay = rec.session_lost(now);
                self.drop_session();
                if self.stopping {
                    info!("session lost while logging out: {reason}");
                    return Flow::Exit(EXIT_OK);
                }
                info!("session lost: {reason}; next login in {}", seconds(delay));
            }
            SessionEvent::Seated => {
                rec.seated(now);
                info!("seated on {}", self.config.object);
            }
            SessionEvent::Standing => match rec.standing(now) {
                Some(delay) => info!("unseated; sitting again in {}", seconds(delay)),
                None => info!("not seated on the target"),
            },
            SessionEvent::SitFailed(reason) => {
                let delay = rec.sit_failed(now);
                info!("sit failed: {reason}; next attempt in {}", seconds(delay));
            }
            SessionEvent::TeleportArrived => {
                rec.teleport_arrived();
                info!("teleport arrived at the configured position");
            }
            SessionEvent::TeleportFailed(reason) => {
                let delay = rec.teleport_failed(now);
                info!("teleport failed: {reason}; next attempt in {}", seconds(delay));
            }
            SessionEvent::Alert(message) => info!("simulator alert: {message}"),
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
        let Some(session) = &mut self.session else { return Flow::Continue };
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
                    session.start_sit(now);
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
            (Event::Datagram { generation, data }, Some(session)) => {
                session.handle_datagram(generation, &data, now)
            }
            (Event::Caps { generation, event }, Some(session)) => {
                session.handle_caps(generation, event, now)
            }
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
            eprintln!("slbot: {e}");
            return EXIT_CONFIG;
        }
    };
    let (events, incoming) = mpsc::channel();
    install_signal_handler(events.clone());
    let mut bot = Bot {
        reconciler: Reconciler::new(&config.region),
        config,
        session: None,
        events,
        http: caps::http_agent(LOGIN_HTTP_TIMEOUT),
        next_generation: 1,
        stopping: false,
    };

    loop {
        if let Flow::Exit(code) = bot.service_session() {
            return code;
        }
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
