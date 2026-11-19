# Spec Delta

## MODIFIED Requirements

### Requirement: Commands that suspend automatic placement
The bot SHALL be in one of two placement modes: holding, in which it works towards the held placement (see `placement-persist`), and manual, in which it does not move or seat the avatar by itself. It SHALL start in holding mode. A `stand`, `unsit`, `sit`, `tp`, `requesttp` or `accepttp` command that is carried out SHALL put the bot in manual mode before it acts; a command that is refused SHALL NOT change the mode. A `stand`, `sit` or `tp` that is carried out for a forced RLV command, and a teleport by global coordinates carried out for one (see `rlv-commands`, "Force commands"), SHALL put the bot in manual mode in the same way, and one that is refused SHALL NOT change the mode; whether the bot then stays in manual mode or returns to holding mode is decided by the RLV persist setting (see `rlv-commands`, "Forced movement and the held placement"). A forced `@sit` that is carried out through a persist under that setting is no `sit` command and SHALL NOT put the bot in manual mode. `sendtp`, `touch`, `status`, `say`, `shout`, `whisper` and `im` SHALL NOT change the mode, and neither SHALL the `rlv` commands by themselves, the RLV queries, or a `detach` or `unwear` carried out for an RLV command. Manual mode SHALL last until a `persist` or `home` command is carried out, until the bot returns to holding mode after a forced RLV command (see `rlv-commands`, "Forced movement and the held placement"), or until the bot process ends, across session losses and logins. Besides those commands, given by the operator or carried out for RLV, the bot enters manual mode by itself only once, after startup, when the config's `persist` setting is off, which it is when the config says `persist = "off"`, has no `persist` key, or has a `[persist]` table whose `atstart` is false. The held placement SHALL NOT be changed by entering or being in manual mode.

#### Scenario: Staying where it was put
- **WHEN** `rslbot ctl tp 128 128 25 Home` has been carried out and an hour passes
- **THEN** the avatar is still in `Home` and the bot has not teleported or sat since

#### Scenario: Refused command
- **WHEN** `rslbot ctl sit chair` is refused while the bot is in holding mode
- **THEN** the bot stays in holding mode

#### Scenario: Persist ends manual mode
- **WHEN** `rslbot ctl persist` is carried out after a `tp` command put the bot in manual mode
- **THEN** the bot is in holding mode and holds the place the avatar is at

#### Scenario: Refused persist
- **WHEN** `rslbot ctl persist cur chair` is refused while the bot is in manual mode
- **THEN** the bot stays in manual mode

#### Scenario: Forced away while holding
- **WHEN** the bot holds a place and sits there, RLV is on with the RLV persist setting `drop`, and an object the avatar owns says `@unsit=force` to its owner
- **THEN** the avatar stands up, the bot is in manual mode, and it does not sit the avatar down again by itself

#### Scenario: Forced away while following
- **WHEN** the bot holds a place and sits there, RLV is on with the RLV persist setting `follow`, and an object the avatar owns says `@unsit=force` to its owner
- **THEN** the avatar stands up, and the bot is in holding mode again with the sit state `stand`

#### Scenario: RLV command that is refused
- **WHEN** the bot is in holding mode, RLV is on, and an object the avatar owns says `@sit:chair=force` to its owner
- **THEN** the bot stays in holding mode

#### Scenario: RLV query while holding
- **WHEN** the bot is in holding mode, RLV is on, and an object the avatar owns says `@versionnew=5` to its owner
- **THEN** the bot answers and stays in holding mode
