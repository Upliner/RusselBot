# Tasks

## 1. One socket (`src/net.rs`)

- [x] 1.1 Replace the per-circuit socket and reader with one unconnected UDP socket per `RealNet`, bound at the first `open`, one reader thread using `recv_from`, and a shared map from simulator address to generation (design decision 1). `send` uses the circuit's address; a datagram from an unknown address is dropped. Keep the socket, the map and the reader in a struct of their own, without the event queue. Verify with loopback tests: two circuits send from one source port, each simulator's answer comes back under its generation, a third sender's datagram produces no event
- [x] 1.2 Add `leave(generation)` to the `Net` trait and to `RealNet` (stops the circuit's event-queue thread only), make `open` for an address that already has a circuit hand the address to the new generation and remove the older link, and make `close` remove the address only while it belongs to that generation (design decision 4). Verify with loopback tests: after a second `open` for the same address the answers carry the new generation, `close` of the old generation afterwards changes nothing, and after `close` of the new one nothing from that address is reported

## 2. Circuits that pause (`src/udp/circuit.rs`)

- [x] 2.1 Add `Circuit::resume(now)`: `last_receive` becomes `now`, the duplicate filter is emptied and the packets still awaiting an ack are dropped; the next sequence number and the pending acks stay. Verify with unit tests for each of the five

## 3. The session keeps what it leaves (`src/session.rs`)

- [x] 3.1 Give `Sim` its address and `Session` the list of left circuits (generation, address, `Circuit`). `hand_over` calls `Net::leave` and moves the current circuit there instead of closing it; `lose` closes the left circuits too. Extend `FakeNet` with `Call::Leave` and update `teleport_finish_hands_the_session_over_to_the_new_simulator`; add a test that a lost session closes the left circuit
- [x] 3.2 Service left circuits (design decision 2): datagrams of a left generation go through its `Circuit`, pings are answered and acks taken on that generation, `DisableSimulator`, `KickUser` and `LogoutReply` remove it with `Net::close`, everything else is ignored; `tick` flushes its acks, resends, and removes it after `SILENCE_LIMIT` of silence or exhausted resends; `next_deadline` covers it. Tests: a ping and a reliable packet on the left generation are answered there and give no event; each of the five endings closes the circuit without `Lost` and without touching the current region; `late_traffic_from_the_old_simulator_is_ignored_after_arrival` still passes with the kick and the handshake arriving before the `DisableSimulator`
- [x] 3.3 Continue a left circuit on the way back (design decision 3): a `TeleportFinish` for a left circuit's address (or the current one) resumes that `Circuit` in the new `Sim` under a new generation. Tests: `UseCircuitCode` and `CompleteAgentMovement` go out on the new generation numbered after the circuit's last packet; the list of left circuits then holds the simulator just left and not the one returned to; a packet stamped with the superseded generation is dropped; the region's objects and the seat start empty; after a `DisableSimulator` on the left circuit the same finish numbers from 1; a returned-to simulator that never answers is a loss after 30 seconds, not at once

## 4. The rig

- [x] 4.1 Confirm the design's reading of the simulator on the rig, with temporary builds that are not kept: with task 1 alone (one socket, a new `Circuit` on the way back) run the round trip of the proposal with `scan_distance = 64` and record what the bot and the simulator log; then with the kept `Circuit` not serviced, the same. Write both outcomes into `design.md` under Context
- [x] 4.2 Run the round trip (`ctl tp 128 128 25 Home`, 20 seconds, `ctl home`, wait for `seated`) three times in a row with `scan_distance = 64` and three times with `scan_distance = 128`, against the config of `docs/test-rig.md` item 1 plus `persist = "cfg"` and `socket = true`. Pass: six arrivals in `Target`, six sits, no `session lost`, no second login, and in the simulator's log no `Close has been called` for the region returned to. Also once each: a stay of five minutes in `Home` before `ctl home` (the simulator must not log `No packets received from child agent`), and `ctl scan` and `ctl status` after a return
- [x] 4.3 Run items 44, 45 and 51 of `docs/test-rig.md` again without `scan_distance` and item 9 (`teleport user Sit Bot Home/128/128/25` from the console) with `scan_distance = 64`; all must pass as recorded
- [x] 4.4 Replace the note at the end of "Area scan" in `docs/test-rig.md` with a short section for these runs (what was run, the results, what the simulator logs), and rewrite the OpenSimulator paragraph under "Scanning for objects" in `README.md` to say what is true now: the simulator still keeps the presence, the bot keeps that connection and returns over it. Verify both texts against the logs of 4.2

## 5. Integration

- [x] 5.1 `cargo test` and `cargo clippy --all-targets` clean (apart from the `wrong_self_convention` warning in `src/main.rs` that is there today), and `openspec validate keep-left-circuits --strict` passes

## Workflow follow-up

- Archive the change once the rig runs are recorded and the work is reviewed; nothing is committed without being asked.
