# object-sit Specification

## Purpose

Covers sitting the avatar on the configured object and knowing, at any time, whether it is still seated there.

## Requirements

### Requirement: Sit on the target object
The bot SHALL be able to sit the avatar on the object whose UUID is configured as `sit`; that object is the target object. A sit attempt SHALL end in exactly one reported outcome: seated on the target, or failed with a reason. An attempt SHALL be reported as failed when the simulator does not confirm the sit within 15 seconds.

#### Scenario: Sitting on a nearby object
- **WHEN** the avatar is in the target region near the object and a sit is attempted
- **THEN** the avatar is seated on that object within 15 seconds, as seen by other viewers in the region

#### Scenario: Object does not exist
- **WHEN** no object with the configured UUID exists in the region
- **THEN** the attempt is reported as failed within 15 seconds and the avatar stays connected

### Requirement: Track the seated state
While connected, the bot SHALL track whether the avatar is seated on the target object. The state SHALL be one of: unknown (not yet learned since entering the region), not seated, or seated on the target. The avatar SHALL be considered seated on the target only when the simulator has confirmed a sit on the target object and has not since reported the avatar as standing. The simulator's answer to the bot's own sit request SHALL count as that confirmation even when it names another prim of the target's linkset; a sit the bot did not request SHALL count only when it names the target object itself. The state SHALL return to unknown whenever the avatar enters a region or the session is lost.

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
While the avatar is seated on the target, the bot SHALL NOT of its own accord send anything that makes it stand up, move, or sit again. It does so only on an operator's command.

#### Scenario: Long seated session
- **WHEN** the avatar has been seated on the target for 30 minutes with no outside interference
- **THEN** it has stayed seated the whole time and no further sit requests were sent

#### Scenario: Operator stands the avatar up
- **WHEN** the operator sends a `stand` command while the avatar is seated on the target
- **THEN** the avatar stands up

### Requirement: Sit on the ground when configured
When `sit` is configured as `ground`, the bot SHALL sit the avatar on the ground where it stands, by the same request the `sit ground` command makes. The simulator does not confirm a ground sit, so the bot SHALL treat the request as carried out once it has been handed to the simulator connection, SHALL NOT report a failure for it, and SHALL NOT repeat it until the presence reconciler calls for a ground sit again. There is no target object with this setting, and the avatar is never considered seated on the target.

#### Scenario: Ground sit after login
- **WHEN** `sit = "ground"` is configured and the avatar has logged in to the region it is to stay in
- **THEN** other viewers see the avatar sitting on the ground, and the bot sends no further sit request while nothing changes

#### Scenario: Ground sit after a teleport
- **WHEN** `sit = "ground"` is configured and the avatar arrives in the configured region by a teleport
- **THEN** the bot sits it on the ground again

### Requirement: No sitting without a configured seat
When neither `sit` nor `object` is configured, the bot SHALL NOT send a sit request of its own accord. Sit requests are then made only on an operator's command.

#### Scenario: No seat configured
- **WHEN** the config has no `sit` and the avatar has been connected for 30 minutes without any command
- **THEN** the bot has sent no sit request and the avatar stands where it arrived

#### Scenario: Operator sits the avatar
- **WHEN** the config has no `sit` and the operator sends a `sit <uuid>` command
- **THEN** the avatar sits on that object as the command specifies
