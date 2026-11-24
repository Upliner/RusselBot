# transaction-log Specification

## Purpose

Records the avatar's L$ transactions, the payments the bot sends and the transactions the grid reports to the avatar, on standard error and in an optional TSV file, and reads that file back at startup so that the spending limit and the list of transactions outlast the bot.

## Requirements

### Requirement: Transactions the grid reports
While connected, the bot SHALL record every report of the grid about the avatar's money that names a transaction: one that carries a payer or a payee, and an amount greater than zero. A report that carries a balance and no transaction SHALL NOT be recorded. Transactions SHALL be recorded with and without a `[payments]` table.

#### Scenario: Payment received
- **WHEN** Jane Doe pays the bot's avatar L$50
- **THEN** one transaction of kind `received` with Jane Doe's name and the amount 50 is recorded

#### Scenario: Balance only
- **WHEN** the grid answers the bot's request for the balance
- **THEN** nothing is recorded

#### Scenario: Payments switched off
- **WHEN** the config has no `[payments]` table and Jane Doe pays the avatar L$50
- **THEN** the transaction is recorded all the same

### Requirement: Transactions that happened offline
The bot SHALL NOT ask the grid, or any web site of it, for transactions it was not told of: the grid has no such request for a viewer. A transaction made while the avatar was not connected SHALL show in the balance only.

#### Scenario: Paid while offline
- **WHEN** Jane Doe pays the avatar L$50 while the bot is between logins
- **THEN** no transaction is recorded, and after the login `status` shows the balance with the L$50

### Requirement: A payment of the bot is one record
Every payment the bot sends (see `payments`) SHALL be recorded as one transaction, at the moment its outcome is known: confirmed, failed, or not confirmed. The grid's report that confirms or refuses it SHALL be merged into that record and SHALL NOT make a second one.

#### Scenario: Confirmed payment
- **WHEN** `rslbot ctl pay 25 avatar "Jane Doe"` is run and the grid confirms it
- **THEN** exactly one transaction is recorded, of kind `paid`, with the balance the grid reported

#### Scenario: Refused before it was sent
- **WHEN** a `pay` command is refused for the limit
- **THEN** no transaction is recorded

### Requirement: Kinds of transaction
Each record SHALL have one of these kinds:

| Kind | Meaning |
|---|---|
| `paid` | The bot's payment, confirmed by the grid |
| `unconfirmed` | The bot's payment, neither confirmed nor refused in time |
| `failed` | A transaction the grid reports as failed |
| `received` | A reported transaction with the avatar as payee |
| `spent` | A reported one with the avatar as payer that is no payment of the bot |

#### Scenario: Fee
- **WHEN** the grid reports that the avatar paid L$10 for an upload that a viewer made on the same account
- **THEN** a transaction of kind `spent` is recorded

#### Scenario: Confirmation that comes late
- **WHEN** a payment was recorded as `unconfirmed` and the grid confirms it a minute later
- **THEN** the report is recorded as `spent`, and the amount is not counted against the limit a second time

### Requirement: Money fields of a record
Each record SHALL carry: `time`, when the bot learned the outcome; `amount`, in whole L$; `balance`, the avatar's balance as the grid reported it with the transaction, empty when it reported none; `counted`, how much of the amount counts against the limit (see `payments`, "When a payment counts"), 0 for all but a `paid` or `unconfirmed` payment that counts; `type`, the grid's number for the kind of transaction; and `text`, the description the grid gave. For a gift between avatars (type 5001) that carries a description of the payer's, that is an item description other than the simulator's word `Payment`, `text` SHALL be as a viewer shows the payment: the grid's description without its final full stop, then `: ` and the payment's description. For the bot's own payment whose report carries no such description, and for an `unconfirmed` one, the description the bot sent SHALL take its place.

#### Scenario: Payment with a description
- **WHEN** the bot pays Jane Doe L$3 with the description `test payment` and the grid reports `You paid Jane Doe L$3.`
- **THEN** the record's text is `You paid Jane Doe L$3: test payment`

#### Scenario: Payment received with a description
- **WHEN** Jane Doe pays the avatar L$5 with the description `for the hat`
- **THEN** the record's text is `Jane Doe paid you L$5: for the hat`

#### Scenario: Gift without a description
- **WHEN** the grid reports a gift whose item description is `Payment`
- **THEN** the record's text is the grid's description as it is

#### Scenario: Payment to someone on the unlimited list
- **WHEN** the bot pays L$500 to an avatar that `unlimited_list` names
- **THEN** the record has amount 500 and counted 0

#### Scenario: Unconfirmed payment
- **WHEN** a payment of L$25 is not confirmed
- **THEN** the record has amount 25, counted 25 and an empty balance

### Requirement: Party fields of a record
Each record SHALL carry the other party: `party_uuid`, the UUID of the avatar or group that paid or was paid; `avatar_name` and `display_name`, that avatar's names, empty for a group; and, for a payment of the bot to an object, `object_name` and `object_uuid`, with the object's owner as the other party. For every other record the two object fields SHALL be empty.

#### Scenario: Payment to an object
- **WHEN** the bot pays the vendor `Hat Vendor` of Jane Doe (display name `JD`)
- **THEN** the record has Jane Doe's UUID, `Jane Doe`, `JD`, `Hat Vendor` and the vendor's UUID

#### Scenario: Payout from an object
- **WHEN** an object of Jane Doe pays the avatar L$10
- **THEN** the record has kind `received`, Jane Doe as the other party and empty object fields

### Requirement: Names in a record
The bot SHALL obtain the names of the other party from the grid as it does for a message (see `message-log`, "Names recorded with a message"): a name the grid does not supply within 5 seconds SHALL be left empty, recording SHALL NOT be delayed by more than 5 seconds for the sake of a name, and transactions SHALL be recorded in the order their outcomes were learned.

#### Scenario: Name lookup unavailable
- **WHEN** a payment is received and the grid does not answer the name lookup
- **THEN** the transaction is recorded within 5 seconds with the payer's UUID and empty names

### Requirement: Transactions are logged to standard error
The bot SHALL write every recorded transaction to standard error as one timestamped log line, at the normal log level and regardless of whether a transactions file is configured. The line SHALL say that it is a transaction and SHALL contain the kind, the amount, the names and UUIDs that are not empty, the balance when there is one, and the text.

#### Scenario: Received without a file
- **WHEN** `logs.transactions` is not configured and Jane Doe pays the avatar L$50
- **THEN** standard error receives one line containing `received`, `L$50` and Jane Doe's name

### Requirement: Transactions file
When `logs.transactions` is configured the bot SHALL append one row per recorded transaction to that file: one line of twelve tab-separated fields in this order: `time`, `kind`, `amount`, `balance`, `counted`, `type`, `party_uuid`, `avatar_name`, `display_name`, `object_name`, `object_uuid`, `text`. Time, escaping, encoding, appending, the header row and writing at once SHALL be as `message-log`, "TSV log files", says for its files.

#### Scenario: Row of a payment
- **WHEN** the bot's payment of L$25 to Jane Doe (display name `JD`) is confirmed at 2026-12-01 12:00:00 UTC, leaving L$975, and counts against the limit
- **THEN** the file gains the line `2026-12-01T12:00:00Z`, `paid`, `25`, `975`, `25`, the grid's type number, Jane Doe's UUID, `Jane Doe`, `JD`, empty, empty and the grid's description, separated by tabs

#### Scenario: Restart keeps earlier rows
- **WHEN** the bot is restarted with the same transactions file
- **THEN** the rows written before are still there, new rows follow them, and no second header row is written

### Requirement: Recording transactions never disturbs the bot
Recording a transaction SHALL NOT change what the bot does to stay logged in, in its place and seated. A failure to look up a name or to write a row SHALL be reported on standard error and SHALL NOT end the session or the bot; what it means for later payments is said in `payments`, "The limit needs its record". A payment still waiting for the grid's answer when the bot stops SHALL be recorded as `unconfirmed` before it exits.

#### Scenario: File becomes unwritable
- **WHEN** writing a row fails
- **THEN** standard error reports the failure, the avatar stays logged in, and the transaction still appears on standard error

#### Scenario: Stopped while a payment waits
- **WHEN** the bot is stopped two seconds after it sent a payment that the grid has not confirmed
- **THEN** the file has an `unconfirmed` row for it before the bot exits

### Requirement: Reading the file back
When `logs.transactions` names a file that exists, the bot SHALL read it at startup, before the first login. The rows of kind `paid` and `unconfirmed` whose `counted` is more than zero SHALL give the period of the limit and what is counted in it (see `payments`, "The limit outlasts the bot"). The newest 500 rows SHALL be kept for the web interface (see `web-server`, "Transactions are kept for pages that open later").

#### Scenario: Limit restored
- **WHEN** the file has a `paid` row of an hour ago with amount 900 and counted 900, and the bot starts with `limit = 1000`
- **THEN** L$100 are left

#### Scenario: Rows that do not count
- **WHEN** the file has `received`, `spent` and `failed` rows and a `paid` row with counted 0
- **THEN** none of them takes anything off the limit

### Requirement: Rows the bot cannot read
A row of the transactions file that does not have twelve fields, or whose `time`, `amount` or `counted` cannot be read, SHALL be skipped; the bot SHALL say on standard error, once, how many rows it skipped. A file that cannot be read at all, or whose first line is not the header row, is rejected as `bot-config` says.

#### Scenario: Damaged row
- **WHEN** one row of the file was cut short by a crash
- **THEN** the bot starts, counts the other rows, and standard error says that one row was skipped
