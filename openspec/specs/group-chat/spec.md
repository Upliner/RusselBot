# group-chat Specification

## Purpose

Reading the chat and the notices of the groups the bot's avatar is in, recording them on standard error and in an optional TSV file, and saying a line in a group's chat on the operator's command. The groups themselves and the other `group` commands are `groups`.

## Requirements

### Requirement: Read group chat
While connected, the bot SHALL record every line of chat with text that the grid delivers to the avatar in the chat session of a group the avatar is in, as a group message of kind `chat`, with the group, the speaker and the text. It SHALL read a line in whichever of the two ways the grid delivers it: as an instant message of the simulator connection, or, as grids deliver the first line of a session, as an event of the simulator's event queue. A line the grid delivers back to the avatar that the avatar itself said SHALL NOT be recorded a second time (see "Record lines the bot says"). Lines of a session that is not the session of one of the avatar's groups, such as a conference, typing notifications, and the grid's messages about who is in a session SHALL NOT be recorded.

#### Scenario: Someone speaks in a group
- **WHEN** the avatar is in `Builders` and Jane Doe says `anyone around?` in the group's chat
- **THEN** one group message of kind `chat` with the group `Builders`, Jane Doe's name and the text `anyone around?` is recorded

#### Scenario: First line of a session
- **WHEN** nobody has spoken in `Builders` since the bot logged in, and Jane Doe then says `hello` and `anyone?`
- **THEN** both lines are recorded, in that order, though the grid delivered the first as an event and the second as an instant message

#### Scenario: The avatar's own line comes back
- **WHEN** the bot says `hi` in `Builders` on command and the grid delivers that line back to the avatar
- **THEN** one group message of kind `chat_out` is recorded and no `chat` one

#### Scenario: Conference
- **WHEN** Jane Doe starts a conference with the avatar and John Roe and says `hi both`
- **THEN** nothing is recorded

### Requirement: Staying in a session
When the grid offers the avatar a group's chat session with the session's first line, the bot SHALL take the session up as a viewer does that shows the line, where the grid has a service for that, so that the grid goes on delivering the session's lines. It SHALL need no command for that and SHALL NOT leave a session by itself.

#### Scenario: Later lines keep coming
- **WHEN** Jane Doe says three lines in `Builders` over ten minutes, on a grid that delivers later lines only to who has taken the session up
- **THEN** all three are recorded

#### Scenario: Grid without such a service
- **WHEN** the simulator offers no service for taking up a session
- **THEN** the bot sends nothing for it and records the lines the grid delivers

### Requirement: Read group notices
While connected, the bot SHALL record every group notice the grid delivers to the avatar as a group message of kind `notice`, with the group, the sender as far as the notice names it, the notice's subject, its text, and the name of the item attached to it when it has an attachment. A notice the grid stored while the avatar was offline and delivers on the bot's request (see `message-log`, "Fetch offline instant messages on request") SHALL be recorded with the time it was sent and marked as stored. The bot SHALL NOT take or refuse a notice's attachment and SHALL send nothing in answer to a notice.

#### Scenario: Notice
- **WHEN** Jane Doe sends the group `Builders` a notice with the subject `Meeting` and the text `Tonight at eight`
- **THEN** one group message of kind `notice` is recorded with the group `Builders`, Jane Doe's name, the subject `Meeting` and the text `Tonight at eight`

#### Scenario: Notice with an attachment
- **WHEN** a notice carries the notecard `Agenda`
- **THEN** its record names `Agenda` as the attachment, nothing is added to the avatar's inventory, and no answer is sent

#### Scenario: Notice without a subject
- **WHEN** a notice's text holds no subject
- **THEN** it is recorded with an empty subject and its whole text

#### Scenario: Stored notice
- **WHEN** a notice was sent at 09:00 while the avatar was offline and the bot logs in at 10:00 with `fetch_offline_im = true` on a grid that stores notices
- **THEN** the notice is recorded with 09:00 as its time

### Requirement: Group chat command
`group chat <group> <message>` SHALL say the message in the chat of that group (see `groups`, "Naming a group"), as a viewer's group chat window sends a line. The words of the message MAY be separate arguments and SHALL be joined by single spaces, as `im` joins them. A message that is empty or longer than 1023 bytes SHALL be refused with an error, and nothing SHALL be sent. When the avatar is not in the group's session yet, the bot SHALL enter it first, as a viewer does when its user opens the group's chat, and SHALL wait for the grid's word on that for up to 10 seconds: when the grid refuses, or has not answered by then, the command SHALL be answered with an error holding the grid's reason, when it gave one, and the line SHALL NOT be sent later. The command SHALL be answered with success, naming the group, once the line has been handed to the simulator connection.

#### Scenario: Saying a line
- **WHEN** `rslbot ctl group chat Builders see you at eight` is run
- **THEN** the members of `Builders` that are online read `see you at eight` from the bot's avatar in the group's chat, and the command exits with status 0 naming `Builders`

#### Scenario: First line after login
- **WHEN** nobody has spoken in `Builders` since the bot logged in and `rslbot ctl group chat Builders hello` is run
- **THEN** the bot enters the session and then says `hello`, and the command is answered once the line was sent

#### Scenario: Session refused
- **WHEN** the grid answers that the avatar may not enter the session of `Builders`
- **THEN** the command exits with status 1 with the grid's reason, and nothing is said

#### Scenario: Too long
- **WHEN** the message is 2000 bytes long
- **THEN** the command exits with status 1 naming 1023 bytes, and nothing is said

### Requirement: Record lines the bot says
The bot SHALL record every line it says in a group's chat on command as a group message of kind `chat_out`, at the moment the line is handed to the simulator connection, with the group, the names of the bot's own avatar and the text sent. A line the bot refused to send SHALL NOT be recorded.

#### Scenario: Line said on command
- **WHEN** `rslbot ctl group chat Builders hello` is carried out at 2026-12-01 12:00:00 UTC
- **THEN** one group message of kind `chat_out` with the group `Builders`, the avatar's own name and the text `hello` is recorded with that time

#### Scenario: Refused line
- **WHEN** a `group chat` command fails because the avatar is not in the group
- **THEN** nothing is recorded

### Requirement: Fields of a group message
Each recorded group message SHALL carry: `time`, when the bot received or sent it, or for a stored notice when it was sent; `kind`; `group_name` and `group_uuid`; `avatar_name` and `display_name`, the names of who said or sent it; `subject` and `attachment`, the subject of a notice and the name of the item attached to it, both empty for chat and each empty for a notice without one; and `text`. The group's name SHALL be the one the bot knows for the group (see `groups`, "The avatar's groups"), or else the name the message carries. The avatar's name SHALL be the one the message carries; the display name SHALL be obtained and left empty as `message-log`, "Names recorded with a message", says for a message from an avatar, and SHALL be empty for a notice that does not give its sender's UUID. Recording a group message SHALL NOT be delayed by more than 5 seconds for the sake of a name, and group messages SHALL be recorded in the order they arrived, among themselves and with local chat and instant messages.

#### Scenario: Speaker with a display name
- **WHEN** Jane Doe, display name `JD`, says `hello` in `Builders`
- **THEN** the record has the group name `Builders` with its UUID, avatar name `Jane Doe`, display name `JD`, empty subject and attachment, and the text `hello`

#### Scenario: Name lookup unavailable
- **WHEN** a line arrives and the grid does not answer the lookup of the speaker's display name
- **THEN** the line is recorded within 5 seconds with the display name empty

### Requirement: Group messages are logged to standard error
The bot SHALL write every recorded group message to standard error as one timestamped log line, at the normal log level and regardless of whether a group file is configured. The line SHALL say that it is a group message and SHALL contain the kind, the group's name, the names that are not empty, for a notice its subject and its attachment's name when it has them, and the text. Text spanning several lines SHALL be written on one line.

#### Scenario: Chat without a group file
- **WHEN** the config has no `group` under `[logs]` and Jane Doe says `hello` in `Builders`
- **THEN** standard error receives one line containing `Builders`, Jane Doe's name and `hello`

#### Scenario: Notice of several lines
- **WHEN** a notice's text has four lines
- **THEN** it produces exactly one line on standard error

### Requirement: Group file
When the config names a group file, as `group` in its `[logs]` table (see `bot-config`), the bot SHALL append one row per recorded group message to that file: one line of nine tab-separated fields in this order: `time`, `kind`, `group_name`, `group_uuid`, `avatar_name`, `display_name`, `subject`, `attachment`, `text`. Time, escaping, encoding, appending, the header row and writing at once SHALL be as `message-log`, "TSV log files", says for its files. Group messages SHALL be written to no other file.

#### Scenario: Chat row
- **WHEN** `group` is set under `[logs]` and Jane Doe (display name `JD`) says `hello` in `Builders` at 2026-12-01 12:00:00 UTC
- **THEN** the file gains the line `2026-12-01T12:00:00Z`, `chat`, `Builders`, the group's UUID, `Jane Doe`, `JD`, empty, empty, `hello`, separated by tabs

#### Scenario: Notice row
- **WHEN** Jane Doe sends `Builders` a notice with the subject `Meeting`, the two-line text `Tonight` and `at eight`, and the notecard `Agenda`
- **THEN** the row has the kind `notice`, the subject `Meeting`, the attachment `Agenda` and the text `Tonight\nat eight`, and is a single line

#### Scenario: Only a group file
- **WHEN** `group` is set under `[logs]`, `im` is not, and both a line of group chat and an instant message arrive
- **THEN** the group file gains one row and no file receives the instant message

#### Scenario: Restart keeps earlier rows
- **WHEN** the bot is restarted with the same group file
- **THEN** the rows written before are still there, new rows follow them, and no second header row is written

### Requirement: Group messages never disturb the bot
Reading and recording group messages SHALL NOT change what the bot does to stay logged in, in its place and seated; a failure to look up a name or to write a row SHALL be reported on standard error and SHALL NOT end the session or the bot. The bot SHALL NOT answer a group message by itself: no autoreply (see `im-autoreply`) and no other message SHALL be sent for one, and it speaks in a group only on an operator's command. Group messages SHALL NOT be sent to the owners of the XMPP gateway. Group messages still waiting for names when the bot stops SHALL be recorded before it exits.

#### Scenario: Line that looks like a command
- **WHEN** someone says `say hello` in a group's chat
- **THEN** the line is recorded and the avatar says nothing

#### Scenario: Autoreply is on
- **WHEN** autoreply is on and Jane Doe speaks in `Builders`
- **THEN** no autoreply is sent to Jane Doe or to the group, and her pause is as it was

#### Scenario: XMPP gateway connected
- **WHEN** the XMPP gateway is connected and a line of group chat and a notice arrive
- **THEN** the owners receive neither

### Requirement: Group chat appears in the operator log
The bot SHALL write a line to its operator log for every `group chat` command: the group by name and UUID, and what became of it: refused and why, or sent. It SHALL write a line when it enters a group's session and when the grid refuses that, with the grid's reason.

#### Scenario: Refused for the length
- **WHEN** `rslbot ctl group chat Builders` is run with a message of 2000 bytes
- **THEN** the log has a line naming `Builders` and the reason
