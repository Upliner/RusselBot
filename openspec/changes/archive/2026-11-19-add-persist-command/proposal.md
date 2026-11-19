# Proposal

## Why

What the bot holds, the auto-return location and the seat, is fixed by the config file: `home` can only go back to `region`/`position`/`sit`, and an avatar the operator has moved and seated somewhere else by command cannot be told "stay here now" without editing the config and restarting. A config without `region` gives no auto-return at all, and a configured `position` is kept even when the avatar actually ended up somewhere else.

## What Changes

- New control command `persist [<location>] [<sit_state>]`: sets the placement the bot holds (an auto-return location and a sit state) at runtime and puts the bot in holding mode.
  - `<location>` is `<region>/<x>/<y>/<z>` (SLURL style, one argument), e.g. `"Blue Lagoon/128/128/25"`.
  - `<sit_state>` is an object UUID, `ground`, `stand` (stand up if sitting) or `unchanged` (leave sitting and standing alone).
  - Both also take `cur` (what the simulator reports now), `cfg` (the config's `position`/`sit`; without them the first location the first login reported, and `stand`), `last` (the last persist, else as `cfg`) and `last_cmd` (the last `tp` destination / the last `sit` command's target, else as `last`). Both default to `cur`.
  - The command fails, changing nothing, when the sit object is known to be in another region than the location.
- The bot reacts to a region change by teleporting to the held location, at once if the avatar is already elsewhere. Inside the held region it does not react to movement; only a held object seat is re-taken when the avatar is unseated, as today.
- `home` becomes an alias for `persist last last`.
- New config key `persist`: `"off"`/`false`, `"cur"`, `"cfg"`, an array `[<position>, <sit>]` of `"cur"`/`"cfg"`, or a table `[persist]` with `position` and `sit`. After the bot has reached the configured placement at startup (logged in, and seated when `sit` is set), it carries out the matching `persist`; with `off` it goes to manual mode instead.
- The bot follows the avatar's position after arrival, including the true region position while seated on an object (seat position and rotation plus the avatar's offset), so that `cur` means where the avatar is. `status` and `tp` without `z` use the same position.
- The seat the bot reports and takes for `cur` is the prim the simulator actually seated the avatar on.
- `status` gains a `persist` line showing the held placement.
- **BREAKING**: the default is `persist = "cur"`. After startup the bot holds the location and seat prim the simulator reported, not the configured `position` and `sit` UUID. Add `persist = "cfg"` to keep holding the configured values.
- **BREAKING**: a config without `region` now has an auto-return location too: the place the bot settled at (`cur`) or the first login's location (`cfg`). The bot teleports back there when the avatar is moved to another region, logs in there after a session loss, and repositions there after three failed sits. `persist = "off"` gives a bot that never moves the avatar by itself after startup.
- **BREAKING**: `home` returns to the last persisted placement rather than to the config; `persist cfg cfg` is the command that returns to the config.
- **BREAKING**: becoming seated on an object makes the bot ask the simulator to describe that one object, also when no `touch` scan is running.

## Capabilities

### New Capabilities
- `placement-persist`: the placement the bot holds (location and sit state), the `persist` command with its value forms and special values, what `last` and `last_cmd` remember, the region check, and how the config's `persist` setting is applied after startup.

### Modified Capabilities
- `avatar-commands`: `home` becomes an alias for `persist last last`; holding mode is defined by the held placement and is entered by `persist`; `status` shows the held placement and judges `seat: target` against it; `requesttp` waits also end on `persist`; the object scan rule allows the single request about the seat.
- `bot-config`: new key `persist` with its forms and validation; `region`/`position`/`start`/`sit` describe the startup placement.
- `presence-reconciler`: the desired state comes from the held placement instead of the config; logins in holding mode ask for the held location; repositioning after failed sits goes to the held location; `persist` clears backoff delays.
- `object-sit`: the target object is the held sit state's object; the seated state reports the prim the simulator named; ground sits and "no sit" follow the held sit state.
- `region-teleport`: the target region is the held location's region; teleports go to the held location; new requirement to know the avatar's current position, also while seated.
- `grid-session`: the login request asks for the held location when one is known, and for `start` otherwise.

## Impact

- `src/config.rs`: `persist` key (string, boolean, array or table) and its validation; `russelbot.example.toml`.
- New `src/persist.rs`: value parsing, resolution of `cur`/`cfg`/`last`/`last_cmd`, the region check, formatting.
- `src/reconciler.rs`: goal replaced by a changeable placement; sit states `stand` and `unchanged`; ground sit waits while on an object.
- `src/main.rs`: holds the placement state, applies the config's `persist` once after startup, carries out `persist`/`home`, `status` line, login start from the held location.
- `src/control.rs`, `src/commands.rs`: `persist` request and usage text; remembering the last `tp` destination and `sit` target.
- `src/session.rs`, `src/udp/messages.rs`: decoding of object positions and rotations from object updates (full, compressed, terse), own-avatar position tracking, the request about the seat, the prim actually sat on, the ground-sit belief.
- `src/login.rs`: start location from the held location instead of the config.
- `README.md`, `docs/test-rig.md`, `test-rig/listen.py` (to check positions).
- No new dependencies. Existing configs keep parsing; their behaviour changes as listed under BREAKING.
