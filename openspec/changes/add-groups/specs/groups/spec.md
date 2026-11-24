# Spec Delta

## Purpose

The groups of the bot's avatar: learning from the grid which groups the avatar is in and which one is active, and the commands by which an operator lists them, activates one, invites an avatar into one, reads a group's profile and joins a group, with or without a fee. Group chat and notices are `group-chat`; invitations the avatar receives are `permission-requests`.

## ADDED Requirements

### Requirement: The avatar's groups
While connected, the bot SHALL know the groups the avatar is in, each with its UUID, its name, whether the avatar may invite into it and whether the avatar gets its notices, and which of them, if any, is the active group. It SHALL ask the grid for them once after each login, as a viewer does, SHALL take them in whichever of the two ways the grid sends them, as a message of the simulator connection or as an event of the simulator's event queue, and SHALL follow every later word of the grid about them: a new list, a group the avatar was dropped from, and a change of the active group. What the bot knows of the groups SHALL be forgotten when the session ends.

#### Scenario: After login
- **WHEN** the avatar is in the groups `Builders` and `Dancers`, `Builders` being active, and the bot has logged in
- **THEN** `rslbot ctl group list` lists both, `Builders` with the flag `active`

#### Scenario: Grid that sends the list on the event queue
- **WHEN** the simulator sends the list of the avatar's groups as an event and not as a message
- **THEN** the bot knows the groups all the same

#### Scenario: Ejected
- **WHEN** an officer of `Dancers` ejects the avatar while the bot runs
- **THEN** `group list` no longer lists `Dancers`

#### Scenario: Joined from a viewer earlier
- **WHEN** the avatar joined a group from a viewer yesterday and the bot logs in today
- **THEN** the group is in `group list`

### Requirement: Naming a group
Where a command takes `<group>`, it SHALL be one argument: a UUID, or the name of a group the avatar is in. A name SHALL be compared with the names of the avatar's groups without regard to letter case and SHALL match as a whole only. When no group of the avatar has that name the command SHALL be answered with an error naming it. When more than one has it the command SHALL be answered with an error that says so and lists each of them with its UUID, so that the command can be given again with a UUID, and SHALL do nothing else. A UUID SHALL name the group with that UUID; for `group activate`, `group invite` and `group chat` it SHALL be answered with an error when the avatar is in no such group.

#### Scenario: By name
- **WHEN** the avatar is in `Builders` and `rslbot ctl group activate builders` is run
- **THEN** `Builders` is the group the command acts on

#### Scenario: Name with spaces
- **WHEN** the avatar is in `Night Owls` and `rslbot ctl group chat "Night Owls" hello` is run
- **THEN** `hello` is said in `Night Owls`

#### Scenario: Two groups with one name
- **WHEN** the avatar is in two groups named `Friends` and `rslbot ctl group chat Friends hello` is run
- **THEN** the command exits with status 1, its error lists the UUIDs of both groups, and nothing is said in either

#### Scenario: Part of a name
- **WHEN** the avatar is in `Builders` only and `rslbot ctl group activate Build` is run
- **THEN** the command exits with status 1 naming `Build`

#### Scenario: Group the avatar is not in
- **WHEN** `rslbot ctl group chat <uuid> hello` names a group the avatar is not in
- **THEN** the command exits with status 1 saying that the avatar is not in that group, and nothing is sent

### Requirement: Group commands need the avatar in a region
Every `group` command SHALL be answered with an error and send nothing when the avatar is not connected to a region or is in the middle of a teleport or a logout. A `group` command that needs the avatar's groups and arrives before the grid has told them after a login SHALL wait for them for up to 10 seconds, and SHALL be answered with an error saying that the grid has not told the avatar's groups when they have not come by then.

#### Scenario: Between logins
- **WHEN** `rslbot ctl group list` is run while the bot is waiting to log in again
- **THEN** the command exits with status 1 saying that the avatar is not connected

#### Scenario: Right after login
- **WHEN** `group list` arrives a moment after the avatar entered its first region, before the grid has sent the groups, and the grid sends them a second later
- **THEN** the command is answered then, with the list

### Requirement: Group list command
`group list` SHALL take no further arguments and SHALL be answered with success and one line for each group the avatar is in, ordered by name without regard to letter case. A line SHALL have three fields separated by tabs: the group's UUID; its flags, separated by commas and in this order, `active` for the active group, `invite` for a group the avatar may invite into, `notices` for a group whose notices the avatar gets, or `-` for none; and the group's name, with a tab, line feed, carriage return or backslash in it written as `\t`, `\n`, `\r` and `\\`. The answer SHALL have no text when the avatar is in no group.

#### Scenario: Listing
- **WHEN** the avatar is in `Dancers` and in `Builders`, which is active and into which it may invite, and gets the notices of both
- **THEN** `group list` prints a line with the UUID of `Builders`, `active,invite,notices` and `Builders`, and then one with the UUID of `Dancers`, `notices` and `Dancers`

#### Scenario: No groups
- **WHEN** the avatar is in no group
- **THEN** `group list` is answered with success and no text

### Requirement: Group activate command
`group activate <group>` SHALL make that group the avatar's active group, as a viewer's Activate does, and `group activate none` SHALL leave the avatar with no active group; a group named `none` is activated by its UUID. The words of a name MAY be given as separate arguments and SHALL then be joined by single spaces. The command SHALL be answered with success, naming the group and the title the avatar now wears, when the grid says that this group is the active one, and with an error saying that the grid has not confirmed it when the grid has not said so within 10 seconds.

#### Scenario: Activate
- **WHEN** `rslbot ctl group activate Builders` is run while `Dancers` is active
- **THEN** other avatars see the tag of `Builders` over the avatar, `group list` shows `Builders` with `active` and `Dancers` without, and the command exits with status 0 naming `Builders`

#### Scenario: None
- **WHEN** `rslbot ctl group activate none` is run
- **THEN** the avatar wears no group tag and no line of `group list` has `active`

#### Scenario: Name in two words
- **WHEN** `rslbot ctl group activate Night Owls` is run
- **THEN** it is read as the group `Night Owls`

### Requirement: Group invite command
`group invite <group> <avatar>` SHALL invite the avatar into the group, into its role Everyone, as a viewer's Invite does. `<avatar>` SHALL be one argument, taken as `im` takes its recipient (see `message-send`, "Recipient resolution"). The command SHALL be refused with an error, and nothing SHALL be sent, when the bot's avatar may not invite into that group and when `<avatar>` is the bot's own avatar. It SHALL be answered with success once the invitation has been handed to the simulator connection, naming the avatar and the group and saying that the grid does not report whether an invitation arrived.

#### Scenario: Invite by name
- **WHEN** the avatar may invite into `Builders` and `rslbot ctl group invite Builders "Jane Doe"` is run
- **THEN** Jane Doe gets an invitation to `Builders` from the bot's avatar, and the command exits with status 0 naming Jane Doe and `Builders`

#### Scenario: No right to invite
- **WHEN** the avatar is in `Dancers` without the right to invite and `rslbot ctl group invite Dancers "Jane Doe"` is run
- **THEN** the command exits with status 1 saying that the avatar may not invite into `Dancers`, and nothing is sent

#### Scenario: Unknown avatar
- **WHEN** `rslbot ctl group invite Builders "Nobody Here"` is run and the grid knows no such avatar
- **THEN** the command exits with status 1 naming `Nobody Here`

### Requirement: Group info command
`group info <group>` SHALL ask the grid for the group's profile and SHALL be answered with success and a text of `name: value` lines, in this order: `name`; `group`, the UUID; `founder`, the founder's login name, or its UUID when the grid does not supply the name within 5 seconds; `fee`, `L$` and the cost of joining, or `none`; `enrollment`, `open` when anyone may join and `closed` when joining takes an invitation; `members` and `roles`, the numbers the grid gives; `listed`, `yes` or `no`, whether the group shows in search; `mature`, `yes` or `no`; `member`, `yes` or `no`, whether the bot's avatar is in the group; `title`, the avatar's title in the group, left out when it is in none or has no title; and last `charter`, the group's description. A tab, line feed, carriage return or backslash in a value SHALL be written as `\t`, `\n`, `\r` and `\\`, so that every value is one line. `<group>` SHALL be a UUID, of any group, or the name of a group the avatar is in; the words of a name MAY be separate arguments. The command SHALL change nothing.

#### Scenario: Group the avatar is not in
- **WHEN** `rslbot ctl group info <uuid>` names the group `Builders` that Jane Doe founded, which costs L$250 to join and is open
- **THEN** it prints `name: Builders`, the `group` line, `founder: Jane Doe`, `fee: L$250`, `enrollment: open`, the other lines with `member: no` and no `title` line, and the charter, and exits with status 0

#### Scenario: Charter of several lines
- **WHEN** a group's charter has three lines
- **THEN** the `charter` line has `\n` between them and is one line

#### Scenario: Free group
- **WHEN** a group costs nothing to join
- **THEN** `group info` prints `fee: none`

### Requirement: Asking the grid for a group's profile
For `group info`, for `group join` and for an invitation (see `permission-requests`) the bot SHALL wait for the grid's answer about a group for up to 10 seconds; a command SHALL be answered with an error saying that the grid knows no such group or has not answered when it has not come by then. An answer the bot got less than 30 seconds ago SHALL be used again without asking the grid.

#### Scenario: Unknown group
- **WHEN** `rslbot ctl group info <uuid>` names no group
- **THEN** the command exits with status 1 after 10 seconds

#### Scenario: Info, then join
- **WHEN** `group info <uuid>` has been answered and `group join <uuid>` follows five seconds later
- **THEN** the join is decided on the answer the bot has, without a second request for the profile

### Requirement: Group join command
`group join <uuid>` SHALL make the avatar a member of the group with that UUID, as a viewer's Join does, and SHALL take the group by its UUID only. Before it sends anything it SHALL ask for the group's profile (see "Asking the grid for a group's profile") and SHALL refuse the command with an error, sending nothing, when the avatar is in that group already, and when the group is not open to join, saying that joining it takes an invitation. A group without a fee SHALL be joined whether or not the config has a `[payments]` table. The command SHALL be answered with success, naming the group, when the grid says that the avatar has joined; with an error holding the grid's reason, when it gives one, when the grid refuses; and with an error saying that the grid has not confirmed the join when the grid has said neither within 15 seconds.

#### Scenario: Free group
- **WHEN** `rslbot ctl group join <uuid>` names the open group `Dancers`, which has no fee
- **THEN** the avatar is in `Dancers`, `group list` lists it, and the command exits with status 0 naming `Dancers`

#### Scenario: By name
- **WHEN** `rslbot ctl group join Dancers` is run
- **THEN** the command exits with status 1 saying that a group is joined by its UUID

#### Scenario: Closed group
- **WHEN** `rslbot ctl group join <uuid>` names a group whose enrollment is closed
- **THEN** the command exits with status 1 saying that joining it takes an invitation, and nothing is sent

#### Scenario: Already a member
- **WHEN** `rslbot ctl group join <uuid>` names a group the avatar is in
- **THEN** the command exits with status 1 saying so

### Requirement: Joining a group with a fee
`group join` SHALL take `--pay <sum>` after the UUID, where `<sum>` is a whole number of L$ from 0 to 2147483647 written in digits only; any other sum SHALL be refused with an error naming it. A group whose profile gives a fee greater than zero SHALL be joined only when `--pay` is given with a sum that is not less than the fee; otherwise the command SHALL be refused with an error that names the fee and `--pay`, and nothing SHALL be sent. The fee, not the sum, is what the avatar pays, and it SHALL be paid as `payments`, "Fee for joining a group", says: only from an interface that may pay, within the limit and the lists, and with its outcome as a payment's. `--pay` given for a group without a fee SHALL have no effect.

#### Scenario: Fee covered
- **WHEN** `payments.perms` has `ctl`, the group `Builders` costs L$250, and `rslbot ctl group join <uuid> --pay 300` is run
- **THEN** the avatar joins `Builders`, pays L$250, and the answer names `Builders` and `L$250`

#### Scenario: No sum
- **WHEN** `Builders` costs L$250 and `rslbot ctl group join <uuid>` is run without `--pay`
- **THEN** the command exits with status 1 naming `L$250` and `--pay`, and nothing is sent

#### Scenario: Sum below the fee
- **WHEN** `Builders` costs L$250 and `rslbot ctl group join <uuid> --pay 100` is run
- **THEN** the command exits with status 1 naming `L$250`, and nothing is sent or paid

#### Scenario: Interface that may not pay
- **WHEN** `payments.perms` is `["ctl"]` and the page's console sends `group join <uuid> --pay 250` for a group that costs L$250
- **THEN** the answer is an error saying that payments are not enabled for `web`, and the avatar does not join

#### Scenario: Sum for a free group
- **WHEN** the config has no `[payments]` table and `rslbot ctl group join <uuid> --pay 100` names a group without a fee
- **THEN** the avatar joins and pays nothing

### Requirement: Group commands leave the placement alone
No `group` command SHALL move the avatar or change the bot's placement mode or the held placement. `group list`, `group info` and `group invitations` SHALL only read: no RLV restriction SHALL be applied to them, and none of them SHALL be the exempt command of an RLV override (see `rlv-restrictions`, "Override exempts one command"). No RLV restriction SHALL be applied to `group invite` or `group join`. What RLV does to `group activate` and `group chat` is said in `rlv-restrictions`.

#### Scenario: Seated
- **WHEN** `rslbot ctl group activate Builders` is run while the avatar is seated on the held object
- **THEN** the avatar stays seated and `status` still prints `mode: holding`

#### Scenario: During an override
- **WHEN** `rslbot ctl rlv mode override` has been carried out and `rslbot ctl group list` is run
- **THEN** the list is printed and `rlv status` still prints `rlv: override`

### Requirement: Group commands appear in the operator log
The bot SHALL write a line to its operator log for every `group activate`, `group invite` and `group join` command: the group by name and UUID as far as known, for `group invite` the avatar by name and UUID, and what became of it: refused and why, sent, confirmed, or not confirmed. A join with a fee is logged as `payments`, "Payments appear in the operator log", says in addition. It SHALL write a line when the grid tells the avatar's groups after a login, with their number and the active group, and when the avatar joins or is dropped from a group while the bot runs. `group list`, `group info` and `group invitations` that are carried out SHALL leave no line.

#### Scenario: Activation logged
- **WHEN** `rslbot ctl group activate Builders` is carried out
- **THEN** the log has a line naming `Builders`, its UUID and that it is the active group now

#### Scenario: Dropped from a group
- **WHEN** the avatar is ejected from `Dancers`
- **THEN** the log has a line naming `Dancers` and saying that the avatar is no longer in it
