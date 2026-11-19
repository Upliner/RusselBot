//! One logged-in session: the simulator circuit, its handshake and
//! keepalive, teleports with the hand-over to a new simulator, sitting, and
//! seat tracking. All I/O goes through `Net`, so the logic is testable with
//! a fake clock and a recording network.

use crate::caps::{CapsEvent, EqEvent, TeleportFinish};
use crate::login::SessionInfo;
use crate::persist::{Location, Sitting, World};
use crate::reconciler::same_region;
use crate::udp::circuit::Circuit;
use crate::udp::messages::{self, Agent, Incoming, Outgoing, Transform, Vector3};
use crate::udp::packet;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const SILENCE_LIMIT: Duration = Duration::from_secs(60);
pub const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
pub const SIT_LIMIT: Duration = Duration::from_secs(15);
pub const TELEPORT_LIMIT: Duration = Duration::from_secs(60);
pub const LOGOUT_LIMIT: Duration = Duration::from_secs(5);
const AGENT_UPDATE_INTERVAL: Duration = Duration::from_secs(1);
/// How long a touch waits for a scan to turn its object up.
pub const TOUCH_WAIT: Duration = Duration::from_secs(10);
/// Most objects remembered in one region; more are not learned.
const MAX_OBJECTS: usize = 50_000;
/// Most objects remembered as mentioned but not described.
const MAX_UNDESCRIBED: usize = 20_000;
/// RequestMultipleObjects counts its ids in one byte.
const REQUEST_BATCH: usize = 255;

/// Bits per second for resend, land, wind, cloud, task, texture and asset.
/// Only the task channel matters to the bot: it carries the avatar's own
/// object updates, which seat tracking depends on. Checked against OpenSim:
/// with these values the update arrives within a second of sitting or
/// standing.
const THROTTLES: [f32; 7] = [20_000.0, 0.0, 0.0, 0.0, 30_000.0, 0.0, 10_000.0];
/// A short draw distance keeps the simulator from streaming the whole scene.
pub const DRAW_DISTANCE: f32 = 32.0;
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
    /// The avatar is seated on an object: the simulator confirmed a sit on
    /// it and shows the avatar attached.
    Seated(Sat),
    /// The avatar is known not to be seated on an object the simulator
    /// confirmed a sit on.
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
    /// An avatar offers to teleport the bot's avatar to it; `lure` is what
    /// `accept_teleport` takes.
    TeleportOffer {
        from: Uuid,
        name: String,
        lure: Uuid,
    },
    /// An avatar asks to be teleported to the bot's avatar.
    TeleportRequested {
        from: Uuid,
        name: String,
        text: String,
    },
    /// A touch that waited for its object went out.
    Touched(Uuid),
    /// A touch gave up waiting: the object did not turn up.
    TouchFailed(Uuid),
    /// The answer to `find_avatar`: the avatars the grid offers for the
    /// name, as id and `First Last`.
    AvatarsFound {
        query: Uuid,
        avatars: Vec<(Uuid, String)>,
    },
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
    Avatar {
        id: Uuid,
        name: String,
    },
    /// `id` is missing when the simulator does not say which object it was.
    Object {
        id: Option<Uuid>,
        name: String,
        owner: Uuid,
    },
    /// The grid itself.
    System {
        name: String,
    },
}

const CHAT_SOURCE_AGENT: u8 = 1;
const CHAT_SOURCE_OBJECT: u8 = 2;
const CHAT_TYPE_START_TYPING: u8 = 4;
const CHAT_TYPE_STOP_TYPING: u8 = 5;
const CHAT_NOT_AUDIBLE: i8 = -1;
const IM_NOTHING_SPECIAL: u8 = messages::IM_PLAIN;
const IM_FROM_TASK: u8 = 19;

/// A confirmed sit on an object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sat {
    /// The prim the simulator named when it seated the avatar. In a linkset
    /// that may be another prim than the one asked for.
    pub prim: Uuid,
    /// The object the bot asked to sit on, when the sit was its own.
    pub requested: Option<Uuid>,
}

impl Sat {
    /// Whether this is a sit on `object`: the simulator named it, or it
    /// answered the bot's request for it.
    pub fn is_on(&self, object: Uuid) -> bool {
        self.prim == object || self.requested == Some(object)
    }
}

/// What the avatar sits on, as far as the simulator has said.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seat {
    /// Not yet learned since entering the region.
    Unknown,
    /// Not attached to anything; a ground sit looks like this too.
    None,
    /// Seated on an object.
    Object(Sat),
    /// Attached to something the simulator did not name.
    Other,
}

/// What became of a touch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    Sent,
    /// The object is not known yet; a scan is looking for it. Ends in
    /// `Touched` or `TouchFailed`.
    Waiting,
}

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
    /// Where the avatar is in the region, as last reported or worked out.
    position: Option<Vector3>,
    /// The avatar's own region-local id, once its description has come.
    own_local: Option<u32>,
    /// The position in the avatar's last update: in the region while it
    /// stands, relative to its seat while it sits.
    own_offset: Option<Vector3>,
    /// Where the object the avatar sits on is and how it is turned.
    seat_frame: Option<Transform>,
    /// Both halves of the handshake are done and the region is known.
    entered: bool,
    next_agent_update: Instant,
    /// The objects the simulator has described, by id and by its own
    /// region-local id, which is what it takes touches by.
    objects: HashMap<Uuid, u32>,
    local_ids: HashMap<u32, Uuid>,
    /// Objects mentioned as cached, which the bot would have to ask about.
    undescribed: HashSet<u32>,
    /// When the bot started asking about them, if it has.
    scan_started: Option<Instant>,
    /// Touches waiting for their object to be described, with their deadline.
    wanted: Vec<(Uuid, Instant)>,
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
    /// How far the simulator is asked to describe the scene, when the
    /// config says; a scan then starts on entering each region.
    scan_distance: Option<f32>,
    sim: Sim,
    /// The sit the simulator confirmed since the avatar last stood up: an
    /// answer to our request, or one it made unprompted.
    seat_confirmed: Option<Sat>,
    /// Local id of what the avatar sits on, from its own object updates.
    own_parent: Option<u32>,
    /// The seat as last reported in an event.
    seat: Option<Sat>,
    seat_reported: bool,
    /// The last sit or stand request sent since the avatar arrived was a
    /// ground sit. The simulator does not report one, so this is a belief.
    ground: bool,
    /// The object of the sit in progress.
    sit_request: Option<Uuid>,
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
        scan_distance: Option<f32>,
        fetch_offline_im: bool,
        generation: u64,
        now: Instant,
    ) -> Session<N> {
        let mut session = Session {
            net,
            agent: Agent {
                agent_id: info.agent_id,
                session_id: info.session_id,
            },
            agent_name: agent_name.to_string(),
            circuit_code: info.circuit_code,
            scan_distance,
            sim: Sim::new(0, now),
            seat_confirmed: None,
            own_parent: None,
            seat: None,
            seat_reported: false,
            ground: false,
            sit_request: None,
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
        consider(if sim.entered {
            Some(sim.next_agent_update)
        } else {
            Some(sim.opened_at + HANDSHAKE_LIMIT)
        });
        consider(self.sit_deadline);
        consider(sim.wanted.iter().map(|(_, deadline)| *deadline).min());
        consider(self.teleport.as_ref().map(|t| t.deadline));
        consider(self.logout_deadline);
        deadline
    }

    fn open_circuit(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str, now: Instant) {
        self.fail_touches();
        self.sim = Sim::new(generation, now);
        self.seat_confirmed = None;
        self.own_parent = None;
        self.seat = None;
        self.seat_reported = false;
        self.ground = false;
        self.sit_request = None;
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
            self.fail_touches();
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

    /// Asks to sit on `object`. Ends in `Seated` or `SitFailed`.
    pub fn start_sit(&mut self, object: Uuid, now: Instant) {
        self.sit_request = Some(object);
        self.sit_deadline = Some(now + SIT_LIMIT);
        self.send(messages::agent_request_sit(&self.agent, object), true, now);
    }

    /// One avatar update that carries `control_flags`; the periodic ones
    /// carry none, which is how a viewer ends a key press.
    fn send_control(&mut self, control_flags: u32, now: Instant) {
        let camera = self.sim.position.unwrap_or_default();
        let update = messages::agent_update(&self.agent, camera, self.draw_distance(), control_flags);
        self.send(update, true, now);
    }

    /// Stands the avatar up from whatever it sits on. A sit in progress is
    /// given up without a `SitFailed`.
    pub fn stand(&mut self, now: Instant) -> Result<(), String> {
        self.present()?;
        self.sit_request = None;
        self.sit_deadline = None;
        self.ground = false;
        self.send_control(messages::CONTROL_STAND_UP, now);
        Ok(())
    }

    /// Sits the avatar down on the ground where it stands.
    pub fn sit_ground(&mut self, now: Instant) -> Result<(), String> {
        self.present()?;
        self.ground = true;
        self.send_control(messages::CONTROL_SIT_ON_GROUND, now);
        Ok(())
    }

    /// The name of the region the avatar is in, once it has entered one.
    pub fn region(&self) -> Option<&str> {
        self.sim.region_name.as_deref().filter(|_| self.sim.entered)
    }

    /// Where the avatar is in the region: what the simulator said on
    /// arrival or after a teleport inside the region, and what its updates
    /// about the avatar have said since. While the avatar sits on an
    /// object this is worked out from where the object is; until that is
    /// known it stays the last position before.
    pub fn position(&self) -> Option<Vector3> {
        self.sim.position
    }

    /// Whether the position of a seated avatar has been worked out from
    /// its seat. False while it does not sit on an object.
    pub fn seat_frame_known(&self) -> bool {
        self.own_parent.is_some_and(|parent| parent != 0) && self.sim.seat_frame.is_some() && self.sim.own_offset.is_some()
    }

    /// Works the avatar's position out again after an update about it or
    /// about its seat.
    fn update_position(&mut self) {
        let Some(offset) = self.sim.own_offset else {
            return;
        };
        match (self.own_parent, self.sim.seat_frame) {
            (Some(0), _) => self.sim.position = Some(offset),
            (Some(_), Some(seat)) => {
                let turned = messages::rotate(seat.rotation, offset);
                self.sim.position = Some([0, 1, 2].map(|i| seat.position[i] + turned[i]));
            }
            _ => {}
        }
    }

    /// An update about the object with this region-local id says where it
    /// is: of interest when it is the avatar or its seat.
    fn moved(&mut self, local_id: u32, transform: Transform) {
        if self.sim.own_local == Some(local_id) {
            self.sim.own_offset = Some(transform.position);
        } else if local_id != 0 && self.own_parent == Some(local_id) {
            self.sim.seat_frame = Some(transform);
        } else {
            return;
        }
        self.update_position();
    }

    /// What the avatar sits on.
    pub fn seat(&self) -> Seat {
        match (self.own_parent, self.seat_confirmed) {
            (None, _) => Seat::Unknown,
            (Some(0), _) => Seat::None,
            (Some(_), Some(sat)) => Seat::Object(sat),
            (Some(_), None) => Seat::Other,
        }
    }

    /// Whether the bot last sat the avatar on the ground: a ground sit was
    /// sent, and since then the avatar has not arrived anywhere, been told
    /// to stand, or been seated on an object.
    pub fn on_ground(&self) -> bool {
        self.ground
    }

    fn draw_distance(&self) -> f32 {
        self.scan_distance.unwrap_or(DRAW_DISTANCE)
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
        self.send_im(to, messages::IM_PLAIN, text, now)
    }

    fn send_im(&mut self, to: Uuid, dialog: u8, text: &str, now: Instant) -> Result<(), String> {
        self.present()?;
        let position = self.sim.position.unwrap_or_default();
        let message = messages::instant_message(&self.agent, &self.agent_name, to, position, dialog, text);
        self.send(message, true, now);
        Ok(())
    }

    /// Asks the avatar `to` to teleport this avatar to it. If it agrees, a
    /// `TeleportOffer` from it follows.
    pub fn request_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String> {
        self.send_im(to, messages::IM_TELEPORT_REQUEST, message, now)
    }

    /// Offers the avatar `to` a teleport to where this avatar is.
    pub fn offer_teleport(&mut self, to: Uuid, message: &str, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::start_lure(&self.agent, message, to), true, now);
        Ok(())
    }

    /// Accepts a teleport offer. Ends like `start_teleport`.
    pub fn accept_teleport(&mut self, lure: Uuid, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::teleport_lure_request(&self.agent, lure), true, now);
        self.teleport = Some(Teleport {
            region: String::new(),
            position: [0.0; 3],
            phase: TeleportPhase::Requested,
            deadline: now + TELEPORT_LIMIT,
        });
        Ok(())
    }

    // ------------------------------------------------------------ objects

    /// Starts asking the simulator about the objects it has only mentioned,
    /// and keeps asking as more are mentioned. Costs traffic, which is why
    /// it waits for a reason.
    fn start_scan(&mut self, now: Instant) {
        if self.sim.scan_started.is_some() {
            return;
        }
        self.sim.scan_started = Some(now);
        crate::info!("scanning for objects within {} m", self.draw_distance());
        let ids: Vec<u32> = self.sim.undescribed.drain().collect();
        self.request_objects(&ids, now);
    }

    fn request_objects(&mut self, local_ids: &[u32], now: Instant) {
        for batch in local_ids.chunks(REQUEST_BATCH) {
            self.send(messages::request_multiple_objects(&self.agent, batch), true, now);
        }
    }

    fn send_touch(&mut self, local_id: u32, now: Instant) {
        self.send(messages::object_grab(&self.agent, local_id), true, now);
        self.send(messages::object_degrab(&self.agent, local_id), true, now);
    }

    /// The simulator described an object.
    fn learn(&mut self, object: Uuid, local_id: u32, now: Instant) {
        let sim = &mut self.sim;
        sim.undescribed.remove(&local_id);
        if sim.objects.len() < MAX_OBJECTS || sim.objects.contains_key(&object) {
            if let Some(old) = sim.objects.insert(object, local_id) {
                sim.local_ids.remove(&old);
            }
            sim.local_ids.insert(local_id, object);
        }
        if sim.wanted.iter().any(|(wanted, _)| *wanted == object) {
            sim.wanted.retain(|(wanted, _)| *wanted != object);
            self.send_touch(local_id, now);
            self.events.push(SessionEvent::Touched(object));
        }
    }

    fn fail_touches(&mut self) {
        for (object, _) in std::mem::take(&mut self.sim.wanted) {
            self.events.push(SessionEvent::TouchFailed(object));
        }
    }

    /// Clicks `object` once. An object the simulator has not described is
    /// looked for with a scan, if none has run its course yet.
    pub fn touch(&mut self, object: Uuid, now: Instant) -> Result<Touch, String> {
        self.present()?;
        if let Some(local_id) = self.sim.objects.get(&object).copied() {
            self.send_touch(local_id, now);
            return Ok(Touch::Sent);
        }
        self.start_scan(now);
        let deadline = self.sim.scan_started.unwrap_or(now) + TOUCH_WAIT;
        if now >= deadline {
            return Err(self.unknown_object(object));
        }
        if !self.sim.wanted.iter().any(|(wanted, _)| *wanted == object) {
            self.sim.wanted.push((object, deadline));
        }
        Ok(Touch::Waiting)
    }

    /// Whether the simulator has described `object` in this region.
    pub fn knows(&self, object: Uuid) -> bool {
        self.sim.objects.contains_key(&object)
    }

    /// Why a touch cannot find its object.
    pub fn unknown_object(&self, object: Uuid) -> String {
        format!(
            "object {object} is not known to the bot; it must exist and be within {} m of the avatar",
            self.draw_distance()
        )
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
            self.send(messages::agent_update(&self.agent, camera, self.draw_distance(), 0), false, now);
        }
        let (late, wanted) = std::mem::take(&mut self.sim.wanted).into_iter().partition(|(_, at)| now >= *at);
        self.sim.wanted = wanted;
        for (object, _) in late {
            self.events.push(SessionEvent::TouchFailed(object));
        }
        if self.sit_deadline.is_some_and(|deadline| now >= deadline) {
            self.sit_deadline = None;
            self.sit_request = None;
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
                // Whatever the avatar's last update said is out of date now.
                self.sim.own_offset = None;
                self.ground = false;
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
                    self.seat_confirmed = Some(Sat {
                        prim: object,
                        requested: self.sit_request,
                    });
                    crate::debug!("sit accepted on {object} (autopilot: {autopilot})");
                    self.send(messages::agent_sit(&self.agent), true, now);
                } else {
                    self.seat_confirmed = Some(Sat {
                        prim: object,
                        requested: None,
                    });
                    crate::debug!("unrequested sit on {object}");
                }
                self.update_seat();
            }
            Incoming::ObjectUpdate(objects) => {
                let own_id = self.agent.agent_id;
                for object in &objects {
                    self.learn(object.full_id, object.local_id, now);
                }
                for object in &objects {
                    if object.full_id != own_id {
                        if let Some(transform) = object.transform {
                            self.moved(object.local_id, transform);
                        }
                        continue;
                    }
                    let stood_up = object.parent_id == 0 && self.own_parent.is_some_and(|p| p != 0);
                    if stood_up {
                        self.seat_confirmed = None;
                    }
                    if object.parent_id != 0 {
                        self.ground = false;
                    }
                    if self.own_parent != Some(object.parent_id) {
                        crate::debug!("own avatar's parent is now {}", object.parent_id);
                        // The avatar's position is relative to something
                        // else from here on: to the root prim of what it
                        // sits on, which is what simulators name as its
                        // parent (OpenSim: LLClientView.cs:7153). That prim
                        // has to be asked about: nothing says the simulator
                        // described it, and what it said earlier was not
                        // kept.
                        self.sim.seat_frame = None;
                        self.sim.own_offset = None;
                        if object.parent_id != 0 {
                            self.request_objects(&[object.parent_id], now);
                        }
                    }
                    self.own_parent = Some(object.parent_id);
                    self.sim.own_local = Some(object.local_id);
                    if let Some(transform) = object.transform {
                        self.sim.own_offset = Some(transform.position);
                        self.update_position();
                    }
                    self.update_seat();
                }
            }
            Incoming::ObjectsCompressed(objects) => {
                for object in objects {
                    self.learn(object.full_id, object.local_id, now);
                    if let Some(transform) = object.transform {
                        self.moved(object.local_id, transform);
                    }
                }
            }
            Incoming::ObjectsMoved(objects) => {
                for (local_id, trasform) in objects {
                    self.moved(local_id, trasform);
                }
            }
            Incoming::ObjectsCached(local_ids) => {
                let unknown: Vec<u32> = local_ids.into_iter().filter(|id| !self.sim.local_ids.contains_key(id)).collect();
                if self.sim.scan_started.is_some() {
                    self.request_objects(&unknown, now);
                } else {
                    let room = MAX_UNDESCRIBED.saturating_sub(self.sim.undescribed.len());
                    self.sim.undescribed.extend(unknown.into_iter().take(room));
                }
            }
            Incoming::ObjectsKilled(local_ids) => {
                for local_id in local_ids {
                    self.sim.undescribed.remove(&local_id);
                    if let Some(object) = self.sim.local_ids.remove(&local_id) {
                        self.sim.objects.remove(&object);
                    }
                }
            }
            Incoming::KickUser { reason } => self.lose(format!("kicked: {reason}")),
            Incoming::Alert { message } => self.events.push(SessionEvent::Alert(message)),
            Incoming::Chat {
                from_name,
                source_id,
                owner_id,
                source_type,
                chat_type,
                audible,
                text,
            } => {
                let typing = matches!(chat_type, CHAT_TYPE_START_TYPING | CHAT_TYPE_STOP_TYPING);
                if typing || text.is_empty() || audible == CHAT_NOT_AUDIBLE {
                    return;
                }
                let source = match source_type {
                    CHAT_SOURCE_AGENT => Source::Avatar {
                        id: source_id,
                        name: from_name,
                    },
                    CHAT_SOURCE_OBJECT => Source::Object {
                        id: Some(source_id),
                        name: from_name,
                        owner: owner_id,
                    },
                    _ => Source::System { name: from_name },
                };
                let kind = chat_kind(chat_type);
                self.events.push(SessionEvent::Chat(Message {
                    kind,
                    source,
                    sent_at: None,
                    text,
                }));
            }
            Incoming::InstantMessage {
                from_agent,
                from_name,
                dialog,
                id,
                offline,
                timestamp,
                text,
            } => {
                let (kind, source) = match dialog {
                    IM_NOTHING_SPECIAL if from_agent.is_nil() => ("im", Source::System { name: from_name }),
                    IM_NOTHING_SPECIAL => (
                        "im",
                        Source::Avatar {
                            id: from_agent,
                            name: from_name,
                        },
                    ),
                    // The sender is the object's owner and the id is the object.
                    IM_FROM_TASK => {
                        let id = (!id.is_nil()).then_some(id);
                        (
                            "object",
                            Source::Object {
                                id,
                                name: from_name,
                                owner: from_agent,
                            },
                        )
                    }
                    messages::IM_TELEPORT_OFFER => {
                        let offer = SessionEvent::TeleportOffer {
                            from: from_agent,
                            name: from_name,
                            lure: id,
                        };
                        return self.events.push(offer);
                    }
                    messages::IM_TELEPORT_REQUEST => {
                        let request = SessionEvent::TeleportRequested {
                            from: from_agent,
                            name: from_name,
                            text,
                        };
                        return self.events.push(request);
                    }
                    other => return crate::debug!("ignoring instant message with dialog {other}"),
                };
                let sent_at = (offline && timestamp != 0).then_some(timestamp);
                let message = Message {
                    kind: kind.to_string(),
                    source,
                    sent_at,
                    text,
                };
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
            if self.scan_distance.is_some() {
                self.start_scan(now);
            }
            if std::mem::take(&mut self.fetch_offline_im) {
                self.send(messages::retrieve_instant_messages(&self.agent), true, now);
            }
        }
    }

    fn handle_map_blocks(&mut self, blocks: &[messages::MapBlock], now: Instant) {
        let Some(teleport) = &mut self.teleport else {
            return;
        };
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
            self.events
                .push(SessionEvent::TeleportFailed(format!("region \"{region}\" not found")));
        }
    }

    /// Re-evaluates the seat from what the simulator has reported: seated on
    /// an object means it confirmed a sit there and shows the avatar parented.
    fn update_seat(&mut self) {
        let parented = self.own_parent.is_some_and(|p| p != 0);
        let seat = self.seat_confirmed.filter(|_| parented);
        match seat {
            // A sit in progress is answered even when it changes nothing.
            Some(sat) if self.seat != seat || (self.sit_request.is_some() && self.sit_request == sat.requested) => {
                self.sit_request = None;
                self.sit_deadline = None;
                self.events.push(SessionEvent::Seated(sat));
            }
            None if self.seat.is_some() || (!self.seat_reported && self.own_parent.is_some()) => {
                self.events.push(SessionEvent::Standing);
            }
            _ => return,
        }
        self.seat = seat;
        self.seat_reported = true;
    }
}

/// The avatar's surroundings as `persist` sees them. Only to be asked
/// while the avatar is `present`.
impl<N: Net> World for Session<N> {
    fn here(&self) -> Location {
        Location {
            region: self.region().unwrap_or_default().to_string(),
            position: self.position().unwrap_or_default(),
        }
    }

    fn sitting(&self) -> Sitting {
        match self.seat() {
            Seat::Object(sat) => Sitting::Object(sat.prim),
            Seat::Other => Sitting::Unnamed,
            Seat::None | Seat::Unknown if self.on_ground() => Sitting::Ground,
            Seat::None | Seat::Unknown => Sitting::Standing,
        }
    }

    fn region_of(&self, object: Uuid) -> Option<String> {
        self.region().filter(|_| self.knows(object)).map(str::to_string)
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
            own_local: None,
            own_offset: None,
            seat_frame: None,
            entered: false,
            next_agent_update: now,
            objects: HashMap::new(),
            local_ids: HashMap::new(),
            undescribed: HashSet::new(),
            scan_started: None,
            wanted: Vec::new(),
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
            self.0
                .borrow_mut()
                .calls
                .push(Call::Open(generation, addr, seed_capability.to_string()));
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

    /// A sit the bot asked for, answered naming the same object.
    fn asked(object: Uuid) -> Sat {
        Sat {
            prim: object,
            requested: Some(object),
        }
    }

    /// A sit the simulator made unprompted.
    fn forced(object: Uuid) -> Sat {
        Sat {
            prim: object,
            requested: None,
        }
    }

    impl Rig {
        fn new() -> Rig {
            Rig::with_offline_im(false)
        }

        fn with_offline_im(fetch_offline_im: bool) -> Rig {
            Rig::with(None, fetch_offline_im)
        }

        fn with(scan_distance: Option<f32>, fetch_offline_im: bool) -> Rig {
            let net = FakeNet::default();
            let info = SessionInfo {
                agent_id: agent_id(),
                session_id: Uuid::from_bytes([0x5E; 16]),
                circuit_code: 77,
                sim_addr: "127.0.0.1:9000".parse().unwrap(),
                seed_capability: "http://127.0.0.1:9000/CAPS/seed/".to_string(),
            };
            let now = Instant::now();
            let session = Session::new(net.clone(), &info, "Sit Bot", scan_distance, fetch_offline_im, 1, now);
            Rig {
                session,
                net,
                now,
                sequence: 0,
            }
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
            rig.session.start_sit(target(), rig.now);
            rig.sit_response(target());
            rig.own_update(4242);
            assert_eq!(rig.events(), [SessionEvent::Seated(asked(target()))]);
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
            [Call::Open(
                1,
                "127.0.0.1:9000".parse().unwrap(),
                "http://127.0.0.1:9000/CAPS/seed/".into()
            )]
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
        rig.session.start_sit(target(), rig.now);
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
        assert_eq!(
            rig.calls(),
            [
                Call::Open(1, "127.0.0.1:9000".parse().unwrap(), "http://127.0.0.1:9000/CAPS/seed/".into()),
                Call::Close(1)
            ]
        );
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
        rig.session.start_sit(target(), rig.now);
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
        assert!(
            matches!(events.last(), Some(SessionEvent::Lost(r)) if r.contains("acknowledging")),
            "{events:?}"
        );
    }

    #[test]
    fn event_queue_gone_is_a_loss() {
        let mut rig = Rig::entered();
        rig.session
            .handle_caps(1, CapsEvent::Gone("event queue reports no such session".into()), rig.now);
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
        rig.session
            .handle_caps(1, CapsEvent::Gone("event queue reports no such session".into()), rig.now);
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

        rig.session.start_sit(target(), rig.now);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::AGENT_REQUEST_SIT);
        assert_eq!(sent[0].body[32..48], *target().as_bytes());

        rig.sit_response(target());
        assert_eq!(rig.sent_ids(), [messages::AGENT_SIT]);
        assert_eq!(rig.events(), [], "not seated until the simulator shows the avatar parented");

        rig.own_update(4242);
        assert_eq!(rig.events(), [SessionEvent::Seated(asked(target()))]);
        // Becoming seated asks where the seat is, once.
        assert_eq!(rig.sent_ids(), [messages::REQUEST_MULTIPLE_OBJECTS]);

        // The deadline is disarmed and nothing more is sent about sitting.
        rig.ack_all();
        rig.advance(secs(20));
        assert_eq!(rig.events(), []);
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn sit_fails_when_the_simulator_never_confirms() {
        let mut rig = Rig::entered();
        rig.session.start_sit(target(), rig.now);
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
        rig.session.start_sit(target(), rig.now);
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
        rig.session.start_sit(target(), rig.now);
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
        rig.session.start_sit(target(), rig.now);
        rig.sent();
        // The simulator answers with the prim it chose, not the one requested.
        rig.sit_response(Uuid::from_bytes([0xEE; 16]));
        assert_eq!(rig.sent_ids(), [messages::AGENT_SIT]);
        rig.own_update(555);
        // Both are reported: the object asked for and the prim sat on.
        let sat = Sat {
            prim: Uuid::from_bytes([0xEE; 16]),
            requested: Some(target()),
        };
        assert_eq!(rig.events(), [SessionEvent::Seated(sat)]);
        assert!(sat.is_on(target()) && sat.is_on(Uuid::from_bytes([0xEE; 16])));
        assert!(!sat.is_on(other_object()));
    }

    #[test]
    fn forced_sit_on_a_different_object_is_not_seated_on_target() {
        let mut rig = Rig::entered();
        rig.sit_response(Uuid::from_bytes([0xEE; 16]));
        assert_eq!(rig.sent_ids(), [], "no AgentSit without a sit in progress");
        rig.own_update(555);
        // Seated, but on that object: whether it is the right one is for
        // whoever holds the seat to say.
        let sat = forced(Uuid::from_bytes([0xEE; 16]));
        assert_eq!(rig.events(), [SessionEvent::Seated(sat)]);
        assert_eq!(rig.session.seat(), Seat::Object(sat));
        assert!(!sat.is_on(target()));
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
        assert_eq!(rig.events(), [SessionEvent::Seated(forced(target()))]);
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
            handshake
                .iter()
                .map(|(generation, p)| (*generation, p.id, p.sequence))
                .collect::<Vec<_>>(),
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
        Source::Avatar {
            id: Uuid::from_bytes([0x77; 16]),
            name: name.into(),
        }
    }

    fn message(kind: &str, source: Source, text: &str) -> Message {
        Message {
            kind: kind.into(),
            source,
            sent_at: None,
            text: text.into(),
        }
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
            self.sent_ids()
                .iter()
                .filter(|id| **id == messages::RETRIEVE_INSTANT_MESSAGES)
                .count()
        }
    }

    #[test]
    fn chat_is_reported_with_its_kind() {
        let mut rig = Rig::entered();
        let kinds = [
            (0, "whisper"),
            (1, "say"),
            (2, "shout"),
            (6, "debug"),
            (7, "region"),
            (8, "owner"),
            (9, "direct"),
        ];
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
        let source = Source::Object {
            id: Some(object),
            name: "Sender".into(),
            owner: sender,
        };
        assert_eq!(rig.events(), [SessionEvent::InstantMessage(message("object", source, "done"))]);

        // An older region does not say which object it was.
        rig.im(sender, 19, Uuid::nil(), None, "done");
        let source = Source::Object {
            id: None,
            name: "Sender".into(),
            owner: sender,
        };
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
        let expected = messages::instant_message(&rig.agent(), "Sit Bot", to, [10.0, 20.0, 30.0], 0, "hello");
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
        rig.receive(
            messages::AVATAR_PICKER_REPLY,
            &wire::avatar_picker_reply(query, &[(jane, "Jane", "Doe")]),
        );
        assert_eq!(
            rig.events(),
            [SessionEvent::AvatarsFound {
                query,
                avatars: vec![(jane, "Jane Doe".to_string())]
            }]
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
            let chatty = [
                messages::CHAT_FROM_VIEWER,
                messages::IMPROVED_INSTANT_MESSAGE,
                messages::AVATAR_PICKER_REQUEST,
            ];
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

    // ------------------------------------------------ moving on command

    fn other_object() -> Uuid {
        Uuid::from_bytes([0x0C; 16])
    }

    impl Rig {
        /// Everything sent since the last call, agent updates included.
        fn sent_all(&mut self) -> Vec<Packet> {
            std::mem::take(&mut self.net.0.borrow_mut().sent)
                .into_iter()
                .map(|(_, packet)| packet)
                .filter(|p| p.id != messages::PACKET_ACK)
                .collect()
        }
    }

    /// The control flags and draw distance of an AgentUpdate.
    fn update_fields(packet: &Packet) -> (f32, u32) {
        assert_eq!(packet.id, messages::AGENT_UPDATE);
        let far = f32::from_le_bytes(packet.body[105..109].try_into().unwrap());
        (far, u32::from_le_bytes(packet.body[109..113].try_into().unwrap()))
    }

    #[test]
    fn sits_on_any_object_and_says_which() {
        let mut rig = Rig::seated();
        assert_eq!(rig.session.seat(), Seat::Object(asked(target())));
        rig.session.start_sit(other_object(), rig.now);
        let sent = rig.sent();
        assert_eq!(sent[0].id, messages::AGENT_REQUEST_SIT);
        assert_eq!(sent[0].body[32..48], *other_object().as_bytes());
        // The answer may name another prim of the linkset.
        rig.sit_response(Uuid::from_bytes([0x0D; 16]));
        assert_eq!(rig.sent_ids(), [messages::AGENT_SIT]);
        rig.own_update(777);
        let sat = Sat {
            prim: Uuid::from_bytes([0x0D; 16]),
            requested: Some(other_object()),
        };
        assert_eq!(rig.events(), [SessionEvent::Seated(sat)]);
        assert_eq!(rig.session.seat(), Seat::Object(sat));
    }

    #[test]
    fn failed_sit_elsewhere_leaves_the_avatar_seated_where_it_was() {
        let mut rig = Rig::seated();
        rig.session.start_sit(other_object(), rig.now);
        rig.own_update(4242);
        assert_eq!(rig.events(), [], "still on the target as far as anyone knows");
        rig.ack_all();
        rig.advance(secs(15));
        assert!(matches!(&rig.events()[..], [SessionEvent::SitFailed(_)]));
        assert_eq!(rig.session.seat(), Seat::Object(asked(target())));
    }

    #[test]
    fn sitting_again_where_the_avatar_already_sits_is_confirmed() {
        let mut rig = Rig::seated();
        rig.session.start_sit(target(), rig.now);
        rig.sit_response(target());
        assert_eq!(rig.events(), [SessionEvent::Seated(asked(target()))]);
        rig.ack_all();
        rig.advance(secs(20));
        assert_eq!(rig.events(), [], "the sit ended when it was confirmed");
    }

    #[test]
    fn seat_is_unknown_then_none_then_other() {
        let mut rig = Rig::entered();
        assert_eq!(rig.session.seat(), Seat::Unknown);
        rig.own_update(0);
        assert_eq!(rig.session.seat(), Seat::None);
        // Attached to something nobody confirmed a sit on.
        rig.own_update(31);
        assert_eq!(rig.session.seat(), Seat::Other);
        assert_eq!(rig.events(), [SessionEvent::Standing]);
    }

    #[test]
    fn stand_and_ground_sit_are_single_flagged_updates() {
        let mut rig = Rig::seated();
        rig.sent_all();
        assert_eq!(rig.session.stand(rig.now), Ok(()));
        assert_eq!(rig.session.sit_ground(rig.now), Ok(()));
        let sent = rig.sent_all();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(Packet::is_reliable));
        assert_eq!(update_fields(&sent[0]), (32.0, messages::CONTROL_STAND_UP));
        assert_eq!(update_fields(&sent[1]), (32.0, messages::CONTROL_SIT_ON_GROUND));
        // The periodic update that follows carries no flags.
        rig.ack_all();
        rig.advance(secs(1));
        let sent = rig.sent_all();
        assert_eq!(sent.len(), 1);
        assert_eq!(update_fields(&sent[0]), (32.0, 0));
    }

    #[test]
    fn the_ground_sit_is_believed_until_something_ends_it() {
        let mut rig = Rig::entered();
        assert!(!rig.session.on_ground());
        assert_eq!(rig.session.sit_ground(rig.now), Ok(()));
        assert!(rig.session.on_ground());
        // The avatar's own updates while it is not on an object change nothing.
        rig.own_update(0);
        assert!(rig.session.on_ground());

        // Told to stand.
        assert_eq!(rig.session.stand(rig.now), Ok(()));
        assert!(!rig.session.on_ground());

        // Seated on an object.
        rig.session.sit_ground(rig.now).unwrap();
        rig.own_update(31);
        assert!(!rig.session.on_ground());
        rig.own_update(0);
        assert!(!rig.session.on_ground());

        // A teleport inside the region.
        rig.session.sit_ground(rig.now).unwrap();
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([4.0, 5.0, 6.0]));
        assert!(!rig.session.on_ground());

        // Another simulator.
        rig.session.sit_ground(rig.now).unwrap();
        rig.session.handle_caps(rig.generation(), finish(), rig.now);
        assert!(!rig.session.on_ground());
    }

    #[test]
    fn a_ground_sit_outside_a_region_is_not_believed() {
        let mut rig = Rig::new();
        assert!(rig.session.sit_ground(rig.now).is_err());
        assert!(!rig.session.on_ground());
    }

    #[test]
    fn stand_gives_up_a_sit_in_progress() {
        let mut rig = Rig::entered();
        rig.session.start_sit(target(), rig.now);
        assert_eq!(rig.session.stand(rig.now), Ok(()));
        rig.ack_all();
        rig.advance(secs(20));
        assert_eq!(rig.events(), [], "no SitFailed for a sit that was called off");
    }

    #[test]
    fn region_and_position_are_known_once_entered() {
        let rig = Rig::new();
        assert_eq!((rig.session.region(), rig.session.position()), (None, None));
        let mut rig = Rig::entered();
        assert_eq!(rig.session.region(), Some("Home"));
        assert_eq!(rig.session.position(), Some([10.0, 20.0, 30.0]));
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([1.0, 2.0, 3.0]));
        assert_eq!(rig.session.position(), Some([1.0, 2.0, 3.0]));
    }

    #[test]
    fn teleport_inside_the_current_region_needs_no_lookup() {
        let mut rig = Rig::entered();
        rig.session.start_teleport("home", [100.0, 120.0, 40.0], rig.now);
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        let expected = messages::teleport_location_request(&rig.agent(), HOME_HANDLE, [100.0, 120.0, 40.0]);
        assert_eq!((sent[0].id, &sent[0].body), (messages::TELEPORT_LOCATION_REQUEST, &expected.body));
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([100.0, 120.0, 40.0]));
        assert_eq!(rig.events(), [SessionEvent::TeleportArrived]);
    }

    #[test]
    fn stand_sit_and_offers_are_refused_outside_a_region() {
        let mut rig = Rig::new();
        let now = rig.now;
        let refused = Err(NOT_CONNECTED.to_string());
        assert_eq!(rig.session.stand(now), refused);
        assert_eq!(rig.session.sit_ground(now), refused);
        assert_eq!(rig.session.request_teleport(other_object(), "", now), refused);
        assert_eq!(rig.session.offer_teleport(other_object(), "", now), refused);
        assert_eq!(rig.session.accept_teleport(other_object(), now), refused);
        assert_eq!(rig.session.touch(other_object(), now), Err(NOT_CONNECTED.to_string()));
        let handshake = [messages::USE_CIRCUIT_CODE, messages::COMPLETE_AGENT_MOVEMENT];
        assert_eq!(rig.sent_all().iter().map(|p| p.id).collect::<Vec<_>>(), handshake);
    }

    // --------------------------------------------------- teleport offers

    #[test]
    fn teleport_offers_and_requests_are_reported() {
        let mut rig = Rig::entered();
        let (jane, lure) = (Uuid::from_bytes([0x77; 16]), Uuid::from_bytes([0x44; 16]));
        rig.im(jane, 22, lure, None, "Join me in Elsewhere");
        rig.im(jane, 26, Uuid::nil(), None, "may I come over?");
        assert_eq!(
            rig.events(),
            [
                SessionEvent::TeleportOffer {
                    from: jane,
                    name: "Sender".into(),
                    lure
                },
                SessionEvent::TeleportRequested {
                    from: jane,
                    name: "Sender".into(),
                    text: "may I come over?".into()
                },
            ]
        );
        // Neither is a message for the log, and other offers still are not.
        rig.im(jane, 4, lure, None, "an inventory offer");
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn asks_for_and_offers_teleports() {
        let mut rig = Rig::entered();
        let jane = Uuid::from_bytes([0x77; 16]);
        assert_eq!(rig.session.request_teleport(jane, "may I?", rig.now), Ok(()));
        assert_eq!(rig.session.offer_teleport(jane, "come", rig.now), Ok(()));
        let sent = rig.sent();
        let request = messages::instant_message(&rig.agent(), "Sit Bot", jane, [10.0, 20.0, 30.0], 26, "may I?");
        assert_eq!((sent[0].id, &sent[0].body), (messages::IMPROVED_INSTANT_MESSAGE, &request.body));
        let offer = messages::start_lure(&rig.agent(), "come", jane);
        assert_eq!((sent[1].id, &sent[1].body), (messages::START_LURE, &offer.body));
        assert!(sent.iter().all(Packet::is_reliable));
    }

    #[test]
    fn accepted_offer_is_followed_to_the_other_simulator() {
        let mut rig = Rig::entered();
        let lure = Uuid::from_bytes([0x44; 16]);
        assert_eq!(rig.session.accept_teleport(lure, rig.now), Ok(()));
        let sent = rig.sent();
        let expected = messages::teleport_lure_request(&rig.agent(), lure);
        assert_eq!((sent[0].id, &sent[0].body), (messages::TELEPORT_LURE_REQUEST, &expected.body));
        // A teleport is under way: nothing else is said meanwhile.
        assert_eq!(rig.session.say(0, 1, "hello", rig.now), Err(NOT_CONNECTED.to_string()));

        rig.session.handle_caps(1, finish(), rig.now);
        rig.enter("Elsewhere", HOME_HANDLE + 256);
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Elsewhere".into())]);
    }

    #[test]
    fn offer_that_goes_unanswered_fails_like_any_teleport() {
        let mut rig = Rig::entered();
        rig.session.accept_teleport(Uuid::from_bytes([0x44; 16]), rig.now).unwrap();
        rig.ack_all();
        // Kept alive meanwhile, so that only the teleport's own limit ends it.
        for _ in 0..6 {
            rig.advance(secs(10));
            rig.receive(messages::START_PING_CHECK, &[1, 0, 0, 0, 0]);
            rig.ack_all();
        }
        assert_eq!(rig.events(), [SessionEvent::TeleportFailed("no answer within 60 s".into())]);
    }

    // ---------------------------------------------------------- position

    /// Region-local ids of the avatar and of the seat in these tests.
    const OWN: u32 = 900;
    const CHAIR: u32 = 4242;
    /// A quarter turn about the vertical axis, as x, y and z of its
    /// quaternion and in full.
    const QUARTER: f32 = std::f32::consts::FRAC_1_SQRT_2;

    impl Rig {
        /// The avatar's own full update: where it is, in the region when
        /// `parent` is 0 and on its seat otherwise.
        fn own_at(&mut self, parent: u32, position: Vector3) {
            let data = wire::object_data(position, [0.0; 3], true);
            let block = wire::Block {
                full_id: agent_id(),
                local_id: OWN,
                parent_id: parent,
                data,
            };
            self.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        }

        /// A full update describing the chair.
        fn chair_at(&mut self, position: Vector3, rotation: Vector3) {
            let data = wire::object_data(position, rotation, false);
            let block = wire::Block {
                full_id: target(),
                local_id: CHAIR,
                parent_id: 0,
                data,
            };
            self.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        }

        fn terse(&mut self, local_id: u32, avatar: bool, position: Vector3) {
            let object = wire::Terse {
                local_id,
                avatar,
                position,
                rotation: [0.0, 0.0, 0.0, 1.0],
            };
            self.receive(
                messages::IMPROVED_TERSE_OBJECT_UPDATE,
                &wire::improved_terse_object_update(&[object]),
            );
        }

        fn position(&self) -> Vector3 {
            self.session.position().unwrap()
        }

        /// How many requests about objects went out since the last call.
        fn object_requests(&mut self) -> Vec<Vec<u8>> {
            requests(&self.sent())
        }

        /// Standing at 100, 100, 25, then seated on the chair with the
        /// given offset on it. The chair has not been described yet.
        fn on_chair(offset: Vector3) -> Rig {
            let mut rig = Rig::entered();
            rig.own_at(0, [100.0, 100.0, 25.0]);
            rig.session.start_sit(target(), rig.now);
            rig.sit_response(target());
            rig.sent();
            rig.own_at(CHAIR, offset);
            assert_eq!(rig.events(), [SessionEvent::Standing, SessionEvent::Seated(asked(target()))]);
            rig
        }
    }

    fn near(a: Vector3, b: Vector3) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    #[test]
    fn the_avatars_own_updates_say_where_it_stands() {
        let mut rig = Rig::entered();
        assert_eq!(rig.position(), [10.0, 20.0, 30.0]);
        rig.own_at(0, [100.0, 100.0, 25.0]);
        assert_eq!(rig.position(), [100.0, 100.0, 25.0]);
        assert!(!rig.session.seat_frame_known());
        // Nothing is asked about objects for that.
        assert_eq!(rig.object_requests(), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn seated_position_is_worked_out_from_the_seat() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        // One request about the seat, and until it is answered the avatar
        // is where it was last known to be.
        let expected = messages::request_multiple_objects(&rig.agent(), &[CHAIR]);
        assert_eq!(rig.object_requests(), [expected.body]);
        assert_eq!(rig.position(), [100.0, 100.0, 25.0]);
        assert!(!rig.session.seat_frame_known());

        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        assert!(near(rig.position(), [131.0, 64.5, 23.0]), "{:?}", rig.position());
        assert!(rig.session.seat_frame_known());
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn a_turned_seat_turns_the_offset() {
        // One metre along the seat's own x axis, the seat turned a quarter
        // turn about the vertical: one metre along the region's y axis.
        let mut rig = Rig::on_chair([1.0, 0.0, 0.0]);
        rig.chair_at([131.0, 64.0, 23.0], [0.0, 0.0, QUARTER]);
        assert!(near(rig.position(), [131.0, 65.0, 23.0]), "{:?}", rig.position());
    }

    #[test]
    fn the_seat_may_be_described_in_the_compressed_form() {
        let mut rig = Rig::on_chair([1.0, 0.0, 0.0]);
        let body = wire::object_update_compressed_at(&[(target(), CHAIR, [131.0, 64.0, 23.0], [0.0, 0.0, QUARTER])]);
        rig.receive(messages::OBJECT_UPDATE_COMPRESSED, &body);
        assert!(near(rig.position(), [131.0, 65.0, 23.0]), "{:?}", rig.position());
    }

    #[test]
    fn standing_up_leaves_the_avatar_next_to_the_seat() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        rig.own_at(0, [132.0, 65.0, 23.5]);
        assert_eq!(rig.events(), [SessionEvent::Standing]);
        assert_eq!(rig.position(), [132.0, 65.0, 23.5]);
        assert!(!rig.session.seat_frame_known());
        // The chair moving on is nothing to the avatar now.
        rig.terse(CHAIR, false, [10.0, 10.0, 10.0]);
        assert_eq!(rig.position(), [132.0, 65.0, 23.5]);
    }

    #[test]
    fn a_seat_that_moves_takes_the_avatar_along() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        rig.terse(CHAIR, false, [141.0, 64.0, 22.4]);
        assert!(near(rig.position(), [141.0, 64.5, 23.0]), "{:?}", rig.position());
        // The avatar shifting on the seat counts too.
        rig.terse(OWN, true, [0.0, 0.0, 1.0]);
        assert!(near(rig.position(), [141.0, 64.0, 23.4]), "{:?}", rig.position());
        // Other objects moving do not.
        rig.terse(77, false, [1.0, 2.0, 3.0]);
        assert!(near(rig.position(), [141.0, 64.0, 23.4]));
    }

    #[test]
    fn a_pushed_avatar_is_where_the_simulator_says() {
        let mut rig = Rig::entered();
        rig.own_at(0, [100.0, 100.0, 25.0]);
        rig.terse(OWN, true, [120.0, 100.0, 25.5]);
        assert_eq!(rig.position(), [120.0, 100.0, 25.5]);
        // Another avatar's movement is not the bot's.
        rig.terse(OWN + 1, true, [1.0, 2.0, 3.0]);
        assert_eq!(rig.position(), [120.0, 100.0, 25.5]);
    }

    #[test]
    fn a_seat_that_is_never_described_leaves_the_last_position() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.object_requests();
        for _ in 0..30 {
            rig.ack_all();
            rig.advance(secs(1));
        }
        assert_eq!(rig.events(), [], "still connected and seated");
        assert_eq!(rig.session.seat(), Seat::Object(asked(target())));
        assert_eq!(rig.position(), [100.0, 100.0, 25.0]);
        assert!(!rig.session.seat_frame_known());
        // And it is not asked about again.
        assert_eq!(rig.object_requests(), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn the_seat_is_asked_about_once_per_sit() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        assert_eq!(rig.object_requests().len(), 1);
        // More updates of the seated avatar ask nothing.
        rig.own_at(CHAIR, [0.0, 0.5, 0.7]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        rig.own_at(CHAIR, [0.0, 0.5, 0.6]);
        assert_eq!(rig.object_requests().len(), 0);
        // Standing up asks nothing; the next sit asks again.
        rig.own_at(0, [132.0, 65.0, 23.5]);
        assert_eq!(rig.object_requests().len(), 0);
        rig.own_at(CHAIR + 1, [0.0, 0.0, 0.5]);
        let expected = messages::request_multiple_objects(&rig.agent(), &[CHAIR + 1]);
        assert_eq!(rig.object_requests(), [expected.body]);
    }

    #[test]
    fn a_teleport_inside_the_region_outdates_the_seat() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([5.0, 6.0, 7.0]));
        // The chair moving before the avatar is reported standing must not
        // drag the position back.
        rig.terse(CHAIR, false, [141.0, 64.0, 22.4]);
        assert_eq!(rig.position(), [5.0, 6.0, 7.0]);
        rig.own_at(0, [5.0, 6.0, 7.2]);
        assert_eq!(rig.position(), [5.0, 6.0, 7.2]);
    }

    #[test]
    fn position_tracking_starts_over_in_a_new_region() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        rig.session.handle_caps(rig.generation(), finish(), rig.now);
        rig.enter("Elsewhere", HOME_HANDLE + 256);
        assert_eq!(rig.position(), [10.0, 20.0, 30.0]);
        assert!(!rig.session.seat_frame_known());
        // Local ids of the old region mean nothing here.
        rig.terse(CHAIR, false, [1.0, 2.0, 3.0]);
        rig.terse(OWN, true, [1.0, 2.0, 3.0]);
        assert_eq!(rig.position(), [10.0, 20.0, 30.0]);
    }

    #[test]
    fn an_update_without_a_readable_position_changes_nothing() {
        let mut rig = Rig::entered();
        rig.own_at(0, [100.0, 100.0, 25.0]);
        // The same avatar in a form that carries no motion the bot reads.
        let block = wire::Block {
            full_id: agent_id(),
            local_id: OWN,
            parent_id: 0,
            data: vec![0x12; 32],
        };
        rig.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        assert_eq!(rig.position(), [100.0, 100.0, 25.0]);
        let data = wire::object_data([f32::NAN, 1.0, 2.0], [0.0; 3], true);
        let block = wire::Block {
            full_id: agent_id(),
            local_id: OWN,
            parent_id: 0,
            data,
        };
        rig.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        assert_eq!(rig.position(), [100.0, 100.0, 25.0]);
    }

    #[test]
    fn the_session_tells_persist_where_the_avatar_is_and_how_it_sits() {
        // Just arrived: standing, as far as anyone knows.
        let mut rig = Rig::entered();
        assert_eq!(
            rig.session.here(),
            Location {
                region: "Home".to_string(),
                position: [10.0, 20.0, 30.0]
            }
        );
        assert_eq!(rig.session.sitting(), Sitting::Standing);
        rig.own_at(0, [100.0, 100.0, 25.0]);
        assert_eq!(rig.session.sitting(), Sitting::Standing);
        rig.session.sit_ground(rig.now).unwrap();
        assert_eq!(rig.session.sitting(), Sitting::Ground);

        // Seated: the prim the simulator named, not the object asked for.
        let prim = Uuid::from_bytes([0xEE; 16]);
        rig.session.start_sit(target(), rig.now);
        rig.sit_response(prim);
        rig.own_at(CHAIR, [0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        assert_eq!(rig.session.sitting(), Sitting::Object(prim));
        assert!(near(rig.session.here().position, [131.0, 64.5, 23.0]));

        // Attached to something nobody confirmed a sit on.
        let mut rig = Rig::entered();
        rig.own_update(31);
        assert_eq!(rig.session.sitting(), Sitting::Unnamed);
    }

    #[test]
    fn objects_described_here_are_known_to_be_in_this_region() {
        let mut rig = Rig::entered();
        assert_eq!(rig.session.region_of(target()), None);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        assert!(rig.session.knows(target()));
        assert_eq!(rig.session.region_of(target()), Some("Home".to_string()));
        assert_eq!(rig.session.region_of(other_object()), None);
        // In the next region they are not.
        rig.session.handle_caps(rig.generation(), finish(), rig.now);
        rig.enter("Elsewhere", HOME_HANDLE + 256);
        assert_eq!(rig.session.region_of(target()), None);
    }

    // ----------------------------------------------------------- objects

    fn object(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn requests(packets: &[Packet]) -> Vec<Vec<u8>> {
        packets
            .iter()
            .filter(|p| p.id == messages::REQUEST_MULTIPLE_OBJECTS)
            .map(|p| p.body.clone())
            .collect()
    }

    #[test]
    fn nothing_is_asked_about_objects_before_a_scan() {
        let mut rig = Rig::entered();
        rig.receive(messages::OBJECT_UPDATE, &wire::object_update(&[(object(1), 0)]));
        rig.receive(
            messages::OBJECT_UPDATE_COMPRESSED,
            &wire::object_update_compressed(&[(object(2), 500)]),
        );
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[600, 601]));
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[601]));
        rig.advance(secs(30));
        let sent = rig.sent_all();
        assert!(
            sent.iter().all(|p| p.id == messages::AGENT_UPDATE),
            "{:?}",
            sent.iter().map(|p| p.id).collect::<Vec<_>>()
        );
        assert!(sent.iter().all(|p| update_fields(p) == (32.0, 0)));
    }

    #[test]
    fn touching_a_described_object_presses_and_releases_it() {
        let mut rig = Rig::entered();
        rig.receive(messages::OBJECT_UPDATE, &wire::object_update(&[(object(1), 0)]));
        rig.receive(
            messages::OBJECT_UPDATE_COMPRESSED,
            &wire::object_update_compressed(&[(object(2), 500)]),
        );
        assert_eq!(rig.session.touch(object(2), rig.now), Ok(Touch::Sent));
        assert_eq!(rig.session.touch(object(1), rig.now), Ok(Touch::Sent));
        let sent = rig.sent();
        let ids: Vec<_> = sent.iter().map(|p| p.id).collect();
        let (grab, release) = (messages::OBJECT_GRAB, messages::OBJECT_DEGRAB);
        assert_eq!(ids, [grab, release, grab, release]);
        assert_eq!(sent[0].body, messages::object_grab(&rig.agent(), 500).body);
        assert_eq!(sent[1].body, messages::object_degrab(&rig.agent(), 500).body);
        assert_eq!(sent[2].body, messages::object_grab(&rig.agent(), 100).body);
        assert!(sent.iter().all(Packet::is_reliable));
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn first_touch_of_an_unknown_object_scans_for_it() {
        let mut rig = Rig::entered();
        let many: Vec<u32> = (1000..1300).collect();
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&many[..200]));
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&many[200..]));
        rig.sent();

        assert_eq!(rig.session.touch(object(3), rig.now), Ok(Touch::Waiting));
        // Everything mentioned so far is asked about, 255 at a time.
        let asked = requests(&rig.sent());
        assert_eq!(asked.iter().map(|body| body[32]).collect::<Vec<_>>(), [255, 45]);
        let mut ids: Vec<u32> = asked
            .iter()
            .flat_map(|body| {
                body[33..]
                    .chunks(5)
                    .map(|c| u32::from_le_bytes(c[1..].try_into().unwrap()))
                    .collect::<Vec<_>>()
            })
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, many);

        // From now on a cached update is asked about as it comes.
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[2000]));
        assert_eq!(
            requests(&rig.sent()),
            [messages::request_multiple_objects(&rig.agent(), &[2000]).body]
        );

        // The answer describes the object, and the touch goes out.
        rig.receive(
            messages::OBJECT_UPDATE_COMPRESSED,
            &wire::object_update_compressed(&[(object(3), 2000)]),
        );
        assert_eq!(rig.sent_ids(), [messages::OBJECT_GRAB, messages::OBJECT_DEGRAB]);
        assert_eq!(rig.events(), [SessionEvent::Touched(object(3))]);
        // Known now: later touches go straight out, and it is not asked about again.
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[2000]));
        assert_eq!(rig.session.touch(object(3), rig.now), Ok(Touch::Sent));
        assert_eq!(rig.sent_ids(), [messages::OBJECT_GRAB, messages::OBJECT_DEGRAB]);
    }

    #[test]
    fn touch_gives_up_when_the_scan_does_not_find_the_object() {
        let mut rig = Rig::entered();
        assert_eq!(rig.session.touch(object(3), rig.now), Ok(Touch::Waiting));
        assert!(rig.session.next_deadline() <= rig.now + TOUCH_WAIT);
        rig.advance(secs(9));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        assert_eq!(rig.events(), [SessionEvent::TouchFailed(object(3))]);
        // The scan has had its time: an unknown object is refused at once.
        let refused = rig.session.touch(object(4), rig.now).unwrap_err();
        assert!(refused.contains(&object(4).to_string()) && refused.contains("32 m"), "{refused}");
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn killed_objects_are_forgotten() {
        let mut rig = Rig::entered();
        rig.receive(
            messages::OBJECT_UPDATE_COMPRESSED,
            &wire::object_update_compressed(&[(object(2), 500)]),
        );
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[500]));
        assert_eq!(rig.session.touch(object(2), rig.now), Ok(Touch::Waiting));
        assert_eq!(rig.sent_ids(), [], "nothing to press, and nothing left to ask about");
    }

    #[test]
    fn objects_are_forgotten_with_the_simulator() {
        let mut rig = Rig::entered();
        rig.receive(
            messages::OBJECT_UPDATE_COMPRESSED,
            &wire::object_update_compressed(&[(object(2), 500)]),
        );
        assert_eq!(rig.session.touch(object(9), rig.now), Ok(Touch::Waiting));
        rig.sent();
        rig.session.handle_caps(1, finish(), rig.now);
        assert_eq!(rig.events(), [SessionEvent::TouchFailed(object(9))]);
        rig.enter("Elsewhere", HOME_HANDLE + 256);
        rig.events();
        // A new region: nothing known, and no scan until it is needed again.
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[700]));
        assert_eq!(requests(&rig.sent()), Vec::<Vec<u8>>::new());
        assert_eq!(rig.session.touch(object(2), rig.now), Ok(Touch::Waiting));
        assert_eq!(
            requests(&rig.sent()),
            [messages::request_multiple_objects(&rig.agent(), &[700]).body]
        );
    }

    #[test]
    fn configured_scan_distance_scans_from_the_start() {
        let mut rig = Rig::with(Some(64.0), false);
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[600]));
        rig.enter("Home", HOME_HANDLE);
        rig.events();
        // What was mentioned before the region was entered is asked about on entry...
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[601]));
        assert_eq!(
            requests(&rig.sent()),
            [messages::request_multiple_objects(&rig.agent(), &[601]).body]
        );
        // ...and the simulator is told how far to look.
        rig.advance(secs(1));
        let updates = rig.sent_all();
        assert_eq!(update_fields(updates.last().unwrap()), (64.0, 0));
        // After its time, an unknown object is refused naming that distance.
        rig.advance(secs(10));
        assert!(rig.session.touch(object(4), rig.now).unwrap_err().contains("64 m"));
    }
}
