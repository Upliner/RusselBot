# Spec Delta

## MODIFIED Requirements

### Requirement: Interfaces that may pay
A command reaches the bot through one of three interfaces: `ctl`, the control socket and the startup script; `xmpp`, the control contact of the XMPP gateway; `web`, the page of the web interface and its console. `pay`, `prices` and `payees` SHALL be carried out only for an interface that `payments.perms` lists, and SHALL otherwise be refused with an error naming the interface. The same SHALL hold for a `group join` and an `accept group` that would pay a fee (see "Fee for joining a group"): without a `[payments]` table, and for an interface that is not listed, they SHALL be refused with that error and SHALL send nothing.

#### Scenario: Interface not listed
- **WHEN** the config has `perms = ["ctl"]` and the operator types `pay 10 avatar "Jane Doe"` into the page's console
- **THEN** the answer is an error saying that payments are not enabled for `web`, and nothing is paid

#### Scenario: Interface listed
- **WHEN** the config has `perms = ["ctl"]` and `rslbot ctl pay 10 avatar "Jane Doe"` is run
- **THEN** Jane Doe is paid L$10

#### Scenario: Startup script
- **WHEN** the config has `perms = ["web"]` and the startup script has the line `pay 10 avatar "Jane Doe"`
- **THEN** the line is refused as one from `rslbot ctl` is, and the script stops there

#### Scenario: Nobody listed
- **WHEN** the config has `perms = []`
- **THEN** `pay` is refused from every interface, as without the table

#### Scenario: Fee from an interface that is not listed
- **WHEN** the config has `perms = ["ctl"]` and an owner sends `group join <uuid> --pay 250` over XMPP for a group that costs L$250 to join
- **THEN** the answer is an error saying that payments are not enabled for `xmpp`, and the avatar neither joins nor pays

### Requirement: Whom a payment goes to
A payment to an avatar goes to that avatar. A payment to an object goes to the object and to its owner. An entry of `whitelist`, `blacklist` or `unlimited_list` SHALL match a payment when it is the UUID of the avatar, of the object, or of the avatar that owns the object, or the login name of the avatar or of that owner. An object that a group owns SHALL be matched by its own UUID only. The fee for joining a group goes to that group: an entry SHALL match it when it is the group's UUID, and no name SHALL match it, neither the group's nor that of the avatar that sent an invitation.

#### Scenario: Object of a listed avatar
- **WHEN** `blacklist` has `Jane Doe` and `pay 10 object <uuid>` names a tip jar Jane Doe owns
- **THEN** the entry matches the payment

#### Scenario: Object by its own UUID
- **WHEN** `whitelist` has the UUID of one vendor of Jane Doe and nothing else
- **THEN** a payment to that vendor is matched, and a payment to Jane Doe or to another of her objects is not

#### Scenario: Group's object
- **WHEN** an object is owned by a group and the lists hold avatars only
- **THEN** no entry matches a payment to it

#### Scenario: Fee of a listed group
- **WHEN** `whitelist` has the UUID of the group `Builders` and nothing else, and `group join` is run with `--pay` for `Builders` and for another group with a fee
- **THEN** the fee of `Builders` is matched and paid, and the other join is refused as not on the whitelist

#### Scenario: Inviter on the unlimited list
- **WHEN** `unlimited_list` has `Jane Doe` and Jane Doe has invited the avatar into a group with a fee
- **THEN** no entry matches the fee, and it counts against the limit

## ADDED Requirements

### Requirement: Fee for joining a group
The fee the avatar pays when it joins a group that costs L$, by `group join` (see `groups`, "Joining a group with a fee") or by `accept group` (see `permission-requests`, "Paying the fee of an invitation"), SHALL be a payment of that amount to that group, and everything this capability says of a payment SHALL hold for it: the interface (see "Interfaces that may pay"), the lists (see "Whom a payment goes to", "Blacklist", "Whitelist"), the limit and its period, the balance (see "Paying more than the avatar has"), and "The limit needs its record". These SHALL be checked once the fee is known and immediately before the join or the acceptance is sent, and when one of them refuses, nothing SHALL be sent to the grid: the avatar SHALL NOT join. While a payment or a fee waits for the grid's answer, a further fee SHALL be refused as "One payment at a time" says, and so SHALL a `pay` while a fee waits. The grid, not the bot, takes the fee: the bot sends the join or the acceptance and no payment of its own, so that a fee is never paid twice. A join or an acceptance without a fee is no payment and SHALL NOT be held to this capability.

#### Scenario: Within the limit
- **WHEN** `limit = 1000`, L$700 are counted in the period, and `rslbot ctl group join <uuid> --pay 250` is run for a group that costs L$250
- **THEN** the join is sent and L$50 are left

#### Scenario: Over the limit
- **WHEN** `limit = 1000`, L$900 are counted in the period, and `rslbot ctl group join <uuid> --pay 250` is run for a group that costs L$250
- **THEN** the command exits with status 1 saying that L$100 are left, and the avatar does not join

#### Scenario: Blacklisted group
- **WHEN** `blacklist` has the UUID of `Builders` and `rslbot ctl accept group Builders --pay 250` is run for an invitation with a fee of L$250
- **THEN** the command exits with status 1 saying that the group is on the blacklist, nothing is sent, and the invitation still waits

#### Scenario: Not enough money
- **WHEN** the avatar has L$100 and `rslbot ctl group join <uuid> --pay 250` is run for a group that costs L$250
- **THEN** the command exits with status 1 saying that the avatar has L$100, and nothing is sent

#### Scenario: While a payment waits
- **WHEN** `pay 10 avatar "Jane Doe"` has been sent and is not yet answered, and `group join <uuid> --pay 250` arrives for a group with a fee
- **THEN** the join is refused at once with an error saying that a payment is under way

#### Scenario: Free group without the table
- **WHEN** the config has no `[payments]` table and `rslbot ctl group join <uuid>` names an open group without a fee
- **THEN** the avatar joins

### Requirement: Outcome of a fee
A fee SHALL count against the limit, when no entry of `unlimited_list` matches it, from the moment the join or the acceptance is handed to the simulator connection. The bot SHALL then wait up to 15 seconds for two things: the grid's word on whether the avatar is in the group, which answers the command, and the grid's report of the fee, a transaction it reports with the avatar as payer and the fee as amount, which settles what became of the fee:

| Within the 15 seconds the grid | The fee | The command is answered |
|---|---|---|
| reports the fee as made, and says that the avatar is in the group | is made and keeps counting | with success, naming the group and the fee, and the new balance when the report came first |
| says that the avatar is in the group and reports no fee | is not confirmed and keeps counting | with success when the grid says so, naming the group and the fee and saying that the grid has not reported the fee and that it counts as paid |
| reports the fee as made and does not say that the avatar is in the group | is made and keeps counting | with an error when the time is up, saying that the fee was paid and that the grid has not confirmed the join |
| refuses the join, or reports the fee as failed | failed, and stops counting | with an error holding the grid's reason |
| says nothing, or the session is lost first | is not confirmed and keeps counting | with an error saying that the join was sent, that the grid has not confirmed it, and that the fee counts as paid |

A command that is answered before the report has come SHALL NOT end the wait for the report: until it comes or the 15 seconds are over, a further payment is refused as "One payment at a time" says.

#### Scenario: Reported and joined
- **WHEN** `rslbot ctl group join <uuid> --pay 250` is run for `Builders`, the grid says that the avatar has joined and reports that the avatar paid L$250, leaving L$750
- **THEN** the command exits with status 0 with a line naming `Builders`, `L$250` and `L$750`

#### Scenario: Grid refuses the join
- **WHEN** a join with a fee of L$250 is sent and the grid answers that the avatar has too little money
- **THEN** the command exits with status 1 with the grid's reason, and the L$250 do not count against the limit

#### Scenario: Grid that reports no fee
- **WHEN** a join with a fee of L$250 is sent on a grid that makes the avatar a member and reports nothing about money
- **THEN** the command exits with status 0 when the avatar is a member, saying that the fee was not reported and counts as paid, and L$250 less are left of the limit

#### Scenario: Nothing heard
- **WHEN** an acceptance with a fee of L$250 is sent and the grid says nothing for 15 seconds
- **THEN** the command exits with status 1 saying that it was sent and is not confirmed, and the L$250 keep counting
