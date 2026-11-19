# Spec Delta

## ADDED Requirements

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
