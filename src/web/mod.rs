//! The web interface: the bot serves a page over HTTP, carries out the
//! commands the page sends as the control socket's are carried out, and
//! sends the page what the avatar hears and where it is as it happens.
//!
//! A listener thread accepts connections and gives each a thread of its
//! own, which serves one request. Commands and pages that open their
//! event stream reach the main loop as `Event::Web`; nothing here ever
//! makes the main loop wait.

pub mod auth;
pub mod http;
pub mod hub;

use crate::config::WebConfig;
use crate::control::{self, Request, Response};
use crate::net::Event;
use auth::{Auth, Login};
use http::{Bad, Method, Route};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// What the web interface asks of the main loop.
pub enum WebEvent {
    /// A command of a page, to be answered on `reply`. `own` when the page
    /// sent it by itself, to keep a list up to date.
    Command {
        request: Request,
        reply: Sender<Response>,
        own: bool,
    },
    /// A page opens its event stream, having got `last` as its last
    /// event.
    Subscribe { last: Option<String>, page: SyncSender<Arc<str>> },
}

/// How many connections are served at once, event streams included.
const MAX_CONNECTIONS: usize = 32;
/// How many of them may be event streams.
const MAX_STREAMS: usize = 8;
/// How many commands may wait for their answers at once.
pub const MAX_WAITING: usize = 16;
/// How long a command's answer is waited for, as on the control socket.
const ANSWER_LIMIT: Duration = Duration::from_secs(75);
/// How long a client may take to read what it is sent.
const WRITE_LIMIT: Duration = Duration::from_secs(10);
/// How often a quiet stream is written to, which is also how soon a page
/// that is gone, or a session that has ended, is noticed.
const KEEPALIVE_EVERY: Duration = Duration::from_secs(15);
/// How long answers still owed may hold up the bot's exit.
const STOP_LIMIT: Duration = Duration::from_secs(2);

const PAGE: &str = include_str!("assets/index.html");
const SCRIPT: &str = include_str!("assets/app.js");
const STYLE: &str = include_str!("assets/style.css");

struct Shared {
    auth: Auth,
    events: Sender<Event>,
    /// Connection threads that are running.
    connections: AtomicUsize,
    streams: AtomicUsize,
    /// Commands whose answers are waited for.
    waiting: AtomicUsize,
    stopping: AtomicBool,
    /// The tags the page's three files are cached by.
    tags: [String; 3],
    limits: Limits,
    #[cfg(test)]
    logged: std::sync::Mutex<Vec<String>>,
}

impl Shared {
    /// A line for the operator log.
    fn log(&self, line: String) {
        crate::info!("{line}");
        #[cfg(test)]
        self.logged.lock().unwrap().push(line);
    }
}

/// The two waits that tests shorten.
#[derive(Clone, Copy)]
struct Limits {
    /// How long a client has to send its request.
    request: Duration,
    /// How often a quiet stream is written to.
    keepalive: Duration,
}

/// The listening web interface. The listener thread lives until the
/// process exits.
pub struct Server {
    shared: Arc<Shared>,
    #[cfg_attr(not(test), allow(dead_code))]
    address: SocketAddr,
}

/// A tag for one of the page's files: the bot's version and the file
/// itself, so that a new build is seen by the browsers that kept the old.
fn tag(file: &str) -> String {
    // FNV-1a: nothing depends on it but a cache.
    let hash = file.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("\"{}-{hash:016x}\"", env!("CARGO_PKG_VERSION"))
}

/// The line the operator log gets when the web interface starts.
pub fn start_line(address: SocketAddr, password: bool) -> String {
    let asked = match password {
        true => "a password is asked for",
        false => "no password is asked for",
    };
    let open = match !password && !address.ip().is_loopback() {
        true => format!(
            "; WARNING: anyone who can reach port {} of this machine controls the avatar: \
             set `password` in the `[web]` table, or listen on 127.0.0.1",
            address.port()
        ),
        false => String::new(),
    };
    format!("web: listening on http://{address}/, {asked}{open}")
}

/// Starts listening on the address of `config`. Commands and pages
/// arrive on `events` as `Event::Web`.
pub fn listen(config: &WebConfig, events: Sender<Event>) -> Result<Server, String> {
    let limits = Limits {
        request: http::REQUEST_LIMIT,
        keepalive: KEEPALIVE_EVERY,
    };
    listen_with(config, events, limits)
}

fn listen_with(config: &WebConfig, events: Sender<Event>, limits: Limits) -> Result<Server, String> {
    let listener = TcpListener::bind(config.listen).map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
    let address = listener
        .local_addr()
        .map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
    let shared = Arc::new(Shared {
        auth: Auth::new(config.password.clone(), address.port()),
        events,
        connections: AtomicUsize::new(0),
        streams: AtomicUsize::new(0),
        waiting: AtomicUsize::new(0),
        stopping: AtomicBool::new(false),
        tags: [tag(PAGE), tag(SCRIPT), tag(STYLE)],
        limits,
        #[cfg(test)]
        logged: std::sync::Mutex::new(Vec::new()),
    });
    shared.log(start_line(address, config.password.is_some()));
    let accepting = shared.clone();
    thread::spawn(move || accept(&listener, &accepting));
    Ok(Server { shared, address })
}

impl Server {
    /// The bot is done: no new request is served, and the answers still
    /// owed get a moment to be written. The pages' streams end when the
    /// hub is closed, which is the main loop's to do first.
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + STOP_LIMIT;
        while self.shared.connections.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>) {
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                shared.log(format!("web: cannot accept a connection: {e}"));
                // Whatever is wrong, do not spin on it.
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        if shared.stopping.load(Ordering::SeqCst) {
            continue;
        }
        if shared.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
            // A few bytes into the socket's buffer: this does not wait.
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            let busy = http::Response::text(503, "too many connections to the web interface; try again\n");
            let _ = stream.write_all(&busy.encode());
            let _ = stream.shutdown(Shutdown::Write);
            continue;
        }
        let shared = shared.clone();
        thread::spawn(move || {
            serve(stream, &shared);
            shared.connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// Writes `response` and ends the connection. `more_coming` when the
/// client may still be sending what was refused: it is heard out briefly,
/// so that the answer is not lost to a reset.
fn send(stream: &mut TcpStream, response: &http::Response, more_coming: bool) {
    let _ = stream.set_write_timeout(Some(WRITE_LIMIT));
    if stream.write_all(&response.encode()).is_err() {
        return;
    }
    let _ = stream.shutdown(Shutdown::Write);
    if more_coming {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut rest = [0; 4096];
        let mut left = 16 * http::MAX_BODY;
        while let Ok(count @ 1..) = stream.read(&mut rest) {
            left = left.saturating_sub(count);
            if left == 0 {
                break;
            }
        }
    }
}

/// What a request comes to.
enum Outcome {
    Answer(http::Response),
    /// The event stream, for a page that got `last` as its last event and
    /// has the session `token`.
    Stream {
        last: Option<String>,
        token: Option<String>,
    },
}

/// Handles one connection: one request in, one answer out.
fn serve(mut stream: TcpStream, shared: &Arc<Shared>) {
    let peer = stream.peer_addr().map_or_else(|_| "?".to_string(), |peer| peer.ip().to_string());
    let deadline = Instant::now() + shared.limits.request;
    let request = http::read_request(&mut stream, |stream| {
        let left = deadline.saturating_duration_since(Instant::now());
        !left.is_zero() && stream.set_read_timeout(Some(left)).is_ok()
    });
    let request = match request {
        Ok(request) => request,
        Err(Bad::Gone) => return,
        Err(Bad::Refused(status, why)) => return send(&mut stream, &http::Response::text(status, format!("{why}\n")), true),
    };
    match handle(&request, &peer, shared) {
        Outcome::Answer(response) => send(&mut stream, &response, false),
        Outcome::Stream { last, token } => events(stream, last, token, shared),
    }
}

/// A piece of a request for the operator log: short, and on one line.
fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(100).collect()
}

fn handle(request: &http::Request, peer: &str, shared: &Arc<Shared>) -> Outcome {
    let answer = Outcome::Answer;
    let host = request.header("host");
    // Without a password, a name someone made to point at this address is
    // not answered: a page of theirs would be the bot's own page to the
    // browser.
    if !shared.auth.has_password() && !http::host_is_address(host) {
        shared.log(format!(
            "web: refused a request from {peer} for \"{}\": without a password the web interface answers only to an IP address or localhost",
            shown(host.unwrap_or_default())
        ));
        return answer(http::Response::text(403, http::HOST_REFUSED));
    }
    let route = match http::route(request.method, &request.path, request.query.is_some()) {
        Ok(route) => route,
        Err(status) => return answer(http::Response::status(status)),
    };
    // What changes anything comes from the page itself: its script marks
    // it, which a form or a script of another site cannot.
    if request.method == Method::Post && !request.marked(http::MARKER) {
        shared.log(format!(
            "web: refused a request from {peer} for \"{}{}\": it was not sent by the page of the web interface",
            shown(host.unwrap_or_default()),
            shown(&request.path)
        ));
        return answer(http::Response::text(
            403,
            "this request was not sent by the page of the web interface\n",
        ));
    }
    let now = Instant::now();
    let token = shared.auth.token(request.header("cookie"));
    let allowed = shared.auth.allows(token, now);
    let etag = request.header("if-none-match");
    match route {
        Route::Page => answer(http::Response::file("text/html; charset=utf-8", PAGE, &shared.tags[0], etag)),
        Route::Script => answer(http::Response::file(
            "text/javascript; charset=utf-8",
            SCRIPT,
            &shared.tags[1],
            etag,
        )),
        Route::Style => answer(http::Response::file("text/css; charset=utf-8", STYLE, &shared.tags[2], etag)),
        Route::Session => answer(http::Response::text(
            200,
            match (shared.auth.has_password(), allowed) {
                (false, _) => "open\n",
                (true, true) => "in\n",
                (true, false) => "login\n",
            },
        )),
        Route::Login => answer(login(request, peer, shared, now)),
        Route::Logout => {
            if token.is_some_and(|token| shared.auth.logout(token)) {
                shared.log(format!("web: logout from {peer}"));
            }
            answer(http::Response::empty(204).with("Set-Cookie", shared.auth.clear_cookie()))
        }
        _ if !allowed => answer(http::Response::text(401, "log in first\n")),
        Route::Command | Route::Console => answer(command(route, request, shared)),
        Route::Events => Outcome::Stream {
            last: request.header("last-event-id").or(request.parameter("last")).map(str::to_string),
            token: token.map(str::to_string),
        },
    }
}

fn login(request: &http::Request, peer: &str, shared: &Arc<Shared>, now: Instant) -> http::Response {
    if !shared.auth.has_password() {
        return http::Response::text(400, "this web interface asks for no password\n");
    }
    let given = String::from_utf8_lossy(&request.body);
    match shared.auth.login(&given, now) {
        Login::Session(token) => {
            shared.log(format!("web: login from {peer}"));
            http::Response::empty(204).with("Set-Cookie", shared.auth.set_cookie(&token))
        }
        Login::Wrong => {
            shared.log(format!("web: wrong password from {peer}"));
            http::Response::text(403, "wrong password\n")
        }
        Login::TooMany { first } => {
            if first {
                shared.log(format!(
                    "web: too many wrong passwords: logins are refused until {} s after the last of them",
                    auth::WRONG_WINDOW.as_secs()
                ));
            }
            http::Response::text(429, "too many attempts; try again in a minute\n")
        }
        Login::Failed(reason) => {
            shared.log(format!("web: {reason}"));
            http::Response::text(500, "the bot could not make a session\n")
        }
    }
}

/// A console line: the command it is, or its answer when there is
/// nothing to carry out. Read as the XMPP control contact reads a line.
fn console(line: &str, shared: &Shared) -> Result<Request, Response> {
    let args = control::split(line).map_err(Response::Error)?;
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => return Err(Response::Error("there is no command; send `help` for the list".to_string())),
        ["help"] => return Err(Response::Ok(Some(control::help()))),
        _ => {}
    }
    Request::decode(control::request_line(&args).as_bytes()).map_err(|reason| {
        shared.log(format!("web: bad command: {reason}"));
        Response::Error(reason)
    })
}

/// Carries out a command of the page and gives its answer, as the line
/// the control socket would write.
fn command(route: Route, request: &http::Request, shared: &Arc<Shared>) -> http::Response {
    let answer = |response: Response| http::Response::text(200, response.encode());
    let decoded = match (route, std::str::from_utf8(&request.body)) {
        (_, Err(_)) => Err(Response::Error("the command is not valid UTF-8".to_string())),
        (Route::Console, Ok(line)) => console(line, shared),
        (_, Ok(line)) => Request::decode(line.as_bytes()).map_err(|reason| {
            shared.log(format!("web: bad command: {reason}"));
            Response::Error(reason)
        }),
    };
    let command = match decoded {
        Ok(command) => command,
        Err(response) => return answer(response),
    };
    let own = request.marked(http::OWN_MARKER);
    if own && !http::may_be_own(&command) {
        let reason = format!(
            "`{}` cannot be sent as the page's own; only `scan avatars`, `friends`, `payees`, `group list` and `group invitations` can",
            command.name()
        );
        shared.log(format!("web: bad command: {reason}"));
        return answer(Response::Error(reason));
    }
    if shared.waiting.fetch_add(1, Ordering::SeqCst) >= MAX_WAITING {
        shared.waiting.fetch_sub(1, Ordering::SeqCst);
        let reason = format!("the bot is busy: {MAX_WAITING} commands wait for their answers; try again");
        return answer(Response::Error(reason));
    }
    let (reply, answered) = mpsc::channel();
    let event = Event::Web(WebEvent::Command {
        request: command,
        reply,
        own,
    });
    let response = match shared.events.send(event) {
        Ok(()) => answered
            .recv_timeout(ANSWER_LIMIT)
            .unwrap_or_else(|_| Response::Error("the bot did not answer".to_string())),
        Err(_) => Response::Error("the bot is shutting down".to_string()),
    };
    shared.waiting.fetch_sub(1, Ordering::SeqCst);
    answer(response)
}

/// Serves a page's event stream until the page is gone, its session has
/// ended, it does not keep up, or the bot stops.
fn events(mut stream: TcpStream, last: Option<String>, token: Option<String>, shared: &Arc<Shared>) {
    if shared.streams.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
        shared.streams.fetch_sub(1, Ordering::SeqCst);
        return send(&mut stream, &http::Response::text(503, "too many open pages; try again\n"), false);
    }
    let (page, queue) = mpsc::sync_channel(hub::QUEUE);
    let subscribed = shared.events.send(Event::Web(WebEvent::Subscribe { last, page })).is_ok();
    let _ = stream.set_write_timeout(Some(WRITE_LIMIT));
    let mut alive = subscribed && stream.write_all(&http::stream_head()).is_ok();
    while alive {
        let written = match queue.recv_timeout(shared.limits.keepalive) {
            Ok(event) => stream.write_all(event.as_bytes()),
            Err(RecvTimeoutError::Timeout) => stream.write_all(hub::KEEPALIVE.as_bytes()),
            // The hub let the page go: it did not keep up, or the bot stops.
            Err(RecvTimeoutError::Disconnected) => break,
        };
        alive = written.is_ok() && stream.flush().is_ok() && shared.auth.allows(token.as_deref(), Instant::now());
    }
    shared.streams.fetch_sub(1, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) mod tests;
