# presence-reconciler Specification

## Purpose

Defines the control loop that keeps the avatar logged in, in the target region and seated on the target object by repeatedly comparing what it observes with what is wanted, and the policies for retrying, giving up and shutting down.

## Requirements

### Requirement: Converge on the desired state
The bot SHALL continuously work towards a desired state that depends on its placement mode and on the config. In holding mode, the mode it starts in, the desired state is "connected", and in addition "in the target region" when a region is configured and "seated as configured" when `sit` is configured. In manual mode, which the operator enters and leaves with commands, the desired state is "connected". Whenever no action is in progress and no backoff delay is pending, it SHALL choose its next action from the currently observed state, the mode and the config alone, by the first rule that applies:

1. not connected: log in;
2. in manual mode: do nothing;
3. a region is configured and the avatar is not in it: teleport to the target region;
4. an object is configured as the seat and the avatar is not seated on it (including seated state unknown): sit on the target object;
5. a ground sit is configured and the bot has not sat the avatar on the ground since the avatar last arrived somewhere, last stood up from an object, or the bot last returned to holding mode: sit on the ground;
6. otherwise: do nothing.

At most one action SHALL be in progress at a time. The bot SHALL NOT assume the result of an earlier action still holds; a change in observed state SHALL lead to the matching rule regardless of what was done before. A login in holding mode SHALL ask for the configured region and position, or without them for the location `start` names; a login made in manual mode SHALL ask for the avatar's last location. Entering manual mode SHALL abandon an automatic teleport or sit that is in progress, without counting it as a failure. Leaving manual mode SHALL clear pending teleport and sit backoff delays, so that the return starts at once.

#### Scenario: Normal start with login landing in the target region
- **WHEN** the bot starts and the grid places the avatar in the target region
- **THEN** the bot sits on the target object without teleporting

#### Scenario: Start in another region
- **WHEN** the bot starts and the grid places the avatar in a different region
- **THEN** the bot teleports to the target region and then sits on the target object

#### Scenario: Kicked while seated
- **WHEN** the avatar is seated on the target and gets kicked
- **THEN** the bot logs in again, gets to the target region, and sits on the target object again without operator action

#### Scenario: Region restart
- **WHEN** the target region is restarted while the avatar is seated
- **THEN** once the region is back online the avatar ends up seated on the target object again without operator action

#### Scenario: Moved to another region
- **WHEN** the avatar is teleported away to a different region while it stays connected
- **THEN** the bot teleports back to the target region and sits again

#### Scenario: Nothing configured but the account
- **WHEN** the config names neither a region nor a seat
- **THEN** the bot logs in, stays where the grid placed the avatar, logs in again whenever the session is lost, and never teleports or sits by itself

#### Scenario: Moved while no region is configured
- **WHEN** no region is configured and the avatar is teleported to another region while it stays connected
- **THEN** the bot stays in that region and does not teleport

#### Scenario: Seat without a region
- **WHEN** an object is configured as the seat, no region is configured, and the login places the avatar in the region that holds the object
- **THEN** the bot sits on the object without teleporting

#### Scenario: Region without a seat
- **WHEN** a region is configured, no seat is, and the avatar is in another region
- **THEN** the bot teleports to the configured region and position and then does nothing more

#### Scenario: Configured ground sit
- **WHEN** a ground sit is configured and the avatar has arrived where it is to stay
- **THEN** the bot sits it on the ground once and sends no further sit until the avatar arrives somewhere again, stands up from an object, or the bot returns to holding mode

#### Scenario: Left alone in manual mode
- **WHEN** the bot is in manual mode and the avatar stands in a region that is not the target region
- **THEN** the bot neither teleports nor sits

#### Scenario: Kicked in manual mode
- **WHEN** the bot is in manual mode and the avatar gets kicked
- **THEN** the bot logs in again asking for the last location, and then neither teleports nor sits

#### Scenario: Back to holding mode
- **WHEN** the bot leaves manual mode while the avatar stands in another region
- **THEN** it teleports to the target region and sits on the target object, starting without a backoff delay

### Requirement: Backoff between attempts
After a failed action the bot SHALL wait before the next attempt of the same kind. The delay SHALL double with each consecutive failure of that kind between these bounds:

| Action | First delay | Maximum delay |
|---|---|---|
| Login | 10 s | 5 min |
| Teleport | 10 s | 5 min |
| Sit | 5 s | 1 min |

The login delay SHALL also apply before logging in again after a session is lost, whatever the cause, and SHALL be reset to its first value only after a session has stayed connected for 5 minutes. The teleport delay SHALL be reset by a successful teleport. After being unseated, the bot SHALL wait the current sit delay before sitting again, and the sit delay SHALL be reset only after the avatar has stayed seated for 5 minutes.

#### Scenario: Grid down for a long time
- **WHEN** the login server is unreachable for an hour
- **THEN** login attempts become no more frequent than one every 5 minutes, and the bot logs in once the server returns

#### Scenario: Session that keeps dropping
- **WHEN** every session is lost within a minute of login
- **THEN** the interval between logins grows to 5 minutes instead of looping at full speed

#### Scenario: Repeatedly ejected from the seat
- **WHEN** the object unseats the avatar every time it sits
- **THEN** sit attempts become no more frequent than one per minute

### Requirement: Reposition after repeated sit failures
When three consecutive sit attempts on the target object have failed while the avatar is in the configured region, the bot SHALL teleport to the configured position before the next sit attempt. When no region is configured the bot SHALL NOT reposition the avatar; it SHALL keep attempting the sit with the sit backoff.

#### Scenario: Avatar too far from the seat
- **WHEN** the avatar is in the target region but too far from the object for the sit to succeed
- **THEN** after three failed sits the bot teleports to the configured position and the next sit attempt is made from there

#### Scenario: Object not in reach and no region configured
- **WHEN** an object is configured as the seat, no region is configured, and the object is not in the region the avatar is in
- **THEN** the bot keeps attempting the sit, no more often than once a minute once the backoff is at its maximum, and never teleports

### Requirement: Refused logins
After a login attempt is classified as refused, the bot SHALL make its next login attempt asking for the avatar's last location instead of the configured start, because a grid may refuse a login only because the requested location is unavailable. If the bot has not had a successful login since it was started and an attempt asking for the last location is refused, the bot SHALL exit with status 1 and a message stating that the credentials were rejected, and SHALL NOT retry. If the bot has had a successful login since it was started, refused logins SHALL be retried with the login backoff, with a maximum delay of 30 minutes instead of 5.

#### Scenario: Wrong password at startup
- **WHEN** the bot is started with a wrong password
- **THEN** it makes at most two login attempts, prints that the credentials were rejected, and exits with status 1

#### Scenario: Wrong password when starting at the last location
- **WHEN** the bot is started with a wrong password and a config without `region` and `start`
- **THEN** it makes one login attempt, prints that the credentials were rejected, and exits with status 1

#### Scenario: Target region down at startup
- **WHEN** the bot is started with correct credentials while the target region is offline and the grid refuses a login to that region
- **THEN** the bot logs in at the avatar's last location and keeps trying to teleport to the target region

#### Scenario: Home refused at startup
- **WHEN** the bot is started with `start = "home"` and correct credentials, and the grid refuses the login asking for the home location
- **THEN** the bot logs in at the avatar's last location and stays there

#### Scenario: Refusals after a period of normal running
- **WHEN** the bot has been logged in before and logins are now refused
- **THEN** the bot keeps retrying, no more often than every 30 minutes once the backoff is at its maximum, and does not exit

### Requirement: Logins that need the operator
When a login attempt is classified as needs operator, the bot SHALL exit with status 1 and a message naming what the grid demanded, and SHALL NOT retry.

#### Scenario: Account with multi-factor authentication
- **WHEN** the grid answers a login with a multi-factor challenge
- **THEN** the bot exits with status 1 and its message says that multi-factor authentication is not supported

### Requirement: Shutdown on operator signal
On an interrupt or termination signal the bot SHALL stop starting new actions, log out cleanly if connected, and exit with status 0. A second signal during shutdown SHALL make it exit immediately.

#### Scenario: Stop while seated
- **WHEN** the bot receives a termination signal while seated on the target
- **THEN** the avatar goes offline on the grid and the process exits with status 0 within 6 seconds

#### Scenario: Stop while waiting in backoff
- **WHEN** the bot receives a termination signal while disconnected and waiting to retry
- **THEN** the process exits with status 0 immediately

### Requirement: Operator-visible log
The bot SHALL write a timestamped line to standard error for every action it starts, every action outcome with the reason on failure, every change in connection, region or seated state, and every backoff delay it begins, including its length. It SHALL also log the text of every alert message the simulator sends to the avatar, since simulators use these to say why a request was refused or that a region is about to restart. It SHALL NOT log other individual simulator packets unless verbose logging is requested on the command line.

#### Scenario: Simulator refuses a sit with an alert
- **WHEN** the simulator answers a sit request only with an alert such as "There is no suitable surface to sit on"
- **THEN** the log contains that alert text before the sit is reported as failed

#### Scenario: Reading the log after a region restart
- **WHEN** the target region has been restarted and the bot has recovered
- **THEN** the log shows, in order, the session loss with its cause, each login and teleport attempt with its outcome, the delays between them, and the final successful sit
