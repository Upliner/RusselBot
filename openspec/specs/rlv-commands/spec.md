# rlv-commands Specification

## Purpose

Lets scripted objects the avatar owns steer it through the RestrainedLove API (RLV), as they would steer an avatar logged in with an RLV viewer: the bot reads the `@` commands such objects say to their owner, answers their version queries, and carries out the force commands it has a control-socket command for. Defines RLV mode, the `rlv` control-socket commands, what a forced movement does to the placement the bot holds, and what happens to RLV commands the bot does not support yet.

## Requirements

### Requirement: RLV mode
The bot SHALL be in one of two RLV modes, on and off. It SHALL start in the mode the config key `rlv` gives, which is off when the key is absent. While RLV is off the bot SHALL NOT read any chat line as RLV commands, SHALL NOT answer any RLV query, and SHALL send the simulator nothing it would not send without this capability. The mode SHALL change only through the `rlv mode` command, and SHALL then last until the next `rlv mode` command or until the bot process ends, across session losses and logins. The mode SHALL NOT be saved: a restarted bot starts from its config.

#### Scenario: Off by default
- **WHEN** the config has no `rlv` key and an object the avatar owns says `@versionnew=5` to its owner
- **THEN** the bot says nothing on channel 5, and the line is recorded in the chat log like any other chat of kind `owner`

#### Scenario: On from the config
- **WHEN** the config contains `rlv = true` and an object the avatar owns says `@versionnew=5` to its owner
- **THEN** the bot answers on channel 5

#### Scenario: Switched on while running
- **WHEN** the config has no `rlv` key, `rslbot ctl rlv mode on` has been carried out, and an object the avatar owns says `@versionnew=5` to its owner
- **THEN** the bot answers on channel 5

#### Scenario: Mode survives a login
- **WHEN** `rslbot ctl rlv mode on` has been carried out, the avatar is kicked and the bot has logged in again
- **THEN** RLV is still on

#### Scenario: Mode is not saved
- **WHEN** `rslbot ctl rlv mode on` has been carried out with a config that has no `rlv` key, and the bot is stopped and started again
- **THEN** RLV is off

### Requirement: RLV mode and status commands
The control socket SHALL take the command `rlv` with a subcommand as its first argument. `rlv mode on` SHALL switch RLV on and `rlv mode off` SHALL switch it off; each SHALL be answered with success and the text `rlv: on` or `rlv: off`, also when RLV was in that mode already. `rlv status` SHALL be answered with success and two lines: `rlv: on` or `rlv: off` for the mode RLV is in, and `persist: drop` or `persist: follow` for the RLV persist setting (see "RLV persist setting"). All three SHALL work in every state of the bot, connected or not, and SHALL NOT change the bot's placement mode. `rlv` without a subcommand, with a subcommand other than `mode`, `status`, `persist` and `cmd`, `rlv mode` with anything other than exactly one of `on` and `off`, and `rlv status` with further arguments SHALL be answered with an error and change nothing. Switching RLV off SHALL drop the RLV commands that are kept for later (see "Commands wait until the avatar can act").

#### Scenario: Switching on
- **WHEN** `rslbot ctl rlv mode on` is run while RLV is off
- **THEN** it prints `rlv: on`, exits with status 0, and RLV is on

#### Scenario: Switching off
- **WHEN** `rslbot ctl rlv mode off` is run while RLV is on, and an object the avatar owns then says `@sit:<uuid>=force` to its owner
- **THEN** the command prints `rlv: off` and the avatar does not sit

#### Scenario: Already in that mode
- **WHEN** `rslbot ctl rlv mode on` is run while RLV is on
- **THEN** it prints `rlv: on` and exits with status 0

#### Scenario: Status
- **WHEN** `rslbot ctl rlv status` is run with a config that has neither `rlv` nor `persist`, and no `rlv` command before it
- **THEN** it prints `rlv: off` and `persist: drop`, and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl rlv mode on` is run while the bot is waiting to log in again
- **THEN** it exits with status 0 and RLV is on when the avatar is back

#### Scenario: Bad arguments
- **WHEN** `rslbot ctl rlv`, `rslbot ctl rlv mode`, `rslbot ctl rlv mode maybe` or `rslbot ctl rlv restrictions` is run
- **THEN** the command exits with status 1 with a text naming what `rlv` takes, and the mode does not change

### Requirement: Which chat lines are RLV commands
While RLV is on, a local chat message SHALL be read as RLV commands when it is of kind `owner` (see `message-log`, "Read local chat"), the simulator names an object as its source, and its text starts with `@`. That object is the sender of the commands in it. No other message SHALL be read as RLV commands: not chat of any other kind, not chat whose source is an avatar or the grid, and not an instant message, whatever its text. A message that is read as RLV commands SHALL still be recorded as `message-log` says, like any other chat.

#### Scenario: Object the avatar owns
- **WHEN** RLV is on and an object the avatar owns says `@unsit=force` to its owner while the avatar sits
- **THEN** the avatar stands up

#### Scenario: Rezzed and worn objects alike
- **WHEN** RLV is on and the same line comes once from an attachment and once from an object rezzed in the region, both owned by the avatar
- **THEN** both are carried out

#### Scenario: Avatar says the same text
- **WHEN** RLV is on and another avatar says `@unsit=force` in local chat
- **THEN** the avatar stays seated and nothing about RLV is written to the operator log

#### Scenario: Instant message
- **WHEN** RLV is on and another avatar sends the instant message `@version`
- **THEN** the bot sends no answer, and the message is recorded as an instant message

#### Scenario: Ordinary owner message
- **WHEN** RLV is on and an object the avatar owns says `Battery low` to its owner
- **THEN** nothing is carried out and the line is recorded in the chat log

#### Scenario: Still in the chat log
- **WHEN** RLV is on, `log_chat` is configured and an object the avatar owns says `@versionnum=5` to its owner
- **THEN** the chat log has a row of kind `owner` with that text

### Requirement: Command syntax
The text after the leading `@` SHALL be split at commas into commands, and empty pieces SHALL be skipped. Each command SHALL be read as `<behaviour>[:<option>]=<param>`: the param is everything after the first `=`; in what is before it, the option is everything after the first `:`, and the behaviour is the rest. The behaviour and the param SHALL be compared without regard to letter case; the option SHALL be taken as it is. Nothing SHALL be trimmed: a space is part of the piece it is in.

A command without `=`, or with an empty behaviour, is malformed, with one exception: `clear`, which RLV writes alone, as `@clear`, or with a text after its `=` that says what to clear. Either form is well-formed, whatever that text is. So is a command whose param is not one of: `force`; a whole number (a query, the number being the channel to answer on); `n`, `add`, `y` or `rem` (a restriction). A malformed command SHALL NOT be carried out. The commands of one message SHALL be dealt with in the order they are written, each on its own: a command that is malformed, not supported or refused SHALL NOT keep the others from being carried out.

#### Scenario: Several commands in one line
- **WHEN** an object the avatar owns says `@versionnum=5,unsit=force` to its owner
- **THEN** the bot answers on channel 5 and the avatar stands up

#### Scenario: Letter case
- **WHEN** an object the avatar owns says `@UnSit=FORCE` to its owner
- **THEN** the avatar stands up

#### Scenario: One bad command among good ones
- **WHEN** an object the avatar owns says `@nonsense,versionnum=5` to its owner
- **THEN** the operator log notes `nonsense` as malformed, and the bot answers on channel 5

#### Scenario: Spaces
- **WHEN** an object the avatar owns says `@unsit = force` to its owner
- **THEN** the avatar stays seated and the operator log notes the command as not carried out

#### Scenario: A second at-sign
- **WHEN** an object the avatar owns says `@versionnum=5,@unsit=force` to its owner
- **THEN** the bot answers on channel 5, and the avatar stays seated because `@unsit` is no behaviour the bot knows

#### Scenario: Clear alone
- **WHEN** an object the avatar owns says `@clear,unsit=force` or `@clear=tp,unsit=force` to its owner
- **THEN** the operator log notes the first command as not supported, not as malformed, and the avatar stands up

#### Scenario: Trailing comma
- **WHEN** an object the avatar owns says `@unsit=force,` to its owner
- **THEN** the avatar stands up and nothing is noted as malformed

### Requirement: Version queries
The bot implements version 0.1.0 of the RLV API, whatever the bot's own version is. While RLV is on it SHALL answer these queries by saying the reply once, at normal volume, on the channel the param names:

| Command | Reply |
|---|---|
| `@version=<channel>` | `RestrainedLife viewer v0.1.0 (RusselBot <bot version>)` |
| `@versionnew=<channel>` | `RestrainedLove viewer v0.1.0 (RusselBot <bot version>)` |
| `@versionnum=<channel>` | `10000` |

`<bot version>` is the version of the bot program. The channel SHALL be a whole number other than 0, positive or negative, that a chat channel can be; with channel 0 or a number out of that range the query SHALL NOT be answered. An option on a version query SHALL be ignored. A version behaviour with the param `force` or a restriction param is not supported.

#### Scenario: Old version query
- **WHEN** an object the avatar owns says `@version=2222` to its owner
- **THEN** a script listening on channel 2222 hears the avatar say `RestrainedLife viewer v0.1.0 (RusselBot 0.1.0)`, for a bot of version 0.1.0

#### Scenario: New version query
- **WHEN** an object the avatar owns says `@versionnew=2222` to its owner
- **THEN** a script listening on channel 2222 hears the avatar say `RestrainedLove viewer v0.1.0 (RusselBot 0.1.0)`, for a bot of version 0.1.0

#### Scenario: Version number
- **WHEN** an object the avatar owns says `@versionnum=2222` to its owner
- **THEN** a script listening on channel 2222 hears the avatar say `10000`

#### Scenario: Negative channel
- **WHEN** an object the avatar owns says `@versionnum=-873246` to its owner
- **THEN** the avatar says `10000` on channel -873246

#### Scenario: Channel zero
- **WHEN** an object the avatar owns says `@versionnew=0` to its owner
- **THEN** nothing is said in local chat and the operator log notes that channel 0 cannot be answered on

### Requirement: Force commands
While RLV is on, these commands with the param `force` SHALL be carried out as the named control-socket command is with the arguments shown: with the same checks, the same requests to the simulator and the same refusals.

| RLV command | Carried out as |
|---|---|
| `@sit:<uuid>=force` | `sit <uuid>` (see `avatar-commands`) |
| `@sitground=force` | `sit ground` |
| `@unsit=force` | `stand` |
| `@tpto:<region>/<x>/<y>/<z>=force` | `tp <x> <y> <z> <region>` |
| `@detach:<point>=force`, `@remattach:<point>=force` | `detach <point>` (see `avatar-outfit`) |
| `@detach:<uuid>=force`, `@remattach:<uuid>=force` | `detach <item>`, the item being the inventory item the attachment with that UUID was made from |
| `@remoutfit:<kind>=force` | `unwear <kind>` |
| `@detachme=force` | `detach <item>`, the item being the inventory item the sender was made from |

- `@sit` SHALL be refused when its option is not a UUID. While the RLV persist setting is `follow`, `@sit` is carried out through a persist in place of the `sit` command, as "Forced movement and the held placement" says. An option on `@sitground`, `@unsit` or `@detachme` SHALL be ignored.
- `@tpto` with an option of four parts separated by `/` names a region and coordinates inside it. With three parts it gives global coordinates X, Y and Z in metres from the grid's origin: the bot SHALL teleport the avatar to the region that contains that point, taking regions to be 256 metres square, at the point's place inside the region and at height Z. Global coordinates that are both below 256 SHALL be refused, with a reason that names the form with the region's name: they lie in the region at 0, 0, which no grid has and which a grid asked for does not answer about, and are most likely coordinates inside a region. `ctl tp` has no such form; the teleport SHALL otherwise be carried out and end as a `tp` does. In either form everything from a `;` on SHALL be ignored (RLV puts a direction to look in there), the coordinates SHALL be finite, non-negative decimal numbers, and an option of any other shape SHALL be refused.
- `<point>` SHALL be one of the attachment point names of `avatar-outfit` ("Body-part names"), compared without regard to case, or one of the names RLV uses instead: `center 2`, `top right`, `top`, `top left`, `center`, `bottom left`, `bottom`, `bottom right` for the points of the heads-up display, and `root` for `Avatar Center`. A number SHALL NOT be taken as a point.
- `<uuid>` is the UUID of an attachment on the avatar: of the object, which is what a script knows of it. A UUID that is no such object's SHALL be taken as the UUID of the inventory item itself, which is what the bot's own commands show, and SHALL be dealt with as `detach <uuid>` deals with one.
- `<kind>` SHALL be one of the kinds of clothing of `avatar-outfit`.
- `@detachme` SHALL be refused when the sender is not an attachment the bot knows to be on the avatar (see `avatar-outfit`, "Knowing what is attached").

Nobody waits for the outcome of a force command: where the control-socket command would answer its client, the outcome SHALL be written to the operator log instead. What a `sit`, `stand` or `tp` carried out for RLV, and a teleport by global coordinates, do to the placement mode and to the held placement is said in "Forced movement and the held placement". No command carried out for RLV SHALL count as the last `tp` or `sit` command for the `last_cmd` value of `placement-persist`.

#### Scenario: Forced sit
- **WHEN** an object the avatar owns says `@sit:<uuid>=force` to its owner, naming an object near the avatar that can be sat on
- **THEN** the avatar is seated on that object and the operator log says so

#### Scenario: Forced sit that fails
- **WHEN** the RLV persist setting is `drop` and an object the avatar owns says `@sit:<uuid>=force` to its owner, naming no object in the region
- **THEN** the avatar stays as it was and the operator log gives the reason within 16 seconds

#### Scenario: Sit on the ground and stand up
- **WHEN** an object the avatar owns says `@sitground=force` to its owner and, some seconds later, `@unsit=force`
- **THEN** other viewers see the avatar sit on the ground and then stand

#### Scenario: Teleport to a named region
- **WHEN** an object the avatar owns says `@tpto:Blue Lagoon/128/100/25=force` to its owner
- **THEN** the avatar arrives in `Blue Lagoon` near 128, 100, 25

#### Scenario: Look-at is ignored
- **WHEN** an object the avatar owns says `@tpto:Home/128/100/25;1.57=force` to its owner
- **THEN** the avatar arrives in `Home` near 128, 100, 25

#### Scenario: Teleport by global coordinates
- **WHEN** region `Home` has its south-west corner at 256000, 256000 and an object the avatar owns says `@tpto:256128/256100/25=force` to its owner while the avatar is in another region
- **THEN** the avatar arrives in `Home` near 128, 100, 25

#### Scenario: Local coordinates without the region
- **WHEN** an object the avatar owns says `@tpto:219/102/611=force` to its owner
- **THEN** nothing is sent to the simulator, the avatar can be given other commands at once, and the operator log gives a reason that names `region/x/y/z`

#### Scenario: Malformed destination
- **WHEN** an object the avatar owns says `@tpto:north/128=force` or `@tpto:Home/128/-5/25=force` to its owner
- **THEN** nothing is sent to the simulator and the operator log gives the reason

#### Scenario: Detaching from a point
- **WHEN** `Watch` and `Radio` are attached to the left hand and an object the avatar owns says `@detach:left hand=force` to its owner
- **THEN** both are detached, and attachments elsewhere stay

#### Scenario: Detaching by the attachment's UUID
- **WHEN** `Radio` is attached to the avatar and an object the avatar owns says `@remattach:<uuid>=force` to its owner, with the UUID of the attached object
- **THEN** `Radio` is detached into the inventory

#### Scenario: Detaching by the item's UUID
- **WHEN** `Radio` is attached and `rslbot ctl rlv cmd @remattach:<uuid>=force` is run with the UUID `inventory -a` prints for it
- **THEN** `Radio` is detached

#### Scenario: UUID of nothing that is attached
- **WHEN** an object the avatar owns says `@detach:<uuid>=force` to its owner with a UUID that is neither an attachment's nor an inventory item's
- **THEN** nothing is detached and the operator log gives the reason `detach <uuid>` gives

#### Scenario: HUD point by its RLV name
- **WHEN** `Meter` is attached to `HUD Center` and an object the avatar owns says `@remattach:center=force` to its owner
- **THEN** `Meter` is detached

#### Scenario: Nothing on the point
- **WHEN** nothing is attached to the chest and an object the avatar owns says `@detach:chest=force` to its owner
- **THEN** nothing is sent to the simulator and the operator log says nothing is attached to `Chest`

#### Scenario: Taking off clothing
- **WHEN** the avatar wears a shirt on Second Life and an object the avatar owns says `@remoutfit:shirt=force` to its owner
- **THEN** the avatar no longer wears the shirt

#### Scenario: Body parts stay
- **WHEN** an object the avatar owns says `@remoutfit:skin=force` to its owner
- **THEN** nothing changes and the operator log gives the reason `unwear skin` gives

#### Scenario: Attachment detaches itself
- **WHEN** `Collar` is attached to the avatar and says `@detachme=force` to its owner
- **THEN** `Collar` is detached into the inventory

#### Scenario: Rezzed object asks to be detached
- **WHEN** an object rezzed in the region and owned by the avatar says `@detachme=force` to its owner
- **THEN** nothing is detached and the operator log says the sender is not attached

#### Scenario: Avatar not in a region
- **WHEN** `rslbot ctl rlv cmd @unsit=force` is run while the bot is waiting to log in again
- **THEN** the command exits with status 1 saying the avatar is not connected to a region

#### Scenario: Not the operator's last command
- **WHEN** `rslbot ctl sit <chair>` has been carried out, an object the avatar owns later says `@sit:<bench>=force` to its owner, and `rslbot ctl persist cur last_cmd` is then run
- **THEN** the held sit state is `<chair>`

### Requirement: RLV persist setting
The bot SHALL have an RLV persist setting with the two values `drop` and `follow`, which says what a forced movement does to the placement the bot holds (see "Forced movement and the held placement"). It SHALL start with the value the config gives in the `rlv` key of its `[persist]` table, which is `drop` when the config gives none (see `bot-config`). `rlv persist follow` and `rlv persist drop` SHALL set it and SHALL be answered with success and the text `rlv persist: follow` or `rlv persist: drop`, also when it had that value already. They SHALL work in every state of the bot, connected or not, and whether RLV is on or off; they SHALL NOT change the RLV mode, the placement mode or the held placement. `rlv persist` with anything other than exactly one of `follow` and `drop` SHALL be answered with an error and change nothing. A new value SHALL apply to the forced movements the bot accepts from then on; one that is under way SHALL end by the value it was accepted under. The value SHALL last until the next `rlv persist` command or until the bot process ends, and SHALL NOT be saved.

#### Scenario: Default
- **WHEN** the config has no `[persist]` table and `rslbot ctl rlv status` is run
- **THEN** its second line is `persist: drop`

#### Scenario: From the config
- **WHEN** the config ends with a `[persist]` table containing `rlv = "follow"` and `rslbot ctl rlv status` is run
- **THEN** its second line is `persist: follow`

#### Scenario: Switched while running
- **WHEN** `rslbot ctl rlv persist follow` is run with a config that gives no value
- **THEN** it prints `rlv persist: follow`, exits with status 0, and the next forced movement is followed

#### Scenario: While RLV is off
- **WHEN** `rslbot ctl rlv persist follow` is run while RLV is off
- **THEN** it exits with status 0, RLV stays off, and the setting is `follow` when RLV is switched on later

#### Scenario: Bad arguments
- **WHEN** `rslbot ctl rlv persist` or `rslbot ctl rlv persist keep` is run
- **THEN** the command exits with status 1 naming `follow` and `drop`, and the setting does not change

#### Scenario: Not saved
- **WHEN** `rslbot ctl rlv persist follow` has been carried out with a config that gives no value, and the bot is stopped and started again
- **THEN** the setting is `drop`

### Requirement: Forced movement and the held placement
A forced movement is one of `@sit`, `@sitground`, `@unsit` and `@tpto` with the param `force` that the bot has accepted and begun to carry out. A command that is refused SHALL change neither the placement mode nor the held placement.

While the RLV persist setting is `drop`, every forced movement SHALL put the bot in manual mode before it acts, as the control-socket command it is carried out as does, and nothing more happens: the bot stays in manual mode, and the held placement stays what it was, for `home` to return to.

While the setting is `follow`, the result of a forced movement SHALL become the held placement, through persists carried out exactly as the `persist` command with these arguments would be (see `placement-persist`):

| Forced movement | Persist | When |
|---|---|---|
| `@sit:<uuid>=force` | `persist cur <uuid>` | at once, in place of the `sit <uuid>` |
| | `persist cur cur` | when the simulator has confirmed the avatar seated on the held object and its seated position is known |
| `@sitground=force` | `persist cur ground` | when the ground sit has been handed to the simulator connection |
| `@unsit=force` | `persist cur stand` | when the stand has been handed to the simulator connection |
| `@tpto:...=force` | `persist cur stand` | when the avatar has arrived |

A followed `@sit` SHALL NOT be carried out as a `sit` command and SHALL NOT put the bot in manual mode. Its first persist makes the place the avatar is at and the named object the held placement and puts the bot in holding mode, and the bot then seats the avatar as it does on any held seat (see `presence-reconciler`): when a sit fails it tries again with growing delays, and nothing of the earlier held placement is kept. Besides an option that is not a UUID, a followed `@sit` SHALL be refused when that first persist is refused, with its reason: the avatar is not in a region, or the object is known to be in another region. Its second persist replaces the location with where the avatar sits and the sit state with the prim the simulator named. The bot SHALL wait for the seated position for at most 10 seconds after the sit was confirmed and then carry the second persist out with the position it has, saying so in the log, as it does for the config's setting. A session loss before the avatar is seated SHALL NOT cancel the second persist.

A followed `@sitground`, `@unsit` or `@tpto` SHALL put the bot in manual mode before it acts, so that the bot does not work against it, and its persist puts the bot in holding mode again. When that persist cannot be carried out, the bot SHALL stay in manual mode and note the reason in the operator log. When a followed `@tpto` does not end well, because the teleport fails or the session is lost before the avatar has arrived, the bot SHALL NOT persist anything and SHALL return to the placement mode it was in before the forced movement, with the held placement unchanged.

Every persist of a followed forced movement SHALL be carried out whichever placement mode the bot was in before: a bot that held nothing holds the result from then on. Each counts as a persist for `last`, as one carried out for the config's setting does.

When a `stand`, `unsit`, `sit`, `tp`, `requesttp`, `accepttp`, `persist` or `home` command of the operator is carried out before a followed forced movement has had its last persist, that command's own rules SHALL apply and nothing more SHALL be persisted for the forced movement; what a first persist has made the held placement stays so. The same holds when another forced movement is accepted before then; when the earlier one was a `@tpto`, the mode to return to after a failure SHALL be the one from before that `@tpto`.

The other force commands (`@detach`, `@remattach`, `@remoutfit`, `@detachme`) and the queries SHALL NOT change the placement mode or the held placement under either value.

#### Scenario: Dropped
- **WHEN** the bot holds a chair in `Target` and sits on it, the setting is `drop`, and an object the avatar owns says `@unsit=force` to its owner
- **THEN** the avatar stands, `status` shows `mode: manual` and the chair still on its `persist` line, and `rslbot ctl home` seats the avatar on the chair again

#### Scenario: Following a forced sit
- **WHEN** the bot holds a chair and sits on it, the setting is `follow`, and an object the avatar owns says `@sit:<bench>=force` to its owner
- **THEN** at once `status` shows `mode: holding` with the place the avatar is at and the bench's UUID on its `persist` line, and the bot seats the avatar on the bench; once the seated position is known the `persist` line shows that position and the prim the simulator named; and the bot sits on the bench again when the avatar is unseated

#### Scenario: Seated position not known in time
- **WHEN** a followed `@sit` has seated the avatar and the simulator does not describe the seat within 10 seconds
- **THEN** the second persist is carried out with the position the bot has, and the log says so

#### Scenario: Following a forced stand
- **WHEN** the bot holds a chair and sits on it, the setting is `follow`, and an object the avatar owns says `@unsit=force` to its owner
- **THEN** the avatar stands, the held sit state is `stand`, the bot is in holding mode, and it does not sit the avatar down again

#### Scenario: Following a ground sit
- **WHEN** the setting is `follow` and an object the avatar owns says `@sitground=force` to its owner
- **THEN** the held sit state is `ground`

#### Scenario: Following a forced teleport
- **WHEN** the bot holds a place in `Target`, the setting is `follow`, and an object the avatar owns says `@tpto:Home/100/120/30=force` to its owner
- **THEN** once the avatar has arrived the bot holds where it arrived in `Home` with sit state `stand`, and teleports back there when the avatar is later moved to another region

#### Scenario: Following from manual mode
- **WHEN** the config has no `persist` setting, the bot is in manual mode, the setting has been switched to `follow`, and an object the avatar owns says `@sit:<bench>=force` to its owner
- **THEN** the bot is in holding mode at once, holds the bench, and seats the avatar on it

#### Scenario: Forced sit that fails while following
- **WHEN** the bot holds a chair and sits on it, the setting is `follow`, and an object the avatar owns says `@sit:<uuid>=force` to its owner, naming no object in the region
- **THEN** the bot holds that UUID with the place the avatar was at, no longer holds the chair, and keeps attempting the sit as the backoff allows

#### Scenario: Forced sit on an object elsewhere
- **WHEN** the avatar sat on a bench in `Target` earlier in this run and is now in `Home`, the setting is `follow`, and an object the avatar owns says `@sit:<bench>=force` to its owner
- **THEN** the command is refused with a reason naming `Home` and `Target`, and the placement mode and the held placement are what they were

#### Scenario: Operator steps in before the avatar is seated
- **WHEN** the setting is `follow`, an object the avatar owns says `@sit:<bench>=force` to its owner, and `rslbot ctl stand` is carried out before the avatar is seated
- **THEN** the bot is in manual mode, its `persist` line still shows the bench with the place the avatar was at, and no persist follows when the avatar sits somewhere later

#### Scenario: Session lost before the avatar is seated
- **WHEN** a followed `@sit` has been accepted and the session is lost before the simulator has confirmed the sit
- **THEN** after the next login the bot sits on the held object, and carries out the second persist once the seated position is known

#### Scenario: Forced teleport that fails while following
- **WHEN** the bot is in manual mode, the setting is `follow`, and an object the avatar owns says `@tpto:Nowhere/128/128/25=force` to its owner, the grid having no such region
- **THEN** the bot is in manual mode and its held placement is unchanged

#### Scenario: Operator steps in during a teleport
- **WHEN** the setting is `follow`, an object the avatar owns says `@tpto:Home/100/120/30=force` to its owner, and `rslbot ctl tp 128 128 25 Target` is carried out before the avatar has arrived
- **THEN** the bot is in manual mode after the operator's teleport and the held placement is what it was before both

#### Scenario: Home after following
- **WHEN** a forced sit on a bench has been followed, the operator then ran `rslbot ctl tp 128 128 25 Home`, and runs `rslbot ctl home`
- **THEN** the bot returns to the bench and sits on it

#### Scenario: Detaching changes nothing
- **WHEN** the bot holds a place, the setting is `follow`, and an object the avatar owns says `@detach:left hand=force` to its owner
- **THEN** the placement mode and the held placement are what they were

### Requirement: Commands that are not supported
Every well-formed RLV command that "Version queries" and "Force commands" do not name is not supported. That includes every restriction (the params `n`, `add`, `y` and `rem`, whatever the behaviour), `@clear`, `@notify`, every other query (among them `@getstatus`, `@getstatusall`, `@getsitid`, `@getoutfit`, `@getattach`, `@getinv`, `@versionnumbl` and `@getblacklist`), every other force command (among them `@attach` and the other commands that name inventory folders, and `@setrot`), and these forms of the commands above: `@detach`, `@remattach` and `@remoutfit` without an option, and `@detach` or `@remattach` with an option that is neither the name of an attachment point nor a UUID. For a command that is not supported the bot SHALL send the simulator nothing, say nothing in chat, and change nothing about itself; it SHALL note the command in the operator log. In particular the bot SHALL NOT restrict anything: every control-socket command works as it does while RLV is off.

#### Scenario: Restriction
- **WHEN** an object the avatar owns says `@detach=n` to its owner and the operator then runs `rslbot ctl detach Objects/Radio`
- **THEN** `Radio` is detached, and the operator log has a line saying `detach=n` is not supported

#### Scenario: Unanswered query
- **WHEN** an object the avatar owns says `@getstatus=2222` to its owner
- **THEN** the avatar says nothing on channel 2222

#### Scenario: Detach everything
- **WHEN** an object the avatar owns says `@detach=force` to its owner
- **THEN** every attachment stays on the avatar

#### Scenario: Shared folder
- **WHEN** an object the avatar owns says `@detach:Outfits/Beach=force` to its owner
- **THEN** nothing is detached

### Requirement: Commands wait until the avatar can act
RLV commands that an object sends while the bot is connected to a simulator but the avatar is not yet in the region or is in the middle of a teleport SHALL be kept, in the order they arrived, and dealt with as soon as the avatar is in a region and not teleporting. At most 64 messages SHALL be kept; further ones SHALL be dropped and noted in the operator log. Kept messages SHALL be dropped when the session is lost, when the bot logs out, and when RLV is switched off.

#### Scenario: Version asked during the login
- **WHEN** RLV is on and an attachment says `@versionnew=5` to its owner before the bot has entered the region
- **THEN** the bot answers on channel 5 once it has entered the region

#### Scenario: Order is kept
- **WHEN** two messages with RLV commands arrive during a teleport
- **THEN** after the arrival the commands of the first are dealt with before those of the second

#### Scenario: Session lost while waiting
- **WHEN** RLV commands are kept and the session is lost before the avatar is in a region
- **THEN** they are not carried out after the next login

### Requirement: Running RLV commands by hand
`rlv cmd <command>...` SHALL join its arguments with single spaces and read the result as RLV commands (see "Command syntax"), with or without the leading `@`. The commands SHALL be dealt with at once, exactly as if an object had sent them, with the avatar itself as the sender; they SHALL NOT be kept for later. The command SHALL be answered when every RLV command in it has been dealt with, with one line per RLV command: the command as it was given, `: `, and its outcome. The outcome is the text said for a query, `ok` for a force command the bot has accepted and begun to carry out, and the reason for a command that was malformed, not supported or refused. The answer SHALL be success when every outcome is a reply or `ok`, and an error otherwise. It SHALL NOT wait for the simulator's verdict on a sit, a teleport or a detach; that appears in the operator log. While RLV is off, `rlv cmd` SHALL be answered with an error that says so and SHALL do nothing. `rlv cmd` without a command SHALL be answered with an error.

#### Scenario: Version query by hand
- **WHEN** RLV is on and `rslbot ctl rlv cmd @versionnum=5` is run
- **THEN** the avatar says `10000` on channel 5, and the command prints `versionnum=5: 10000` and exits with status 0

#### Scenario: Without the at-sign
- **WHEN** RLV is on and `rslbot ctl rlv cmd unsit=force` is run while the avatar sits
- **THEN** the avatar stands up, and the command prints `unsit=force: ok`

#### Scenario: Region name of several words
- **WHEN** RLV is on and `rslbot ctl rlv cmd @tpto:Blue Lagoon/128/100/25=force` is run
- **THEN** the avatar is teleported to `Blue Lagoon`, and the command exits with status 0 without waiting for the arrival

#### Scenario: One command fails
- **WHEN** RLV is on and `rslbot ctl rlv cmd @versionnum=5,detach=n` is run
- **THEN** the avatar says `10000` on channel 5, and the command prints both lines on standard error, the second saying `detach=n` is not supported, and exits with status 1

#### Scenario: RLV off
- **WHEN** RLV is off and `rslbot ctl rlv cmd @unsit=force` is run while the avatar sits
- **THEN** the avatar stays seated, and the command exits with status 1 saying RLV is off and that `rlv mode on` switches it on

#### Scenario: Sender needed
- **WHEN** RLV is on and `rslbot ctl rlv cmd @detachme=force` is run
- **THEN** nothing is detached and the command exits with status 1 saying the sender is not attached

### Requirement: RLV appears in the operator log
The bot SHALL write a line to its operator log when it starts with RLV on, for every change of RLV mode, and for every change of the RLV persist setting. For a followed forced movement it SHALL log each persist with the placement that results, or that the earlier placement mode was returned to and why. For every RLV command it deals with it SHALL write a line that names the sender, as the object's name and UUID or as the operator for `rlv cmd`, gives the command, and says what was done: the reply and the channel it was said on, the control-socket command it was carried out as, that it is not supported, or why it was malformed or refused. It SHALL write a line when messages are kept for later, with how many are waiting, and when one is dropped. The outcome of a sit, a teleport or a detach carried out for RLV SHALL be logged as `avatar-commands` and `avatar-outfit` say for those commands. While RLV is off no line about RLV commands SHALL be written.

#### Scenario: Reading the log after a collar attached
- **WHEN** RLV is on and an attachment named `Collar` says `@versionnew=5,detach=n` to its owner
- **THEN** the log has two lines naming `Collar` and its UUID: one with the reply said on channel 5, and one saying `detach=n` is not supported

#### Scenario: Forced teleport
- **WHEN** an object the avatar owns says `@tpto:Home/128/100/25=force` to its owner while the bot holds a place and the RLV persist setting is `drop`
- **THEN** the log shows, in order, the RLV command and its sender, the switch to manual mode, the teleport, and its outcome

#### Scenario: Followed teleport
- **WHEN** the same happens while the RLV persist setting is `follow`
- **THEN** the log shows the same lines and then the persist with the placement the bot now holds

#### Scenario: Off means silent
- **WHEN** RLV is off and an object the avatar owns says `@detach=n` to its owner
- **THEN** the operator log has the chat line as before and nothing else about it
