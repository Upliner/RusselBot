# Design

## Context

See proposal.md for motivation and the specs for the required behaviour. What shapes the approach:

- `Session` (`src/session.rs`) is pure logic behind the `Net` trait and reports to the main loop through `SessionEvent`. `src/udp/messages.rs` decodes by hand against the template and returns `Incoming::Other` for everything else; `ChatFromSimulator` (Low 139) already has a constant, `ImprovedInstantMessage` (Low 254) does not.
- The main loop (`src/main.rs`) is single-threaded over one `mpsc` channel of `net::Event`; blocking work (UDP reads, the event-queue long poll) lives on threads that feed that channel. It sleeps until the earliest of the reconciler's and the session's deadlines.
- The seed capability request in `src/caps.rs` asks only for `EventQueueGet`, and the caps thread is blocked in the long poll most of the time.
- Neither message carries a display name. `ChatFromSimulator` gives `FromName`, `SourceID`, `OwnerID`, `SourceType`, `ChatType`, `Audible`, `Message`. `ImprovedInstantMessage` gives the sender's `AgentID`, `FromAgentName`, `Dialog`, `ID`, `Offline`, `Timestamp`, `Message`. For an object IM (`Dialog` 19) the viewer source (`llimprocessing.cpp`, `IM_FROM_TASK`) shows `AgentID` is the owner, `ID` the object and `FromAgentName` the object's name.
- Display names and legacy names come from the `GetDisplayNames` capability: `GET <cap>/?ids=<uuid>&ids=<uuid>…` returns LLSD `{agents: [{id, display_name, legacy_first_name, legacy_last_name, …}], bad_ids: […]}` (`llavatarnamecache.cpp`). OpenSim implements the same capability (`BunchOfCaps.cs`).
- `log::write` already collapses whitespace so a stderr entry is one line, and `log::format_timestamp` produces the UTC form the spec asks for.

## Goals / Non-Goals

**Goals:**
- Keep `Session` free of I/O and names: it only turns packets into events.
- Keep the recorder (queueing, name cache, formatting, escaping) free of threads and clocks so it is unit-testable like the reconciler.
- One code path for stderr and files, so both always agree and keep arrival order.

**Non-Goals:**
- Sending chat or IMs, auto-replies, acting on message content.
- Group and conference chat, inventory/friendship/teleport offers, group notices.
- Log rotation, or reopening the files while running.
- Recording the sender's avatar UUID, position or chat channel (the simulator only delivers channel 0 and debug chat to viewers).
- A UDP fallback (`UUIDNameRequest`) for grids without `GetDisplayNames`; names are left empty there.

## Decisions

### 1. Decode in `messages.rs`, forward as session events

`decode` gains two variants: `Incoming::Chat { from_name, source_id, owner_id, source_type, chat_type, audible, text }` and `Incoming::InstantMessage { from_agent, from_name, dialog, id, offline, timestamp, text }`. `Session` filters (typing, empty text, `Audible` = -1, unlisted dialogs) and emits `SessionEvent::Chat(Message)` / `SessionEvent::InstantMessage(Message)` with one shared struct:

```
Message { kind: String, source: Source, sent_at: Option<u32>, text: String }
Source::Avatar { id, name } | Source::Object { id: Option<Uuid>, name, owner } | Source::System { name }
```

`sent_at` is set only for an IM with `Offline` ≠ 0 and a non-zero `Timestamp`. The kind mapping lives in the session: chat types 0/1/2/6/7/8/9 → `whisper`/`say`/`shout`/`debug`/`region`/`owner`/`direct`, others `type-N`; dialog 0 → `im`, dialog 19 → `object`. Dropped dialogs are named at verbose level only.

*Alternative:* filter and classify in the recorder. Rejected: the session already owns "what does this packet mean", and its test rig is where packet-level cases are cheap to write.

### 2. A recorder module owned by the main loop

New `src/msglog.rs` with a `Recorder` that the `Bot` owns for the life of the process (not per session, so messages pending when a session is lost are still written). It holds a FIFO of pending entries, a name cache, and the two optional files. Its interface is clock-free:

- `push(channel, message, received_wall_secs, now)` — queues an entry and notes which agent id it needs (sender, or owner; none for `Source::System` or a nil id).
- `take_lookups() -> Vec<Uuid>` — ids not cached and not already requested.
- `names_resolved(found, missing)` / `lookup_failed(ids)`.
- `flush(now)` — writes every entry at the head of the queue whose name is settled (cached, known missing, lookup failed, or 5 s old); stops at the first unsettled one. `flush_all()` for shutdown.
- `next_deadline()` — when the head entry times out, folded into the main loop's `wake`.

Writing an entry means one `info!` line and, if the matching file is configured, one row. Head-of-line release is what guarantees arrival order on both outputs.

*Alternative:* log to stderr immediately and only delay the file row. Rejected: the two outputs would disagree on order and stderr would never show display names.

### 3. Name lookup on its own thread

New `src/names.rs`: a worker thread with an `mpsc::Receiver<(String /*cap url*/, Vec<Uuid>)>`. It performs the GET with the shared `ureq` agent settings (10 s timeout), parses with `xml::parse_llsd`, and posts `net::Event::Names { found: Vec<(Uuid, Names)>, missing: Vec<Uuid> }` or `Event::NamesFailed(ids)` to the main channel. `Names { legacy: String, display: String }`, legacy being `legacy_first_name + " " + legacy_last_name`. Up to 50 ids per request.

The capability URL reaches the main loop from the caps thread: the seed request asks for `GetDisplayNames` as well, and after the seed reply the thread emits a new `CapsEvent::DisplayNames(Option<String>)`. `Session::handle_caps` drops stale generations as today and forwards it as `SessionEvent::DisplayNamesUrl(Option<String>)`; the bot keeps the latest value. With no URL known yet entries simply wait out their 5 s; with `None` (grid has no such capability) lookups are failed at once.

*Alternative:* do the GET on the caps thread. Rejected: it is parked in a long poll for up to 90 s.
*Alternative:* block the main loop on the GET. Rejected: it would stall pings and acks.

### 4. Name cache

`HashMap<Uuid, (Option<Names>, Instant)>`, entries valid for one hour, both positive and "grid says unknown" (`bad_ids`, e.g. a group-owned object). Transport failures are not cached, so the next message from that sender retries. This keeps a chatty neighbour to one lookup per hour while still picking up display-name changes.

For an avatar message the `avatar_name` column uses the name the simulator sent with the message; only `display_name` comes from the lookup. For an object message both columns come from the lookup of the owner.

### 5. Files

`Config` gains `log_chat: Option<PathBuf>`, `log_im: Option<PathBuf>`, `fetch_offline_im: bool`. `Config::parse` checks only shape (non-empty string, boolean). `run_bot` opens the files (`OpenOptions::new().create(true).append(true)`) right after `Config::load` and before the channel, signal handler or any network use; failure prints `slbot: \`log_chat\`: cannot open <path>: <error>` and returns `EXIT_CONFIG`. A header row is written when the file's length is 0.

Handles stay open for the life of the process and each row is one `write_all` on the unbuffered `File`, so a row is complete on disk when written and concurrent readers (`tail -f`) never see a partial line. A failed write logs `cannot write to <path>: <error>` and drops that row; the next row tries again.

Escaping is a small function applied to every field: `\` → `\\`, tab → `\t`, LF → `\n`, CR → `\r`. Backslash is escaped so the encoding is reversible.

*Alternative:* open the file per row to be friendly to log rotation. Rejected for now: rotation is a non-goal, and a startup check plus a kept handle gives clearer failure behaviour.

### 6. Timestamps

The recorder takes the wall-clock seconds from the caller (`main` reads `SystemTime` when it handles the session event), and uses `Message::sent_at` instead when present. Lookup latency therefore never shifts the time, and tests pass fixed values.

### 7. stderr format

```
chat [say] Jane Doe (JD): hello
chat [say] Clock <2f3503ed-…>, owner Jane Doe (JD): tick
im [im] Jane Doe (JD): are you there?
im [object] Clock <2f3503ed-…>, owner Jane Doe (JD): done
```

Empty parts are left out (no `()` without a display name, no `, owner …` when the owner is unknown).

### 8. Offline IMs over UDP

`Session::new` takes `fetch_offline_im`. A session is created once per login, so "once after each login" is a flag on the session: when the first region is entered it sends `RetrieveInstantMessages` (Low 255, agent id and session id, reliable) and never again, including after teleports. Replies arrive as ordinary `ImprovedInstantMessage` packets with `Offline` set.

*Alternative:* the newer `ReadOfflineMsgs` capability. Rejected: a second HTTP path and LLSD shape for the same data; the current viewer still falls back to the UDP message, and OpenSim's offline-IM module only answers the UDP one.

## Risks / Trade-offs

- [Second Life may no longer answer the UDP `RetrieveInstantMessages`] → Verified as its own task on the main grid; if it is ignored there, the option still works on OpenSim and `ReadOfflineMsgs` becomes a follow-up change. The option is off by default, so nothing else depends on it.
- [Fetching offline IMs removes them from the grid's store] → Off by default; README and example config state the consequence.
- [A chat flood grows the pending queue while lookups are slow] → Entries time out after 5 s regardless; additionally, when more than 1000 entries are pending new ones are settled without names.
- [Up to 5 s delay before a line appears when the name service is slow or absent] → Only the first message per sender pays it; an absent capability is detected from the seed reply and costs nothing.
- [Log files can hold private conversations] → They are written only where the operator points them; nothing is added to verbose packet logging beyond what the normal log line already shows.
- [Display names contain arbitrary Unicode, including control characters] → File fields escape tab/CR/LF/backslash; stderr goes through the existing one-line collapse.

## Open Questions

- Whether Second Life delivers `FromName` for avatar chat in legacy form (`First Last`) in every case; if a different form shows up on the main grid, switching `avatar_name` to the looked-up legacy name is a one-line change in the recorder and does not affect the file layout.
