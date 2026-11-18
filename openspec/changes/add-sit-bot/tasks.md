# Tasks

## 1. Project setup

- [x] 1.1 Add `rust-toolchain.toml` pinning channel `1.100` and verify `cargo --version` in the repository reports 1.100.x
- [x] 1.2 Set the binary name to `slbot` and add the dependencies from design D11 to `Cargo.toml`; verify `cargo build` succeeds
- [x] 1.3 Create the module files from design D10 as empty modules wired into `main.rs`, and add `test-rig/opensim/` to `.gitignore`; verify `cargo build` succeeds with no warnings about missing modules

## 2. Local OpenSim test rig

- [x] 2.1 Check that a .NET 8 SDK is available (`dotnet --list-sdks`); if not, stop and give the user the install command for their distribution. Verify the command lists an 8.x SDK
- [x] 2.2 Clone `references/opensim` into `test-rig/opensim/`, run `runprebuild.sh` and `dotnet build --configuration Release OpenSim.sln` there; verify `test-rig/opensim/bin/OpenSim.dll` exists
- [x] 2.3 Add `test-rig/config/` with a standalone SQLite `OpenSim.ini`, a `Regions.ini` defining regions "Home" and "Target" on `127.0.0.1`, and a `setup.sh` that copies them into the clone; verify OpenSim starts and its console command `show regions` lists both regions
- [x] 2.4 Create the bot user on the OpenSim console, rez a seat prim in "Target" from a viewer and record its UUID; verify by logging in as the bot user with a viewer and sitting on the prim by hand
- [x] 2.5 Write `docs/test-rig.md` covering build, start, user creation, seat prim, and how to inject each failure (kick, region restart, simulator stop, unseat script, delete prim); verify by following it from a fresh clone state up to a running rig

## 3. Config and hash subcommand

- [x] 3.1 Implement the config model, defaults and validation in `src/config.rs` per the `bot-config` spec; verify with unit tests for each rejection case and for the single-word user name
- [x] 3.2 Implement CLI parsing in `main.rs` (config path argument, `hash` subcommand, `--verbose`) with exit status 2 on config errors; verify `slbot missing.toml` exits 2 with a message naming the file
- [x] 3.3 Implement `hash` reading from standard input without echo on a terminal; verify `printf 'secret\n' | slbot hash` prints `$1$5ebe2294ecd0e0f08eab7690d2a6ee69` and that empty input exits non-zero
- [x] 3.4 Implement `src/log.rs` with UTC-timestamped stderr lines and a verbose level; verify by unit test of the timestamp formatter against known epoch values
- [x] 3.5 Add an annotated `slbot.example.toml` and a config reference section in `README.md`; verify the example file passes validation in a unit test

## 4. Login

- [x] 4.1 Implement the XML value model, LLSD XML parser and XML-RPC reply parser and request writer in `src/xml.rs`; verify with unit tests on sample login replies and LLSD documents, including nested maps, arrays and escaped text
- [x] 4.2 Implement the login request per design D6 and reply parsing into a session-parameters struct; verify with a unit test against a recorded OpenSim success reply
- [x] 4.3 Implement login outcome classification (needs operator, refused, temporary, follow `next_url`) per the `grid-session` spec; verify with unit tests for `key`, `presence`, `mfa_challenge`, `indeterminate`, a malformed reply and a connection error
- [x] 4.4 Verify against the rig: a run with valid credentials logs a successful login with region address and circuit code, and a run with a wrong password logs a refused outcome without printing the password or hash

## 5. UDP circuit and keepalive

- [x] 5.1 Implement packet header encode/decode, zero-coding both ways and appended-ack extraction in `src/udp/packet.rs`; verify with unit tests including round-trips, runs of more than 255 zeros, and a packet with appended acks
- [x] 5.2 Implement the handshake and keepalive messages in `src/udp/messages.rs` (UseCircuitCode, CompleteAgentMovement, RegionHandshake, RegionHandshakeReply, AgentMovementComplete, AgentThrottle, AgentUpdate, StartPingCheck, CompletePingCheck, PacketAck, KickUser, LogoutRequest, LogoutReply, DisableSimulator); verify each with a unit test against bytes built from the template layout
- [x] 5.3 Implement `src/udp/circuit.rs`: sequence numbers, pending-ack queue with 200 ms flush, resend queue per design D3, duplicate suppression, last-receive time; verify with unit tests using a fake clock for resend timing, resend exhaustion and duplicate handling
- [x] 5.4 Implement the UDP reader thread and the main-loop event channel with generation tagging per design D1 and D4; verify with a unit test that events from a non-current generation are discarded
- [x] 5.5 Implement the circuit handshake, ping replies, periodic `AgentUpdate`, `AgentThrottle`, region-name tracking and session-loss detection (kick, logout reply, disable simulator, 60 s silence, resend exhaustion) in `src/session.rs`; verify with unit tests feeding decoded messages and a fake clock
- [x] 5.6 Verify against the rig: the avatar appears in a viewer after login, stays online for 30 minutes with no errors logged, and `kick user` on the console is logged as session loss with the kick reason within 5 seconds

## 6. Reconciler, reconnect and shutdown

- [x] 6.1 Implement `src/backoff.rs` (doubling delay, bounds, reset rule); verify with unit tests of the sequences for login, teleport and sit bounds
- [x] 6.2 Implement `Observed`, `Memory` and `decide()` in `src/reconciler.rs` per design D8 covering all four rules, backoff, repositioning after three failed sits, the refused-login rules and needs-operator exit; verify with unit tests, one per scenario in the `presence-reconciler` spec that does not need a simulator
- [x] 6.3 Wire the main loop: execute actions returned by `decide`, enforce action deadlines, feed outcomes back, log every action, outcome, state change and delay; with teleport and sit still stubbed as "failed: not implemented", verify on the rig that after `kick user` the bot logs in again after the backoff delay
- [x] 6.4 Implement signal handling and clean logout (logout request, 5 s wait, second signal exits immediately); verify on the rig that Ctrl-C while connected exits 0 within 6 seconds and an immediate restart logs in without an "already logged in" reply
- [x] 6.5 Verify on the rig: with the simulator stopped the bot retries login at growing intervals and recovers when it is started again; with a wrong password the bot makes at most two attempts and exits 1

## 7. Sit and seat tracking

- [x] 7.1 Implement AgentRequestSit, AgentSit, AvatarSitResponse and the `ObjectUpdate` prefix decoder (per block: FullID and ParentID, skipping the variable tail) in `src/udp/messages.rs`; verify with unit tests, including an `ObjectUpdate` with several blocks where only one is the agent
- [x] 7.2 Implement the sit action and seat-state tracking in `src/session.rs` per design D9, including the 15 s deadline and reset to unknown on region entry; verify with unit tests for seated, unseated, sit on a different object, and other avatars' updates
- [x] 7.3 Verify on the rig with the bot logging in to "Target": it sits on the prim; a script calling `llUnSit` makes it re-sit after the sit delay; deleting the prim leads to slow retries with no re-login; re-rezzing is not expected to recover since the UUID changes, and the log shows the failures clearly
- [x] 7.4 Verify on the rig that the avatar's own `ObjectUpdate` arrives under the chosen throttle values, and adjust the throttle in design D3 terms if it does not; record the final values in a code comment
- [x] 7.5 Verify on the rig that the avatar stays seated for 30 minutes and verbose logging shows no further sit requests

## 8. Capabilities, event queue and teleport

- [x] 8.1 Implement the seed-capability request and the event-queue poller thread in `src/caps.rs` per design D5 (ack id, 502 and timeout handling, 404 and 60 s failure as session loss); verify with unit tests of reply parsing and of the failure-timing logic with a fake clock
- [x] 8.2 Implement MapNameRequest, MapBlockReply, TeleportLocationRequest, TeleportStart, TeleportProgress, TeleportFailed and TeleportLocal in `src/udp/messages.rs`, and `TeleportFinish` parsing from event-queue LLSD; verify each with a unit test
- [x] 8.3 Implement the teleport action in `src/session.rs`: region lookup by name, request, outcomes (arrived, failed with reason, region not found, 60 s timeout), and in-region teleport ending in `TeleportLocal`; verify with unit tests of each outcome
- [x] 8.4 Implement the circuit handoff on `TeleportFinish` per design D4: new circuit and poller with a new generation, switch after handshake, stop the old ones; verify with a unit test that a late `DisableSimulator` from the old generation does not cause session loss
- [x] 8.5 Verify on the rig: with the bot user's last location in "Home" and the start location not honoured or disabled, the bot teleports to "Target" and sits; with a wrong region name in the config the log shows "region not found" retries at growing intervals while the avatar stays online
- [x] 8.6 Verify on the rig: restarting region "Target" while seated ends with the avatar seated again without operator action, and the log shows loss cause, attempts, delays and final sit in order

## 9. Integration checks

- [x] 9.1 Run the full failure-injection checklist from `docs/test-rig.md` in one session and record the result of each item in that document
- [x] 9.2 Update `README.md` with build and run instructions, the `hash` subcommand, exit statuses, and a statement that Second Life and multi-factor login are not yet verified; verify every command shown runs as written
- [x] 9.3 Run `cargo test` and `cargo clippy` and verify both finish clean; run `openspec validate add-sit-bot --strict` and verify it passes
