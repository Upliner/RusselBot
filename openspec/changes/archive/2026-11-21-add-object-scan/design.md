# Design

## Context

See `proposal.md` for the motivation. What the code has today, and what shapes the approach:

- `Session` (`src/session.rs`) already keeps, per region, every object the simulator has described: `objects` (UUID to local id), `local_ids`, `parents`, `places` (position of each, relative to its parent when it has one), plus the attachment roots keyed by inventory item. `thing()` already works out an object's root, its linkset and its distance from the avatar. All of it is dropped with `Sim::new` on a region change.
- The object scan (`start_scan`) only asks for objects the simulator mentioned as cached (`RequestMultipleObjects`). Objects the simulator describes unasked are learned whether a scan runs or not. The distance is `draw_distance()`: `scan_distance` or 32 m, sent as `Far` in the `AgentUpdate` every second.
- The decoders in `src/udp/messages.rs` skip two things the scan needs: `PCode` (prim, avatar, tree, grass) and `UpdateFlags`, both present in `ObjectUpdate` and `ObjectUpdateCompressed`.
- Names and owners are not in object updates. The bot has `RequestObjectPropertiesFamily` (one object per message, by UUID) and decodes only owner and group from the answer; `Commands::object_owner` uses it for inventory offers.
- Owner names: `names.rs` resolves avatar UUIDs through `GetDisplayNames` on a worker thread; `Commands::take_name_lookups` / `names_resolved` already feed it for permission requests.
- Commands that wait are parked in `Commands` with the reply sender and finished from `tick` or from a session event; `status`, `rlv`, `perms`, `autoreply` and `quit` are answered in `Bot::control` and never use up an RLV override.
- The task throttle is 30 kbit/s (`THROTTLES`), enough for the avatar's own updates. Object properties and object descriptions travel on that channel.

## Goals / Non-Goals

**Goals:**
- A `scan` answer that is complete for what the simulator has described, in one command, without the operator having to run it twice.
- No traffic for it until it is used, and none repeated for objects already named.
- Keep the listing logic (arguments, filters, rows) free of the session, so it is unit-tested with plain data.

**Non-Goals:**
- Avatars in the listing (a radar), group names, creators, last owners, sale and permission data, click actions.
- A child prim's own position: it counts at its linkset's root.
- Refreshing names during a stay in a region, and a switch to force it.
- Applying RLV restrictions such as `shownames` to the listing.
- Changing how `touch`, the RLV look-ups or `scan_distance` behave, beyond the raised distance.

## Decisions

### 1. Properties through ObjectSelect and ObjectDeselect

A scan asks for properties the way the viewer's area search does (`fsareasearch.cpp:603`): `ObjectSelect` (Low 110) with up to 255 local ids, followed at once by `ObjectDeselect` (Low 111) with the same ids. The simulator answers with `ObjectProperties` (Medium 9), several objects per message, each with `ObjectID`, `OwnerID`, `GroupID`, `Name` and `Description` among fields the decoder skips.

Alternative considered: `RequestObjectPropertiesFamily`, which the bot already sends. It takes one object per message, so a scan of a thousand objects is a thousand reliable packets out, and OpenSimulator answers it for the linkset (`SceneGraph.RequestObjectPropertiesFamily` goes through `GetGroupByPrim` and replies with the root part), so child prims could not be named with it. Using it for roots and selection for children would mean two mechanisms for one listing.

The price: an object the avatar may edit counts as selected for the moment between the two messages (OpenSim `Scene.SelectPrim` sets `IsSelected`; Second Life pauses a physical object its owner selects). The viewer's area search does the same to every object it lists.

### 2. The seat's name through RequestObjectPropertiesFamily

The seat is asked about without selecting it: a seat the avatar owns may be a moving vehicle, and it would stutter on every sit. The avatar's parent is the root prim of the seat's linkset, and the bot already asks the simulator to describe that prim on every sit (`request_objects(&[parent])`). Once it is described, and so has a UUID, the session sends one `RequestObjectPropertiesFamily` for it and keeps the `Name` of the answer whose `ObjectID` is that UUID. The decoder is extended to read `Name` (it stops after `GroupID` today); `SessionEvent::ObjectOwner` stays as it is.

The properties a scan has for that local id are used instead when they are there. The family answer is kept beside them, for the seat only, and a scan reads it as that object's properties, so it does not select the seat while the avatar sits on it. The name is asked for once per sit: the `own_parent` change that clears `seat_frame` also clears the asked mark and the answer.

### 3. Keep the kind and the flags of each object

`ObjectParent` and `CompressedObject` get `pcode: u8` and `flags: u32` (`UpdateFlags`). `Sim` gets one map from local id to these two, filled in `learn` and cleared where `parents` and `places` are. Only `PCode` 9 (`LL_PCODE_VOLUME`) is ever listed. The flag bits are those of `object_flags.h`: physics `1<<0`, scripted `1<<6`, touch `1<<7`, phantom `1<<10`, temporary `1<<29`.

`attachment` and `child` are derived, not flags: a prim is an attachment when its lineage (`lineage()`) contains an avatar, which also covers attachments whose `AttachItemID` the bot never saw; it is a child when its parent is a prim.

### 4. A reach per region

`Sim` gets `reach: Option<f32>`, the largest distance a `scan` named in the region. `draw_distance()` becomes the larger of it and `scan_distance.unwrap_or(32)`, so the `AgentUpdate`s, `unknown_object`'s message and the default distance of `scan` all follow. A raise sends an `AgentUpdate` at once instead of waiting for the next second, logs the new distance and records the time for decision 7. `Sim::new` resets it.

### 5. A wider task throttle while a scan runs

`start_scan` sends `AgentThrottle` again with the task channel at 300 kbit/s; the handshake of the next region sends the base values as before, and a configured `scan_distance` then widens it again at once. At 30 kbit/s the simulator delivers some twenty `ObjectProperties` blocks a second, and OpenSimulator sends a full `ObjectUpdate` for every selected object the avatar does not own on top (`Scene.SelectPrim`), which would put a scan of a busy place past the 30 seconds. 300 kbit/s is below what a viewer asks for at its default bandwidth.

Alternative considered: raising it only while a `scan` command waits. That needs a second message to lower it and a count of waiting scans, for traffic that only flows when there is something to send anyway.

### 6. Where the pieces live

- `src/scan.rs` (new): `Query` parsed from the arguments (distance, texts, owner as `Uuid` or lower-cased full name through `names::full_name`, flag mask, the two switches); `Sighting` (UUID, local id, distance, flags, attachment, child, optional properties); `matches` in two steps, `reachable` (distance, flags, switches) and the text and owner filters; and `listing`, which sorts and formats with `escape::escape`. No session, no clock.
- `Session`: `reach(distance, now)` (refuses when not present, starts the scan, raises the reach, returns the distance in force), `sightings()` (every described prim with a known position, with the distance `thing()` computes), `ask_properties(&[u32], now)`, `settled_at()` and `seat_name()`. Properties are stored by local id and removed in `ObjectsKilled`.
- `Grid` (`src/commands.rs`) gets those four scan methods; `FakeGrid` returns canned sightings.
- `Commands` parks a `Scanning { query, reply, deadline, owners }` and finishes it from `tick`.
- `Bot::control` adds `Request::Scan` to the commands that never use up an override (the `never` list) and hands it to `Commands`; `status_text` takes the seat name through `Whereabouts`.
- `control.rs`: `Request::Scan(Vec<String>)`, passed on unread like `Inventory`, so the control socket and the XMPP contact share one parser in `scan.rs`.

### 7. When a scan answers

`Session::settled_at()` is 3 seconds after the latest of: the start of the scan, the last raise of the reach, and the last time `learn` stored an object it did not have (a new `Instant` in `Sim`). While a `Scanning` waits, `Commands::next_deadline` returns a time 250 ms ahead, and each `tick` does, for each waiting scan:

1. take `sightings()`, keep those `reachable` for the query;
2. hand `ask_properties` the ones without properties that were not asked about in the last 10 seconds (the session keeps the time of asking per local id, so two scans share one request and a lost answer is asked for again);
3. queue the owners not yet looked up in `wanted_names`, once each;
4. answer when `now >= settled_at()`, nothing reachable lacks properties and no owner lookup is open, or when the 30 second deadline has passed.

Owner names come back through `names_resolved` and `names_failed`, which fill an `owners` map the waiting scans share; an owner the service does not know, or a missing service, closes the lookup with no name. Names stay in that map from scan to scan; the entries without a name are dropped once no scan waits, so the next scan asks for them again. Only the owners of prims that meet the filters on name, description and owner UUID are looked up. An owner that is nil or equal to the group is a group, as in `object_owner`.

Alternative considered: a fixed wait of 10 seconds after the scan starts, as `touch` uses. It makes the common case slow (a handful of objects arrive within a second) and still cuts a busy place short.

Alternative considered: session events for "properties arrived" and "settled" instead of the 250 ms poll. More plumbing through `Bot::apply` for no gain at this rate; the poll runs only while a scan waits.

### 8. What is asked, and when

Properties are requested only from step 2 above, so only for prims within the command's distance that meet its flag filters and switches; the text and owner filters cannot narrow the request, as they need the properties. `Session` never asks by itself apart from the seat's name.

## Risks / Trade-offs

- Selecting pauses a physical object the avatar owns, for the time between two messages → accepted, as the viewer's area search does it; the seat is exempt (decision 2). `--physical` does not make it worse: flags are known without selecting.
- Whether the root prim carries `scripted` and `touch` for a script in a child prim differs between simulators → the spec promises only the listed prim's own flags; the README says to add `-c` when `-s` or `-t` misses an object, and the rig run records what OpenSimulator does.
- On OpenSimulator a draw distance that, with the 64 m the simulator adds, reaches a neighbouring region makes it treat that region as in view: it offers it (`EnableSimulator`, which the bot ignores) and, after a teleport there, keeps the region the avatar left as a child agent. The bot keeps no child agents, and a teleport back into such a region ends the session (seen on the rig, and with the unchanged bot under `scan_distance = 64`). For `scan` this needs the distance to be widened in the moment of arrival, as the next region starts at the configured distance again → recorded in `docs/test-rig.md`; keeping child agents, or coming back from the same UDP port, is work of its own and not part of this change.
- Second Life may take longer than 3 seconds to start sending after the reach is raised → the listing is then short and a second `scan` completes it; the 3 seconds are one constant to raise if a run on Second Life shows it.
- A wider throttle means bursts on entering a busy region with `scan_distance` configured → only the cap changes, the data is the same the scan asked for before.
- Names are not refreshed during a stay → stated in the spec and the README; a teleport away and back, or a relog, asks anew.
- A place with thousands of objects within the distance may not be named in 30 seconds → rows show `?`, the request is not repeated for those already answered, so a second `scan` continues where the first stopped; a smaller distance or a flag filter asks for less.
- The `Scanning` poll wakes the main loop four times a second → only while a scan waits, at most 30 seconds each.
- Memory: properties are kept per known object, bounded by `MAX_OBJECTS`, and only for objects a scan reached.

## Migration Plan

Nothing to migrate: no config key, no stored data. `status` gains one line, which a script reading `name: value` lines by name is not affected by.
