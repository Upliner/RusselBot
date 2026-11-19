# Spec Delta

## MODIFIED Requirements

### Requirement: Commands that suspend automatic placement
The bot SHALL be in one of two placement modes: holding, in which it keeps the avatar in the configured region and seated as configured, as far as the config names a region and a seat, and manual, in which it does not move or seat the avatar by itself. It SHALL start in holding mode. A `stand`, `unsit`, `sit`, `tp`, `requesttp` or `accepttp` command that is carried out SHALL put the bot in manual mode before it acts; a command that is refused SHALL NOT change the mode. `sendtp`, `touch`, `status`, `say`, `shout`, `whisper` and `im` SHALL NOT change the mode. Manual mode SHALL last until a `home` command or until the bot process ends, across session losses and logins. The modes exist whatever the config names: with neither a region nor a seat configured, the two differ only in the location a login asks for.

#### Scenario: Staying where it was put
- **WHEN** `rslbot ctl tp 128 128 25 Home` has been carried out and an hour passes
- **THEN** the avatar is still in `Home` and the bot has not teleported or sat since

#### Scenario: Refused command
- **WHEN** `rslbot ctl sit chair` is refused while the bot is in holding mode
- **THEN** the bot stays in holding mode

### Requirement: Home command
`home` SHALL put the bot in holding mode and SHALL be answered with success at once, whether or not the avatar is connected. The bot then works towards the configured region and seat as it does after a start. When no region is configured, `home` SHALL NOT move the avatar, whatever `start` says: the bot only seats it again as configured, where it is. In holding mode already, `home` SHALL change nothing.

#### Scenario: Returning after a trip
- **WHEN** `rslbot ctl home` is run while the avatar stands in another region in manual mode
- **THEN** the bot teleports to the configured region and position and sits on the configured object

#### Scenario: Home while disconnected
- **WHEN** `rslbot ctl home` is run while the bot is waiting to log in again in manual mode
- **THEN** the command exits with status 0 and the next login is followed by the usual teleport and sit

#### Scenario: Home without a configured region
- **WHEN** the config has `start = "home"` and `sit = "ground"`, and `rslbot ctl home` is run after a `tp` command took the avatar to another region
- **THEN** the avatar stays in that region and the bot sits it on the ground there

### Requirement: Status command
`status` SHALL be answered with success and a text of one `name: value` line each for: `connected` (`yes` or `no`); `region` (the region's name); `position` (the last position the simulator reported for the avatar, as `x, y, z`); `seat` (`target` when seated on the object configured as `sit`, `object <uuid>` when seated on an object by a `sit` command, `other` when seated on anything else, `none` when not seated on an object, `unknown` when not yet learned); and `mode` (`holding` or `manual`). A ground sit, configured or commanded, is not a seat on an object and SHALL show as `none`. `region`, `position` and `seat` SHALL be left out while not connected to a region. `status` SHALL work in every state of the bot.

#### Scenario: Seated on the target
- **WHEN** `rslbot ctl status` is run while the avatar is seated on the configured object in `Target` and no movement command was given
- **THEN** it prints `connected: yes`, `region: Target`, a `position` line, `seat: target` and `mode: holding`, and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl status` is run while the bot is waiting to log in again
- **THEN** it prints `connected: no` and the `mode` line, and exits with status 0

#### Scenario: Configured ground sit
- **WHEN** `rslbot ctl status` is run after the bot has sat the avatar on the ground as `sit = "ground"` asks
- **THEN** it prints `seat: none` and `mode: holding`
