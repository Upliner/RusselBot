# Spec Delta

## MODIFIED Requirements

### Requirement: Flags in the listing
The flags field SHALL be made of these words, in this order and separated by commas, or `-` when none applies: `scripted`, `touch`, `pay`, `physical`, `temporary` and `phantom`, each when the simulator's description of the listed prim itself marks it so (`touch` when it marks the prim as handling touches, `pay` when it marks it as taking money); `attachment` for a prim an avatar wears; `child` for a prim that is not the root of its linkset.

#### Scenario: Plain object
- **WHEN** an unscripted, non-physical object rezzed for good is listed
- **THEN** its flags field is `-`

#### Scenario: Flags are the prim's own
- **WHEN** a linkset's only script is in a child prim and the simulator marks only that prim as scripted
- **THEN** the root prim's line does not have `scripted`, and with `-c` the child prim's line has `scripted,child`

#### Scenario: Script added later
- **WHEN** a script is dropped into an object the bot has listed as `-` and `rslbot ctl scan` is run again
- **THEN** its flags field has `scripted`

#### Scenario: Object that takes money
- **WHEN** a tip jar whose script handles payments is listed
- **THEN** its flags field has `scripted,pay`, or `scripted,touch,pay` when it handles touches as well

### Requirement: Flag filters
`-s` (`--scripted`), `-t` (`--touch`), `--pay`, `-p` (`--physical`), `--temporary` and `--phantom` SHALL each list only prims whose flags field would have that word.

#### Scenario: Things to touch
- **WHEN** `rslbot ctl scan -t` is run and of the objects nearby only a button's root prim handles touches
- **THEN** only the button is listed

#### Scenario: Two flags
- **WHEN** `rslbot ctl scan -s -p` is run
- **THEN** only prims that are both scripted and physical are listed

#### Scenario: Things to pay
- **WHEN** `rslbot ctl scan --pay` is run and of the objects nearby only a tip jar's root prim takes money
- **THEN** only the tip jar is listed
