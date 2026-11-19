# Design

## Context

See proposal.md for motivation. What the code looks like today and what that means here:

- `control.rs` decodes one request line into a `Request`; `ctl` sends every argument as its own field, so new commands need no client changes beyond the usage text. `Response` has no size limit: `ctl` reads the answer to the end of the connection, and `status` already sends several lines in one escaped field. The connection thread waits 75 s (`ANSWER_LIMIT`) and `ctl` 90 s.
- `Commands` (`commands.rs`) answers at once or parks the reply: `Waiting` with an `After` for a name lookup, `pending_sit` and `pending_tp` for session outcomes, replies by UUID for `touch`. It reaches the session only through the `Grid` trait, which `main.rs` implements in `Live`.
- HTTP never runs on the main loop. `names.rs` has two worker threads that take a request with the capability URL in it and answer with an `Event`. `caps.rs` asks the seed capability for three names (`EventQueueGet`, `GetDisplayNames`, `AvatarPickerSearch`) and reports the URLs once per simulator as `CapsEvent`s.
- `login.rs` sends an empty `options` array, so the login reply carries no inventory ids.
- `Session` decodes `ObjectUpdate` and `ObjectUpdateCompressed` to UUID, local id, parent and transform. It knows its own avatar's local id (`own_local`) and the region handle. In `ObjectUpdate` it skips `State` and `NameValue`; those two fields are what say "this is an attachment, on this point, from this inventory item".
- The bot knows nothing about inventory: no folder ids, no items, no Current Outfit folder.

Checked against `references/kokua` (message template, viewer source) and `references/opensim` while planning; task 1.1 repeats the check field by field before anything is built on it:

- `RezSingleAttachmentFromInv` Low 395 (Zerocoded): agent and session, then ItemID, OwnerID, AttachmentPt U8, ItemFlags, GroupMask, EveryoneMask, NextOwnerMask, Name, Description. `AttachmentPt` 0 means "where it was last"; bit `0x80` (`ATTACHMENT_ADD`) means "do not replace what is there". OpenSim reads both the same way (`AttachmentsModule`).
- `DetachAttachmentIntoInv` Low 397: AgentID, ItemID.
- `AgentAnimation` High 5: agent and session, a list of (AnimID, StartAnim), an empty event list. AnimID is the animation's asset id, not the inventory item's.
- `SoundTrigger` High 29: SoundID, OwnerID, ObjectID, ParentID (viewers send null for the three; the simulator fills them in), region handle, position, gain.
- An attachment's root prim has the avatar as its parent, carries the name-value `AttachItemID STRING RW SV <item uuid>` (OpenSim writes it in full and compressed updates alike), and has the point in `State` with the nibbles swapped (`ATTACHMENT_ID_FROM_STATE`). In `ObjectUpdateCompressed` the name-values are present when flag `0x100` is set, after the optional fields for `0x80`, `0x20`, tree, scratch pad, text, media URL (`0x200`), particles, extra parameters and sound (`0x10`).
- Inventory over HTTP: `FetchInventoryDescendents2` (POST `{folders: [{folder_id, owner_id, sort_order, fetch_folders, fetch_items}]}`; the reply lists `categories` with `category_id`, `name`, `type_default` and `items` with `item_id`, `parent_id`, `name`, `desc`, `type`, `inv_type`, `flags`, `asset_id`, `permissions`) and `FetchInventory2` (items by id). Both exist on Second Life and in the reference OpenSim. The root folder's id comes from the login option `inventory-root`. The Current Outfit folder is the root's child with `type_default` 46.
- Clothing: current viewers no longer tell the simulator what they wear (`AgentIsNowWearing` is sent with dummy ids). They add links to the Current Outfit folder through `InventoryAPIv3` (POST `<cap>/category/<folder>/links?tid=<uuid>` with `{links: [{linked_id, type, inv_type, name, desc}]}`, DELETE `<cap>/item/<link>`) and POST `{cof_version}` to `UpdateAvatarAppearance`; the server makes the textures. The viewer has no fallback without `InventoryAPIv3`. The reference OpenSim offers neither capability and expects baked textures from the viewer.

## Goals / Non-Goals

**Goals:**
- A bot that is given none of these commands sends what it sends today, plus a few names in two requests it makes anyway.
- Inventory work stays off the main loop and cannot stall the simulator connection.
- `Commands` stays testable with a fake `Grid`, `Session` with the fake `Net`, and the inventory code with canned replies and a local HTTP listener, as `names.rs` is.
- One place decides what "this item" means (UUID or path), used by every command.

**Non-Goals:**
- Changing the inventory: creating, moving, renaming, deleting, giving or receiving items. The only writes are links in the Current Outfit folder.
- The Library, other avatars' inventories, and the contents of objects.
- A cached copy of the inventory, or folder versions to keep one fresh.
- Playing an animation or sound by asset id, built-in animations by name, gestures, looping or timed animations, stopping "everything".
- Baking textures. Clothing on grids without the outfit service stays out.
- Replacing what is on an attachment point, moving an attached item to another point, wearing whole outfit folders.
- Restoring animations after a new login. Attachments and clothing persist on the grid by themselves.
- Showing in the listing which clothing is worn.

## Decisions

### Wire: flags and one argument per item, parsed in the bot
New `Request` variants: `Inventory(Vec<String>)`, `Attach { item, part }`, `Wear(String)`, `Detach(String)`, `Anim { stop: bool, item }`, `Sound(String)`. `Request::decode` checks counts and joins trailing words where the grammar says so (`attach`: one item, then the body part's words; `detach`/`unwear`: all words joined; `anim`: exactly `play|stop` and one item; `wear`, `sound`: exactly one). Everything that needs judgement is parsed by pure functions in `inventory.rs`: the `inventory` flags and folder, paths (`parse_path` with `\/` and `\\`), body-part names and numbers, kinds of clothing, and the order in which `detach` reads its argument (UUID, then point or kind, then path).

`detach` joins its words because point names have several (`detach left hand`); an item path with spaces still has to be quoted as one argument, and a joined argument that is neither a point nor a kind is taken as that path.

*Alternative:* keyword filters without dashes (`inventory attached Objects`). A folder named `attached` would then be unreachable without an escape; dashes are what `ctl` users expect for options, and `ctl` already passes them through untouched.

### One inventory worker thread, one job per command
New `src/inventory.rs` with a worker thread in the manner of `names.rs`. `Commands` sends it a `Job { id, generation, urls, root, deadline, what }` and parks the reply under `id`; the worker answers with `Event::Inventory { id, result }`. `what` is one of:

- `List { folder: Ref, recursive }` → entries (id, type, flags, linked id, path);
- `Find { item: Ref }` → the item record (id, name, description, type, inventory type, flags, asset id, permissions), links followed with one `FetchInventory2`;
- `Link { item }`, `Unlink { item }` → Current Outfit folder bookkeeping for attachments;
- `WearClothing { item }`, `TakeOff { items }` → the clothing sequence below.

`Ref` is a UUID or a parsed path. The worker resolves a path by fetching one folder per name, folders only until the last name. A recursive listing walks breadth-first, several folders per request (start with 10 per request; task 1.1 checks what the grids accept). The job's `deadline` is 50 s after the command arrived; the worker gives up when it passes, and the HTTP agent's timeout is the time left. Jobs run one after another; a job that waited past its deadline in the queue is answered as timed out.

`Commands` fails every parked job when the session is lost or the avatar changes simulator (the URLs are the old simulator's), and ignores an answer whose `id` it no longer holds. `generation` is the session's, as in `Event::Caps`.

What happens after `Find` is an enum next to `After`: `Attach { point }`, `Wear`, `Detach`, `AnimPlay`, `AnimStop`, `Sound`. `Commands` checks the item's type there and calls the `Grid`.

*Alternative:* fetch the whole inventory once and answer from memory. It makes every command instant after the first, but costs megabytes on a large inventory, goes stale the moment a viewer or script changes something, and needs the update messages decoded to stay right. The bot's selling point is its size; commands are rare.

*Alternative:* UDP `FetchInventoryDescendents`. The reference viewer fetches inventory only through the capabilities; the UDP message is the path Second Life retired, and it would put inventory replies of any size on the simulator circuit.

### Capabilities and the root folder
`caps::seed_request` also asks for `FetchInventoryDescendents2`, `FetchInventory2`, `InventoryAPIv3` and `UpdateAvatarAppearance`; `CapsEvent` gets one `Inventory(InventoryUrls)` with four optional URLs, sent once per simulator after the existing two, and `Bot` keeps the current set. `login::request_xml` asks for the option `inventory-root`; `SessionInfo` gets `inventory_root: Option<Uuid>`. Without the root id or without the two fetch URLs every inventory command is refused with "this simulator offers no inventory service".

### What is attached: read from object updates
The session gets `attachments: HashMap<Uuid, Attachment>` keyed by inventory item id, with the object's UUID, local id, parent local id and point. `decode_object_update` keeps `State` and looks for `AttachItemID` in `NameValue`; the compressed decoder walks the optional fields up to the name-values. An object with that name-value whose parent is `own_local` is an attachment; because the avatar's own update may arrive after its attachments', entries are kept with their parent and counted as attached only while the parent equals `own_local`. `KillObject` for the local id removes the entry. The table is emptied with the circuit, like the object tables. The session reports `SessionEvent::Attached { item, point }` and `Detached { item }` on changes; `Bot` logs them and passes them to `Commands`.

Nothing is requested for this. It does not touch the object scan: the scan's tables and its "nothing is sent before a scan" rule stay as they are.

*Alternative:* take the Current Outfit folder as the truth. It is a record kept by viewers, not the simulator's state: it is wrong after a script detaches something, is not maintained on every grid, and costs a request per question.

### Attach and detach wait for the simulator
`Session::attach(item_record, point, now)` sends `RezSingleAttachmentFromInv` (reliable) with `point | 0x80` and the item's flags, masks, name and description from the `Find` result, and remembers the item in `awaited_attach` with a 15 s deadline, as `touch` does with `wanted`. The `Attached` event for that item settles it; the deadline produces `AttachFailed(item)`. `Session::detach(item, now)` sends `DetachAttachmentIntoInv` and waits the same way for the entry to go. `Commands` keeps the replies by item id; a `detach` of a point waits for all of its items and answers once, naming those that went and failing if any did not.

Before sending, `Commands` asks `Grid::attached()` (item → point): an item already attached is refused, and `detach <point>` gets its item list from there. The names for the answer come from `Find` for an item, and from one `FetchInventory2` for the items on a point.

A simulator alert that arrives while an attach is awaited (for instance "too many attachments") is appended to the timeout error, since the simulator has no dedicated refusal.

### Body-part names
A fixed table of 55 names in `inventory.rs`, taken from `avatar_lad.xml` (ids 1 to 55), with `HUD ` in front of ids 31 to 38. Lookup lower-cases and compares; a number from 1 to 127 is accepted as it is. A point outside the table prints as its number. The item's last point is `flags & 0xff` for an object; for clothing and body parts the same byte is the wearable type, which gives the kind names.

### Animation and sound
After `Find`, `Commands` checks the asset type (animation 20, sound 1) and that `asset_id` is not null, then calls `Grid::animate(asset, start)` or `Grid::play_sound(asset)`. The session sends `AgentAnimation` (reliable, one list entry) or `SoundTrigger` with the null ids a viewer sends, the region handle, `sim.position` and gain 1.0. Both answer at once.

### Clothing: the outfit folder, then the bake request
Only when the simulator gave both `InventoryAPIv3` and `UpdateAvatarAppearance`; otherwise `Commands` refuses before any job.

`WearClothing { item }` in the worker: find the Current Outfit folder among the root's folders (`type_default` 46); fetch it; resolve its links with one `FetchInventory2` to learn each target's wearable type; refuse when the item is linked already; for a body part, delete the link to the one of the same kind; POST the new link; fetch the folder again for its version; POST `{cof_version}` to `UpdateAvatarAppearance`. If the appearance request is refused, undo the link changes in reverse and report the grid's reason. `TakeOff { items }` deletes the links and makes the same request. For `unwear <kind>` the worker selects the folder's links by wearable type.

`Link` and `Unlink` for attachments use the first half only (no appearance request: attachments are not baked). They are sent after the `Attached` or `Detached` event, their failure is logged, and nobody waits for them.

*Alternative:* `AgentIsNowWearing`. Second Life ignores it; OpenSim records it but shows nothing new without baked textures.

### Listing text
`inventory.rs` builds the text from the `List` result and the session's attachment table: five tab-separated fields per entry, sorted by path bytes, names escaped with the existing `escape` plus `/` → `\/`. The type comes from the asset type, with wearables split into `clothing` and `bodypart`. `-a` filters after the walk; `-n` makes the walk one request.

## Risks / Trade-offs

- [Second Life may describe the avatar's own attachments in a way the bot cannot read: compressed without name-values, or as cached updates it never asks about] → Then `-a`, `detach <point>` and the attach confirmation fail there while the rig passes. Task 6.3 checks it on Second Life with `--verbose` before the feature is called working there; if cached updates are the cause, asking for the avatar's own children (a handful of objects) is the fix, and that is a change to the "Object scan" requirement to be proposed then, not slipped in.
- [The compressed layout up to the name-values is long and flag-dependent; one wrong width shifts everything] → Byte-level tests with builders for each optional field; an update that does not decode is skipped, never guessed.
- [A whole inventory cannot be listed in 50 s on a large account] → The error says to name a folder. `inventory -a` walks the same folders and has the same limit.
- [Listing a large inventory holds it in memory once and sends it as one answer] → Transient; nothing is kept. A 50 000-entry inventory is some megabytes for the length of the command.
- [The layout of the two outfit-service requests is taken from viewer source, and cannot be tried on the rig at all] → Canned-reply tests fix what the bot sends; behaviour on Second Life is checked by hand in task 6.3 and stated as unverified in the README until then.
- [A failed undo leaves the Current Outfit folder changed while the avatar looks as before] → The error says so; `unwear` or a viewer puts it right.
- [Without the folder bookkeeping, a viewer that logs in later may take off what the bot attached, or put back what it detached; with it, a failed bookkeeping request has the same effect for that item] → Logged; attach and detach themselves do not depend on it.
- [`attach` adds, so repeating a command with different items piles them up on one point] → `inventory -a` and `detach <point>` are the way out; the grid's own limit on attachments produces a timeout with the simulator's alert in it.
- [Names that differ only in case, or duplicates in one folder, cannot be addressed by path] → The error lists the UUIDs, which always work.
- [An inventory offer or a change in a viewer during a path lookup can make the lookup miss] → The command fails with "not found"; nothing is cached, so the next try sees the new state.

## Open Questions

- Whether a later change should let `anim play` and `sound` take an asset UUID or a built-in animation's name. Nothing here depends on it.
- Whether the Library should become listable with a path prefix. It would be an addition to "Naming an inventory item or folder".
