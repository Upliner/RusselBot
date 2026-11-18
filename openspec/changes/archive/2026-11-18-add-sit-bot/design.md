# Design

## Context

See `proposal.md` for motivation. The crate is a "Hello, world!" stub with no dependencies. Protocol knowledge comes from the checkouts under `references/`:

- `message_template.msg` from the viewer sources (`scripts/messages/` in the Kokua checkout, template version 2.0) is the authoritative layout of every UDP message.
- LibreMetaverse (inside the SecondBot checkout) is a complete client library; `Login.cs`, `NetworkManager.cs`, `Simulator.cs`, `Caps.cs` and `AgentManager.cs` show the login parameters, circuit handshake, event queue and teleport handling.
- `references/opensim/` is the simulator used for testing; `OpenSim/Services/LLLoginService/` and `OpenSim/Region/ClientStack/Linden/UDP/` show the server side.

Findings from those sources that shape the design:

- Teleport cannot be done over UDP alone. `TeleportFinish`, `EnableSimulator`, `EstablishAgentCommunication` and `CrossedRegion` are delivered through the EventQueueGet capability as LLSD XML (`AgentManager.cs:1519`), so the bot needs an HTTP client, seed-capability request and LLSD parsing.
- `ObjectUpdate.ParentID` is a region-local U32, not a UUID.
- OpenSim uses the login failure reason `key` for a wrong password and also for "region not responding" and inventory-service problems (`LLLoginResponse.cs`), so `key` alone does not prove bad credentials. `presence` covers "already logged in" and "logins restricted".
- OpenSim drops a client after 60 s without packets (`AckTimeout`, `LLUDPServer.cs:412`).
- Rust 1.100.0 is installed and is the default toolchain.
- `references/opensim` has no built binaries and no `OpenSim.ini` or region files; it targets .NET 8 and `dotnet --list-sdks` printed nothing on this machine.

## Goals / Non-Goals

**Goals:**

- One static-ish binary with a small dependency tree and a resident memory footprint of a few megabytes.
- A reconciler whose decision logic is a pure function, testable without a network.
- Grid-neutral code: nothing that only works on OpenSim, HTTPS supported from the start.
- Every spec scenario checkable against the local OpenSim rig.

**Non-Goals:**

- A general-purpose protocol library. Only the messages this scenario needs are implemented.
- Tracking the scene (objects, other avatars, terrain, inventory, appearance).
- Async runtime, multiple bots per process, plugin or scripting interfaces.
- Automated end-to-end tests that start OpenSim themselves; rig checks are run by hand from a documented checklist.

## Decisions

### D1. Threads and blocking I/O, one state owner

Per circuit, a UDP reader thread and an event-queue thread feed one `std::sync::mpsc` channel of events, as does the signal handler; the main thread owns all state.

```
 [udp reader thread] ----+
 [event queue thread] ---+--> mpsc --> [main thread: session state + reconciler]
 [signal handler] -------+                 |-- sends UDP directly (socket is shared, send is thread-safe)
                                           |-- does login as a blocking HTTP call
```

The event-queue thread makes the seed-capability request itself before it starts polling, so the main thread never blocks on a simulator's HTTP server.

The main loop is `recv_timeout(next_deadline)`: it handles one event or one timer expiry, then runs the reconcile step. Timers (agent update, ack flush, resend, action deadlines, backoff) are plain `Instant`s compared in the loop.

Login is a blocking HTTP call made from the main thread, which is acceptable because there is no session to service while disconnected. The stop signal is checked before and after it; the HTTP timeout (30 s) bounds the delay, and a second signal exits immediately as the spec requires.

*Alternative: tokio + reqwest.* Rejected: one bot per process has at most two concurrent waits, and the async stack would be most of the binary and the dependency tree.

### D2. Hand-written codec for the needed messages

Each message is a small struct with `encode`/`decode` written by hand against `message_template.msg`, with a comment giving the template name, frequency and number. Needed set:

| Direction | Messages |
|---|---|
| Out | UseCircuitCode, CompleteAgentMovement, RegionHandshakeReply, AgentThrottle, AgentUpdate, CompletePingCheck, PacketAck, MapNameRequest, TeleportLocationRequest, AgentRequestSit, AgentSit, LogoutRequest |
| In | RegionHandshake, StartPingCheck, PacketAck, AgentMovementComplete, MapBlockReply, TeleportStart, TeleportProgress, TeleportFailed, TeleportLocal, AvatarSitResponse, ObjectUpdate (prefix only), KickUser, AlertMessage, AgentAlertMessage, LogoutReply, DisableSimulator |

Incoming packets are dispatched on (frequency, number). Unknown numbers are dropped after header processing, so reliable ones still get acked. Decoders only read the fields the bot uses and tolerate trailing data.

*Alternative: generate all ~480 messages from the template in `build.rs`.* Rejected for now: a template parser and code generator are more code than 28 hand-written messages, and nothing else needs the rest. The dispatch table leaves room to switch later.

### D3. LLUDP layer

- Header: flags byte (`0x80` zerocoded, `0x40` reliable, `0x20` resent, `0x10` appended acks), sequence number u32 big-endian, extra-header length byte, then message number (1 byte High; `FF` + 1 byte Medium; `FFFF` + u16 BE Low; `FFFFFF` + 1 byte Fixed). Body fields are little-endian.
- Zerocoding is applied to everything after the six header bytes. Decoding is required for all incoming packets; encoding is applied to outgoing messages the template marks Zerocoded.
- Appended acks on incoming packets are stripped before zero-decoding (count in the last byte, u32 BE each).
- Outgoing reliability: unacked reliable packets are resent after 1 s, then with doubling interval, up to 5 sends in total; exhaustion is a session-loss cause.
- Incoming reliability: sequence numbers of reliable packets are queued and flushed in a `PacketAck` at most 200 ms later; a bounded set of recently seen sequence numbers suppresses duplicate processing.
- Liveness: last-receive time per circuit; 60 s of silence is session loss.
- `AgentUpdate` is sent once per second with a fixed camera, zero control flags and a small draw distance; `AgentThrottle` is sent once after the handshake with low values that still allow the task channel to deliver the avatar's own object update.

### D4. Circuits are tagged with a generation number

Every circuit gets a generation id when it is opened (login or teleport hand-over), increasing for the life of the process. Each circuit has its own UDP socket; its reader and event-queue threads stamp each event with the circuit's generation, and the session discards events whose generation is not current. This implements "stale simulator traffic is ignored" and prevents a late `DisableSimulator` from the old region, or leftovers from a previous session, from looking like session loss.

During a teleport the old circuit stays current until `TeleportFinish` arrives. At that moment the old circuit and poller are stopped and a new circuit and poller for the destination become current immediately. Arrival is reported once the new circuit's handshake completes; if it does not complete within 30 s the session is lost. A `TeleportFinish` the bot did not ask for is followed the same way.

### D5. Capabilities and event queue

- When a circuit is opened, its event-queue thread POSTs an LLSD array with `EventQueueGet` to the seed capability and reads back the URL; failures are retried under the same 60 s rule as polls.
- The event-queue thread long-polls that URL, sending the last received `id` as `ack`. HTTP 502 or a timeout means "no events, poll again". HTTP 404 means the session is gone. Other errors are retried after 1 s; 60 s of continuous failure is session loss.
- Only `TeleportFinish` is acted on. `EnableSimulator`, `EstablishAgentCommunication` and `CrossedRegion` are logged in verbose mode and ignored, since the bot never opens child circuits and never walks across a border.
- LLSD XML and XML-RPC are parsed with one small XML reader into a simple value enum (map, array, string, integer, uuid, binary, boolean, real); requests are written with string formatting. Binary LLSD is not needed. `TeleportFinish` carries the simulator IP and region handle as base64 binary, decoded by a small hand-written decoder.

### D6. Login

XML-RPC `login_to_simulator` with `first`, `last`, `passwd` (`$1$` + md5 hex), `start`, `channel` (`SLBot-rs`), `version` (crate version), `platform`, `mac` and `id0` (stable md5 values derived from the user name, so each bot account presents a consistent identity), `agree_to_tos` and `read_critical` both true, and an empty `options` array. `start` is `uri:<region>&<x>&<y>&<z>` normally and `last` after a refused attempt.

`agree_to_tos`/`read_critical` are sent as true so that an unattended bot is not blocked by a terms prompt; whoever runs a bot account has accepted the grid's terms when creating it.

From the reply the bot uses `login`, `reason`, `message`, `next_url`, `agent_id`, `session_id`, `secure_session_id`, `circuit_code`, `sim_ip`, `sim_port`, `seed_capability`. Classification: `login == "indeterminate"` follows `next_url`; reason `mfa_challenge` is needs-operator; reason `key` is refused; everything else is temporary.

### D7. Refused-login policy instead of message matching

Because `key` is ambiguous on OpenSim, the reconciler does not treat one refusal as proof of a bad password. It retries once with `start = last`; only if that is also refused, and no login has succeeded since process start, does it exit. Once a login has succeeded in this process the credentials are known to have been good, so later refusals are retried with a 30-minute cap, slow enough not to provoke lockouts if the password was changed.

*Alternative: match the human-readable `message` text.* Rejected: texts differ between grids and versions.

### D8. Reconciler as a pure function

```
 Observed { session: Disconnected | Connected{region_name}, seat: Unknown | NotSeated | SeatedOnTarget }
 Memory   { per-action failure counts, next-allowed times, in-flight action + deadline,
            had_successful_login, connected_since, seated_since, last_login_refused }

 decide(observed, memory, config, now) -> Wait(until) | Start(Login{start} | Teleport | Sit) | Exit(code, message)
```

`decide` has no I/O. The main loop executes the returned action through the session layer and feeds outcomes back as events that update `Observed` and `Memory`. Unit tests drive `decide` with scripted event sequences and a fake clock to cover every reconciler scenario, including backoff bounds and the refused-login rules.

The repositioning rule (three failed sits) is expressed inside `decide` as "start Teleport although already in the target region"; the session layer handles it as an in-region teleport ending in `TeleportLocal`.

### D9. Seated detection without scene tracking

Seated-on-target requires that `AvatarSitResponse.SitObject.ID` equalled the configured UUID (the bot answers it with `AgentSit` when a sit is in progress), and is confirmed or cleared by the avatar's own `ObjectUpdate`: the block whose `FullID` equals the agent id carries `ParentID`, non-zero when seated and zero when standing. Only the fixed-layout prefix of each `ObjectData` block is decoded up to `ParentID`; the variable-length tail is skipped using its length prefixes to reach the next block. `ObjectUpdateCompressed`, `ObjectUpdateCached` and terse updates are ignored, since simulators send avatars as full updates when the parent changes.

A non-zero `ParentID` without a preceding sit response for the target (the avatar was seated somewhere else) counts as not seated on target; the next sit request moves it.

*Alternative: map `ParentID` to the object's UUID.* Rejected: requires tracking every object in the region.

### D10. Module layout

```
 src/main.rs         CLI (run | hash), signal setup, main loop
 src/config.rs       TOML model + validation
 src/log.rs          timestamped stderr logging, verbose flag
 src/xml.rs          LLSD XML + XML-RPC value parsing and writing
 src/login.rs        login request, reply parsing, classification
 src/udp/packet.rs   header, zerocoding, appended acks
 src/udp/messages.rs message structs
 src/udp/circuit.rs  sequence numbers, ack queue, resend queue, liveness
 src/caps.rs         seed capability, event-queue thread
 src/net.rs          per-circuit UDP socket, reader and event-queue threads behind the session's `Net` trait
 src/session.rs      circuits by generation, handshake, teleport handoff, sit, seat tracking
 src/reconciler.rs   Observed, Memory, decide()
 src/backoff.rs      doubling delay with bounds and reset rule
```

The binary is named `slbot`.

### D11. Dependencies and toolchain

A `rust-toolchain.toml` pins channel `1.100`, the installed default, so builds stay reproducible if the machine's default changes later.

Dependencies: `ureq` with rustls (HTTP/HTTPS), `roxmltree` (XML parsing), `md-5`, `serde` + `toml` (config), `uuid`, `ctrlc` with the `termination` feature (SIGINT and SIGTERM), `rpassword` (no-echo input for `hash`). Timestamps are formatted by hand from `SystemTime` in UTC to avoid a date-time crate.

### D12. Test rig

`references/opensim` is a symlink to a checkout outside this repository and is left untouched. The rig is a local clone of it at `test-rig/opensim/` (git-ignored), built there in standalone mode with SQLite, HTTP on `127.0.0.1:9000`, and two regions "Home" and "Target" in `bin/Regions/Regions.ini`. The rig's `OpenSim.ini`, `Regions.ini` and a setup script are tracked under `test-rig/config/` and copied into the clone. A bot user is created on the console; the seat prim is rezzed by `test-rig/rezprim.py` (a small script that logs in as the estate owner and sends one ObjectAdd) or from a viewer, and its UUID copied into the bot config. `setup.sh` derives `OpenSim.ini` and `StandaloneCommon.ini` from OpenSim's shipped examples instead of tracking full copies; only `Regions.ini` is tracked. The setup steps and the failure-injection checklist live in `docs/test-rig.md`.

Protocol code that does not need a simulator (zerocoding, header and ack parsing, message encode/decode, XML, login reply classification, config validation, `decide`) is covered by `cargo test`.

## Risks / Trade-offs

- [The avatar's own `ObjectUpdate` might not arrive under the low throttle, leaving the seat unconfirmed] -> Verify on the rig in the sit task group; raise the task throttle if needed. A sit with a matching `AvatarSitResponse` but no object update within the sit deadline counts as failed, so the bot retries rather than believing a false state.
- [A bug in loss detection becomes a login loop against a grid's servers] -> Login backoff applies after every session end regardless of cause and resets only after 5 minutes connected; covered by `decide` unit tests.
- [Hand-written codecs can silently disagree with the template] -> Each decoder has a unit test against bytes captured from the rig or built field by field from the template; verbose logging prints message names and sizes for diagnosis.
- [OpenSim-only verification hides Second Life differences: TLS, MFA, login throttling, stricter sit range, different interest-list traffic] -> Design stays grid-neutral and HTTPS is implemented, but Second Life behaviour is unverified in this change and stated as such in the README.
- [Blocking login on the main thread delays reaction to a stop signal by up to the HTTP timeout] -> Timeout is 30 s and a second signal exits immediately.
- [`agree_to_tos = true` accepts changed terms without the operator seeing them] -> Documented in the README.
- [Building OpenSim needs the .NET 8 SDK, which is not installed] -> First rig task checks for it and stops with instructions if missing; no bot code depends on the rig to compile or unit-test.

## Open Questions

- Password length on Second Life: official viewers have historically hashed only the first 16 characters of a password for Second Life logins. `hash` and the config hash the whole password, which is what the spec says and what OpenSim expects. To be settled in the Second Life verification pass.
- `AgentThrottle` values and draw distance are first guesses that work on OpenSim (the avatar's own object update arrives within a second); they may need tuning for Second Life. Affects traffic volume only.
