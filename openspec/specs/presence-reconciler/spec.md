# presence-reconciler Specification

## Purpose

Defines the control loop that keeps the avatar logged in, in the target region and seated on the target object by repeatedly comparing what it observes with what is wanted, and the policies for retrying, giving up and shutting down.

## Requirements

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
- **WHEN** the bot is in holding mode, the avatar is seated on the target and gets kicked
- **THEN** the bot logs in again asking for the held location, gets to the target region, and sits on the target object again without operator action

#### Scenario: Region restart
- **WHEN** the bot is in holding mode and the target region is restarted while the avatar is seated
- **THEN** once the region is back online the avatar ends up seated on the target object again without operator action

#### Scenario: Moved to another region
- **WHEN** the bot is in holding mode and the avatar is teleported away to a different region while it stays connected
- **THEN** the bot teleports back to the held location and sits again

#### Scenario: Moved inside the target region
- **WHEN** the held sit state is `stand` or `ground` and the avatar is moved to another spot in the target region
- **THEN** the bot does not teleport

#### Scenario: Nothing configured but the account
- **WHEN** the config names neither a region, a seat nor `persist`
- **THEN** the bot logs in at the avatar's last location, is in manual mode once the avatar is connected, logs in at the last location again whenever the session is lost, and never teleports or sits the avatar by itself

#### Scenario: Holding the place it arrived at
- **WHEN** the config has `persist = "cur"` and names neither a region nor a seat
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
When three consecutive sit attempts on the target object have failed while the avatar is in the held location's region, the bot SHALL teleport to the held location before the next sit attempt.

#### Scenario: Avatar too far from the seat
- **WHEN** the avatar is in the target region but too far from the object for the sit to succeed
- **THEN** after three failed sits the bot teleports to the held location and the next sit attempt is made from there

#### Scenario: Object not in reach and no region configured
- **WHEN** an object is configured as the seat, no region is configured, and the first login places the avatar in a region that does not hold the object
- **THEN** after three failed sits the bot teleports to the location that login reported, and keeps attempting the sit there, no more often than once a minute once the backoff is at its maximum

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

### Requirement: Steps that RLV restrictions forbid are held back
While an RLV restriction is in force (see `rlv-restrictions`, "When restrictions are in force"), the bot SHALL NOT start a step of its own towards the held placement that the restriction would refuse the operator's command for, whatever "Converge on the desired state" chooses, unless the persist override setting names the step's sort (see `rlv-commands`, "Persist override setting"):

| Step | Held back as | Sort |
|---|---|---|
| teleport to the held location, and the teleport that repositions the avatar after repeated sit failures | a `tp` to that destination (`rlv-restrictions`, "Teleports and teleport offers") | `tp` |
| sit on the held object | `sit <uuid>` (`rlv-restrictions`, "Sitting and standing") | `sit` |
| sit on the ground | `sit ground` | `sit` |
| the stand a `persist` or `home` calls for, when the held sit state does not match the seat the avatar is on | `stand` | `sit` |

A sit on the held object that a restriction holds back because the bot does not know how far the object is SHALL start the object scan (see `rlv-restrictions`, "Objects the bot does not know"), so that the step is no longer held back once the object is described and within reach.

A step whose sort the persist override setting names SHALL be started as if no restriction were kept: no restriction SHALL hold it back, `unsit` while the avatar sits among them. The setting SHALL change nothing else: the operator's commands, RLV force commands, the acceptance of offers and requests and the restoring of attachments SHALL be held to restrictions whatever it names.

A login SHALL never be held back. A step that is held back is not a failed attempt: it SHALL NOT lengthen any backoff delay, SHALL NOT count as a sit failure, and SHALL NOT make the bot give up. The bot SHALL stay in holding mode with its held placement unchanged, and SHALL choose its next action anew whenever a restriction is set, lifted or dropped, whenever the RLV mode or the persist override setting changes, and whenever what the simulator shows of the avatar changes. It SHALL write one line to the operator log when a step starts being held back, naming the step, the restriction and a holder of it, and one when the step is no longer held back; it SHALL NOT repeat the line while nothing changes. When it starts a step that a restriction would hold back but for the persist override setting, the step's line in the operator log SHALL name the restriction and say that the setting lets the step through.

The `persist` and `home` commands SHALL be carried out whatever restrictions are in force: they change what the bot holds, and the steps that follow are held back or not as this requirement says. The steps that belong to an exempt `persist` or `home`, and the steps towards a seat that a followed `@sit` made the held one, are excepted as `rlv-restrictions` says ("Override exempts one command", "Force commands under restrictions"). While RLV is off no step SHALL be held back.

#### Scenario: Caged
- **WHEN** the bot holds a chair in `Target` and the avatar sits on it, the persist override setting is empty, and a cage the avatar owns then moves the avatar to `Elsewhere` and holds `tploc`
- **THEN** the bot starts no teleport, stays in holding mode, and the operator log says once that the teleport to `Target` is held back because of `tploc`, naming the cage

#### Scenario: Released
- **WHEN** the cage then says `@tploc=y` to its owner
- **THEN** the bot teleports the avatar to `Target` and seats it on the chair, and the operator log says the teleport is no longer held back

#### Scenario: The holder is left behind
- **WHEN** the bot holds a chair in `Target`, a rezzed object in `Target` holds `sit`, and the avatar is unseated
- **THEN** the bot starts no sit while the object holds `sit`, and sits the avatar down again once the object is deleted

#### Scenario: No backoff from waiting
- **WHEN** a sit on the held object has been held back for an hour because of `sittp` and the restriction is lifted
- **THEN** the bot starts the sit at once, with the delay it would have had an hour ago

#### Scenario: Persist is accepted
- **WHEN** a chair holds `unsit`, the avatar sits on it, the persist override setting is empty, and `rslbot ctl persist cur stand` is run
- **THEN** the command exits with status 0 and the held sit state is `stand`, the avatar stays seated, and the operator log says the stand is held back because of `unsit`

#### Scenario: Login is never held back
- **WHEN** the operator has set `tploc` with `rlv cmd` and the session is lost
- **THEN** the bot logs in again as it does without the restriction

#### Scenario: RLV off
- **WHEN** a cage holds `tploc`, the avatar is outside the held region, and `rslbot ctl rlv mode off` is run
- **THEN** the bot teleports the avatar to the held location

#### Scenario: Back from the hub after a region restart
- **WHEN** the config has `override = ["sit", "tp"]` in its `[persist]` table, the bot holds a chair in `Target`, a collar the avatar wears holds `tploc`, `sittp` and `unsit`, and a restart of `Target` leaves the avatar in a hub region without the session being lost
- **THEN** once `Target` is up the bot teleports the avatar there and seats it on the chair, and the operator log names `tploc` and `sittp` as let through by the setting

#### Scenario: Teleports only
- **WHEN** the persist override setting is `tp`, the bot holds a chair in `Target`, a collar holds `tploc` and `sit`, and the avatar is moved to another region
- **THEN** the bot teleports the avatar to `Target`, starts no sit, and the operator log says the sit is held back because of `sit`

#### Scenario: Seated elsewhere
- **WHEN** the persist override setting is `tp`, the bot holds a place in `Target`, and the avatar sits on an object in another region whose script holds `unsit`
- **THEN** the bot teleports the avatar to `Target`

#### Scenario: Stand under the setting
- **WHEN** the persist override setting is `sit`, a chair holds `unsit`, the avatar sits on it, and `rslbot ctl persist cur stand` is run
- **THEN** the avatar stands up

#### Scenario: Commands are not covered
- **WHEN** the persist override setting is `sit` and `tp`, a collar holds `tploc`, and `rslbot ctl tp 128 128 25 Home` is run
- **THEN** the command is refused naming `tploc`

#### Scenario: Switched while a step waits
- **WHEN** the teleport to `Target` is held back because of `tploc` and `rslbot ctl rlv persist override tp` is run
- **THEN** the bot starts the teleport at once

#### Scenario: Seat not described yet
- **WHEN** `scan_distance` is not configured, the persist override setting is empty, a collar holds `sittp:20`, and the avatar arrives 2 metres from the seat the bot holds, which the simulator has only mentioned
- **THEN** the bot scans, and seats the avatar as soon as the seat is described; the operator log says that the sit was held back and that it no longer is

#### Scenario: Seat out of reach
- **WHEN** the same happens with `sittp` held without a distance
- **THEN** once the seat is described the sit stays held back, and the log gives the seat's distance
