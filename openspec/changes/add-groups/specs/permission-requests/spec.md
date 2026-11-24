# Spec Delta

## MODIFIED Requirements

### Requirement: Kinds of request
While connected, the bot SHALL read every teleport offer an avatar sends its avatar, every inventory offer an avatar or an object sends it, every script permission request a script makes of it, and every invitation to a group it gets. Each request SHALL have one or more kinds:

| Kind | Request |
|---|---|
| `tp` | a teleport offer from an avatar |
| `inventory` | an inventory offer, of an item or of a folder, from an avatar or from an object |
| `anim` | a script asks for permission to trigger animations on the avatar, or to override its animations |
| `attach` | a script asks for permission to attach its object to the avatar |
| `controls` | a script asks for permission to take the avatar's controls |
| `camera` | a script asks for permission to track the avatar's camera, or to control it |
| `group` | an invitation to join a group, sent by a member of that group |

A script permission request SHALL have the kind of every permission it asks for. A script permission request that asks for a permission without a kind in this table (to take money from the avatar, to teleport it, or any other) SHALL NOT be accepted, by itself or on command, and SHALL NOT be kept; no answer SHALL be sent for it. An invitation that carries nothing by which it can be answered SHALL NOT be kept either, and the operator log SHALL say so. Messages of other sorts that a viewer shows as a dialog (friendship offers, group notices, script dialogs) are not requests of this capability and SHALL be left unanswered; group notices are read as `group-chat` says.

#### Scenario: Request with two kinds
- **WHEN** a script asks for permission to trigger animations and to take controls in one request
- **THEN** the request has the kinds `anim` and `controls`

#### Scenario: Animation override
- **WHEN** a script asks for permission to override the avatar's animations
- **THEN** the request has the kind `anim`

#### Scenario: Request for money
- **WHEN** a script asks for permission to take money from the avatar and to trigger animations in one request
- **THEN** the bot sends no answer, the request cannot be accepted with `accept`, and the operator log says which permission stands in the way

#### Scenario: Folder offered
- **WHEN** an avatar offers the bot's avatar a folder
- **THEN** the bot has read a request of kind `inventory`

#### Scenario: Invitation
- **WHEN** Jane Doe invites the bot's avatar into the group `Builders`
- **THEN** the bot has read a request of kind `group`

### Requirement: Whom an entry stands for
An entry SHALL match a request as this table says, and in no other way:

| Request | A UUID entry matches when it is | A name entry matches when it is the name of |
|---|---|---|
| Teleport offer | the offering avatar's UUID | the offering avatar |
| Inventory offer from an avatar | the giving avatar's UUID | the giving avatar |
| Inventory offer from an object owned by an avatar | the owner's UUID | the owner |
| Inventory offer from an object owned by a group | never | never |
| Script permission request from an object owned by an avatar | the object's UUID or the owner's UUID | the owner |
| Script permission request from an object owned by a group | the object's UUID | never |
| Invitation to a group | the group's UUID or the inviter's UUID | the inviter |

An avatar's name SHALL be compared as the grid gives it with the request, normalised as entries are, so that `Jane Resident`, `jane.resident` and `jane` are one name. The bot SHALL NOT look an entry up on the grid, and an entry that names no avatar or object the grid knows SHALL simply match nothing. An inventory offer from an object does not say which object sent it, so an object's UUID SHALL NOT match an inventory offer. A group's name SHALL NOT match an entry: a group is entered in a rule by its UUID.

#### Scenario: Owner by name
- **WHEN** the rule for `anim` is the list `Jane Doe` and a script in an object Jane Doe owns asks for permission to trigger animations
- **THEN** the entry matches the request

#### Scenario: Object by UUID
- **WHEN** the rule for `anim` lists the UUID of a dance ball and a script in that dance ball asks for permission to trigger animations
- **THEN** the entry matches the request, whoever owns the dance ball

#### Scenario: Owner by UUID
- **WHEN** the rule for `attach` lists Jane Doe's UUID and a script in an object she owns asks for permission to attach
- **THEN** the entry matches the request

#### Scenario: Single-word name
- **WHEN** the rule for `tp` is the list `sitbot` and the avatar `sitbot Resident` offers a teleport
- **THEN** the entry matches the request

#### Scenario: Group with an avatar's name
- **WHEN** the rule for `anim` is the list `Jane Doe` and a script in an object owned by a group named `Jane Doe` asks for permission to trigger animations
- **THEN** the entry does not match the request

#### Scenario: Object UUID and an inventory offer
- **WHEN** the rule for `inventory` lists the UUID of a vendor object and that object gives the avatar a notecard
- **THEN** the entry does not match the request

#### Scenario: Object's owner and an inventory offer
- **WHEN** the rule for `inventory` is the list `Jane Doe` and an object Jane Doe owns gives the avatar a notecard
- **THEN** the entry matches the request

#### Scenario: Group by UUID
- **WHEN** the rule for `group` lists the UUID of the group `Builders` and John Roe invites the avatar into `Builders`
- **THEN** the entry matches the request, whoever sent the invitation

#### Scenario: Inviter by name
- **WHEN** the rule for `group` is the list `Jane Doe` and Jane Doe invites the avatar into a group
- **THEN** the entry matches the request, whichever group it is

#### Scenario: Group with an avatar's name and an invitation
- **WHEN** the rule for `group` is the list `Jane Doe` and John Roe invites the avatar into a group named `Jane Doe`
- **THEN** the entry does not match the request

### Requirement: Perms status command
`perms status` SHALL take no arguments and SHALL be carried out whether or not the avatar is connected. It SHALL answer with one line for every kind, in the order `tp`, `inventory`, `anim`, `attach`, `controls`, `camera`, `group`: the kind, a colon, and the rule as `all`, as `none` for an empty list, or as its entries separated by commas, followed by the word `except` and the exceptions separated by commas when the kind has exceptions. After these it SHALL print one line for every waiting request, oldest first, that starts with `waiting:` and gives the request's kinds, who sent it and, for an inventory offer, what is offered; for an invitation it gives the group by name and UUID, the inviter and, when joining costs L$, the fee.

#### Scenario: Rules from the config
- **WHEN** the config says `tp = ["Jane Doe"]` and `anim = "all"` under `[perms.autoaccept]` and `anim = ["01234567-9abc-defe-dcba-987654321000"]` under `[perms.except]`, and `rslbot ctl perms status` is run
- **THEN** it prints `tp: Jane Doe`, `inventory: none`, `anim: all except 01234567-9abc-defe-dcba-987654321000`, `attach: none`, `controls: none`, `camera: none` and `group: none`, one to a line

#### Scenario: Waiting request shown
- **WHEN** a dance ball's request for permission to trigger animations and to take controls waits and `rslbot ctl perms status` is run
- **THEN** the answer has a line starting with `waiting:` that names `anim` and `controls`, the dance ball with its UUID, and its owner

#### Scenario: Between logins
- **WHEN** `rslbot ctl perms status` is run while the bot is waiting to log in again
- **THEN** it prints the seven rule lines and exits with status 0

#### Scenario: Waiting invitation shown
- **WHEN** Jane Doe's invitation into `Builders`, which costs L$250 to join, waits and `rslbot ctl perms status` is run
- **THEN** the answer has a line starting with `waiting:` that names `group`, `Builders` with its UUID, Jane Doe and `L$250`

### Requirement: Accept command
`accept <kind> [<who>]` SHALL accept one waiting request of that kind. For every kind but `group` that is: without `<who>` the one received last, and with `<who>`, an entry given as one argument, the one received last among those the entry matches (see "Whom an entry stands for"). For `group`, which invitation is accepted, what `<who>` may be, and the further argument `--pay` are said in "Accept group command", and what this requirement says of `<who>` and of more than one `<who>` SHALL NOT apply. The bot SHALL NOT look `<who>` up on the grid. A script permission request SHALL be found under each of its kinds, and accepting it SHALL grant everything it asks for, whichever of its kinds the command named. The command SHALL be answered with an error when the avatar is not connected to a region, when the kind is not one of the kinds, when `<who>` is neither a UUID nor a name of one or two words, when more than one `<who>` is given, and when no request of that kind waits, or none from `<who>`; the last error SHALL name the kind and `<who>`. It SHALL be answered:

- for `tp` as `avatar-commands` says ("Accept a teleport offer on command");
- for `inventory` with success once the answer has been handed to the simulator connection, saying what was accepted and from whom, and with an error that leaves the offer waiting when the destination folder cannot be found;
- for `anim`, `attach`, `controls` and `camera` with success once the answer has been handed to the simulator connection, naming the object, its owner, and every permission that was granted;
- for `group` as "Accept group command" and "Paying the fee of an invitation" say.

A request the command accepted SHALL be forgotten. `accept inventory` and `accept` of a script permission request SHALL NOT change the bot's placement mode.

#### Scenario: Latest animation request
- **WHEN** two scripts' requests for permission to trigger animations wait and `rslbot ctl accept anim` is run
- **THEN** the script that asked last is granted the permission, the other request still waits, and the command exits with status 0

#### Scenario: Request of a named object
- **WHEN** two scripts' requests for permission to trigger animations wait and `rslbot ctl accept anim` is run with the UUID of the object that asked first
- **THEN** that object's script is granted the permission and the other request still waits

#### Scenario: Request of a named owner
- **WHEN** `rslbot ctl accept attach "Jane Doe"` is run and a request to attach from an object Jane Doe owns waits
- **THEN** that request is granted

#### Scenario: Request with two kinds
- **WHEN** a request for permission to trigger animations and to take controls waits and `rslbot ctl accept anim` is run
- **THEN** both permissions are granted, the answer names both, and no request waits under `controls`

#### Scenario: Inventory offer accepted on command
- **WHEN** an avatar that is not auto-accepted has given the bot's avatar an object and `rslbot ctl accept inventory` is run
- **THEN** the giver is told the offer was accepted and the command exits with status 0 naming the object and the giver

#### Scenario: Nothing waits
- **WHEN** `rslbot ctl accept anim` is run and no request of kind `anim` waits
- **THEN** the command exits with status 1 saying no such request waits, and nothing is sent

#### Scenario: Nothing from that avatar
- **WHEN** `rslbot ctl accept inventory "Jane Doe"` is run and only an offer from John Roe waits
- **THEN** the command exits with status 1 naming `inventory` and `Jane Doe`, and John Roe's offer still waits

#### Scenario: Request used once
- **WHEN** `rslbot ctl accept anim` has been carried out and is run again without a new request
- **THEN** the second command exits with status 1

## ADDED Requirements

### Requirement: Learning the group and the inviter of an invitation
An invitation names its group by UUID only and its inviter by name only, and says what joining costs. Before it decides an invitation the bot SHALL ask the grid for the group's profile (see `groups`, "Asking the grid for a group's profile"), for the group's name, and SHALL look the inviter's name up on the grid, where only an exact match of the full name counts, for the inviter's UUID. It SHALL wait for both for up to 10 seconds and SHALL then decide the invitation with what it has: without the inviter's UUID only the group's UUID and the inviter's name can match an entry, and an invitation whose group's name is not known is shown and named by the group's UUID. The fee of an invitation SHALL be the one the invitation itself gives, as a viewer shows it. Neither lookup SHALL delay the bot's simulator traffic or the answer to any command.

#### Scenario: Inviter by UUID
- **WHEN** the rule for `group` lists Jane Doe's UUID and Jane Doe invites the avatar into `Builders`
- **THEN** the bot looks her name up, finds her UUID, and accepts the invitation by itself

#### Scenario: Grid does not find the inviter
- **WHEN** the rule for `group` lists Jane Doe's UUID, she invites the avatar, and the grid does not answer the lookup of her name within 10 seconds
- **THEN** the invitation is not accepted by itself and waits for the operator, named by its group and by `Jane Doe`

#### Scenario: Group's name for an invitation that waits
- **WHEN** nothing is auto-accepted for `group` and John Roe invites the avatar into `Builders`
- **THEN** the invitation waits with the group's name known, and `rslbot ctl accept group Builders` accepts it

### Requirement: Invitations with a fee are not accepted by themselves
Whatever "Deciding a request" says, the bot SHALL accept an invitation by itself only when the invitation gives no fee: an invitation into a group that costs L$ to join SHALL wait for the operator, also when the rule for `group` is `all` or an entry matches it. The operator log SHALL say that the fee is why it waits.

#### Scenario: Free group of a listed inviter
- **WHEN** the rule for `group` is the list `Jane Doe` and Jane Doe invites the avatar into a group that costs nothing to join
- **THEN** the bot accepts the invitation by itself and the avatar is in the group

#### Scenario: Paid group under all
- **WHEN** the rule for `group` is `all` and Jane Doe invites the avatar into a group that costs L$250 to join
- **THEN** nothing is sent, nothing is paid, and the invitation waits; the log names the fee and the `rslbot ctl accept group` command with `--pay`

### Requirement: Accepting and declining an invitation
Accepting an invitation, by itself or on command, SHALL answer it as a viewer does when its user presses Join, and declining one SHALL answer it as a viewer does when its user presses Decline. Neither SHALL move the avatar or change the bot's placement mode, and no RLV restriction SHALL be applied to an invitation, when it arrives, when it is decided or when it is answered. The bot SHALL take an invitation as accepted when the grid lists the group among the avatar's groups (see `groups`, "The avatar's groups"): an invitation the bot accepted by itself whose group is not listed within 15 seconds SHALL be noted in the operator log as not confirmed, and SHALL NOT be answered a second time.

#### Scenario: Seated bot stays seated
- **WHEN** the bot is in holding mode with the avatar seated and an invitation is accepted by itself
- **THEN** the avatar stays seated, the bot stays in holding mode, and `group list` lists the group

#### Scenario: RLV does not apply
- **WHEN** a collar holds `recvim`, `sendim` and `setgroup`, the rule for `group` is `all`, and Jane Doe invites the avatar into a group without a fee
- **THEN** the bot accepts the invitation

### Requirement: Invitations that wait
An invitation that was not accepted by itself SHALL wait until it is accepted with `accept group`, declined with `decline group`, or forgotten. The bot SHALL keep the latest invitation into each group, and at least the 100 it received last; when it forgets one to keep to that bound it SHALL forget the oldest. A waiting invitation SHALL be forgotten when a newer one into the same group arrives, once it has been accepted or declined, when the avatar becomes a member of its group in another way, and when the session ends. A change of the rules SHALL NOT accept an invitation that is already waiting.

#### Scenario: Invited twice into one group
- **WHEN** Jane Doe and then John Roe invite the avatar into `Builders` and neither invitation is accepted by itself
- **THEN** one invitation into `Builders` waits, John Roe's

#### Scenario: Joined another way
- **WHEN** an invitation into the open group `Dancers` waits and `rslbot ctl group join <uuid>` makes the avatar a member of `Dancers`
- **THEN** no invitation into `Dancers` waits

#### Scenario: Session lost
- **WHEN** an invitation waits and the session is lost
- **THEN** after the next login no invitation waits

### Requirement: Accept group command
`accept group [<who>]` SHALL accept the one waiting invitation that `<who>` names. `<who>` SHALL be one argument and SHALL name an invitation when it is the UUID of the invitation's group or of its inviter, when it is the name of its group, compared without regard to letter case and as a whole, or when it is the name of its inviter in one of the forms of an entry (see "Auto-accept rules"). Without `<who>` every waiting invitation is named. The command SHALL be answered with an error, and SHALL answer no invitation, when the avatar is not connected to a region, when no invitation waits or none that `<who>` names, and when more than one is named; the last error SHALL say so and list each invitation that is named, with its group's UUID, its group's name and its inviter, so that the command can be given again with a UUID.

An invitation without a fee SHALL be answered as accepted, and the command SHALL be answered with success, naming the group and the inviter, when the grid lists the group among the avatar's groups, and with an error saying that the grid has not confirmed it when the group is not listed within 15 seconds. The invitation SHALL be forgotten once the answer has been sent, whatever the grid then says. `accept group` SHALL NOT change the bot's placement mode.

#### Scenario: By group name
- **WHEN** invitations into `Builders` and `Dancers` wait and `rslbot ctl accept group builders` is run
- **THEN** the avatar joins `Builders`, the invitation into `Dancers` still waits, and the command exits with status 0 naming `Builders` and the inviter

#### Scenario: By inviter
- **WHEN** Jane Doe's invitation into `Builders` waits and `rslbot ctl accept group "Jane Doe"` is run
- **THEN** the avatar joins `Builders`

#### Scenario: Only one waits
- **WHEN** one invitation waits and `rslbot ctl accept group` is run
- **THEN** that invitation is accepted

#### Scenario: Inviter of two groups
- **WHEN** Jane Doe's invitations into `Builders` and into `Dancers` wait and `rslbot ctl accept group "Jane Doe"` is run
- **THEN** the command exits with status 1, its error lists both groups with their UUIDs, and neither invitation is answered

#### Scenario: Several wait and none is named
- **WHEN** two invitations wait and `rslbot ctl accept group` is run
- **THEN** the command exits with status 1 listing both, and neither is answered

#### Scenario: Two groups with one name
- **WHEN** invitations into two groups named `Friends` wait and `rslbot ctl accept group Friends` is run
- **THEN** the command exits with status 1 listing the UUIDs of both groups

#### Scenario: Nothing from that avatar
- **WHEN** `rslbot ctl accept group "Jane Doe"` is run and only an invitation from John Roe waits
- **THEN** the command exits with status 1 naming `group` and `Jane Doe`, and John Roe's invitation still waits

### Requirement: Paying the fee of an invitation
`accept group` SHALL take `--pay <sum>`, before or after `<who>`, where `<sum>` is a whole number of L$ from 0 to 2147483647 written in digits only; any other sum SHALL be refused with an error naming it. An invitation that gives a fee greater than zero SHALL be accepted only when `--pay` is given with a sum that is not less than the fee; otherwise the command SHALL be refused with an error that names the fee and `--pay`, nothing SHALL be sent, and the invitation SHALL keep waiting. The fee, not the sum, is what the avatar pays, and it SHALL be paid as `payments`, "Fee for joining a group", says: only from an interface that may pay, within the limit and the lists, and with its outcome as a payment's; an invitation whose fee the bot refuses to pay SHALL keep waiting. `--pay` given for an invitation without a fee SHALL have no effect.

#### Scenario: Fee covered
- **WHEN** `payments.perms` has `ctl`, an invitation into `Builders` with a fee of L$250 waits, and `rslbot ctl accept group Builders --pay 250` is run
- **THEN** the avatar joins `Builders`, pays L$250, and the answer names `Builders` and `L$250`

#### Scenario: No sum
- **WHEN** an invitation into `Builders` with a fee of L$250 waits and `rslbot ctl accept group Builders` is run
- **THEN** the command exits with status 1 naming `L$250` and `--pay`, nothing is sent, and the invitation still waits

#### Scenario: Refused for the limit
- **WHEN** L$100 are left of the limit and `rslbot ctl accept group Builders --pay 250` is run for an invitation with a fee of L$250
- **THEN** the command exits with status 1 saying that L$100 are left, and the invitation still waits

### Requirement: Decline command
`decline group [<who>]` SHALL decline the one waiting invitation that `<who>` names, with `<who>` read, and more than one or none named answered, as "Accept group command" says. It SHALL be answered with success once the answer has been handed to the simulator connection, naming the group and the inviter, and the invitation SHALL be forgotten. It SHALL pay nothing and SHALL take no `--pay`. `decline` with another kind than `group` SHALL be answered with an error saying that only invitations to groups can be declined. `decline group` SHALL NOT change the bot's placement mode.

#### Scenario: Declined
- **WHEN** Jane Doe's invitation into `Builders` waits and `rslbot ctl decline group Builders` is run
- **THEN** the grid is told that the avatar declines, the avatar is not in `Builders`, no invitation waits, and the command exits with status 0

#### Scenario: Another kind
- **WHEN** `rslbot ctl decline tp "Jane Doe"` is run
- **THEN** the command exits with status 1 saying that only invitations to groups can be declined, and nothing is sent

### Requirement: Group invitations command
`group invitations` SHALL take no further arguments and SHALL be answered with success and one line for each waiting invitation, oldest first. A line SHALL have five fields separated by tabs: the group's UUID; the fee in L$, in digits, `0` for none; the inviter's UUID, or `?` when the bot has not learned it; the inviter's name; and the group's name, or `?` when the bot has not learned it; with a tab, line feed, carriage return or backslash in a name written as `\t`, `\n`, `\r` and `\\`. The answer SHALL have no text when no invitation waits. The command SHALL be held to what `groups` says of `group` commands that only read.

#### Scenario: Two invitations
- **WHEN** Jane Doe's invitation into `Builders`, with a fee of L$250, and then John Roe's into the free group `Dancers` wait
- **THEN** `group invitations` prints a line with the UUID of `Builders`, `250`, Jane Doe's UUID, `Jane Doe` and `Builders`, and then one with the UUID of `Dancers`, `0`, John Roe's UUID, `John Roe` and `Dancers`

#### Scenario: None
- **WHEN** no invitation waits
- **THEN** `group invitations` is answered with success and no text

### Requirement: Invitations appear in the operator log
For an invitation, the line "Requests appear in the operator log" asks for SHALL name the group by its UUID and, once known, its name, the inviter, and the fee when there is one, and SHALL give `rslbot ctl accept group` with the group's UUID, and with `--pay` and the fee when there is one, as the command that accepts it. The bot SHALL write a line for every `accept group` and `decline group` command it carries out or refuses, with the reason.

#### Scenario: Invitation that waits
- **WHEN** Jane Doe invites the avatar into `Builders`, which costs L$250, and `group` is not auto-accepted for her
- **THEN** the log has a line naming `Builders`, its UUID, Jane Doe and `L$250`, and saying that `rslbot ctl accept group <uuid> --pay 250` accepts it
