# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
