# Spec Delta

## MODIFIED Requirements

### Requirement: Restrictions and their holders
A restriction is an RLV command with one of the params `n`, `add`, `y` and `rem` (see `rlv-commands`, "Command syntax"). Its text is its behaviour, in lower case, followed by `:` and its option, as it was written, when it has an option. Two restrictions SHALL be the same when their behaviours are the same and both have no option or options that are equal without regard to letter case. Restrictions with an option that the RLV API calls exceptions (such as `sendim:<uuid>`) are restrictions here and SHALL be kept in the same way.

Every restriction has a holder: the object that sent it, known by its UUID, or the operator for a restriction given with `rlv cmd`. For every holder the bot SHALL keep the restrictions that holder has set, in the order they were set. `n` and `add` SHALL set the restriction for the sender; `y` and `rem` SHALL lift it for the sender. Setting a restriction the sender holds already and lifting one it does not hold SHALL change nothing and SHALL count as carried out. A command of one holder SHALL NOT set, lift or clear anything of another holder: a restriction two holders have set stays kept until both have lifted it. The operator SHALL be a holder apart from every object, also from an object that sends the same commands.

The bot SHALL keep every restriction whose behaviour is in one of these two lists, whatever its option:

- With an effect, as the requirements below say: `sendchat`, `emote`, `chatshout`, `chatnormal`, `chatwhisper`, `redirchat`, `rediremote`, `sendchannel`, `sendchannel_sec`, `sendchannel_except`, `sendim`, `sendim_sec`, `sendimto`, `startim`, `startimto`, `recvim`, `recvim_sec`, `recvimfrom`, `unsit`, `sit`, `sittp`, `interact`, `tploc`, `tplocal`, `tplure`, `tplure_sec`, `tprequest`, `tprequest_sec`, `accepttp`, `accepttprequest`, `touchall`, `touchworld`, `touchthis`, `touchme`, `touchattach`, `touchattachself`, `touchattachother`, `touchhud`, `fartouch`, `touchfar`, `detach`, `addattach`, `remattach`, `addoutfit`, `remoutfit`, `acceptpermission`, `denypermission`, `showinv`, `permissive`.
- Without an effect: `fly`, `temprun`, `alwaysrun`, `camzoommax`, `camzoommin`, `setcam_fovmin`, `setcam_fovmax`, `camdistmax`, `setcam_avdistmax`, `camdistmin`, `setcam_avdistmin`, `camdrawmin`, `camdrawmax`, `camdrawalphamin`, `camdrawalphamax`, `camdrawcolor`, `camunlock`, `setcam_unlock`, `camavdist`, `camtextures`, `setcam_textures`, `recvchat`, `recvchat_sec`, `recvchatfrom`, `recvemote`, `recvemote_sec`, `recvemotefrom`, `sendgesture`, `tplm`, `standtp`, `viewnote`, `viewscript`, `viewtexture`, `edit`, `editobj`, `editworld`, `editattach`, `rez`, `share`, `share_sec`, `defaultwear`, `unsharedwear`, `unsharedunwear`, `sharedwear`, `sharedunwear`, `detachthis`, `detachallthis`, `attachthis`, `attachallthis`, `detachthis_except`, `detachallthis_except`, `attachthis_except`, `attachallthis_except`, `showworldmap`, `showminimap`, `showloc`, `shownames`, `shownames_sec`, `shownametags`, `shownearby`, `showhovertextall`, `showhovertext`, `showhovertexthud`, `showhovertextworld`, `setgroup`, `setdebug`, `setenv`, `allowidle`.

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

### Requirement: Instant messages the avatar receives
An instant message of kind `im` that an avatar sends the avatar (see `message-log`, "Read instant messages") SHALL be blocked when it arrives while `recvim` is in force against the sender, with `recvim:<uuid>` as its exceptions, or while a holder holds `recvimfrom:<uuid>` with the sender's UUID. An option of these restrictions that names a group SHALL have no effect. Instant messages from objects and from the grid SHALL NOT be blocked.

For a blocked message the bot SHALL:

- record it with the sender's names as usual and with the text `(blocked by RLV: <restriction>)` in place of the text it had, where `<restriction>` is the text of the restriction that blocked it; the text that was sent SHALL NOT appear in the message log, on standard error, in a log file or at the XMPP owners;
- send the sender, as an autoresponse in the same way as an autoreply (see `im-autoreply`, "How an autoreply is sent"), the text `The Resident you messaged is currently prevented from reading your instant messages at the moment, please try again later.`, which is what RLV viewers send. This reply SHALL be sent for every blocked message, whether autoreply is on or off and whatever pause the sender is in, and SHALL NOT be held to `sendim` or `sendimto`. It SHALL NOT be sent for a message stored while the avatar was offline. It SHALL be recorded as an autoreply is, as kind `im_auto`;
- send no autoreply for that message and leave the sender's pause as it was.

#### Scenario: Instant messages blocked
- **WHEN** a collar holds `recvim` and Jane Doe sends the avatar `are you there?`
- **THEN** Jane Doe receives the RLV viewers' autoresponse, and the message log records an `im` from Jane Doe with the text `(blocked by RLV: recvim)` followed by an `im_auto` to her

#### Scenario: Exception
- **WHEN** a collar holds `recvim` and `recvim:<jane>`, and Jane Doe sends the avatar `hi`
- **THEN** the message is recorded with the text `hi` and no RLV reply is sent

#### Scenario: One sender blocked
- **WHEN** a collar holds `recvimfrom:<jane>`, and Jane Doe and John Roe each send the avatar `hi`
- **THEN** Jane Doe's message is blocked and answered with the RLV reply, and John Roe's is recorded as sent

#### Scenario: Blocked while autoreply is on
- **WHEN** autoreply is on, a collar holds `recvim`, and Jane Doe, who is not in her pause, sends the avatar `hi`
- **THEN** Jane Doe receives the RLV reply and not the autoreply, and a message she sends after the collar has lifted `recvim` is answered with the autoreply

#### Scenario: RLV off
- **WHEN** a collar holds `recvim`, RLV is switched off, and Jane Doe sends the avatar `hi`
- **THEN** the message is recorded with the text `hi` and no RLV reply is sent

#### Scenario: Object messages are not blocked
- **WHEN** a collar holds `recvim` and a scripted object sends the avatar an instant message
- **THEN** it is recorded with its text and no reply is sent
