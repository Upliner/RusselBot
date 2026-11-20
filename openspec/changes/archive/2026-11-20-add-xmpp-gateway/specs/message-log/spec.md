# Spec Delta

## MODIFIED Requirements

### Requirement: Logging never disturbs the bot
Reading and recording messages SHALL NOT change what the bot does to stay logged in, in the target region and seated. A failure to look up a name or to write to a log file SHALL be reported on standard error and SHALL NOT end the session or the bot; later messages SHALL still be attempted. The bot SHALL NOT send chat or instant messages in response to what it reads; it sends them only on an operator's command, given through the control socket or, as an owner's message, through the XMPP gateway (see `xmpp-gateway`). Messages still waiting for names when the bot stops SHALL be recorded before it exits.

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
