# Spec Delta

## MODIFIED Requirements

### Requirement: Status command
`status` SHALL be answered with success and a text of one `name: value` line each for: `connected` (`yes` or `no`); `avatar` (the name of the bot's avatar, `First Last`, as the config gives it, a single word standing for the last name `Resident`); `region` (the region's name); `position` (the avatar's current position as the bot knows it, as `x, y, z`); `seat` (`target` when seated on the object that is the held sit state, `object <uuid>` when seated on another object, naming the prim the simulator seated the avatar on, `other` when attached to something the simulator did not name, `none` when not seated on an object, `unknown` when not yet learned); `seat name` (the name of the object the avatar is seated on); `persist` (the held placement as `<location> <sit state>` in the forms the `persist` command accepts, with `unknown` in place of a location that is not known yet); and `mode` (`holding` or `manual`). A ground sit, held or commanded, is not a seat on an object and SHALL show as `none`. `region`, `position`, `seat` and `seat name` SHALL be left out while not connected to a region. The `avatar` line SHALL be printed in every state of the bot and for every interface. After the `mode` line the text SHALL have the lines "Money lines of status" gives, for an interface that may pay. `status` SHALL work in every state of the bot. In manual mode the `persist` line shows the placement a `home` command would return to.

The `seat name` line SHALL be printed while `seat` is `target`, `object <uuid>` or `other` and the bot knows the name, and SHALL be left out otherwise. The name SHALL be the one the simulator gives for the root prim of the linkset the avatar is seated on, with a tab, line feed, carriage return or backslash in it written as `\t`, `\n`, `\r` and `\\`. Each time the avatar becomes seated on an object the bot SHALL ask the simulator for that name once, unless it has it from a `scan`, and SHALL forget it when the avatar stands up, leaves the region or the session is lost. `status` SHALL NOT wait for the name.

#### Scenario: Seated on the target
- **WHEN** `rslbot ctl status` is run in holding mode while the avatar is seated on the held object in `Target`
- **THEN** it prints `connected: yes`, an `avatar` line, `region: Target`, a `position` line, `seat: target`, a `persist` line naming a location in `Target` and that object, and `mode: holding`, and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl status` is run while the bot is waiting to log in again
- **THEN** it prints `connected: no`, the `avatar` line, the `persist` line and the `mode` line, and exits with status 0

#### Scenario: Configured ground sit
- **WHEN** `rslbot ctl status` is run in holding mode after the bot has sat the avatar on the ground as the held sit state `ground` asks
- **THEN** it prints `seat: none`, a `persist` line ending in `ground`, and `mode: holding`

#### Scenario: Seated elsewhere in manual mode
- **WHEN** `rslbot ctl status` is run after `rslbot ctl sit <uuid>` seated the avatar on an object that is not the held sit state
- **THEN** it prints `seat: object` with the UUID of the prim the simulator seated the avatar on, the unchanged `persist` line, and `mode: manual`

#### Scenario: Location not known yet
- **WHEN** the config names no `position` and `rslbot ctl status` is run before the first login has succeeded
- **THEN** the `persist` line shows `unknown` as the location

#### Scenario: Name of the seat
- **WHEN** the avatar has been seated on the held object, which is named `Park Bench`, for a few seconds and `rslbot ctl status` is run
- **THEN** it prints `seat: target` and, on the next line, `seat name: Park Bench`

#### Scenario: Seated on a child prim
- **WHEN** the simulator seated the avatar on the child prim `Cushion` of the linkset whose root prim is named `Sofa`
- **THEN** `status` prints `seat: object` with the cushion's UUID and `seat name: Sofa`

#### Scenario: Name not told yet
- **WHEN** `rslbot ctl status` is run at the moment the avatar becomes seated, before the simulator has answered for the name
- **THEN** it is answered at once with the `seat` line and without a `seat name` line

#### Scenario: Not seated on an object
- **WHEN** `rslbot ctl status` is run while the avatar stands or sits on the ground
- **THEN** it prints `seat: none` and no `seat name` line

#### Scenario: Name of the avatar
- **WHEN** the config has `user = "sitbot"` and `rslbot ctl status` is run before the first login has succeeded
- **THEN** the second line it prints is `avatar: sitbot Resident`

## ADDED Requirements

### Requirement: Money lines of status
For an interface that may pay (see `payments`, "Interfaces that may pay"), the text of `status` SHALL end with two lines: `balance`, the avatar's balance as `L$<amount>`, or `unknown` while the grid has not reported one; and `limit`, `unlimited` or what is left of the limit as `L$<amount>`. For an interface that may not pay, and without a `[payments]` table, the text SHALL have neither line.

#### Scenario: Interface that may pay
- **WHEN** `payments.perms` has `ctl`, the avatar has L$1200, `limit = 1000` and nothing is counted
- **THEN** `rslbot ctl status` ends with `balance: L$1200` and `limit: L$1000`

#### Scenario: Interface that may not pay
- **WHEN** `payments.perms` is `["web"]` and `rslbot ctl status` is run
- **THEN** it prints neither a `balance` nor a `limit` line, and the page's status has both

#### Scenario: Before the first login
- **WHEN** `payments.perms` has `ctl`, `limit = "unlimited"`, and `rslbot ctl status` is run before the first login has succeeded
- **THEN** it ends with `balance: unknown` and `limit: unlimited`

### Requirement: Renewal in the limit line
When `limit_renew` is set and part of the limit is counted in the current period, the `limit` line SHALL go on with ` (renews to L$<limit> on <time>)`, where the time is the end of the period in UTC as `YYYY-MM-DDTHH:MM:SSZ`. Without `limit_renew`, and while nothing is counted, the line SHALL have no such part.

#### Scenario: Part of the limit used
- **WHEN** `limit = 1000`, `limit_renew = "7d"`, and L$250 were paid on 2026-12-01 at 12:00:00 UTC
- **THEN** the line is `limit: L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)`

#### Scenario: Nothing used
- **WHEN** `limit = 1000`, `limit_renew = "7d"`, and nothing is counted
- **THEN** the line is `limit: L$1000`

#### Scenario: No renewal configured
- **WHEN** `limit = 1000` without `limit_renew` and L$250 are counted
- **THEN** the line is `limit: L$750`
