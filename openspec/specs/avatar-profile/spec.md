# avatar-profile Specification

## Purpose

Lets the operator read an avatar's profile and online status through the `profile` command, to find out who an avatar is without logging in with a viewer.

## Requirements

### Requirement: Profile command
`profile <avatar>` SHALL be answered with success and the profile of that avatar as the grid gives it, as a text of `name: value` lines. The arguments after `profile` SHALL be joined with single spaces into the avatar, which SHALL be given and resolved as the recipient of an `im` is (see `message-send`, "Recipient resolution"). The bot SHALL answer with an error, and SHALL ask the grid for no profile, when no avatar is given or the avatar cannot be resolved.

#### Scenario: Avatar by name
- **WHEN** `rslbot ctl profile Jane Doe` is run while the bot is connected to a region
- **THEN** it prints the profile of the avatar named `Jane Doe` and exits with status 0

#### Scenario: Name in one argument
- **WHEN** `rslbot ctl profile "Jane Doe"` or `rslbot ctl profile jane.doe` is run
- **THEN** it prints the same profile

#### Scenario: Avatar by UUID
- **WHEN** `rslbot ctl profile 11111111-2222-3333-4444-555555555555` is run
- **THEN** it prints the profile of that avatar, and the grid is not asked to search for a name

#### Scenario: Unknown name
- **WHEN** `rslbot ctl profile Nobody Here` is run and the grid has no such avatar
- **THEN** it exits with status 1 naming `Nobody Here`

#### Scenario: No avatar
- **WHEN** `rslbot ctl profile` is run
- **THEN** it exits with status 1 and the grid is asked nothing

### Requirement: Lines of a profile
The text SHALL have these lines, in this order: `id`, the avatar's UUID; `name`, as `First Last`; `display name`; `online` (see "Online status in a profile"); `friend`, which is `yes` when the avatar is a friend of the bot's avatar (see `friend-list`) and `no` otherwise; `born`, the day the account was made; `partner`, the UUID of the avatar's partner; `about`, the text of the profile; and `first life`, its text about real life.

#### Scenario: Full profile
- **WHEN** Jane Doe, display name `JD`, a friend who is online, was born on 14 March 2010, has John Roe as her partner, and has the texts `Builder.` and `Not telling.`
- **THEN** `rslbot ctl profile Jane Doe` prints, in this order, `id: ` with her UUID, `name: Jane Doe`, `display name: JD`, `online: yes`, `friend: yes`, `born: 2010-03-14`, `partner: ` with John Roe's UUID, `about: Builder.` and `first life: Not telling.`

#### Scenario: Not a friend
- **WHEN** `rslbot ctl profile` names an avatar that is not a friend of the bot's avatar
- **THEN** it prints `friend: no`

### Requirement: Profile lines that are left out
`id`, `online` and `friend` SHALL always be printed. `name` and `display name` SHALL be left out when the bot does not know them, `born` when the grid does not tell it, `partner` when the avatar has none, and `about` and `first life` when the text is empty.

#### Scenario: Bare profile
- **WHEN** an avatar has no partner and both texts empty, and the grid does not tell when it was born
- **THEN** its profile has the lines `id`, `name`, `display name`, `online` and `friend`, and no other

#### Scenario: Name service that does not answer
- **WHEN** `rslbot ctl profile <uuid>` is run and the name service does not name that avatar
- **THEN** the profile is printed without the `name` and `display name` lines

### Requirement: Values of profile lines
`born` SHALL be written as `YYYY-MM-DD`, or as the grid wrote it when the bot cannot read it as a day. A tab, line feed, carriage return or backslash in a value SHALL be written as `\t`, `\n`, `\r` and `\\`, so that every value stays on its line.

#### Scenario: Text of several lines
- **WHEN** an avatar's profile text is `Builder.`, a line feed and `Ask me about sculpts.`
- **THEN** its `about` line is `about: Builder.\nAsk me about sculpts.`, with the backslash and the `n` as two characters

#### Scenario: Day written the grid's way
- **WHEN** the grid gives the day an account was made as `3/14/2010`
- **THEN** the profile has `born: 2010-03-14`

### Requirement: Online status in a profile
`online` SHALL be `yes` when the grid's answer says the avatar is online. Otherwise it SHALL be `no` when the avatar is a friend that lets the bot's avatar see when it is online (see `friend-list`), and `unknown` in every other case, since a grid answers for an avatar that hides its status as it does for one that is offline.

#### Scenario: Friend who is online
- **WHEN** Jane Doe is a friend who lets the avatar see when she is online, and is online
- **THEN** her profile has `online: yes`

#### Scenario: Friend who is offline
- **WHEN** Jane Doe is such a friend and is not logged in
- **THEN** her profile has `online: no`

#### Scenario: Stranger shown as online
- **WHEN** an avatar who is not a friend shows its online status to everyone and is online
- **THEN** its profile has `online: yes`

#### Scenario: Stranger not shown as online
- **WHEN** the grid's answer for an avatar who is not a friend does not say it is online
- **THEN** its profile has `online: unknown`

#### Scenario: Friend who hides its status
- **WHEN** the grid's answer for a friend who does not let the avatar see when it is online does not say it is online
- **THEN** its profile has `online: unknown`

### Requirement: Asking the grid for a profile
The bot SHALL ask the grid for the profile anew for every `profile` command, and the name service for the avatar's names when it does not have them. It SHALL get profiles from simulators that offer a profile service, as those of Second Life do, and from simulators that answer for profiles on the simulator connection only, as those of OpenSimulator do. Without a `profile` command the bot SHALL ask for no profile.

#### Scenario: Asked every time
- **WHEN** `rslbot ctl profile Jane Doe` is run twice, and Jane Doe logs out between the two
- **THEN** the grid is asked for her profile twice, and the second answer does not have `online: yes`

#### Scenario: On OpenSimulator
- **WHEN** the simulator offers no profile service and `rslbot ctl profile Rig Owner` is run
- **THEN** the profile is printed, from what the simulator answers on the simulator connection

#### Scenario: No command, no requests
- **WHEN** the bot runs for a day and no `profile` command is given
- **THEN** it asks the grid for no profile

### Requirement: When a profile is answered
The bot SHALL answer a `profile` once the profile has arrived and the name service has answered for the avatar or its names are known. It SHALL answer no later than 10 seconds after the avatar was resolved: with the profile when it has arrived by then, and otherwise with an error that names the avatar and says the grid did not answer.

#### Scenario: Grid that does not answer
- **WHEN** the grid does not answer for the profile of an avatar
- **THEN** `rslbot ctl profile` for it exits with status 1 within 21 seconds, naming the avatar

#### Scenario: Avatar the grid does not know
- **WHEN** `rslbot ctl profile 11111111-2222-3333-4444-555555555555` is run and the grid has no such avatar
- **THEN** the command exits with status 1 naming that UUID

#### Scenario: Profile before the names
- **WHEN** the profile arrives after a second and the name service never answers
- **THEN** the command exits with status 0 within 11 seconds of the avatar being resolved, without the `name` and `display name` lines

### Requirement: Profile needs the avatar in a region
`profile` SHALL be answered with an error and SHALL ask the grid nothing when the avatar is not connected to a region or is in the middle of a teleport or a logout. A `profile` that is waiting when the avatar leaves the region or the session is lost SHALL be answered with an error that says so.

#### Scenario: Between logins
- **WHEN** `rslbot ctl profile Jane Doe` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Teleported away while waiting
- **WHEN** `rslbot ctl profile Jane Doe` is waiting for the grid and the avatar is teleported to another region
- **THEN** the command exits with status 1 saying the avatar left the region

### Requirement: Profile reads only
`profile` SHALL NOT move the avatar, SHALL NOT change the bot's placement mode or the held placement, and SHALL NOT change anything of the profile it reads or of the avatar's own. No RLV restriction SHALL be applied to it, and it SHALL NOT be the exempt command of an RLV override (see `rlv-restrictions`, "Override exempts one command").

#### Scenario: During an override
- **WHEN** `rslbot ctl rlv mode override` has been carried out and `rslbot ctl profile Jane Doe` is run
- **THEN** the profile is printed and `rlv status` still prints `rlv: override`

#### Scenario: Restriction on names
- **WHEN** a collar holds `shownames`, RLV is on, and `rslbot ctl profile Jane Doe` is run
- **THEN** the profile is printed with its `name` line

### Requirement: Profiles appear in the operator log
The bot SHALL write a line to its operator log for every `profile` it answers, naming the avatar by its UUID and, when known, its name, and for every `profile` it refuses or fails, with the reason.

#### Scenario: Reading the log after a profile
- **WHEN** `rslbot ctl profile jane.doe` has been answered
- **THEN** the log shows a line for the command with Jane Doe's UUID and name
