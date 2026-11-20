# Tasks

## 1. Prosody on the rig and the transport check

- [x] 1.1 Add a Prosody to the test rig (`test-rig/setup.sh`, a config under `test-rig/config/` with two accounts `owner@rig.test` and `stranger@rig.test` and a component `bot.rig.test` bound to `127.0.0.1:5347`), and describe starting it in `docs/test-rig.md`; verify `prosodyctl check` passes and both accounts can log in with a client
- [x] 1.2 Add `quick-xml` and `sha1` to `Cargo.toml`; in a new `src/xmpp/stream.rs`, open the component stream, do the XEP-0114 handshake (SHA-1 of stream id and secret, in lower-case hex), and read top-level stanzas one at a time from a blocking `BufReader<TcpStream>` into the small owned element of design.md, ignoring whitespace keepalives; verify against the rig's Prosody with an ignored-by-default test that the handshake is accepted, a wrong secret gives a `not-authorized` stream error, and a message sent from `owner@rig.test` arrives as one element. If `quick-xml` cannot follow the open stream, switch to `minidom` + `rxml` as design.md says and record the outcome in design.md

## 2. Config

- [x] 2.1 In `src/config.rs`, read the `[xmpp]` table into `Config::xmpp: Option<XmppConfig>` (domain in lower case, `server` defaulting to `127.0.0.1:5347` with a loopback-only host, secret, owners as a de-duplicated set of lower-cased bare addresses) and reject every malformed form the `bot-config` delta lists, naming `xmpp.<key>`; verify with one config test per scenario the `bot-config` delta adds
- [x] 2.2 Keep the secret out of `Debug` output and of every error message (a wrapper type whose `Debug` prints `***`); verify with a test that formats a `Config` holding a secret and finds no trace of it
- [x] 2.3 Add a commented `[xmpp]` table to `russelbot.example.toml`; verify the example config with the table uncommented passes validation in a test

## 3. Pure pieces (`src/xmpp/`)

- [x] 3.1 Addresses: XEP-0106 `escape_local`/`unescape_local`, the local part for an avatar (`first`, `first.last`, UUID fallback for an unknown name, `local`, a UUID or `obj-` prefix), `obj-<uuid>` for objects, the address-to-UUID map with pruning, and the lookup that turns an address into a UUID or a name to resolve; verify with unit tests for every scenario of "Contact addresses", including the hypergrid name and `local Resident`
- [x] 3.2 Owners: parse a stanza's `from`, drop the resource, lower-case, compare; verify with unit tests for "Owner from any device", a stranger, a malformed `from` and a missing `from`
- [x] 3.3 Command lines: split a body at spaces, tabs and line feeds with double quotes, `\"` and `\\`; refuse an unclosed quote and a body over 8192 bytes; make `control::request_line` usable on split arguments; build the `help` text from `CTL_USAGE`; verify with unit tests for "Name with a space", "Unclosed quote", an empty body and that `say a  b` gives the request `say a b` as `rslbot ctl` does
- [x] 3.4 Local chat lines: the speaker's name (display and legacy, legacy alone, object name), ` whispers`, ` shouts`, `[kind]` and `/me`/`/me'`; verify with unit tests for each scenario of "Local chat contact" that concerns the text, and for a display name equal to the legacy name
- [x] 3.5 Stanzas out: chat message with optional nickname (XEP-0172) and delay (XEP-0203), error reply with condition and text, presence (available with status, away with status, unavailable), `subscribed`/`unsubscribed`, disco#info result, ping result, `service-unavailable`; all text and attributes escaped; verify with unit tests comparing the built XML, including a body with `<`, `&` and `]]>`
- [x] 3.6 Stanzas in: classify an element as message (type, from, to, id, body), presence (type, from, to), iq (type, id, from, to, child namespace) or other, and never answer an error stanza; verify with unit tests built from stanzas captured on the rig in 1.2
- [x] 3.7 Presence state from the bot's state (region or none, stopping) and the difference to what was last sent; verify with unit tests for the three rows of "Presence and subscriptions" and that nothing is sent when nothing changed
- [x] 3.8 Wait queue: bound of 500, oldest dropped and counted, drained in order with the delay stamp, offline IMs keeping their sent time; verify with unit tests for "XMPP server restart during chat" and "Long outage"

## 4. Records carry the avatar's UUID

- [x] 4.1 Add `avatar_uuid: Option<Uuid>` to `msglog::Record`, filled from `Source::Avatar` for received messages and from the recipient for `im_out`; leave the TSV row and stderr line unchanged; verify the existing `msglog` and `main` tests pass and a new test shows the UUID on an `im` record and an `im_out` record and none on an object's

## 5. The connection

- [x] 5.1 Reader thread in `src/xmpp/conn.rs`: connect, handshake, read stanzas into `Event::Xmpp(XmppEvent::{Connected, Stanza, Disconnected})` with a generation; on failure log the reason without the secret and retry with `Backoff` from 5 s to 5 min, reset after an accepted handshake; verify with tests against an in-process fake component server on `127.0.0.1:0` for "Server up at startup", "Server comes up later" (with short delays injected), "Wrong secret" and "Malformed stanza"
- [x] 5.2 Writer thread per connection with a bounded channel of 1000, a single space after 60 s of quiet, and shutdown of the socket when the main loop gives the connection up; verify with fake-server tests that a server that stops reading leads to `try_send` failing, a reconnect, and no blocking of the sender

## 6. The gateway in the bot (`src/main.rs`)

- [x] 6.1 Start the reader thread before the first login when `Config::xmpp` is set and never otherwise; keep `XmppState` in `Bot`; on `Connected` send presence then the wait queue; on `Disconnected` start queueing; verify with bot tests using the fake server that the order on connect is presence, queued messages, live messages
- [x] 6.2 Refuse non-owners: `forbidden` to messages, `unsubscribed` to subscription requests, one log line naming the sender; answer disco#info and ping to anyone and `service-unavailable` to other `get`/`set`; verify with bot tests for the scenarios of "Only owners use the gateway" and "Service discovery and ping"
- [x] 6.3 Turn owners' messages into control requests (bare domain, `local@`, avatar addresses, `obj-` refused, bodiless ignored) and pass them to `Bot::control`; answer through waiter threads capped at 16 together, as design.md describes; verify with `FakeGrid` bot tests for "Status", "Name with a space", "Command refused", "Reply to an avatar", "Unknown name", "Reply to an object", "Typing notification", "Owner speaks in local chat", "Chat bar forms", "Too long", "Teleport while chatting", and a seventeenth waiting command answered as busy
- [x] 6.4 Hand every record written to the logs to `XmppState`: `im` and `object` to their contacts with nickname, `object` without UUID and system messages to the control contact, chat to `local@`, `im_out` only into the address map, offline IMs with their sent time; verify with bot tests for the scenarios of "Instant messages to the owners" and "Chat heard", "Shout and emote", "Object speaking to its owner", and with the existing message-log test "Message that looks like a command" extended to the gateway
- [x] 6.5 Presence and subscriptions: approve owners' subscriptions, answer probes, send presence changes after each handled event, unavailable for avatar and object addresses; verify with bot tests for "Adding the bot", "Teleport" and "Grid lost"
- [x] 6.6 Stopping: flush records to XMPP, unavailable presence, close the stream, give the writer up to 2 s; verify with a bot test for "Bot stopped" and "Quit" (the `ok` arrives before the unavailable presence)
- [x] 6.7 README: an "XMPP gateway" section with the Prosody (and ejabberd) component config, the `[xmpp]` table, the addresses and what each contact does, the command-line quoting, that strangers are refused, that the component port must stay on `127.0.0.1`, and what is not supported yet; the config key table and the Status section; verify each config and command shown runs as written against the rig

## 7. On the rig

- [x] 7.1 Add rig items from 165 on to `docs/test-rig.md` and run them with the local OpenSim and Prosody and one XMPP client: IM both ways with an avatar of the rig, local chat both ways with `/shout` and a channel, `status`, `tp` and `quit` from the control contact, a stranger refused, Prosody restarted during chat with the waiting messages delivered, a wrong secret, and the bot started before Prosody; record pass or fail per item with what was seen
- [x] 7.2 Try the contacts in Conversations, Gajim and Dino (failed message with its error text, nickname shown, delayed messages shown with their time, presence of the bare domain); record in `docs/test-rig.md` what each client shows and in design.md any difference that needs a later change
