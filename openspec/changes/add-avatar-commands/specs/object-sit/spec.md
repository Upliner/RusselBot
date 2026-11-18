# Spec Delta

## MODIFIED Requirements

### Requirement: Sitting is not disturbed while seated
While the avatar is seated on the target, the bot SHALL NOT of its own accord send anything that makes it stand up, move, or sit again. It does so only on an operator's command.

#### Scenario: Long seated session
- **WHEN** the avatar has been seated on the target for 30 minutes with no outside interference
- **THEN** it has stayed seated the whole time and no further sit requests were sent

#### Scenario: Operator stands the avatar up
- **WHEN** the operator sends a `stand` command while the avatar is seated on the target
- **THEN** the avatar stands up
