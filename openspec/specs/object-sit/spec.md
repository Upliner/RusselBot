# object-sit Specification

## Purpose

Covers sitting the avatar on the configured object and knowing, at any time, whether it is still seated there.

## Requirements

### Requirement: Sit on the target object
The bot SHALL be able to sit the avatar on the object whose UUID is configured as `object`. A sit attempt SHALL end in exactly one reported outcome: seated on the target, or failed with a reason. An attempt SHALL be reported as failed when the simulator does not confirm the sit within 15 seconds.

#### Scenario: Sitting on a nearby object
- **WHEN** the avatar is in the target region near the object and a sit is attempted
- **THEN** the avatar is seated on that object within 15 seconds, as seen by other viewers in the region

#### Scenario: Object does not exist
- **WHEN** no object with the configured UUID exists in the region
- **THEN** the attempt is reported as failed within 15 seconds and the avatar stays connected

### Requirement: Track the seated state
While connected, the bot SHALL track whether the avatar is seated on the target object. The state SHALL be one of: unknown (not yet learned since entering the region), not seated, or seated on the target. The avatar SHALL be considered seated on the target only when the simulator has confirmed a sit on the target object and has not since reported the avatar as standing. The state SHALL return to unknown whenever the avatar enters a region or the session is lost.

#### Scenario: Unseated by a script
- **WHEN** the avatar is seated on the target and a script in the object unseats it
- **THEN** the bot's state changes to not seated within 5 seconds

#### Scenario: Seat object deleted
- **WHEN** the avatar is seated on the target and the object is deleted
- **THEN** the bot's state changes to not seated within 5 seconds

#### Scenario: Other avatars sitting and standing
- **WHEN** other avatars in the region sit down or stand up
- **THEN** the bot's own seated state does not change

### Requirement: Sitting is not disturbed while seated
While the avatar is seated on the target, the bot SHALL NOT send anything that makes it stand up, move, or sit again.

#### Scenario: Long seated session
- **WHEN** the avatar has been seated on the target for 30 minutes with no outside interference
- **THEN** it has stayed seated the whole time and no further sit requests were sent
