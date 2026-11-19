# Spec Delta

## MODIFIED Requirements

### Requirement: Sit on the target object
The bot SHALL be able to sit the avatar on the object whose UUID is configured as `sit`; that object is the target object. A sit attempt SHALL end in exactly one reported outcome: seated on the target, or failed with a reason. An attempt SHALL be reported as failed when the simulator does not confirm the sit within 15 seconds.

#### Scenario: Sitting on a nearby object
- **WHEN** the avatar is in the target region near the object and a sit is attempted
- **THEN** the avatar is seated on that object within 15 seconds, as seen by other viewers in the region

#### Scenario: Object does not exist
- **WHEN** no object with the configured UUID exists in the region
- **THEN** the attempt is reported as failed within 15 seconds and the avatar stays connected

## ADDED Requirements

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
