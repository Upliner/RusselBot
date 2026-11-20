# Tasks

## 1. Check the RLV viewers' reply

- [x] 1.1 Read the RLVa source (Firestorm or RLVa repository, `rlva_strings.xml` key `blocked_recvim_remote`, and the `recvim` branch of `llimprocessing.cpp` with `RlvUtil::sendBusyMessage`) and confirm the reply text and that it goes out with dialog 20 in the incoming session; when either differs, correct the `rlv-restrictions` delta and `design.md` before any code. Done when the spec's quoted string matches the source

## 2. Durations and the autoreply state (`src/autoreply.rs`)

- [x] 2.1 Add `parse_duration` and `format_duration`: groups of a whole number and `d`, `h`, `m`, `s` in that order, each unit once, no spaces, or `0`; at most 365 days; unit tests for `90s`, `30m`, `2h`, `1h30m`, `0`, and the refusals `2 hours`, `30`, `30m2h`, `1h1h`, `400d`, empty, overflow
- [x] 2.2 Add `Autoreply` (on, optional text, pause, pause start per avatar) with `answer`, `sent` (with pruning of run-out pauses), `refresh` (returning the count), `set_text`, `set_pause`, `switch`, and the status text; unit tests for every scenario of "Pause per avatar" and "Which instant messages are answered" that needs only the state (first message, second within the pause, after the pause, not extended by received messages, per avatar, shorter pause applies at once, pause `0`, off, refresh, on without a text)

## 3. Config

- [x] 3.1 Parse the `autoreply` table in `src/config.rs` into `Option<AutoreplyConfig>` (message, pause defaulting to one hour), rejecting what the `bot-config` delta lists with errors naming `autoreply`, `autoreply.message` or `autoreply.pause`; unit tests, one per scenario of the delta
- [x] 3.2 Add a commented `[autoreply]` block to `russelbot.example.toml` (before or after `[xmpp]`, as tables have to come last) explaining the text, the pause form, what is answered and the commands; verify the existing example-file tests in `src/config.rs` still pass and add one that uncomments the block and checks the parsed `autoreply`

## 4. Sending autoresponses

- [x] 4.1 Add `IM_BUSY_AUTO_RESPONSE = 20` to `src/udp/messages.rs` and give dialog 20 the `agent ^ to` conversation id in `instant_message`; wire test that the built message has dialog 20, that id and the text
- [x] 4.2 Add `Session::autoresponse(to, text, now)` through `send_im`; session test with the rig helpers that it sends one `ImprovedInstantMessage` with dialog 20, and that it fails while not present

## 5. Answering in the bot (`src/main.rs`)

- [x] 5.1 Own an `Autoreply` in `Bot`, built from the config; call `sent(to, now)` in `record_sent`; add `record_auto` (kind `im_auto`)
- [x] 5.2 In `apply` for `SessionEvent::InstantMessage` of kind `im` from an avatar: record, then for a live message (`sent_at` is `None`) ask `answer`; check `Limits::im(from, true)` from `own_limits`; on refusal log sender, rule and holder; otherwise send with `autoresponse`, `sent`, `record_auto`. Bot-level tests: answered once within a pause, object and stored messages not answered, `sendim` holds it back with the log line and no pause, `startim` does not, `ctl im` then a received IM gives no reply, recorded order is `im` then `im_auto`
- [x] 5.3 Drop `im_auto` in `src/xmpp/chat.rs::delivery` like `im_out`, and update its doc comment; unit test that an `im_auto` record has no delivery
- [x] 5.4 Update the `message-log` part of `README.md` (the kinds `im_out` and `im_auto`) and verify the documented row matches the "Autoreply in the IM log" scenario

## 6. RLV: blocking received IMs

- [x] 6.1 In `src/rlv.rs`, move `recvim`, `recvim_sec`, `recvimfrom` to the behaviours with an effect and add `Limits::recvim(from) -> Option<Refusal>` (`recvim` with its exceptions and `_sec`/`permissive` rules, `recvimfrom:<uuid>`; group options without effect); unit tests mirroring the `sendim` ones, plus the changed "Kept without an effect" scenario now using `recvchat`
- [x] 6.2 In `Bot::apply`, before recording an `im` from an avatar: when `recvim` blocks it, replace the text with `(blocked by RLV: <rule>)`, record it, and for a live message send the RLV reply with `autoresponse` and record it as `im_auto`, without asking `Autoreply` and without starting a pause. Bot-level tests for every scenario of "Instant messages the avatar receives" (blocked, exception, `recvimfrom` for one sender, autoreply on, RLV off, objects not blocked, stored message without a reply)
- [x] 6.3 Update the RLV section of `README.md`: `recvim`, `recvim_sec` and `recvimfrom` now hide IMs and answer the sender; note for upgrades that a held `recvim` takes effect at once

## 7. Command

- [x] 7.1 Parse `autoreply message TEXT...`, `pause DURATION`, `refresh`, `on`, `off`, `status` in `src/control.rs` into `Request::Autoreply`, with the errors of the spec (no form, unknown form, arguments where none are taken, `pause` without exactly one argument); add the forms to `CTL_USAGE`; parser unit tests
- [x] 7.2 Handle `Request::Autoreply` in `Bot` like `persist`/`status` (no session needed, never refused by RLV, not exempted by override); `message` checks the text as `im` does; answers and operator log lines as the spec says. Tests through the control request path for each scenario of "Autoreply command", including between logins and through the XMPP control contact
- [x] 7.3 Document the command in `README.md` next to `perms` and verify each documented example parses with the 7.1 parser tests

## 8. Rig

- [x] 8.1 Make `test-rig/listen.py` print the dialog of every IM it receives and not answer dialog 20; verify by running it against the bot with autoreply on
- [x] 8.2 Add an "Autoreply" section to `docs/test-rig.md` and run it: config `[autoreply]` with `pause = "2m"`, `talk.py` twice within two minutes (one `im_auto` row, `listen.py` shows dialog 20), again after two minutes (a second one), `ctl autoreply refresh` then `talk.py` (answered), `ctl im` to `Rig Owner` then `talk.py` (not answered), `ctl autoreply off` (not answered), `ctl rlv cmd @recvim=n` then `talk.py` (RLV reply, hidden text in `im.tsv`), and, if a viewer is at hand, how the viewer shows the autoresponse. Record pass or fail per row; use the rig's own `RUSSELBOT_SOCK`
- [x] 8.3 `cargo clippy --all-targets` and `cargo test` clean
