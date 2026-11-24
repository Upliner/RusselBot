# Proposal

## Why

The bot's avatar is deaf and mute where groups are concerned: group chat and group notices are dropped unread, an invitation to a group waits for nobody, and the operator who wants the avatar to wear a group's tag, to say a line in a group or to join one has to log in with a viewer, which throws the bot off the grid. Much of what goes on around an avatar on Second Life goes on in groups, so the bot now reads and writes group chat, answers invitations and does the handful of group actions an operator needs, from every interface and from a tab of the page.

## What Changes

- **The avatar's groups.** The bot learns the groups the avatar is in, and which one is active, from the grid after each login and whenever they change.
- **Commands**, taken by every interface:
  - `group list` lists the avatar's groups: UUID, flags (`active`, `invite` when the avatar may invite into it, `notices` when it gets the group's notices) and name.
  - `group activate <group>` makes a group the active one, the one whose tag the avatar wears; `group activate none` leaves none active.
  - `group invite <group> <avatar>` invites an avatar into a group, into its role Everyone.
  - `group chat <group> <message>` says a line in a group's chat.
  - `group info <group>` prints a group's name, founder, fee, enrollment, numbers of members and roles, the avatar's own title in it, and its charter. It works for any group by UUID, a group the avatar is not in included.
  - `group join <uuid> [--pay <sum>]` joins a group that is open to join. A group with a fee is joined only with `--pay` and a sum that is not less than the fee.
  - `group invitations` lists the invitations that wait.
  - `accept group [<who>] [--pay <sum>]` accepts a waiting invitation and `decline group [<who>]` declines one. `<who>` is the group's name or UUID or the inviter's name or UUID, and has to name exactly one waiting invitation.
- **Naming a group.** `<group>` is one argument: a UUID, or the name of a group the avatar is in, compared without regard to letter case. When two of the avatar's groups have that name the command fails and lists their UUIDs.
- **Group chat and notices are read.** Every line of group chat in a group the avatar is in, the lines the bot says itself, and every group notice are recorded: on standard error, in the file `logs.group` when set, and on the event stream of the web interface. A notice's attachment is named, not taken.
- **Invitations** are a new kind of request, `group`, beside `tp`, `inventory` and the script permissions: one waits until `accept group` or `decline group`, `perms autoaccept group <who>...` names the groups and inviters whose invitations the bot accepts by itself, and `perms status` lists them. An invitation with a fee is never accepted by the bot itself.
- **Fees are payments.** Joining a group that costs L$, by `group join` or by `accept group`, is held to everything `[payments]` says: the interface has to be in `payments.perms`, the fee counts against the limit, the lists are matched against the group's UUID, the balance is checked, and the fee is recorded as a transaction. Without `--pay`, or with a sum below the fee, nothing is sent and the error names the fee.
- **`[logs]`** gets the key `group`, the path of the TSV file for group chat and notices.
- **Web interface:** a **Groups** tab between IM and Inventory:
  - the invitations that wait, on top, each with the group, the inviter, the fee when there is one, and Accept and Reject buttons; accepting one with a fee opens the confirmation dialog of the Money tab first;
  - the avatar's groups, the active one marked, each with the number of new lines;
  - the chosen group's chat with its notices in the same list, the notices set apart in bold on a tinted background with their subject as a heading, and a box to say a line;
  - buttons to activate the group, to invite someone into it (a name, a UUID or a friend) and to show its info.
- **RLV:** `sendim` and `sendimto` hold back `group chat` and `recvim` and `recvimfrom` hide group chat that is received, with groups named in their options and `allgroups` as RLV viewers read them; `setgroup` refuses `group activate`.
- The rig gets groups: an in-memory store for OpenSim's groups module, a money module that charges and reports the fee of a join, and a helper that creates a group, invites, chats and sends a notice.

Not in this change: leaving a group, creating one, ejecting members, roles and titles; sending notices and taking a notice's attachment; conferences (ad-hoc sessions) and who is in a session; searching groups by name; `group join` and `group info` by the name of a group the avatar is not in; group chat, notices and invitations over the XMPP gateway (the commands work there, the messages are not passed on); accepting by itself an invitation that costs L$; paying L$ to a group other than its fee; the RLV commands `@setgroup:<name>=force` and `@getgroup`; group chat that was said while the avatar was offline, which no grid keeps.

## Capabilities

### New Capabilities

- `groups`: the avatar's groups and the active one as the grid tells them; how a command names a group and what happens when the name is ambiguous; the commands `group list`, `group activate`, `group invite`, `group info`, `group join` and `group invitations`; joining with a fee; what the operator log says.
- `group-chat`: reading group chat and group notices, saying a line with `group chat`, the record of a group message and its names, the line on standard error and the `logs.group` file.

### Modified Capabilities

- `permission-requests`: the kind `group` for invitations: what an entry stands for, what the bot learns before it decides, that only an invitation without a fee is accepted by itself, which invitations wait, `accept group` with its unique match and `--pay`, `decline group`, the line of `perms status`.
- `payments`: the fee for joining a group is a payment: the interfaces that may pay it, how the lists match a group, the limit, its outcome.
- `transaction-log`: the record of a fee.
- `message-log`: group chat and notices are not recorded as instant messages; `group-chat` records them.
- `bot-config`: `group` under `[logs]`; `group` as a kind under the deprecated `[perms.autoaccept]` and `[perms.except]`.
- `rlv-restrictions`: `setgroup` gets an effect; `sendim`, `sendimto`, `recvim` and `recvimfrom` apply to group chat.
- `web-server`: group messages on the event stream, kept and replayed as messages are; an event that says the groups or the invitations changed; `group list` and `group invitations` as requests the page makes by itself.
- `web-ui`: the Groups tab: invitations, the list of groups, a group's chat and notices, the box, the three buttons, the confirmation for a fee, the marks for new lines.

## Impact

- New `src/groups.rs`: the membership list and the active group, naming a group, the texts of `group list` and `group info`, the session state of group chat; all without I/O. New `src/grouplog.rs`: the record of a group message, its row and its log line. New `src/commands/group.rs`: the `group` commands and what waits for the grid.
- `src/udp/messages.rs`: `AgentDataUpdateRequest`, `AgentDataUpdate`, `AgentGroupDataUpdate`, `AgentDropGroup`, `ActivateGroup`, `InviteGroupRequest`, `GroupProfileRequest`, `GroupProfileReply`, `JoinGroupRequest`, `JoinGroupReply`; the instant-message dialogs for an invitation and its two answers, a notice, and the start and the lines of a session; the recipient of an instant message.
- `src/caps.rs`, `src/session.rs`: the event-queue forms of the membership list, of a dropped group, of the first line of a session and of the answer to starting one; the `ChatSessionRequest` capability; session events for all of these; the `Grid` trait and its fakes.
- `src/perms.rs`, `src/commands.rs`, `src/control.rs`, `src/main.rs`: the kind `group`, `accept group` and `decline group`, the `group` request and its usage text, dispatch.
- `src/payments.rs`, `src/commands/pay.rs`, `src/txlog.rs`: a group as a payee, the fee in the one slot of a payment, transaction type 1004.
- `src/msglog.rs`, `src/config.rs`, `russelbot.example.toml`: group messages in the recorder beside messages and transactions, the fourth file, `logs.group`.
- `src/rlv.rs`: `setgroup` enforced; group options and `allgroups`.
- `src/web/hub.rs`, `src/web/http.rs`, `src/web/assets/*`: the `group` and `groups` events and a third message ring, the page's own requests, the Groups tab.
- `README.md`: a "Groups" section under "Control socket", the group file in "Chat and IM logs", the kind `group` in "Permission requests", fees in "Payments", the tab in "Web interface", the RLV table; `docs/test-rig.md`, `test-rig/`: the groups store, the fee in the money module, `group.py`, rig items.
- Dependencies: none new.
- Risk: the bot can now join groups and pay their fees. A fee is paid only on a command that names a sum, only from an interface that may pay, and within the limit; nothing that costs L$ is accepted by the bot itself.
