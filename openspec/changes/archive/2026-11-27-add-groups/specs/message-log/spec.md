# Spec Delta

## MODIFIED Requirements

### Requirement: Read instant messages
While connected, the bot SHALL record every instant message sent to the avatar by another avatar (kind `im`) and every instant message sent to it by an object (kind `object`). Other instant-message traffic — typing notifications, group and conference sessions, offers, invitations, notices and automatic responses — SHALL NOT be recorded as an instant message and SHALL NOT be written to the IM file. Of these, the chat and the notices of the avatar's groups are recorded as group messages, as `group-chat` says, and invitations to groups are requests (see `permission-requests`).

#### Scenario: IM from an avatar
- **WHEN** another avatar sends the bot's avatar the instant message `are you there?`
- **THEN** one instant message of kind `im` with that avatar's name and the text `are you there?` is recorded

#### Scenario: IM from an object
- **WHEN** a script sends the bot's avatar an instant message
- **THEN** one instant message of kind `object` is recorded with the object's name and the object's UUID

#### Scenario: Inventory offer
- **WHEN** another avatar offers the bot's avatar an inventory item
- **THEN** nothing is recorded

#### Scenario: Group chat
- **WHEN** `im` is set under `[logs]` and Jane Doe says `hello` in the chat of a group the avatar is in
- **THEN** no instant message is recorded and the IM file gains no row; the line is a group message
