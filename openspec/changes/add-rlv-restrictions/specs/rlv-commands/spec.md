# Spec Delta

## MODIFIED Requirements

### Requirement: RLV mode
The bot SHALL be in one of three RLV modes: on, override and off. It SHALL start in the mode the config key `rlv` gives, which is on for `true` and off when the key is absent or `false`; it never starts in override. Override is on in every respect but one: the next command of the operator is exempt from the RLV restrictions, and that command puts the bot back in on (see `rlv-restrictions`, "Override exempts one command"). Wherever this capability says that RLV is on, it SHALL hold in override too, unless it says otherwise.

While RLV is off the bot SHALL NOT read any chat line as RLV commands, SHALL NOT answer any RLV query, SHALL send the simulator nothing it would not send without this capability, and SHALL hold itself to no RLV restriction. The restrictions it keeps SHALL stay kept while RLV is off (see `rlv-restrictions`, "When restrictions are in force").

The mode SHALL change only through the `rlv mode` command and, from override to on, through the exempt command. It SHALL last until then or until the bot process ends, across session losses and logins. The mode SHALL NOT be saved: a restarted bot starts from its config.

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

#### Scenario: Override reads commands as on does
- **WHEN** `rslbot ctl rlv mode override` has been carried out and an object the avatar owns says `@versionnew=5` to its owner
- **THEN** the bot answers on channel 5, and RLV is still in override

#### Scenario: Override survives a login
- **WHEN** `rslbot ctl rlv mode override` has been carried out, the avatar is kicked and the bot has logged in again
- **THEN** RLV is still in override

#### Scenario: Off keeps what is stored
- **WHEN** an attachment has said `@sendim=n` to its owner, and `rslbot ctl rlv mode off` and then `rslbot ctl rlv mode on` are run
- **THEN** `rslbot ctl im "Jane Doe" hello` is carried out while RLV is off and refused once it is on again

### Requirement: RLV mode and status commands
The control socket SHALL take the command `rlv` with a subcommand as its first argument. `rlv mode on` SHALL switch RLV on, `rlv mode off` SHALL switch it off, and `rlv mode override` SHALL put it in override; each SHALL be answered with success and the text `rlv: on`, `rlv: off` or `rlv: override`, also when RLV was in that mode already. `rlv mode override` while RLV is off SHALL be answered with an error that says RLV is off, and RLV SHALL stay off. `rlv mode on` while RLV is in override SHALL end the override without a command having been exempt. Override SHALL have no time limit.

`rlv status` SHALL be answered with success and these lines: `rlv: on`, `rlv: override` or `rlv: off` for the mode RLV is in; `persist: drop` or `persist: follow` for the RLV persist setting (see "RLV persist setting"); `persist override: none`, `persist override: sit`, `persist override: tp` or `persist override: sit, tp` for the persist override setting (see "Persist override setting"); and the restrictions the bot keeps (see `rlv-restrictions`), in any mode: the line `restrictions: none` when it keeps none, and otherwise the line `restrictions: <n>` with their number, followed by one line for each in the order the holders first set one and each holder's in the order it set them. Such a line SHALL be the holder, as `operator` or as the object's name and its UUID, then `: ` and the restriction's text, then ` (no effect)` when the restriction is one of those that have no effect in the bot.

`rlv mode`, `rlv status` and `rlv reset` (see `rlv-restrictions`, "Reset command") SHALL work in every state of the bot, connected or not, and SHALL NOT change the bot's placement mode. `rlv` without a subcommand, with a subcommand other than `mode`, `status`, `persist`, `cmd` and `reset`, `rlv mode` with anything other than exactly one of `on`, `off` and `override`, and `rlv status` with further arguments SHALL be answered with an error and change nothing. Switching RLV off SHALL drop the RLV commands that are kept for later (see "Commands wait until the avatar can act") and SHALL drop no restriction.

#### Scenario: Switching on
- **WHEN** `rslbot ctl rlv mode on` is run while RLV is off
- **THEN** it prints `rlv: on`, exits with status 0, and RLV is on

#### Scenario: Switching off
- **WHEN** `rslbot ctl rlv mode off` is run while RLV is on, and an object the avatar owns then says `@sit:<uuid>=force` to its owner
- **THEN** the command prints `rlv: off` and the avatar does not sit

#### Scenario: Already in that mode
- **WHEN** `rslbot ctl rlv mode on` is run while RLV is on
- **THEN** it prints `rlv: on` and exits with status 0

#### Scenario: Override
- **WHEN** `rslbot ctl rlv mode override` is run while RLV is on
- **THEN** it prints `rlv: override`, exits with status 0, and `rlv status` prints `rlv: override` as its first line

#### Scenario: Override while off
- **WHEN** `rslbot ctl rlv mode override` is run while RLV is off
- **THEN** the command exits with status 1 saying RLV is off and that `rlv mode on` switches it on, and RLV is off

#### Scenario: Override taken back
- **WHEN** `rslbot ctl rlv mode override` and then `rslbot ctl rlv mode on` are run, a collar holds `sendchat`, and `rslbot ctl say hello` is run
- **THEN** the `say` is refused

#### Scenario: Status
- **WHEN** `rslbot ctl rlv status` is run with a config that has neither `rlv` nor `persist`, and no `rlv` command before it
- **THEN** it prints `rlv: off`, `persist: drop`, `persist override: none` and `restrictions: none`, and exits with status 0

#### Scenario: Status with restrictions
- **WHEN** an attachment named `Collar` holds `detach` and `showloc`, the operator has set `tploc` with `rlv cmd`, and `rslbot ctl rlv status` is run
- **THEN** it prints `restrictions: 3` and after it the lines `Collar <uuid>: detach`, `Collar <uuid>: showloc (no effect)` and `operator: tploc`, with the collar's UUID

#### Scenario: Status while off
- **WHEN** the same is run after `rslbot ctl rlv mode off`
- **THEN** it prints `rlv: off` and the same restriction lines

#### Scenario: Between logins
- **WHEN** `rslbot ctl rlv mode on` is run while the bot is waiting to log in again
- **THEN** it exits with status 0 and RLV is on when the avatar is back

#### Scenario: Bad arguments
- **WHEN** `rslbot ctl rlv`, `rslbot ctl rlv mode`, `rslbot ctl rlv mode maybe` or `rslbot ctl rlv restrictions` is run
- **THEN** the command exits with status 1 with a text naming what `rlv` takes, and the mode does not change

### Requirement: RLV persist setting
The bot SHALL have an RLV persist setting with the two values `drop` and `follow`, which says what a forced movement does to the placement the bot holds (see "Forced movement and the held placement"). It SHALL start with the value the config gives in the `rlv` key of its `[persist]` table, which is `drop` when the config gives none (see `bot-config`). `rlv persist follow` and `rlv persist drop` SHALL set it and SHALL be answered with success and the text `rlv persist: follow` or `rlv persist: drop`, also when it had that value already. They SHALL work in every state of the bot, connected or not, and whether RLV is on or off; they SHALL NOT change the RLV mode, the placement mode or the held placement. `rlv persist` with anything other than exactly one of `follow` and `drop`, or `override` and what follows it (see "Persist override setting"), SHALL be answered with an error and change nothing. A new value SHALL apply to the forced movements the bot accepts from then on; one that is under way SHALL end by the value it was accepted under. The value SHALL last until the next `rlv persist` command or until the bot process ends, and SHALL NOT be saved.

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
- **THEN** the first command is carried out as a clear of the sender's restrictions (see `rlv-restrictions`, "Clearing a holder's restrictions"), not noted as malformed, and the avatar stands up

#### Scenario: Trailing comma
- **WHEN** an object the avatar owns says `@unsit=force,` to its owner
- **THEN** the avatar stands up and nothing is noted as malformed

#### Scenario: In the order written
- **WHEN** a chair the avatar sits on holds `unsit` and says `@unsit=y,unsit=force,unsit=n` to its owner
- **THEN** the avatar stands up, and the chair holds `unsit` again

### Requirement: Commands that are not supported
Every well-formed RLV command that "Version queries", "Force commands" and `rlv-restrictions` do not name is not supported. `rlv-restrictions` names the restrictions the bot keeps (the params `n`, `add`, `y` and `rem` with one of the behaviours it lists), `@clear`, and the queries `@getstatus` and `@getstatusall`. Not supported are, among others: a restriction with any other behaviour, `@notify` among them; every other query (among them `@getsitid`, `@getoutfit`, `@getattach`, `@getinv`, `@versionnumbl` and `@getblacklist`); every other force command (among them `@attach` and the other commands that name inventory folders, and `@setrot`); and these forms of the commands above: `@detach`, `@remattach` and `@remoutfit` with the param `force` and without an option, and `@detach` or `@remattach` with the param `force` and an option that is neither the name of an attachment point nor a UUID. For a command that is not supported the bot SHALL send the simulator nothing, say nothing in chat, and change nothing about itself, and it SHALL note the command in the operator log. A restriction the bot keeps without an effect (see `rlv-restrictions`, "Restrictions and their holders") is supported: it SHALL NOT be noted as not supported.

#### Scenario: Restriction
- **WHEN** an object the avatar owns says `@notify:5=add` to its owner and the operator then runs `rslbot ctl rlv status`
- **THEN** no restriction is listed, and the operator log has a line saying `notify:5=add` is not supported

#### Scenario: Restriction without an effect
- **WHEN** an object the avatar owns says `@showworldmap=n` to its owner
- **THEN** the operator log has no line calling the command unsupported, and `rlv status` lists `showworldmap` with `(no effect)`

#### Scenario: Unanswered query
- **WHEN** an object the avatar owns says `@getsitid=2222` to its owner
- **THEN** the avatar says nothing on channel 2222

#### Scenario: Detach everything
- **WHEN** an object the avatar owns says `@detach=force` to its owner
- **THEN** every attachment stays on the avatar

#### Scenario: Shared folder
- **WHEN** an object the avatar owns says `@detach:Outfits/Beach=force` to its owner
- **THEN** nothing is detached

### Requirement: Running RLV commands by hand
`rlv cmd <command>...` SHALL join its arguments with single spaces and read the result as RLV commands (see "Command syntax"), with or without the leading `@`. The commands SHALL be dealt with at once, exactly as if an object had sent them, except for who sent them: the sender is the operator, with the avatar's own UUID where a command needs the sender's, and a holder of restrictions apart from every object (see `rlv-restrictions`, "Restrictions and their holders"). A restriction given with `rlv cmd` is the operator's, `rlv cmd @clear` clears the operator's restrictions and no object's, and `rlv cmd @getstatus=<channel>` reports the operator's. The commands SHALL NOT be kept for later.

The command SHALL be answered when every RLV command in it has been dealt with, with one line per RLV command: the command as it was given, `: `, and its outcome. The outcome is the text said for a query, `ok` for a force command the bot has accepted and begun to carry out and for a restriction or a `clear` that was carried out, and the reason for a command that was malformed, not supported or refused. The answer SHALL be success when every outcome is a reply or `ok`, and an error otherwise. It SHALL NOT wait for the simulator's verdict on a sit, a teleport or a detach; that appears in the operator log. While RLV is off, `rlv cmd` SHALL be answered with an error that says so and SHALL do nothing; while RLV is in override it SHALL be carried out as while RLV is on and SHALL leave RLV in override. `rlv cmd` without a command SHALL be answered with an error.

#### Scenario: Version query by hand
- **WHEN** RLV is on and `rslbot ctl rlv cmd @versionnum=5` is run
- **THEN** the avatar says `10000` on channel 5, and the command prints `versionnum=5: 10000` and exits with status 0

#### Scenario: Without the at-sign
- **WHEN** RLV is on and `rslbot ctl rlv cmd unsit=force` is run while the avatar sits
- **THEN** the avatar stands up, and the command prints `unsit=force: ok`

#### Scenario: Region name of several words
- **WHEN** RLV is on and `rslbot ctl rlv cmd @tpto:Blue Lagoon/128/100/25=force` is run
- **THEN** the avatar is teleported to `Blue Lagoon`, and the command exits with status 0 without waiting for the arrival

#### Scenario: Restriction by hand
- **WHEN** RLV is on and `rslbot ctl rlv cmd @sendim=n` is run
- **THEN** the command prints `sendim=n: ok` and exits with status 0, `rlv status` lists `operator: sendim`, and `rslbot ctl im "Jane Doe" hello` is refused

#### Scenario: Clear by hand
- **WHEN** a collar holds `detach`, the operator has set `sendim` and `tploc` with `rlv cmd`, and `rslbot ctl rlv cmd @clear` is run
- **THEN** the command prints `clear: ok`, the operator holds nothing, and the collar still holds `detach`

#### Scenario: An object's clear
- **WHEN** the operator has set `tploc` with `rlv cmd` and an object the avatar owns says `@clear` to its owner
- **THEN** the operator still holds `tploc`

#### Scenario: One command fails
- **WHEN** RLV is on and `rslbot ctl rlv cmd @versionnum=5,notify:5=add` is run
- **THEN** the avatar says `10000` on channel 5, and the command prints both lines on standard error, the second saying `notify:5=add` is not supported, and exits with status 1

#### Scenario: RLV off
- **WHEN** RLV is off and `rslbot ctl rlv cmd @unsit=force` is run while the avatar sits
- **THEN** the avatar stays seated, and the command exits with status 1 saying RLV is off and that `rlv mode on` switches it on

#### Scenario: Sender needed
- **WHEN** RLV is on and `rslbot ctl rlv cmd @detachme=force` is run
- **THEN** nothing is detached and the command exits with status 1 saying the sender is not attached

### Requirement: RLV appears in the operator log
The bot SHALL write a line to its operator log when it starts with RLV on, for every change of RLV mode, the return from override to on with the command that was exempt among them, for every change of the RLV persist setting and of the persist override setting, and when it starts with a persist override setting that is not empty. For a followed forced movement it SHALL log each persist with the placement that results, or that the earlier placement mode was returned to and why. For every RLV command it deals with it SHALL write a line that names the sender, as the object's name and UUID or as the operator for `rlv cmd`, gives the command, and says what was done: the reply and the channel it was said on, the control-socket command it was carried out as, that a restriction was set or lifted or was so already, how many restrictions a `clear` lifted, that it is not supported, or why it was malformed or refused, with the restriction and its holder when a restriction refused it. It SHALL write a line when messages are kept for later, with how many are waiting, and when one is dropped. The outcome of a sit, a teleport or a detach carried out for RLV SHALL be logged as `avatar-commands` and `avatar-outfit` say for those commands.

About restrictions the bot SHALL also write a line: when the restrictions of an object are dropped without a command, naming the object, how many there were and why (the object is gone, the avatar left its region, the session ended); for `rlv reset`, with how many were cleared; when a control-socket command is refused by a restriction, naming the command, the restriction and its holder; when a command is carried out with another volume, on other channels or with a shortened text because of a restriction; when an offer or a request is dropped or accepted because of a restriction; and when a step of the bot's own is held back by a restriction and when it no longer is (see `presence-reconciler`).

While RLV is off no line about RLV commands SHALL be written, and none about commands or steps being refused or held back; lines about restrictions being dropped and about `rlv reset` SHALL be written in every mode.

#### Scenario: Reading the log after a collar attached
- **WHEN** RLV is on and an attachment named `Collar` says `@versionnew=5,detach=n` to its owner
- **THEN** the log has two lines naming `Collar` and its UUID: one with the reply said on channel 5, and one saying the restriction `detach` was set

#### Scenario: Forced teleport
- **WHEN** an object the avatar owns says `@tpto:Home/128/100/25=force` to its owner while the bot holds a place and the RLV persist setting is `drop`
- **THEN** the log shows, in order, the RLV command and its sender, the switch to manual mode, the teleport, and its outcome

#### Scenario: Followed teleport
- **WHEN** the same happens while the RLV persist setting is `follow`
- **THEN** the log shows the same lines and then the persist with the placement the bot now holds

#### Scenario: Refused operator command
- **WHEN** `Collar` holds `sendim` and `rslbot ctl im "Jane Doe" hello` is run
- **THEN** the log has a line saying the `im` was refused because of `sendim`, held by `Collar` with its UUID

#### Scenario: Restrictions dropped
- **WHEN** `Collar` holds `detach` and `sendim` and is detached by its own script
- **THEN** the log has a line saying two restrictions of `Collar` were dropped because the object is gone

#### Scenario: Override used
- **WHEN** `rslbot ctl rlv mode override` and then `rslbot ctl stand` are run
- **THEN** the log has a line for the switch to override and a line saying that `stand` was exempt and RLV is on again

#### Scenario: Off means silent
- **WHEN** RLV is off and an object the avatar owns says `@detach=n` to its owner
- **THEN** the operator log has the chat line as before and nothing else about it

## ADDED Requirements

### Requirement: Persist override setting
The bot SHALL have a persist override setting: a set of the two words `sit` and `tp`, possibly empty, which names the sorts of the bot's own steps towards the held placement that RLV restrictions do not hold back (see `presence-reconciler`, "Steps that RLV restrictions forbid are held back"). It SHALL start with what the config gives in the `override` key of its `[persist]` table, which is empty when the config gives none (see `bot-config`).

`rlv persist override none` SHALL empty the setting. `rlv persist override` followed by one or both of `sit` and `tp`, in either order, SHALL set it to the words given; a word given twice SHALL count once. Each SHALL be answered with success and the text `rlv persist override: none`, `rlv persist override: sit`, `rlv persist override: tp` or `rlv persist override: sit, tp`, also when the setting had that value already. They SHALL work in every state of the bot, connected or not, and in every RLV mode; they SHALL NOT change the RLV mode, the RLV persist setting, the placement mode or the held placement. `rlv persist override` without a word, with a word other than `none`, `sit` and `tp`, and with `none` beside another word SHALL be answered with an error and change nothing.

A new value SHALL apply to the steps the bot starts from then on, and a step that is held back SHALL be looked at again at once. The value SHALL last until the next `rlv persist override` command or until the bot process ends, across session losses and logins, and SHALL NOT be saved.

#### Scenario: Default
- **WHEN** the config has no `[persist]` table and `rslbot ctl rlv status` is run
- **THEN** its third line is `persist override: none`

#### Scenario: From the config
- **WHEN** the config ends with a `[persist]` table containing `override = ["tp", "sit"]` and `rslbot ctl rlv status` is run
- **THEN** its third line is `persist override: sit, tp`

#### Scenario: Switched while running
- **WHEN** `rslbot ctl rlv persist override tp` is run with a config that gives no value
- **THEN** it prints `rlv persist override: tp`, exits with status 0, and the bot's next teleport towards the held location is not held back by `tploc`

#### Scenario: Both words
- **WHEN** `rslbot ctl rlv persist override tp sit` is run
- **THEN** it prints `rlv persist override: sit, tp`

#### Scenario: Emptied
- **WHEN** the config gives `override = "tp"` and `rslbot ctl rlv persist override none` is run
- **THEN** it prints `rlv persist override: none`, and a teleport towards the held location that `tploc` forbids is held back from then on

#### Scenario: While RLV is off
- **WHEN** `rslbot ctl rlv persist override sit` is run while RLV is off
- **THEN** it exits with status 0, RLV stays off, and the setting is `sit` when RLV is switched on later

#### Scenario: The other settings stay
- **WHEN** the RLV persist setting is `follow` and `rslbot ctl rlv persist override tp` is run
- **THEN** `rlv status` prints `persist: follow` and `persist override: tp`

#### Scenario: Bad arguments
- **WHEN** `rslbot ctl rlv persist override`, `rslbot ctl rlv persist override stand` or `rslbot ctl rlv persist override none tp` is run
- **THEN** the command exits with status 1 naming `none`, `sit` and `tp`, and the setting does not change

#### Scenario: Not saved
- **WHEN** `rslbot ctl rlv persist override tp` has been carried out with a config that gives no value, and the bot is stopped and started again
- **THEN** the setting is empty
