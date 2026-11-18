//! Avatar name lookup through the simulator's `GetDisplayNames` capability,
//! on its own thread so the main loop never waits for HTTP.

use crate::caps;
use crate::net::Event;
use crate::xml::{self, Value};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

/// The service takes a limited number of ids per request.
const BATCH: usize = 50;
const TIMEOUT: Duration = Duration::from_secs(10);

/// What the grid knows an avatar by.
#[derive(Debug, Clone, PartialEq)]
pub struct Names {
    /// `First Last`.
    pub legacy: String,
    pub display: String,
}

/// A request to the worker: look these avatars up at this capability URL.
#[derive(Debug, PartialEq)]
pub struct Lookup {
    pub url: String,
    pub ids: Vec<Uuid>,
}

/// The avatars a reply names, and the ids it calls unknown.
type Reply = (Vec<(Uuid, Names)>, Vec<Uuid>);

pub fn request_url(capability: &str, ids: &[Uuid]) -> String {
    let mut url = capability.to_string();
    if !url.ends_with('/') {
        url.push('/');
    }
    for (index, id) in ids.iter().enumerate() {
        url.push_str(if index == 0 { "?ids=" } else { "&ids=" });
        url.push_str(&id.to_string());
    }
    url
}

/// Parses a reply into the avatars it names and the ids it calls unknown.
pub fn parse_reply(body: &str) -> Result<Reply, String> {
    let reply = xml::parse_llsd(body).map_err(|e| e.to_string())?;
    if !matches!(reply, Value::Map(_)) {
        return Err("display-name reply is not a map".to_string());
    }
    let list = |key| reply.get(key).and_then(Value::as_array).unwrap_or(&[]);
    let field = |agent: &Value, key| agent.str_field(key).unwrap_or("").trim().to_string();
    let mut found = Vec::new();
    for agent in list("agents") {
        let Some(id) = agent.get("id").and_then(Value::as_uuid) else { continue };
        let first = field(agent, "legacy_first_name");
        let last = field(agent, "legacy_last_name");
        let legacy = format!("{first} {last}").trim().to_string();
        found.push((id, Names { legacy, display: field(agent, "display_name") }));
    }
    let missing = list("bad_ids").iter().filter_map(Value::as_uuid).collect();
    Ok((found, missing))
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Reply, String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/llsd+xml")
        .call()
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    parse_reply(&body)
}

/// Worker thread body: answers every lookup with `Event::Names` or
/// `Event::NamesFailed` until either channel closes.
fn run(lookups: Receiver<Lookup>, events: Sender<Event>) {
    let agent = caps::http_agent(TIMEOUT);
    for lookup in lookups {
        for ids in lookup.ids.chunks(BATCH) {
            let event = match fetch(&agent, &request_url(&lookup.url, ids)) {
                Ok((found, mut missing)) => {
                    // An id the reply does not mention at all is unknown too.
                    for id in ids {
                        if !found.iter().any(|(known, _)| known == id) && !missing.contains(id) {
                            missing.push(*id);
                        }
                    }
                    Event::Names { found, missing }
                }
                Err(e) => {
                    crate::info!("name lookup failed: {e}");
                    Event::NamesFailed(ids.to_vec())
                }
            };
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

/// Starts the worker; lookups sent to the returned channel are answered on
/// `events`.
pub fn spawn(events: Sender<Event>) -> Sender<Lookup> {
    let (sender, lookups) = mpsc::channel();
    thread::spawn(move || run(lookups, events));
    sender
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    const JANE: &str = "11111111-2222-3333-4444-555555555555";
    const GROUP: &str = "99999999-2222-3333-4444-555555555555";

    fn reply() -> String {
        format!(
            "<llsd><map><key>agents</key><array><map>\
               <key>username</key><string>jane.doe</string>\
               <key>display_name</key><string>JD</string>\
               <key>legacy_first_name</key><string>Jane</string>\
               <key>legacy_last_name</key><string>Doe</string>\
               <key>id</key><uuid>{JANE}</uuid>\
               <key>is_display_name_default</key><boolean>0</boolean>\
             </map></array>\
             <key>bad_ids</key><array><uuid>{GROUP}</uuid></array></map></llsd>"
        )
    }

    fn uuid(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    fn jane() -> (Uuid, Names) {
        (uuid(JANE), Names { legacy: "Jane Doe".into(), display: "JD".into() })
    }

    #[test]
    fn builds_the_request_url() {
        let ids = [uuid(JANE), uuid(GROUP)];
        assert_eq!(
            request_url("http://sim/CAPS/names", &ids),
            format!("http://sim/CAPS/names/?ids={JANE}&ids={GROUP}")
        );
        assert_eq!(request_url("http://sim/CAPS/names/", &ids[..1]), format!("http://sim/CAPS/names/?ids={JANE}"));
    }

    #[test]
    fn parses_names_and_unknown_ids() {
        assert_eq!(parse_reply(&reply()), Ok((vec![jane()], vec![uuid(GROUP)])));
        assert_eq!(parse_reply("<llsd><map></map></llsd>"), Ok((vec![], vec![])));
    }

    #[test]
    fn rejects_malformed_replies() {
        assert!(parse_reply("not xml").is_err());
        assert!(parse_reply("<llsd><string>no</string></llsd>").is_err());
    }

    /// Serves `response` to one HTTP request and returns the request line.
    fn serve_once(listener: TcpListener, response: String) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            stream.write_all(response.as_bytes()).unwrap();
            request_line
        })
    }

    #[test]
    fn worker_answers_a_lookup_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/CAPS/names/", listener.local_addr().unwrap());
        let body = reply();
        let server = serve_once(
            listener,
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );
        let (events, incoming) = mpsc::channel();
        let lookups = spawn(events);
        let stranger = Uuid::from_bytes([7; 16]);
        lookups.send(Lookup { url, ids: vec![uuid(JANE), uuid(GROUP), stranger] }).unwrap();

        let Ok(Event::Names { found, missing }) = incoming.recv_timeout(Duration::from_secs(10)) else {
            panic!("no Names event")
        };
        assert_eq!(found, [jane()]);
        assert_eq!(missing, [uuid(GROUP), stranger]);
        let request_line = server.join().unwrap();
        assert!(request_line.starts_with(&format!("GET /CAPS/names/?ids={JANE}&ids={GROUP}&ids=")), "{request_line}");
    }

    #[test]
    fn worker_reports_a_failed_lookup() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/CAPS/names/", listener.local_addr().unwrap());
        let server = serve_once(
            listener,
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
        );
        let (events, incoming) = mpsc::channel();
        let lookups = spawn(events);
        lookups.send(Lookup { url, ids: vec![uuid(JANE)] }).unwrap();
        let Ok(Event::NamesFailed(ids)) = incoming.recv_timeout(Duration::from_secs(10)) else {
            panic!("no NamesFailed event")
        };
        assert_eq!(ids, [uuid(JANE)]);
        server.join().unwrap();
    }
}
