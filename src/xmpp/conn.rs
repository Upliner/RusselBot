//! The component connection: a reader thread that connects, does the
//! handshake, reads stanzas into the main loop's channel and connects again
//! when the connection is lost; and per connection a writer thread fed by a
//! bounded channel, so that the main loop never waits for the server.

use super::stream::{
    handshake_digest, stream_error_text, stream_header, Element, StreamError, StreamReader, NS_COMPONENT, NS_STREAM, STREAM_END,
};
use crate::backoff::Backoff;
use crate::config::Secret;
use crate::net::Event;
use std::io::{BufReader, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Timings and bounds of the connection.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The delay before the next attempt after the first failure in a row.
    pub first_delay: Duration,
    pub max_delay: Duration,
    /// After this long without anything written, a single space is.
    pub keepalive: Duration,
    /// How long connecting, the handshake and each write may take.
    pub io_limit: Duration,
    /// How many stanzas may wait for the writer.
    pub channel: usize,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            first_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(300),
            keepalive: Duration::from_secs(60),
            io_limit: Duration::from_secs(30),
            channel: 1000,
        }
    }
}

/// What the component connection reports to the main loop.
pub enum XmppEvent {
    /// The server accepted the handshake; stanzas go out through `writer`.
    Connected {
        generation: u64,
        writer: Writer,
    },
    Stanza {
        generation: u64,
        element: Element,
    },
    /// The connection is lost; the reader connects again after the delay.
    Disconnected {
        generation: u64,
        reason: String,
    },
    /// The answer to a request from an owner.
    Answer(super::gateway::Answer),
}

/// The way to one connection's writer thread.
pub struct Writer {
    stanzas: SyncSender<String>,
    socket: TcpStream,
    done: Receiver<()>,
}

impl Writer {
    fn spawn(socket: TcpStream, settings: &Settings) -> std::io::Result<Writer> {
        let (stanzas, incoming) = mpsc::sync_channel(settings.channel);
        let (finished, done) = mpsc::channel();
        let (theirs, keepalive) = (socket.try_clone()?, settings.keepalive);
        thread::spawn(move || {
            write_stanzas(theirs, &incoming, keepalive);
            let _ = finished.send(());
        });
        Ok(Writer { stanzas, socket, done })
    }

    /// Hands a stanza to the writer without waiting. An error means the
    /// writer cannot keep up or is gone: the connection is no good.
    pub fn try_send(&self, stanza: String) -> Result<(), ()> {
        match self.stanzas.try_send(stanza) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => Err(()),
        }
    }

    /// Ends the connection at once; the reader notices and connects again.
    pub fn give_up(self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    /// Ends the stream after what is waiting, giving the writer up to
    /// `limit` to write it.
    pub fn close(self, limit: Duration) {
        let _ = self.stanzas.try_send(STREAM_END.to_string());
        drop(self.stanzas);
        let _ = self.done.recv_timeout(limit);
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

fn write_stanzas(mut socket: TcpStream, incoming: &Receiver<String>, keepalive: Duration) {
    loop {
        let written = match incoming.recv_timeout(keepalive) {
            Ok(stanza) => socket.write_all(stanza.as_bytes()),
            // A closed connection shows when something is written.
            Err(RecvTimeoutError::Timeout) => socket.write_all(b" "),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if let Err(e) = written {
            crate::debug!("xmpp: write failed: {e}");
            break;
        }
    }
    // Ends the reader's wait too, if it is still reading.
    let _ = socket.shutdown(Shutdown::Both);
}

/// Opens the stream and does the handshake. The error says why, without
/// the secret.
pub fn connect(
    domain: &str,
    server: &str,
    secret: &Secret,
    limit: Duration,
) -> Result<(TcpStream, StreamReader<BufReader<TcpStream>>), StreamError> {
    let mut last = None;
    let mut socket = None;
    for address in server.to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, limit) {
            Ok(connected) => {
                socket = Some(connected);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let mut socket = match (socket, last) {
        (Some(socket), _) => socket,
        (None, Some(e)) => return Err(StreamError::Io(e)),
        (None, None) => return Err(StreamError::Xml(format!("{server} names no address"))),
    };
    socket.set_read_timeout(Some(limit))?;
    socket.set_write_timeout(Some(limit))?;
    socket.set_nodelay(true)?;
    socket.write_all(stream_header(domain).as_bytes())?;
    let mut reader = StreamReader::new(BufReader::new(socket.try_clone()?));
    let id = reader.read_header()?;
    socket.write_all(format!("<handshake>{}</handshake>", handshake_digest(&id, secret.expose())).as_bytes())?;
    let answer = reader.next_stanza()?;
    match (answer.name.as_str(), answer.ns.as_str()) {
        ("handshake", NS_COMPONENT) => {}
        ("error", NS_STREAM) => return Err(StreamError::Refused(stream_error_text(&answer))),
        (name, _) => return Err(StreamError::Xml(format!("<{name}> instead of the handshake's answer"))),
    }
    // From now on the server may stay quiet for as long as it likes.
    socket.set_read_timeout(None)?;
    Ok((socket, reader))
}

/// Stops the reader from connecting again once the bot is done.
#[derive(Clone)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Starts the reader thread for the component `domain` at `server`.
pub fn spawn(domain: String, server: String, secret: Secret, settings: Settings, events: Sender<Event>) -> Stop {
    let stop = Stop(Arc::new(AtomicBool::new(false)));
    let stopped = stop.clone();
    thread::spawn(move || run(&domain, &server, &secret, &settings, &events, &stopped.0));
    stop
}

fn run(domain: &str, server: &str, secret: &Secret, settings: &Settings, events: &Sender<Event>, stopped: &AtomicBool) {
    let mut backoff = Backoff::new(settings.first_delay);
    let mut generation = 0;
    while !stopped.load(Ordering::Relaxed) {
        generation += 1;
        match connect(domain, server, secret, settings.io_limit) {
            Ok((socket, mut reader)) => {
                backoff.reset();
                let writer = match socket.try_clone().and_then(|socket| Writer::spawn(socket, settings)) {
                    Ok(writer) => writer,
                    Err(e) => {
                        crate::info!("xmpp: component {domain}: cannot start writing: {e}");
                        continue;
                    }
                };
                crate::info!("xmpp: component {domain} connected to {server}");
                if events.send(Event::Xmpp(XmppEvent::Connected { generation, writer })).is_err() {
                    return;
                }
                let reason = loop {
                    match reader.next_stanza() {
                        Ok(element) => {
                            if events.send(Event::Xmpp(XmppEvent::Stanza { generation, element })).is_err() {
                                return;
                            }
                        }
                        Err(e) => break e.to_string(),
                    }
                };
                let _ = socket.shutdown(Shutdown::Both);
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                crate::info!("xmpp: component {domain}: connection to {server} lost: {reason}");
                let reason = reason.clone();
                if events.send(Event::Xmpp(XmppEvent::Disconnected { generation, reason })).is_err() {
                    return;
                }
            }
            Err(e) => crate::info!("xmpp: component {domain}: cannot connect to {server}: {e}"),
        }
        let delay = backoff.next_delay(settings.max_delay);
        crate::info!("xmpp: component {domain}: next attempt in {} s", delay.as_secs_f32());
        thread::sleep(delay);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::xmpp::stanza::Stanza;
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::Instant;

    pub const SECRET: &str = "s3cret-for-tests";
    pub const DOMAIN: &str = "bot.rig.test";

    /// Reads from `socket` until `pattern` has arrived; returns everything.
    pub fn read_until(socket: &mut TcpStream, pattern: &str, limit: Duration) -> String {
        socket.set_read_timeout(Some(limit)).unwrap();
        let mut seen = Vec::new();
        let mut chunk = [0; 4096];
        while !String::from_utf8_lossy(&seen).contains(pattern) {
            match socket.read(&mut chunk) {
                Ok(0) | Err(_) => panic!("no {pattern:?}; got {:?}", String::from_utf8_lossy(&seen)),
                Ok(count) => seen.extend(&chunk[..count]),
            }
        }
        String::from_utf8_lossy(&seen).into_owned()
    }

    /// Plays the server's part of the handshake on an accepted socket:
    /// accepts the secret `SECRET`, refuses any other.
    pub fn serve_handshake(socket: &mut TcpStream) -> bool {
        read_until(socket, "<stream:stream", Duration::from_secs(5));
        socket
            .write_all(
                b"<?xml version='1.0'?><stream:stream xmlns:stream='http://etherx.jabber.org/streams' \
                  xml:lang='en' xmlns='jabber:component:accept' id='fake1' from='bot.rig.test'>",
            )
            .unwrap();
        let handshake = read_until(socket, "</handshake>", Duration::from_secs(5));
        if handshake.contains(&handshake_digest("fake1", SECRET)) {
            socket.write_all(b"<handshake/>").unwrap();
            true
        } else {
            socket
                .write_all(b"<stream:error><not-authorized xmlns='urn:ietf:params:xml:ns:xmpp-streams'/></stream:error></stream:stream>")
                .unwrap();
            false
        }
    }

    /// The server's end of a component connection, for tests.
    pub struct Fake {
        listener: TcpListener,
        socket: Option<TcpStream>,
        /// Read and not yet expected.
        buffer: String,
    }

    impl Fake {
        pub fn new() -> Fake {
            Fake {
                listener: TcpListener::bind("127.0.0.1:0").unwrap(),
                socket: None,
                buffer: String::new(),
            }
        }

        pub fn address(&self) -> String {
            self.listener.local_addr().unwrap().to_string()
        }

        /// Takes the component's connection and accepts its handshake.
        pub fn accept(&mut self) {
            let (mut socket, _) = self.listener.accept().unwrap();
            assert!(serve_handshake(&mut socket));
            self.socket = Some(socket);
            self.buffer.clear();
        }

        /// Sends the component a stanza, as from a client.
        pub fn send(&mut self, xml: &str) {
            self.socket.as_mut().expect("accepted").write_all(xml.as_bytes()).unwrap();
        }

        fn fill(&mut self, wait: Duration) -> bool {
            let socket = self.socket.as_mut().expect("accepted");
            socket.set_read_timeout(Some(wait)).unwrap();
            let mut chunk = [0; 65536];
            match socket.read(&mut chunk) {
                Ok(0) => false,
                Ok(count) => {
                    self.buffer.push_str(&String::from_utf8_lossy(&chunk[..count]));
                    true
                }
                Err(_) => false,
            }
        }

        /// What the component wrote up to and including `pattern`, which
        /// must come within 5 seconds.
        pub fn expect(&mut self, pattern: &str) -> String {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !self.buffer.contains(pattern) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || !self.fill(left) {
                    panic!("no {pattern:?} from the component; it wrote {:?}", self.buffer);
                }
            }
            let end = self.buffer.find(pattern).unwrap() + pattern.len();
            let seen: String = self.buffer.drain(..end).collect();
            seen.trim().to_string()
        }

        /// Everything the component writes within `wait`, without the
        /// spaces it keeps the connection alive with.
        pub fn rest(&mut self, wait: Duration) -> String {
            let deadline = Instant::now() + wait;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || !self.fill(left) {
                    break;
                }
            }
            std::mem::take(&mut self.buffer).trim().to_string()
        }
    }

    fn fast() -> Settings {
        Settings {
            first_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(200),
            keepalive: Duration::from_secs(60),
            io_limit: Duration::from_secs(5),
            channel: 1000,
        }
    }

    fn start(server: String, secret: &str, settings: Settings) -> (Stop, Receiver<Event>) {
        let (events, incoming) = mpsc::channel();
        let stop = spawn(DOMAIN.to_string(), server, Secret::new(secret), settings, events);
        (stop, incoming)
    }

    fn next_xmpp(incoming: &Receiver<Event>) -> XmppEvent {
        match incoming.recv_timeout(Duration::from_secs(5)).expect("an event") {
            Event::Xmpp(event) => event,
            _ => panic!("not an XMPP event"),
        }
    }

    #[test]
    fn server_up_at_startup() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (_stop, incoming) = start(listener.local_addr().unwrap().to_string(), SECRET, fast());
        let (mut socket, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut socket));
        let XmppEvent::Connected { generation, writer } = next_xmpp(&incoming) else {
            panic!("not connected")
        };
        assert_eq!(generation, 1);
        socket
            .write_all(b" <message type='chat' from='owner@rig.test/r' to='bot.rig.test'><body>status</body></message>")
            .unwrap();
        let XmppEvent::Stanza { generation, element } = next_xmpp(&incoming) else {
            panic!("no stanza")
        };
        assert_eq!(generation, 1);
        assert!(matches!(crate::xmpp::stanza::classify(&element), Stanza::Message { body: Some(b), .. } if b == "status"));
        writer.try_send("<presence/>".into()).unwrap();
        assert!(read_until(&mut socket, "<presence/>", Duration::from_secs(5)).contains("<presence/>"));
        writer.close(Duration::from_secs(2));
        assert!(read_until(&mut socket, STREAM_END, Duration::from_secs(5)).ends_with(STREAM_END));
    }

    #[test]
    fn server_comes_up_later() {
        // A port nobody listens on, for now.
        let address = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let started = Instant::now();
        let (_stop, incoming) = start(address.to_string(), SECRET, fast());
        thread::sleep(Duration::from_millis(400));
        assert!(incoming.try_recv().is_err(), "nothing to report while nothing listens");
        let listener = TcpListener::bind(address).unwrap();
        let (mut socket, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut socket));
        assert!(matches!(next_xmpp(&incoming), XmppEvent::Connected { .. }));
        // Attempts every 50, 100, 200, 200 ... ms: connected soon after.
        assert!(started.elapsed() < Duration::from_millis(400 + 200 + 1000));
    }

    #[test]
    fn wrong_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = listener.local_addr().unwrap().to_string();
        let refusing = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            serve_handshake(&mut socket)
        });
        let error = connect(DOMAIN, &server, &Secret::new("not-the-secret"), Duration::from_secs(5))
            .err()
            .unwrap();
        assert!(!refusing.join().unwrap());
        let text = error.to_string();
        assert!(text.contains("not-authorized"), "{text}");
        assert!(!text.contains("not-the-secret"), "{text}");
    }

    #[test]
    fn wrong_secret_is_tried_again_after_the_delay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (_stop, incoming) = start(listener.local_addr().unwrap().to_string(), "wrong", fast());
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            assert!(!serve_handshake(&mut socket));
        }
        assert!(incoming.try_recv().is_err());
    }

    #[test]
    fn nothing_listening() {
        let address = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let error = connect(DOMAIN, &address.to_string(), &Secret::new(SECRET), Duration::from_secs(1))
            .err()
            .unwrap();
        assert!(matches!(error, StreamError::Io(_)), "{error}");
    }

    #[test]
    fn malformed_stanza() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let (_stop, incoming) = start(listener.local_addr().unwrap().to_string(), SECRET, fast());
        let (mut socket, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut socket));
        // Dropping the writer would end the connection.
        let XmppEvent::Connected {
            generation: 1,
            writer: _writer,
        } = next_xmpp(&incoming)
        else {
            panic!("not connected")
        };
        socket.write_all(b"<message><body>x</bdy></message>").unwrap();
        let XmppEvent::Disconnected { generation, reason } = next_xmpp(&incoming) else {
            panic!("not disconnected")
        };
        assert_eq!(generation, 1);
        assert!(reason.contains("malformed"), "{reason}");
        // And again, after the delay.
        let (mut again, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut again));
        assert!(matches!(next_xmpp(&incoming), XmppEvent::Connected { generation: 2, .. }));
    }

    #[test]
    fn a_quiet_connection_gets_a_space() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let settings = Settings {
            keepalive: Duration::from_millis(50),
            ..fast()
        };
        let (_stop, incoming) = start(listener.local_addr().unwrap().to_string(), SECRET, settings);
        let (mut socket, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut socket));
        let XmppEvent::Connected { writer: _writer, .. } = next_xmpp(&incoming) else {
            panic!("not connected")
        };
        let mut space = [0; 1];
        socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        socket.read_exact(&mut space).unwrap();
        assert_eq!(&space, b" ");
    }

    #[test]
    fn a_server_that_stops_reading_fills_the_channel_without_blocking() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let settings = Settings { channel: 8, ..fast() };
        let (_stop, incoming) = start(listener.local_addr().unwrap().to_string(), SECRET, settings);
        let (mut socket, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut socket));
        let XmppEvent::Connected { writer, .. } = next_xmpp(&incoming) else {
            panic!("not connected")
        };
        // The server reads nothing more: socket buffers fill, the writer
        // blocks, and then the channel.
        let big = format!("<message><body>{}</body></message>", "x".repeat(64 * 1024));
        let started = Instant::now();
        let mut sent = 0;
        while writer.try_send(big.clone()).is_ok() {
            sent += 1;
            assert!(sent < 10_000, "the channel never filled");
        }
        assert!(started.elapsed() < Duration::from_secs(2), "try_send waited");
        writer.give_up();
        assert!(matches!(next_xmpp(&incoming), XmppEvent::Disconnected { generation: 1, .. }));
        drop(socket);
        let (mut again, _) = listener.accept().unwrap();
        assert!(serve_handshake(&mut again));
        assert!(matches!(next_xmpp(&incoming), XmppEvent::Connected { generation: 2, .. }));
    }
}
