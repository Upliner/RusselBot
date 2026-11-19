# Spec Delta

## MODIFIED Requirements

### Requirement: Object scan
To touch an object the bot must have learned of it from the simulator. Until a scan has started in a region, the bot SHALL NOT send the simulator anything for the sake of learning objects, with two exceptions: the request about the object the avatar has become seated on, which `region-teleport` ("Know the current position") calls for, and the requests about objects of the bot's own account that the simulator mentions without describing, which `avatar-outfit` ("Knowing what is attached") calls for. Apart from those requests its traffic SHALL be what it is without the `touch` command. A scan SHALL start when the avatar enters a region if `scan_distance` is configured, and otherwise at the first `touch` command in that region that names an object the bot does not know. From the start of a scan until the avatar leaves the region the bot SHALL ask the simulator to describe every object it mentions without describing, whoever owns it, and SHALL remember the objects described; an object it has asked about before the scan SHALL NOT be asked about again. The distance within which the simulator is asked to mention objects SHALL be `scan_distance` when configured and 32 metres otherwise. What the bot has learned in a region SHALL be forgotten when the avatar leaves it or the session is lost.

#### Scenario: No touch, no extra traffic
- **WHEN** `scan_distance` is not configured, no `touch` command is given, the avatar sits on one object, and the simulator mentions 200 objects without describing them, 3 of them marked as owned by the bot's account
- **THEN** the only things the bot sends the simulator for the sake of learning objects are one request about the seat and requests about those 3 objects

#### Scenario: Scan from the start
- **WHEN** `scan_distance = 64` is configured and the avatar has been in the region for a minute
- **THEN** `rslbot ctl touch <uuid>` on an object 50 metres away is answered without a scan wait

#### Scenario: Scan after own objects were asked about
- **WHEN** the bot has asked about 3 objects of its own account, the simulator has mentioned 197 others without describing them, and the first `touch` then starts a scan
- **THEN** the bot asks about the 197 and not again about the 3

#### Scenario: New region
- **WHEN** the avatar teleports to another region after a scan
- **THEN** the objects of the old region are forgotten, and without `scan_distance` nothing is asked of the new simulator until the next `touch` or sit, except about objects of the bot's own account that it mentions without describing
