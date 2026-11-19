# Spec Delta

## MODIFIED Requirements

### Requirement: Request a teleport from an avatar
`requesttp <avatar> [<message>]` SHALL send that avatar a request to teleport the bot's avatar to them, with the message if one is given. The avatar SHALL be given and resolved as for the `im` command. The command SHALL be answered with success, naming the avatar, once the request has been handed to the simulator connection; success does not mean the avatar agreed. When that avatar sends the bot a teleport offer within the configured `requesttp_timeout` (10 minutes unless configured) of the request, the bot SHALL accept it and follow the teleport. It SHALL accept at most one offer per request. A `persist` or `home` command that is carried out SHALL end the wait: an offer that arrives after it is not accepted by itself. The bot SHALL NOT by itself accept a teleport offer from an avatar it has not asked within that time; such an offer is kept for the `accepttp` command.

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

#### Scenario: Persisted while waiting
- **WHEN** `rslbot ctl persist` is run after a `requesttp`, and the asked avatar's offer arrives a minute later
- **THEN** the avatar stays where it was persisted and the offer is kept for `accepttp`

#### Scenario: Unknown avatar
- **WHEN** `rslbot ctl requesttp "Nobody Here"` is run and the grid has no such avatar
- **THEN** the command exits with status 1 naming `Nobody Here` and no request is sent

### Requirement: Object scan
To touch an object the bot must have learned of it from the simulator. Until a scan has started in a region, the bot SHALL NOT send the simulator anything for the sake of learning objects, with one exception: the request about the object the avatar has become seated on, which `region-teleport` ("Know the current position") calls for. Apart from that request its traffic SHALL be what it is without the `touch` command. A scan SHALL start when the avatar enters a region if `scan_distance` is configured, and otherwise at the first `touch` command in that region that names an object the bot does not know. From the start of a scan until the avatar leaves the region the bot SHALL ask the simulator to describe every object it mentions without describing, and SHALL remember the objects described. The distance within which the simulator is asked to mention objects SHALL be `scan_distance` when configured and 32 metres otherwise. What the bot has learned in a region SHALL be forgotten when the avatar leaves it or the session is lost.

#### Scenario: No touch, no extra traffic
- **WHEN** `scan_distance` is not configured, no `touch` command is given, and the avatar sits on one object
- **THEN** the only thing the bot sends the simulator for the sake of learning objects is one request about that object

#### Scenario: Scan from the start
- **WHEN** `scan_distance = 64` is configured and the avatar has been in the region for a minute
- **THEN** `rslbot ctl touch <uuid>` on an object 50 metres away is answered without a scan wait

#### Scenario: New region
- **WHEN** the avatar teleports to another region after a scan
- **THEN** the objects of the old region are forgotten, and without `scan_distance` nothing is asked of the new simulator until the next `touch` or sit

### Requirement: Commands that suspend automatic placement
The bot SHALL be in one of two placement modes: holding, in which it works towards the held placement (see `placement-persist`), and manual, in which it does not move or seat the avatar by itself. It SHALL start in holding mode. A `stand`, `unsit`, `sit`, `tp`, `requesttp` or `accepttp` command that is carried out SHALL put the bot in manual mode before it acts; a command that is refused SHALL NOT change the mode. `sendtp`, `touch`, `status`, `say`, `shout`, `whisper` and `im` SHALL NOT change the mode. Manual mode SHALL last until a `persist` or `home` command is carried out or until the bot process ends, across session losses and logins. Besides those commands, the bot enters manual mode by itself only once, after startup, when the config says `persist = "off"`. The held placement SHALL NOT be changed by entering or being in manual mode.

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
- **WHEN** the config has `start = "home"`, `sit = "ground"` and no `region`, the bot has settled in `Target` and holds its place there, and `rslbot ctl home` is run after a `tp` command took the avatar to another region
- **THEN** the bot teleports back to the place it holds in `Target` and sits the avatar on the ground there

#### Scenario: Home prints the placement
- **WHEN** `rslbot ctl home` is carried out
- **THEN** it prints the placement the bot now holds, as `persist` does

### Requirement: Status command
`status` SHALL be answered with success and a text of one `name: value` line each for: `connected` (`yes` or `no`); `region` (the region's name); `position` (the avatar's current position as the bot knows it, as `x, y, z`); `seat` (`target` when seated on the object that is the held sit state, `object <uuid>` when seated on another object, naming the prim the simulator seated the avatar on, `other` when attached to something the simulator did not name, `none` when not seated on an object, `unknown` when not yet learned); `persist` (the held placement as `<location> <sit state>` in the forms the `persist` command accepts, with `unknown` in place of a location that is not known yet); and `mode` (`holding` or `manual`). A ground sit, held or commanded, is not a seat on an object and SHALL show as `none`. `region`, `position` and `seat` SHALL be left out while not connected to a region. `status` SHALL work in every state of the bot. In manual mode the `persist` line shows the placement a `home` command would return to.

#### Scenario: Seated on the target
- **WHEN** `rslbot ctl status` is run while the avatar is seated on the held object in `Target` and no movement command was given
- **THEN** it prints `connected: yes`, `region: Target`, a `position` line, `seat: target`, a `persist` line naming a location in `Target` and that object, and `mode: holding`, and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl status` is run while the bot is waiting to log in again
- **THEN** it prints `connected: no`, the `persist` line and the `mode` line, and exits with status 0

#### Scenario: Configured ground sit
- **WHEN** `rslbot ctl status` is run after the bot has sat the avatar on the ground as the held sit state `ground` asks
- **THEN** it prints `seat: none`, a `persist` line ending in `ground`, and `mode: holding`

#### Scenario: Seated elsewhere in manual mode
- **WHEN** `rslbot ctl status` is run after `rslbot ctl sit <uuid>` seated the avatar on an object that is not the held sit state
- **THEN** it prints `seat: object` with the UUID of the prim the simulator seated the avatar on, the unchanged `persist` line, and `mode: manual`

#### Scenario: Location not known yet
- **WHEN** the config names no `position` and `rslbot ctl status` is run before the first login has succeeded
- **THEN** the `persist` line shows `unknown` as the location
