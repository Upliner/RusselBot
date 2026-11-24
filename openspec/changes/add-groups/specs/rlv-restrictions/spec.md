# Spec Delta

## MODIFIED Requirements

### Requirement: Restrictions and their holders
A restriction is an RLV command with one of the params `n`, `add`, `y` and `rem` (see `rlv-commands`, "Command syntax"). Its text is its behaviour, in lower case, followed by `:` and its option, as it was written, when it has an option. Two restrictions SHALL be the same when their behaviours are the same and both have no option or options that are equal without regard to letter case. Restrictions with an option that the RLV API calls exceptions (such as `sendim:<uuid>`) are restrictions here and SHALL be kept in the same way.

Every restriction has a holder: the object that sent it, known by its UUID, or the operator for a restriction given with `rlv cmd`. For every holder the bot SHALL keep the restrictions that holder has set, in the order they were set. `n` and `add` SHALL set the restriction for the sender; `y` and `rem` SHALL lift it for the sender. Setting a restriction the sender holds already and lifting one it does not hold SHALL change nothing and SHALL count as carried out. A command of one holder SHALL NOT set, lift or clear anything of another holder: a restriction two holders have set stays kept until both have lifted it. The operator SHALL be a holder apart from every object, also from an object that sends the same commands.

The bot SHALL keep every restriction whose behaviour is in one of these two lists, whatever its option:

- With an effect, as the requirements below say: `sendchat`, `emote`, `chatshout`, `chatnormal`, `chatwhisper`, `redirchat`, `rediremote`, `sendchannel`, `sendchannel_sec`, `sendchannel_except`, `sendim`, `sendim_sec`, `sendimto`, `startim`, `startimto`, `recvim`, `recvim_sec`, `recvimfrom`, `setgroup`, `unsit`, `sit`, `sittp`, `interact`, `tploc`, `tplocal`, `tplure`, `tplure_sec`, `tprequest`, `tprequest_sec`, `accepttp`, `accepttprequest`, `touchall`, `touchworld`, `touchthis`, `touchme`, `touchattach`, `touchattachself`, `touchattachother`, `touchhud`, `fartouch`, `touchfar`, `detach`, `addattach`, `remattach`, `addoutfit`, `remoutfit`, `acceptpermission`, `denypermission`, `showinv`, `shownames`, `shownames_sec`, `shownametags`, `shownearby`, `showminimap`, `camdrawmax`, `camdrawalphamax`, `permissive`.
- Without an effect: `fly`, `temprun`, `alwaysrun`, `camzoommax`, `camzoommin`, `setcam_fovmin`, `setcam_fovmax`, `camdistmax`, `setcam_avdistmax`, `camdistmin`, `setcam_avdistmin`, `camdrawmin`, `camdrawalphamin`, `camdrawcolor`, `camunlock`, `setcam_unlock`, `camavdist`, `camtextures`, `setcam_textures`, `recvchat`, `recvchat_sec`, `recvchatfrom`, `recvemote`, `recvemote_sec`, `recvemotefrom`, `sendgesture`, `tplm`, `standtp`, `viewnote`, `viewscript`, `viewtexture`, `edit`, `editobj`, `editworld`, `editattach`, `rez`, `share`, `share_sec`, `defaultwear`, `unsharedwear`, `unsharedunwear`, `sharedwear`, `sharedunwear`, `detachthis`, `detachallthis`, `attachthis`, `attachallthis`, `detachthis_except`, `detachallthis_except`, `attachthis_except`, `attachallthis_except`, `showworldmap`, `showloc`, `showhovertextall`, `showhovertext`, `showhovertexthud`, `showhovertextworld`, `setdebug`, `setenv`, `allowidle`.

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

### Requirement: Group chat the avatar sends
The `group chat` command (see `group-chat`) SHALL be refused:

- while `sendim` is in force and none of its exceptions names the group or is `sendim:allgroups`;
- while a holder holds `sendimto` with an option that names the group, or `sendimto:allgroups`.

An option SHALL name a group when it is the group's name, with the commas and semicolons of the name left out and compared without regard to letter case, or the group's UUID. `startim` and `startimto` SHALL NOT be applied to group chat. Nothing SHALL be said in the group in place of a refused line, and the bot SHALL NOT enter the group's session for it.

#### Scenario: No instant messages
- **WHEN** a collar holds `sendim` and `rslbot ctl group chat Builders hello` is run
- **THEN** nothing is said in `Builders` and the command exits with status 1 naming `sendim`

#### Scenario: One group excepted
- **WHEN** a collar holds `sendim` and `sendim:Builders`, and `rslbot ctl group chat Builders hello` and `rslbot ctl group chat Dancers hello` are run
- **THEN** the first is said and the second is refused

#### Scenario: All groups excepted
- **WHEN** a collar holds `sendim` and `sendim:allgroups`, and `rslbot ctl group chat Dancers hello` and `rslbot ctl im "Jane Doe" hello` are run
- **THEN** the line is said in `Dancers` and the instant message is refused

#### Scenario: One group barred
- **WHEN** a collar holds `sendimto:Builders` and nothing else
- **THEN** `group chat Builders hello` is refused naming `sendimto:Builders`, and `group chat Dancers hello` is said

#### Scenario: Override
- **WHEN** a collar holds `sendim`, `rslbot ctl rlv mode override` has been carried out and `rslbot ctl group chat Builders hello` is run
- **THEN** the line is said, and RLV is on again

### Requirement: Group chat the avatar receives
A line of group chat (see `group-chat`, "Read group chat") SHALL be blocked when it arrives:

- while `recvim` is in force, none of its exceptions names the group or is `recvim:allgroups`, and either none of its exceptions names the speaker or a holder holds `recvimfrom:<uuid>` with the speaker's UUID;
- while a holder holds `recvimfrom` with an option that names the group, or `recvimfrom:allgroups`.

An option SHALL name a group as "Group chat the avatar sends" says. A blocked line SHALL be recorded with the group and the speaker's names as usual and with the text `(blocked by RLV: <restriction>)` in place of the text it had, where `<restriction>` is the text of the restriction that blocked it; the text that was said SHALL NOT appear in the record, on standard error, in a log file or on the event stream of the web interface. The bot SHALL send nothing to the speaker or to the group for a blocked line. Group notices, and the lines the bot itself says, SHALL NOT be blocked.

#### Scenario: Group chat blocked
- **WHEN** a collar holds `recvim` and Jane Doe says `are you there?` in `Builders`
- **THEN** a group message of kind `chat` from Jane Doe in `Builders` is recorded with the text `(blocked by RLV: recvim)`, and Jane Doe receives nothing from the avatar

#### Scenario: Group excepted
- **WHEN** a collar holds `recvim` and `recvim:allgroups`, and Jane Doe says `hi` in `Builders` and sends the avatar the instant message `hi`
- **THEN** the line in `Builders` is recorded with the text `hi`, and the instant message is blocked

#### Scenario: Speaker excepted
- **WHEN** a collar holds `recvim` and `recvim:<jane>`, and Jane Doe and John Roe each say `hi` in `Builders`
- **THEN** Jane Doe's line is recorded with the text `hi` and John Roe's is blocked

#### Scenario: One group barred
- **WHEN** a collar holds `recvimfrom:Builders` and nothing else, and Jane Doe says `hi` in `Builders` and in `Dancers`
- **THEN** her line in `Builders` is blocked and her line in `Dancers` is recorded as said

#### Scenario: Notice
- **WHEN** a collar holds `recvim` and Jane Doe sends `Builders` a notice
- **THEN** the notice is recorded with its subject and text

### Requirement: Active group under setgroup
While `setgroup` is in force, `group activate` (see `groups`), with a group or with `none`, SHALL be refused. The restriction SHALL change nothing else: the bot still reads the avatar's groups and their chat, invitations are still accepted and joins still made, and an active group the grid changes by itself, as some grids do when the avatar joins a group, is not undone.

#### Scenario: Group tag locked
- **WHEN** a collar holds `setgroup` and `rslbot ctl group activate Dancers` is run while `Builders` is active
- **THEN** `Builders` stays active and the command exits with status 1 naming `setgroup`

#### Scenario: Listing is not refused
- **WHEN** a collar holds `setgroup` and `rslbot ctl group list` is run
- **THEN** the groups are listed
