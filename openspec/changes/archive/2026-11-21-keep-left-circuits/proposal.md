# Proposal

## Why

On OpenSimulator the bot loses its session when it teleports back into a region it left less than a minute ago, whenever that region still counts as in view from where the avatar went: with `scan_distance = 64` in the middle of two neighbouring regions, `rslbot ctl tp 128 128 25 Home`, twenty seconds, `rslbot ctl home` ends in `session lost`, a login refused as "already logged in", and a minute offline. The simulator keeps the avatar's presence in the region it left (a child agent) together with the connection the bot had there; the bot abandons that connection and comes back from another UDP port, and the simulator answers by closing the presence it was about to hand the avatar back to. A bot that holds a place is teleported back within seconds, so this is the path it takes after every forced move next door.

## What Changes

- All of a session's simulator connections come from one local UDP port, as a viewer's do. Today every connection has a socket and a port of its own.
- The connection to a simulator the avatar has left is no longer dropped at the teleport. The bot keeps it up for as long as that simulator does: it acknowledges what arrives and answers the pings, and acts on nothing else. It gives the connection up when the simulator closes it or falls silent; that is not a session loss.
- A teleport that leads back to a simulator the bot still has a connection to continues that connection, with its packet numbering, instead of opening a new one. The bot still announces itself, completes the handshake and learns the region from scratch, as on any arrival.
- The event queue of a simulator the avatar has left is not polled any more, as today.
- The note at the end of "Area scan" in `docs/test-rig.md` and the OpenSimulator caveat under "Scanning for objects" in `README.md` are rewritten to say what is true afterwards.

Nothing changes for a teleport to a simulator that does not keep the avatar's presence: that simulator closes the connection itself, as before. `EnableSimulator` and `EstablishAgentCommunication` are still ignored: the bot opens no connection to a region the avatar has not been in.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `region-teleport`: "Teleport to the target region" no longer says the previous simulator connection is dropped, and gains the round trip as a scenario. New requirements for what the bot does with the connection to a simulator the avatar has left, for a teleport back to such a simulator, and for the single local port.

## Impact

- `src/net.rs`: one UDP socket and one reader thread per session, datagrams routed to circuits by the address they come from; a way to stop a circuit's event queue without closing the circuit.
- `src/session.rs`: the `Net` trait gets that call; the session keeps the circuits of simulators it left, services them, drops them, and reuses one on the way back.
- `src/udp/circuit.rs`: a circuit can be taken up again after a pause.
- `docs/test-rig.md`, `README.md`.
- Traffic: a simulator that keeps the avatar's presence keeps sending what it sends a viewer's child agent (pings, and updates about what is in view from next door), and the bot now acknowledges it. Before, the simulator sent the same unanswered for a minute and then gave up.
- No config keys, no new dependencies, no change to stored data or to any command.
