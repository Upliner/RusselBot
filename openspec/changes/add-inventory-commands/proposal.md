# Proposal

## Why

The bot can move the avatar and make it talk, but it cannot use anything the avatar owns. An operator who wants the avatar to carry a scripted attachment, change what it wears, dance or make a noise has to log in with a viewer. These commands let `rslbot ctl` do it: see what is in the inventory, put things on and take them off, and play the animations and sounds found there.

## What Changes

New `rslbot ctl` commands:

- `inventory [-a] [-n] [<folder>]`: list the avatar's inventory, one line per entry with its UUID, type, attach or wear point, whether it is attached now, and its path. Without arguments it lists everything in My Inventory. `<folder>` (a path or a folder's UUID) limits it to what is under that folder; `-n` to the folder's direct children instead of everything below; `-a` to the items attached to the avatar now.
- `attach <item> [<body part>]`: attach an object from the inventory, to the named attachment point or, without one, to the point it was last attached to.
- `wear <item>`: wear a piece of clothing or a body part (shape, skin, hair, eyes). For an object it does what `attach <item>` does.
- `detach <item or body part>`, also under the name `unwear`: take off an attached object or a worn piece of clothing, or everything on the named attachment point, or all clothing of the named kind (`unwear shirt`).
- `anim play <item>`, `anim stop <item>`: start and stop an animation from the inventory on the avatar.
- `sound <item>`: play a sound from the inventory at the avatar's position.

`<item>` is one argument: the item's UUID as `inventory` prints it, or its path from the top of My Inventory, such as `"Objects/Bot gear/Radio"`.

Behaviour that comes with them:

- **What is attached** is what the simulator shows on the avatar. The bot reads it from the object descriptions the simulator sends anyway, so knowing it costs no traffic.
- **Inventory is read when asked.** The bot keeps no copy of the inventory: each command asks the grid for the folders it needs, and a bot that is never given one of these commands asks the grid for nothing more than today.
- **Clothing needs the grid's outfit service.** Second Life makes the avatar's look from its Current Outfit folder on the server; the bot changes that folder and asks for the new look. A grid without that service (OpenSimulator) expects the viewer to paint the avatar's textures itself, which the bot cannot do: there `wear` and `unwear` refuse clothing and body parts with that reason. Attachments, animations and sounds work on both.
- **Current Outfit folder.** Where the grid has the outfit service, the bot also records attachments it puts on and takes off in that folder, as viewers do, so that a viewer logging in to the account later finds the avatar as the bot left it.
- None of these commands moves the avatar or changes whether the bot is holding its place.

Assumptions recorded here because the request left them open:

- `attach` and `wear` add: nothing already on the avatar is taken off to make room, whether on the same attachment point or of the same kind of clothing. The exception is a body part, of which the avatar always wears exactly one of each kind, so wearing one replaces the one worn.
- "Attachments only" means attached to the avatar now, not "items that could be attached". It filters the listing it is given with: `inventory -a` looks through all of My Inventory, `inventory -a <folder>` only under that folder.
- The listing is recursive unless `-n` is given, since the request says everything from My Inventory is the default.
- Body parts are named as viewers name attachment points (`Left Hand`, `R Upper Arm`, `HUD Center`), compared without regard to case, or given by number. Kinds of clothing are named `shirt`, `pants`, `skin` and so on.
- A UUID is an inventory item's UUID, not an asset's: `anim play` and `sound` take what `inventory` lists, and cannot be given an animation or sound that is not in My Inventory.
- The Library is not listed and its items cannot be named. "My Inventory" is the only tree these commands see.
- Paths match names exactly, including case. When two entries in one folder have the same name, the path is refused and the answer lists their UUIDs.
- `attach` and `detach` answer when the simulator shows the object on or off the avatar, or with an error after 15 seconds. `wear` and `unwear` of clothing answer when the grid has accepted the new outfit. `anim` and `sound` answer once the request has been handed to the simulator connection.
- An animation keeps playing until it is stopped, ends by itself, or the session is lost; the bot does not start it again after a new login.
- A listing or a lookup that the grid has not finished answering within 50 seconds is refused; a very large inventory has to be listed folder by folder.

## Capabilities

### New Capabilities
- `inventory-list`: the `inventory` command, its filters and output, and how the other commands name an inventory item or folder by UUID or path.
- `avatar-outfit`: `attach`, `wear`, `detach` and `unwear`, how the bot knows what is attached, body-part names, and keeping the Current Outfit folder in step.
- `animation-sound`: `anim play`, `anim stop` and `sound`.

### Modified Capabilities

None. The new commands need the control socket and the session as they are; the requirements of `control-socket`, `avatar-commands` and `grid-session` do not change.

## Impact

- `src/control.rs`: new request variants, argument checks, usage text.
- `src/commands.rs`: the new commands, replies parked while the inventory is asked and while an attach or detach is awaited.
- New `src/inventory.rs`: a worker thread for the inventory requests over HTTP (folder contents, single items, outfit links, the appearance request), path parsing and resolution, and the listing's text.
- `src/caps.rs`: asks the seed capability for `FetchInventoryDescendents2`, `FetchInventory2`, `InventoryAPIv3` and `UpdateAvatarAppearance`.
- `src/login.rs`: asks the login for the id of the inventory's root folder.
- `src/session.rs`: the table of the avatar's attachments, attach and detach, animation start and stop, sound trigger.
- `src/udp/messages.rs`: `RezSingleAttachmentFromInv`, `DetachAttachmentIntoInv`, `AgentAnimation`, `SoundTrigger` (outgoing); the attachment point and the inventory item id in object updates.
- `src/main.rs`: wiring.
- `README.md`, `docs/test-rig.md`, `test-rig/`: the commands, what works on which grid, a way to stock the bot account's inventory on the rig, rig results.
- No new dependencies and no new config keys. No extra traffic unless one of these commands is used, apart from one more name in the login request and four in the capability request.
