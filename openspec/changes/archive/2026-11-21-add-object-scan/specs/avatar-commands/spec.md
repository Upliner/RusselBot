# Spec Delta

## MODIFIED Requirements

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
