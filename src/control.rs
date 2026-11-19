//! The control socket: a local socket on which a running bot takes one
//! command per connection, and the `ctl` subcommand that sends one. The
//! socket threads only pass requests to the main loop and write its answers.

use crate::escape::{escape, unescape};
use crate::net::Event;
use std::io::{self, ErrorKind, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Names the socket address for both the bot and `ctl`.
pub const ADDRESS_VARIABLE: &str = "RUSSELBOT_SOCK";
/// The socket file's name in the home directory.
#[cfg(unix)]
const DEFAULT_FILE: &str = "russelbot-ctl";
#[cfg(not(unix))]
const DEFAULT_TCP_ADDRESS: &str = "127.0.0.1:47115";

/// The longest request line, with its line ending.
const MAX_LINE: usize = 8192;
/// How long a client has to send its line.
const READ_LIMIT: Duration = Duration::from_secs(5);
/// How long a connection waits for the main loop's answer. A teleport may
/// take a minute to end one way or the other.
const ANSWER_LIMIT: Duration = Duration::from_secs(75);
/// How long `ctl` waits for the bot's answer.
const CLIENT_LIMIT: Duration = Duration::from_secs(90);
const MAX_CONNECTIONS: usize = 16;
/// How long a client turned away for that has to send its line.
const BUSY_READ_LIMIT: Duration = Duration::from_secs(1);

pub const CTL_USAGE: &str = "\
Usage: rslbot ctl say MESSAGE...
       rslbot ctl shout MESSAGE...
       rslbot ctl whisper MESSAGE...
       rslbot ctl im AVATAR MESSAGE...
       rslbot ctl stand | unsit
       rslbot ctl sit UUID | ground
       rslbot ctl tp X Y [Z] [REGION...]
       rslbot ctl requesttp AVATAR [MESSAGE...]
       rslbot ctl accepttp AVATAR
       rslbot ctl sendtp AVATAR [MESSAGE...]
       rslbot ctl touch UUID
       rslbot ctl home
       rslbot ctl status
       rslbot ctl quit

Sends a command to a running bot that has `socket = true` in its config.
`say` speaks in local chat; MESSAGE is read as a viewer's chat bar reads it,
so `/1 hello` goes to channel 1. `im` sends an instant message. AVATAR is
one argument: \"First Last\", first.last, a single word or a UUID.

`stand`, `sit`, `tp`, `requesttp` and `accepttp` move the avatar, and the
bot then leaves it where it is until `home` sends it back to the configured
region and seat. `tp` without REGION stays in the current region, and
without Z keeps the current height. `requesttp` asks AVATAR to teleport the
bot to them; `accepttp` accepts an offer AVATAR has sent; `sendtp` offers
AVATAR a teleport to the bot. `touch` clicks an object. `status` shows
where the avatar is; `quit` logs out and stops the bot.

The socket address is taken from the environment variable RUSSELBOT_SOCK
when it is set.";

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Local chat, still to be read the way a chat bar reads it.
    Say(String),
    /// Local chat, shouted as it is.
    Shout(String),
    /// Local chat, whispered as it is.
    Whisper(String),
    Im { to: String, text: String },
    Stand,
    /// A UUID or `ground`, still to be read.
    Sit(String),
    /// `x y [z] [region words]`, still to be read.
    Tp(Vec<String>),
    AskTp { to: String, message: String },
    AcceptTp(String),
    SendTp { to: String, message: String },
    /// A UUID, still to be read.
    Touch(String),
    Home,
    Status,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Ok(Option<String>),
    Error(String),
}

/// One line of tab-separated, escaped fields, with its line ending.
fn encode_line(fields: &[&str]) -> String {
    fields.iter().map(|field| escape(field)).collect::<Vec<_>>().join("\t") + "\n"
}

/// The fields of a line; its line ending is optional.
fn decode_line(line: &str) -> Vec<String> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.split('\t').map(unescape).collect()
}

impl Request {
    /// Reads a request line as a client sent it.
    pub fn decode(line: &[u8]) -> Result<Request, String> {
        let line = std::str::from_utf8(line).map_err(|_| "the command is not valid UTF-8".to_string())?;
        let mut fields = decode_line(line);
        let command = fields.remove(0);
        let given = fields.len();
        let plural = |count: usize| if count == 1 { "" } else { "s" };
        let arguments = |wanted: usize| match given == wanted {
            true => Ok(()),
            false if wanted == 0 => Err(format!("`{command}` takes no arguments")),
            false => Err(format!("`{command}` takes {wanted} argument{}, not {given}", plural(wanted))),
        };
        let at_least = |wanted: usize| match given >= wanted {
            true => Ok(()),
            false => Err(format!("`{command}` takes at least {wanted} argument{}, not {given}", plural(wanted))),
        };
        // An avatar, then the words of an optional message.
        let avatar_and_message = |mut fields: Vec<String>| {
            let to = fields.remove(0);
            (to, fields.join(" "))
        };
        match command.as_str() {
            "say" => {
                arguments(1)?;
                Ok(Request::Say(fields.remove(0)))
            }
            "shout" => {
                at_least(1)?;
                Ok(Request::Shout(fields.join(" ")))
            }
            "whisper" => {
                at_least(1)?;
                Ok(Request::Whisper(fields.join(" ")))
            }
            "im" => {
                arguments(2)?;
                let text = fields.remove(1);
                Ok(Request::Im { to: fields.remove(0), text })
            }
            "stand" | "unsit" => arguments(0).map(|()| Request::Stand),
            "sit" => {
                arguments(1)?;
                Ok(Request::Sit(fields.remove(0)))
            }
            "tp" => {
                at_least(2)?;
                Ok(Request::Tp(fields))
            }
            "requesttp" => {
                at_least(1)?;
                let (to, message) = avatar_and_message(fields);
                Ok(Request::AskTp { to, message })
            }
            "accepttp" => {
                arguments(1)?;
                Ok(Request::AcceptTp(fields.remove(0)))
            }
            "sendtp" => {
                at_least(1)?;
                let (to, message) = avatar_and_message(fields);
                Ok(Request::SendTp { to, message })
            }
            "touch" => {
                arguments(1)?;
                Ok(Request::Touch(fields.remove(0)))
            }
            "home" => arguments(0).map(|()| Request::Home),
            "status" => arguments(0).map(|()| Request::Status),
            "quit" => arguments(0).map(|()| Request::Quit),
            "" => Err("no command given".to_string()),
            other => Err(format!("unknown command `{other}`")),
        }
    }
}

impl Response {
    pub fn encode(&self) -> String {
        match self {
            Response::Ok(None) => encode_line(&["ok"]),
            Response::Ok(Some(text)) => encode_line(&["ok", text]),
            Response::Error(text) => encode_line(&["error", text]),
        }
    }

    pub fn decode(line: &str) -> Result<Response, String> {
        let fields = decode_line(line);
        match fields.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            ["ok"] => Ok(Response::Ok(None)),
            ["ok", text] => Ok(Response::Ok(Some(text.to_string()))),
            ["error", text] => Ok(Response::Error(text.to_string())),
            _ => Err("the bot's answer makes no sense".to_string()),
        }
    }
}

/// The request line for `ctl`'s arguments. The message words of `say` and
/// `im` become one argument; the bot judges everything else.
pub fn request_line(args: &[String]) -> String {
    let mut fields: Vec<String> = Vec::new();
    match args.first().map(String::as_str) {
        Some("say") if args.len() > 1 => fields.extend([args[0].clone(), args[1..].join(" ")]),
        Some("im") if args.len() > 2 => fields.extend([args[0].clone(), args[1].clone(), args[2..].join(" ")]),
        _ => fields.extend(args.iter().cloned()),
    }
    encode_line(&fields.iter().map(String::as_str).collect::<Vec<_>>())
}

/// The socket address from the environment: the override if there is one,
/// and the platform's default otherwise.
pub fn address_from(configured: Option<String>, home: Option<String>) -> Result<String, String> {
    if let Some(address) = configured.filter(|address| !address.is_empty()) {
        return Ok(address);
    }
    default_address(home)
}

#[cfg(unix)]
fn default_address(home: Option<String>) -> Result<String, String> {
    match home.filter(|home| !home.is_empty()) {
        Some(home) => Ok(format!("{}/{DEFAULT_FILE}", home.trim_end_matches('/'))),
        None => Err(format!("no home directory to put the socket in; set {ADDRESS_VARIABLE}")),
    }
}

#[cfg(not(unix))]
fn default_address(_home: Option<String>) -> Result<String, String> {
    Ok(DEFAULT_TCP_ADDRESS.to_string())
}

pub fn address() -> Result<String, String> {
    address_from(std::env::var(ADDRESS_VARIABLE).ok(), std::env::var("HOME").ok())
}

#[cfg(unix)]
mod sys {
    //! A Unix domain socket; the address is its file's path.
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

    pub fn bind(address: &str) -> io::Result<Listener> {
        match fs::symlink_metadata(address) {
            Ok(found) if found.file_type().is_socket() => {
                if Stream::connect(address).is_ok() {
                    return Err(io::Error::new(ErrorKind::AddrInUse, "another process is listening on it"));
                }
                // Left behind by a bot that is gone.
                fs::remove_file(address)?;
            }
            Ok(_) => return Err(io::Error::new(ErrorKind::AlreadyExists, "the file exists and is not a socket")),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = Listener::bind(address)?;
        fs::set_permissions(address, fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub fn connect(address: &str) -> io::Result<Stream> {
        Stream::connect(address)
    }

    pub fn remove(address: &str) {
        let _ = fs::remove_file(address);
    }
}

#[cfg(not(unix))]
mod sys {
    //! A TCP socket; the address is `host:port`.
    use std::io;
    pub use std::net::{TcpListener as Listener, TcpStream as Stream};

    pub fn bind(address: &str) -> io::Result<Listener> {
        Listener::bind(address)
    }

    pub fn connect(address: &str) -> io::Result<Stream> {
        Stream::connect(address)
    }

    pub fn remove(_address: &str) {}
}

/// The listening socket. Dropping it removes the socket file; the listener
/// thread lives until the process exits.
pub struct Server {
    address: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        sys::remove(&self.address);
    }
}

/// Starts listening on `address`. Every request arrives on `events` as an
/// `Event::Control`, to be answered on the sender it carries.
pub fn listen(address: &str, events: Sender<Event>) -> Result<Server, String> {
    let listener = sys::bind(address).map_err(|e| format!("cannot listen on {address}: {e}"))?;
    let connections = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    crate::info!("control socket: cannot accept a connection: {e}");
                    // Whatever is wrong, do not spin on it.
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            if connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                connections.fetch_sub(1, Ordering::SeqCst);
                // Hear the client out, briefly, so that the answer reaches it.
                read_line(&mut stream, BUSY_READ_LIMIT);
                let busy = Response::Error("too many control connections; try again".to_string());
                let _ = stream.write_all(busy.encode().as_bytes());
                continue;
            }
            let (events, connections) = (events.clone(), connections.clone());
            thread::spawn(move || {
                serve(stream, &events);
                connections.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(Server { address: address.to_string() })
}

/// What became of reading a client's line.
enum Line {
    Complete(Vec<u8>),
    TooLong,
    /// The client hung up, went quiet or the read failed.
    Missing,
}


fn read_line(stream: &mut sys::Stream, limit: Duration) -> Line {
    let deadline = Instant::now() + limit;
    let mut line = Vec::new();
    let mut too_long = false;
    let mut chunk = [0; 1024];
    loop {
        if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
            line.truncate(end);
            return if too_long { Line::TooLong } else { Line::Complete(line) };
        }
        if line.len() >= MAX_LINE {
            // Read the rest of it all the same, and forget it: an answer
            // sent while the client is still writing would not reach it.
            too_long = true;
            line.clear();
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return Line::Missing;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Line::Missing,
            Ok(count) => line.extend(&chunk[..count]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Line::Missing,
        }
    }
}

/// Handles one connection: one request in, one answer out.
fn serve(mut stream: sys::Stream, events: &Sender<Event>) {
    let request = match read_line(&mut stream, READ_LIMIT) {
        Line::Complete(line) => Request::decode(&line),
        Line::TooLong => Err(format!("the command is longer than {MAX_LINE} bytes")),
        Line::Missing => return,
    };
    let response = match request {
        Ok(request) => {
            let (reply, answer) = mpsc::channel();
            match events.send(Event::Control { request, reply }) {
                Ok(()) => answer
                    .recv_timeout(ANSWER_LIMIT)
                    .unwrap_or_else(|_| Response::Error("the bot did not answer".to_string())),
                Err(_) => Response::Error("the bot is shutting down".to_string()),
            }
        }
        Err(reason) => {
            crate::info!("control: bad command: {reason}");
            Response::Error(reason)
        }
    };
    // The client may be gone already; the command has been dealt with.
    let _ = stream.write_all(response.encode().as_bytes());
}

/// Sends one request line to the bot at `address` and reads its answer.
fn exchange(address: &str, line: &str, limit: Duration) -> Result<Response, String> {
    let failed = |what: &str, e: io::Error| format!("{what} the bot at {address}: {e}");
    let mut stream = sys::connect(address).map_err(|e| failed("cannot reach", e))?;
    stream.write_all(line.as_bytes()).map_err(|e| failed("cannot write to", e))?;
    stream.set_read_timeout(Some(limit)).map_err(|e| failed("cannot read from", e))?;
    let mut answer = String::new();
    match stream.read_to_string(&mut answer) {
        Ok(_) if answer.is_empty() => Err(format!("the bot at {address} closed the connection without answering")),
        Ok(_) => Response::decode(&answer),
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Err(format!("no answer from the bot at {address} within {} s", limit.as_secs()))
        }
        Err(e) => Err(failed("cannot read from", e)),
    }
}

/// The `ctl` subcommand: sends `args` as one command to the bot listening
/// on `address` and reports the answer. Returns the exit status.
pub fn client(
    args: &[String],
    address: Result<String, String>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            let _ = writeln!(err, "rslbot: ctl needs a command\n\n{CTL_USAGE}");
            return 2;
        }
        Some("-h" | "--help") => {
            let _ = writeln!(out, "{CTL_USAGE}");
            return 0;
        }
        Some(_) => {}
    }
    let answer = address.and_then(|address| exchange(&address, &request_line(args), CLIENT_LIMIT));
    match answer {
        Ok(Response::Ok(None)) => 0,
        Ok(Response::Ok(Some(text))) => {
            let _ = writeln!(out, "{text}");
            0
        }
        Ok(Response::Error(text)) | Err(text) => {
            let _ = writeln!(err, "rslbot: {text}");
            1
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
    fn requests_are_read_from_their_lines() {
        assert_eq!(Request::decode(b"say\thello there"), Ok(Request::Say("hello there".into())));
        assert_eq!(Request::decode(b"say\t/1 test message\r"), Ok(Request::Say("/1 test message".into())));
        assert_eq!(
            Request::decode(b"im\tJane Doe\tare you there?"),
            Ok(Request::Im { to: "Jane Doe".into(), text: "are you there?".into() })
        );
        // Escapes carry what a line cannot.
        assert_eq!(Request::decode(b"say\ta\\tb\\nc\\\\d"), Ok(Request::Say("a\tb\nc\\d".into())));
    }

    #[test]
    fn movement_requests_are_read_from_their_lines() {
        let text = |s: &str| s.to_string();
        assert_eq!(Request::decode(b"stand"), Ok(Request::Stand));
        assert_eq!(Request::decode(b"unsit"), Ok(Request::Stand));
        assert_eq!(Request::decode(b"sit\tground"), Ok(Request::Sit(text("ground"))));
        assert_eq!(
            Request::decode(b"tp\t128\t128\t25\tBlue\tLagoon"),
            Ok(Request::Tp(vec![text("128"), text("128"), text("25"), text("Blue"), text("Lagoon")]))
        );
        assert_eq!(
            Request::decode(b"requesttp\tJane Doe"),
            Ok(Request::AskTp { to: text("Jane Doe"), message: String::new() })
        );
        assert_eq!(
            Request::decode(b"sendtp\tJane Doe\tcome\tand see"),
            Ok(Request::SendTp { to: text("Jane Doe"), message: text("come and see") })
        );
        assert_eq!(Request::decode(b"accepttp\tjane.doe"), Ok(Request::AcceptTp(text("jane.doe"))));
        assert_eq!(Request::decode(b"touch\tabc"), Ok(Request::Touch(text("abc"))));
        assert_eq!(Request::decode(b"home"), Ok(Request::Home));
        assert_eq!(Request::decode(b"status"), Ok(Request::Status));
        assert_eq!(Request::decode(b"quit"), Ok(Request::Quit));
        assert_eq!(Request::decode(b"shout\tover\there"), Ok(Request::Shout(text("over here"))));
        assert_eq!(Request::decode(b"whisper\tpsst"), Ok(Request::Whisper(text("psst"))));
    }

    #[test]
    fn movement_requests_with_the_wrong_arguments_are_refused() {
        assert!(Request::decode(b"stand\tup").unwrap_err().contains("`stand` takes no arguments"));
        assert!(Request::decode(b"quit\tnow").unwrap_err().contains("`quit`"));
        assert!(Request::decode(b"sit").unwrap_err().contains("`sit` takes 1 argument, not 0"));
        assert!(Request::decode(b"sit\ta\tb").unwrap_err().contains("`sit`"));
        assert!(Request::decode(b"tp\t128").unwrap_err().contains("at least 2 arguments, not 1"));
        assert!(Request::decode(b"requesttp").unwrap_err().contains("at least 1 argument, not 0"));
        assert!(Request::decode(b"sendtp").unwrap_err().contains("`sendtp`"));
        assert!(Request::decode(b"accepttp\ta\tb").unwrap_err().contains("`accepttp`"));
        assert!(Request::decode(b"touch").unwrap_err().contains("`touch`"));
        assert!(Request::decode(b"shout").unwrap_err().contains("`shout`"));
    }

    #[test]
    fn bad_requests_are_refused_with_a_reason() {
        assert!(Request::decode(b"dance").unwrap_err().contains("`dance`"));
        assert!(Request::decode(b"dance\twith\tme").unwrap_err().contains("`dance`"));
        assert!(Request::decode(b"").unwrap_err().contains("no command"));
        assert!(Request::decode(b"say").unwrap_err().contains("1 argument,"));
        assert!(Request::decode(b"say\ta\tb").unwrap_err().contains("`say`"));
        assert!(Request::decode(b"im\tJane Doe").unwrap_err().contains("2 arguments, not 1"));
        assert!(Request::decode(b"im\ta\tb\tc").unwrap_err().contains("`im`"));
        assert!(Request::decode(b"say\t\xFF\xFE").unwrap_err().contains("UTF-8"));
    }

    #[test]
    fn responses_survive_the_wire() {
        let responses = [
            Response::Ok(None),
            Response::Ok(Some("sent to Jane Doe".into())),
            Response::Ok(Some(String::new())),
            Response::Error("line one\nline two\ttabbed \\ slashed".into()),
        ];
        for response in responses {
            let line = response.encode();
            assert_eq!(line.matches('\n').count(), 1, "{line:?}");
            assert_eq!(Response::decode(&line), Ok(response));
        }
        assert_eq!(Response::Ok(None).encode(), "ok\n");
        assert_eq!(Response::Error("no".into()).encode(), "error\tno\n");
        assert!(Response::decode("maybe\n").is_err());
        assert!(Response::decode("error\n").is_err());
    }

    #[test]
    fn ctl_arguments_become_one_request_line() {
        assert_eq!(request_line(&strings(&["say", "hello", "there"])), "say\thello there\n");
        assert_eq!(request_line(&strings(&["say", "-v", "/1", "x"])), "say\t-v /1 x\n");
        assert_eq!(request_line(&strings(&["im", "Jane Doe", "are", "you there?"])), "im\tJane Doe\tare you there?\n");
        assert_eq!(request_line(&strings(&["say", "a\tb"])), "say\ta\\tb\n");
        // What is missing is for the bot to complain about.
        assert_eq!(request_line(&strings(&["say"])), "say\n");
        assert_eq!(request_line(&strings(&["im", "Jane Doe"])), "im\tJane Doe\n");
        assert_eq!(request_line(&strings(&["dance", "with", "me"])), "dance\twith\tme\n");
    }

    #[test]
    fn address_comes_from_the_environment() {
        let set = |value: &str| Some(value.to_string());
        assert_eq!(address_from(set("/run/bot/ctl"), set("/home/bot")), Ok("/run/bot/ctl".to_string()));
        assert_eq!(address_from(set("/run/bot/ctl"), None), Ok("/run/bot/ctl".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn default_address_is_in_the_home_directory() {
        let set = |value: &str| Some(value.to_string());
        assert_eq!(address_from(None, set("/home/bot")), Ok("/home/bot/russelbot-ctl".to_string()));
        assert_eq!(address_from(set(""), set("/home/bot/")), Ok("/home/bot/russelbot-ctl".to_string()));
        for home in [None, set("")] {
            assert!(address_from(None, home).unwrap_err().contains("RUSSELBOT_SOCK"));
        }
    }

    #[cfg(unix)]
    mod socket {
        use super::super::*;
        use super::strings;
        use std::fs;
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::os::unix::net::UnixStream;
        use std::path::PathBuf;
        use std::sync::mpsc::Receiver;

        /// A fresh directory for one test's socket.
        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("rslbot-ctl-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn socket_path(name: &str) -> String {
            temp_dir(name).join("ctl").to_str().unwrap().to_string()
        }

        /// A listening server whose requests are answered by `answer`, as
        /// the main loop would.
        fn serving(name: &str, answer: fn(Request) -> Option<Response>) -> (Server, String) {
            let path = socket_path(name);
            let (events, incoming) = mpsc::channel();
            let server = listen(&path, events).unwrap();
            thread::spawn(move || answer_events(&incoming, answer));
            (server, path)
        }

        fn answer_events(incoming: &Receiver<Event>, answer: fn(Request) -> Option<Response>) {
            for event in incoming {
                if let Event::Control { request, reply } = event {
                    if let Some(response) = answer(request) {
                        let _ = reply.send(response);
                    }
                }
            }
        }

        fn echo(request: Request) -> Option<Response> {
            Some(match request {
                Request::Say(text) if text == "fail" => Response::Error("refused".to_string()),
                Request::Say(_) => Response::Ok(None),
                Request::Im { to, text } => Response::Ok(Some(format!("sent to {to}: {text}"))),
                other => Response::Ok(Some(format!("{other:?}"))),
            })
        }

        fn run(args: &[&str], address: &str) -> (i32, String, String) {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let status = client(&strings(args), Ok(address.to_string()), &mut out, &mut err);
            (status, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
        }

        #[test]
        fn socket_file_is_private_and_removed_with_the_server() {
            let path = socket_path("private");
            let (events, _incoming) = mpsc::channel();
            let server = listen(&path, events).unwrap();
            let found = fs::symlink_metadata(&path).unwrap();
            assert!(found.file_type().is_socket());
            assert_eq!(found.permissions().mode() & 0o777, 0o600);
            drop(server);
            assert!(fs::symlink_metadata(&path).is_err());
        }

        #[test]
        fn stale_socket_file_is_replaced() {
            let path = socket_path("stale");
            // A listener that is gone leaves its file behind.
            drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
            assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
            let (server, path) = {
                let (events, incoming) = mpsc::channel();
                let server = listen(&path, events).unwrap();
                thread::spawn(move || answer_events(&incoming, echo));
                (server, path)
            };
            assert_eq!(run(&["say", "hello"], &path), (0, String::new(), String::new()));
            drop(server);
        }

        #[test]
        fn live_socket_is_not_taken_over() {
            let (_server, path) = serving("live", echo);
            let (events, _incoming) = mpsc::channel();
            let refused = listen(&path, events).err().unwrap();
            assert!(refused.contains(&path) && refused.contains("another process"), "{refused}");
            // The first one still answers, and still has its file.
            assert_eq!(run(&["say", "hello"], &path).0, 0);
        }

        #[test]
        fn other_files_are_left_alone() {
            let path = socket_path("file");
            fs::write(&path, "precious").unwrap();
            let (events, _incoming) = mpsc::channel();
            let refused = listen(&path, events).err().unwrap();
            assert!(refused.contains("not a socket"), "{refused}");
            assert_eq!(fs::read_to_string(&path).unwrap(), "precious");
            // A directory that does not exist is an error naming the address.
            let missing = temp_dir("missing").join("no/such/ctl");
            let (events, _incoming) = mpsc::channel();
            let refused = listen(missing.to_str().unwrap(), events).err().unwrap();
            assert!(refused.contains("no/such/ctl"), "{refused}");
        }

        #[test]
        fn client_reports_the_bots_answer() {
            let (_server, path) = serving("client", echo);
            assert_eq!(run(&["say", "hello", "there"], &path), (0, String::new(), String::new()));
            assert_eq!(
                run(&["im", "Jane Doe", "are", "you", "there?"], &path),
                (0, "sent to Jane Doe: are you there?\n".to_string(), String::new())
            );
            assert_eq!(run(&["say", "fail"], &path), (1, String::new(), "rslbot: refused\n".to_string()));
            let (status, out, err) = run(&["dance"], &path);
            assert_eq!((status, out.as_str()), (1, ""));
            assert!(err.contains("unknown command `dance`"), "{err}");
            let (status, _, err) = run(&["im", "Jane Doe"], &path);
            assert!(status == 1 && err.contains("2 arguments"), "{err}");
        }

        #[test]
        fn client_fails_without_a_bot_or_a_command() {
            let path = socket_path("nobody");
            let (status, out, err) = run(&["say", "hello"], &path);
            assert_eq!((status, out.as_str()), (1, ""));
            assert!(err.contains(&path), "{err}");

            let (status, _, err) = run(&[], &path);
            assert!(status == 2 && err.contains("Usage"), "{err}");
            let (status, out, _) = run(&["--help"], &path);
            assert!(status == 0 && out.contains("Usage"), "{out}");

            // No address to be had at all.
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let status = client(&strings(&["say", "x"]), Err("no home directory".to_string()), &mut out, &mut err);
            assert_eq!((status, String::from_utf8(err).unwrap().as_str()), (1, "rslbot: no home directory\n"));
        }

        #[test]
        fn over_long_lines_are_refused() {
            let (_server, path) = serving("long", echo);
            let (status, _, err) = run(&["say", &"x".repeat(MAX_LINE)], &path);
            assert!(status == 1 && err.contains("8192"), "{err}");
            // Just under the limit is a request like any other.
            assert_eq!(run(&["say", &"x".repeat(MAX_LINE - 5)], &path).0, 0);
        }

        #[test]
        fn silent_client_is_dropped_and_others_are_served() {
            let (_server, path) = serving("silent", echo);
            let mut silent = UnixStream::connect(&path).unwrap();
            let mut partial = UnixStream::connect(&path).unwrap();
            partial.write_all(b"say\thalf a li").unwrap();
            // Others are served while these two say nothing.
            assert_eq!(run(&["say", "hello"], &path).0, 0);
            let started = Instant::now();
            let mut rest = Vec::new();
            for stream in [&mut silent, &mut partial] {
                stream.set_read_timeout(Some(READ_LIMIT * 2)).unwrap();
                assert_eq!(stream.read_to_end(&mut rest).unwrap(), 0);
            }
            let waited = started.elapsed();
            assert!(waited >= READ_LIMIT - Duration::from_secs(1) && waited < READ_LIMIT * 2, "{waited:?}");
        }

        #[test]
        fn client_that_leaves_early_still_has_its_command_carried_out() {
            let path = socket_path("early");
            let (events, incoming) = mpsc::channel();
            let _server = listen(&path, events).unwrap();
            let mut stream = UnixStream::connect(&path).unwrap();
            stream.write_all(b"say\thello\n").unwrap();
            drop(stream);
            match incoming.recv_timeout(Duration::from_secs(5)) {
                Ok(Event::Control { request, reply }) => {
                    assert_eq!(request, Request::Say("hello".into()));
                    reply.send(Response::Ok(None)).unwrap();
                }
                _ => panic!("the request did not arrive"),
            }
            // The server is none the worse for it.
            thread::spawn(move || answer_events(&incoming, echo));
            assert_eq!(run(&["say", "again"], &path).0, 0);
        }

        #[test]
        fn connections_beyond_the_limit_are_turned_away() {
            let (_server, path) = serving("crowd", echo);
            let idle: Vec<UnixStream> = (0..MAX_CONNECTIONS).map(|_| UnixStream::connect(&path).unwrap()).collect();
            // Give the listener time to take them all in.
            thread::sleep(Duration::from_millis(300));
            let (status, _, err) = run(&["say", "hello"], &path);
            assert!(status == 1 && err.contains("too many"), "{err}");
            drop(idle);
            thread::sleep(Duration::from_millis(300));
            assert_eq!(run(&["say", "hello"], &path).0, 0);
        }
    }
}
