# Spec Delta

## ADDED Requirements

### Requirement: Record of a group fee
A fee the avatar pays to join a group on the bot's command (see `payments`, "Fee for joining a group") SHALL be recorded as one transaction, as "A payment of the bot is one record" says of a payment: of kind `paid` when the grid reported it as made, `failed` when the grid reported it as failed or refused the join, and `unconfirmed` when the grid reported no transaction in time, whether or not the avatar joined. Its `party_uuid` SHALL be the group's UUID; its `avatar_name`, `display_name`, `object_name` and `object_uuid` SHALL be empty; its `type` SHALL be the grid's number from the report, or 1004, the number of a fee for joining a group, when there is no report; its `counted` SHALL be as for any payment; and its `text` SHALL be the grid's description, or, when the grid gave none, `Fee to join ` and the group's name, with the grid's reason after `: ` for a join it refused. A fee the grid takes for a join the bot did not send, such as one made from a viewer on the same account, SHALL be recorded as any other reported transaction, of kind `spent`.

#### Scenario: Fee that was reported
- **WHEN** the bot joins `Builders` for L$250 and the grid reports that the avatar paid L$250 to join a group, leaving L$750
- **THEN** exactly one transaction is recorded, of kind `paid`, with amount 250, balance 750, type 1004, the UUID of `Builders` as the party, empty names and the grid's description

#### Scenario: Fee that was not reported
- **WHEN** the bot joins `Builders` for L$250 on a grid that reports nothing about money
- **THEN** one transaction of kind `unconfirmed` is recorded with amount 250, counted 250, an empty balance, type 1004 and the text `Fee to join Builders`

#### Scenario: Join refused
- **WHEN** the grid refuses a join with a fee of L$250
- **THEN** one transaction of kind `failed` is recorded with counted 0

#### Scenario: Free join
- **WHEN** the bot joins a group without a fee
- **THEN** no transaction is recorded
