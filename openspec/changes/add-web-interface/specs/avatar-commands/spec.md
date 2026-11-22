# Spec Delta

## MODIFIED Requirements

### Requirement: Touch command
`touch <uuid>` SHALL make the avatar click the object with that UUID once, as a press followed at once by a release. The UUID of an inventory item that is attached to the bot's avatar (see `avatar-outfit`, "Knowing what is attached") SHALL stand for the root prim of that attachment when the bot has learned of no object with that UUID in the current region; the touch SHALL then be held to every rule that holds a touch of that attachment. It SHALL be answered with success once both have been handed to the simulator connection. It SHALL be answered with an error and send nothing when the argument is not a UUID, or when the bot has not learned of an object with that UUID in the current region and the UUID is not that of an item attached to the avatar. When the bot has not yet scanned the region (see "Object scan"), a `touch` with a UUID that is neither that of an object it knows nor that of such an item SHALL start the scan and wait up to 10 seconds for the object to be learned before answering.

#### Scenario: Touching a scripted object
- **WHEN** `rslbot ctl touch <uuid>` names a nearby object whose script reacts to touches
- **THEN** the script's touch events fire once with the bot's avatar as the toucher

#### Scenario: First touch without a configured scan
- **WHEN** `scan_distance` is not configured and `rslbot ctl touch <uuid>` is run for the first time, naming an object 10 metres away
- **THEN** the bot scans, the object is touched, and the command exits with status 0 within 11 seconds

#### Scenario: Object out of range or unknown
- **WHEN** `rslbot ctl touch <uuid>` names an object outside the scan distance or one that does not exist
- **THEN** the command exits with status 1 saying the object is not known and naming the scan distance

#### Scenario: Touching does not move the avatar
- **WHEN** `rslbot ctl touch <uuid>` is run while the avatar is seated on the configured object
- **THEN** the avatar stays seated and the bot's placement mode does not change

#### Scenario: Touching an attachment by its inventory item
- **WHEN** the avatar wears the object `Radio`, whose script reacts to touches, and `rslbot ctl touch <uuid>` is run with the UUID `rslbot ctl inventory -a` printed for `Radio`
- **THEN** the radio's touch events fire once with the bot's avatar as the toucher, and the command exits with status 0 at once

#### Scenario: Inventory item that is not attached
- **WHEN** `rslbot ctl touch <uuid>` is run with the UUID of an inventory item that is not attached to the avatar
- **THEN** the command exits with status 1 saying the object is not known
