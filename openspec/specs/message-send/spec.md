# message-send Specification

## Purpose

Defines the messages the avatar sends on the operator's command: local chat parsed the way a viewer's chat bar parses it, and instant messages to an avatar given by name.

## Requirements

### Requirement: Say command
The `say` command SHALL take a message and send it as local chat from the avatar in its current region. Several command-line arguments after `say` SHALL be joined with single spaces into one message. The bot SHALL answer with success once the chat has been handed to the simulator connection. It SHALL answer with an error and send nothing when the avatar is not connected to a region, when the text left after parsing is empty, or when that text is longer than 1023 bytes of UTF-8.

#### Scenario: Plain chat
- **WHEN** `rslbot ctl say hello there` is run while the avatar is in a region
- **THEN** the avatar says `hello there` on channel 0 with normal range

#### Scenario: Not connected
- **WHEN** `say` arrives while the bot is waiting to log in again
- **THEN** the bot answers with an error saying it is not connected and does not send the message later

#### Scenario: Message too long
- **WHEN** `say` arrives with a 2000-byte message
- **THEN** the bot answers with an error naming the limit and says nothing

### Requirement: Chat-bar parsing of say messages
The bot SHALL interpret the start of a `say` message the way a viewer's chat bar does:

| Message starts with | Meaning |
|---|---|
| `/` followed by decimal digits | The digits are the channel; the rest, with leading spaces removed, is the text |
| `//` | The channel is the one the previous `say` used; the rest is the text |
| `/shout` followed by a space | The rest is shouted on channel 0 |
| `/whisper` followed by a space | The rest is whispered on channel 0 |
| anything else, including `/me` and other words after a slash | The whole message is the text, said on channel 0 |

The channel remembered for `//` SHALL start as 0 when the bot starts and SHALL be updated by every `say` that names a channel. A channel number that does not fit a signed 32-bit integer SHALL be answered with an error.

#### Scenario: Channel prefix
- **WHEN** `rslbot ctl say /1 test message` is run
- **THEN** the avatar says `test message` on channel 1 and nothing appears on channel 0

#### Scenario: Channel prefix without a space
- **WHEN** the message is `/42open`
- **THEN** the avatar says `open` on channel 42

#### Scenario: Repeat the last channel
- **WHEN** `say /7 first` is followed by `say //second`
- **THEN** the avatar says `second` on channel 7

#### Scenario: Emote
- **WHEN** the message is `/me waves`
- **THEN** the avatar says `/me waves` on channel 0

#### Scenario: Shout
- **WHEN** the message is `/shout over here`
- **THEN** the avatar shouts `over here` on channel 0

#### Scenario: Only a channel
- **WHEN** the message is `/5`
- **THEN** the bot answers with an error because there is no text, and says nothing

### Requirement: IM command
The `im` command SHALL take an avatar and a message and send the message to that avatar as an instant message from the bot's avatar. The avatar SHALL be the first argument after `im`; several arguments after it SHALL be joined with single spaces into one message. The bot SHALL answer with success, naming the avatar the message went to, once the message has been handed to the simulator connection; success does not mean the recipient was online or has read it. It SHALL answer with an error and send nothing when the avatar is not connected to a region, when the message is empty or longer than 1023 bytes of UTF-8, or when the recipient cannot be resolved.

#### Scenario: Message to an online avatar
- **WHEN** `rslbot ctl im "Jane Doe" are you there?` is run while Jane Doe is online
- **THEN** Jane Doe receives the instant message `are you there?` from the bot's avatar and the command exits with status 0

#### Scenario: Reply lands in the same conversation
- **WHEN** Jane Doe answers the bot's instant message
- **THEN** the bot receives her answer as an instant message like any other

#### Scenario: Not connected
- **WHEN** `im` arrives while the bot has no simulator connection
- **THEN** the bot answers with an error and does not send the message later

### Requirement: Recipient resolution
The recipient of an `im` command SHALL be given as a UUID, which is used as is, or as a name, which the bot SHALL resolve on the grid. A name SHALL be accepted as `First Last`, as `first.last`, or as a single word, which stands for last name `Resident`. Names SHALL be compared without regard to letter case, and only an avatar whose full name matches exactly SHALL be accepted; a partial match SHALL NOT be used. The bot SHALL answer with an error naming the recipient when the grid knows no avatar by that name, and with an error when the grid has not answered within 10 seconds. A name the bot has resolved SHALL NOT need another lookup while the bot keeps running.

#### Scenario: Name with a dot
- **WHEN** the recipient is `jane.doe`
- **THEN** the message goes to the avatar named `Jane Doe`

#### Scenario: Single-word name
- **WHEN** the recipient is `sitbot`
- **THEN** the message goes to the avatar named `sitbot Resident`

#### Scenario: Recipient by UUID
- **WHEN** the recipient is `11111111-2222-3333-4444-555555555555`
- **THEN** the message goes to that avatar without a name lookup

#### Scenario: Unknown name
- **WHEN** the recipient is `Nobody Here` and the grid has no such avatar
- **THEN** the bot answers with an error naming `Nobody Here` and sends no instant message

#### Scenario: Similar names
- **WHEN** the recipient is `Jane Doe` and the grid's search also returns `Jane Doester`
- **THEN** the message goes to `Jane Doe` only

#### Scenario: Grid does not answer
- **WHEN** the grid does not answer the name lookup within 10 seconds
- **THEN** the bot answers with an error and sends no instant message

### Requirement: Commands appear in the operator log
The bot SHALL write one line to its operator log for every chat message it sends on command, with the channel and the text, and one line for every command it refused, with the reason. An instant message sent on command appears in the operator log as a recorded message of kind `im_out`, as the `message-log` capability defines.

#### Scenario: Chat on a channel
- **WHEN** the avatar says `test message` on channel 1 on command
- **THEN** the operator log has a line with the channel and the text

#### Scenario: Refused command
- **WHEN** an `im` command fails because the name is unknown
- **THEN** the operator log has a line saying so

#### Scenario: Instant message sent
- **WHEN** the bot sends an instant message on command
- **THEN** the operator log has one line for it, of kind `im_out`

### Requirement: Shout and whisper commands
The `shout` and `whisper` commands SHALL take a message and send it as local chat on channel 0, shouted or whispered. Several command-line arguments SHALL be joined with single spaces into one message. The message SHALL be sent as it is, without chat-bar parsing, so that `shout <message>` does what `say /shout <message>` does. The answers, the limits on the message and the operator log line SHALL be those of the `say` command, and the channel remembered for `//` SHALL NOT change.

#### Scenario: Shout
- **WHEN** `rslbot ctl shout over here` is run
- **THEN** the avatar shouts `over here` on channel 0

#### Scenario: Whisper
- **WHEN** `rslbot ctl whisper psst` is run
- **THEN** the avatar whispers `psst` on channel 0

#### Scenario: Slash is not special
- **WHEN** `rslbot ctl shout /1 hello` is run
- **THEN** the avatar shouts `/1 hello` on channel 0

#### Scenario: Not connected
- **WHEN** `shout` arrives while the bot has no simulator connection
- **THEN** the bot answers with an error and does not send the message later
