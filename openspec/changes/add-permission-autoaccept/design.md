# Design

## Context

What the bot does with requests today, as read in the code:

- **Teleport offers** (`ImprovedInstantMessage`, dialog 22) become `SessionEvent::TeleportOffer` and go to `Commands::teleport_offer` (`src/commands.rs`). An offer from an avatar asked with `requesttp` within `requesttp_timeout` is accepted; any other is put into `Commands::offers`, a map from avatar UUID to lure id capped at 100, where `accepttp AVATAR` finds it after resolving the name on the grid.
- **Inventory offers** (dialogs 4 and 9) fall into the `other => debug!("ignoring instant message with dialog ...")` arm of `src/session.rs`. The parser in `src/udp/messages.rs` reads neither `FromGroup` nor the binary bucket of an instant message.
- **Script permission requests** (`ScriptQuestion`, Low 188) are not decoded at all.

What the grid tells the receiver, checked against the viewer source (`references/kokua`, `llimprocessing.cpp`, `llviewermessage.cpp`) and OpenSim (`references/opensim`, `InventoryTransferModule.cs`, `LSL_Api.cs`):

| Request | Carries | Does not carry |
|---|---|---|
| Teleport offer | avatar UUID, avatar name, lure id | |
| Inventory offer from an avatar (dialog 4) | avatar UUID and name, transaction id, bucket: asset type and item or folder UUID, text: the item's name | |
| Inventory offer from an object (dialog 9) | owner UUID (the group's with `FromGroup`), object name, transaction id, bucket: asset type, text: item name and place | the object's UUID, the owner's name |
| `ScriptQuestion` | object UUID, script item UUID, object name, owner name as text, permission bits | the owner's UUID, whether the owner is an avatar or a group |

Two more facts shape the design. The item of an inventory offer is already in the receiver's inventory when the offer arrives, on both grids; accepting tells the giver so and names the folder to file it in (the viewer sends the system folder for the type; its "decline" is an accept into the Trash). And the viewer answers a `ScriptQuestion` with `ScriptAnswerYes` (Low 132) carrying the bits it grants; there is no refusal message other than the same one with no bits.

## Goals / Non-Goals

**Goals:**
- One set of rules, per kind of request, deciding what the bot accepts by itself, set in the config and changeable through the control socket.
- A request that is not accepted stays acceptable by hand, with one command shape for every kind.
- The rules cannot be satisfied by someone who merely looks like a trusted avatar.
- A bot without a `perms` table moves and answers exactly as it does today.

**Non-Goals:**
- Refusing requests (`decline`): a waiting request is simply left, as when a viewer's user ignores the dialog.
- Friendship offers, group invitations, group notices with attachments, script dialogs, experience permission requests.
- Granting debit, script teleport or any permission outside the four script kinds.
- Taking back permissions a script was already granted (`perms revoke` changes the rules only).
- Saving runtime rule changes.
- RLV restrictions that force acceptance (`@accepttp`, `@acceptpermission`); the decision is kept in one place so that they can feed it later.

## Decisions

### 1. A new module `src/perms.rs` owns the rules and the waiting requests

It holds, with no network code: `Kind` (the six kinds and their names), `Entry` (`Id(Uuid)` or `Name(String)` normalised), `Rule` (`All` or `List`), `Rules` (rule and exceptions per kind, built from the config, changed by the `perms` commands), `Sender` (what is known of who asks), the matching and decision functions, and `Waiting` (the three bounded collections). `Bot` in `src/main.rs` owns one `Rules` and passes it where a decision is needed. Pure functions over plain data are what the unit tests exercise, as with `rlv.rs`.

Name normalisation is the one `parse_recipient` in `commands.rs` already does for `im` (case-folded, `first.last`, single word means `Resident`); it moves to where both can use it.

*Alternative:* put the rules into `Commands`. Rejected: `Commands` is already the largest file, and the rules are needed by code that is no command (script questions arriving).

### 2. An avatar entry covers the avatar's objects; an object is named by UUID

The request's examples put avatar names and object UUIDs into one list, so an entry is not tagged with what it names. A UUID matches whatever id of the sender is known (avatar, object, owner); a name matches the avatar or the owner. Names are compared, not resolved: the config can be checked and used without a connection, and the request itself carries the name in every case but one (decision 4).

### 3. A script's owner is confirmed with the simulator

`ScriptQuestion` gives the owner as text. For an object deeded to a group that text is the group's name (OpenSim's `resolveName`, and the viewer's dialog on Second Life), and nothing stops a group from being named like an avatar. A rule `anim = ["Jane Doe"]` would then be met by anybody who founds a group of that name. So before deciding, the bot sends `RequestObjectPropertiesFamily` (Medium 5) for the object and reads `OwnerID` and `GroupID` from `ObjectPropertiesFamily` (Medium 10): the owner is an avatar when `OwnerID` is neither nil nor equal to `GroupID`. This also gives the owner's UUID, so that a UUID entry for an avatar covers its objects in script requests too, which matters since avatars can be renamed.

The wait is bounded at 10 seconds (`LOOKUP_LIMIT`, the limit name lookups already use). Without an answer only the object's own UUID can match, so the failure is on the safe side.

*Alternative:* trust the text. Cheaper by one round trip, but it makes `attach` and `controls` grantable to strangers. *Alternative:* resolve the owner's name to a UUID by avatar search. Slower, and it does not tell a group from an avatar.

### 4. An object's inventory offer is matched by its owner only

The offer has no object UUID, so there is nothing else to match. The owner's UUID is in the offer; the owner's name is fetched through the existing display-name lookup (the worker in `src/names.rs` that the message log uses for owners of talking objects). `Bot` adds the UUID to the batch it already builds from `Recorder::take_lookups` and hands the reply to both. The name is fetched for every such offer, not only where a rule has a name in it: the rig run showed that an offer that waits is otherwise shown without its owner's name and cannot be found by `accept inventory "Owner Name"`.

The config still takes any UUID under `inventory`: the bot cannot tell an object's UUID from an avatar's. The README says which one works.

### 5. All or nothing for a request with several kinds

A script that asks for animations and controls gets both or neither by itself. Granting a subset is possible on the wire (the viewer does it for bits it does not know), but scripts commonly test for the exact set they asked for and then sit idle, which is worse than a request that visibly waits. RLVa's `@acceptpermission` works the same way. The log names the kind that was missing, so the operator knows which rule to add.

`accept` follows from this: it grants the whole request and says what it granted. A request is listed under each of its kinds so that `accept anim` finds a request for animations and controls.

Bits outside the four script kinds make a request unacceptable altogether, also by hand. The bits and their kinds:

| Bit | Permission | Kind |
|---|---|---|
| `0x4` | take controls | `controls` |
| `0x10` | trigger animation | `anim` |
| `0x20` | attach | `attach` |
| `0x400`, `0x800` | track camera, control camera | `camera` |
| `0x8000` | override animations | `anim` |
| anything else (`0x2` debit, `0x1000` teleport, ...) | | none: never granted |

`controls` and `camera` were not in the request. They are here because all-or-nothing would otherwise make the common "animate and take controls" request impossible to auto-accept; both are harmless for an avatar nobody steers.

### 6. `perms revoke` under `all` writes an exception

The runtime commands have two verbs and the config has three lists (`all`, entries, exceptions). The mapping in the spec's table keeps both verbs meaning what they say: after `autoaccept K X`, X is auto-accepted; after `revoke K X`, X as an entry is not. `autoaccept K all` replaces a list (it is covered anyway); `revoke K all` leaves an empty list. Exceptions survive both, since they are the cautious half.

What the two verbs cannot express is an exception for one object of a listed avatar while the rule is a list: `revoke` on an entry that is not in the list only warns, as the request asks. That state is reachable from the config. A third verb can be added if it turns out to be wanted.

### 7. Teleport offers move under the common rules; `accept tp` matches locally

`Commands::teleport_offer` keeps its `requesttp` check and then asks the rules instead of going straight to "kept". The offers map becomes the teleport part of `Waiting`, keeping the avatar's name beside the lure id so that `accept tp "Jane Doe"` and `perms status` need no lookup. `accepttp` used to resolve its argument on the grid; `accept tp` compares it with the names the offers came with, which is faster and gives "no offer from X" where it used to give "no such avatar". `Request::AcceptTp` gets an optional argument and both spellings decode to it.

A teleport offer accepted by rule calls the same path as `accept tp`, including the switch to manual mode. Without it a bot that holds a place would arrive and be teleported home by its own reconciler.

### 8. Accepting an inventory offer names the system folder

The answer is dialog 5 (to an avatar) or 10 (to an object's owner) with the offer's transaction id and the destination folder's UUID as the bucket. The folder comes from a new inventory job, `What::SystemFolder(asset_type)`: one `FetchInventoryDescendents2` of the top folder, folders only, picking the child whose `type_default` equals the asset type, else the top folder. `inventory.rs` already parses `type_default`. Nothing is cached, in line with "Inventory is read on demand", which gets one more reason to read.

*Alternative:* send a nil folder. OpenSim leaves the item where it is, but what Second Life does with it is unknown and an item moved to nowhere is hard to get back. *Alternative:* take the folders from the login reply's inventory skeleton. That is a login-time snapshot of every folder for one lookup.

### 9. Waiting requests

Three collections, each bounded at 100 with the oldest dropped: teleport offers keyed by avatar, script requests keyed by object and script item, inventory offers in arrival order. All are emptied where `Commands` forgets its offers today (session end); script requests also on a region change, since the answer goes to the simulator the avatar is on and the object stays behind.

Rules changes do not touch what waits. Accepting retroactively would mean a teleport as a side effect of editing a list.

A script that asks again closes what it asked before, also when the newer request is accepted or can never be: `llRequestPermissions` replaces the script's open question, so the older one has nothing left to answer.

### 10. Commands and answers

`Request::Perms(PermsRequest)` with `Autoaccept { kind, who }`, `Revoke { kind, who }`, `Status`, and `Request::Accept { kind, who }`, decoded in `src/control.rs` beside the `rlv` requests. `perms` requests are answered by `Bot` directly (no session needed); `accept` goes through `Commands` for `tp` and `inventory` (both wait for something) and is answered at once for script kinds.

A warning is part of a success answer (a line starting with `warning:`, exit status 0): the state the operator asked for holds.

## Risks / Trade-offs

- **`tp = "all"` lets anyone move the bot.** → It is what the operator wrote; the log line for each accepted offer names the avatar, and the bot is left in manual mode where `home` brings it back.
- **A script can ask for permissions in a loop**, each request costing a properties lookup and a log line. → A repeated request from the same script replaces the waiting one; the lookup is one small message. A throttle like the viewer's can follow if a real object does this.
- **Whether Second Life accepts the inventory answers and `ScriptAnswerYes` exactly as built** is known from the viewer source only until tried there. → The rig covers OpenSim; a Second Life check is a task of its own and its result goes into the README's status section.
- **`accept tp NAME` no longer asks the grid**, so a misspelt name reads "no offer from X". → The answer names what was typed; `perms status` shows who the waiting offers are from.
- **Inventory offers accepted by rule fill the inventory.** → The item is there whether or not the offer is accepted; accepting only files it and tells the giver.
- **The object-properties answer may not come** for an object far away or gone. → The request then waits for the operator, who can accept it by the object's UUID.

## Open Questions

- Whether `controls` and `camera` should be kinds of their own, as here, or be granted along with `anim`.
- Whether a `decline` command is wanted. For scripts it would send an answer with no permissions; for inventory it would file the item into the Trash, as the viewer does.
