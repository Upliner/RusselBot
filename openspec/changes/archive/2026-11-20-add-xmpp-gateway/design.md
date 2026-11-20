# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- The bot is one process per avatar, synchronous, with no async runtime. Worker threads (`net`, `names`, `inventory`, the control socket's connection threads) send `net::Event`s into one channel, and the main loop in `src/main.rs` owns all state.
- The control socket's threads only decode a line into a `control::Request` and send `Event::Control { request, reply }`; `Bot::control` carries it out and answers on `reply`, possibly much later (a teleport, a name lookup for `im`). The connection thread waits for the answer with a limit of 75 seconds; at most 16 connections are served at once.
- `control::request_line` turns `rslbot ctl`'s arguments into a request line (joining the words of `say` and `im` messages), and `Request::decode` reads it. `CTL_USAGE` is the command list.
- Recorded messages leave the `Recorder` (`src/msglog.rs`) as `Record`s with names settled, in `Bot::service_messages` and, at exit, `flush_all`, and are written by `Logs::write`. `Record` keeps names and the object UUID but drops the sending avatar's UUID, which `session::Source::Avatar { id, .. }` has. `im_out` records are made in `Bot::record_sent` with the recipient as the source.
- `Bot::status()` gives the multi-line text of `status`; the session knows the region the avatar is in.
- `Backoff` (`src/backoff.rs`) gives a doubling delay with bounds.
- Dependencies are few and synchronous (`ureq`, `roxmltree`, `md-5`, `toml`, `serde`, `uuid`); the bot is meant to run in a few MB.
- Decided with the user in exploration: a stock XMPP server (Prosody suggested) on the same host as the bots; the gateway is an XEP-0114 component inside each bot rather than a server module, a server of our own, or a central gateway process; local chat is a plain contact in this change; contacts are addressed by name with UUID fallback, replies go by UUID.

## Goals / Non-Goals

**Goals:**

- One new module that owns the XMPP connection, reached from the main loop only through events and a non-blocking send, as the control socket is.
- Commands from XMPP go through exactly the path the control socket uses, so that every rule about commands (RLV restrictions, override, refusals, logging) holds without being repeated.
- Everything that decides something (address mapping, escaping, command-line splitting, chat-line formatting, presence state, the wait queue, stanza building and reading) is pure code with unit tests.

**Non-Goals:**

- A general XMPP library: only the stanzas the specification names are read or written; everything else is ignored or answered with `service-unavailable`.
- Full PRECIS/stringprep of addresses. Owners and the domain are compared in lower case; that is enough for the ASCII addresses people configure.
- Keeping the address-to-UUID map across restarts.

## Decisions

### An external component inside each bot

Rejected in exploration: an ejabberd (Erlang) or Prosody (Lua) module ties the gateway to one server and still needs a link to every bot process; a server of our own means accounts, TLS, storage, history, push, server-to-server and client quirks for years; a central gateway process needs an event stream on the control socket and one more daemon. With bots and server on one host, a component per bot has the fewest moving parts, and the component's presence follows the bot's life by itself. Each bot gets its own domain, so several bots and grids share one server and one owner account.

### Own small stanza layer on `quick-xml`, not `xmpp-parsers`

The gateway reads about six stanza shapes (stream header, handshake result, stream error, message, presence, iq) and writes about eight. `xmpp-parsers` with `minidom`, `jid` and `rxml` would give typed stanzas but brings a wide tree of crates (several hashes, base64, a second XML stack beside `roxmltree`) for a few of its types, and its streaming reader is built for `tokio-xmpp`. `quick-xml` reads events incrementally from any `BufRead`, which fits a blocking reader on a `TcpStream`, and writing is a few `format!`s through one escaping function. The handshake needs SHA-1, from the `sha1` crate of the same family as `md-5`. Task 1.1 checks this on a real Prosody before anything is built on it; if `quick-xml` cannot follow a stream that never closes its root element, the fallback is `minidom` + `rxml` without `xmpp-parsers`.

Outcome of the check (Prosody 13.0.7, `quick-xml` 0.42): `quick-xml` reads the stream header and then one top-level stanza at a time from a `BufReader<TcpStream>` whose root element never closes; the handshake is accepted, a wrong secret gives the stream error `not-authorized`, and a message from `owner@rig.test` arrives as one element. The fallback is not needed. In this version entity and character references come as events of their own (`GeneralRef`), which the reader resolves itself (the five predefined entities and character references; any other entity is a malformed stream).

The reader turns each top-level child of the stream into a small owned element (name, namespace, attributes, children, text) and hands it to the main loop; matching on it happens in pure code.

### Threads and events

```
                 main loop (owns XmppState)
   Event::Xmpp(..) ^                     | try_send(Outgoing)
                   |                     v   bounded channel (1000)
   +---------------+------+     +--------+---------+
   | xmpp reader thread   |     | xmpp writer thread|
   | connect, handshake,  |     | writes stanzas,   |
   | read stanzas, reconnect    | a space every 60 s|
   +----------+-----------+     +--------+---------+
              |      one TcpStream (cloned)          |
              +--------------> Prosody <-------------+
```

- The reader thread connects, does the handshake, then reads stanzas and sends `Event::Xmpp(XmppEvent::Connected { generation, writer })`, `Stanza(..)` and `Disconnected { reason }`. It sleeps the backoff delay between attempts and resets the backoff after an accepted handshake.
- Each connection has its own writer thread and bounded channel. The main loop sends with `try_send`; a full channel means the server has stopped reading, so the main loop marks the connection as down (the writer shuts the socket, which ends the reader's read) and further messages go to the wait queue. The main loop never blocks on XMPP.
- A `generation` per connection lets the main loop ignore events from a connection it has already given up.

### Commands and messages to contacts enter as control requests

A stanza the main loop accepts becomes a `control::Request`:

| To | Request |
|---|---|
| bare domain | split the body into arguments, `request_line`, `Request::decode` |
| `local@` | `Request::Say(body)` |
| avatar address | `Request::Im { to, text }`, with `to` the UUID (cases 1 and 2 of "Contact addresses") or the unescaped name (case 3) |

`Bot::control(request, reply, now)` carries it out unchanged. The `reply` receiver goes to a small waiter thread, as a control connection waits, with the same 75-second limit; the waiter sends `Event::Xmpp(XmppEvent::Answer { to, id, kind, response })` back, and the main loop turns it into a chat message (control contact) or an error stanza (`local@`, avatar addresses; nothing on success). A shared counter caps waiters at 16, as `MAX_CONNECTIONS` caps control connections. Splitting the body (quotes, `\"`, `\\`) is a new pure function in `src/xmpp`; `request_line` is made usable on already split arguments.

`help` is answered from `CTL_USAGE` with the `rslbot ctl ` prefixes taken off.

Error conditions for error stanzas: `forbidden` (not an owner), `item-not-found` (no such avatar), `feature-not-implemented` (an object's address), `not-acceptable` (empty or too long), `service-unavailable` (no region, busy, any other refusal); the text is always the command's error text.

### Records carry the avatar's UUID

`Record` gets `avatar_uuid: Option<Uuid>`: the sender of a message from an avatar, the recipient of `im_out`, `None` otherwise. The TSV row and the stderr line do not change. Where `Bot` writes a record to the logs it also hands it to the XMPP state, which maps it to a contact, learns the address-to-UUID pair, and sends or queues it. `im_out` records only teach the map.

### Addresses

- `escape_local(name)` and `unescape_local` implement XEP-0106 for the ten characters it lists; the local part is built from the lower-cased legacy name and then escaped.
- The map is `HashMap<String, Uuid>` (address local part to UUID), pruned like the recorder's name cache once it grows past 10 000 entries.
- Owners are kept as lower-cased bare addresses; a stanza's `from` is lower-cased and its resource dropped before comparing.

### Presence

The XMPP state keeps the last presence it sent for the bare domain and `local@`. After each event the main loop handles, it works out the presence from the session (region or none) and sends to every owner only what changed. Probes are answered from the same state. The order is: messages the change produced, then the presence, so that a `quit` answer arrives before the addresses go offline.

### Waiting messages

`VecDeque<Outgoing>` with the time each was made, bounded at 500, dropping the oldest and counting what was dropped. On `Connected`, presence goes first, then the queue in order, each with `<delay xmlns='urn:xmpp:delay' stamp='...'/>`, then live traffic. An offline IM always carries its sent time, queued or not.

### Stopping

On exit the bot flushes the recorder (as now), hands the last records to the XMPP state, sends unavailable presence for both addresses, closes the stream, and gives the writer up to 2 seconds to drain before the process ends.

### Server example

The README gets the Prosody lines:

```lua
component_interfaces = { "127.0.0.1" }
Component "russel.sl.example.org"
    component_secret = "..."
```

and a note that ejabberd's `ejabberd_service` listener does the same.

## Risks / Trade-offs

- [The component connection is plain TCP with a shared secret] → the config refuses a `server` that is not on the loopback interface; the README says to bind the component port to `127.0.0.1`.
- [Owners' clients drop presence from addresses they have not added] → the README says to add the bare domain and `local@` as contacts; messages arrive either way.
- [Stalled server fills the writer channel] → `try_send` and a reconnect; the wait queue keeps the newest 500 messages.
- [A dead connection is not noticed when nothing is sent] → the writer sends a single space every 60 seconds of quiet, which a closed socket turns into an error.
- [Simplified address comparison] → owners with non-ASCII addresses may not be recognised; such an owner is refused, never wrongly let in.
- [Hypergrid names do not resolve by search] → replies use the remembered UUID; after a restart an owner has to wait for a new message from that visitor, or use the UUID address.
- [Clients differ in how they show error stanzas] → the error's text is always set, and Conversations, Gajim and Dino are tried on the rig (task 7).
- [`quick-xml` and a stream that never ends] → checked first, in task 1.1, with the fallback named above.

## Client differences (task 7.2)

- Dino 0.5.1 starts every new conversation with OMEMO on and will not send
  to an address without OMEMO devices, so an owner cannot write to any of
  the gateway's addresses until the conversation is switched to
  unencrypted. The gateway cannot change that: OMEMO is end to end, and
  the gateway's contacts have no keys. A later change could make the
  gateway an OMEMO endpoint of its own (keys held by the bot, messages
  decrypted there before they reach the grid), which protects the hop
  from the owner's client to the XMPP server and the server's storage,
  but not the grid; until then the README tells Dino users to switch the
  conversation, or Dino's default, to unencrypted.
- Once a conversation was unencrypted, Dino 0.5.1 showed errors with their
  text and nicknames as the specification describes, as did Gajim 2.6.0.
- Conversations has not been tried.

## Migration Plan

Nothing changes for a config without `[xmpp]`. To use the gateway: add a component to the XMPP server, add the `[xmpp]` table, restart the bot, and add the bare domain and `local@` to the owner's contacts. To roll back, remove the table.
