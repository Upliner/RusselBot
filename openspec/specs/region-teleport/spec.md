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
The bot SHALL be able to teleport the avatar to the held location: its coordinates in its region. It SHALL look the region up by name on the grid, request the teleport, and follow the session to the destination simulator so that the avatar ends up connected there and the previous simulator is no longer the one the bot acts on. A teleport attempt SHALL end in exactly one reported outcome: arrived, or failed with a reason.

#### Scenario: Teleport between two regions
- **WHEN** the avatar is in region "Home" and a teleport to the held location in region "Target" is attempted while both are online
- **THEN** the avatar is in "Target" near the held location's coordinates within 60 seconds and the session continues without a new login

#### Scenario: Back into the region next door
- **WHEN** the bot runs with `scan_distance = 64` on OpenSimulator, the avatar is teleported from the middle of "Target" to the middle of the neighbouring "Home", and 20 seconds later a teleport to the held location in "Target" is attempted
- **THEN** the avatar is in "Target" within 60 seconds, the session continues without a new login, and the bot sits on its seat again

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

### Requirement: Simulators the avatar has left
After a teleport to another simulator the bot SHALL keep its connection to the simulator the avatar left for as long as that simulator keeps it: it SHALL acknowledge that simulator's reliable packets and answer its pings, and SHALL NOT act on anything else it sends. The bot SHALL give that connection up when that simulator closes it, sends nothing for 60 seconds, or leaves one of the bot's packets unacknowledged through every resend. None of these is a session loss.

#### Scenario: The simulator keeps the avatar's presence
- **WHEN** the avatar has teleported from "Target" to the neighbouring "Home" with `scan_distance = 64` on OpenSimulator and stays there for five minutes
- **THEN** the simulator of "Target" has not timed the bot's connection out, and the bot has reported nothing from "Target" as chat, objects or a seat

#### Scenario: The simulator closes the connection
- **WHEN** the simulator the avatar left announces that it is closing its connection
- **THEN** the bot sends it nothing more, and the session continues in the region the avatar is in

#### Scenario: The simulator falls silent
- **WHEN** the simulator the avatar left sends nothing for 60 seconds
- **THEN** the bot sends it nothing more, and the session is not reported as lost

#### Scenario: Kicked from where the avatar no longer is
- **WHEN** the simulator the avatar left sends a kick
- **THEN** the session is not reported as lost

### Requirement: Teleport back to a simulator the avatar has left
When a teleport leads to a simulator the bot still has a connection to, the bot SHALL continue that connection and SHALL NOT open another: its packets carry on the numbering that connection had. It SHALL announce itself and complete the handshake as on any arrival, and SHALL start over with what it knows of the region: its objects, the seat and the avatar's position. When the bot has given the connection up, the teleport SHALL open a new one.

#### Scenario: Numbering continues
- **WHEN** the bot's last packet to "Target" before leaving it had number 40, and a teleport leads back to "Target" while the bot still has that connection
- **THEN** the first packet the bot sends "Target" on arrival has a number above 40

#### Scenario: Three round trips in a row
- **WHEN** the round trip of "Back into the region next door" is made three times in a row, with `scan_distance = 64` and again with `scan_distance = 128`
- **THEN** every teleport ends with the avatar arrived and the session is never lost

#### Scenario: The region is learned again
- **WHEN** the avatar is back in "Target" after such a round trip
- **THEN** `rslbot ctl scan` lists the objects around the seat, and `rslbot ctl status` shows the avatar seated on its target

#### Scenario: The simulator had closed the connection
- **WHEN** the simulator of "Target" closed the bot's connection while the avatar was away, and a teleport then leads back to "Target"
- **THEN** the bot opens a new connection, numbered from the start

### Requirement: One local port for a session
All the simulator connections of one login session SHALL send from, and receive on, the same local UDP port, so that a simulator the avatar returns to sees the address it knows. A packet from an address the bot has no connection to SHALL be ignored.

#### Scenario: Second simulator
- **WHEN** a teleport hands the session over from one simulator to another
- **THEN** the second simulator receives the bot's packets from the same address and port as the first did

#### Scenario: Unknown sender
- **WHEN** a packet arrives from an address that is not one of the session's simulators
- **THEN** the bot does not answer it and nothing it believes changes

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
