# avatar-outfit Specification

## Purpose

Lets the operator change what the avatar carries and wears through the control socket: attach objects from the inventory, wear clothing and body parts, and take either off again, by item or by body part.

## Requirements

### Requirement: Outfit commands need the avatar in a region
The commands `attach`, `wear`, `detach` and `unwear` SHALL be answered with an error, and SHALL send nothing and change nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout. A command SHALL be carried out at once or not at all; it SHALL NOT be kept for later. None of these commands SHALL change the bot's placement mode or move the avatar, whether it is carried out or refused.

#### Scenario: Command between logins
- **WHEN** `rslbot ctl attach Objects/Radio` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Attaching while seated in holding mode
- **WHEN** `rslbot ctl attach Objects/Radio` is carried out while the avatar is seated on the held object
- **THEN** the avatar stays seated and `rslbot ctl status` still shows `mode: holding`

### Requirement: Knowing what is attached
The bot SHALL know which inventory items are attached to its avatar, and to which attachment point each is, from what the simulator sends about the objects on the avatar. It SHALL NOT send the simulator anything for the sake of learning this. What it knows SHALL follow the simulator: an attachment that appears or disappears without a command of the bot, for instance through a script or a viewer logged in to the same account earlier, SHALL be known or forgotten the same way. What it knows SHALL be forgotten when the session is lost and learned anew from the next simulator.

#### Scenario: Attached before the bot started
- **WHEN** the avatar was left with `Radio` on its left hand by a viewer and the bot then logs in
- **THEN** `rslbot ctl inventory -a` lists `Radio` with `Left Hand` and `attached`, without any attach command having been given

#### Scenario: Detached by a script
- **WHEN** an attachment's script detaches it from the avatar
- **THEN** the next `rslbot ctl inventory -a` does not list it

#### Scenario: No extra traffic
- **WHEN** the avatar wears three attachments and no command is given
- **THEN** the bot sends the simulator nothing it would not send for an avatar without attachments

#### Scenario: After a teleport to another region
- **WHEN** the avatar teleports to another region with `Radio` attached
- **THEN** `rslbot ctl inventory -a` in the new region lists `Radio` once the new simulator has shown it on the avatar

### Requirement: Body-part names
An attachment point SHALL be named by the name viewers give it, with `HUD ` in front for the points of the heads-up display, or by its number from 1 to 127. Names SHALL be compared without regard to case. The names, by number, are: 1 `Chest`, 2 `Skull`, 3 `Left Shoulder`, 4 `Right Shoulder`, 5 `Left Hand`, 6 `Right Hand`, 7 `Left Foot`, 8 `Right Foot`, 9 `Spine`, 10 `Pelvis`, 11 `Mouth`, 12 `Chin`, 13 `Left Ear`, 14 `Right Ear`, 15 `Left Eyeball`, 16 `Right Eyeball`, 17 `Nose`, 18 `R Upper Arm`, 19 `R Forearm`, 20 `L Upper Arm`, 21 `L Forearm`, 22 `Right Hip`, 23 `R Upper Leg`, 24 `R Lower Leg`, 25 `Left Hip`, 26 `L Upper Leg`, 27 `L Lower Leg`, 28 `Stomach`, 29 `Left Pec`, 30 `Right Pec`, 31 `HUD Center 2`, 32 `HUD Top Right`, 33 `HUD Top`, 34 `HUD Top Left`, 35 `HUD Center`, 36 `HUD Bottom Left`, 37 `HUD Bottom`, 38 `HUD Bottom Right`, 39 `Neck`, 40 `Avatar Center`, 41 `Left Ring Finger`, 42 `Right Ring Finger`, 43 `Tail Base`, 44 `Tail Tip`, 45 `Left Wing`, 46 `Right Wing`, 47 `Jaw`, 48 `Alt Left Ear`, 49 `Alt Right Ear`, 50 `Alt Left Eye`, 51 `Alt Right Eye`, 52 `Tongue`, 53 `Groin`, 54 `Left Hind Foot`, 55 `Right Hind Foot`. A point the bot has no name for SHALL be printed as its number.

A kind of clothing or body part SHALL be named `shape`, `skin`, `hair` or `eyes` (the body parts) or `shirt`, `pants`, `shoes`, `socks`, `jacket`, `gloves`, `undershirt`, `underpants`, `skirt`, `alpha`, `tattoo`, `physics` or `universal` (clothing), compared without regard to case.

Where a command takes a body part after the item, the arguments after the item SHALL be joined with single spaces to form the name.

#### Scenario: Name of several words
- **WHEN** `rslbot ctl attach Objects/Radio left hand` is run
- **THEN** `Radio` is attached to the left hand

#### Scenario: By number
- **WHEN** `rslbot ctl attach Objects/Radio 5` is run
- **THEN** `Radio` is attached to the left hand

#### Scenario: HUD point
- **WHEN** `rslbot ctl attach Objects/Meter HUD Center` is run
- **THEN** `Meter` is attached to the centre point of the heads-up display, not to any point named `Center` on the body

#### Scenario: Unknown name
- **WHEN** `rslbot ctl attach Objects/Radio elbow` is run
- **THEN** the command exits with status 1 saying `elbow` is not an attachment point, and the grid is asked nothing

### Requirement: Attach command
`attach <item> [<body part>]` SHALL attach the object `<item>` names (see `inventory-list`, "Naming an inventory item or folder") to the avatar: to the named attachment point when a body part is given, and otherwise to the point the inventory records for the item, which is the one it was last attached to, leaving the choice to the simulator when it records none. Nothing already attached SHALL be taken off, on that point or any other. The command SHALL be answered with success, naming the item and the point it is attached to, when the simulator shows the object on the avatar, and with an error when it has not done so within 30 seconds of the request.

The command SHALL be answered with an error and send the simulator nothing when the body part is not an attachment point, when the item cannot be found, when the item is not an object, in which case the error names its type, and when the item is attached already, in which case the error names the point it is on.

#### Scenario: To a named point
- **WHEN** `rslbot ctl attach Objects/Radio Left Hand` is run and `Radio` is an object that is not attached
- **THEN** other viewers see `Radio` on the avatar's left hand, the command prints the item and `Left Hand`, and exits with status 0

#### Scenario: To the point it was on before
- **WHEN** `rslbot ctl attach Objects/Hat` is run and `Hat` was last attached to the skull
- **THEN** `Hat` is attached to the skull and the command prints `Skull`

#### Scenario: Point already in use
- **WHEN** `Watch` is attached to the left hand and `rslbot ctl attach Objects/Radio Left Hand` is run
- **THEN** both `Watch` and `Radio` are attached to the left hand

#### Scenario: Not an object
- **WHEN** `rslbot ctl attach "Animations/Wave"` names an animation
- **THEN** the command exits with status 1 saying the item is an animation, and nothing is sent to the simulator

#### Scenario: Already attached
- **WHEN** `Radio` is attached to the left hand and `rslbot ctl attach Objects/Radio Chest` is run
- **THEN** the command exits with status 1 saying `Radio` is attached to `Left Hand`, and it stays there

#### Scenario: Simulator does not attach it
- **WHEN** `rslbot ctl attach Objects/Radio` is run and the simulator does not show `Radio` on the avatar
- **THEN** the command exits with status 1 within 31 seconds of the request saying the simulator did not attach it, and the avatar stays connected

#### Scenario: Simulator is slow
- **WHEN** `rslbot ctl attach Objects/Radio` is run and the simulator shows `Radio` on the avatar 21 seconds later
- **THEN** the command prints the item and its point and exits with status 0

### Requirement: Wear command
`wear <item>` SHALL put the item on the avatar according to its type. For an object it SHALL do, and be answered, exactly as `attach <item>` without a body part. For clothing it SHALL add the item to what the avatar wears, taking nothing off. For a body part (shape, skin, hair, eyes) it SHALL replace the body part of that kind the avatar wears. Clothing and body parts are worn as "Clothing through the grid's outfit service" says. The command SHALL be answered with an error and change nothing when the item cannot be found, when it is of any other type, in which case the error names the type, and when it is clothing or a body part the avatar wears already.

#### Scenario: Wearing an object
- **WHEN** `rslbot ctl wear Objects/Hat` is run and `Hat` was last attached to the skull
- **THEN** `Hat` is attached to the skull, as with `rslbot ctl attach Objects/Hat`

#### Scenario: Wearing a shirt
- **WHEN** `rslbot ctl wear "Clothing/Blue shirt"` is run on Second Life while the avatar wears a white shirt
- **THEN** other viewers see the avatar with the blue shirt over the white one, and the command exits with status 0 naming the item and `shirt`

#### Scenario: Wearing another skin
- **WHEN** `rslbot ctl wear "Body Parts/Tanned skin"` is run on Second Life
- **THEN** the avatar wears `Tanned skin` and no longer the skin it wore before

#### Scenario: Something that cannot be worn
- **WHEN** `rslbot ctl wear "Notecards/Readme"` is run
- **THEN** the command exits with status 1 saying the item is a notecard

### Requirement: Detach command
`detach <what>`, also available under the name `unwear`, SHALL take `<what>` off the avatar. `<what>` is one argument, or for a body part of several words several arguments joined with single spaces, read in this order: as an inventory item's UUID when it has the form of one; as an attachment point or a kind of clothing when it is the name or number of one (see "Body-part names"); and as an inventory path otherwise.

- An item that is attached SHALL be detached into the inventory.
- An attachment point SHALL have everything attached to it detached.
- An item that is clothing the avatar wears SHALL be taken off, and a kind of clothing SHALL have every piece of that kind the avatar wears taken off, as "Clothing through the grid's outfit service" says.

A detach SHALL be answered with success, naming what was detached, when the simulator shows the objects gone from the avatar, and with an error when it has not done so within 15 seconds of the request. The command SHALL be answered with an error and change nothing when the item cannot be found; when the item is neither attached nor worn; when nothing is attached to the named point, or nothing of the named kind is worn; and when `<what>` is a body part item or one of the kinds `shape`, `skin`, `hair` and `eyes`, since the avatar cannot be without them.

#### Scenario: Detaching an item
- **WHEN** `Radio` is attached and `rslbot ctl detach Objects/Radio` is run
- **THEN** other viewers no longer see `Radio` on the avatar, the item is in the inventory where it was, and the command exits with status 0

#### Scenario: Alias
- **WHEN** `rslbot ctl unwear Objects/Radio` is run
- **THEN** the effect is that of `rslbot ctl detach Objects/Radio`

#### Scenario: Everything on a point
- **WHEN** `Watch` and `Radio` are attached to the left hand, `Hat` to the skull, and `rslbot ctl detach left hand` is run
- **THEN** `Watch` and `Radio` are detached, `Hat` stays, and the command names both detached items

#### Scenario: Nothing on the point
- **WHEN** `rslbot ctl detach Chest` is run and nothing is attached to the chest
- **THEN** the command exits with status 1 saying nothing is attached to `Chest`, and nothing is sent to the simulator

#### Scenario: Not attached
- **WHEN** `rslbot ctl detach Objects/Hat` is run and `Hat` is an object that is not attached
- **THEN** the command exits with status 1 saying `Hat` is not attached

#### Scenario: Taking off a kind of clothing
- **WHEN** the avatar wears two shirts on Second Life and `rslbot ctl unwear shirt` is run
- **THEN** the avatar wears neither, its other clothing is unchanged, and the command names both items

#### Scenario: Body parts stay
- **WHEN** `rslbot ctl unwear skin` is run
- **THEN** the command exits with status 1 saying a skin can only be replaced, with `wear`

### Requirement: Clothing through the grid's outfit service
Clothing and body parts SHALL be worn and taken off by changing the avatar's Current Outfit folder and asking the grid to make the avatar's appearance from it. A `wear` or `unwear` of clothing or a body part SHALL be answered with success when the grid has accepted the request for the new appearance, and with an error giving the grid's reason when it refuses the change or the request; when the request is refused after the folder was changed, the bot SHALL put the folder back as it was, and SHALL say so in the error when that fails too. When the simulator the avatar is on does not offer this service, as on OpenSimulator, `wear` and `unwear` of clothing and body parts SHALL be answered with an error saying that this grid makes the avatar's appearance in the viewer, which the bot cannot do, and SHALL change nothing. Attached objects are not affected by that limit.

#### Scenario: Grid without the service
- **WHEN** `rslbot ctl wear "Clothing/Blue shirt"` is run on an OpenSimulator grid
- **THEN** the command exits with status 1 with that reason, and nothing in the inventory changes

#### Scenario: Objects still work there
- **WHEN** `rslbot ctl wear Objects/Hat` is run on the same grid
- **THEN** `Hat` is attached

#### Scenario: Grid refuses the outfit
- **WHEN** `rslbot ctl wear "Clothing/Blue shirt"` is run on Second Life and the grid refuses to add the item to the outfit
- **THEN** the command exits with status 1 with the grid's reason, and the avatar's appearance is unchanged

### Requirement: Current Outfit folder follows attachments
Where the simulator offers the outfit service, the bot SHALL add a link to an item to the avatar's Current Outfit folder when an `attach` or `wear` of the bot has attached it, and SHALL remove the links to an item from that folder when a `detach` or `unwear` of the bot has detached it. When the simulator shows an object on the avatar that an `attach` or `wear` of the bot asked for in the same session and that the command has already been answered as not attached, the bot SHALL add the link all the same and note the late arrival in the operator log. A link that is already there SHALL NOT be added again. A failure to change the folder SHALL be noted in the operator log and SHALL NOT change the command's answer. Where the service is not offered the bot SHALL leave the folder alone.

#### Scenario: A viewer logs in afterwards
- **WHEN** the bot has attached `Radio` on Second Life, is stopped, and the operator logs in to the account with a viewer
- **THEN** the viewer shows `Radio` as worn and does not take it off

#### Scenario: Folder cannot be changed
- **WHEN** `rslbot ctl attach Objects/Radio` attaches `Radio` and the grid refuses the change to the Current Outfit folder
- **THEN** the command still exits with status 0 and the log has a line about the folder

#### Scenario: Attached after the command gave up
- **WHEN** `rslbot ctl attach Objects/Radio` has exited with status 1 because the simulator did not attach `Radio` in time, and the simulator shows `Radio` on the avatar ten seconds later
- **THEN** the Current Outfit folder gets a link to `Radio`, and the log has a line saying `Radio` was attached late

### Requirement: Restoring attachments after a login
While the config option `restore_attachments` is on, which it is unless the config says otherwise, the bot SHALL make sure after every login that the objects listed in the avatar's Current Outfit folder are attached. Once per login, the first time the avatar has been in one region for 20 seconds after that login, the bot SHALL read the links in the Current Outfit folder and SHALL ask the simulator to attach every object a link points to that the simulator does not show on the avatar at that moment: each to the point the inventory records for it, without taking anything off. A change of region before the comparison is made SHALL start the 20 seconds anew, since the new simulator has yet to show what is attached.

The bot SHALL do this only where the simulator offers the outfit service; elsewhere, and with `restore_attachments = false`, it SHALL do nothing of it and ask the grid nothing. It SHALL NOT take off anything, whether the folder lists it or not. It SHALL leave alone links to anything but objects and links whose item the inventory does not have. It SHALL NOT add a link for an object it attaches this way. It SHALL NOT repeat the comparison in the same login: not after a teleport, and not when an object it asked for does not arrive.

The bot SHALL write to its operator log one line for every object it asks the simulator to attach this way, naming the item, one line when the simulator shows it on the avatar, naming the point, and one line when the simulator has not shown it within 30 seconds. When the folder cannot be read it SHALL write one line with the reason and try again only after the next login. A restore SHALL NOT delay the bot's simulator traffic, SHALL NOT change the bot's placement mode, and SHALL NOT keep a command given meanwhile from being carried out.

#### Scenario: An attachment the simulator dropped
- **WHEN** the Current Outfit folder has links to `Radio` and `Hat`, the simulator shows only `Hat` on the avatar after a login, and 20 seconds have passed since the avatar entered the region
- **THEN** the bot asks the simulator to attach `Radio`, the log names `Radio` as being restored, and once the simulator shows it `rslbot ctl inventory -a` lists `Radio` and `Hat`

#### Scenario: Nothing is missing
- **WHEN** everything the Current Outfit folder lists is on the avatar 20 seconds after the region was entered
- **THEN** the bot sends the simulator no attach request

#### Scenario: Worn but not listed
- **WHEN** the simulator shows `Watch` on the avatar and the Current Outfit folder has no link to it
- **THEN** `Watch` stays attached and the folder is left as it is

#### Scenario: Switched off
- **WHEN** the config has `restore_attachments = false` and the simulator does not show an object the Current Outfit folder lists
- **THEN** the bot asks the grid nothing about the folder and attaches nothing

#### Scenario: Grid without the outfit service
- **WHEN** the bot logs in to an OpenSimulator grid whose simulator offers no outfit service
- **THEN** the bot asks the grid nothing about the folder and attaches nothing

#### Scenario: Clothing links
- **WHEN** the Current Outfit folder has links to a shirt, a skin and `Radio`, and `Radio` is on the avatar
- **THEN** the bot sends the simulator no attach request

#### Scenario: Broken link
- **WHEN** the Current Outfit folder has a link to an object that is no longer in the inventory
- **THEN** the bot asks the simulator nothing for that link and restores the others

#### Scenario: Simulator does not bring it
- **WHEN** the bot has asked the simulator to attach `Radio` this way and the simulator does not show it within 30 seconds
- **THEN** the log has a line saying so, the bot does not ask again in this login, and the avatar stays connected

#### Scenario: After a teleport
- **WHEN** attachments were compared after a login and the avatar then teleports to another region
- **THEN** the bot does not read the Current Outfit folder again

#### Scenario: Logged in again after losing the session
- **WHEN** the session is lost, the bot logs in again, and 20 seconds have passed in the region
- **THEN** the bot compares the folder with the avatar again

#### Scenario: Teleported before the comparison
- **WHEN** the avatar teleports to another region ten seconds after the login
- **THEN** the bot reads the Current Outfit folder 20 seconds after it has entered that region, and not before

#### Scenario: Command during the wait
- **WHEN** `rslbot ctl attach Objects/Radio` is run ten seconds after the login and `Radio` is one of the objects the simulator did not bring
- **THEN** the command attaches `Radio` and is answered as usual, and the later comparison does not ask for `Radio` again

### Requirement: Outfit commands appear in the operator log
The bot SHALL write a line to its operator log for every command of this capability it carries out, saying what was asked, for every one it refuses, with the reason, and for the outcome of every attach, detach and change of clothing. It SHALL also log every attachment that appears on or disappears from the avatar without a command of the bot, naming the inventory item's UUID and the point.

#### Scenario: Reading the log after a change
- **WHEN** the operator has run `attach Objects/Radio` and `detach Left Hand`
- **THEN** the log shows, in order, the attach request, that `Radio` is attached and where, the detach request, and that `Radio` is detached
