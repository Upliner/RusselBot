# avatar-scan Specification

## Purpose

Lets the operator see which other avatars are in the avatar's region through the control socket, with how far each is and where, so that it can tell who can hear the avatar and whom to address, without logging in with a viewer.

## Requirements

### Requirement: Scan avatars command
`scan avatars [<distance>]` SHALL be answered with success and a text of one line for each avatar other than the bot's own that the simulator says is in the avatar's region, or with success and no text when there is none. Without `<distance>` every such avatar SHALL be listed, however far it is; with it, only those whose distance from the avatar is known and is no more than it.

#### Scenario: Everyone in the region
- **WHEN** `scan_distance` is not configured, one avatar stands 5 metres and another 180 metres from the bot's avatar in the same region, and `rslbot ctl scan avatars` is run
- **THEN** the output has one line for each of the two, the nearer first, and the command exits with status 0

#### Scenario: Alone in the region
- **WHEN** `rslbot ctl scan avatars` is run and the bot's avatar is the only one in the region
- **THEN** the command exits with status 0 and prints nothing

#### Scenario: Avatars only
- **WHEN** another avatar wearing a hat stands beside a bench and `rslbot ctl scan avatars` is run
- **THEN** the output has one line, for the avatar, and none for the hat or the bench

#### Scenario: Avatar in the region next door
- **WHEN** an avatar stands 10 metres from the bot's avatar, across the border in the neighbouring region
- **THEN** `rslbot ctl scan avatars` has no line for it

### Requirement: Where an avatar is
The position of an avatar SHALL be the one the simulator gave when it described the avatar or reported that it moved, for as long as the bot has it; a described avatar seated on a described object SHALL count as being where that object's root prim is. Otherwise the position SHALL be the one in the simulator's list of the avatars in the region.

#### Scenario: Avatar the simulator has described
- **WHEN** the bot's avatar stands at 100, 100, 25, the simulator has described an avatar standing at 103.24, 100, 25, and `rslbot ctl scan avatars` is run
- **THEN** that avatar's line has the distance `3.2`

#### Scenario: Avatar known by the list only
- **WHEN** the bot's avatar stands at 100, 100, 25, the simulator has not described an avatar standing at 200.6, 40.2, 27.9, and its list of the avatars in the region gives that avatar as 200, 40, 24
- **THEN** that avatar's line has the distance `116.6` and the position `200,40,24`

#### Scenario: Seated avatar
- **WHEN** the simulator has described an avatar seated on a bench whose root prim is 4 metres from the bot's avatar
- **THEN** that avatar's line has the distance `4.0`

### Requirement: Positions from the simulator's list
A position from the simulator's list of the avatars in the region SHALL be used as the list gives it: in whole metres east and north and in steps of 4 metres in height. The height of an avatar known by that list only SHALL be unknown when the list gives it as 0 or as 1020 metres, which is how simulators write a height the list cannot hold.

#### Scenario: Avatar on a sky platform
- **WHEN** an avatar stands on a platform 2000 metres up, the bot's avatar stands on the ground, and the simulator has not described that avatar
- **THEN** the height of that avatar is unknown to `scan avatars`

#### Scenario: Described avatar on a sky platform
- **WHEN** the bot's avatar and another avatar stand 6 metres apart on a platform 2000 metres up, and the simulator has described that avatar
- **THEN** that avatar's line has the distance `6.0` and its height in its position

### Requirement: Distance of an avatar
The distance of an avatar SHALL be the distance in three dimensions between the bot's avatar's current position (see `region-teleport`, "Know the current position") and that avatar's position, and SHALL be unknown when that avatar's height is.

#### Scenario: Avatar above the bot
- **WHEN** the bot's avatar stands at 100, 100, 25, the simulator has described an avatar hovering at 100, 100, 55, and `rslbot ctl scan avatars chat` is run
- **THEN** that avatar is not listed, and `rslbot ctl scan avatars 40` lists it with the distance `30.0`

#### Scenario: Measured from the seat
- **WHEN** the bot's avatar is seated on a chair 50 metres from where it stood before, and another avatar stands 3 metres from that chair
- **THEN** that avatar's distance is measured from where the bot's avatar is seated

### Requirement: Distance filter of scan avatars
`<distance>` SHALL be a number of metres from 1 to 1024, or one of the words `shout`, `chat` and `whisper`, written in lower case, which stand for 100, 20 and 10 metres. An avatar whose distance is unknown SHALL be listed only when no `<distance>` is given.

#### Scenario: Who hears local chat
- **WHEN** avatars stand 8, 15 and 60 metres from the bot's avatar and `rslbot ctl scan avatars chat` is run
- **THEN** the avatars at 8 and 15 metres are listed and the one at 60 metres is not

#### Scenario: Whisper and shout
- **WHEN** avatars stand 8, 15 and 60 metres from the bot's avatar
- **THEN** `rslbot ctl scan avatars whisper` lists the one at 8 metres only, and `rslbot ctl scan avatars shout` lists all three

#### Scenario: Any number of metres
- **WHEN** avatars stand 8, 15 and 60 metres from the bot's avatar and `rslbot ctl scan avatars 12.5` is run
- **THEN** only the avatar at 8 metres is listed

#### Scenario: Unknown distance and a filter
- **WHEN** the height of an avatar in the region is unknown
- **THEN** `rslbot ctl scan avatars` lists it, and `rslbot ctl scan avatars 1024` does not

### Requirement: Avatar listing format
Each line SHALL consist of five fields separated by tab characters: the avatar's UUID; its distance in metres with one decimal; its position in the region as `<x>,<y>,<z>`, rounded to whole metres; its name, `First Last`; its display name. There SHALL be no header line.

#### Scenario: A line of the listing
- **WHEN** Jane Doe, whose display name is `JD`, is described as standing at 103.24, 100, 25, the bot's avatar stands at 100, 100, 25, and `rslbot ctl scan avatars` is run
- **THEN** her line is her UUID, `3.2`, `103,100,25`, `Jane Doe` and `JD`, separated by tabs

### Requirement: Unknown and escaped fields of the avatar listing
An unknown distance, an unknown height, and a name or a display name the bot does not know SHALL each be written as `?`. A tab, line feed, carriage return or backslash inside a name or a display name SHALL be written as `\t`, `\n`, `\r` and `\\`.

#### Scenario: Unknown height
- **WHEN** an avatar whose height is unknown is at 128 metres east and 64 metres north in the region
- **THEN** its line has `?` as the second field and `128,64,?` as the third

#### Scenario: Name the name service does not give
- **WHEN** the name service does not name an avatar in the region, or the simulator offers no name service
- **THEN** that avatar's line has its UUID, its distance and its position, and `?` as the fourth and as the fifth field

#### Scenario: Tab inside a display name
- **WHEN** an avatar's display name contains a tab character
- **THEN** its line has five fields, with `\t` in the display name

### Requirement: Avatar listing order
The lines SHALL be ordered by distance, nearest first, with the avatars whose distance is unknown after all others, and by UUID where that does not decide.

#### Scenario: Nearest first
- **WHEN** avatars 60, 8 and 15 metres from the bot's avatar are listed
- **THEN** their lines come in the order 8, 15, 60, and the first line of the output is an avatar's

#### Scenario: Unknown distance last
- **WHEN** one avatar is 200 metres away and the height of another is unknown
- **THEN** the line with `?` as its distance is the last

### Requirement: Scan avatars needs the avatar in a region
`scan avatars` SHALL be answered with an error and SHALL list nothing when the avatar is not connected to a region or is in the middle of a teleport or a logout. A `scan avatars` that is waiting when the avatar leaves the region or the session is lost SHALL be answered with an error that says so, and SHALL list nothing.

#### Scenario: Between logins
- **WHEN** `rslbot ctl scan avatars` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Teleported away while waiting
- **WHEN** `rslbot ctl scan avatars` is waiting for names and the avatar is teleported to another region
- **THEN** the command exits with status 1 saying the avatar left the region

### Requirement: Scan avatars reads only
`scan avatars` SHALL NOT move the avatar, make it sit or stand, or change the bot's placement mode or the held placement. It SHALL NOT send the simulator anything, SHALL NOT start the object scan and SHALL NOT change its distance (see `avatar-commands`, "Object scan").

#### Scenario: No object scan for avatars
- **WHEN** `scan_distance` is not configured, no command has started the object scan in the region, and `rslbot ctl scan avatars shout` is run
- **THEN** it is answered, the bot has sent the simulator nothing for it, the simulator is still asked to describe what is within 32 metres only, and a later `rslbot ctl scan` is the first to start the object scan

#### Scenario: While seated and holding
- **WHEN** `rslbot ctl scan avatars` is run in holding mode while the avatar is seated on the held object
- **THEN** the avatar stays seated and `status` still prints `mode: holding`

### Requirement: Scan avatars under RLV restrictions
`scan avatars` SHALL be held to the RLV restrictions that `rlv-restrictions` names for it ("Avatars hidden from the scans", "The nearby list" and "Positions under showminimap") and to no other; a vision limit SHALL NOT shorten its listing. It SHALL be the exempt command of an RLV override as any other command is (see `rlv-restrictions`, "Override exempts one command").

#### Scenario: Hidden names
- **WHEN** a collar holds `shownames`, RLV is on, and `rslbot ctl scan avatars` is run in a region with two other avatars
- **THEN** it prints two lines with their distances and positions, and `?` as UUID, name and display name

#### Scenario: Nearby list closed
- **WHEN** a collar holds `shownearby`, RLV is on, and `rslbot ctl scan avatars` is run
- **THEN** the command exits with status 1 naming `shownearby`

#### Scenario: During an override
- **WHEN** a collar holds `shownearby`, `rslbot ctl rlv mode override` has been carried out, and `rslbot ctl scan avatars` is run
- **THEN** the avatars are listed as with no restriction kept, and `rlv status` then prints `rlv: on`

#### Scenario: Restrictions that are not about avatars
- **WHEN** a collar holds `camdrawmax:10`, `showloc` and `sendim`, RLV is on, and `rslbot ctl scan avatars` is run
- **THEN** every avatar in the region is listed as without them

### Requirement: Arguments scan avatars refuses
`scan avatars` SHALL be answered with an error, and SHALL list nothing, when more than one argument follows `avatars`, when the argument starts with `-`, or when it is neither a number from 1 to 1024 nor one of the words `shout`, `chat` and `whisper`.

#### Scenario: Filter of the object scan
- **WHEN** `rslbot ctl scan avatars -n jane` is run
- **THEN** the command exits with status 1 naming `-n`

#### Scenario: Distance out of bounds
- **WHEN** `rslbot ctl scan avatars 0`, `rslbot ctl scan avatars 2000` or `rslbot ctl scan avatars near` is run
- **THEN** the command exits with status 1 saying the distance is a number of metres from 1 to 1024 or `shout`, `chat` or `whisper`

#### Scenario: Two distances
- **WHEN** `rslbot ctl scan avatars chat 30` is run
- **THEN** the command exits with status 1

### Requirement: When scan avatars answers
The bot SHALL answer a `scan avatars` at once when the simulator has told it who is in the region since the avatar entered it and the names the listing needs are known. Otherwise it SHALL wait for both, and SHALL answer no later than 10 seconds after the command was given with what it knows by then: the avatars the simulator has described when it has not said who is in the region, and `?` for the names it lacks.

#### Scenario: Second listing
- **WHEN** `rslbot ctl scan avatars` has been answered and is run again while nobody has come to the region
- **THEN** the second is answered within a second

#### Scenario: Just arrived
- **WHEN** `rslbot ctl scan avatars` is run at the moment the avatar arrives in a region where three other avatars are
- **THEN** the command waits for the simulator's list and lists the three, within 11 seconds

#### Scenario: Name service that does not answer
- **WHEN** the name service does not answer and `rslbot ctl scan avatars` is run in a region with two other avatars
- **THEN** the command exits with status 0 within 11 seconds, with two lines that have `?` as name and as display name

### Requirement: Names of the avatars listed
The bot SHALL ask for names through the name service the message log uses, only for the avatars a `scan avatars` lists with their names and only for those whose names it does not have. Without a `scan avatars` command the bot SHALL ask for the name of no avatar on account of its being in the region.

#### Scenario: Only who is listed
- **WHEN** forty avatars are in the region, two of them within 20 metres, and `rslbot ctl scan avatars chat` is the first `scan avatars` in the region
- **THEN** the name service is asked for those two avatars and for no other

#### Scenario: Names are asked for once
- **WHEN** `rslbot ctl scan avatars` has been answered with every name and is run again while nobody has come to the region
- **THEN** the name service is asked nothing the second time

#### Scenario: No command, no lookups
- **WHEN** the bot stays in a busy region for an hour and no `scan avatars` is given
- **THEN** the name service is asked for nobody on account of the avatars in the region

### Requirement: Avatar scans appear in the operator log
The bot SHALL write a line to its operator log for every `scan avatars` it answers, with its arguments and the number of lines listed, and for every `scan avatars` it refuses or fails, with the reason. A `scan avatars` the page of the web interface sent by itself (see `web-server`, "Requests the page makes by itself") SHALL get no line, whatever its answer.

#### Scenario: Reading the log after a scan
- **WHEN** `rslbot ctl scan avatars chat` has listed two avatars
- **THEN** the log shows a line for the command with `chat` and the count 2

#### Scenario: A page left open
- **WHEN** a page of the web interface shows its Local chat tab for an hour and refreshes its list of avatars by itself all the while
- **THEN** the log has no line for any of those scans
