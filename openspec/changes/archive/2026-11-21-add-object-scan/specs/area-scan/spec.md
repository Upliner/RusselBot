# Spec Delta

## Purpose

Lets the operator see which objects are around the avatar through the control socket, with what is needed to pick one for `touch`, `sit` or `persist`: its UUID, how far it is, what it is called and whose it is.

## ADDED Requirements

### Requirement: Scan command
`scan [<distance>] [<filter>...]` SHALL be answered with success and a text of one line for each object within the distance of the avatar that the simulator has described to the bot, or with success and no text when there is none. Unless a switch says otherwise (see "Attachments and child prims"), the objects listed SHALL be the root prims of objects in the world: no avatar, no tree or grass, nothing an avatar wears, and no child prim of a linkset.

#### Scenario: Objects around the avatar
- **WHEN** a bench stands 3 metres and a lamp 10 metres from the avatar and `rslbot ctl scan` is run
- **THEN** the output has one line for the bench and one for the lamp, the bench first, and the command exits with status 0

#### Scenario: Nothing nearby
- **WHEN** `rslbot ctl scan 5` is run and no object is within 5 metres of the avatar
- **THEN** the command exits with status 0 and prints nothing

#### Scenario: A linkset is one object
- **WHEN** a chair made of four linked prims stands beside the avatar and `rslbot ctl scan` is run
- **THEN** the output has one line for the chair, with the UUID of its root prim

#### Scenario: Avatars, plants and what avatars wear
- **WHEN** another avatar wearing a hat stands beside a tree, both within the distance, and `rslbot ctl scan` is run
- **THEN** the output has no line for the avatar, the hat or the tree

### Requirement: Scan needs the avatar in a region
`scan` SHALL be answered with an error and SHALL send nothing when the avatar is not connected to a region or is in the middle of a teleport or a logout. A `scan` that is waiting when the avatar leaves the region or the session is lost SHALL be answered with an error that says so, and SHALL list nothing.

#### Scenario: Scan between logins
- **WHEN** `rslbot ctl scan` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Teleported away during a scan
- **WHEN** `rslbot ctl scan 96` is waiting for the simulator and the avatar is teleported to another region
- **THEN** the command exits with status 1 saying the avatar left the region

### Requirement: Scan reads only
`scan` SHALL NOT move the avatar, make it sit or stand, or change the bot's placement mode or the held placement. No RLV restriction SHALL be applied to `scan`, and `scan` SHALL NOT be the exempt command of an RLV override (see `rlv-restrictions`, "Override exempts one command").

#### Scenario: Scan while seated and holding
- **WHEN** `rslbot ctl scan` is run in holding mode while the avatar is seated on the held object
- **THEN** the avatar stays seated, `status` still prints `mode: holding`, and the bot sends no sit or teleport request

#### Scenario: Scan during an override
- **WHEN** `rslbot ctl rlv mode override` has been carried out and `rslbot ctl scan` is run
- **THEN** the scan is answered and `rlv status` still prints `rlv: override`

### Requirement: Scan distance
`<distance>` SHALL be a number of metres from 1 to 1024. Without it the distance SHALL be that of the object scan in the region (see `avatar-commands`, "Object scan"). When `<distance>` is more than that, the object scan's distance SHALL be raised to it before the listing is made, and SHALL stay raised until the avatar leaves the region. A smaller `<distance>` SHALL only shorten the listing and SHALL NOT lower the object scan's distance.

#### Scenario: Default distance
- **WHEN** `scan_distance` is not configured, no `scan` has named a distance in the region, and `rslbot ctl scan` is run
- **THEN** objects up to 32 metres from the avatar are listed and an object 40 metres away is not

#### Scenario: Further than the object scan reaches
- **WHEN** `scan_distance` is not configured and `rslbot ctl scan 96` is run
- **THEN** the bot asks the simulator to describe objects within 96 metres, and an object 80 metres away is listed

#### Scenario: Raised distance stays
- **WHEN** `rslbot ctl scan 96` has been answered and `rslbot ctl touch <uuid>` then names an object 80 metres away
- **THEN** the touch is answered without a scan wait, and a later `rslbot ctl scan` without a distance lists objects up to 96 metres away

#### Scenario: Shorter than the object scan reaches
- **WHEN** `scan_distance = 64` is configured and `rslbot ctl scan 10` is run
- **THEN** only objects within 10 metres are listed, and the bot still asks the simulator for objects within 64 metres

#### Scenario: New region
- **WHEN** `rslbot ctl scan 96` has been answered and the avatar teleports to another region
- **THEN** the object scan's distance in the new region is `scan_distance`, or 32 metres without it

### Requirement: Where an object is
The distance of an object SHALL be the distance in three dimensions between the avatar's current position (see `region-teleport`, "Know the current position") and the position of the object's root prim. A child prim SHALL count as being where its linkset's root prim is, and what an avatar wears as being where that avatar is. A prim whose position the simulator has not told the bot SHALL NOT be listed.

#### Scenario: Object above the avatar
- **WHEN** the avatar stands at 100, 100, 25, a platform's root prim is at 100, 100, 55, and `rslbot ctl scan 20` is run
- **THEN** the platform is not listed, and `rslbot ctl scan 40` lists it with the distance `30.0`

#### Scenario: Seated avatar
- **WHEN** the avatar is seated on a chair 50 metres from where it stood before and `rslbot ctl scan 5` is run
- **THEN** the distances are measured from where the avatar is seated, and the chair is listed

### Requirement: Listing format
Each line SHALL consist of seven fields separated by tab characters: the prim's UUID; its distance in metres with one decimal; its flags (see "Flags in the listing"); its owner's UUID, which for an object owned by a group is the group's; its owner's name, `First Last` for an avatar and `(group)` for a group; its name; its description, which may be empty. A field from the fourth on that the bot does not know SHALL be `?`.

#### Scenario: A line of the listing
- **WHEN** Jane Doe's object `Bench`, described as `Sit here`, is 3.24 metres from the avatar, holds a script that handles touches, and `rslbot ctl scan` is run
- **THEN** its line is its UUID, `3.2`, `scripted,touch`, Jane Doe's UUID, `Jane Doe`, `Bench` and `Sit here`, separated by tabs

#### Scenario: Object owned by a group
- **WHEN** an object deeded to a group is within the distance
- **THEN** its line has the group's UUID as the fourth field and `(group)` as the fifth

#### Scenario: No description
- **WHEN** an object without a description is listed
- **THEN** its line ends with a tab after the name

#### Scenario: Owner the name service does not know
- **WHEN** the name service does not name an object's owner, or the simulator offers no name service
- **THEN** the object's line has the owner's UUID as the fourth field and `?` as the fifth

### Requirement: Listing order and escaping
The lines SHALL be ordered by distance, nearest first, and by UUID where distances are equal. There SHALL be no header line. A tab, line feed, carriage return or backslash inside a name or description SHALL be written as `\t`, `\n`, `\r` and `\\`.

#### Scenario: Nearest first
- **WHEN** objects 12, 3 and 7 metres from the avatar are listed
- **THEN** their lines come in the order 3, 7, 12, and the first line of the output is an object's

#### Scenario: Tab inside a name
- **WHEN** an object's name contains a tab character
- **THEN** its line has seven fields, with `\t` in the name

### Requirement: Flags in the listing
The flags field SHALL be made of these words, in this order and separated by commas, or `-` when none applies: `scripted`, `touch`, `physical`, `temporary` and `phantom`, each when the simulator's description of the listed prim itself marks it so (`touch` when it marks the prim as handling touches); `attachment` for a prim an avatar wears; `child` for a prim that is not the root of its linkset.

#### Scenario: Plain object
- **WHEN** an unscripted, non-physical object rezzed for good is listed
- **THEN** its flags field is `-`

#### Scenario: Flags are the prim's own
- **WHEN** a linkset's only script is in a child prim and the simulator marks only that prim as scripted
- **THEN** the root prim's line does not have `scripted`, and with `-c` the child prim's line has `scripted,child`

#### Scenario: Script added later
- **WHEN** a script is dropped into an object the bot has listed as `-` and `rslbot ctl scan` is run again
- **THEN** its flags field has `scripted`

### Requirement: Text filters
`-n <text>` (also spelled `--name <text>`) SHALL list only prims whose name contains `<text>`, and `-d <text>` (also spelled `--desc <text>`) only prims whose description contains `<text>`, in both cases whatever the case of the letters. A prim whose name and description the bot does not know SHALL match neither.

#### Scenario: Part of a name
- **WHEN** objects named `Park Bench`, `Workbench` and `Lamp` are within the distance and `rslbot ctl scan -n bench` is run
- **THEN** the lines for `Park Bench` and `Workbench` are printed and the one for `Lamp` is not

#### Scenario: Name of several words
- **WHEN** `rslbot ctl scan --name "park bench"` is run
- **THEN** `Park Bench` is listed

#### Scenario: Description
- **WHEN** `rslbot ctl scan -d teleport` is run and one object nearby is described as `Teleporter to the shop`
- **THEN** only that object is listed

### Requirement: Owner filter
`-o <who>` (also spelled `--owner <who>`) SHALL list only prims owned by `<who>`: an avatar given as `im` takes one (`First Last`, `first.last`, a single word or a UUID), or a group given by its UUID. A name SHALL be compared with the owners' names as the name service gives them, whatever the case of the letters, and the grid SHALL NOT be asked to search for it. A prim whose owner the bot does not know, or whose owner's name it does not know when a name is given, SHALL NOT match.

#### Scenario: Owner by name
- **WHEN** objects of Jane Doe and of John Roe are within the distance and `rslbot ctl scan -o "jane doe"` is run
- **THEN** only Jane Doe's objects are listed

#### Scenario: Owner by UUID
- **WHEN** `rslbot ctl scan -o <uuid>` names Jane Doe's UUID
- **THEN** only Jane Doe's objects are listed, also when the name service is not available

#### Scenario: Group's objects
- **WHEN** `rslbot ctl scan -o <uuid>` names a group's UUID
- **THEN** only the objects owned by that group are listed

#### Scenario: Nobody of that name owns anything here
- **WHEN** `rslbot ctl scan -o "Nobody Here"` is run
- **THEN** the command exits with status 0 and prints nothing

### Requirement: Flag filters
`-s` (`--scripted`), `-t` (`--touch`), `-p` (`--physical`), `--temporary` and `--phantom` SHALL each list only prims whose flags field would have that word.

#### Scenario: Things to touch
- **WHEN** `rslbot ctl scan -t` is run and of the objects nearby only a button's root prim handles touches
- **THEN** only the button is listed

#### Scenario: Two flags
- **WHEN** `rslbot ctl scan -s -p` is run
- **THEN** only prims that are both scripted and physical are listed

### Requirement: Attachments and child prims
With `-a` (also spelled `--attachments`) the listing SHALL also have what avatars wear, the bot's avatar included, as far as the simulator has described it. With `-c` (also spelled `--children`) it SHALL also have the child prims of the linksets it lists, each with its own name and description. With both, the child prims of what avatars wear are listed too.

#### Scenario: What avatars wear
- **WHEN** another avatar 4 metres away wears a hat and `rslbot ctl scan -a` is run
- **THEN** the hat's root prim is listed with the distance `4.0` and `attachment` in its flags

#### Scenario: The bot's own attachments
- **WHEN** the bot's avatar wears a radio and `rslbot ctl scan -a` is run
- **THEN** the radio is listed with the distance `0.0`

#### Scenario: Child prims
- **WHEN** a chair's child prim is named `Cushion` and `rslbot ctl scan -c -n cushion` is run
- **THEN** the cushion's line is printed, with the child prim's UUID, the chair's distance and `child` in its flags

#### Scenario: Child prims are not searched without the switch
- **WHEN** a chair's child prim is named `Cushion`, no root prim nearby has `cushion` in its name, and `rslbot ctl scan -n cushion` is run
- **THEN** nothing is printed

### Requirement: Combining and refusing arguments
The distance and the filters MAY be given in any order, and every filter given SHALL have to be met for a prim to be listed. `scan` SHALL be answered with an error, and SHALL send and change nothing, when an argument starts with `-` and is not one of the filters, when `-n`, `-d` or `-o` is the last argument, when `<who>` is not something `im` would take, when more than one argument is left over as the distance, or when the distance is not a number from 1 to 1024.

#### Scenario: Several filters
- **WHEN** `rslbot ctl scan -s 20 -o "Jane Doe" -n door` is run
- **THEN** only scripted prims of Jane Doe within 20 metres with `door` in their name are listed

#### Scenario: Unknown option
- **WHEN** `rslbot ctl scan -x` is run
- **THEN** the command exits with status 1 naming `-x`, and the simulator is asked nothing

#### Scenario: Filter without its text
- **WHEN** `rslbot ctl scan -n` is run
- **THEN** the command exits with status 1 saying `-n` takes a text

#### Scenario: Distance out of bounds
- **WHEN** `rslbot ctl scan 0`, `rslbot ctl scan 2000` or `rslbot ctl scan far` is run
- **THEN** the command exits with status 1 saying the distance is a number of metres from 1 to 1024, and the object scan's distance does not change

#### Scenario: Two distances
- **WHEN** `rslbot ctl scan 10 20` is run
- **THEN** the command exits with status 1

### Requirement: Waiting for the simulator
The bot SHALL answer a `scan` once the simulator has described no object new to the bot for 3 seconds, counted at the earliest from the start of the object scan in the region and from the last raise of its distance, and the properties and owners' names the listing needs are known. It SHALL answer no later than 30 seconds after the command was given, listing what it knows by then.

#### Scenario: First scan in a region
- **WHEN** `scan_distance` is not configured and `rslbot ctl scan` is the first thing to start the object scan in the region
- **THEN** the command is answered no sooner than 3 seconds and no later than 30 seconds after it was given

#### Scenario: Scan after the region has settled
- **WHEN** `scan_distance = 64` is configured, the avatar has been in the region for a minute and `rslbot ctl scan` has been answered once
- **THEN** a second `rslbot ctl scan` is answered within a second

#### Scenario: Simulator that does not answer for an object
- **WHEN** the simulator gives no properties for one of the objects within the distance and `rslbot ctl scan` is run without a filter
- **THEN** the command exits with status 0 within 31 seconds, with `?` as that object's owner, owner's name, name and description

### Requirement: What a scan asks for
For the sake of a `scan` the bot SHALL ask the simulator for the properties only of prims that are within the distance and meet the flag filters and switches of that command, and only of those whose properties it does not have. Without a `scan` command the bot SHALL NOT ask for the properties of objects, the seat excepted (see `avatar-commands`, "Status command"). It SHALL ask for owners' names through the name service the message log uses.

#### Scenario: Only what the command reaches
- **WHEN** `scan_distance = 256` is configured and `rslbot ctl scan 10` is run
- **THEN** the bot asks for the properties of the objects within 10 metres and of no other

#### Scenario: No scan command
- **WHEN** `scan_distance = 64` is configured and no `scan` command is given
- **THEN** the bot asks the simulator for the properties of no object but the one the avatar is seated on

#### Scenario: Owners' names
- **WHEN** the objects listed belong to two avatars
- **THEN** the name service is asked for those two avatars and for no other

### Requirement: Properties are kept for the stay
The bot SHALL keep the properties the simulator gave it for an object until the object is gone, the avatar leaves the region or the session is lost, and SHALL NOT ask for them again in that time.

#### Scenario: Second scan of the same objects
- **WHEN** `rslbot ctl scan` has been answered and is run again while nothing around the avatar has changed
- **THEN** the bot asks the simulator for no properties the second time

#### Scenario: Back in the region
- **WHEN** the avatar has teleported to another region and back and `rslbot ctl scan` is run
- **THEN** the bot asks for the properties of the objects it lists anew

#### Scenario: Object renamed after a scan
- **WHEN** an object is renamed after `rslbot ctl scan` listed it, and `rslbot ctl scan` is run again in the same stay in the region
- **THEN** the listing MAY still show the old name, and shows the new one after the avatar has left the region and come back

### Requirement: Scans appear in the operator log
The bot SHALL write a line to its operator log for every `scan` it answers, with its arguments and the number of lines listed, for every `scan` it refuses or fails, with the reason, and for every raise of the object scan's distance, with the new distance.

#### Scenario: Reading the log after a scan
- **WHEN** `rslbot ctl scan 96 -n bench` has listed two objects in a region scanned to 32 metres before
- **THEN** the log shows the raise of the scan distance to 96 metres and a line for the scan with its arguments and the count 2
