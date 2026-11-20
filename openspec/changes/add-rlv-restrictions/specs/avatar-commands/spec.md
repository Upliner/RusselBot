# Spec Delta

## MODIFIED Requirements

### Requirement: Object scan
To touch an object the bot must have learned of it from the simulator. Until a scan has started in a region, the bot SHALL NOT send the simulator anything for the sake of learning objects, with one exception: the request about the object the avatar has become seated on, which `region-teleport` ("Know the current position") calls for. Apart from that request its traffic SHALL be what it is without the `touch` command and without RLV restrictions. A scan SHALL start when the avatar enters a region if `scan_distance` is configured, and otherwise at the first of these in that region: a `touch` command that names an object the bot does not know, or an RLV restriction that has to be applied to an object the bot does not know (see `rlv-restrictions`, "Objects the bot does not know"). A restriction that is kept and has nothing to be applied to SHALL NOT start a scan. From the start of a scan until the avatar leaves the region the bot SHALL ask the simulator to describe every object it mentions without describing, and SHALL remember the objects described. The distance within which the simulator is asked to mention objects SHALL be `scan_distance` when configured and 32 metres otherwise. What the bot has learned in a region SHALL be forgotten when the avatar leaves it or the session is lost.

#### Scenario: No touch, no extra traffic
- **WHEN** `scan_distance` is not configured, no `touch` command is given, and the avatar sits on one object
- **THEN** the only thing the bot sends the simulator for the sake of learning objects is one request about that object

#### Scenario: Scan from the start
- **WHEN** `scan_distance = 64` is configured and the avatar has been in the region for a minute
- **THEN** `rslbot ctl touch <uuid>` on an object 50 metres away is answered without a scan wait

#### Scenario: New region
- **WHEN** the avatar teleports to another region after a scan
- **THEN** the objects of the old region are forgotten, and without `scan_distance` nothing is asked of the new simulator until the next `touch` or sit

#### Scenario: Restriction that needs a distance
- **WHEN** `scan_distance` is not configured, RLV is on, an object the avatar owns holds `sittp`, and `rslbot ctl sit <uuid>` names an object the bot does not know
- **THEN** the bot scans, as it does for a `touch` of such an object

#### Scenario: Restrictions without a use for a scan
- **WHEN** `scan_distance` is not configured, an object the avatar owns holds `sittp` and `fartouch`, and no command or step of the bot's names an object
- **THEN** the bot sends the simulator nothing for the sake of learning objects
