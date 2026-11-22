# Spec Delta

## MODIFIED Requirements

### Requirement: Listing format
Each line SHALL consist of seven fields separated by tab characters: the prim's UUID; its distance in metres with one decimal; its flags (see "Flags in the listing"); its owner's UUID, which for an object owned by a group is the group's; its owner's name, `First Last` for an avatar and `(group)` for a group; its name; its description, which may be empty. A field from the owner's UUID on that the bot does not know SHALL be `?`.

With the switch `--position` each line SHALL consist of eight fields: the same seven, and as the third field, between the distance and the flags, the position the distance is measured to (see "Where an object is"), written as `<x>,<y>,<z>` in the region and rounded to whole metres.

#### Scenario: A line of the listing
- **WHEN** Jane Doe's object `Bench`, described as `Sit here`, is 3.24 metres from the avatar, holds a script that handles touches, and `rslbot ctl scan` is run
- **THEN** its line is its UUID, `3.2`, `scripted,touch`, Jane Doe's UUID, `Jane Doe`, `Bench` and `Sit here`, separated by tabs

#### Scenario: Object owned by a group
- **WHEN** an object deeded to a group is within the distance
- **THEN** its line has the group's UUID as the fourth field and `(group)` as the fifth

#### Scenario: No description
- **WHEN** an object without a description is listed
- **THEN** its line ends with a tab after the name

#### Scenario: Owner the name service does not know
- **WHEN** the name service does not name an object's owner, or the simulator offers no name service
- **THEN** the object's line has the owner's UUID as the fourth field and `?` as the fifth

#### Scenario: A line with its position
- **WHEN** the avatar stands at 100, 100, 25, the root prim of Jane Doe's object `Bench`, described as `Sit here` and holding a script that handles touches, is at 103.24, 100, 25, and `rslbot ctl scan --position` is run
- **THEN** its line is its UUID, `3.2`, `103,100,25`, `scripted,touch`, Jane Doe's UUID, `Jane Doe`, `Bench` and `Sit here`, separated by tabs

#### Scenario: Position of a child prim
- **WHEN** a chair's root prim is at 50, 60, 22, its child prim `Cushion` is listed with `-c`, and `--position` is given
- **THEN** the cushion's line has `50,60,22` as its third field

## ADDED Requirements

### Requirement: Position switch
`scan` SHALL take the switch `--position` wherever it takes a filter (see "Combining and refusing arguments"), and SHALL NOT refuse it as an unknown option. The switch SHALL filter nothing and SHALL change nothing but the fields of a line (see "Listing format"): with it and without it the same prims SHALL be listed, in the same order. `scan avatars` SHALL refuse it as it refuses every argument that starts with `-`.

#### Scenario: Same objects
- **WHEN** `rslbot ctl scan 20 -n bench` and `rslbot ctl scan --position 20 -n bench` are run one after the other while nothing around the avatar changes
- **THEN** both list the same objects in the same order, the second with eight fields in every line

#### Scenario: Any place among the arguments
- **WHEN** `rslbot ctl scan -t --position whisper` is run
- **THEN** the prims within 10 metres that handle touches are listed, each with its position

#### Scenario: Not for avatars
- **WHEN** `rslbot ctl scan avatars --position` is run
- **THEN** the command exits with status 1 and lists nothing
