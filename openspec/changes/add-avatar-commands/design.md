# Design

## Context

See proposal.md for motivation. What the code looks like today and what that means here:

- `control.rs` knows two requests, `Say` and `Im`. `ctl` joins the message words of those two itself and sends every other command's arguments as separate fields, so new commands need no client changes beyond usage text and the wait.
- `Commands` (`commands.rs`) answers a request at once, or parks an `im` until a name lookup answers. It reaches the session only through the `Grid` trait, which `main.rs` implements in `Live`. Nothing yet waits for a session *outcome* such as "seated" or "arrived".
- `Session` (`session.rs`) can sit only on its fixed `target_object` (`start_sit(now)`), tracks only "seated on the target", and reports `Seated`, `Standing`, `SitFailed`, `TeleportArrived`, `TeleportFailed`, `RegionEntered`. `start_teleport(region, position, now)` already takes any region and position. `present()` refuses while a teleport or logout is under way.
- The session sends an idle `AgentUpdate` every second with control flags fixed at 0, drops every instant message whose dialog is not 0 or 19, and decodes `ObjectUpdate` only far enough to find its own avatar's parent. `ObjectUpdateCached` and `KillObject` are known by name only; `ObjectUpdateCompressed` not at all.
- `reconciler::decide` is a pure function over `Observed` and `Memory` with the rules log in / teleport / sit / nothing. `Bot` (`main.rs`) feeds it every session event.
- The connection thread waits 20 s for the main loop's answer and `ctl` waits 30 s. A teleport may take 60 s (`TELEPORT_LIMIT`), a sit 15 s (`SIT_LIMIT`).
- `references/` holds viewer and OpenSim sources but no `message_template.msg`; the layouts below are from the template as remembered and from those sources, and each is to be checked when implemented (task 1.1).

## Goals / Non-Goals

**Goals:**
- Manual mode is one flag in the reconciler; everything else it does stays as it is.
- Commands that have an outcome (`sit <uuid>`, `tp`) answer with it, using the session's existing deadlines rather than timers of their own.
- `Commands` stays testable with a fake `Grid`; `Session` with the fake `Net`.

**Non-Goals:**
- Walking, flying, turning, autopilot; touching by name or by position; holding a touch.
- Accepting teleport offers by policy (friends, a list in the config). Only answers to `requesttp`, and offers the operator names with `accepttp`.
- Answering an avatar's request to be brought to the bot automatically.
- A manual mode that survives a restart, or a timeout that ends it.
- Tracking a ground sit, or the avatar's position while it is carried by what it sits on.
- Remembering objects across regions or sessions, or anything about them beyond UUID and local number.

## Decisions

### Wire: one field per argument, parsed in the bot
New `Request` variants: `Stand`, `Sit(String)`, `Tp(Vec<String>)`, `RequestTp { to, message }`, `AcceptTp(String)`, `SendTp { to, message }`, `Touch(String)`, `Home`, `Status`, `Quit`, `Shout(String)`, `Whisper(String)`. `Request::decode` maps `stand` and `unsit` to `Stand`, checks argument counts (`tp` 2 or more, `requesttp`/`sendtp` 1 or more, `accepttp` exactly 1, `shout`/`whisper` 1 or more), and joins trailing fields with single spaces where the command takes free text (message, region name). Everything that needs judgement (UUID or `ground`, numbers, z versus region) is parsed in `commands.rs` by pure functions next to `parse_chat`.

`tp` arguments: `x`, `y` must parse as `f32`, be finite and non-negative. If the third field parses as a number it is `z`; the remaining fields are the region. A status text spans lines; the existing escaping carries it and `ctl` prints it as it comes.

*Alternative:* have `ctl` understand each command. It would put the grammar in two places for no gain.

### Waiting for outcomes: `Commands` keeps one pending sit and one pending teleport
`Commands` gains `pending_sit: Option<Sender<Response>>` and `pending_tp: Option<Sender<Response>>`; the latter serves `tp` and `accepttp`. `Bot::apply` passes the relevant session events on: `Seated`/`SitFailed(reason)` settle the pending sit, `TeleportArrived`/`RegionEntered(name)`/`TeleportFailed(reason)` the pending teleport, and a lost session fails both. The session already ends every sit within 15 s and every teleport within 60 s with one of these events, so `Commands` needs no deadline of its own for them. A new `sit <uuid>` while one is pending answers the older one with an error ("superseded"); a new `tp` cannot arrive while one is pending because `present()` refuses during a teleport.

`ANSWER_LIMIT` in `control.rs` goes from 20 s to 75 s and `CLIENT_LIMIT` from 30 s to 90 s.

*Alternative:* answer `tp` with "started" and leave the outcome to the log. A script could then not tell a failed teleport from a good one.

### Session: sit on any object, and say what the avatar sits on
`start_sit` takes the object. `sat_on_target: bool` becomes `seat_confirmed: Option<Uuid>`: the object named in the bot's own request while that request is pending (the simulator may answer with another prim of the linkset, as today), or the object named in an unrequested `AvatarSitResponse`. `SessionEvent::Seated` carries that UUID; `Standing` stays. `Bot` tells the reconciler `seated` only when the UUID is the configured object and `standing` otherwise, so the reconciler's three seat states keep their meaning. An accessor returns the seat for `status`: confirmed object, parented to something unconfirmed (`other`), not parented (`none`), or unknown.

`stand(now)` sends one reliable `AgentUpdate` with `AGENT_CONTROL_STAND_UP` (0x00010000) and drops a pending sit; `sit_ground(now)` sends one with `AGENT_CONTROL_SIT_ON_GROUND` (0x00020000). `messages::agent_update` gets a control-flags parameter; the periodic update keeps sending 0. `sit <uuid>` while seated sends `AgentRequestSit` directly; simulators move the avatar.

### Teleport on command reuses `start_teleport`
`tp` without a region passes the current region's name, which takes the existing same-region path (`TeleportLocationRequest` with the current handle). With a region it takes the lookup path. The default `z` is `sim.position[2]`, which the session sets from `AgentMovementComplete` and `TeleportLocal`; `Grid` gets `position()` and `region()` accessors, also used by `status`.

### Teleport requests and offers
- `requesttp`: `messages::instant_message` gets a dialog parameter; the request is dialog 26 (`IM_TELEPORT_REQUEST`) with the message as text. `Commands` records `asked: HashMap<Uuid, Instant>`.
- Incoming dialog 22 (`IM_LURE_USER`) becomes `SessionEvent::TeleportOffer { from, name, lure }`, where `lure` is the message's `ID` field. `Commands::teleport_offer` looks at `asked`: an entry for `from` no older than `requesttp_timeout` is removed and the offer accepted through `Grid::accept_teleport(lure)`. Otherwise the offer goes into `offers: HashMap<Uuid, Uuid>` (avatar to lure, the latest replacing an earlier one, at most 100 avatars, emptied when the session is lost) and the log says who offered and that `accepttp` would accept. The sender's name from the offer is put into the name cache, so `accepttp "Jane Doe"` needs no lookup.
- `accepttp`: resolves the avatar, takes the entry out of `offers`, calls `Grid::accept_teleport(lure)` and parks its reply as the pending teleport (see "Waiting for outcomes"), so it answers with the arrival or the grid's refusal. No entry: an error, nothing sent.
- The session's `accept_teleport` sends `TeleportLureRequest` and sets its `teleport` state with the usual 60 s deadline, so the arrival is followed and reported like any other teleport. Offers stay out of the TSV log, as the `message-log` spec already says.
- Incoming dialog 26 becomes `SessionEvent::TeleportRequested { from, name, text }` and is only logged.
- `sendtp`: `StartLure` with lure type 0, the message, and one target.
- All three resolve the avatar the way `im` does. `Waiting` in `commands.rs` gets a field saying what to do once the name is known (`Im`, `RequestTp`, `AcceptTp`, `SendTp`) in place of assuming an IM.
- Default messages when none is given: none for `requesttp`; `Join me in <region>` for `sendtp`, as viewers send.
- `requesttp_timeout` is parsed in `config.rs` as a positive number of minutes (integer or decimal) into a `Duration`, default 10 minutes, and handed to `Commands` when it is created.

### Touch: learn objects for free, pay only for a scan
`ObjectGrab` and `ObjectDeGrab` take the simulator's local number, not the UUID, and no message translates one into the other. What the simulator sends about objects falls in two kinds:

- `ObjectUpdate` and `ObjectUpdateCompressed` (High 13) carry both UUID and local number. They arrive today and are thrown away. Reading the pair out of them costs no traffic.
- `ObjectUpdateCached` (High 14) carries only a local number and a checksum, on the assumption that the viewer has the object cached. The bot has no cache; to learn the UUID it must answer with `RequestMultipleObjects` (Medium 3, cache-miss type 0), and the simulator then sends the full update. This is the only part that costs traffic.

So the session keeps, per circuit: `objects: HashMap<Uuid, u32>` from the first kind, and `undescribed: Vec<u32>` from the second, both pruned by `KillObject` (High 16) and capped (50 000 and 20 000 entries; beyond that new ones are dropped). Until a scan starts nothing is sent. A scan (`start_scan`) sends `RequestMultipleObjects` for everything in `undescribed`, in batches of 255, and from then on each `ObjectUpdateCached` is answered as it arrives. `scanning` is reset with the circuit.

When a scan starts:
- `scan_distance` configured: on entering a region. The periodic `AgentUpdate` then also carries that distance as its draw distance instead of the fixed 32 m, from the start.
- not configured: at the first `touch` of an unknown UUID. The draw distance stays 32 m, which is what the simulator has been working with all along.

`Session::touch(object, now)` returns one of: sent (known: grab and de-grab go out, both reliable, zero offset, no surface block); waiting (unknown and no scan had run: the scan starts and the UUID is kept in `wanted` with a 10 s deadline); unknown (a scan has run already). While a UUID is wanted, an update that names it triggers the touch and `SessionEvent::Touched(uuid)`; the deadline produces `SessionEvent::TouchFailed(uuid)`. `Commands` keeps the replies by UUID and settles them from those events, failing them on session loss.

*Alternative:* learn nothing until the scan and ask the simulator to send everything again then. Simulators do not resend what they have already described, so objects seen before the scan would stay unknown. *Alternative:* ask for the object by UUID when the command arrives. There is no such request.

### Manual mode in the reconciler
`Memory` gets `manual: bool`. In `decide`, after the login rule: `if memory.manual { return Wait(None) }`; the login rule itself chooses `Start::Last` when manual. `Reconciler::set_manual(on)`:
- on: clears an in-flight `Teleport` or `Sit` (not a `Login`) without touching the backoffs;
- off: clears `next_teleport`, `next_sit`, `sit_failures` and `reposition`.

The event methods keep updating `Observed` in both modes, so the state is right when holding resumes. Those that settle an action (`teleport_failed`, `sit_failed`, ...) are only called by `Bot` for actions the reconciler started; outcomes of commanded sits and teleports go to `Commands` instead. `Bot` tells them apart by whether `Commands` has a pending sit or teleport.

`Commands` does not own the mode: `handle` returns what `Bot` must do besides answering — set manual, set holding, stop — in a small `Effect` enum, and `Bot` applies it. `status` is assembled in `Bot`, which sees reconciler, session and config.

### Quit
`quit` is answered first, then `Bot::stop()` runs, the same function the signal handler leads to. The socket file is removed on the way out as today.

## Risks / Trade-offs

- [Message layouts written from memory] → Task 1.1 checks every new layout against the template or the OpenSim/viewer sources before any encoder is written; byte-level tests then pin them.
- [Second Life sends most objects compressed or cached; if the compressed layout or the cache-miss request is wrong, `touch` knows nothing there] → Rig first, then a check on Second Life with `--verbose`; `touch` fails with a clear error rather than touching the wrong thing, since it only ever uses a number the simulator gave for that UUID.
- [Answering cache misses makes the simulator send full updates it would have skipped] → Only once a scan has started: after the first `touch`, or from the start when the operator set `scan_distance`. Inside the existing 30 kbit/s task throttle either way.
- [The free part still costs memory: two tables per region even when `touch` is never used] → Capped; a few hundred kilobytes in a crowded region. Traffic, which is what the operator asked to protect, is unchanged.
- [OpenSimulator ignores the draw distance for object updates, so `scan_distance` cannot be shown to limit anything on the rig] → Verify there that the distance is sent; its effect needs Second Life.
- [An old teleport offer accepted with `accepttp` may have expired on the grid] → The grid's failure is the command's answer.
- [The OpenSim rig cannot show a touch arriving in a script or a ground sit to a second viewer without help] → Extend `test-rig/listen.py` where possible (teleport requests and offers, seeing the bot sit and stand); what cannot be seen on the rig is listed as such in `docs/test-rig.md`, not claimed.
- [`tp` without `z` after the avatar was carried by a vehicle or moved by a script uses a stale height] → Documented; give `z`.
- [A teleport offer accepted on request takes the bot wherever that avatar is] → Only an avatar the operator named, only for 10 minutes, once.
- [An operator forgets the bot in manual mode; after a region restart it logs in at its last location and stays] → `status` shows the mode, the log notes every change, `home` fixes it.

## Open Questions

- Whether `sit ground` should count as `seat: ground` in `status`. It would need the avatar's animations decoded; left out until someone needs it.
