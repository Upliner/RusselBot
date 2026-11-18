# Proposal

## Why

Keeping an avatar seated on an object in Second Life or OpenSimulator today means running a heavyweight .NET/Mono bot (Corrade, SecondBot) for a job that needs a tiny fraction of the protocol. The repository is an empty Rust crate; this change delivers the first working version described in `README.md`: a small single-binary bot that logs in, gets to a target region, sits on a target object, and stays there.

## What Changes

- Add a single TOML config file holding grid login URL (Second Life Agni by default), user name, password or password hash, target region name and coordinates, and the UUID of the object to sit on.
- Add a `hash` subcommand that turns a plain password into the hash form the config accepts, so the plain password never has to be stored.
- Add grid login and a simulator session that answers everything the simulator needs to keep the avatar online, and ignores the rest of the traffic.
- Add teleport to the target region when the avatar is somewhere else.
- Add sitting on the target object, and detection of being unseated.
- Add a reconciler that continuously compares observed state (connected? right region? seated?) with the desired state and takes the next corrective action, with backoff. It recovers from kicks, region restarts, network loss, failed teleports and being unseated without operator action. It exits only on errors retrying cannot fix (bad config, rejected credentials) or on an operator stop signal, when it logs out cleanly.
- Add a documented local OpenSim test rig (two regions, a bot user, a seat prim) used to verify every behaviour, including injected failures.

Out of scope for this change: multi-factor login, verification against the live Second Life grid, running several bots in one process, chat/IM/inventory/appearance, and any remote control interface.

## Capabilities

### New Capabilities

- `bot-config`: the config file format, its validation, defaults, and the password `hash` subcommand.
- `grid-session`: logging in to a grid, keeping the simulator connection alive, noticing when it is lost, and logging out cleanly.
- `region-teleport`: recognising which region the avatar is in and teleporting it to the configured region and coordinates.
- `object-sit`: sitting the avatar on the configured object and tracking whether it is still seated there.
- `presence-reconciler`: the control loop that drives login, teleport and sit from observed state, including retry, backoff, fatal-error and shutdown policy.

### Modified Capabilities

None. The project has no existing specs.

## Impact

- **Code**: `src/main.rs` is a "Hello, world!" stub; all bot code is new. `Cargo.toml` gains its first dependencies.
- **Toolchain**: the crate pins Rust 1.100 through `rust-toolchain.toml` so builds do not depend on the machine's default toolchain.
- **External systems**: talks to a grid login server over HTTP(S), to simulators over UDP, and to simulator capabilities over HTTP(S). Verified in this change against a local OpenSim only.
- **Test environment**: `references/opensim` is an unbuilt, unconfigured source checkout and the .NET 8 SDK it needs does not appear to be installed; building and configuring it is part of this change's tasks.
- **Documentation**: `README.md` gains usage, config reference and test-rig instructions.
