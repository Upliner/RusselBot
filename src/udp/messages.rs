//! The handful of messages the bot uses, hand-written against
//! `message_template.msg` (template version 2.0). Body fields are
//! little-endian; strings are `Variable 1` with a trailing NUL.

use super::packet::{high, low, medium, MessageId};
use std::fmt;
use uuid::Uuid;

// Template name, frequency and number for every message handled here.
pub const START_PING_CHECK: MessageId = high(1);
pub const COMPLETE_PING_CHECK: MessageId = high(2);
pub const AGENT_UPDATE: MessageId = high(4);
pub const AGENT_REQUEST_SIT: MessageId = high(6);
pub const AGENT_SIT: MessageId = high(7);
pub const OBJECT_UPDATE: MessageId = high(12);
pub const AVATAR_SIT_RESPONSE: MessageId = high(21);
pub const USE_CIRCUIT_CODE: MessageId = low(3);
pub const AVATAR_PICKER_REQUEST: MessageId = low(26);
pub const AVATAR_PICKER_REPLY: MessageId = low(28);
pub const START_LURE: MessageId = low(70);
pub const TELEPORT_LURE_REQUEST: MessageId = low(71);
pub const OBJECT_GRAB: MessageId = low(117);
pub const OBJECT_DEGRAB: MessageId = low(119);
pub const REQUEST_MULTIPLE_OBJECTS: MessageId = medium(3);
pub const OBJECT_UPDATE_COMPRESSED: MessageId = high(13);
pub const TELEPORT_LOCATION_REQUEST: MessageId = low(63);
pub const TELEPORT_LOCAL: MessageId = low(64);
pub const TELEPORT_PROGRESS: MessageId = low(66);
pub const TELEPORT_START: MessageId = low(73);
pub const TELEPORT_FAILED: MessageId = low(74);
pub const CHAT_FROM_VIEWER: MessageId = low(80);
pub const AGENT_THROTTLE: MessageId = low(81);
pub const ALERT_MESSAGE: MessageId = low(134);
pub const AGENT_ALERT_MESSAGE: MessageId = low(135);
pub const CHAT_FROM_SIMULATOR: MessageId = low(139);
pub const REGION_HANDSHAKE: MessageId = low(148);
pub const REGION_HANDSHAKE_REPLY: MessageId = low(149);
pub const DISABLE_SIMULATOR: MessageId = low(152);
pub const KICK_USER: MessageId = low(163);
pub const COMPLETE_AGENT_MOVEMENT: MessageId = low(249);
pub const AGENT_MOVEMENT_COMPLETE: MessageId = low(250);
pub const LOGOUT_REQUEST: MessageId = low(252);
pub const LOGOUT_REPLY: MessageId = low(253);
pub const IMPROVED_INSTANT_MESSAGE: MessageId = low(254);
pub const RETRIEVE_INSTANT_MESSAGES: MessageId = low(255);
pub const MAP_NAME_REQUEST: MessageId = low(408);
pub const MAP_BLOCK_REPLY: MessageId = low(409);
pub const PACKET_ACK: MessageId = 0xFFFF_FFFB;

// Messages a real simulator sends that the bot ignores; listed so verbose
// logging can name them.
pub const LAYER_DATA: MessageId = high(11);
pub const OBJECT_UPDATE_CACHED: MessageId = high(14);
pub const IMPROVED_TERSE_OBJECT_UPDATE: MessageId = high(15);
pub const KILL_OBJECT: MessageId = high(16);
pub const AVATAR_ANIMATION: MessageId = high(20);
pub const CAMERA_CONSTRAINT: MessageId = high(22);
pub const SOUND_TRIGGER: MessageId = high(29);
pub const COARSE_LOCATION_UPDATE: MessageId = medium(6);
pub const ATTACHED_SOUND: MessageId = medium(13);
pub const VIEWER_EFFECT: MessageId = medium(17);
pub const HEALTH_MESSAGE: MessageId = low(138);
pub const SIM_STATS: MessageId = low(140);
pub const SIMULATOR_VIEWER_TIME_MESSAGE: MessageId = low(150);
pub const AVATAR_APPEARANCE: MessageId = low(158);
pub const SCRIPT_CONTROL_CHANGE: MessageId = low(189);
pub const SCRIPT_DIALOG: MessageId = low(190);
pub const PARCEL_OVERLAY: MessageId = low(196);
pub const ONLINE_NOTIFICATION: MessageId = low(322);
pub const AGENT_DATA_UPDATE: MessageId = low(387);

pub type Vector3 = [f32; 3];

/// Template name of a message the bot knows, for verbose logging.
pub fn name(id: MessageId) -> String {
    let known = match id {
        START_PING_CHECK => "StartPingCheck",
        COMPLETE_PING_CHECK => "CompletePingCheck",
        AGENT_UPDATE => "AgentUpdate",
        AGENT_REQUEST_SIT => "AgentRequestSit",
        AGENT_SIT => "AgentSit",
        OBJECT_UPDATE => "ObjectUpdate",
        AVATAR_SIT_RESPONSE => "AvatarSitResponse",
        USE_CIRCUIT_CODE => "UseCircuitCode",
        AVATAR_PICKER_REQUEST => "AvatarPickerRequest",
        AVATAR_PICKER_REPLY => "AvatarPickerReply",
        START_LURE => "StartLure",
        TELEPORT_LURE_REQUEST => "TeleportLureRequest",
        OBJECT_GRAB => "ObjectGrab",
        OBJECT_DEGRAB => "ObjectDeGrab",
        REQUEST_MULTIPLE_OBJECTS => "RequestMultipleObjects",
        OBJECT_UPDATE_COMPRESSED => "ObjectUpdateCompressed",
        TELEPORT_LOCATION_REQUEST => "TeleportLocationRequest",
        TELEPORT_LOCAL => "TeleportLocal",
        TELEPORT_PROGRESS => "TeleportProgress",
        TELEPORT_START => "TeleportStart",
        TELEPORT_FAILED => "TeleportFailed",
        CHAT_FROM_VIEWER => "ChatFromViewer",
        AGENT_THROTTLE => "AgentThrottle",
        ALERT_MESSAGE => "AlertMessage",
        AGENT_ALERT_MESSAGE => "AgentAlertMessage",
        REGION_HANDSHAKE => "RegionHandshake",
        REGION_HANDSHAKE_REPLY => "RegionHandshakeReply",
        DISABLE_SIMULATOR => "DisableSimulator",
        KICK_USER => "KickUser",
        COMPLETE_AGENT_MOVEMENT => "CompleteAgentMovement",
        AGENT_MOVEMENT_COMPLETE => "AgentMovementComplete",
        LOGOUT_REQUEST => "LogoutRequest",
        LOGOUT_REPLY => "LogoutReply",
        IMPROVED_INSTANT_MESSAGE => "ImprovedInstantMessage",
        RETRIEVE_INSTANT_MESSAGES => "RetrieveInstantMessages",
        MAP_NAME_REQUEST => "MapNameRequest",
        MAP_BLOCK_REPLY => "MapBlockReply",
        PACKET_ACK => "PacketAck",
        LAYER_DATA => "LayerData",
        OBJECT_UPDATE_CACHED => "ObjectUpdateCached",
        IMPROVED_TERSE_OBJECT_UPDATE => "ImprovedTerseObjectUpdate",
        KILL_OBJECT => "KillObject",
        AVATAR_ANIMATION => "AvatarAnimation",
        CAMERA_CONSTRAINT => "CameraConstraint",
        SOUND_TRIGGER => "SoundTrigger",
        COARSE_LOCATION_UPDATE => "CoarseLocationUpdate",
        ATTACHED_SOUND => "AttachedSound",
        VIEWER_EFFECT => "ViewerEffect",
        HEALTH_MESSAGE => "HealthMessage",
        CHAT_FROM_SIMULATOR => "ChatFromSimulator",
        SIM_STATS => "SimStats",
        SIMULATOR_VIEWER_TIME_MESSAGE => "SimulatorViewerTimeMessage",
        AVATAR_APPEARANCE => "AvatarAppearance",
        SCRIPT_CONTROL_CHANGE => "ScriptControlChange",
        SCRIPT_DIALOG => "ScriptDialog",
        PARCEL_OVERLAY => "ParcelOverlay",
        ONLINE_NOTIFICATION => "OnlineNotification",
        AGENT_DATA_UPDATE => "AgentDataUpdate",
        0..=0xFE => return format!("High {id}"),
        0xFF00..=0xFFFE => return format!("Medium {}", id & 0xFF),
        _ => return format!("Low {}", id & 0xFFFF),
    };
    known.to_string()
}

/// A message ready to be framed and sent.
#[derive(Debug, PartialEq)]
pub struct Outgoing {
    pub id: MessageId,
    /// Whether the template marks the message Zerocoded.
    pub zerocoded: bool,
    pub body: Vec<u8>,
}

/// The agent and session ids that prefix most viewer messages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agent {
    pub agent_id: Uuid,
    pub session_id: Uuid,
}

/// The longest chat or instant-message text a simulator takes, in bytes,
/// not counting the trailing NUL.
pub const MAX_TEXT: usize = 1023;

/// The first `limit` bytes of `text`, then a NUL.
fn terminated(text: &str, limit: usize) -> Vec<u8> {
    let mut data = text.as_bytes()[..text.len().min(limit)].to_vec();
    data.push(0);
    data
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn agent(agent: &Agent) -> Writer {
        Writer::default()
            .uuid(agent.agent_id)
            .uuid(agent.session_id)
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn i32(mut self, v: i32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn f32(mut self, v: f32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn uuid(mut self, v: Uuid) -> Self {
        self.0.extend(v.as_bytes());
        self
    }
    fn vector3(self, v: Vector3) -> Self {
        self.f32(v[0]).f32(v[1]).f32(v[2])
    }
    /// `Variable 1`: one length byte, then the data.
    fn variable1(mut self, data: &[u8]) -> Self {
        self.0.push(data.len() as u8);
        self.0.extend(data);
        self
    }
    /// `Variable 2`: two length bytes, then the data.
    fn variable2(mut self, data: &[u8]) -> Self {
        self.0.extend((data.len() as u16).to_le_bytes());
        self.0.extend(data);
        self
    }
    /// Text with its trailing NUL, as `Variable 1`; cut to what fits.
    fn text1(self, text: &str) -> Self {
        self.variable1(&terminated(text, 254))
    }
    /// Text with its trailing NUL, as `Variable 2`; cut to what fits.
    fn text2(self, text: &str) -> Self {
        self.variable2(&terminated(text, MAX_TEXT))
    }
    fn finish(self, id: MessageId, zerocoded: bool) -> Outgoing {
        Outgoing {
            id,
            zerocoded,
            body: self.0,
        }
    }
}

/// UseCircuitCode, Low 3.
pub fn use_circuit_code(agent: &Agent, circuit_code: u32) -> Outgoing {
    Writer::default()
        .u32(circuit_code)
        .uuid(agent.session_id)
        .uuid(agent.agent_id)
        .finish(USE_CIRCUIT_CODE, false)
}

/// CompleteAgentMovement, Low 249.
pub fn complete_agent_movement(agent: &Agent, circuit_code: u32) -> Outgoing {
    Writer::agent(agent)
        .u32(circuit_code)
        .finish(COMPLETE_AGENT_MOVEMENT, false)
}

/// RegionHandshakeReply, Low 149, Zerocoded.
pub fn region_handshake_reply(agent: &Agent) -> Outgoing {
    Writer::agent(agent)
        .u32(0)
        .finish(REGION_HANDSHAKE_REPLY, true)
}

/// AgentThrottle, Low 81, Zerocoded. `throttles` are bits per second for
/// resend, land, wind, cloud, task, texture and asset, in that order.
pub fn agent_throttle(agent: &Agent, circuit_code: u32, throttles: [f32; 7]) -> Outgoing {
    let data: Vec<u8> = throttles.iter().flat_map(|t| t.to_le_bytes()).collect();
    Writer::agent(agent)
        .u32(circuit_code)
        .u32(0)
        .variable1(&data)
        .finish(AGENT_THROTTLE, true)
}

/// AgentUpdate control flag: stand up from whatever the avatar sits on.
pub const CONTROL_STAND_UP: u32 = 1 << 16;
/// AgentUpdate control flag: sit down on the ground.
pub const CONTROL_SIT_ON_GROUND: u32 = 1 << 17;

/// AgentUpdate, High 4, Zerocoded: an avatar with a fixed camera that does
/// nothing but what `control_flags` say.
pub fn agent_update(agent: &Agent, camera: Vector3, far: f32, control_flags: u32) -> Outgoing {
    Writer::agent(agent)
        .vector3([0.0; 3]) // BodyRotation: identity (x, y, z; w is implied)
        .vector3([0.0; 3]) // HeadRotation
        .u8(0) // State
        .vector3(camera) // CameraCenter
        .vector3([1.0, 0.0, 0.0]) // CameraAtAxis
        .vector3([0.0, 1.0, 0.0]) // CameraLeftAxis
        .vector3([0.0, 0.0, 1.0]) // CameraUpAxis
        .f32(far)
        .u32(control_flags)
        .u8(0) // Flags
        .finish(AGENT_UPDATE, true)
}

/// CompletePingCheck, High 2.
pub fn complete_ping_check(ping_id: u8) -> Outgoing {
    Writer::default()
        .u8(ping_id)
        .finish(COMPLETE_PING_CHECK, false)
}

/// PacketAck, Fixed 0xFFFFFFFB. At most 255 ids fit in one message.
pub fn packet_ack(ids: &[u32]) -> Outgoing {
    let mut writer = Writer::default().u8(ids.len() as u8);
    for id in ids {
        writer = writer.u32(*id);
    }
    writer.finish(PACKET_ACK, false)
}

/// LogoutRequest, Low 252.
pub fn logout_request(agent: &Agent) -> Outgoing {
    Writer::agent(agent).finish(LOGOUT_REQUEST, false)
}

/// RetrieveInstantMessages, Low 255: asks for the IMs stored while offline.
pub fn retrieve_instant_messages(agent: &Agent) -> Outgoing {
    Writer::agent(agent).finish(RETRIEVE_INSTANT_MESSAGES, false)
}

/// MapNameRequest, Low 408.
pub fn map_name_request(agent: &Agent, name: &str) -> Outgoing {
    Writer::agent(agent)
        .u32(0) // Flags
        .u32(0) // EstateID, filled in on the simulator
        .u8(0) // Godlike, filled in on the simulator
        .text1(name)
        .finish(MAP_NAME_REQUEST, false)
}

/// TeleportLocationRequest, Low 63.
pub fn teleport_location_request(agent: &Agent, region_handle: u64, position: Vector3) -> Outgoing {
    Writer::agent(agent)
        .u64(region_handle)
        .vector3(position)
        .vector3([1.0, 0.0, 0.0]) // LookAt
        .finish(TELEPORT_LOCATION_REQUEST, false)
}

/// AgentRequestSit, High 6, Zerocoded.
pub fn agent_request_sit(agent: &Agent, target: Uuid) -> Outgoing {
    Writer::agent(agent)
        .uuid(target)
        .vector3([0.0; 3]) // Offset
        .finish(AGENT_REQUEST_SIT, true)
}

/// AgentSit, High 7.
pub fn agent_sit(agent: &Agent) -> Outgoing {
    Writer::agent(agent).finish(AGENT_SIT, false)
}

/// ChatFromViewer, Low 80, Zerocoded: Message Variable 2, Type U8,
/// Channel S32. `chat_type` is 0 whisper, 1 say, 2 shout.
pub fn chat_from_viewer(agent: &Agent, text: &str, chat_type: u8, channel: i32) -> Outgoing {
    Writer::agent(agent)
        .text2(text)
        .u8(chat_type)
        .i32(channel)
        .finish(CHAT_FROM_VIEWER, true)
}

/// Instant-message dialog: an ordinary message.
pub const IM_PLAIN: u8 = 0;
/// Instant-message dialog: an offer to teleport the recipient to the sender.
pub const IM_TELEPORT_OFFER: u8 = 22;
/// Instant-message dialog: a request to be teleported to the recipient.
pub const IM_TELEPORT_REQUEST: u8 = 26;
/// TeleportFlags: the teleport answers an offer.
const TELEPORT_VIA_LURE: u32 = 1 << 2;

/// ImprovedInstantMessage, Low 254, Zerocoded: an instant message from the
/// agent to another avatar. For a plain message the conversation id is the
/// two agent ids combined, which is how viewers pair a reply with the
/// message; other dialogs carry no id.
pub fn instant_message(
    agent: &Agent,
    from_name: &str,
    to: Uuid,
    position: Vector3,
    dialog: u8,
    text: &str,
) -> Outgoing {
    let conversation = match dialog {
        IM_PLAIN => Uuid::from_u128(agent.agent_id.as_u128() ^ to.as_u128()),
        _ => Uuid::nil(),
    };
    Writer::agent(agent)
        .u8(0) // FromGroup
        .uuid(to)
        .u32(0) // ParentEstateID, filled in on the simulator
        .uuid(Uuid::nil()) // RegionID, filled in on the simulator
        .vector3(position)
        .u8(0) // Offline
        .u8(dialog)
        .uuid(conversation)
        .u32(0) // Timestamp
        .text1(from_name)
        .text2(text)
        .variable2(&[0]) // BinaryBucket
        .u32(0) // EstateBlock
        .u8(0) // MetaData block count
        .finish(IMPROVED_INSTANT_MESSAGE, true)
}

/// AvatarPickerRequest, Low 26: QueryID LLUUID, Name Variable 1. Asks for
/// the avatars whose names match `name`.
pub fn avatar_picker_request(agent: &Agent, query: Uuid, name: &str) -> Outgoing {
    Writer::agent(agent)
        .uuid(query)
        .text1(name)
        .finish(AVATAR_PICKER_REQUEST, false)
}

/// StartLure, Low 70: LureType U8, Message Variable 1, then the avatars
/// offered a teleport to where the agent is.
pub fn start_lure(agent: &Agent, message: &str, target: Uuid) -> Outgoing {
    Writer::agent(agent)
        .u8(0) // LureType
        .text1(message)
        .u8(1) // TargetData block count
        .uuid(target)
        .finish(START_LURE, false)
}

/// TeleportLureRequest, Low 71: accepts the teleport offer `lure`.
pub fn teleport_lure_request(agent: &Agent, lure: Uuid) -> Outgoing {
    Writer::agent(agent)
        .uuid(lure)
        .u32(TELEPORT_VIA_LURE)
        .finish(TELEPORT_LURE_REQUEST, false)
}

/// ObjectGrab, Low 117, Zerocoded: LocalID U32, GrabOffset, then
/// SurfaceInfo Variable. The press of a touch.
pub fn object_grab(agent: &Agent, local_id: u32) -> Outgoing {
    Writer::agent(agent)
        .u32(local_id)
        .vector3([0.0; 3]) // GrabOffset
        .u8(0) // SurfaceInfo block count
        .finish(OBJECT_GRAB, true)
}

/// ObjectDeGrab, Low 119: LocalID U32, then SurfaceInfo Variable. The
/// release of a touch.
pub fn object_degrab(agent: &Agent, local_id: u32) -> Outgoing {
    Writer::agent(agent)
        .u32(local_id)
        .u8(0) // SurfaceInfo block count
        .finish(OBJECT_DEGRAB, false)
}

/// RequestMultipleObjects, Medium 3, Zerocoded: ObjectData Variable
/// { CacheMissType U8, ID U32 }. Asks for the full description of objects
/// the simulator assumed to be cached. At most 255 ids fit in one message.
pub fn request_multiple_objects(agent: &Agent, local_ids: &[u32]) -> Outgoing {
    let mut writer = Writer::agent(agent).u8(local_ids.len() as u8);
    for id in local_ids {
        writer = writer.u8(0).u32(*id); // CacheMissType: full
    }
    writer.finish(REQUEST_MULTIPLE_OBJECTS, true)
}

// ---------------------------------------------------------------- incoming

#[derive(Debug, PartialEq)]
pub struct DecodeError;

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("message body is shorter than its layout")
    }
}

type Decoded<T> = Result<T, DecodeError>;

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Decoded<&'a [u8]> {
        if self.0.len() < len {
            return Err(DecodeError);
        }
        let (head, tail) = self.0.split_at(len);
        self.0 = tail;
        Ok(head)
    }
    fn skip(&mut self, len: usize) -> Decoded<()> {
        self.bytes(len).map(|_| ())
    }
    fn array<const N: usize>(&mut self) -> Decoded<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("length checked"))
    }
    fn u8(&mut self) -> Decoded<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Decoded<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Decoded<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Decoded<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn uuid(&mut self) -> Decoded<Uuid> {
        Ok(Uuid::from_bytes(self.array()?))
    }
    fn vector3(&mut self) -> Decoded<Vector3> {
        let mut v = [0.0; 3];
        for item in &mut v {
            *item = f32::from_le_bytes(self.array()?);
        }
        Ok(v)
    }
    fn variable1(&mut self) -> Decoded<&'a [u8]> {
        let len = usize::from(self.u8()?);
        self.bytes(len)
    }
    fn variable2(&mut self) -> Decoded<&'a [u8]> {
        let len = usize::from(self.u16()?);
        self.bytes(len)
    }
}

/// Text fields carry a trailing NUL and are not guaranteed to be UTF-8.
fn text(data: &[u8]) -> String {
    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapBlock {
    /// Grid coordinates in region units (metres / 256).
    pub x: u16,
    pub y: u16,
    pub name: String,
    pub access: u8,
}

/// A rotation as a unit quaternion `[x, y, z, w]`.
pub type Quaternion = [f32; 4];

/// Where an object is and how it is turned: in the region, or relative to
/// its parent when it has one (a child prim, or a seated avatar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Vector3,
    pub rotation: Quaternion,
}

impl Transform {
    /// `None` for anything that is not a usable position and rotation: a
    /// wrong offset tends to produce such values.
    fn checked(position: Vector3, rotation: Quaternion) -> Option<Transform> {
        let norm = rotation.iter().map(|part| part * part).sum::<f32>().sqrt();
        let usable = position.iter().all(|part| part.is_finite())
            && norm.is_finite()
            && norm > 0.5
            && norm < 2.0;
        usable.then(|| Transform {
            position,
            rotation: rotation.map(|part| part / norm),
        })
    }
}

/// `v` turned by the unit quaternion `q`.
pub fn rotate(q: Quaternion, v: Vector3) -> Vector3 {
    let cross = |a: Vector3, b: Vector3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let axis = [q[0], q[1], q[2]];
    let once = cross(axis, v);
    let twice = cross(axis, once);
    [0, 1, 2].map(|i| v[i] + 2.0 * (q[3] * once[i] + twice[i]))
}

/// One object from an ObjectUpdate, reduced to what seat and position
/// tracking need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectParent {
    pub full_id: Uuid,
    /// Region-local id, which is what the simulator takes touches by.
    pub local_id: u32,
    /// Region-local id of the parent; zero when the object has none.
    pub parent_id: u32,
    /// Missing when the update carries it in a form the bot does not read.
    pub transform: Option<Transform>,
}

/// One object from an ObjectUpdateCompressed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressedObject {
    pub full_id: Uuid,
    pub local_id: u32,
    pub transform: Option<Transform>,
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    StartPingCheck {
        ping_id: u8,
    },
    PacketAck(Vec<u32>),
    RegionHandshake {
        sim_name: String,
    },
    AgentMovementComplete {
        position: Vector3,
        region_handle: u64,
    },
    MapBlockReply(Vec<MapBlock>),
    TeleportStart,
    TeleportProgress {
        message: String,
    },
    TeleportFailed {
        reason: String,
    },
    TeleportLocal {
        position: Vector3,
    },
    AvatarSitResponse {
        object: Uuid,
        autopilot: bool,
    },
    ObjectUpdate(Vec<ObjectParent>),
    /// Objects described in the compressed form.
    ObjectsCompressed(Vec<CompressedObject>),
    /// Objects that moved, as region-local id and where they are now.
    ObjectsMoved(Vec<(u32, Transform)>),
    /// Region-local ids of objects the simulator assumes to be cached.
    ObjectsCached(Vec<u32>),
    /// Region-local ids of objects that are gone or out of view.
    ObjectsKilled(Vec<u32>),
    KickUser {
        reason: String,
    },
    /// Text the simulator wants shown to the user.
    Alert {
        message: String,
    },
    /// Local chat, including typing notifications (empty text).
    Chat {
        from_name: String,
        /// The speaking avatar or object.
        source_id: Uuid,
        /// The object's owner; the avatar itself for avatar chat.
        owner_id: Uuid,
        source_type: u8,
        chat_type: u8,
        /// -1 not audible, 0 barely, 1 fully.
        audible: i8,
        text: String,
    },
    InstantMessage {
        /// The sending avatar; for an object's IM, the object's owner.
        from_agent: Uuid,
        from_name: String,
        dialog: u8,
        /// Meaning depends on `dialog`; the object for an object's IM.
        id: Uuid,
        /// The message was stored while the avatar was offline.
        offline: bool,
        /// When a stored message was sent, in seconds since the Unix epoch.
        timestamp: u32,
        text: String,
    },
    /// The answer to an avatar search: the avatars found, as id and
    /// `First Last`.
    AvatarPickerReply {
        query: Uuid,
        avatars: Vec<(Uuid, String)>,
    },
    LogoutReply,
    DisableSimulator,
    /// A message the bot has no use for.
    Other,
}

pub fn decode(id: MessageId, body: &[u8]) -> Decoded<Incoming> {
    let mut r = Reader(body);
    Ok(match id {
        // StartPingCheck, High 1: PingID U8, OldestUnacked U32.
        START_PING_CHECK => Incoming::StartPingCheck { ping_id: r.u8()? },
        // PacketAck, Fixed: Packets Variable { ID U32 }.
        PACKET_ACK => {
            let count = r.u8()?;
            Incoming::PacketAck((0..count).map(|_| r.u32()).collect::<Decoded<_>>()?)
        }
        // RegionHandshake, Low 148: RegionFlags U32, SimAccess U8, SimName Variable 1, ...
        REGION_HANDSHAKE => {
            r.skip(5)?;
            Incoming::RegionHandshake {
                sim_name: text(r.variable1()?),
            }
        }
        // AgentMovementComplete, Low 250: AgentID, SessionID, Position,
        // LookAt, RegionHandle U64, Timestamp U32, ...
        AGENT_MOVEMENT_COMPLETE => {
            r.skip(32)?;
            let position = r.vector3()?;
            r.skip(12)?;
            Incoming::AgentMovementComplete {
                position,
                region_handle: r.u64()?,
            }
        }
        // MapBlockReply, Low 409: AgentID, Flags U32, then Data Variable
        // { X U16, Y U16, Name Variable 1, Access U8, RegionFlags U32,
        //   WaterHeight U8, Agents U8, MapImageID LLUUID }.
        MAP_BLOCK_REPLY => {
            r.skip(20)?;
            let count = r.u8()?;
            let mut blocks = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let (x, y) = (r.u16()?, r.u16()?);
                let name = text(r.variable1()?);
                let access = r.u8()?;
                r.skip(4 + 1 + 1 + 16)?;
                blocks.push(MapBlock { x, y, name, access });
            }
            Incoming::MapBlockReply(blocks)
        }
        // TeleportStart, Low 73: TeleportFlags U32.
        TELEPORT_START => Incoming::TeleportStart,
        // TeleportProgress, Low 66: AgentID, TeleportFlags U32, Message Variable 1.
        TELEPORT_PROGRESS => {
            r.skip(20)?;
            Incoming::TeleportProgress {
                message: text(r.variable1()?),
            }
        }
        // TeleportFailed, Low 74: AgentID, Reason Variable 1, ...
        TELEPORT_FAILED => {
            r.skip(16)?;
            Incoming::TeleportFailed {
                reason: text(r.variable1()?),
            }
        }
        // TeleportLocal, Low 64: AgentID, LocationID U32, Position, LookAt, TeleportFlags U32.
        TELEPORT_LOCAL => {
            r.skip(20)?;
            Incoming::TeleportLocal {
                position: r.vector3()?,
            }
        }
        // AvatarSitResponse, High 21: ID LLUUID, AutoPilot BOOL, ...
        AVATAR_SIT_RESPONSE => Incoming::AvatarSitResponse {
            object: r.uuid()?,
            autopilot: r.u8()? != 0,
        },
        OBJECT_UPDATE => Incoming::ObjectUpdate(decode_object_update(&mut r)?),
        // ObjectUpdateCompressed, High 13: RegionHandle U64, TimeDilation
        // U16, then ObjectData Variable { UpdateFlags U32, Data Variable 2 }.
        // The data is FullID LLUUID, LocalID U32, PCode U8
        // (llviewerobjectlist.cpp:538-540), then State U8, CRC U32,
        // Material U8, ClickAction U8, Scale LLVector3, Position LLVector3,
        // Rotation LLVector3 (llviewerobject.cpp:1719, 1778-1795), so the
        // position is at offset 40 and the rotation at 52.
        OBJECT_UPDATE_COMPRESSED => {
            r.skip(8 + 2)?;
            let count = r.u8()?;
            let mut objects = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                r.skip(4)?;
                let mut data = Reader(r.variable2()?);
                let (full_id, local_id) = (data.uuid()?, data.u32()?);
                // Whatever is too short to hold them describes the object
                // all the same.
                let transform = data
                    .skip(20)
                    .and_then(|()| Ok((data.vector3()?, data.vector3()?)))
                    .ok();
                let transform = transform.and_then(|(position, rotation)| {
                    Transform::checked(position, unpack(rotation))
                });
                objects.push(CompressedObject {
                    full_id,
                    local_id,
                    transform,
                });
            }
            Incoming::ObjectsCompressed(objects)
        }
        // ImprovedTerseObjectUpdate, High 15: RegionHandle U64, TimeDilation
        // U16, then ObjectData Variable { Data Variable 1, TextureEntry
        // Variable 2 } (message_template.msg:3393). The data is LocalID U32
        // (llviewerobjectlist.cpp:563), State U8, a U8 that is non-zero for
        // an avatar and then followed by a 16-byte collision plane, Position
        // LLVector3, velocity and acceleration as U16 x 3 each, the rotation
        // as U16 x 4 mapped to -1..1 in the order x, y, z, w, and the
        // angular velocity as U16 x 3 (llviewerobject.cpp:1719-1765; written
        // by OpenSim's CreateImprovedTerseBlock, LLClientView.cs:6749).
        IMPROVED_TERSE_OBJECT_UPDATE => {
            r.skip(8 + 2)?;
            let count = r.u8()?;
            let mut objects = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let mut data = Reader(r.variable1()?);
                r.variable2()?; // TextureEntry
                let local_id = data.u32()?;
                data.skip(1)?; // State
                if data.u8()? != 0 {
                    data.skip(16)?; // collision plane
                }
                let position = data.vector3()?;
                data.skip(6 + 6)?; // velocity, acceleration
                let mut rotation = [0.0; 4];
                for part in &mut rotation {
                    *part = f32::from(data.u16()?) / f32::from(u16::MAX) * 2.0 - 1.0;
                }
                if let Some(transform) = Transform::checked(position, rotation) {
                    objects.push((local_id, transform));
                }
            }
            Incoming::ObjectsMoved(objects)
        }
        // ObjectUpdateCached, High 14: RegionHandle U64, TimeDilation U16,
        // then ObjectData Variable { ID U32, CRC U32, UpdateFlags U32 }.
        OBJECT_UPDATE_CACHED => {
            r.skip(8 + 2)?;
            let count = r.u8()?;
            let mut ids = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                ids.push(r.u32()?);
                r.skip(4 + 4)?;
            }
            Incoming::ObjectsCached(ids)
        }
        // KillObject, High 16: ObjectData Variable { ID U32 }.
        KILL_OBJECT => {
            let count = r.u8()?;
            Incoming::ObjectsKilled((0..count).map(|_| r.u32()).collect::<Decoded<_>>()?)
        }
        // KickUser, Low 163: TargetIP IPADDR, TargetPort IPPORT, AgentID,
        // SessionID, Reason Variable 2.
        KICK_USER => {
            r.skip(4 + 2 + 32)?;
            Incoming::KickUser {
                reason: text(r.variable2()?),
            }
        }
        // AlertMessage, Low 134: Message Variable 1, ...
        ALERT_MESSAGE => Incoming::Alert {
            message: text(r.variable1()?),
        },
        // AgentAlertMessage, Low 135: AgentID, Modal BOOL, Message Variable 1.
        AGENT_ALERT_MESSAGE => {
            r.skip(16 + 1)?;
            Incoming::Alert {
                message: text(r.variable1()?),
            }
        }
        // ChatFromSimulator, Low 139: FromName Variable 1, SourceID, OwnerID,
        // SourceType U8, ChatType U8, Audible U8, Position, Message Variable 2.
        CHAT_FROM_SIMULATOR => {
            let from_name = text(r.variable1()?);
            let (source_id, owner_id) = (r.uuid()?, r.uuid()?);
            let (source_type, chat_type, audible) = (r.u8()?, r.u8()?, r.u8()? as i8);
            r.skip(12)?;
            Incoming::Chat {
                from_name,
                source_id,
                owner_id,
                source_type,
                chat_type,
                audible,
                text: text(r.variable2()?),
            }
        }
        // ImprovedInstantMessage, Low 254: AgentID, SessionID, FromGroup BOOL,
        // ToAgentID, ParentEstateID U32, RegionID, Position, Offline U8,
        // Dialog U8, ID, Timestamp U32, FromAgentName Variable 1,
        // Message Variable 2, BinaryBucket Variable 2, ...
        IMPROVED_INSTANT_MESSAGE => {
            let from_agent = r.uuid()?;
            r.skip(16 + 1 + 16 + 4 + 16 + 12)?;
            let (offline, dialog) = (r.u8()? != 0, r.u8()?);
            let (id, timestamp) = (r.uuid()?, r.u32()?);
            let from_name = text(r.variable1()?);
            Incoming::InstantMessage {
                from_agent,
                from_name,
                dialog,
                id,
                offline,
                timestamp,
                text: text(r.variable2()?),
            }
        }
        // AvatarPickerReply, Low 28: AgentID, QueryID, then Data Variable
        // { AvatarID, FirstName Variable 1, LastName Variable 1 }.
        AVATAR_PICKER_REPLY => {
            r.skip(16)?;
            let query = r.uuid()?;
            let count = r.u8()?;
            let mut avatars = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let id = r.uuid()?;
                let (first, last) = (text(r.variable1()?), text(r.variable1()?));
                avatars.push((id, format!("{first} {last}").trim().to_string()));
            }
            Incoming::AvatarPickerReply { query, avatars }
        }
        LOGOUT_REPLY => Incoming::LogoutReply,
        DISABLE_SIMULATOR => Incoming::DisableSimulator,
        _ => Incoming::Other,
    })
}

/// A rotation sent as the x, y and z of a unit quaternion whose w is not
/// negative (LLQuaternion::unpackFromVector3, llquaternion.cpp:943).
fn unpack(rotation: Vector3) -> Quaternion {
    let [x, y, z] = rotation;
    [x, y, z, (1.0 - (x * x + y * y + z * z)).max(0.0).sqrt()]
}

/// The motion in the ObjectData field of a full update, in the two layouts
/// simulators send (llviewerobject.cpp:1386-1420): 60 bytes of Position,
/// Velocity, Acceleration, Rotation and AngularVelocity, each an
/// LLVector3; or 76 bytes for an avatar, the same after a 16-byte collision
/// plane. 124 and 140 bytes are the same with room for later additions.
/// The quantized 48, 32 and 16-byte forms are not sent any more and not
/// read here.
fn transform_from_motion(data: &[u8]) -> Option<Transform> {
    let offset = match data.len() {
        60 | 124 => 0,
        76 | 140 => 16,
        _ => return None,
    };
    let mut r = Reader(&data[offset..]);
    let position = r.vector3().ok()?;
    r.skip(12 + 12).ok()?;
    Transform::checked(position, unpack(r.vector3().ok()?))
}

/// ObjectUpdate, High 12: RegionHandle U64, TimeDilation U16, then
/// ObjectData Variable. Each block is read up to ParentID and the rest is
/// skipped by its length prefixes to reach the next block.
fn decode_object_update(r: &mut Reader) -> Decoded<Vec<ObjectParent>> {
    r.skip(8 + 2)?;
    let count = r.u8()?;
    let mut objects = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let local_id = r.u32()?;
        r.skip(1)?; // State
        let full_id = r.uuid()?;
        r.skip(4 + 1 + 1 + 1 + 12)?; // CRC, PCode, Material, ClickAction, Scale
        let transform = transform_from_motion(r.variable1()?); // ObjectData
        let parent_id = r.u32()?;
        r.skip(4)?; // UpdateFlags
        r.skip(23)?; // PathCurve .. ProfileHollow
        r.variable2()?; // TextureEntry
        r.variable1()?; // TextureAnim
        r.variable2()?; // NameValue
        r.variable2()?; // Data
        r.variable1()?; // Text
        r.skip(4)?; // TextColor
        r.variable1()?; // MediaURL
        r.variable1()?; // PSBlock
        r.variable1()?; // ExtraParams
                        // Sound, OwnerID, Gain, Flags, Radius, JointType, JointPivot, JointAxisOrAnchor
        r.skip(16 + 16 + 4 + 1 + 4 + 1 + 12 + 12)?;
        objects.push(ObjectParent {
            full_id,
            local_id,
            parent_id,
            transform,
        });
    }
    Ok(objects)
}

#[cfg(test)]
pub mod testing {
    //! Builders for incoming message bodies, laid out field by field from the
    //! template. Shared with the session tests.
    use super::*;

    fn string1(text: &str) -> Vec<u8> {
        let mut out = vec![text.len() as u8 + 1];
        out.extend(text.as_bytes());
        out.push(0);
        out
    }

    fn vector(v: Vector3) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    pub fn region_handshake(sim_name: &str) -> Vec<u8> {
        let mut body = vec![0; 5]; // RegionFlags, SimAccess
        body.extend(string1(sim_name));
        body.extend([0; 16 + 1 + 4 + 4 + 16 + 8 * 16 + 8 * 4]); // SimOwner .. TerrainHeightRange11
        body.extend([0; 16]); // RegionInfo2
        body
    }

    pub fn agent_movement_complete(agent: &Agent, position: Vector3, handle: u64) -> Vec<u8> {
        let mut body = agent.agent_id.as_bytes().to_vec();
        body.extend(agent.session_id.as_bytes());
        body.extend(vector(position));
        body.extend(vector([1.0, 0.0, 0.0]));
        body.extend(handle.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend([1, 0, 0]); // ChannelVersion, Variable 2
        body
    }

    pub fn map_block_reply(blocks: &[(u16, u16, &str, u8)]) -> Vec<u8> {
        let mut body = vec![0; 20];
        body.push(blocks.len() as u8);
        for (x, y, name, access) in blocks {
            body.extend(x.to_le_bytes());
            body.extend(y.to_le_bytes());
            body.extend(string1(name));
            body.push(*access);
            body.extend([0; 4 + 1 + 1 + 16]);
        }
        body.push(0); // Size block count
        body
    }

    pub fn teleport_failed(reason: &str) -> Vec<u8> {
        let mut body = vec![0; 16];
        body.extend(string1(reason));
        body.push(0); // AlertInfo block count
        body
    }

    pub fn teleport_progress(message: &str) -> Vec<u8> {
        let mut body = vec![0; 20];
        body.extend(string1(message));
        body
    }

    pub fn teleport_local(position: Vector3) -> Vec<u8> {
        let mut body = vec![0; 20];
        body.extend(vector(position));
        body.extend(vector([1.0, 0.0, 0.0]));
        body.extend(0u32.to_le_bytes());
        body
    }

    pub fn avatar_sit_response(object: Uuid, autopilot: bool) -> Vec<u8> {
        let mut body = object.as_bytes().to_vec();
        body.push(u8::from(autopilot));
        body.extend([0; 12 + 12 + 12 + 12 + 1]);
        body
    }

    pub fn alert_message(message: &str) -> Vec<u8> {
        let mut body = string1(message);
        body.push(0); // AlertInfo block count
        body
    }

    pub fn agent_alert_message(message: &str) -> Vec<u8> {
        let mut body = vec![0; 16 + 1];
        body.extend(string1(message));
        body
    }

    pub fn kick_user(reason: &str) -> Vec<u8> {
        let mut body = vec![0; 4 + 2 + 32];
        body.extend((reason.len() as u16).to_le_bytes());
        body.extend(reason.as_bytes());
        body
    }

    fn string2(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u16 + 1).to_le_bytes().to_vec();
        out.extend(text.as_bytes());
        out.push(0);
        out
    }

    /// `source` is (SourceID, OwnerID, SourceType).
    pub fn chat_from_simulator(
        from_name: &str,
        source: (Uuid, Uuid, u8),
        chat_type: u8,
        audible: i8,
        message: &str,
    ) -> Vec<u8> {
        let mut body = string1(from_name);
        body.extend(source.0.as_bytes());
        body.extend(source.1.as_bytes());
        body.extend([source.2, chat_type, audible as u8]);
        body.extend(vector([1.0, 2.0, 3.0]));
        body.extend(string2(message));
        body
    }

    /// `stored` is the Timestamp of a message kept while offline.
    pub fn improved_instant_message(
        from_agent: Uuid,
        from_name: &str,
        dialog: u8,
        id: Uuid,
        stored: Option<u32>,
        message: &str,
    ) -> Vec<u8> {
        let mut body = from_agent.as_bytes().to_vec();
        body.extend([0; 16 + 1 + 16 + 4 + 16 + 12]); // SessionID .. Position
        body.extend([u8::from(stored.is_some()), dialog]);
        body.extend(id.as_bytes());
        body.extend(stored.unwrap_or(0).to_le_bytes());
        body.extend(string1(from_name));
        body.extend(string2(message));
        body.extend([1, 0, 0]); // BinaryBucket
        body.extend([0; 4]); // EstateID
        body.push(0); // MetaData block count
        body
    }

    pub fn avatar_picker_reply(query: Uuid, avatars: &[(Uuid, &str, &str)]) -> Vec<u8> {
        let mut body = vec![0; 16]; // AgentID
        body.extend(query.as_bytes());
        body.push(avatars.len() as u8);
        for (id, first, last) in avatars {
            body.extend(id.as_bytes());
            body.extend(string1(first));
            body.extend(string1(last));
        }
        body
    }

    /// One object's transform in an ImprovedTerseObjectUpdate.
    pub struct Terse {
        pub local_id: u32,
        pub avatar: bool,
        pub position: Vector3,
        pub rotation: Quaternion,
    }

    fn quantized(value: f32) -> [u8; 2] {
        (((value + 1.0) / 2.0 * f32::from(u16::MAX)).round() as u16).to_le_bytes()
    }

    pub fn improved_terse_object_update(objects: &[Terse]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(objects.len() as u8);
        for (index, object) in objects.iter().enumerate() {
            let mut data = object.local_id.to_le_bytes().to_vec();
            data.push(0); // State
            data.push(u8::from(object.avatar));
            if object.avatar {
                data.extend([0x5A; 16]); // collision plane
            }
            data.extend(vector(object.position));
            data.extend([0x6B; 6 + 6]); // velocity, acceleration
            data.extend(object.rotation.iter().flat_map(|part| quantized(*part)));
            data.extend([0x7C; 6]); // angular velocity
            body.push(data.len() as u8);
            body.extend(data);
            body.extend((index as u16 * 5).to_le_bytes()); // TextureEntry
            body.extend(vec![0x8D; index * 5]);
        }
        body
    }

    /// Blocks are (FullID, local ID, position, rotation as x, y, z).
    pub fn object_update_compressed_at(objects: &[(Uuid, u32, Vector3, Vector3)]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(objects.len() as u8);
        for (full_id, local_id, position, rotation) in objects {
            body.extend([0xEF; 4]); // UpdateFlags
            body.extend(80u16.to_le_bytes());
            body.extend(full_id.as_bytes());
            body.extend(local_id.to_le_bytes());
            body.extend([0x99; 1 + 1 + 4 + 1 + 1 + 12]); // PCode .. Scale
            body.extend(vector(*position));
            body.extend(vector(*rotation));
            body.extend([0x99; 16]); // what follows
        }
        body
    }

    /// Blocks are (FullID, local ID); the rest of each block's data is
    /// filler, too short to hold a position.
    pub fn object_update_compressed(objects: &[(Uuid, u32)]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(objects.len() as u8);
        for (full_id, local_id) in objects {
            body.extend([0xEF; 4]); // UpdateFlags
            body.extend(50u16.to_le_bytes());
            body.extend(full_id.as_bytes());
            body.extend(local_id.to_le_bytes());
            body.extend([0x99; 30]); // PCode and what follows
        }
        body
    }

    pub fn object_update_cached(local_ids: &[u32]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(local_ids.len() as u8);
        for id in local_ids {
            body.extend(id.to_le_bytes());
            body.extend([0xCC; 4 + 4]); // CRC, UpdateFlags
        }
        body
    }

    pub fn kill_object(local_ids: &[u32]) -> Vec<u8> {
        let mut body = vec![local_ids.len() as u8];
        body.extend(local_ids.iter().flat_map(|id| id.to_le_bytes()));
        body
    }

    /// One block of an ObjectUpdate.
    pub struct Block {
        pub full_id: Uuid,
        pub local_id: u32,
        pub parent_id: u32,
        /// The ObjectData field.
        pub data: Vec<u8>,
    }

    /// An ObjectData field in full precision: 60 bytes, or 76 for an
    /// avatar. The rotation is the x, y and z of a unit quaternion.
    pub fn object_data(position: Vector3, rotation: Vector3, avatar: bool) -> Vec<u8> {
        let mut data = if avatar { vec![0x5A; 16] } else { Vec::new() };
        data.extend(vector(position));
        data.extend([0x6B; 12 + 12]); // velocity, acceleration
        data.extend(vector(rotation));
        data.extend([0x7C; 12]); // angular velocity
        data
    }

    /// Blocks are (FullID, ParentID), with local ids counting up from 100
    /// and an ObjectData field that carries no motion the bot reads.
    pub fn object_update(objects: &[(Uuid, u32)]) -> Vec<u8> {
        let blocks: Vec<Block> = objects
            .iter()
            .enumerate()
            .map(|(index, (full_id, parent_id))| Block {
                full_id: *full_id,
                local_id: index as u32 + 100,
                parent_id: *parent_id,
                data: vec![0xCD; 32],
            })
            .collect();
        object_blocks(&blocks)
    }

    /// Variable fields get assorted lengths so a wrong skip misaligns the
    /// next block.
    pub fn object_blocks(objects: &[Block]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(objects.len() as u8);
        for (
            index,
            Block {
                full_id,
                local_id,
                parent_id,
                data,
            },
        ) in objects.iter().enumerate()
        {
            let pad = index + 1;
            body.extend(local_id.to_le_bytes()); // ID
            body.push(0); // State
            body.extend(full_id.as_bytes());
            body.extend([0xAB; 4 + 1 + 1 + 1 + 12]);
            body.push(data.len() as u8); // ObjectData
            body.extend(data);
            body.extend(parent_id.to_le_bytes());
            body.extend([0xEF; 4]); // UpdateFlags
            body.extend([0x11; 23]); // PathCurve .. ProfileHollow
            body.extend((pad as u16 * 7).to_le_bytes()); // TextureEntry
            body.extend(vec![0x22; pad * 7]);
            body.push(0); // TextureAnim
            body.extend((pad as u16 * 20).to_le_bytes()); // NameValue
            body.extend(vec![0x33; pad * 20]);
            body.extend(0u16.to_le_bytes()); // Data
            body.push(pad as u8); // Text
            body.extend(vec![0x44; pad]);
            body.extend([0x55; 4]); // TextColor
            body.push(0); // MediaURL
            body.push(3); // PSBlock
            body.extend([0x66; 3]);
            body.push(0); // ExtraParams
            body.extend([0x77; 16 + 16 + 4 + 1 + 4 + 1 + 12 + 12]);
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn agent() -> Agent {
        Agent {
            agent_id: Uuid::from_bytes([0xA1; 16]),
            session_id: Uuid::from_bytes([0x5E; 16]),
        }
    }

    fn agent_prefix() -> Vec<u8> {
        [[0xA1; 16], [0x5E; 16]].concat()
    }

    #[test]
    fn encodes_use_circuit_code() {
        let msg = use_circuit_code(&agent(), 0x0403_0201);
        assert_eq!((msg.id, msg.zerocoded), (low(3), false));
        // Code, SessionID, ID: the session id comes before the agent id here.
        assert_eq!(
            msg.body,
            [vec![1, 2, 3, 4], vec![0x5E; 16], vec![0xA1; 16]].concat()
        );
    }

    #[test]
    fn encodes_complete_agent_movement() {
        let msg = complete_agent_movement(&agent(), 0x0403_0201);
        assert_eq!((msg.id, msg.zerocoded), (low(249), false));
        assert_eq!(msg.body, [agent_prefix(), vec![1, 2, 3, 4]].concat());
    }

    #[test]
    fn encodes_region_handshake_reply() {
        let msg = region_handshake_reply(&agent());
        assert_eq!((msg.id, msg.zerocoded), (low(149), true));
        assert_eq!(msg.body, [agent_prefix(), vec![0; 4]].concat());
    }

    #[test]
    fn encodes_agent_throttle() {
        let msg = agent_throttle(&agent(), 7, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!((msg.id, msg.zerocoded), (low(81), true));
        assert_eq!(msg.body.len(), 32 + 4 + 4 + 1 + 28);
        assert_eq!(msg.body[32..41], [7, 0, 0, 0, 0, 0, 0, 0, 28]);
        assert_eq!(msg.body[41..45], 1.0f32.to_le_bytes());
        assert_eq!(msg.body[65..], 7.0f32.to_le_bytes());
    }

    #[test]
    fn encodes_agent_update() {
        let msg = agent_update(&agent(), [10.0, 20.0, 30.0], 32.0, 0);
        assert_eq!((msg.id, msg.zerocoded), (high(4), true));
        // AgentID, SessionID, two quaternions, State, four vectors, Far, ControlFlags, Flags.
        assert_eq!(msg.body.len(), 32 + 12 + 12 + 1 + 4 * 12 + 4 + 4 + 1);
        assert_eq!(msg.body[32..57], [0; 25]);
        assert_eq!(msg.body[57..61], 10.0f32.to_le_bytes());
        assert_eq!(msg.body[105..109], 32.0f32.to_le_bytes());
        assert_eq!(msg.body[109..], [0; 5]);
        // The control flags sit between the draw distance and the flags.
        let msg = agent_update(&agent(), [10.0, 20.0, 30.0], 32.0, CONTROL_STAND_UP);
        assert_eq!(msg.body[109..], [0, 0, 1, 0, 0]);
        let msg = agent_update(&agent(), [10.0, 20.0, 30.0], 32.0, CONTROL_SIT_ON_GROUND);
        assert_eq!(msg.body[109..], [0, 0, 2, 0, 0]);
    }

    #[test]
    fn encodes_ping_and_ack() {
        let ping = complete_ping_check(9);
        assert_eq!((ping.id, ping.body.as_slice()), (high(2), &[9u8][..]));
        let ack = packet_ack(&[1, 0x0102_0304]);
        assert_eq!(ack.id, 0xFFFF_FFFB);
        assert_eq!(ack.body, [2, 1, 0, 0, 0, 4, 3, 2, 1]);
    }

    #[test]
    fn encodes_logout_request() {
        let msg = logout_request(&agent());
        assert_eq!((msg.id, msg.body), (low(252), agent_prefix()));
    }

    #[test]
    fn encodes_map_name_request() {
        let msg = map_name_request(&agent(), "Target");
        assert_eq!(msg.id, low(408));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![0; 9], vec![7], b"Target\0".to_vec()].concat()
        );
    }

    #[test]
    fn encodes_chat_from_viewer() {
        let msg = chat_from_viewer(&agent(), "hi", 2, 0x0102_0304);
        assert_eq!((msg.id, msg.zerocoded), (low(80), true));
        assert_eq!(
            msg.body,
            [
                agent_prefix(),
                vec![3, 0],
                b"hi\0".to_vec(),
                vec![2, 4, 3, 2, 1]
            ]
            .concat()
        );
    }

    #[test]
    fn encodes_instant_message() {
        let to = Uuid::from_bytes([0x0F; 16]);
        let msg = instant_message(&agent(), "Sit Bot", to, [1.0, 2.0, 3.0], IM_PLAIN, "hello");
        assert_eq!((msg.id, msg.zerocoded), (low(254), true));
        let expected = [
            agent_prefix(),
            vec![0],         // FromGroup
            vec![0x0F; 16],  // ToAgentID
            vec![0; 4 + 16], // ParentEstateID, RegionID
            [1.0f32, 2.0, 3.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect(),
            vec![0, 0],            // Offline, Dialog
            vec![0xA1 ^ 0x0F; 16], // ID
            vec![0; 4],            // Timestamp
            vec![8],
            b"Sit Bot\0".to_vec(),
            vec![6, 0],
            b"hello\0".to_vec(),
            vec![1, 0, 0],       // BinaryBucket
            vec![0, 0, 0, 0, 0], // EstateID, MetaData block count
        ]
        .concat();
        assert_eq!(msg.body, expected);
        // What goes out is what the decoder reads as an incoming message.
        let Ok(Incoming::InstantMessage {
            from_agent,
            from_name,
            dialog,
            text,
            ..
        }) = decode(msg.id, &msg.body)
        else {
            panic!("not an instant message")
        };
        assert_eq!(
            (from_agent, from_name.as_str(), dialog, text.as_str()),
            (agent().agent_id, "Sit Bot", 0, "hello")
        );
    }

    #[test]
    fn teleport_request_is_an_instant_message_without_a_conversation() {
        let to = Uuid::from_bytes([0x0F; 16]);
        let msg = instant_message(
            &agent(),
            "Sit Bot",
            to,
            [0.0; 3],
            IM_TELEPORT_REQUEST,
            "may I?",
        );
        let Ok(Incoming::InstantMessage {
            dialog, id, text, ..
        }) = decode(msg.id, &msg.body)
        else {
            panic!("not an instant message")
        };
        assert_eq!((dialog, id, text.as_str()), (26, Uuid::nil(), "may I?"));
    }

    #[test]
    fn encodes_teleport_offers() {
        let to = Uuid::from_bytes([0x0F; 16]);
        let msg = start_lure(&agent(), "come", to);
        assert_eq!((msg.id, msg.zerocoded), (low(70), false));
        assert_eq!(
            msg.body,
            [
                agent_prefix(),
                vec![0, 5],
                b"come\0".to_vec(),
                vec![1],
                vec![0x0F; 16]
            ]
            .concat()
        );

        let msg = teleport_lure_request(&agent(), Uuid::from_bytes([0x44; 16]));
        assert_eq!((msg.id, msg.zerocoded), (low(71), false));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![0x44; 16], vec![4, 0, 0, 0]].concat()
        );
    }

    #[test]
    fn encodes_a_touch() {
        let msg = object_grab(&agent(), 0x0403_0201);
        assert_eq!((msg.id, msg.zerocoded), (low(117), true));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![1, 2, 3, 4], vec![0; 12], vec![0]].concat()
        );
        let msg = object_degrab(&agent(), 0x0403_0201);
        assert_eq!((msg.id, msg.zerocoded), (low(119), false));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![1, 2, 3, 4], vec![0]].concat()
        );
    }

    #[test]
    fn encodes_request_multiple_objects() {
        let msg = request_multiple_objects(&agent(), &[7, 0x0102]);
        assert_eq!((msg.id, msg.zerocoded), (medium(3), true));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![2, 0, 7, 0, 0, 0, 0, 2, 1, 0, 0]].concat()
        );
    }

    #[test]
    fn decodes_the_other_object_messages() {
        let (a, b) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let body = object_update_compressed(&[(a, 500), (b, 501)]);
        let plain = |full_id, local_id| CompressedObject {
            full_id,
            local_id,
            transform: None,
        };
        assert_eq!(
            decode(high(13), &body),
            Ok(Incoming::ObjectsCompressed(vec![
                plain(a, 500),
                plain(b, 501)
            ]))
        );
        assert_eq!(decode(high(13), &body[..body.len() - 40]), Err(DecodeError));

        let body = object_update_cached(&[500, 501]);
        assert_eq!(
            decode(high(14), &body),
            Ok(Incoming::ObjectsCached(vec![500, 501]))
        );
        assert_eq!(decode(high(14), &body[..body.len() - 1]), Err(DecodeError));

        let body = kill_object(&[500, 501]);
        assert_eq!(
            decode(high(16), &body),
            Ok(Incoming::ObjectsKilled(vec![500, 501]))
        );
        assert_eq!(decode(high(16), &body[..body.len() - 1]), Err(DecodeError));
    }

    #[test]
    fn encodes_avatar_picker_request() {
        let query = Uuid::from_bytes([0x33; 16]);
        let msg = avatar_picker_request(&agent(), query, "jane doe");
        assert_eq!((msg.id, msg.zerocoded), (low(26), false));
        assert_eq!(
            msg.body,
            [
                agent_prefix(),
                vec![0x33; 16],
                vec![9],
                b"jane doe\0".to_vec()
            ]
            .concat()
        );
    }

    #[test]
    fn decodes_avatar_picker_reply() {
        let query = Uuid::from_bytes([0x33; 16]);
        let (jane, other) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let body = avatar_picker_reply(query, &[(jane, "Jane", "Doe"), (other, "Jane", "Doester")]);
        assert_eq!(
            decode(low(28), &body),
            Ok(Incoming::AvatarPickerReply {
                query,
                avatars: vec![
                    (jane, "Jane Doe".to_string()),
                    (other, "Jane Doester".to_string())
                ],
            })
        );
        assert_eq!(decode(low(28), &body[..body.len() - 1]), Err(DecodeError));
        let none = avatar_picker_reply(query, &[]);
        assert_eq!(
            decode(low(28), &none),
            Ok(Incoming::AvatarPickerReply {
                query,
                avatars: vec![]
            })
        );
    }

    #[test]
    fn encodes_teleport_location_request() {
        let msg = teleport_location_request(&agent(), 0x0003_E800_0003_E900, [1.0, 2.0, 3.0]);
        assert_eq!(msg.id, low(63));
        assert_eq!(msg.body.len(), 32 + 8 + 12 + 12);
        assert_eq!(msg.body[32..40], [0, 0xE9, 3, 0, 0, 0xE8, 3, 0]);
        assert_eq!(msg.body[40..44], 1.0f32.to_le_bytes());
    }

    #[test]
    fn encodes_sit_messages() {
        let target = Uuid::from_bytes([0x0B; 16]);
        let request = agent_request_sit(&agent(), target);
        assert_eq!((request.id, request.zerocoded), (high(6), true));
        assert_eq!(
            request.body,
            [agent_prefix(), vec![0x0B; 16], vec![0; 12]].concat()
        );
        let sit = agent_sit(&agent());
        assert_eq!((sit.id, sit.body), (high(7), agent_prefix()));
    }

    #[test]
    fn decodes_ping_and_acks() {
        assert_eq!(
            decode(high(1), &[7, 1, 0, 0, 0]),
            Ok(Incoming::StartPingCheck { ping_id: 7 })
        );
        assert_eq!(
            decode(PACKET_ACK, &[2, 5, 0, 0, 0, 0, 1, 0, 0]),
            Ok(Incoming::PacketAck(vec![5, 256]))
        );
        assert_eq!(decode(PACKET_ACK, &[2, 5, 0, 0, 0]), Err(DecodeError));
    }

    #[test]
    fn decodes_region_handshake() {
        assert_eq!(
            decode(low(148), &region_handshake("Target")),
            Ok(Incoming::RegionHandshake {
                sim_name: "Target".into()
            })
        );
    }

    #[test]
    fn decodes_agent_movement_complete() {
        let body = agent_movement_complete(&agent(), [1.0, 2.0, 3.0], 0x0003_E800_0003_E900);
        assert_eq!(
            decode(low(250), &body),
            Ok(Incoming::AgentMovementComplete {
                position: [1.0, 2.0, 3.0],
                region_handle: 0x0003_E800_0003_E900
            })
        );
    }

    #[test]
    fn decodes_map_block_reply() {
        let body = map_block_reply(&[(1000, 1001, "Target", 13), (0, 0, "Nowhere", 255)]);
        assert_eq!(
            decode(low(409), &body),
            Ok(Incoming::MapBlockReply(vec![
                MapBlock {
                    x: 1000,
                    y: 1001,
                    name: "Target".into(),
                    access: 13
                },
                MapBlock {
                    x: 0,
                    y: 0,
                    name: "Nowhere".into(),
                    access: 255
                },
            ]))
        );
    }

    #[test]
    fn decodes_teleport_messages() {
        assert_eq!(decode(low(73), &[0; 4]), Ok(Incoming::TeleportStart));
        assert_eq!(
            decode(low(66), &teleport_progress("resolving")),
            Ok(Incoming::TeleportProgress {
                message: "resolving".into()
            })
        );
        assert_eq!(
            decode(low(74), &teleport_failed("Destination offline")),
            Ok(Incoming::TeleportFailed {
                reason: "Destination offline".into()
            })
        );
        assert_eq!(
            decode(low(64), &teleport_local([4.0, 5.0, 6.0])),
            Ok(Incoming::TeleportLocal {
                position: [4.0, 5.0, 6.0]
            })
        );
    }

    #[test]
    fn decodes_avatar_sit_response() {
        let object = Uuid::from_bytes([0x0B; 16]);
        assert_eq!(
            decode(high(21), &avatar_sit_response(object, true)),
            Ok(Incoming::AvatarSitResponse {
                object,
                autopilot: true
            })
        );
    }

    #[test]
    fn decodes_kick_logout_and_disable() {
        assert_eq!(
            decode(low(163), &kick_user("Bye")),
            Ok(Incoming::KickUser {
                reason: "Bye".into()
            })
        );
        assert_eq!(decode(low(253), &[0; 33]), Ok(Incoming::LogoutReply));
        assert_eq!(decode(low(152), &[]), Ok(Incoming::DisableSimulator));
    }

    #[test]
    fn decodes_both_alert_messages() {
        let alert = Incoming::Alert {
            message: "No suitable surface".into(),
        };
        assert_eq!(
            decode(low(134), &alert_message("No suitable surface")),
            Ok(alert)
        );
        let alert = Incoming::Alert {
            message: "Region restarting".into(),
        };
        assert_eq!(
            decode(low(135), &agent_alert_message("Region restarting")),
            Ok(alert)
        );
    }

    #[test]
    fn encodes_retrieve_instant_messages() {
        let msg = retrieve_instant_messages(&agent());
        assert_eq!(
            (msg.id, msg.zerocoded, msg.body),
            (low(255), false, agent_prefix())
        );
    }

    #[test]
    fn decodes_chat_from_simulator() {
        let (avatar, object, owner) = (
            Uuid::from_bytes([1; 16]),
            Uuid::from_bytes([2; 16]),
            Uuid::from_bytes([3; 16]),
        );
        assert_eq!(
            decode(
                low(139),
                &chat_from_simulator("Jane Doe", (avatar, avatar, 1), 1, 1, "hello")
            ),
            Ok(Incoming::Chat {
                from_name: "Jane Doe".into(),
                source_id: avatar,
                owner_id: avatar,
                source_type: 1,
                chat_type: 1,
                audible: 1,
                text: "hello".into(),
            })
        );
        let body = chat_from_simulator("Clock", (object, owner, 2), 2, -1, "tick\ntock");
        assert_eq!(
            decode(low(139), &body),
            Ok(Incoming::Chat {
                from_name: "Clock".into(),
                source_id: object,
                owner_id: owner,
                source_type: 2,
                chat_type: 2,
                audible: -1,
                text: "tick\ntock".into(),
            })
        );
        assert_eq!(decode(low(139), &body[..body.len() - 1]), Err(DecodeError));
    }

    #[test]
    fn decodes_improved_instant_message() {
        let (sender, session, object) = (
            Uuid::from_bytes([1; 16]),
            Uuid::from_bytes([2; 16]),
            Uuid::from_bytes([3; 16]),
        );
        let im = |dialog, id, offline, timestamp, from_name: &str, text: &str| {
            Incoming::InstantMessage {
                from_agent: sender,
                from_name: from_name.into(),
                dialog,
                id,
                offline,
                timestamp,
                text: text.into(),
            }
        };
        assert_eq!(
            decode(
                low(254),
                &improved_instant_message(sender, "Jane Doe", 0, session, None, "hi")
            ),
            Ok(im(0, session, false, 0, "Jane Doe", "hi"))
        );
        assert_eq!(
            decode(
                low(254),
                &improved_instant_message(sender, "Clock", 19, object, None, "done")
            ),
            Ok(im(19, object, false, 0, "Clock", "done"))
        );
        let body =
            improved_instant_message(sender, "Jane Doe", 0, session, Some(1_700_000_000), "later");
        assert_eq!(
            decode(low(254), &body),
            Ok(im(0, session, true, 1_700_000_000, "Jane Doe", "later"))
        );
        assert_eq!(decode(low(254), &body[..80]), Err(DecodeError));
    }

    #[test]
    fn decodes_object_update_with_several_blocks() {
        let ids = [
            Uuid::from_bytes([1; 16]),
            agent().agent_id,
            Uuid::from_bytes([3; 16]),
        ];
        let body = object_update(&[(ids[0], 0), (ids[1], 4242), (ids[2], 77)]);
        assert_eq!(
            decode(high(12), &body),
            Ok(Incoming::ObjectUpdate(vec![
                ObjectParent {
                    full_id: ids[0],
                    local_id: 100,
                    parent_id: 0,
                    transform: None
                },
                ObjectParent {
                    full_id: ids[1],
                    local_id: 101,
                    parent_id: 4242,
                    transform: None
                },
                ObjectParent {
                    full_id: ids[2],
                    local_id: 102,
                    parent_id: 77,
                    transform: None
                },
            ]))
        );
    }

    /// A quarter turn about the vertical axis, as the x, y and z of its
    /// quaternion and in full.
    const QUARTER_TURN: (Vector3, Quaternion) = (
        [0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2],
        [
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ],
    );

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    #[test]
    fn object_update_carries_position_and_rotation() {
        let (prim, avatar, old) = (
            Uuid::from_bytes([1; 16]),
            Uuid::from_bytes([2; 16]),
            Uuid::from_bytes([3; 16]),
        );
        let body = object_blocks(&[
            Block {
                full_id: prim,
                local_id: 7,
                parent_id: 0,
                data: object_data([131.0, 64.0, 23.0], QUARTER_TURN.0, false),
            },
            Block {
                full_id: avatar,
                local_id: 8,
                parent_id: 7,
                data: object_data([0.5, 0.0, 0.75], [0.0; 3], true),
            },
            // A quantized form: described, but without a motion.
            Block {
                full_id: old,
                local_id: 9,
                parent_id: 0,
                data: vec![0x12; 32],
            },
        ]);
        let Ok(Incoming::ObjectUpdate(objects)) = decode(high(12), &body) else {
            panic!("not an object update")
        };
        assert_eq!(
            objects
                .iter()
                .map(|o| (o.full_id, o.local_id, o.parent_id))
                .collect::<Vec<_>>(),
            [(prim, 7, 0), (avatar, 8, 7), (old, 9, 0)]
        );
        let seat = objects[0].transform.unwrap();
        assert_eq!(seat.position, [131.0, 64.0, 23.0]);
        assert!(
            close(&seat.rotation, &QUARTER_TURN.1),
            "{:?}",
            seat.rotation
        );
        let rider = objects[1].transform.unwrap();
        assert_eq!(rider.position, [0.5, 0.0, 0.75]);
        assert!(close(&rider.rotation, &[0.0, 0.0, 0.0, 1.0]));
        assert_eq!(objects[2].transform, None);
    }

    #[test]
    fn object_data_of_the_longer_layouts_is_read_the_same() {
        let mut prim = object_data([1.0, 2.0, 3.0], [0.0; 3], false);
        prim.extend([0xEE; 64]);
        let mut avatar = object_data([4.0, 5.0, 6.0], [0.0; 3], true);
        avatar.extend([0xEE; 64]);
        assert_eq!((prim.len(), avatar.len()), (124, 140));
        assert_eq!(
            transform_from_motion(&prim).unwrap().position,
            [1.0, 2.0, 3.0]
        );
        assert_eq!(
            transform_from_motion(&avatar).unwrap().position,
            [4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn positions_that_are_not_numbers_are_dropped() {
        assert_eq!(
            transform_from_motion(&object_data([f32::NAN, 2.0, 3.0], [0.0; 3], false)),
            None
        );
        assert_eq!(
            transform_from_motion(&object_data([1.0, f32::INFINITY, 3.0], [0.0; 3], false)),
            None
        );
        assert_eq!(transform_from_motion(&[0xFF; 60]), None);
        assert_eq!(transform_from_motion(&[0; 61]), None);
    }

    #[test]
    fn compressed_update_carries_position_and_rotation() {
        let (a, b) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let body = object_update_compressed_at(&[
            (a, 500, [10.0, 20.0, 30.0], QUARTER_TURN.0),
            (b, 501, [1.0, 2.0, 3.0], [0.0; 3]),
        ]);
        let Ok(Incoming::ObjectsCompressed(objects)) = decode(high(13), &body) else {
            panic!("not compressed")
        };
        assert_eq!(
            objects
                .iter()
                .map(|o| (o.full_id, o.local_id))
                .collect::<Vec<_>>(),
            [(a, 500), (b, 501)]
        );
        assert_eq!(objects[0].transform.unwrap().position, [10.0, 20.0, 30.0]);
        assert!(close(
            &objects[0].transform.unwrap().rotation,
            &QUARTER_TURN.1
        ));
        assert_eq!(objects[1].transform.unwrap().position, [1.0, 2.0, 3.0]);
        // Cut inside the second block's data: an error, not a guess.
        assert_eq!(decode(high(13), &body[..body.len() - 20]), Err(DecodeError));
    }

    #[test]
    fn terse_update_carries_position_and_rotation() {
        let body = improved_terse_object_update(&[
            Terse {
                local_id: 7,
                avatar: false,
                position: [141.0, 64.0, 23.0],
                rotation: QUARTER_TURN.1,
            },
            Terse {
                local_id: 8,
                avatar: true,
                position: [0.5, 0.25, 0.75],
                rotation: [0.0, 0.0, 0.0, 1.0],
            },
        ]);
        let Ok(Incoming::ObjectsMoved(objects)) = decode(high(15), &body) else {
            panic!("not a terse update")
        };
        assert_eq!(
            objects.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [7, 8]
        );
        assert_eq!(objects[0].1.position, [141.0, 64.0, 23.0]);
        assert!(
            close(&objects[0].1.rotation, &QUARTER_TURN.1),
            "{:?}",
            objects[0].1.rotation
        );
        // The avatar's block is longer by its collision plane.
        assert_eq!(objects[1].1.position, [0.5, 0.25, 0.75]);
        assert!(close(&objects[1].1.rotation, &[0.0, 0.0, 0.0, 1.0]));
        for cut in [1, 10, 30] {
            assert_eq!(
                decode(high(15), &body[..body.len() - cut]),
                Err(DecodeError),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn rotate_turns_a_vector_by_a_quaternion() {
        // A quarter turn about the vertical axis takes x to y.
        assert!(close(
            &rotate(QUARTER_TURN.1, [1.0, 0.0, 0.0]),
            &[0.0, 1.0, 0.0]
        ));
        assert!(close(
            &rotate(QUARTER_TURN.1, [0.0, 1.0, 2.0]),
            &[-1.0, 0.0, 2.0]
        ));
        assert!(close(
            &rotate([0.0, 0.0, 0.0, 1.0], [3.0, 4.0, 5.0]),
            &[3.0, 4.0, 5.0]
        ));
    }

    #[test]
    fn truncated_bodies_are_errors_and_unknown_ids_are_ignored() {
        let body = object_update(&[(Uuid::nil(), 1)]);
        assert_eq!(decode(high(12), &body[..body.len() - 1]), Err(DecodeError));
        assert_eq!(decode(low(148), &[0; 5]), Err(DecodeError));
        assert_eq!(decode(low(9999), &[1, 2, 3]), Ok(Incoming::Other));
        assert_eq!(decode(high(9), &[]), Ok(Incoming::Other));
    }
}
