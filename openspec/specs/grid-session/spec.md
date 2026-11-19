# grid-session Specification

## Purpose

Covers getting the avatar online and keeping it online: logging in to a grid, answering everything a simulator needs to keep the connection open, noticing when the connection is lost, and logging out cleanly.

## Requirements

### Requirement: Login to the configured grid
The bot SHALL log in to the grid at the configured login URL with the configured user and password hash, over HTTPS when the URL is `https` and plain HTTP when it is `http`. A plain `password` from the config SHALL be converted to the hash form before sending. The login request SHALL ask to start at the configured region and position when the config names them; otherwise it SHALL ask for the avatar's home location when `start` is `home`, and for the avatar's last location when `start` is `latest` or absent. The bot SHALL identify itself with its own client name and version and SHALL NOT claim to be another viewer. The bot SHALL follow a login server's instruction to continue the login at another URL.

#### Scenario: Successful login to OpenSim
- **WHEN** the bot is started with valid credentials for a running local OpenSim
- **THEN** the avatar appears online in a region of that grid within 30 seconds

#### Scenario: Login lands directly in the target region
- **WHEN** the target region is online and the grid honours the requested start location
- **THEN** the avatar's first region is the target region

#### Scenario: Requested start location not honoured
- **WHEN** the grid places the avatar in a different region than requested
- **THEN** the login still counts as successful and the session is established in the region the grid chose

#### Scenario: Login at the last location
- **WHEN** the config has neither `region` nor `start`, and the avatar was last in region `Home`
- **THEN** the login request asks for the last location and the avatar's first region is `Home`

#### Scenario: Login at home
- **WHEN** the config has `start = "home"` and the avatar's home is in region `Target`
- **THEN** the login request asks for the home location and the avatar's first region is `Target`

### Requirement: Login outcome classification
The bot SHALL classify every unsuccessful login attempt as one of:

- **needs operator**: the grid demands something the bot cannot supply, such as a multi-factor challenge;
- **refused**: the grid answered and rejected the attempt with its credential-or-location reason (`key`);
- **temporary**: anything else, including network errors, timeouts, HTTP errors, malformed replies, "already logged in" and "logins restricted" replies.

The classification SHALL be reported to the reconciler together with the grid's reason and message text.

#### Scenario: Wrong password
- **WHEN** the grid rejects the login with reason `key`
- **THEN** the attempt is classified as refused

#### Scenario: Login server unreachable
- **WHEN** the connection to the login URL is refused or times out
- **THEN** the attempt is classified as temporary

#### Scenario: Already logged in
- **WHEN** the grid replies that the account appears to be logged in already
- **THEN** the attempt is classified as temporary

#### Scenario: Multi-factor challenge
- **WHEN** the grid answers with a multi-factor authentication challenge
- **THEN** the attempt is classified as needs operator

### Requirement: Keep the simulator connection alive
While connected, the bot SHALL complete the simulator's connection handshake, acknowledge every reliable packet it receives, retransmit its own reliable packets until acknowledged, answer every ping, and send the periodic agent update the simulator expects from a live viewer. Messages the bot has no use for SHALL be acknowledged when reliable and otherwise ignored; they SHALL NOT cause errors or disconnection. The bot SHALL request the lowest practical volume of scene data from the simulator.

#### Scenario: Long idle session
- **WHEN** the avatar is logged in and nothing else happens for 30 minutes
- **THEN** the avatar is still shown as online in the same region and the simulator has not timed it out

#### Scenario: Busy region
- **WHEN** the region contains many objects and other avatars moving and chatting
- **THEN** the bot stays connected and logs no errors about unrecognised messages

#### Scenario: Lost outgoing packet
- **WHEN** a reliable packet sent by the bot is not acknowledged
- **THEN** the bot sends it again, marked as a resend, until it is acknowledged or the session is declared lost

#### Scenario: Duplicate incoming packet
- **WHEN** the simulator resends a reliable packet the bot has already processed
- **THEN** the bot acknowledges it again and does not process it a second time

### Requirement: Detect session loss
The bot SHALL treat the session as lost, and report this to the reconciler with the cause, when any of these happens: the simulator kicks the avatar; the simulator reports the avatar logged out; the current simulator tells the bot it is closing the connection; nothing is received from the current simulator for 60 seconds; a reliable packet remains unacknowledged after all retransmissions; the simulator's event channel reports that the session no longer exists, or cannot be reached for 60 seconds. After a session is lost the bot SHALL stop sending to that simulator.

#### Scenario: Kicked by an administrator
- **WHEN** an administrator kicks the avatar
- **THEN** the bot reports the session as lost within 5 seconds, including the kick reason

#### Scenario: Simulator process dies
- **WHEN** the simulator stops responding without any goodbye
- **THEN** the bot reports the session as lost no later than 60 seconds after the last packet it received

#### Scenario: Event channel gone while UDP still flows
- **WHEN** the simulator's event channel answers that the session does not exist while packets still arrive
- **THEN** the bot reports the session as lost

### Requirement: Clean logout
When asked to stop, a connected bot SHALL tell the simulator it is logging out and wait up to 5 seconds for confirmation before closing. A bot that is not connected SHALL stop immediately.

#### Scenario: Logout confirmed
- **WHEN** the bot is asked to stop while connected
- **THEN** the avatar is shown as offline on the grid within 5 seconds and no "already logged in" error occurs on an immediate restart

#### Scenario: Simulator does not confirm
- **WHEN** the simulator does not answer the logout request
- **THEN** the bot stops anyway after 5 seconds
