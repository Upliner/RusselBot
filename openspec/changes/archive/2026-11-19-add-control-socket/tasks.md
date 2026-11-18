# Tasks

## 1. Config key

- [x] 1.1 Add `socket` to `Raw` and `Config` in `src/config.rs` (boolean, default false, wrong type rejected naming `socket`); verify with new unit tests for absent, `true`, `false` and `"yes"`
- [x] 1.2 Add a commented `socket = true` entry to `russelbot.example.toml` and a `socket` row to the README config table; verify the existing example-config tests still pass

## 2. Wire format and address

- [x] 2.1 Move `escape` out of `src/msglog.rs` to a place both modules use and add `unescape`; verify with round-trip unit tests including backslash, tab, CR, LF and a dangling backslash
- [x] 2.2 Create `src/control.rs` with `Request` (`Say`, `Im`), `Response` (`Ok(Option<String>)`, `Error(String)`), and line encoding/decoding for both; verify unit tests cover unknown command, wrong argument count, invalid UTF-8, over-long line and escaped arguments
- [x] 2.3 Implement `control::address()` (env `RUSSELBOT_SOCK`, else `$HOME/russelbot-ctl` on Unix, `127.0.0.1:47115` elsewhere) as a function of injected environment values; verify unit tests cover override, empty override, default and missing `HOME`

## 3. Message parsing

- [x] 3.1 Implement the chat-bar parser (`/N`, `//`, `/shout `, `/whisper `, everything else on channel 0, channel overflow, empty text, 1023-byte limit); verify unit tests cover every scenario of "Chat-bar parsing of say messages" in the `message-send` spec
- [x] 3.2 Implement recipient normalization (UUID, `First Last`, `first.last`, single word → `Resident`, rejection of other shapes) and exact case-insensitive matching against a candidate list; verify unit tests cover the "Recipient resolution" scenarios including similar names

## 4. Sending from the session

- [x] 4.1 Add `ChatFromViewer`, outgoing `ImprovedInstantMessage` and `AvatarPickerRequest` encoders and an `AvatarPickerReply` decoder to `src/udp/messages.rs`; verify byte-level unit tests in the style of the existing ones, including a truncated reply
- [x] 4.2 Add `Session::say`, `Session::instant_message` and `Session::find_avatar`, refusing when the region is not entered, and `SessionEvent::AvatarsFound`; verify `Rig` tests show the packets sent when entered, an error before entry and during a teleport, and the event for a reply

## 5. Bot wiring

- [x] 5.1 Add `Event::Control { request, reply }` and handle `say` in `Bot`: parse, remember the last channel, call the session, reply, log; verify by unit-testing the handling function with a fake session or by a `Rig`-level test, including "not connected"
- [x] 5.2 Handle `im` in `Bot`: UUID or cached name sends at once; otherwise start a lookup, park the command with a 10 s deadline included in `run_loop`'s wake time, complete it on `AvatarsFound`, fail it on timeout, unknown name or session loss; verify tests for each outcome and for two commands waiting on the same name
- [x] 5.3 Log every sent chat message and every refused command with `info!`; verify the lines by running against the test rig
- [x] 5.4 On the OpenSim test rig (`docs/test-rig.md`), and on Second Life if an account is at hand, confirm that `AvatarPickerRequest` is answered and that an IM sent by the bot opens a conversation the recipient can reply in; record the result in `docs/test-rig.md`
- [x] 5.5 Record each sent IM through `Recorder` as kind `im_out` with the recipient as its avatar, making the recorder fill an empty avatar name from the lookup if it does not already; verify `msglog` tests for the row and stderr line of an `im_out` record, for a UUID recipient with and without a resolved name, and for `im_out`/`im` order, and a `Bot`-level test that a refused `im` records nothing
- [x] 5.6 Add `im_out` to the README's description of the `log_im` file; verify the documented row matches a row produced on the test rig

## 6. Socket server

- [x] 6.1 Implement the Unix listener: stale-file check, refusal when a live bot or a non-socket file is at the path, bind, mode `0600`, removal on exit; verify tests in a temp directory for a fresh path, a stale socket, a live listener and a regular file
- [x] 6.2 Implement the `cfg(not(unix))` TCP `bind`/`connect`; verify with `cargo check --target x86_64-pc-windows-gnu` when that target is installed, otherwise note in the README that the Windows path is unexercised
- [x] 6.3 Implement the accept loop and per-connection thread (5 s read timeout, 8192-byte cap, 16-connection cap, 20 s wait for the reply); verify tests with a real socket and a stub that answers events: a good request, a silent client, an over-long line and a client that disconnects early
- [x] 6.4 Start the listener in `run_bot` after `Logs::open` when `config.socket` is set, reporting failure as a `socket` config error with exit status 2, and remove the socket file when `run_bot` returns; verify by starting the bot with `RUSSELBOT_SOCK` pointing into a missing directory and checking the message and status

## 7. Client subcommand

- [x] 7.1 Implement `control::client(args)`: build the request line, connect, send, read the answer within 30 s, print and return the exit status from the `control-socket` spec; verify tests against a stub listener for ok, ok with text, error, no listener and no command
- [x] 7.2 Dispatch `ctl` in `run_cli` before option parsing so later arguments pass through untouched, and extend `USAGE`; verify a test that `ctl say -v /1 x` reaches the client as given and that `rslbot ctl` returns status 2
- [x] 7.3 Document the control socket in the README: the `socket` key, `RUSSELBOT_SOCK`, both commands with examples (including `/1 test message` and a quoted avatar name), exit statuses, and the Windows caveat that any local user can connect; verify the documented commands run as written against the test rig

## 8. Integration check

- [x] 8.1 With the bot running on the test rig with `socket = true`: `rslbot ctl say hello`, `rslbot ctl say /1 test message` (observed by a listening script or `test-rig/talk.py`), `rslbot ctl im <test avatar> hi` (with `log_im` set, checking the `im_out` row and the reply's `im` row), a command while the bot is logged out, and a second bot on the same socket; confirm each behaves as specified, the socket file is gone after Ctrl-C, and `cargo test` passes
