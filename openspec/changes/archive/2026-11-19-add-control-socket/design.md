# Design

## Context

See proposal.md for motivation. What shapes the approach:

- The bot is single-threaded at its core. `run_loop` in `src/main.rs` owns `Bot` and blocks on one `mpsc::Receiver<Event>` (`src/net.rs`); every other thread (UDP reader, event queue, name lookups, signal handler) only sends `Event`s. `Session` (`src/session.rs`) is driven entirely from that loop and is tested through a fake `Net`.
- `Session` already encodes outgoing UDP messages through `udp::messages::Writer` and decodes incoming `ImprovedInstantMessage`, so the wire layout of an IM is known to the code. It sends nothing chat-like today.
- `run_cli` treats every argument starting with `-` as an option and `-v` as global, wherever it appears.
- `names.rs` resolves UUID → name through the `GetDisplayNames` capability. Nothing resolves name → UUID.
- No async runtime, and only `serde`/`toml` for parsing; there is no JSON crate. The toolchain is pinned to Rust 1.100, whose standard library has Unix domain sockets on Unix only.
- Config problems, including log files that cannot be opened, are reported before any network activity with exit status 2 (`run_bot`).

## Goals / Non-Goals

**Goals:**
- Commands reach the session through the existing event channel, so the main loop stays the only owner of `Bot` and `Session`.
- Parsing (chat bar, recipient names, wire lines) is pure code with unit tests; the socket layer is thin.
- One code path for both platforms above the listener/stream types.

**Non-Goals:**
- Authentication, remote access, TLS. The socket is for the local operator.
- Queueing commands for later. A command that cannot run now fails now.
- Negative chat channels, gestures, typing animation, group or conference IM, delivery confirmation.
- Commands other than `say` and `im`. The wire format leaves room for them.

## Decisions

### A `control` module with a listener thread and one short-lived thread per connection
`src/control.rs` holds the address logic, the listener, the wire format and the client. The listener thread accepts connections; each connection gets a thread that reads one line (5 s read timeout, 8192-byte cap), parses it into a `Request`, sends `Event::Control { request, reply: Sender<Response> }` to the main loop, waits for the `Response` and writes it. The main loop never touches a socket.

Concurrent connections are capped (16); further ones are answered with an error at once. With the 5 s read timeout that bounds what an idle client can hold.

*Alternative:* a non-blocking listener polled from the main loop. It would tie command latency to the loop's wake-up and add socket handling to the loop that every other input avoids.

### Replies are sent by the main loop, possibly later
`Bot` answers `say` at once. `im` to a name not yet resolved is parked in `Bot` as a pending command (recipient key, message, reply sender, deadline) until the lookup answers or 10 s pass; the deadline joins the other wake-up times in `run_loop`. Pending commands are failed with "not connected" when the session is dropped, and the reply sender being closed (client gone) is ignored.

The connection thread waits for the reply with a 20 s timeout as a backstop; the client waits 30 s.

### Wire format: tab-separated line with the TSV log's escaping
Request: `command<TAB>arg<TAB>arg\n`. Response: `ok\n`, `ok<TAB>text\n` or `error<TAB>text\n`. Arguments and texts use the escaping `msglog.rs` already has (`\\`, `\t`, `\n`, `\r`); `escape` moves to a shared place together with a new `unescape`.

*Alternative:* JSON lines. It needs a new dependency for no gain at two commands. *Alternative:* sending the raw command line and splitting on spaces in the bot. It cannot carry `"Jane Doe"` as one argument.

The client does no command-specific parsing beyond joining message arguments: `ctl say a b` sends `say<TAB>a b`, `ctl im X a b` sends `im<TAB>X<TAB>a b`. Chat-bar parsing happens in the bot, which is where the last-used channel lives.

### Address resolution and platform split
`control::address()` returns `RUSSELBOT_SOCK` if set and non-empty, else `$HOME/russelbot-ctl` under `cfg(unix)` and `127.0.0.1:47115` otherwise. Two small `cfg` blocks provide `bind(addr)` and `connect(addr)` returning boxed `Read + Write` streams; everything else is shared. On Windows the bot does not require a `RUSSELBOT_SOCK` value to be a loopback address: the operator who sets it decides.

*Alternative:* Unix sockets on Windows too (supported by the OS since Windows 10). The standard library does not expose them there and a dependency just for that is not worth it.

### Unix socket file handling
On start, if the path exists: connect to it. A successful connect means a live bot → config error. A refused connect on a socket file → remove it and bind. A path that is not a socket → config error, nothing removed. After `bind`, permissions are set to `0600`; to close the window between `bind` and `chmod`, the process `umask` is not touched but the listener does not `accept` until the mode is set. The file is removed when `run_bot` returns; the second-signal `process::exit` path in `install_signal_handler` leaves it behind, which the stale-file rule covers.

The listener is created in `run_bot` right after `Logs::open`, before the event loop starts, so a failure is an exit-2 config error reported as ``socket``.

### `ctl` is split off before option parsing
`run_cli` checks whether the first argument is `ctl` before looking for options and hands the remaining arguments, untouched, to `control::client`. `rslbot -v ctl ...` is therefore not a `ctl` invocation; it falls through to the existing handling and is rejected as too many arguments. `rslbot ctl` with no command, `ctl -h` and `ctl --help` print the usage, which gains the `ctl` lines.

### Chat-bar parsing as a pure function
`chat::parse(input, last_channel) -> Result<Chat { channel: i32, kind: Whisper|Say|Shout, text }, String>` implements the table in the `message-send` spec, mirroring the viewer's `stripChannelNumber` and chat-type triggers. `Bot` keeps `last_channel` (process lifetime, starts at 0). Length is checked after parsing, on the text that would go on the wire (1023 bytes, the `ChatFromViewer` limit less the terminator).

### Sending through `Session`
Two new methods, usable only once the region is entered (they return an error string otherwise, which becomes the reply):

- `say(channel, kind, text)` → `ChatFromViewer` (Low 80): agent block, then message (variable-2, NUL-terminated), type `u8`, channel `i32`. Sent reliable.
- `instant_message(to, text)` → `ImprovedInstantMessage` (Low 254) with dialog 0, `FromAgentName` = the avatar's `First Last`, `ID` = agent id XOR recipient id (the session id viewers use, so replies and the bot's messages share a conversation), position and region id from what the session already knows or zero, empty binary bucket. Sent reliable.

"Handed to the simulator connection" in the specs means these calls returned `Ok`; the circuit's existing resend logic takes it from there.

### Recipient resolution: `AvatarPickerSearch` capability, UDP `AvatarPickerRequest` without it
*Revised after trying Second Life:* the UDP request described below found nobody there. The bot now asks the seed capability for `AvatarPickerSearch` as well; when the simulator offers it, a lookup is an HTTP GET (`?page_size=100&names=<name>`) on a worker thread in `names.rs`, whose LLSD reply has the same `agents` layout as `GetDisplayNames`, and the answer comes back to the main loop as `Event::AvatarsFound` or `Event::AvatarSearchFailed`. Without the capability the UDP path below is used unchanged. `Bot` picks between them in a small `Grid` wrapper around the session; `Commands` sees no difference.

`Session::find_avatar(query_id, name)` sends `AvatarPickerRequest` (Low 26); the reply `AvatarPickerReply` (Low 28) carries the query id and a list of id/first/last. The session surfaces it as `SessionEvent::AvatarsFound { query, avatars }`; `Bot` picks the entry whose `first last` equals the normalized request case-insensitively, caches it (lowercased name → UUID and proper name) and completes every pending command waiting on that query.

Normalization, a pure function: UUID → used directly; `a.b` → `a b`; one word → `word resident`; exactly two words → as is; anything else → error.

*Alternative:* the `AvatarPickerSearch` capability, which Second Life viewers use today. It needs another capability request, HTTP on the worker thread and LLSD parsing, and OpenSimulator standalone does not offer it by default. The UDP message is served by both; if Second Life ever drops it, the capability can be added behind the same `Bot` interface.

### Outgoing IMs go through the existing `Recorder`
When `Session::instant_message` returns `Ok`, `Bot` calls `self.record(Channel::Im, Message { kind: "im_out", source: Source::Avatar { id: recipient, name }, sent_at: None, text }, now)`. The recorder then treats the recipient exactly as it treats a sender: it asks `GetDisplayNames` for the display name, waits at most 5 s, keeps arrival order, and produces the stderr line and the TSV row. `name` is the legacy name from the picker reply or the cache, and empty for a recipient given by UUID that the bot has not resolved; the recorder must then take the avatar name from the lookup, which task 5.5 checks and fixes if `Recorder::record` only uses the message's own name today.

No new column: the row's `kind` tells a reader that the avatar columns name the recipient. *Alternative:* a direction column. It would change the file format for every existing row and reader.

Chat sent on command is not recorded here. Channel 0 chat comes back from the simulator as `ChatFromSimulator` and is recorded as heard; recording it at send time as well would duplicate it.

### Operator log
Sent chat and refused commands are logged with `info!` in `Bot`, e.g. `control: said on channel 1: test message`, `control: im refused: no avatar named "Nobody Here"`. A sent IM is not logged a second time: its `im_out` record already produces a stderr line.

## Risks / Trade-offs

- [Any local user can connect to the Windows TCP socket and speak as the avatar] → Off by default; documented in the README next to the key. A token or named pipe can follow if Windows use becomes real.
- [Windows code cannot be run here] → Keep the `cfg(not(unix))` part to `bind`/`connect`; build-check it with `cargo check --target x86_64-pc-windows-gnu` if the target is installed, and say so in the README if it was not exercised.
- [`AvatarPickerRequest` returns prefix matches and at most a page of them; a very common name might not include the exact avatar] → Exact-match rule means the command fails rather than messaging the wrong person; the UUID form always works.
- [Second Life may answer `AvatarPickerRequest` differently from OpenSimulator] → Verify against the real grid during apply (task 5.4); the 10 s timeout turns silence into a clear error.
- [Chat is rate-limited and length-limited by the simulator] → Length is enforced; rate is the operator's business and the simulator's alerts already reach the log.
- [A command arriving during a teleport] → Treated as "not connected to a region"; the client can retry.

## Open Questions

- The default Windows port `47115` is arbitrary; it can change without touching anything else.
