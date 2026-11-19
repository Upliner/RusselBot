# Spec Delta

## MODIFIED Requirements

### Requirement: Converge on the desired state
The bot SHALL continuously work towards a desired state that depends on its placement mode and on the held placement (see `placement-persist`). In holding mode, the mode it starts in, the desired state is "connected", and in addition "in the region of the held location" when a location is held, and "seated" as the held sit state says when that is an object or `ground`. In manual mode, which the operator enters and leaves with commands, the desired state is "connected". Whenever no action is in progress and no backoff delay is pending, it SHALL choose its next action from the currently observed state, the mode and the held placement alone, by the first rule that applies:

1. not connected: log in;
2. in manual mode: do nothing;
3. a location is held and the avatar is not in its region: teleport to the held location;
4. the held sit state is an object and the avatar is not seated on it (including seated state unknown): sit on that object;
5. the held sit state is `ground`, the avatar is not seated on an object, and the bot has not sat the avatar on the ground since the avatar last arrived somewhere, last stood up from an object, or a persist was last carried out: sit on the ground;
6. otherwise: do nothing.

At most one action SHALL be in progress at a time. The bot SHALL NOT assume the result of an earlier action still holds; a change in observed state or in the held placement SHALL lead to the matching rule regardless of what was done before. A login in holding mode SHALL ask for the held location, or while no location is held for the location `start` names; a login made in manual mode SHALL ask for the avatar's last location. Entering manual mode SHALL abandon an automatic teleport or sit that is in progress, without counting it as a failure. A persist that is carried out SHALL clear pending teleport and sit backoff delays and the count of failed sits, whether the bot was in manual mode or already holding, so that the bot acts on the held placement at once.

#### Scenario: Normal start with login landing in the target region
- **WHEN** the bot starts and the grid places the avatar in the target region
- **THEN** the bot sits on the target object without teleporting

#### Scenario: Start in another region
- **WHEN** the bot starts with a configured region and the grid places the avatar in a different region
- **THEN** the bot teleports to the target region and then sits on the target object

#### Scenario: Kicked while seated
- **WHEN** the avatar is seated on the target and gets kicked
- **THEN** the bot logs in again asking for the held location, gets to the target region, and sits on the target object again without operator action

#### Scenario: Region restart
- **WHEN** the target region is restarted while the avatar is seated
- **THEN** once the region is back online the avatar ends up seated on the target object again without operator action

#### Scenario: Moved to another region
- **WHEN** the avatar is teleported away to a different region while it stays connected
- **THEN** the bot teleports back to the held location and sits again

#### Scenario: Moved inside the target region
- **WHEN** the held sit state is `stand` or `ground` and the avatar is moved to another spot in the target region
- **THEN** the bot does not teleport

#### Scenario: Nothing configured but the account
- **WHEN** the config names neither a region, a seat nor `persist`
- **THEN** the bot logs in at the avatar's last location, holds the location it arrived at, logs in there again whenever the session is lost, teleports back there when the avatar is moved to another region, and never sits the avatar by itself

#### Scenario: Moved while no region is configured
- **WHEN** no region is configured, the bot holds the location it arrived at, and the avatar is teleported to another region while it stays connected
- **THEN** the bot teleports back to the held location

#### Scenario: Seat without a region
- **WHEN** an object is configured as the seat, no region is configured, and the login places the avatar in the region that holds the object
- **THEN** the bot sits on the object without teleporting

#### Scenario: Region without a seat
- **WHEN** a region is configured, no seat is, and the avatar is in another region
- **THEN** the bot teleports to the configured region and position and then does nothing more

#### Scenario: Configured ground sit
- **WHEN** the held sit state is `ground` and the avatar has arrived where it is to stay
- **THEN** the bot sits it on the ground once and sends no further sit until the avatar arrives somewhere again, stands up from an object, or a persist is carried out

#### Scenario: Stand and unchanged ask for no sit
- **WHEN** the held sit state is `stand` or `unchanged` and the avatar is in the target region
- **THEN** the bot starts no action, whether the avatar sits or stands

#### Scenario: Left alone in manual mode
- **WHEN** the bot is in manual mode and the avatar stands in a region that is not the target region
- **THEN** the bot neither teleports nor sits

#### Scenario: Kicked in manual mode
- **WHEN** the bot is in manual mode and the avatar gets kicked
- **THEN** the bot logs in again asking for the last location, and then neither teleports nor sits

#### Scenario: Back to holding mode
- **WHEN** a `home` or `persist` command is carried out in manual mode while the avatar stands in a region other than the held location's
- **THEN** the bot teleports to the held location and sits as the held sit state says, starting without a backoff delay

#### Scenario: Held placement changed while holding
- **WHEN** the bot is in holding mode, waiting out a teleport backoff delay, and a `persist` naming a location in another region is carried out
- **THEN** the bot teleports to the new location at once

### Requirement: Reposition after repeated sit failures
When three consecutive sit attempts on the target object have failed while the avatar is in the held location's region, the bot SHALL teleport to the held location before the next sit attempt.

#### Scenario: Avatar too far from the seat
- **WHEN** the avatar is in the target region but too far from the object for the sit to succeed
- **THEN** after three failed sits the bot teleports to the held location and the next sit attempt is made from there

#### Scenario: Object not in reach and no region configured
- **WHEN** an object is configured as the seat, no region is configured, and the first login places the avatar in a region that does not hold the object
- **THEN** after three failed sits the bot teleports to the location that login reported, and keeps attempting the sit there, no more often than once a minute once the backoff is at its maximum
