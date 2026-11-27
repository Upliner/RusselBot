# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- Nothing of groups exists today. `Session::handle_message` (`src/session.rs`) drops every instant message whose dialog it does not know with a debug line, the group ones among them, and the decoder in `src/udp/messages.rs` does not keep an instant message's recipient. `caps::parse_events` decodes `TeleportFinish` only; every other event of the event queue becomes `EqEvent::Other` and is dropped. The long-poll loop, its thread and the way its events reach `Session::handle_caps` are there.
- The wire protocol, read from `references/kokua` (message template, `llimview.cpp`, `llimprocessing.cpp`, `llgroupmgr.cpp`, `llviewermessage.cpp`) and `references/opensim` (`Addons/Groups`, `LLClientView.cs`, `EventQueueGetHandlers.cs`):
  - **Membership.** `AgentGroupDataUpdate` (Low 389) lists the groups, each with `GroupID`, `GroupPowers` (U64), `AcceptNotices`, `GroupName`; OpenSim sends it **only as an event** of the event queue when the client has one, with the powers as eight bytes of LLSD binary, big-endian. `AgentDataUpdate` (Low 387, UDP) carries `ActiveGroupID`, `GroupName`, `GroupTitle`, `GroupPowers`. `AgentDropGroup` (Low 390) names a group the avatar left, as a message or as an event. A viewer sends `AgentDataUpdateRequest` (Low 386) at startup and after a join; OpenSim also sends both updates unprompted when the avatar becomes root, after activate, join, leave and an accepted invitation.
  - **Activate.** `ActivateGroup` (Low 368) with the group's UUID, nil for none; answered by `AgentDataUpdate`.
  - **Invite.** `InviteGroupRequest` (Low 349) with `InviteeID` and `RoleID`, nil for Everyone; the power is bit 1 of `GroupPowers`. Neither grid's simulator tells the inviter anything.
  - **Invitation.** An instant message with dialog 3: `AgentID` is the group (`FromGroup` set), `ID` the id to answer with, `FromAgentName` the inviter's name, and a bucket of 20 bytes, the fee as a big-endian S32 and the role. The inviter's UUID is not on the wire. The viewer answers with an instant message to the group, dialog 35 (accept) or 36 (decline), `ID` the invitation's, name `name`, message `message`. No `JoinGroupReply` follows; the new list does. OpenSim's Groups V2 never puts a fee into the bucket and takes none on acceptance.
  - **Profile.** `GroupProfileRequest` (Low 351) and `GroupProfileReply` (Low 352): name, charter, founder, `MembershipFee`, `OpenEnrollment`, member and role counts, `ShowInList`, `MaturePublish`, and the asker's own title. OpenSim answers for non-members, gives no answer for an unknown group, and drops a repeated request for one group within 10 seconds.
  - **Join.** `JoinGroupRequest` (Low 343) and `JoinGroupReply` (Low 344, `Success`). The simulator takes the fee itself: OpenSim through `IMoneyModule.ApplyCharge(..., MoneyTransactionType.GroupJoin, ...)`, Second Life with a transaction of type 1004 (`TRANS_GROUP_JOIN`) that the viewer reads from `MoneyBalanceReply`. The viewer checks the balance and open enrollment itself; OpenSim V2 checks neither enrollment nor an invitation.
  - **Leave.** `LeaveGroupRequest` (Low 347) with the group's UUID, answered by `LeaveGroupReply` (Low 348, `Success`). OpenSim answers with success, then sends `AgentDropGroup` and a new list, and clears the active group when it was the one left. The viewer refuses by itself when the avatar is the group's only owner and only member (`LLGroupActions::processLeaveGroupDataResponse`), from member data the bot does not fetch; neither Kokua nor Firestorm checks `setgroup` for a leave.
  - **Group chat.** The session's id is the group's UUID. A line is an instant message with dialog 17, `ID` the group, `AgentID` the speaker, the bucket the group's name. The **first** line a member gets of a session comes as the event `ChatterBoxInvitation` with the message inside; later ones over UDP. Second Life's viewer then posts `accept invitation` to the capability `ChatSessionRequest`; OpenSim has no such capability and needs none. To speak, a viewer sends dialog 15 (start) to the group and holds its lines until the event `ChatterBoxSessionStartReply`. OpenSim delivers the speaker's own line back to it.
  - **Notice.** An instant message with dialog 32: message `subject|body`, bucket `has_attachment`, `asset_type`, the group's UUID and the item's name; `ID` is the notice. On OpenSim `AgentID` is the group and the sender is a name only.
- `msglog::Recorder` already holds two sorts of entry, messages and transactions, with one name cache, and `flush` returns `Recorded::Message` or `Recorded::Transaction` in arrival order. `Channel` is matched exhaustively in the recorder, `Logs`, the hub and the XMPP gateway.
- `perms::Rules` keeps `[Rule; 6]` and `[Vec<Entry>; 6]`; `perms::Waiting::take(kind, who)` returns the **latest** match and knows no ambiguity; `Commands::decide` and the `Undecided` list (a request that waits up to 10 seconds for an owner or a name) are where a request is settled.
- A payment is the one slot `Money::pay` in `src/commands/pay.rs`, a `Wait` with a `Target` and a `Stage`; `send_payment` runs the checks in order and ends in `grid.pay`; `txlog::from_report` takes a `MoneyBalanceReply` as the answer to the payment in flight when its source is the avatar and its amount is the payment's.
- `web::Hub` has rings for `chat`, `im` and `money`; `http::may_be_own` lists the commands a page may mark as its own; `app.js` builds the IM tab from `conversationOf`, `ensureConversation`, `addIm`, `renderConversations`, and the payment dialog from `confirmPayment`.
- The rig's OpenSim has `[Groups]` commented out, Groups V2 has storage for MySQL and PostgreSQL only while the rig runs on SQLite, and `RigMoneyModule.ApplyCharge` does nothing.

## Goals / Non-Goals

**Goals:**

- One path for what the grid says in two ways: a membership list or a chat line that came as an event and one that came over UDP are the same `SessionEvent`.
- Everything that decides is pure and tested without a grid: naming a group, matching an invitation, the RLV rules for a group, the texts of the listings, reading a notice.
- A fee cannot leave the avatar by any way that `[payments]` does not see: it goes through the slot, the checks and the ledger a `pay` goes through.
- Groups are tried end to end on the rig, as every other feature is.

**Non-Goals:**

- A model of a group's roles, members or sessions' participants.
- Keeping a session's state across logins, or anything of groups across restarts beyond the log file.
- Making OpenSim's Groups V2 behave like Second Life where it differs (fees of invitations, enrollment).

## Decisions

### `groups.rs`: what the bot knows, without I/O

`Groups { known: bool, list: Vec<Membership>, active: Option<Uuid>, title: String }` with `Membership { id, name, powers: u64, notices: bool }` lives in `Commands`, is replaced by a list from the grid, edited by a drop and by `AgentDataUpdate`, and cleared in `session_lost`. `Groups::named(&str) -> Result<&Membership, Naming>` is the one place a `<group>` argument is read: a UUID by identity, a name by lower-case comparison of the whole name, and `Naming::Ambiguous(Vec<&Membership>)` renders the error with the UUIDs. `list_text()` and `info_text(&Profile, founder, member)` build the answers. The session keeps none of this, as it keeps no money state: it decodes and emits events.

A command that needs the list while `known` is false is parked in `Commands` with a 10-second deadline and resumed by the first list, which is cheaper than failing a startup-script line that ran a moment early.

### Both ways in end in the same events

`caps::EqEvent` gets `Groups(Vec<Membership>)`, `GroupDropped(Uuid)`, `SessionInvite { .. }` (the chat line inside `ChatterBoxInvitation`) and `SessionStarted { session, ok, error }`; `parse_events` reads their LLSD, `GroupPowers` as eight big-endian bytes. `messages::decode` gets `AgentGroupDataUpdate`, `AgentDataUpdate`, `AgentDropGroup`, `GroupProfileReply`, `JoinGroupReply` and `LeaveGroupReply`, and `InstantMessage` gains `to`. `Session` turns both into `SessionEvent::{Groups, GroupDropped, ActiveGroup, GroupProfile, Joined, Left, GroupInvitation, GroupChat, GroupNotice, GroupSession}`. It sends `AgentDataUpdateRequest` once per session on the first region entered, where `fetch_offline_im` is sent, and again after a `JoinGroupReply` that succeeded and after a drop, as the viewer does.

`ChatSessionRequest` joins the capabilities `seed_request` asks for. Taking a session up is one POST, sent from a worker thread of the shape `names::spawn` has; its failure is logged and changes nothing, since the lines the grid delivers are recorded either way.

Alternative considered: polling the grid for the list on every `group` command. The grid pushes every change; a second source would only disagree with it.

### Sessions: enter on the first line said, never on login

`Commands` keeps `sessions: HashMap<Uuid, Chat>` with `Chat::In` and `Chat::Entering { lines: Vec<(text, reply, origin)>, deadline }`. A group is `In` once the grid has delivered a line of its session (and the bot has taken it up) or has answered a start with success. `group chat` for a group that is not `In` sends dialog 15 and parks the line; `SessionStarted` sends the parked lines or fails them with the grid's `error`; 10 seconds without it fails them. The map is cleared with the session.

A line from the avatar's own UUID is dropped when it arrives: the record of a line the bot said is made when it is sent, as `im_out` is, so the grid that echoes and the grid that does not give the same log. A dialog-17 line whose session is not in `Groups` is a conference and is dropped as before.

Alternatives considered: starting every group's session after login (it announces the avatar in every group's participant list and is not needed to receive, on either grid); sending dialog 17 without a start (works on OpenSim, and on Second Life leaves the line unsent without an error).

### Group messages are a third sort of record, not a third channel

New `src/grouplog.rs` owns `GroupMessage { time, kind: Chat | ChatOut | Notice, group, group_name, avatar: Option<Uuid>, avatar_name, display_name, subject, attachment, text, offline }`, the nine `COLUMNS`, `row()`, `log_line()` and the pure `notice(text, bucket)` that splits `subject|body` and reads the bucket. `Recorder` gets `push_group`, which asks for the display name through the cache it has, and `Recorded::Group`; `Logs` gets the fourth file; `Bot::write_recorded` hands a group record to the logs and the hub and not to the XMPP gateway.

Alternative considered: `Channel::Group` and three more fields on `Record`. Every reader of a `Record` would carry fields that are empty for chat and IMs, the seven-column row is shared by two files that must not change, and the XMPP gateway would have to learn to skip a channel. The transactions took the same way out.

### An invitation is a request of kind `group`

`perms::Kind::Group` (the arrays become seven), `Ask::Group { group, id, fee, name: Option<String> }` and `Sender::Inviter { name, id: Option<Uuid>, group }`; `Entry::matches` gives an `Id` the group and the inviter's id and a `Name` the inviter's name. An invitation enters `Commands::undecided` needing two things, the group's profile and the inviter's UUID, the latter through `find_avatar` and `exact_match` as `im` uses them, and is decided when both are in or after 10 seconds. `decide` asks `rules.decide` only when `fee == 0`; otherwise it waits and the log line carries the `--pay` form of the command. `Waiting::replaced_by` makes a newer invitation into the same group replace the older one, and a new list from the grid removes the waiting invitations into groups it holds.

`accept group` does not use `Waiting::take`: `Waiting::invitations(who: Option<&str>)` returns every invitation the argument names, and the command goes on only with exactly one. That differs from the other kinds, which take the newest match, and it is meant to: an acceptance joins a group and may cost money, so "whichever came last" is not good enough, and the caller of a group's name does not know whether a second group of that name has invited too. `Request::Accept` gains `pay: Option<u32>`, and `Request::Decline { kind, who }` is new; the invitation's fee is the bucket's, as the viewer shows it, not the profile's.

The answer is the viewer's: an instant message to the group with dialog 35 or 36, the invitation's id, the name `name` and the message `message`. Whether the avatar joined is read from the next membership list, so `Commands` keeps `joining: Vec<Joining { group, reply, deadline }>` for the 15 seconds the spec gives, for `group join` and for acceptances alike. A `JoinGroupReply` refuses a join, or marks it as accepted by the grid, but does not answer it: OpenSim sends the reply before the list with the group, and a command naming the group right after the answer would not find it. A join the grid accepted and never listed fails after the 15 seconds with an error that says so.

### A fee rides in the payment slot

`pay.rs` gets `Target::Group { group, name, how: Join | Accept { id } }`. `Wait` is built by `group join` and `accept group` when the fee is greater than zero and goes through `send_payment` unchanged up to its last step: `Payee { avatar: None, object: Some(group) }` gives the lists the group's UUID to match and no name, the balance, the unwritten row and `Ledger::reserve` are checked as for a `pay`, and where a `pay` calls `grid.pay` a fee calls `grid.join_group(group)` or `grid.answer_invitation(group, id, true)`. `InFlight` carries type 1004, the group as party and `party_group`. `txlog::from_report` already recognises the grid's report by source and amount, so a reported fee becomes the `paid` record with no new matching rule.

`Stage::Sent` gains what the table of "Outcome of a fee" needs: whether the command was answered (by the list, by `JoinGroupReply`, or by its refusal) and whether the report came. The slot is freed, and the record made, when the report came or the 15 seconds are over, not when the command is answered; that keeps the one-payment rule, which is what makes matching by amount safe.

Alternatives considered: a slot of its own for fees (two payments in flight, and a fee and a `pay` of the same amount could each take the other's report); paying the group with `MoneyTransferRequest` (the grid takes the fee itself, so the avatar would pay twice); counting nothing and recording the fee as `spent` when the grid reports it (the limit would not hold a fee back, which is the point of asking for `--pay`).

### One profile cache for three askers

`Commands` keeps `profiles: HashMap<Uuid, (Profile, Instant)>` and a list of who waits for one. `group info`, `group join` and an arriving invitation all ask through it; an entry younger than 30 seconds answers at once. The age is above OpenSim's 10 seconds of dropping repeats, so that "info, then join" from the page works, and small enough that a fee is not stale by more than half a minute.

### Lists that come too late

OpenSim answers a leave with `LeaveGroupReply`, `AgentDropGroup` and a list, and a join with `JoinGroupReply` and a list; the lists go out over the event queue and its handlers run in parallel, so a leave and a join a few milliseconds apart arrive interleaved: on the rig a rejoin was confirmed and then undone by the leave's late list and drop. `Commands` keeps `settled: Vec<Settled { group, member, until }>` for 5 seconds after each confirmed join or leave. A list or a drop that contradicts one, for a group without a join or a leave of its own in flight, is held back for that group (the rest of the list is taken), and `ask_again` is set to the end of the window, when `groups_due` sends `AgentDataUpdateRequest` through `Grid::ask_groups`; that answer, made after both, is taken as it comes. A real ejection within 5 seconds of a join is thus seen 5 seconds late.

Alternative considered: ordering the lists by the event queue's ids. They order the delivery, not the moment the simulator made each list, which is what goes wrong.

### Leaving

`group leave` names the group as every `<group>` is named, sends `LeaveGroupRequest` through `Grid::leave_group(group)`, and parks a `Leaving { group, reply, deadline }` in `Commands::leaving` for 15 seconds, the time a join gets. A `SessionEvent::Left { group, ok: true }` is taken as the drop, as the viewer takes it: OpenSim sends it before `AgentDropGroup` and the list, and a `group list` right after the answer must not list the group. The drop, by that reply, by `AgentDropGroup` or by a list without the group, whichever comes first, ends the wait with success, and the later ones find the group gone and change nothing; `Left` with `ok == false` ends it with the refusal. The bot does not check for an only owner as the viewer does: it would need the member list, and the grid refuses on its own.

### Commands and their dispatch

`control::Request::Group(GroupRequest)` with `List`, `Activate(String)`, `Invite { group, avatar }`, `Chat { group, text }`, `Info(String)`, `Join { group: Uuid, pay: Option<u32> }`, `Leave(String)` and `Invitations`. `Activate`, `Info` and `Leave` join their arguments with single spaces; `Chat` takes the group as one argument and joins the rest, in `request_line` as for `im`. `--pay` is read where `pay` reads `-d`. `Bot::control_as` passes `via` on with `Group(Join)` and `Accept`, the two that may pay; `List`, `Info` and `Invitations` join the commands that are never the exempt one of an override, and `Group(List | Invitations)` joins `http::may_be_own`.

### RLV

`setgroup` moves from `KEPT` to `ENFORCED`. `rlv::Target` gets `Group { id, name }`, whose `named_by` takes an option that is the UUID, the name without `,` and `;` in lower case, or, for the caller that asks so, `allgroups`. `Limits::group_chat(group)` and `Limits::recv_group(speaker, group)` are the two rules of the spec, written after `KokuaRLVExtras::cannotSendIM` and `cannotRecvIM`; the avatar forms `Limits::im` and `recvim` stay as they are, with their test that a group option does nothing to an avatar's IM.

### Web

`Hub` gets a `group` ring, numbered in the one sequence, with the eleven fields of a group message as an event's data, and `groups_changed()`, which publishes an event `groups` with no data and keeps nothing. `Commands` raises a flag when the list, a flag of a group, the active group or the waiting invitations change; `Bot` reads it once per loop turn, where it publishes the status. The page asks `group list` and `group invitations` when that event comes, which keeps the lists out of the stream's format and gives one way to read them for every interface.

Alternative considered: the lists as events. They are state, not history: a ring would replay stale lists, and the page would still need the commands for its refresh button.

In `app.js` `TABS` gets `groups` after `im`. The tab reuses the IM tab's shapes (`.im`, `.picks`, `.log`, `.msg`) with a conversation per group keyed by its UUID; a notice is a `.msg.notice` with its own background token in both themes. The fee dialog is `confirmPayment` with the group, the inviter and the fee as its lines. The Leave button's dialog has the same shape, its element, focus on cancel and Escape, with a question and a button text of its own; `confirmPayment` becomes one case of a general confirmation rather than a second dialog being written beside it.

### Groups on the rig

- `test-rig/groups/RigGroupsData.cs`: `OpenSim.Data.IGroupsData` kept in memory (groups, members, roles, role members, principals, invitations, notices), built as an addon like the money module, and named as `StorageProvider` under `[Groups]`. `setup.sh` writes `[Groups]` (`Module = "Groups Module V2"`, `ServicesConnectorModule = "Groups Local Service Connector"`, `MessagingEnabled`, `MessagingModule = "Groups Messaging Module V2"`, `MessageOnlineUsersOnly = true`, `NoticesEnabled`) and gives `[GridUserService]` its SQLite `StorageProvider`, which `GroupsServiceBase` would otherwise look for in the groups store. Groups are gone when the simulator stops; the helper makes them again.
- `RigMoneyModule.ApplyCharge` takes the amount and reports it with the transaction type it was given, so a join with a fee is charged and reported as type 1004.
- `test-rig/group.py`, as `Rig Owner`: creates a group (name, fee, open or closed), invites the bot, says lines, sends a notice with or without an attachment, ejects the bot, and prints the group chat it gets.

Alternative considered: MariaDB for the rig. A database server to install and start for a store of a few rows, on a rig that otherwise needs nothing but the simulator.

What the rig cannot show, because Groups V2 does not do it: an invitation with a fee, a join refused for closed enrollment by the grid, stored notices. The first is covered by tests against the fake grid with the viewer's bucket; all three are on the list for Second Life.

## Risks / Trade-offs

- [Second Life's side is read from the viewer, not tried: taking a session up, the form of an inviter's name, what follows an acceptance, the report of a fee] → each has a fallback that is safe: lines are recorded however they come, a name that finds no UUID still matches by name, a join the grid does not confirm is an error after 15 seconds, a fee that is not reported is `unconfirmed` and counted. The last task group tries them on Second Life.
- [The fee changes between the profile and the join] → the grid takes what it takes; the profile is at most 30 seconds old. The record then shows `unconfirmed` for the fee the bot knew and `spent` for what the grid reported, and the README says so.
- [OpenSim's Groups V2 takes no fee on an accepted invitation and says "no cost" in every invitation] → the bot goes by the invitation, pays nothing and counts nothing, which is what happens.
- [A grid makes a joined group the active one] → shown as the grid says; `setgroup` does not undo it.
- [The avatar is a group's only owner] → the bot sends the leave as asked; the grid's refusal, or the missing confirmation after 15 seconds, is the answer.
- [`accept group` asks for a unique match where the other kinds take the newest] → deliberate, said in the usage text and the README.
- [`perms status` gains a seventh line] → at the end, after `camera`; scripts that read the first six are not disturbed.
- [Group chat of a busy group fills the ring and the log] → the ring holds 500 on its own count; the file is the operator's choice, off by default.
- [The owners of the XMPP gateway see nothing of groups] → named in the proposal as not in this change; the commands work from the control contact.
- [The event queue is the only way some of this arrives] → it is already required for a session (`EventQueueGet`), and its loss already ends the session.

## Migration Plan

- Nothing to migrate: without `group` under `[logs]` no file is written, and without rules for `group` every invitation waits. Rolling back is running the older binary; a `group` key under `[logs]` or `[perms.autoaccept]` must then be removed, since the older one rejects it.
- A rig set up before this change needs `test-rig/setup.sh` run again to build the groups store, and the `[Groups]` and `[GridUserService]` lines added by hand to an existing `OpenSim.ini`, which `setup.sh` never overwrites; `docs/test-rig.md` gives them.

## Open Questions

- Whether Second Life delivers the avatar's own group line back, in which form it names the inviter (`First Last` or `first.last`), and which transaction type and party it reports for a fee. None changes the design (own lines are dropped either way, both forms of a name are read, the report is matched by payer and amount); they settle the wording of the README and are tried in the last task group.
