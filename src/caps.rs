//! Simulator capabilities: the seed request and the event-queue long poll.

use crate::xml::{self, Value};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const EVENT_QUEUE: &str = "EventQueueGet";
const DISPLAY_NAMES: &str = "GetDisplayNames";
const AVATAR_SEARCH: &str = "AvatarPickerSearch";
/// Continuous failure for this long means the session is gone.
pub const FAILURE_LIMIT: Duration = Duration::from_secs(60);
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// Long polls are held by the simulator for about half a minute.
const POLL_TIMEOUT: Duration = Duration::from_secs(90);

pub fn http_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent(concat!("RusselBot/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Where a teleport hands the session over to.
#[derive(Debug, Clone, PartialEq)]
pub struct TeleportFinish {
    pub sim_addr: SocketAddr,
    pub seed_capability: String,
    pub region_handle: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EqEvent {
    TeleportFinish(TeleportFinish),
    /// An event the bot does not act on, by name.
    Other(String),
}

/// What the event-queue thread reports to the main loop.
#[derive(Debug, Clone, PartialEq)]
pub enum CapsEvent {
    Event(EqEvent),
    /// The simulator no longer knows the session, or cannot be reached.
    Gone(String),
    /// The simulator's display-name lookup URL, or that it has none. Sent
    /// once, when the seed capability has answered.
    DisplayNames(Option<String>),
    /// The simulator's avatar search URL, or that it has none. Sent once,
    /// after `DisplayNames`.
    AvatarSearch(Option<String>),
}

/// The capabilities the bot uses, from the seed capability's reply.
#[derive(Debug, PartialEq)]
pub struct Seed {
    pub event_queue: String,
    pub display_names: Option<String>,
    pub avatar_search: Option<String>,
}

pub fn seed_request() -> String {
    let names = [EVENT_QUEUE, DISPLAY_NAMES, AVATAR_SEARCH].map(|name| Value::Str(name.to_string()));
    xml::write_llsd(&Value::Array(names.to_vec()))
}

pub fn parse_seed_reply(body: &str) -> Result<Seed, String> {
    let reply = xml::parse_llsd(body).map_err(|e| e.to_string())?;
    let url = |name| reply.str_field(name).filter(|url| !url.is_empty()).map(str::to_string);
    match url(EVENT_QUEUE) {
        Some(event_queue) => {
            Ok(Seed { event_queue, display_names: url(DISPLAY_NAMES), avatar_search: url(AVATAR_SEARCH) })
        }
        None => Err("simulator offers no EventQueueGet capability".to_string()),
    }
}

pub fn poll_request(ack: Option<i64>) -> String {
    let mut map = BTreeMap::new();
    map.insert("ack".to_string(), ack.map_or(Value::Undef, Value::Int));
    map.insert("done".to_string(), Value::Bool(false));
    xml::write_llsd(&Value::Map(map))
}

fn teleport_finish(body: &Value) -> Option<TeleportFinish> {
    let info = body.get("Info")?.as_array()?.first()?;
    let ip: [u8; 4] = info.get("SimIP")?.as_bytes()?.try_into().ok()?;
    let port = u16::try_from(info.get("SimPort")?.as_i64()?).ok()?;
    let handle: [u8; 8] = info.get("RegionHandle")?.as_bytes()?.try_into().ok()?;
    Some(TeleportFinish {
        sim_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port),
        seed_capability: info.str_field("SeedCapability")?.to_string(),
        region_handle: u64::from_be_bytes(handle),
    })
}

/// Parses an event-queue reply into its id and events.
pub fn parse_events(body: &str) -> Result<(Option<i64>, Vec<EqEvent>), String> {
    let reply = xml::parse_llsd(body).map_err(|e| e.to_string())?;
    let events = reply
        .get("events")
        .and_then(Value::as_array)
        .ok_or("event-queue reply has no `events`")?;
    let mut parsed = Vec::new();
    for event in events {
        let name = event.str_field("message").unwrap_or("");
        parsed.push(match (name, event.get("body")) {
            ("TeleportFinish", Some(body)) => EqEvent::TeleportFinish(
                teleport_finish(body).ok_or("TeleportFinish event is missing fields")?,
            ),
            _ => EqEvent::Other(name.to_string()),
        });
    }
    Ok((reply.get("id").and_then(Value::as_i64), parsed))
}

#[derive(Debug, PartialEq)]
pub enum PollResult {
    Events(Option<i64>, Vec<EqEvent>),
    /// The poll ended with nothing to report; poll again.
    Idle,
    /// The simulator says the session does not exist.
    Gone,
    Failed(String),
}

pub fn classify_poll(status: u16, body: &str) -> PollResult {
    match status {
        200 => match parse_events(body) {
            Ok((id, events)) => PollResult::Events(id, events),
            Err(e) => PollResult::Failed(e),
        },
        502 => PollResult::Idle,
        404 => PollResult::Gone,
        other => PollResult::Failed(format!("HTTP {other}")),
    }
}

/// Tracks how long requests have been failing without a success in between.
#[derive(Default)]
pub struct FailureWindow {
    failing_since: Option<Instant>,
}

impl FailureWindow {
    /// Records a failure; true once failures have lasted `FAILURE_LIMIT`.
    pub fn failed(&mut self, now: Instant) -> bool {
        now.duration_since(*self.failing_since.get_or_insert(now)) >= FAILURE_LIMIT
    }

    pub fn succeeded(&mut self) {
        self.failing_since = None;
    }
}

fn post(agent: &ureq::Agent, url: &str, body: &str) -> Result<(u16, String), ureq::Error> {
    let mut response = agent.post(url).content_type("application/llsd+xml").send(body)?;
    let status = response.status().as_u16();
    Ok((status, response.body_mut().read_to_string()?))
}

fn poll(agent: &ureq::Agent, url: &str, ack: Option<i64>) -> PollResult {
    match post(agent, url, &poll_request(ack)) {
        Ok((status, body)) => classify_poll(status, &body),
        // A long poll that simply ran out of time had nothing to report.
        Err(ureq::Error::Timeout(_)) => PollResult::Idle,
        Err(e) => PollResult::Failed(e.to_string()),
    }
}

fn fetch_seed(agent: &ureq::Agent, seed_capability: &str) -> Result<Seed, String> {
    match post(agent, seed_capability, &seed_request()) {
        Ok((200, body)) => parse_seed_reply(&body),
        Ok((status, _)) => Err(format!("seed capability answered HTTP {status}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Event-queue thread body: asks the seed capability for the queue URL, then
/// long-polls it until `stop` is set or the session is gone. `emit` returns
/// false when nobody is listening any more.
pub fn run(seed_capability: &str, stop: &AtomicBool, mut emit: impl FnMut(CapsEvent) -> bool) {
    let agent = http_agent(POLL_TIMEOUT);
    let mut window = FailureWindow::default();
    let mut queue_url: Option<String> = None;
    let mut ack = None;

    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        let failure = match &queue_url {
            None => match fetch_seed(&agent, seed_capability) {
                Ok(seed) => {
                    queue_url = Some(seed.event_queue);
                    if !emit(CapsEvent::DisplayNames(seed.display_names))
                        || !emit(CapsEvent::AvatarSearch(seed.avatar_search))
                    {
                        return;
                    }
                    None
                }
                Err(e) => Some(e),
            },
            Some(url) => match poll(&agent, url, ack) {
                PollResult::Events(id, events) => {
                    ack = id.or(ack);
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    for event in events {
                        if !emit(CapsEvent::Event(event)) {
                            return;
                        }
                    }
                    None
                }
                PollResult::Idle => {
                    // Guard against a server that answers "nothing" at once.
                    if started.elapsed() < RETRY_DELAY {
                        thread::sleep(RETRY_DELAY);
                    }
                    None
                }
                PollResult::Gone => {
                    if !stop.load(Ordering::Relaxed) {
                        emit(CapsEvent::Gone("event queue reports no such session".to_string()));
                    }
                    return;
                }
                PollResult::Failed(e) => Some(e),
            },
        };
        match failure {
            None => window.succeeded(),
            Some(reason) => {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                crate::debug!("event queue: {reason}");
                if window.failed(Instant::now()) {
                    emit(CapsEvent::Gone(format!("event queue unreachable for 60 s: {reason}")));
                    return;
                }
                thread::sleep(RETRY_DELAY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &str = "<llsd><map><key>events</key><array>\
        <map><key>message</key><string>EnableSimulator</string><key>body</key><map></map></map>\
        <map><key>message</key><string>TeleportFinish</string><key>body</key><map>\
          <key>Info</key><array><map>\
            <key>AgentID</key><uuid>11111111-2222-3333-4444-555555555555</uuid>\
            <key>LocationID</key><binary>AAAABA==</binary>\
            <key>SimIP</key><binary>fwAAAQ==</binary>\
            <key>SimPort</key><integer>9001</integer>\
            <key>RegionHandle</key><binary>AAPoAAAD6QA=</binary>\
            <key>SeedCapability</key><string>http://127.0.0.1:9001/CAPS/xyz0000/</string>\
            <key>SimAccess</key><integer>13</integer>\
            <key>TeleportFlags</key><binary>AAAAEA==</binary>\
          </map></array></map></map>\
        </array><key>id</key><integer>42</integer></map></llsd>";

    #[test]
    fn seed_request_asks_for_the_event_queue() {
        assert!(seed_request().ends_with(
            "<llsd><array><string>EventQueueGet</string><string>GetDisplayNames</string>\
             <string>AvatarPickerSearch</string></array></llsd>"
        ));
    }

    #[test]
    fn parses_seed_reply() {
        let body = "<llsd><map><key>EventQueueGet</key>\
                    <string>http://127.0.0.1:9000/CAPS/EQG/abc/</string></map></llsd>";
        let seed = parse_seed_reply(body).unwrap();
        assert_eq!(seed.event_queue, "http://127.0.0.1:9000/CAPS/EQG/abc/");
        assert_eq!(seed.display_names, None);
        let body = body.replace(
            "</map>",
            "<key>GetDisplayNames</key><string>http://127.0.0.1:9000/CAPS/names/</string></map>",
        );
        let seed = parse_seed_reply(&body).unwrap();
        assert_eq!(seed.display_names.as_deref(), Some("http://127.0.0.1:9000/CAPS/names/"));
        assert!(parse_seed_reply("<llsd><map></map></llsd>").is_err());
        assert!(parse_seed_reply("oops").is_err());
    }

    #[test]
    fn poll_request_carries_the_ack() {
        assert!(poll_request(None).contains("<key>ack</key><undef />"));
        let body = poll_request(Some(42));
        assert!(body.contains("<key>ack</key><integer>42</integer>"));
        assert!(body.contains("<key>done</key><boolean>0</boolean>"));
    }

    #[test]
    fn parses_teleport_finish_among_other_events() {
        let (id, events) = parse_events(EVENTS).unwrap();
        assert_eq!(id, Some(42));
        assert_eq!(
            events,
            [
                EqEvent::Other("EnableSimulator".into()),
                EqEvent::TeleportFinish(TeleportFinish {
                    sim_addr: "127.0.0.1:9001".parse().unwrap(),
                    seed_capability: "http://127.0.0.1:9001/CAPS/xyz0000/".into(),
                    region_handle: 0x0003_E800_0003_E900,
                }),
            ]
        );
    }

    #[test]
    fn incomplete_teleport_finish_is_a_failure() {
        let body = EVENTS.replace("<key>SimPort</key><integer>9001</integer>", "");
        assert!(matches!(classify_poll(200, &body), PollResult::Failed(_)));
    }

    #[test]
    fn classifies_poll_statuses() {
        assert!(matches!(classify_poll(200, EVENTS), PollResult::Events(Some(42), e) if e.len() == 2));
        assert_eq!(classify_poll(502, ""), PollResult::Idle);
        assert_eq!(classify_poll(404, ""), PollResult::Gone);
        assert!(matches!(classify_poll(500, ""), PollResult::Failed(_)));
        assert!(matches!(classify_poll(200, "garbage"), PollResult::Failed(_)));
    }

    #[test]
    fn failure_window_trips_after_sixty_seconds_of_failures() {
        let start = Instant::now();
        let secs = Duration::from_secs;
        let mut window = FailureWindow::default();
        assert!(!window.failed(start));
        assert!(!window.failed(start + secs(59)));
        assert!(window.failed(start + secs(60)));

        // A success in between starts the count again.
        let mut window = FailureWindow::default();
        assert!(!window.failed(start));
        window.succeeded();
        assert!(!window.failed(start + secs(61)));
        assert!(!window.failed(start + secs(120)));
        assert!(window.failed(start + secs(121)));
    }
}
