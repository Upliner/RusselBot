# Spec Delta

## ADDED Requirements

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
