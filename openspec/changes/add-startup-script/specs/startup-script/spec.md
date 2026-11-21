# Spec Delta

## Purpose

Defines the startup script: commands from the config that the bot carries out by itself when it starts. It says where the script comes from, how it is written, which commands run before the first login and which wait until the avatar is in place, and what a refused command does.

## ADDED Requirements

### Requirement: Script source
The startup script SHALL be the text of the config key `startup`, or the contents of the file the config key `startup_file` names (see `bot-config`). The bot SHALL read it once, when it starts. A bot whose config has neither key SHALL carry out no script.

#### Scenario: Script in the config
- **WHEN** the config contains `startup` with the two lines `rlv mode on` and `persist`
- **THEN** the bot carries out those two commands by itself, as this capability says

#### Scenario: Script in a file
- **WHEN** the config contains `startup_file = "startup.rsl"` and that file holds the same two lines
- **THEN** the bot behaves as with those lines under `startup`

#### Scenario: File changed while the bot runs
- **WHEN** the file `startup_file` names is changed after the bot has started
- **THEN** nothing happens until the bot is stopped and started again

#### Scenario: No script
- **WHEN** the config has neither `startup` nor `startup_file`
- **THEN** the bot carries out no command by itself

### Requirement: Script lines
The script SHALL be read as commands, one per line. A line end inside double quotes SHALL belong to the argument it is in, and the command goes on in the next line. A carriage return in front of a line feed SHALL be ignored. A blank line, and a line whose first character after spaces and tabs is `#`, SHALL be skipped, unless it is inside double quotes. A script without commands is valid and carries out nothing.

#### Scenario: Comments and blank lines
- **WHEN** the script is a `# settings` line, the line `rlv mode on`, an empty line and the line `persist`
- **THEN** the bot carries out `rlv mode on` and `persist`, and nothing else

#### Scenario: Number sign inside a command
- **WHEN** the script has the line `say #1 is here`
- **THEN** the avatar says `#1 is here` in local chat

#### Scenario: Argument of two lines
- **WHEN** the script has `autoreply message "I am a bot.` on one line and `Nobody reads this."` on the next
- **THEN** the autoreply text is those two sentences with a line end between them, and the script has one command

#### Scenario: Only comments
- **WHEN** the script holds nothing but `#` lines
- **THEN** the bot starts and carries out no command by itself

### Requirement: Arguments of a command
Each command SHALL be split into its name and arguments at spaces and tabs. A part between double quotes SHALL be one argument and keep its spaces and line ends; inside it `\"` SHALL stand for a double quote and `\\` for a backslash. The arguments SHALL then be passed on as `rslbot ctl` passes on its own, so that the words of a message are joined with single spaces.

#### Scenario: Name of two words
- **WHEN** the script has the line `perms autoaccept tp "Jane Doe"`
- **THEN** the rule for `tp` holds the one entry `Jane Doe`

#### Scenario: Words of a message
- **WHEN** the script has the line `autoreply message back in  an hour`
- **THEN** the autoreply text is `back in an hour`

#### Scenario: Quote inside an argument
- **WHEN** the script has the line `say "she said \"hi\" and left"`
- **THEN** the avatar says `she said "hi" and left` in local chat

### Requirement: Commands the script takes
The script SHALL take every command the control socket takes, written without `rslbot ctl` in front, and no other. A command longer than 8192 bytes, or one that ends inside double quotes, is not in its form; `bot-config` says what the bot does with a script that has a command it does not take or one that is not in its form.

#### Scenario: Any command of the control socket
- **WHEN** the script has the lines `anim play "Animations/Wave"` and `scan 20 -s`
- **THEN** both are carried out as `rslbot ctl` with the same arguments would be

#### Scenario: Help is no command
- **WHEN** the script has the line `help`
- **THEN** the bot rejects its config as `bot-config` says, naming the line and `help` as an unknown command

#### Scenario: Written as for a shell
- **WHEN** the script has the line `rslbot ctl rlv mode on`
- **THEN** the bot rejects its config, naming the line and `rslbot` as an unknown command

### Requirement: Script commands are the operator's
A script command SHALL be carried out as the control socket carries out the same command, with the same effects, and SHALL count as a command of the operator: RLV restrictions apply to it and `rlv mode override` exempts it as they do the operator's, and it changes the placement mode as the operator's does. The script SHALL need no other way of sending commands to be enabled: it SHALL be carried out with and without the control socket.

#### Scenario: No socket
- **WHEN** the config has a script and no `socket = true`
- **THEN** the script is carried out all the same

#### Scenario: Command that moves the avatar
- **WHEN** the script's last command is `tp 128 128 25 Home` and it is carried out
- **THEN** the avatar is in `Home`, the bot is in manual mode, and it does not teleport the avatar back by itself

#### Scenario: Restricted command
- **WHEN** an attachment has said `@sendchat=n` to its owner before the script's command `say hello` is due, and RLV is on
- **THEN** the command is refused as `rslbot ctl say hello` would be

### Requirement: Order of the commands
The commands SHALL be carried out in the order the script has them, one at a time: a command SHALL start only when the one before it has been answered. The script SHALL be carried out at most once in a run of the bot; it SHALL NOT start again, or go on, because of a session loss or a later login.

#### Scenario: Teleport, then sit
- **WHEN** the script has `tp 128 128 25 Home` followed by `sit <uuid>` with an object in `Home`
- **THEN** the sit request is sent only after the avatar has arrived in `Home`, and the avatar ends up seated on the object

#### Scenario: Not again after a kick
- **WHEN** the script has been carried out to its end, the avatar is kicked and the bot logs in again
- **THEN** no script command is carried out after that login

### Requirement: Setting commands run before the first login
The setting commands are every `perms` command, every `autoreply` command, and every `rlv` command other than `rlv cmd`. The setting commands at the head of the script, those in front of its first command that is not a setting command, SHALL be carried out when the bot starts, before it contacts any server and so before the first login. One of them that is refused SHALL stop the bot as a config error (see `bot-config`).

#### Scenario: RLV on from the start
- **WHEN** the script starts with `rlv mode on`, and an attachment says `@sendim=n` to its owner in the second the avatar arrives
- **THEN** the bot keeps that restriction

#### Scenario: Rules there at the first login
- **WHEN** the script starts with `perms autoaccept tp "Jane Doe"`, the config has `sit` with an object that does not exist, and Jane Doe offers the bot a teleport after the login
- **THEN** the bot accepts the offer by itself, although the config placement was never reached

#### Scenario: Autoreply from the start
- **WHEN** the script starts with `autoreply message Away` and `autoreply pause 2h`, and an avatar sends an instant message right after the login
- **THEN** the bot answers `Away`

#### Scenario: Setting command further down
- **WHEN** the script has `say hello` followed by `rlv mode on`
- **THEN** RLV is off during the login, and is switched on once `say hello` has been answered

#### Scenario: Refused at the head
- **WHEN** the script starts with `perms autoaccept money all`
- **THEN** the bot prints an error and exits with status 2 without contacting the login server

### Requirement: The rest waits for the config placement
The rest of the script, from its first command that is not a setting command on, SHALL start the first time the config placement is reached (see `placement-persist`, "Config persist setting applied after startup"), right after the bot has applied the config's `persist` setting or switched to manual mode there. When the avatar has just been seated on an object for the config placement, the rest SHALL NOT start before the seated position is known or 10 seconds have passed.

#### Scenario: Persist after sitting
- **WHEN** the config has `region`, `position` and `sit` with the UUID of an object, and the script is the one line `persist`
- **THEN** after sitting the bot holds the avatar's seated position and the prim the simulator seated it on, and `status` shows both on its `persist` line

#### Scenario: Persist of the config placement
- **WHEN** the same config has the script `persist cfg cfg`
- **THEN** the bot holds the configured `position` and `sit`, before and after the command is carried out

#### Scenario: Account alone
- **WHEN** the config names neither `region` nor `sit` and the script is the one line `say hello`
- **THEN** the avatar says `hello` as soon as it is connected to the region the login put it in

#### Scenario: Nothing left to run
- **WHEN** the config has `region`, `position` and `sit`, no `persist` key, and a script of setting commands only
- **THEN** the bot sits on the object once and then is in manual mode

#### Scenario: Session lost on the way
- **WHEN** the avatar is kicked before it has sat on the configured object
- **THEN** the bot logs in again and sits, and the rest of the script starts then

#### Scenario: Seat never reached
- **WHEN** the configured object does not exist
- **THEN** the bot keeps attempting the sit as the backoff allows, and the rest of the script is not carried out

### Requirement: The operator acts first
The rest of the script SHALL NOT be carried out when the config's `persist` setting is not applied (see `placement-persist`): when, before the config placement was reached, a `persist` or `home` command or a command that puts the bot in manual mode was carried out. The bot SHALL then write to the operator log that the rest is not carried out, and how many commands that is. Once the rest has started, a command of the operator's SHALL NOT stop it, however it reaches the bot.

#### Scenario: Teleported away before sitting
- **WHEN** the operator runs `rslbot ctl tp 128 128 25 Home` before the bot has sat on the configured object, and the script's rest is `say hello` and `persist`
- **THEN** neither command is carried out in this run, and the log says that two commands of the startup script are not carried out

#### Scenario: Settings stay
- **WHEN** the same happens with a script that starts with `rlv mode on`
- **THEN** RLV is on

#### Scenario: Operator command while the rest runs
- **WHEN** the script's `tp 128 128 25 Home` is on its way and the operator runs `rslbot ctl status`
- **THEN** `status` is answered, and the script goes on with its next command once the teleport has ended

### Requirement: A refused command stops the script
When a command of the rest is answered with an error, the bot SHALL NOT carry out the commands after it, and SHALL keep running. A command that gets no answer SHALL count as refused. The script SHALL NOT wait for the avatar to be connected again: a command that is due while the avatar is not in a region is refused as the control socket refuses it. A `quit` command SHALL stop the bot as it does from the control socket.

#### Scenario: Sit that fails
- **WHEN** the script's rest is `sit <uuid>` with an object that does not exist, followed by `persist`
- **THEN** `persist` is not carried out, the bot is in manual mode, and it stays connected

#### Scenario: Refused for its arguments
- **WHEN** the script's rest is `sit chair` followed by `say hello`
- **THEN** the bot starts, `sit chair` is refused when it is due, and the avatar does not say `hello`

#### Scenario: Kicked in the middle
- **WHEN** the avatar is kicked while the script's `tp 128 128 25 Home` is on its way, and `sit ground` follows it
- **THEN** `sit ground` is not carried out, neither when the teleport has been answered with an error nor after the next login, and the bot logs in again as its placement mode says

#### Scenario: Quit at the end
- **WHEN** the script's rest is `say done` followed by `quit`
- **THEN** the avatar says `done`, and the bot logs out and exits with the status `rslbot ctl quit` gives it

### Requirement: The script appears in the operator log
For every script command it carries out, the bot SHALL write to the operator log a line starting with `startup:` that gives the line the command starts on and the command, and then the command's answer or the reason it was refused. When the script stops before its end, the log SHALL name the line that stopped it and how many commands were not carried out. Chat and instant messages a script command sends SHALL be recorded as those of the same command from the control socket are.

#### Scenario: Command and answer
- **WHEN** the script's line 4 is `persist cfg cfg` and it is carried out
- **THEN** the log has a `startup:` line naming line 4 and `persist cfg cfg`, and one with the placement the command answered with

#### Scenario: Stopped early
- **WHEN** the command in line 6 is refused and three commands follow it
- **THEN** the log has a `startup:` line with line 6 and the reason, saying that three commands are not carried out

#### Scenario: Instant message from the script
- **WHEN** the script's command `im "Jane Doe" I am back` is carried out and `log_im` is configured
- **THEN** the message is in the IM file as one sent with `rslbot ctl im` would be
