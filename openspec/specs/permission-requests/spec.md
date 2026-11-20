# permission-requests Specification

## Purpose

Answering what avatars and scripted objects ask of the bot's avatar and a viewer answers with a dialog: teleport offers, inventory offers and script permission requests. The capability defines the kinds of request, the rules by which the bot accepts requests by itself, the requests that wait for the operator, and the `perms` and `accept` commands.

## Requirements

### Requirement: Kinds of request
While connected, the bot SHALL read every teleport offer an avatar sends its avatar, every inventory offer an avatar or an object sends it, and every script permission request a script makes of it. Each request SHALL have one or more kinds:

| Kind | Request |
|---|---|
| `tp` | a teleport offer from an avatar |
| `inventory` | an inventory offer, of an item or of a folder, from an avatar or from an object |
| `anim` | a script asks for permission to trigger animations on the avatar, or to override its animations |
| `attach` | a script asks for permission to attach its object to the avatar |
| `controls` | a script asks for permission to take the avatar's controls |
| `camera` | a script asks for permission to track the avatar's camera, or to control it |

A script permission request SHALL have the kind of every permission it asks for. A script permission request that asks for a permission without a kind in this table (to take money from the avatar, to teleport it, or any other) SHALL NOT be accepted, by itself or on command, and SHALL NOT be kept; no answer SHALL be sent for it. Messages of other sorts that a viewer shows as a dialog (friendship offers, group invitations, group notices, script dialogs) are not requests of this capability and SHALL be left unanswered.

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

### Requirement: Auto-accept rules
For every kind the bot SHALL have an auto-accept rule, which is either `all` or a list of entries, and a list of exceptions, which are entries too. An entry SHALL be a UUID or an avatar's name. A name SHALL be accepted as `First Last`, as `first.last`, or as a single word, which stands for last name `Resident`; names SHALL be compared without regard to letter case. Two entries SHALL be the same entry when they are the same UUID or, compared that way, the same name; a name and a UUID SHALL be different entries even when they belong to one avatar. The bot SHALL start with the rules and exceptions its config gives (see `bot-config`), which without a `perms` table are an empty list and no exceptions for every kind. The `perms` commands SHALL change them while the bot runs; such a change SHALL last until the bot process ends, across session losses and logins, and SHALL NOT be written to the config.

#### Scenario: No perms table
- **WHEN** the config has no `perms` table
- **THEN** the rule of every kind is an empty list, and the bot accepts no request by itself except a teleport offer it asked for with `requesttp`

#### Scenario: Rules survive a session loss
- **WHEN** `rslbot ctl perms autoaccept anim all` has been carried out, the session is lost and the bot logs in again
- **THEN** the rule for `anim` is still `all`

#### Scenario: Rules are not saved
- **WHEN** `rslbot ctl perms autoaccept anim all` has been carried out and the bot is restarted with a config that has no `perms` table
- **THEN** the rule for `anim` is an empty list

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

An avatar's name SHALL be compared as the grid gives it with the request, normalised as entries are, so that `Jane Resident`, `jane.resident` and `jane` are one name. The bot SHALL NOT look an entry up on the grid, and an entry that names no avatar or object the grid knows SHALL simply match nothing. An inventory offer from an object does not say which object sent it, so an object's UUID SHALL NOT match an inventory offer.

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

### Requirement: Learning who is behind a request
A script permission request names its object and, as text only, the object's owner. Before it decides such a request the bot SHALL ask the simulator who owns the object, and SHALL take the owner to be an avatar with the name the request gave only when the simulator's answer names an avatar as the owner. When the simulator has not answered within 10 seconds the bot SHALL decide the request with the owner unknown: only the object's UUID can then match. An inventory offer from an object owned by an avatar names the owner by UUID only; the bot SHALL look that avatar's name up on the grid before it decides the offer, so that a name entry, and an `accept` command that names the owner, can match it. When the grid has not answered within 10 seconds, or has no service to ask, the bot SHALL decide the offer with the name unknown: only the owner's UUID can then match. Neither lookup SHALL delay the bot's simulator traffic or the answer to any command.

#### Scenario: Owner confirmed
- **WHEN** a script permission request says its object is owned by `Jane Doe` and the simulator answers that the object's owner is an avatar
- **THEN** the request is decided as one from an object of the avatar `Jane Doe` with that avatar's UUID

#### Scenario: Simulator does not answer
- **WHEN** the rule for `anim` is the list `Jane Doe`, a script in her object asks for permission to trigger animations, and the simulator does not say who owns the object within 10 seconds
- **THEN** the request is not accepted by itself and waits for the operator

#### Scenario: Owner's name looked up
- **WHEN** the rule for `inventory` is the list `Jane Doe` and an object owned by her gives the avatar an item
- **THEN** the bot looks the owner's name up and accepts the offer by itself

#### Scenario: Owner named for an offer that waits
- **WHEN** nothing is auto-accepted for `inventory` and an object owned by Jane Doe gives the avatar an item
- **THEN** the offer waits with its owner's name known, and `rslbot ctl accept inventory "Jane Doe"` accepts it

#### Scenario: Grid does not name the owner
- **WHEN** the rule for `inventory` lists Jane Doe's UUID, an object owned by her gives the avatar an item, and the grid does not answer the name lookup within 10 seconds
- **THEN** the offer is accepted by itself all the same

### Requirement: Deciding a request
A kind SHALL be auto-accepted for a request when the kind's rule is `all` or one of the rule's entries matches the request, and none of the kind's exceptions matches it. The bot SHALL accept a request by itself when every kind the request has is auto-accepted for it, and it SHALL then accept the whole request. It SHALL NOT accept part of a request: a script permission request of which one kind is not auto-accepted is not accepted by itself at all. A request that is not accepted by itself SHALL wait for the operator (see "Requests that wait"). The bot SHALL NOT send a refusal for a request by itself. A teleport offer from an avatar the bot asked with `requesttp` SHALL be accepted as `avatar-commands` says, whatever the rules and exceptions of `tp` are.

#### Scenario: Listed avatar
- **WHEN** the rule for `tp` is the list `Jane Doe` and Jane Doe offers the bot a teleport
- **THEN** the bot accepts the offer by itself

#### Scenario: Unlisted avatar
- **WHEN** the rule for `tp` is the list `Jane Doe` and John Roe offers the bot a teleport
- **THEN** the avatar stays where it is and the offer waits

#### Scenario: All with an exception
- **WHEN** the rule for `anim` is `all`, its exceptions list the UUID of one dance ball, and a script in that dance ball asks for permission to trigger animations
- **THEN** the request is not accepted by itself and waits

#### Scenario: Exception for one object of a listed owner
- **WHEN** the rule for `anim` is the list `Jane Doe`, its exceptions list the UUID of one of her objects, and a script in that object asks for permission to trigger animations
- **THEN** the request is not accepted by itself, while a request from another of her objects is

#### Scenario: Two kinds, both auto-accepted
- **WHEN** the rules for `anim` and `controls` are `all` and a script asks for permission to trigger animations and to take controls in one request
- **THEN** the bot grants both permissions

#### Scenario: Two kinds, one auto-accepted
- **WHEN** the rule for `anim` is `all`, the rule for `controls` is an empty list, and a script asks for permission to trigger animations and to take controls in one request
- **THEN** the bot grants neither permission by itself, and the request waits as one request with the kinds `anim` and `controls`

#### Scenario: Exception and requesttp
- **WHEN** the exceptions of `tp` list `Jane Doe`, `rslbot ctl requesttp "Jane Doe"` has been carried out and Jane Doe sends her offer a minute later
- **THEN** the bot accepts the offer

### Requirement: Accepting a request
Accepting a request, by itself or on command, SHALL do what a viewer does when its user accepts the dialog:

- A teleport offer SHALL be accepted and the teleport followed, and the bot SHALL be put in manual mode before it acts (see `avatar-commands`, "Commands that suspend automatic placement"). When the offer cannot be accepted at that moment, for instance because another teleport is under way, an offer the bot was about to accept by itself SHALL wait for the operator instead.
- An inventory offer SHALL be answered as accepted, naming as the destination the folder directly under the top of My Inventory that the grid marks as the system folder for the offered item's type, or the top of My Inventory when there is no such folder or a folder was offered. The bot SHALL read the top level of the inventory to find that folder (see `inventory-list`, "Inventory is read on demand"). When the folder cannot be found (the simulator offers no inventory service, the avatar is in the middle of a teleport, or the grid does not answer in time) the offer SHALL NOT be answered: one the bot was about to accept by itself SHALL wait for the operator, and an `accept` command SHALL be answered with an error and leave the offer waiting.
- A script permission request SHALL be answered by granting the script every permission it asked for.

Accepting an inventory offer or a script permission request SHALL NOT move the avatar and SHALL NOT change the bot's placement mode.

#### Scenario: Teleport offer accepted by rule
- **WHEN** the bot holds a place and sits there, the rule for `tp` is the list `Jane Doe`, and Jane Doe offers the bot a teleport
- **THEN** the avatar arrives where Jane Doe offered, and the bot is in manual mode and does not return by itself

#### Scenario: Notecard accepted
- **WHEN** the rule for `inventory` is `all` and an avatar gives the bot's avatar a notecard
- **THEN** the giver is told the offer was accepted, and the notecard is in the avatar's Notecards folder

#### Scenario: No inventory service
- **WHEN** the rule for `inventory` is `all`, the simulator offers no inventory service, and an avatar gives the bot's avatar a notecard
- **THEN** no answer is sent, the offer waits, and the operator log says why it was not accepted

#### Scenario: Animation permission accepted
- **WHEN** the rule for `anim` is `all` and a dance ball's script asks for permission to trigger animations
- **THEN** the script is granted the permission and can animate the avatar

#### Scenario: Seated bot stays seated
- **WHEN** the bot is in holding mode with the avatar seated and a script permission request is accepted by itself
- **THEN** the avatar stays seated and the bot stays in holding mode

### Requirement: Requests that wait
A request that was not accepted by itself SHALL wait until it is accepted with `accept` or forgotten. The bot SHALL keep the latest teleport offer of each avatar, the latest permission request of each script, and every inventory offer, and for each of these three sorts at least the 100 it received last; when it forgets a request to keep to such a bound it SHALL forget the oldest. A request SHALL be forgotten once it has been accepted, and a waiting script permission request also when the same script asks again, whatever becomes of its newer request. All waiting requests SHALL be forgotten when the session ends, and a waiting script permission request also when the avatar leaves the region it was made in. A change of the rules SHALL NOT accept a request that is already waiting.

#### Scenario: Newer request of the same script
- **WHEN** a script whose permission request waits asks again
- **THEN** one request of that script waits, the later one

#### Scenario: Newer request that is accepted
- **WHEN** a script's request for permission to take controls waits, the script then asks for permission to trigger animations, and that request is accepted by itself
- **THEN** no request of that script waits

#### Scenario: Several offers of one avatar
- **WHEN** an avatar that is not auto-accepted gives the bot's avatar three items
- **THEN** three inventory offers wait

#### Scenario: Session lost
- **WHEN** requests wait and the session is lost
- **THEN** after the next login no request waits

#### Scenario: Rule added later
- **WHEN** a teleport offer from Jane Doe waits and `rslbot ctl perms autoaccept tp "Jane Doe"` is carried out
- **THEN** the avatar stays where it is, and the offer still waits for `accept tp`

### Requirement: Perms autoaccept and revoke commands
The control socket SHALL take `perms autoaccept <kind> <who>...` and `perms revoke <kind> <who>...`, where `<kind>` is one of the kinds and each `<who>` is one argument holding an entry, a UUID or an avatar's name, or the single argument `all`. They SHALL be carried out whether or not the avatar is connected, and SHALL change the rules as follows:

| Command | Effect | Warning when |
|---|---|---|
| `perms autoaccept <kind> all` | the kind's rule becomes `all`; its exceptions stay | the rule was `all` already |
| `perms autoaccept <kind> <who>` | the entry is taken out of the kind's exceptions, and is added to the rule when the rule is a list that does not hold it | neither changed anything |
| `perms revoke <kind> all` | the kind's rule becomes an empty list; its exceptions stay | the rule was an empty list already |
| `perms revoke <kind> <who>` | the entry is taken out of the rule when the rule is a list that holds it, and is added to the kind's exceptions when the rule is `all` | the rule is a list that does not hold the entry, or the entry was an exception already |

A command that is carried out SHALL be answered with success and one line for each `<who>`, saying what changed; the line for a `<who>` that the table gives a warning for SHALL start with `warning:` and say that it was auto-accepted, or not auto-accepted, already. The command SHALL be answered with an error, and SHALL change nothing, when the kind is not one of the kinds, when no `<who>` is given, when a `<who>` is neither `all`, a UUID nor a name of one or two words, and when `all` is given together with another `<who>`. A `perms` command SHALL NOT change the bot's placement mode.

#### Scenario: Adding an object and an avatar
- **WHEN** `rslbot ctl perms autoaccept inventory 01234567-9abc-defe-dcba-987654321000 Avatar1` is run and the rule for `inventory` is an empty list
- **THEN** the rule holds both entries and the command exits with status 0 with one line for each

#### Scenario: Already in the list
- **WHEN** `rslbot ctl perms autoaccept inventory Avatar1` is run and the rule for `inventory` already holds `Avatar1`
- **THEN** the command exits with status 0, prints a line starting with `warning:` that says `Avatar1` was auto-accepted already, and the rule is unchanged

#### Scenario: Everyone
- **WHEN** `rslbot ctl perms autoaccept anim all` is run
- **THEN** the rule for `anim` is `all`

#### Scenario: Revoking everything
- **WHEN** `rslbot ctl perms revoke inventory all` is run while the rule for `inventory` holds three entries
- **THEN** the rule is an empty list and no inventory offer is accepted by itself

#### Scenario: Revoking one of a list
- **WHEN** `rslbot ctl perms revoke anim 01234567-9abc-defe-dcba-987654321000` is run and the rule for `anim` is a list holding that UUID
- **THEN** the rule no longer holds it

#### Scenario: Revoking under all
- **WHEN** `rslbot ctl perms revoke anim 01234567-9abc-defe-dcba-987654321000` is run and the rule for `anim` is `all`
- **THEN** the UUID is an exception of `anim`, and requests from that object are no longer accepted by themselves

#### Scenario: Revoking what was not auto-accepted
- **WHEN** `rslbot ctl perms revoke anim 01234567-9abc-defe-dcba-987654321000` is run and the rule for `anim` is a list without that UUID
- **THEN** the command exits with status 0, prints a line starting with `warning:` that says it was not auto-accepted, and nothing changes

#### Scenario: Auto-accepting an exception again
- **WHEN** the rule for `anim` is `all` with one exception, and `rslbot ctl perms autoaccept anim` is run with that entry
- **THEN** `anim` has no exceptions

#### Scenario: Unknown kind
- **WHEN** `rslbot ctl perms autoaccept money all` is run
- **THEN** the command exits with status 1 naming the kinds, and nothing changes

#### Scenario: All among others
- **WHEN** `rslbot ctl perms autoaccept anim all Avatar1` is run
- **THEN** the command exits with status 1 and nothing changes

#### Scenario: Between logins
- **WHEN** `rslbot ctl perms autoaccept tp "Jane Doe"` is run while the bot is waiting to log in again
- **THEN** the command exits with status 0 and the rule applies from the next login on

### Requirement: Perms status command
`perms status` SHALL take no arguments and SHALL be carried out whether or not the avatar is connected. It SHALL answer with one line for every kind, in the order `tp`, `inventory`, `anim`, `attach`, `controls`, `camera`: the kind, a colon, and the rule as `all`, as `none` for an empty list, or as its entries separated by commas, followed by the word `except` and the exceptions separated by commas when the kind has exceptions. After these it SHALL print one line for every waiting request, oldest first, that starts with `waiting:` and gives the request's kinds, who sent it and, for an inventory offer, what is offered.

#### Scenario: Rules from the config
- **WHEN** the config says `tp = ["Jane Doe"]` and `anim = "all"` under `[perms.autoaccept]` and `anim = ["01234567-9abc-defe-dcba-987654321000"]` under `[perms.except]`, and `rslbot ctl perms status` is run
- **THEN** it prints `tp: Jane Doe`, `inventory: none`, `anim: all except 01234567-9abc-defe-dcba-987654321000`, `attach: none`, `controls: none` and `camera: none`, one to a line

#### Scenario: Waiting request shown
- **WHEN** a dance ball's request for permission to trigger animations and to take controls waits and `rslbot ctl perms status` is run
- **THEN** the answer has a line starting with `waiting:` that names `anim` and `controls`, the dance ball with its UUID, and its owner

#### Scenario: Between logins
- **WHEN** `rslbot ctl perms status` is run while the bot is waiting to log in again
- **THEN** it prints the six rule lines and exits with status 0

### Requirement: Accept command
`accept <kind> [<who>]` SHALL accept one waiting request of that kind: without `<who>` the one received last, and with `<who>`, an entry given as one argument, the one received last among those the entry matches (see "Whom an entry stands for"). The bot SHALL NOT look `<who>` up on the grid. A script permission request SHALL be found under each of its kinds, and accepting it SHALL grant everything it asks for, whichever of its kinds the command named. The command SHALL be answered with an error when the avatar is not connected to a region, when the kind is not one of the kinds, when `<who>` is neither a UUID nor a name of one or two words, when more than one `<who>` is given, and when no request of that kind waits, or none from `<who>`; the last error SHALL name the kind and `<who>`. It SHALL be answered:

- for `tp` as `avatar-commands` says ("Accept a teleport offer on command");
- for `inventory` with success once the answer has been handed to the simulator connection, saying what was accepted and from whom, and with an error that leaves the offer waiting when the destination folder cannot be found;
- for `anim`, `attach`, `controls` and `camera` with success once the answer has been handed to the simulator connection, naming the object, its owner, and every permission that was granted.

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

### Requirement: Requests appear in the operator log
The bot SHALL write a line to its operator log for every request it reads, saying who sent it (an avatar with its name and UUID; an object with its name, its UUID where the request gives it, and its owner) and what it asks for (for an inventory offer what is offered, for a script permission request the kinds), and what became of it: that it was accepted by itself, or that it waits, with the `rslbot ctl accept` command that accepts it, or that it cannot be accepted, with the permission that stands in the way. It SHALL write a line when a request that was to be accepted could not be, with the reason, for every `perms autoaccept`, `perms revoke` and `accept` command it carries out, saying what changed or what was accepted, and for every such command it refuses, with the reason.

#### Scenario: Request that waits
- **WHEN** a dance ball owned by Jane Doe asks for permission to trigger animations and `anim` is not auto-accepted for it
- **THEN** the log has a line naming the dance ball, its UUID, Jane Doe and `anim`, and saying that `rslbot ctl accept anim` accepts it

#### Scenario: Request accepted by rule
- **WHEN** the rule for `inventory` is `all` and an avatar gives the bot's avatar a notecard
- **THEN** the log has a line naming the avatar and the notecard and saying the offer was accepted by itself

#### Scenario: Rule changed
- **WHEN** `rslbot ctl perms revoke anim all` is carried out
- **THEN** the log has a line saying that nothing is auto-accepted for `anim` any more
