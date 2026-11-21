# Spec Delta

## MODIFIED Requirements

### Requirement: Scan command
`scan [objects] [<distance>] [<filter>...]` SHALL be answered with success and a text of one line for each object within the distance of the avatar that the simulator has described to the bot, or with success and no text when there is none. Unless a switch says otherwise (see "Attachments and child prims"), the objects listed SHALL be the root prims of objects in the world: no avatar, no tree or grass, nothing an avatar wears, and no child prim of a linkset.

The word `objects` as the first argument SHALL change nothing: `scan objects` followed by any arguments SHALL be the same command as `scan` followed by those arguments. A `scan` whose first argument is `avatars` is the command of `avatar-scan`; it is not a `scan` command where this capability or another speaks of one, and no requirement of this capability applies to it.

#### Scenario: Objects around the avatar
- **WHEN** a bench stands 3 metres and a lamp 10 metres from the avatar and `rslbot ctl scan` is run
- **THEN** the output has one line for the bench and one for the lamp, the bench first, and the command exits with status 0

#### Scenario: Nothing nearby
- **WHEN** `rslbot ctl scan 5` is run and no object is within 5 metres of the avatar
- **THEN** the command exits with status 0 and prints nothing

#### Scenario: A linkset is one object
- **WHEN** a chair made of four linked prims stands beside the avatar and `rslbot ctl scan` is run
- **THEN** the output has one line for the chair, with the UUID of its root prim

#### Scenario: Avatars, plants and what avatars wear
- **WHEN** another avatar wearing a hat stands beside a tree, both within the distance, and `rslbot ctl scan` is run
- **THEN** the output has no line for the avatar, the hat or the tree

#### Scenario: The word objects
- **WHEN** `rslbot ctl scan objects 20 -n bench` and `rslbot ctl scan 20 -n bench` are run one after the other while nothing around the avatar changes
- **THEN** both print the same lines

#### Scenario: Objects without anything else
- **WHEN** `rslbot ctl scan objects` is run
- **THEN** it lists what `rslbot ctl scan` lists

### Requirement: Scan distance
`<distance>` SHALL be a number of metres from 1 to 1024, or one of the words `shout`, `chat` and `whisper`, written in lower case, which stand for 100, 20 and 10 metres. Without it the distance SHALL be that of the object scan in the region (see `avatar-commands`, "Object scan"). When `<distance>` is more than that, the object scan's distance SHALL be raised to it before the listing is made, and SHALL stay raised until the avatar leaves the region. A smaller `<distance>` SHALL only shorten the listing and SHALL NOT lower the object scan's distance. A vision limit of RLV SHALL shorten the distance of a `scan` as `rlv-restrictions` says ("Object scan within the vision limit").

#### Scenario: Default distance
- **WHEN** `scan_distance` is not configured, no `scan` has named a distance in the region, and `rslbot ctl scan` is run
- **THEN** objects up to 32 metres from the avatar are listed and an object 40 metres away is not

#### Scenario: Further than the object scan reaches
- **WHEN** `scan_distance` is not configured and `rslbot ctl scan 96` is run
- **THEN** the bot asks the simulator to describe objects within 96 metres, and an object 80 metres away is listed

#### Scenario: Raised distance stays
- **WHEN** `rslbot ctl scan 96` has been answered and `rslbot ctl touch <uuid>` then names an object 80 metres away
- **THEN** the touch is answered without a scan wait, and a later `rslbot ctl scan` without a distance lists objects up to 96 metres away

#### Scenario: Shorter than the object scan reaches
- **WHEN** `scan_distance = 64` is configured and `rslbot ctl scan 10` is run
- **THEN** only objects within 10 metres are listed, and the bot still asks the simulator for objects within 64 metres

#### Scenario: New region
- **WHEN** `rslbot ctl scan 96` has been answered and the avatar teleports to another region
- **THEN** the object scan's distance in the new region is `scan_distance`, or 32 metres without it

#### Scenario: Distance by a word
- **WHEN** objects stand 8, 15 and 25 metres from the avatar and `rslbot ctl scan chat` is run
- **THEN** the objects at 8 and 15 metres are listed and the one at 25 metres is not, as with `rslbot ctl scan 20`

#### Scenario: A word raises the distance as its number does
- **WHEN** `scan_distance` is not configured and `rslbot ctl scan shout` is run
- **THEN** the bot asks the simulator to describe objects within 100 metres, and an object 80 metres away is listed

### Requirement: Scan reads only
`scan` SHALL NOT move the avatar, make it sit or stand, or change the bot's placement mode or the held placement. It SHALL be held to the RLV restrictions that `rlv-restrictions` names for the object scan ("Owners hidden from the object scan" and "Object scan within the vision limit") and to no other, and it SHALL be the exempt command of an RLV override as any other command is (see `rlv-restrictions`, "Override exempts one command").

#### Scenario: Scan while seated and holding
- **WHEN** `rslbot ctl scan` is run in holding mode while the avatar is seated on the held object
- **THEN** the avatar stays seated, `status` still prints `mode: holding`, and the bot sends no sit or teleport request

#### Scenario: Scan during an override
- **WHEN** a collar holds `shownames` and `camdrawmax:10`, `rslbot ctl rlv mode override` has been carried out, and `rslbot ctl scan` is run
- **THEN** the scan is answered as with no restriction kept, with the owners and out to the object scan's distance, and `rlv status` then prints `rlv: on`

#### Scenario: Override used up without a restriction on scans
- **WHEN** a collar holds `sendchat` only, `rslbot ctl rlv mode override` has been carried out, and `rslbot ctl scan` is run
- **THEN** the scan is answered and `rlv status` then prints `rlv: on`

#### Scenario: Restrictions that are not about scans
- **WHEN** a collar holds `interact`, `touchall`, `fartouch`, `showinv` and `showloc`, RLV is on, and `rslbot ctl scan` is run
- **THEN** the scan is answered as without them

### Requirement: Combining and refusing arguments
The distance and the filters MAY be given in any order, and every filter given SHALL have to be met for a prim to be listed. `scan` SHALL be answered with an error, and SHALL send and change nothing, when an argument starts with `-` and is not one of the filters, when `-n`, `-d` or `-o` is the last argument, when `<who>` is not something `im` would take, when more than one argument is left over as the distance, or when the distance is neither a number from 1 to 1024 nor one of the words `shout`, `chat` and `whisper`. The word `objects` anywhere but as the first argument SHALL be read as a distance, and so be refused.

#### Scenario: Several filters
- **WHEN** `rslbot ctl scan -s 20 -o "Jane Doe" -n door` is run
- **THEN** only scripted prims of Jane Doe within 20 metres with `door` in their name are listed

#### Scenario: Unknown option
- **WHEN** `rslbot ctl scan -x` is run
- **THEN** the command exits with status 1 naming `-x`, and the simulator is asked nothing

#### Scenario: Filter without its text
- **WHEN** `rslbot ctl scan -n` is run
- **THEN** the command exits with status 1 saying `-n` takes a text

#### Scenario: Distance out of bounds
- **WHEN** `rslbot ctl scan 0`, `rslbot ctl scan 2000` or `rslbot ctl scan far` is run
- **THEN** the command exits with status 1 saying the distance is a number of metres from 1 to 1024 or `shout`, `chat` or `whisper`, and the object scan's distance does not change

#### Scenario: Two distances
- **WHEN** `rslbot ctl scan 10 20` is run
- **THEN** the command exits with status 1

#### Scenario: A word and a number
- **WHEN** `rslbot ctl scan chat 20` is run
- **THEN** the command exits with status 1, as with two numbers

#### Scenario: Word in capitals
- **WHEN** `rslbot ctl scan Chat` is run
- **THEN** the command exits with status 1 as for any other word that is not a distance

#### Scenario: Objects after the distance
- **WHEN** `rslbot ctl scan 20 objects` is run
- **THEN** the command exits with status 1, and the simulator is asked nothing

#### Scenario: Filters with the word objects
- **WHEN** `rslbot ctl scan objects -t whisper` is run
- **THEN** only prims within 10 metres that handle touches are listed
