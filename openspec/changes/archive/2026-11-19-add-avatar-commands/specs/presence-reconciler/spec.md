# Spec Delta

## MODIFIED Requirements

### Requirement: Converge on the desired state
The bot SHALL continuously work towards a desired state that depends on its placement mode. In holding mode, the mode it starts in, the desired state is "connected, in the target region, seated on the target object". In manual mode, which the operator enters and leaves with commands, the desired state is "connected". Whenever no action is in progress and no backoff delay is pending, it SHALL choose its next action from the currently observed state and the mode alone, by the first rule that applies:

1. not connected: log in;
2. in manual mode: do nothing;
3. connected but not in the target region: teleport to the target region;
4. in the target region but not seated on the target (including seated state unknown): sit on the target object;
5. seated on the target: do nothing.

At most one action SHALL be in progress at a time. The bot SHALL NOT assume the result of an earlier action still holds; a change in observed state SHALL lead to the matching rule regardless of what was done before. A login made in manual mode SHALL ask for the avatar's last location rather than the target region. Entering manual mode SHALL abandon an automatic teleport or sit that is in progress, without counting it as a failure. Leaving manual mode SHALL clear pending teleport and sit backoff delays, so that the return starts at once.

#### Scenario: Normal start with login landing in the target region
- **WHEN** the bot starts and the grid places the avatar in the target region
- **THEN** the bot sits on the target object without teleporting

#### Scenario: Start in another region
- **WHEN** the bot starts and the grid places the avatar in a different region
- **THEN** the bot teleports to the target region and then sits on the target object

#### Scenario: Kicked while seated
- **WHEN** the avatar is seated on the target and gets kicked
- **THEN** the bot logs in again, gets to the target region, and sits on the target object again without operator action

#### Scenario: Region restart
- **WHEN** the target region is restarted while the avatar is seated
- **THEN** once the region is back online the avatar ends up seated on the target object again without operator action

#### Scenario: Moved to another region
- **WHEN** the avatar is teleported away to a different region while it stays connected
- **THEN** the bot teleports back to the target region and sits again

#### Scenario: Left alone in manual mode
- **WHEN** the bot is in manual mode and the avatar stands in a region that is not the target region
- **THEN** the bot neither teleports nor sits

#### Scenario: Kicked in manual mode
- **WHEN** the bot is in manual mode and the avatar gets kicked
- **THEN** the bot logs in again asking for the last location, and then neither teleports nor sits

#### Scenario: Back to holding mode
- **WHEN** the bot leaves manual mode while the avatar stands in another region
- **THEN** it teleports to the target region and sits on the target object, starting without a backoff delay
