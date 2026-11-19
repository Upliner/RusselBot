# Spec Delta

## MODIFIED Requirements

### Requirement: Knowing what is attached
The bot SHALL know which inventory items are attached to its avatar, and to which attachment point each is, from what the simulator sends about the objects on the avatar. Where the simulator mentions an object without describing it, as one a viewer is taken to have cached, and marks it as owned by the bot's account, the bot SHALL ask the simulator to describe that object, at once and whether or not an object scan has started (see `avatar-commands`, "Object scan"). It SHALL ask about such an object once, not each time the object is mentioned, and SHALL NOT ask about an object it already has a description of. Apart from those requests it SHALL NOT send the simulator anything for the sake of learning what is attached. What it knows SHALL follow the simulator: an attachment that appears or disappears without a command of the bot, for instance through a script or a viewer logged in to the same account earlier, SHALL be known or forgotten the same way. What it knows SHALL be forgotten when the session is lost and learned anew from the next simulator.

#### Scenario: Attached before the bot started
- **WHEN** the avatar was left with `Radio` on its left hand by a viewer and the bot then logs in
- **THEN** `rslbot ctl inventory -a` lists `Radio` with `Left Hand` and `attached`, without any attach command having been given

#### Scenario: Attachment mentioned as cached
- **WHEN** the bot logs in, the simulator describes two of the avatar's attachments and only mentions the third, `Suit`, as a cached object owned by the bot's account
- **THEN** the bot asks the simulator about that object, and once it is described `rslbot ctl inventory -a` lists `Suit` as attached and `rslbot ctl attach Suit` is refused because it is attached already

#### Scenario: Detached by a script
- **WHEN** an attachment's script detaches it from the avatar
- **THEN** the next `rslbot ctl inventory -a` does not list it

#### Scenario: No extra traffic
- **WHEN** the avatar wears three attachments, the simulator describes all three by itself and mentions no object of the bot's account as cached, and no command is given
- **THEN** the bot sends the simulator nothing it would not send for an avatar without attachments

#### Scenario: Cached objects of other owners
- **WHEN** the simulator mentions 200 objects as cached, none of them marked as owned by the bot's account, and no object scan has started
- **THEN** the bot asks about none of them

#### Scenario: Own object that is not an attachment
- **WHEN** the simulator mentions as cached a box that belongs to the bot's account and stands on the ground
- **THEN** the bot asks about it, and `rslbot ctl inventory -a` does not list anything for it

#### Scenario: Mentioned again
- **WHEN** the simulator mentions the same cached object of the bot's account a second time before or after describing it
- **THEN** the bot does not ask about it a second time

#### Scenario: After a teleport to another region
- **WHEN** the avatar teleports to another region with `Radio` attached
- **THEN** `rslbot ctl inventory -a` in the new region lists `Radio` once the new simulator has shown it on the avatar, whether that simulator described it by itself or after being asked
