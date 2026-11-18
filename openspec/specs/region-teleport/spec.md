# region-teleport Specification

## Purpose

Covers knowing which region the avatar is in and moving it to the configured region and coordinates by teleport, including following the session over to the destination simulator.

## Requirements

### Requirement: Know the current region
While connected, the bot SHALL know the name of the region the avatar is currently in, as reported by the simulator. The avatar SHALL be considered to be in the target region when that name equals the configured `region`, compared without regard to letter case or surrounding whitespace.

#### Scenario: Already in the target region
- **WHEN** the bot is connected to a region whose name differs from the configured `region` only in letter case
- **THEN** the avatar is considered to be in the target region and no teleport is requested

#### Scenario: In another region
- **WHEN** the bot is connected to a region with a different name
- **THEN** the avatar is considered not to be in the target region

### Requirement: Teleport to the target region
The bot SHALL be able to teleport the avatar to the configured `position` in the configured `region`. It SHALL look the region up by name on the grid, request the teleport, and follow the session to the destination simulator so that the avatar ends up connected there and the previous simulator connection is dropped. A teleport attempt SHALL end in exactly one reported outcome: arrived, or failed with a reason.

#### Scenario: Teleport between two regions
- **WHEN** the avatar is in region "Home" and a teleport to region "Target" is attempted while both are online
- **THEN** the avatar is in "Target" near the configured position within 60 seconds and the session continues without a new login

#### Scenario: Region name not found
- **WHEN** the grid has no region with the configured name
- **THEN** the attempt is reported as failed with a reason saying the region was not found, and the avatar stays connected where it was

#### Scenario: Destination refuses or is offline
- **WHEN** the grid reports that the teleport failed
- **THEN** the attempt is reported as failed with the grid's reason, and the avatar stays connected where it was

#### Scenario: No answer
- **WHEN** neither arrival nor failure is reported within 60 seconds of the request
- **THEN** the attempt is reported as failed with a timeout reason

### Requirement: Teleport within the current region
The bot SHALL be able to teleport the avatar to the configured `position` when it is already in the target region, without changing simulator.

#### Scenario: Repositioning inside the target region
- **WHEN** the avatar is in the target region and a teleport to the configured position is attempted
- **THEN** the avatar is at the configured position and still connected to the same simulator

### Requirement: Stale simulator traffic is ignored after a teleport
After the avatar has arrived in a new region, anything still arriving from the previous simulator SHALL NOT change what the bot believes about its current region, seat or connection.

#### Scenario: Old simulator closes its connection late
- **WHEN** the previous simulator announces it is closing its connection after the avatar has arrived in the new region
- **THEN** the session is not reported as lost
