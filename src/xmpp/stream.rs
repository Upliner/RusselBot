//! The component stream (XEP-0114): opening it, the handshake, and reading
//! its top-level stanzas one at a time from a blocking reader into small
//! owned elements.

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use sha1::{Digest, Sha1};
use std::fmt;
use std::io::{self, BufRead};

pub const NS_COMPONENT: &str = "jabber:component:accept";
pub const NS_STREAM: &str = "http://etherx.jabber.org/streams";
pub const NS_STREAM_ERRORS: &str = "urn:ietf:params:xml:ns:xmpp-streams";

/// A stanza bigger than this is taken for a broken stream.
const MAX_STANZA: usize = 1 << 20;

/// An XML element read from the stream, with what the gateway looks at.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// The local name, without a prefix.
    pub name: String,
    pub ns: String,
    /// Attributes by their name as written (`xml:lang` keeps its prefix).
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The text directly inside the element, all pieces joined.
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    pub fn child(&self, name: &str, ns: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name && child.ns == ns)
    }

    fn size(&self) -> usize {
        let attrs: usize = self.attrs.iter().map(|(key, value)| key.len() + value.len()).sum();
        self.name.len() + self.text.len() + attrs + self.children.iter().map(Element::size).sum::<usize>()
    }
}

#[derive(Debug)]
pub enum StreamError {
    Io(io::Error),
    /// Not well-formed XML, or not a stream.
    Xml(String),
    /// The server ended the stream.
    Closed,
    /// The server sent a stream error: its condition and text.
    Refused(String),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StreamError::Io(e) => write!(f, "{e}"),
            StreamError::Xml(reason) => write!(f, "the server sent malformed XML: {reason}"),
            StreamError::Closed => f.write_str("the server closed the stream"),
            StreamError::Refused(reason) => write!(f, "the server refused: {reason}"),
        }
    }
}

impl From<io::Error> for StreamError {
    fn from(e: io::Error) -> StreamError {
        StreamError::Io(e)
    }
}

impl From<quick_xml::Error> for StreamError {
    fn from(e: quick_xml::Error) -> StreamError {
        match e {
            quick_xml::Error::Io(e) => StreamError::Io(io::Error::new(e.kind(), e.to_string())),
            other => StreamError::Xml(other.to_string()),
        }
    }
}

/// The opening tag the component sends.
pub fn stream_header(domain: &str) -> String {
    format!(
        "<?xml version='1.0'?><stream:stream xmlns='{NS_COMPONENT}' xmlns:stream='{NS_STREAM}' to={}>",
        super::stanza::quote(domain)
    )
}

pub const STREAM_END: &str = "</stream:stream>";

/// The handshake's content: SHA-1 of the stream id and the secret, in
/// lower-case hex.
pub fn handshake_digest(stream_id: &str, secret: &str) -> String {
    let digest = Sha1::digest(format!("{stream_id}{secret}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The text of a stream error, for the log: the condition and the text the
/// server gave, if any.
pub fn stream_error_text(error: &Element) -> String {
    let condition = error
        .children
        .iter()
        .find(|child| child.ns == NS_STREAM_ERRORS && child.name != "text")
        .map_or("undefined-condition", |child| child.name.as_str());
    match error.child("text", NS_STREAM_ERRORS).map(|text| text.text.trim()) {
        Some(text) if !text.is_empty() => format!("{condition} ({text})"),
        _ => condition.to_string(),
    }
}

/// Namespace declarations in force at one level of the tree.
type Scope = Vec<(String, String)>;

/// Reads a stream: its header, then its stanzas one by one.
pub struct StreamReader<R: BufRead> {
    reader: Reader<R>,
    buf: Vec<u8>,
    /// The scopes of the stream root (and of the element being read).
    scopes: Vec<Scope>,
}

impl<R: BufRead> StreamReader<R> {
    pub fn new(source: R) -> StreamReader<R> {
        let mut reader = Reader::from_reader(source);
        let config = reader.config_mut();
        config.check_end_names = true;
        config.expand_empty_elements = false;
        StreamReader {
            reader,
            buf: Vec::new(),
            scopes: Vec::new(),
        }
    }

    /// Reads up to and including the server's stream header, and returns
    /// its `id`.
    pub fn read_header(&mut self) -> Result<String, StreamError> {
        loop {
            self.buf.clear();
            match self.reader.read_event_into(&mut self.buf)? {
                Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
                Event::Text(text) if text.xml10_content().trim().is_empty() => {}
                Event::Start(start) => {
                    let start = start.into_owned();
                    let element = self.open(&start)?;
                    if element.name != "stream" || element.ns != NS_STREAM {
                        return Err(StreamError::Xml(format!("expected a stream header, got <{}>", element.name)));
                    }
                    return element
                        .attr("id")
                        .map(str::to_string)
                        .ok_or_else(|| StreamError::Xml("the stream header has no id".to_string()));
                }
                Event::Eof => return Err(StreamError::Closed),
                _ => return Err(StreamError::Xml("expected a stream header".to_string())),
            }
        }
    }

    /// The element `start` opens, with the namespaces in force; its scope
    /// is pushed.
    fn open(&mut self, start: &BytesStart) -> Result<Element, StreamError> {
        let mut scope = Scope::new();
        let mut attrs = Vec::new();
        for attr in start.attributes() {
            let attr = attr.map_err(|e| StreamError::Xml(e.to_string()))?;
            let key = attr.key.as_ref().to_string();
            let value = attr.normalized_value(XmlVersion::Implicit1_0)?.into_owned();
            if key == "xmlns" {
                scope.push((String::new(), value));
            } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                scope.push((prefix.to_string(), value));
            } else {
                attrs.push((key, value));
            }
        }
        self.scopes.push(scope);
        let qname = start.name().as_ref().to_string();
        let (prefix, name) = match qname.split_once(':') {
            Some((prefix, name)) => (prefix.to_string(), name.to_string()),
            None => (String::new(), qname),
        };
        let ns = self
            .scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter())
            .find(|(declared, _)| *declared == prefix)
            .map(|(_, ns)| ns.clone())
            .unwrap_or_default();
        Ok(Element {
            name,
            ns,
            attrs,
            ..Element::default()
        })
    }

    /// The next top-level stanza. Whitespace between stanzas is skipped;
    /// the end of the stream is `Closed`.
    pub fn next_stanza(&mut self) -> Result<Element, StreamError> {
        // The open elements of the stanza being read, innermost last.
        let mut open: Vec<Element> = Vec::new();
        loop {
            self.buf.clear();
            let event = self.reader.read_event_into(&mut self.buf)?;
            match event {
                Event::Start(start) => {
                    let start = start.into_owned();
                    open.push(self.open(&start)?);
                }
                Event::Empty(start) => {
                    let start = start.into_owned();
                    let element = self.open(&start)?;
                    self.scopes.pop();
                    match open.last_mut() {
                        Some(parent) => parent.children.push(element),
                        None => return Ok(element),
                    }
                }
                Event::End(_) => {
                    self.scopes.pop();
                    match open.pop() {
                        Some(element) => match open.last_mut() {
                            Some(parent) => parent.children.push(element),
                            None => return Ok(element),
                        },
                        // The stream root itself ended.
                        None => return Err(StreamError::Closed),
                    }
                }
                Event::Text(text) => {
                    if let Some(element) = open.last_mut() {
                        element.text.push_str(&text.xml10_content());
                    }
                }
                Event::CData(data) => {
                    if let Some(element) = open.last_mut() {
                        element.text.push_str(&data.xml10_content());
                    }
                }
                Event::GeneralRef(reference) => {
                    let resolved = match reference.resolve_char_ref()? {
                        Some(ch) => ch.to_string(),
                        None => resolve_predefined_entity(&reference.xml10_content())
                            .ok_or_else(|| StreamError::Xml(format!("unknown entity &{};", reference.xml10_content())))?
                            .to_string(),
                    };
                    if let Some(element) = open.last_mut() {
                        element.text.push_str(&resolved);
                    }
                }
                Event::Comment(_) | Event::PI(_) => {}
                Event::Decl(_) | Event::DocType(_) => return Err(StreamError::Xml("a declaration inside the stream".to_string())),
                Event::Eof => return Err(StreamError::Closed),
            }
            if open.iter().map(Element::size).sum::<usize>() > MAX_STANZA {
                return Err(StreamError::Xml(format!("a stanza longer than {MAX_STANZA} bytes")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn reader(text: &str) -> StreamReader<Cursor<Vec<u8>>> {
        StreamReader::new(Cursor::new(text.as_bytes().to_vec()))
    }

    const HEADER: &str = "<?xml version='1.0'?><stream:stream xmlns:stream='http://etherx.jabber.org/streams' \
        xml:lang='en' xmlns='jabber:component:accept' id='a1b2c3' from='bot.rig.test'>";

    #[test]
    fn the_handshake_is_the_sha1_of_id_and_secret() {
        let digest = handshake_digest("3BF96D75", "Calli0pe");
        assert_eq!(digest.len(), 40);
        assert!(digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        // sha1("abc") is well known.
        assert_eq!(handshake_digest("ab", "c"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn header_then_stanzas_one_at_a_time() {
        let mut stream = reader(&format!(
            "{HEADER}<handshake/> \n\t<message from='owner@rig.test/rig' to='bot.rig.test' type='chat' id='m1'>\
             <body>a &lt; b &amp; c &#x263A; <![CDATA[<raw>]]></body>\
             <active xmlns='http://jabber.org/protocol/chatstates'/></message> "
        ));
        assert_eq!(stream.read_header().unwrap(), "a1b2c3");
        let handshake = stream.next_stanza().unwrap();
        assert_eq!((handshake.name.as_str(), handshake.ns.as_str()), ("handshake", NS_COMPONENT));
        let message = stream.next_stanza().unwrap();
        assert_eq!(message.name, "message");
        assert_eq!(message.ns, NS_COMPONENT);
        assert_eq!(message.attr("from"), Some("owner@rig.test/rig"));
        assert_eq!(message.attr("type"), Some("chat"));
        assert_eq!(message.child("body", NS_COMPONENT).unwrap().text, "a < b & c \u{263A} <raw>");
        assert!(message.child("active", "http://jabber.org/protocol/chatstates").is_some());
        // The cursor runs dry: as good as a closed connection.
        assert!(matches!(stream.next_stanza(), Err(StreamError::Closed)));
    }

    #[test]
    fn stream_errors_are_read_with_their_namespace() {
        let mut stream = reader(&format!(
            "{HEADER}<stream:error><not-authorized xmlns='urn:ietf:params:xml:ns:xmpp-streams'/>\
             <text xmlns='urn:ietf:params:xml:ns:xmpp-streams'>Bad secret</text></stream:error></stream:stream>"
        ));
        stream.read_header().unwrap();
        let error = stream.next_stanza().unwrap();
        assert_eq!((error.name.as_str(), error.ns.as_str()), ("error", NS_STREAM));
        assert_eq!(stream_error_text(&error), "not-authorized (Bad secret)");
        assert!(matches!(stream.next_stanza(), Err(StreamError::Closed)));
    }

    #[test]
    fn malformed_xml_is_an_error() {
        let mut stream = reader(&format!("{HEADER}<message><body>x</bdy></message>"));
        stream.read_header().unwrap();
        assert!(matches!(stream.next_stanza(), Err(StreamError::Xml(_))));
        let mut stream = reader("<message/>");
        assert!(matches!(stream.read_header(), Err(StreamError::Xml(_))));
        let mut stream = reader(&format!("{HEADER}<message><body>&nbsp;</body></message>"));
        stream.read_header().unwrap();
        assert!(matches!(stream.next_stanza(), Err(StreamError::Xml(_))));
    }

    #[test]
    fn an_oversized_stanza_is_an_error() {
        let body = "x".repeat(MAX_STANZA + 10);
        let mut stream = reader(&format!("{HEADER}<message><body>{body}</body></message>"));
        stream.read_header().unwrap();
        assert!(matches!(stream.next_stanza(), Err(StreamError::Xml(_))));
    }

    /// Against the rig's Prosody (docs/test-rig.md): run with
    /// `cargo test -- --ignored rig_prosody`.
    #[test]
    #[ignore]
    fn rig_prosody_handshake() {
        use std::io::{BufReader, Write};
        use std::net::TcpStream;
        let open = |secret: &str| -> Result<(TcpStream, StreamReader<BufReader<TcpStream>>), StreamError> {
            let mut socket = TcpStream::connect("127.0.0.1:5347")?;
            socket.write_all(stream_header("bot.rig.test").as_bytes())?;
            let mut stream = StreamReader::new(BufReader::new(socket.try_clone()?));
            let id = stream.read_header()?;
            socket.write_all(format!("<handshake>{}</handshake>", handshake_digest(&id, secret)).as_bytes())?;
            let answer = stream.next_stanza()?;
            match answer.name.as_str() {
                "handshake" => Ok((socket, stream)),
                "error" => Err(StreamError::Refused(stream_error_text(&answer))),
                other => Err(StreamError::Xml(format!("unexpected <{other}>"))),
            }
        };
        match open("wrong") {
            Err(StreamError::Refused(reason)) => assert!(reason.starts_with("not-authorized"), "{reason}"),
            other => panic!("a wrong secret was not refused: {:?}", other.map(|_| ())),
        }
        let (_socket, mut stream) = open("rigsecret").expect("handshake");
        let client = std::process::Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/test-rig/xmpp.py"))
            .args(["owner", "--wait", "0.5", "msg:bot.rig.test:hello <there> & you"])
            .output()
            .expect("test-rig/xmpp.py");
        assert!(client.status.success());
        loop {
            let stanza = stream.next_stanza().expect("a stanza");
            eprintln!("{stanza:?}");
            if stanza.name == "message" {
                assert_eq!(stanza.attr("from"), Some("owner@rig.test/rig"));
                assert_eq!(stanza.child("body", NS_COMPONENT).unwrap().text, "hello <there> & you");
                break;
            }
        }
    }
}
