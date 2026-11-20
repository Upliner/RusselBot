# Spec Delta

## ADDED Requirements

### Requirement: Steps that RLV restrictions forbid are held back
While an RLV restriction is in force (see `rlv-restrictions`, "When restrictions are in force"), the bot SHALL NOT start a step of its own towards the held placement that the restriction would refuse the operator's command for, whatever "Converge on the desired state" chooses, unless the persist override setting names the step's sort (see `rlv-commands`, "Persist override setting"):

| Step | Held back as | Sort |
|---|---|---|
| teleport to the held location, and the teleport that repositions the avatar after repeated sit failures | a `tp` to that destination (`rlv-restrictions`, "Teleports and teleport offers") | `tp` |
| sit on the held object | `sit <uuid>` (`rlv-restrictions`, "Sitting and standing") | `sit` |
| sit on the ground | `sit ground` | `sit` |
| the stand a `persist` or `home` calls for, when the held sit state does not match the seat the avatar is on | `stand` | `sit` |

A sit on the held object that a restriction holds back because the bot does not know how far the object is SHALL start the object scan (see `rlv-restrictions`, "Objects the bot does not know"), so that the step is no longer held back once the object is described and within reach.

A step whose sort the persist override setting names SHALL be started as if no restriction were kept: no restriction SHALL hold it back, `unsit` while the avatar sits among them. The setting SHALL change nothing else: the operator's commands, RLV force commands, the acceptance of offers and requests and the restoring of attachments SHALL be held to restrictions whatever it names.

A login SHALL never be held back. A step that is held back is not a failed attempt: it SHALL NOT lengthen any backoff delay, SHALL NOT count as a sit failure, and SHALL NOT make the bot give up. The bot SHALL stay in holding mode with its held placement unchanged, and SHALL choose its next action anew whenever a restriction is set, lifted or dropped, whenever the RLV mode or the persist override setting changes, and whenever what the simulator shows of the avatar changes. It SHALL write one line to the operator log when a step starts being held back, naming the step, the restriction and a holder of it, and one when the step is no longer held back; it SHALL NOT repeat the line while nothing changes. When it starts a step that a restriction would hold back but for the persist override setting, the step's line in the operator log SHALL name the restriction and say that the setting lets the step through.

The `persist` and `home` commands SHALL be carried out whatever restrictions are in force: they change what the bot holds, and the steps that follow are held back or not as this requirement says. The steps that belong to an exempt `persist` or `home`, and the steps towards a seat that a followed `@sit` made the held one, are excepted as `rlv-restrictions` says ("Override exempts one command", "Force commands under restrictions"). While RLV is off no step SHALL be held back.

#### Scenario: Caged
- **WHEN** the bot holds a chair in `Target` and the avatar sits on it, the persist override setting is empty, and a cage the avatar owns then moves the avatar to `Elsewhere` and holds `tploc`
- **THEN** the bot starts no teleport, stays in holding mode, and the operator log says once that the teleport to `Target` is held back because of `tploc`, naming the cage

#### Scenario: Released
- **WHEN** the cage then says `@tploc=y` to its owner
- **THEN** the bot teleports the avatar to `Target` and seats it on the chair, and the operator log says the teleport is no longer held back

#### Scenario: The holder is left behind
- **WHEN** the bot holds a chair in `Target`, a rezzed object in `Target` holds `sit`, and the avatar is unseated
- **THEN** the bot starts no sit while the object holds `sit`, and sits the avatar down again once the object is deleted

#### Scenario: No backoff from waiting
- **WHEN** a sit on the held object has been held back for an hour because of `sittp` and the restriction is lifted
- **THEN** the bot starts the sit at once, with the delay it would have had an hour ago

#### Scenario: Persist is accepted
- **WHEN** a chair holds `unsit`, the avatar sits on it, the persist override setting is empty, and `rslbot ctl persist cur stand` is run
- **THEN** the command exits with status 0 and the held sit state is `stand`, the avatar stays seated, and the operator log says the stand is held back because of `unsit`

#### Scenario: Login is never held back
- **WHEN** the operator has set `tploc` with `rlv cmd` and the session is lost
- **THEN** the bot logs in again as it does without the restriction

#### Scenario: RLV off
- **WHEN** a cage holds `tploc`, the avatar is outside the held region, and `rslbot ctl rlv mode off` is run
- **THEN** the bot teleports the avatar to the held location

#### Scenario: Back from the hub after a region restart
- **WHEN** the config has `override = ["sit", "tp"]` in its `[persist]` table, the bot holds a chair in `Target`, a collar the avatar wears holds `tploc`, `sittp` and `unsit`, and a restart of `Target` leaves the avatar in a hub region without the session being lost
- **THEN** once `Target` is up the bot teleports the avatar there and seats it on the chair, and the operator log names `tploc` and `sittp` as let through by the setting

#### Scenario: Teleports only
- **WHEN** the persist override setting is `tp`, the bot holds a chair in `Target`, a collar holds `tploc` and `sit`, and the avatar is moved to another region
- **THEN** the bot teleports the avatar to `Target`, starts no sit, and the operator log says the sit is held back because of `sit`

#### Scenario: Seated elsewhere
- **WHEN** the persist override setting is `tp`, the bot holds a place in `Target`, and the avatar sits on an object in another region whose script holds `unsit`
- **THEN** the bot teleports the avatar to `Target`

#### Scenario: Stand under the setting
- **WHEN** the persist override setting is `sit`, a chair holds `unsit`, the avatar sits on it, and `rslbot ctl persist cur stand` is run
- **THEN** the avatar stands up

#### Scenario: Commands are not covered
- **WHEN** the persist override setting is `sit` and `tp`, a collar holds `tploc`, and `rslbot ctl tp 128 128 25 Home` is run
- **THEN** the command is refused naming `tploc`

#### Scenario: Switched while a step waits
- **WHEN** the teleport to `Target` is held back because of `tploc` and `rslbot ctl rlv persist override tp` is run
- **THEN** the bot starts the teleport at once

#### Scenario: Seat not described yet
- **WHEN** `scan_distance` is not configured, the persist override setting is empty, a collar holds `sittp:20`, and the avatar arrives 2 metres from the seat the bot holds, which the simulator has only mentioned
- **THEN** the bot scans, and seats the avatar as soon as the seat is described; the operator log says that the sit was held back and that it no longer is

#### Scenario: Seat out of reach
- **WHEN** the same happens with `sittp` held without a distance
- **THEN** once the seat is described the sit stays held back, and the log gives the seat's distance
