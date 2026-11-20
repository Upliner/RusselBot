# Design

## Context

How instant messages move through the bot today, as read in the code:

- `src/session.rs` turns an `ImprovedInstantMessage` into `SessionEvent::InstantMessage` for dialog 0 (kind `im` from an avatar, or from the grid when the sender is nil) and dialog 19 (kind `object`). Every other dialog, the Do Not Disturb autoresponse (20) among them, is dropped with a debug line. A stored message carries `sent_at`.
- `Bot::apply` in `src/main.rs` handles `SessionEvent::InstantMessage`: it tells `Commands::im_received` that the avatar is talking to the bot (for `startim`) and records the message with `Bot::record`.
- IMs sent on command (`ctl im` and an owner's XMPP message both go through `Commands::handle_from` → `After::Im`) come back to `Bot` as a `SentIm` and are recorded as `im_out` by `Bot::record_sent`, from four call sites.
- `messages::instant_message` gives a plain IM (dialog 0) the conversation id `agent ^ to`, the id viewers compute for a one-to-one session, and every other dialog the nil id.
- RLV: `Limits::against` and `Limits::names` answer "is this behaviour in force against this target" with the holder; `Bot::own_limits(&self.rlv, false, None)` gives the limits for the bot's own actions. `recvim`, `recvim_sec` and `recvimfrom` are in the list of behaviours kept without an effect.
- `src/xmpp/chat.rs::delivery` forwards every `Channel::Im` record to the owners except kind `im_out`.
- There is no duration parser; `requesttp_timeout` is a number of minutes.

## Goals / Non-Goals

**Goals:**
- Autoreply decisions as pure functions over plain data (the setting, a pause map, `now`), testable like `perms.rs` and `persist.rs`.
- One place in `Bot` where a received IM is decided: blocked by RLV, answered, or left.
- No loop between two bots that both answer.

**Non-Goals:**
- Saving runtime changes or pauses across restarts.
- Per-avatar texts, exceptions lists, or schedules (on at night only).
- Answering objects, group chats or stored messages.
- The local, viewer-side text RLVa shows its user in place of a blocked IM; the bot has its own (`(blocked by RLV: …)`).
- `recvchat`, `recvemote` and the other receive restrictions; only the IM ones get an effect.

## Decisions

### 1. A new module `src/autoreply.rs` owns the setting, the pauses and durations

It holds `Autoreply { on: bool, text: Option<String>, pause: Duration, started: HashMap<Uuid, Instant> }` with:

- `answer(&mut self, from, now) -> Option<String>`: the text to send when on, a text is set, and `from` is not in its pause. It does not start the pause; the caller does that once the message is really sent (`sent(to, now)`), because an RLV refusal must leave the pause alone.
- `sent(to, now)`: starts the pause; called for autoreplies and for every `SentIm`.
- `refresh() -> usize`, `set_text`, `set_pause`, `switch(on) -> bool` (whether it changed), `status() -> String`.
- `parse_duration(&str) -> Result<Duration, String>` and `format_duration(Duration) -> String` (largest units first, `0` for zero), shared by the config and the command.

The pause map is pruned in `sent`: entries whose pause has run out under the current setting are dropped. A later, longer `pause` setting cannot bring them back, which is harmless: they would only suppress a reply. This keeps the map bounded by the avatars answered within one pause.

*Alternative:* keep the pauses in `Commands` next to `talking`. Rejected: `talking` is per session and cleared on loss, pauses outlive sessions, and `Commands` is already the largest file.

### 2. The decision for a received IM is made in `Bot::apply`, before recording

For `SessionEvent::InstantMessage` with an avatar source and kind `im`:

1. If RLV restrictions are in force and `recvim` (against the sender) or `recvimfrom` (naming it) holds, the message is blocked: its text is replaced by `(blocked by RLV: <rule>)` before `record`, and — for a live message (`sent_at` is `None`) — the RLV reply is sent and recorded as `im_auto`. Nothing else happens.
2. Otherwise the message is recorded as today, then, for a live message, `Autoreply::answer` is asked. With a text, the `sendim`/`sendimto` check runs (`Limits::im(from, true)`, which with `talking = true` skips `startim`); refused → one `info!` line naming sender, rule and holder; allowed → send, `sent(from, now)`, record `im_auto`.

The received message is recorded before the reply so that the log has them in that order (the recorder keeps push order). `rlv mode override` is not consulted: it exempts one *command*, and neither the reply nor the block is one.

The RLV check is a new `Limits::recvim(from) -> Option<Refusal>` in `src/rlv.rs` beside `im`, with `recvim`, `recvim_sec`, `recvimfrom` moved to the list of behaviours with an effect. Object (`kind = object`) and grid messages never reach it.

### 3. Autoresponses are sent with dialog 20 in the plain IM's conversation

`Session::autoresponse(to, text, now)` sends `messages::IM_BUSY_AUTO_RESPONSE` (new constant, 20) through `send_im`. `messages::instant_message` gives dialog 20 the same `agent ^ to` conversation id as dialog 0, as viewers do, so the reply lands in the sender's open IM window instead of a stray session. Viewers do not answer dialog 20, and the bot already drops it, so two bots cannot loop.

*Alternative:* a plain IM. Rejected by the user: it would look typed and another bot would answer it.

### 4. `im_auto` is a third IM kind in the recorder

`Bot::record_auto(to, name, text, now)` builds the record like `record_sent` with kind `im_auto`; names are filled in by the recorder's lookups as for `im_out`. `xmpp/chat.rs::delivery` drops `im_auto` as it drops `im_out`.

### 5. Every `SentIm` starts the pause

`Bot::record_sent` is the single funnel for IMs sent on command (ctl and XMPP alike), so calling `autoreply.sent(sent.to, now)` there covers both without touching `Commands`.

### 6. The command is parsed in `control.rs` and handled in `Bot`

`Request::Autoreply(AutoreplyRequest)` with `Message(String)`, `Pause(String)`, `Refresh`, `On`, `Off`, `Status`. Like `persist`, `home` and `status`, it is the bot's own business: `Bot` handles it directly against its `Autoreply` and answers at once; it needs no session and is in the never-refused list. The message text is the remaining arguments joined with spaces, checked with the same rules as `im` (`check_text`: not blank, ≤ 1023 bytes). The XMPP control contact parses through the same `Request::parse`, so it gets the command for free.

### 7. Config

`Raw` gains `autoreply: Option<toml::Value>`, parsed by hand like `xmpp` so that errors name `autoreply.message` / `autoreply.pause`. `Config::autoreply: Option<AutoreplyConfig { message, pause }>`; `Autoreply::new(config)` builds the starting state.

## Risks / Trade-offs

- [The reply must match what RLV viewers send] → checked against Firestorm's RLVa (task 1.1): `skins/default/xui/en/rlva_strings.xml` key `blocked_recvim_remote`, sent by `RlvUtil::sendBusyMessage` with `IM_DO_NOT_DISTURB_AUTO_RESPONSE` (20) in the incoming IM's session, skipped for offline IMs and muted senders. The bot has no mute list, so it answers every live blocked message; RLVa also never blocks Linden accounts, which the bot does not tell apart.
- [OpenSim may not pass dialog 20 on, or a viewer may hide it] → checked on the rig with `listen.py` (which then prints the dialog) and with a viewer; if OpenSim drops it, the spec still holds for Second Life and the rig note says so.
- [An autoreply answers a sender whose viewer also has an autoresponse] → dialog 20 is never answered by viewers or by the bot, so at most one reply each way.
- [A busy sender gets one reply per pause even if the operator is watching through XMPP] → the operator's own reply starts the pause; `autoreply off` is one command away.
- [`refresh` while many avatars are paused] → the map is small (pruned per decision 1); clearing it is O(n).

## Migration Plan

None: without `[autoreply]` the bot answers nothing, as today. The only change for an existing setup is that a kept `recvim`/`recvimfrom` now blocks IMs; a bot running with RLV on and such a restriction held will start hiding those IMs after the upgrade. The README's RLV section says so.

