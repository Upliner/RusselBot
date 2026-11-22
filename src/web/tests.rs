//! The web interface over real connections, with a stand-in for the main
//! loop that answers a few commands and keeps a hub.

use super::*;
use crate::config::Secret;
use crate::msglog::{Channel, Record};
use std::io::{BufRead, BufReader};
use std::sync::mpsc::Receiver;

const START: u64 = 1_700_000_000;
const PASSWORD: &str = "Open-Sesame-17";

fn chat(text: &str) -> Record {
    Record {
        channel: Channel::Chat,
        time: START + 5,
        kind: "say".to_string(),
        avatar_name: "Sit Bot".to_string(),
        display_name: String::new(),
        object_name: String::new(),
        object_uuid: None,
        text: text.to_string(),
        avatar_uuid: None,
        offline: false,
    }
}

/// What the main loop does with the web interface's events, reduced to
/// what the tests need: `say` is recorded as chat the avatar heard, `sit`
/// is never answered, `scan` and `friends` say whether they were the
/// page's own, and everything else is refused.
fn main_loop(incoming: &Receiver<Event>) {
    let mut hub = hub::Hub::new(START);
    let mut held = Vec::new();
    for event in incoming {
        match event {
            Event::Web(WebEvent::Command { request, reply, own }) => {
                let response = match request {
                    Request::Say(text) => {
                        hub.record(&chat(&text));
                        Response::Ok(None)
                    }
                    Request::Status => Response::Ok(Some("connected: no\nmode: holding".to_string())),
                    Request::Sit(_) => {
                        held.push(reply);
                        continue;
                    }
                    Request::Friends => Response::Ok(Some(format!("friends own: {own}"))),
                    Request::Scan(args) => Response::Ok(Some(format!("scan {} own: {own}", args.join(" ")))),
                    Request::Quit => {
                        hub.close();
                        Response::Ok(None)
                    }
                    other => Response::Error(format!("refused: {}", other.name())),
                };
                let _ = reply.send(response);
            }
            Event::Web(WebEvent::Subscribe { last, page }) => hub.subscribe(last.as_deref(), page, "connected: no"),
            _ => {}
        }
    }
}

const QUICK: Limits = Limits {
    request: Duration::from_millis(400),
    keepalive: Duration::from_millis(100),
};

fn serving_with(password: Option<&str>, limits: Limits) -> Server {
    let config = WebConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        password: password.map(Secret::new),
    };
    let (events, incoming) = mpsc::channel();
    let server = listen_with(&config, events, limits).unwrap();
    thread::spawn(move || main_loop(&incoming));
    server
}

/// The web interface of `config` with the waits shortened, for the
/// tests of the bot.
pub(crate) fn listen_quick(config: &WebConfig, events: Sender<Event>) -> Server {
    listen_with(config, events, QUICK).unwrap()
}

fn serving(password: Option<&str>) -> Server {
    serving_with(password, QUICK)
}

/// Sends `raw` to `address` and reads the whole answer.
pub(crate) fn exchange_with(address: SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer).into_owned()
}

/// A `POST` as the page sends one, with its marker.
pub(crate) fn post_to(address: SocketAddr, path: &str, headers: &[&str], body: &str) -> String {
    let headers: String = headers.iter().map(|header| format!("{header}\r\n")).collect();
    exchange_with(
        address,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: {address}\r\nX-Rslbot: 1\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
}

impl Server {
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn logged(&self) -> Vec<String> {
        self.shared.logged.lock().unwrap().clone()
    }

    /// Sends `raw` and reads the whole answer.
    pub(crate) fn exchange(&self, raw: &str) -> String {
        exchange_with(self.address, raw)
    }

    pub(crate) fn get(&self, path: &str, headers: &[&str]) -> String {
        let headers: String = headers.iter().map(|header| format!("{header}\r\n")).collect();
        self.exchange(&format!("GET {path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n", self.address))
    }

    /// A `POST` as the page sends one, with its marker.
    pub(crate) fn post(&self, path: &str, headers: &[&str], body: &str) -> String {
        post_to(self.address, path, headers, body)
    }

    /// Logs in and gives the `Cookie` header of the session.
    fn login(&self) -> String {
        let answer = self.post("/api/login", &[], PASSWORD);
        assert_eq!(status(&answer), 204, "{answer}");
        let cookie = header(&answer, "Set-Cookie").expect("a cookie");
        format!("Cookie: {}", cookie.split(';').next().unwrap())
    }

    /// Opens an event stream and gives it once its head has been read.
    pub(crate) fn stream(&self, path: &str, headers: &[&str]) -> Stream {
        let headers: String = headers.iter().map(|header| format!("{header}\r\n")).collect();
        let mut stream = TcpStream::connect(self.address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let request = format!("GET {path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n", self.address);
        stream.write_all(request.as_bytes()).unwrap();
        let mut stream = Stream(BufReader::new(stream));
        let head = stream.block();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.contains("Content-Type: text/event-stream\r\n"), "{head}");
        stream
    }
}

pub(crate) struct Stream(BufReader<TcpStream>);

impl Stream {
    /// The lines up to the next empty one; empty when the stream ended.
    pub(crate) fn block(&mut self) -> String {
        let mut block = String::new();
        loop {
            let mut line = String::new();
            match self.0.read_line(&mut line) {
                Ok(0) | Err(_) => return block,
                Ok(_) if line == "\n" || line == "\r\n" => return block,
                Ok(_) => block.push_str(&line),
            }
        }
    }

    /// The next event that is not a keepalive comment.
    pub(crate) fn event(&mut self) -> String {
        loop {
            let block = self.block();
            if block != ": \n" {
                return block;
            }
        }
    }

    /// The text of the next chat line.
    pub(crate) fn chat(&mut self) -> String {
        let event = self.event();
        assert!(event.starts_with("event: chat\n"), "{event}");
        event.trim_end().rsplit('\t').next().unwrap().to_string()
    }

    /// The text of the next message, of either channel.
    pub(crate) fn chat_or_im(&mut self) -> String {
        let event = self.event();
        assert!(event.starts_with("event: chat\n") || event.starts_with("event: im\n"), "{event}");
        event.trim_end().rsplit('\t').next().unwrap().to_string()
    }

    /// Reads what a page is sent when it opens: the word to start over,
    /// the kept messages and the status. Gives the kept messages' texts.
    pub(crate) fn opening(&mut self) -> Vec<String> {
        assert_eq!(self.event(), "event: reset\ndata: -\n");
        let mut texts = Vec::new();
        loop {
            let event = self.event();
            assert!(!event.is_empty(), "the stream ended before the status");
            if event.starts_with("event: status\n") {
                assert!(event.starts_with("event: status\ndata: connected: no"), "{event}");
                return texts;
            }
            texts.push(event.trim_end().rsplit('\t').next().unwrap().to_string());
        }
    }
}

pub(crate) fn status(answer: &str) -> u16 {
    answer.split(' ').nth(1).and_then(|code| code.parse().ok()).unwrap_or(0)
}

fn header<'a>(answer: &'a str, name: &str) -> Option<&'a str> {
    let head = answer.split("\r\n\r\n").next()?;
    head.lines()
        .filter_map(|line| line.split_once(": "))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

pub(crate) fn body(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

// ------------------------------------------------------------- lifetime

#[test]
fn the_start_is_logged_with_the_address_and_the_password() {
    let line = start_line("127.0.0.1:8080".parse().unwrap(), true);
    assert_eq!(line, "web: listening on http://127.0.0.1:8080/, a password is asked for");
    let line = start_line("127.0.0.1:8080".parse().unwrap(), false);
    assert_eq!(line, "web: listening on http://127.0.0.1:8080/, no password is asked for");
    // Open to the network without a password: said, and started all the same.
    for address in ["0.0.0.0:8080", "192.168.1.5:8080", "[::]:8080"] {
        let line = start_line(address.parse().unwrap(), false);
        assert!(
            line.contains("anyone who can reach port 8080 of this machine controls the avatar"),
            "{line}"
        );
        assert!(!line.contains('\n'));
        assert!(!start_line(address.parse().unwrap(), true).contains("anyone"));
    }
    assert!(!start_line("[::1]:8080".parse().unwrap(), false).contains("anyone"));
    let server = serving(Some(PASSWORD));
    assert_eq!(
        server.logged(),
        [format!("web: listening on http://{}/, a password is asked for", server.address)]
    );
}

#[test]
fn an_address_in_use_is_an_error_that_names_it() {
    let server = serving(None);
    let config = WebConfig {
        listen: server.address,
        password: None,
    };
    let (events, _incoming) = mpsc::channel();
    let error = listen(&config, events).err().expect("the address is taken");
    assert!(error.contains(&server.address.to_string()), "{error}");
    // The first keeps serving.
    assert_eq!(status(&server.get("/api/session", &[])), 200);
}

#[test]
fn a_silent_client_is_dropped_and_others_are_served() {
    let server = serving(None);
    let mut silent = TcpStream::connect(server.address).unwrap();
    let mut partial = TcpStream::connect(server.address).unwrap();
    partial.write_all(b"GET / HTTP/1.1\r\nHost: half a he").unwrap();
    // Others are served while these two say nothing.
    assert_eq!(status(&server.get("/api/session", &[])), 200);
    let started = Instant::now();
    let mut rest = Vec::new();
    for stream in [&mut silent, &mut partial] {
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(stream.read_to_end(&mut rest).unwrap_or(0), 0);
    }
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert!(rest.is_empty());
}

#[test]
fn a_further_connection_is_answered_as_busy() {
    let limits = Limits {
        request: Duration::from_secs(20),
        ..QUICK
    };
    let server = serving_with(None, limits);
    let idle: Vec<TcpStream> = (0..MAX_CONNECTIONS).map(|_| TcpStream::connect(server.address).unwrap()).collect();
    // The accept thread has to have taken them all.
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.shared.connections.load(Ordering::SeqCst) < MAX_CONNECTIONS && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let answer = server.get("/api/session", &[]);
    assert_eq!(status(&answer), 503, "{answer}");
    assert!(body(&answer).contains("too many connections"), "{answer}");
    drop(idle);
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.shared.connections.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(body(&server.get("/api/session", &[])), "open\n");
}

#[test]
fn an_oversized_request_is_refused_and_nothing_is_carried_out() {
    let server = serving(None);
    let answer = server.post("/api/command", &[], &format!("say\t{}", "x".repeat(100_000)));
    assert_eq!(status(&answer), 413, "{answer}");
    let long_head = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nX-Filler: {}\r\n\r\n",
        server.address,
        "x".repeat(9000)
    );
    assert_eq!(status(&server.exchange(&long_head)), 431);
    // Nothing was said: a page that opens has no chat.
    assert_eq!(server.stream("/api/events", &[]).opening(), [] as [&str; 0]);
}

#[test]
fn stopping_does_not_wait_long_for_an_answer_still_owed() {
    let server = serving(None);
    let started = Instant::now();
    server.stop();
    assert!(started.elapsed() < Duration::from_millis(500));
    // A command that is never answered holds its connection.
    let server = serving(None);
    let address = server.address;
    thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        let _ = stream.write_all(
            format!("POST /api/command HTTP/1.1\r\nHost: {address}\r\nX-Rslbot: 1\r\nContent-Length: 10\r\n\r\nsit\tground").as_bytes(),
        );
        let _ = stream.read(&mut [0; 16]);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.shared.waiting.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    server.stop();
    let took = started.elapsed();
    assert!(took >= STOP_LIMIT && took < STOP_LIMIT + Duration::from_secs(1), "{took:?}");
}

#[test]
fn the_answer_to_quit_reaches_the_page_before_the_bot_is_gone() {
    let server = serving(None);
    let mut page = server.stream("/api/events", &[]);
    page.opening();
    let answer = server.post("/api/command", &[], "quit");
    assert_eq!(body(&answer), "ok\n");
    // The hub was closed: the stream ends, and stopping has nothing to
    // wait for.
    assert_eq!(page.event(), "");
    let started = Instant::now();
    server.stop();
    assert!(started.elapsed() < Duration::from_secs(1));
}

// ---------------------------------------------------------------- files

#[test]
fn the_page_is_served_from_the_binary_and_cached_by_its_tag() {
    let server = serving(None);
    for (path, file, kind) in [
        ("/", PAGE, "text/html; charset=utf-8"),
        ("/app.js", SCRIPT, "text/javascript; charset=utf-8"),
        ("/style.css", STYLE, "text/css; charset=utf-8"),
    ] {
        let answer = server.get(path, &[]);
        assert_eq!(status(&answer), 200, "{path}");
        assert_eq!(body(&answer), file, "{path}");
        assert_eq!(header(&answer, "Content-Type"), Some(kind));
        assert_eq!(header(&answer, "Cache-Control"), Some("no-cache"));
        let tag = header(&answer, "ETag").expect("a tag").to_string();
        assert!(tag.contains(env!("CARGO_PKG_VERSION")), "{tag}");
        let again = server.get(path, &[&format!("If-None-Match: {tag}")]);
        assert_eq!(status(&again), 304, "{again}");
        assert_eq!(body(&again), "");
    }
    for path in ["/russelbot.toml", "/../etc/passwd", "/index.html", "/app.js?v=2"] {
        let answer = server.get(path, &[]);
        assert_eq!(status(&answer), 404, "{path}");
        assert_eq!(body(&answer), "Not Found\n");
    }
    assert_eq!(
        status(&server.exchange(&format!("OPTIONS /api/command HTTP/1.1\r\nHost: {}\r\n\r\n", server.address))),
        405
    );
    assert_eq!(
        status(&server.exchange(&format!("HEAD / HTTP/1.1\r\nHost: {}\r\n\r\n", server.address))),
        405
    );
    // None of this is worth a line in the log.
    assert_eq!(server.logged().len(), 1, "{:?}", server.logged());
}

// ------------------------------------------------------------- password

#[test]
fn without_a_session_nothing_of_the_bot_is_served() {
    let server = serving(Some(PASSWORD));
    // What the login box needs.
    assert_eq!(status(&server.get("/", &[])), 200);
    assert_eq!(status(&server.get("/app.js", &[])), 200);
    assert_eq!(status(&server.get("/style.css", &[])), 200);
    assert_eq!(body(&server.get("/api/session", &[])), "login\n");
    // And nothing else.
    for path in ["/api/command", "/api/console"] {
        let answer = server.post(path, &[], "say\thello");
        assert_eq!(status(&answer), 401, "{answer}");
    }
    assert_eq!(status(&server.get("/api/events", &[])), 401);
    let forged = format!("Cookie: rslbot_{}={}", server.address.port(), "ab".repeat(32));
    assert_eq!(status(&server.post("/api/command", &[&forged], "say\thello")), 401);
    // Nothing was said.
    let cookie = server.login();
    assert_eq!(server.stream("/api/events", &[&cookie]).opening(), [] as [&str; 0]);
}

#[test]
fn the_right_password_gives_a_session_that_stays() {
    let server = serving(Some(PASSWORD));
    let answer = server.post("/api/login", &[], PASSWORD);
    assert_eq!(status(&answer), 204, "{answer}");
    let cookie = header(&answer, "Set-Cookie").unwrap();
    assert!(cookie.starts_with(&format!("rslbot_{}=", server.address.port())), "{cookie}");
    assert!(cookie.ends_with("; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000"), "{cookie}");
    let cookie = server.login();
    assert_eq!(body(&server.get("/api/session", &[&cookie])), "in\n");
    assert_eq!(
        body(&server.post("/api/command", &[&cookie], "status")),
        "ok\tconnected: no\\nmode: holding\n"
    );
    assert_eq!(body(&server.get("/api/session", &[&cookie])), "in\n");
    assert_eq!(server.logged()[1..], ["web: login from 127.0.0.1", "web: login from 127.0.0.1"]);
}

#[test]
fn logging_out_ends_one_browsers_session() {
    let server = serving(Some(PASSWORD));
    let (phone, desktop) = (server.login(), server.login());
    let answer = server.post("/api/logout", &[&phone], "");
    assert_eq!(status(&answer), 204);
    assert!(header(&answer, "Set-Cookie").unwrap().ends_with("Max-Age=0"));
    assert_eq!(body(&server.get("/api/session", &[&phone])), "login\n");
    assert_eq!(status(&server.post("/api/command", &[&phone], "status")), 401);
    assert_eq!(body(&server.get("/api/session", &[&desktop])), "in\n");
    assert_eq!(server.logged().last().unwrap(), "web: logout from 127.0.0.1");
    // A logout without a session is not worth a line.
    let lines = server.logged().len();
    assert_eq!(status(&server.post("/api/logout", &[], "")), 204);
    assert_eq!(server.logged().len(), lines);
}

#[test]
fn without_a_password_everything_is_served_at_once() {
    let server = serving(None);
    assert_eq!(body(&server.get("/api/session", &[])), "open\n");
    assert_eq!(
        body(&server.post("/api/command", &[], "status")),
        "ok\tconnected: no\\nmode: holding\n"
    );
    assert_eq!(status(&server.post("/api/login", &[], "anything")), 400);
    assert_eq!(server.logged().len(), 1);
}

#[test]
fn a_wrong_password_is_logged_without_any_password() {
    let server = serving(Some(PASSWORD));
    let cookie = server.login();
    let answer = server.post("/api/login", &[], "Let-Me-In-99");
    assert_eq!(status(&answer), 403);
    assert_eq!(body(&answer), "wrong password\n");
    assert_eq!(header(&answer, "Set-Cookie"), None);
    assert_eq!(server.logged().last().unwrap(), "web: wrong password from 127.0.0.1");
    // Guessing: the sixth is refused unlooked at, and so is the right one.
    for _ in 0..4 {
        assert_eq!(status(&server.post("/api/login", &[], "Let-Me-In-99")), 403);
    }
    for given in ["Let-Me-In-99", PASSWORD, PASSWORD] {
        let answer = server.post("/api/login", &[], given);
        assert_eq!(status(&answer), 429, "{answer}");
        assert!(body(&answer).contains("too many attempts"));
    }
    // Said once, and the session that was there keeps working.
    let logged = server.logged();
    assert_eq!(
        logged.iter().filter(|line| line.contains("too many wrong passwords")).count(),
        1,
        "{logged:?}"
    );
    assert_eq!(body(&server.get("/api/session", &[&cookie])), "in\n");
    for line in &logged {
        assert!(!line.contains(PASSWORD) && !line.contains("Let-Me-In-99"), "{line}");
    }
    // Nothing the bot sends holds the password either.
    for answer in [
        server.get("/", &[]),
        server.get("/api/session", &[&cookie]),
        server.post("/api/command", &[&cookie], "status"),
    ] {
        assert!(!answer.contains(PASSWORD), "{answer}");
    }
}

// ----------------------------------------------------------- other sites

#[test]
fn what_another_sites_page_makes_a_browser_send_is_not_carried_out() {
    for password in [None, Some(PASSWORD)] {
        let server = serving(password);
        let cookie = password.map(|_| server.login()).unwrap_or_default();
        // A form on another site: the browser's cookie or none, but never
        // the page's marker.
        let form = format!(
            "POST /api/command HTTP/1.1\r\nHost: {}\r\nOrigin: https://elsewhere.example\r\n{}Content-Type: text/plain\r\nContent-Length: 9\r\n\r\nsay\thello",
            server.address,
            if cookie.is_empty() { String::new() } else { format!("{cookie}\r\n") }
        );
        let answer = server.exchange(&form);
        assert_eq!(status(&answer), 403, "{answer}");
        for path in ["/api/console", "/api/login", "/api/logout"] {
            let bare = format!("POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: 5\r\n\r\nhello", server.address);
            assert_eq!(status(&server.exchange(&bare)), 403, "{path}");
        }
        // A script there asks first, and is not told it may.
        let preflight = format!(
            "OPTIONS /api/command HTTP/1.1\r\nHost: {}\r\nOrigin: https://elsewhere.example\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: x-rslbot\r\n\r\n",
            server.address
        );
        let answer = server.exchange(&preflight);
        assert_eq!(status(&answer), 405);
        assert!(!answer.to_lowercase().contains("access-control"), "{answer}");
        // Nor is anything it could read marked as readable by it.
        let headers: Vec<&str> = if cookie.is_empty() { vec![] } else { vec![&cookie] };
        let mut page = server.stream("/api/events", &headers);
        // Nothing was said.
        assert_eq!(page.opening(), [] as [&str; 0]);
        let logged = server.logged();
        let refused: Vec<&String> = logged.iter().filter(|line| line.contains("refused a request")).collect();
        assert_eq!(refused.len(), 4, "{logged:?}");
        assert!(
            refused[0].contains("from 127.0.0.1") && refused[0].contains(&format!("{}/api/command", server.address)),
            "{}",
            refused[0]
        );
        // The session is untouched by the refused logout.
        if password.is_some() {
            assert_eq!(body(&server.get("/api/session", &[&cookie])), "in\n");
        }
    }
}

#[test]
fn without_a_password_only_an_address_is_answered() {
    let server = serving(None);
    let port = server.address.port();
    for host in [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        "192.168.1.5".to_string(),
        format!("[::1]:{port}"),
    ] {
        let answer = server.exchange(&format!("GET /api/session HTTP/1.1\r\nHost: {host}\r\n\r\n"));
        assert_eq!(status(&answer), 200, "{host}");
    }
    // A name made to resolve to this address.
    let rebound = format!("bot.attacker.example:{port}");
    for request in [
        format!("GET / HTTP/1.1\r\nHost: {rebound}\r\n\r\n"),
        format!("GET /api/events HTTP/1.1\r\nHost: {rebound}\r\n\r\n"),
        format!("POST /api/command HTTP/1.1\r\nHost: {rebound}\r\nX-Rslbot: 1\r\nContent-Length: 9\r\n\r\nsay\thello"),
        "GET / HTTP/1.0\r\n\r\n".to_string(),
    ] {
        let answer = server.exchange(&request);
        assert_eq!(status(&answer), 403, "{request}");
        assert!(
            body(&answer).contains("IP address") && body(&answer).contains("`password`"),
            "{answer}"
        );
    }
    assert_eq!(server.stream("/api/events", &[]).opening(), [] as [&str; 0]);
    let logged = server.logged();
    assert!(
        logged[1].contains("from 127.0.0.1") && logged[1].contains("bot.attacker.example"),
        "{logged:?}"
    );
    // With a password the bot answers under any name: a page under that
    // name has no session.
    let server = serving(Some(PASSWORD));
    let answer = server.exchange("GET /api/session HTTP/1.1\r\nHost: bot.example.org\r\n\r\n");
    assert_eq!(body(&answer), "login\n");
}

// -------------------------------------------------------------- commands

#[test]
fn commands_are_answered_as_the_control_socket_answers() {
    let server = serving(None);
    let command = |line: &str| body(&server.post("/api/command", &[], line)).to_string();
    assert_eq!(command("status"), "ok\tconnected: no\\nmode: holding\n");
    assert_eq!(command("say\thello there"), "ok\n");
    assert_eq!(command("stand"), "error\trefused: stand\n");
    // Arguments are fields: no quoting, whatever they hold.
    assert_eq!(
        command("scan\t20\t-n\tJoe's \"Blue\" Lagoon"),
        "ok\tscan 20 -n Joe's \"Blue\" Lagoon own: false\n"
    );
    // What the control socket cannot read is answered and goes nowhere.
    for (line, part) in [("dance", "dance"), ("im\tjane", "im"), ("", "")] {
        let answer = command(line);
        assert!(answer.starts_with("error\t") && answer.contains(part), "{line}: {answer}");
    }
    // Bytes that are no text are answered the same way.
    let mut stream = TcpStream::connect(server.address).unwrap();
    let head = format!(
        "POST /api/command HTTP/1.1\r\nHost: {}\r\nX-Rslbot: 1\r\nContent-Length: 6\r\n\r\n",
        server.address
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(b"say\t\xff\xfe").unwrap();
    let mut invalid = String::new();
    let _ = stream.read_to_string(&mut invalid);
    assert_eq!(body(&invalid), "error\tthe command is not valid UTF-8\n");
    let logged = server.logged();
    assert!(
        logged
            .iter()
            .any(|line| line.starts_with("web: bad command: ") && line.contains("dance")),
        "{logged:?}"
    );
}

#[test]
fn console_lines_are_read_as_the_control_contact_reads_them() {
    let server = serving(None);
    let console = |line: &str| body(&server.post("/api/console", &[], line)).to_string();
    assert_eq!(console("status"), "ok\tconnected: no\\nmode: holding\n");
    assert_eq!(console("say a  b"), "ok\n");
    assert_eq!(console("scan 20 -n \"park bench\""), "ok\tscan 20 -n park bench own: false\n");
    let help = console("help");
    assert!(help.starts_with("ok\tCommands") && help.contains("\\nsay MESSAGE...\\n"), "{help}");
    assert!(!help.contains("rslbot ctl"), "{help}");
    let unclosed = console("im \"Jane Doe hi");
    assert!(unclosed.starts_with("error\t") && unclosed.contains("double quotes"), "{unclosed}");
    assert!(console("").contains("there is no command"));
    assert!(console("   ").contains("there is no command"));
    assert!(console("dance").contains("dance"));
    let long = console(&format!("say {}", "x".repeat(9000)));
    assert!(long.starts_with("error\t") && long.contains("8192"), "{long}");
    // What was said is what the lines said, joined as `rslbot ctl` joins.
    assert_eq!(server.stream("/api/events", &[]).opening(), ["a b"]);
}

#[test]
fn only_the_two_listings_are_taken_as_the_pages_own() {
    let server = serving(None);
    let own = |line: &str| body(&server.post("/api/command", &["X-Rslbot-Own: 1"], line)).to_string();
    assert_eq!(own("friends"), "ok\tfriends own: true\n");
    assert_eq!(own("scan\tavatars\tchat"), "ok\tscan avatars chat own: true\n");
    assert_eq!(
        body(&server.post("/api/command", &[], "scan\tavatars\tchat")),
        "ok\tscan avatars chat own: false\n"
    );
    for line in ["say\thello", "scan\t20", "status", "quit"] {
        let answer = own(line);
        assert!(
            answer.starts_with("error\t") && answer.contains("the page's own"),
            "{line}: {answer}"
        );
    }
    assert_eq!(server.stream("/api/events", &[]).opening(), [] as [&str; 0]);
}

#[test]
fn a_seventeenth_waiting_command_is_answered_as_busy() {
    let server = serving(None);
    let address = server.address;
    for _ in 0..MAX_WAITING {
        thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let request = format!("POST /api/command HTTP/1.1\r\nHost: {address}\r\nX-Rslbot: 1\r\nContent-Length: 10\r\n\r\nsit\tground");
            let _ = stream.write_all(request.as_bytes());
            let _ = stream.read(&mut [0; 16]);
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.shared.waiting.load(Ordering::SeqCst) < MAX_WAITING && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let answer = server.post("/api/command", &[], "sit\tground");
    assert!(body(&answer).starts_with("error\tthe bot is busy: 16 commands"), "{answer}");
    // Another page's stream and what needs no answer still work.
    assert_eq!(body(&server.get("/api/session", &[])), "open\n");
    assert_eq!(server.stream("/api/events", &[]).opening(), [] as [&str; 0]);
}

// ---------------------------------------------------------------- events

#[test]
fn every_open_page_gets_what_is_recorded_as_it_happens() {
    let server = serving(None);
    assert_eq!(body(&server.post("/api/command", &[], "say\tbefore")), "ok\n");
    let (mut one, mut two) = (server.stream("/api/events", &[]), server.stream("/api/events", &[]));
    assert_eq!(one.opening(), ["before"]);
    assert_eq!(two.opening(), ["before"]);
    assert_eq!(body(&server.post("/api/command", &[], "say\thi all")), "ok\n");
    let event = one.event();
    assert_eq!(
        event,
        format!("event: chat\nid: {START}-2\ndata: {}\tsay\t\tSit Bot\t\t\t\t0\thi all\n", START + 5)
    );
    assert_eq!(two.chat(), "hi all");
    // A quiet stream is kept alive with comments.
    assert_eq!(one.block(), ": \n");
}

#[test]
fn a_page_that_comes_back_gets_what_it_missed() {
    let server = serving(None);
    for text in ["one", "two", "three"] {
        server.post("/api/command", &[], &format!("say\t{text}"));
    }
    // By the header the browser sends by itself, and by the parameter the
    // page sends when it opens the stream anew.
    let last = format!("{START}-1");
    let mut by_header = server.stream("/api/events", &[&format!("Last-Event-ID: {last}")]);
    assert_eq!((by_header.chat(), by_header.chat()), ("two".to_string(), "three".to_string()));
    assert!(by_header.event().starts_with("event: status\n"));
    let mut by_parameter = server.stream(&format!("/api/events?last={last}"), &[]);
    assert_eq!((by_parameter.chat(), by_parameter.chat()), ("two".to_string(), "three".to_string()));
    // An id of another run starts the page over.
    let mut other = server.stream("/api/events?last=5-1", &[]);
    assert_eq!(other.opening(), ["one", "two", "three"]);
}

#[test]
fn a_stream_ends_when_its_session_does() {
    let server = serving(Some(PASSWORD));
    let (phone, desktop) = (server.login(), server.login());
    let (mut on_phone, mut on_desktop) = (server.stream("/api/events", &[&phone]), server.stream("/api/events", &[&desktop]));
    on_phone.opening();
    on_desktop.opening();
    server.post("/api/logout", &[&phone], "");
    let started = Instant::now();
    assert_eq!(on_phone.event(), "");
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    // The other browser's goes on.
    server.post("/api/command", &[&desktop], "say\tstill here");
    assert_eq!(on_desktop.chat(), "still here");
}

#[test]
fn a_ninth_stream_is_answered_as_busy() {
    let server = serving(None);
    let mut open: Vec<Stream> = (0..MAX_STREAMS).map(|_| server.stream("/api/events", &[])).collect();
    for stream in &mut open {
        stream.opening();
    }
    let answer = server.get("/api/events", &[]);
    assert_eq!(status(&answer), 503, "{answer}");
    drop(open);
    // Their threads notice with the next keepalive.
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.shared.streams.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    server.stream("/api/events", &[]).opening();
}

#[test]
fn a_browser_that_stops_reading_holds_nobody_up() {
    let server = serving(None);
    let mut stalled = server.stream("/api/events", &[]);
    stalled.opening();
    let mut reading = server.stream("/api/events", &[]);
    reading.opening();
    // More than the socket's buffers and the page's queue hold together.
    let filler = "x".repeat(8000);
    let lines = 700;
    let reader = thread::spawn(move || (0..lines).map(|_| reading.chat().len()).collect::<Vec<usize>>());
    for _ in 0..lines {
        assert_eq!(body(&server.post("/api/command", &[], &format!("say\t{filler}"))), "ok\n");
    }
    assert_eq!(reader.join().unwrap(), vec![8000; lines]);
    // The bot still answers.
    assert_eq!(
        body(&server.post("/api/command", &[], "status")),
        "ok\tconnected: no\\nmode: holding\n"
    );
    drop(stalled);
}
