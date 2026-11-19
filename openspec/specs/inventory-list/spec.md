# inventory-list Specification

## Purpose

Lets the operator see what the avatar's inventory holds through the control socket, and defines how every command that works on an inventory item or folder names it, by UUID or by path.

## Requirements

### Requirement: Inventory command
`inventory [-a] [-n] [<folder>]` SHALL be answered with success and a text of one line for each entry under a folder of the avatar's My Inventory, or with success and no text when there is none. Without `<folder>` the folder is My Inventory itself. Without `-n` the entries are everything below the folder at any depth; with `-n` (also spelled `--no-recurse`) they are the folder's direct children only. With `-a` (also spelled `--attached`) only the items attached to the avatar now are listed (see `avatar-outfit`, "Knowing what is attached"), out of the entries the other arguments select. The flags MAY be given in any order and before or after `<folder>`. The folder itself SHALL NOT be listed. The Library SHALL NOT be listed.

The command SHALL be answered with an error and list nothing when an argument starts with `-` and is not one of the flags, when more than one folder is given, when the folder cannot be found (see "Naming an inventory item or folder"), or when the grid has not answered everything the listing needs within 50 seconds; that last error SHALL say that a folder can be named to list less.

#### Scenario: Everything
- **WHEN** `rslbot ctl inventory` is run and My Inventory holds the folder `Objects` with the object `Radio` in it
- **THEN** the output has a line for `Objects/` and a line for `Objects/Radio`, and the command exits with status 0

#### Scenario: Under a folder
- **WHEN** `rslbot ctl inventory "Objects/Bot gear"` is run
- **THEN** only entries under `Objects/Bot gear` are listed, at every depth, and `Objects/Bot gear/` itself is not

#### Scenario: Direct children only
- **WHEN** `rslbot ctl inventory -n Objects` is run and `Objects` holds the item `Radio` and the folder `Bot gear` with items of its own
- **THEN** the output has exactly the lines for `Objects/Radio` and `Objects/Bot gear/`

#### Scenario: Attached only
- **WHEN** `Radio` is attached to the avatar, nothing else is, and `rslbot ctl inventory -a` is run
- **THEN** the output is the one line for `Radio`

#### Scenario: Attached only, under a folder
- **WHEN** `Objects/Radio` is attached and `rslbot ctl inventory -a Clothing` is run
- **THEN** the command exits with status 0 and prints nothing

#### Scenario: Empty folder
- **WHEN** `rslbot ctl inventory Empty` names a folder with nothing in it
- **THEN** the command exits with status 0 and prints nothing

#### Scenario: Unknown flag
- **WHEN** `rslbot ctl inventory -x` is run
- **THEN** the command exits with status 1 naming `-x`, and the grid is asked nothing

#### Scenario: Folder that does not exist
- **WHEN** `rslbot ctl inventory Nowhere` is run and My Inventory has no folder `Nowhere`
- **THEN** the command exits with status 1 saying which part of the path was not found

#### Scenario: Inventory too large to list in time
- **WHEN** `rslbot ctl inventory` is run and the grid needs more than 50 seconds to answer for every folder
- **THEN** the command exits with status 1 within 55 seconds, saying the listing took too long and that a folder can be named

### Requirement: Listing format
Each line of the listing SHALL consist of five fields separated by tab characters, in this order:

1. the entry's UUID;
2. its type: `folder`, `object`, `clothing`, `bodypart`, `animation`, `sound`, `gesture`, `texture`, `notecard`, `script`, `landmark`, `callingcard`, `link` for a link to another item or folder, or `other`;
3. its attach or wear point: for an object, the name of the attachment point it is attached to now, or else of the point it was last attached to, or `-` when the inventory records none; for clothing and body parts, the kind (`shirt`, `pants`, `skin`, ...); `-` for every other type;
4. `attached` when the item is attached to the avatar now, `-` otherwise;
5. its path from the top of My Inventory, in the form "Naming an inventory item or folder" accepts, with a `/` at the end for a folder.

Attachment points SHALL be named as `avatar-outfit` ("Body-part names") names them. A tab, line feed, carriage return or backslash inside a name SHALL be written as `\t`, `\n`, `\r` and `\\`, and a `/` inside a name as `\/`. The lines SHALL be in the order of their paths. There SHALL be no header line. A link SHALL be listed with the link's own UUID and SHALL NOT be shown as attached.

#### Scenario: An attached object
- **WHEN** the object `Objects/Radio` is attached to the left hand and the listing includes it
- **THEN** its line is its UUID, `object`, `Left Hand`, `attached` and `Objects/Radio`, separated by tabs

#### Scenario: An object that is not attached
- **WHEN** the object `Objects/Hat` was last attached to the skull and is not attached now
- **THEN** its line has `Skull` in the third field and `-` in the fourth

#### Scenario: Clothing
- **WHEN** the listing includes the shirt `Clothing/Blue shirt`
- **THEN** its line has `clothing` in the second field and `shirt` in the third

#### Scenario: A folder
- **WHEN** the listing includes the folder `Objects`
- **THEN** its line is the folder's UUID, `folder`, `-`, `-` and `Objects/`

#### Scenario: A slash in a name
- **WHEN** the folder `Objects` holds an item named `Radio/TV`
- **THEN** its path is printed as `Objects/Radio\/TV`, and giving that path to `attach` names the same item

#### Scenario: Order
- **WHEN** `Objects` holds `Radio` and the folder `Bot gear` holding `Antenna`
- **THEN** the lines come in the order `Objects/`, `Objects/Bot gear/`, `Objects/Bot gear/Antenna`, `Objects/Radio`

### Requirement: Naming an inventory item or folder
A command that takes an inventory item or folder SHALL take it as one argument, which is read as a UUID when it has the form of one and as a path otherwise.

A UUID names the item, or for `inventory` the folder, with that UUID in the avatar's own inventory. A path consists of names separated by `/`, starting at the top of My Inventory: every name but the last names a folder inside the one before it, and the last names the entry. A `/` at the start or the end of a path SHALL be ignored, so a path that would otherwise be read as a flag, a UUID or a body part can be written with a `/` in front. Inside a name, `\/` stands for a `/` and `\\` for a backslash. An empty path, or `/` alone, names My Inventory itself and is accepted only where a folder is expected. Names SHALL match exactly, including case. For the last name only entries of the kind the command works on are considered: folders for `inventory`, items for every other command. A link to an item SHALL stand for the item it points to.

The command SHALL be answered with an error and do nothing further when no entry matches, saying which part of the path was not found; when more than one entry in a folder matches a name of the path, listing the UUIDs of those entries; when a UUID names nothing in the avatar's inventory; or when the grid does not answer the lookup within 50 seconds.

#### Scenario: By path
- **WHEN** `rslbot ctl attach "Objects/Bot gear/Radio"` is run and that item exists
- **THEN** the item `Radio` in the folder `Bot gear` in the folder `Objects` is the one attached

#### Scenario: By UUID
- **WHEN** `rslbot ctl attach <uuid>` is run with the UUID the `inventory` command printed for `Radio`
- **THEN** `Radio` is the item attached

#### Scenario: Leading slash
- **WHEN** `rslbot ctl detach /Chest` is run and My Inventory holds an attached object named `Chest`
- **THEN** that object is detached, and other things attached to the chest stay

#### Scenario: Case matters
- **WHEN** `rslbot ctl attach objects/radio` is run and the folder is named `Objects`
- **THEN** the command exits with status 1 saying `objects` was not found

#### Scenario: Two entries with one name
- **WHEN** `rslbot ctl attach Objects/Radio` is run and `Objects` holds two items named `Radio`
- **THEN** the command exits with status 1, the error lists both UUIDs, and nothing is attached

#### Scenario: Item and folder with one name
- **WHEN** `Objects` holds both an item `Radio` and a folder `Radio`
- **THEN** `rslbot ctl attach Objects/Radio` takes the item and `rslbot ctl inventory Objects/Radio` lists the folder

#### Scenario: Through a link
- **WHEN** `rslbot ctl attach "Outfits/Work/Radio"` names a link to the object `Objects/Radio`
- **THEN** `Objects/Radio` is attached

#### Scenario: UUID not in the inventory
- **WHEN** `rslbot ctl attach <uuid>` is run with a UUID that is no item of the avatar's inventory
- **THEN** the command exits with status 1 saying there is no such item, and nothing is sent to the simulator

### Requirement: Inventory is read on demand
The bot SHALL ask the grid about the inventory only to carry out a command that needs it and, while `restore_attachments` is on, to restore attachments after a login (see `avatar-outfit`, "Restoring attachments after a login"). It SHALL NOT keep what it learned for a later command: every command sees the inventory as the grid has it then. The `inventory` command and every lookup of an item or folder SHALL be answered with an error, and SHALL ask the grid nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout, and when the simulator the avatar is on offers no inventory service. A command whose lookup is under way when the session is lost or the avatar changes simulator SHALL be answered with an error. Reading the inventory SHALL NOT delay the bot's simulator traffic and SHALL NOT change the bot's placement mode.

#### Scenario: No command, no inventory traffic
- **WHEN** the bot runs for an hour with `restore_attachments = false` and is given none of the commands that name an inventory item or folder
- **THEN** it has asked the grid for no inventory folder and no inventory item

#### Scenario: No command and no outfit service
- **WHEN** the bot runs for an hour with `restore_attachments` on, on a grid whose simulator offers no outfit service, and is given none of those commands
- **THEN** it has asked the grid for no inventory folder and no inventory item

#### Scenario: Change made in a viewer
- **WHEN** an item is renamed with a viewer between two `rslbot ctl inventory` commands
- **THEN** the second listing shows the new name

#### Scenario: Between logins
- **WHEN** `rslbot ctl inventory` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Session lost during a listing
- **WHEN** the session is lost while a listing is being collected
- **THEN** the command exits with status 1 saying the session was lost, and the bot logs in again as usual

#### Scenario: Listing while seated
- **WHEN** `rslbot ctl inventory` is run in holding mode while the avatar is seated
- **THEN** the avatar stays seated, stays connected, and the bot stays in holding mode

### Requirement: Inventory commands appear in the operator log
The bot SHALL write a line to its operator log for every `inventory` command it carries out, saying which folder and filters were asked for and how many entries were listed, and for every one it refuses, with the reason. The entries themselves SHALL NOT be written to the operator log.

#### Scenario: Reading the log after a listing
- **WHEN** `rslbot ctl inventory -a Objects` has listed two entries
- **THEN** the log has one line naming `Objects`, the attached-only filter and the count 2, and no line per entry
