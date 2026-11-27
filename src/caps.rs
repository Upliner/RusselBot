//! Simulator capabilities: the seed request and the event-queue long poll.

use crate::groups::Membership;
use crate::xml::{self, Value};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const EVENT_QUEUE: &str = "EventQueueGet";
const DISPLAY_NAMES: &str = "GetDisplayNames";
const AVATAR_SEARCH: &str = "AvatarPickerSearch";
const AGENT_PROFILE: &str = "AgentProfile";
const FETCH_DESCENDENTS: &str = "FetchInventoryDescendents2";
const FETCH_ITEMS: &str = "FetchInventory2";
const INVENTORY_API: &str = "InventoryAPIv3";
const UPDATE_APPEARANCE: &str = "UpdateAvatarAppearance";
const CHAT_SESSIONS: &str = "ChatSessionRequest";
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

/// A line of a chat session as an event delivers it: the first line of a
/// group's session comes this way.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLine {
    /// The session: a group's UUID for a group's session.
    pub session: Uuid,
    pub from: Uuid,
    pub from_name: String,
    pub dialog: u8,
    pub text: String,
    pub bucket: Vec<u8>,
    pub offline: bool,
    pub timestamp: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EqEvent {
    TeleportFinish(TeleportFinish),
    /// `AgentGroupDataUpdate`: the groups the avatar is in.
    Groups(Vec<Membership>),
    /// `AgentDropGroup`: the avatar is no longer in this group.
    GroupDropped(Uuid),
    /// `ChatterBoxInvitation` with the line that opens a session.
    SessionInvite(SessionLine),
    /// `ChatterBoxSessionStartReply`: whether the avatar is in the session
    /// it asked to start, and the grid's reason when not.
    SessionStarted {
        session: Uuid,
        ok: bool,
        error: String,
    },
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
    /// The simulator's profile service URL, or that it has none. Sent once,
    /// after `AvatarSearch`.
    Profiles(Option<String>),
    /// The simulator's inventory services, as far as it has them. Sent
    /// once, after `Profiles`.
    Inventory(InventoryUrls),
    /// The simulator's chat-session service URL, or that it has none. Sent
    /// once, after `Inventory`.
    ChatSessions(Option<String>),
}

/// Where the simulator answers for the inventory; each may be missing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InventoryUrls {
    /// `FetchInventoryDescendents2`: the contents of folders.
    pub descendents: Option<String>,
    /// `FetchInventory2`: items by id.
    pub items: Option<String>,
    /// `InventoryAPIv3`: changes to the inventory, used for the links in
    /// the Current Outfit folder.
    pub api: Option<String>,
    /// `UpdateAvatarAppearance`: makes the avatar's look from that folder.
    pub appearance: Option<String>,
}

impl InventoryUrls {
    /// Whether the simulator makes the avatar's appearance on the server,
    /// which is what clothing needs.
    pub fn outfit_service(&self) -> bool {
        self.api.is_some() && self.appearance.is_some()
    }
}

/// The capabilities the bot uses, from the seed capability's reply.
#[derive(Debug, PartialEq)]
pub struct Seed {
    pub event_queue: String,
    pub display_names: Option<String>,
    pub avatar_search: Option<String>,
    pub profiles: Option<String>,
    pub inventory: InventoryUrls,
    pub chat_sessions: Option<String>,
}

pub fn seed_request() -> String {
    let names = [
        EVENT_QUEUE,
        DISPLAY_NAMES,
        AVATAR_SEARCH,
        AGENT_PROFILE,
        FETCH_DESCENDENTS,
        FETCH_ITEMS,
        INVENTORY_API,
        UPDATE_APPEARANCE,
        CHAT_SESSIONS,
    ]
    .map(|name| Value::Str(name.to_string()));
    xml::write_llsd(&Value::Array(names.to_vec()))
}

pub fn parse_seed_reply(body: &str) -> Result<Seed, String> {
    let reply = xml::parse_llsd(body).map_err(|e| e.to_string())?;
    let url = |name| reply.str_field(name).filter(|url| !url.is_empty()).map(str::to_string);
    match url(EVENT_QUEUE) {
        Some(event_queue) => Ok(Seed {
            event_queue,
            display_names: url(DISPLAY_NAMES),
            avatar_search: url(AVATAR_SEARCH),
            profiles: url(AGENT_PROFILE),
            inventory: InventoryUrls {
                descendents: url(FETCH_DESCENDENTS),
                items: url(FETCH_ITEMS),
                api: url(INVENTORY_API),
                appearance: url(UPDATE_APPEARANCE),
            },
            chat_sessions: url(CHAT_SESSIONS),
        }),
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

/// A boolean as grids write one: a boolean, a number or a string.
fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Int(number) => Some(*number != 0),
        Value::Str(text) => match text.trim() {
            "true" | "1" => Some(true),
            "false" | "0" | "" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A U64 as LLSD carries one: eight bytes, big-endian.
fn powers(value: &Value) -> Option<u64> {
    match value {
        Value::Binary(bytes) => Some(u64::from_be_bytes(bytes.as_slice().try_into().ok()?)),
        Value::Int(number) => u64::try_from(*number).ok(),
        _ => None,
    }
}

/// `AgentGroupDataUpdate`: `GroupData` is the list; `NewGroupData`, with
/// whether each shows in the profile, is of no use to the bot.
fn group_data(body: &Value) -> Option<Vec<Membership>> {
    let mut groups = Vec::new();
    for group in body.get("GroupData")?.as_array()? {
        groups.push(Membership {
            id: group.get("GroupID")?.as_uuid()?,
            name: group.str_field("GroupName")?.to_string(),
            powers: powers(group.get("GroupPowers")?)?,
            notices: flag(group.get("AcceptNotices")?)?,
        });
    }
    Some(groups)
}

/// `AgentDropGroup`: `AgentData`, an array of one map with `GroupID`.
fn dropped_group(body: &Value) -> Option<Uuid> {
    body.get("AgentData")?.as_array()?.first()?.get("GroupID")?.as_uuid()
}

/// `ChatterBoxInvitation`: the line is the instant message in
/// `instantmessage.message_params`.
fn session_invite(body: &Value) -> Option<SessionLine> {
    let params = body.get("instantmessage")?.get("message_params")?;
    let bucket = match params.get("data").and_then(|data| data.get("binary_bucket")) {
        Some(Value::Binary(bytes)) => bytes.clone(),
        _ => Vec::new(),
    };
    Some(SessionLine {
        session: params.get("id")?.as_uuid()?,
        from: params.get("from_id")?.as_uuid()?,
        from_name: params.str_field("from_name").unwrap_or_default().to_string(),
        dialog: u8::try_from(params.get("type")?.as_i64()?).ok()?,
        text: params.str_field("message")?.to_string(),
        bucket,
        offline: params.get("offline").and_then(flag).unwrap_or(false),
        timestamp: params
            .get("timestamp")
            .and_then(Value::as_i64)
            .and_then(|stamp| u32::try_from(stamp).ok())
            .unwrap_or(0),
    })
}

/// `ChatterBoxSessionStartReply`.
fn session_started(body: &Value) -> Option<EqEvent> {
    Some(EqEvent::SessionStarted {
        session: body.get("session_id")?.as_uuid()?,
        ok: flag(body.get("success")?)?,
        error: body.str_field("error").unwrap_or_default().to_string(),
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
            ("TeleportFinish", Some(body)) => {
                EqEvent::TeleportFinish(teleport_finish(body).ok_or("TeleportFinish event is missing fields")?)
            }
            (known @ ("AgentGroupDataUpdate" | "AgentDropGroup" | "ChatterBoxInvitation" | "ChatterBoxSessionStartReply"), body) => {
                let read = body.and_then(|body| match known {
                    "AgentGroupDataUpdate" => group_data(body).map(EqEvent::Groups),
                    "AgentDropGroup" => dropped_group(body).map(EqEvent::GroupDropped),
                    "ChatterBoxInvitation" => session_invite(body).map(EqEvent::SessionInvite),
                    _ => session_started(body),
                });
                match read {
                    Some(event) => event,
                    None => {
                        // One malformed event is no reason to stop polling.
                        crate::debug!("event queue: skipping a {known} event that is missing fields");
                        EqEvent::Other(known.to_string())
                    }
                }
            }
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

/// The body of the request that takes up the chat session `session`, as
/// a viewer posts it when it shows a session's first line.
pub fn accept_invitation(session: Uuid) -> String {
    let mut map = BTreeMap::new();
    map.insert("method".to_string(), Value::Str("accept invitation".to_string()));
    map.insert("session-id".to_string(), Value::Uuid(session));
    xml::write_llsd(&Value::Map(map))
}

/// A chat session to take up, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeUp {
    pub url: String,
    pub session: Uuid,
}

/// Starts the worker that takes chat sessions up. Its failures are only
/// logged: the lines the grid delivers are recorded either way.
pub fn spawn_take_up() -> std::sync::mpsc::Sender<TakeUp> {
    let (sender, requests) = std::sync::mpsc::channel::<TakeUp>();
    thread::spawn(move || {
        let agent = http_agent(Duration::from_secs(30));
        for TakeUp { url, session } in requests {
            match post(&agent, &url, &accept_invitation(session)) {
                Ok((200, _)) => crate::debug!("took up the chat session {session}"),
                Ok((status, _)) => crate::info!("the grid answered HTTP {status} to taking up the chat session {session}"),
                Err(e) => crate::info!("the chat session {session} could not be taken up: {e}"),
            }
        }
    });
    sender
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
                    let found = [
                        CapsEvent::DisplayNames(seed.display_names),
                        CapsEvent::AvatarSearch(seed.avatar_search),
                        CapsEvent::Profiles(seed.profiles),
                        CapsEvent::Inventory(seed.inventory),
                        CapsEvent::ChatSessions(seed.chat_sessions),
                    ];
                    for event in found {
                        if !emit(event) {
                            return;
                        }
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

    fn events_of(llsd: &str) -> Vec<EqEvent> {
        let body = format!("<llsd><map><key>events</key><array>{llsd}</array><key>id</key><integer>7</integer></map></llsd>");
        parse_events(&body).unwrap().1
    }

    const BUILDERS: &str = "b1b1b1b1-b1b1-b1b1-b1b1-b1b1b1b1b1b1";
    const DANCERS: &str = "d1d1d1d1-d1d1-d1d1-d1d1-d1d1d1d1d1d1";
    const JANE: &str = "33333333-3333-3333-3333-333333333333";

    fn uuid(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    /// `AgentGroupDataUpdate` as OpenSim's event queue sends it: the
    /// powers as eight bytes, big-endian.
    fn group_data_event() -> String {
        format!(
            "<map><key>message</key><string>AgentGroupDataUpdate</string><key>body</key><map>\
               <key>AgentData</key><array><map><key>AgentID</key><uuid>{JANE}</uuid></map></array>\
               <key>GroupData</key><array>\
                 <map><key>GroupID</key><uuid>{BUILDERS}</uuid><key>GroupPowers</key><binary>AAAAAQAAAAI=</binary>\
                   <key>AcceptNotices</key><boolean>1</boolean><key>GroupInsigniaID</key><uuid>{JANE}</uuid>\
                   <key>Contribution</key><integer>0</integer><key>GroupName</key><string>Builders</string></map>\
                 <map><key>GroupID</key><uuid>{DANCERS}</uuid><key>GroupPowers</key><binary>AAAAAAAAAAA=</binary>\
                   <key>AcceptNotices</key><boolean>0</boolean><key>GroupInsigniaID</key><uuid>{JANE}</uuid>\
                   <key>Contribution</key><integer>0</integer><key>GroupName</key><string>Dancers</string></map>\
               </array>\
               <key>NewGroupData</key><array><map><key>ListInProfile</key><boolean>1</boolean></map>\
                 <map><key>ListInProfile</key><boolean>0</boolean></map></array>\
             </map></map>"
        )
    }

    #[test]
    fn the_groups_of_the_avatar_come_as_an_event() {
        let events = events_of(&group_data_event());
        assert_eq!(
            events,
            [EqEvent::Groups(vec![
                Membership {
                    id: uuid(BUILDERS),
                    name: "Builders".into(),
                    powers: 0x0000_0001_0000_0002,
                    notices: true,
                },
                Membership {
                    id: uuid(DANCERS),
                    name: "Dancers".into(),
                    powers: 0,
                    notices: false,
                },
            ])]
        );
        let none = "<map><key>message</key><string>AgentGroupDataUpdate</string><key>body</key><map>\
                    <key>GroupData</key><array /><key>NewGroupData</key><array /></map></map>";
        assert_eq!(events_of(none), [EqEvent::Groups(vec![])]);
    }

    #[test]
    fn a_dropped_group_comes_as_an_event() {
        let event = format!(
            "<map><key>message</key><string>AgentDropGroup</string><key>body</key><map>\
               <key>AgentData</key><array><map><key>AgentID</key><uuid>{JANE}</uuid>\
               <key>GroupID</key><uuid>{DANCERS}</uuid></map></array></map></map>"
        );
        assert_eq!(events_of(&event), [EqEvent::GroupDropped(uuid(DANCERS))]);
    }

    #[test]
    fn the_first_line_of_a_session_comes_as_an_invitation() {
        let event = format!(
            "<map><key>message</key><string>ChatterBoxInvitation</string><key>body</key><map>\
               <key>session_id</key><uuid>{BUILDERS}</uuid><key>from_name</key><string>Jane Doe</string>\
               <key>session_name</key><string>Builders</string><key>from_id</key><uuid>{JANE}</uuid>\
               <key>instantmessage</key><map><key>message_params</key><map>\
                 <key>type</key><integer>17</integer><key>position</key><array><real>0</real><real>0</real><real>0</real></array>\
                 <key>region_id</key><uuid>00000000-0000-0000-0000-000000000000</uuid>\
                 <key>to_id</key><uuid>a1a1a1a1-a1a1-a1a1-a1a1-a1a1a1a1a1a1</uuid><key>source</key><integer>0</integer>\
                 <key>data</key><map><key>binary_bucket</key><binary>QnVpbGRlcnMA</binary></map>\
                 <key>message</key><string>hello</string><key>id</key><uuid>{BUILDERS}</uuid>\
                 <key>from_name</key><string>Jane Doe</string><key>timestamp</key><integer>0</integer>\
                 <key>offline</key><integer>0</integer><key>parent_estate_id</key><integer>1</integer>\
                 <key>ttl</key><integer>0</integer><key>from_id</key><uuid>{JANE}</uuid>\
                 <key>from_group</key><boolean>0</boolean></map>\
               <key>agent_params</key><map><key>agent_id</key><uuid>{JANE}</uuid></map></map>\
             </map></map>"
        );
        assert_eq!(
            events_of(&event),
            [EqEvent::SessionInvite(SessionLine {
                session: uuid(BUILDERS),
                from: uuid(JANE),
                from_name: "Jane Doe".into(),
                dialog: 17,
                text: "hello".into(),
                bucket: b"Builders\0".to_vec(),
                offline: false,
                timestamp: 0,
            })]
        );
    }

    #[test]
    fn the_answer_to_starting_a_session_comes_as_an_event() {
        let ok = format!(
            "<map><key>message</key><string>ChatterBoxSessionStartReply</string><key>body</key><map>\
               <key>session_id</key><uuid>{BUILDERS}</uuid><key>temp_session_id</key><uuid>{BUILDERS}</uuid>\
               <key>success</key><boolean>1</boolean><key>session_info</key><map>\
               <key>session_name</key><string>Builders</string></map></map></map>"
        );
        assert_eq!(
            events_of(&ok),
            [EqEvent::SessionStarted {
                session: uuid(BUILDERS),
                ok: true,
                error: String::new()
            }]
        );
        let refused = format!(
            "<map><key>message</key><string>ChatterBoxSessionStartReply</string><key>body</key><map>\
               <key>session_id</key><uuid>{BUILDERS}</uuid><key>temp_session_id</key><uuid>{BUILDERS}</uuid>\
               <key>success</key><boolean>0</boolean><key>error</key><string>not a member</string></map></map>"
        );
        assert_eq!(
            events_of(&refused),
            [EqEvent::SessionStarted {
                session: uuid(BUILDERS),
                ok: false,
                error: "not a member".into()
            }]
        );
    }

    #[test]
    fn an_event_missing_a_field_is_skipped_and_the_rest_are_read() {
        let broken = "<map><key>message</key><string>AgentGroupDataUpdate</string><key>body</key><map>\
                      <key>GroupData</key><array><map><key>GroupName</key><string>X</string></map></array>\
                      </map></map>";
        let no_body = "<map><key>message</key><string>ChatterBoxSessionStartReply</string></map>";
        let events = events_of(&format!("{broken}{no_body}{}", group_data_event()));
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], EqEvent::Other("AgentGroupDataUpdate".into()));
        assert_eq!(events[1], EqEvent::Other("ChatterBoxSessionStartReply".into()));
        assert!(matches!(&events[2], EqEvent::Groups(groups) if groups.len() == 2));
    }

    #[test]
    fn the_chat_session_service_is_read_from_the_seed() {
        let reply = "<llsd><map><key>EventQueueGet</key><string>http://sim/eq</string>\
                     <key>ChatSessionRequest</key><string>http://sim/chat</string></map></llsd>";
        assert_eq!(parse_seed_reply(reply).unwrap().chat_sessions.as_deref(), Some("http://sim/chat"));
        let without = "<llsd><map><key>EventQueueGet</key><string>http://sim/eq</string></map></llsd>";
        assert_eq!(parse_seed_reply(without).unwrap().chat_sessions, None);
        let body = accept_invitation(uuid(BUILDERS));
        let read = xml::parse_llsd(&body).unwrap();
        assert_eq!(read.str_field("method"), Some("accept invitation"));
        assert_eq!(read.get("session-id").and_then(Value::as_uuid), Some(uuid(BUILDERS)));
    }

    /// An event of the rig's captures, as the event queue delivers it.
    fn captured_event(name: &str, heading: &str) -> Vec<EqEvent> {
        let body = crate::udp::messages::testing::capture(heading);
        events_of(&format!("<map><key>message</key><string>{name}</string>{body}</map>"))
    }

    #[test]
    fn reads_the_events_captured_on_the_rig() {
        let free = uuid("4328d03e-69ce-4e18-81f4-ccf63ad9607e");
        let dancers = uuid("0f27615f-2550-44c7-a0ec-76a0cc585778");
        let owner = uuid("7e570000-0000-4000-8000-0000000000aa");
        // OpenSim writes an empty insignia and contribution.
        let [EqEvent::Groups(groups)] = &captured_event("AgentGroupDataUpdate", "AgentGroupDataUpdate, event")[..] else {
            panic!("not the groups")
        };
        let names: Vec<(Uuid, &str, bool)> = groups.iter().map(|g| (g.id, g.name.as_str(), g.notices)).collect();
        assert!(
            names.contains(&(free, "Free", true)) && names.contains(&(dancers, "Dancers", true)),
            "{names:?}"
        );
        let [EqEvent::SessionInvite(line)] = &captured_event("ChatterBoxInvitation", "ChatterBoxInvitation, event")[..] else {
            panic!("not a line")
        };
        assert_eq!((line.session, line.from, line.dialog), (free, owner, 17));
        assert_eq!((line.from_name.as_str(), line.text.as_str()), ("Rig Owner", "capture line one"));
        assert_eq!(
            captured_event("ChatterBoxSessionStartReply", "ChatterBoxSessionStartReply, event"),
            [EqEvent::SessionStarted {
                session: dancers,
                ok: true,
                error: String::new()
            }]
        );
    }

    #[test]
    fn seed_request_asks_for_the_event_queue() {
        assert!(seed_request().ends_with(
            "<llsd><array><string>EventQueueGet</string><string>GetDisplayNames</string>\
             <string>AvatarPickerSearch</string><string>AgentProfile</string><string>FetchInventoryDescendents2</string>\
             <string>FetchInventory2</string><string>InventoryAPIv3</string>\
             <string>UpdateAvatarAppearance</string><string>ChatSessionRequest</string></array></llsd>"
        ));
    }

    #[test]
    fn seed_reply_may_carry_the_inventory_services() {
        let entry = |name: &str| format!("<key>{name}</key><string>http://sim/CAPS/{name}/</string>");
        let reply = |names: &[&str]| {
            let entries: String = names.iter().map(|name| entry(name)).collect();
            format!("<llsd><map>{}{entries}</map></llsd>", entry("EventQueueGet"))
        };
        let url = |name: &str| Some(format!("http://sim/CAPS/{name}/"));

        // None of them: a simulator without an inventory service.
        let seed = parse_seed_reply(&reply(&[])).unwrap();
        assert_eq!(seed.inventory, InventoryUrls::default());
        assert!(!seed.inventory.outfit_service());

        // The two that read, as on OpenSimulator.
        let seed = parse_seed_reply(&reply(&["FetchInventoryDescendents2", "FetchInventory2"])).unwrap();
        assert_eq!(seed.inventory.descendents, url("FetchInventoryDescendents2"));
        assert_eq!(seed.inventory.items, url("FetchInventory2"));
        assert_eq!((&seed.inventory.api, &seed.inventory.appearance), (&None, &None));
        assert!(!seed.inventory.outfit_service());

        // All four, as on Second Life.
        let all = [
            "FetchInventoryDescendents2",
            "FetchInventory2",
            "InventoryAPIv3",
            "UpdateAvatarAppearance",
        ];
        let seed = parse_seed_reply(&reply(&all)).unwrap();
        assert_eq!(seed.inventory.api, url("InventoryAPIv3"));
        assert_eq!(seed.inventory.appearance, url("UpdateAvatarAppearance"));
        assert!(seed.inventory.outfit_service());
        // One of the two alone is no outfit service.
        let seed = parse_seed_reply(&reply(&["InventoryAPIv3"])).unwrap();
        assert!(!seed.inventory.outfit_service());
        // An empty URL is no URL.
        let empty = "<llsd><map><key>EventQueueGet</key><string>http://sim/eq</string>\
                     <key>FetchInventory2</key><string></string></map></llsd>";
        assert_eq!(parse_seed_reply(empty).unwrap().inventory.items, None);
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
    fn seed_reply_may_carry_the_profile_service() {
        let bare = "<llsd><map><key>EventQueueGet</key>\
                    <string>http://127.0.0.1:9000/CAPS/EQG/abc/</string></map></llsd>";
        assert_eq!(parse_seed_reply(bare).unwrap().profiles, None);
        let with = |url: &str| bare.replace("</map>", &format!("<key>AgentProfile</key><string>{url}</string></map>"));
        let seed = parse_seed_reply(&with("https://example.com/cap/profile")).unwrap();
        assert_eq!(seed.profiles.as_deref(), Some("https://example.com/cap/profile"));
        // An empty URL is none.
        assert_eq!(parse_seed_reply(&with("")).unwrap().profiles, None);
        assert!(seed_request().contains("<string>AgentProfile</string>"));
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
