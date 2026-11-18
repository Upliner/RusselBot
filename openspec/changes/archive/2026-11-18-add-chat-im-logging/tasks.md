# Tasks

## 1. Config keys

- [x] 1.1 Add `log_chat`, `log_im` (optional paths) and `fetch_offline_im` (boolean, default false) to `Raw` and `Config` in `src/config.rs`, rejecting empty path strings with a message naming the key; verify with unit tests for absent keys, valid values, an empty path and a non-boolean `fetch_offline_im`
- [x] 1.2 Document the three keys in `README.md` (config table, plus a note that fetching offline IMs removes them from the grid) and add commented examples to `slbot.example.toml`; verify `Config::parse` accepts the example file with the examples uncommented

## 2. Message decoding

- [x] 2.1 Decode `ChatFromSimulator` into `Incoming::Chat` in `src/udp/messages.rs` with a `testing` builder; verify with a decode test covering an avatar and an object source and a truncated body
- [x] 2.2 Add `IMPROVED_INSTANT_MESSAGE` (Low 254) with its name, decode it into `Incoming::InstantMessage` with a `testing` builder; verify with a decode test covering dialog 0, dialog 19 with an object id, and an offline message with a timestamp
- [x] 2.3 Add `RETRIEVE_INSTANT_MESSAGES` (Low 255) with its name and the `retrieve_instant_messages` encoder; verify with an encode test on id and body

## 3. Session events

- [x] 3.1 Add the `Message`/`Source` types and `SessionEvent::Chat` / `SessionEvent::InstantMessage`; map chat types and dialogs to kinds and drop typing, empty, inaudible and unlisted-dialog messages in `Session::handle_message`; verify with session tests for each kind, each dropped case, an unknown chat type (`type-N`) and an object IM with a nil object id
- [x] 3.2 Pass `fetch_offline_im` to `Session::new` and send `RetrieveInstantMessages` once when the first region is entered; verify with session tests that it is sent exactly once when on (not again after a teleport hand-over) and never when off

## 4. Name lookup

- [x] 4.1 Ask the seed capability for `GetDisplayNames` too and emit `CapsEvent::DisplayNames(Option<String>)` after the seed reply in `src/caps.rs`; forward it from `Session::handle_caps` as `SessionEvent::DisplayNamesUrl`; verify with tests for the seed request body, a reply with and without the capability, and a stale-generation event being dropped
- [x] 4.2 Create `src/names.rs` with the request URL builder and the reply parser (`agents` → legacy and display names, `bad_ids` → missing); verify with unit tests on a multi-id URL, a reply with one agent and one bad id, and a malformed reply
- [x] 4.3 Add the worker thread and the `net::Event::Names` / `Event::NamesFailed` variants (50 ids per request, 10 s timeout); verify with a test that runs the worker against a local `TcpListener` serving a canned reply and receives the expected event

## 5. Recorder and TSV files

- [x] 5.1 Create `src/msglog.rs` with field escaping and row/header formatting; verify with unit tests for the spec's row example, a text containing tab, CR, LF and backslash, and an object row
- [x] 5.2 Implement the `Recorder` queue: `push`, `take_lookups`, `names_resolved`, `lookup_failed`, `flush`, `flush_all`, `next_deadline`, the one-hour name cache and the 1000-entry bound; verify with unit tests for arrival order when the second name resolves first, the 5 s timeout, a cached name needing no lookup, a system source needing no lookup, and a group-owned object leaving the avatar columns empty
- [x] 5.3 Implement file output: open for append, header only when the file is empty, one `write_all` per row, a failed write logged and skipped; verify with tests in a temporary directory for creation with header, append without a second header across two recorders, and chat and IM rows going only to their own files
- [x] 5.4 Implement the stderr line format from design decision 7; verify with unit tests on the formatted string for an avatar, an object with a known owner, and an object with an unknown owner

## 6. Main loop wiring

- [x] 6.1 Open the configured log files in `run_bot` before any network activity and exit with status 2 naming the key on failure; verify by running `slbot` with `log_im` in a missing directory and checking the message, exit status 2 and that no login is attempted
- [x] 6.2 Own the `Recorder` and the names worker in `Bot`: push chat/IM session events with the current wall-clock time, keep the latest display-names URL, send `take_lookups` to the worker, handle the names events, call `flush` each loop turn and include `next_deadline` in the wake time; verify with `cargo test` and a run against the local OpenSim rig where chat typed by `Rig Owner` appears on standard error with a name
- [x] 6.3 Call `flush_all` on every exit path of `run_bot` (signal, fatal login, logout confirmed); verify on the OpenSim rig by stopping the bot immediately after a chat line is sent and finding the row in the file

## 7. Integration checks

- [x] 7.1 On the OpenSim rig with `log_chat` and `log_im` set, send local chat and an IM from `Rig Owner` and chat from a scripted prim (`test-rig/talk.py`); verify the rows in both files against the spec (seven fields, owner names on object rows, escaping of a multi-line message) and record the results in `docs/test-rig.md`. An IM from a script cannot be produced by the rig helper and is verified on Second Life in 7.3
- [x] 7.2 On the OpenSim rig, verify with the verbose log that `RetrieveInstantMessages` is sent exactly once with `fetch_offline_im = true` and never without it. The rig's OpenSim cannot store offline IMs (that needs a MySQL/PostgreSQL store or an external service), so delivery is verified on Second Life in 7.3
- [x] 7.3 On Second Life, run with both files and verify chat from an avatar with a display name, chat from an object, an IM from an avatar and an IM from a script are recorded with correct names; with `fetch_offline_im = true` verify a stored IM is delivered with the time it was sent, and record the outcome in the README status section
