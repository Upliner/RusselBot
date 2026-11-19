# Spec Delta

## MODIFIED Requirements

### Requirement: Held placement
The bot SHALL keep one held placement, made of a location and a sit state. A location is a region name with coordinates inside that region. A sit state is one of: the UUID of an object, `ground`, `stand` or `unchanged`. In holding mode the bot works towards the held placement as the presence reconciler defines: it teleports to the held location whenever the avatar is in another region, and it seats the avatar as an object or `ground` sit state says. While the avatar is in the held location's region the bot SHALL NOT teleport it because of where in that region it is; the only teleport inside the region is the repositioning after repeated sit failures. With the sit states `stand` and `unchanged` the bot SHALL NOT react to the avatar sitting down or standing up.

The held placement SHALL change only when a `persist` or `home` command is carried out, the config's `persist` setting is applied, or a persist is carried out after a forced RLV command (see `rlv-commands`, "Forced movement and the held placement"), and SHALL be kept through manual mode and across session losses until the bot process ends.

When the bot starts, the held placement is the config placement: `region` and `position` as the location, and `sit` as the sit state, or `stand` when `sit` is absent. When the config names no `position`, no location is held until the simulator reports the avatar's location after the first login of this run; that location, the first location, then becomes the held location, also when a `cfg` location has been persisted in the meantime.

#### Scenario: Moved inside the held region
- **WHEN** the held placement is `Target/128/128/25` with sit state `stand` and a script moves the avatar to 40, 200 in `Target`
- **THEN** the bot sends no teleport and the avatar stays where it was moved

#### Scenario: Moved out of the held region
- **WHEN** the held placement is `Target/128/128/25` with sit state `ground` and the avatar is teleported to region `Home` while it stays connected
- **THEN** the bot teleports to 128, 128, 25 in `Target` and sits the avatar on the ground there

#### Scenario: Kept through manual mode
- **WHEN** a `persist` has been carried out, the operator then runs `stand` and `tp` commands, and an hour passes
- **THEN** the avatar is where the commands put it, and `status` still shows the persisted placement on its `persist` line

#### Scenario: No position in the config
- **WHEN** the config has `sit` with an object's UUID and no `region`, and the first login places the avatar at 60, 70, 22 in `Target`
- **THEN** the held location is `Target/60/70/22` from then on, until a persist changes it

#### Scenario: Changed by a forced RLV command
- **WHEN** the held placement is `Target/128/128/25` with a chair as its sit state, the RLV persist setting is `follow`, and an object the avatar owns says `@tpto:Home/100/120/30=force` to its owner
- **THEN** once the avatar has arrived, the held placement is where it arrived in `Home` with sit state `stand`

#### Scenario: Not changed by a forced RLV command that is dropped
- **WHEN** the same happens with the RLV persist setting `drop`
- **THEN** the held placement is still `Target/128/128/25` with the chair, and the bot is in manual mode

### Requirement: Config persist setting applied after startup
The config's `persist` setting (see `bot-config`) names `cur` or `cfg` for the location and for the sit state, or is off; a config without the setting counts as off, and so does a `[persist]` table whose `atstart` is false. The table's `rlv` key is no part of this setting: it says what follows a forced RLV command (see `rlv-commands`, "Forced movement and the held placement"). The bot SHALL start in holding mode with the config placement and work towards it. The config placement counts as reached when the avatar is connected and in the held location's region and, according to the config's `sit`: with an object, the simulator has confirmed the avatar seated on it; with `ground`, the ground sit has been handed to the simulator connection; without `sit`, nothing more.

The first time the config placement is reached, the bot SHALL carry out a persist with the setting's two values, exactly as the `persist` command with those arguments would, and log it with the placement that results; when the setting is off it SHALL switch to manual mode instead and log that. When the location value is `cur` and the avatar has just been seated on an object, the bot SHALL wait for the avatar's seated position to be known, for at most 10 seconds, before carrying the persist out; after 10 seconds it SHALL carry it out with the position it has and say so in the log.

The setting SHALL be applied at most once in a run of the bot. It SHALL NOT be applied when, before the config placement was reached, a `persist` or `home` command or a command that puts the bot in manual mode was carried out. A session loss before the config placement is reached SHALL NOT cancel it: the bot keeps working towards the config placement after the next login.

#### Scenario: Setting cur with a full config
- **WHEN** the config has `region`, `position`, `sit` with the UUID of a child prim and `persist = "cur"`, and the simulator seats the avatar on the root prim of that linkset
- **THEN** after sitting the bot holds the avatar's seated position as its location and the root prim as its sit state, and `status` shows both on its `persist` line

#### Scenario: Setting cfg
- **WHEN** the same config has `persist = "cfg"`
- **THEN** the bot holds the configured `position` and the configured `sit` UUID: it logs in there, teleports there and sits on that UUID, before and after the setting is applied

#### Scenario: Seat without a region
- **WHEN** the config has `sit` with an object's UUID, `persist = "cur"` and no `region`, and the login places the avatar in `Target`, where the object is
- **THEN** the bot sits on the object, then holds the seated position in `Target`; when the avatar is later teleported to `Home`, the bot teleports back to that position and sits again

#### Scenario: Account alone
- **WHEN** the config has `persist = "cur"` and names neither `region` nor `sit`, and the login places the avatar in `Home`
- **THEN** the bot holds the position it arrived at in `Home` with sit state `stand`: it does not sit, and it teleports back there when the avatar is moved to another region

#### Scenario: Setting off
- **WHEN** the config has `region`, `position`, `sit` with an object's UUID and `persist = "off"`
- **THEN** the bot logs in at the configured place and sits on the object, then switches to manual mode: when unseated it does not sit again, and after a kick it logs in at the last location and neither teleports nor sits

#### Scenario: No setting
- **WHEN** the config has `region`, `position`, `sit` with an object's UUID and no `persist` key
- **THEN** the bot behaves as with `persist = "off"`: it sits on the object once and then is in manual mode

#### Scenario: No setting and nothing but the account
- **WHEN** the config names neither `region`, `sit` nor `persist`, and the login places the avatar in `Home`
- **THEN** the bot switches to manual mode as soon as the avatar is connected: it does not sit, and it does not teleport back when the avatar is moved to another region

#### Scenario: Different values for location and sit state
- **WHEN** the config has `region`, `position`, `sit` with an object's UUID and a `[persist]` table with `position = "cfg"` and `sit = "cur"`
- **THEN** after sitting the bot holds the configured `position` and the prim the simulator seated the avatar on

#### Scenario: Operator acts first
- **WHEN** the operator runs `rslbot ctl tp 128 128 25 Home` before the bot has sat on the configured object
- **THEN** the bot is in manual mode and the config's `persist` setting is not applied later in this run

#### Scenario: Seat never reached
- **WHEN** the configured object does not exist
- **THEN** the bot keeps attempting the sit as the backoff allows, holding the config placement, and the config's `persist` setting is not applied

#### Scenario: Table that holds nothing at the start
- **WHEN** the config has `region`, `position`, `sit` with an object's UUID and a `[persist]` table containing only `rlv = "follow"`
- **THEN** the bot sits on the object once and then is in manual mode, as with `persist = "off"`

#### Scenario: Table with atstart alone
- **WHEN** the same config has a `[persist]` table containing only `atstart = true`
- **THEN** after sitting the bot holds the avatar's seated position and the prim the simulator seated it on, as with `persist = "cur"`
