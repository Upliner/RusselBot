//! The little of HTTP/1.1 the web interface speaks: one request per
//! connection, `GET` and `POST`, a body only with `Content-Length`, hard
//! limits on everything. No sockets here: a request is read from any
//! `Read` and a response is a byte vector, so that all of it can be tested.

use crate::control::Request as Command;
use std::io::{ErrorKind, Read};
use std::time::Duration;

/// The longest request head: request line, headers and the empty line.
pub const MAX_HEAD: usize = 8192;
/// The longest request body.
pub const MAX_BODY: usize = 16384;
/// How long a client has to send its whole request.
pub const REQUEST_LIMIT: Duration = Duration::from_secs(10);

/// The header every `POST` of the page carries. A form on another site
/// cannot set it, and a script there cannot without a preflight request,
/// which is never answered with permission.
pub const MARKER: &str = "x-rslbot";
/// The header that marks a command as one the page sent by itself.
pub const OWN_MARKER: &str = "x-rslbot-own";

const CSP: &str = "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Method {
    Get,
    Post,
    /// Anything else, `OPTIONS` and `HEAD` included: never served.
    Other,
}

/// A request as far as the web interface looks at one.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: Method,
    /// The path without its query.
    pub path: String,
    /// What follows the `?`, when there is one.
    pub query: Option<String>,
    /// Names in lower case, in the order they came.
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The value of the first header of this name, given in lower case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    /// Whether the header `name` is there and is `1`.
    pub fn marked(&self, name: &str) -> bool {
        self.header(name) == Some("1")
    }

    /// The value of the query parameter `name`, as it was written: the
    /// page puts nothing there that needs decoding.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        let pairs = self.query.as_deref()?.split('&');
        pairs
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }
}

/// Why a request was not read.
#[derive(Debug, Clone, PartialEq)]
pub enum Bad {
    /// The client hung up, went quiet or ran out of time: nothing to
    /// answer, the connection is closed.
    Gone,
    /// To be answered with this status and text.
    Refused(u16, &'static str),
}

/// Where the head ends: the index after its empty line.
fn head_end(bytes: &[u8]) -> Option<usize> {
    let crlf = bytes.windows(4).position(|w| w == b"\r\n\r\n").map(|at| at + 4);
    let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|at| at + 2);
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Reads one request. `ready` is asked before every read and says whether
/// there is time left; on a socket it also sets the read timeout to what
/// is left.
pub fn read_request<S: Read>(stream: &mut S, mut ready: impl FnMut(&mut S) -> bool) -> Result<Request, Bad> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 2048];
    let mut fill = |stream: &mut S, bytes: &mut Vec<u8>| -> Result<(), Bad> {
        loop {
            if !ready(stream) {
                return Err(Bad::Gone);
            }
            match stream.read(&mut chunk) {
                Ok(0) => return Err(Bad::Gone),
                Ok(count) => {
                    bytes.extend(&chunk[..count]);
                    return Ok(());
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Err(Bad::Gone),
            }
        }
    };
    let end = loop {
        match head_end(&bytes) {
            Some(end) if end > MAX_HEAD => return Err(Bad::Refused(431, "the request head is too long")),
            Some(end) => break end,
            None if bytes.len() > MAX_HEAD => return Err(Bad::Refused(431, "the request head is too long")),
            None => fill(stream, &mut bytes)?,
        }
    };
    let mut request = parse_head(&bytes[..end])?;
    let length = match request.headers.iter().filter(|(key, _)| key == "content-length").count() {
        0 => 0,
        1 => {
            let given = request.header("content-length").unwrap_or_default();
            let digits = !given.is_empty() && given.bytes().all(|b| b.is_ascii_digit());
            match given.parse::<usize>() {
                Ok(length) if digits => length,
                // Too many digits for a number is too long as well.
                Err(_) if digits => MAX_BODY + 1,
                _ => return Err(Bad::Refused(400, "Content-Length is not a number")),
            }
        }
        _ => return Err(Bad::Refused(400, "more than one Content-Length")),
    };
    if request.header("transfer-encoding").is_some() {
        return Err(Bad::Refused(400, "Transfer-Encoding is not supported; send Content-Length"));
    }
    if length > MAX_BODY {
        return Err(Bad::Refused(413, "the request body is too long"));
    }
    let mut body = bytes.split_off(end);
    while body.len() < length {
        fill(stream, &mut body)?;
    }
    // One request per connection: what follows the body is not looked at.
    body.truncate(length);
    request.body = body;
    Ok(request)
}

fn parse_head(head: &[u8]) -> Result<Request, Bad> {
    const MALFORMED: Bad = Bad::Refused(400, "malformed request");
    let head = std::str::from_utf8(head).map_err(|_| MALFORMED)?;
    let mut lines = head.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line));
    let mut words = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version), None) = (words.next(), words.next(), words.next(), words.next()) else {
        return Err(MALFORMED);
    };
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") || !target.starts_with('/') {
        return Err(MALFORMED);
    }
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(MALFORMED);
    }
    let method = match method {
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ => Method::Other,
    };
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query.to_string())),
        None => (target, None),
    };
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(MALFORMED);
        };
        // A name with a space in it is how requests are smuggled past
        // proxies; none is taken.
        if name.is_empty() || name.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
            return Err(MALFORMED);
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok(Request {
        method,
        path: path.to_string(),
        query,
        headers,
        body: Vec::new(),
    })
}

/// What a request is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route {
    Page,
    Script,
    Style,
    /// Whether this browser has to log in.
    Session,
    Login,
    Logout,
    /// A command as a control-socket request line.
    Command,
    /// A command line as it was typed.
    Console,
    Events,
}

/// The route of a request, or the status it is refused with: 404 for a
/// path that is not served, 405 for a method the path is not served with.
/// Paths are compared as they are written: there is nothing to decode,
/// and so nothing that decodes to a path outside.
pub fn route(method: Method, path: &str, has_query: bool) -> Result<Route, u16> {
    let (wanted, route) = match path {
        "/" => (Method::Get, Route::Page),
        "/app.js" => (Method::Get, Route::Script),
        "/style.css" => (Method::Get, Route::Style),
        "/api/session" => (Method::Get, Route::Session),
        "/api/login" => (Method::Post, Route::Login),
        "/api/logout" => (Method::Post, Route::Logout),
        "/api/command" => (Method::Post, Route::Command),
        "/api/console" => (Method::Post, Route::Console),
        "/api/events" => (Method::Get, Route::Events),
        _ => return Err(404),
    };
    // Only the event stream takes a query, the event it goes on from.
    if has_query && route != Route::Events {
        return Err(404);
    }
    match method == wanted {
        true => Ok(route),
        false => Err(405),
    }
}

/// Whether a `Host` header names the bot by an IP address or as
/// `localhost`, with or without a port: the names a request is served
/// under while there is no password, since a name someone else made to
/// point at the bot is never one of them.
pub fn host_is_address(host: Option<&str>) -> bool {
    let Some(host) = host.map(str::trim) else {
        return false;
    };
    let port_ok = |port: &str| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
    if let Some(rest) = host.strip_prefix('[') {
        // An IPv6 address is always in brackets here.
        let Some((address, after)) = rest.split_once(']') else {
            return false;
        };
        let port = after.is_empty() || after.strip_prefix(':').is_some_and(port_ok);
        return port && address.parse::<std::net::Ipv6Addr>().is_ok();
    }
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port_ok(port) => name,
        Some(_) => return false,
        None => host,
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::Ipv4Addr>().is_ok()
}

/// What a request under another name is told.
pub const HOST_REFUSED: &str = "This web interface has no password, so it answers only when it is opened by IP address \
or as localhost, which keeps other web sites from reaching it under a name of their own.\n\
Open it by its IP address, or set `password` in the `[web]` table of the bot's config.\n";

/// Whether a command may be marked as the page's own: those that keep a
/// list up to date, `scan avatars`, `friends` and `payees`.
pub fn may_be_own(command: &Command) -> bool {
    match command {
        Command::Friends | Command::Payees => true,
        Command::Scan(args) => args.first().is_some_and(|first| first == "avatars"),
        _ => false,
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// A response. Whatever it is, it closes the connection, tells the
/// browser to take its content for what it says it is, to show it in no
/// frame and to load nothing from elsewhere, and never allows another
/// site to read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    status: u16,
    content_type: &'static str,
    /// `no-store`, or for the page's files the tag they are cached by.
    etag: Option<String>,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    /// A plain text answer that is not kept by anyone.
    pub fn text(status: u16, body: impl Into<String>) -> Response {
        Response {
            status,
            content_type: "text/plain; charset=utf-8",
            etag: None,
            headers: Vec::new(),
            body: body.into().into_bytes(),
        }
    }

    /// An answer without a body.
    pub fn empty(status: u16) -> Response {
        Response::text(status, "")
    }

    /// The answer to a refusal by status alone.
    pub fn status(status: u16) -> Response {
        Response::text(status, format!("{}\n", reason(status)))
    }

    /// One of the page's files: kept by the browser, and asked for again
    /// each time with its tag, so that a new build is seen at once.
    pub fn file(content_type: &'static str, body: &str, etag: &str, if_none_match: Option<&str>) -> Response {
        let fresh = if_none_match.is_some_and(|given| given.split(',').any(|tag| tag.trim() == etag));
        Response {
            status: if fresh { 304 } else { 200 },
            content_type,
            etag: Some(etag.to_string()),
            headers: Vec::new(),
            body: if fresh { Vec::new() } else { body.as_bytes().to_vec() },
        }
    }

    pub fn with(mut self, name: &'static str, value: String) -> Response {
        self.headers.push((name, value));
        self
    }

    #[cfg(test)]
    pub fn code(&self) -> u16 {
        self.status
    }

    /// The bytes to write.
    pub fn encode(&self) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", self.status, reason(self.status));
        head.push_str(&common_headers());
        match &self.etag {
            Some(etag) => head.push_str(&format!("Cache-Control: no-cache\r\nETag: {etag}\r\n")),
            None => head.push_str("Cache-Control: no-store\r\n"),
        }
        if self.status != 204 && self.status != 304 {
            head.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        }
        if self.status != 204 && self.status != 304 {
            head.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend(&self.body);
        bytes
    }
}

fn common_headers() -> String {
    format!(
        "Connection: close\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
         X-Frame-Options: DENY\r\nContent-Security-Policy: {CSP}\r\n"
    )
}

/// The head of the event stream: a response that does not end.
pub fn stream_head() -> Vec<u8> {
    let mut head = String::from("HTTP/1.1 200 OK\r\n");
    head.push_str(&common_headers());
    // The last line keeps a proxy from holding events back to fill a buffer.
    head.push_str("Cache-Control: no-store\r\nContent-Type: text/event-stream\r\nX-Accel-Buffering: no\r\n\r\n");
    head.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn read(bytes: &[u8]) -> Result<Request, Bad> {
        let mut source = bytes;
        read_request(&mut source, |_| true)
    }

    /// Gives its bytes a few at a time, as a slow client does.
    struct Trickle<'a>(&'a [u8], usize);

    impl Read for Trickle<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let count = self.1.min(self.0.len()).min(out.len());
            out[..count].copy_from_slice(&self.0[..count]);
            self.0 = &self.0[count..];
            Ok(count)
        }
    }

    #[test]
    fn a_get_is_read() {
        let request = read(b"GET /api/events?last=17-4&x=y HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nLast-Event-ID:  17-3 \r\n\r\n").unwrap();
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.path, "/api/events");
        assert_eq!(request.parameter("last"), Some("17-4"));
        assert_eq!(request.parameter("x"), Some("y"));
        assert_eq!(request.parameter("z"), None);
        assert_eq!(request.header("host"), Some("127.0.0.1:8080"));
        assert_eq!(request.header("last-event-id"), Some("17-3"));
        assert_eq!(request.header("cookie"), None);
        assert!(request.body.is_empty());
    }

    #[test]
    fn a_post_is_read_with_its_body_however_it_arrives() {
        let bytes = b"POST /api/command HTTP/1.1\r\nhost: localhost\r\nX-Rslbot: 1\r\nContent-Length: 9\r\n\r\nsay\thello";
        for step in [1, 3, 7, 4096] {
            let request = read_request(&mut Trickle(bytes, step), |_| true).unwrap();
            assert_eq!(request.method, Method::Post, "{step}");
            assert_eq!(request.body, b"say\thello");
            assert!(request.marked(MARKER) && !request.marked(OWN_MARKER));
        }
        // Bare line feeds are taken too, and what follows the body is not.
        let request = read(b"POST /api/console HTTP/1.0\nContent-Length: 2\n\nhiGET / HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!((request.path.as_str(), request.body.as_slice()), ("/api/console", &b"hi"[..]));
    }

    #[test]
    fn other_methods_are_read_and_left_to_the_router() {
        for method in ["OPTIONS", "HEAD", "PUT", "DELETE"] {
            let request = read(format!("{method} / HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
            assert_eq!(request.method, Method::Other);
        }
    }

    #[test]
    fn a_long_head_is_refused() {
        let long = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert_eq!(read(long.as_bytes()), Err(Bad::Refused(431, "the request head is too long")));
        // Also one that never ends.
        let endless = "GET / HTTP/1.1\r\nX: ".to_string() + &"a".repeat(3 * MAX_HEAD);
        assert_eq!(read(endless.as_bytes()), Err(Bad::Refused(431, "the request head is too long")));
        // A head just within the limit is read.
        let fits = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD - 23));
        assert_eq!(fits.len(), MAX_HEAD);
        assert!(read(fits.as_bytes()).is_ok());
    }

    #[test]
    fn a_long_body_is_refused_before_it_is_read() {
        let head = "POST /api/command HTTP/1.1\r\nContent-Length: 100000\r\n\r\n";
        assert_eq!(read(head.as_bytes()), Err(Bad::Refused(413, "the request body is too long")));
        let huge = "POST /api/command HTTP/1.1\r\nContent-Length: 99999999999999999999999999\r\n\r\n";
        assert_eq!(read(huge.as_bytes()), Err(Bad::Refused(413, "the request body is too long")));
        let fits = format!(
            "POST /api/command HTTP/1.1\r\nContent-Length: {MAX_BODY}\r\n\r\n{}",
            "a".repeat(MAX_BODY)
        );
        assert_eq!(read(fits.as_bytes()).unwrap().body.len(), MAX_BODY);
    }

    #[test]
    fn malformed_requests_are_refused() {
        for bad in [
            &b"GET /\r\n\r\n"[..],
            b"GET / HTTP/2\r\n\r\n",
            b"GET http://example.org/ HTTP/1.1\r\n\r\n",
            b"get / HTTP/1.1\r\n\r\n",
            b"GET / HTTP/1.1 extra\r\n\r\n",
            b"GET / HTTP/1.1\r\nno colon here\r\n\r\n",
            b"GET / HTTP/1.1\r\nContent-Length : 3\r\n\r\nabc",
            b"GET / HTTP/1.1\r\n: empty\r\n\r\n",
            b"GET / HTTP/1.1\r\nX: \xff\r\n\r\n",
        ] {
            assert_eq!(
                read(bad),
                Err(Bad::Refused(400, "malformed request")),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        for (bad, why) in [
            (
                &b"POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\nabc"[..],
                "more than one Content-Length",
            ),
            (b"POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n", "Content-Length is not a number"),
            (b"POST / HTTP/1.1\r\nContent-Length: 3x\r\n\r\n", "Content-Length is not a number"),
            (
                b"POST / HTTP/1.1\r\nContent-Length: +3\r\n\r\nabc",
                "Content-Length is not a number",
            ),
            (
                b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
                "Transfer-Encoding is not supported; send Content-Length",
            ),
        ] {
            assert_eq!(read(bad), Err(Bad::Refused(400, why)));
        }
    }

    #[test]
    fn a_client_that_stops_is_gone() {
        // Nothing at all, half a head, and a body shorter than announced.
        for partial in [
            &b""[..],
            b"GET / HTT",
            b"POST /api/command HTTP/1.1\r\nContent-Length: 9\r\n\r\nsay",
        ] {
            assert_eq!(read(partial), Err(Bad::Gone));
        }
        // The time runs out between two reads.
        let bytes = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut reads = 0;
        let late = read_request(&mut Trickle(bytes, 4), |_| {
            reads += 1;
            reads <= 3
        });
        assert_eq!(late, Err(Bad::Gone));
        // A read that times out, as a socket's does.
        struct Stalled;
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(ErrorKind::WouldBlock))
            }
        }
        assert_eq!(read_request(&mut Stalled, |_| true), Err(Bad::Gone));
    }

    #[test]
    fn requests_find_their_routes() {
        let routes = [
            (Method::Get, "/", Route::Page),
            (Method::Get, "/app.js", Route::Script),
            (Method::Get, "/style.css", Route::Style),
            (Method::Get, "/api/session", Route::Session),
            (Method::Post, "/api/login", Route::Login),
            (Method::Post, "/api/logout", Route::Logout),
            (Method::Post, "/api/command", Route::Command),
            (Method::Post, "/api/console", Route::Console),
            (Method::Get, "/api/events", Route::Events),
        ];
        for (method, path, wanted) in routes {
            assert_eq!(route(method, path, false), Ok(wanted), "{path}");
            // The other method, and any other, is not allowed there.
            let other = if method == Method::Get { Method::Post } else { Method::Get };
            assert_eq!(route(other, path, false), Err(405), "{path}");
            assert_eq!(route(Method::Other, path, false), Err(405), "{path}");
            // Only the event stream takes a query.
            let with_query = route(method, path, true);
            assert_eq!(with_query, if wanted == Route::Events { Ok(wanted) } else { Err(404) }, "{path}");
        }
        for path in [
            "/russelbot.toml",
            "/../etc/passwd",
            "/..",
            "/app.js/",
            "//",
            "/index.html",
            "/api",
            "/api/",
            "/api/command/",
            "/API/command",
            "/%2e%2e/etc/passwd",
            "/api%2fcommand",
            "/app.js%00",
            "/favicon.ico",
        ] {
            for method in [Method::Get, Method::Post, Method::Other] {
                assert_eq!(route(method, path, false), Err(404), "{path}");
            }
        }
    }

    #[test]
    fn names_the_bot_answers_to_without_a_password() {
        for host in [
            "127.0.0.1:8080",
            "127.0.0.1",
            "localhost:8080",
            "LOCALHOST",
            "192.168.1.5:8080",
            "[::1]:8080",
            "[::1]",
            "[2001:db8::1]:80",
            " 10.0.0.1:1 ",
        ] {
            assert!(host_is_address(Some(host)), "{host}");
        }
        for host in [
            "bot.attacker.example",
            "bot.attacker.example:8080",
            "127.0.0.1.attacker.example",
            "localhost.attacker.example:8080",
            "::1",
            "[::1]x",
            "[::1]:",
            "[localhost]",
            "127.0.0.1:",
            "127.0.0.1:80a",
            "127.1",
            "0x7f.1",
            "",
        ] {
            assert!(!host_is_address(Some(host)), "{host}");
        }
        assert!(!host_is_address(None));
        assert!(HOST_REFUSED.contains("IP address") && HOST_REFUSED.contains("`password`"));
    }

    #[test]
    fn only_the_two_listings_can_be_the_pages_own() {
        let scan = |words: &[&str]| Command::Scan(words.iter().map(|word| word.to_string()).collect());
        assert!(may_be_own(&Command::Friends));
        assert!(may_be_own(&Command::Payees));
        assert!(!may_be_own(&Command::Prices("x".into())));
        assert!(may_be_own(&scan(&["avatars"])));
        assert!(may_be_own(&scan(&["avatars", "chat"])));
        assert!(!may_be_own(&scan(&[])));
        assert!(!may_be_own(&scan(&["objects"])));
        assert!(!may_be_own(&scan(&["20", "avatars"])));
        assert!(!may_be_own(&Command::Say("hello".into())));
        assert!(!may_be_own(&Command::Status));
        assert!(!may_be_own(&Command::Quit));
    }

    fn text(response: &Response) -> String {
        String::from_utf8(response.encode()).unwrap()
    }

    #[test]
    fn an_api_answer_is_written_in_full() {
        let response = Response::text(200, "ok\thello\n").with("Set-Cookie", "a=b".to_string());
        assert_eq!(
            text(&response),
            "HTTP/1.1 200 OK\r\n\
             Connection: close\r\n\
             X-Content-Type-Options: nosniff\r\n\
             Referrer-Policy: no-referrer\r\n\
             X-Frame-Options: DENY\r\n\
             Content-Security-Policy: default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'\r\n\
             Cache-Control: no-store\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Length: 9\r\n\
             Set-Cookie: a=b\r\n\
             \r\n\
             ok\thello\n"
        );
    }

    #[test]
    fn no_answer_lets_another_site_in() {
        let answers = [
            text(&Response::text(200, "x")),
            text(&Response::empty(204)),
            text(&Response::status(405)),
            text(&Response::file("text/html; charset=utf-8", "<p>", "\"v\"", None)),
            String::from_utf8(stream_head()).unwrap(),
        ];
        for answer in answers {
            assert!(!answer.to_lowercase().contains("access-control"), "{answer}");
            assert!(answer.contains("Connection: close\r\n"), "{answer}");
            assert!(answer.contains("X-Content-Type-Options: nosniff\r\n"), "{answer}");
            assert!(answer.contains("frame-ancestors 'none'"), "{answer}");
        }
        let empty = text(&Response::empty(204));
        assert!(empty.starts_with("HTTP/1.1 204 No Content\r\n") && empty.ends_with("\r\n\r\n"));
        assert!(!empty.contains("Content-Length") && !empty.contains("\r\nContent-Type:"));
        let refused = text(&Response::status(404));
        assert!(refused.starts_with("HTTP/1.1 404 Not Found\r\n") && refused.ends_with("\r\n\r\nNot Found\n"));
    }

    #[test]
    fn a_file_is_cached_by_its_tag() {
        let first = Response::file("text/css; charset=utf-8", "body{}", "\"0.1.0\"", None);
        let sent = text(&first);
        assert!(sent.starts_with("HTTP/1.1 200 OK\r\n"), "{sent}");
        assert!(sent.contains("Cache-Control: no-cache\r\nETag: \"0.1.0\"\r\n"), "{sent}");
        assert!(
            sent.contains("Content-Type: text/css; charset=utf-8\r\nContent-Length: 6\r\n"),
            "{sent}"
        );
        assert!(sent.ends_with("\r\n\r\nbody{}"));
        for given in ["\"0.1.0\"", "\"old\", \"0.1.0\""] {
            let again = Response::file("text/css; charset=utf-8", "body{}", "\"0.1.0\"", Some(given));
            let sent = text(&again);
            assert_eq!(again.code(), 304);
            assert!(
                sent.starts_with("HTTP/1.1 304 Not Modified\r\n") && sent.ends_with("ETag: \"0.1.0\"\r\n\r\n"),
                "{sent}"
            );
            assert!(!sent.contains("Content-Length"), "{sent}");
        }
        let stale = Response::file("text/css; charset=utf-8", "body{}", "\"0.1.0\"", Some("\"0.0.9\""));
        assert_eq!(stale.code(), 200);
    }

    #[test]
    fn the_stream_head_does_not_end_the_response() {
        let head = String::from_utf8(stream_head()).unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n") && head.ends_with("\r\n\r\n"));
        assert!(head.contains("Content-Type: text/event-stream\r\n"));
        assert!(head.contains("Cache-Control: no-store\r\n"));
        assert!(!head.contains("Content-Length"));
    }
}
