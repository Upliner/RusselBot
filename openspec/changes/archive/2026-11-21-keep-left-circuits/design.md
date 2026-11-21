# Design

## Context

See `proposal.md` for the failure. What the code and the simulator do today:

- `Session::hand_over` (`src/session.rs`) answers a `TeleportFinish` by closing the current circuit (`Net::close`) and opening the next one with a new generation number. `open_circuit` builds a new `Sim`, with a new `Circuit` whose packet numbers start at 1, and sends `UseCircuitCode` and `CompleteAgentMovement`.
- `RealNet::open` (`src/net.rs`) binds a UDP socket of its own for every circuit, connects it to the simulator, and starts a reader thread and an event-queue thread for it. `close` stops both. Events reach the main loop stamped with the circuit's generation, and the session drops those of any generation but the current one.
- OpenSimulator (`references/opensim`, `feccd9cc53`):
  - A region the avatar teleports out of keeps the avatar as a child agent, with its `LLClientView` and the UDP endpoint it knows, when the destination still has that region in view (`ScenePresence.RegionViewDistance`, the draw distance plus 64 m). Otherwise the destination tells it to close the agent (`CloseOldChildAgents`), and the bot's old circuit gets a `DisableSimulator`.
  - On the way back the source region calls `CreateAgent` on the destination in every case (`EntityTransferModule.TransferAgent_V2`). `Scene.NewUserConnection` finds the child agent (`Reusing existing child scene presence`), stores the new circuit data and makes new capabilities under a new seed path (`CapabilitiesModule.CreateCaps` deregisters the old handlers, the old event queue URL among them).
  - `LLUDPServer.PacketReceived` looks the sender up by endpoint. A `UseCircuitCode` from an endpoint that has a client is handed to that client like any packet and changes nothing. One from an unknown endpoint goes to `HandleUseCircuitCode` and `AddClient`, which finds the agent's existing client, compares `client.RemoteEndPoint == remoteEndPoint` (two `IPEndPoint` objects, so never equal) and calls `Scene.CloseAgent(agentID, true)`: the child agent, its circuit data and its capabilities are removed, and creating the new client then fails (`Value cannot be null` in the simulator's log). The bot sees its seed capability answer 404, or the event queue report no such session.
  - Every reliable packet's sequence number is checked against the last 200 the client has seen (`IncomingPacketHistoryCollection`); one it has seen is acknowledged and dropped.
  - A client that sends nothing for 60 seconds is closed (`AckTimeout`, `DeactivateClientDueToTimeout`), child agents included. The simulator pings every client every few seconds, so a viewer's child circuits never get there.
  - `ScenePresence.CompleteMovement` sends `RegionHandshake` again and clears `m_gotRegionHandShake`; the reply makes `SendInitialData` describe the whole scene again. An agent that was a child here is told everything a new arrival is told.
  - There is no handler for a viewer's `CloseCircuit`. The only thing a client can send that closes its agent is `LogoutRequest`, which logs the avatar out of the grid.
- The viewer (`references/kokua`, `process_teleport_finish` in `llviewermessage.cpp`) has one socket for all simulators, keeps the circuit to every region it has a child agent in, and on `TeleportFinish` calls `enableCircuit` for the destination (a circuit that exists is kept) and sends `UseCircuitCode` and `CompleteAgentMovement` on it.

So the simulator needs two things of a client that returns to a region where it is still a child agent: the packets must come from the endpoint the child agent's client has, and they must not repeat sequence numbers that client has seen. Both follow from doing what the viewer does.

Tried on the rig on 2026-11-21 (`scan_distance = 64`, `ctl tp 128 128 25 Home`, 20 seconds, `ctl home`), with builds that were not kept:

- The bot as it was: the return's `UseCircuitCode` comes from a new port, the simulator logs `Close has been called for Sit Bot attached to scene Target` and the `Value cannot be null` exception, and the bot logs `session lost: event queue reports no such session` in the same second.
- One socket, but a new `Circuit` on the way back: the simulator hands the packets to the child agent's client and logs `Received a duplicate (not marked as resend) of packet #2, type CompleteAgentMovement`. The avatar never arrives; 25 seconds later the simulator gives the teleport up and the bot logs `session lost: the simulator closed the connection`. The port alone is not enough.
- One socket and the `Circuit` kept, but not serviced while the avatar is away: the first round trip passes. The second loses the session: `Home` had closed its agent after the first return (its `DisableSimulator` went unread), and on the next arrival there the kept circuit resent a `TeleportLocationRequest` of the first stay that had never been marked acknowledged. The simulator's new client carried it out as a new request and sent the avatar straight back. A circuit that is kept has to be kept properly.

## Goals / Non-Goals

**Goals:**
- A teleport back into a region that kept the avatar's presence succeeds, whatever the draw distance and however soon.
- The session's protocol state stays pure and unit-tested with the fake network; the socket handling stays in `src/net.rs`.
- No new way to lose a session: nothing a simulator the avatar has left does, or fails to do, ends it.

**Non-Goals:**
- Circuits to regions the avatar has not been in. `EnableSimulator` and `EstablishAgentCommunication` stay ignored.
- Reading anything from a simulator the avatar has left: its objects, chat and avatars are not the bot's business.
- Polling the event queue of a simulator the avatar has left.
- Region crossings on foot or by vehicle (`CrossedRegion`), which the bot does not follow today either.
- Changing what a teleport does on Second Life beyond the same two things; nothing here is verified there.

## Decisions

### 1. One socket per session, routed by sender

`RealNet` gets one UDP socket, bound when the first circuit opens and not connected to any peer, and one reader thread. A map from simulator address to generation, shared with the reader, says which circuit a datagram belongs to; a datagram from an address that is not in it is dropped. `send(generation, datagram)` looks the address up in the circuit's link and uses `send_to`. The socket and its reader live in a small struct of their own inside `src/net.rs`, without the event queue, so that they can be tested against loopback sockets.

`RealNet` is created per login (`Bot::login` in `src/main.rs`) and dropped with the session, so the port lasts exactly one session, as the requirement says.

Alternative considered: keep a socket per circuit and bind them all to one port with `SO_REUSEPORT`. It needs platform-specific socket options (the bot runs on Windows too), and the kernel would then pick the socket by its connected peer, which is the same routing done in a less visible place.

### 2. The session keeps the circuits it leaves

`Session` gets `left`: for each simulator the avatar teleported out of, the generation, the address and the `Circuit` (sequence numbers, pending acks, unacknowledged packets, the duplicate filter). `Sim` gets its address. `hand_over` moves the current circuit there instead of closing it.

A left circuit is serviced, and nothing more:

- a datagram of its generation goes through `Circuit::receive`, so reliable packets are acknowledged and duplicates are not counted twice;
- `StartPingCheck` is answered with `CompletePingCheck`; `PacketAck` clears the bot's unacknowledged packets;
- `DisableSimulator`, `KickUser` and `LogoutReply` end the left circuit: it is removed and `Net::close` is called for it;
- every other message is decoded no further and ignored;
- `tick` polls each left circuit (ack flushes, resends) and removes one that is silent for `SILENCE_LIMIT` or whose resends are exhausted; `next_deadline` includes them.

None of this pushes a `SessionEvent`: the main loop does not know left circuits exist. `lose` closes them with the current one.

Alternative considered: keep the `Circuit` but not service it. Sequence numbers would still continue, and OpenSimulator would close the unanswered child agent after 60 seconds, which looks cheaper. Tried on the rig, it lost the session on the second round trip (see Context): the bot does not learn that the simulator closed the agent, nor which of its packets were acknowledged. It would also race the simulator's timeout on every return: `NewUserConnection` decides to reuse the child agent, and if the 60 seconds run out before the bot's `UseCircuitCode` arrives, the agent and the circuit data just stored for the teleport are removed. Answering the pings removes the timeout, and it is what the simulator expects of a client it keeps a child agent for.

Alternative considered: close the old region's agent when leaving. The simulator offers a client no way to do that short of logging out (see Context).

Alternative considered: lower the draw distance before a teleport so that the destination closes the old agent. It cannot cover teleports the bot does not start (a console `teleport user`, a forced RLV teleport, an accepted offer), nor an avatar that stands near a region border, where 32 m is already enough to keep the neighbour in view.

### 3. A teleport back continues the circuit

When `TeleportFinish` names an address that a left circuit has (or, defensively, the current one), `hand_over` takes that `Circuit` for the new `Sim` instead of making a new one, after `Circuit::resume(now)` has reset its silence clock and emptied its duplicate filter. The bot's own numbering and its pending acks stay. The filter goes because the simulator may have replaced its client without the bot hearing of it (its `DisableSimulator` is sent once, unreliably): the new client numbers its packets from 1 again, and a filter that still held the old client's low numbers would drop the new `RegionHandshake` as a duplicate. What an empty filter can let through twice is a resend of something the old client sent while the avatar was away, which the bot ignored the first time. `resume` also drops the packets of the bot's that still awaited an ack: they belong to the stay that ended, and a new client would carry out an old request as a new one (the rig showed exactly that with an unserviced circuit, see Context). On a serviced circuit there is rarely anything to drop. Everything else is as on any arrival: a new generation, a new `Sim` (no objects, no seat, no position), `UseCircuitCode` and `CompleteAgentMovement` sent reliably, the handshake limit of 30 seconds. The simulator ignores the `UseCircuitCode` on a client that exists and needs it when it closed that client a moment ago, so it is always sent, as the viewer does.

The generation is new even though the circuit is not, because generations also guard against the old event-queue thread: its last events, and a `Gone` when its queue URL stops existing, still arrive stamped with the old number and are dropped.

### 4. `Net::leave` stops the event queue, `Net::open` takes over the address

The `Net` trait gets `leave(generation)`: the circuit's event-queue thread is told to stop, its datagrams keep coming and `send` keeps working. `hand_over` calls it for the circuit it moves to `left`.

`open(generation, addr, seed)` for an address that already has a circuit makes the new generation the owner of that address: datagrams from it are stamped with the new generation from then on, and the older link is removed. A datagram still in the main loop's channel under the old generation finds neither the current `Sim` nor a left circuit and is dropped unacknowledged; the simulator resends it if it was reliable. `close(generation)` removes the link, and the address only while it still belongs to that generation.

Alternative considered: keep polling the left simulator's event queue, as the viewer does. Nothing the bot acts on arrives there, the thread would hold a long poll open per left region, and on the way back OpenSimulator hands out a new seed and a new queue URL anyway.

### 5. Tests

- `src/net.rs`: two loopback sockets stand in for two simulators. Datagrams sent on two circuits arrive from one source port; an answer from each comes back as `Event::Datagram` with its generation; a datagram from a third socket produces no event; after `open` with a new generation for the first address its answers carry the new generation; after `close` nothing from that address is reported.
- `src/udp/circuit.rs`: `resume` makes `last_receive` the given moment, leaves the next sequence number and the pending acks alone, lets a sequence number seen before the pause through again, and resends nothing from before it.
- `src/session.rs`, with `FakeNet` recording `Leave`: the hand-over leaves instead of closing; a ping and a reliable packet on the left generation are answered on that generation and produce no event; `DisableSimulator`, a kick, silence and exhausted resends each close the left circuit without a `Lost`; a `TeleportFinish` back to the left address opens a new generation for it and sends `UseCircuitCode` with the sequence number that follows the circuit's last; after a `DisableSimulator` the same finish starts at 1; a lost session closes the left circuits; `late_traffic_from_the_old_simulator_is_ignored_after_arrival` and `teleport_finish_hands_the_session_over_to_the_new_simulator` are updated for `Leave`.

## Risks / Trade-offs

- [A simulator that keeps the avatar's presence keeps sending a child agent's traffic, and the bot now acknowledges it for as long as the session lasts] → It is what a viewer does, and the bot only has such a circuit to regions the avatar was in. The bot reads none of it beyond the packet header and message number. Should it matter on a busy Second Life border, the throttle could be lowered on a circuit when it is left; not done here.
- [Second Life is untested] → The change makes the bot behave more like a viewer there (one port, circuits kept until the simulator closes them), not less. The README says so.
- [The simulator closes the child agent in the moment the bot returns to it] → The bot's `UseCircuitCode` then reaches a simulator with no client for the address, which creates one. The bot's numbers continuing from the old circuit are harmless to a new client, and the emptied duplicate filter (decision 3) accepts the new client's. What remains is OpenSimulator's own race between that close and the `CreateAgent` of the teleport, which a viewer has too; it ends as today, in a handshake timeout and a new login.
- [An unconnected UDP socket on Windows reports an ICMP port-unreachable as an error on the next receive] → The reader already logs receive errors at debug level, waits 100 ms and goes on.
- [A left circuit whose simulator has gone away is serviced for up to 60 seconds] → Bounded by `SILENCE_LIMIT`; it costs nothing but the entry.

## Migration Plan

None: no config, no stored data. Rolling back is reverting the change.
