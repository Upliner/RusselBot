# Spec Delta

## MODIFIED Requirements

### Requirement: Know the current region
While connected, the bot SHALL know the name of the region the avatar is currently in, as reported by the simulator. When a region is configured, the avatar SHALL be considered to be in the target region when that name equals the configured `region`, compared without regard to letter case or surrounding whitespace. When no region is configured there is no target region: the avatar is in place in whatever region it is in, and the bot SHALL NOT request a teleport of its own accord.

#### Scenario: Already in the target region
- **WHEN** the bot is connected to a region whose name differs from the configured `region` only in letter case
- **THEN** the avatar is considered to be in the target region and no teleport is requested

#### Scenario: In another region
- **WHEN** the bot is connected to a region with a different name
- **THEN** the avatar is considered not to be in the target region

#### Scenario: No region configured
- **WHEN** the config has no `region` and the bot is connected to any region
- **THEN** no teleport is requested, and the log names the region without calling it the target or not the target
