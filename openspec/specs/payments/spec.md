# payments Specification

## Purpose

Lets the operator have the avatar pay avatars and objects in L$, through the interfaces the config allows and within a spending limit and lists of who may be paid, and tells the operator the avatar's balance and what is left of the limit.

## Requirements

### Requirement: Payments are off without the payments table
Without a `[payments]` table in the config (see `bot-config`) the bot SHALL refuse `pay`, `prices` and `payees` from every interface with an error saying that payments are not enabled, and SHALL send the grid nothing for them.

#### Scenario: No table
- **WHEN** the config has no `[payments]` table and `rslbot ctl pay 10 avatar "Jane Doe"` is run
- **THEN** the command exits with status 1 with an error naming `[payments]`, and nothing is sent to the grid

### Requirement: Interfaces that may pay
A command reaches the bot through one of three interfaces: `ctl`, the control socket and the startup script; `xmpp`, the control contact of the XMPP gateway; `web`, the page of the web interface and its console. `pay`, `prices` and `payees` SHALL be carried out only for an interface that `payments.perms` lists, and SHALL otherwise be refused with an error naming the interface.

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

### Requirement: Forms of the pay command
`pay` SHALL take `<amount> avatar <avatar>`, where the words of the avatar's name may be separate arguments, optionally followed by `-d` or `--desc` and a description (see "Description of a payment to an avatar"), or `<amount> object <uuid>`. Any other form, `-d` with `object` included, SHALL be refused with an error that shows both forms, and nothing SHALL be sent.

#### Scenario: Name in two words
- **WHEN** `rslbot ctl pay 10 avatar Jane Doe` is run
- **THEN** it is read as a payment of L$10 to the avatar `Jane Doe`

#### Scenario: Name in two words and a description
- **WHEN** `rslbot ctl pay 10 avatar Jane Doe -d for the hat` is run
- **THEN** it is read as a payment of L$10 to the avatar `Jane Doe` with the description `for the hat`

#### Scenario: Description for an object
- **WHEN** `rslbot ctl pay 10 object <uuid> -d thanks` is run
- **THEN** the command exits with status 1 with an error showing the forms, and nothing is paid

#### Scenario: No kind of payee
- **WHEN** `rslbot ctl pay 10 "Jane Doe"` or `rslbot ctl pay 10` is run
- **THEN** the command exits with status 1 with an error showing `pay AMOUNT avatar AVATAR` and `pay AMOUNT object UUID`

#### Scenario: Object that is no UUID
- **WHEN** `rslbot ctl pay 10 object "Tip Jar"` is run
- **THEN** the command exits with status 1 saying that an object is given by its UUID

### Requirement: Amounts
An amount SHALL be a whole number of L$ from 1 to 2147483647, written in digits only. `pay` with any other amount SHALL be refused with an error naming the amount, and nothing SHALL be sent.

#### Scenario: Not a whole positive number
- **WHEN** `pay` is given the amount `0`, `-5`, `10.5`, `L$10`, `ten` or `3000000000`
- **THEN** it is refused with an error naming the amount, and nothing is paid

### Requirement: Paying needs the avatar in a region
`pay`, `prices` and `payees` SHALL be answered with an error and send nothing when the avatar is not connected to a region or is in the middle of a teleport or a logout.

#### Scenario: Between logins
- **WHEN** `rslbot ctl pay 10 avatar "Jane Doe"` is run while the bot is waiting to log in again
- **THEN** the command exits with status 1 saying that the avatar is not connected, and nothing is paid later

### Requirement: Paying an avatar
`pay <amount> avatar <avatar>` SHALL take the avatar as `im` takes its recipient (see `message-send`): a UUID, or a login name that the bot resolves on the grid, where only an exact match of the full name counts. It SHALL pay that avatar the amount as a viewer's Pay on an avatar does, without a message.

#### Scenario: By name
- **WHEN** `rslbot ctl pay 25 avatar jane.doe` is run and the grid knows Jane Doe
- **THEN** Jane Doe receives L$25 from the bot's avatar

#### Scenario: By UUID
- **WHEN** `rslbot ctl pay 25 avatar <uuid>` is run with Jane Doe's UUID
- **THEN** Jane Doe receives L$25, and the answer names her

#### Scenario: Unknown name
- **WHEN** `rslbot ctl pay 25 avatar "Nobody Here"` is run and the grid has no such avatar
- **THEN** the command exits with status 1 naming `Nobody Here`, and nothing is paid

### Requirement: Description of a payment to an avatar
A description SHALL be the words after `-d` or `--desc`, joined by single spaces, and SHALL be sent with the payment as a viewer's Pay Resident dialog sends one. A description that is empty, is longer than 127 bytes or holds a line break SHALL be refused with an error naming the limit, and nothing SHALL be sent. When, at the moment the payment is sent, the RLV restrictions in force would refuse an `im` to the payee (see `rlv-restrictions`), the payment SHALL be sent without the description, and the answer, whatever it is, SHALL begin with the line `warning: Payment description is blocked by RLV`.

#### Scenario: With a description
- **WHEN** `rslbot ctl pay 25 avatar jane.doe -d for the hat` is run
- **THEN** Jane Doe receives L$25 with the description `for the hat`

#### Scenario: Too long
- **WHEN** the description is 128 bytes long
- **THEN** the command exits with status 1 naming 127 bytes, and nothing is paid

#### Scenario: Blocked by RLV
- **WHEN** an object has said `@sendim=n` to its owner, RLV is on, and `rslbot ctl pay 25 avatar jane.doe -d for the hat` is run
- **THEN** Jane Doe receives L$25 without a description, and the answer begins with `warning: Payment description is blocked by RLV`

### Requirement: Payees that cannot be paid as avatars
`pay <amount> avatar` SHALL be refused, and nothing SHALL be sent, when the avatar is the bot's own, when the grid knows no avatar by the UUID given, and when the grid has not said within 5 seconds whether it knows one.

#### Scenario: The avatar itself
- **WHEN** `pay 10 avatar` is run with the name or the UUID of the bot's own avatar
- **THEN** it is refused with an error saying that the avatar cannot pay itself

#### Scenario: UUID of an object
- **WHEN** `pay 10 avatar <uuid>` is run with the UUID of an object
- **THEN** it is refused with an error saying that the grid knows no avatar by that UUID, and that an object is paid with `pay AMOUNT object UUID`

### Requirement: Paying an object
`pay <amount> object <uuid>` SHALL pay the object with that UUID in the avatar's region as a viewer's Pay on that object does, so that the object's script is told of the payment by the bot's avatar. The bot SHALL learn of the object as `touch` does (see `avatar-commands`, "Touch command"), the scan and its wait of up to 10 seconds included, and SHALL refuse to pay an object it has not learned of.

#### Scenario: Tip jar
- **WHEN** `rslbot ctl pay 10 object <uuid>` names a nearby tip jar whose script takes any amount
- **THEN** the script's money event fires once with the bot's avatar as the payer and 10 as the amount

#### Scenario: Object out of range or unknown
- **WHEN** `rslbot ctl pay 10 object <uuid>` names an object outside the scan distance or one that does not exist
- **THEN** the command exits with status 1 saying the object is not known and naming the scan distance, and nothing is paid

### Requirement: Only objects that take money
The bot SHALL refuse `pay <amount> object` and `prices`, and send no payment, for an object that the simulator marks as taking money neither in the prim itself nor in the root prim of its linkset.

#### Scenario: Plain object
- **WHEN** `rslbot ctl pay 10 object <uuid>` names a bench without a script
- **THEN** the command exits with status 1 saying that the object does not take money

#### Scenario: Child prim of a vendor
- **WHEN** `rslbot ctl pay 10 object <uuid>` names a child prim of a linkset whose root prim takes money
- **THEN** the payment is sent for that prim

### Requirement: Amounts an object offers
An object offers up to four quick-pay amounts and says whether any other amount may be paid, with or without a suggested one. The bot SHALL read them as a viewer shows them: a quick-pay amount the object hides is left out; one it leaves at the default is 1, 5, 10 or 20, by its place among the four; other amounts may be paid unless the object hides the amount field.

#### Scenario: Vendor with two prices
- **WHEN** an object's script sets no amount field and the quick-pay amounts 100 and 250, hiding the other two
- **THEN** the object offers 100 and 250 and no other amount

#### Scenario: Object that sets nothing
- **WHEN** an object takes money and its script sets no pay prices
- **THEN** it offers 1, 5, 10 and 20 and any other amount

### Requirement: Asking an object for what it offers
Before it pays an object or answers `prices`, the bot SHALL ask the simulator for the amounts the object offers and for its name and owner, and SHALL wait up to 10 seconds for them. When they have not come by then, or the avatar leaves the region or the session is lost first, the command SHALL be answered with an error, and nothing SHALL be paid.

#### Scenario: Simulator does not answer
- **WHEN** `rslbot ctl pay 10 object <uuid>` is run and the simulator does not say what the object offers
- **THEN** the command exits with status 1 after 10 seconds, and nothing is paid

### Requirement: Only an amount the object offers
`pay <amount> object` SHALL be refused, and nothing SHALL be sent, when the amount is none of the object's quick-pay amounts and the object does not take other amounts. The error SHALL list the amounts the object offers.

#### Scenario: Wrong price
- **WHEN** a vendor offers 100 and 250 and no other amount, and `rslbot ctl pay 120 object <uuid>` is run
- **THEN** the command exits with status 1 naming 100 and 250, and nothing is paid

#### Scenario: Any amount
- **WHEN** a tip jar offers 1, 5, 10 and 20 and any other amount, and `rslbot ctl pay 37 object <uuid>` is run
- **THEN** L$37 is paid

### Requirement: Prices command
`prices <uuid>` SHALL be answered with success and four lines: `object: ` and the name of the object's linkset; `owner: ` and the owner's login name, its UUID when the grid does not supply the name within 5 seconds, or `group` and the group's UUID; `prices: ` and the quick-pay amounts, separated by a comma and a space, or `none`; `other amounts: ` and `yes`, `yes, suggested <amount>` or `no`. It SHALL pay nothing.

#### Scenario: Vendor
- **WHEN** `rslbot ctl prices <uuid>` names the vendor `Hat Vendor` of Jane Doe, which offers 100 and 250 only
- **THEN** it prints `object: Hat Vendor`, `owner: Jane Doe`, `prices: 100, 250` and `other amounts: no`, and exits with status 0

#### Scenario: Suggested amount
- **WHEN** an object's script hides all four quick-pay amounts and suggests 50
- **THEN** `prices` prints `prices: none` and `other amounts: yes, suggested 50`

#### Scenario: Name with a tab
- **WHEN** the object's name holds a tab or a line feed
- **THEN** the `object` line has `\t` or `\n` in its place and stays one line

### Requirement: Whom a payment goes to
A payment to an avatar goes to that avatar. A payment to an object goes to the object and to its owner. An entry of `whitelist`, `blacklist` or `unlimited_list` SHALL match a payment when it is the UUID of the avatar, of the object, or of the avatar that owns the object, or the login name of the avatar or of that owner. An object that a group owns SHALL be matched by its own UUID only.

#### Scenario: Object of a listed avatar
- **WHEN** `blacklist` has `Jane Doe` and `pay 10 object <uuid>` names a tip jar Jane Doe owns
- **THEN** the entry matches the payment

#### Scenario: Object by its own UUID
- **WHEN** `whitelist` has the UUID of one vendor of Jane Doe and nothing else
- **THEN** a payment to that vendor is matched, and a payment to Jane Doe or to another of her objects is not

#### Scenario: Group's object
- **WHEN** an object is owned by a group and the lists hold avatars only
- **THEN** no entry matches a payment to it

### Requirement: Login names, not display names
A name on a list SHALL be compared, without regard to letter case, with the name the grid gives for the avatar's account, `First Last`, and SHALL NOT be compared with a display name.

#### Scenario: Display name set to a listed name
- **WHEN** `whitelist` has `Jane Doe`, and the avatar `John Roe` has set its display name to `Jane Doe`
- **THEN** a payment to John Roe is not matched by that entry

#### Scenario: Spellings of one name
- **WHEN** `whitelist` has `jane.doe` and the payee's account is `Jane Doe`
- **THEN** the entry matches

### Requirement: Blacklist
A payment that an entry of `blacklist` matches SHALL be refused with an error saying so, whatever the other lists and the limit say, and nothing SHALL be sent.

#### Scenario: Blacklisted avatar
- **WHEN** `blacklist` has `John Roe` and `rslbot ctl pay 1 avatar "John Roe"` is run
- **THEN** the command exits with status 1 saying that John Roe is on the blacklist, and nothing is paid

### Requirement: Whitelist
When the `[payments]` table has a `whitelist`, a payment that neither an entry of `whitelist` nor an entry of `unlimited_list` matches SHALL be refused with an error saying so, and nothing SHALL be sent. Without the key, every payment the blacklist does not refuse SHALL be allowed.

#### Scenario: Not on the whitelist
- **WHEN** `whitelist` has `Jane Doe` only and `rslbot ctl pay 1 avatar "John Roe"` is run
- **THEN** the command exits with status 1 saying that John Roe is not on the whitelist

#### Scenario: Empty whitelist
- **WHEN** the table has `whitelist = []` and `unlimited_list = ["Jane Doe"]`
- **THEN** Jane Doe can be paid and nobody else

#### Scenario: No whitelist
- **WHEN** the table has no `whitelist` and no `blacklist`
- **THEN** any avatar and any object that takes money can be paid, within the limit

### Requirement: Names the lists need
When a list has a name among its entries and the bot cannot learn, within 5 seconds, the login name of the avatar a payment goes to or of the avatar that owns the object, the payment SHALL be refused with an error saying that the lists could not be checked, and nothing SHALL be sent.

#### Scenario: Grid does not name the owner
- **WHEN** `blacklist` has `John Roe`, and the grid does not answer for the name of the owner of the object in `pay 10 object <uuid>`
- **THEN** the command exits with status 1 after 5 seconds, and nothing is paid

### Requirement: Limit
A payment that no entry of `unlimited_list` matches counts against the limit. While `limit` is a number, the bot SHALL refuse a payment that counts and whose amount is more than what is left: the limit less the amounts of the payments that count in the current period. The error SHALL name what is left. While `limit` is `"unlimited"` no payment SHALL be refused for the limit.

#### Scenario: Within the limit
- **WHEN** `limit = 1000`, L$900 are counted in the period, and `pay 100 avatar "Jane Doe"` is run
- **THEN** the payment is sent, and nothing is left

#### Scenario: Over the limit
- **WHEN** `limit = 1000`, L$900 are counted in the period, and `pay 101 avatar "Jane Doe"` is run
- **THEN** the command exits with status 1 saying that L$100 are left, and nothing is paid

#### Scenario: Limit of zero
- **WHEN** `limit = 0` and `unlimited_list = ["Jane Doe"]`
- **THEN** Jane Doe can be paid and a payment to anyone else is refused for the limit

### Requirement: Period of the limit
With `limit_renew`, a period SHALL start at the moment of the first payment that counts while no period runs, and SHALL end `limit_renew` after it started: from then on the whole limit is left again, and the next payment that counts starts a new period. Without `limit_renew` there is one period, which never ends. An error for the limit SHALL name the time the limit renews, when it does.

#### Scenario: Renewal
- **WHEN** `limit = 1000` and `limit_renew = "7d"`, and L$1000 were paid on 1 December at 12:00 UTC
- **THEN** a payment on 8 December at 11:59 UTC is refused with an error naming 8 December 12:00 UTC, and one at 12:00 UTC is sent

#### Scenario: Later payments do not move the renewal
- **WHEN** a period started on 1 December at 12:00 UTC with `limit_renew = "7d"`, and a further payment is made on 5 December
- **THEN** the limit still renews on 8 December at 12:00 UTC

#### Scenario: No renewal
- **WHEN** `limit = 1000` without `limit_renew`, and L$1000 were paid a year ago
- **THEN** a payment that counts is refused

### Requirement: Unlimited list
A payment that an entry of `unlimited_list` matches SHALL NOT count against the limit and SHALL NOT be refused for it, also when nothing is left. It SHALL NOT start a period.

#### Scenario: Limit used up
- **WHEN** `limit = 100`, L$100 are counted, and `unlimited_list` has `Jane Doe`
- **THEN** `pay 5000 avatar "Jane Doe"` is sent, and what is left stays L$0

### Requirement: When a payment counts
A payment SHALL count against the limit from the moment it is handed to the simulator connection, and SHALL stop counting only when the grid reports that it failed. A payment the grid has neither confirmed nor refused SHALL keep counting. A payment the bot refused SHALL NOT count.

#### Scenario: Grid refuses
- **WHEN** `limit = 1000`, `pay 400 avatar "Jane Doe"` is sent, and the grid reports that it failed
- **THEN** L$1000 are left again

#### Scenario: Grid says nothing
- **WHEN** `limit = 1000` and `pay 400 avatar "Jane Doe"` is sent and never confirmed
- **THEN** L$600 are left

### Requirement: The limit outlasts the bot
The bot SHALL start with the period and the counted amounts that the transaction log holds (see `transaction-log`, "Reading the file back"), under the `limit` and `limit_renew` of the config it starts with. What is left SHALL NOT be less than zero when the limit was lowered.

#### Scenario: Restart
- **WHEN** `limit = 1000`, L$900 were paid an hour ago, and the bot is stopped and started again
- **THEN** `status` shows L$100 left, and `pay 101 avatar "Jane Doe"` is refused

#### Scenario: Limit lowered
- **WHEN** L$900 are counted in the period and the bot is started with `limit = 500`
- **THEN** `status` shows L$0 left

#### Scenario: Period over while the bot was down
- **WHEN** `limit_renew = "12h"`, the last period started a day ago, and the bot is started
- **THEN** the whole limit is left

### Requirement: The limit needs its record
While `limit` is a number and the row of a payment that counts could not be written to the transaction log, the bot SHALL refuse every further payment that counts, with an error naming the file, until that row is written; it SHALL try to write it again at each `pay`.

#### Scenario: Disk full
- **WHEN** the row of a payment of L$100 could not be written because the disk is full, and `pay 10 avatar "Jane Doe"` is run while it still is
- **THEN** the command exits with status 1 naming the transactions file, and nothing is paid

#### Scenario: Disk has room again
- **WHEN** the disk has room again and `pay 10 avatar "Jane Doe"` is run
- **THEN** the row of the earlier payment is written, and the payment is sent

### Requirement: Paying more than the avatar has
While the bot knows the avatar's balance (see "Balance"), a `pay` whose amount is more than that balance SHALL be refused with an error naming the balance, as a viewer refuses it, and nothing SHALL be sent. While the balance is unknown the payment SHALL be sent and the grid decides.

#### Scenario: Not enough money
- **WHEN** the avatar has L$5 and `rslbot ctl pay 25 avatar "Jane Doe"` is run
- **THEN** the command exits with status 1 saying that the avatar has L$5, nothing is sent, and nothing counts against the limit

### Requirement: One payment at a time
While a payment waits for the grid's answer, the bot SHALL refuse a further `pay`, from any interface, with an error saying that a payment is under way, and SHALL send nothing for it.

#### Scenario: Two at once
- **WHEN** `pay 10 avatar "Jane Doe"` has been sent and is not yet answered, and `pay 10 avatar "John Roe"` arrives
- **THEN** the second is refused at once, and John Roe is not paid

### Requirement: Outcome of a payment
The bot SHALL answer a `pay` it has sent when the grid reports the payment: with success and a text naming the amount, the payee and the new balance when it was made, and with an error holding the grid's reason when it failed.

#### Scenario: Paid
- **WHEN** `rslbot ctl pay 25 avatar "Jane Doe"` is run and the grid confirms the payment, leaving L$975
- **THEN** it prints a line with `L$25`, `Jane Doe` and `L$975`, and exits with status 0

#### Scenario: Grid refuses
- **WHEN** `rslbot ctl pay 25 avatar "Jane Doe"` is sent and the grid reports that the payment failed
- **THEN** the command exits with status 1 with the grid's reason, and the L$25 do not count against the limit

#### Scenario: Object
- **WHEN** `rslbot ctl pay 100 object <uuid>` pays the vendor `Hat Vendor` of Jane Doe
- **THEN** the answer names `L$100`, `Hat Vendor`, Jane Doe as its owner and the new balance

### Requirement: Payments the grid does not confirm
When the grid has reported nothing about a payment 15 seconds after it was sent, or the session is lost before, the bot SHALL answer the `pay` with an error saying that the payment was sent, that the grid has not confirmed it, and that it counts as paid.

#### Scenario: No report
- **WHEN** `rslbot ctl pay 25 avatar "Jane Doe"` is run on a grid that reports nothing about payments
- **THEN** the command exits with status 1 after 15 seconds, saying that the payment was sent and is not confirmed

#### Scenario: Kicked while waiting
- **WHEN** the avatar is logged off the grid two seconds after a payment was sent
- **THEN** the `pay` is answered at once with the same error, and the bot logs in again as it does after any kick

### Requirement: Payees command
`payees` SHALL be answered with success and one line for each entry of `whitelist` and `unlimited_list` that is an avatar: a name entry, and a UUID entry when the grid knows an avatar by that UUID. A line SHALL have three fields separated by tabs: the UUID, or `?` for a name entry; the login name; and `whitelist`, `unlimited` or `whitelist,unlimited`. It SHALL take no arguments, and SHALL have no text when there is no such entry.

#### Scenario: Names and UUIDs
- **WHEN** `whitelist` has `Jane Doe`, the UUID of John Roe and the UUID of a vendor, and `unlimited_list` has `jane.doe`
- **THEN** `payees` prints a line `?`, `Jane Doe`, `whitelist,unlimited` and a line with John Roe's UUID, `John Roe`, `whitelist`, and none for the vendor

#### Scenario: No lists
- **WHEN** the table has neither `whitelist` nor `unlimited_list`
- **THEN** `payees` is answered with success and no text

### Requirement: Balance
When the config has a `[payments]` table the bot SHALL ask the grid for the avatar's balance once after each login. The balance the bot shows SHALL be the newest one the grid has reported, alone or with a transaction, and SHALL be kept while the avatar is not connected. Without the table the bot SHALL NOT ask for the balance.

#### Scenario: After login
- **WHEN** the avatar has L$1200 and the bot has logged in with a `[payments]` table whose `perms` has `ctl`
- **THEN** `rslbot ctl status` prints `balance: L$1200`

#### Scenario: Payment received
- **WHEN** Jane Doe pays the avatar L$50 while it has L$1200
- **THEN** `status` prints `balance: L$1250` without the bot having asked again

#### Scenario: No table
- **WHEN** the config has no `[payments]` table
- **THEN** the bot sends no request for the balance

### Requirement: Paying leaves the placement and RLV alone
`pay`, `prices` and `payees` SHALL NOT move the avatar and SHALL NOT change the bot's placement mode or the held placement. No RLV restriction SHALL be applied to them, but for the description of a payment (see "Description of a payment to an avatar"), and none of them SHALL be the exempt command of an RLV override (see `rlv-restrictions`, "Override exempts one command").

#### Scenario: Seated
- **WHEN** `rslbot ctl pay 10 object <uuid>` is run while the avatar is seated on the held object
- **THEN** the avatar stays seated and `status` still prints `mode: holding`

#### Scenario: During an override
- **WHEN** `rslbot ctl rlv mode override` has been carried out and `rslbot ctl prices <uuid>` is run
- **THEN** the prices are printed and `rlv status` still prints `rlv: override`

### Requirement: Payments appear in the operator log
The bot SHALL write a line to its operator log for every `pay` command: the interface it came through, the amount, the payee by name and UUID, the description when one was given and whether RLV held it back, and what became of it: refused and why, sent, made, failed with the grid's reason, or not confirmed.

#### Scenario: Refused for the limit
- **WHEN** the page's console sends `pay 500 avatar "Jane Doe"` and L$100 are left
- **THEN** the log has a line with `web`, `L$500`, Jane Doe and the limit as the reason

#### Scenario: Made
- **WHEN** `rslbot ctl pay 25 avatar "Jane Doe"` is confirmed by the grid
- **THEN** the log has a line saying that it was sent and a line saying that it was made, both with `ctl`, `L$25` and Jane Doe's UUID

### Requirement: Payments at startup
When the config has a `[payments]` table the bot SHALL write a line to standard error at startup that names the interfaces that may pay, the limit, what is left of it and, when a renewal is due, its time. When `web` may pay and the `[web]` table has no `password`, it SHALL also write a warning that whoever reaches the web interface can pay.

#### Scenario: Start line
- **WHEN** the bot starts with `limit = 1000`, `limit_renew = "7d"`, `perms = ["ctl", "web"]` and L$250 counted since yesterday noon
- **THEN** standard error has a line naming `ctl` and `web`, L$1000, L$750 left and the renewal six days from now

#### Scenario: Web without a password
- **WHEN** `perms` has `web` and the `[web]` table has no `password`
- **THEN** standard error has a warning naming `web.password`
