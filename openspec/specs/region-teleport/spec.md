# region-teleport Specification

## Purpose

Covers knowing which region the avatar is in and where in it, and moving it to the location the bot holds by teleport, including following the session over to the destination simulator.

## Requirements

### Requirement: Know the current region
While connected, the bot SHALL know the name of the region the avatar is currently in, as reported by the simulator. The target region is the region of the held location (see `placement-persist`). The avatar SHALL be considered to be in the target region when the current region's name equals the held location's region, compared without regard to letter case or surrounding whitespace. While no location is held, which is the case only until the first login of a config without `position` has reported one, there is no target region and the bot SHALL NOT request a teleport of its own accord.

#### Scenario: Already in the target region
- **WHEN** the bot is connected to a region whose name differs from the held location's region only in letter case
- **THEN** the avatar is considered to be in the target region and no teleport is requested

#### Scenario: In another region
- **WHEN** the bot is connected to a region with a different name
- **THEN** the avatar is considered not to be in the target region

#### Scenario: No region configured
- **WHEN** the config has no `region` and the first login places the avatar in `Home`
- **THEN** no teleport is requested, and `Home` is the target region from then on until a persist names another

#### Scenario: Target region changed by a persist
- **WHEN** `rslbot ctl persist "Blue Lagoon/128/128/25" stand` is carried out while the avatar is in `Target`
- **THEN** `Blue Lagoon` is the target region and the avatar is considered not to be in it

### Requirement: Teleport to the target region
The bot SHALL be able to teleport the avatar to the held location: its coordinates in its region. It SHALL look the region up by name on the grid, request the teleport, and follow the session to the destination simulator so that the avatar ends up connected there and the previous simulator connection is dropped. A teleport attempt SHALL end in exactly one reported outcome: arrived, or failed with a reason.

#### Scenario: Teleport between two regions
- **WHEN** the avatar is in region "Home" and a teleport to the held location in region "Target" is attempted while both are online
- **THEN** the avatar is in "Target" near the held location's coordinates within 60 seconds and the session continues without a new login

#### Scenario: Region name not found
- **WHEN** the grid has no region with the held location's region name
- **THEN** the attempt is reported as failed with a reason saying the region was not found, and the avatar stays connected where it was

#### Scenario: Destination refuses or is offline
- **WHEN** the grid reports that the teleport failed
- **THEN** the attempt is reported as failed with the grid's reason, and the avatar stays connected where it was

#### Scenario: No answer
- **WHEN** neither arrival nor failure is reported within 60 seconds of the request
- **THEN** the attempt is reported as failed with a timeout reason

### Requirement: Teleport within the current region
The bot SHALL be able to teleport the avatar to the held location's coordinates when it is already in the target region, without changing simulator.

#### Scenario: Repositioning inside the target region
- **WHEN** the avatar is in the target region and a teleport to the held location is attempted
- **THEN** the avatar is at the held location's coordinates and still connected to the same simulator

### Requirement: Stale simulator traffic is ignored after a teleport
After the avatar has arrived in a new region, anything still arriving from the previous simulator SHALL NOT change what the bot believes about its current region, seat or connection.

#### Scenario: Old simulator closes its connection late
- **WHEN** the previous simulator announces it is closing its connection after the avatar has arrived in the new region
- **THEN** the session is not reported as lost

### Requirement: Know the current position
While connected to a region, the bot SHALL know the avatar's current position in region coordinates. It SHALL take it from what the simulator reports: the position given on arrival in the region, the position given at the end of a teleport inside the region, and the position in every later update the simulator sends about the avatar itself. While the avatar is seated on an object the simulator reports its position relative to that object; the bot SHALL then work out the region position from the object's position and rotation and the avatar's offset on it. Each time the avatar becomes seated on an object, the bot SHALL ask the simulator to describe that one object, so that its position and rotation are known, and SHALL follow the object's later position updates for as long as the avatar sits on it. Until the object's position is known, the current position SHALL remain the last one known.

The current position is what `status` prints, what `tp` without `z` takes its height from, and what a `cur` location stands for. It SHALL be replaced on arrival in a region and forgotten when the session is lost.

#### Scenario: Seated away from where the avatar stood
- **WHEN** the avatar stands at 100, 100, 25 and is seated on a chair whose seat is at 131, 64, 23
- **THEN** within 5 seconds `rslbot ctl status` prints a position within a metre of 131, 64, 23

#### Scenario: Rotated seat
- **WHEN** the avatar's offset on its seat is one metre along the seat's own x axis and the seat is turned a quarter turn about the vertical axis
- **THEN** the current position is one metre from the seat's position along the region's y axis

#### Scenario: Standing up
- **WHEN** the avatar stands up from that chair
- **THEN** the current position is where the simulator put the standing avatar, next to the chair, not 100, 100, 25

#### Scenario: Seat that moves
- **WHEN** the object the avatar sits on moves ten metres
- **THEN** the current position follows it

#### Scenario: Pushed while standing
- **WHEN** a script pushes the standing avatar twenty metres
- **THEN** the current position is where the simulator reports the avatar afterwards

#### Scenario: Seat not described
- **WHEN** the simulator does not answer the request about the seat
- **THEN** the current position stays the last one known and the avatar stays connected and seated
