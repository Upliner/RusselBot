# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- `Config` carries `region: String`, `position: [f32; 3]` and `object: Uuid`, all required (`src/config.rs`). They are read in four places outside the config: the login start string (`src/login.rs:63`), the teleport and sit actions and the `status` and `Seated` handling in `src/main.rs`, and `Reconciler::new(&config.region)`.
- `decide` in `src/reconciler.rs` is a pure function of `Observed`, `Memory` and the target region name. Its rules assume a region and an object always exist. `Start` is `Target` or `Last`; a refused login and manual mode already fall back to `Last`.
- The session can sit on the ground (`Session::sit_ground`, used by `ctl sit ground`), but the simulator does not confirm it: the avatar's parent stays 0, so `Session::seat()` reports `Seat::None` (`src/session.rs:135`). The session emits `Standing` whenever the simulator reports parent 0, which includes the first report after entering a region.
- A login string of `home` or `last` is what viewers send; the bot already sends `last`.

## Goals / Non-Goals

**Goals:**

- One place in the code that says what the config wants: where to log in, which region to hold if any, what to sit on if anything.
- Decision rules that stay a pure, table-testable function.
- Existing config files parse to exactly the behaviour they have today.

**Non-Goals:**

- Teleporting to the home location. `start = "home"` only chooses the login location; `ctl home` does not go there (see the `avatar-commands` delta). A "teleport home" command can be added later on its own.
- Observing a ground sit. The bot does not read avatar animations, so it cannot tell whether the avatar still sits on the ground and does not show it in `status`.
- Changing commands, the control socket or message logging.
- Rewriting the user's untracked config files.

## Decisions

### Config model: a place and an optional seat

`Config` drops `region`, `position` and `object` and gains:

```rust
pub enum Place {
    Region { name: String, position: [f32; 3] },
    Home,
    Latest,
}
pub enum Sit {
    Object(Uuid),
    Ground,
}
pub struct Config { pub place: Place, pub sit: Option<Sit>, pub deprecated_object: bool, /* ... */ }
```

The three ways to start are mutually exclusive in the file, so one enum makes the invalid combinations unrepresentable after parsing. Alternative considered: keeping `region: Option<String>`, `position: Option<[f32; 3]>`, `start: Start` side by side — every reader would have to re-check the pairing.

`deprecated_object` lets `Config::parse` stay pure; `run_bot` logs the warning once after the config loads. Alternative considered: logging from inside `parse` — it would also fire in tests and in any future caller.

`commands.rs` has its own `SitTarget::{Ground, Object}` for `ctl sit`. The config type stays separate: the command's parser reports errors for a socket client, the config's for a key in a file, and the two types would otherwise couple the config module to the command module for two variants.

### `object` stays as a deprecated spelling, UUID only

Existing configs, including the user's two untracked ones, keep working. `object` does not accept `ground`: the new value belongs to the new key, and an old key with new values would have to be documented twice. Both keys together are an error naming both, like `password` and `password_hash`.

Alternative considered: dropping `object` outright. It is a one-line edit for the operator but an unannounced break for a bot that restarts unattended; the deprecation line costs nothing.

### `start` accepts exactly `home` and `latest`

`latest` is the word asked for; on the wire it becomes the protocol's `last`. `last` is not accepted as a second spelling: one name per value keeps the table short, and the error for an unknown value lists the two accepted ones, so a user who writes `last` is told what to write.

`start` with `region`/`position` is rejected rather than ignored: a config that names both a region and `start = "home"` has two contradicting answers to "where does the avatar log in".

### The reconciler takes a goal, not a region name

```rust
pub struct Goal {
    pub region: Option<String>,   // hold this region, or none
    pub start: Start,             // Target, Home or Last
    pub seat: Option<SeatGoal>,   // Object or Ground
}
```

`Reconciler::new(goal)` replaces `Reconciler::new(&config.region)`; `main.rs` builds the goal from the config. `Start` gains `Home`, and `login::start_location` maps it to `home`. `decide` follows the six rules of the `presence-reconciler` delta:

- Login start: `Last` when the last login was refused or the mode is manual (unchanged), otherwise `goal.start`.
- Teleport rule: only with `goal.region` set. `in_target_region()` is true for any region when none is configured, so every caller keeps working.
- Reposition (`memory.reposition`) leads to a teleport only with `goal.region` set; without one, three failed sits just continue under the sit backoff. There is no position to go back to.
- The "credentials rejected" exit needs no change: it already fires when a first-ever login asking for `Last` is refused, which with `start = "latest"` is the first attempt.

### Ground sit: a new action that finishes at once

`Action::SitGround` joins `Login`, `Teleport` and `Sit`. `main.rs` carries it out with `Session::sit_ground` and immediately tells the reconciler it is done; it never becomes the in-flight action, has no backoff and cannot fail from the reconciler's point of view (the session refuses it only when not in a region, and the rule applies only while in one).

`Memory` gains `ground_done: bool`. It is cleared by `region_entered`, `teleport_arrived`, `set_manual(false)`, and by `standing` when the avatar had been reported seated on an object since the last ground sit. The last condition matters: the session reports `Standing` for the first parent-0 update after every arrival, and clearing on every `Standing` would send each ground sit twice. So the reconciler remembers "seated on some object" (`Seated` events for any object, which `main.rs` today turns into `standing()` for non-target objects) and clears `ground_done` only on a `Standing` that follows it.

Alternative considered: treating the ground sit as a `Sit` with a fake confirmation. That would run it through `seated_since`, the sit backoff and `Seat::SeatedOnTarget`, all of which describe something the simulator said — and it said nothing.

Alternative considered: confirming through `AvatarAnimation`. That is the only way to really know, but it adds a decoder and a table of animation IDs for a state the bot then could not repair any better than by sending the same request again.

### Log lines and `status`

"(the target region)" / "(not the target region)" and "returning to the configured region and seat" are worded from the goal: with no region configured the region is just named. `status` prints `seat: target` only when `config.sit` is that object; with `Ground` or nothing configured the comparison is never true and the other values are unchanged.

## Risks / Trade-offs

- [A ground sit sent right at region entry could be ignored by a simulator that is not ready for it] → Object sits are already sent at the same moment and work on OpenSim and Second Life. Verified on the rig in task group 5; if it turns out to be dropped, the rule can wait for the first avatar update (`Seat::Unknown` gone) without a spec change.
- [The bot cannot see a ground sit end: if something stands the avatar up without moving it, it stays standing] → Accepted and documented in the README; `ctl home` or `ctl sit ground` sits it again.
- [With a seat object but no region, the object may not be where the avatar logs in; the bot then retries the sit once a minute forever] → That is the configured wish and each failure is logged with its reason. Documented next to `sit` in the README.
- [`rslbot ctl home` does not go to the home location with `start = "home"`, which the names suggest] → Stated in the README's command table and the example config.
- [With `start = "latest"` a wrong password ends the bot after one attempt instead of two] → Same outcome as before, one request sooner; covered by a scenario.

## Migration Plan

No migration is needed: every config valid today stays valid and behaves the same, with one extra log line for `object`. Operators switch to `sit` at their leisure. Rollback is reverting the commit; a config already using `sit`, `start` or no `region` would then be rejected at startup with the key named.
