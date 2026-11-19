# Design

## Context

See proposal.md for the motivation. What shapes the approach today:

- `src/reconciler.rs` decides from a fixed `Goal { region, start, seat }` built once from the config (`goal()` in `src/main.rs`). `Reconciler::set_manual` is the only thing that changes how it behaves at runtime. The teleport destination and the sit object are not in the goal; `Bot::reconcile` reads them from `self.config` when it carries an action out.
- `Bot::apply` judges "seated on the target" by comparing `SessionEvent::Seated(object)` with `config.sit`, and `status` does the same.
- `Session::position()` is set only by `AgentMovementComplete` and `TeleportLocal`. Object updates are decoded up to `ParentID` (`decode_object_update`), and the `ObjectData` blob with the position is skipped. `ImprovedTerseObjectUpdate` is not decoded at all.
- `Session` keeps `seat_confirmed = sit_request` when the simulator answers the bot's own sit: the prim the simulator names in `AvatarSitResponse` is only logged.
- `Commands` (`src/commands.rs`) lives as long as the bot and already tracks command state across sessions; `Request::Home` is handled in `Bot::control`, not in `Commands`.
- `login::start_location` takes the region and position from the config.
- The bot's selling point is a few megabytes of memory. `Sim::objects` holds up to 50 000 UUID-to-local-id pairs and nothing else per object.

Three points were settled with the user before this design: the location form is `<region>/<x>/<y>/<z>`; a `cur` location while seated is the true seated position, not the last standing one; `persist = "off"` places the avatar once as configured and then goes manual.

## Goals / Non-Goals

**Goals:**

- One place that owns "what the bot holds" and everything `cur`/`cfg`/`last`/`last_cmd` stand for, testable without a session.
- The reconciler stays a pure decision function with the same rules; it only stops assuming its goal is constant.
- Position tracking that costs constant memory and one small packet per sit.
- `persist = "cfg"` with a full config (`region`, `position`, `sit`) behaves as the bot does today.

**Non-Goals:**

- Writing the held placement anywhere: it is not saved to the config or to disk and ends with the process. "Persist" means "keep putting the avatar back", not "survive a restart".
- Walking or teleporting the avatar back to the held coordinates inside the held region.
- Telling a ground sit from standing by what the simulator reports (animations). `cur` uses what the bot last asked for.
- A location form without a region (`x/y/z`), full SLURLs, or map URLs.
- Recording `accepttp`/`requesttp` destinations for `last_cmd`.

## Decisions

### 1. A pure `persist` module owns the placement state

New `src/persist.rs`, no I/O, in the style of `reconciler.rs`:

- `Location { region: String, position: Vector3 }`, `SitState { Object(Uuid), Ground, Stand, Unchanged }`, `Placement { location: Option<Location>, sit: SitState }`. `location` is `None` only for "config has no position and no login has reported one yet".
- `LocationArg { Cur, Cfg, Last, LastCmd, At(Location) }` and `SitArg { Cur, Cfg, Last, LastCmd, Is(SitState) }` with `parse_location` / `parse_sit_state`. A location is split from the right on `/`: exactly four parts, the last three numbers. Formatting (`Display`) is the inverse, rounded to 0.01 with trailing zeros dropped, and is used for the command's answer, the `status` line and the log.
- `Book`: the config placement, the first location, the last persisted placement, the held placement, and a bounded list (64, most recent first) of `(object, region)` pairs the avatar was seated on.
- `Book::resolve(location, sit, &dyn World, last_tp, last_sit) -> Result<Placement, String>` does the special-word table and the region check and changes nothing; `Book::commit(placement)` makes it held and `last`. `World` is a small trait over the live session (`here() -> Option<Location>`, `seat()`, `on_ground()`, `knows(Uuid)`), absent while not connected, with a fake in tests. This mirrors how `commands.rs` uses `Grid`.

Alternative considered: put the state into `Reconciler`. Rejected: the reconciler needs only "which region" and "which kind of seat", and mixing in argument parsing and `last`/`last_cmd` memory would end its being a small pure state machine.

Alternative considered: put `persist` into `Commands::handle`. Rejected for the same reason `home` is not there: it changes the bot's mode and goal, which `Commands` does not own. `Commands` only gains `last_tp()` and `last_sit()`, recorded where `went_manual` is set in `tp()` and `sit()`.

### 2. The reconciler's goal becomes settable, its shape stays

`Goal { region, start, seat }` is kept. `Bot` derives it from the held placement: `region` from the location; `start` is `Start::Target` when a location is held and the config's `start` otherwise; `seat` is `Some(Object)`, `Some(Ground)` or `None` for `stand` and `unchanged`. New `Reconciler::set_goal(goal, seated_on_target: bool)` replaces the goal and re-judges `observed.seat` from what the session says now, so that persisting the current seat does not trigger a sit request (object-sit, "Current seat becomes the target"). The body of the `!manual` branch of `set_manual` (reset backoffs, failure count, `ground_done`) moves to a `rearm()` that `persist` calls whether or not the bot was manual.

The request said `persist` "does `Bot::set_manual(false)`". `Bot::set_manual` returns early when the mode is unchanged, which would leave a pending teleport backoff in place when a persist changes the location while holding. So `Bot::persist` sets the goal, calls `rearm()`, and then `set_manual(false)` for the mode and its log line.

Rule 5 (ground) gains "and the avatar is not seated on an object" by testing the existing `on_object` flag. Together with a stand request sent by the persist, this gives stand, then `Standing`, then the ground sit, instead of a ground-sit flag sent to a seated avatar, which simulators ignore.

`stand` needs no reconciler rule: it is a one-off at the moment the persist is carried out (`Session::stand` when seated on an object or `on_ground()`), which is all the spec asks.

Rule 3 and the reposition rule lose their "only with a configured region" special case in the tests rather than in the code: a location is always held once a region has been entered, because `Bot` fills a `None` location on the first `RegionEntered` before the reconciler is asked.

### 3. The config's `persist` is applied by `Bot`, once

`Config.persist: Persist` (`Off` or `On { position: Source, sit: Source }`, `Source` = `Cur | Cfg`), parsed from an untyped `toml::Value` like the other optional keys so that a wrong type is reported with the key's name. `Bot.startup: Option<Persist>` is `Some` until applied or cancelled.

After each batch of session events (`service_session`) the bot checks whether the config placement is reached: holding mode, session present, in the target region, and by config `sit`: reconciler seat is `SeatedOnTarget`; or the ground sit has been started; or nothing. When the location source is `cur` and the avatar is seated, it additionally waits until the session has the seat's frame (Decision 5) or 10 s have passed since `Seated`; that deadline joins the main loop's wake-up times. Then it runs the same `Bot::persist` path as the command (so `last`, the log line and the region check are shared), or `set_manual(true)` for `Off`.

Cancelling: `Bot::control` clears `startup` when a `persist` or `home` is carried out, and `follow_commands` clears it when a command went manual.

Alternative considered: start in manual mode and treat startup placement as a scripted sequence of commands. Rejected: the reconciler already does login, teleport, sit with retries and backoff; a second path would duplicate it and lose the retries.

### 4. The session reports the prim the simulator named

`Session` keeps both `seat_requested` (what `seat_confirmed` is today) and `seat_prim` (the `AvatarSitResponse` object). `SessionEvent::Seated { prim, requested }` and `Seat::Object { prim, requested }` carry both. "On target" in `Bot` is `prim == target || requested == Some(target)`, which keeps today's linkset rule for a `cfg` child prim and lets a `cur` persist hold the named prim. `status`, the `sit` command's answer and the `(object, region)` list use `prim` (and record `requested` too for the region check).

`Session` also gets `on_ground: bool`, set by `sit_ground`, cleared by `stand`, on opening a circuit, on `TeleportLocal`, and when the avatar becomes parented. It serves only `cur` and the one-off stand.

### 5. Position: follow the avatar, and keep one seat frame

Decoding added in `src/udp/messages.rs`:

- `ObjectUpdate`: read the `ObjectData` blob instead of skipping it. 60 bytes: position at 0, rotation (three floats, w derived) at 36. 76 bytes (avatars): the same after a 16-byte collision plane. Other lengths (the quantized 48/32/16-byte forms) yield no motion.
- `ObjectUpdateCompressed`: position and rotation sit at fixed offsets (40 and 52) behind FullID, LocalID, PCode, State, CRC, Material, ClickAction and Scale.
- `ImprovedTerseObjectUpdate` (High 15, already named in the table): LocalID, State, IsAvatar, an optional collision plane, position as three floats, then quantized velocity, acceleration, rotation (four U16 mapped to -1..1) and angular velocity.

Each decoded object becomes `{ full_id: Option<Uuid>, local_id, parent_id: Option<u32>, motion: Option<Motion> }`; terse updates have no UUID and no parent. The layouts are to be checked against `references/kokua/indra/newview/llviewerobject.cpp` and OpenSim's `LLClientView` before coding, since they are from memory here.

In `Session`: the avatar's own local id is learned from its full update (`full_id == agent_id`). Updates for that local id set `own_offset`; when `own_parent` is 0 that is the region position and goes straight into `sim.position`. When the avatar becomes parented to local id `P`, the session clears `seat_frame`, sends one `RequestMultipleObjects` for `P` through the existing `request_objects`, and from then on any update for `P` that carries a motion sets `seat_frame`. `position()` while parented is `frame.position + frame.rotation * own_offset` once the frame is known, and stays at the last value before that. `P` is the linkset's root, which is what avatars are parented to, so child-prim seats need nothing extra.

Alternative considered: remember position and rotation for every object in `Sim::objects`. Rejected: up to 50 000 × 28 bytes for a value needed for one object, and it would still not cover objects the simulator only mentions as cached, so the request would be needed anyway.

Alternative considered: use `SitPosition` from `AvatarSitResponse`. Rejected: it is the offset on the seat, the same information as the avatar's own update, and still needs the seat's frame.

The request is sent whether or not a `touch` scan runs, which is why the `avatar-commands` "Object scan" requirement is modified. It is one reliable packet per sit.

### 6. Wire format and `home`

`Request::Persist { location: Option<String>, sit: Option<String> }`, decoded from zero to two fields; the bot judges the values, as it does for `sit` and `tp`. `request_line` passes `ctl` arguments through one per field, so a region name with spaces must be quoted, and an unquoted one fails with the argument-count error. `Request::Home` stays a variant and `Bot::control` runs it as `persist(Last, Last)`; its answer now carries the placement text. `CTL_USAGE` gains the command and a paragraph on the special words.

### 7. Login start

`login::start_location` and `login::login` take the held `Option<&Location>` instead of reading `cfg.place`; `Start::Target` formats it as `uri:<region>&<x>&<y>&<z>` as today. `Place` in the config is used only to build the config placement and the pre-location `start`.

## Risks / Trade-offs

- [The new default holds a simulator-reported spot instead of the configured one, so an existing bot behaves differently after an upgrade without a config change] → Called out as BREAKING, documented in the README and the example config next to `persist = "cfg"`, which restores today's behaviour for full configs. The startup log line states the placement that was persisted.
- [A config without `region` used to mean "never teleport"; now the bot returns to where it settled] → Same documentation; `persist = "off"` is the setting for a bot that should not move itself.
- [A seated position can be a poor arrival point: inside furniture, or on a parcel that routes arrivals to a landing point] → Simulators adjust arrival positions themselves; the sit is attempted from wherever the avatar lands, and the reposition rule still applies. `persist = ["cfg", "cur"]` keeps a hand-picked position with the reported seat.
- [A simulator sends the avatar's or seat's update in a quantized form, or does not answer the seat request] → The position stays the last one known, which is where the sit was made from and therefore a working return point; the startup persist goes ahead after 10 s and says so in the log. Verified on OpenSim in the rig; Second Life needs a manual check, recorded as not verified until done.
- [The layouts of compressed and terse updates are easy to get wrong, and a wrong offset yields plausible garbage] → Unit tests with encoded samples for each form; decoded positions that are not finite are dropped; the rig's `listen.py` prints where it sees the bot for comparison.
- [The region check can refuse a valid persist when an object has moved to another region since the bot sat on it] → Sitting on it again replaces the remembered region; the error names both regions so the cause is visible.
- [`home` can now fail (region check) where it could not before] → Only when the bot itself has evidence that the pair is inconsistent; the error says so, and `persist` with explicit values is the way out.
- [`rearm()` on every persist lets an operator bypass backoff by repeating `home`] → Operator-driven and rate-limited by the operator; the automatic paths keep their backoff.

## Migration Plan

No file migration: existing configs parse unchanged. Operators who want the old behaviour add `persist = "cfg"` (full configs) or `persist = "off"` (bots that must not move themselves). Rolling back is running the previous binary; nothing is stored between runs.
