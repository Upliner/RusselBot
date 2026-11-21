# Spec Delta

## MODIFIED Requirements

### Requirement: Restrictions and their holders
A restriction is an RLV command with one of the params `n`, `add`, `y` and `rem` (see `rlv-commands`, "Command syntax"). Its text is its behaviour, in lower case, followed by `:` and its option, as it was written, when it has an option. Two restrictions SHALL be the same when their behaviours are the same and both have no option or options that are equal without regard to letter case. Restrictions with an option that the RLV API calls exceptions (such as `sendim:<uuid>`) are restrictions here and SHALL be kept in the same way.

Every restriction has a holder: the object that sent it, known by its UUID, or the operator for a restriction given with `rlv cmd`. For every holder the bot SHALL keep the restrictions that holder has set, in the order they were set. `n` and `add` SHALL set the restriction for the sender; `y` and `rem` SHALL lift it for the sender. Setting a restriction the sender holds already and lifting one it does not hold SHALL change nothing and SHALL count as carried out. A command of one holder SHALL NOT set, lift or clear anything of another holder: a restriction two holders have set stays kept until both have lifted it. The operator SHALL be a holder apart from every object, also from an object that sends the same commands.

The bot SHALL keep every restriction whose behaviour is in one of these two lists, whatever its option:

- With an effect, as the requirements below say: `sendchat`, `emote`, `chatshout`, `chatnormal`, `chatwhisper`, `redirchat`, `rediremote`, `sendchannel`, `sendchannel_sec`, `sendchannel_except`, `sendim`, `sendim_sec`, `sendimto`, `startim`, `startimto`, `recvim`, `recvim_sec`, `recvimfrom`, `unsit`, `sit`, `sittp`, `interact`, `tploc`, `tplocal`, `tplure`, `tplure_sec`, `tprequest`, `tprequest_sec`, `accepttp`, `accepttprequest`, `touchall`, `touchworld`, `touchthis`, `touchme`, `touchattach`, `touchattachself`, `touchattachother`, `touchhud`, `fartouch`, `touchfar`, `detach`, `addattach`, `remattach`, `addoutfit`, `remoutfit`, `acceptpermission`, `denypermission`, `showinv`, `shownames`, `shownames_sec`, `shownametags`, `shownearby`, `showminimap`, `camdrawmax`, `camdrawalphamax`, `permissive`.
- Without an effect: `fly`, `temprun`, `alwaysrun`, `camzoommax`, `camzoommin`, `setcam_fovmin`, `setcam_fovmax`, `camdistmax`, `setcam_avdistmax`, `camdistmin`, `setcam_avdistmin`, `camdrawmin`, `camdrawalphamin`, `camdrawcolor`, `camunlock`, `setcam_unlock`, `camavdist`, `camtextures`, `setcam_textures`, `recvchat`, `recvchat_sec`, `recvchatfrom`, `recvemote`, `recvemote_sec`, `recvemotefrom`, `sendgesture`, `tplm`, `standtp`, `viewnote`, `viewscript`, `viewtexture`, `edit`, `editobj`, `editworld`, `editattach`, `rez`, `share`, `share_sec`, `defaultwear`, `unsharedwear`, `unsharedunwear`, `sharedwear`, `sharedunwear`, `detachthis`, `detachallthis`, `attachthis`, `attachallthis`, `detachthis_except`, `detachallthis_except`, `attachthis_except`, `attachallthis_except`, `showworldmap`, `showloc`, `showhovertextall`, `showhovertext`, `showhovertexthud`, `showhovertextworld`, `setgroup`, `setdebug`, `setenv`, `allowidle`.

A restriction of the second list SHALL be kept, lifted, cleared and reported like any other and SHALL change nothing else about the bot: in particular the message log and the `status` command SHALL show what they show without it. A restriction whose behaviour is in neither list, `notify` among them, is not supported (see `rlv-commands`, "Commands that are not supported") and SHALL NOT be kept. The bot SHALL keep at most 1024 restrictions in all; one that would be the 1025th SHALL NOT be kept, and the command SHALL count as refused.

#### Scenario: Set and lifted
- **WHEN** an attachment says `@sendim=n` to its owner and later `@sendim=y`
- **THEN** the bot keeps `sendim` for that attachment after the first message and keeps nothing for it after the second

#### Scenario: Add and rem
- **WHEN** an object the avatar owns says `@sendim:<uuid>=add` and later `@sendim:<uuid>=rem` to its owner
- **THEN** the exception is kept after the first message and gone after the second

#### Scenario: Two holders
- **WHEN** a collar and a cuff, both attached, have each said `@sendchat=n` to their owner, and the collar says `@sendchat=y`
- **THEN** `sendchat` is still kept for the cuff, and `rslbot ctl say hello` is refused

#### Scenario: Set twice
- **WHEN** an attachment says `@unsit=n` to its owner twice and then `@unsit=y` once
- **THEN** the bot keeps no `unsit` for it

#### Scenario: Letter case
- **WHEN** an attachment says `@Detach:Chest=N` to its owner and later `@detach:chest=y`
- **THEN** the restriction is kept with the text `detach:Chest` after the first message and gone after the second

#### Scenario: Kept without an effect
- **WHEN** an attachment says `@showloc=n,recvchat=n` to its owner
- **THEN** both are kept and listed by `rlv status` as having no effect, `status` still prints the region, local chat is still recorded, and the operator log does not call either command unsupported

#### Scenario: Unknown behaviour
- **WHEN** an attachment says `@frobnicate=n` to its owner
- **THEN** nothing is kept and the operator log notes `frobnicate=n` as not supported

## ADDED Requirements

### Requirement: Avatars hidden from the scans
An avatar other than the bot's own SHALL be hidden while `shownames`, `shownames_sec` or `shownametags` is in force against it; the exceptions of `shownames` and of `shownametags` are avatars' UUIDs. In the listing of `scan avatars` (see `avatar-scan`) the UUID, the name and the display name of a hidden avatar SHALL be written as `?`, and its distance and position as they are without the restriction. The bot SHALL NOT ask the name service for a hidden avatar on account of a scan.

#### Scenario: Names hidden
- **WHEN** a collar holds `shownames`, Jane Doe stands 3.24 metres from the bot's avatar at 103.24, 100, 25, and `rslbot ctl scan avatars` is run
- **THEN** her line is `?`, `3.2`, `103,100,25`, `?` and `?`, separated by tabs, and the name service is not asked for her

#### Scenario: Name tags hidden
- **WHEN** a collar holds `shownametags` and `rslbot ctl scan avatars` is run in a region with two other avatars
- **THEN** it prints two lines, each with `?` as its first, fourth and fifth field

#### Scenario: Exception
- **WHEN** a collar holds `shownames` and `shownames:<jane>`, Jane Doe and John Roe are in the region, and `rslbot ctl scan avatars` is run
- **THEN** Jane Doe's line has her UUID, her name and her display name, and John Roe's has `?` in their place

#### Scenario: Exception of one restriction only
- **WHEN** a collar holds `shownames`, `shownametags` and `shownames:<jane>`
- **THEN** Jane Doe is hidden, by `shownametags`

#### Scenario: Secure restriction
- **WHEN** a collar holds `shownames_sec`, a relay holds `shownames:<jane>`, and `rslbot ctl scan avatars` is run
- **THEN** Jane Doe is hidden; once the collar itself holds `shownames:<jane>` she is shown

#### Scenario: Hidden avatars keep their order and their filter
- **WHEN** a collar holds `shownames`, avatars stand 8, 15 and 60 metres from the bot's avatar, and `rslbot ctl scan avatars chat` is run
- **THEN** it prints two lines with the distances `8.0` and `15.0`, in that order

#### Scenario: RLV off
- **WHEN** a collar holds `shownames`, `rslbot ctl rlv mode off` is run, and `rslbot ctl scan avatars` is run
- **THEN** every avatar is listed with its UUID and its names

### Requirement: Owners hidden from the object scan
In the listing of `scan objects` (see `area-scan`) the UUID and the name of an owner that is a hidden avatar SHALL be written as `?`; an object's own UUID, name and description, an owner that is a group, and the bot's own avatar as owner SHALL be written as they are without the restriction. While `shownames`, `shownames_sec` or `shownametags` is in force, a `scan objects` with `-o` SHALL be refused, whoever `-o` names.

#### Scenario: Owner hidden
- **WHEN** a collar holds `shownames`, Jane Doe's object `Bench` stands 3.24 metres from the avatar, and `rslbot ctl scan` is run
- **THEN** the bench's line is its UUID, `3.2`, its flags, `?`, `?`, `Bench` and its description

#### Scenario: Owner an exception names
- **WHEN** a collar holds `shownames` and `shownames:<jane>`, and `rslbot ctl scan` is run
- **THEN** Jane Doe's objects are listed with her UUID and her name, and John Roe's with `?` in their place

#### Scenario: The bot's own objects and a group's
- **WHEN** a collar holds `shownametags` and `rslbot ctl scan -a` is run near a sign deeded to a group
- **THEN** what the bot's avatar wears is listed with the avatar's own UUID and name as its owner, and the sign with the group's UUID and `(group)`

#### Scenario: Owner filter
- **WHEN** a collar holds `shownames` and `shownames:<jane>`, and `rslbot ctl scan -o "Jane Doe"` is run
- **THEN** the command exits with status 1 naming `shownames` and the collar, and the simulator is asked nothing

#### Scenario: Other filters
- **WHEN** a collar holds `shownames` and `rslbot ctl scan 20 -n bench -t` is run
- **THEN** it is answered as without the restriction, apart from the owners

### Requirement: The nearby list
While `shownearby` is in force, `scan avatars` SHALL be refused. `scan objects` SHALL NOT be refused or changed by `shownearby`.

#### Scenario: Scan avatars refused
- **WHEN** a collar holds `shownearby` and `rslbot ctl scan avatars chat` is run
- **THEN** the command exits with status 1 naming `shownearby`, the collar and `rlv mode override`, and lists nothing

#### Scenario: Objects still scanned
- **WHEN** a collar holds `shownearby` and `rslbot ctl scan` is run
- **THEN** the objects around the avatar are listed, with their owners

### Requirement: Positions under showminimap
While `showminimap` is in force, the position field of every line of `scan avatars` SHALL be written as `?`, for avatars that are hidden and for those that are not. The distances, the order of the lines and the distance filter SHALL be as without the restriction, and `scan objects` SHALL NOT be changed by it.

#### Scenario: No positions
- **WHEN** a collar holds `showminimap`, Jane Doe stands 3.24 metres from the bot's avatar, and `rslbot ctl scan avatars` is run
- **THEN** her line is her UUID, `3.2`, `?`, `Jane Doe` and her display name

#### Scenario: With hidden names
- **WHEN** a collar holds `showminimap` and `shownames` and `rslbot ctl scan avatars whisper` is run with one avatar 8 metres away
- **THEN** it prints one line: `?`, `8.0`, `?`, `?` and `?`

### Requirement: Vision limit
The vision limit SHALL be the smallest distance of the `camdrawmax` restrictions in force, in metres. There SHALL be no vision limit while no `camdrawmax` with a distance is in force, and none while a `camdrawalphamax` restriction is in force and the largest value so held is below 0.99, which leaves the avatar able to see through the fog. `camdrawmin` and the other camera restrictions SHALL NOT change it.

#### Scenario: Smallest of two
- **WHEN** a collar holds `camdrawmax:25` and a blindfold holds `camdrawmax:10`
- **THEN** the vision limit is 10 metres, and 25 metres once the blindfold has lifted its restriction

#### Scenario: Thin fog
- **WHEN** a blindfold holds `camdrawmax:10` and `camdrawalphamax:0.5`
- **THEN** there is no vision limit

#### Scenario: Thin and thick fog
- **WHEN** a blindfold holds `camdrawmax:10` and `camdrawalphamax:0.5`, and a hood holds `camdrawalphamax:1`
- **THEN** the vision limit is 10 metres

#### Scenario: No distance
- **WHEN** a collar holds `camdrawmax` without an option, or `camdrawmax:far`
- **THEN** the restriction is kept and there is no vision limit

### Requirement: Object scan within the vision limit
While there is a vision limit, a `scan objects` whose distance would be more than the limit SHALL be carried out as if its `<distance>` were the limit: it SHALL list nothing farther away and SHALL NOT raise the object scan's distance beyond the limit. The operator log's line for that scan SHALL give the limit and name `camdrawmax`. `scan avatars` SHALL NOT be changed by a vision limit.

#### Scenario: Scan in the fog
- **WHEN** a blindfold holds `camdrawmax:10`, objects stand 3 and 20 metres from the avatar, and `rslbot ctl scan` is run
- **THEN** the object at 3 metres is listed and the one at 20 metres is not, and the command exits with status 0

#### Scenario: Distance beyond the limit
- **WHEN** `scan_distance` is not configured, a blindfold holds `camdrawmax:10`, and `rslbot ctl scan 96` is run
- **THEN** nothing farther than 10 metres is listed, and the simulator is still asked to describe objects within 32 metres only

#### Scenario: Limit beyond the object scan
- **WHEN** `scan_distance` is not configured, a blindfold holds `camdrawmax:50`, and `rslbot ctl scan 96` is run
- **THEN** the bot asks the simulator to describe objects within 50 metres, and objects up to 50 metres away are listed

#### Scenario: Distance within the limit
- **WHEN** a blindfold holds `camdrawmax:10` and `rslbot ctl scan 5` is run
- **THEN** only objects within 5 metres are listed

#### Scenario: What the avatar wears
- **WHEN** a blindfold holds `camdrawmax:0.5` and `rslbot ctl scan -a` is run
- **THEN** the attachments of the bot's avatar are listed with the distance `0.0`

#### Scenario: Limit lifted
- **WHEN** the blindfold lifts `camdrawmax:10` and `rslbot ctl scan` is run again
- **THEN** objects up to 32 metres away are listed

#### Scenario: Avatars beyond the limit
- **WHEN** a blindfold holds `camdrawmax:10` and `rslbot ctl scan avatars shout` is run with an avatar 60 metres away
- **THEN** that avatar is listed

#### Scenario: In the log
- **WHEN** a blindfold holds `camdrawmax:10` and `rslbot ctl scan 96` has been answered
- **THEN** the log's line for the scan says the listing was cut to 10 metres by `camdrawmax`

### Requirement: Restrictions on scans change nothing else
The restrictions of the six requirements above SHALL change nothing but the two scans: the message log, `status`, `friends`, `profile`, `im` and every other command SHALL do what they do without them, also for an avatar that is hidden. A `scan` that is the exempt command of an override (see "Override exempts one command") SHALL be answered as with no restriction kept.

#### Scenario: Message log
- **WHEN** a collar holds `shownames` and Jane Doe says something in local chat
- **THEN** the message is recorded with her name and her UUID as without the restriction

#### Scenario: Profile and friends
- **WHEN** a collar holds `shownames`, `shownearby` and `showminimap`, and `rslbot ctl profile Jane Doe` and `rslbot ctl friends` are run
- **THEN** both are answered as without the restrictions

#### Scenario: Override
- **WHEN** a collar holds `shownearby` and `shownames`, and `rslbot ctl rlv mode override` and then `rslbot ctl scan avatars` are run
- **THEN** the avatars are listed with their UUIDs and names, and `rlv status` then prints `rlv: on`

#### Scenario: Only one
- **WHEN** `rslbot ctl scan avatars` is then run once more
- **THEN** it is refused naming `shownearby`
