# Spec Delta

## Purpose

Lets the bot answer an avatar who sends it an instant message with a configured text, once per pause per avatar, as a viewer's Do Not Disturb autoresponse does, and lets the operator change that text, the pause and whether it is on while the bot runs.

## ADDED Requirements

### Requirement: Autoreply setting
The bot SHALL keep an autoreply setting made of: whether autoreply is on, the autoreply text (which may be missing), and the pause, a duration in the form `bot-config` gives. The bot SHALL start with the values its config gives (see `bot-config`, `autoreply`): with an `[autoreply]` table, on, with that table's `message` and `pause`; without one, off, with no text and a pause of one hour. The `autoreply` command (see "Autoreply command") SHALL change the setting while the bot runs. A change SHALL NOT be written anywhere: a restarted bot starts from its config again. The setting SHALL be kept across lost sessions and logins.

#### Scenario: Started from the config
- **WHEN** the config has `[autoreply]` with `message = "Away"` and `pause = "2h"`, and `rslbot ctl autoreply status` is run
- **THEN** the answer says autoreply is on, with the text `Away` and the pause `2h`

#### Scenario: Changes are not saved
- **WHEN** `rslbot ctl autoreply pause 30m` is run and the bot is restarted with the same config
- **THEN** the pause is again the config's `2h`

### Requirement: Which instant messages are answered
While autoreply is on, the bot SHALL answer with the autoreply text every instant message of kind `im` that an avatar sends the avatar (see `message-log`, "Read instant messages") and that arrives while the sender is not in its pause (see "Pause per avatar"). It SHALL NOT answer:

- an instant message from an object or from the grid itself;
- an instant message stored while the avatar was offline;
- any instant-message traffic that `message-log` does not record, among it group and conference messages, typing notifications, offers and automatic responses, including other viewers' and bots' autoresponses;
- an instant message that RLV blocks (see `rlv-restrictions`, "Instant messages the avatar receives"), which gets the RLV reply in its place.

While autoreply is off, the bot SHALL answer no instant message with the autoreply text. The received message SHALL be recorded as it is without autoreply, whether or not it is answered.

#### Scenario: First message answered
- **WHEN** autoreply is on with the text `Hello, I'm bot. I won't respond to you in next 2 hours` and the pause `2h`, and Jane Doe sends the avatar `hi`
- **THEN** Jane Doe receives that text as an autoresponse from the avatar

#### Scenario: Object
- **WHEN** autoreply is on and a scripted object sends the avatar an instant message
- **THEN** no autoreply is sent

#### Scenario: Stored message
- **WHEN** autoreply is on, `fetch_offline_im = true`, and the bot fetches an instant message Jane Doe sent while the avatar was offline
- **THEN** the message is recorded and no autoreply is sent

#### Scenario: Another bot's autoresponse
- **WHEN** two bots both have autoreply on and one sends the other an instant message
- **THEN** the other answers it with one autoresponse, and the first bot does not answer that autoresponse

#### Scenario: Off
- **WHEN** autoreply is off and Jane Doe sends the avatar `hi`
- **THEN** no autoreply is sent and the message is recorded

### Requirement: Pause per avatar
For every avatar the bot SHALL keep the time its pause started, if any. The pause of an avatar SHALL start, or start again, when the bot sends it an autoreply and when the bot sends it an instant message on command: through the `im` command (see `message-send`) or an owner's message through the XMPP gateway (see `xmpp-gateway`). An avatar SHALL be in its pause while less time than the current pause setting has passed since its pause started; a change of the pause setting SHALL therefore apply at once to every avatar. With a pause of `0` no avatar is ever in its pause. An instant message the avatar receives SHALL NOT start or extend the sender's pause. The pauses SHALL be kept across lost sessions and logins and while autoreply is off, and SHALL be forgotten when the bot stops and on `autoreply refresh`. The RLV viewers' reply to a blocked message SHALL NOT start a pause.

#### Scenario: Second message within the pause
- **WHEN** autoreply is on with the pause `2h`, Jane Doe sends the avatar `hi`, and sends `hello?` an hour later
- **THEN** only `hi` is answered

#### Scenario: Message after the pause
- **WHEN** autoreply is on with the pause `2h`, Jane Doe sends the avatar `hi` and sends `hello?` three hours later
- **THEN** both are answered

#### Scenario: Messages within the pause do not extend it
- **WHEN** autoreply is on with the pause `2h`, and Jane Doe sends the avatar a message every 30 minutes for three hours
- **THEN** the first message and the first one sent two hours or more after it are answered, and no other

#### Scenario: Pauses are per avatar
- **WHEN** autoreply is on, Jane Doe sends the avatar `hi`, and John Roe sends `hi` a minute later
- **THEN** both are answered

#### Scenario: Operator answers
- **WHEN** autoreply is on with the pause `2h`, the operator runs `rslbot ctl im "Jane Doe" one moment`, and Jane Doe sends the avatar `ok` ten minutes later
- **THEN** no autoreply is sent to Jane Doe

#### Scenario: Owner answers through XMPP
- **WHEN** autoreply is on and an owner sends `brb` to `jane.doe@<domain>`, and Jane Doe answers the avatar within the pause
- **THEN** no autoreply is sent to Jane Doe

#### Scenario: Shorter pause applies at once
- **WHEN** autoreply is on with the pause `2h`, Jane Doe was answered 40 minutes ago, `rslbot ctl autoreply pause 30m` is run, and Jane Doe sends the avatar `hi`
- **THEN** Jane Doe is answered

#### Scenario: Pause zero
- **WHEN** autoreply is on with the pause `0` and Jane Doe sends the avatar three messages in a row
- **THEN** each of them is answered

#### Scenario: Pause kept over a relogin
- **WHEN** autoreply is on with the pause `2h`, Jane Doe was answered, the session is lost and the bot logs in again, and Jane Doe sends the avatar `hi` within the pause
- **THEN** no autoreply is sent

### Requirement: How an autoreply is sent
An autoreply SHALL be sent to the sender of the message it answers as an instant message with the dialog viewers use for their Do Not Disturb autoresponse (IM dialog 20, `IM_BUSY_AUTO_RESPONSE`), in the same conversation as a plain instant message between the two avatars, with the autoreply text as it is set, line breaks included. It SHALL be sent at once, without waiting for the sender's name. It SHALL be recorded as kind `im_auto` (see `message-log`, "Record autoreplies").

#### Scenario: Shown as automatic
- **WHEN** Jane Doe, in a viewer, sends the avatar `hi` while autoreply is on
- **THEN** her viewer shows the autoreply text in her conversation with the avatar, marked as an automatic response

#### Scenario: Multi-line text
- **WHEN** the autoreply text is `Away.` and `Back tonight.` on two lines
- **THEN** the sender receives both lines in one message

### Requirement: RLV restrictions on autoreplies
While RLV restrictions are in force (see `rlv-restrictions`, "When restrictions are in force"), an autoreply SHALL NOT be sent while `sendim` is in force against the sender, with `sendim:<uuid>` as its exceptions, or while a holder holds `sendimto:<uuid>` with the sender's UUID. `startim` and `startimto` SHALL NOT hold back an autoreply, since the sender has started the conversation. When an autoreply is held back the bot SHALL write a line to the operator log naming the sender, the restriction and one holder of it, SHALL send nothing in its place, and SHALL NOT start the sender's pause. `rlv mode override` SHALL NOT exempt an autoreply.

#### Scenario: Autoreply held back
- **WHEN** autoreply is on, a collar holds `sendim`, and Jane Doe sends the avatar `hi`
- **THEN** Jane Doe receives nothing, and the operator log names Jane Doe, `sendim` and the collar

#### Scenario: Restriction lifted
- **WHEN** the collar then lifts `sendim` and Jane Doe sends the avatar `hi` again
- **THEN** Jane Doe receives the autoreply

#### Scenario: Startim does not count
- **WHEN** autoreply is on, a collar holds `startim`, and John Roe, who has never sent the avatar a message, sends it `hi`
- **THEN** John Roe receives the autoreply

### Requirement: Autoreply command
The control socket SHALL accept the command `autoreply` with these forms:

| Form | Effect |
|---|---|
| `autoreply message TEXT...` | Sets the autoreply text to the remaining arguments joined with single spaces, and switches autoreply on |
| `autoreply pause DURATION` | Sets the pause; DURATION is one argument in the duration form `bot-config` gives |
| `autoreply refresh` | Forgets the pause of every avatar, so that the next message from any avatar is answered |
| `autoreply on` | Switches autoreply on with the text it has |
| `autoreply off` | Switches autoreply off; the text, the pause and the avatars' pauses are kept |
| `autoreply status` | Changes nothing |

Every form SHALL answer with success and a text that gives what it changed; `status` SHALL answer with whether autoreply is on, the text (or that there is none), the pause, and how many avatars are in their pause. `refresh` SHALL say how many avatars' pauses it forgot. `on` while autoreply is on and `off` while it is off SHALL succeed and say that nothing changed. The command SHALL be answered with an error and change nothing for: no form or an unknown one; `message` with a text that is blank or longer than 1023 bytes; `pause` without exactly one argument or with one that is not a duration of at most 365 days; `refresh`, `on`, `off` or `status` with arguments; and `on` while there is no autoreply text, the error saying that `autoreply message` sets one. The command SHALL be carried out whether or not the bot is in a region, SHALL NOT be refused by any RLV restriction, and SHALL be accepted through the XMPP control contact as through the control socket. Every change SHALL be written to the operator log. `rslbot ctl` with no arguments SHALL list the `autoreply` forms in its usage.

#### Scenario: New text
- **WHEN** autoreply is off and `rslbot ctl autoreply message "New autoreply from the bot"` is run
- **THEN** the command exits with status 0, autoreply is on, and the next message from an avatar not in its pause is answered with `New autoreply from the bot`

#### Scenario: Text of several arguments
- **WHEN** `rslbot ctl autoreply message back in an hour` is run
- **THEN** the autoreply text is `back in an hour`

#### Scenario: New pause
- **WHEN** `rslbot ctl autoreply pause 30m` is run
- **THEN** the command exits with status 0 and the pause is 30 minutes

#### Scenario: Malformed pause
- **WHEN** `rslbot ctl autoreply pause soon` is run
- **THEN** the command exits with status 1 with a text that gives the duration form, and the pause is unchanged

#### Scenario: Refresh
- **WHEN** autoreply is on with the pause `2h`, Jane Doe and John Roe were answered ten minutes ago, `rslbot ctl autoreply refresh` is run, and Jane Doe sends the avatar `hi`
- **THEN** the command answers that it forgot 2 pauses, and Jane Doe is answered

#### Scenario: Off and on
- **WHEN** `rslbot ctl autoreply off` is run, Jane Doe sends the avatar `hi`, then `rslbot ctl autoreply on` is run and John Roe sends `hi`
- **THEN** Jane Doe gets no autoreply and John Roe gets the text that was set before `off`

#### Scenario: On without a text
- **WHEN** the config has no `[autoreply]` table and `rslbot ctl autoreply on` is run
- **THEN** the command exits with status 1 naming `autoreply message`, and autoreply stays off

#### Scenario: Between logins
- **WHEN** the bot is between logins and `rslbot ctl autoreply pause 10m` is run
- **THEN** the command exits with status 0

#### Scenario: Through XMPP
- **WHEN** an owner sends `autoreply off` to the control contact
- **THEN** autoreply is switched off and the owner receives the command's answer
