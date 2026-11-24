# Spec Delta

## MODIFIED Requirements

### Requirement: Commands from the page
A command the page sends, as a command name and its arguments, SHALL be carried out as the control socket carries out the same command (see `control-socket` and the capabilities its commands belong to), with the same answers and the same effects, but as a command of the interface `web` (see `payments`, "Interfaces that may pay"): `payments.perms` says for `web` on its own whether `pay`, `prices` and `payees` are carried out and whether `status` has its money lines. This holds for a console line as well. The command SHALL be answered to the page with the command's answer: success with its text, if it has one, or an error with its text.

#### Scenario: Status
- **WHEN** the page sends `status`
- **THEN** it gets the text `rslbot ctl status` prints, with the `balance` and `limit` lines when `payments.perms` has `web` and without them when it does not

#### Scenario: Command refused
- **WHEN** the page sends `sit ground` while the bot is between logins
- **THEN** it gets the error the control socket would give

#### Scenario: Argument with spaces and quotes
- **WHEN** the page sends `tp` with the arguments `128`, `128`, `25` and `Joe's "Blue" Lagoon`
- **THEN** the bot teleports the avatar to the region of exactly that name

#### Scenario: Quit
- **WHEN** the page sends `quit`
- **THEN** it gets the success answer, and the bot logs out and exits

#### Scenario: Paying from the console
- **WHEN** `payments.perms` is `["ctl"]` and the console line is `pay 10 avatar "Jane Doe"`
- **THEN** the answer is an error saying that payments are not enabled for `web`, and nothing is paid

### Requirement: Status on the event stream
On every open event stream the bot SHALL send the text of `status` as the interface `web` gets it (see `avatar-commands`, "Status command" and "Money lines of status") when the stream opens and whenever that text changes, within one second of the change.

#### Scenario: Teleport
- **WHEN** the avatar arrives in another region, by a command from anywhere or by the bot's own doing
- **THEN** every open page gets the new status within a second

#### Scenario: Balance changes
- **WHEN** `payments.perms` has `web` and Jane Doe pays the avatar L$50
- **THEN** every open page gets the status with the new `balance` line within a second

### Requirement: Requests the page makes by itself
The page SHALL mark as its own the commands it sends to keep a list up to date without the operator having asked for that list then: the `scan avatars` for the avatars in chat range and the `friends` for the friend list that it sends when their tab is shown, when it has found the bot again, and on a timer, and the `payees` it sends for the Money tab (see `web-ui`). Only `scan avatars`, `friends` and `payees` SHALL be accepted as the page's own; any other command marked so SHALL be refused and not carried out.

#### Scenario: Something else marked as the page's own
- **WHEN** a client sends `say hello` marked as the page's own
- **THEN** it is refused and nothing is said

## ADDED Requirements

### Requirement: Transactions on the event stream
While `payments.perms` has `web`, the bot SHALL send every open page every transaction the transaction log records (see `transaction-log`), as soon as it is recorded and in the order of recording, with all the fields of its record. While it does not, the bot SHALL send the pages no transaction.

#### Scenario: Payment received
- **WHEN** `payments.perms` has `web` and Jane Doe pays the avatar L$50 while a page is open
- **THEN** the page has the transaction within a second of its being recorded, with Jane Doe's UUID and names, the amount and the new balance

#### Scenario: Payment from elsewhere
- **WHEN** `rslbot ctl pay 25 avatar "Jane Doe"` is confirmed while a page is open
- **THEN** the page gets the `paid` record

#### Scenario: Web may not pay
- **WHEN** `payments.perms` is `["ctl"]` and a payment is received while a page is open
- **THEN** the page gets no transaction

### Requirement: Transactions are kept for pages that open later
While `payments.perms` has `web` and the config has a `[web]` table, the bot SHALL keep the newest 500 transactions, counted apart from chat and instant messages, starting with the rows it read from the transactions file (see `transaction-log`, "Reading the file back"). A page SHALL get them when it opens its stream and when it catches up, under the rules for kept messages. Otherwise the bot SHALL keep none.

#### Scenario: Page opened after a restart
- **WHEN** the transactions file has 40 rows, the bot is restarted, and a page is opened
- **THEN** the page gets those 40 transactions, oldest first, with the times of their rows

#### Scenario: No transactions file
- **WHEN** the config has no `transactions` under `[logs]` and three payments were received since the bot started
- **THEN** a page that opens gets those three, and after a restart none

#### Scenario: Busy chat
- **WHEN** 2000 chat lines and 3 transactions have been recorded since the bot started
- **THEN** a page that opens gets all 3 transactions
