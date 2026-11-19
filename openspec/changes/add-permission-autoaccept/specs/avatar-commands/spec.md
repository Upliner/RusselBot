# Spec Delta

## MODIFIED Requirements

### Requirement: Commands need the avatar in a region
The commands `stand`, `unsit`, `sit`, `tp`, `requesttp`, `accept` (with its alias `accepttp`), `sendtp` and `touch` SHALL be answered with an error, and SHALL send nothing and change nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout. A command SHALL be carried out at once or not at all; it SHALL NOT be kept for later.

#### Scenario: Command between logins
- **WHEN** `rslbot ctl stand` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region, exits with status 1, and the bot's placement mode does not change

#### Scenario: Accept between logins
- **WHEN** `rslbot ctl accept tp` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

### Requirement: Request a teleport from an avatar
`requesttp <avatar> [<message>]` SHALL send that avatar a request to teleport the bot's avatar to them, with the message if one is given. The avatar SHALL be given and resolved as for the `im` command. The command SHALL be answered with success, naming the avatar, once the request has been handed to the simulator connection; success does not mean the avatar agreed. When that avatar sends the bot a teleport offer within the configured `requesttp_timeout` (10 minutes unless configured) of the request, the bot SHALL accept it and follow the teleport. It SHALL accept at most one offer per request. A `persist` or `home` command that is carried out SHALL end the wait: an offer that arrives after it is not accepted by itself. A teleport offer from an avatar the bot has not asked within that time SHALL be decided by the auto-accept rules (see `permission-requests`, "Deciding a request"): the bot SHALL accept it by itself only when `tp` is auto-accepted for that avatar, and otherwise the offer SHALL wait for the `accept tp` command.

#### Scenario: Request accepted
- **WHEN** `rslbot ctl requesttp "Jane Doe"` is run and Jane Doe accepts the request in her viewer
- **THEN** the bot's avatar arrives at Jane Doe's position without further commands

#### Scenario: Unsolicited offer
- **WHEN** an avatar the bot has not asked, and for which `tp` is not auto-accepted, offers the bot a teleport
- **THEN** the avatar stays where it is and the operator log notes the offer, who sent it, and that `rslbot ctl accept tp` would accept it

#### Scenario: Offer after the time limit
- **WHEN** `requesttp_timeout` is not configured and Jane Doe answers a request 15 minutes after it was sent
- **THEN** the bot does not go by itself

#### Scenario: Longer time limit
- **WHEN** `requesttp_timeout = 30` is configured and Jane Doe answers a request 15 minutes after it was sent
- **THEN** the bot accepts the offer and goes

#### Scenario: Sent home while waiting
- **WHEN** `rslbot ctl home` is run after a `requesttp`, and the asked avatar's offer arrives a minute later
- **THEN** the avatar stays on its seat and the offer waits for `accept tp`, unless `tp` is auto-accepted for that avatar

#### Scenario: Persisted while waiting
- **WHEN** `rslbot ctl persist` is run after a `requesttp`, and the asked avatar's offer arrives a minute later
- **THEN** the avatar stays where it was persisted and the offer waits for `accept tp`, unless `tp` is auto-accepted for that avatar

#### Scenario: Unknown avatar
- **WHEN** `rslbot ctl requesttp "Nobody Here"` is run and the grid has no such avatar
- **THEN** the command exits with status 1 naming `Nobody Here` and no request is sent

#### Scenario: Unsolicited offer from an auto-accepted avatar
- **WHEN** the rule for `tp` is the list `Jane Doe` and Jane Doe, whom the bot has not asked, offers the bot a teleport
- **THEN** the bot's avatar arrives at the place Jane Doe offered without a command

### Requirement: Accept a teleport offer on command
The bot SHALL remember, for each avatar, the latest teleport offer that avatar sent it during the current session and that it has not accepted (see `permission-requests`, "Requests that wait"). `accept tp` SHALL accept the remembered offer that was received last and follow the teleport. `accept tp <avatar>` SHALL accept the remembered offer of that avatar; the avatar SHALL be given as one argument, a UUID or a name, and SHALL be compared with the avatars whose offers are remembered as an auto-accept entry is (see `permission-requests`, "Whom an entry stands for"), without a lookup on the grid. `accepttp` SHALL be a deprecated alias of `accept tp` that takes the same optional argument, is carried out and answered in the same way, and is named as deprecated in the usage text. The command SHALL be answered with success, naming the region arrived in, when the avatar has arrived, and with an error giving the reason when the bot remembers no offer, or none from that avatar, when the grid refuses the offer (for instance because it is too old), or when no outcome is known within 60 seconds. An offer SHALL be forgotten once it has been accepted: by this command, by itself under the auto-accept rules, or by itself after a `requesttp`.

#### Scenario: Accepting an unsolicited offer
- **WHEN** Jane Doe has offered the bot a teleport and `rslbot ctl accept tp "Jane Doe"` is run
- **THEN** the bot's avatar arrives at the place Jane Doe offered and the command exits with status 0

#### Scenario: No offer
- **WHEN** `rslbot ctl accept tp "Jane Doe"` is run and Jane Doe has sent no offer in this session
- **THEN** the command exits with status 1 saying there is no offer from Jane Doe, and nothing is sent

#### Scenario: Offer used once
- **WHEN** `accept tp "Jane Doe"` has been carried out and is run again without a new offer
- **THEN** the second command exits with status 1

#### Scenario: Someone asks to be brought to the bot
- **WHEN** an avatar sends the bot a request to be teleported to it
- **THEN** the operator log notes the request and who sent it, and the bot sends no offer unless the operator runs `sendtp`

#### Scenario: Latest offer without a name
- **WHEN** John Roe and then Jane Doe have offered the bot a teleport and `rslbot ctl accept tp` is run
- **THEN** the bot's avatar arrives at the place Jane Doe offered, and John Roe's offer is still remembered

#### Scenario: No offer at all
- **WHEN** `rslbot ctl accept tp` is run and no teleport offer is remembered
- **THEN** the command exits with status 1 saying no teleport offer waits, and nothing is sent

#### Scenario: Avatar by UUID
- **WHEN** Jane Doe has offered the bot a teleport and `rslbot ctl accept tp` is run with Jane Doe's UUID
- **THEN** the bot's avatar arrives at the place Jane Doe offered

#### Scenario: Name the grid does not know
- **WHEN** `rslbot ctl accept tp "Nobody Here"` is run while an offer from Jane Doe is remembered
- **THEN** the command exits with status 1 saying there is no offer from `Nobody Here`, without asking the grid about the name

#### Scenario: Deprecated alias
- **WHEN** Jane Doe has offered the bot a teleport and `rslbot ctl accepttp "Jane Doe"` or `rslbot ctl accepttp` is run
- **THEN** the command does what `accept tp` with the same argument does

### Requirement: Commands that suspend automatic placement
The bot SHALL be in one of two placement modes: holding, in which it works towards the held placement (see `placement-persist`), and manual, in which it does not move or seat the avatar by itself. It SHALL start in holding mode. A `stand`, `unsit`, `sit`, `tp`, `requesttp` or `accept tp` command (or its alias `accepttp`) that is carried out SHALL put the bot in manual mode before it acts; a command that is refused SHALL NOT change the mode. A teleport offer the bot accepts by itself under the auto-accept rules (see `permission-requests`, "Accepting a request") SHALL put the bot in manual mode in the same way, and one it does not accept SHALL NOT change the mode. A `stand`, `sit` or `tp` that is carried out for a forced RLV command, and a teleport by global coordinates carried out for one (see `rlv-commands`, "Force commands"), SHALL put the bot in manual mode in the same way, and one that is refused SHALL NOT change the mode; whether the bot then stays in manual mode or returns to holding mode is decided by the RLV persist setting (see `rlv-commands`, "Forced movement and the held placement"). A forced `@sit` that is carried out through a persist under that setting is no `sit` command and SHALL NOT put the bot in manual mode. `sendtp`, `touch`, `status`, `say`, `shout`, `whisper` and `im` SHALL NOT change the mode, and neither SHALL the `perms` commands, the acceptance of an inventory offer or of a script permission request, by itself or with `accept`, the `rlv` commands by themselves, the RLV queries, or a `detach` or `unwear` carried out for an RLV command. Manual mode SHALL last until a `persist` or `home` command is carried out, until the bot returns to holding mode after a forced RLV command (see `rlv-commands`, "Forced movement and the held placement"), or until the bot process ends, across session losses and logins. Besides those commands, given by the operator or carried out for RLV, and the teleport offers it accepts by itself, the bot enters manual mode by itself only once, after startup, when the config's `persist` setting is off, which it is when the config says `persist = "off"`, has no `persist` key, or has a `[persist]` table whose `atstart` is false. The held placement SHALL NOT be changed by entering or being in manual mode.

#### Scenario: Staying where it was put
- **WHEN** `rslbot ctl tp 128 128 25 Home` has been carried out and an hour passes
- **THEN** the avatar is still in `Home` and the bot has not teleported or sat since

#### Scenario: Refused command
- **WHEN** `rslbot ctl sit chair` is refused while the bot is in holding mode
- **THEN** the bot stays in holding mode

#### Scenario: Persist ends manual mode
- **WHEN** `rslbot ctl persist` is carried out after a `tp` command put the bot in manual mode
- **THEN** the bot is in holding mode and holds the place the avatar is at

#### Scenario: Refused persist
- **WHEN** `rslbot ctl persist cur chair` is refused while the bot is in manual mode
- **THEN** the bot stays in manual mode

#### Scenario: Forced away while holding
- **WHEN** the bot holds a place and sits there, RLV is on with the RLV persist setting `drop`, and an object the avatar owns says `@unsit=force` to its owner
- **THEN** the avatar stands up, the bot is in manual mode, and it does not sit the avatar down again by itself

#### Scenario: Forced away while following
- **WHEN** the bot holds a place and sits there, RLV is on with the RLV persist setting `follow`, and an object the avatar owns says `@unsit=force` to its owner
- **THEN** the avatar stands up, and the bot is in holding mode again with the sit state `stand`

#### Scenario: RLV command that is refused
- **WHEN** the bot is in holding mode, RLV is on, and an object the avatar owns says `@sit:chair=force` to its owner
- **THEN** the bot stays in holding mode

#### Scenario: RLV query while holding
- **WHEN** the bot is in holding mode, RLV is on, and an object the avatar owns says `@versionnew=5` to its owner
- **THEN** the bot answers and stays in holding mode

#### Scenario: Teleport offer accepted by rule while holding
- **WHEN** the bot holds a place and sits there, the rule for `tp` is the list `Jane Doe`, and Jane Doe offers the bot a teleport
- **THEN** the bot is in manual mode, the avatar goes where Jane Doe offered, and the bot does not return it by itself until `persist` or `home`

#### Scenario: Teleport offer that waits
- **WHEN** the bot is in holding mode and an avatar for which `tp` is not auto-accepted offers the bot a teleport
- **THEN** the bot stays in holding mode

#### Scenario: Permission granted while holding
- **WHEN** the bot is in holding mode and `rslbot ctl accept anim` is carried out
- **THEN** the bot stays in holding mode
