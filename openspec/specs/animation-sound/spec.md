# animation-sound Specification

## Purpose

Lets the operator make the avatar play animations and sounds from its inventory through the control socket, so that the avatar can dance, gesture or make a noise without a viewer.

## Requirements

### Requirement: Animation and sound commands need the avatar in a region
The commands `anim` and `sound` SHALL be answered with an error, and SHALL send nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout. A command SHALL be carried out at once or not at all; it SHALL NOT be kept for later. Neither command SHALL change the bot's placement mode or move the avatar.

#### Scenario: Command between logins
- **WHEN** `rslbot ctl sound Sounds/Bell` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Dancing on the seat
- **WHEN** `rslbot ctl anim play Animations/Dance` is carried out while the avatar is seated on the held object in holding mode
- **THEN** the avatar stays seated and the bot stays in holding mode

### Requirement: Play an animation
`anim play <item>` SHALL start the animation `<item>` names (see `inventory-list`, "Naming an inventory item or folder") on the avatar, in addition to whatever else it is playing. It SHALL be answered with success, naming the item, once the request has been handed to the simulator connection. It SHALL be answered with an error and send the simulator nothing when the item cannot be found, when the item is not an animation, in which case the error names its type, and when the inventory does not give the bot the animation itself for that item. An animation SHALL play until it is stopped, ends by itself or the session is lost; the bot SHALL NOT start it again by itself after a new login.

#### Scenario: Starting a dance
- **WHEN** `rslbot ctl anim play Animations/Dance` is run and `Dance` is an animation
- **THEN** other viewers see the avatar play it, and the command exits with status 0

#### Scenario: By UUID
- **WHEN** `rslbot ctl anim play <uuid>` is run with the UUID `inventory` printed for `Dance`
- **THEN** the avatar plays `Dance`

#### Scenario: Not an animation
- **WHEN** `rslbot ctl anim play Objects/Radio` is run
- **THEN** the command exits with status 1 saying the item is an object, and nothing is sent to the simulator

#### Scenario: After a new login
- **WHEN** the avatar is playing `Dance`, the session is lost and the bot logs in again
- **THEN** the bot does not start `Dance` by itself

### Requirement: Stop an animation
`anim stop <item>` SHALL stop the animation `<item>` names on the avatar. It SHALL be answered with success, naming the item, once the request has been handed to the simulator connection, whether or not the avatar was playing it. It SHALL be refused in the cases in which `anim play` is.

#### Scenario: Stopping the dance
- **WHEN** the avatar plays `Dance` and `rslbot ctl anim stop Animations/Dance` is run
- **THEN** other viewers see the avatar stop playing it, and the command exits with status 0

#### Scenario: Not playing
- **WHEN** `rslbot ctl anim stop Animations/Dance` is run while the avatar is not playing `Dance`
- **THEN** the command exits with status 0

### Requirement: Anim subcommands
`anim` SHALL take exactly two arguments, `play` or `stop` and the item. Any other first argument, and any other number of arguments, SHALL be answered with an error naming the two forms, and SHALL send nothing.

#### Scenario: Unknown subcommand
- **WHEN** `rslbot ctl anim pause Animations/Dance` is run
- **THEN** the command exits with status 1 and nothing is sent

#### Scenario: Missing item
- **WHEN** `rslbot ctl anim play` is run
- **THEN** the command exits with status 1 and nothing is sent

### Requirement: Play a sound
`sound <item>` SHALL play the sound `<item>` names once, at the avatar's position and at full volume, so that avatars nearby hear it. It SHALL be answered with success, naming the item, once the request has been handed to the simulator connection. It SHALL be answered with an error and send the simulator nothing when the item cannot be found, when the item is not a sound, in which case the error names its type, and when the inventory does not give the bot the sound itself for that item.

#### Scenario: Ringing a bell
- **WHEN** `rslbot ctl sound Sounds/Bell` is run and `Bell` is a sound
- **THEN** an avatar standing next to the bot's avatar hears it once, and the command exits with status 0

#### Scenario: Not a sound
- **WHEN** `rslbot ctl sound Animations/Dance` is run
- **THEN** the command exits with status 1 saying the item is an animation, and nothing is sent to the simulator

### Requirement: Animation and sound commands appear in the operator log
The bot SHALL write a line to its operator log for every `anim` and `sound` command it carries out, naming the item, and for every one it refuses, with the reason.

#### Scenario: Reading the log
- **WHEN** the operator has run `anim play Animations/Dance`, `sound Sounds/Bell` and `anim stop Animations/Dance`
- **THEN** the log has one line for each, in that order
