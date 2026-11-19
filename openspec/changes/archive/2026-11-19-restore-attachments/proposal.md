# Proposal

## Why

On Second Life an attachment can be gone from the avatar after a login although nothing took it off. In the hand checks of 2026-11-19 (README.md, Status) it happened each time after another object had been attached to the same point and detached again: the object was reported at two logins, then the other object came and went, and at the next login the simulator did not show the first one any more. A viewer hides this, because at every login it compares the Current Outfit folder with what is on the avatar and asks the simulator for what is missing. The bot keeps that folder up to date and never reads it back, so an avatar run by the bot alone loses attachments for good.

Two smaller gaps came up in the same runs and feed the first: the simulator took 21 seconds to carry out an `attach`, the bot gave up after 15 and answered that it had failed, and the object that then arrived got no link in the Current Outfit folder. Without that link nothing, the bot included, knows the object is meant to be worn.

## What Changes

- After each login, 20 seconds after the avatar has entered a region, the bot reads the Current Outfit folder and asks the simulator to attach every object the folder lists that is not on the avatar, each to the point it was last on and without taking anything off. It does this once per login, only where the simulator offers the outfit service, and notes in the operator log what it asked for and what came of it.
- New config key `restore_attachments`, a boolean, on by default. With `false` the bot does none of the above and reads nothing from the inventory without a command, as today.
- **BREAKING** (behaviour): with the default, a bot on Second Life now asks the grid for two inventory folders after every login without having been given a command, and may attach objects by itself. An object taken off by something that does not update the Current Outfit folder (a script, or a `detach` whose folder update failed) comes back at the next login.
- `attach` waits 30 seconds for the simulator instead of 15 before it answers with an error. `detach` keeps its 15.
- An object the bot asked the simulator to attach, and that arrives after the command was already answered as failed, still gets its link in the Current Outfit folder, and the log says it arrived late.
- README (config table, Attachments, Status) and `docs/test-rig.md` are brought in line. The local OpenSim offers no outfit service, so the option does nothing there, and the loss was not reproduced there either (`docs/test-rig.md`, item 105); README will say that it can be left on or switched off on such a grid without a difference.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `avatar-outfit`: new requirement "Restoring attachments after a login"; "Attach command" waits 30 seconds; "Current Outfit folder follows attachments" covers an attach that arrives late.
- `inventory-list`: "Inventory is read on demand" gets one exception, the reading of the Current Outfit folder after a login while `restore_attachments` is on.
- `bot-config`: the key `restore_attachments` in "Single TOML config file" and its validation in "Config validation before any network activity".

## Impact

- `src/config.rs`: the new key.
- `src/commands.rs`: the restore itself (it already owns the inventory jobs and the Current Outfit notes), late arrivals, the log lines.
- `src/inventory.rs`: a job that returns the objects the Current Outfit folder links to.
- `src/session.rs`: the attach limit apart from the detach limit.
- `src/main.rs`: passing the option and the moment of region entry after a login to the commands.
- `README.md`, `docs/test-rig.md`, `russelbot.example.toml`.
- No new dependency and no new command. Traffic with the option on: two folder listings and, when something is missing, one item lookup and one attach request per object, once per login.
- Not verifiable on the rig beyond "does nothing without the outfit service"; the proof is a hand check on Second Life.
