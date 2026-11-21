# Proposal

## Why

`touch`, `sit` and `persist` take an object's UUID, and the only way to find one today is to log in with a viewer and look. The bot already learns the objects around the avatar for `touch` and for the RLV restrictions, but it cannot show them. `status` likewise prints the seat as a bare UUID, or as `target`, which does not tell the operator what the avatar is sitting on.

## What Changes

- New control-socket command `scan [<distance>] [filters]`, modelled on a viewer's area search: it lists the objects within `<distance>` metres of the avatar, one line each, with the UUID, the distance, flags, the owner (UUID and name), the name and the description, nearest first.
- `<distance>` defaults to the distance the bot's object scan covers in the region (`scan_distance`, or 32 m). A larger one raises that distance until the avatar leaves the region, so that the simulator describes the objects out to it; `touch` then reaches that far too.
- Filters, all of which can be combined:
  - `-n`/`--name TEXT` and `-d`/`--desc TEXT`: name or description contains TEXT, whatever the case.
  - `-o`/`--owner WHO`: owned by that avatar (name or UUID) or that group (UUID).
  - `-s`/`--scripted`, `-t`/`--touch`, `-p`/`--physical`, `--temporary`, `--phantom`: only prims the simulator flags so.
  - `-a`/`--attachments` and `-c`/`--children`: also list what avatars wear, and the child prims of linksets. Without them only the root prims of objects in the world are listed, as in the viewer.
- To name an object the bot asks the simulator for its properties the way the viewer's area search does, and looks the owners' names up through the name service it already uses for the message log. Both are remembered for the stay in the region. The bot asks only when a `scan` command is given.
- `scan` starts the object scan in a region where none is running, and for as long as a scan runs the bot lets the simulator send object data faster than the trickle it allows otherwise.
- `status` gets a line `seat name: <name>` while the avatar is seated on an object, with the name of that object. For that the bot asks the simulator one more question each time the avatar becomes seated.
- `scan` reads only: it does not move the avatar, does not change the placement mode, is not held to RLV restrictions and does not use up an RLV override.

Nothing is removed and no existing command changes its answer, apart from the added `status` line.

## Capabilities

### New Capabilities
- `area-scan`: the `scan` command: which objects it lists, the distance, the listing format, the filters, how long it waits for the simulator, and what it asks the simulator and the name service.

### Modified Capabilities
- `avatar-commands`: "Object scan" (a `scan` command starts the scan too and can raise its distance for the stay in the region; one more request about the seat, for its name) and "Status command" (the `seat name` line).

## Impact

- `src/udp/messages.rs`: object updates keep the kind of object and its flags; new `ObjectSelect` and `ObjectDeselect`; new decoding of `ObjectProperties`; `ObjectPropertiesFamily` keeps the name.
- `src/session.rs`: per-object kind, flags and properties; the raised distance and throttle of a running scan; the seat's name; what a `scan` reads out.
- `src/control.rs`: `Request::Scan`, parsing, `CTL_USAGE`. `src/xmpp/command.rs` takes the command through the same parser.
- New `src/scan.rs` for the arguments, filters and listing; `src/commands.rs` for the waiting command and the owner-name lookups; `src/main.rs` for the wiring and `status`.
- `README.md`, `russelbot.example.toml` (the `scan_distance` comment), `docs/test-rig.md` and a rig helper.
- Traffic: none added until `scan` is used, except one small request per sit. A `scan` selects and deselects the objects it lists once, which the simulator answers with their properties; on OpenSimulator it also sends each such object's full description again.
- No new dependencies, no config keys, no change to stored data.
