# Spec Delta

## Purpose

Lets scripted objects the avatar owns hold it to RLV restrictions, as they would hold an avatar logged in with an RLV viewer: the bot keeps the restrictions and exceptions each object sets, applies them to the operator's commands, to what the bot does by itself and to RLV force commands, and tells scripts what it keeps. Defines who holds a restriction and how long it is kept, which restrictions have an effect and on what, the exemption of `rlv mode override`, the status queries, the acceptances RLV switches on, and `rlv reset`.

## ADDED Requirements

### Requirement: Restrictions and their holders
A restriction is an RLV command with one of the params `n`, `add`, `y` and `rem` (see `rlv-commands`, "Command syntax"). Its text is its behaviour, in lower case, followed by `:` and its option, as it was written, when it has an option. Two restrictions SHALL be the same when their behaviours are the same and both have no option or options that are equal without regard to letter case. Restrictions with an option that the RLV API calls exceptions (such as `sendim:<uuid>`) are restrictions here and SHALL be kept in the same way.

Every restriction has a holder: the object that sent it, known by its UUID, or the operator for a restriction given with `rlv cmd`. For every holder the bot SHALL keep the restrictions that holder has set, in the order they were set. `n` and `add` SHALL set the restriction for the sender; `y` and `rem` SHALL lift it for the sender. Setting a restriction the sender holds already and lifting one it does not hold SHALL change nothing and SHALL count as carried out. A command of one holder SHALL NOT set, lift or clear anything of another holder: a restriction two holders have set stays kept until both have lifted it. The operator SHALL be a holder apart from every object, also from an object that sends the same commands.

The bot SHALL keep every restriction whose behaviour is in one of these two lists, whatever its option:

- With an effect, as the requirements below say: `sendchat`, `emote`, `chatshout`, `chatnormal`, `chatwhisper`, `redirchat`, `rediremote`, `sendchannel`, `sendchannel_sec`, `sendchannel_except`, `sendim`, `sendim_sec`, `sendimto`, `startim`, `startimto`, `unsit`, `sit`, `sittp`, `interact`, `tploc`, `tplocal`, `tplure`, `tplure_sec`, `tprequest`, `tprequest_sec`, `accepttp`, `accepttprequest`, `touchall`, `touchworld`, `touchthis`, `touchme`, `touchattach`, `touchattachself`, `touchattachother`, `touchhud`, `fartouch`, `touchfar`, `detach`, `addattach`, `remattach`, `addoutfit`, `remoutfit`, `acceptpermission`, `denypermission`, `showinv`, `permissive`.
- Without an effect: `fly`, `temprun`, `alwaysrun`, `camzoommax`, `camzoommin`, `setcam_fovmin`, `setcam_fovmax`, `camdistmax`, `setcam_avdistmax`, `camdistmin`, `setcam_avdistmin`, `camdrawmin`, `camdrawmax`, `camdrawalphamin`, `camdrawalphamax`, `camdrawcolor`, `camunlock`, `setcam_unlock`, `camavdist`, `camtextures`, `setcam_textures`, `recvchat`, `recvchat_sec`, `recvchatfrom`, `recvemote`, `recvemote_sec`, `recvemotefrom`, `recvim`, `recvim_sec`, `recvimfrom`, `sendgesture`, `tplm`, `standtp`, `viewnote`, `viewscript`, `viewtexture`, `edit`, `editobj`, `editworld`, `editattach`, `rez`, `share`, `share_sec`, `defaultwear`, `unsharedwear`, `unsharedunwear`, `sharedwear`, `sharedunwear`, `detachthis`, `detachallthis`, `attachthis`, `attachallthis`, `detachthis_except`, `detachallthis_except`, `attachthis_except`, `attachallthis_except`, `showworldmap`, `showminimap`, `showloc`, `shownames`, `shownames_sec`, `shownametags`, `shownearby`, `showhovertextall`, `showhovertext`, `showhovertexthud`, `showhovertextworld`, `setgroup`, `setdebug`, `setenv`, `allowidle`.

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
- **WHEN** an attachment says `@showloc=n,recvim=n` to its owner
- **THEN** both are kept and listed by `rlv status` as having no effect, `status` still prints the region, instant messages are still recorded, and the operator log does not call either command unsupported

#### Scenario: Unknown behaviour
- **WHEN** an attachment says `@frobnicate=n` to its owner
- **THEN** nothing is kept and the operator log notes `frobnicate=n` as not supported

### Requirement: Clearing a holder's restrictions
`@clear` SHALL lift every restriction its sender holds. `@clear=<text>` SHALL lift those restrictions of its sender whose text contains `<text>`, compared without regard to letter case; an empty `<text>` lifts all of them. Either SHALL count as carried out also when nothing was lifted. Neither SHALL lift a restriction of another holder.

#### Scenario: Clear all
- **WHEN** a collar holds `detach`, `sendim` and `tplure:<uuid>` and says `@clear` to its owner
- **THEN** the bot keeps nothing for the collar

#### Scenario: Clear by text
- **WHEN** an object holds `tploc`, `tplure`, `tplure:<uuid>` and `sittp` and says `@clear=tpl` to its owner
- **THEN** it holds `sittp` only

#### Scenario: Only exceptions
- **WHEN** the same object holds `tplure` and `tplure:<uuid>` and says `@clear=tplure:` to its owner
- **THEN** it holds `tplure`

#### Scenario: Another holder's restrictions stay
- **WHEN** a collar and a cuff each hold `unsit` and the cuff says `@clear` to its owner
- **THEN** the collar still holds `unsit`

### Requirement: How long restrictions are kept
The restrictions of an object SHALL be dropped, all at once and without a command:

- when the simulator reports that the object is gone, which includes an attachment that is detached;
- when the avatar leaves the region and the object is not attached to the avatar at that moment, as an attachment or as a part of one;
- when the session is lost and when the bot logs out.

An attachment keeps its UUID through a teleport, so its restrictions SHALL stay kept from region to region. The restrictions of an object the simulator has not described to the bot SHALL be kept until one of the last two cases applies.

The restrictions of the operator SHALL be kept until they are lifted or cleared with `rlv cmd`, until `rlv reset`, or until the bot process ends, across session losses and logins.

All of this SHALL hold in every RLV mode, also while RLV is off. Nothing SHALL be saved: a restarted bot keeps no restriction.

#### Scenario: Attachment detached
- **WHEN** a collar holds `sendchat` and is detached from the avatar
- **THEN** the bot keeps nothing for the collar, and `rslbot ctl say hello` is carried out

#### Scenario: Rezzed object left behind
- **WHEN** a cage rezzed in the region holds `tplure` and `sittp`, and the avatar arrives in another region
- **THEN** the bot keeps nothing for the cage

#### Scenario: Attachment through a teleport
- **WHEN** a collar holds `sendim` and the avatar is teleported to another region
- **THEN** `sendim` is still kept for the collar after the arrival

#### Scenario: Part of an attachment
- **WHEN** a script in a child prim of an attached collar has said `@sendim=n` to its owner and the avatar is teleported to another region
- **THEN** `sendim` is still kept for that prim after the arrival

#### Scenario: Session lost
- **WHEN** a collar holds `unsit`, the operator has set `tploc` with `rlv cmd`, the avatar is kicked and the bot has logged in again
- **THEN** the bot keeps `tploc` for the operator and nothing for the collar's old UUID

#### Scenario: Gone while RLV is off
- **WHEN** a cage holds `sittp`, `rslbot ctl rlv mode off` is run, the cage is deleted, and `rslbot ctl rlv mode on` is run
- **THEN** the bot keeps nothing for the cage

#### Scenario: Not saved
- **WHEN** the operator has set `tploc` with `rlv cmd` and the bot is stopped and started again
- **THEN** the bot keeps no restriction

### Requirement: When restrictions are in force
A restriction the bot keeps is in force while RLV is on or in override (see `rlv-commands`, "RLV mode"), with the one exemption "Override exempts one command" gives. While RLV is off no restriction SHALL be in force: every command and every action of the bot SHALL be carried out as it is without this capability, and what is kept SHALL be in force again, as it then is, as soon as RLV is switched on, without any object sending it again.

Where a requirement below names a restriction with exceptions, the restriction SHALL be in force against a target (an avatar, an object or a channel) when a holder holds it without an option and no exception that counts names the target. An exception is the same behaviour with the target as its option. An exception SHALL count against a holder's restriction when any holder holds it; against a restriction whose behaviour ends in `_sec`, and against every restriction while a `permissive` restriction is in force, it SHALL count only when that same holder holds it. A restriction ending in `_sec` SHALL otherwise do what the restriction without that ending does, and its exceptions are those of the behaviour without the ending. An option that is to be a UUID, a channel number, a distance or a name and cannot be read as one SHALL leave the restriction kept and without an effect.

#### Scenario: Off lifts nothing and applies nothing
- **WHEN** a collar holds `sendchat`, `rslbot ctl rlv mode off` is run, and `rslbot ctl say hello` is run
- **THEN** the avatar says `hello`, and `rlv status` still lists `sendchat` for the collar

#### Scenario: Back on
- **WHEN** `rslbot ctl rlv mode on` is then run and `rslbot ctl say hello` is run again
- **THEN** the command is refused, naming `sendchat` and the collar

#### Scenario: Exception from another holder
- **WHEN** a collar holds `sendim`, a relay holds `sendim:<jane>`, and `rslbot ctl im <jane> hello` is run
- **THEN** the message is sent

#### Scenario: Secure restriction
- **WHEN** a collar holds `sendim_sec`, a relay holds `sendim:<jane>`, and `rslbot ctl im <jane> hello` is run
- **THEN** the command is refused; once the collar itself holds `sendim:<jane>`, the same command is carried out

#### Scenario: Permissive
- **WHEN** a collar holds `sendim` and `permissive`, a relay holds `sendim:<jane>`, and `rslbot ctl im <jane> hello` is run
- **THEN** the command is refused

### Requirement: Operator commands under restrictions
While a restriction is in force that the requirements below say refuses a control-socket command, that command SHALL be answered with an error and SHALL send nothing, move nothing and change neither the placement mode nor the held placement. The error SHALL name the restriction by its text, name one holder of it, as the object's name and UUID or as the operator, and say that `rslbot ctl rlv mode override` lets one command through. The restrictions SHALL be looked at when the command acts: for a command that first has to find an avatar or an inventory item, once that is found. A command that is refused for another reason before that SHALL be answered with that reason. The commands `status`, `perms`, `rlv`, `anim`, `sound`, `sendtp`, `persist`, `home` and `quit` SHALL NOT be refused by any restriction; what `persist` and `home` set in motion is held to restrictions as `presence-reconciler` says, where the persist override setting (see `rlv-commands`, "Persist override setting") can take the bot's own teleports and sits out of them.

#### Scenario: Refused command
- **WHEN** an attached collar named `Collar` holds `unsit`, the avatar sits on a chair, and `rslbot ctl stand` is run
- **THEN** the avatar stays seated, the placement mode does not change, and the command exits with status 1 with a text that names `unsit`, `Collar` with its UUID, and `rlv mode override`

#### Scenario: Restriction of the operator
- **WHEN** `rslbot ctl rlv cmd @tploc=n` has been carried out and `rslbot ctl tp 128 128 25 Home` is run
- **THEN** the command exits with status 1 naming `tploc` and the operator as its holder

#### Scenario: Other reasons come first
- **WHEN** a collar holds `sendim` and `rslbot ctl im "Nobody Here" hello` is run for a name the grid does not know
- **THEN** the command exits with status 1 saying the avatar was not found

#### Scenario: Never refused
- **WHEN** a collar holds `sendchat`, `sendim`, `unsit`, `tploc` and `showinv`, and `rslbot ctl status` and `rslbot ctl persist cur cur` are run
- **THEN** both are carried out

### Requirement: Chat the avatar sends
Restrictions SHALL apply to the `say`, `shout` and `whisper` commands (see `message-send`) after the message has been read as the chat bar reads it, in this order:

1. On channel 0, the volume: while `chatwhisper` is in force a whisper SHALL be said at normal volume; while `chatshout` is in force a shout SHALL be said at normal volume; then, while `chatnormal` is in force, what is to be said at normal volume or shouted SHALL be whispered. The text SHALL NOT be changed.
2. On channel 0, redirection. An emote is a text that starts with `/me ` or `/me'`. While a holder holds `rediremote:<channel>`, an emote SHALL be said on every channel so named and not on channel 0; while a holder holds `redirchat:<channel>`, a text that is no emote SHALL be. A named channel on which step 4 would not let the avatar chat SHALL be left out; when every named channel is left out, the command SHALL be refused.
3. On channel 0, for what was not redirected, `sendchat`: a text that does not start with `/` SHALL be refused. An emote, and any other text that starts with `/`, SHALL be refused when it contains one of the characters `(`, `)`, `"`, `-`, `*`, `=`, `_` and `^`; otherwise it SHALL be said cut off before its first `.` and then to its first 30 characters for an emote and its first 15 characters for other text. While an `emote` restriction is in force an emote SHALL NOT be cut off.
4. On any other channel: the command SHALL be refused while `sendchannel` is in force against that channel, with `sendchannel:<channel>` as its exceptions, and while a holder holds `sendchannel_except:<channel>` for it.

A command that is carried out with another volume, on other channels or with a shortened text SHALL be answered with success, and its answer and the operator log SHALL say what was changed and name the restriction. What the bot says by itself for RLV (the replies to queries) SHALL NOT be held to any of these.

#### Scenario: Chat refused
- **WHEN** a collar holds `sendchat` and `rslbot ctl say hello there` is run
- **THEN** nothing is said and the command exits with status 1 naming `sendchat`

#### Scenario: Emote goes through, shortened
- **WHEN** a collar holds `sendchat` and `rslbot ctl say /me nods slowly. Then she speaks at length` is run
- **THEN** the avatar says `/me nods slowly` on channel 0, and the answer says the text was shortened because of `sendchat`

#### Scenario: Emote with special signs
- **WHEN** a collar holds `sendchat` and `rslbot ctl say /me says "hello"` is run
- **THEN** nothing is said and the command exits with status 1

#### Scenario: Emote exception
- **WHEN** a collar holds `sendchat` and `emote`, and `rslbot ctl say /me walks all the way across the room and sits down` is run
- **THEN** the avatar says the whole text on channel 0

#### Scenario: Shout becomes normal
- **WHEN** a collar holds `chatshout` and `rslbot ctl shout over here` is run
- **THEN** the avatar says `over here` on channel 0 at normal volume, and the answer names `chatshout`

#### Scenario: Everything whispered
- **WHEN** a collar holds `chatnormal` and `rslbot ctl say hello` is run
- **THEN** the avatar whispers `hello` on channel 0

#### Scenario: Redirected chat
- **WHEN** a gag holds `redirchat:5` and `sendchat`, and `rslbot ctl say hello` is run
- **THEN** the avatar says `hello` on channel 5 and nothing on channel 0, and the command exits with status 0

#### Scenario: Private channel refused
- **WHEN** a collar holds `sendchannel` and `sendchannel:7`, and `rslbot ctl say /8 open` and `rslbot ctl say /7 open` are run
- **THEN** the first is refused naming `sendchannel`, and the second says `open` on channel 7

#### Scenario: Replies still go out
- **WHEN** a collar holds `sendchannel` and says `@versionnum=5` to its owner
- **THEN** the avatar says `10000` on channel 5

### Requirement: Instant messages the avatar sends
The `im` command (see `message-send`) SHALL be refused:

- while `sendim` is in force against the recipient, with `sendim:<uuid>` as its exceptions;
- while a holder holds `sendimto:<uuid>` with the recipient's UUID;
- when the bot has no conversation with the recipient, while `startim` is in force against the recipient, with `startim:<uuid>` as its exceptions, or a holder holds `startimto:<uuid>` with the recipient's UUID. The bot has a conversation with an avatar that has sent it an instant message since the last login, or to whom it has sent one since then.

Nothing SHALL be sent to the recipient in place of a refused message. An option of these restrictions that names a group SHALL have no effect.

#### Scenario: No instant messages
- **WHEN** a collar holds `sendim` and `rslbot ctl im "Jane Doe" hello` is run
- **THEN** Jane Doe receives nothing and the command exits with status 1 naming `sendim`

#### Scenario: One avatar barred
- **WHEN** a collar holds `sendimto:<jane>`, and `rslbot ctl im "Jane Doe" hello` and `rslbot ctl im "John Roe" hello` are run
- **THEN** the first is refused and the second is sent

#### Scenario: No new conversations
- **WHEN** a collar holds `startim`, Jane Doe has sent the bot an instant message since the login and John Roe has not, and `rslbot ctl im "Jane Doe" hi` and `rslbot ctl im "John Roe" hi` are run
- **THEN** the first is sent and the second is refused naming `startim`

### Requirement: Sitting and standing
While the avatar sits, on an object or on the ground, and `unsit` is in force, the bot SHALL NOT make the avatar leave its seat: `stand` (and `unsit`) SHALL be refused, `sit` SHALL be refused, since it leaves the seat for another, and every teleport SHALL be refused, as "Teleports and teleport offers" says. While `sit` is in force, `sit <uuid>` and `sit ground` SHALL be refused. While `interact` is in force, `sit <uuid>` SHALL be refused. While `sittp` is in force, `sit <uuid>` SHALL be refused when the object is farther from the avatar than the smallest distance its holders give, a `sittp` without a readable distance giving 1.5 metres; an object whose place the bot does not know SHALL count as too far.

#### Scenario: Cannot stand
- **WHEN** a chair's script holds `unsit` while the avatar sits on the chair, and `rslbot ctl stand` is run
- **THEN** the avatar stays seated and the command exits with status 1 naming `unsit`

#### Scenario: Unsit while standing
- **WHEN** a collar holds `unsit`, the avatar stands, and `rslbot ctl sit <chair>` is run
- **THEN** the avatar sits on the chair

#### Scenario: No changing seats
- **WHEN** a collar holds `unsit`, the avatar sits on a chair, and `rslbot ctl sit <bench>` is run
- **THEN** the avatar stays on the chair and the command is refused naming `unsit`

#### Scenario: Cannot sit
- **WHEN** a collar holds `sit`, and `rslbot ctl sit <chair>` and `rslbot ctl sit ground` are run
- **THEN** both are refused naming `sit`

#### Scenario: Sit from afar
- **WHEN** a cage holds `sittp`, and `rslbot ctl sit <uuid>` is run once for a chair 10 metres from the avatar and once for a stool 1 metre from it
- **THEN** the first is refused naming `sittp` and the avatar sits on the stool

#### Scenario: Smallest distance
- **WHEN** one object holds `sittp:5` and another `sittp:3`, and `rslbot ctl sit <uuid>` names a chair 4 metres from the avatar
- **THEN** the command is refused

#### Scenario: Object of unknown place
- **WHEN** a cage holds `sittp` and `rslbot ctl sit <uuid>` names an object the simulator has not described to the bot
- **THEN** the command is refused, saying that the bot does not know how far the object is

### Requirement: Teleports and teleport offers
While `tploc` is in force, `tp` SHALL be refused. While `tplocal` is in force, a `tp` to the region the avatar is in SHALL be refused, unless every holder's `tplocal` gives a distance and the destination is within the smallest of them from the avatar. While the avatar sits and `unsit` is in force, `tp` and `accept tp` (and `accepttp`) SHALL be refused.

While `tplure` is in force against an avatar, with `tplure:<uuid>` as its exceptions, a teleport offer from that avatar SHALL NOT be accepted, by command, by the auto-accept rules, after a `requesttp` or for `accepttp`: `accept tp` naming a remembered offer of that avatar SHALL be refused, and an offer that arrives SHALL be dropped: it SHALL NOT be kept for the operator, and the operator log SHALL say who offered and which restriction dropped it. The bot SHALL send the offering avatar nothing. `requesttp` and `sendtp` SHALL NOT be refused by any restriction.

While `tprequest` is in force against an avatar, with `tprequest:<uuid>` as its exceptions, a request of that avatar to be teleported to the bot's avatar SHALL be dropped: the operator log SHALL say so, and `accepttprequest` SHALL NOT answer it.

#### Scenario: No teleport by coordinates
- **WHEN** a collar holds `tploc` and `rslbot ctl tp 128 128 25 Home` is run
- **THEN** the avatar stays and the command exits with status 1 naming `tploc`

#### Scenario: No local teleport
- **WHEN** a cage holds `tplocal` and `rslbot ctl tp 200 200 25` is run without a region
- **THEN** the command is refused naming `tplocal`; a `tp` that names another region is carried out

#### Scenario: Local teleport within the distance
- **WHEN** a cage holds `tplocal:10`, the avatar is at 100, 100, 25, and `rslbot ctl tp 104 103 25` is run
- **THEN** the avatar is teleported

#### Scenario: Seated and held
- **WHEN** a chair holds `unsit`, the avatar sits on it, and `rslbot ctl tp 128 128 25 Home` is run
- **THEN** the command is refused naming `unsit`

#### Scenario: Offer dropped
- **WHEN** a collar holds `tplure` and Jane Doe offers the bot a teleport
- **THEN** the avatar stays, `rslbot ctl accept tp` says no teleport offer waits, and the operator log names Jane Doe and `tplure`

#### Scenario: Offer from an excepted avatar
- **WHEN** a collar holds `tplure` and `tplure:<jane>`, and Jane Doe offers the bot a teleport
- **THEN** the offer is decided as it is without `tplure`

#### Scenario: Offer that waited
- **WHEN** Jane Doe's offer waits for the operator, a collar then sets `tplure`, and `rslbot ctl accept tp` is run
- **THEN** the command is refused naming `tplure`, and the offer still waits

#### Scenario: Asked for, and dropped
- **WHEN** a collar holds `tplure`, `rslbot ctl requesttp "Jane Doe"` is run, and Jane Doe offers a teleport
- **THEN** the request is sent, and the offer is dropped

### Requirement: Touching
The `touch <uuid>` command (see `avatar-commands`) SHALL be refused as this table says. A HUD is an attachment of the bot's avatar on one of the points of the heads-up display; an attachment is any other object attached to an avatar; every other object is in-world. An object is taken with the linkset it is part of: a restriction or exception that names the root or the touched prim applies.

| In force | Refused |
|---|---|
| `interact` | every touch |
| `touchall` | every touch but that of a HUD |
| `touchworld`, with `touchworld:<uuid>` as its exceptions | a touch of an in-world object |
| `touchthis:<uuid>` | a touch of that object |
| `touchattach` | a touch of an attachment |
| `touchattachself` | a touch of an attachment of the bot's avatar |
| `touchattachother` | a touch of another avatar's attachment |
| `touchattachother:<uuid>` | a touch of an attachment of that avatar |
| `touchhud` | a touch of a HUD |
| `touchhud:<uuid>` | a touch of that HUD |
| `fartouch` or `touchfar`, with or without a distance | a touch of an object farther from the avatar than the smallest distance given, 1.5 metres where none is given; an object on the bot's avatar counts as at no distance, and one whose place the bot does not know as too far |

An object that holds `touchme`, or a part of which holds it, SHALL be touchable whatever the rows of this table other than `interact` say.

#### Scenario: Nothing in-world
- **WHEN** a collar holds `touchworld` and `rslbot ctl touch <uuid>` names a door rezzed in the region
- **THEN** the door's script sees no touch and the command exits with status 1 naming `touchworld`

#### Scenario: Excepted object
- **WHEN** a collar holds `touchworld` and `touchworld:<door>`, and `rslbot ctl touch <door>` is run
- **THEN** the door is touched

#### Scenario: Touch me
- **WHEN** a collar holds `touchall`, a lever rezzed in the region holds `touchme`, and `rslbot ctl touch <lever>` is run
- **THEN** the lever is touched

#### Scenario: Own attachment
- **WHEN** a cuff holds `touchattachself` and `rslbot ctl touch <uuid>` names the collar the avatar wears
- **THEN** the command is refused naming `touchattachself`

#### Scenario: HUD under touchall
- **WHEN** a collar holds `touchall` and `rslbot ctl touch <uuid>` names an object attached to `HUD Center`
- **THEN** the object is touched

#### Scenario: Too far
- **WHEN** a cuff holds `fartouch` and `rslbot ctl touch <uuid>` names a button 6 metres from the avatar
- **THEN** the command is refused naming `fartouch`

#### Scenario: Interact
- **WHEN** a cage holds `interact`, and `rslbot ctl touch <lever>` and `rslbot ctl sit <chair>` are run for a lever that holds `touchme`
- **THEN** both are refused naming `interact`

### Requirement: Attachments and clothing
The `attach`, `wear` and `detach` commands (and `unwear`; see `avatar-outfit`) SHALL be held to these restrictions. An attachment point is named as `rlv-commands` ("Force commands") names one, and a kind of clothing or of body part as `avatar-outfit` names one.

| In force | Effect |
|---|---|
| `detach` without an option, held by an object attached to the avatar or by a part of one | that attachment cannot be detached |
| `detach:<point>` | nothing attached to that point can be detached, and nothing can be attached to it |
| `remattach` | no attachment can be detached |
| `remattach:<point>` | nothing attached to that point can be detached |
| `addattach` | nothing can be attached |
| `addattach:<point>` | nothing can be attached to that point |
| `addoutfit` | no clothing and no body part can be put on |
| `addoutfit:<kind>` | none of that kind can be put on |
| `remoutfit` | no clothing can be taken off, and no body part replaced |
| `remoutfit:<kind>` | none of that kind can be taken off or replaced |

A `detach` without an option whose holder is the operator, or an object that is not on the avatar, SHALL have no effect. An `attach`, or a `wear` of an object, SHALL be refused when nothing can be attached to the point the object would go to; when that point is not known before the simulator chooses it, the command SHALL be refused while any point is closed to attaching. A `wear` of clothing or of a body part SHALL be refused when its kind cannot be put on, and a `wear` of a body part also when its kind cannot be replaced. A `detach` of one attachment or of one piece of clothing SHALL be refused when that cannot be detached or taken off. A `detach` of an attachment point SHALL detach those attachments on the point that can be detached and leave the others; it SHALL be answered with an error that names the restriction and what stayed on when anything did, and SHALL send nothing when nothing on the point can be detached.

When the bot restores attachments after a login (see `avatar-outfit`, "Restoring attachments after a login"), it SHALL leave out every object that an `attach` of it would be refused for, and say so in the operator log. The bot SHALL NOT, for the sake of a restriction, put back an attachment that left the avatar, or take off one that came onto it.

#### Scenario: Locked attachment
- **WHEN** an attached collar holds `detach` and `rslbot ctl detach Objects/Collar` is run
- **THEN** the collar stays on and the command exits with status 1 naming `detach` and the collar

#### Scenario: Lock from a child prim
- **WHEN** a script in a child prim of the attached collar has said `@detach=n` to its owner and `rslbot ctl detach Objects/Collar` is run
- **THEN** the collar stays on

#### Scenario: Unlocked again
- **WHEN** the collar then says `@detach=y` to its owner and the command is run again
- **THEN** the collar is detached into the inventory

#### Scenario: Locked point
- **WHEN** a collar holds `detach:left hand`, `Watch` is attached to the left hand, and `rslbot ctl detach Objects/Watch` and `rslbot ctl attach Objects/Radio left hand` are run
- **THEN** both are refused naming `detach:left hand`

#### Scenario: Point with one locked attachment
- **WHEN** `Cuff` and `Watch` are attached to the left hand, `Cuff` holds `detach`, and `rslbot ctl detach left hand` is run
- **THEN** `Watch` is detached, `Cuff` stays on, and the command exits with status 1 naming `detach` and `Cuff`

#### Scenario: Nothing may be attached
- **WHEN** a collar holds `addattach` and `rslbot ctl attach Objects/Radio` is run
- **THEN** nothing is attached and the command is refused naming `addattach`

#### Scenario: Point not known beforehand
- **WHEN** a collar holds `addattach:chest`, the inventory records no attachment point for `Radio`, and `rslbot ctl attach Objects/Radio` is run without a body part
- **THEN** the command is refused, saying that the point is not known and naming `addattach:chest`; with `right hand` given, `Radio` is attached to the right hand

#### Scenario: Clothing held on
- **WHEN** a collar holds `remoutfit:shirt`, and `rslbot ctl unwear shirt` and `rslbot ctl unwear pants` are run
- **THEN** the first is refused naming `remoutfit:shirt` and the pants are taken off

#### Scenario: No new clothing
- **WHEN** a collar holds `addoutfit` and `rslbot ctl wear Clothing/Jacket` is run
- **THEN** the command is refused naming `addoutfit`

#### Scenario: Restoring after a login
- **WHEN** the operator has set `addattach:chest` with `rlv cmd`, the bot logs in again, and the Current Outfit folder lists `Badge` for the chest and `Watch` for the left hand, neither attached
- **THEN** `Watch` is attached, `Badge` is not, and the operator log says `Badge` was left out because of `addattach:chest`

### Requirement: Listing the inventory
While `showinv` is in force, the `inventory` command (see `inventory-list`) SHALL be refused. Naming an item by its path in another command SHALL NOT be refused for it.

#### Scenario: Inventory closed
- **WHEN** a collar holds `showinv`, and `rslbot ctl inventory Objects` and `rslbot ctl attach Objects/Radio` are run
- **THEN** the first is refused naming `showinv`, and `Radio` is attached

### Requirement: Acceptances that RLV switches on
While a holder holds `accepttp` without an option, the bot SHALL accept every teleport offer by itself, and while a holder holds `accepttp:<uuid>`, every offer of that avatar: as it accepts an offer under the auto-accept rules (see `permission-requests`, "Accepting a request"), whatever the rules and exceptions of `tp` are, and going to manual mode in the same way. `tplure` and a seated `unsit` come first, as "Teleports and teleport offers" says; an offer that a seated `unsit` keeps the bot from accepting SHALL wait for the operator.

While a holder holds `accepttprequest` without an option, the bot SHALL answer every request of an avatar to be teleported to the bot's avatar by sending that avatar a teleport offer, as `sendtp` does and without a message, and while a holder holds `accepttprequest:<uuid>`, every request of that avatar. `tprequest` comes first. The placement mode SHALL NOT change.

While `acceptpermission` is in force, the kinds `attach` and `controls` SHALL count as auto-accepted for every script permission request (see `permission-requests`, "Deciding a request"), whatever their rules and exceptions are. While `denypermission` is in force, a script permission request that has the kind `attach` or `controls` SHALL NOT be accepted, by itself or with `accept`, and SHALL NOT be kept; `denypermission` comes before `acceptpermission`.

#### Scenario: Teleport offers accepted
- **WHEN** a collar holds `accepttp:<jane>`, the config has no `perms` table, and Jane Doe offers the bot a teleport
- **THEN** the bot's avatar arrives at the place Jane Doe offered, and the bot is in manual mode

#### Scenario: Accept from anyone
- **WHEN** a collar holds `accepttp` and John Roe offers the bot a teleport
- **THEN** the bot accepts it by itself

#### Scenario: Lure restriction comes first
- **WHEN** a collar holds `accepttp` and `tplure`, and John Roe offers the bot a teleport
- **THEN** the offer is dropped

#### Scenario: Teleport request answered
- **WHEN** a collar holds `accepttprequest` and Jane Doe asks to be teleported to the bot's avatar
- **THEN** Jane Doe receives a teleport offer from the bot's avatar, and the placement mode is what it was

#### Scenario: Teleport request dropped
- **WHEN** a collar holds `accepttprequest` and `tprequest`, and Jane Doe asks to be teleported to the bot's avatar
- **THEN** no offer is sent and the operator log says the request was dropped because of `tprequest`

#### Scenario: Permissions granted
- **WHEN** a collar holds `acceptpermission`, the config has no `perms` table, and a script asks for permission to attach its object and to take controls
- **THEN** the bot grants both

#### Scenario: Other kinds still follow the rules
- **WHEN** a collar holds `acceptpermission`, the rule for `anim` is an empty list, and a script asks for permission to take controls and to trigger animations
- **THEN** the request waits for the operator

### Requirement: Force commands under restrictions
An RLV force command (see `rlv-commands`, "Force commands"), from an object or from `rlv cmd`, SHALL be refused while one of these restrictions is in force, and the operator log SHALL name the restriction and a holder of it:

| Force command | Refused by |
|---|---|
| `@sit:<uuid>=force` | `sit`; `interact`; `sittp`, by its distance; `unsit` while the avatar sits |
| `@sitground=force` | `sit`; `unsit` while the avatar sits on an object |
| `@unsit=force` | `unsit` |
| `@tpto:...=force`, in either form | `tploc`; `unsit` while the avatar sits |
| `@detach` and `@remattach` with a point or a UUID, `@detachme=force` | for each attachment concerned, what keeps a `detach` of it from being carried out: the attachments that cannot be detached stay, and the command is refused when none can |
| `@remoutfit:<kind>=force` | `remoutfit`, `remoutfit:<kind>` |

For this, the restrictions that the sender of the force command itself holds SHALL NOT count: an object's own restrictions never stand in the way of its own force commands, and neither do the operator's restrictions in the way of `rlv cmd`. No other restriction SHALL refuse a force command. The check SHALL be made when the command is dealt with. For as long as the object a followed `@sit` named is the held seat because of that command (see `rlv-commands`, "Forced movement and the held placement"), the restrictions its sender holds SHALL NOT count against the bot's steps towards that seat either.

#### Scenario: Another object's restriction
- **WHEN** a collar holds `unsit`, the avatar sits on a chair, and a bench the avatar owns says `@sit:<bench>=force` to its owner
- **THEN** the avatar stays on the chair and the operator log says the command was refused because of `unsit`, held by the collar

#### Scenario: The sender's own restriction
- **WHEN** a cage holds `sittp` and says `@sit:<uuid>=force` to its owner, naming a seat 20 metres from the avatar
- **THEN** the avatar is seated on that seat

#### Scenario: Release by the holder
- **WHEN** a chair holds `unsit`, the avatar sits on it, and the chair says `@unsit=force` to its owner
- **THEN** the avatar stands up

#### Scenario: Forced teleport inhibited
- **WHEN** a collar holds `tploc` and a teleporter the avatar owns says `@tpto:Home/128/100/25=force` to its owner
- **THEN** the avatar stays and the operator log names `tploc` and the collar

#### Scenario: Locked attachments stay
- **WHEN** `Cuff` and `Watch` are attached to the left hand, `Cuff` holds `detach`, and a relay says `@detach:left hand=force` to its owner
- **THEN** `Watch` is detached and `Cuff` stays on

#### Scenario: Operator's console
- **WHEN** the operator has set `unsit` with `rlv cmd`, the avatar sits, and `rslbot ctl stand` and then `rslbot ctl rlv cmd @unsit=force` are run
- **THEN** the first is refused naming `unsit` and the operator, and the second makes the avatar stand up

#### Scenario: Followed sit by the holder of sittp
- **WHEN** the RLV persist setting is `follow`, a cage holds `sittp` and says `@sit:<uuid>=force` to its owner, naming a seat 20 metres from the avatar
- **THEN** the seat becomes the held seat and the bot seats the avatar on it

### Requirement: Override exempts one command
While RLV is in override (see `rlv-commands`, "RLV mode"), the next control-socket command the bot receives other than `rlv`, `status`, `perms` and `quit` SHALL be the exempt command. For the exempt command no restriction SHALL be in force: it SHALL be carried out exactly as it would be with no restriction kept. RLV SHALL be `on` again at the moment the exempt command is taken up, before anything is known of its outcome and whatever its outcome is, also when it is refused for a reason that has nothing to do with RLV. A line the bot cannot read as a command SHALL NOT be the exempt command.

The exemption SHALL cover everything that command does until it is answered, and in addition:

- for `requesttp`, the acceptance of the one offer that answers the request;
- for `persist` and `home`, the bot's steps towards the placement the command made the held one (see `presence-reconciler`), until the avatar is in that placement for the first time, or until the placement mode or the held placement is changed by anything else.

Override SHALL change nothing else. While RLV is in override, the commands objects send SHALL be read, kept and answered as while it is on; force commands SHALL be refused as "Force commands under restrictions" says; the acceptances and the drops of offers and requests SHALL be made as while it is on; the bot's own steps that do not belong to an exempt command SHALL be held back as while it is on; and `rlv cmd` SHALL be carried out as while it is on, SHALL NOT be the exempt command, and SHALL leave RLV in override.

#### Scenario: One command through
- **WHEN** a collar holds `unsit`, the avatar sits on a chair, and `rslbot ctl rlv mode override` and then `rslbot ctl stand` are run
- **THEN** the avatar stands up, and `rlv status` then prints `rlv: on`

#### Scenario: Only one
- **WHEN** the avatar is then seated on the chair again and `rslbot ctl stand` is run once more
- **THEN** the command is refused naming `unsit`

#### Scenario: Status does not use it up
- **WHEN** `rslbot ctl rlv mode override`, `rslbot ctl status` and `rslbot ctl rlv status` are run
- **THEN** the last prints `rlv: override`

#### Scenario: Used up by a command that fails
- **WHEN** `rslbot ctl rlv mode override` is run, and then `rslbot ctl sit <uuid>` naming no object in the region
- **THEN** the sit fails with the simulator's reason, and RLV is `on`

#### Scenario: Unrestricted command uses it up
- **WHEN** `rslbot ctl rlv mode override` and `rslbot ctl sendtp "Jane Doe"` are run
- **THEN** RLV is `on` after the second

#### Scenario: Force commands are not exempt
- **WHEN** a collar holds `unsit`, the avatar sits, `rslbot ctl rlv mode override` is run, and a bench the avatar owns says `@sit:<bench>=force` to its owner
- **THEN** the forced sit is refused because of `unsit`, and RLV is still in override

#### Scenario: rlv cmd is not exempt
- **WHEN** a collar holds `tploc`, and `rslbot ctl rlv mode override` and `rslbot ctl rlv cmd @tpto:Home/128/100/25=force` are run
- **THEN** the second is refused naming `tploc`, and `rlv status` prints `rlv: override`

#### Scenario: Offers are not exempt
- **WHEN** a collar holds `tplure`, `rslbot ctl rlv mode override` is run, and Jane Doe offers the bot a teleport
- **THEN** the offer is dropped, and RLV is still in override

#### Scenario: Requesting a teleport under override
- **WHEN** a collar holds `tplure`, and `rslbot ctl rlv mode override` and `rslbot ctl requesttp "Jane Doe"` are run, and Jane Doe then offers a teleport
- **THEN** the bot accepts that offer and goes; a second offer of hers is dropped

#### Scenario: Home under override
- **WHEN** the persist override setting is empty, the bot holds a chair in `Target`, the avatar is in `Home`, a collar holds `tploc` and `sittp`, and `rslbot ctl rlv mode override` and `rslbot ctl home` are run
- **THEN** the bot teleports the avatar to `Target` and seats it on the chair; when the avatar is unseated afterwards, the bot's next sit is held to `sittp`

### Requirement: Status queries
While RLV is on or in override the bot SHALL answer `@getstatus[:<part>[;<separator>]]=<channel>` by saying, once and at normal volume on that channel, the restrictions its sender holds, and `@getstatusall` with the same option and param by saying the restrictions every holder holds, the operator included. The reply SHALL be, for each restriction in the order the holders first set one and each holder's in the order it set them, the separator followed by the restriction's text; a restriction two holders hold appears once for each. `<part>` SHALL leave out the restrictions whose text does not contain it, compared without regard to letter case. The separator SHALL be `/` when none is given or the one given is empty. The reply SHALL be sent as it is also when it is empty, and SHALL be cut off at 1023 bytes. The channel SHALL be as for the version queries (see `rlv-commands`, "Version queries"). Restrictions without an effect SHALL be part of the reply like the others.

#### Scenario: Own restrictions
- **WHEN** a collar holds `tploc`, `tplure`, `tplm` and `sittp`, a cuff holds `detach`, and the collar says `@getstatus=2222` to its owner
- **THEN** a script listening on channel 2222 hears the avatar say `/tploc/tplure/tplm/sittp`

#### Scenario: Part of a rule
- **WHEN** the same collar says `@getstatus:tpl=2222` to its owner
- **THEN** the avatar says `/tploc/tplure/tplm` on channel 2222

#### Scenario: Separator
- **WHEN** the same collar says `@getstatus:;#=2222` and then `@getstatus:;=2222` to its owner
- **THEN** the avatar says `#tploc#tplure#tplm#sittp` and then `/tploc/tplure/tplm/sittp` on channel 2222

#### Scenario: All holders
- **WHEN** the cuff says `@getstatusall=2222` to its owner
- **THEN** the avatar says `/tploc/tplure/tplm/sittp/detach` on channel 2222

#### Scenario: After a clear
- **WHEN** the collar says `@clear,getstatus=2222` to its owner
- **THEN** the reply on channel 2222 is empty

#### Scenario: By hand
- **WHEN** the operator has set `tploc` with `rlv cmd`, a collar holds `detach`, and `rslbot ctl rlv cmd @getstatus=5` is run
- **THEN** the command prints `getstatus=5: /tploc`

### Requirement: Reset command
`rlv reset` SHALL drop every restriction the bot keeps, of every object and of the operator. It SHALL work in every RLV mode and in every state of the bot, connected or not, and SHALL be answered with success and the text `rlv reset: <n> restrictions cleared`, `<n>` being how many were kept. It SHALL NOT change the RLV mode, the RLV persist setting, the placement mode or the held placement, SHALL NOT drop the RLV commands that are kept for later, and SHALL send nothing in-world: the objects are not told, and one that sets a restriction afterwards holds it. `rlv reset` with further arguments SHALL be answered with an error and change nothing.

#### Scenario: Everything cleared
- **WHEN** a collar holds `detach` and `sendim`, the operator has set `tploc`, and `rslbot ctl rlv reset` is run
- **THEN** it prints `rlv reset: 3 restrictions cleared`, exits with status 0, and `rlv status` prints `restrictions: none`

#### Scenario: While RLV is off
- **WHEN** a collar holds `unsit`, RLV is off, and `rslbot ctl rlv reset` and then `rslbot ctl rlv mode on` are run
- **THEN** RLV is on and the bot keeps no restriction

#### Scenario: Set again afterwards
- **WHEN** `rslbot ctl rlv reset` has been carried out and the collar then says `@detach=n` to its owner
- **THEN** the collar holds `detach`

#### Scenario: Bad arguments
- **WHEN** `rslbot ctl rlv reset all` is run
- **THEN** the command exits with status 1 and nothing is cleared
