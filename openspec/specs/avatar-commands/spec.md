# avatar-commands Specification

## Purpose

Defines the control-socket commands that move the avatar and make it interact with the world on the operator's command, the commands that report on and stop the bot, and how those commands relate to the bot's automatic placement.

## Requirements

### Requirement: Commands need the avatar in a region
The commands `stand`, `unsit`, `sit`, `tp`, `requesttp`, `accept` (with its alias `accepttp`), `sendtp` and `touch` SHALL be answered with an error, and SHALL send nothing and change nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout. A command SHALL be carried out at once or not at all; it SHALL NOT be kept for later.

#### Scenario: Command between logins
- **WHEN** `rslbot ctl stand` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region, exits with status 1, and the bot's placement mode does not change

#### Scenario: Accept between logins
- **WHEN** `rslbot ctl accept tp` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

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

### Requirement: Offer a teleport to an avatar
`sendtp <avatar> [<message>]` SHALL send that avatar an offer to teleport to the bot avatar's current position, with the message if one is given. The avatar SHALL be given and resolved as for the `im` command. The command SHALL be answered with success, naming the avatar, once the offer has been handed to the simulator connection; success does not mean the avatar came.

#### Scenario: Offer accepted
- **WHEN** `rslbot ctl sendtp "Jane Doe" come and see` is run and Jane Doe accepts the offer
- **THEN** Jane Doe arrives next to the bot's avatar

#### Scenario: Bot stays put
- **WHEN** `rslbot ctl sendtp "Jane Doe"` is run while the avatar is seated on the configured object
- **THEN** the avatar stays seated and the bot's placement mode does not change

### Requirement: Touch command
`touch <uuid>` SHALL make the avatar click the object with that UUID once, as a press followed at once by a release. The UUID of an inventory item that is attached to the bot's avatar (see `avatar-outfit`, "Knowing what is attached") SHALL stand for the root prim of that attachment when the bot has learned of no object with that UUID in the current region; the touch SHALL then be held to every rule that holds a touch of that attachment. It SHALL be answered with success once both have been handed to the simulator connection. It SHALL be answered with an error and send nothing when the argument is not a UUID, or when the bot has not learned of an object with that UUID in the current region and the UUID is not that of an item attached to the avatar. When the bot has not yet scanned the region (see "Object scan"), a `touch` with a UUID that is neither that of an object it knows nor that of such an item SHALL start the scan and wait up to 10 seconds for the object to be learned before answering.

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

#### Scenario: Touching an attachment by its inventory item
- **WHEN** the avatar wears the object `Radio`, whose script reacts to touches, and `rslbot ctl touch <uuid>` is run with the UUID `rslbot ctl inventory -a` printed for `Radio`
- **THEN** the radio's touch events fire once with the bot's avatar as the toucher, and the command exits with status 0 at once

#### Scenario: Inventory item that is not attached
- **WHEN** `rslbot ctl touch <uuid>` is run with the UUID of an inventory item that is not attached to the avatar
- **THEN** the command exits with status 1 saying the object is not known

### Requirement: Object scan
To touch an object the bot must have learned of it from the simulator. Until a scan has started in a region, the bot SHALL NOT send the simulator anything for the sake of learning objects, with one exception: the requests about the object the avatar has become seated on, which are the one `region-teleport` ("Know the current position") calls for and the one for its name (see "Status command"). Apart from those requests its traffic SHALL be what it is without the `touch` and `scan` commands and without RLV restrictions. A scan SHALL start when the avatar enters a region if `scan_distance` is configured, and otherwise at the first of these in that region: a `touch` command that names an object the bot does not know, a `scan` command (see `area-scan`), or an RLV restriction that has to be applied to an object the bot does not know (see `rlv-restrictions`, "Objects the bot does not know"). A restriction that is kept and has nothing to be applied to SHALL NOT start a scan. From the start of a scan until the avatar leaves the region the bot SHALL ask the simulator to describe every object it mentions without describing, and SHALL remember the objects described. The distance within which the simulator is asked to mention objects SHALL be `scan_distance` when configured and 32 metres otherwise, or the largest distance a `scan` command has named in that region when that is more (see `area-scan`, "Scan distance"). What the bot has learned in a region, and a distance a `scan` command raised there, SHALL be forgotten when the avatar leaves it or the session is lost.

#### Scenario: No touch, no extra traffic
- **WHEN** `scan_distance` is not configured, no `touch` or `scan` command is given, and the avatar sits on one object
- **THEN** the only things the bot sends the simulator for the sake of learning objects are the two requests about that object

#### Scenario: Scan from the start
- **WHEN** `scan_distance = 64` is configured and the avatar has been in the region for a minute
- **THEN** `rslbot ctl touch <uuid>` on an object 50 metres away is answered without a scan wait

#### Scenario: New region
- **WHEN** the avatar teleports to another region after a scan
- **THEN** the objects of the old region are forgotten, and without `scan_distance` nothing is asked of the new simulator until the next `touch`, `scan` or sit

#### Scenario: Restriction that needs a distance
- **WHEN** `scan_distance` is not configured, RLV is on, an object the avatar owns holds `sittp`, and `rslbot ctl sit <uuid>` names an object the bot does not know
- **THEN** the bot scans, as it does for a `touch` of such an object

#### Scenario: Restrictions without a use for a scan
- **WHEN** `scan_distance` is not configured, an object the avatar owns holds `sittp` and `fartouch`, and no command or step of the bot's names an object
- **THEN** the bot sends the simulator nothing for the sake of learning objects

#### Scenario: Scan command starts the scan
- **WHEN** `scan_distance` is not configured and `rslbot ctl scan` is the first command in the region to need objects
- **THEN** the bot scans within 32 metres, and a later `rslbot ctl touch <uuid>` on an object 10 metres away is answered without a scan wait

#### Scenario: Scan command raises the distance
- **WHEN** `scan_distance = 64` is configured and `rslbot ctl scan 128` is run
- **THEN** the simulator is asked to mention objects within 128 metres until the avatar leaves the region, and within 64 metres again in the next region

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

### Requirement: Home command
`home` SHALL be carried out and answered exactly as `persist last last` is (see `placement-persist`): it puts the bot in holding mode with the placement of the last persist that was carried out, or with the config placement when there was none. As neither value needs the simulator, `home` SHALL be carried out whether or not the avatar is connected. `home` SHALL take no arguments.

#### Scenario: Returning after a trip
- **WHEN** `rslbot ctl home` is run while the avatar stands in another region in manual mode
- **THEN** the bot teleports to the held location and sits as the held sit state says

#### Scenario: Home while disconnected
- **WHEN** `rslbot ctl home` is run while the bot is waiting to log in again in manual mode
- **THEN** the command exits with status 0, the next login asks for the held location, and the usual sit follows

#### Scenario: Home after a persist
- **WHEN** the config names `region = "Target"`, the operator has persisted a place in `Blue Lagoon`, moved on with `tp 128 128 25 Home`, and runs `rslbot ctl home`
- **THEN** the bot teleports to the persisted place in `Blue Lagoon`, not to `Target`

#### Scenario: Home without an earlier persist
- **WHEN** the config has `persist = "off"`, `region`, `position` and `sit`, and `rslbot ctl home` is run after a `tp` command took the avatar to another region
- **THEN** the bot teleports to the configured region and position and sits as configured

#### Scenario: Home without a configured region
- **WHEN** the config has `start = "home"`, `sit = "ground"` and no `region`, the first login placed the avatar in `Target`, and `rslbot ctl home` is run after a `tp` command took the avatar to another region
- **THEN** the bot teleports back to the place it holds in `Target` and sits the avatar on the ground there

#### Scenario: Home prints the placement
- **WHEN** `rslbot ctl home` is carried out
- **THEN** it prints the placement the bot now holds, as `persist` does

### Requirement: Status command
`status` SHALL be answered with success and a text of one `name: value` line each for: `connected` (`yes` or `no`); `region` (the region's name); `position` (the avatar's current position as the bot knows it, as `x, y, z`); `seat` (`target` when seated on the object that is the held sit state, `object <uuid>` when seated on another object, naming the prim the simulator seated the avatar on, `other` when attached to something the simulator did not name, `none` when not seated on an object, `unknown` when not yet learned); `seat name` (the name of the object the avatar is seated on); `persist` (the held placement as `<location> <sit state>` in the forms the `persist` command accepts, with `unknown` in place of a location that is not known yet); and `mode` (`holding` or `manual`). A ground sit, held or commanded, is not a seat on an object and SHALL show as `none`. `region`, `position`, `seat` and `seat name` SHALL be left out while not connected to a region. `status` SHALL work in every state of the bot. In manual mode the `persist` line shows the placement a `home` command would return to.

The `seat name` line SHALL be printed while `seat` is `target`, `object <uuid>` or `other` and the bot knows the name, and SHALL be left out otherwise. The name SHALL be the one the simulator gives for the root prim of the linkset the avatar is seated on, with a tab, line feed, carriage return or backslash in it written as `\t`, `\n`, `\r` and `\\`. Each time the avatar becomes seated on an object the bot SHALL ask the simulator for that name once, unless it has it from a `scan`, and SHALL forget it when the avatar stands up, leaves the region or the session is lost. `status` SHALL NOT wait for the name.

#### Scenario: Seated on the target
- **WHEN** `rslbot ctl status` is run in holding mode while the avatar is seated on the held object in `Target`
- **THEN** it prints `connected: yes`, `region: Target`, a `position` line, `seat: target`, a `persist` line naming a location in `Target` and that object, and `mode: holding`, and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl status` is run while the bot is waiting to log in again
- **THEN** it prints `connected: no`, the `persist` line and the `mode` line, and exits with status 0

#### Scenario: Configured ground sit
- **WHEN** `rslbot ctl status` is run in holding mode after the bot has sat the avatar on the ground as the held sit state `ground` asks
- **THEN** it prints `seat: none`, a `persist` line ending in `ground`, and `mode: holding`

#### Scenario: Seated elsewhere in manual mode
- **WHEN** `rslbot ctl status` is run after `rslbot ctl sit <uuid>` seated the avatar on an object that is not the held sit state
- **THEN** it prints `seat: object` with the UUID of the prim the simulator seated the avatar on, the unchanged `persist` line, and `mode: manual`

#### Scenario: Location not known yet
- **WHEN** the config names no `position` and `rslbot ctl status` is run before the first login has succeeded
- **THEN** the `persist` line shows `unknown` as the location

#### Scenario: Name of the seat
- **WHEN** the avatar has been seated on the held object, which is named `Park Bench`, for a few seconds and `rslbot ctl status` is run
- **THEN** it prints `seat: target` and, on the next line, `seat name: Park Bench`

#### Scenario: Seated on a child prim
- **WHEN** the simulator seated the avatar on the child prim `Cushion` of the linkset whose root prim is named `Sofa`
- **THEN** `status` prints `seat: object` with the cushion's UUID and `seat name: Sofa`

#### Scenario: Name not told yet
- **WHEN** `rslbot ctl status` is run at the moment the avatar becomes seated, before the simulator has answered for the name
- **THEN** it is answered at once with the `seat` line and without a `seat name` line

#### Scenario: Not seated on an object
- **WHEN** `rslbot ctl status` is run while the avatar stands or sits on the ground
- **THEN** it prints `seat: none` and no `seat name` line

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
