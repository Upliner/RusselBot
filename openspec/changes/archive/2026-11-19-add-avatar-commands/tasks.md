# Tasks

## 1. Protocol groundwork

- [x] 1.1 Find the layouts of `StartLure`, `TeleportLureRequest`, `ObjectGrab`, `ObjectDeGrab`, `RequestMultipleObjects`, `ObjectUpdateCompressed`, `ObjectUpdateCached`, `KillObject` and the `AgentUpdate` control flags in a `message_template.msg` (viewer source) or, failing that, the OpenSim packet handlers under `references/`, and correct design.md where it differs; verify by listing each layout with its source in the task's commit or notes
- [x] 1.2 Add a control-flags parameter to `messages::agent_update` and a dialog parameter to `messages::instant_message`; verify the existing byte-level tests pass with the old values and new ones cover a non-zero flag and dialog 26
- [x] 1.3 Add encoders for `StartLure`, `TeleportLureRequest`, `ObjectGrab`, `ObjectDeGrab` and `RequestMultipleObjects`; verify byte-level unit tests for each
- [x] 1.4 Decode the local id in `ObjectUpdate`, and decode `ObjectUpdateCompressed`, `ObjectUpdateCached` and `KillObject` to UUID/local-id pairs; verify unit tests with builders in `messages::testing`, including truncated bodies

## 2. Session

- [x] 2.1 Generalize sitting: `start_sit(object, now)`, `Seated(Uuid)`, the seat accessor (`target`/`object`/`other`/`none`/`unknown`), with `Bot` passing the configured object and mapping `Seated` for the reconciler; verify the existing sit and seat-tracking `Rig` tests pass unchanged in meaning and new ones cover a sit on a second object and an unrequested sit
- [x] 2.2 Add `stand` and `sit_ground` (one reliable `AgentUpdate` with the flag; `stand` drops a pending sit); verify `Rig` tests for the packets, for the periodic update still carrying flags 0, and for refusal outside a region
- [x] 2.3 Add `region()` and `position()` accessors and make `start_teleport` usable for a commanded teleport inside the current region by name; verify `Rig` tests for same-region and other-region requests with arbitrary coordinates
- [x] 2.4 Teleport offers: report dialog 22 as `SessionEvent::TeleportOffer` and dialog 26 as `SessionEvent::TeleportRequested`, add `accept_teleport(lure, now)` (sends `TeleportLureRequest`, starts the 60 s teleport wait), `request_teleport(to, message, now)` and `offer_teleport(to, message, now)`; verify `Rig` tests for each packet, for the hand-over after an accepted offer, and that other dialogs are still ignored
- [x] 2.5 Keep the object tables without sending anything (`objects` from full and compressed updates, `undescribed` from cached ones, pruned by `KillObject`, emptied on a new circuit, capped) and add `start_scan` (cache-miss requests for what is undescribed, then for each cached update as it comes); verify `Rig` tests that no packet is sent for any object update before a scan, that a scan requests exactly the undescribed ids in batches of at most 255, and that the tables are empty after a teleport hand-over
- [x] 2.6 Add `touch(object, now)` with its three results, the `wanted` list with the 10 s deadline, and `Touched`/`TouchFailed` events; verify `Rig` tests for a known object (grab then de-grab with its local id), an unknown object before a scan (scan starts, touch goes out when the update arrives), the same timing out, an unknown object after a scan (refused at once), and an object killed since
- [x] 2.7 Take the scan distance as a session parameter: when set, start the scan on entering each region and send it as the draw distance in `AgentUpdate`; verify `Rig` tests for both, and that without it the draw distance is 32 and nothing is requested

## 3. Reconciler and config

- [x] 3.1 Add manual mode to `Memory`, `decide` and `Reconciler::set_manual`, including `Start::Last` for logins in manual mode and the clearing rules; verify reconciler unit tests for every scenario of the modified "Converge on the desired state" requirement, and that all existing reconciler tests pass
- [x] 3.2 Add `requesttp_timeout` (positive number of minutes, default 10) and `scan_distance` (1 to 1024, optional) to `config.rs`, the README config table and `russelbot.example.toml`; verify unit tests for defaults, good values, and each rejected value naming its key

## 4. Commands

- [x] 4.1 Add the new `Request` variants and their decoding in `control.rs` (aliases, argument counts, joining of trailing words), raise `ANSWER_LIMIT` and `CLIENT_LIMIT`, and extend `CTL_USAGE`; verify unit tests for each command's line, for wrong counts, and the socket tests still pass
- [x] 4.2 Add the pure parsers in `commands.rs`: sit target (UUID or `ground`), `tp` arguments (numbers, optional z, region words, bounds); verify unit tests covering every scenario of the "Sit command" and "Teleport command" requirements that concerns parsing
- [x] 4.3 Extend `Grid` and `Live` with the session calls from group 2, and implement `stand`, `sit ground`, `sendtp`, `shout` and `whisper` in `Commands` (answer at once, log line each); verify fake-grid tests for success, refusal while not connected, and that `shout`/`whisper` leave the `//` channel alone
- [x] 4.4 Implement `sit <uuid>` and `tp` with the pending-outcome replies, settled from session events and failed on session loss; verify tests for success, simulator refusal, timeout event, supersession of a pending sit, default z, missing z with no known height, and unknown region
- [x] 4.5 Implement `requesttp` (name resolution through the generalized `Waiting`, the `asked` table with the configured timeout) and the handling of teleport offers (accept once within the timeout, otherwise keep for `accepttp` and log) and of teleport requests (log only); verify tests for accept, unsolicited, expired with the default and accepted with a longer timeout, second offer after one was accepted, and unknown avatar
- [x] 4.6 Implement `accepttp` (offer table, name known from the offer, reply parked as the pending teleport) and `touch` (replies kept by UUID, settled by `Touched`/`TouchFailed`, failed on session loss); verify tests for accepting a kept offer, no offer, an offer used once, a newer offer replacing an older one, offers forgotten with the session, and for the three touch results
- [x] 4.7 Return an `Effect` from `Commands::handle` for mode changes and `quit`, and implement `home`, `status` and `quit` in `Bot`; route session outcomes to the reconciler or to `Commands` as the design describes; verify tests that each pausing command yields the manual effect only when carried out, that `status` text has the specified lines connected and disconnected, and that `quit` is answered before the stop

## 5. Documentation

- [x] 5.1 Document every new command, manual mode and `home`, the teleport-offer rule and `accepttp`, the object scan with its two config keys and what it costs, and the 90 s wait in README.md (Control socket section and the opening scenario list), and note in "Chat and IM logs" that teleport offers are not logged to the files; verify each documented command line runs as written against the test rig

## 6. Integration check

- [x] 6.1 Extend `test-rig/listen.py` (or add a helper) so the second avatar can answer a teleport request with an offer, accept an offer, and report where the bot's avatar is and whether it is seated; verify by running it against the rig
- [x] 6.2 On the rig with `socket = true` and `--verbose`: `stand` (bot stays standing), `sit <uuid>` on a second prim, `sit ground`, `tp` inside the region with and without z, `tp` to `Home` and back by name, `tp` to an unknown region, `requesttp` and `sendtp` with the helper, an unsolicited offer and `accepttp` for it, `accepttp` with no offer, `touch` on a near and on an unknown object with and without `scan_distance` (checking with `--verbose` that no `RequestMultipleObjects` is sent before the first `touch` when it is absent), `status` in each state, `home` from another region, a kick in manual mode (logs in at the last location and stays), `shout`, `whisper`, and `quit`; record each as an item in `docs/test-rig.md` with its result, marking what the rig cannot show
- [x] 6.3 Run `cargo test` and a release build; verify both succeed and that a bot that receives no commands behaves as before (items 1, 3 and 9 of `docs/test-rig.md` repeated)
