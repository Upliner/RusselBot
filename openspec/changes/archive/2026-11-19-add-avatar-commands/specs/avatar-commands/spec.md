# Spec Delta

## Purpose

Defines the control-socket commands that move the avatar and make it interact with the world on the operator's command, the commands that report on and stop the bot, and how those commands relate to the bot's automatic placement.

## ADDED Requirements

### Requirement: Commands need the avatar in a region
The commands `stand`, `unsit`, `sit`, `tp`, `requesttp`, `accepttp`, `sendtp` and `touch` SHALL be answered with an error, and SHALL send nothing and change nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout. A command SHALL be carried out at once or not at all; it SHALL NOT be kept for later.

#### Scenario: Command between logins
- **WHEN** `rslbot ctl stand` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region, exits with status 1, and the bot's placement mode does not change

### Requirement: Stand command
The `stand` command, also available under the name `unsit`, SHALL make the avatar stand up from whatever it sits on, object or ground. It SHALL be answered with success once the request has been handed to the simulator connection, whether or not the avatar was sitting.

#### Scenario: Standing up from the target
- **WHEN** `rslbot ctl stand` is run while the avatar is seated on the configured object
- **THEN** other viewers see the avatar standing, the bot reports it as not seated, and it does not sit down again by itself

#### Scenario: Alias
- **WHEN** `rslbot ctl unsit` is run
- **THEN** the effect is that of `rslbot ctl stand`

### Requirement: Sit command
`sit <uuid>` SHALL make the avatar sit on the object with that UUID, standing up first from anything else it sits on. It SHALL be answered with success when the simulator has confirmed the sit, and with an error giving the reason when the simulator refuses or has not confirmed within 15 seconds. `sit ground` SHALL make the avatar sit on the ground where it stands and SHALL be answered with success once the request has been handed to the simulator connection. An argument that is neither a UUID nor the word `ground` SHALL be answered with an error.

#### Scenario: Sitting on another object
- **WHEN** `rslbot ctl sit <uuid>` names an object near the avatar that can be sat on
- **THEN** the avatar is seated on it within 15 seconds and the command exits with status 0

#### Scenario: Object does not exist
- **WHEN** `rslbot ctl sit <uuid>` names no object in the region
- **THEN** the command exits with status 1 within 16 seconds with a reason, and the avatar stays connected

#### Scenario: Sitting on the ground
- **WHEN** `rslbot ctl sit ground` is run while the avatar stands
- **THEN** other viewers see the avatar sitting on the ground

#### Scenario: Not a UUID
- **WHEN** `rslbot ctl sit chair` is run
- **THEN** the command exits with status 1 and nothing is sent

### Requirement: Teleport command
`tp <x> <y> [<z>] [<region>]` SHALL teleport the avatar to those coordinates. `x`, `y` and `z` SHALL be finite, non-negative decimal numbers. A third argument that is a number SHALL be taken as `z`; any other third argument, and everything after `z`, SHALL be taken as the region name, with several arguments joined by single spaces. Without a region name the destination is in the region the avatar is in. Without `z` the destination height is the last height the simulator reported for the avatar; when none is known the command SHALL be answered with an error asking for `z`. The command SHALL be answered with success, naming the region arrived in, when the avatar has arrived, and with an error giving the reason when the teleport fails, the region is not found, or no outcome is known within 60 seconds.

#### Scenario: Inside the current region
- **WHEN** `rslbot ctl tp 100 120` is run while the avatar is in region `Target` at height 25
- **THEN** the avatar is at 100, 120, 25 in `Target`, still on the same simulator, and the command exits with status 0

#### Scenario: With a height
- **WHEN** `rslbot ctl tp 100 120 40` is run
- **THEN** the avatar is teleported to 100, 120, 40 in the current region

#### Scenario: To another region
- **WHEN** `rslbot ctl tp 128 128 25 Home` is run while the avatar is in `Target`
- **THEN** the avatar is in `Home` near 128, 128, 25 without a new login, and the command prints that it arrived in `Home`

#### Scenario: Region name of several words, no height
- **WHEN** `rslbot ctl tp 128 128 Blue Lagoon` is run
- **THEN** the avatar is teleported to region `Blue Lagoon` at the height it had

#### Scenario: Unknown region
- **WHEN** `rslbot ctl tp 128 128 25 Nowhere` is run and the grid has no region `Nowhere`
- **THEN** the command exits with status 1 saying the region was not found, and the avatar stays where it was

#### Scenario: Bad coordinates
- **WHEN** `rslbot ctl tp north 128` or `rslbot ctl tp 128` is run
- **THEN** the command exits with status 1 and nothing is sent

### Requirement: Request a teleport from an avatar
`requesttp <avatar> [<message>]` SHALL send that avatar a request to teleport the bot's avatar to them, with the message if one is given. The avatar SHALL be given and resolved as for the `im` command. The command SHALL be answered with success, naming the avatar, once the request has been handed to the simulator connection; success does not mean the avatar agreed. When that avatar sends the bot a teleport offer within the configured `requesttp_timeout` (10 minutes unless configured) of the request, the bot SHALL accept it and follow the teleport. It SHALL accept at most one offer per request. A `home` command SHALL end the wait: an offer that arrives after it is not accepted by itself. The bot SHALL NOT by itself accept a teleport offer from an avatar it has not asked within that time; such an offer is kept for the `accepttp` command.

#### Scenario: Request accepted
- **WHEN** `rslbot ctl requesttp "Jane Doe"` is run and Jane Doe accepts the request in her viewer
- **THEN** the bot's avatar arrives at Jane Doe's position without further commands

#### Scenario: Unsolicited offer
- **WHEN** an avatar the bot has not asked offers the bot a teleport
- **THEN** the avatar stays where it is and the operator log notes the offer, who sent it, and that `accepttp` would accept it

#### Scenario: Offer after the time limit
- **WHEN** `requesttp_timeout` is not configured and Jane Doe answers a request 15 minutes after it was sent
- **THEN** the bot does not go by itself

#### Scenario: Longer time limit
- **WHEN** `requesttp_timeout = 30` is configured and Jane Doe answers a request 15 minutes after it was sent
- **THEN** the bot accepts the offer and goes

#### Scenario: Sent home while waiting
- **WHEN** `rslbot ctl home` is run after a `requesttp`, and the asked avatar's offer arrives a minute later
- **THEN** the avatar stays on its seat and the offer is kept for `accepttp`

#### Scenario: Unknown avatar
- **WHEN** `rslbot ctl requesttp "Nobody Here"` is run and the grid has no such avatar
- **THEN** the command exits with status 1 naming `Nobody Here` and no request is sent

### Requirement: Accept a teleport offer on command
The bot SHALL remember, for each avatar, the latest teleport offer that avatar sent it during the current session and that it has not accepted. `accepttp <avatar>` SHALL accept the remembered offer from that avatar and follow the teleport; the avatar SHALL be given and resolved as for the `im` command. The command SHALL be answered with success, naming the region arrived in, when the avatar has arrived, and with an error giving the reason when the bot remembers no offer from that avatar, when the grid refuses the offer (for instance because it is too old), or when no outcome is known within 60 seconds. An offer SHALL be forgotten once it has been accepted, by this command or by itself after a `requesttp`.

#### Scenario: Accepting an unsolicited offer
- **WHEN** Jane Doe has offered the bot a teleport and `rslbot ctl accepttp "Jane Doe"` is run
- **THEN** the bot's avatar arrives at the place Jane Doe offered and the command exits with status 0

#### Scenario: No offer
- **WHEN** `rslbot ctl accepttp "Jane Doe"` is run and Jane Doe has sent no offer in this session
- **THEN** the command exits with status 1 saying there is no offer from Jane Doe, and nothing is sent

#### Scenario: Offer used once
- **WHEN** `accepttp "Jane Doe"` has been carried out and is run again without a new offer
- **THEN** the second command exits with status 1

#### Scenario: Someone asks to be brought to the bot
- **WHEN** an avatar sends the bot a request to be teleported to it
- **THEN** the operator log notes the request and who sent it, and the bot sends no offer unless the operator runs `sendtp`

### Requirement: Offer a teleport to an avatar
`sendtp <avatar> [<message>]` SHALL send that avatar an offer to teleport to the bot avatar's current position, with the message if one is given. The avatar SHALL be given and resolved as for the `im` command. The command SHALL be answered with success, naming the avatar, once the offer has been handed to the simulator connection; success does not mean the avatar came.

#### Scenario: Offer accepted
- **WHEN** `rslbot ctl sendtp "Jane Doe" come and see` is run and Jane Doe accepts the offer
- **THEN** Jane Doe arrives next to the bot's avatar

#### Scenario: Bot stays put
- **WHEN** `rslbot ctl sendtp "Jane Doe"` is run while the avatar is seated on the configured object
- **THEN** the avatar stays seated and the bot's placement mode does not change

### Requirement: Touch command
`touch <uuid>` SHALL make the avatar click the object with that UUID once, as a press followed at once by a release. It SHALL be answered with success once both have been handed to the simulator connection. It SHALL be answered with an error and send nothing when the argument is not a UUID, or when the bot has not learned of an object with that UUID in the current region. When the bot has not yet scanned the region (see "Object scan"), a `touch` on an object it does not know SHALL start the scan and wait up to 10 seconds for the object to be learned before answering.

#### Scenario: Touching a scripted object
- **WHEN** `rslbot ctl touch <uuid>` names a nearby object whose script reacts to touches
- **THEN** the script's touch events fire once with the bot's avatar as the toucher

#### Scenario: First touch without a configured scan
- **WHEN** `scan_distance` is not configured and `rslbot ctl touch <uuid>` is run for the first time, naming an object 10 metres away
- **THEN** the bot scans, the object is touched, and the command exits with status 0 within 11 seconds

#### Scenario: Object out of range or unknown
- **WHEN** `rslbot ctl touch <uuid>` names an object outside the scan distance or one that does not exist
- **THEN** the command exits with status 1 saying the object is not known and naming the scan distance

#### Scenario: Touching does not move the avatar
- **WHEN** `rslbot ctl touch <uuid>` is run while the avatar is seated on the configured object
- **THEN** the avatar stays seated and the bot's placement mode does not change

### Requirement: Object scan
To touch an object the bot must have learned of it from the simulator. Until a scan has started in a region, the bot SHALL NOT send the simulator anything for the sake of learning objects, and its traffic SHALL be what it is without the `touch` command. A scan SHALL start when the avatar enters a region if `scan_distance` is configured, and otherwise at the first `touch` command in that region that names an object the bot does not know. From the start of a scan until the avatar leaves the region the bot SHALL ask the simulator to describe every object it mentions without describing, and SHALL remember the objects described. The distance within which the simulator is asked to mention objects SHALL be `scan_distance` when configured and 32 metres otherwise. What the bot has learned in a region SHALL be forgotten when the avatar leaves it or the session is lost.

#### Scenario: No touch, no extra traffic
- **WHEN** `scan_distance` is not configured and no `touch` command is given
- **THEN** the bot sends the simulator nothing it did not send before this capability existed

#### Scenario: Scan from the start
- **WHEN** `scan_distance = 64` is configured and the avatar has been in the region for a minute
- **THEN** `rslbot ctl touch <uuid>` on an object 50 metres away is answered without a scan wait

#### Scenario: New region
- **WHEN** the avatar teleports to another region after a scan
- **THEN** the objects of the old region are forgotten, and without `scan_distance` nothing is asked of the new simulator until the next `touch`

### Requirement: Commands that suspend automatic placement
The bot SHALL be in one of two placement modes: holding, in which it keeps the avatar in the configured region on the configured object, and manual, in which it does not move or seat the avatar by itself. It SHALL start in holding mode. A `stand`, `unsit`, `sit`, `tp`, `requesttp` or `accepttp` command that is carried out SHALL put the bot in manual mode before it acts; a command that is refused SHALL NOT change the mode. `sendtp`, `touch`, `status`, `say`, `shout`, `whisper` and `im` SHALL NOT change the mode. Manual mode SHALL last until a `home` command or until the bot process ends, across session losses and logins.

#### Scenario: Staying where it was put
- **WHEN** `rslbot ctl tp 128 128 25 Home` has been carried out and an hour passes
- **THEN** the avatar is still in `Home` and the bot has not teleported or sat since

#### Scenario: Refused command
- **WHEN** `rslbot ctl sit chair` is refused while the bot is in holding mode
- **THEN** the bot stays in holding mode

### Requirement: Home command
`home` SHALL put the bot in holding mode and SHALL be answered with success at once, whether or not the avatar is connected. The bot then works towards the configured region and object as it does after a start. In holding mode already, `home` SHALL change nothing.

#### Scenario: Returning after a trip
- **WHEN** `rslbot ctl home` is run while the avatar stands in another region in manual mode
- **THEN** the bot teleports to the configured region and position and sits on the configured object

#### Scenario: Home while disconnected
- **WHEN** `rslbot ctl home` is run while the bot is waiting to log in again in manual mode
- **THEN** the command exits with status 0 and the next login is followed by the usual teleport and sit

### Requirement: Status command
`status` SHALL be answered with success and a text of one `name: value` line each for: `connected` (`yes` or `no`); `region` (the region's name); `position` (the last position the simulator reported for the avatar, as `x, y, z`); `seat` (`target` when seated on the configured object, `object <uuid>` when seated on an object by a `sit` command, `other` when seated on anything else, `none` when not seated on an object, `unknown` when not yet learned); and `mode` (`holding` or `manual`). `region`, `position` and `seat` SHALL be left out while not connected to a region. `status` SHALL work in every state of the bot.

#### Scenario: Seated on the target
- **WHEN** `rslbot ctl status` is run while the avatar is seated on the configured object in `Target` and no movement command was given
- **THEN** it prints `connected: yes`, `region: Target`, a `position` line, `seat: target` and `mode: holding`, and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl status` is run while the bot is waiting to log in again
- **THEN** it prints `connected: no` and the `mode` line, and exits with status 0

### Requirement: Quit command
`quit` SHALL be answered with success and SHALL then stop the bot the way a termination signal does: no new actions, a clean logout if connected, exit status 0.

#### Scenario: Quit while seated
- **WHEN** `rslbot ctl quit` is run while the avatar is seated
- **THEN** the command exits with status 0, the avatar goes offline on the grid, and the bot process exits with status 0 within 6 seconds

### Requirement: Movement commands appear in the operator log
The bot SHALL write a line to its operator log for every command of this capability it carries out, saying what was asked, for every such command it refuses, with the reason, for the outcome of a `sit <uuid>` or `tp`, for every change of placement mode, for every teleport offer it receives, saying who sent it and whether the bot accepted it, and for every request it receives from an avatar to be teleported to the bot. It SHALL log the start of an object scan with its distance.

#### Scenario: Reading the log after a trip
- **WHEN** the operator has run `tp`, `sit <uuid>` and `home`
- **THEN** the log shows, in order, the switch to manual mode, the teleport and its outcome, the sit and its outcome, the switch back to holding mode, and the bot's own teleport and sit that followed
