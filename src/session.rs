//! One logged-in session: the simulator circuit, its handshake and
//! keepalive, teleports with the hand-over to a new simulator, sitting, and
//! seat tracking. All I/O goes through `Net`, so the logic is testable with
//! a fake clock and a recording network.

use crate::caps::{CapsEvent, EqEvent, TeleportFinish};
use crate::login::SessionInfo;
use crate::reconciler::same_region;
use crate::udp::circuit::Circuit;
use crate::udp::messages::{self, Agent, Incoming, Outgoing, Vector3};
use crate::udp::packet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const SILENCE_LIMIT: Duration = Duration::from_secs(60);
pub const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
pub const SIT_LIMIT: Duration = Duration::from_secs(15);
pub const TELEPORT_LIMIT: Duration = Duration::from_secs(60);
pub const LOGOUT_LIMIT: Duration = Duration::from_secs(5);
const AGENT_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Bits per second for resend, land, wind, cloud, task, texture and asset.
/// Only the task channel matters to the bot: it carries the avatar's own
/// object updates, which seat tracking depends on. Checked against OpenSim:
/// with these values the update arrives within a second of sitting or
/// standing.
const THROTTLES: [f32; 7] = [20_000.0, 0.0, 0.0, 0.0, 30_000.0, 0.0, 10_000.0];
/// A short draw distance keeps the simulator from streaming the whole scene.
const DRAW_DISTANCE: f32 = 32.0;
/// Why the session will not send a message for the operator right now.
pub const NOT_CONNECTED: &str = "the avatar is not connected to a region";
/// MapBlockReply marks "no such region" with this access level.
const ACCESS_NONEXISTENT: u8 = 255;

/// The network as the session sees it. Each simulator connection is a
/// circuit with its own generation number; events coming back from the
/// network carry that number so stale ones can be told apart.
pub trait Net {
    /// Opens a UDP circuit to `addr` and starts listening on its event queue.
    fn open(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str);
    fn send(&mut self, generation: u64, datagram: &[u8]);
    fn close(&mut self, generation: u64);
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// The avatar is connected and present in the named region.
    RegionEntered(String),
    /// The session is over; nothing more will be sent.
    Lost(String),
    /// The avatar is seated on the target object.
    Seated,
    /// The avatar is known not to be seated on the target object.
    Standing,
    SitFailed(String),
    /// A teleport inside the current region finished.
    TeleportArrived,
    TeleportFailed(String),
    /// The logout was confirmed, or its wait ran out.
    LoggedOut,
    /// Text the simulator sent for the user to read, such as why a sit was
    /// refused or that the region is about to restart.
    Alert(String),
    /// Local chat the avatar heard.
    Chat(Message),
    /// An instant message sent to the avatar.
    InstantMessage(Message),
    /// Where the current simulator answers display-name lookups, if it does.
    DisplayNamesUrl(Option<String>),
    /// Where the current simulator answers avatar searches over HTTP, if it
    /// does.
    AvatarSearchUrl(Option<String>),
    /// The answer to `find_avatar`: the avatars the grid offers for the
    /// name, as id and `First Last`.
    AvatarsFound { query: Uuid, avatars: Vec<(Uuid, String)> },
}

/// A chat or instant message as the simulator delivered it; the names that
/// are not part of the message are looked up later.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// `say`, `shout`, ... for chat; `im` or `object` for instant messages
    /// received and `im_out` for one sent.
    pub kind: String,
    pub source: Source,
    /// When a message stored while the avatar was offline was sent, in
    /// seconds since the Unix epoch.
    pub sent_at: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Avatar { id: Uuid, name: String },
    /// `id` is missing when the simulator does not say which object it was.
    Object { id: Option<Uuid>, name: String, owner: Uuid },
    /// The grid itself.
    System { name: String },
}

const CHAT_SOURCE_AGENT: u8 = 1;
const CHAT_SOURCE_OBJECT: u8 = 2;
const CHAT_TYPE_START_TYPING: u8 = 4;
const CHAT_TYPE_STOP_TYPING: u8 = 5;
const CHAT_NOT_AUDIBLE: i8 = -1;
const IM_NOTHING_SPECIAL: u8 = 0;
const IM_FROM_TASK: u8 = 19;

fn chat_kind(chat_type: u8) -> String {
    let kind = match chat_type {
        0 => "whisper",
        1 => "say",
        2 => "shout",
        6 => "debug",
        7 => "region",
        8 => "owner",
        9 => "direct",
        other => return format!("type-{other}"),
    };
    kind.to_string()
}

struct Sim {
    generation: u64,
    circuit: Circuit,
    opened_at: Instant,
    region_name: Option<String>,
    region_handle: Option<u64>,
    position: Option<Vector3>,
    /// Both halves of the handshake are done and the region is known.
    entered: bool,
    next_agent_update: Instant,
}

enum TeleportPhase {
    /// Waiting for the map to resolve the region name.
    Lookup,
    Requested,
}

struct Teleport {
    region: String,
    position: Vector3,
    phase: TeleportPhase,
    deadline: Instant,
}

pub struct Session<N: Net> {
    net: N,
    agent: Agent,
    /// `First Last`, the name instant messages are sent under.
    agent_name: String,
    circuit_code: u32,
    target_object: Uuid,
    sim: Sim,
    /// The simulator confirmed a sit on the target since the avatar last stood
    /// up: it answered our sit request, or named the target unprompted.
    sat_on_target: bool,
    /// Local id of what the avatar sits on, from its own object updates.
    own_parent: Option<u32>,
    seated: bool,
    seat_reported: bool,
    sit_deadline: Option<Instant>,
    teleport: Option<Teleport>,
    logout_deadline: Option<Instant>,
    /// Ask for the IMs stored while offline, once, on entering the first region.
    fetch_offline_im: bool,
    lost: bool,
    events: Vec<SessionEvent>,
}

impl<N: Net> Session<N> {
    /// `generation` numbers this session's first circuit; later circuits
    /// count up from it.
    pub fn new(
        net: N,
        info: &SessionInfo,
        agent_name: &str,
        target_object: Uuid,
        fetch_offline_im: bool,
        generation: u64,
        now: Instant,
    ) -> Session<N> {
        let mut session = Session {
            net,
            agent: Agent { agent_id: info.agent_id, session_id: info.session_id },
            agent_name: agent_name.to_string(),
            circuit_code: info.circuit_code,
            target_object,
            sim: Sim::new(0, now),
            sat_on_target: false,
            own_parent: None,
            seated: false,
            seat_reported: false,
            sit_deadline: None,
            teleport: None,
            logout_deadline: None,
            fetch_offline_im,
            lost: false,
            events: Vec::new(),
        };
        session.open_circuit(generation, info.sim_addr, &info.seed_capability, now);
        session
    }

    /// The generation of the current circuit.
    pub fn generation(&self) -> u64 {
        self.sim.generation
    }

    pub fn take_events(&mut self) -> Vec<SessionEvent> {
        std::mem::take(&mut self.events)
    }

    /// When `tick` next needs to run.
    pub fn next_deadline(&self) -> Instant {
        let sim = &self.sim;
        let mut deadline = sim.circuit.last_receive() + SILENCE_LIMIT;
        let mut consider = |candidate: Option<Instant>| {
            if let Some(at) = candidate {
                deadline = deadline.min(at);
            }
        };
        consider(sim.circuit.next_deadline());
        consider(if sim.entered { Some(sim.next_agent_update) } else { Some(sim.opened_at + HANDSHAKE_LIMIT) });
        consider(self.sit_deadline);
        consider(self.teleport.as_ref().map(|t| t.deadline));
        consider(self.logout_deadline);
        deadline
    }

    fn open_circuit(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str, now: Instant) {
        self.sim = Sim::new(generation, now);
        self.sat_on_target = false;
        self.own_parent = None;
        self.seated = false;
        self.seat_reported = false;
        self.sit_deadline = None;
        self.teleport = None;
        self.net.open(generation, addr, seed_capability);
        self.send(messages::use_circuit_code(&self.agent, self.circuit_code), true, now);
        self.send(messages::complete_agent_movement(&self.agent, self.circuit_code), true, now);
    }

    fn send(&mut self, message: Outgoing, reliable: bool, now: Instant) {
        crate::debug!("sending {}", messages::name(message.id));
        let datagram = self.sim.circuit.send(&message, reliable, now);
        self.net.send(self.sim.generation, &datagram);
    }

    fn lose(&mut self, reason: impl Into<String>) {
        if !self.lost {
            self.lost = true;
            self.net.close(self.sim.generation);
            // A session that ends while logging out has done what was asked;
            // simulators often drop the event queue before confirming.
            self.events.push(match self.logout_deadline.take() {
                Some(_) => SessionEvent::LoggedOut,
                None => SessionEvent::Lost(reason.into()),
            });
        }
    }

    // ------------------------------------------------------------ actions

    /// Asks to sit on the target object. Ends in `Seated` or `SitFailed`.
    pub fn start_sit(&mut self, now: Instant) {
        self.sat_on_target = false;
        self.sit_deadline = Some(now + SIT_LIMIT);
        self.send(messages::agent_request_sit(&self.agent, self.target_object), true, now);
    }

    /// Teleports to `position` in `region`. Ends in `RegionEntered` (another
    /// region), `TeleportArrived` (same region), `TeleportFailed` or `Lost`.
    pub fn start_teleport(&mut self, region: &str, position: Vector3, now: Instant) {
        let here = self.sim.region_name.as_deref().is_some_and(|name| same_region(name, region));
        let phase = match self.sim.region_handle {
            Some(handle) if here => {
                self.send(messages::teleport_location_request(&self.agent, handle, position), true, now);
                TeleportPhase::Requested
            }
            _ => {
                self.send(messages::map_name_request(&self.agent, region), true, now);
                TeleportPhase::Lookup
            }
        };
        self.teleport = Some(Teleport {
            region: region.to_string(),
            position,
            phase,
            deadline: now + TELEPORT_LIMIT,
        });
    }

    /// Tells the simulator the avatar is leaving. Ends in `LoggedOut`.
    pub fn start_logout(&mut self, now: Instant) {
        self.logout_deadline = Some(now + LOGOUT_LIMIT);
        self.send(messages::logout_request(&self.agent), true, now);
    }

    /// Whether the avatar is where it can talk: in a region, and not on its
    /// way out of it.
    pub fn present(&self) -> Result<(), String> {
        let here = self.sim.entered && !self.lost && self.teleport.is_none() && self.logout_deadline.is_none();
        if here {
            Ok(())
        } else {
            Err(NOT_CONNECTED.to_string())
        }
    }

    /// Says `text` in local chat. `chat_type` is 0 whisper, 1 say, 2 shout.
    pub fn say(&mut self, channel: i32, chat_type: u8, text: &str, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::chat_from_viewer(&self.agent, text, chat_type, channel), true, now);
        Ok(())
    }

    /// Sends the avatar `to` an instant message.
    pub fn instant_message(&mut self, to: Uuid, text: &str, now: Instant) -> Result<(), String> {
        self.present()?;
        let position = self.sim.position.unwrap_or_default();
        let message = messages::instant_message(&self.agent, &self.agent_name, to, position, text);
        self.send(message, true, now);
        Ok(())
    }

    /// Asks the grid which avatars go by `name`. Ends in `AvatarsFound` with
    /// the same `query`, if the grid answers.
    pub fn find_avatar(&mut self, query: Uuid, name: &str, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::avatar_picker_request(&self.agent, query, name), true, now);
        Ok(())
    }

    // ------------------------------------------------------------- timers

    pub fn tick(&mut self, now: Instant) {
        if self.lost {
            return;
        }
        let poll = self.sim.circuit.poll(now);
        for datagram in &poll.datagrams {
            self.net.send(self.sim.generation, datagram);
        }
        if poll.exhausted {
            return self.lose("simulator stopped acknowledging packets");
        }
        if now.duration_since(self.sim.circuit.last_receive()) >= SILENCE_LIMIT {
            return self.lose("nothing received from the simulator for 60 s");
        }
        if !self.sim.entered && now.duration_since(self.sim.opened_at) >= HANDSHAKE_LIMIT {
            return self.lose("simulator handshake timed out");
        }
        if self.sim.entered && now >= self.sim.next_agent_update {
            self.sim.next_agent_update = now + AGENT_UPDATE_INTERVAL;
            let camera = self.sim.position.unwrap_or_default();
            self.send(messages::agent_update(&self.agent, camera, DRAW_DISTANCE), false, now);
        }
        if self.sit_deadline.is_some_and(|deadline| now >= deadline) {
            self.sit_deadline = None;
            self.events.push(SessionEvent::SitFailed(
                "the simulator did not confirm the sit within 15 s".to_string(),
            ));
        }
        if self.teleport.as_ref().is_some_and(|t| now >= t.deadline) {
            self.teleport = None;
            self.events.push(SessionEvent::TeleportFailed("no answer within 60 s".to_string()));
        }
        if self.logout_deadline.is_some_and(|deadline| now >= deadline) {
            self.logout_deadline = None;
            self.events.push(SessionEvent::LoggedOut);
        }
    }

    // ------------------------------------------------------------- events

    pub fn handle_datagram(&mut self, generation: u64, datagram: &[u8], now: Instant) {
        if self.lost || generation != self.sim.generation {
            return;
        }
        let packet = match packet::parse(datagram) {
            Ok(packet) => packet,
            Err(e) => {
                crate::debug!("dropping malformed packet: {e}");
                return;
            }
        };
        crate::debug!("received {} ({} bytes)", messages::name(packet.id), datagram.len());
        if !self.sim.circuit.receive(&packet, now) {
            return;
        }
        match messages::decode(packet.id, &packet.body) {
            Ok(message) => self.handle_message(message, now),
            Err(e) => crate::debug!("dropping {}: {e}", messages::name(packet.id)),
        }
    }

    pub fn handle_caps(&mut self, generation: u64, event: CapsEvent, now: Instant) {
        if self.lost || generation != self.sim.generation {
            return;
        }
        match event {
            CapsEvent::Gone(reason) => self.lose(reason),
            CapsEvent::Event(EqEvent::TeleportFinish(finish)) => self.hand_over(finish, now),
            CapsEvent::Event(EqEvent::Other(name)) => crate::debug!("event queue: ignoring {name}"),
            CapsEvent::DisplayNames(url) => self.events.push(SessionEvent::DisplayNamesUrl(url)),
            CapsEvent::AvatarSearch(url) => self.events.push(SessionEvent::AvatarSearchUrl(url)),
        }
    }

    /// The avatar now belongs to another simulator: drop this circuit and
    /// connect there. Whatever was in progress here is void.
    fn hand_over(&mut self, finish: TeleportFinish, now: Instant) {
        crate::debug!("teleport finishing at {}", finish.sim_addr);
        self.net.close(self.sim.generation);
        self.open_circuit(self.sim.generation + 1, finish.sim_addr, &finish.seed_capability, now);
        self.sim.region_handle = Some(finish.region_handle);
    }

    fn handle_message(&mut self, message: Incoming, now: Instant) {
        match message {
            Incoming::StartPingCheck { ping_id } => {
                self.send(messages::complete_ping_check(ping_id), false, now);
            }
            Incoming::PacketAck(sequences) => self.sim.circuit.acknowledged(&sequences),
            Incoming::RegionHandshake { sim_name } => {
                self.sim.region_name = Some(sim_name);
                self.send(messages::region_handshake_reply(&self.agent), true, now);
                self.send(messages::agent_throttle(&self.agent, self.circuit_code, THROTTLES), true, now);
                self.check_entered(now);
            }
            Incoming::AgentMovementComplete { position, region_handle } => {
                self.sim.position = Some(position);
                self.sim.region_handle = Some(region_handle);
                self.check_entered(now);
            }
            Incoming::MapBlockReply(blocks) => self.handle_map_blocks(&blocks, now),
            Incoming::TeleportStart => crate::debug!("teleport started"),
            Incoming::TeleportProgress { message } => crate::debug!("teleport progress: {message}"),
            Incoming::TeleportFailed { reason } => {
                if self.teleport.take().is_some() {
                    self.events.push(SessionEvent::TeleportFailed(reason));
                }
            }
            Incoming::TeleportLocal { position } => {
                self.sim.position = Some(position);
                if self.teleport.take().is_some() {
                    self.events.push(SessionEvent::TeleportArrived);
                }
            }
            Incoming::AvatarSitResponse { object, autopilot } => {
                // The response names the prim the simulator seated the avatar
                // on, which may be another prim of the target's linkset (the
                // one with the sit target, or the root). While a sit is in
                // progress it answers our request, whatever it names.
                if self.sit_deadline.is_some() {
                    self.sat_on_target = true;
                    crate::debug!("sit accepted on {object} (autopilot: {autopilot})");
                    self.send(messages::agent_sit(&self.agent), true, now);
                } else {
                    self.sat_on_target = object == self.target_object;
                    if !self.sat_on_target {
                        crate::debug!("unrequested sit on {object}, not the target");
                    }
                }
                self.update_seat();
            }
            Incoming::ObjectUpdate(objects) => {
                let own_id = self.agent.agent_id;
                for object in objects.iter().filter(|o| o.full_id == own_id) {
                    let stood_up = object.parent_id == 0 && self.own_parent.is_some_and(|p| p != 0);
                    if stood_up {
                        self.sat_on_target = false;
                    }
                    if self.own_parent != Some(object.parent_id) {
                        crate::debug!("own avatar's parent is now {}", object.parent_id);
                    }
                    self.own_parent = Some(object.parent_id);
                    self.update_seat();
                }
            }
            Incoming::KickUser { reason } => self.lose(format!("kicked: {reason}")),
            Incoming::Alert { message } => self.events.push(SessionEvent::Alert(message)),
            Incoming::Chat { from_name, source_id, owner_id, source_type, chat_type, audible, text } => {
                let typing = matches!(chat_type, CHAT_TYPE_START_TYPING | CHAT_TYPE_STOP_TYPING);
                if typing || text.is_empty() || audible == CHAT_NOT_AUDIBLE {
                    return;
                }
                let source = match source_type {
                    CHAT_SOURCE_AGENT => Source::Avatar { id: source_id, name: from_name },
                    CHAT_SOURCE_OBJECT => {
                        Source::Object { id: Some(source_id), name: from_name, owner: owner_id }
                    }
                    _ => Source::System { name: from_name },
                };
                let kind = chat_kind(chat_type);
                self.events.push(SessionEvent::Chat(Message { kind, source, sent_at: None, text }));
            }
            Incoming::InstantMessage { from_agent, from_name, dialog, id, offline, timestamp, text } => {
                let (kind, source) = match dialog {
                    IM_NOTHING_SPECIAL if from_agent.is_nil() => ("im", Source::System { name: from_name }),
                    IM_NOTHING_SPECIAL => ("im", Source::Avatar { id: from_agent, name: from_name }),
                    // The sender is the object's owner and the id is the object.
                    IM_FROM_TASK => {
                        let id = (!id.is_nil()).then_some(id);
                        ("object", Source::Object { id, name: from_name, owner: from_agent })
                    }
                    other => return crate::debug!("ignoring instant message with dialog {other}"),
                };
                let sent_at = (offline && timestamp != 0).then_some(timestamp);
                let message = Message { kind: kind.to_string(), source, sent_at, text };
                self.events.push(SessionEvent::InstantMessage(message));
            }
            Incoming::AvatarPickerReply { query, avatars } => {
                self.events.push(SessionEvent::AvatarsFound { query, avatars });
            }
            Incoming::LogoutReply => {
                if self.logout_deadline.take().is_some() {
                    self.events.push(SessionEvent::LoggedOut);
                } else {
                    self.lose("the simulator logged the avatar out");
                }
            }
            Incoming::DisableSimulator => self.lose("the simulator closed the connection"),
            Incoming::Other => {}
        }
    }

    fn check_entered(&mut self, now: Instant) {
        if self.sim.entered || self.sim.position.is_none() {
            return;
        }
        if let Some(name) = self.sim.region_name.clone() {
            self.sim.entered = true;
            self.sim.next_agent_update = now;
            self.events.push(SessionEvent::RegionEntered(name));
            if std::mem::take(&mut self.fetch_offline_im) {
                self.send(messages::retrieve_instant_messages(&self.agent), true, now);
            }
        }
    }

    fn handle_map_blocks(&mut self, blocks: &[messages::MapBlock], now: Instant) {
        let Some(teleport) = &mut self.teleport else { return };
        if !matches!(teleport.phase, TeleportPhase::Lookup) {
            return;
        }
        let found = blocks
            .iter()
            .find(|b| same_region(&b.name, &teleport.region) && (b.x != 0 || b.y != 0));
        if let Some(block) = found {
            // Region handles are the region's south-west corner in metres.
            let handle = ((u64::from(block.x) * 256) << 32) | (u64::from(block.y) * 256);
            teleport.phase = TeleportPhase::Requested;
            let request = messages::teleport_location_request(&self.agent, handle, teleport.position);
            self.send(request, true, now);
        } else if blocks.iter().any(|b| b.access == ACCESS_NONEXISTENT || (b.x == 0 && b.y == 0)) {
            let region = self.teleport.take().map(|t| t.region).unwrap_or_default();
            self.events.push(SessionEvent::TeleportFailed(format!("region \"{region}\" not found")));
        }
    }

    /// Re-evaluates the seat from what the simulator has reported: seated on
    /// the target means it confirmed a sit there and shows the avatar parented.
    fn update_seat(&mut self) {
        let seated = self.sat_on_target && self.own_parent.is_some_and(|p| p != 0);
        if seated && !self.seated {
            self.sit_deadline = None;
            self.events.push(SessionEvent::Seated);
        } else if !seated && (self.seated || (!self.seat_reported && self.own_parent.is_some())) {
            self.events.push(SessionEvent::Standing);
        } else {
            return;
        }
        self.seated = seated;
        self.seat_reported = true;
    }
}

impl Sim {
    fn new(generation: u64, now: Instant) -> Sim {
        Sim {
            generation,
            circuit: Circuit::new(now),
            opened_at: now,
            region_name: None,
            region_handle: None,
            position: None,
            entered: false,
            next_agent_update: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::messages::testing as wire;
    use crate::udp::packet::{Packet, FLAG_RELIABLE};
    use std::cell::RefCell;
    use std::rc::Rc;

    const HOME_HANDLE: u64 = ((1000 * 256) << 32) | (1000 * 256);

    #[derive(Debug, PartialEq)]
    enum Call {
        Open(u64, SocketAddr, String),
        Close(u64),
    }

    #[derive(Default)]
    struct Recorder {
        calls: Vec<Call>,
        sent: Vec<(u64, Packet)>,
    }

    #[derive(Clone, Default)]
    struct FakeNet(Rc<RefCell<Recorder>>);

    impl Net for FakeNet {
        fn open(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str) {
            self.0.borrow_mut().calls.push(Call::Open(generation, addr, seed_capability.to_string()));
        }
        fn send(&mut self, generation: u64, datagram: &[u8]) {
            self.0.borrow_mut().sent.push((generation, packet::parse(datagram).unwrap()));
        }
        fn close(&mut self, generation: u64) {
            self.0.borrow_mut().calls.push(Call::Close(generation));
        }
    }

    struct Rig {
        session: Session<FakeNet>,
        net: FakeNet,
        now: Instant,
        /// Sequence number for packets "from the simulator".
        sequence: u32,
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn agent_id() -> Uuid {
        Uuid::from_bytes([0xA1; 16])
    }

    fn target() -> Uuid {
        Uuid::from_bytes([0x0B; 16])
    }

    impl Rig {
        fn new() -> Rig {
            Rig::with_offline_im(false)
        }

        fn with_offline_im(fetch_offline_im: bool) -> Rig {
            let net = FakeNet::default();
            let info = SessionInfo {
                agent_id: agent_id(),
                session_id: Uuid::from_bytes([0x5E; 16]),
                circuit_code: 77,
                sim_addr: "127.0.0.1:9000".parse().unwrap(),
                seed_capability: "http://127.0.0.1:9000/CAPS/seed/".to_string(),
            };
            let now = Instant::now();
            let session = Session::new(net.clone(), &info, "Sit Bot", target(), fetch_offline_im, 1, now);
            Rig { session, net, now, sequence: 0 }
        }

        /// A session that has completed the handshake in region "Home",
        /// with everything sent so far acknowledged.
        fn entered() -> Rig {
            let mut rig = Rig::new();
            rig.enter("Home", HOME_HANDLE);
            assert_eq!(rig.events(), [SessionEvent::RegionEntered("Home".into())]);
            rig
        }

        fn agent(&self) -> Agent {
            self.session.agent
        }

        fn generation(&self) -> u64 {
            self.session.generation()
        }

        fn enter(&mut self, region: &str, handle: u64) {
            self.receive(messages::REGION_HANDSHAKE, &wire::region_handshake(region));
            let body = wire::agent_movement_complete(&self.agent(), [10.0, 20.0, 30.0], handle);
            self.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
            self.ack_all();
            self.sent();
        }

        fn receive(&mut self, id: packet::MessageId, body: &[u8]) {
            let generation = self.generation();
            self.receive_from(generation, id, body, true);
        }

        fn receive_from(&mut self, generation: u64, id: packet::MessageId, body: &[u8], reliable: bool) {
            self.sequence += 1;
            let flags = if reliable { FLAG_RELIABLE } else { 0 };
            let datagram = packet::build(flags, self.sequence, id, body, false);
            self.session.handle_datagram(generation, &datagram, self.now);
        }

        /// Acknowledges every sequence number the session could have used.
        fn ack_all(&mut self) {
            let sequences: Vec<u32> = (1..=200).collect();
            let body = messages::packet_ack(&sequences).body;
            self.receive_from(self.generation(), messages::PACKET_ACK, &body, false);
        }

        fn advance(&mut self, by: Duration) {
            self.now += by;
            self.session.tick(self.now);
        }

        fn events(&mut self) -> Vec<SessionEvent> {
            self.session.take_events()
        }

        /// Packets sent since the last call, without acks and agent updates.
        fn sent(&mut self) -> Vec<Packet> {
            std::mem::take(&mut self.net.0.borrow_mut().sent)
                .into_iter()
                .map(|(_, packet)| packet)
                .filter(|p| p.id != messages::PACKET_ACK && p.id != messages::AGENT_UPDATE)
                .collect()
        }

        fn sent_ids(&mut self) -> Vec<packet::MessageId> {
            self.sent().iter().map(|p| p.id).collect()
        }

        fn calls(&mut self) -> Vec<Call> {
            std::mem::take(&mut self.net.0.borrow_mut().calls)
        }

        fn own_update(&mut self, parent: u32) {
            let body = wire::object_update(&[(Uuid::from_bytes([9; 16]), 5), (agent_id(), parent)]);
            self.receive(messages::OBJECT_UPDATE, &body);
        }

        fn sit_response(&mut self, object: Uuid) {
            self.receive(messages::AVATAR_SIT_RESPONSE, &wire::avatar_sit_response(object, false));
        }

        fn seated() -> Rig {
            let mut rig = Rig::entered();
            rig.session.start_sit(rig.now);
            rig.sit_response(target());
            rig.own_update(4242);
            assert_eq!(rig.events(), [SessionEvent::Seated]);
            rig.ack_all();
            rig.sent();
            rig
        }

        fn lost_reason(&mut self) -> String {
            match &self.events()[..] {
                [SessionEvent::Lost(reason)] => reason.clone(),
                other => panic!("expected exactly one Lost event, got {other:?}"),
            }
        }
    }

    // ------------------------------------------------- handshake, keepalive

    #[test]
    fn opens_the_circuit_and_completes_the_handshake() {
        let mut rig = Rig::new();
        assert_eq!(
            rig.calls(),
            [Call::Open(1, "127.0.0.1:9000".parse().unwrap(), "http://127.0.0.1:9000/CAPS/seed/".into())]
        );
        let sent = rig.sent();
        assert_eq!(
            sent.iter().map(|p| p.id).collect::<Vec<_>>(),
            [messages::USE_CIRCUIT_CODE, messages::COMPLETE_AGENT_MOVEMENT]
        );
        assert!(sent.iter().all(Packet::is_reliable));

        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Target"));
        assert_eq!(rig.sent_ids(), [messages::REGION_HANDSHAKE_REPLY, messages::AGENT_THROTTLE]);
        assert_eq!(rig.events(), []);

        let body = wire::agent_movement_complete(&rig.agent(), [1.0, 2.0, 3.0], HOME_HANDLE);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Target".into())]);
    }

    #[test]
    fn handshake_messages_may_arrive_in_either_order() {
        let mut rig = Rig::new();
        let body = wire::agent_movement_complete(&rig.agent(), [1.0, 2.0, 3.0], HOME_HANDLE);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        assert_eq!(rig.events(), []);
        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Target"));
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Target".into())]);
    }

    #[test]
    fn handshake_that_never_completes_is_a_loss() {
        let mut rig = Rig::new();
        rig.ack_all();
        rig.advance(secs(29));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        assert!(rig.lost_reason().contains("handshake"));
    }

    #[test]
    fn answers_pings() {
        let mut rig = Rig::entered();
        rig.receive_from(1, messages::START_PING_CHECK, &[42, 0, 0, 0, 0], false);
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].id, sent[0].body.as_slice()), (messages::COMPLETE_PING_CHECK, &[42u8][..]));
        assert!(!sent[0].is_reliable());
    }

    #[test]
    fn sends_an_agent_update_every_second_once_entered() {
        let mut rig = Rig::new();
        rig.ack_all();
        rig.advance(secs(2));
        let updates = |rig: &Rig| {
            let net = rig.net.0.borrow();
            net.sent.iter().filter(|(_, p)| p.id == messages::AGENT_UPDATE).count()
        };
        assert_eq!(updates(&rig), 0);

        rig.enter("Home", HOME_HANDLE);
        for _ in 0..5 {
            rig.advance(secs(1));
            rig.ack_all();
        }
        assert_eq!(updates(&rig), 5);
        assert_eq!(rig.session.next_deadline(), rig.now + secs(1));
    }

    #[test]
    fn acknowledges_reliable_packets_it_has_no_use_for() {
        let mut rig = Rig::entered();
        rig.receive(packet::low(9999), &[1, 2, 3]);
        let first = rig.sequence;
        rig.receive(packet::high(200), &[]);
        assert_eq!(rig.events(), []);
        rig.advance(Duration::from_millis(200));
        let net = rig.net.0.borrow();
        let acks: Vec<_> = net.sent.iter().filter(|(_, p)| p.id == messages::PACKET_ACK).collect();
        assert_eq!(acks.len(), 1);
        let Ok(Incoming::PacketAck(acked)) = messages::decode(acks[0].1.id, &acks[0].1.body) else {
            panic!("not a PacketAck")
        };
        assert!(acked.ends_with(&[first, first + 1]), "{acked:?}");
    }

    #[test]
    fn malformed_packets_are_ignored() {
        let mut rig = Rig::entered();
        rig.session.handle_datagram(1, &[1, 2, 3], rig.now);
        rig.receive(messages::REGION_HANDSHAKE, &[0; 3]);
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn duplicate_reliable_packets_are_processed_once() {
        let mut rig = Rig::entered();
        rig.session.start_sit(rig.now);
        rig.sent();
        let datagram = packet::build(
            FLAG_RELIABLE,
            900,
            messages::AVATAR_SIT_RESPONSE,
            &wire::avatar_sit_response(target(), false),
            false,
        );
        rig.session.handle_datagram(1, &datagram, rig.now);
        rig.session.handle_datagram(1, &datagram, rig.now);
        assert_eq!(rig.sent_ids(), [messages::AGENT_SIT]);
    }

    // ------------------------------------------------------------ loss

    #[test]
    fn kick_is_a_loss_with_the_reason() {
        let mut rig = Rig::entered();
        rig.receive(messages::KICK_USER, &wire::kick_user("You have been kicked"));
        assert_eq!(rig.lost_reason(), "kicked: You have been kicked");
        assert_eq!(rig.calls(), [Call::Open(1, "127.0.0.1:9000".parse().unwrap(), "http://127.0.0.1:9000/CAPS/seed/".into()), Call::Close(1)]);
    }

    #[test]
    fn unsolicited_logout_reply_and_disable_simulator_are_losses() {
        let mut rig = Rig::entered();
        rig.receive(messages::LOGOUT_REPLY, &[0; 33]);
        assert!(rig.lost_reason().contains("logged the avatar out"));

        let mut rig = Rig::entered();
        rig.receive(messages::DISABLE_SIMULATOR, &[]);
        assert!(rig.lost_reason().contains("closed the connection"));
    }

    #[test]
    fn sixty_seconds_of_silence_is_a_loss() {
        let mut rig = Rig::entered();
        rig.now += secs(59);
        rig.session.tick(rig.now);
        assert_eq!(rig.events(), []);
        rig.now += secs(1);
        rig.session.tick(rig.now);
        assert!(rig.lost_reason().contains("60 s"));
    }

    #[test]
    fn unacknowledged_reliable_packet_is_resent_then_a_loss() {
        let mut rig = Rig::entered();
        rig.session.start_sit(rig.now);
        let original = rig.sent();
        assert_eq!(original.len(), 1);
        let mut resends = 0;
        // Keep the simulator "alive" with unreliable traffic that acks nothing.
        for _ in 0..31 {
            rig.receive_from(1, packet::high(200), &[], false);
            rig.advance(secs(1));
            if rig.session.lost {
                break;
            }
            for packet in rig.sent() {
                assert_eq!(packet.id, messages::AGENT_REQUEST_SIT);
                assert_eq!(packet.sequence, original[0].sequence);
                assert!(packet.flags & packet::FLAG_RESENT != 0);
                resends += 1;
            }
        }
        assert_eq!(resends, 4);
        let events = rig.events();
        assert!(matches!(events.last(), Some(SessionEvent::Lost(r)) if r.contains("acknowledging")), "{events:?}");
    }

    #[test]
    fn event_queue_gone_is_a_loss() {
        let mut rig = Rig::entered();
        rig.session.handle_caps(1, CapsEvent::Gone("event queue reports no such session".into()), rig.now);
        assert_eq!(rig.lost_reason(), "event queue reports no such session");
    }

    #[test]
    fn nothing_is_sent_or_processed_after_a_loss() {
        let mut rig = Rig::entered();
        rig.receive(messages::DISABLE_SIMULATOR, &[]);
        rig.events();
        rig.net.0.borrow_mut().sent.clear();
        rig.receive_from(1, messages::START_PING_CHECK, &[1, 0, 0, 0, 0], false);
        rig.receive(messages::KICK_USER, &wire::kick_user("again"));
        rig.advance(secs(120));
        assert_eq!(rig.events(), []);
        assert!(rig.net.0.borrow().sent.is_empty());
    }

    #[test]
    fn events_from_another_generation_are_discarded() {
        let mut rig = Rig::entered();
        rig.receive_from(7, messages::KICK_USER, &wire::kick_user("stale"), true);
        rig.receive_from(0, messages::DISABLE_SIMULATOR, &[], true);
        rig.session.handle_caps(7, CapsEvent::Gone("stale".into()), rig.now);
        assert_eq!(rig.events(), []);
        assert!(!rig.session.lost);
    }

    // ------------------------------------------------------------ logout

    #[test]
    fn logout_is_confirmed_by_the_simulator() {
        let mut rig = Rig::entered();
        rig.session.start_logout(rig.now);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::LOGOUT_REQUEST);
        assert!(sent[0].is_reliable());
        rig.receive(messages::LOGOUT_REPLY, &[0; 33]);
        assert_eq!(rig.events(), [SessionEvent::LoggedOut]);
    }

    #[test]
    fn session_ending_during_logout_counts_as_logged_out() {
        let mut rig = Rig::entered();
        rig.session.start_logout(rig.now);
        rig.session.handle_caps(1, CapsEvent::Gone("event queue reports no such session".into()), rig.now);
        assert_eq!(rig.events(), [SessionEvent::LoggedOut]);
        // The late confirmation is not a second outcome.
        rig.receive(messages::LOGOUT_REPLY, &[0; 33]);
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn logout_gives_up_waiting_after_five_seconds() {
        let mut rig = Rig::entered();
        rig.session.start_logout(rig.now);
        rig.advance(secs(4));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        assert_eq!(rig.events(), [SessionEvent::LoggedOut]);
    }

    // --------------------------------------------------------------- sit

    #[test]
    fn sit_is_confirmed_by_response_and_own_object_update() {
        let mut rig = Rig::entered();
        rig.own_update(0);
        assert_eq!(rig.events(), [SessionEvent::Standing]);

        rig.session.start_sit(rig.now);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::AGENT_REQUEST_SIT);
        assert_eq!(sent[0].body[32..48], *target().as_bytes());

        rig.sit_response(target());
        assert_eq!(rig.sent_ids(), [messages::AGENT_SIT]);
        assert_eq!(rig.events(), [], "not seated until the simulator shows the avatar parented");

        rig.own_update(4242);
        assert_eq!(rig.events(), [SessionEvent::Seated]);

        // The deadline is disarmed and nothing more is sent about sitting.
        rig.ack_all();
        rig.advance(secs(20));
        assert_eq!(rig.events(), []);
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn sit_fails_when_the_simulator_never_confirms() {
        let mut rig = Rig::entered();
        rig.session.start_sit(rig.now);
        rig.ack_all();
        rig.advance(secs(14));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        assert!(matches!(&rig.events()[..], [SessionEvent::SitFailed(_)]));
    }

    #[test]
    fn sit_fails_when_accepted_but_the_avatar_never_gets_seated() {
        let mut rig = Rig::entered();
        rig.own_update(0);
        rig.events();
        rig.session.start_sit(rig.now);
        rig.sit_response(target());
        // A routine update still showing the avatar standing changes nothing.
        rig.own_update(0);
        rig.ack_all();
        rig.advance(secs(15));
        assert!(matches!(&rig.events()[..], [SessionEvent::SitFailed(_)]));
    }

    #[test]
    fn simulator_alerts_are_passed_on() {
        let mut rig = Rig::entered();
        rig.session.start_sit(rig.now);
        let text = "There is no suitable surface to sit on, try another spot.";
        rig.receive(messages::ALERT_MESSAGE, &wire::alert_message(text));
        rig.receive(messages::AGENT_ALERT_MESSAGE, &wire::agent_alert_message("Region restarting"));
        assert_eq!(
            rig.events(),
            [SessionEvent::Alert(text.into()), SessionEvent::Alert("Region restarting".into())]
        );
        // An alert is information only; the sit still runs to its deadline.
        rig.ack_all();
        rig.advance(secs(15));
        assert!(matches!(&rig.events()[..], [SessionEvent::SitFailed(_)]));
    }

    #[test]
    fn sit_response_naming_another_prim_of_the_linkset_confirms_the_sit() {
        let mut rig = Rig::entered();
        rig.session.start_sit(rig.now);
        rig.sent();
        // The simulator answers with the prim it chose, not the one requested.
        rig.sit_response(Uuid::from_bytes([0xEE; 16]));
        assert_eq!(rig.sent_ids(), [messages::AGENT_SIT]);
        rig.own_update(555);
        assert_eq!(rig.events(), [SessionEvent::Seated]);
    }

    #[test]
    fn forced_sit_on_a_different_object_is_not_seated_on_target() {
        let mut rig = Rig::entered();
        rig.sit_response(Uuid::from_bytes([0xEE; 16]));
        assert_eq!(rig.sent_ids(), [], "no AgentSit without a sit in progress");
        rig.own_update(555);
        assert_eq!(rig.events(), [SessionEvent::Standing]);
    }

    #[test]
    fn being_unseated_is_reported() {
        let mut rig = Rig::seated();
        rig.own_update(4242);
        assert_eq!(rig.events(), []);
        rig.own_update(0);
        assert_eq!(rig.events(), [SessionEvent::Standing]);
        // Sitting again needs a fresh confirmation from the simulator.
        rig.own_update(4242);
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn other_avatars_do_not_change_the_seat() {
        let mut rig = Rig::seated();
        let other = Uuid::from_bytes([0x77; 16]);
        rig.receive(messages::OBJECT_UPDATE, &wire::object_update(&[(other, 0), (other, 4242)]));
        rig.receive(messages::OBJECT_UPDATE, &wire::object_update(&[(target(), 0)]));
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn forced_sit_on_the_target_counts_as_seated() {
        let mut rig = Rig::entered();
        rig.sit_response(target());
        assert_eq!(rig.sent_ids(), [], "no AgentSit without a sit in progress");
        rig.own_update(4242);
        assert_eq!(rig.events(), [SessionEvent::Seated]);
    }

    // ---------------------------------------------------------- teleport

    fn start_teleport(rig: &mut Rig) {
        rig.session.start_teleport("Target", [128.0, 129.0, 25.0], rig.now);
    }

    #[test]
    fn teleport_looks_the_region_up_then_requests_it() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::MAP_NAME_REQUEST);
        assert!(sent[0].body.ends_with(b"\x07Target\0"));

        // Partial matches are not the region; the exact one is, in any case.
        let body = wire::map_block_reply(&[(1002, 1000, "Target Annex", 13), (1001, 1000, "TARGET", 13)]);
        rig.receive(messages::MAP_BLOCK_REPLY, &body);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::TELEPORT_LOCATION_REQUEST);
        let handle = ((1001u64 * 256) << 32) | (1000 * 256);
        assert_eq!(sent[0].body[32..40], handle.to_le_bytes());
        assert_eq!(sent[0].body[40..44], 128.0f32.to_le_bytes());
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn teleport_fails_when_the_region_is_not_on_the_map() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        rig.receive(messages::MAP_BLOCK_REPLY, &wire::map_block_reply(&[(0, 0, "Target", 255)]));
        assert_eq!(rig.events(), [SessionEvent::TeleportFailed("region \"Target\" not found".into())]);
        // Still connected where it was.
        assert!(!rig.session.lost);
        assert_eq!(rig.generation(), 1);
    }

    #[test]
    fn teleport_reports_the_grids_failure_reason() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        rig.receive(messages::MAP_BLOCK_REPLY, &wire::map_block_reply(&[(1001, 1000, "Target", 13)]));
        rig.receive(messages::TELEPORT_START, &[0; 4]);
        rig.receive(messages::TELEPORT_PROGRESS, &wire::teleport_progress("contacting"));
        rig.receive(messages::TELEPORT_FAILED, &wire::teleport_failed("Destination refused"));
        assert_eq!(rig.events(), [SessionEvent::TeleportFailed("Destination refused".into())]);
        // A second failure report for the same attempt is not a second outcome.
        rig.receive(messages::TELEPORT_FAILED, &wire::teleport_failed("Destination refused"));
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn teleport_times_out_after_sixty_seconds() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        rig.ack_all();
        for _ in 0..59 {
            rig.receive_from(1, packet::high(200), &[], false);
            rig.advance(secs(1));
        }
        assert_eq!(rig.events(), []);
        rig.receive_from(1, packet::high(200), &[], false);
        rig.advance(secs(1));
        assert_eq!(rig.events(), [SessionEvent::TeleportFailed("no answer within 60 s".into())]);
    }

    #[test]
    fn teleport_inside_the_current_region_skips_the_lookup() {
        let mut rig = Rig::entered();
        rig.session.start_teleport(" home", [5.0, 6.0, 7.0], rig.now);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::TELEPORT_LOCATION_REQUEST);
        assert_eq!(sent[0].body[32..40], HOME_HANDLE.to_le_bytes());
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([5.0, 6.0, 7.0]));
        assert_eq!(rig.events(), [SessionEvent::TeleportArrived]);
        assert_eq!(rig.generation(), 1);
        assert_eq!(rig.session.sim.position, Some([5.0, 6.0, 7.0]));
    }

    fn finish() -> CapsEvent {
        CapsEvent::Event(EqEvent::TeleportFinish(TeleportFinish {
            sim_addr: "127.0.0.1:9001".parse().unwrap(),
            seed_capability: "http://127.0.0.1:9001/CAPS/seed2/".to_string(),
            region_handle: ((1001 * 256) << 32) | (1000 * 256),
        }))
    }

    #[test]
    fn teleport_finish_hands_the_session_over_to_the_new_simulator() {
        let mut rig = Rig::seated();
        rig.calls();
        start_teleport(&mut rig);
        rig.receive(messages::MAP_BLOCK_REPLY, &wire::map_block_reply(&[(1001, 1000, "Target", 13)]));
        rig.sent();

        rig.session.handle_caps(1, finish(), rig.now);
        assert_eq!(
            rig.calls(),
            [
                Call::Close(1),
                Call::Open(2, "127.0.0.1:9001".parse().unwrap(), "http://127.0.0.1:9001/CAPS/seed2/".into()),
            ]
        );
        let handshake = std::mem::take(&mut rig.net.0.borrow_mut().sent);
        assert_eq!(
            handshake.iter().map(|(generation, p)| (*generation, p.id, p.sequence)).collect::<Vec<_>>(),
            [(2, messages::USE_CIRCUIT_CODE, 1), (2, messages::COMPLETE_AGENT_MOVEMENT, 2)]
        );
        assert_eq!(rig.events(), [], "arrival is reported once the new handshake is done");

        rig.enter("Target", ((1001 * 256) << 32) | (1000 * 256));
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Target".into())]);

        // The seat is unknown again in the new region, and the teleport's
        // deadline died with the old circuit.
        rig.own_update(0);
        assert_eq!(rig.events(), [SessionEvent::Standing]);
        for _ in 0..70 {
            rig.receive_from(2, packet::high(200), &[], false);
            rig.advance(secs(1));
        }
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn late_traffic_from_the_old_simulator_is_ignored_after_arrival() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        rig.enter("Target", HOME_HANDLE);
        rig.events();

        rig.receive_from(1, messages::DISABLE_SIMULATOR, &[], true);
        rig.receive_from(1, messages::KICK_USER, &wire::kick_user("old"), true);
        rig.receive_from(1, messages::REGION_HANDSHAKE, &wire::region_handshake("Home"), true);
        rig.session.handle_caps(1, CapsEvent::Gone("old queue closed".into()), rig.now);
        assert_eq!(rig.events(), []);
        assert!(!rig.session.lost);
        assert_eq!(rig.session.sim.region_name.as_deref(), Some("Target"));
    }

    #[test]
    fn unsolicited_teleport_finish_is_followed_too() {
        let mut rig = Rig::seated();
        rig.session.handle_caps(1, finish(), rig.now);
        rig.enter("Elsewhere", HOME_HANDLE);
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Elsewhere".into())]);
    }

    #[test]
    fn new_simulator_that_never_answers_is_a_loss() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        rig.advance(secs(30));
        assert!(matches!(&rig.events()[..], [SessionEvent::Lost(_)]));
    }

    // ------------------------------------------------------ chat and IMs

    fn avatar(name: &str) -> Source {
        Source::Avatar { id: Uuid::from_bytes([0x77; 16]), name: name.into() }
    }

    fn message(kind: &str, source: Source, text: &str) -> Message {
        Message { kind: kind.into(), source, sent_at: None, text: text.into() }
    }

    impl Rig {
        fn chat(&mut self, source_type: u8, chat_type: u8, audible: i8, text: &str) {
            let ids = (Uuid::from_bytes([0x77; 16]), Uuid::from_bytes([0x88; 16]), source_type);
            let body = wire::chat_from_simulator("Speaker", ids, chat_type, audible, text);
            self.receive(messages::CHAT_FROM_SIMULATOR, &body);
        }

        fn im(&mut self, from: Uuid, dialog: u8, id: Uuid, stored: Option<u32>, text: &str) {
            let body = wire::improved_instant_message(from, "Sender", dialog, id, stored, text);
            self.receive(messages::IMPROVED_INSTANT_MESSAGE, &body);
        }

        /// How many RetrieveInstantMessages were sent since the last call to `sent`.
        fn offline_requests(&mut self) -> usize {
            self.sent_ids().iter().filter(|id| **id == messages::RETRIEVE_INSTANT_MESSAGES).count()
        }
    }

    #[test]
    fn chat_is_reported_with_its_kind() {
        let mut rig = Rig::entered();
        let kinds = [(0, "whisper"), (1, "say"), (2, "shout"), (6, "debug"), (7, "region"), (8, "owner"), (9, "direct")];
        for (chat_type, kind) in kinds {
            rig.chat(1, chat_type, 1, "hello");
            assert_eq!(rig.events(), [SessionEvent::Chat(message(kind, avatar("Speaker"), "hello"))]);
        }
        rig.chat(1, 42, 1, "hello");
        assert_eq!(rig.events(), [SessionEvent::Chat(message("type-42", avatar("Speaker"), "hello"))]);
        // Barely audible chat is still chat.
        rig.chat(1, 1, 0, "far away");
        assert_eq!(rig.events(), [SessionEvent::Chat(message("say", avatar("Speaker"), "far away"))]);
    }

    #[test]
    fn chat_from_objects_and_the_grid_keeps_its_source() {
        let mut rig = Rig::entered();
        rig.chat(2, 1, 1, "tick");
        let object = Source::Object {
            id: Some(Uuid::from_bytes([0x77; 16])),
            name: "Speaker".into(),
            owner: Uuid::from_bytes([0x88; 16]),
        };
        assert_eq!(rig.events(), [SessionEvent::Chat(message("say", object, "tick"))]);
        rig.chat(0, 1, 1, "notice");
        let system = Source::System { name: "Speaker".into() };
        assert_eq!(rig.events(), [SessionEvent::Chat(message("say", system, "notice"))]);
    }

    #[test]
    fn typing_empty_and_inaudible_chat_is_dropped() {
        let mut rig = Rig::entered();
        rig.chat(1, 4, 1, "");
        rig.chat(1, 5, 1, "");
        rig.chat(1, 4, 1, "typing with text");
        rig.chat(1, 1, 1, "");
        rig.chat(1, 1, -1, "muffled");
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn instant_messages_from_avatars_and_objects_are_reported() {
        let mut rig = Rig::entered();
        let (sender, object) = (Uuid::from_bytes([0x77; 16]), Uuid::from_bytes([0x99; 16]));
        rig.im(sender, 0, Uuid::from_bytes([0x66; 16]), None, "are you there?");
        assert_eq!(
            rig.events(),
            [SessionEvent::InstantMessage(message("im", avatar("Sender"), "are you there?"))]
        );

        rig.im(sender, 19, object, None, "done");
        let source = Source::Object { id: Some(object), name: "Sender".into(), owner: sender };
        assert_eq!(rig.events(), [SessionEvent::InstantMessage(message("object", source, "done"))]);

        // An older region does not say which object it was.
        rig.im(sender, 19, Uuid::nil(), None, "done");
        let source = Source::Object { id: None, name: "Sender".into(), owner: sender };
        assert_eq!(rig.events(), [SessionEvent::InstantMessage(message("object", source, "done"))]);

        rig.im(Uuid::nil(), 0, Uuid::nil(), None, "maintenance");
        let source = Source::System { name: "Sender".into() };
        assert_eq!(rig.events(), [SessionEvent::InstantMessage(message("im", source, "maintenance"))]);
    }

    #[test]
    fn stored_instant_messages_carry_the_time_they_were_sent() {
        let mut rig = Rig::entered();
        rig.im(Uuid::from_bytes([0x77; 16]), 0, Uuid::nil(), Some(1_700_000_000), "earlier");
        let mut expected = message("im", avatar("Sender"), "earlier");
        expected.sent_at = Some(1_700_000_000);
        assert_eq!(rig.events(), [SessionEvent::InstantMessage(expected)]);
    }

    #[test]
    fn other_instant_message_traffic_is_dropped() {
        let mut rig = Rig::entered();
        // Inventory offer, group notice, session message, auto response, typing.
        for dialog in [4, 32, 17, 20, 41, 42] {
            rig.im(Uuid::from_bytes([0x77; 16]), dialog, Uuid::nil(), None, "x");
        }
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn offline_messages_are_requested_once_per_session_when_enabled() {
        let mut rig = Rig::with_offline_im(true);
        assert_eq!(rig.offline_requests(), 0, "not before the region is entered");
        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Home"));
        let body = wire::agent_movement_complete(&rig.agent(), [1.0, 2.0, 3.0], HOME_HANDLE);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        let sent = rig.sent();
        let request: Vec<_> = sent.iter().filter(|p| p.id == messages::RETRIEVE_INSTANT_MESSAGES).collect();
        assert_eq!(request.len(), 1);
        assert!(request[0].is_reliable());
        rig.ack_all();

        // Not again in the next region.
        rig.session.handle_caps(1, finish(), rig.now);
        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Target"));
        let body = wire::agent_movement_complete(&rig.agent(), [1.0, 2.0, 3.0], HOME_HANDLE);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        assert_eq!(rig.offline_requests(), 0);
    }

    #[test]
    fn offline_messages_are_not_requested_by_default() {
        let mut rig = Rig::new();
        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Home"));
        let body = wire::agent_movement_complete(&rig.agent(), [1.0, 2.0, 3.0], HOME_HANDLE);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        assert_eq!(rig.offline_requests(), 0);
    }

    #[test]
    fn display_names_url_is_passed_on_for_the_current_circuit_only() {
        let mut rig = Rig::entered();
        let url = Some("http://127.0.0.1:9000/CAPS/names/".to_string());
        rig.session.handle_caps(1, CapsEvent::DisplayNames(url.clone()), rig.now);
        rig.session.handle_caps(1, CapsEvent::DisplayNames(None), rig.now);
        assert_eq!(
            rig.events(),
            [SessionEvent::DisplayNamesUrl(url.clone()), SessionEvent::DisplayNamesUrl(None)]
        );
        rig.session.handle_caps(7, CapsEvent::DisplayNames(url.clone()), rig.now);
        assert_eq!(rig.events(), []);
        // The avatar search URL travels the same way.
        rig.session.handle_caps(1, CapsEvent::AvatarSearch(url.clone()), rig.now);
        assert_eq!(rig.events(), [SessionEvent::AvatarSearchUrl(url.clone())]);
        rig.session.handle_caps(7, CapsEvent::AvatarSearch(url), rig.now);
        assert_eq!(rig.events(), []);
    }

    // ------------------------------------------------- messages on command

    #[test]
    fn says_and_sends_instant_messages_once_in_a_region() {
        let mut rig = Rig::entered();
        let to = Uuid::from_bytes([0x0F; 16]);
        assert_eq!(rig.session.say(1, 2, "test message", rig.now), Ok(()));
        assert_eq!(rig.session.instant_message(to, "hello", rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(Packet::is_reliable));
        let expected = messages::chat_from_viewer(&rig.agent(), "test message", 2, 1);
        assert_eq!((sent[0].id, &sent[0].body), (messages::CHAT_FROM_VIEWER, &expected.body));
        // The message carries the avatar's name and where it stands.
        let expected = messages::instant_message(&rig.agent(), "Sit Bot", to, [10.0, 20.0, 30.0], "hello");
        assert_eq!((sent[1].id, &sent[1].body), (messages::IMPROVED_INSTANT_MESSAGE, &expected.body));
    }

    #[test]
    fn avatar_search_is_sent_and_its_answer_reported() {
        let mut rig = Rig::entered();
        let query = Uuid::from_bytes([0x33; 16]);
        assert_eq!(rig.session.find_avatar(query, "jane doe", rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::AVATAR_PICKER_REQUEST);
        assert!(sent[0].body.ends_with(b"\x09jane doe\0"));

        let jane = Uuid::from_bytes([1; 16]);
        rig.receive(messages::AVATAR_PICKER_REPLY, &wire::avatar_picker_reply(query, &[(jane, "Jane", "Doe")]));
        assert_eq!(
            rig.events(),
            [SessionEvent::AvatarsFound { query, avatars: vec![(jane, "Jane Doe".to_string())] }]
        );
    }

    #[test]
    fn nothing_is_sent_on_command_outside_a_region() {
        let to = Uuid::from_bytes([0x0F; 16]);
        let refused = |rig: &mut Rig| {
            let now = rig.now;
            assert_eq!(rig.session.say(0, 1, "hello", now), Err(NOT_CONNECTED.to_string()));
            assert_eq!(rig.session.instant_message(to, "hello", now), Err(NOT_CONNECTED.to_string()));
            assert_eq!(rig.session.find_avatar(to, "jane doe", now), Err(NOT_CONNECTED.to_string()));
            let chatty = [messages::CHAT_FROM_VIEWER, messages::IMPROVED_INSTANT_MESSAGE, messages::AVATAR_PICKER_REQUEST];
            assert!(rig.sent_ids().iter().all(|id| !chatty.contains(id)));
        };
        // Before the handshake is done.
        refused(&mut Rig::new());
        // While a teleport is under way.
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        refused(&mut rig);
        // While logging out.
        let mut rig = Rig::entered();
        rig.session.start_logout(rig.now);
        refused(&mut rig);
        // After the session is lost.
        let mut rig = Rig::entered();
        rig.receive(messages::KICK_USER, &wire::kick_user("bye"));
        refused(&mut rig);
    }
}
