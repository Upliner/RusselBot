# Proposal

## Why

The config forces every bot to name a region, a position and an object to sit on, although the control socket has made the bot useful without any of them: an avatar that only has to be online, log messages and take commands needs no seat and no fixed place. The key `object` also says nothing about what it is for, and there is no way to ask for a ground sit.

## What Changes

- The key `object` is renamed to `sit`. `object` stays accepted as a deprecated spelling of the same setting; the bot logs a warning when it is used, and rejects a config that has both.
- `sit` accepts the word `ground` besides a UUID: the bot then sits the avatar on the ground instead of on an object.
- `sit` becomes optional. Without it the bot does not sit the avatar anywhere by itself.
- `region` and `position` become optional, as a pair: both or neither.
- New optional key `start`, for configs without `region` and `position`: `"home"` logs in at the avatar's home location, `"latest"` at its last location. Default: `"latest"`. `start` together with `region`/`position` is rejected.
- Without `region`/`position` the bot has no target region: it never teleports by itself, stays in whatever region the login or the operator puts it, and still sits as `sit` says, wherever it is.
- `rslbot ctl home` keeps meaning "hold the configured place again". Without a configured region that is only the configured sit; the avatar is not moved, also not with `start = "home"`.
- Existing configs keep working unchanged. Nothing is **BREAKING**.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `bot-config`: the key table (`sit`, deprecated `object`, optional `region`/`position`, `start`) and the validation rules for them.
- `presence-reconciler`: the desired state and the decision rules depend on which of region and seat are configured; repositioning and the refused-login fallback are restated for configs without a region.
- `object-sit`: the target object comes from `sit`; new behaviour for a configured ground sit and for no configured seat.
- `region-teleport`: what "in the target region" means when no region is configured.
- `grid-session`: the start location the login asks for.
- `avatar-commands`: what holding mode and `home` mean without a configured region or seat; the `seat` line of `status`.

## Impact

- `src/config.rs`: the `Config` model, parsing, validation and tests.
- `src/reconciler.rs`: decision rules take the configured place instead of a region name; new ground-sit action.
- `src/login.rs`: `Start` gains the home location; the start string comes from the configured place.
- `src/main.rs`: carrying out the actions, the log lines that mention the target, `status`.
- `README.md`, `russelbot.example.toml`, `docs/test-rig.md`.
- The user's own config files are not touched; they keep working through the `object` spelling.
- No new dependencies and no protocol messages the bot does not already send.
