//! One logged-in session: the simulator circuit, its handshake and
//! keepalive, teleports with the hand-over to a new simulator, sitting, and
//! seat tracking. All I/O goes through `Net`, so the logic is testable with
//! a fake clock and a recording network.

use crate::caps::{CapsEvent, EqEvent, InventoryUrls, TeleportFinish};
use crate::login::SessionInfo;
use crate::persist::{Location, Sitting, World};
use crate::reconciler::same_region;
use crate::scan::Sighting;
use crate::udp::circuit::Circuit;
use crate::udp::messages::{self, Agent, AttachItem, Incoming, Outgoing, Properties, Transform, Vector3};
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
/// How long the simulator has to show an object on the avatar after an
/// attach. Second Life has been seen to take 21 seconds; a viewer gives up
/// after 30.
pub const ATTACH_LIMIT: Duration = Duration::from_secs(30);
/// How long the simulator has to show an object gone from the avatar
/// after a detach.
pub const DETACH_LIMIT: Duration = Duration::from_secs(15);
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
/// The same while a scan runs in the region: the task channel is widened,
/// as it then carries the descriptions and properties of the objects
/// around, which at the rate above take minutes in a busy place. A viewer
/// asks for more than this at its default bandwidth.
const SCAN_THROTTLES: [f32; 7] = [20_000.0, 0.0, 0.0, 0.0, 300_000.0, 0.0, 10_000.0];
/// How long the simulator has to have described nothing new for the
/// objects around the avatar to count as all described.
pub const SCAN_QUIET: Duration = Duration::from_secs(3);
/// How long the simulator is given to answer for an object's properties
/// before it is asked again.
const PROPERTIES_RETRY: Duration = Duration::from_secs(10);
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
    /// All circuits send from one local port. An earlier circuit to the same
    /// address ends here: what that simulator sends from now on is reported
    /// under `generation`.
    fn open(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str);
    fn send(&mut self, generation: u64, datagram: &[u8]);
    /// The avatar has left this circuit's simulator: its event queue is not
    /// listened to any more, its datagrams still come and go.
    fn leave(&mut self, generation: u64);
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
    /// An object `look_for` waited for has been described, and the update
    /// that described it has been taken in whole: its parent and its place
    /// are known as far as that update told them.
    Described(Uuid),
    /// An object `look_for` waited for was not described within the scan's
    /// time, or the region was left or the session ended first.
    NotDescribed(Uuid),
    /// The answer to `find_avatar`: the avatars the grid offers for the
    /// name, as id and `First Last`.
    AvatarsFound {
        query: Uuid,
        avatars: Vec<(Uuid, String)>,
    },
    /// The current simulator's inventory services, as far as it has them.
    InventoryUrls(InventoryUrls),
    /// The simulator shows the object of this inventory item on the avatar,
    /// on this attachment point.
    Attached {
        item: Uuid,
        point: u8,
    },
    /// The object of this inventory item is gone from the avatar; `point`
    /// is where it was.
    Detached {
        item: Uuid,
        point: u8,
    },
    /// An `attach` was not followed by the object within `ATTACH_LIMIT`.
    AttachFailed(Uuid),
    /// A `detach` did not make the object go within `DETACH_LIMIT`.
    DetachFailed(Uuid),
    /// An avatar or an object offers the avatar an inventory item or folder.
    InventoryOffer(InventoryOffer),
    /// A script asks the avatar for permissions.
    PermissionRequest {
        object: Uuid,
        /// The script, as an item of the object's inventory.
        item: Uuid,
        object_name: String,
        /// The owner as the request names it: an avatar's name, or a
        /// group's for a group's object.
        owner_name: String,
        /// The permission bits asked for (`PERMISSION_*` in LSL).
        permissions: i32,
    },
    /// The answer to `request_object_owner`. A group's object has no
    /// owner, or the group as its owner.
    ObjectOwner {
        object: Uuid,
        owner: Uuid,
        group: Uuid,
    },
    /// An object the simulator had described is gone: deleted, detached,
    /// or out of what the simulator shows.
    ObjectGone(Uuid),
    /// The avatar has left a region for another. `worn` are the objects
    /// the old simulator had described that were on the avatar, the prims
    /// of its attachments: they come along, everything else stays behind.
    RegionLeft {
        worn: Vec<Uuid>,
    },
}

/// What kind of thing an object is, as far as the simulator has shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// On a point of the bot avatar's heads-up display.
    Hud,
    /// Attached to the bot's avatar elsewhere.
    Worn,
    /// Attached to this other avatar; nil when the simulator has not
    /// described that avatar.
    WornBy(Uuid),
    /// Rezzed in the region.
    InWorld,
    /// Not described by the simulator.
    Unknown,
}

/// An object as the simulator has described it.
#[derive(Debug, Clone, PartialEq)]
pub struct Thing {
    pub placement: Placement,
    /// The prims of the linkset the object is part of, the object first.
    pub prims: Vec<Uuid>,
    /// How far the object is from the avatar, in metres.
    pub distance: Option<f32>,
}

impl Thing {
    /// An object the simulator has not described.
    pub fn unknown(object: Uuid) -> Thing {
        Thing {
            placement: Placement::Unknown,
            prims: vec![object],
            distance: None,
        }
    }
}

/// The attachment points of the heads-up display.
const HUD_POINTS: std::ops::RangeInclusive<u8> = 31..=38;
/// How many parents are followed from an object: a child prim of an
/// attachment is two below the avatar.
const MAX_DEPTH: usize = 8;

/// An inventory offer as it arrived. The item is in the avatar's inventory
/// already; accepting files it and tells the giver.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryOffer {
    /// The giving avatar; for an object's offer its owner, which is a
    /// group when `from_group` is set.
    pub from: Uuid,
    /// The giving avatar's name, or the giving object's.
    pub name: String,
    pub from_group: bool,
    /// An object gives. The offer does not say which.
    pub from_object: bool,
    /// What the answer has to name.
    pub transaction: Uuid,
    /// The asset type of what is offered; `inventory::ASSET_FOLDER` for a
    /// folder.
    pub asset_type: i32,
    /// What the offer says about the item: its name, and for an object's
    /// offer where the object is.
    pub text: String,
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

/// The asset type an inventory offer's binary bucket names. An avatar's
/// offer carries the type and the item's UUID; an object's the type alone,
/// which older simulators wrote as text in front of a `|`.
fn offered_type(bucket: &[u8], from_object: bool) -> Option<i32> {
    match (from_object, bucket) {
        (false, [asset_type, item @ ..]) if item.len() == 16 => Some(i32::from(*asset_type as i8)),
        (false, _) => None,
        (true, [asset_type]) => Some(i32::from(*asset_type as i8)),
        (true, text) => {
            let digits = text.iter().take_while(|byte| byte.is_ascii_digit()).count();
            std::str::from_utf8(&text[..digits]).ok()?.parse().ok()
        }
    }
}

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

/// What became of looking for an object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Look {
    /// The simulator has described it.
    Known,
    /// A scan is looking for it. Ends in `Described` or `NotDescribed`.
    Waiting,
    /// The scan in this region has had its time and did not turn it up.
    GaveUp,
}

/// An object the simulator describes as made from an inventory item and
/// worn: the root prim of an attachment, on the bot's avatar or another.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Attachment {
    local_id: u32,
    /// Region-local id of what it is attached to.
    parent: u32,
    point: u8,
    /// Reported as attached to the bot's own avatar.
    reported: bool,
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
    addr: SocketAddr,
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
    /// The parent of each described object whose update named one, by
    /// region-local id; zero for an object without a parent.
    parents: HashMap<u32, u32>,
    /// Where each described object is, as last reported: in the region,
    /// or relative to its parent when it has one.
    places: HashMap<u32, Vector3>,
    /// What kind of object each described object is, and its flags: the
    /// PCode and UpdateFlags of the last update that described it.
    kinds: HashMap<u32, (u8, u32)>,
    /// When the simulator last described an object the bot did not know.
    last_new: Option<Instant>,
    /// Objects mentioned as cached, which the bot would have to ask about.
    undescribed: HashSet<u32>,
    /// When the bot started asking about them, if it has.
    scan_started: Option<Instant>,
    /// The largest distance a `scan` command named in this region, when
    /// that is more than the configured one, and when it was last raised.
    reach: Option<f32>,
    raised_at: Option<Instant>,
    /// What the simulator has said of the objects a `scan` asked about:
    /// what they are called and whose they are.
    properties: HashMap<u32, Properties>,
    /// When the simulator was asked for the properties of each object it
    /// has not answered for.
    asked: HashMap<u32, Instant>,
    /// The root prim of the seat whose name has been asked for since the
    /// avatar last became seated, and what the simulator said of it.
    seat_asked: Option<u32>,
    seat_named: Option<(u32, Properties)>,
    /// Touches waiting for their object to be described, with their deadline.
    wanted: Vec<(Uuid, Instant)>,
    /// Objects `look_for` waits to see described, with their deadline.
    sought: Vec<(Uuid, Instant)>,
    /// Attachments the simulator has described, by inventory item. One
    /// counts as attached to the avatar while its parent is `own_local`:
    /// the avatar's own description may come after its attachments'.
    attachments: HashMap<Uuid, Attachment>,
    /// Items an `attach` is waiting to see on the avatar, with the deadline.
    awaited_attach: Vec<(Uuid, Instant)>,
    /// Items a `detach` is waiting to see gone, with the deadline.
    awaited_detach: Vec<(Uuid, Instant)>,
}

/// The circuit to a simulator the avatar has teleported out of. Such a
/// simulator may keep the avatar's presence (a child agent) and with it the
/// circuit, and then expects the avatar back on it, so it is kept up, with
/// acks and ping replies and nothing else, until that simulator ends it.
struct Left {
    generation: u64,
    addr: SocketAddr,
    circuit: Circuit,
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
    /// The circuits to the simulators the avatar has left, while they last.
    left: Vec<Left>,
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
            sim: Sim::new(0, info.sim_addr, Circuit::new(now), now),
            left: Vec::new(),
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
        session.open_circuit(generation, info.sim_addr, Circuit::new(now), &info.seed_capability, now);
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
        consider(sim.wanted.iter().chain(&sim.sought).map(|(_, deadline)| *deadline).min());
        consider(
            sim.awaited_attach
                .iter()
                .chain(&sim.awaited_detach)
                .map(|(_, deadline)| *deadline)
                .min(),
        );
        consider(self.teleport.as_ref().map(|t| t.deadline));
        consider(self.logout_deadline);
        for left in &self.left {
            consider(Some(left.circuit.last_receive() + SILENCE_LIMIT));
            consider(left.circuit.next_deadline());
        }
        deadline
    }

    /// Makes the simulator at `addr` the avatar's, over `circuit`, and
    /// returns the one it was. Whatever was in progress there is void.
    fn open_circuit(&mut self, generation: u64, addr: SocketAddr, circuit: Circuit, seed_capability: &str, now: Instant) -> Sim {
        self.fail_touches();
        self.fail_attaches();
        if self.sim.entered {
            let worn = self.worn_prims();
            self.events.push(SessionEvent::RegionLeft { worn });
        }
        let old = std::mem::replace(&mut self.sim, Sim::new(generation, addr, circuit, now));
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
        old
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
            self.fail_attaches();
            self.net.close(self.sim.generation);
            for left in self.left.drain(..) {
                self.net.close(left.generation);
            }
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
        if self.sim.local_ids.contains_key(&local_id) {
            self.sim.places.insert(local_id, transform.position);
        }
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

    /// How far around the avatar the simulator is asked to describe
    /// objects: what the config says, or what a `scan` command has raised
    /// it to in this region.
    fn draw_distance(&self) -> f32 {
        let configured = self.scan_distance.unwrap_or(DRAW_DISTANCE);
        self.sim.reach.map_or(configured, |reach| reach.max(configured))
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

    /// Teleports to `position` in the region with this handle, which is
    /// its south-west corner in metres. Nothing is looked up. Ends as
    /// `start_teleport` does.
    pub fn start_teleport_to(&mut self, handle: u64, position: Vector3, now: Instant) {
        self.send(messages::teleport_location_request(&self.agent, handle, position), true, now);
        self.teleport = Some(Teleport {
            region: String::new(),
            position,
            phase: TeleportPhase::Requested,
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

    /// Sends the avatar `to` an automatic answer, as a viewer in Do Not
    /// Disturb mode does.
    pub fn autoresponse(&mut self, to: Uuid, text: &str, now: Instant) -> Result<(), String> {
        self.send_im(to, messages::IM_AUTO_RESPONSE, text, now)
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

    /// Asks the simulator who owns `object`; an `ObjectOwner` follows if
    /// it knows the object.
    pub fn request_object_owner(&mut self, object: Uuid, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::request_object_properties_family(&self.agent, object), true, now);
        Ok(())
    }

    /// Grants the script `item` in `object` the permissions it asked for
    /// with a `PermissionRequest`.
    pub fn grant_permissions(&mut self, object: Uuid, item: Uuid, permissions: i32, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::script_answer_yes(&self.agent, object, item, permissions), true, now);
        Ok(())
    }

    /// Accepts an inventory offer: tells whoever gave, `to`, and has the
    /// item filed in `folder`.
    pub fn accept_inventory(&mut self, to: Uuid, from_object: bool, transaction: Uuid, folder: Uuid, now: Instant) -> Result<(), String> {
        self.present()?;
        let position = self.sim.position.unwrap_or_default();
        let offered = match from_object {
            true => messages::IM_TASK_INVENTORY_OFFERED,
            false => messages::IM_INVENTORY_OFFERED,
        };
        let message = messages::inventory_accepted(&self.agent, &self.agent_name, to, position, offered, transaction, folder);
        self.send(message, true, now);
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
        self.send(messages::agent_throttle(&self.agent, self.circuit_code, SCAN_THROTTLES), true, now);
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
    fn learn(&mut self, object: Uuid, local_id: u32, kind: (u8, u32), now: Instant) {
        let sim = &mut self.sim;
        sim.undescribed.remove(&local_id);
        if sim.objects.len() < MAX_OBJECTS || sim.objects.contains_key(&object) {
            // The simulator may have given the number to another object
            // without saying the first is gone: what was known under it,
            // its name among it, was the first one's.
            if let Some(before) = sim.local_ids.get(&local_id).copied().filter(|before| *before != object) {
                sim.objects.remove(&before);
                sim.forget(local_id);
            }
            match sim.objects.insert(object, local_id) {
                Some(old) if old != local_id => sim.forget(old),
                Some(_) => {}
                None => sim.last_new = Some(now),
            }
            sim.local_ids.insert(local_id, object);
            sim.kinds.insert(local_id, kind);
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
        for (object, _) in std::mem::take(&mut self.sim.sought) {
            self.events.push(SessionEvent::NotDescribed(object));
        }
    }

    /// Reports the objects `look_for` waits for that an update just
    /// described. Called once the whole update has been taken in.
    fn report_sought(&mut self) {
        if self.sim.sought.is_empty() {
            return;
        }
        let objects = &self.sim.objects;
        let (found, sought) = std::mem::take(&mut self.sim.sought)
            .into_iter()
            .partition(|(object, _)| objects.contains_key(object));
        self.sim.sought = sought;
        for (object, _) in found {
            self.events.push(SessionEvent::Described(object));
        }
    }

    /// The deadline of a wait for an object that starts now: 10 seconds
    /// after the scan in this region started, which this starts if need be.
    fn scan_deadline(&mut self, now: Instant) -> Instant {
        self.start_scan(now);
        self.sim.scan_started.unwrap_or(now) + TOUCH_WAIT
    }

    /// Clicks `object` once. An object the simulator has not described is
    /// looked for with a scan, if none has run its course yet.
    pub fn touch(&mut self, object: Uuid, now: Instant) -> Result<Touch, String> {
        self.present()?;
        if let Some(local_id) = self.sim.objects.get(&object).copied() {
            self.send_touch(local_id, now);
            return Ok(Touch::Sent);
        }
        let deadline = self.scan_deadline(now);
        if now >= deadline {
            return Err(self.unknown_object(object));
        }
        if !self.sim.wanted.iter().any(|(wanted, _)| *wanted == object) {
            self.sim.wanted.push((object, deadline));
        }
        Ok(Touch::Waiting)
    }

    /// Looks for `object` without doing anything to it: known when the
    /// simulator has described it; otherwise the scan is started if need
    /// be, and the object is waited for until 10 seconds after the scan
    /// started, which a touch shares.
    pub fn look_for(&mut self, object: Uuid, now: Instant) -> Result<Look, String> {
        self.present()?;
        if self.knows(object) {
            return Ok(Look::Known);
        }
        let deadline = self.scan_deadline(now);
        if now >= deadline {
            return Ok(Look::GaveUp);
        }
        if !self.sim.sought.iter().any(|(sought, _)| *sought == object) {
            self.sim.sought.push((object, deadline));
        }
        Ok(Look::Waiting)
    }

    /// Starts the scan in this region, if it has not started, without
    /// waiting for anything in particular. Nothing while the avatar is
    /// not in a region.
    pub fn scan(&mut self, now: Instant) {
        if self.present().is_ok() {
            self.start_scan(now);
        }
    }

    /// Makes ready for a `scan` command that reaches `distance` metres,
    /// or as far as the scan in this region does: starts that scan if it
    /// has not started, and has the simulator describe objects further
    /// away than before when `distance` is more than it was asked for.
    /// That lasts until the avatar leaves the region. Returns how far the
    /// command reaches.
    pub fn reach(&mut self, distance: Option<f32>, now: Instant) -> Result<f32, String> {
        self.present()?;
        self.start_scan(now);
        let Some(distance) = distance else {
            return Ok(self.draw_distance());
        };
        if distance > self.draw_distance() {
            self.sim.reach = Some(distance);
            self.sim.raised_at = Some(now);
            crate::info!("scan distance raised to {distance} m");
            // The simulator is told now, not with the next update.
            self.sim.next_agent_update = now + AGENT_UPDATE_INTERVAL;
            let camera = self.sim.position.unwrap_or_default();
            self.send(messages::agent_update(&self.agent, camera, distance, 0), false, now);
        }
        Ok(distance)
    }

    /// When the objects around the avatar count as all described: once
    /// the simulator has described nothing new for `SCAN_QUIET`, counted
    /// from the start of the scan and from the last raise of its distance
    /// at the earliest.
    pub fn settled_at(&self) -> Instant {
        let sim = &self.sim;
        let last = [sim.scan_started, sim.raised_at, sim.last_new].into_iter().flatten().max();
        last.unwrap_or(sim.opened_at) + SCAN_QUIET
    }

    /// Whether the object with this local id is an avatar: the simulator
    /// described it as one, or it is the bot's own.
    fn is_avatar(&self, local_id: u32) -> bool {
        self.sim.own_local == Some(local_id) || self.sim.kinds.get(&local_id).is_some_and(|kind| kind.0 == messages::PCODE_AVATAR)
    }

    /// What the simulator has said the object with this local id is
    /// called and whose it is: in answer to a `scan`, or about the seat.
    fn known(&self, local_id: u32) -> Option<&Properties> {
        let seat = self.sim.seat_named.as_ref().filter(|(seat, _)| *seat == local_id);
        self.sim.properties.get(&local_id).or(seat.map(|(_, properties)| properties))
    }

    /// Every prim the simulator has described whose place is known, with
    /// how far it is from the avatar: a child prim counts as being where
    /// its linkset's root is, and what an avatar wears as being where
    /// that avatar is. No avatars, trees or grass.
    pub fn sightings(&self) -> Vec<Sighting> {
        let sim = &self.sim;
        let Some(avatar) = sim.position else {
            return Vec::new();
        };
        let prims = sim.local_ids.iter().filter_map(|(local_id, object)| {
            let (pcode, flags) = *sim.kinds.get(local_id)?;
            if pcode != messages::PCODE_PRIM || self.is_avatar(*local_id) {
                return None;
            }
            let lineage = self.lineage(*local_id);
            let wearer = lineage.iter().skip(1).find(|above| self.is_avatar(**above));
            let at = match wearer {
                Some(wearer) if sim.own_local == Some(*wearer) => avatar,
                // An avatar that sits is where its seat is.
                _ => *sim.places.get(lineage.last()?)?,
            };
            Some(Sighting {
                object: *object,
                local_id: *local_id,
                distance: [0, 1, 2].map(|i| (at[i] - avatar[i]).powi(2)).iter().sum::<f32>().sqrt(),
                flags,
                attachment: wearer.is_some(),
                child: lineage.get(1).is_some_and(|parent| !self.is_avatar(*parent)),
                properties: self.known(*local_id).cloned(),
            })
        });
        prims.collect()
    }

    /// Asks the simulator what the objects with these local ids are
    /// called and whose they are, the way a viewer's area search does: by
    /// selecting them and ending the selection at once. Objects it has
    /// answered for, and those asked about less than 10 seconds ago, are
    /// left out.
    pub fn ask_properties(&mut self, local_ids: &[u32], now: Instant) {
        if self.present().is_err() {
            return;
        }
        let sim = &self.sim;
        let waiting = |id: &u32| sim.asked.get(id).is_some_and(|at| now.duration_since(*at) < PROPERTIES_RETRY);
        let mut due: Vec<u32> = local_ids
            .iter()
            .copied()
            .filter(|id| sim.local_ids.contains_key(id) && self.known(*id).is_none() && !waiting(id))
            .collect();
        due.sort_unstable();
        due.dedup();
        for batch in due.chunks(messages::SELECT_BATCH) {
            self.send(messages::object_select(&self.agent, batch), true, now);
            self.send(messages::object_deselect(&self.agent, batch), true, now);
        }
        for id in due {
            self.sim.asked.insert(id, now);
        }
    }

    /// The name of the object the avatar sits on: that of the root prim
    /// of its linkset, once the simulator has told it.
    pub fn seat_name(&self) -> Option<&str> {
        let seat = self.own_parent.filter(|parent| *parent != 0)?;
        self.known(seat).map(|properties| properties.name.as_str())
    }

    /// Asks the simulator for the name of the object the avatar sits on,
    /// once per sit and as soon as that object's root prim is described.
    /// Without selecting it: a seat the avatar owns may be a vehicle on
    /// the move, which a selection would hold up.
    fn ask_seat_name(&mut self, now: Instant) {
        let Some(seat) = self.own_parent.filter(|parent| *parent != 0) else {
            return;
        };
        if self.sim.seat_asked == Some(seat) || self.sim.properties.contains_key(&seat) {
            return;
        }
        let Some(object) = self.sim.local_ids.get(&seat).copied() else {
            return;
        };
        self.sim.seat_asked = Some(seat);
        self.send(messages::request_object_properties_family(&self.agent, object), true, now);
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

    // -------------------------------------------------------- attachments

    /// The inventory items attached to the avatar now, with their points,
    /// as far as the simulator has shown them.
    pub fn attached(&self) -> Vec<(Uuid, u8)> {
        let own = self.sim.own_local;
        let worn = self.sim.attachments.iter().filter(|(_, worn)| own == Some(worn.parent));
        worn.map(|(item, worn)| (*item, worn.point)).collect()
    }

    /// The described object with this region-local id and its parents, up
    /// to the one that has none or whose parent is not known.
    fn lineage(&self, local_id: u32) -> Vec<u32> {
        let mut lineage = vec![local_id];
        while lineage.len() < MAX_DEPTH {
            match self.sim.parents.get(&lineage[lineage.len() - 1]) {
                Some(parent) if *parent != 0 && !lineage.contains(parent) => lineage.push(*parent),
                _ => break,
            }
        }
        lineage
    }

    /// The attachment the object with this local id is, or is a part of,
    /// with the inventory item it was made from.
    fn attachment_of(&self, local_id: u32) -> Option<(Uuid, Attachment)> {
        let lineage = self.lineage(local_id);
        let root = |(item, worn): (&Uuid, &Attachment)| lineage.contains(&worn.local_id).then_some((*item, *worn));
        self.sim.attachments.iter().find_map(root)
    }

    /// The prims of the linkset whose root has this local id: the root
    /// and what names it as its parent.
    fn linkset(&self, root: u32) -> Vec<Uuid> {
        let children = self
            .sim
            .parents
            .iter()
            .filter(|(_, parent)| **parent == root)
            .map(|(child, _)| *child);
        let prims = std::iter::once(root).chain(children);
        prims.filter_map(|local_id| self.sim.local_ids.get(&local_id).copied()).collect()
    }

    /// Where in the region the object with this local id is: where the
    /// topmost of its parents is.
    fn place(&self, local_id: u32) -> Option<Vector3> {
        let lineage = self.lineage(local_id);
        self.sim.places.get(lineage.last()?).copied()
    }

    /// The inventory item of the attachment on the avatar that `object`
    /// is, or is a prim of, as far as the simulator has described it.
    pub fn attached_item(&self, object: Uuid) -> Option<Uuid> {
        let local_id = *self.sim.objects.get(&object)?;
        let own = self.sim.own_local?;
        let (item, worn) = self.attachment_of(local_id)?;
        (worn.parent == own).then_some(item)
    }

    /// The prims of the attachment on the avatar made from `item`.
    pub fn attachment_prims(&self, item: Uuid) -> Vec<Uuid> {
        match self.sim.attachments.get(&item) {
            Some(worn) if Some(worn.parent) == self.sim.own_local => self.linkset(worn.local_id),
            _ => Vec::new(),
        }
    }

    /// The prims of everything attached to the avatar.
    fn worn_prims(&self) -> Vec<Uuid> {
        let own = self.sim.own_local;
        let worn = self.sim.attachments.values().filter(|worn| Some(worn.parent) == own);
        worn.flat_map(|worn| self.linkset(worn.local_id)).collect()
    }

    /// What `object` is, which prims belong to it, and how far it is from
    /// the avatar, as far as the simulator has described it.
    pub fn thing(&self, object: Uuid) -> Thing {
        let Some(local_id) = self.sim.objects.get(&object).copied() else {
            return Thing::unknown(object);
        };
        let lineage = self.lineage(local_id);
        let (placement, root, at) = match self.attachment_of(local_id) {
            Some((_, worn)) if Some(worn.parent) == self.sim.own_local => {
                let placement = if HUD_POINTS.contains(&worn.point) {
                    Placement::Hud
                } else {
                    Placement::Worn
                };
                (placement, worn.local_id, self.sim.position)
            }
            Some((_, worn)) => {
                let wearer = self.sim.local_ids.get(&worn.parent).copied().unwrap_or_default();
                (Placement::WornBy(wearer), worn.local_id, self.place(worn.parent))
            }
            None => {
                let root = lineage[lineage.len() - 1];
                (Placement::InWorld, root, self.place(root))
            }
        };
        let mut prims = vec![object];
        prims.extend(self.linkset(root).into_iter().filter(|prim| *prim != object));
        let distance = match (at, self.sim.position) {
            (Some(at), Some(avatar)) => Some([0, 1, 2].map(|i| (at[i] - avatar[i]).powi(2)).iter().sum::<f32>().sqrt()),
            _ => None,
        };
        Thing {
            placement,
            prims,
            distance,
        }
    }

    /// The simulator described an object that carries an inventory item's
    /// id: an attachment, on this avatar or on another.
    fn attachment_described(&mut self, item: Uuid, local_id: u32, parent: u32, point: u8) {
        let known = self.sim.attachments.get(&item).copied();
        let reported = known.is_some_and(|known| known.reported && known.parent == parent && known.point == point);
        let attachment = Attachment {
            local_id,
            parent,
            point,
            reported,
        };
        self.sim.attachments.insert(item, attachment);
        if let Some(known) = known.filter(|known| known.reported && !reported) {
            // On another point or another avatar than it was: gone from
            // where it was first.
            self.detached(item, known.point);
        }
        self.report_attachments();
    }

    /// Reports the attachments that are on the avatar and were not known
    /// to be: after one is described, and after the avatar itself is.
    fn report_attachments(&mut self) {
        let Some(own) = self.sim.own_local else {
            return;
        };
        let mut found = Vec::new();
        for (item, worn) in &mut self.sim.attachments {
            if worn.parent == own && !worn.reported {
                worn.reported = true;
                found.push((*item, worn.point));
            }
        }
        for (item, point) in found {
            self.sim.awaited_attach.retain(|(awaited, _)| *awaited != item);
            self.events.push(SessionEvent::Attached { item, point });
        }
    }

    fn detached(&mut self, item: Uuid, point: u8) {
        self.sim.awaited_detach.retain(|(awaited, _)| *awaited != item);
        self.events.push(SessionEvent::Detached { item, point });
    }

    /// The object with this region-local id is gone.
    fn attachment_killed(&mut self, local_id: u32) {
        let gone: Vec<Uuid> = self
            .sim
            .attachments
            .iter()
            .filter(|(_, worn)| worn.local_id == local_id)
            .map(|(item, _)| *item)
            .collect();
        for item in gone {
            if let Some(worn) = self.sim.attachments.remove(&item).filter(|worn| worn.reported) {
                self.detached(item, worn.point);
            }
        }
    }

    fn fail_attaches(&mut self) {
        for (item, _) in std::mem::take(&mut self.sim.awaited_attach) {
            self.events.push(SessionEvent::AttachFailed(item));
        }
        for (item, _) in std::mem::take(&mut self.sim.awaited_detach) {
            self.events.push(SessionEvent::DetachFailed(item));
        }
    }

    /// Attaches the object of an inventory item to the avatar: to `point`,
    /// or with 0 where the simulator puts it, which is where it was last.
    /// Nothing is taken off to make room. Ends in `Attached` for the item
    /// or, after `ATTACH_LIMIT`, in `AttachFailed`.
    pub fn attach(&mut self, item: &AttachItem, point: u8, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::rez_single_attachment_from_inv(&self.agent, item, point), true, now);
        self.sim.awaited_attach.retain(|(awaited, _)| *awaited != item.item_id);
        self.sim.awaited_attach.push((item.item_id, now + ATTACH_LIMIT));
        Ok(())
    }

    /// Detaches the object of an inventory item into the inventory. Ends
    /// in `Detached` for the item or, after `DETACH_LIMIT`, in
    /// `DetachFailed`.
    pub fn detach(&mut self, item: Uuid, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::detach_attachment_into_inv(&self.agent, item), true, now);
        self.sim.awaited_detach.retain(|(awaited, _)| *awaited != item);
        self.sim.awaited_detach.push((item, now + DETACH_LIMIT));
        Ok(())
    }

    // ------------------------------------------------ animations and sounds

    /// Starts or stops an animation on the avatar; `animation` is the
    /// asset's id.
    pub fn animate(&mut self, animation: Uuid, start: bool, now: Instant) -> Result<(), String> {
        self.present()?;
        self.send(messages::agent_animation(&self.agent, animation, start), true, now);
        Ok(())
    }

    /// Plays a sound once where the avatar is, at full volume; `sound` is
    /// the asset's id.
    pub fn play_sound(&mut self, sound: Uuid, now: Instant) -> Result<(), String> {
        self.present()?;
        let handle = self.sim.region_handle.ok_or(NOT_CONNECTED)?;
        let position = self.sim.position.unwrap_or_default();
        self.send(messages::sound_trigger(sound, handle, position, 1.0), true, now);
        Ok(())
    }

    // ------------------------------------------------------------- timers

    pub fn tick(&mut self, now: Instant) {
        if self.lost {
            return;
        }
        self.tick_left(now);
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
        let (late, sought) = std::mem::take(&mut self.sim.sought).into_iter().partition(|(_, at)| now >= *at);
        self.sim.sought = sought;
        for (object, _) in late {
            self.events.push(SessionEvent::NotDescribed(object));
        }
        let (late, awaited) = std::mem::take(&mut self.sim.awaited_attach)
            .into_iter()
            .partition(|(_, at)| now >= *at);
        self.sim.awaited_attach = awaited;
        for (item, _) in late {
            self.events.push(SessionEvent::AttachFailed(item));
        }
        let (late, awaited) = std::mem::take(&mut self.sim.awaited_detach)
            .into_iter()
            .partition(|(_, at)| now >= *at);
        self.sim.awaited_detach = awaited;
        for (item, _) in late {
            self.events.push(SessionEvent::DetachFailed(item));
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
        if self.lost {
            return;
        }
        if generation != self.sim.generation {
            return self.handle_left_datagram(generation, datagram, now);
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
            CapsEvent::Inventory(urls) => self.events.push(SessionEvent::InventoryUrls(urls)),
        }
    }

    /// The avatar now belongs to another simulator: connect there, over the
    /// circuit the bot still has to it when the avatar has been there, and
    /// keep this one up for as long as its simulator does. Whatever was in
    /// progress here is void.
    fn hand_over(&mut self, finish: TeleportFinish, now: Instant) {
        crate::debug!("teleport finishing at {}", finish.sim_addr);
        let here = finish.sim_addr == self.sim.addr;
        let kept = if here {
            // Not something a simulator is known to do; its client is the
            // one this circuit has, so the numbering must go on all the same.
            Some(std::mem::replace(&mut self.sim.circuit, Circuit::new(now)))
        } else {
            let at = self.left.iter().position(|left| left.addr == finish.sim_addr);
            at.map(|at| self.left.swap_remove(at).circuit)
        };
        let circuit = match kept {
            Some(mut circuit) => {
                crate::debug!("continuing the circuit to {}", finish.sim_addr);
                circuit.resume(now);
                circuit
            }
            None => Circuit::new(now),
        };
        self.net.leave(self.sim.generation);
        let generation = self.sim.generation + 1;
        let old = self.open_circuit(generation, finish.sim_addr, circuit, &finish.seed_capability, now);
        self.sim.region_handle = Some(finish.region_handle);
        if !here {
            self.left.push(Left {
                generation: old.generation,
                addr: old.addr,
                circuit: old.circuit,
            });
        }
    }

    /// A datagram from a simulator the avatar has left. It is acknowledged
    /// and its pings are answered, so that the simulator keeps the circuit
    /// for the avatar's return; nothing in it is acted on, and nothing it
    /// does ends the session.
    fn handle_left_datagram(&mut self, generation: u64, datagram: &[u8], now: Instant) {
        let Some(at) = self.left.iter().position(|left| left.generation == generation) else {
            return;
        };
        let Ok(packet) = packet::parse(datagram) else {
            return;
        };
        let left = &mut self.left[at];
        if !left.circuit.receive(&packet, now) {
            return;
        }
        // Whatever else it sends, about its objects and avatars, is not
        // even decoded.
        const HEEDED: [packet::MessageId; 5] = [
            messages::START_PING_CHECK,
            messages::PACKET_ACK,
            messages::DISABLE_SIMULATOR,
            messages::KICK_USER,
            messages::LOGOUT_REPLY,
        ];
        if !HEEDED.contains(&packet.id) {
            return;
        }
        match messages::decode(packet.id, &packet.body) {
            Ok(Incoming::StartPingCheck { ping_id }) => {
                let reply = left.circuit.send(&messages::complete_ping_check(ping_id), false, now);
                self.net.send(generation, &reply);
            }
            Ok(Incoming::PacketAck(sequences)) => left.circuit.acknowledged(&sequences),
            Ok(Incoming::DisableSimulator | Incoming::KickUser { .. } | Incoming::LogoutReply) => {
                crate::debug!("the simulator at {} closed the circuit the avatar left", left.addr);
                self.left.swap_remove(at);
                self.net.close(generation);
            }
            _ => {}
        }
    }

    /// Flushes acks and resends on the circuits the avatar has left, and
    /// gives up those whose simulator has gone quiet.
    fn tick_left(&mut self, now: Instant) {
        let mut at = 0;
        while at < self.left.len() {
            let left = &mut self.left[at];
            let poll = left.circuit.poll(now);
            for datagram in &poll.datagrams {
                self.net.send(left.generation, datagram);
            }
            if poll.exhausted || now.duration_since(left.circuit.last_receive()) >= SILENCE_LIMIT {
                crate::debug!("giving up the circuit to {}, which the avatar left", left.addr);
                let generation = left.generation;
                self.left.swap_remove(at);
                self.net.close(generation);
            } else {
                at += 1;
            }
        }
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
                    self.learn(object.full_id, object.local_id, (object.pcode, object.flags), now);
                    if self.sim.local_ids.contains_key(&object.local_id) {
                        self.sim.parents.insert(object.local_id, object.parent_id);
                    }
                }
                for object in &objects {
                    if object.full_id != own_id {
                        if let Some(transform) = object.transform {
                            self.moved(object.local_id, transform);
                        }
                        if let Some(item) = object.attach_item {
                            self.attachment_described(item, object.local_id, object.parent_id, object.attachment_point());
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
                        self.sim.seat_asked = None;
                        self.sim.seat_named = None;
                        if object.parent_id != 0 {
                            self.request_objects(&[object.parent_id], now);
                        }
                    }
                    self.own_parent = Some(object.parent_id);
                    self.sim.own_local = Some(object.local_id);
                    self.report_attachments();
                    if let Some(transform) = object.transform {
                        self.sim.own_offset = Some(transform.position);
                        self.update_position();
                    }
                    self.update_seat();
                }
                self.report_sought();
                self.ask_seat_name(now);
            }
            Incoming::ObjectsCompressed(objects) => {
                for object in objects {
                    self.learn(object.full_id, object.local_id, (object.pcode, object.flags), now);
                    if let Some(parent) = object.parent_id.filter(|_| self.sim.local_ids.contains_key(&object.local_id)) {
                        self.sim.parents.insert(object.local_id, parent);
                    }
                    if let Some(transform) = object.transform {
                        self.moved(object.local_id, transform);
                    }
                    if let Some(worn) = object.attachment {
                        self.attachment_described(worn.item, object.local_id, worn.parent_id, worn.point);
                    }
                }
                self.report_sought();
                self.ask_seat_name(now);
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
                    let gone = self.sim.local_ids.get(&local_id).copied();
                    self.sim.forget(local_id);
                    if let Some(object) = gone {
                        self.sim.objects.remove(&object);
                    }
                    self.attachment_killed(local_id);
                    if let Some(object) = gone {
                        self.events.push(SessionEvent::ObjectGone(object));
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
                from_group,
                from_name,
                dialog,
                id,
                offline,
                timestamp,
                text,
                bucket,
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
                    messages::IM_INVENTORY_OFFERED | messages::IM_TASK_INVENTORY_OFFERED => {
                        let from_object = dialog == messages::IM_TASK_INVENTORY_OFFERED;
                        let Some(asset_type) = offered_type(&bucket, from_object) else {
                            return crate::debug!("ignoring a malformed inventory offer from {from_name} <{from_agent}>");
                        };
                        let offer = InventoryOffer {
                            from: from_agent,
                            name: from_name,
                            from_group,
                            from_object,
                            transaction: id,
                            asset_type,
                            text,
                        };
                        return self.events.push(SessionEvent::InventoryOffer(offer));
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
            Incoming::ScriptQuestion {
                object,
                item,
                object_name,
                owner_name,
                questions,
            } => self.events.push(SessionEvent::PermissionRequest {
                object,
                item,
                object_name,
                owner_name,
                permissions: questions,
            }),
            Incoming::ObjectPropertiesFamily {
                object,
                owner,
                group,
                named,
            } => {
                self.events.push(SessionEvent::ObjectOwner { object, owner, group });
                // The answer about the seat, when it is one.
                let seat = self.sim.seat_asked.filter(|seat| self.sim.local_ids.get(seat) == Some(&object));
                if let (Some(seat), Some((name, description))) = (seat, named) {
                    let properties = Properties {
                        object,
                        owner,
                        group,
                        name,
                        description,
                    };
                    self.sim.seat_named = Some((seat, properties));
                }
            }
            Incoming::ObjectProperties(objects) => {
                for properties in objects {
                    if let Some(local_id) = self.sim.objects.get(&properties.object).copied() {
                        self.sim.asked.remove(&local_id);
                        self.sim.properties.insert(local_id, properties);
                    }
                }
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
    /// Drops what is known of the object with this region-local id.
    fn forget(&mut self, local_id: u32) {
        self.local_ids.remove(&local_id);
        self.parents.remove(&local_id);
        self.places.remove(&local_id);
        self.kinds.remove(&local_id);
        self.properties.remove(&local_id);
        self.asked.remove(&local_id);
    }

    fn new(generation: u64, addr: SocketAddr, circuit: Circuit, now: Instant) -> Sim {
        Sim {
            generation,
            addr,
            circuit,
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
            parents: HashMap::new(),
            places: HashMap::new(),
            kinds: HashMap::new(),
            last_new: None,
            undescribed: HashSet::new(),
            scan_started: None,
            reach: None,
            raised_at: None,
            properties: HashMap::new(),
            asked: HashMap::new(),
            seat_asked: None,
            seat_named: None,
            wanted: Vec::new(),
            sought: Vec::new(),
            attachments: HashMap::new(),
            awaited_attach: Vec::new(),
            awaited_detach: Vec::new(),
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
        Leave(u64),
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
        fn leave(&mut self, generation: u64) {
            self.0.borrow_mut().calls.push(Call::Leave(generation));
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
                inventory_root: None,
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
    fn teleport_by_handle_asks_for_the_region_at_once() {
        let mut rig = Rig::entered();
        let handle = ((1001u64 * 256) << 32) | (1000 * 256);
        rig.session.start_teleport_to(handle, [128.0, 100.0, 25.0], rig.now);
        // No lookup: the handle is the region.
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, messages::TELEPORT_LOCATION_REQUEST);
        assert_eq!(sent[0].body[32..40], handle.to_le_bytes());
        assert_eq!(sent[0].body[40..44], 128.0f32.to_le_bytes());
        assert_eq!(sent[0].body[44..48], 100.0f32.to_le_bytes());
        assert_eq!(sent[0].body[48..52], 25.0f32.to_le_bytes());
        // The avatar is on its way, and a map answer has nothing to say.
        assert!(rig.session.present().is_err());
        rig.receive(messages::MAP_BLOCK_REPLY, &wire::map_block_reply(&[(0, 0, "Target", 255)]));
        assert_eq!(rig.events(), []);
        // It ends as any teleport does: refused by the grid ...
        rig.receive(messages::TELEPORT_FAILED, &wire::teleport_failed("Destination refused"));
        assert_eq!(rig.events(), [SessionEvent::TeleportFailed("Destination refused".into())]);
        assert!(rig.session.present().is_ok());
        // ... or arrived, here in the region the avatar is in already.
        rig.session.start_teleport_to(HOME_HANDLE, [5.0, 6.0, 7.0], rig.now);
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([5.0, 6.0, 7.0]));
        assert_eq!(rig.events(), [SessionEvent::TeleportArrived]);
    }

    #[test]
    fn teleport_by_handle_times_out_after_sixty_seconds() {
        let mut rig = Rig::entered();
        rig.session.start_teleport_to(HOME_HANDLE + 256, [5.0, 6.0, 7.0], rig.now);
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
                Call::Leave(1),
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
        assert_eq!(
            rig.events(),
            [SessionEvent::RegionLeft { worn: vec![] }],
            "arrival is reported once the new handshake is done"
        );

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

        rig.receive_from(1, messages::REGION_HANDSHAKE, &wire::region_handshake("Home"), true);
        rig.receive_from(1, messages::KICK_USER, &wire::kick_user("old"), true);
        rig.receive_from(1, messages::DISABLE_SIMULATOR, &[], true);
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
        assert_eq!(
            rig.events(),
            [
                SessionEvent::RegionLeft { worn: vec![] },
                SessionEvent::RegionEntered("Elsewhere".into())
            ]
        );
    }

    #[test]
    fn new_simulator_that_never_answers_is_a_loss() {
        let mut rig = Rig::entered();
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        rig.advance(secs(30));
        assert!(matches!(
            &rig.events()[..],
            [SessionEvent::RegionLeft { .. }, SessionEvent::Lost(_)]
        ));
    }

    // ------------------------------------------ simulators the avatar left

    fn target_addr() -> SocketAddr {
        "127.0.0.1:9001".parse().unwrap()
    }

    fn home_addr() -> SocketAddr {
        "127.0.0.1:9000".parse().unwrap()
    }

    /// A teleport back to the simulator every `Rig` starts at.
    fn finish_home() -> CapsEvent {
        CapsEvent::Event(EqEvent::TeleportFinish(TeleportFinish {
            sim_addr: home_addr(),
            seed_capability: "http://127.0.0.1:9000/CAPS/seed3/".to_string(),
            region_handle: HOME_HANDLE,
        }))
    }

    impl Rig {
        /// A session whose avatar has teleported from "Home" (circuit 1) to
        /// "Target" (circuit 2) and entered it.
        fn moved() -> Rig {
            let mut rig = Rig::entered();
            rig.session.handle_caps(1, finish(), rig.now);
            rig.enter("Target", ((1001 * 256) << 32) | (1000 * 256));
            rig.events();
            rig.calls();
            rig
        }

        /// Everything sent since the last call, with the circuit it went on.
        fn sent_on(&mut self) -> Vec<(u64, Packet)> {
            std::mem::take(&mut self.net.0.borrow_mut().sent)
        }

        /// A second passes in which the avatar's simulator is heard from.
        fn second(&mut self) {
            let generation = self.generation();
            self.receive_from(generation, packet::high(200), &[], false);
            self.advance(secs(1));
        }

        fn left(&self) -> Vec<u64> {
            self.session.left.iter().map(|left| left.generation).collect()
        }
    }

    #[test]
    fn the_circuit_to_a_simulator_the_avatar_left_is_kept_up() {
        let mut rig = Rig::moved();
        assert_eq!(rig.left(), [1]);
        rig.advance(Duration::from_millis(200));
        rig.sent_on();

        // Its pings are answered on it, numbered after what went before.
        rig.receive_from(1, messages::START_PING_CHECK, &[42, 0, 0, 0, 0], false);
        let sent = rig.sent_on();
        assert_eq!(sent.len(), 1);
        let (generation, pong) = &sent[0];
        assert_eq!(
            (*generation, pong.id, pong.body.as_slice()),
            (1, messages::COMPLETE_PING_CHECK, &[42u8][..])
        );
        assert!(pong.sequence > 4, "the circuit's numbering goes on: {}", pong.sequence);

        // Its reliable packets are acknowledged on it, once, and not read.
        let chat = wire::chat_from_simulator("Rig Owner", (Uuid::from_bytes([7; 16]), Uuid::nil(), 1), 1, 1, "hello");
        rig.receive_from(1, messages::CHAT_FROM_SIMULATOR, &chat, true);
        let heard = rig.sequence;
        rig.receive_from(1, messages::REGION_HANDSHAKE, &wire::region_handshake("Home"), true);
        rig.receive_from(1, messages::OBJECT_UPDATE, &wire::object_update(&[(agent_id(), 99)]), true);
        rig.advance(Duration::from_millis(200));
        let acks: Vec<(u64, Incoming)> = rig
            .sent_on()
            .into_iter()
            .filter(|(_, p)| p.id == messages::PACKET_ACK)
            .map(|(generation, p)| (generation, messages::decode(p.id, &p.body).unwrap()))
            .collect();
        assert_eq!(acks, [(1, Incoming::PacketAck(vec![heard, heard + 1, heard + 2]))]);

        assert_eq!(rig.events(), []);
        assert_eq!(rig.calls(), []);
        assert_eq!(rig.session.region(), Some("Target"));
        assert_eq!(rig.session.seat(), Seat::Unknown);
    }

    #[test]
    fn a_simulator_the_avatar_left_ends_its_circuit_without_ending_the_session() {
        let endings: [(packet::MessageId, Vec<u8>); 3] = [
            (messages::DISABLE_SIMULATOR, vec![]),
            (messages::KICK_USER, wire::kick_user("The simulator is going down.")),
            (messages::LOGOUT_REPLY, vec![0; 33]),
        ];
        for (id, body) in endings {
            let mut rig = Rig::moved();
            rig.receive_from(1, id, &body, true);
            assert_eq!(rig.calls(), [Call::Close(1)], "{}", messages::name(id));
            assert_eq!(rig.left(), [] as [u64; 0]);
            assert_eq!(rig.events(), []);
            assert!(!rig.session.lost);

            // Nothing more is sent there, whatever still comes from it.
            rig.sent_on();
            rig.receive_from(1, messages::START_PING_CHECK, &[1, 0, 0, 0, 0], false);
            rig.advance(secs(1));
            assert!(rig.sent_on().iter().all(|(generation, _)| *generation == 2));
        }
    }

    #[test]
    fn a_silent_simulator_the_avatar_left_is_given_up_after_sixty_seconds() {
        let mut rig = Rig::moved();
        for _ in 0..59 {
            rig.second();
        }
        assert_eq!(rig.calls(), []);
        rig.second();
        assert_eq!(rig.calls(), [Call::Close(1)]);
        assert_eq!(rig.left(), [] as [u64; 0]);
        for _ in 0..70 {
            rig.second();
        }
        assert_eq!(rig.events(), []);
        assert!(!rig.session.lost);
    }

    #[test]
    fn a_simulator_the_avatar_left_that_stops_acknowledging_is_given_up() {
        let mut rig = Rig::entered();
        // The teleport request is still unacknowledged when the avatar goes.
        rig.session
            .start_teleport_to(((1001 * 256) << 32) | (1000 * 256), [128.0, 128.0, 25.0], rig.now);
        rig.session.handle_caps(1, finish(), rig.now);
        rig.enter("Target", ((1001 * 256) << 32) | (1000 * 256));
        rig.events();
        rig.calls();
        rig.sent_on();

        // The simulator left behind keeps pinging but never acknowledges.
        let mut resends = 0;
        for second in 1..=31 {
            rig.receive_from(1, messages::START_PING_CHECK, &[second, 0, 0, 0, 0], false);
            rig.second();
            resends += rig
                .sent_on()
                .iter()
                .filter(|(generation, p)| *generation == 1 && p.id == messages::TELEPORT_LOCATION_REQUEST)
                .count();
            if second < 31 {
                assert_eq!(rig.calls(), [], "after {second} s");
            }
        }
        assert_eq!(resends, 4);
        assert_eq!(rig.calls(), [Call::Close(1)]);
        assert_eq!(rig.events(), []);
        assert!(!rig.session.lost);
    }

    #[test]
    fn a_lost_session_closes_the_circuits_the_avatar_left() {
        let mut rig = Rig::moved();
        rig.receive(messages::KICK_USER, &wire::kick_user("bye"));
        assert_eq!(rig.lost_reason(), "kicked: bye");
        assert_eq!(rig.calls(), [Call::Close(2), Call::Close(1)]);
        assert_eq!(rig.left(), [] as [u64; 0]);
    }

    #[test]
    fn a_teleport_back_continues_the_circuit_the_avatar_left() {
        let mut rig = Rig::moved();
        // What the bot knows of "Target" is not carried back to "Home".
        rig.own_update(4242);
        rig.events();
        rig.sent_on();
        rig.receive_from(1, messages::START_PING_CHECK, &[1, 0, 0, 0, 0], false);
        let last = rig.sent_on()[0].1.sequence;

        rig.session.handle_caps(2, finish_home(), rig.now);
        assert_eq!(
            rig.calls(),
            [
                Call::Leave(2),
                Call::Open(3, home_addr(), "http://127.0.0.1:9000/CAPS/seed3/".into()),
            ]
        );
        let handshake: Vec<_> = rig
            .sent_on()
            .iter()
            .map(|(generation, p)| (*generation, p.id, p.sequence, p.is_reliable()))
            .collect();
        assert_eq!(
            handshake,
            [
                (3, messages::USE_CIRCUIT_CODE, last + 1, true),
                (3, messages::COMPLETE_AGENT_MOVEMENT, last + 2, true)
            ]
        );
        assert_eq!(rig.left(), [2], "the simulator just left, not the one returned to");
        assert_eq!(rig.events(), [SessionEvent::RegionLeft { worn: vec![] }]);
        assert_eq!(rig.session.region(), None);
        assert!(rig.session.sim.objects.is_empty());
        assert_eq!(rig.session.seat(), Seat::Unknown);

        // What the main loop still holds under the circuit's old number is
        // dropped: not answered, not acknowledged, not read.
        rig.receive_from(1, messages::START_PING_CHECK, &[2, 0, 0, 0, 0], false);
        rig.receive_from(1, messages::REGION_HANDSHAKE, &wire::region_handshake("Stale"), true);
        let stale = rig.sequence;
        rig.advance(Duration::from_millis(200));
        for (_, packet) in rig.sent_on() {
            match messages::decode(packet.id, &packet.body) {
                Ok(Incoming::PacketAck(acked)) => assert!(!acked.contains(&stale)),
                _ => assert_ne!(packet.id, messages::COMPLETE_PING_CHECK),
            }
        }
        assert_eq!(rig.session.region(), None);

        // The handshake completes as on any arrival, and "Target" is now
        // the simulator whose pings are answered on the side.
        rig.enter("Home", HOME_HANDLE);
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Home".into())]);
        rig.sent_on();
        rig.receive_from(2, messages::START_PING_CHECK, &[3, 0, 0, 0, 0], false);
        let sent = rig.sent_on();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].0, sent[0].1.id), (2, messages::COMPLETE_PING_CHECK));
    }

    #[test]
    fn a_teleport_back_to_a_simulator_that_closed_its_circuit_starts_a_new_one() {
        let mut rig = Rig::moved();
        rig.receive_from(1, messages::DISABLE_SIMULATOR, &[], true);
        rig.calls();
        rig.sent_on();

        rig.session.handle_caps(2, finish_home(), rig.now);
        assert_eq!(
            rig.calls(),
            [
                Call::Leave(2),
                Call::Open(3, home_addr(), "http://127.0.0.1:9000/CAPS/seed3/".into()),
            ]
        );
        let handshake: Vec<_> = rig
            .sent_on()
            .iter()
            .map(|(generation, p)| (*generation, p.id, p.sequence))
            .collect();
        assert_eq!(
            handshake,
            [(3, messages::USE_CIRCUIT_CODE, 1), (3, messages::COMPLETE_AGENT_MOVEMENT, 2)]
        );
    }

    #[test]
    fn a_simulator_returned_to_gets_the_handshake_time_of_any_other() {
        let mut rig = Rig::moved();
        // "Home" was last heard from 45 seconds ago when the avatar returns.
        for _ in 0..45 {
            rig.second();
        }
        rig.session.handle_caps(2, finish_home(), rig.now);
        rig.events();
        // "Target", now left, keeps pinging; "Home" says nothing at all.
        for second in 1..=29 {
            rig.receive_from(2, messages::START_PING_CHECK, &[second, 0, 0, 0, 0], false);
            rig.advance(secs(1));
            assert_eq!(rig.events(), [], "after {second} s");
        }
        rig.advance(secs(1));
        assert_eq!(rig.lost_reason(), "simulator handshake timed out");
    }

    #[test]
    fn a_teleport_to_the_simulator_the_avatar_is_at_keeps_its_numbering() {
        let mut rig = Rig::entered();
        rig.calls();
        rig.sent_on();
        rig.receive_from(1, messages::START_PING_CHECK, &[1, 0, 0, 0, 0], false);
        let last = rig.sent_on()[0].1.sequence;

        rig.session.handle_caps(1, finish_home(), rig.now);
        assert_eq!(
            rig.calls(),
            [
                Call::Leave(1),
                Call::Open(2, home_addr(), "http://127.0.0.1:9000/CAPS/seed3/".into()),
            ]
        );
        let handshake: Vec<_> = rig
            .sent_on()
            .iter()
            .map(|(generation, p)| (*generation, p.id, p.sequence))
            .collect();
        assert_eq!(
            handshake,
            [
                (2, messages::USE_CIRCUIT_CODE, last + 1),
                (2, messages::COMPLETE_AGENT_MOVEMENT, last + 2)
            ]
        );
        assert_eq!(rig.left(), [] as [u64; 0]);
        assert_ne!(home_addr(), target_addr());
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
        // Group notice, session message, auto response, typing, and the
        // answers to offers the bot does not make.
        for dialog in [5, 6, 32, 17, 20, 41, 42] {
            rig.im(Uuid::from_bytes([0x77; 16]), dialog, Uuid::nil(), None, "x");
        }
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn inventory_offers_are_reported_and_not_recorded_as_messages() {
        let mut rig = Rig::entered();
        let (giver, transaction, item) = (Uuid::from_bytes([0x77; 16]), Uuid::from_bytes([0x66; 16]), [0x55; 16]);
        let offer = |rig: &mut Rig, group, dialog, text: &str, bucket: &[u8]| {
            let body = wire::instant_message_with_bucket(giver, group, "Sender", dialog, transaction, None, text, bucket);
            rig.receive(messages::IMPROVED_INSTANT_MESSAGE, &body);
            rig.events()
        };
        let offered = |from_group, from_object, asset_type, text: &str| {
            SessionEvent::InventoryOffer(InventoryOffer {
                from: giver,
                name: "Sender".into(),
                from_group,
                from_object,
                transaction,
                asset_type,
                text: text.into(),
            })
        };
        // An avatar gives a notecard.
        let bucket = [vec![7], item.to_vec()].concat();
        assert_eq!(offer(&mut rig, false, 4, "Rules", &bucket), [offered(false, false, 7, "Rules")]);
        // An object gives one, and a group's object a folder.
        assert_eq!(offer(&mut rig, false, 9, "'Rules'", &[7]), [offered(false, true, 7, "'Rules'")]);
        assert_eq!(offer(&mut rig, true, 9, "'Kit'", &[8]), [offered(true, true, 8, "'Kit'")]);
        // The older form of an object's offer.
        assert_eq!(offer(&mut rig, false, 9, "'Rules'", b"7|x\0"), [offered(false, true, 7, "'Rules'")]);
        // Malformed offers are dropped.
        assert_eq!(offer(&mut rig, false, 4, "Rules", &[7]), []);
        assert_eq!(offer(&mut rig, false, 9, "'Rules'", b"x|y"), []);
        assert_eq!(offer(&mut rig, false, 9, "'Rules'", &[]), []);
    }

    #[test]
    fn script_permission_requests_and_object_owners_are_reported() {
        let mut rig = Rig::entered();
        let (object, item, owner, group) = (
            Uuid::from_bytes([0x91; 16]),
            Uuid::from_bytes([0x92; 16]),
            Uuid::from_bytes([0x93; 16]),
            Uuid::from_bytes([0x94; 16]),
        );
        let body = wire::script_question(object, item, "Dance Ball", "Jane Doe", 0x14, true);
        rig.receive(messages::SCRIPT_QUESTION, &body);
        assert_eq!(
            rig.events(),
            [SessionEvent::PermissionRequest {
                object,
                item,
                object_name: "Dance Ball".into(),
                owner_name: "Jane Doe".into(),
                permissions: 0x14,
            }]
        );
        rig.receive(
            messages::OBJECT_PROPERTIES_FAMILY,
            &wire::object_properties_family(object, owner, group),
        );
        assert_eq!(rig.events(), [SessionEvent::ObjectOwner { object, owner, group }]);
    }

    #[test]
    fn requests_are_answered_with_their_own_messages() {
        let mut rig = Rig::entered();
        rig.sent();
        let now = rig.now;
        let (object, item, folder) = (
            Uuid::from_bytes([0x91; 16]),
            Uuid::from_bytes([0x92; 16]),
            Uuid::from_bytes([0x95; 16]),
        );
        assert_eq!(rig.session.request_object_owner(object, now), Ok(()));
        assert_eq!(rig.session.grant_permissions(object, item, 0x14, now), Ok(()));
        assert_eq!(rig.session.accept_inventory(object, false, item, folder, now), Ok(()));
        assert_eq!(rig.session.accept_inventory(object, true, item, folder, now), Ok(()));
        assert_eq!(
            rig.sent_ids(),
            [
                messages::REQUEST_OBJECT_PROPERTIES_FAMILY,
                messages::SCRIPT_ANSWER_YES,
                messages::IMPROVED_INSTANT_MESSAGE,
                messages::IMPROVED_INSTANT_MESSAGE
            ]
        );
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
    fn sends_automatic_answers() {
        let mut rig = Rig::entered();
        let to = Uuid::from_bytes([0x0F; 16]);
        assert_eq!(rig.session.autoresponse(to, "Away", rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].is_reliable());
        let expected = messages::instant_message(&rig.agent(), "Sit Bot", to, [10.0, 20.0, 30.0], 20, "Away");
        assert_eq!((sent[0].id, &sent[0].body), (messages::IMPROVED_INSTANT_MESSAGE, &expected.body));
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
            assert_eq!(rig.session.autoresponse(to, "Away", now), Err(NOT_CONNECTED.to_string()));
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
        assert_eq!(
            rig.events(),
            [
                SessionEvent::RegionLeft { worn: vec![] },
                SessionEvent::RegionEntered("Elsewhere".into())
            ]
        );
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
                worn: None,
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
                worn: None,
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
            worn: None,
        };
        rig.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        assert_eq!(rig.position(), [100.0, 100.0, 25.0]);
        let data = wire::object_data([f32::NAN, 1.0, 2.0], [0.0; 3], true);
        let block = wire::Block {
            full_id: agent_id(),
            local_id: OWN,
            parent_id: 0,
            data,
            worn: None,
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
        // Nothing to ask about yet: the scan starts with its wider
        // throttle alone.
        assert_eq!(rig.sent_ids(), [messages::AGENT_THROTTLE]);
        rig.ack_all();
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
        assert_eq!(
            rig.sent_ids(),
            [messages::AGENT_THROTTLE],
            "nothing to press, and nothing left to ask about"
        );
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
        assert_eq!(
            rig.events(),
            [SessionEvent::TouchFailed(object(9)), SessionEvent::RegionLeft { worn: vec![] }]
        );
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

    // -------------------------------------------------------- attachments

    /// The avatar's own region-local id after `Rig::own_update`.
    const OWN_LOCAL: u32 = 101;
    const RADIO_LOCAL: u32 = 700;

    fn radio_item() -> Uuid {
        Uuid::from_bytes([0x17; 16])
    }

    fn radio_object() -> Uuid {
        Uuid::from_bytes([0x18; 16])
    }

    fn radio() -> AttachItem {
        AttachItem {
            item_id: radio_item(),
            owner_id: agent_id(),
            flags: 0x105,
            group_mask: 0,
            everyone_mask: 0x8000,
            next_owner_mask: 0x0008_2000,
            name: "Radio".into(),
            description: "plays".into(),
        }
    }

    impl Rig {
        /// A full update describing an attachment's root prim.
        fn attachment(&mut self, object: Uuid, local_id: u32, parent: u32, point: u8, item: Uuid) {
            let block = wire::attachment_block(object, local_id, parent, point, item);
            self.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        }

        /// The same in the compressed form.
        fn attachment_compressed(&mut self, object: Uuid, local_id: u32, parent: u32, point: u8, item: Uuid) {
            let data = wire::compressed_data(object, local_id, &wire::Compressed::attachment(parent, point, item));
            let body = wire::object_update_compressed_blocks(&[data]);
            self.receive(messages::OBJECT_UPDATE_COMPRESSED, &body);
        }

        /// In a region, standing, with the avatar's own update seen and
        /// nothing left to report or acknowledge.
        fn standing() -> Rig {
            let mut rig = Rig::entered();
            rig.own_update(0);
            assert_eq!(rig.events(), [SessionEvent::Standing]);
            rig.ack_all();
            rig.sent();
            rig
        }

        fn attached(&self) -> Vec<(Uuid, u8)> {
            let mut attached = self.session.attached();
            attached.sort();
            attached
        }
    }

    #[test]
    fn an_attachment_on_the_avatar_is_known_from_its_description() {
        let mut rig = Rig::standing();
        assert_eq!(rig.attached(), []);
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        assert_eq!(
            rig.events(),
            [SessionEvent::Attached {
                item: radio_item(),
                point: 5
            }]
        );
        assert_eq!(rig.attached(), [(radio_item(), 5)]);
        // Described again, it is nothing new.
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        assert_eq!(rig.events(), []);
        // A second one, on a HUD point, in the compressed form.
        let (meter, meter_item) = (Uuid::from_bytes([0x28; 16]), Uuid::from_bytes([0x27; 16]));
        rig.attachment_compressed(meter, 701, OWN_LOCAL, 35, meter_item);
        assert_eq!(
            rig.events(),
            [SessionEvent::Attached {
                item: meter_item,
                point: 35
            }]
        );
        assert_eq!(rig.attached(), [(radio_item(), 5), (meter_item, 35)]);
        // None of this made the bot send anything.
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn attachments_described_before_the_avatar_count_once_it_is_known() {
        let mut rig = Rig::entered();
        let (hat, hat_item) = (Uuid::from_bytes([0x38; 16]), Uuid::from_bytes([0x37; 16]));
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.attachment_compressed(hat, 701, OWN_LOCAL, 2, hat_item);
        // Whose they are is not known yet.
        assert_eq!(rig.events(), []);
        assert_eq!(rig.attached(), []);
        rig.own_update(0);
        let mut events = rig.events();
        events.sort_by_key(|event| format!("{event:?}"));
        assert_eq!(
            events,
            [
                SessionEvent::Attached {
                    item: radio_item(),
                    point: 5
                },
                SessionEvent::Attached { item: hat_item, point: 2 },
                SessionEvent::Standing,
            ]
        );
        assert_eq!(rig.attached(), [(radio_item(), 5), (hat_item, 2)]);
        // The avatar described again changes nothing.
        rig.own_update(0);
        assert_eq!(rig.events(), []);
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn other_avatars_attachments_and_plain_objects_are_not_the_bots() {
        let mut rig = Rig::standing();
        // On another avatar; and an object with no item in its name-values.
        rig.attachment(radio_object(), RADIO_LOCAL, 4321, 5, radio_item());
        rig.receive(messages::OBJECT_UPDATE, &wire::object_update(&[(object(3), OWN_LOCAL)]));
        let plain = wire::Compressed {
            parent_id: Some(OWN_LOCAL),
            ..wire::Compressed::default()
        };
        let body = wire::object_update_compressed_blocks(&[wire::compressed_data(object(4), 702, &plain)]);
        rig.receive(messages::OBJECT_UPDATE_COMPRESSED, &body);
        assert_eq!(rig.events(), []);
        assert_eq!(rig.attached(), []);
        // Their going is the going of two objects, and no detach.
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[RADIO_LOCAL, 702]));
        let gone = rig.events();
        assert_eq!(gone.len(), 2);
        assert!(gone.iter().all(|event| matches!(event, SessionEvent::ObjectGone(_))), "{gone:?}");
    }

    #[test]
    fn an_attachment_on_the_avatar_names_its_inventory_item() {
        let mut rig = Rig::standing();
        // The avatar's own, another avatar's, and an object that is not worn.
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        let (hat, hat_item) = (Uuid::from_bytes([0x38; 16]), Uuid::from_bytes([0x37; 16]));
        rig.attachment(hat, 701, 4321, 2, hat_item);
        rig.receive(messages::OBJECT_UPDATE, &wire::object_update(&[(object(3), 702)]));
        assert_eq!(rig.session.attached_item(radio_object()), Some(radio_item()));
        assert_eq!(rig.session.attached_item(hat), None);
        assert_eq!(rig.session.attached_item(object(3)), None);
        assert_eq!(rig.session.attached_item(Uuid::from_bytes([0x99; 16])), None);
        // By its object, not by its item.
        assert_eq!(rig.session.attached_item(radio_item()), None);
        // Gone from the avatar, it names nothing.
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[RADIO_LOCAL]));
        assert_eq!(rig.session.attached_item(radio_object()), None);
    }

    #[test]
    fn a_killed_attachment_is_gone_from_the_avatar() {
        let mut rig = Rig::standing();
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.events();
        // Some other object going changes nothing.
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[RADIO_LOCAL + 1]));
        assert_eq!(rig.events(), []);
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[RADIO_LOCAL]));
        assert_eq!(
            rig.events(),
            [
                SessionEvent::Detached {
                    item: radio_item(),
                    point: 5
                },
                SessionEvent::ObjectGone(radio_object())
            ]
        );
        assert_eq!(rig.attached(), []);
        // Attached again, by a script or a viewer, it is known again.
        rig.attachment(radio_object(), RADIO_LOCAL + 9, OWN_LOCAL, 6, radio_item());
        assert_eq!(
            rig.events(),
            [SessionEvent::Attached {
                item: radio_item(),
                point: 6
            }]
        );
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn an_attachment_shown_on_another_point_has_moved() {
        let mut rig = Rig::standing();
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.events();
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 1, radio_item());
        assert_eq!(
            rig.events(),
            [
                SessionEvent::Detached {
                    item: radio_item(),
                    point: 5
                },
                SessionEvent::Attached {
                    item: radio_item(),
                    point: 1
                },
            ]
        );
        assert_eq!(rig.attached(), [(radio_item(), 1)]);
    }

    #[test]
    fn attachments_are_learned_anew_from_the_next_simulator() {
        let mut rig = Rig::standing();
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.events();
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        // Forgotten with the circuit, and nothing was detached: it is
        // named as what came along.
        assert_eq!(rig.attached(), []);
        assert_eq!(
            rig.events(),
            [SessionEvent::RegionLeft {
                worn: vec![radio_object()]
            }]
        );
        rig.enter("Target", ((1001 * 256) << 32) | (1000 * 256));
        assert_eq!(rig.events(), [SessionEvent::RegionEntered("Target".into())]);
        // The new simulator numbers things its own way.
        rig.attachment(radio_object(), 55, OWN_LOCAL, 5, radio_item());
        assert_eq!(rig.attached(), []);
        rig.own_update(0);
        let attached = SessionEvent::Attached {
            item: radio_item(),
            point: 5,
        };
        assert!(rig.events().contains(&attached));
        assert_eq!(rig.attached(), [(radio_item(), 5)]);
        // A kill for the old simulator's number is not about it.
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[RADIO_LOCAL]));
        assert_eq!(rig.events(), []);
        assert_eq!(rig.sent_ids(), []);
    }

    #[test]
    fn attach_asks_the_simulator_and_waits_for_the_object() {
        let mut rig = Rig::standing();
        assert_eq!(rig.session.attach(&radio(), 5, rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, messages::REZ_SINGLE_ATTACHMENT_FROM_INV);
        assert!(sent[0].is_reliable());
        // The item's flags, masks, name and description, and the add bit.
        assert_eq!(
            sent[0].body,
            messages::rez_single_attachment_from_inv(&rig.agent(), &radio(), 5).body
        );
        assert_eq!(sent[0].body[64], 0x85);
        assert_eq!(rig.session.next_deadline().min(rig.now + ATTACH_LIMIT), rig.session.next_deadline());
        rig.ack_all();
        // The object turning up on the avatar is the answer.
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        assert_eq!(
            rig.events(),
            [SessionEvent::Attached {
                item: radio_item(),
                point: 5
            }]
        );
        rig.advance(ATTACH_LIMIT);
        assert_eq!(rig.events(), []);

        // Without a body part the point is the simulator's to choose.
        let hat = AttachItem {
            item_id: Uuid::from_bytes([0x37; 16]),
            ..radio()
        };
        assert_eq!(rig.session.attach(&hat, 0, rig.now), Ok(()));
        assert_eq!(rig.sent()[0].body[64], 0x80);
    }

    #[test]
    fn an_attach_that_shows_nothing_fails_after_thirty_seconds() {
        let mut rig = Rig::standing();
        rig.session.attach(&radio(), 0, rig.now).unwrap();
        rig.ack_all();
        // Somebody else's attachment of the same item is not it.
        rig.attachment(radio_object(), RADIO_LOCAL, 4321, 5, radio_item());
        rig.advance(ATTACH_LIMIT - secs(1));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        assert_eq!(rig.events(), [SessionEvent::AttachFailed(radio_item())]);
        // Once only, and the avatar stays connected.
        rig.advance(secs(5));
        assert_eq!(rig.events(), []);
        assert_eq!(rig.session.present(), Ok(()));
    }

    #[test]
    fn a_slow_attach_is_waited_for_longer_than_a_detach() {
        // Second Life has taken 21 seconds over an attach.
        let mut rig = Rig::standing();
        rig.session.attach(&radio(), 0, rig.now).unwrap();
        rig.ack_all();
        rig.advance(secs(21));
        assert_eq!(rig.events(), []);
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        assert_eq!(
            rig.events(),
            [SessionEvent::Attached {
                item: radio_item(),
                point: 5
            }]
        );
        rig.advance(ATTACH_LIMIT);
        assert_eq!(rig.events(), []);
        // A detach is given up on after fifteen.
        rig.session.detach(radio_item(), rig.now).unwrap();
        rig.ack_all();
        rig.advance(secs(15));
        assert_eq!(rig.events(), [SessionEvent::DetachFailed(radio_item())]);
        assert_eq!((ATTACH_LIMIT, DETACH_LIMIT), (secs(30), secs(15)));
    }

    #[test]
    fn detach_asks_the_simulator_and_waits_for_the_object_to_go() {
        let mut rig = Rig::standing();
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.events();
        assert_eq!(rig.session.detach(radio_item(), rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, messages::DETACH_ATTACHMENT_INTO_INV);
        assert!(sent[0].is_reliable());
        assert_eq!(
            sent[0].body,
            [agent_id().as_bytes().to_vec(), radio_item().as_bytes().to_vec()].concat()
        );
        rig.ack_all();
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[RADIO_LOCAL]));
        assert_eq!(
            rig.events(),
            [
                SessionEvent::Detached {
                    item: radio_item(),
                    point: 5
                },
                SessionEvent::ObjectGone(radio_object())
            ]
        );
        rig.advance(DETACH_LIMIT);
        assert_eq!(rig.events(), []);

        // One the simulator leaves on.
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.events();
        rig.session.detach(radio_item(), rig.now).unwrap();
        rig.ack_all();
        rig.advance(DETACH_LIMIT - secs(1));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        assert_eq!(rig.events(), [SessionEvent::DetachFailed(radio_item())]);
        assert_eq!(rig.attached(), [(radio_item(), 5)]);
    }

    #[test]
    fn what_was_awaited_fails_when_the_avatar_changes_simulator_or_is_lost() {
        let mut rig = Rig::standing();
        let hat = Uuid::from_bytes([0x37; 16]);
        rig.session.attach(&radio(), 0, rig.now).unwrap();
        rig.session.detach(hat, rig.now).unwrap();
        rig.ack_all();
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        assert_eq!(
            rig.events(),
            [
                SessionEvent::AttachFailed(radio_item()),
                SessionEvent::DetachFailed(hat),
                SessionEvent::RegionLeft { worn: vec![] }
            ]
        );
        let mut rig = Rig::standing();
        rig.session.attach(&radio(), 0, rig.now).unwrap();
        rig.receive(messages::KICK_USER, &wire::kick_user("Bye"));
        let events = rig.events();
        assert_eq!(events[0], SessionEvent::AttachFailed(radio_item()));
        assert!(matches!(events[1], SessionEvent::Lost(_)));
    }

    #[test]
    fn attachments_animations_and_sounds_are_refused_outside_a_region() {
        let hat = Uuid::from_bytes([0x37; 16]);
        let refused = |rig: &mut Rig| {
            let now = rig.now;
            assert_eq!(rig.session.attach(&radio(), 5, now), Err(NOT_CONNECTED.to_string()));
            assert_eq!(rig.session.detach(hat, now), Err(NOT_CONNECTED.to_string()));
            assert_eq!(rig.session.animate(hat, true, now), Err(NOT_CONNECTED.to_string()));
            assert_eq!(rig.session.animate(hat, false, now), Err(NOT_CONNECTED.to_string()));
            assert_eq!(rig.session.play_sound(hat, now), Err(NOT_CONNECTED.to_string()));
        };
        // Before the handshake is done.
        let mut rig = Rig::new();
        rig.sent();
        refused(&mut rig);
        assert_eq!(rig.sent_ids(), []);
        // In the middle of a teleport.
        let mut rig = Rig::standing();
        start_teleport(&mut rig);
        rig.sent();
        refused(&mut rig);
        // While logging out.
        let mut rig = Rig::standing();
        rig.session.start_logout(rig.now);
        rig.sent();
        refused(&mut rig);
        assert_eq!(rig.sent_ids(), []);
        // Nothing is kept for later, and nothing is waited for.
        rig.advance(ATTACH_LIMIT);
        assert!(!rig
            .events()
            .iter()
            .any(|event| matches!(event, SessionEvent::AttachFailed(_) | SessionEvent::DetachFailed(_))));
    }

    // ------------------------------------------------ animations and sounds

    #[test]
    fn animate_starts_and_stops_an_animation() {
        let mut rig = Rig::standing();
        let dance = Uuid::from_bytes([0x2A; 16]);
        assert_eq!(rig.session.animate(dance, true, rig.now), Ok(()));
        assert_eq!(rig.session.animate(dance, false, rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent.iter().map(|p| p.id).collect::<Vec<_>>(), [messages::AGENT_ANIMATION; 2]);
        assert!(sent.iter().all(Packet::is_reliable));
        assert_eq!(sent[0].body, messages::agent_animation(&rig.agent(), dance, true).body);
        assert_eq!(sent[1].body, messages::agent_animation(&rig.agent(), dance, false).body);
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn play_sound_triggers_it_where_the_avatar_is() {
        let mut rig = Rig::standing();
        let bell = Uuid::from_bytes([0x50; 16]);
        assert_eq!(rig.session.play_sound(bell, rig.now), Ok(()));
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, messages::SOUND_TRIGGER);
        // The region's handle, the avatar's position, full volume, and
        // null owner and object ids.
        let expected = messages::sound_trigger(bell, HOME_HANDLE, [10.0, 20.0, 30.0], 1.0);
        assert_eq!(sent[0].body, expected.body);
        assert_eq!(sent[0].body[16..64], [0; 48]);
        assert_eq!(sent[0].body[sent[0].body.len() - 4..], 1.0f32.to_le_bytes());
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn inventory_services_are_passed_on_for_the_current_circuit_only() {
        let mut rig = Rig::entered();
        let urls = InventoryUrls {
            descendents: Some("http://127.0.0.1:9000/CAPS/fid/".to_string()),
            items: Some("http://127.0.0.1:9000/CAPS/fi/".to_string()),
            api: None,
            appearance: None,
        };
        rig.session.handle_caps(1, CapsEvent::Inventory(urls.clone()), rig.now);
        assert_eq!(rig.events(), [SessionEvent::InventoryUrls(urls.clone())]);
        rig.session.handle_caps(7, CapsEvent::Inventory(urls), rig.now);
        assert_eq!(rig.events(), []);
        assert_eq!(rig.sent_ids(), []);
    }

    // --------------------------------------------- where described objects are

    impl Rig {
        /// A full update describing a prim: in the region when `parent` is
        /// 0, and relative to its parent otherwise.
        fn prim(&mut self, full_id: Uuid, local_id: u32, parent: u32, position: Vector3) {
            let block = wire::Block {
                full_id,
                local_id,
                parent_id: parent,
                data: wire::object_data(position, [0.0; 3], false),
                worn: None,
            };
            self.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&[block]));
        }
    }

    fn sorted(mut prims: Vec<Uuid>) -> Vec<Uuid> {
        prims.sort();
        prims
    }

    #[test]
    fn a_described_object_is_placed_by_what_it_hangs_on() {
        // The avatar stands at 10, 20, 30.
        let mut rig = Rig::standing();
        let (door, knob) = (object(0x30), object(0x31));
        rig.prim(door, 800, 0, [13.0, 24.0, 30.0]);
        rig.prim(knob, 801, 800, [0.5, 0.0, 0.0]);
        let thing = rig.session.thing(knob);
        assert_eq!(thing.placement, Placement::InWorld);
        // The touched prim first, then the rest of its linkset.
        assert_eq!(thing.prims, [knob, door]);
        // A child prim is where its root is.
        assert_eq!(thing.distance, Some(5.0));
        assert_eq!(rig.session.thing(door).prims, [door, knob]);
        assert_eq!(rig.session.attached_item(knob), None);

        // The avatar's own attachment, and a prim of it.
        let dial = object(0x32);
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.prim(dial, 705, RADIO_LOCAL, [0.1, 0.0, 0.0]);
        let thing = rig.session.thing(dial);
        assert_eq!((thing.placement, thing.distance), (Placement::Worn, Some(0.0)));
        assert_eq!(thing.prims, [dial, radio_object()]);
        assert_eq!(rig.session.attached_item(dial), Some(radio_item()));
        assert_eq!(rig.session.attached_item(radio_object()), Some(radio_item()));
        assert_eq!(rig.session.attachment_prims(radio_item()), sorted(vec![radio_object(), dial]));

        // One on the heads-up display, described in the compressed form.
        let (meter, meter_item) = (object(0x28), object(0x27));
        rig.attachment_compressed(meter, 701, OWN_LOCAL, 35, meter_item);
        assert_eq!(rig.session.thing(meter).placement, Placement::Hud);

        // Another avatar's attachment is where that avatar is.
        let (jane, hat, hat_item) = (object(0x40), object(0x41), object(0x42));
        rig.prim(jane, 300, 0, [20.0, 20.0, 30.0]);
        rig.attachment(hat, 310, 300, 2, hat_item);
        let thing = rig.session.thing(hat);
        assert_eq!((thing.placement, thing.distance), (Placement::WornBy(jane), Some(10.0)));
        assert_eq!(rig.session.attached_item(hat), None);
        assert!(rig.session.attachment_prims(hat_item).is_empty());

        // What the simulator has not described is unknown.
        assert_eq!(rig.session.thing(object(0x55)), Thing::unknown(object(0x55)));
        // One described without a place is known, but not how far it is.
        let far = object(0x56);
        rig.receive(messages::OBJECT_UPDATE_COMPRESSED, &wire::object_update_compressed(&[(far, 900)]));
        let thing = rig.session.thing(far);
        assert_eq!((thing.placement, thing.distance), (Placement::InWorld, None));
    }

    #[test]
    fn a_described_object_is_followed_when_it_moves_and_gone_when_killed() {
        let mut rig = Rig::standing();
        let (door, knob) = (object(0x30), object(0x31));
        rig.prim(door, 800, 0, [13.0, 24.0, 30.0]);
        rig.prim(knob, 801, 800, [0.5, 0.0, 0.0]);
        rig.events();
        // A terse update moves it; one about an object not described is nothing.
        rig.terse(800, false, [10.0, 20.0, 33.0]);
        rig.terse(4000, false, [1.0, 1.0, 1.0]);
        assert_eq!(rig.session.thing(knob).distance, Some(3.0));
        // The avatar moving changes the distance as well.
        rig.receive(messages::TELEPORT_LOCAL, &wire::teleport_local([10.0, 20.0, 37.0]));
        assert_eq!(rig.session.thing(door).distance, Some(4.0));
        assert_eq!(rig.events(), [], "a teleport inside the region leaves nothing behind");
        // Killed, each is reported gone, and what is left stands alone.
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[801, 4000]));
        assert_eq!(rig.events(), [SessionEvent::ObjectGone(knob)]);
        assert_eq!(rig.session.thing(knob), Thing::unknown(knob));
        assert_eq!(rig.session.thing(door).prims, [door]);
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[800]));
        assert_eq!(rig.events(), [SessionEvent::ObjectGone(door)]);
    }

    #[test]
    fn leaving_a_region_names_what_is_on_the_avatar() {
        let mut rig = Rig::standing();
        let (door, dial) = (object(0x30), object(0x32));
        let (jane, hat) = (object(0x40), object(0x41));
        rig.prim(door, 800, 0, [13.0, 24.0, 30.0]);
        rig.attachment(radio_object(), RADIO_LOCAL, OWN_LOCAL, 5, radio_item());
        rig.prim(dial, 705, RADIO_LOCAL, [0.1, 0.0, 0.0]);
        rig.prim(jane, 300, 0, [20.0, 20.0, 30.0]);
        rig.attachment(hat, 310, 300, 2, object(0x42));
        rig.events();
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        let events = rig.events();
        let [SessionEvent::RegionLeft { worn }] = &events[..] else {
            panic!("expected the region to be left, got {events:?}");
        };
        // The attachment and its prim, not the door and not another's hat.
        assert_eq!(sorted(worn.clone()), sorted(vec![radio_object(), dial]));
        // Nothing of the old region is known in the new one.
        rig.enter("Target", ((1001 * 256) << 32) | (1000 * 256));
        assert_eq!(rig.session.thing(door), Thing::unknown(door));
    }

    // ------------------------------------------------- looking for objects

    #[test]
    fn an_object_looked_for_is_asked_about_and_reported_with_its_place() {
        let mut rig = Rig::standing();
        let (door, knob) = (object(0x30), object(0x31));
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[800, 801]));
        assert_eq!(requests(&rig.sent()), Vec::<Vec<u8>>::new(), "no scan before it is needed");

        assert_eq!(rig.session.look_for(knob, rig.now), Ok(Look::Waiting));
        let asked = requests(&rig.sent());
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0][32], 2, "both mentioned objects are asked about");
        assert!(rig.session.next_deadline() <= rig.now + TOUCH_WAIT);
        assert_eq!(rig.events(), []);

        // The child comes first in the update; the event comes after the
        // root, so that the place is known when it is read.
        let blocks = [
            wire::Block {
                full_id: knob,
                local_id: 801,
                parent_id: 800,
                data: wire::object_data([0.5, 0.0, 0.0], [0.0; 3], false),
                worn: None,
            },
            wire::Block {
                full_id: door,
                local_id: 800,
                parent_id: 0,
                data: wire::object_data([13.0, 24.0, 30.0], [0.0; 3], false),
                worn: None,
            },
        ];
        rig.receive(messages::OBJECT_UPDATE, &wire::object_blocks(&blocks));
        assert_eq!(rig.events(), [SessionEvent::Described(knob)]);
        let thing = rig.session.thing(knob);
        assert_eq!((thing.placement, thing.distance), (Placement::InWorld, Some(5.0)));
        assert_eq!(rig.sent_ids(), [], "looking touches nothing");

        // Known now, and not waited for again.
        assert_eq!(rig.session.look_for(knob, rig.now), Ok(Look::Known));
        rig.advance(secs(11));
        assert_eq!(rig.events(), []);
    }

    #[test]
    fn an_object_never_described_is_given_up_on_with_the_scan() {
        let mut rig = Rig::standing();
        let chair = object(0x30);
        assert_eq!(rig.session.look_for(chair, rig.now), Ok(Look::Waiting));
        rig.advance(secs(4));
        // A second look, and a touch, share the scan's deadline.
        assert_eq!(rig.session.look_for(chair, rig.now), Ok(Look::Waiting));
        assert_eq!(rig.session.touch(object(0x31), rig.now), Ok(Touch::Waiting));
        rig.advance(secs(5));
        assert_eq!(rig.events(), []);
        rig.advance(secs(1));
        let mut events = rig.events();
        events.sort_by_key(|event| format!("{event:?}"));
        assert_eq!(events, [SessionEvent::NotDescribed(chair), SessionEvent::TouchFailed(object(0x31))]);
        // The scan has had its time: the next look gives up at once.
        assert_eq!(rig.session.look_for(chair, rig.now), Ok(Look::GaveUp));
        rig.advance(secs(30));
        assert_eq!(rig.events(), []);
        // Described later all the same, it is known.
        rig.prim(chair, 900, 0, [11.0, 20.0, 30.0]);
        assert_eq!(rig.events(), [], "nobody waits for it any more");
        assert_eq!(rig.session.look_for(chair, rig.now), Ok(Look::Known));
    }

    #[test]
    fn leaving_the_region_ends_a_wait() {
        let mut rig = Rig::standing();
        let chair = object(0x30);
        assert_eq!(rig.session.look_for(chair, rig.now), Ok(Look::Waiting));
        start_teleport(&mut rig);
        rig.session.handle_caps(1, finish(), rig.now);
        assert_eq!(
            rig.events(),
            [SessionEvent::NotDescribed(chair), SessionEvent::RegionLeft { worn: vec![] }]
        );
        // Not in a region while the teleport runs.
        assert_eq!(rig.session.look_for(chair, rig.now), Err(NOT_CONNECTED.to_string()));
    }

    #[test]
    fn a_scan_can_be_started_alone() {
        let mut rig = Rig::standing();
        rig.receive(messages::OBJECT_UPDATE_CACHED, &wire::object_update_cached(&[800]));
        rig.session.scan(rig.now);
        assert_eq!(
            requests(&rig.sent()),
            [messages::request_multiple_objects(&rig.agent(), &[800]).body]
        );
        rig.prim(object(0x30), 800, 0, [13.0, 24.0, 30.0]);
        assert_eq!(rig.events(), [], "nothing was waited for");
        // A second start changes nothing.
        rig.session.scan(rig.now);
        assert_eq!(requests(&rig.sent()), Vec::<Vec<u8>>::new());
    }

    // ------------------------------------------------------ the area scan

    use crate::udp::messages::{FLAG_SCRIPTED, FLAG_TOUCH, PCODE_AVATAR, PCODE_PRIM};

    /// A full update's block for a prim at `position`: in the region, or
    /// on its parent when it has one.
    fn prim(full_id: Uuid, local_id: u32, parent_id: u32, position: Vector3) -> wire::Block {
        wire::Block {
            full_id,
            local_id,
            parent_id,
            data: wire::object_data(position, [0.0; 3], false),
            worn: None,
        }
    }

    fn properties(object: Uuid, name: &str) -> Properties {
        Properties {
            object,
            owner: Uuid::from_bytes([0x1A; 16]),
            group: Uuid::nil(),
            name: name.to_string(),
            description: "Sit here".to_string(),
        }
    }

    const BENCH: u32 = 700;

    fn bench() -> Uuid {
        object(0x70)
    }

    impl Rig {
        /// Standing at 100, 100, 25 with a bench 3 metres away.
        fn by_a_bench() -> Rig {
            let mut rig = Rig::entered();
            rig.own_at(0, [100.0, 100.0, 25.0]);
            rig.describe(&[prim(bench(), BENCH, 0, [103.0, 100.0, 25.0])], &[]);
            rig.events();
            rig.ack_all();
            rig.sent();
            rig
        }

        fn describe(&mut self, blocks: &[wire::Block], kinds: &[(u8, u32)]) {
            self.receive(messages::OBJECT_UPDATE, &wire::object_blocks_of(blocks, kinds));
        }

        fn sighting(&self, object: Uuid) -> Option<Sighting> {
            self.session.sightings().into_iter().find(|sighting| sighting.object == object)
        }

        /// The selections made and ended since the last call, as the
        /// message and the local ids it names.
        fn selections(&mut self) -> Vec<(packet::MessageId, Vec<u32>)> {
            let select = |p: &Packet| p.id == messages::OBJECT_SELECT || p.id == messages::OBJECT_DESELECT;
            let ids = |body: &[u8]| -> Vec<u32> {
                assert_eq!(usize::from(body[32]) * 4, body.len() - 33);
                body[33..].chunks(4).map(|id| u32::from_le_bytes(id.try_into().unwrap())).collect()
            };
            self.sent().iter().filter(|p| select(p)).map(|p| (p.id, ids(&p.body))).collect()
        }

        fn named(&mut self, objects: &[Properties]) {
            self.receive(messages::OBJECT_PROPERTIES, &wire::object_properties(objects));
        }

        /// The owner requests sent since the last call, as the objects
        /// they are about.
        fn family_requests(&mut self) -> Vec<Uuid> {
            let family = |p: &&Packet| p.id == messages::REQUEST_OBJECT_PROPERTIES_FAMILY;
            let sent = self.sent();
            sent.iter()
                .filter(family)
                .map(|p| Uuid::from_slice(&p.body[36..52]).unwrap())
                .collect()
        }

        /// Time passes without the simulator going silent or the circuit
        /// sending anything again.
        fn wait(&mut self, by: Duration) {
            self.ack_all();
            self.advance(by);
            self.ack_all();
        }
    }

    #[test]
    fn a_later_update_changes_an_objects_flags() {
        let mut rig = Rig::by_a_bench();
        assert_eq!(rig.sighting(bench()).unwrap().flags, 0);
        rig.describe(
            &[prim(bench(), BENCH, 0, [103.0, 100.0, 25.0])],
            &[(PCODE_PRIM, FLAG_SCRIPTED | FLAG_TOUCH)],
        );
        assert_eq!(rig.sighting(bench()).unwrap().flags, FLAG_SCRIPTED | FLAG_TOUCH);
    }

    #[test]
    fn a_killed_object_takes_what_was_known_of_it_along() {
        let mut rig = Rig::by_a_bench();
        rig.session.reach(None, rig.now).unwrap();
        rig.session.ask_properties(&[BENCH], rig.now);
        rig.named(&[properties(bench(), "Bench")]);
        assert_eq!(rig.sighting(bench()).unwrap().properties, Some(properties(bench(), "Bench")));
        rig.receive(messages::KILL_OBJECT, &wire::kill_object(&[BENCH]));
        assert_eq!(rig.session.sightings(), []);
        // The simulator may give the number to another object.
        rig.describe(&[prim(object(0x71), BENCH, 0, [101.0, 100.0, 25.0])], &[]);
        let lamp = rig.sighting(object(0x71)).unwrap();
        assert_eq!((lamp.properties, lamp.flags), (None, 0));
        rig.sent();
        rig.session.ask_properties(&[BENCH], rig.now);
        assert_eq!(rig.selections().len(), 2, "asked about anew");
    }

    #[test]
    fn a_number_given_to_another_object_does_not_keep_the_first_ones_name() {
        let mut rig = Rig::by_a_bench();
        rig.session.reach(None, rig.now).unwrap();
        rig.session.ask_properties(&[BENCH], rig.now);
        rig.named(&[properties(bench(), "Bench")]);
        // No KillObject in between.
        let lamp = object(0x71);
        rig.describe(&[prim(lamp, BENCH, 0, [101.0, 100.0, 25.0])], &[]);
        assert_eq!(rig.sighting(bench()), None);
        assert_eq!(rig.sighting(lamp).unwrap().properties, None);
        assert!(!rig.session.knows(bench()) && rig.session.knows(lamp));
    }

    #[test]
    fn a_scan_command_raises_the_distance_until_the_region_is_left() {
        let mut rig = Rig::entered();
        rig.sent_all();
        assert_eq!(rig.session.reach(Some(96.0), rig.now), Ok(96.0));
        // The simulator is told at once, and the scan starts.
        let sent = rig.sent_all();
        let update = sent.iter().find(|p| p.id == messages::AGENT_UPDATE).expect("an update at once");
        assert_eq!(update_fields(update), (96.0, 0));
        assert!(sent.iter().any(|p| p.id == messages::AGENT_THROTTLE));
        rig.wait(secs(2));
        let updates: Vec<Packet> = rig.sent_all().into_iter().filter(|p| p.id == messages::AGENT_UPDATE).collect();
        assert!(!updates.is_empty() && updates.iter().all(|p| update_fields(p) == (96.0, 0)));

        // A shorter scan shortens its own listing only.
        assert_eq!(rig.session.reach(Some(10.0), rig.now), Ok(10.0));
        assert_eq!(rig.session.reach(None, rig.now), Ok(96.0));
        rig.wait(secs(2));
        assert!(rig
            .sent_all()
            .iter()
            .filter(|p| p.id == messages::AGENT_UPDATE)
            .all(|p| update_fields(p) == (96.0, 0)));
        assert!(rig.session.unknown_object(object(9)).contains("96 m"));

        // The next region starts over.
        rig.session.handle_caps(rig.generation(), finish(), rig.now);
        rig.enter("Elsewhere", HOME_HANDLE + 256);
        rig.wait(secs(2));
        let updates: Vec<Packet> = rig.sent_all().into_iter().filter(|p| p.id == messages::AGENT_UPDATE).collect();
        assert!(!updates.is_empty() && updates.iter().all(|p| update_fields(p) == (32.0, 0)));
        assert_eq!(rig.session.reach(None, rig.now), Ok(32.0));
    }

    #[test]
    fn a_scan_command_does_not_lower_the_configured_distance() {
        let mut rig = Rig::with(Some(64.0), false);
        rig.enter("Home", HOME_HANDLE);
        assert_eq!(rig.session.reach(Some(10.0), rig.now), Ok(10.0));
        assert_eq!(rig.session.reach(None, rig.now), Ok(64.0));
        rig.wait(secs(2));
        assert!(rig
            .sent_all()
            .iter()
            .filter(|p| p.id == messages::AGENT_UPDATE)
            .all(|p| update_fields(p) == (64.0, 0)));
        assert_eq!(rig.session.reach(Some(128.0), rig.now), Ok(128.0));
        assert_eq!(rig.session.reach(None, rig.now), Ok(128.0));
    }

    #[test]
    fn a_scan_command_needs_the_avatar_in_a_region() {
        let mut rig = Rig::new();
        rig.sent_all();
        assert_eq!(rig.session.reach(Some(96.0), rig.now), Err(NOT_CONNECTED.to_string()));
        rig.session.ask_properties(&[BENCH], rig.now);
        assert_eq!(rig.sent_all().len(), 0);

        let mut rig = Rig::by_a_bench();
        start_teleport(&mut rig);
        rig.sent_all();
        assert_eq!(rig.session.reach(None, rig.now), Err(NOT_CONNECTED.to_string()));
        rig.session.ask_properties(&[BENCH], rig.now);
        assert_eq!(rig.sent_all().len(), 0);
    }

    #[test]
    fn the_task_throttle_is_widened_while_a_scan_runs() {
        let throttles = |packets: &[Packet]| -> Vec<Vec<u8>> {
            let throttle = |p: &&Packet| p.id == messages::AGENT_THROTTLE;
            packets.iter().filter(throttle).map(|p| p.body.clone()).collect()
        };
        let mut rig = Rig::by_a_bench();
        let agent = rig.agent();
        let (base, wide) = (
            messages::agent_throttle(&agent, 77, THROTTLES).body,
            messages::agent_throttle(&agent, 77, SCAN_THROTTLES).body,
        );
        assert_ne!(base, wide);
        rig.session.reach(None, rig.now).unwrap();
        assert_eq!(throttles(&rig.sent()), vec![wide.clone()]);
        // Once per scan.
        rig.session.reach(Some(96.0), rig.now).unwrap();
        assert_eq!(rig.session.touch(object(9), rig.now), Ok(Touch::Waiting));
        assert_eq!(throttles(&rig.sent()), Vec::<Vec<u8>>::new());

        // The next simulator gets the narrow one again.
        rig.session.handle_caps(rig.generation(), finish(), rig.now);
        rig.sent();
        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Elsewhere"));
        let body = wire::agent_movement_complete(&agent, [10.0, 20.0, 30.0], HOME_HANDLE + 256);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        assert_eq!(throttles(&rig.sent()), vec![base.clone()]);

        // With a configured distance the scan starts with the region.
        let mut rig = Rig::with(Some(64.0), false);
        rig.sent();
        rig.receive(messages::REGION_HANDSHAKE, &wire::region_handshake("Home"));
        let body = wire::agent_movement_complete(&agent, [10.0, 20.0, 30.0], HOME_HANDLE);
        rig.receive(messages::AGENT_MOVEMENT_COMPLETE, &body);
        assert_eq!(throttles(&rig.sent()), [base, wide]);
    }

    #[test]
    fn properties_are_asked_for_by_selecting_and_only_once() {
        let mut rig = Rig::by_a_bench();
        let lamp = object(0x71);
        rig.describe(&[prim(lamp, 701, 0, [100.0, 110.0, 25.0])], &[]);
        rig.session.reach(None, rig.now).unwrap();
        rig.sent();

        // Twice the bench, and a number the simulator never described.
        rig.session.ask_properties(&[701, BENCH, BENCH, 999], rig.now);
        let (select, deselect) = (messages::OBJECT_SELECT, messages::OBJECT_DESELECT);
        assert_eq!(rig.selections(), [(select, vec![BENCH, 701]), (deselect, vec![BENCH, 701])]);
        // A second scan right away shares the request.
        rig.session.ask_properties(&[BENCH, 701], rig.now);
        assert_eq!(rig.selections(), []);

        rig.named(&[properties(bench(), "Bench")]);
        assert_eq!(rig.sighting(bench()).unwrap().properties, Some(properties(bench(), "Bench")));
        assert_eq!(rig.sighting(lamp).unwrap().properties, None);
        rig.wait(secs(9));
        rig.sent();
        rig.session.ask_properties(&[BENCH, 701], rig.now);
        assert_eq!(rig.selections(), []);
        // An answer that did not come in 10 seconds is asked for again;
        // one that came, never.
        rig.wait(secs(1));
        rig.sent();
        rig.session.ask_properties(&[BENCH, 701], rig.now);
        assert_eq!(rig.selections(), [(select, vec![701]), (deselect, vec![701])]);
        rig.named(&[properties(lamp, "Lamp")]);
        rig.wait(secs(30));
        rig.sent();
        rig.session.ask_properties(&[BENCH, 701], rig.now);
        assert_eq!(rig.selections(), []);

        // Properties of an object the simulator has not described are
        // dropped.
        rig.named(&[properties(object(0x7F), "Ghost")]);
        assert_eq!(rig.session.sightings().len(), 2);
    }

    #[test]
    fn selections_name_at_most_255_objects() {
        let mut rig = Rig::by_a_bench();
        let many: Vec<(Uuid, u32, Vector3, Vector3)> = (0..300u32)
            .map(|n| (Uuid::from_u128(u128::from(n) + 1), 2000 + n, [101.0, 100.0, 25.0], [0.0; 3]))
            .collect();
        for half in many.chunks(150) {
            rig.receive(messages::OBJECT_UPDATE_COMPRESSED, &wire::object_update_compressed_at(half));
        }
        rig.session.reach(None, rig.now).unwrap();
        rig.sent();
        let ids: Vec<u32> = many.iter().map(|object| object.1).collect();
        rig.session.ask_properties(&ids, rig.now);
        let sent = rig.selections();
        let (select, deselect) = (messages::OBJECT_SELECT, messages::OBJECT_DESELECT);
        let shape: Vec<(packet::MessageId, usize)> = sent.iter().map(|(id, ids)| (*id, ids.len())).collect();
        assert_eq!(shape, [(select, 255), (deselect, 255), (select, 45), (deselect, 45)]);
        assert_eq!([&sent[0].1[..], &sent[2].1[..]].concat(), ids);
        assert_eq!(sent[0].1, sent[1].1);
    }

    #[test]
    fn sightings_are_the_prims_around_with_their_distance() {
        let mut rig = Rig::by_a_bench();
        let (cushion, jane, hat, strap, radio, tree, far, lost) = (
            object(0x71),
            object(0x80),
            object(0x81),
            object(0x82),
            object(0x90),
            object(0xA0),
            object(0xB0),
            object(0xC0),
        );
        let blocks = [
            prim(cushion, 701, BENCH, [0.5, 0.0, 0.0]),
            wire::Block {
                data: wire::object_data([100.0, 104.0, 25.0], [0.0; 3], true),
                ..prim(jane, 800, 0, [0.0; 3])
            },
            prim(hat, 801, 800, [0.0, 0.0, 0.9]),
            prim(strap, 802, 801, [0.0, 0.1, 0.0]),
            prim(radio, 810, OWN, [0.0, 0.2, 0.0]),
            prim(tree, 820, 0, [101.0, 100.0, 25.0]),
            prim(far, 830, 0, [100.0, 100.0, 55.0]),
            // A prim whose update says nothing the bot reads of where it is.
            wire::Block {
                data: vec![0xCD; 32],
                ..prim(lost, 840, 0, [0.0; 3])
            },
        ];
        let kinds = [
            (PCODE_PRIM, FLAG_SCRIPTED),
            (PCODE_AVATAR, 0),
            (PCODE_PRIM, 0),
            (PCODE_PRIM, 0),
            (PCODE_PRIM, FLAG_TOUCH),
            (255, 0),
        ];
        rig.describe(&blocks, &kinds);

        let seen = |rig: &Rig, object: Uuid| {
            let sighting = rig.sighting(object).unwrap_or_else(|| panic!("{object} is not seen"));
            (sighting.distance, sighting.attachment, sighting.child, sighting.flags)
        };
        assert_eq!(seen(&rig, bench()), (3.0, false, false, 0));
        // A child prim is where its linkset's root is.
        assert_eq!(seen(&rig, cushion), (3.0, false, true, FLAG_SCRIPTED));
        // What an avatar wears is where the avatar is.
        assert_eq!(seen(&rig, hat), (4.0, true, false, 0));
        assert_eq!(seen(&rig, strap), (4.0, true, true, 0));
        assert_eq!(seen(&rig, radio), (0.0, true, false, FLAG_TOUCH));
        assert_eq!(seen(&rig, far), (30.0, false, false, 0));
        // No avatars, the bot's own included, no trees, and nothing
        // whose place is not known.
        let mut all: Vec<Uuid> = rig.session.sightings().iter().map(|sighting| sighting.object).collect();
        all.sort();
        assert_eq!(all, [bench(), cushion, hat, strap, radio, far]);
        assert_eq!(rig.sighting(bench()).unwrap().local_id, BENCH);

        // An avatar that moves takes what it wears along.
        rig.terse(800, true, [100.0, 110.0, 25.0]);
        assert_eq!(seen(&rig, hat).0, 10.0);
    }

    #[test]
    fn distances_are_measured_from_where_the_avatar_sits() {
        let mut rig = Rig::on_chair([0.0, 0.0, 1.0]);
        rig.chair_at([150.0, 100.0, 25.0], [0.0; 3]);
        assert!(near(rig.position(), [150.0, 100.0, 26.0]));
        let chair = rig.sighting(target()).unwrap();
        assert!((chair.distance - 1.0).abs() < 1e-3 && !chair.child && !chair.attachment);
    }

    #[test]
    fn compressed_updates_say_what_an_object_is_too() {
        let mut rig = Rig::by_a_bench();
        let (door, jane) = (object(0x71), object(0x80));
        let avatar = wire::Compressed {
            pcode: Some(PCODE_AVATAR),
            ..wire::Compressed::default()
        };
        // `compressed_data` puts everything at 1, 2, 3.
        let blocks = [
            (FLAG_TOUCH, wire::compressed_data(door, 701, &wire::Compressed::default())),
            (0, wire::compressed_data(jane, 800, &avatar)),
        ];
        rig.receive(messages::OBJECT_UPDATE_COMPRESSED, &wire::object_update_compressed_flagged(&blocks));
        let seen = rig.sighting(door).unwrap();
        assert_eq!(seen.flags, FLAG_TOUCH);
        assert!((seen.distance - (99.0f32.powi(2) + 98.0f32.powi(2) + 22.0f32.powi(2)).sqrt()).abs() < 1e-3);
        assert_eq!(rig.sighting(jane), None);
    }

    #[test]
    fn the_surroundings_settle_three_seconds_after_the_last_new_object() {
        let mut rig = Rig::entered();
        rig.own_at(0, [100.0, 100.0, 25.0]);
        rig.wait(secs(20));
        let start = rig.now;
        rig.session.reach(None, start).unwrap();
        assert_eq!(rig.session.settled_at(), start + SCAN_QUIET);

        rig.wait(secs(1));
        rig.describe(&[prim(bench(), BENCH, 0, [103.0, 100.0, 25.0])], &[]);
        assert_eq!(rig.session.settled_at(), start + secs(1) + SCAN_QUIET);
        // The same object again is nothing new, nor is one that moves.
        rig.wait(secs(1));
        rig.describe(&[prim(bench(), BENCH, 0, [104.0, 100.0, 25.0])], &[]);
        rig.terse(BENCH, false, [105.0, 100.0, 25.0]);
        assert_eq!(rig.session.settled_at(), start + secs(1) + SCAN_QUIET);

        // A raise of the distance starts the wait over; a scan that
        // stays within it does not.
        assert_eq!(rig.session.reach(Some(96.0), rig.now), Ok(96.0));
        assert_eq!(rig.session.settled_at(), start + secs(2) + SCAN_QUIET);
        rig.wait(secs(10));
        assert_eq!(rig.session.reach(Some(50.0), rig.now), Ok(50.0));
        assert_eq!(rig.session.reach(Some(96.0), rig.now), Ok(96.0));
        assert_eq!(rig.session.settled_at(), start + secs(2) + SCAN_QUIET);
    }

    #[test]
    fn the_seats_name_is_asked_for_once_per_sit() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        // Nothing to ask by until the seat is described.
        assert_eq!(rig.family_requests(), Vec::<Uuid>::new());
        assert_eq!(rig.session.seat_name(), None);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        assert_eq!(rig.family_requests(), [target()]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        assert_eq!(rig.family_requests(), Vec::<Uuid>::new());
        assert_eq!(rig.session.seat_name(), None);

        // An answer about another object is not the seat's.
        let (owner, group) = (object(0x1A), Uuid::nil());
        let body = wire::object_properties_family_named(object(0x55), owner, group, "Dance Ball", "");
        rig.receive(messages::OBJECT_PROPERTIES_FAMILY, &body);
        assert_eq!(rig.session.seat_name(), None);
        let body = wire::object_properties_family_named(target(), owner, group, "Park Bench", "Sit here");
        rig.receive(messages::OBJECT_PROPERTIES_FAMILY, &body);
        assert_eq!(rig.session.seat_name(), Some("Park Bench"));
        rig.events();

        // The name goes when the avatar stands up, and is asked for
        // again when it sits down again.
        rig.own_at(0, [132.0, 65.0, 23.5]);
        assert_eq!(rig.session.seat_name(), None);
        assert_eq!(rig.family_requests(), Vec::<Uuid>::new());
        rig.own_at(CHAIR, [0.0, 0.5, 0.6]);
        assert_eq!(rig.family_requests(), [target()], "the seat is still described");
        assert_eq!(rig.session.seat_name(), None);
    }

    #[test]
    fn sitting_costs_two_requests_about_the_seat_and_nothing_else() {
        let mut rig = Rig::entered();
        rig.own_at(0, [100.0, 100.0, 25.0]);
        rig.session.start_sit(target(), rig.now);
        rig.sit_response(target());
        rig.own_at(CHAIR, [0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        rig.wait(secs(30));
        let learning = [messages::REQUEST_MULTIPLE_OBJECTS, messages::REQUEST_OBJECT_PROPERTIES_FAMILY];
        let ids: Vec<packet::MessageId> = rig
            .sent_ids()
            .into_iter()
            .filter(|id| *id != messages::AGENT_REQUEST_SIT && *id != messages::AGENT_SIT)
            .collect();
        assert_eq!(ids, learning);
    }

    #[test]
    fn standing_and_ground_sits_ask_for_no_name() {
        let mut rig = Rig::by_a_bench();
        rig.session.sit_ground(rig.now).unwrap();
        rig.own_at(0, [100.0, 100.0, 24.5]);
        rig.describe(&[prim(object(0x71), 701, 0, [101.0, 100.0, 25.0])], &[]);
        assert_eq!(rig.family_requests(), Vec::<Uuid>::new());
        assert_eq!(rig.session.seat_name(), None);
    }

    #[test]
    fn the_seats_name_is_forgotten_with_the_region() {
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        let body = wire::object_properties_family_named(target(), object(0x1A), Uuid::nil(), "Park Bench", "");
        rig.receive(messages::OBJECT_PROPERTIES_FAMILY, &body);
        assert_eq!(rig.session.seat_name(), Some("Park Bench"));
        rig.session.handle_caps(rig.generation(), finish(), rig.now);
        rig.enter("Elsewhere", HOME_HANDLE + 256);
        assert_eq!(rig.session.seat_name(), None);
    }

    #[test]
    fn a_scan_and_the_seat_share_what_the_simulator_said() {
        // The scan named the chair before the avatar sat down: no request.
        let mut rig = Rig::entered();
        rig.own_at(0, [100.0, 100.0, 25.0]);
        rig.chair_at([103.0, 100.0, 25.0], [0.0; 3]);
        rig.session.reach(None, rig.now).unwrap();
        rig.session.ask_properties(&[CHAIR], rig.now);
        rig.named(&[properties(target(), "Sofa")]);
        rig.session.start_sit(target(), rig.now);
        rig.sit_response(target());
        rig.own_at(CHAIR, [0.0, 0.0, 0.5]);
        rig.chair_at([103.0, 100.0, 25.0], [0.0; 3]);
        assert_eq!(rig.family_requests(), Vec::<Uuid>::new());
        assert_eq!(rig.session.seat_name(), Some("Sofa"));

        // The seat was named for `status`: a scan does not select it.
        let mut rig = Rig::on_chair([0.0, 0.5, 0.6]);
        rig.chair_at([131.0, 64.0, 22.4], [0.0; 3]);
        let body = wire::object_properties_family_named(target(), object(0x1A), Uuid::nil(), "Park Bench", "Sit here");
        rig.receive(messages::OBJECT_PROPERTIES_FAMILY, &body);
        rig.session.reach(None, rig.now).unwrap();
        rig.sent();
        rig.session.ask_properties(&[CHAIR], rig.now);
        assert_eq!(rig.selections(), []);
        assert_eq!(rig.sighting(target()).unwrap().properties, Some(properties(target(), "Park Bench")));
    }
}
