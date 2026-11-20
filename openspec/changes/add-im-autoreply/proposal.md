# Proposal

## Why

The bot runs unattended, and an avatar who sends it an instant message gets no answer and cannot tell whether anyone will read it. A viewer has Do Not Disturb with an autoresponse for this; the bot needs the same: a configured text that it sends once to each avatar who IMs it, and then not again to that avatar for a while, so that a conversation is not flooded with the same line.

## What Changes

- New config table `autoreply`:

  ```toml
  [autoreply]
  message = "Hello, I'm bot. I won't respond to you in next 2 hours"
  pause = "2h"
  ```

  `message` is the text sent back; `pause` is how long the bot then stays quiet towards that avatar, written as numbers with the units `s`, `m`, `h` and `d` (`30m`, `2h`, `1h30m`, `0` to answer every message). `pause` defaults to one hour. Without the table the bot sends no autoreplies, as today.
- When autoreply is on and an avatar sends the avatar an instant message, the bot answers with the message, unless it has already answered that avatar, or sent it an instant message on command, within the pause. The answer goes out as a viewer's Do Not Disturb autoresponse (IM dialog 20), so viewers show it as automatic and other bots and viewers never answer it back. Objects, the grid itself, group messages and instant messages stored while the avatar was offline get no answer.
- An instant message the operator sends an avatar (`rslbot ctl im`, or an owner's message through the XMPP gateway) also starts that avatar's pause: an avatar the operator is talking to is not told the bot will not answer.
- Autoreplies are recorded in the message log as kind `im_auto`, on standard error and in the `log_im` file, after the message they answer. They are not sent to the XMPP owners.
- While RLV restrictions are in force (see `rlv-restrictions`), an autoreply that `sendim` or `sendimto` bars is not sent; the operator log says so.
- The RLV restrictions `recvim`, `recvim_sec` and `recvimfrom`, which the bot keeps today without an effect, block incoming instant messages from avatars as an RLV viewer does. A blocked message is recorded with its text hidden, and the sender gets the autoresponse RLV viewers send ("The Resident you messaged is currently prevented from reading your instant messages at the moment, please try again later."), every time, whether autoreply is on or not. A blocked message gets no autoreply.
- New control-socket commands, also reachable through the XMPP control contact:
  - `rslbot ctl autoreply message TEXT...` sets the text and switches autoreply on.
  - `rslbot ctl autoreply pause DURATION` sets the pause; it applies at once to the avatars already answered.
  - `rslbot ctl autoreply refresh` forgets every avatar's pause, so the next message from anyone is answered.
  - `rslbot ctl autoreply on` and `off` switch autoreply on and off; `on` keeps the last text and is refused while there is none.
  - `rslbot ctl autoreply status` shows whether autoreply is on, the text, the pause and how many avatars are in their pause.

  Like `perms` and `rlv mode`, these changes are not saved: a restarted bot starts from its config.

## Capabilities

### New Capabilities
- `im-autoreply`: when the bot answers an instant message by itself, with what text and as what kind of message, the per-avatar pause and what starts it, RLV restrictions on autoreplies, and the `autoreply` command.

### Modified Capabilities
- `bot-config`: new key `autoreply`, a table with `message` and `pause`, and its validation.
- `message-log`: autoreplies, and the RLV reply to a blocked message, are recorded as instant messages of kind `im_auto`.
- `rlv-restrictions`: `recvim`, `recvim_sec` and `recvimfrom` move from the restrictions without an effect to those with one; a new requirement says what a blocked instant message does.

## Impact

- Code: a new module for the autoreply state, the pause timers and the duration format; `src/config.rs` (the `autoreply` table); `src/control.rs` (the `autoreply` command, usage text); `src/session.rs` and `src/udp/messages.rs` (sending dialog 20 with the conversation id of a plain IM); `src/main.rs` (answering received IMs, starting the pause on IMs sent on command, recording `im_auto`); `src/xmpp/chat.rs` (not forwarding `im_auto` to the owners); `src/commands.rs` and `src/rlv.rs` (the RLV checks for an autoreply and for a received message).
- Behaviour without an `[autoreply]` table is as today.
- Docs: `README.md`, `russelbot.example.toml`, `docs/test-rig.md`; `test-rig/listen.py` reports the dialog of the IMs it receives and does not answer autoresponses.
- No new dependencies.
