# placement-persist Specification

## Purpose

Defines the placement the bot holds for the avatar, an auto-return location and a sit state, and how the operator changes it while the bot runs: the `persist` command, the values it accepts, and how the config's `persist` setting chooses the placement held after startup.

## Requirements

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

### Requirement: Persist command
`persist [<location>] [<sit_state>]` SHALL set the held placement and put the bot in holding mode. An argument that is left out SHALL be taken as `cur`, so `persist` alone is `persist cur cur`. More than two arguments SHALL be answered with an error.

The bot SHALL first turn both arguments into a location and a sit state as "Location and sit state values" and "Special values" define, and check them as "Location and seat must agree on the region" defines. When either fails, the command SHALL be answered with an error that names the argument and the reason, and SHALL change nothing: not the held placement, not the placement mode, not what `last` stands for. `persist` SHALL be carried out whether or not the avatar is connected, except that a value that needs the simulator fails while the avatar is not in a region.

When carried out, the command SHALL:

- make the location and sit state the held placement, and what `last` stands for;
- put the bot in holding mode and clear pending teleport and sit backoff delays, as the presence reconciler defines;
- end the wait of earlier `requesttp` commands, as `home` did;
- with the sit state `stand`, stand the avatar up at once when it is connected and is seated on an object or was sat on the ground by the bot;
- with the sit state `ground`, when the avatar is connected and seated on an object, stand it up, so that the ground sit follows once the simulator reports it standing;
- with the sit state `unchanged`, send nothing about sitting or standing;
- be answered with success at once, with a text that gives the held placement as `<location> <sit state>` in the forms this command accepts, coordinates rounded to centimetres. Success does not wait for, and does not mean, that a teleport or sit that follows has succeeded.

When the avatar is in another region than the held location, the teleport there SHALL start at once. When it is in the held location's region already, the command SHALL NOT move it.

#### Scenario: Stay here
- **WHEN** the operator has run `tp 128 128 25 Blue Lagoon` and `sit <uuid>`, and then runs `rslbot ctl persist`
- **THEN** the command exits with status 0 and prints the avatar's current position in `Blue Lagoon` and the prim it is seated on, the bot is in holding mode, and the avatar stays seated without a new sit request

#### Scenario: Persisted place survives a kick
- **WHEN** `persist` has been carried out while seated in `Blue Lagoon` and the avatar is then kicked
- **THEN** the bot logs in again asking for the persisted location in `Blue Lagoon` and sits on the persisted object again

#### Scenario: Location in another region
- **WHEN** `rslbot ctl persist "Blue Lagoon/128/128/25" ground` is run while the avatar is in `Target`
- **THEN** the command exits with status 0 at once, and the bot teleports to 128, 128, 25 in `Blue Lagoon` and sits the avatar on the ground there

#### Scenario: Location in the current region
- **WHEN** `rslbot ctl persist Target/10/10/25 stand` is run while the avatar stands at 200, 200 in `Target`
- **THEN** the avatar is not moved; when it is later teleported to `Home`, the bot teleports it to 10, 10, 25 in `Target`

#### Scenario: Stand
- **WHEN** `rslbot ctl persist cur stand` is run while the avatar is seated on an object
- **THEN** the avatar stands up, stays in the region, and the bot does not sit it again

#### Scenario: Unchanged
- **WHEN** `rslbot ctl persist cur unchanged` is run while the avatar is seated on an object that a script later unseats it from
- **THEN** the avatar stays seated after the command, and after being unseated the bot sends no sit request

#### Scenario: Ground while seated on an object
- **WHEN** `rslbot ctl persist cur ground` is run while the avatar is seated on an object
- **THEN** the avatar stands up from the object and other viewers then see it sitting on the ground

#### Scenario: While disconnected
- **WHEN** `rslbot ctl persist cfg cfg` is run while the bot is waiting to log in again in manual mode
- **THEN** the command exits with status 0, and the next login asks for the config's location and is followed by the configured sit

#### Scenario: Current values while disconnected
- **WHEN** `rslbot ctl persist` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region, exits with status 1, and the held placement and the mode are what they were

#### Scenario: Too many arguments
- **WHEN** `rslbot ctl persist Target/128/128/25 stand now` is run
- **THEN** the command exits with status 1 and nothing changes

### Requirement: Location and sit state values
A location argument SHALL be either one of the special words or `<region>/<x>/<y>/<z>`: the text after each of the last three `/` characters SHALL be a finite, non-negative decimal number, and the text before them, without surrounding whitespace, SHALL be a non-empty region name that contains no `/`. A sit state argument SHALL be either one of the special words, the UUID of an object, or one of the words `ground`, `stand` and `unchanged`. The special words are `cur`, `cfg`, `last` and `last_cmd`. All words SHALL be recognised whatever the case of their letters. Any other argument SHALL be answered with an error naming it and the accepted forms.

#### Scenario: Region name of several words
- **WHEN** `rslbot ctl persist "Blue Lagoon/128/128.5/25" stand` is run
- **THEN** the held location is 128, 128.5, 25 in region `Blue Lagoon`

#### Scenario: Malformed location
- **WHEN** `rslbot ctl persist Target/128/128 stand` or `rslbot ctl persist Target/north/128/25 stand` is run
- **THEN** the command exits with status 1 naming the location and its form, and nothing changes

#### Scenario: Malformed sit state
- **WHEN** `rslbot ctl persist cur chair` is run
- **THEN** the command exits with status 1 naming `chair` and the accepted sit states, and nothing changes

### Requirement: Special values
The special words SHALL stand for these values at the moment the command is carried out; what is held afterwards is the value, not the word:

| Word | As location | As sit state |
|---|---|---|
| `cur` | The region the avatar is in, with its current position as the bot knows it (see `region-teleport`, "Know the current position") | The prim the simulator has seated the avatar on. When it is not seated on an object: `ground` if the last sit or stand request the bot sent since the avatar last arrived somewhere, by command or by itself, was a ground sit, and `stand` otherwise |
| `cfg` | `region` and `position` from the config; when the config has none, the first location (see "Held placement") | `sit` from the config; `stand` when the config has none |
| `last` | The location of the last persist that was carried out; when there was none, as `cfg` | The sit state of the last persist that was carried out; when there was none, as `cfg` |
| `last_cmd` | The destination of the last `tp` command that was carried out, with the region and height it resolved to; when there was none, as `last` | What the last `sit` command that was carried out named, an object's UUID or `ground`; when there was none, as `last` |

A persist carried out because of the config's `persist` setting counts as a persist for `last`. A `tp` or `sit` command counts for `last_cmd` once it has been handed to the simulator connection, whatever its outcome; `accepttp`, `requesttp` and `stand` do not count. `last` and `last_cmd` are remembered until the bot process ends.

`cur` SHALL be answered with an error when the avatar is not connected to a region or is in the middle of a teleport or a logout. `cur` as sit state SHALL also be answered with an error when the avatar is attached to an object the simulator has not named. A `cfg` location given before the first location is known SHALL be accepted: no location is then held until the first location is known.

#### Scenario: Current values while seated
- **WHEN** `rslbot ctl persist` is run while the avatar is seated on a chair whose seat is at 131, 64, 23 in `Target`, after a `sit` command sent from 100, 100, 25
- **THEN** the held location is within a metre of 131, 64, 23 in `Target`, not 100, 100, 25, and the held sit state is the prim the simulator named when it seated the avatar

#### Scenario: Current sit state on the ground
- **WHEN** `rslbot ctl sit ground` has been carried out and `rslbot ctl persist` is run
- **THEN** the held sit state is `ground`

#### Scenario: Current sit state standing
- **WHEN** `rslbot ctl stand` has been carried out and `rslbot ctl persist` is run
- **THEN** the held sit state is `stand`

#### Scenario: Config values
- **WHEN** the config has `region = "Target"`, `position = [128, 128, 25]` and `sit` with an object's UUID, and `rslbot ctl persist cfg cfg` is run after a trip
- **THEN** the bot teleports to 128, 128, 25 in `Target` and sits on the configured object

#### Scenario: Config values without a configured position
- **WHEN** the config has neither `region` nor `sit`, the first login placed the avatar at 60, 70, 22 in `Home`, and `rslbot ctl persist cfg cfg` is run after a `tp` to `Target`
- **THEN** the bot teleports to 60, 70, 22 in `Home` and does not sit the avatar

#### Scenario: Last without an earlier persist
- **WHEN** the config has `persist = "off"` and no `persist` command has been carried out, and `rslbot ctl persist last last` is run
- **THEN** the held placement is the one `persist cfg cfg` would give

#### Scenario: Last after a persist
- **WHEN** `persist` was carried out while seated in `Blue Lagoon`, the operator then ran `tp 128 128 25 Home`, and `rslbot ctl persist last last` is run
- **THEN** the bot teleports to the persisted location in `Blue Lagoon` and sits on the persisted object

#### Scenario: Last teleport destination
- **WHEN** `rslbot ctl tp 100 120 30 Home` has been carried out, later commands took the avatar elsewhere, and `rslbot ctl persist last_cmd stand` is run
- **THEN** the held location is `Home/100/120/30` and the bot teleports there

#### Scenario: Last sit target
- **WHEN** `rslbot ctl sit <uuid>` and then `rslbot ctl stand` have been carried out, and `rslbot ctl persist cur last_cmd` is run
- **THEN** the held sit state is that UUID and the bot sits the avatar on that object again

#### Scenario: No earlier tp or sit command
- **WHEN** no `tp` and no `sit` command has been carried out and `rslbot ctl persist last_cmd last_cmd` is run
- **THEN** the effect is that of `persist last last`

#### Scenario: Seat the simulator did not name
- **WHEN** the avatar is attached to an object for which the simulator sent no sit confirmation, and `rslbot ctl persist` is run
- **THEN** the command exits with status 1 saying the seat is not known and that its UUID can be given instead

### Requirement: Location and seat must agree on the region
`persist` SHALL be answered with an error naming both regions, and SHALL change nothing, when the sit state is an object the bot knows to be in a region other than the location's. Region names SHALL be compared as the target region is: without regard to letter case or surrounding whitespace. The bot knows an object's region in two ways: the simulator has confirmed the avatar seated on that object during this run of the bot, in the region the avatar was in then (the bot remembers at least the 64 objects most recently sat on); or the object is among those the bot has learned in the region the avatar is in now. An object the bot knows nothing about SHALL pass the check. The check SHALL apply to `home` and to special words as well as to values given in full.

#### Scenario: Seat here, location elsewhere
- **WHEN** `rslbot ctl persist "Home/128/128/25" cur` is run while the avatar is seated on an object in `Target`
- **THEN** the command exits with status 1 naming `Home` and `Target`, the avatar stays seated, and the held placement and the mode are what they were

#### Scenario: Location here, seat elsewhere
- **WHEN** the avatar sat on an object in `Target` earlier in this run, the operator then ran `tp 128 128 25 Home`, and `rslbot ctl persist cur <that object's uuid>` is run
- **THEN** the command exits with status 1 naming `Home` and `Target`

#### Scenario: Object the bot has not met
- **WHEN** `rslbot ctl persist "Home/128/128/25" <uuid>` names an object the bot has never sat on and has not learned in the current region
- **THEN** the command is carried out

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
