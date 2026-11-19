# Spec Delta

## MODIFIED Requirements

### Requirement: Sit on the target object
The bot SHALL be able to sit the avatar on the object whose UUID is the held sit state (see `placement-persist`); that object is the target object. At startup it is the object configured as `sit`. A sit attempt SHALL end in exactly one reported outcome: seated on the target, or failed with a reason. An attempt SHALL be reported as failed when the simulator does not confirm the sit within 15 seconds.

#### Scenario: Sitting on a nearby object
- **WHEN** the avatar is in the target region near the object and a sit is attempted
- **THEN** the avatar is seated on that object within 15 seconds, as seen by other viewers in the region

#### Scenario: Object does not exist
- **WHEN** no object with the target's UUID exists in the region
- **THEN** the attempt is reported as failed within 15 seconds and the avatar stays connected

#### Scenario: Target changed by a persist
- **WHEN** a `persist` naming another object as sit state is carried out
- **THEN** that object is the target object from then on, and the next sit attempt is made on it

### Requirement: Track the seated state
While connected, the bot SHALL track whether the avatar is seated on the target object, and which object the simulator has seated it on. The state SHALL be one of: unknown (not yet learned since entering the region), not seated, or seated on the target. The avatar SHALL be considered seated on the target only when the simulator has confirmed a sit on the target object and has not since reported the avatar as standing. The simulator's answer to the bot's own sit request SHALL count as that confirmation even when it names another prim of the target's linkset; a sit the bot did not request SHALL count only when it names the target object itself. In both cases the bot SHALL remember the prim the simulator named as the object the avatar is seated on: that prim is what `status` prints and what a `cur` sit state stands for. When the held sit state changes, the state SHALL be judged again at once: an avatar seated on the prim that has become the target is seated on the target, and the bot SHALL NOT send a new sit request for it. The state SHALL return to unknown whenever the avatar enters a region or the session is lost.

#### Scenario: Unseated by a script
- **WHEN** the avatar is seated on the target and a script in the object unseats it
- **THEN** the bot's state changes to not seated within 5 seconds

#### Scenario: Seat object deleted
- **WHEN** the avatar is seated on the target and the object is deleted
- **THEN** the bot's state changes to not seated within 5 seconds

#### Scenario: Other avatars sitting and standing
- **WHEN** other avatars in the region sit down or stand up
- **THEN** the bot's own seated state does not change

#### Scenario: Seated on another prim of the linkset
- **WHEN** the target is a child prim and the simulator answers the bot's sit request naming the linkset's root prim
- **THEN** the avatar is seated on the target, and the root prim is what `status` and a `cur` sit state name

#### Scenario: Current seat becomes the target
- **WHEN** the avatar was seated on an object by a `sit` command and `rslbot ctl persist` is carried out
- **THEN** the avatar is seated on the target and the bot sends no sit request

### Requirement: Sit on the ground when configured
When the held sit state is `ground`, as it is at startup with `sit = "ground"` in the config, the bot SHALL sit the avatar on the ground where it stands, by the same request the `sit ground` command makes. The simulator does not confirm a ground sit, so the bot SHALL treat the request as carried out once it has been handed to the simulator connection, SHALL NOT report a failure for it, and SHALL NOT repeat it until the presence reconciler calls for a ground sit again. There is no target object with this sit state, and the avatar is never considered seated on the target.

#### Scenario: Ground sit after login
- **WHEN** `sit = "ground"` is configured and the avatar has logged in to the region it is to stay in
- **THEN** other viewers see the avatar sitting on the ground, and the bot sends no further sit request while nothing changes

#### Scenario: Ground sit after a teleport
- **WHEN** the held sit state is `ground` and the avatar arrives in the target region by a teleport
- **THEN** the bot sits it on the ground again

#### Scenario: Ground sit by a persist
- **WHEN** `rslbot ctl persist cur ground` is carried out while the avatar stands
- **THEN** other viewers see the avatar sitting on the ground

### Requirement: No sitting without a configured seat
When the held sit state is `stand` or `unchanged`, the bot SHALL NOT send a sit request of its own accord. That is the case at startup when neither `sit` nor `object` is configured. Sit requests are then made only on an operator's command.

#### Scenario: No seat configured
- **WHEN** the config has no `sit` and the avatar has been connected for 30 minutes without any command
- **THEN** the bot has sent no sit request and the avatar stands where it arrived

#### Scenario: Operator sits the avatar
- **WHEN** the config has no `sit` and the operator sends a `sit <uuid>` command
- **THEN** the avatar sits on that object as the command specifies

#### Scenario: Seat given up by a persist
- **WHEN** `sit` is configured with an object's UUID and `rslbot ctl persist cur stand` is carried out
- **THEN** the avatar stands up and the bot sends no further sit request
