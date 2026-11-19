# Spec Delta

## ADDED Requirements

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

## MODIFIED Requirements

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
