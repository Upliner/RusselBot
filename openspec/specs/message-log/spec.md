# message-log Specification

## Purpose

Covers reading the local chat the avatar hears and the instant messages sent to it, finding the names that belong to each message, and recording them on standard error and in optional TSV files. The bot only reads; it never answers.

## Requirements

### Requirement: Read local chat
While connected, the bot SHALL record every local chat message the simulator delivers to the avatar that has text and is audible to it. Each message SHALL be classified by kind: `whisper`, `say`, `shout`, `region`, `owner`, `direct` or `debug`; a chat type the bot does not know SHALL be recorded with kind `type-<number>`. Typing notifications, messages with empty text and messages the simulator marks as not audible SHALL NOT be recorded.

#### Scenario: Avatar speaks nearby
- **WHEN** another avatar within chat range says `hello`
- **THEN** one chat message of kind `say` with that avatar's name and the text `hello` is recorded

#### Scenario: Object speaks nearby
- **WHEN** a scripted object within chat range says `tick` in local chat
- **THEN** one chat message of kind `say` is recorded with the object's name, the object's UUID and the text `tick`

#### Scenario: Typing notification
- **WHEN** another avatar starts and stops typing without sending anything
- **THEN** nothing is recorded

### Requirement: Read instant messages
While connected, the bot SHALL record every instant message sent to the avatar by another avatar (kind `im`) and every instant message sent to it by an object (kind `object`). Other instant-message traffic — typing notifications, group and conference sessions, offers, invitations, notices and automatic responses — SHALL NOT be recorded.

#### Scenario: IM from an avatar
- **WHEN** another avatar sends the bot's avatar the instant message `are you there?`
- **THEN** one instant message of kind `im` with that avatar's name and the text `are you there?` is recorded

#### Scenario: IM from an object
- **WHEN** a script sends the bot's avatar an instant message
- **THEN** one instant message of kind `object` is recorded with the object's name and the object's UUID

#### Scenario: Inventory offer
- **WHEN** another avatar offers the bot's avatar an inventory item
- **THEN** nothing is recorded

### Requirement: Fetch offline instant messages on request
When `fetch_offline_im` is true, the bot SHALL ask the grid once after each successful login for the instant messages stored while the avatar was offline, and SHALL record the ones it receives like any other instant message. When `fetch_offline_im` is false or absent the bot SHALL NOT ask for them.

#### Scenario: Stored IM with fetching on
- **WHEN** an avatar sent the bot's avatar an IM while it was offline, and the bot logs in with `fetch_offline_im = true`
- **THEN** that IM is recorded, with the time it was sent as its timestamp

#### Scenario: Fetching off
- **WHEN** the bot logs in without `fetch_offline_im`
- **THEN** it sends no request for stored instant messages

### Requirement: Names recorded with a message
Each recorded message SHALL carry these fields:

| Field | Message from an avatar | Message from an object |
|---|---|---|
| avatar name | the sender's name as the grid gives it | the name of the object's owner |
| display name | the sender's display name | the owner's display name |
| object name | empty | the object's name |
| object UUID | empty | the object's UUID |

The bot SHALL obtain display names and owners' names from the grid. A name the grid does not supply within 5 seconds of the message's arrival SHALL be left empty; this includes grids without display names and objects owned by a group. A message from the grid itself rather than from an avatar or object SHALL carry the name the grid gave as its avatar name and leave the other three fields empty. Recording a message SHALL NOT be delayed by more than 5 seconds for the sake of a name, and messages SHALL be recorded in the order they arrived.

#### Scenario: Avatar with a display name
- **WHEN** an avatar named `Jane Doe` whose display name is `JD` says something
- **THEN** the record has avatar name `Jane Doe`, display name `JD`, and empty object name and object UUID

#### Scenario: Object owned by an avatar
- **WHEN** an object named `Clock` owned by `Jane Doe` (display name `JD`) says something
- **THEN** the record has avatar name `Jane Doe`, display name `JD`, object name `Clock` and the object's UUID

#### Scenario: Name lookup unavailable
- **WHEN** a message arrives and the grid does not answer the name lookup
- **THEN** the message is recorded within 5 seconds with the unresolved fields empty

#### Scenario: Order is kept
- **WHEN** two messages arrive and the names for the second are known before those for the first
- **THEN** the first message is still recorded before the second

### Requirement: Messages are logged to standard error
The bot SHALL write every recorded chat message and instant message to standard error as one timestamped log line, at the normal log level and regardless of whether a log file is configured. The line SHALL say whether it is chat or an instant message and SHALL contain the kind, the names and UUID described above that are not empty, and the text. Text spanning several lines SHALL be written on one line.

#### Scenario: Chat without any log file
- **WHEN** the config names no log file and an avatar says `hello` nearby
- **THEN** standard error receives one line containing the avatar's name and `hello`

#### Scenario: Multi-line text
- **WHEN** a message's text contains line breaks
- **THEN** it still produces exactly one line on standard error

### Requirement: TSV log files
When the config names a chat file, as `chat` in its `[logs]` table or with the deprecated key `log_chat` (see `bot-config`), the bot SHALL append one row per recorded chat message to that file; when it names an IM file, as `im` in `[logs]` or with the deprecated key `log_im`, it SHALL append one row per recorded instant message to that file. Where a requirement or a scenario of this or another capability says that `log_chat` or `log_im` is set or configured, it means that the config names that file, by either key. A row SHALL be one line of seven tab-separated fields in this order: `time`, `kind`, `avatar_name`, `display_name`, `object_name`, `object_uuid`, `text`. `time` SHALL be UTC in the form `YYYY-MM-DDTHH:MM:SSZ`: the time the bot received the message, or for an instant message stored while the avatar was offline the time it was sent. Inside a field, a tab SHALL be written as `\t`, a line feed as `\n`, a carriage return as `\r` and a backslash as `\\`, so that every row is exactly one line with exactly six tabs. The file SHALL be UTF-8. An existing file SHALL be appended to, never truncated. When the bot creates the file or finds it empty it SHALL first write a header row holding the seven field names. Each row SHALL be on disk as soon as the message is recorded, without waiting for more messages or for the bot to exit.

#### Scenario: Chat row
- **WHEN** `chat` is set under `[logs]` and `Jane Doe` (display name `JD`) says `hello` at 2026-11-18 17:04:23 UTC
- **THEN** the file gains the line `2026-11-18T17:04:23Z`, `say`, `Jane Doe`, `JD`, empty, empty, `hello`, separated by tabs

#### Scenario: Text with a tab and a line break
- **WHEN** a recorded message's text is `a`, a tab, `b`, a line feed, `c`
- **THEN** its row's last field is `a\tb\nc` and the row is a single line

#### Scenario: Restart keeps earlier rows
- **WHEN** the bot is restarted with the same chat file
- **THEN** the rows written before are still there, new rows follow them, and no second header row is written

#### Scenario: IMs go to their own file
- **WHEN** `im` is set under `[logs]`, `chat` is not, and both a chat message and an instant message arrive
- **THEN** the IM file gains one row for the instant message and no file receives the chat message

#### Scenario: Deprecated key
- **WHEN** the config has `log_im = "im.tsv"` and no `[logs]` table, and an instant message arrives
- **THEN** `im.tsv` gains its row, exactly as with `im = "im.tsv"` under `[logs]`

### Requirement: Logging never disturbs the bot
Reading and recording messages SHALL NOT change what the bot does to stay logged in, in the target region and seated. A failure to look up a name or to write to a log file SHALL be reported on standard error and SHALL NOT end the session or the bot; later messages SHALL still be attempted. The bot SHALL NOT send chat or instant messages in response to what it reads; it sends them only on an operator's command, given through the control socket, as an owner's message through the XMPP gateway (see `xmpp-gateway`), or from the page of the web interface (see `web-server`). Messages still waiting for names when the bot stops SHALL be recorded before it exits.

#### Scenario: Log file becomes unwritable
- **WHEN** writing a row fails, for example because the disk is full
- **THEN** standard error reports the failure, the avatar stays logged in and seated, and the message still appears on standard error

#### Scenario: Stopping with messages pending
- **WHEN** the bot is stopped a moment after a message arrived and before its names were resolved
- **THEN** the message is recorded, with the unresolved names empty, before the bot exits

#### Scenario: Message that looks like a command
- **WHEN** another avatar sends the bot's avatar the instant message `say hello`
- **THEN** the message is recorded and the avatar says nothing

#### Scenario: Message that looks like a command, with the gateway on
- **WHEN** the XMPP gateway is configured and another avatar sends the bot's avatar the instant message `say hello`
- **THEN** the message is recorded and passed to the owners as a message from that avatar's contact, and the avatar says nothing

#### Scenario: Message that looks like a command, with the web interface on
- **WHEN** the web interface is configured, a page of it is open, and another avatar sends the bot's avatar the instant message `say hello`
- **THEN** the message is recorded and shown on the page as a message from that avatar, and the avatar says nothing

### Requirement: Record outgoing instant messages
The bot SHALL record every instant message it sends on command as an instant message of kind `im_out`, at the moment the message is handed to the simulator connection. The record's avatar name and display name SHALL be the recipient's, obtained and left empty under the same rules as the names of a received message; its object name and object UUID SHALL be empty; its text SHALL be the text sent. An `im_out` record SHALL be written to standard error and to the `log_im` file like any other recorded instant message, in order with the received ones. An instant message the bot refused to send SHALL NOT be recorded. Chat the bot sends on command SHALL NOT be recorded separately: it is recorded only when the simulator delivers it back as chat the avatar heard.

#### Scenario: IM sent on command
- **WHEN** `log_im` is set and the bot sends `Jane Doe` (display name `JD`) the instant message `are you there?` at 2026-11-18 17:04:23 UTC
- **THEN** the file gains the line `2026-11-18T17:04:23Z`, `im_out`, `Jane Doe`, `JD`, empty, empty, `are you there?`, separated by tabs

#### Scenario: Conversation order
- **WHEN** the bot sends an avatar an instant message and the avatar answers
- **THEN** the `log_im` file has the `im_out` row followed by the `im` row, both with that avatar's name

#### Scenario: Recipient given by UUID
- **WHEN** the bot sends an instant message to a UUID and the grid supplies no name for it within 5 seconds
- **THEN** an `im_out` row is still written, with empty avatar name and display name

#### Scenario: Refused command
- **WHEN** an `im` command fails because the recipient's name is unknown
- **THEN** no `im_out` row is written

#### Scenario: No IM log file
- **WHEN** `log_im` is not configured and the bot sends an instant message on command
- **THEN** standard error receives one line with the kind `im_out`, the recipient's name and the text

### Requirement: Record autoreplies
The bot SHALL record every instant message it sends by itself in answer to one it received, an autoreply (see `im-autoreply`) as well as the reply RLV viewers send to a blocked message (see `rlv-restrictions`, "Instant messages the avatar receives"), as an instant message of kind `im_auto`, at the moment the message is handed to the simulator connection. Its fields SHALL be filled as those of an `im_out` record (see "Record outgoing instant messages"), with the recipient's names and the text sent. An `im_auto` record SHALL be written to standard error and to the `log_im` file like any other recorded instant message, after the record of the message it answers. It SHALL NOT be sent to the XMPP owners. An autoreply the bot did not send SHALL NOT be recorded.

#### Scenario: Autoreply in the IM log
- **WHEN** `log_im` is set, autoreply is on with the text `back later`, and `Jane Doe` (display name `JD`) sends the avatar `hi` at 2026-11-20 10:00:00 UTC
- **THEN** the file gains the `im` row for `hi` followed by the line `2026-11-20T10:00:00Z`, `im_auto`, `Jane Doe`, `JD`, empty, empty, `back later`, separated by tabs

#### Scenario: Not sent to the owners
- **WHEN** the XMPP gateway is connected and the bot sends Jane Doe an autoreply
- **THEN** the owners receive Jane Doe's message and not the autoreply

#### Scenario: Autoreply not sent
- **WHEN** an autoreply to Jane Doe is not sent because a collar holds `sendim`
- **THEN** no `im_auto` row is written
