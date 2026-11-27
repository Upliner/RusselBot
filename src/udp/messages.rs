//! The handful of messages the bot uses, hand-written against
//! `message_template.msg` (template version 2.0). Body fields are
//! little-endian; strings are `Variable 1` with a trailing NUL.

use super::packet::{high, low, medium, MessageId};
use crate::groups::{Membership, Profile};
use std::fmt;
use uuid::Uuid;

// Template name, frequency and number for every message handled here.
pub const START_PING_CHECK: MessageId = high(1);
pub const COMPLETE_PING_CHECK: MessageId = high(2);
pub const AGENT_UPDATE: MessageId = high(4);
pub const AGENT_ANIMATION: MessageId = high(5);
pub const AGENT_REQUEST_SIT: MessageId = high(6);
pub const AGENT_SIT: MessageId = high(7);
pub const OBJECT_UPDATE: MessageId = high(12);
pub const AVATAR_SIT_RESPONSE: MessageId = high(21);
pub const USE_CIRCUIT_CODE: MessageId = low(3);
pub const AVATAR_PICKER_REQUEST: MessageId = low(26);
pub const AVATAR_PICKER_REPLY: MessageId = low(28);
pub const START_LURE: MessageId = low(70);
pub const TELEPORT_LURE_REQUEST: MessageId = low(71);
pub const OBJECT_SELECT: MessageId = low(110);
pub const OBJECT_DESELECT: MessageId = low(111);
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
pub const REZ_SINGLE_ATTACHMENT_FROM_INV: MessageId = low(395);
pub const DETACH_ATTACHMENT_INTO_INV: MessageId = low(397);
pub const MAP_NAME_REQUEST: MessageId = low(408);
pub const MAP_BLOCK_REPLY: MessageId = low(409);
pub const SCRIPT_ANSWER_YES: MessageId = low(132);
pub const SCRIPT_QUESTION: MessageId = low(188);
pub const REQUEST_OBJECT_PROPERTIES_FAMILY: MessageId = medium(5);
pub const OBJECT_PROPERTIES_FAMILY: MessageId = medium(10);
pub const OBJECT_PROPERTIES: MessageId = medium(9);
pub const PACKET_ACK: MessageId = 0xFFFF_FFFB;
/// Sent by the bot to play a sound, and by simulators when one plays.
pub const SOUND_TRIGGER: MessageId = high(29);
pub const COARSE_LOCATION_UPDATE: MessageId = medium(6);
pub const AVATAR_PROPERTIES_REQUEST: MessageId = low(169);
pub const AVATAR_PROPERTIES_REPLY: MessageId = low(171);
pub const TERMINATE_FRIENDSHIP: MessageId = low(300);
pub const CHANGE_USER_RIGHTS: MessageId = low(321);
pub const ONLINE_NOTIFICATION: MessageId = low(322);
pub const OFFLINE_NOTIFICATION: MessageId = low(323);

// Messages a real simulator sends that the bot ignores; listed so verbose
// logging can name them.
pub const LAYER_DATA: MessageId = high(11);
pub const OBJECT_UPDATE_CACHED: MessageId = high(14);
pub const IMPROVED_TERSE_OBJECT_UPDATE: MessageId = high(15);
pub const KILL_OBJECT: MessageId = high(16);
pub const AVATAR_ANIMATION: MessageId = high(20);
pub const CAMERA_CONSTRAINT: MessageId = high(22);
pub const ATTACHED_SOUND: MessageId = medium(13);
pub const VIEWER_EFFECT: MessageId = medium(17);
pub const HEALTH_MESSAGE: MessageId = low(138);
pub const SIM_STATS: MessageId = low(140);
pub const SIMULATOR_VIEWER_TIME_MESSAGE: MessageId = low(150);
pub const AVATAR_APPEARANCE: MessageId = low(158);
pub const SCRIPT_CONTROL_CHANGE: MessageId = low(189);
pub const SCRIPT_DIALOG: MessageId = low(190);
pub const PARCEL_OVERLAY: MessageId = low(196);
pub const AGENT_DATA_UPDATE: MessageId = low(387);
pub const REQUEST_PAY_PRICE: MessageId = low(161);
pub const PAY_PRICE_REPLY: MessageId = low(162);
pub const MONEY_TRANSFER_REQUEST: MessageId = low(311);
pub const MONEY_BALANCE_REQUEST: MessageId = low(313);
pub const MONEY_BALANCE_REPLY: MessageId = low(314);
pub const JOIN_GROUP_REQUEST: MessageId = low(343);
pub const JOIN_GROUP_REPLY: MessageId = low(344);
pub const LEAVE_GROUP_REQUEST: MessageId = low(347);
pub const LEAVE_GROUP_REPLY: MessageId = low(348);
pub const INVITE_GROUP_REQUEST: MessageId = low(349);
pub const GROUP_PROFILE_REQUEST: MessageId = low(351);
pub const GROUP_PROFILE_REPLY: MessageId = low(352);
pub const ACTIVATE_GROUP: MessageId = low(368);
pub const AGENT_DATA_UPDATE_REQUEST: MessageId = low(386);
pub const AGENT_GROUP_DATA_UPDATE: MessageId = low(389);
pub const AGENT_DROP_GROUP: MessageId = low(390);

pub type Vector3 = [f32; 3];

/// Template name of a message the bot knows, for verbose logging.
pub fn name(id: MessageId) -> String {
    let known = match id {
        START_PING_CHECK => "StartPingCheck",
        COMPLETE_PING_CHECK => "CompletePingCheck",
        AGENT_UPDATE => "AgentUpdate",
        AGENT_ANIMATION => "AgentAnimation",
        AGENT_REQUEST_SIT => "AgentRequestSit",
        AGENT_SIT => "AgentSit",
        OBJECT_UPDATE => "ObjectUpdate",
        AVATAR_SIT_RESPONSE => "AvatarSitResponse",
        SCRIPT_ANSWER_YES => "ScriptAnswerYes",
        SCRIPT_QUESTION => "ScriptQuestion",
        REQUEST_OBJECT_PROPERTIES_FAMILY => "RequestObjectPropertiesFamily",
        OBJECT_PROPERTIES_FAMILY => "ObjectPropertiesFamily",
        OBJECT_PROPERTIES => "ObjectProperties",
        OBJECT_SELECT => "ObjectSelect",
        OBJECT_DESELECT => "ObjectDeselect",
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
        REZ_SINGLE_ATTACHMENT_FROM_INV => "RezSingleAttachmentFromInv",
        DETACH_ATTACHMENT_INTO_INV => "DetachAttachmentIntoInv",
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
        OFFLINE_NOTIFICATION => "OfflineNotification",
        TERMINATE_FRIENDSHIP => "TerminateFriendship",
        CHANGE_USER_RIGHTS => "ChangeUserRights",
        AVATAR_PROPERTIES_REQUEST => "AvatarPropertiesRequest",
        AVATAR_PROPERTIES_REPLY => "AvatarPropertiesReply",
        AGENT_DATA_UPDATE => "AgentDataUpdate",
        REQUEST_PAY_PRICE => "RequestPayPrice",
        PAY_PRICE_REPLY => "PayPriceReply",
        MONEY_TRANSFER_REQUEST => "MoneyTransferRequest",
        MONEY_BALANCE_REQUEST => "MoneyBalanceRequest",
        MONEY_BALANCE_REPLY => "MoneyBalanceReply",
        JOIN_GROUP_REQUEST => "JoinGroupRequest",
        JOIN_GROUP_REPLY => "JoinGroupReply",
        LEAVE_GROUP_REQUEST => "LeaveGroupRequest",
        LEAVE_GROUP_REPLY => "LeaveGroupReply",
        INVITE_GROUP_REQUEST => "InviteGroupRequest",
        GROUP_PROFILE_REQUEST => "GroupProfileRequest",
        GROUP_PROFILE_REPLY => "GroupProfileReply",
        ACTIVATE_GROUP => "ActivateGroup",
        AGENT_DATA_UPDATE_REQUEST => "AgentDataUpdateRequest",
        AGENT_GROUP_DATA_UPDATE => "AgentGroupDataUpdate",
        AGENT_DROP_GROUP => "AgentDropGroup",
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
        Writer::default().uuid(agent.agent_id).uuid(agent.session_id)
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
    Writer::agent(agent).u32(circuit_code).finish(COMPLETE_AGENT_MOVEMENT, false)
}

/// RegionHandshakeReply, Low 149, Zerocoded.
pub fn region_handshake_reply(agent: &Agent) -> Outgoing {
    Writer::agent(agent).u32(0).finish(REGION_HANDSHAKE_REPLY, true)
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
    Writer::default().u8(ping_id).finish(COMPLETE_PING_CHECK, false)
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
/// Instant-message dialog: a viewer's automatic answer, sent in Do Not
/// Disturb mode (`IM_DO_NOT_DISTURB_AUTO_RESPONSE`, formerly
/// `IM_BUSY_AUTO_RESPONSE`). Viewers never answer it.
pub const IM_AUTO_RESPONSE: u8 = 20;
/// Instant-message dialog: an offer to teleport the recipient to the sender.
pub const IM_TELEPORT_OFFER: u8 = 22;
/// Instant-message dialog: a request to be teleported to the recipient.
pub const IM_TELEPORT_REQUEST: u8 = 26;
/// Instant-message dialog: an avatar offers an inventory item or folder.
/// The answer that accepts an offer is the dialog one higher.
pub const IM_INVENTORY_OFFERED: u8 = 4;
/// Instant-message dialog: an object offers an inventory item or folder.
pub const IM_TASK_INVENTORY_OFFERED: u8 = 9;
/// Instant-message dialog: an invitation to join a group. `AgentID` is the
/// group, `ID` what the answer names, the bucket the fee and the role.
pub const IM_GROUP_INVITATION: u8 = 3;
/// Instant-message dialog: the viewer starts a group's chat session.
pub const IM_SESSION_GROUP_START: u8 = 15;
/// Instant-message dialog: a line of a group's or a conference's chat.
pub const IM_SESSION_SEND: u8 = 17;
/// Instant-message dialog: a group notice.
pub const IM_GROUP_NOTICE: u8 = 32;
/// Instant-message dialog: the answer that accepts a group invitation.
pub const IM_GROUP_INVITATION_ACCEPT: u8 = 35;
/// Instant-message dialog: the answer that declines a group invitation.
pub const IM_GROUP_INVITATION_DECLINE: u8 = 36;
/// TeleportFlags: the teleport answers an offer.
const TELEPORT_VIA_LURE: u32 = 1 << 2;

/// ImprovedInstantMessage, Low 254, Zerocoded: an instant message from the
/// agent to another avatar. For a plain message and an automatic answer
/// the conversation id is the two agent ids combined, which is how viewers
/// pair a reply with the message; other dialogs carry no id.
pub fn instant_message(agent: &Agent, from_name: &str, to: Uuid, position: Vector3, dialog: u8, text: &str) -> Outgoing {
    let conversation = match dialog {
        IM_PLAIN | IM_AUTO_RESPONSE => Uuid::from_u128(agent.agent_id.as_u128() ^ to.as_u128()),
        _ => Uuid::nil(),
    };
    instant_message_with(agent, from_name, to, position, dialog, conversation, text, &[0])
}

/// The answer that accepts the inventory offer `transaction`, which came
/// with the dialog `offered`: the dialog one higher, with the folder the
/// item is to be filed in as the binary bucket. It goes to the avatar the
/// offer came from, which for an object's offer is the object's owner.
pub fn inventory_accepted(
    agent: &Agent,
    from_name: &str,
    to: Uuid,
    position: Vector3,
    offered: u8,
    transaction: Uuid,
    folder: Uuid,
) -> Outgoing {
    instant_message_with(agent, from_name, to, position, offered + 1, transaction, "", folder.as_bytes())
}

/// ImprovedInstantMessage, Low 254, Zerocoded, to a group: the recipient
/// is the group, as a viewer sends the start of a group's chat session
/// (with the group as `id`), a line in it, and the answers to an
/// invitation (with the invitation's `id`); the bucket is empty.
pub fn group_instant_message(agent: &Agent, from_name: &str, group: Uuid, position: Vector3, dialog: u8, id: Uuid, text: &str) -> Outgoing {
    instant_message_with(agent, from_name, group, position, dialog, id, text, &[0])
}

/// ImprovedInstantMessage with the id and the binary bucket given.
#[allow(clippy::too_many_arguments)]
fn instant_message_with(
    agent: &Agent,
    from_name: &str,
    to: Uuid,
    position: Vector3,
    dialog: u8,
    id: Uuid,
    text: &str,
    bucket: &[u8],
) -> Outgoing {
    Writer::agent(agent)
        .u8(0) // FromGroup
        .uuid(to)
        .u32(0) // ParentEstateID, filled in on the simulator
        .uuid(Uuid::nil()) // RegionID, filled in on the simulator
        .vector3(position)
        .u8(0) // Offline
        .u8(dialog)
        .uuid(id)
        .u32(0) // Timestamp
        .text1(from_name)
        .text2(text)
        .variable2(bucket) // BinaryBucket
        .u32(0) // EstateBlock
        .u8(0) // MetaData block count
        .finish(IMPROVED_INSTANT_MESSAGE, true)
}

/// ScriptAnswerYes, Low 132: TaskID, ItemID, Questions S32. Grants the
/// script `item` in the object `object` the permissions `questions`, the
/// bits of its ScriptQuestion.
pub fn script_answer_yes(agent: &Agent, object: Uuid, item: Uuid, questions: i32) -> Outgoing {
    Writer::agent(agent)
        .uuid(object)
        .uuid(item)
        .i32(questions)
        .finish(SCRIPT_ANSWER_YES, false)
}

/// RequestObjectPropertiesFamily, Medium 5, Zerocoded: RequestFlags U32,
/// ObjectID. Asks who owns the object; answered with
/// ObjectPropertiesFamily.
pub fn request_object_properties_family(agent: &Agent, object: Uuid) -> Outgoing {
    Writer::agent(agent)
        .u32(0) // RequestFlags
        .uuid(object)
        .finish(REQUEST_OBJECT_PROPERTIES_FAMILY, true)
}

/// AgentDataUpdateRequest, Low 386: asks for the active group and the
/// list of the avatar's groups, as a viewer does after login and a join.
pub fn agent_data_update_request(agent: &Agent) -> Outgoing {
    Writer::agent(agent).finish(AGENT_DATA_UPDATE_REQUEST, false)
}

/// ActivateGroup, Low 368, Zerocoded: GroupID, nil for none.
pub fn activate_group(agent: &Agent, group: Uuid) -> Outgoing {
    Writer::agent(agent).uuid(group).finish(ACTIVATE_GROUP, true)
}

/// InviteGroupRequest, Low 349: GroupID, then InviteData Variable
/// { InviteeID, RoleID }, one block, the role nil for Everyone.
pub fn invite_group_request(agent: &Agent, group: Uuid, invitee: Uuid) -> Outgoing {
    Writer::agent(agent)
        .uuid(group)
        .u8(1)
        .uuid(invitee)
        .uuid(Uuid::nil())
        .finish(INVITE_GROUP_REQUEST, false)
}

/// GroupProfileRequest, Low 351: GroupID. Answered with GroupProfileReply.
pub fn group_profile_request(agent: &Agent, group: Uuid) -> Outgoing {
    Writer::agent(agent).uuid(group).finish(GROUP_PROFILE_REQUEST, false)
}

/// JoinGroupRequest, Low 343, Zerocoded: GroupID. Answered with
/// JoinGroupReply.
pub fn join_group_request(agent: &Agent, group: Uuid) -> Outgoing {
    Writer::agent(agent).uuid(group).finish(JOIN_GROUP_REQUEST, true)
}

/// LeaveGroupRequest, Low 347: GroupID. Answered with LeaveGroupReply.
pub fn leave_group_request(agent: &Agent, group: Uuid) -> Outgoing {
    Writer::agent(agent).uuid(group).finish(LEAVE_GROUP_REQUEST, false)
}

/// The transaction type of a gift of money to an avatar.
pub const TRANSACTION_GIFT: i32 = 5001;
/// The transaction type of a payment to an object.
pub const TRANSACTION_PAY_OBJECT: i32 = 5008;

/// MoneyTransferRequest, Low 311, Zerocoded: SourceID, DestID, Flags U8,
/// Amount S32, AggregatePermNextOwner U8, AggregatePermInventory U8,
/// TransactionType S32, Description Variable 1. Pays `dest`, an avatar or
/// an object, as a viewer's Pay does.
pub fn money_transfer_request(agent: &Agent, dest: Uuid, amount: i32, kind: i32, description: &str) -> Outgoing {
    Writer::agent(agent)
        .uuid(agent.agent_id)
        .uuid(dest)
        .u8(0) // Flags: neither party is a group
        .i32(amount)
        .u8(0) // AggregatePermNextOwner
        .u8(0) // AggregatePermInventory
        .i32(kind)
        .text1(description)
        .finish(MONEY_TRANSFER_REQUEST, true)
}

/// MoneyBalanceRequest, Low 313, Zerocoded: TransactionID, null as a
/// viewer sends it. Answered with MoneyBalanceReply.
pub fn money_balance_request(agent: &Agent) -> Outgoing {
    Writer::agent(agent).uuid(Uuid::nil()).finish(MONEY_BALANCE_REQUEST, true)
}

/// RequestPayPrice, Low 161: ObjectID, without the agent. Answered with
/// PayPriceReply.
pub fn request_pay_price(object: Uuid) -> Outgoing {
    Writer::default().uuid(object).finish(REQUEST_PAY_PRICE, false)
}

/// AvatarPickerRequest, Low 26: QueryID LLUUID, Name Variable 1. Asks for
/// the avatars whose names match `name`.
pub fn avatar_picker_request(agent: &Agent, query: Uuid, name: &str) -> Outgoing {
    Writer::agent(agent).uuid(query).text1(name).finish(AVATAR_PICKER_REQUEST, false)
}

/// AvatarPropertiesRequest, Low 169: AvatarID. Asks for the profile of
/// `avatar`; answered with AvatarPropertiesReply.
pub fn avatar_properties_request(agent: &Agent, avatar: Uuid) -> Outgoing {
    Writer::agent(agent).uuid(avatar).finish(AVATAR_PROPERTIES_REQUEST, false)
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

/// The most objects one ObjectSelect or ObjectDeselect names: the block
/// count is one byte.
pub const SELECT_BATCH: usize = 255;

fn local_ids(agent: &Agent, local_ids: &[u32]) -> Writer {
    let mut writer = Writer::agent(agent).u8(local_ids.len() as u8);
    for id in local_ids {
        writer = writer.u32(*id);
    }
    writer
}

/// ObjectSelect, Low 110, Zerocoded: ObjectData Variable { ObjectLocalID
/// U32 }. The simulator answers with the ObjectProperties of each object.
/// At most 255 ids fit in one message.
pub fn object_select(agent: &Agent, ids: &[u32]) -> Outgoing {
    local_ids(agent, ids).finish(OBJECT_SELECT, true)
}

/// ObjectDeselect, Low 111, Zerocoded: ObjectData Variable { ObjectLocalID
/// U32 }. Ends the selection an ObjectSelect made.
pub fn object_deselect(agent: &Agent, ids: &[u32]) -> Outgoing {
    local_ids(agent, ids).finish(OBJECT_DESELECT, true)
}

/// RezSingleAttachmentFromInv's AttachmentPt: do not take off what is on
/// the point already (ATTACHMENT_ADD, indra_constants.h:194).
pub const ATTACHMENT_ADD: u8 = 0x80;

/// What RezSingleAttachmentFromInv says about the inventory item besides
/// its id: the fields the simulator would otherwise have to look up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttachItem {
    pub item_id: Uuid,
    pub owner_id: Uuid,
    pub flags: u32,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub next_owner_mask: u32,
    pub name: String,
    pub description: String,
}

/// RezSingleAttachmentFromInv, Low 395, Zerocoded: ItemID, OwnerID,
/// AttachmentPt U8, ItemFlags U32, GroupMask U32, EveryoneMask U32,
/// NextOwnerMask U32, Name Variable 1, Description Variable 1
/// (message_template.msg:8611). `point` 0 leaves the choice to the
/// simulator, which takes the point the item was last on; the add bit is
/// always set, so nothing is taken off to make room.
pub fn rez_single_attachment_from_inv(agent: &Agent, item: &AttachItem, point: u8) -> Outgoing {
    Writer::agent(agent)
        .uuid(item.item_id)
        .uuid(item.owner_id)
        .u8(point | ATTACHMENT_ADD)
        .u32(item.flags)
        .u32(item.group_mask)
        .u32(item.everyone_mask)
        .u32(item.next_owner_mask)
        .text1(&item.name)
        .text1(&item.description)
        .finish(REZ_SINGLE_ATTACHMENT_FROM_INV, true)
}

/// DetachAttachmentIntoInv, Low 397: AgentID, ItemID
/// (message_template.msg:8660). There is no session id in this one.
pub fn detach_attachment_into_inv(agent: &Agent, item: Uuid) -> Outgoing {
    Writer::default()
        .uuid(agent.agent_id)
        .uuid(item)
        .finish(DETACH_ATTACHMENT_INTO_INV, false)
}

/// AgentAnimation, High 5: AnimationList Variable { AnimID LLUUID,
/// StartAnim BOOL }, PhysicalAvatarEventList Variable
/// (message_template.msg:1706). `animation` is the asset's id; `start`
/// false stops it.
pub fn agent_animation(agent: &Agent, animation: Uuid, start: bool) -> Outgoing {
    Writer::agent(agent)
        .u8(1) // AnimationList block count
        .uuid(animation)
        .u8(u8::from(start))
        .u8(0) // PhysicalAvatarEventList block count
        .finish(AGENT_ANIMATION, false)
}

/// SoundTrigger, High 29: SoundID, OwnerID, ObjectID, ParentID, Handle
/// U64, Position LLVector3, Gain F32 (message_template.msg:7368). A viewer
/// leaves the three ids after the sound's null; the simulator fills them
/// in (llviewermessage.cpp:718-742).
pub fn sound_trigger(sound: Uuid, region_handle: u64, position: Vector3, gain: f32) -> Outgoing {
    Writer::default()
        .uuid(sound)
        .uuid(Uuid::nil()) // OwnerID
        .uuid(Uuid::nil()) // ObjectID
        .uuid(Uuid::nil()) // ParentID
        .u64(region_handle)
        .vector3(position)
        .f32(gain)
        .finish(SOUND_TRIGGER, false)
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
    /// A string that ends with a NUL, as data packers write one; without
    /// the NUL.
    fn cstring(&mut self) -> Decoded<&'a [u8]> {
        let len = self.0.iter().position(|byte| *byte == 0).ok_or(DecodeError)?;
        let string = self.bytes(len)?;
        self.skip(1)?;
        Ok(string)
    }
    /// Binary data as data packers write it: a 32-bit length, then the data.
    fn binary(&mut self) -> Decoded<&'a [u8]> {
        let len = usize::try_from(self.u32()?).map_err(|_| DecodeError)?;
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
        let usable = position.iter().all(|part| part.is_finite()) && norm.is_finite() && norm > 0.5 && norm < 2.0;
        usable.then(|| Transform {
            position,
            rotation: rotation.map(|part| part / norm),
        })
    }
}

/// `v` turned by the unit quaternion `q`.
pub fn rotate(q: Quaternion, v: Vector3) -> Vector3 {
    let cross = |a: Vector3, b: Vector3| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
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
    /// For an attachment, the point it is on, with the nibbles swapped.
    pub state: u8,
    /// The inventory item an attachment was made from: the name-value
    /// `AttachItemID` of its root prim.
    pub attach_item: Option<Uuid>,
    /// What kind of object it is: `PCODE_PRIM`, `PCODE_AVATAR`, a tree...
    pub pcode: u8,
    /// `UpdateFlags`: the `FLAG_` bits among others.
    pub flags: u32,
}

/// PCode of a prim (LL_PCODE_VOLUME, llprimitive.h).
pub const PCODE_PRIM: u8 = 9;
/// PCode of an avatar (LL_PCODE_LEGACY_AVATAR).
pub const PCODE_AVATAR: u8 = 47;

// The bits of an object's UpdateFlags the bot reads (object_flags.h).
pub const FLAG_PHYSICS: u32 = 1 << 0;
pub const FLAG_SCRIPTED: u32 = 1 << 6;
pub const FLAG_TOUCH: u32 = 1 << 7;
/// The object takes money: a script in it has a `money` event.
pub const FLAG_MONEY: u32 = 1 << 9;
pub const FLAG_PHANTOM: u32 = 1 << 10;
pub const FLAG_TEMPORARY: u32 = 1 << 29;

/// What the simulator says of an object in answer to a selection or to
/// RequestObjectPropertiesFamily.
#[derive(Debug, Clone, PartialEq)]
pub struct Properties {
    pub object: Uuid,
    /// The owning avatar; nil, or the group, for a group's object.
    pub owner: Uuid,
    pub group: Uuid,
    pub name: String,
    pub description: String,
}

impl ObjectParent {
    /// The attachment point in `State`; meaningful for an attachment.
    pub fn attachment_point(&self) -> u8 {
        attachment_point(self.state)
    }
}

/// What a compressed update says about an attachment's root prim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Worn {
    /// The inventory item it was made from.
    pub item: Uuid,
    /// Region-local id of what it is attached to.
    pub parent_id: u32,
    pub point: u8,
}

/// One object from an ObjectUpdateCompressed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressedObject {
    pub full_id: Uuid,
    pub local_id: u32,
    pub transform: Option<Transform>,
    /// Region-local id of the parent, zero when the object has none;
    /// missing when the update is too short to say.
    pub parent_id: Option<u32>,
    /// Set when the update names the inventory item the object was
    /// attached from; missing also when its optional fields do not read.
    pub attachment: Option<Worn>,
    /// What kind of object it is; zero when the update is too short to say.
    pub pcode: u8,
    /// `UpdateFlags`: the `FLAG_` bits among others.
    pub flags: u32,
}

/// The attachment point in an object's `State`: the two nibbles swapped
/// (ATTACHMENT_ID_FROM_STATE, indra_constants.h:386; OpenSim writes it the
/// same way, LLClientView.cs:7393).
pub fn attachment_point(state: u8) -> u8 {
    state.rotate_left(4)
}

/// The inventory item named by `AttachItemID` in an object's name-values:
/// lines of `Name TYPE CLASS SENDTO value`, of which an attachment's root
/// prim has `AttachItemID STRING RW SV <uuid>` (llviewerobject.cpp:7589;
/// LLClientView.cs:7380).
fn attach_item(name_values: &[u8]) -> Option<Uuid> {
    let list = text(name_values);
    let line = list.lines().find(|line| line.split_whitespace().next() == Some("AttachItemID"))?;
    let item = Uuid::parse_str(line.split_whitespace().last()?).ok()?;
    (!item.is_nil()).then_some(item)
}

/// A MoneyBalanceReply: the balance and, for a transaction, what it was.
#[derive(Debug, Clone, PartialEq)]
pub struct MoneyReport {
    /// The grid's id of the transaction; Second Life sends some reports
    /// twice with one id.
    pub transaction: Uuid,
    pub success: bool,
    pub balance: i32,
    pub description: String,
    pub info: Option<TransactionInfo>,
}

/// The TransactionInfo block of a MoneyBalanceReply. A type of 0 is a
/// balance alone.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionInfo {
    pub kind: i32,
    pub source: Uuid,
    pub source_group: bool,
    pub dest: Uuid,
    pub dest_group: bool,
    pub amount: i32,
    pub item: String,
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
        /// The sender is a group: the owner of the object that sent it.
        from_group: bool,
        /// The recipient: the avatar, or the group of a group's session.
        to: Uuid,
        from_name: String,
        dialog: u8,
        /// Meaning depends on `dialog`; the object for an object's IM.
        id: Uuid,
        /// The message was stored while the avatar was offline.
        offline: bool,
        /// When a stored message was sent, in seconds since the Unix epoch.
        timestamp: u32,
        text: String,
        /// Data whose meaning depends on `dialog`.
        bucket: Vec<u8>,
    },
    /// A script asks the avatar for permissions.
    ScriptQuestion {
        /// The object the script is in.
        object: Uuid,
        /// The script, as an item of the object's inventory.
        item: Uuid,
        object_name: String,
        /// The owner's name as text; a group's name for a group's object.
        owner_name: String,
        /// The permission bits asked for.
        questions: i32,
    },
    /// Who owns an object, in answer to RequestObjectPropertiesFamily.
    ObjectPropertiesFamily {
        object: Uuid,
        owner: Uuid,
        group: Uuid,
        /// Its name and description; missing when the answer ends early.
        named: Option<(String, String)>,
    },
    /// What objects are called and whose they are, in answer to a
    /// selection.
    ObjectProperties(Vec<Properties>),
    /// The answer to an avatar search: the avatars found, as id and
    /// `First Last`.
    AvatarPickerReply {
        query: Uuid,
        avatars: Vec<(Uuid, String)>,
    },
    /// The avatars in the region as the simulator lists them for the
    /// minimap: id and position, `x` and `y` in whole metres, `z` in steps
    /// of 4 metres.
    AvatarsHere(Vec<(Uuid, [u8; 3])>),
    /// Friends that came online.
    FriendsOnline(Vec<Uuid>),
    /// Friends that went offline.
    FriendsOffline(Vec<Uuid>),
    /// The avatar that ended its friendship with the agent.
    FriendshipEnded(Uuid),
    /// Rights between friends changed. When `agent` is another avatar, it
    /// is the friend that changed what it grants the agent; when it is the
    /// agent, the rights are the ones the agent grants.
    RightsChanged {
        agent: Uuid,
        rights: Vec<(Uuid, i32)>,
    },
    /// An avatar's profile, in answer to AvatarPropertiesRequest.
    AvatarProperties {
        avatar: Uuid,
        partner: Uuid,
        about: String,
        first_life: String,
        born: String,
        flags: u32,
    },
    /// The avatar's balance, alone or with a transaction of the avatar.
    MoneyBalanceReply(MoneyReport),
    /// The avatar's active group, with its title and the group's name;
    /// a nil group for none.
    AgentDataUpdate {
        active: Uuid,
        title: String,
        group_name: String,
    },
    /// The groups the avatar is in.
    AgentGroupDataUpdate(Vec<Membership>),
    /// The avatar is no longer in this group.
    AgentDropGroup(Uuid),
    /// A group's profile, in answer to GroupProfileRequest.
    GroupProfileReply(Profile),
    /// The answer to JoinGroupRequest.
    JoinGroupReply {
        group: Uuid,
        success: bool,
    },
    /// The answer to LeaveGroupRequest.
    LeaveGroupReply {
        group: Uuid,
        success: bool,
    },
    /// What an object offers to be paid, in answer to RequestPayPrice.
    PayPriceReply {
        object: Uuid,
        /// -1 hides the amount field, -2 leaves it empty.
        default: i32,
        /// The quick-pay buttons: -1 hidden, -2 the default amount.
        buttons: Vec<i32>,
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
            Incoming::TeleportLocal { position: r.vector3()? }
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
        // position is at offset 40 and the rotation at 52. What follows
        // the rotation is read by `compressed_attachment`.
        OBJECT_UPDATE_COMPRESSED => {
            r.skip(8 + 2)?;
            let count = r.u8()?;
            let mut objects = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let flags = r.u32()?;
                let mut data = Reader(r.variable2()?);
                let (full_id, local_id) = (data.uuid()?, data.u32()?);
                let pcode = data.0.first().copied().unwrap_or(0);
                // Whatever is too short to hold them describes the object
                // all the same.
                let mut parent_id = None;
                let (transform, attachment) = match compressed_motion(&mut data) {
                    Ok((state, position, rotation)) => (
                        Transform::checked(position, unpack(rotation)),
                        compressed_attachment(&mut data, state, &mut parent_id).unwrap_or(None),
                    ),
                    Err(_) => (None, None),
                };
                objects.push(CompressedObject {
                    full_id,
                    local_id,
                    transform,
                    parent_id,
                    attachment,
                    pcode,
                    flags,
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
            r.skip(16)?;
            let from_group = r.u8()? != 0;
            let to = r.uuid()?;
            r.skip(4 + 16 + 12)?;
            let (offline, dialog) = (r.u8()? != 0, r.u8()?);
            let (id, timestamp) = (r.uuid()?, r.u32()?);
            let from_name = text(r.variable1()?);
            let text = text(r.variable2()?);
            Incoming::InstantMessage {
                from_agent,
                from_group,
                to,
                from_name,
                dialog,
                id,
                offline,
                timestamp,
                text,
                bucket: r.variable2()?.to_vec(),
            }
        }
        // ScriptQuestion, Low 188: TaskID, ItemID, ObjectName Variable 1,
        // ObjectOwner Variable 1, Questions S32, then the Experience block,
        // which older simulators leave out.
        SCRIPT_QUESTION => {
            let (object, item) = (r.uuid()?, r.uuid()?);
            let (object_name, owner_name) = (text(r.variable1()?), text(r.variable1()?));
            Incoming::ScriptQuestion {
                object,
                item,
                object_name,
                owner_name,
                questions: r.u32()? as i32,
            }
        }
        // ObjectPropertiesFamily, Medium 10: RequestFlags U32, ObjectID,
        // OwnerID, GroupID, then masks, sale data, names.
        OBJECT_PROPERTIES_FAMILY => {
            r.skip(4)?;
            let (object, owner, group) = (r.uuid()?, r.uuid()?, r.uuid()?);
            // BaseMask .. NextOwnerMask, OwnershipCost S32, SaleType U8,
            // SalePrice S32, Category U32, LastOwnerID, then Name and
            // Description, both Variable 1.
            let mut named = || {
                r.skip(5 * 4 + 4 + 1 + 4 + 4 + 16)?;
                Ok::<_, DecodeError>((text(r.variable1()?), text(r.variable1()?)))
            };
            Incoming::ObjectPropertiesFamily {
                object,
                owner,
                group,
                named: named().ok(),
            }
        }
        // ObjectProperties, Medium 9: ObjectData Variable { ObjectID,
        // CreatorID, OwnerID, GroupID, CreationDate U64, BaseMask ..
        // NextOwnerMask (five U32), OwnershipCost S32, SaleType U8,
        // SalePrice S32, AggregatePerms U8, AggregatePermTextures U8,
        // AggregatePermTexturesOwner U8, Category U32, InventorySerial
        // S16, ItemID, FolderID, FromTaskID, LastOwnerID, then Name,
        // Description, TouchName, SitName and TextureID, each Variable 1 }
        // (message_template.msg:3709).
        OBJECT_PROPERTIES => {
            let count = r.u8()?;
            let mut objects = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let object = r.uuid()?;
                r.skip(16)?; // CreatorID
                let (owner, group) = (r.uuid()?, r.uuid()?);
                r.skip(8 + 5 * 4 + 4 + 1 + 4 + 3 + 4 + 2 + 4 * 16)?;
                let (name, description) = (text(r.variable1()?), text(r.variable1()?));
                for _ in 0..3 {
                    r.variable1()?; // TouchName, SitName, TextureID
                }
                objects.push(Properties {
                    object,
                    owner,
                    group,
                    name,
                    description,
                });
            }
            Incoming::ObjectProperties(objects)
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
        // CoarseLocationUpdate, Medium 6: Location Variable { X U8, Y U8,
        // Z U8 }, Index { You S16, Prey S16 }, AgentData Variable { AgentID }.
        // The two lists pair by index; older simulators leave the ids out,
        // and a list without them names nobody.
        COARSE_LOCATION_UPDATE => {
            let count = r.u8()?;
            let locations = (0..count).map(|_| r.array::<3>()).collect::<Decoded<Vec<_>>>()?;
            r.skip(2 + 2)?;
            let ids = match r.0.is_empty() {
                true => Vec::new(),
                false => {
                    let count = r.u8()?;
                    (0..count).map(|_| r.uuid()).collect::<Decoded<Vec<_>>>()?
                }
            };
            Incoming::AvatarsHere(ids.into_iter().zip(locations).collect())
        }
        // OnlineNotification, Low 322, and OfflineNotification, Low 323:
        // AgentBlock Variable { AgentID }.
        ONLINE_NOTIFICATION | OFFLINE_NOTIFICATION => {
            let count = r.u8()?;
            let ids = (0..count).map(|_| r.uuid()).collect::<Decoded<Vec<_>>>()?;
            match id {
                ONLINE_NOTIFICATION => Incoming::FriendsOnline(ids),
                _ => Incoming::FriendsOffline(ids),
            }
        }
        // TerminateFriendship, Low 300: AgentID, SessionID, ExBlock { OtherID }.
        TERMINATE_FRIENDSHIP => {
            r.skip(32)?;
            Incoming::FriendshipEnded(r.uuid()?)
        }
        // ChangeUserRights, Low 321: AgentID, Rights Variable
        // { AgentRelated, RelatedRights S32 }.
        CHANGE_USER_RIGHTS => {
            let agent = r.uuid()?;
            let count = r.u8()?;
            let rights = (0..count).map(|_| Ok((r.uuid()?, r.u32()? as i32))).collect::<Decoded<Vec<_>>>()?;
            Incoming::RightsChanged { agent, rights }
        }
        // AvatarPropertiesReply, Low 171, Zerocoded: AgentID, AvatarID,
        // ImageID, FLImageID, PartnerID, AboutText Variable 2, FLAboutText
        // Variable 1, BornOn Variable 1, ProfileURL Variable 1,
        // CharterMember Variable 1, Flags U32.
        AVATAR_PROPERTIES_REPLY => {
            r.skip(16)?;
            let avatar = r.uuid()?;
            r.skip(32)?; // ImageID, FLImageID
            let partner = r.uuid()?;
            let about = text(r.variable2()?);
            let first_life = text(r.variable1()?);
            let born = text(r.variable1()?);
            r.variable1()?; // ProfileURL
            r.variable1()?; // CharterMember
            Incoming::AvatarProperties {
                avatar,
                partner,
                about,
                first_life,
                born,
                flags: r.u32()?,
            }
        }
        // MoneyBalanceReply, Low 314, Zerocoded: AgentID, TransactionID,
        // TransactionSuccess BOOL, MoneyBalance S32, SquareMetersCredit
        // S32, SquareMetersCommitted S32, Description Variable 1, then
        // TransactionInfo { TransactionType S32, SourceID, IsSourceGroup
        // BOOL, DestID, IsDestGroup BOOL, Amount S32, ItemDescription
        // Variable 1 }, which older simulators leave out.
        MONEY_BALANCE_REPLY => {
            r.skip(16)?;
            let transaction = r.uuid()?;
            let success = r.u8()? != 0;
            let balance = r.u32()? as i32;
            r.skip(8)?;
            let description = text(r.variable1()?);
            let info = match r.0.is_empty() {
                true => None,
                false => Some(TransactionInfo {
                    kind: r.u32()? as i32,
                    source: r.uuid()?,
                    source_group: r.u8()? != 0,
                    dest: r.uuid()?,
                    dest_group: r.u8()? != 0,
                    amount: r.u32()? as i32,
                    item: text(r.variable1()?),
                }),
            };
            Incoming::MoneyBalanceReply(MoneyReport {
                transaction,
                success,
                balance,
                description,
                info,
            })
        }
        // AgentDataUpdate, Low 387: AgentID, FirstName Variable 1,
        // LastName Variable 1, GroupTitle Variable 1, ActiveGroupID,
        // GroupPowers U64, GroupName Variable 1.
        AGENT_DATA_UPDATE => {
            r.skip(16)?;
            r.variable1()?;
            r.variable1()?;
            let title = text(r.variable1()?);
            let active = r.uuid()?;
            r.skip(8)?;
            Incoming::AgentDataUpdate {
                active,
                title,
                group_name: text(r.variable1()?),
            }
        }
        // AgentGroupDataUpdate, Low 389: AgentID, then GroupData Variable
        // { GroupID, GroupPowers U64, AcceptNotices BOOL, GroupInsigniaID,
        //   Contribution S32, GroupName Variable 1 }.
        AGENT_GROUP_DATA_UPDATE => {
            r.skip(16)?;
            let count = r.u8()?;
            let mut groups = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let id = r.uuid()?;
                let powers = r.u64()?;
                let notices = r.u8()? != 0;
                r.skip(16 + 4)?;
                let name = text(r.variable1()?);
                groups.push(Membership { id, name, powers, notices });
            }
            Incoming::AgentGroupDataUpdate(groups)
        }
        // AgentDropGroup, Low 390: AgentID, GroupID.
        AGENT_DROP_GROUP => {
            r.skip(16)?;
            Incoming::AgentDropGroup(r.uuid()?)
        }
        // GroupProfileReply, Low 352: AgentID, then GroupID, Name Variable
        // 1, Charter Variable 2, ShowInList BOOL, MemberTitle Variable 1,
        // PowersMask U64, InsigniaID, FounderID, MembershipFee S32,
        // OpenEnrollment BOOL, Money S32, GroupMembershipCount S32,
        // GroupRolesCount S32, AllowPublish BOOL, MaturePublish BOOL,
        // OwnerRole.
        GROUP_PROFILE_REPLY => {
            r.skip(16)?;
            let id = r.uuid()?;
            let name = text(r.variable1()?);
            let charter = text(r.variable2()?);
            let listed = r.u8()? != 0;
            let title = text(r.variable1()?);
            r.skip(8 + 16)?;
            let founder = r.uuid()?;
            let fee = r.u32()? as i32;
            let open = r.u8()? != 0;
            r.skip(4)?;
            let members = r.u32()? as i32;
            let roles = r.u32()? as i32;
            r.skip(1)?;
            let mature = r.u8()? != 0;
            Incoming::GroupProfileReply(Profile {
                id,
                name,
                charter,
                listed,
                title,
                founder,
                fee,
                open,
                members,
                roles,
                mature,
            })
        }
        // JoinGroupReply, Low 344: AgentID, then GroupID, Success BOOL.
        JOIN_GROUP_REPLY => {
            r.skip(16)?;
            Incoming::JoinGroupReply {
                group: r.uuid()?,
                success: r.u8()? != 0,
            }
        }
        // LeaveGroupReply, Low 348: AgentID, then GroupID, Success BOOL.
        LEAVE_GROUP_REPLY => {
            r.skip(16)?;
            Incoming::LeaveGroupReply {
                group: r.uuid()?,
                success: r.u8()? != 0,
            }
        }
        // PayPriceReply, Low 162: ObjectID, DefaultPayPrice S32, then
        // ButtonData Variable { PayButton S32 }.
        PAY_PRICE_REPLY => {
            let object = r.uuid()?;
            let default = r.u32()? as i32;
            let count = r.u8()?;
            let buttons = (0..count).map(|_| r.u32().map(|b| b as i32)).collect::<Decoded<_>>()?;
            Incoming::PayPriceReply { object, default, buttons }
        }
        LOGOUT_REPLY => Incoming::LogoutReply,
        DISABLE_SIMULATOR => Incoming::DisableSimulator,
        _ => Incoming::Other,
    })
}

/// The fixed fields of a compressed update after the local id: PCode U8,
/// State U8, CRC U32, Material U8, ClickAction U8, Scale, Position,
/// Rotation. Returns the state, the position and the rotation.
fn compressed_motion(data: &mut Reader) -> Decoded<(u8, Vector3, Vector3)> {
    data.skip(1)?;
    let state = data.u8()?;
    data.skip(4 + 1 + 1 + 12)?;
    Ok((state, data.vector3()?, data.vector3()?))
}

// The flags of a compressed update that say which optional fields it has
// (llviewerobject.cpp:1804-1942; OpenSim's CompressedFlags,
// LLClientView.cs:7566).
const COMPRESSED_SCRATCH_PAD: u32 = 0x01;
const COMPRESSED_TREE: u32 = 0x02;
const COMPRESSED_TEXT: u32 = 0x04;
const COMPRESSED_PARTICLES: u32 = 0x08;
const COMPRESSED_SOUND: u32 = 0x10;
const COMPRESSED_PARENT: u32 = 0x20;
const COMPRESSED_ANGULAR_VELOCITY: u32 = 0x80;
const COMPRESSED_NAME_VALUES: u32 = 0x100;
const COMPRESSED_MEDIA_URL: u32 = 0x200;
/// The legacy particle system block: 68 bytes of system and 18 of
/// particle data (llpartdata.cpp:42-50).
const PARTICLE_BLOCK: usize = 86;

/// Walks what follows the rotation in a compressed update, up to its
/// name-values: the flags U32 and OwnerID, then, each only when its flag
/// is set unless said otherwise: the angular velocity (0x80, LLVector3),
/// ParentID (0x20, U32), a tree's species (0x02, U8) or else the scratch
/// pad (0x01, a U32 and then binary data with its own length), the text
/// (0x04, a string and four colour bytes), the media URL (0x200, a
/// string), the particle system (0x08, 86 bytes), the extra parameters
/// (always: a count U8, then per parameter a type U16 and binary data),
/// the sound (0x10, 25 bytes) and the name-values (0x100, a string).
/// Strings end with a NUL. Returns the attachment the name-values make of
/// the object, if they do; an error when the data ends early, in which
/// case nothing is made of what was read but the parent, which is put
/// into `parent` as soon as it is known.
fn compressed_attachment(data: &mut Reader, state: u8, parent: &mut Option<u32>) -> Decoded<Option<Worn>> {
    let flags = data.u32()?;
    data.skip(16)?; // OwnerID
    if flags & COMPRESSED_ANGULAR_VELOCITY != 0 {
        data.skip(12)?;
    }
    let parent_id = if flags & COMPRESSED_PARENT != 0 { data.u32()? } else { 0 };
    *parent = Some(parent_id);
    if flags & COMPRESSED_TREE != 0 {
        data.skip(1)?;
    } else if flags & COMPRESSED_SCRATCH_PAD != 0 {
        data.skip(4)?;
        data.binary()?;
    }
    if flags & COMPRESSED_TEXT != 0 {
        data.cstring()?;
        data.skip(4)?;
    }
    if flags & COMPRESSED_MEDIA_URL != 0 {
        data.cstring()?;
    }
    if flags & COMPRESSED_PARTICLES != 0 {
        data.skip(PARTICLE_BLOCK)?;
    }
    for _ in 0..data.u8()? {
        data.skip(2)?;
        data.binary()?;
    }
    if flags & COMPRESSED_SOUND != 0 {
        data.skip(16 + 4 + 1 + 4)?;
    }
    if flags & COMPRESSED_NAME_VALUES == 0 {
        return Ok(None);
    }
    let item = attach_item(data.cstring()?);
    Ok(item.map(|item| Worn {
        item,
        parent_id,
        point: attachment_point(state),
    }))
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
/// ObjectData Variable. Each block is read up to ParentID, its NameValue
/// field is looked at, and the rest is skipped by its length prefixes to
/// reach the next block.
fn decode_object_update(r: &mut Reader) -> Decoded<Vec<ObjectParent>> {
    r.skip(8 + 2)?;
    let count = r.u8()?;
    let mut objects = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let local_id = r.u32()?;
        let state = r.u8()?;
        let full_id = r.uuid()?;
        r.skip(4)?; // CRC
        let pcode = r.u8()?;
        r.skip(1 + 1 + 12)?; // Material, ClickAction, Scale
        let transform = transform_from_motion(r.variable1()?); // ObjectData
        let parent_id = r.u32()?;
        let flags = r.u32()?; // UpdateFlags
        r.skip(23)?; // PathCurve .. ProfileHollow
        r.variable2()?; // TextureEntry
        r.variable1()?; // TextureAnim
        let attach_item = attach_item(r.variable2()?); // NameValue
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
            state,
            attach_item,
            pcode,
            flags,
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
    pub fn chat_from_simulator(from_name: &str, source: (Uuid, Uuid, u8), chat_type: u8, audible: i8, message: &str) -> Vec<u8> {
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
        instant_message_with_bucket(from_agent, false, from_name, dialog, id, stored, message, &[0])
    }

    /// An ImprovedInstantMessage to `to`, as a group's session line, an
    /// invitation or a notice comes: the sender's group flag, the id and
    /// the binary bucket given.
    #[allow(clippy::too_many_arguments)]
    pub fn group_im(
        from_agent: Uuid,
        from_group: bool,
        to: Uuid,
        from_name: &str,
        dialog: u8,
        id: Uuid,
        message: &str,
        bucket: &[u8],
    ) -> Vec<u8> {
        let mut body = instant_message_with_bucket(from_agent, from_group, from_name, dialog, id, None, message, bucket);
        body[33..49].copy_from_slice(to.as_bytes());
        body
    }

    /// The capture under the heading that starts with `heading` in
    /// test-rig/captures/groups.txt: hex for a UDP body, LLSD for an event.
    pub fn capture(heading: &str) -> &'static str {
        let file = include_str!("../../test-rig/captures/groups.txt");
        let start = file
            .find(&format!("## {heading}"))
            .unwrap_or_else(|| panic!("no capture {heading}"));
        let after = &file[start..];
        after.lines().nth(1).expect("a capture under its heading")
    }

    /// A hex capture as bytes.
    pub fn captured_bytes(heading: &str) -> Vec<u8> {
        let hex = capture(heading);
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect()
    }

    /// AgentDataUpdate: the active group, its title and name.
    pub fn agent_data_update(agent: Uuid, active: Uuid, title: &str, group_name: &str) -> Vec<u8> {
        let mut body = agent.as_bytes().to_vec();
        body.extend(string1("Sit"));
        body.extend(string1("Bot"));
        body.extend(string1(title));
        body.extend(active.as_bytes());
        body.extend(0x1234u64.to_le_bytes());
        body.extend(string1(group_name));
        body
    }

    /// AgentGroupDataUpdate with these groups.
    pub fn agent_group_data_update(agent: Uuid, groups: &[crate::groups::Membership]) -> Vec<u8> {
        let mut body = agent.as_bytes().to_vec();
        body.push(groups.len() as u8);
        for group in groups {
            body.extend(group.id.as_bytes());
            body.extend(group.powers.to_le_bytes());
            body.push(u8::from(group.notices));
            body.extend([0x11; 16]); // GroupInsigniaID
            body.extend(0i32.to_le_bytes()); // Contribution
            body.extend(string1(&group.name));
        }
        body
    }

    /// AgentDropGroup.
    pub fn agent_drop_group(agent: Uuid, group: Uuid) -> Vec<u8> {
        let mut body = agent.as_bytes().to_vec();
        body.extend(group.as_bytes());
        body
    }

    /// GroupProfileReply for this profile.
    pub fn group_profile_reply(agent: Uuid, profile: &crate::groups::Profile) -> Vec<u8> {
        let mut body = agent.as_bytes().to_vec();
        body.extend(profile.id.as_bytes());
        body.extend(string1(&profile.name));
        body.extend(string2(&profile.charter));
        body.push(u8::from(profile.listed));
        body.extend(string1(&profile.title));
        body.extend(0xFFu64.to_le_bytes()); // PowersMask
        body.extend([0x22; 16]); // InsigniaID
        body.extend(profile.founder.as_bytes());
        body.extend(profile.fee.to_le_bytes());
        body.push(u8::from(profile.open));
        body.extend(0i32.to_le_bytes()); // Money
        body.extend(profile.members.to_le_bytes());
        body.extend(profile.roles.to_le_bytes());
        body.push(1); // AllowPublish
        body.push(u8::from(profile.mature));
        body.extend([0x33; 16]); // OwnerRole
        body
    }

    /// JoinGroupReply.
    pub fn join_group_reply(agent: Uuid, group: Uuid, success: bool) -> Vec<u8> {
        let mut body = agent.as_bytes().to_vec();
        body.extend(group.as_bytes());
        body.push(u8::from(success));
        body
    }

    /// LeaveGroupReply, laid out as JoinGroupReply is.
    pub fn leave_group_reply(agent: Uuid, group: Uuid, success: bool) -> Vec<u8> {
        join_group_reply(agent, group, success)
    }

    /// An ImprovedInstantMessage with the sender's group flag and the
    /// binary bucket given, as offers carry them.
    #[allow(clippy::too_many_arguments)]
    pub fn instant_message_with_bucket(
        from_agent: Uuid,
        from_group: bool,
        from_name: &str,
        dialog: u8,
        id: Uuid,
        stored: Option<u32>,
        message: &str,
        bucket: &[u8],
    ) -> Vec<u8> {
        let mut body = from_agent.as_bytes().to_vec();
        body.extend([0; 16]); // SessionID
        body.push(u8::from(from_group));
        body.extend([0; 16 + 4 + 16 + 12]); // ToAgentID .. Position
        body.extend([u8::from(stored.is_some()), dialog]);
        body.extend(id.as_bytes());
        body.extend(stored.unwrap_or(0).to_le_bytes());
        body.extend(string1(from_name));
        body.extend(string2(message));
        body.extend((bucket.len() as u16).to_le_bytes()); // BinaryBucket
        body.extend(bucket);
        body.extend([0; 4]); // EstateID
        body.push(0); // MetaData block count
        body
    }

    /// A ScriptQuestion, with the Experience block newer simulators add.
    pub fn script_question(object: Uuid, item: Uuid, object_name: &str, owner_name: &str, questions: i32, experience: bool) -> Vec<u8> {
        let mut body = object.as_bytes().to_vec();
        body.extend(item.as_bytes());
        body.extend(string1(object_name));
        body.extend(string1(owner_name));
        body.extend(questions.to_le_bytes());
        if experience {
            body.extend([0; 16]);
        }
        body
    }

    pub fn object_properties_family(object: Uuid, owner: Uuid, group: Uuid) -> Vec<u8> {
        object_properties_family_named(object, owner, group, "Dance Ball", "")
    }

    pub fn object_properties_family_named(object: Uuid, owner: Uuid, group: Uuid, name: &str, description: &str) -> Vec<u8> {
        let mut body = vec![0; 4]; // RequestFlags
        for id in [object, owner, group] {
            body.extend(id.as_bytes());
        }
        body.extend([0x3C; 5 * 4 + 4 + 1 + 4 + 4 + 16]); // masks .. LastOwnerID
        body.extend(string1(name));
        body.extend(string1(description));
        body
    }

    /// An ObjectProperties with one block for each of `objects`.
    pub fn object_properties(objects: &[Properties]) -> Vec<u8> {
        let mut body = vec![objects.len() as u8];
        for (index, object) in objects.iter().enumerate() {
            body.extend(object.object.as_bytes());
            body.extend([0xC1; 16]); // CreatorID
            body.extend(object.owner.as_bytes());
            body.extend(object.group.as_bytes());
            body.extend([0x3C; 8 + 5 * 4 + 4 + 1 + 4 + 3 + 4 + 2 + 4 * 16]); // CreationDate .. LastOwnerID
            body.extend(string1(&object.name));
            body.extend(string1(&object.description));
            body.extend(string1("Touch")); // TouchName
            body.extend(string1("")); // SitName
            body.push(index as u8 * 16); // TextureID
            body.extend(vec![0x7E; index * 16]);
        }
        body
    }

    /// A CoarseLocationUpdate; `ids` may be fewer than `locations`, and
    /// `None` leaves the AgentData block out as older simulators do.
    pub fn coarse_location_update(locations: &[[u8; 3]], ids: Option<&[Uuid]>) -> Vec<u8> {
        let mut body = vec![locations.len() as u8];
        for location in locations {
            body.extend(location);
        }
        body.extend((-1i16).to_le_bytes()); // You
        body.extend((-1i16).to_le_bytes()); // Prey
        if let Some(ids) = ids {
            body.push(ids.len() as u8);
            for id in ids {
                body.extend(id.as_bytes());
            }
        }
        body
    }

    /// An OnlineNotification or OfflineNotification body.
    pub fn friend_notification(ids: &[Uuid]) -> Vec<u8> {
        let mut body = vec![ids.len() as u8];
        for id in ids {
            body.extend(id.as_bytes());
        }
        body
    }

    pub fn terminate_friendship(other: Uuid) -> Vec<u8> {
        let mut body = vec![0x5A; 32]; // AgentID, SessionID
        body.extend(other.as_bytes());
        body
    }

    pub fn change_user_rights(agent: Uuid, rights: &[(Uuid, i32)]) -> Vec<u8> {
        let mut body = agent.as_bytes().to_vec();
        body.push(rights.len() as u8);
        for (related, value) in rights {
            body.extend(related.as_bytes());
            body.extend(value.to_le_bytes());
        }
        body
    }

    /// A MoneyBalanceReply body, before zerocoding; without the
    /// TransactionInfo block when `info` is `None`.
    pub fn money_balance_reply(success: bool, balance: i32, description: &str, info: Option<&TransactionInfo>) -> Vec<u8> {
        let mut body = vec![0xA1; 16]; // AgentID
        body.extend([0x7A; 16]); // TransactionID
        body.push(u8::from(success));
        body.extend(balance.to_le_bytes());
        body.extend([0; 8]); // SquareMetersCredit, SquareMetersCommitted
        body.extend(string1(description));
        if let Some(info) = info {
            body.extend(info.kind.to_le_bytes());
            body.extend(info.source.as_bytes());
            body.push(u8::from(info.source_group));
            body.extend(info.dest.as_bytes());
            body.push(u8::from(info.dest_group));
            body.extend(info.amount.to_le_bytes());
            body.extend(string1(&info.item));
        }
        body
    }

    pub fn pay_price_reply(object: Uuid, default: i32, buttons: &[i32]) -> Vec<u8> {
        let mut body = object.as_bytes().to_vec();
        body.extend(default.to_le_bytes());
        body.push(buttons.len() as u8);
        for button in buttons {
            body.extend(button.to_le_bytes());
        }
        body
    }

    /// An AvatarPropertiesReply body, before zerocoding.
    pub fn avatar_properties_reply(avatar: Uuid, partner: Uuid, about: &str, first_life: &str, born: &str, flags: u32) -> Vec<u8> {
        let mut body = vec![0xA1; 16]; // AgentID
        body.extend(avatar.as_bytes());
        body.extend([0x1E; 32]); // ImageID, FLImageID
        body.extend(partner.as_bytes());
        body.extend(string2(about));
        body.extend(string1(first_life));
        body.extend(string1(born));
        body.extend(string1("http://example.com/profile")); // ProfileURL
        body.extend([1, 3]); // CharterMember
        body.extend(flags.to_le_bytes());
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
            body.extend([0; 4]); // UpdateFlags
            body.extend(80u16.to_le_bytes());
            body.extend(full_id.as_bytes());
            body.extend(local_id.to_le_bytes());
            body.push(PCODE_PRIM);
            body.extend([0x99; 1 + 4 + 1 + 1 + 12]); // State .. Scale
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
            body.extend([0; 4]); // UpdateFlags
            body.extend(50u16.to_le_bytes());
            body.extend(full_id.as_bytes());
            body.extend(local_id.to_le_bytes());
            body.push(PCODE_PRIM);
            body.extend([0x99; 29]); // what follows
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
        /// For an attachment: the point it is on and its NameValue field.
        pub worn: Option<(u8, String)>,
    }

    /// The name-value an attachment's root prim carries.
    pub fn attach_item_id(item: Uuid) -> String {
        format!("AttachItemID STRING RW SV {item}")
    }

    /// An attachment's root prim in a full update, with a motion.
    pub fn attachment_block(full_id: Uuid, local_id: u32, parent_id: u32, point: u8, item: Uuid) -> Block {
        Block {
            full_id,
            local_id,
            parent_id,
            data: object_data([0.1, 0.0, 0.2], [0.0; 3], false),
            worn: Some((point, attach_item_id(item))),
        }
    }

    /// The optional fields of a compressed update.
    #[derive(Default)]
    pub struct Compressed {
        /// The PCode; a prim's when left out.
        pub pcode: Option<u8>,
        pub state: u8,
        pub angular_velocity: bool,
        pub parent_id: Option<u32>,
        pub tree: bool,
        pub scratch_pad: Option<Vec<u8>>,
        pub text: Option<String>,
        pub media_url: Option<String>,
        pub particles: bool,
        /// The data of each extra parameter.
        pub extra_params: Vec<Vec<u8>>,
        pub sound: bool,
        pub name_values: Option<String>,
    }

    impl Compressed {
        /// An attachment's root prim: a parent, and the item in the
        /// name-values.
        pub fn attachment(parent_id: u32, point: u8, item: Uuid) -> Compressed {
            Compressed {
                state: point.rotate_left(4),
                parent_id: Some(parent_id),
                name_values: Some(attach_item_id(item)),
                ..Compressed::default()
            }
        }
    }

    fn cstring(text: &str) -> Vec<u8> {
        [text.as_bytes(), &[0]].concat()
    }

    fn binary(data: &[u8]) -> Vec<u8> {
        [&(data.len() as u32).to_le_bytes()[..], data].concat()
    }

    /// The Data field of one compressed block, laid out field by field
    /// from llviewerobject.cpp:1783-1942, with 23 bytes of what follows the
    /// name-values (the volume) at the end.
    pub fn compressed_data(full_id: Uuid, local_id: u32, fields: &Compressed) -> Vec<u8> {
        let mut data = full_id.as_bytes().to_vec();
        data.extend(local_id.to_le_bytes());
        data.push(fields.pcode.unwrap_or(PCODE_PRIM));
        data.push(fields.state);
        data.extend([0x99; 4 + 1 + 1 + 12]); // CRC, Material, ClickAction, Scale
        data.extend(vector([1.0, 2.0, 3.0]));
        data.extend(vector([0.0; 3]));
        let flag = |set: bool, bit: u32| if set { bit } else { 0 };
        let flags = flag(fields.scratch_pad.is_some() && !fields.tree, 0x01)
            | flag(fields.tree, 0x02)
            | flag(fields.text.is_some(), 0x04)
            | flag(fields.particles, 0x08)
            | flag(fields.sound, 0x10)
            | flag(fields.parent_id.is_some(), 0x20)
            | flag(fields.angular_velocity, 0x80)
            | flag(fields.name_values.is_some(), 0x100)
            | flag(fields.media_url.is_some(), 0x200);
        data.extend(flags.to_le_bytes());
        data.extend([0x0E; 16]); // OwnerID
        if fields.angular_velocity {
            data.extend([0xA0; 12]);
        }
        if let Some(parent_id) = fields.parent_id {
            data.extend(parent_id.to_le_bytes());
        }
        if fields.tree {
            data.push(0x7E);
        } else if let Some(pad) = &fields.scratch_pad {
            data.extend((pad.len() as u32).to_le_bytes());
            data.extend(binary(pad));
        }
        if let Some(text) = &fields.text {
            data.extend(cstring(text));
            data.extend([0xC0; 4]);
        }
        if let Some(url) = &fields.media_url {
            data.extend(cstring(url));
        }
        if fields.particles {
            data.extend([0xFA; 86]);
        }
        data.push(fields.extra_params.len() as u8);
        for (index, param) in fields.extra_params.iter().enumerate() {
            data.extend((0x10 * (index as u16 + 1)).to_le_bytes());
            data.extend(binary(param));
        }
        if fields.sound {
            data.extend([0x50; 16 + 4 + 1 + 4]);
        }
        if let Some(names) = &fields.name_values {
            data.extend(cstring(names));
        }
        data.extend([0x11; 23]);
        data
    }

    /// An ObjectUpdateCompressed made of `compressed_data` blocks, without
    /// UpdateFlags.
    pub fn object_update_compressed_blocks(blocks: &[Vec<u8>]) -> Vec<u8> {
        let flagged: Vec<(u32, Vec<u8>)> = blocks.iter().map(|data| (0, data.clone())).collect();
        object_update_compressed_flagged(&flagged)
    }

    /// An ObjectUpdateCompressed made of `compressed_data` blocks, each
    /// with its UpdateFlags.
    pub fn object_update_compressed_flagged(blocks: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(blocks.len() as u8);
        for (flags, data) in blocks {
            body.extend(flags.to_le_bytes()); // UpdateFlags
            body.extend((data.len() as u16).to_le_bytes());
            body.extend(data);
        }
        body
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
                worn: None,
            })
            .collect();
        object_blocks(&blocks)
    }

    /// An ObjectUpdate of prims without UpdateFlags.
    pub fn object_blocks(objects: &[Block]) -> Vec<u8> {
        object_blocks_of(objects, &[])
    }

    /// An ObjectUpdate whose blocks have the PCode and UpdateFlags `kinds`
    /// gives for their place; a block past its end is a prim without
    /// flags. Variable fields get assorted lengths so a wrong skip
    /// misaligns the next block.
    pub fn object_blocks_of(objects: &[Block], kinds: &[(u8, u32)]) -> Vec<u8> {
        let mut body = vec![0; 8 + 2];
        body.push(objects.len() as u8);
        for (
            index,
            Block {
                full_id,
                local_id,
                parent_id,
                data,
                worn,
            },
        ) in objects.iter().enumerate()
        {
            let pad = index + 1;
            // Filler where the object is no attachment, as before.
            let (state, name_value) = match worn {
                Some((point, names)) => (point.rotate_left(4), [names.as_bytes(), &[0]].concat()),
                None => (0, vec![0x33; pad * 20]),
            };
            let (pcode, flags) = kinds.get(index).copied().unwrap_or((PCODE_PRIM, 0));
            body.extend(local_id.to_le_bytes()); // ID
            body.push(state);
            body.extend(full_id.as_bytes());
            body.extend([0xAB; 4]); // CRC
            body.push(pcode);
            body.extend([0xAB; 1 + 1 + 12]); // Material, ClickAction, Scale
            body.push(data.len() as u8); // ObjectData
            body.extend(data);
            body.extend(parent_id.to_le_bytes());
            body.extend(flags.to_le_bytes()); // UpdateFlags
            body.extend([0x11; 23]); // PathCurve .. ProfileHollow
            body.extend((pad as u16 * 7).to_le_bytes()); // TextureEntry
            body.extend(vec![0x22; pad * 7]);
            body.push(0); // TextureAnim
            body.extend((name_value.len() as u16).to_le_bytes()); // NameValue
            body.extend(name_value);
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
        assert_eq!(msg.body, [vec![1, 2, 3, 4], vec![0x5E; 16], vec![0xA1; 16]].concat());
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
        assert_eq!(msg.body, [agent_prefix(), vec![0; 9], vec![7], b"Target\0".to_vec()].concat());
    }

    #[test]
    fn encodes_chat_from_viewer() {
        let msg = chat_from_viewer(&agent(), "hi", 2, 0x0102_0304);
        assert_eq!((msg.id, msg.zerocoded), (low(80), true));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![3, 0], b"hi\0".to_vec(), vec![2, 4, 3, 2, 1]].concat()
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
            [1.0f32, 2.0, 3.0].iter().flat_map(|f| f.to_le_bytes()).collect(),
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
    fn automatic_answer_is_in_the_conversation_of_a_plain_message() {
        let to = Uuid::from_bytes([0x0F; 16]);
        let msg = instant_message(&agent(), "Sit Bot", to, [0.0; 3], IM_AUTO_RESPONSE, "Away");
        let Ok(Incoming::InstantMessage { dialog, id, text, .. }) = decode(msg.id, &msg.body) else {
            panic!("not an instant message")
        };
        assert_eq!((dialog, id, text.as_str()), (20, Uuid::from_bytes([0xA1 ^ 0x0F; 16]), "Away"));
    }

    #[test]
    fn teleport_request_is_an_instant_message_without_a_conversation() {
        let to = Uuid::from_bytes([0x0F; 16]);
        let msg = instant_message(&agent(), "Sit Bot", to, [0.0; 3], IM_TELEPORT_REQUEST, "may I?");
        let Ok(Incoming::InstantMessage { dialog, id, text, .. }) = decode(msg.id, &msg.body) else {
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
            [agent_prefix(), vec![0, 5], b"come\0".to_vec(), vec![1], vec![0x0F; 16]].concat()
        );

        let msg = teleport_lure_request(&agent(), Uuid::from_bytes([0x44; 16]));
        assert_eq!((msg.id, msg.zerocoded), (low(71), false));
        assert_eq!(msg.body, [agent_prefix(), vec![0x44; 16], vec![4, 0, 0, 0]].concat());
    }

    #[test]
    fn encodes_a_touch() {
        let msg = object_grab(&agent(), 0x0403_0201);
        assert_eq!((msg.id, msg.zerocoded), (low(117), true));
        assert_eq!(msg.body, [agent_prefix(), vec![1, 2, 3, 4], vec![0; 12], vec![0]].concat());
        let msg = object_degrab(&agent(), 0x0403_0201);
        assert_eq!((msg.id, msg.zerocoded), (low(119), false));
        assert_eq!(msg.body, [agent_prefix(), vec![1, 2, 3, 4], vec![0]].concat());
    }

    #[test]
    fn encodes_request_multiple_objects() {
        let msg = request_multiple_objects(&agent(), &[7, 0x0102]);
        assert_eq!((msg.id, msg.zerocoded), (medium(3), true));
        assert_eq!(msg.body, [agent_prefix(), vec![2, 0, 7, 0, 0, 0, 0, 2, 1, 0, 0]].concat());
    }

    #[test]
    fn encodes_rez_single_attachment_from_inv() {
        let item = AttachItem {
            item_id: Uuid::from_bytes([0x17; 16]),
            owner_id: Uuid::from_bytes([0xA1; 16]),
            flags: 0x0000_0105,
            group_mask: 0x0102_0304,
            everyone_mask: 0x0008_0000,
            next_owner_mask: 0x7FFF_FFFF,
            name: "Radio".into(),
            description: "plays".into(),
        };
        let msg = rez_single_attachment_from_inv(&agent(), &item, 5);
        assert_eq!((msg.id, msg.zerocoded), (low(395), true));
        let expected = [
            agent_prefix(),
            vec![0x17; 16],
            vec![0xA1; 16],
            vec![0x85], // left hand, with the add bit
            vec![5, 1, 0, 0],
            vec![4, 3, 2, 1],
            vec![0, 0, 8, 0],
            vec![0xFF, 0xFF, 0xFF, 0x7F],
            vec![6],
            b"Radio\0".to_vec(),
            vec![6],
            b"plays\0".to_vec(),
        ]
        .concat();
        assert_eq!(msg.body, expected);
        // Without a point the simulator chooses, and still adds.
        let msg = rez_single_attachment_from_inv(&agent(), &item, 0);
        assert_eq!(msg.body[64], 0x80);
        // An empty description is a lone NUL.
        let bare = AttachItem {
            name: "x".into(),
            ..AttachItem::default()
        };
        let msg = rez_single_attachment_from_inv(&agent(), &bare, 1);
        assert_eq!(msg.body[64 + 1 + 16..], [2, b'x', 0, 1, 0]);
    }

    #[test]
    fn encodes_detach_attachment_into_inv() {
        let msg = detach_attachment_into_inv(&agent(), Uuid::from_bytes([0x17; 16]));
        assert_eq!((msg.id, msg.zerocoded), (low(397), false));
        // The agent id and the item id, and no session id.
        assert_eq!(msg.body, [vec![0xA1; 16], vec![0x17; 16]].concat());
    }

    #[test]
    fn encodes_agent_animation() {
        let animation = Uuid::from_bytes([0x2A; 16]);
        let start = agent_animation(&agent(), animation, true);
        assert_eq!((start.id, start.zerocoded), (high(5), false));
        assert_eq!(start.body, [agent_prefix(), vec![1], vec![0x2A; 16], vec![1], vec![0]].concat());
        let stop = agent_animation(&agent(), animation, false);
        assert_eq!(stop.body, [agent_prefix(), vec![1], vec![0x2A; 16], vec![0], vec![0]].concat());
    }

    #[test]
    fn encodes_sound_trigger() {
        let msg = sound_trigger(Uuid::from_bytes([0x50; 16]), 0x0003_E800_0003_E900, [1.0, 2.0, 3.0], 1.0);
        assert_eq!((msg.id, msg.zerocoded), (high(29), false));
        let expected = [
            vec![0x50; 16],
            vec![0; 48], // OwnerID, ObjectID, ParentID
            vec![0, 0xE9, 3, 0, 0, 0xE8, 3, 0],
            [1.0f32, 2.0, 3.0, 1.0].iter().flat_map(|f| f.to_le_bytes()).collect(),
        ]
        .concat();
        assert_eq!(msg.body, expected);
    }

    #[test]
    fn decodes_the_other_object_messages() {
        let (a, b) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let body = object_update_compressed(&[(a, 500), (b, 501)]);
        let plain = |full_id, local_id| CompressedObject {
            full_id,
            local_id,
            transform: None,
            parent_id: None,
            attachment: None,
            pcode: PCODE_PRIM,
            flags: 0,
        };
        assert_eq!(
            decode(high(13), &body),
            Ok(Incoming::ObjectsCompressed(vec![plain(a, 500), plain(b, 501)]))
        );
        assert_eq!(decode(high(13), &body[..body.len() - 40]), Err(DecodeError));

        let body = object_update_cached(&[500, 501]);
        assert_eq!(decode(high(14), &body), Ok(Incoming::ObjectsCached(vec![500, 501])));
        assert_eq!(decode(high(14), &body[..body.len() - 1]), Err(DecodeError));

        let body = kill_object(&[500, 501]);
        assert_eq!(decode(high(16), &body), Ok(Incoming::ObjectsKilled(vec![500, 501])));
        assert_eq!(decode(high(16), &body[..body.len() - 1]), Err(DecodeError));
    }

    #[test]
    fn encodes_avatar_picker_request() {
        let query = Uuid::from_bytes([0x33; 16]);
        let msg = avatar_picker_request(&agent(), query, "jane doe");
        assert_eq!((msg.id, msg.zerocoded), (low(26), false));
        assert_eq!(msg.body, [agent_prefix(), vec![0x33; 16], vec![9], b"jane doe\0".to_vec()].concat());
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
                avatars: vec![(jane, "Jane Doe".to_string()), (other, "Jane Doester".to_string())],
            })
        );
        assert_eq!(decode(low(28), &body[..body.len() - 1]), Err(DecodeError));
        let none = avatar_picker_reply(query, &[]);
        assert_eq!(decode(low(28), &none), Ok(Incoming::AvatarPickerReply { query, avatars: vec![] }));
    }

    #[test]
    fn decodes_coarse_location_update() {
        let ids = [Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16])];
        let locations = [[200, 40, 6], [10, 20, 255], [0, 255, 0]];
        let body = coarse_location_update(&locations, Some(&ids));
        let wanted: Vec<(Uuid, [u8; 3])> = ids.into_iter().zip(locations).collect();
        assert_eq!(decode(medium(6), &body), Ok(Incoming::AvatarsHere(wanted.clone())));
        // Without the ids, nobody; with fewer ids, as many as there are.
        let bare = coarse_location_update(&locations, None);
        assert_eq!(decode(medium(6), &bare), Ok(Incoming::AvatarsHere(vec![])));
        let short = coarse_location_update(&locations, Some(&ids[..2]));
        assert_eq!(decode(medium(6), &short), Ok(Incoming::AvatarsHere(wanted[..2].to_vec())));
        for cut in [1, 5, 16] {
            assert_eq!(decode(medium(6), &body[..body.len() - cut]), Err(DecodeError));
        }
        assert_eq!(decode(medium(6), &[]), Err(DecodeError));
    }

    #[test]
    fn decodes_online_and_offline_notifications() {
        let ids = [Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16])];
        let body = friend_notification(&ids);
        assert_eq!(decode(low(322), &body), Ok(Incoming::FriendsOnline(ids.to_vec())));
        assert_eq!(decode(low(323), &body), Ok(Incoming::FriendsOffline(ids.to_vec())));
        assert_eq!(decode(low(323), &body[..body.len() - 1]), Err(DecodeError));
        assert_eq!(
            (name(low(322)), name(low(323))),
            ("OnlineNotification".into(), "OfflineNotification".into())
        );
    }

    #[test]
    fn decodes_terminate_friendship() {
        let jane = Uuid::from_bytes([0x1A; 16]);
        let body = terminate_friendship(jane);
        assert_eq!(body.len(), 48);
        assert_eq!(decode(low(300), &body), Ok(Incoming::FriendshipEnded(jane)));
        assert_eq!(decode(low(300), &body[..47]), Err(DecodeError));
        assert_eq!(name(low(300)), "TerminateFriendship");
    }

    #[test]
    fn decodes_change_user_rights() {
        let (jane, bot) = (Uuid::from_bytes([0x1A; 16]), Uuid::from_bytes([0xA1; 16]));
        let body = change_user_rights(jane, &[(bot, 1), (jane, -2)]);
        assert_eq!(body.len(), 16 + 1 + 2 * 20);
        assert_eq!(
            decode(low(321), &body),
            Ok(Incoming::RightsChanged {
                agent: jane,
                rights: vec![(bot, 1), (jane, -2)],
            })
        );
        assert_eq!(decode(low(321), &body[..body.len() - 1]), Err(DecodeError));
        assert_eq!(name(low(321)), "ChangeUserRights");
    }

    #[test]
    fn encodes_avatar_properties_request() {
        let msg = avatar_properties_request(&agent(), Uuid::from_bytes([0x1A; 16]));
        assert_eq!((msg.id, msg.zerocoded), (low(169), false));
        assert_eq!(msg.body, [agent_prefix(), vec![0x1A; 16]].concat());
    }

    #[test]
    fn decodes_avatar_properties_reply() {
        let jane = Uuid::from_bytes([0x1A; 16]);
        let about = "x".repeat(600);
        let body = avatar_properties_reply(jane, Uuid::nil(), &about, "", "3/14/2010", 0x10 | 0x01);
        assert_eq!(
            decode(low(171), &body),
            Ok(Incoming::AvatarProperties {
                avatar: jane,
                partner: Uuid::nil(),
                about,
                first_life: String::new(),
                born: "3/14/2010".to_string(),
                flags: 0x11,
            })
        );
        assert_eq!(decode(low(171), &body[..body.len() - 1]), Err(DecodeError));
        assert_eq!(name(low(171)), "AvatarPropertiesReply");
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
        assert_eq!(request.body, [agent_prefix(), vec![0x0B; 16], vec![0; 12]].concat());
        let sit = agent_sit(&agent());
        assert_eq!((sit.id, sit.body), (high(7), agent_prefix()));
    }

    #[test]
    fn decodes_ping_and_acks() {
        assert_eq!(decode(high(1), &[7, 1, 0, 0, 0]), Ok(Incoming::StartPingCheck { ping_id: 7 }));
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
            Ok(Incoming::RegionHandshake { sim_name: "Target".into() })
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
            Ok(Incoming::TeleportLocal { position: [4.0, 5.0, 6.0] })
        );
    }

    #[test]
    fn decodes_avatar_sit_response() {
        let object = Uuid::from_bytes([0x0B; 16]);
        assert_eq!(
            decode(high(21), &avatar_sit_response(object, true)),
            Ok(Incoming::AvatarSitResponse { object, autopilot: true })
        );
    }

    #[test]
    fn decodes_kick_logout_and_disable() {
        assert_eq!(decode(low(163), &kick_user("Bye")), Ok(Incoming::KickUser { reason: "Bye".into() }));
        assert_eq!(decode(low(253), &[0; 33]), Ok(Incoming::LogoutReply));
        assert_eq!(decode(low(152), &[]), Ok(Incoming::DisableSimulator));
    }

    #[test]
    fn decodes_both_alert_messages() {
        let alert = Incoming::Alert {
            message: "No suitable surface".into(),
        };
        assert_eq!(decode(low(134), &alert_message("No suitable surface")), Ok(alert));
        let alert = Incoming::Alert {
            message: "Region restarting".into(),
        };
        assert_eq!(decode(low(135), &agent_alert_message("Region restarting")), Ok(alert));
    }

    #[test]
    fn encodes_retrieve_instant_messages() {
        let msg = retrieve_instant_messages(&agent());
        assert_eq!((msg.id, msg.zerocoded, msg.body), (low(255), false, agent_prefix()));
    }

    #[test]
    fn decodes_chat_from_simulator() {
        let (avatar, object, owner) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16]));
        assert_eq!(
            decode(low(139), &chat_from_simulator("Jane Doe", (avatar, avatar, 1), 1, 1, "hello")),
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

    fn group_agent() -> Agent {
        Agent {
            agent_id: Uuid::from_bytes([0xA1; 16]),
            session_id: Uuid::from_bytes([0x5E; 16]),
        }
    }

    #[test]
    fn group_requests_have_their_templates_layout() {
        let agent = group_agent();
        let (group, jane) = (Uuid::from_bytes([0xB1; 16]), Uuid::from_bytes([0x33; 16]));
        let head = [agent.agent_id.as_bytes().as_slice(), agent.session_id.as_bytes()].concat();
        let request = agent_data_update_request(&agent);
        assert_eq!((request.id, request.zerocoded, &request.body), (low(386), false, &head));
        let activate = activate_group(&agent, group);
        assert_eq!((activate.id, activate.zerocoded), (low(368), true));
        assert_eq!(activate.body, [head.as_slice(), group.as_bytes()].concat());
        assert_eq!(activate_group(&agent, Uuid::nil()).body[32..], [0; 16]);
        let invite = invite_group_request(&agent, group, jane);
        assert_eq!((invite.id, invite.zerocoded), (low(349), false));
        let mut wanted = head.clone();
        wanted.extend(group.as_bytes());
        wanted.push(1);
        wanted.extend(jane.as_bytes());
        wanted.extend([0; 16]); // Everyone
        assert_eq!(invite.body, wanted);
        let profile = group_profile_request(&agent, group);
        assert_eq!((profile.id, profile.zerocoded), (low(351), false));
        assert_eq!(profile.body, [head.as_slice(), group.as_bytes()].concat());
        let join = join_group_request(&agent, group);
        assert_eq!((join.id, join.zerocoded), (low(343), true));
        assert_eq!(join.body, [head.as_slice(), group.as_bytes()].concat());
        let leave = leave_group_request(&agent, group);
        assert_eq!((leave.id, leave.zerocoded), (low(347), false));
        assert_eq!(leave.body, [head.as_slice(), group.as_bytes()].concat());
        for id in [
            low(343),
            low(344),
            low(347),
            low(348),
            low(349),
            low(351),
            low(352),
            low(368),
            low(386),
            low(387),
            low(389),
            low(390),
        ] {
            assert!(!name(id).starts_with("Low "), "{id:#x} has no name");
        }
    }

    #[test]
    fn a_group_instant_message_goes_to_the_group() {
        let agent = group_agent();
        let (group, invitation) = (Uuid::from_bytes([0xB1; 16]), Uuid::from_bytes([0x1D; 16]));
        let start = group_instant_message(&agent, "Sit Bot", group, [1.0, 2.0, 3.0], IM_SESSION_GROUP_START, group, "");
        assert_eq!((start.id, start.zerocoded), (IMPROVED_INSTANT_MESSAGE, true));
        // As the simulator delivers it, the recipient and id are the group.
        let Ok(Incoming::InstantMessage {
            from_agent,
            from_group,
            to,
            dialog,
            id,
            text,
            bucket,
            from_name,
            ..
        }) = decode(IMPROVED_INSTANT_MESSAGE, &start.body)
        else {
            panic!("not an instant message")
        };
        assert_eq!((from_agent, from_group, to, dialog, id), (agent.agent_id, false, group, 15, group));
        assert_eq!((from_name.as_str(), text.as_str(), bucket), ("Sit Bot", "", vec![0]));
        let line = group_instant_message(&agent, "Sit Bot", group, [0.0; 3], IM_SESSION_SEND, group, "hello");
        assert!(matches!(
            decode(IMPROVED_INSTANT_MESSAGE, &line.body),
            Ok(Incoming::InstantMessage { dialog: 17, ref text, .. }) if text == "hello"
        ));
        for (dialog, wanted) in [(IM_GROUP_INVITATION_ACCEPT, 35), (IM_GROUP_INVITATION_DECLINE, 36)] {
            let answer = group_instant_message(&agent, "name", group, [0.0; 3], dialog, invitation, "message");
            let Ok(Incoming::InstantMessage {
                to,
                dialog,
                id,
                from_name,
                text,
                ..
            }) = decode(IMPROVED_INSTANT_MESSAGE, &answer.body)
            else {
                panic!("not an instant message")
            };
            assert_eq!((to, dialog, id), (group, wanted, invitation));
            assert_eq!((from_name.as_str(), text.as_str()), ("name", "message"));
        }
        assert_eq!((IM_GROUP_INVITATION, IM_GROUP_NOTICE), (3, 32));
    }

    #[test]
    fn decodes_the_groups_of_the_avatar() {
        use crate::groups::Membership;
        let agent = Uuid::from_bytes([0xA1; 16]);
        let groups = vec![
            Membership {
                id: Uuid::from_bytes([0xB1; 16]),
                name: "Builders".into(),
                powers: 0x0000_0001_0000_0002,
                notices: true,
            },
            Membership {
                id: Uuid::from_bytes([0xD1; 16]),
                name: "Dancers".into(),
                powers: 0,
                notices: false,
            },
        ];
        let body = agent_group_data_update(agent, &groups);
        assert_eq!(
            decode(AGENT_GROUP_DATA_UPDATE, &body),
            Ok(Incoming::AgentGroupDataUpdate(groups.clone()))
        );
        assert_eq!(
            decode(AGENT_GROUP_DATA_UPDATE, &agent_group_data_update(agent, &[])),
            Ok(Incoming::AgentGroupDataUpdate(vec![]))
        );
        assert_eq!(decode(AGENT_GROUP_DATA_UPDATE, &body[..40]), Err(DecodeError));
        assert_eq!(
            decode(AGENT_DROP_GROUP, &agent_drop_group(agent, groups[1].id)),
            Ok(Incoming::AgentDropGroup(groups[1].id))
        );
        assert_eq!(
            decode(
                AGENT_DATA_UPDATE,
                &agent_data_update(agent, groups[0].id, "Master builder", "Builders")
            ),
            Ok(Incoming::AgentDataUpdate {
                active: groups[0].id,
                title: "Master builder".into(),
                group_name: "Builders".into(),
            })
        );
        assert_eq!(
            decode(AGENT_DATA_UPDATE, &agent_data_update(agent, Uuid::nil(), "", "")),
            Ok(Incoming::AgentDataUpdate {
                active: Uuid::nil(),
                title: String::new(),
                group_name: String::new(),
            })
        );
    }

    #[test]
    fn decodes_a_groups_profile_and_a_join() {
        let agent = Uuid::from_bytes([0xA1; 16]);
        let profile = crate::groups::Profile {
            id: Uuid::from_bytes([0xB1; 16]),
            name: "Builders".into(),
            charter: "We build.\nTogether.".into(),
            listed: true,
            title: "Officer".into(),
            founder: Uuid::from_bytes([0x33; 16]),
            fee: 250,
            open: true,
            members: 12,
            roles: 3,
            mature: false,
        };
        let body = group_profile_reply(agent, &profile);
        assert_eq!(decode(GROUP_PROFILE_REPLY, &body), Ok(Incoming::GroupProfileReply(profile.clone())));
        assert_eq!(decode(GROUP_PROFILE_REPLY, &body[..body.len() - 20]), Err(DecodeError));
        assert_eq!(
            decode(JOIN_GROUP_REPLY, &join_group_reply(agent, profile.id, true)),
            Ok(Incoming::JoinGroupReply {
                group: profile.id,
                success: true
            })
        );
        assert_eq!(
            decode(JOIN_GROUP_REPLY, &join_group_reply(agent, profile.id, false)),
            Ok(Incoming::JoinGroupReply {
                group: profile.id,
                success: false
            })
        );
        assert_eq!(
            decode(LEAVE_GROUP_REPLY, &leave_group_reply(agent, profile.id, false)),
            Ok(Incoming::LeaveGroupReply {
                group: profile.id,
                success: false
            })
        );
        assert_eq!(decode(LEAVE_GROUP_REPLY, &[0; 32]), Err(DecodeError));
    }

    #[test]
    fn an_instant_message_keeps_its_recipient() {
        let (group, jane) = (Uuid::from_bytes([0xB1; 16]), Uuid::from_bytes([0x33; 16]));
        let body = group_im(jane, false, group, "Jane Doe", IM_SESSION_SEND, group, "hello", b"Builders\0");
        let Ok(Incoming::InstantMessage { to, from_agent, id, .. }) = decode(IMPROVED_INSTANT_MESSAGE, &body) else {
            panic!("not an instant message")
        };
        assert_eq!((to, from_agent, id), (group, jane, group));
    }

    fn rig_uuid(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    #[test]
    fn decodes_the_group_messages_captured_on_the_rig() {
        let free = rig_uuid("4328d03e-69ce-4e18-81f4-ccf63ad9607e");
        let closed = rig_uuid("bb72b94c-49d2-4ec7-a3c3-289e6f828c78");
        let (owner, alt) = (
            rig_uuid("7e570000-0000-4000-8000-0000000000aa"),
            rig_uuid("7e570000-0000-4000-8000-0000000000a1"),
        );
        assert_eq!(
            decode(
                AGENT_DATA_UPDATE,
                &captured_bytes("AgentDataUpdate (Low 387), UDP: Alt's active group")
            ),
            Ok(Incoming::AgentDataUpdate {
                active: free,
                title: "Member of Free".into(),
                group_name: "Free".into(),
            })
        );
        assert_eq!(
            decode(
                AGENT_DATA_UPDATE,
                &captured_bytes("AgentDataUpdate (Low 387), UDP: no active group")
            ),
            Ok(Incoming::AgentDataUpdate {
                active: Uuid::nil(),
                title: String::new(),
                group_name: String::new(),
            })
        );
        let Ok(Incoming::InstantMessage {
            from_agent,
            from_group,
            to,
            from_name,
            dialog,
            id,
            text,
            ..
        }) = decode(
            IMPROVED_INSTANT_MESSAGE,
            &captured_bytes("ImprovedInstantMessage (Low 254), UDP, dialog 17:"),
        )
        else {
            panic!("not an instant message")
        };
        // OpenSim marks a group's line as from the group, from its speaker.
        assert_eq!((from_agent, from_group, to, dialog, id), (owner, true, alt, IM_SESSION_SEND, free));
        assert_eq!((from_name.as_str(), text.as_str()), ("Rig Owner", "capture line two"));
        let Ok(Incoming::InstantMessage {
            from_agent,
            from_group,
            dialog,
            text,
            bucket,
            ..
        }) = decode(
            IMPROVED_INSTANT_MESSAGE,
            &captured_bytes("ImprovedInstantMessage (Low 254), UDP, dialog 32:"),
        )
        else {
            panic!("not an instant message")
        };
        // OpenSim sends a notice from the group, the sender by name only.
        assert_eq!((from_agent, from_group, dialog), (free, true, IM_GROUP_NOTICE));
        assert_eq!(Uuid::from_slice(&bucket[2..18]).unwrap(), free);
        let notice = crate::grouplog::notice(&text, &bucket);
        assert_eq!(
            (notice.subject.as_str(), notice.text.as_str(), notice.attachment.as_str()),
            ("Meeting", "Tonight at eight", "Agenda")
        );
        let Ok(Incoming::InstantMessage {
            from_agent,
            dialog,
            id,
            bucket,
            from_name,
            ..
        }) = decode(
            IMPROVED_INSTANT_MESSAGE,
            &captured_bytes("ImprovedInstantMessage (Low 254), UDP, dialog 3:"),
        )
        else {
            panic!("not an instant message")
        };
        assert_eq!((from_agent, dialog, from_name.as_str()), (closed, IM_GROUP_INVITATION, "Rig Owner"));
        assert!(!id.is_nil());
        // The fee, nothing for Groups V2, and the role.
        assert_eq!((bucket.len(), &bucket[..4]), (20, &[0u8; 4][..]));
        assert_eq!(
            decode(AGENT_DROP_GROUP, &captured_bytes("AgentDropGroup (Low 390)")),
            Ok(Incoming::AgentDropGroup(free))
        );
        let Ok(Incoming::GroupProfileReply(profile)) = decode(GROUP_PROFILE_REPLY, &captured_bytes("GroupProfileReply (Low 352)")) else {
            panic!("not a profile")
        };
        assert_eq!((profile.id, profile.name.as_str(), profile.founder), (free, "Free", owner));
        assert_eq!((profile.fee, profile.open, profile.members, profile.roles), (0, true, 3, 3));
        assert_eq!(
            (profile.title.as_str(), profile.charter.as_str(), profile.listed),
            ("Member of Free", "A group of the rig.", true)
        );
        assert_eq!(
            decode(JOIN_GROUP_REPLY, &captured_bytes("JoinGroupReply (Low 344)")),
            Ok(Incoming::JoinGroupReply {
                group: free,
                success: true
            })
        );
        let leavers = Uuid::parse_str("e5fb4ab5-efcb-4c4a-9b58-656d65bbaaf1").unwrap();
        assert_eq!(
            decode(LEAVE_GROUP_REPLY, &captured_bytes("LeaveGroupReply (Low 348)")),
            Ok(Incoming::LeaveGroupReply {
                group: leavers,
                success: true
            })
        );
    }

    #[test]
    fn decodes_improved_instant_message() {
        let (sender, session, object) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16]));
        let im = |dialog, id, offline, timestamp, from_name: &str, text: &str| Incoming::InstantMessage {
            from_agent: sender,
            from_group: false,
            to: Uuid::nil(),
            from_name: from_name.into(),
            dialog,
            id,
            offline,
            timestamp,
            text: text.into(),
            bucket: vec![0],
        };
        assert_eq!(
            decode(low(254), &improved_instant_message(sender, "Jane Doe", 0, session, None, "hi")),
            Ok(im(0, session, false, 0, "Jane Doe", "hi"))
        );
        assert_eq!(
            decode(low(254), &improved_instant_message(sender, "Clock", 19, object, None, "done")),
            Ok(im(19, object, false, 0, "Clock", "done"))
        );
        let body = improved_instant_message(sender, "Jane Doe", 0, session, Some(1_700_000_000), "later");
        assert_eq!(
            decode(low(254), &body),
            Ok(im(0, session, true, 1_700_000_000, "Jane Doe", "later"))
        );
        assert_eq!(decode(low(254), &body[..80]), Err(DecodeError));
    }

    #[test]
    fn decodes_the_group_flag_and_the_bucket_of_an_inventory_offer() {
        let (owner, transaction, item) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16]));
        // An avatar's offer: the asset type and the item.
        let bucket = [vec![7], item.as_bytes().to_vec()].concat();
        let body = instant_message_with_bucket(owner, false, "Jane Doe", 4, transaction, None, "Rules", &bucket);
        let Ok(Incoming::InstantMessage {
            from_group,
            dialog,
            id,
            bucket: read,
            ..
        }) = decode(low(254), &body)
        else {
            panic!("not an instant message")
        };
        assert_eq!((from_group, dialog, id, read), (false, 4, transaction, bucket));
        // A group's object: the type alone, and the group as the sender.
        let body = instant_message_with_bucket(owner, true, "Vendor", 9, transaction, None, "'Rules'", &[7]);
        let Ok(Incoming::InstantMessage { from_group, bucket, .. }) = decode(low(254), &body) else {
            panic!("not an instant message")
        };
        assert_eq!((from_group, bucket), (true, vec![7]));
    }

    #[test]
    fn decodes_script_question_with_and_without_the_experience_block() {
        let (object, item) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        for experience in [false, true] {
            let body = script_question(object, item, "Dance Ball", "Jane Doe", 0x8010, experience);
            assert_eq!(
                decode(low(188), &body),
                Ok(Incoming::ScriptQuestion {
                    object,
                    item,
                    object_name: "Dance Ball".into(),
                    owner_name: "Jane Doe".into(),
                    questions: 0x8010,
                })
            );
        }
        let body = script_question(object, item, "Dance Ball", "Jane Doe", 0x10, false);
        assert_eq!(decode(low(188), &body[..body.len() - 1]), Err(DecodeError));
    }

    #[test]
    fn decodes_object_properties_family() {
        let (object, owner, group) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16]));
        let body = object_properties_family(object, owner, group);
        assert_eq!(
            decode(medium(10), &body),
            Ok(Incoming::ObjectPropertiesFamily {
                object,
                owner,
                group,
                named: Some(("Dance Ball".to_string(), String::new())),
            })
        );
        assert_eq!(decode(medium(10), &body[..40]), Err(DecodeError));

        let body = object_properties_family_named(object, owner, group, "Park Bench", "Sit here");
        assert_eq!(
            decode(medium(10), &body),
            Ok(Incoming::ObjectPropertiesFamily {
                object,
                owner,
                group,
                named: Some(("Park Bench".to_string(), "Sit here".to_string())),
            })
        );
        // An answer that ends after the ids still says whose the object is.
        assert_eq!(
            decode(medium(10), &body[..4 + 3 * 16 + 10]),
            Ok(Incoming::ObjectPropertiesFamily {
                object,
                owner,
                group,
                named: None,
            })
        );
    }

    #[test]
    fn decodes_object_properties() {
        let bench = Properties {
            object: Uuid::from_bytes([1; 16]),
            owner: Uuid::from_bytes([2; 16]),
            group: Uuid::from_bytes([3; 16]),
            name: "Park Bench".to_string(),
            description: "Sit here".to_string(),
        };
        let lamp = Properties {
            object: Uuid::from_bytes([4; 16]),
            owner: Uuid::nil(),
            group: Uuid::from_bytes([5; 16]),
            name: "Lamp".to_string(),
            description: String::new(),
        };
        let body = object_properties(&[bench.clone(), lamp.clone()]);
        assert_eq!(decode(medium(9), &body), Ok(Incoming::ObjectProperties(vec![bench, lamp])));
        assert_eq!(decode(medium(9), &body[..body.len() - 1]), Err(DecodeError));
        assert_eq!(decode(medium(9), &[0]), Ok(Incoming::ObjectProperties(vec![])));
    }

    #[test]
    fn encodes_a_selection_and_its_end() {
        let msg = object_select(&agent(), &[7, 0x0102]);
        assert_eq!((msg.id, msg.zerocoded), (low(110), true));
        assert_eq!(msg.body, [agent_prefix(), vec![2, 7, 0, 0, 0, 2, 1, 0, 0]].concat());
        let msg = object_deselect(&agent(), &[7]);
        assert_eq!((msg.id, msg.zerocoded), (low(111), true));
        assert_eq!(msg.body, [agent_prefix(), vec![1, 7, 0, 0, 0]].concat());

        let ids: Vec<u32> = (1..=SELECT_BATCH as u32).collect();
        let msg = object_select(&agent(), &ids);
        assert_eq!(msg.body.len(), 32 + 1 + 255 * 4);
        assert_eq!(msg.body[32], 255);
        assert_eq!(msg.body[msg.body.len() - 4..], [255, 0, 0, 0]);
    }

    #[test]
    fn object_updates_say_what_kind_of_object_and_its_flags() {
        let (bench, jane) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let block = |full_id, local_id| Block {
            full_id,
            local_id,
            parent_id: 0,
            data: vec![0xCD; 32],
            worn: None,
        };
        let kinds = [(PCODE_PRIM, FLAG_SCRIPTED | FLAG_TOUCH), (PCODE_AVATAR, 0)];
        let body = object_blocks_of(&[block(bench, 100), block(jane, 101)], &kinds);
        let Ok(Incoming::ObjectUpdate(objects)) = decode(high(12), &body) else {
            panic!("not an object update");
        };
        let read: Vec<(u8, u32)> = objects.iter().map(|object| (object.pcode, object.flags)).collect();
        assert_eq!(read, kinds);

        let avatar = Compressed {
            pcode: Some(PCODE_AVATAR),
            ..Compressed::default()
        };
        let blocks = [
            (FLAG_PHYSICS | FLAG_TEMPORARY, compressed_data(bench, 100, &Compressed::default())),
            (0, compressed_data(jane, 101, &avatar)),
        ];
        let Ok(Incoming::ObjectsCompressed(objects)) = decode(high(13), &object_update_compressed_flagged(&blocks)) else {
            panic!("not a compressed update");
        };
        let read: Vec<(u8, u32)> = objects.iter().map(|object| (object.pcode, object.flags)).collect();
        assert_eq!(read, [(PCODE_PRIM, FLAG_PHYSICS | FLAG_TEMPORARY), (PCODE_AVATAR, 0)]);
    }

    #[test]
    fn encodes_script_answer_and_owner_request() {
        let (object, item) = (Uuid::from_bytes([0x0B; 16]), Uuid::from_bytes([0x0C; 16]));
        let msg = script_answer_yes(&agent(), object, item, 0x14);
        assert_eq!((msg.id, msg.zerocoded), (low(132), false));
        assert_eq!(
            msg.body,
            [agent_prefix(), vec![0x0B; 16], vec![0x0C; 16], vec![0x14, 0, 0, 0]].concat()
        );
        let msg = request_object_properties_family(&agent(), object);
        assert_eq!((msg.id, msg.zerocoded), (medium(5), true));
        assert_eq!(msg.body, [agent_prefix(), vec![0; 4], vec![0x0B; 16]].concat());
    }

    #[test]
    fn an_accepted_inventory_offer_names_the_transaction_and_the_folder() {
        let (to, transaction, folder) = (
            Uuid::from_bytes([0x0F; 16]),
            Uuid::from_bytes([0x0D; 16]),
            Uuid::from_bytes([0x0E; 16]),
        );
        for (offered, accepted) in [(IM_INVENTORY_OFFERED, 5), (IM_TASK_INVENTORY_OFFERED, 10)] {
            let msg = inventory_accepted(&agent(), "Sit Bot", to, [1.0, 2.0, 3.0], offered, transaction, folder);
            assert_eq!((msg.id, msg.zerocoded), (low(254), true));
            let expected = [
                agent_prefix(),
                vec![0],         // FromGroup
                vec![0x0F; 16],  // ToAgentID
                vec![0; 4 + 16], // ParentEstateID, RegionID
                [1.0f32, 2.0, 3.0].iter().flat_map(|f| f.to_le_bytes()).collect(),
                vec![0, accepted], // Offline, Dialog
                vec![0x0D; 16],    // ID
                vec![0; 4],        // Timestamp
                vec![8],
                b"Sit Bot\0".to_vec(),
                vec![1, 0, 0], // Message
                vec![16, 0],   // BinaryBucket
                vec![0x0E; 16],
                vec![0, 0, 0, 0, 0], // EstateID, MetaData block count
            ]
            .concat();
            assert_eq!(msg.body, expected);
        }
    }

    #[test]
    fn decodes_object_update_with_several_blocks() {
        let ids = [Uuid::from_bytes([1; 16]), agent().agent_id, Uuid::from_bytes([3; 16])];
        let body = object_update(&[(ids[0], 0), (ids[1], 4242), (ids[2], 77)]);
        assert_eq!(
            decode(high(12), &body),
            Ok(Incoming::ObjectUpdate(vec![
                ObjectParent {
                    full_id: ids[0],
                    local_id: 100,
                    parent_id: 0,
                    transform: None,
                    state: 0,
                    attach_item: None,
                    pcode: PCODE_PRIM,
                    flags: 0,
                },
                ObjectParent {
                    full_id: ids[1],
                    local_id: 101,
                    parent_id: 4242,
                    transform: None,
                    state: 0,
                    attach_item: None,
                    pcode: PCODE_PRIM,
                    flags: 0,
                },
                ObjectParent {
                    full_id: ids[2],
                    local_id: 102,
                    parent_id: 77,
                    transform: None,
                    state: 0,
                    attach_item: None,
                    pcode: PCODE_PRIM,
                    flags: 0,
                },
            ]))
        );
    }

    /// A quarter turn about the vertical axis, as the x, y and z of its
    /// quaternion and in full.
    const QUARTER_TURN: (Vector3, Quaternion) = (
        [0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2],
        [0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2],
    );

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    #[test]
    fn object_update_carries_position_and_rotation() {
        let (prim, avatar, old) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16]));
        let body = object_blocks(&[
            Block {
                full_id: prim,
                local_id: 7,
                parent_id: 0,
                data: object_data([131.0, 64.0, 23.0], QUARTER_TURN.0, false),
                worn: None,
            },
            Block {
                full_id: avatar,
                local_id: 8,
                parent_id: 7,
                data: object_data([0.5, 0.0, 0.75], [0.0; 3], true),
                worn: None,
            },
            // A quantized form: described, but without a motion.
            Block {
                full_id: old,
                local_id: 9,
                parent_id: 0,
                data: vec![0x12; 32],
                worn: None,
            },
        ]);
        let Ok(Incoming::ObjectUpdate(objects)) = decode(high(12), &body) else {
            panic!("not an object update")
        };
        assert_eq!(
            objects.iter().map(|o| (o.full_id, o.local_id, o.parent_id)).collect::<Vec<_>>(),
            [(prim, 7, 0), (avatar, 8, 7), (old, 9, 0)]
        );
        let seat = objects[0].transform.unwrap();
        assert_eq!(seat.position, [131.0, 64.0, 23.0]);
        assert!(close(&seat.rotation, &QUARTER_TURN.1), "{:?}", seat.rotation);
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
        assert_eq!(transform_from_motion(&prim).unwrap().position, [1.0, 2.0, 3.0]);
        assert_eq!(transform_from_motion(&avatar).unwrap().position, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn positions_that_are_not_numbers_are_dropped() {
        assert_eq!(transform_from_motion(&object_data([f32::NAN, 2.0, 3.0], [0.0; 3], false)), None);
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
        let body = object_update_compressed_at(&[(a, 500, [10.0, 20.0, 30.0], QUARTER_TURN.0), (b, 501, [1.0, 2.0, 3.0], [0.0; 3])]);
        let Ok(Incoming::ObjectsCompressed(objects)) = decode(high(13), &body) else {
            panic!("not compressed")
        };
        assert_eq!(
            objects.iter().map(|o| (o.full_id, o.local_id)).collect::<Vec<_>>(),
            [(a, 500), (b, 501)]
        );
        assert_eq!(objects[0].transform.unwrap().position, [10.0, 20.0, 30.0]);
        assert!(close(&objects[0].transform.unwrap().rotation, &QUARTER_TURN.1));
        assert_eq!(objects[1].transform.unwrap().position, [1.0, 2.0, 3.0]);
        // Cut inside the second block's data: an error, not a guess.
        assert_eq!(decode(high(13), &body[..body.len() - 20]), Err(DecodeError));
    }

    fn item() -> Uuid {
        Uuid::from_bytes([0x17; 16])
    }

    #[test]
    fn attachment_point_is_the_state_with_its_nibbles_swapped() {
        assert_eq!(attachment_point(0x50), 5);
        assert_eq!(attachment_point(0x10), 1);
        // Points from 16 on spill into the low nibble.
        assert_eq!(attachment_point(0xF1), 31);
        assert_eq!(attachment_point(0x73), 55);
        assert_eq!(attachment_point(0), 0);
        for point in 0..=255u8 {
            assert_eq!(attachment_point(point.rotate_left(4)), point);
        }
    }

    #[test]
    fn name_values_name_the_attached_item() {
        let names = format!("AttachItemID STRING RW SV {}", item());
        assert_eq!(attach_item(names.as_bytes()), Some(item()));
        // With its NUL, and among other name-values.
        let several = format!("AttachPt U32 RW S 5\n{names}\nAttachmentOffset VEC3 RW DS 0, 0, 0\0");
        assert_eq!(attach_item(several.as_bytes()), Some(item()));
        // An avatar's names, nothing at all, and things that only look alike.
        assert_eq!(attach_item(b"FirstName STRING RW SV Sit\nLastName STRING RW SV Bot\0"), None);
        assert_eq!(attach_item(b""), None);
        assert_eq!(attach_item(b"AttachItemID STRING RW SV not-a-uuid"), None);
        assert_eq!(attach_item(b"AttachItemID"), None);
        assert_eq!(attach_item(format!("Title STRING RW SV AttachItemID {}", item()).as_bytes()), None);
        assert_eq!(attach_item(format!("AttachItemID STRING RW SV {}", Uuid::nil()).as_bytes()), None);
        assert_eq!(attach_item(&[0x33; 40]), None);
    }

    #[test]
    fn object_update_tells_an_attachment_from_other_objects() {
        let (hat, avatar, prim) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]), Uuid::from_bytes([3; 16]));
        let body = object_blocks(&[
            attachment_block(hat, 30, 8, 2, item()),
            Block {
                full_id: avatar,
                local_id: 8,
                parent_id: 0,
                data: object_data([1.0, 2.0, 3.0], [0.0; 3], true),
                worn: None,
            },
            // A child prim of the attachment has no item of its own.
            Block {
                full_id: prim,
                local_id: 31,
                parent_id: 30,
                data: object_data([0.0; 3], [0.0; 3], false),
                worn: Some((2, String::new())),
            },
        ]);
        let Ok(Incoming::ObjectUpdate(objects)) = decode(high(12), &body) else {
            panic!("not an object update")
        };
        assert_eq!(
            objects.iter().map(|o| (o.full_id, o.local_id, o.parent_id)).collect::<Vec<_>>(),
            [(hat, 30, 8), (avatar, 8, 0), (prim, 31, 30)]
        );
        assert_eq!((objects[0].attach_item, objects[0].attachment_point()), (Some(item()), 2));
        assert_eq!(objects[0].state, 0x20);
        assert!(objects[0].transform.is_some());
        assert_eq!((objects[1].attach_item, objects[1].state), (None, 0));
        assert_eq!((objects[2].attach_item, objects[2].attachment_point()), (None, 2));
        // Cut anywhere in the last block: an error, not a guess.
        for cut in [1, 60, 150] {
            assert_eq!(decode(high(12), &body[..body.len() - cut]), Err(DecodeError), "cut {cut}");
        }
    }

    /// Decodes one compressed block and returns what it says of an
    /// attachment.
    fn worn(fields: &Compressed) -> Option<Worn> {
        let body = object_update_compressed_blocks(&[compressed_data(Uuid::from_bytes([1; 16]), 500, fields)]);
        let Ok(Incoming::ObjectsCompressed(objects)) = decode(high(13), &body) else {
            panic!("not compressed")
        };
        assert_eq!((objects[0].full_id, objects[0].local_id), (Uuid::from_bytes([1; 16]), 500));
        assert_eq!(objects[0].transform.unwrap().position, [1.0, 2.0, 3.0]);
        objects[0].attachment
    }

    #[test]
    fn compressed_update_tells_an_attachment_from_other_objects() {
        let on_hand = Worn {
            item: item(),
            parent_id: 8,
            point: 5,
        };
        assert_eq!(worn(&Compressed::attachment(8, 5, item())), Some(on_hand));
        let hud = Compressed::attachment(8, 35, item());
        assert_eq!(hud.state, 0x32);
        assert_eq!(worn(&hud).map(|worn| worn.point), Some(35));
        // No name-values, or name-values without the item.
        assert_eq!(worn(&Compressed::default()), None);
        let named = Compressed {
            parent_id: Some(8),
            name_values: Some("Title STRING RW SV Chair".into()),
            ..Compressed::default()
        };
        assert_eq!(worn(&named), None);
        // Without a parent the item is still read; it is on nothing.
        let loose = Compressed {
            name_values: Some(attach_item_id(item())),
            ..Compressed::default()
        };
        assert_eq!(worn(&loose).map(|worn| worn.parent_id), Some(0));
    }

    #[test]
    fn compressed_update_is_walked_past_every_optional_field() {
        // Each optional field present by itself, and all of them at once:
        // a wrong width anywhere loses the name-values at the end.
        let attachment = || Compressed::attachment(8, 5, item());
        let each: Vec<(&str, Compressed)> = vec![
            (
                "angular velocity",
                Compressed {
                    angular_velocity: true,
                    ..attachment()
                },
            ),
            (
                "tree",
                Compressed {
                    tree: true,
                    ..attachment()
                },
            ),
            (
                "scratch pad",
                Compressed {
                    scratch_pad: Some(vec![0x5C; 7]),
                    ..attachment()
                },
            ),
            (
                "empty scratch pad",
                Compressed {
                    scratch_pad: Some(vec![]),
                    ..attachment()
                },
            ),
            // A tree's species takes the scratch pad's place.
            (
                "tree and scratch pad",
                Compressed {
                    tree: true,
                    scratch_pad: Some(vec![1, 2]),
                    ..attachment()
                },
            ),
            (
                "text",
                Compressed {
                    text: Some("hover".into()),
                    ..attachment()
                },
            ),
            (
                "empty text",
                Compressed {
                    text: Some(String::new()),
                    ..attachment()
                },
            ),
            (
                "media url",
                Compressed {
                    media_url: Some("http://example.com/".into()),
                    ..attachment()
                },
            ),
            (
                "particles",
                Compressed {
                    particles: true,
                    ..attachment()
                },
            ),
            (
                "one extra parameter",
                Compressed {
                    extra_params: vec![vec![0xE1; 4]],
                    ..attachment()
                },
            ),
            (
                "extra parameters",
                Compressed {
                    extra_params: vec![vec![0xE1; 16], vec![], vec![0xE3; 17]],
                    ..attachment()
                },
            ),
            (
                "sound",
                Compressed {
                    sound: true,
                    ..attachment()
                },
            ),
            (
                "everything",
                Compressed {
                    angular_velocity: true,
                    scratch_pad: Some(vec![0x5C; 3]),
                    text: Some("hover".into()),
                    media_url: Some("http://example.com/".into()),
                    particles: true,
                    extra_params: vec![vec![0xE1; 16], vec![0xE2; 2]],
                    sound: true,
                    ..attachment()
                },
            ),
        ];
        let expected = Some(Worn {
            item: item(),
            parent_id: 8,
            point: 5,
        });
        for (what, fields) in &each {
            assert_eq!(worn(fields), expected, "{what}");
        }
        // The same fields without a parent or name-values make no attachment.
        for (what, fields) in each {
            let plain = Compressed {
                parent_id: None,
                name_values: None,
                ..fields
            };
            assert_eq!(worn(&plain), None, "{what}");
        }
    }

    #[test]
    fn truncated_compressed_data_describes_the_object_without_an_attachment() {
        let full = compressed_data(
            Uuid::from_bytes([1; 16]),
            500,
            &Compressed {
                text: Some("hover".into()),
                extra_params: vec![vec![0xE1; 16]],
                ..Compressed::attachment(8, 5, item())
            },
        );
        // Whole, it is an attachment.
        let decoded = |data: &[u8]| {
            let Ok(Incoming::ObjectsCompressed(objects)) = decode(high(13), &object_update_compressed_blocks(&[data.to_vec()])) else {
                panic!("not compressed")
            };
            objects[0]
        };
        assert!(decoded(&full).attachment.is_some());
        // The data ends inside the name-values, inside the extra parameters,
        // inside the text, after the flags, before the flags: the object is
        // known, by position where that is whole, and is no attachment.
        let end = full.len() - 23;
        for (cut, positioned) in [(end - 1, true), (end - 50, true), (90, true), (70, true), (62, false), (30, false)] {
            let object = decoded(&full[..cut]);
            assert_eq!((object.local_id, object.attachment), (500, None), "cut at {cut}");
            assert_eq!(object.transform.is_some(), positioned, "cut at {cut}");
        }
        // A length that points past the end is not followed.
        let mut bad = full.clone();
        let params = 64 + 4 + 16 + 4 + 6 + 4;
        assert_eq!(bad[params], 1, "the extra-parameter count");
        bad[params + 3..params + 7].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decoded(&bad).attachment, None);
        // A message cut inside a block is still an error as a whole.
        let body = object_update_compressed_blocks(&[full]);
        assert_eq!(decode(high(13), &body[..body.len() - 5]), Err(DecodeError));
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
        assert_eq!(objects.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [7, 8]);
        assert_eq!(objects[0].1.position, [141.0, 64.0, 23.0]);
        assert!(close(&objects[0].1.rotation, &QUARTER_TURN.1), "{:?}", objects[0].1.rotation);
        // The avatar's block is longer by its collision plane.
        assert_eq!(objects[1].1.position, [0.5, 0.25, 0.75]);
        assert!(close(&objects[1].1.rotation, &[0.0, 0.0, 0.0, 1.0]));
        for cut in [1, 10, 30] {
            assert_eq!(decode(high(15), &body[..body.len() - cut]), Err(DecodeError), "cut {cut}");
        }
    }

    #[test]
    fn rotate_turns_a_vector_by_a_quaternion() {
        // A quarter turn about the vertical axis takes x to y.
        assert!(close(&rotate(QUARTER_TURN.1, [1.0, 0.0, 0.0]), &[0.0, 1.0, 0.0]));
        assert!(close(&rotate(QUARTER_TURN.1, [0.0, 1.0, 2.0]), &[-1.0, 0.0, 2.0]));
        assert!(close(&rotate([0.0, 0.0, 0.0, 1.0], [3.0, 4.0, 5.0]), &[3.0, 4.0, 5.0]));
    }

    #[test]
    fn encodes_money_transfer_request() {
        let dest = Uuid::from_bytes([0xD1; 16]);
        let msg = money_transfer_request(&agent(), dest, 25, TRANSACTION_GIFT, "");
        assert_eq!((msg.id, msg.zerocoded), (low(311), true));
        assert_eq!(name(msg.id), "MoneyTransferRequest");
        assert_eq!(
            msg.body,
            [
                agent_prefix(),
                vec![0xA1; 16], // SourceID: the agent
                vec![0xD1; 16],
                vec![0],                // Flags
                vec![25, 0, 0, 0],      // Amount
                vec![0, 0],             // AggregatePerm*
                vec![0x89, 0x13, 0, 0], // TransactionType 5001
                vec![1, 0],             // Description: empty, with its NUL
            ]
            .concat()
        );
        let msg = money_transfer_request(&agent(), dest, 250, TRANSACTION_PAY_OBJECT, "Hat Vendor");
        assert_eq!(msg.body[32 + 32 + 1 + 4 + 2..][..4], 5008i32.to_le_bytes());
        assert_eq!(msg.body[32 + 32 + 1 + 4 + 2 + 4..], *[&[11u8][..], b"Hat Vendor\0"].concat());
    }

    #[test]
    fn encodes_money_balance_request() {
        let msg = money_balance_request(&agent());
        assert_eq!((msg.id, msg.zerocoded), (low(313), true));
        assert_eq!(name(msg.id), "MoneyBalanceRequest");
        assert_eq!(msg.body, [agent_prefix(), vec![0; 16]].concat());
    }

    #[test]
    fn encodes_request_pay_price() {
        let object = Uuid::from_bytes([0xB0; 16]);
        let msg = request_pay_price(object);
        assert_eq!((msg.id, msg.zerocoded), (low(161), false));
        assert_eq!(name(msg.id), "RequestPayPrice");
        assert_eq!(msg.body, vec![0xB0; 16]);
    }

    #[test]
    fn decodes_money_balance_reply_with_and_without_a_transaction() {
        let info = TransactionInfo {
            kind: TRANSACTION_GIFT,
            source: Uuid::from_bytes([0xA1; 16]),
            source_group: false,
            dest: Uuid::from_bytes([0xD1; 16]),
            dest_group: false,
            amount: 25,
            item: String::new(),
        };
        let body = money_balance_reply(true, 975, "You paid Jane Doe L$25.", Some(&info));
        assert_eq!(
            decode(low(314), &body),
            Ok(Incoming::MoneyBalanceReply(MoneyReport {
                transaction: Uuid::from_bytes([0x7A; 16]),
                success: true,
                balance: 975,
                description: "You paid Jane Doe L$25.".into(),
                info: Some(info),
            }))
        );
        assert_eq!(name(low(314)), "MoneyBalanceReply");
        let body = money_balance_reply(false, -3, "", None);
        assert_eq!(
            decode(low(314), &body),
            Ok(Incoming::MoneyBalanceReply(MoneyReport {
                transaction: Uuid::from_bytes([0x7A; 16]),
                success: false,
                balance: -3,
                description: String::new(),
                info: None,
            }))
        );
        assert_eq!(decode(low(314), &body[..body.len() - 2]), Err(DecodeError));
    }

    #[test]
    fn decodes_pay_price_reply() {
        let object = Uuid::from_bytes([0xB0; 16]);
        let body = pay_price_reply(object, -1, &[100, 250, -1, -1]);
        assert_eq!(
            decode(low(162), &body),
            Ok(Incoming::PayPriceReply {
                object,
                default: -1,
                buttons: vec![100, 250, -1, -1],
            })
        );
        assert_eq!(name(low(162)), "PayPriceReply");
        assert_eq!(decode(low(162), &body[..body.len() - 1]), Err(DecodeError));
    }

    // Packets captured from the rig's simulator with its money module
    // (test-rig/money), message number first, zerocoding undone. The
    // outgoing ones were sent by test-rig/pay.py and taken by the
    // simulator.

    const RIG_ALT: &str = "7e570000-0000-4000-8000-0000000000a1";
    const RIG_OWNER: &str = "7e570000-0000-4000-8000-0000000000aa";
    const PAYEE_BOX: &str = "458f6569-74b2-4e6c-8083-1073973bd16e";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// The message number and the body of a capture.
    fn captured(text: &str) -> (MessageId, Vec<u8>) {
        let bytes = hex(text);
        let id = u32::from_be_bytes(bytes[..4].try_into().unwrap());
        (id, bytes[4..].to_vec())
    }

    fn uuid(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    fn rig_alt() -> Agent {
        Agent {
            agent_id: uuid(RIG_ALT),
            session_id: uuid("75657546-2f67-455a-9a1d-0870fefa40f3"),
        }
    }

    fn reply(text: &str) -> MoneyReport {
        let (id, body) = captured(text);
        assert_eq!(id, MONEY_BALANCE_REPLY);
        match decode(id, &body) {
            Ok(Incoming::MoneyBalanceReply(report)) => report,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn encodes_what_the_rig_takes() {
        let (id, body) = captured("ffff00a1458f656974b24e6c80831073973bd16e");
        let ours = request_pay_price(uuid(PAYEE_BOX));
        assert_eq!((ours.id, ours.body), (id, body));

        let (id, body) = captured(
            "ffff01377e5700000000400080000000000000a1756575462f67455a9a1d0870fefa40f37e570000000040008000\
             0000000000a1458f656974b24e6c80831073973bd16e00fa0000000000901300000a506179656520626f7800",
        );
        let ours = money_transfer_request(&rig_alt(), uuid(PAYEE_BOX), 250, TRANSACTION_PAY_OBJECT, "Payee box");
        assert_eq!((ours.id, ours.body), (id, body));

        // pay.py sends an empty description as nothing at all; the bot, as
        // a viewer, with its NUL. The rest is the same.
        let (id, body) = captured(
            "ffff01377e5700000000400080000000000000aa8ade37f66e7a40f9b8a7db31062bd7b97e570000000040008000\
             0000000000aa7e5700000000400080000000000000a1006400000000008913000000",
        );
        let owner = Agent {
            agent_id: uuid(RIG_OWNER),
            session_id: uuid("8ade37f6-6e7a-40f9-b8a7-db31062bd7b9"),
        };
        let ours = money_transfer_request(&owner, uuid(RIG_ALT), 100, TRANSACTION_GIFT, "");
        assert_eq!(ours.id, id);
        assert_eq!(ours.body[..body.len() - 1], body[..body.len() - 1]);
        assert_eq!(ours.body[body.len() - 1..], [1, 0]);

        // A request for the balance: pay.py names a transaction, the bot
        // (as a viewer) none.
        let (id, body) =
            captured("ffff01397e5700000000400080000000000000a1756575462f67455a9a1d0870fefa40f34babf67f18ed4798b005075cc366ab08");
        let ours = money_balance_request(&rig_alt());
        assert_eq!((ours.id, &ours.body[..32]), (id, &body[..32]));
        assert_eq!(ours.body.len(), body.len());
    }

    #[test]
    fn decodes_what_the_rig_sends() {
        // The answer to a request for the balance: a zeroed transaction.
        let balance = reply(
            "ffff013a7e5700000000400080000000000000a14babf67f18ed4798b005075cc366ab0801e8030000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        );
        assert_eq!((balance.success, balance.balance, balance.description.as_str()), (true, 1000, ""));
        let info = balance.info.unwrap();
        assert_eq!((info.kind, info.amount, info.source, info.dest), (0, 0, Uuid::nil(), Uuid::nil()));

        // The price of a vendor.
        let (id, body) = captured("ffff00a2458f656974b24e6c80831073973bd16effffffff0464000000fa000000ffffffffffffffff");
        assert_eq!(
            decode(id, &body),
            Ok(Incoming::PayPriceReply {
                object: uuid(PAYEE_BOX),
                default: -1,
                buttons: vec![100, 250, -1, -1],
            })
        );

        // Paying the box: the payer is told, and the destination is the
        // box's owner.
        let paid = reply(
            "ffff013a7e5700000000400080000000000000a1abc4bff19eb94b8b96322ac345caf40e01ee02000000000000000000\
             0026596f75207061696420506179656520626f782028526967204f776e657229204c243235302e00901300007e5700\
             000000400080000000000000a1007e5700000000400080000000000000aa00fa0000000a506179656520626f7800",
        );
        assert_eq!(
            paid,
            MoneyReport {
                transaction: uuid("abc4bff1-9eb9-4b8b-9632-2ac345caf40e"),
                success: true,
                balance: 750,
                description: "You paid Payee box (Rig Owner) L$250.".into(),
                info: Some(TransactionInfo {
                    kind: TRANSACTION_PAY_OBJECT,
                    source: uuid(RIG_ALT),
                    source_group: false,
                    dest: uuid(RIG_OWNER),
                    dest_group: false,
                    amount: 250,
                    item: "Payee box".into(),
                }),
            }
        );
        // The owner is told as well.
        let owner = reply(
            "ffff013a7e5700000000400080000000000000aaabc4bff19eb94b8b96322ac345caf40e01e2040000000000000000\
             00001852696720416c74207061696420796f75204c243235302e00901300007e5700000000400080000000000000a1\
             007e5700000000400080000000000000aa00fa0000000a506179656520626f7800",
        );
        assert_eq!((owner.balance, owner.description.as_str()), (1250, "Rig Alt paid you L$250."));

        // A gift, to the payer and to the payee.
        let gave = reply(
            "ffff013a7e5700000000400080000000000000aaeec381b73b294ca2a7227bd0265915e4018403000000000000000000\
             0018596f7520706169642052696720416c74204c243130302e00891300007e5700000000400080000000000000aa00\
             7e5700000000400080000000000000a10064000000085061796d656e7400",
        );
        let info = gave.info.unwrap();
        assert_eq!(
            (gave.balance, info.kind, info.source, info.dest, info.amount, info.item.as_str()),
            (900, TRANSACTION_GIFT, uuid(RIG_OWNER), uuid(RIG_ALT), 100, "Payment")
        );
        let got = reply(
            "ffff013a7e5700000000400080000000000000aa3ee6090dc6c6427692ae8d045d4244b10106040000000000000000\
             00001752696720416c74207061696420796f75204c2433302e00891300007e5700000000400080000000000000a100\
             7e5700000000400080000000000000aa001e000000085061796d656e7400",
        );
        let info = got.info.unwrap();
        assert_eq!(
            (got.balance, info.source, info.dest, info.amount),
            (1030, uuid(RIG_ALT), uuid(RIG_OWNER), 30)
        );

        // A gift the money module refused.
        let refused = reply(
            "ffff013a7e5700000000400080000000000000a14c2aa6e037d748d1ae86e7da82cf6f970032000000000000000000\
             00002c596f752063616e6e6f7420706179204c243130303a20796f75722062616c616e6365206973204c2435302e00\
             891300007e5700000000400080000000000000a1007e5700000000400080000000000000aa0064000000085061796d\
             656e7400",
        );
        assert_eq!(
            (refused.success, refused.balance, refused.description.as_str()),
            (false, 50, "You cannot pay L$100: your balance is L$50.")
        );
        assert_eq!(refused.info.unwrap().amount, 100);
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
