# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- The bot is one synchronous process per avatar with no async runtime. Worker threads (`net`, `names`, `inventory`, the control socket's connection threads, the XMPP reader and writer) send `net::Event`s into one channel, and the main loop in `src/main.rs` owns all state.
- A command is a `control::Request`. The control socket decodes a line of tab-separated, escaped fields with `Request::decode` and sends `Event::Control { request, reply }`; `Bot::control(request, reply, now)` carries it out and answers on `reply`, possibly much later. The XMPP control contact splits a typed line with `control::split`, turns the arguments into such a line with `control::request_line`, and goes the same way; `xmpp::command::help()` builds the `help` text from `CTL_USAGE`.
- Every answer the page needs is already a text with a specified form: `status` is `name: value` lines; `scan`, `scan avatars`, `friends` and `inventory` are tab-separated lines with `\t`, `\n`, `\r`, `\\` (and `\/` in paths) escaped; `im` answers `sent to Jane Doe <uuid>`.
- Recorded messages leave the `Recorder` as `msglog::Record`s in `Bot::service_messages` and `flush_messages`, where they are written to the logs and handed to the XMPP gateway. A `Record` carries the avatar's UUID since the XMPP change.
- `commands::Origin` (`Operator`, `Exempt`, `Rlv`, `Bot`) already says who asks for a command and how the operator log marks it. `Bot::control` decides whether a command is the exempt one of an RLV override before it hands it on.
- `config::Secret` keeps a secret out of `Debug` output. `sha1` is a dependency (XMPP handshake); `getrandom` 0.2 is in the tree through `ring`.
- The bot is meant to run in a dozen MB on the cheapest VPS, and the dependency list is short on purpose (`ureq`, `quick-xml`, `roxmltree`, `toml`, `serde`, `uuid`, two hashes).

## Goals / Non-Goals

**Goals:**

- The web interface adds a transport and a page, not a second way of doing things: every action is a `control::Request` through `Bot::control`, so RLV restrictions, refusals, placement modes and logging hold without being repeated.
- The main loop never blocks on a browser: it reaches the web side only through events in and non-blocking sends out, as it does with XMPP.
- Everything that decides something is pure code with unit tests: reading a request head, routing, the Host and header checks, sessions and the throttle, the backlog and its replay, the event encoding.
- The page is three static files without a build step, readable in the repository as they are served.

**Non-Goals:**

- A general HTTP server: no keep-alive, no chunked request bodies, no compression, no range requests, no HTTP/2, no TLS.
- A public HTTP API. The requests below are the page's own; scripts have the control socket. They may change with the page.
- Rendering the world. The page shows lists, not a map or a 3D view.

## Decisions

### A small HTTP/1.1 server of our own on `std::net`

The server needs five things: serve three static files, read a small POST body, answer it, hold one long response open, and set a cookie. `tiny_http` would do the first four but pulls `chunked_transfer`, `httpdate` and `ascii`, has its own thread pool, and still leaves sessions, limits and the event stream to us; `axum` or `hyper` mean an async runtime, which the project has avoided so far and which would not fit the main loop. Reading a request head (request line, a handful of headers, `Content-Length`) and writing a response is about as much code as the control socket's line reader, and the limits the spec asks for (8192 bytes of head, 16384 of body, 10 seconds to complete, 32 connections) are then ours to enforce exactly.

Every response carries `Connection: close`, and a connection serves one request. That costs a TCP handshake per action on a page that sends a few requests a minute, and removes pipelining, idle keep-alive connections and request smuggling from what has to be right.

A listener thread accepts; each connection gets a thread, counted against 32 as the control socket counts against `MAX_CONNECTIONS`. `web::listen(config, events) -> Result<Server, String>` binds before the first login and fails as `control::listen` does, which `run_bot` turns into the config error naming `web.listen`.

### Server-sent events for the stream, POST for commands

The page needs messages pushed to it and sends short commands now and then. Server-sent events are a plain HTTP response that never ends (`Content-Type: text/event-stream`), which the server above can write as it is; the browser's `EventSource` reconnects by itself and sends the id of the last event it got (`Last-Event-ID`), which is exactly what the replay rule of the spec needs. WebSocket would need the upgrade handshake (SHA-1 and base64), frame masking and ping/pong for no gain: commands are rare and each wants its own answer, which a POST gives without correlating ids. Polling would either delay chat or load the bot.

A stream costs one thread and one connection for as long as the page is open; of the 32 connections at most 8 may be streams, a further stream is answered as busy and the page's retry takes care of the rest. The stream thread writes a comment line every 15 seconds, so that a connection that is gone is noticed, and checks on each wake that its session still exists.

### The requests

| Request | Meaning | Answer |
|---|---|---|
| `GET /` | the page | `index.html` |
| `GET /app.js`, `GET /style.css` | its script and style | the file |
| `GET /api/session` | does this browser have to log in? | `open`, `in` or `login` |
| `POST /api/login` | body: the password | 204 and the cookie; 403 wrong; 429 too many |
| `POST /api/logout` | | 204, cookie cleared |
| `POST /api/command` | body: one request line as the control socket takes it | the answer line as the control socket writes it |
| `POST /api/console` | body: the line as typed | the same |
| `GET /api/events` | the event stream | `text/event-stream` |

Everything else is 404, another method 405, without a session 401 (the three files and `/api/session` and `/api/login` excepted), a request the checks below refuse 403.

**Commands are control-socket lines, not JSON.** `POST /api/command` carries the command name and its arguments separated by tabs with `\\`, `\t`, `\n`, `\r` escaped, which is what `Request::decode` reads; the answer is `Response::encode`'s line (`ok`, `ok` and a text, or `error` and a text), which the page decodes with the same four escapes. The page builds the line from an array of arguments, so a region named `Joe's "Blue" Lagoon` needs no quoting anywhere. This reuses the decoder with all its checks and its tests, needs no JSON crate in the bot, and the page already has to undo these escapes to read the listings. HTTP status 200 is used for both `ok` and `error`: the command was received and answered; other statuses are the server's own (401, 403, 413, 429, 503).

`POST /api/console` runs `control::split` on the body, then `control::request_line` and `Request::decode`, as `xmpp::gateway` does for the control contact; `help` is answered from the shared help text. `xmpp::command::help` moves to `control::help` (or is re-exported) so that the web console does not depend on the XMPP module being configured.

### Threads and events

```
 browser ── POST /api/command ──> connection thread ── Event::Web(Command{request, reply, own}) ──┐
                                   waits on `reply` up to 75 s                                     v
 browser ── GET /api/events ────> stream thread ───── Event::Web(Subscribe{last, sender}) ──> main loop
                                   ^  writes events, a comment every 15 s                       (owns Hub)
                                   └────────── SyncSender<Arc<str>> (256), try_send ─────────────┘
```

- `Event::Web(WebEvent::Command { request, reply, own })` is handled as `Event::Control` is, through `Bot::control`. The connection thread waits for the answer with the control socket's 75-second limit. A shared counter caps waiting commands at 16; the seventeenth is answered `error` busy without reaching the main loop.
- `Event::Web(WebEvent::Subscribe { last, sender })` registers a stream with the hub, which first sends it the replay (below) and the current status.
- The hub (`web::Hub`, owned by `Bot` like the XMPP gateway) holds the two rings of recent records and the subscribers. Publishing is `try_send` of one shared `Arc<str>` to each subscriber's bounded channel of 256; a full or closed channel removes the subscriber, which makes its stream thread end and the browser reconnect and replay. The main loop never waits for a browser.
- Records reach the hub where they reach XMPP: `service_messages` and `flush_messages` call `hub.record(&record)` beside `logs.write` and `xmpp.record`.
- Status: at the place in `run_loop` where `xmpp_presence()` is called, `web_status()` builds the status text and publishes it when it differs from the one published last. It does so only while the hub has subscribers, so a bot nobody watches formats nothing per event.

### Events and the backlog

An event is written as server-sent events ask: an `event:` line with its type, an `id:` line where it has one, and one `data:` line. The data is a line of tab-separated fields escaped as everywhere else, so it never contains a line end.

| Type | Id | Fields |
|---|---|---|
| `chat`, `im` | yes | time in seconds since the epoch, kind, avatar UUID, avatar name, display name, object UUID, object name, `1` when stored offline else `0`, text |
| `status` | no | the status text as one field |
| `reset` | no | none: drop every message the page holds; the kept ones follow |

Ids are `<start>-<n>`: `<start>` is the second the bot started, `<n>` counts records of both channels from 1. The hub keeps two `VecDeque`s of 500 records, one for chat and one for instant messages, and for each the highest `<n>` it has dropped. On `Subscribe { last }`:

- no `last` (a page just loaded): `reset`, then both rings merged by `<n>`;
- `last` with our `<start>` and an `<n>` not below either ring's highest dropped: the records after `<n>`, merged;
- otherwise (a restart, or more was missed than is kept): `reset`, then both rings.

Then the status, then live events. Since the hub does this inside the main loop, between two records, nothing is missed and nothing is sent twice. The replay and the status go into the page's queue as one piece, however many messages it holds, so that a full backlog of 1000 fits a queue of 256. When a page opens with a status the other pages have not been sent yet, they are sent it then, so that "the status sent last" is one value for all of them. Memory: 1000 records of a few hundred bytes, well under a megabyte, and only while `[web]` is configured.

### Password, sessions and the cookie

- The configured password is a `config::Secret`. A login compares the SHA-1 of what was given with the SHA-1 of the configured one, byte by byte without leaving early: equal lengths, and nothing of the comparison's time depends on the password.
- A session is 32 bytes from `getrandom` as hex, kept in an `Arc<Mutex<Sessions>>` that the web threads share: token, last use, at most 32, pruned of those unused for 30 days. It is not main-loop state; the main loop sees only requests that passed.
- The cookie is `rslbot_<port>=<token>; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000`. The port is in the name because browsers share cookies between the ports of one host, and two bots on one machine would otherwise log each other out.
- The throttle is one counter for the whole server: the times of the last five wrong passwords. It cannot tell the operator from a guesser behind a reverse proxy, where every request comes from `127.0.0.1`, so it does not try; the price is that someone guessing can keep the operator from logging in while they guess, and never from using a session they already have.
- Logging out removes the session; the stream threads of that session end on their next wake, at most 15 seconds later, and the page that sent the logout closes its stream itself.

### Other sites: a required header, no CORS, and the Host check

Two attacks are in scope, and both matter most for the setup the config makes easiest, `listen = "127.0.0.1:8080"` without a password:

1. **A page elsewhere makes the operator's browser send a request to the bot** (a form, an image, `fetch`). Every `POST` must carry the header `X-Rslbot: 1`, which the page's script sets. A form cannot set a header, and a script on another origin can only send one after a preflight `OPTIONS`, which the bot answers 405 without any `Access-Control-*` header, so the browser never sends the real request. The bot sends no CORS header on anything, so another origin can read neither answers nor the event stream. `GET` requests change nothing. This holds behind any reverse proxy, which a comparison of `Origin` with `Host` would not (nginx passes the upstream address as `Host` by default). The cookie's `SameSite=Strict` is a second line where there is a password.
2. **DNS rebinding**: a name the attacker controls resolves to `127.0.0.1`, which makes the attacker's script same-origin with the bot. With a password the script has no session and learns nothing. Without one the bot serves only requests whose `Host` is an IP address literal or `localhost` (with any port), since a rebound name is never either.

The page is served with `Content-Security-Policy: default-src 'self'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`, so that it cannot be framed for clickjacking and an injected string could not load or run anything even if the page had a markup bug. API answers are `Cache-Control: no-store`; the three files carry an `ETag` made from the bot's version and a hash of the file itself, and `Cache-Control: no-cache`, so a new build is picked up on the next load, also one of the same version.

### The page's own commands: `Origin::Page`

The page marks a refresh it makes by itself with the header `X-Rslbot-Own: 1`; the connection thread refuses the mark on anything but `scan avatars ...` and `friends`, and passes `own: true` in the event. `Bot::control` takes the origin of the request: for `own` it never takes the RLV override, and hands the command to `Commands::handle_from` with a new `Origin::Page`, which behaves as `Origin::Operator` does and whose log lines are not written. The scan and friends answers already log through the origin they were started with (`scan.origin.tag()`), so the origin is the one place to say "quiet".

Rejected: leaving the refreshes out and listing avatars and friends only on a button. The list of who hears what the operator types is what the Local chat tab is for, and it goes stale in seconds. Rejected: pushing the list of nearby avatars as events. The RLV rules that hide avatars and close the list (`shownames`, `shownearby`, `showminimap`) live in the `scan avatars` command, and a second path past them would have to repeat them.

### The page

Three files in `src/web/assets/`, built into the binary with `include_str!`: `index.html` (the six tab panels and the login box as static markup), `style.css`, `app.js`. No framework, no bundler, no external font or icon: a few hundred lines of plain JavaScript are enough for six panels of lists and forms, the files stay readable where they are served from, and `cargo build` remains the whole build.

- **State** lives in one object in `app.js`: status, chat lines, conversations keyed by avatar or object UUID, scan results, the inventory tree, console output and history, and what is under way. A panel renders from it; hiding a panel does not discard it.
- **Text** is only ever put on the page with `textContent` and `createElement`; `innerHTML` is not used with anything that came from the bot. This is what "Text from the grid is shown as text" rests on, with the content security policy behind it.
- **Tabs** are the fragment of the address (`#world`, `#chat`, `#im`, `#inventory`, `#console`, `#settings`), which gives reload, bookmarks and back and forward for free.
- **Theme**: `style.css` defines every colour as a custom property under `:root`, with the dark values under `@media (prefers-color-scheme: dark)` guarded by `:root:not([data-theme="light"])` and again under `:root[data-theme="dark"]`. The choice is `localStorage["theme"]` (`light`, `dark`, or absent for the browser's), applied to `data-theme` by a few lines at the top of `app.js`, loaded in `<head>` so the login box has it too. Every read and write of `localStorage` is inside `try`, and the page works without it.
- **The stream** is one `EventSource("/api/events")`. `EventSource` retries by itself; the page shows the "lost the bot" bar from its `error` event and, since the browser's own retry interval is fixed, closes and reopens it on a timer that grows to 30 seconds, passing the last id as a query parameter (`?last=`), which the bot reads as it reads `Last-Event-ID`. A 401 from any request shows the login box and drops the state.
- **A conversation opened by name** sends `im <name> <text>`. The answer `sent to Jane Doe <uuid>` gives the UUID (the form is pinned by a test in `src/commands.rs`); the pending conversation is then merged into that UUID's. If the answer ever has no UUID, the `im_out` record that follows carries it, and the page matches it by the text it just sent.
- **Chat lines** are put together in the page by the rules of `xmpp::chat::chat_line` (display and legacy name, ` whispers`, ` shouts`, `[kind]`, `/me`), from the record's fields. The rules are few and the page needs the fields apart anyway (time, speaker) to lay a line out; the bot's function stays the reference, and a table of cases in the test rig checks the page against it.
- **Refresh timers** run only while their tab is shown and `document.visibilityState` is `visible`.

### `scan --position` and `touch` by item

- `scan::Query` gets `position: bool`, set by `--position`; `scan::listing` writes the sighting's position, which it already has to compute the distance, as a third field when the flag is set. Without the switch the output is byte for byte what it is today, so scripts that read seven fields are not touched. A long name only, as `--temporary` and `--phantom` have: every short letter that would fit is taken or would mislead (`-p` is `--physical`).
- `Commands::touch` asks the grid for the object first, as now; when it is not known, it asks `Grid` for the attachment whose `attach_item` is that UUID (the session keeps `attach_item` per object, and `attached_item(object)` already maps the other way) and touches its root prim. The RLV checks on touching attachments run on the object found, unchanged.

### Config

`[web]` is read like `[xmpp]`: a `toml::Value` taken apart by hand so that every error names `web.<key>`. `listen` is required: a default port would collide as soon as a second bot runs on the machine, and an explicit address makes it a decision where the interface is reachable. The host must be an IP address or `localhost` so that the bot never has to resolve a name at startup. A non-loopback address without a password is accepted and logged loudly rather than refused: a container that publishes the port to `127.0.0.1` of its host has to bind `0.0.0.0` inside.

## Risks / Trade-offs

- [No TLS: a password and a session cookie cross the network in clear when `listen` is not a loopback address] → The README says to keep `listen` on `127.0.0.1` and put a reverse proxy with TLS in front for anything else, with an nginx and a Caddy example; the startup log line names the risk when there is no password.
- [An HTTP parser of our own facing the network] → It reads one request per connection, only `GET` and `POST`, only `Content-Length` bodies, with hard limits, and refuses everything else; unit tests cover malformed heads, oversized parts, slow writers and a body shorter or longer than announced.
- [One thread per open page, held for hours] → Capped at 8 streams and 32 connections; a thread blocked on a channel costs a stack that is mostly untouched. If it ever matters, the streams can move to one thread with non-blocking sockets without changing the hub or the page.
- [Without a password and behind a reverse proxy that passes the public host name on (Caddy's default), every request is refused by the Host check] → The refusal's text says what to do; the README shows `password` as the normal setup behind a proxy.
- [The event stream shows names and UUIDs of speakers also while an RLV restriction such as `shownames` hides them from the scans] → This is what the message log, standard error and XMPP already do with the same records; the page adds no new knowledge. Censoring chat under RLV would be a change to `message-log`, not to this one.
- [A page left open on the Local chat tab asks for the avatar list every 15 seconds] → `scan avatars` answers from what the simulator has already told the bot and sends the simulator nothing new once names are known; the timer stops when the tab or the window is hidden.
- [The page reads the commands' text answers; a change of a listing's form breaks it silently] → The forms are specified (`area-scan`, `avatar-scan`, `friend-list`, `inventory-list`, `avatar-commands`), and the rig items parse real answers.
- [Kept messages are lost at a restart, and a page opened after one shows an empty IM tab] → Stated in the spec; `log_im` and `log_chat` remain the durable record. Reading history back from the TSV files is a possible later change.

## Browsers (task 9.2)

The page was tried in one browser, the Chromium of the Claude desktop app,
at desktop width and with the viewport set to 360 by 740 pixels
(`docs/test-rig.md`, items 250 to 263). It uses nothing newer than
`fetch`, `EventSource`, `classList` and CSS custom properties, so no
difference is expected in Firefox or Safari, but none of them and no phone
was tried, and so nothing here asks for a later change yet. One thing seen
in that browser: while its pane is hidden, timers run about once a minute,
so the page's own refreshes pause, as they are meant to for a window that
is not looked at; events and answers arrive regardless.

## Migration Plan

Nothing changes for a config without `[web]`: no port is opened, no record is kept, `scan` without `--position` prints what it printed, and `touch` with an object's UUID does what it did. Rolling back is removing the table.

## Open Questions

- Whether the session cookie should also carry `Secure` when the request came through a proxy that says `X-Forwarded-Proto: https`. It tightens a correct setup and breaks nothing, but trusts a header; left for the first real deployment behind a proxy.
- Whether the Console should offer the commands' names for completion. The help text has them; no task depends on it.
