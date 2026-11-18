# Proposal

## Why

The control socket can only make the avatar talk. To use the bot as more than a seat-warmer the operator also needs to move it: stand it up, sit it somewhere else, teleport it, bring it to someone or someone to it, and click things. And because the bot always returns to its configured seat, moving it needs a way to tell it to stay put.

## What Changes

New `rslbot ctl` commands:

- `stand`, `unsit` (two names for one command): the avatar stands up.
- `sit <uuid>`: sit on that object. `sit ground`: sit on the ground where the avatar stands.
- `tp <x> <y> [<z>] [<region>]`: teleport. Without a region, inside the current region; without `z`, at the avatar's current height.
- `requesttp <avatar> [<message>]`: ask an avatar to teleport the bot to them. When that avatar answers with a teleport offer, the bot accepts it and goes.
- `accepttp <avatar>`: accept a teleport offer that avatar has sent the bot, and go.
- `sendtp <avatar> [<message>]`: offer an avatar a teleport to the bot's position.
- `touch <uuid>`: click an object.
- `home`: go back to the configured region and seat, and keep the avatar there again.
- `status`: print whether the bot is connected, the region and position, what it sits on, and whether it is holding its seat or was moved by hand.
- `quit`: log out and stop the bot, as a termination signal does.
- `shout <message>`, `whisper <message>`: shorthands for `say /shout ...` and `say /whisper ...`.

Changed behaviour:

- **Manual placement.** Today the bot always works towards "in the configured region, seated on the configured object". `stand`, `unsit`, `sit`, `tp`, `requesttp` and `accepttp` now switch that off: the bot stays logged in, and logs in again after a loss (at the avatar's last location), but no longer teleports back or sits by itself. `home` switches it back on. A freshly started bot holds its seat as before.
- **Teleport offers.** By itself the bot accepts a teleport offer only from an avatar it sent `requesttp` to within `requesttp_timeout` minutes (a new config key, default 10). Any other offer is noted in the operator log and kept, so that the operator can accept it with `accepttp`. A request from an avatar to be brought to the bot is noted in the log too; the operator answers it with `sendtp`.
- **Object scan.** `touch` needs the bot to learn the objects around it, which costs traffic. It does so only from the first `touch` on, within 32 m, or from the moment it enters a region when the new config key `scan_distance` (metres) is set. A bot that is never told to touch sends nothing more than today.
- `rslbot ctl` waits up to 90 seconds for an answer instead of 30, because `tp` and `sit <uuid>` answer only when the simulator has said how they ended.

Assumptions recorded here because the request left them open:

- `<avatar>` is given as for `im`: one argument, `"First Last"`, `first.last`, a single word or a UUID.
- In `tp`, a third argument that is a number is `z`; anything else starts the region name, which may be several words. A region whose name starts with a number therefore needs `z` given.
- "Current height" is the last height the simulator reported for the avatar (on arrival in the region or after a teleport inside it). When none is known, `tp` without `z` is refused.
- `accepttp <avatar>` is read as accepting a teleport *offer* that avatar sent (they asked the bot to come to them). The other direction, an avatar asking to be brought to the bot, needs no new command: `sendtp <avatar>` answers it.
- An offer is remembered until it is accepted or the session ends; whether an old one still works is the grid's decision, and its refusal is reported.
- The first `touch` after entering a region may take up to 10 seconds while the scan runs; later ones answer at once.
- On OpenSimulator the scan distance has little effect: it tells the bot about objects regardless of distance.
- `sit ground` cannot be confirmed by the simulator in a way the bot tracks; `status` reports the avatar as not sitting on an object.

## Capabilities

### New Capabilities
- `avatar-commands`: the movement and interaction commands (`stand`/`unsit`, `sit`, `tp`, `requesttp`, `sendtp`, `touch`), `home`, `status` and `quit`, including which of them suspend automatic placement and how teleport offers are accepted.

### Modified Capabilities
- `presence-reconciler`: the desired state gains a manual mode in which only "logged in" is pursued.
- `object-sit`: a seated avatar may be made to stand, move or sit elsewhere on the operator's command.
- `control-socket`: `ctl` waits 90 seconds for an answer.
- `bot-config`: adds `requesttp_timeout` and `scan_distance` and their validation.
- `message-send`: adds the `shout` and `whisper` commands.

## Impact

- `src/control.rs`: new request variants, argument checks, usage text, longer waits.
- `src/commands.rs`: the new commands, commands that wait for a session outcome, the teleport-request memory.
- `src/reconciler.rs`: manual mode.
- `src/session.rs`: stand, sit on any object, ground sit, teleport on command, teleport offers (send, accept), touch, tracking which objects the simulator has described.
- `src/udp/messages.rs`: `AgentUpdate` control flags, `StartLure`, `TeleportLureRequest`, `ObjectGrab`, `ObjectDeGrab`, `RequestMultipleObjects`; decoding `ObjectUpdateCompressed`, `ObjectUpdateCached`, `KillObject` and teleport offers among instant messages.
- `src/main.rs`: wiring, `quit`.
- `README.md`, `docs/test-rig.md`: the commands, manual mode, rig results.
- `src/config.rs`, `russelbot.example.toml`: the two new keys.
- No extra traffic unless `touch` is used or `scan_distance` is set.
