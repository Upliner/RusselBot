# Spec Delta

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Logging never disturbs the bot
Reading and recording messages SHALL NOT change what the bot does to stay logged in, in the target region and seated. A failure to look up a name or to write to a log file SHALL be reported on standard error and SHALL NOT end the session or the bot; later messages SHALL still be attempted. The bot SHALL NOT send chat or instant messages in response to what it reads; it sends them only on an operator's command. Messages still waiting for names when the bot stops SHALL be recorded before it exits.

#### Scenario: Log file becomes unwritable
- **WHEN** writing a row fails, for example because the disk is full
- **THEN** standard error reports the failure, the avatar stays logged in and seated, and the message still appears on standard error

#### Scenario: Stopping with messages pending
- **WHEN** the bot is stopped a moment after a message arrived and before its names were resolved
- **THEN** the message is recorded, with the unresolved names empty, before the bot exits

#### Scenario: Message that looks like a command
- **WHEN** another avatar sends the bot's avatar the instant message `say hello`
- **THEN** the message is recorded and the avatar says nothing
