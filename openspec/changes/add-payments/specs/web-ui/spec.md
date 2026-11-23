# Spec Delta

## MODIFIED Requirements

### Requirement: Tabs
The page SHALL have six tabs, in this order: World, Local chat, IM, Inventory, Console and Settings, and a seventh, Money, between Inventory and Console, while the web interface may pay (see "Money tab"). One tab SHALL be shown at a time, World when the page is opened without naming one. The tab shown SHALL be part of the page's address, so that loading the page again, a bookmark, and the browser's back and forward buttons lead to the same tab. Switching tabs SHALL NOT lose what the other tabs hold: text typed and not sent, the results of scans, the folders opened, and the console's output.

#### Scenario: First visit
- **WHEN** the page is opened at `/`
- **THEN** the World tab is shown

#### Scenario: Reload
- **WHEN** the operator is on the IM tab and loads the page again
- **THEN** the IM tab is shown

#### Scenario: Half-typed message
- **WHEN** the operator types a message on the Local chat tab, looks at the World tab and comes back
- **THEN** the typed text is still in the box

### Requirement: Status panel
The World tab and the Local chat tab SHALL show the same status panel: the avatar's name, whether the avatar is connected, and while it is, the region, the position, what it sits on with the seat's name when the bot knows it, and whether the bot is holding a placement or in manual mode. The panel SHALL show what the `status` command answers (see `avatar-commands`, "Status command") and SHALL follow it by the event stream, without the operator doing anything. The balance and the limit SHALL NOT be on the panel: the Money tab shows them (see "Money header").

#### Scenario: Seated
- **WHEN** the avatar is seated on the held object `Park Bench` in `Target`
- **THEN** the panel shows the region `Target`, the position, the seat `Park Bench` and that the bot is holding

#### Scenario: Teleport from elsewhere
- **WHEN** `rslbot ctl tp 128 128 25 Home` is run while the page is open
- **THEN** the panel shows `Home` and the new position once the avatar has arrived, without the page being loaded again

#### Scenario: Between logins
- **WHEN** the bot is waiting to log in again
- **THEN** the panel shows the avatar's name, says that the avatar is not connected and shows no region, position or seat

## ADDED Requirements

### Requirement: Money tab
The page SHALL show the Money tab and every Pay button while the status it got last has a `limit` line, which is while the web interface may pay (see `avatar-commands`, "Money lines of status"), and SHALL show none of them otherwise. Without the tab, the address of the Money tab SHALL show the World tab.

#### Scenario: Web may pay
- **WHEN** `payments.perms` has `web` and the page is opened
- **THEN** the tabs are World, Local chat, IM, Inventory, Money, Console and Settings

#### Scenario: Web may not pay
- **WHEN** the config has no `[payments]` table, or `payments.perms` has no `web`
- **THEN** the page has no Money tab and no Pay button anywhere, and opening it at the Money tab's address shows the World tab

### Requirement: Money header
The Money tab SHALL begin with a header that shows the avatar's name, its balance and what is left of the limit, and, when the status names a renewal, `(renews to <amount> on <date and time>)` with the time in the browser's time zone. The header SHALL follow the status of the event stream, without the operator doing anything.

#### Scenario: Limit partly used
- **WHEN** the status ends with `balance: L$1200` and `limit: L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)`
- **THEN** the header shows the avatar's name, L$1200, L$750 and that the limit renews to L$1000 on 8 December 2026 at the browser's local time for 12:00 UTC

#### Scenario: Payment received
- **WHEN** Jane Doe pays the avatar L$50 while the Money tab is shown
- **THEN** the balance in the header goes up by L$50 without the page being loaded again

#### Scenario: Balance not known yet
- **WHEN** the bot has not logged in yet
- **THEN** the header says that the balance is unknown

### Requirement: Send money form
The Money tab SHALL have a form with a box for the payee, an avatar's name or UUID as `pay` takes one, a box for the amount, and a button. After the confirmation (see "Confirmation before a payment") the button SHALL send `pay` with the amount, `avatar` and the payee as one argument. The amount box SHALL be emptied on success; on an error both boxes SHALL keep their text and the error be shown.

#### Scenario: Typed name
- **WHEN** the operator types `jane.doe` and `25`, presses the button and confirms
- **THEN** the bot is sent `pay 25 avatar jane.doe`, and the form shows the answer once the grid has confirmed the payment

#### Scenario: Refused for the limit
- **WHEN** the bot answers the form's payment with an error saying that L$10 are left
- **THEN** the error is shown, and the payee and the amount are still in their boxes

### Requirement: Send money form checks
The page SHALL NOT open the confirmation, and SHALL mark the box, when the payee is empty or the amount is not a whole number greater than zero written in digits. While the payment is under way the form SHALL show that it is and SHALL NOT send another.

#### Scenario: Not an amount
- **WHEN** the operator types `10.5` or nothing as the amount and presses the button
- **THEN** the amount box is marked, no dialog opens, and nothing is sent

### Requirement: Choosing a payee
The Money tab SHALL offer, for the payee, the friends as the IM tab lists them and the avatars the answer of `payees` names, each list under its own heading. Choosing one SHALL make it the payee: the form SHALL show its name and pay by its UUID, or by its name for a line of `payees` without a UUID. Typing in the box SHALL make what is typed the payee again.

#### Scenario: Friend
- **WHEN** the operator chooses the friend Jane Doe, types `25`, presses the button and confirms
- **THEN** the bot is sent `pay 25 avatar` with Jane Doe's UUID

#### Scenario: Whitelist
- **WHEN** `whitelist` has `John Roe` and the UUID of Jane Doe
- **THEN** the tab offers John Roe and Jane Doe under the heading of the whitelist, and choosing John Roe pays by the name `John Roe`

#### Scenario: No lists
- **WHEN** the answer of `payees` has no lines
- **THEN** the tab shows no heading for the whitelist

### Requirement: Lists of the Money tab are asked for by the page
The page SHALL send `payees`, marked as its own (see `web-server`, "Requests the page makes by itself"), when the Money tab is first shown and when it has found the bot again, and SHALL refresh the friends there as the IM tab does. An error of either SHALL be shown in place of the list and SHALL NOT keep the form from being used.

#### Scenario: Not connected
- **WHEN** the Money tab is shown while the bot is between logins
- **THEN** the lists say that they could not be read, and the form can still be filled in

### Requirement: Transaction list
The Money tab SHALL list the transactions the page has got from the event stream, newest first, each with its time, its kind in words, the amount, the other party by display name and login name or else by UUID, the object's name when it has one, the grid's text, and the balance after it when the record has one. A new one SHALL appear without the operator doing anything.

#### Scenario: Paid and received
- **WHEN** the bot paid Jane Doe L$25 and John Roe then paid the avatar L$50
- **THEN** the list shows John Roe's L$50 as received above Jane Doe's L$25 as paid, each with the balance after it

#### Scenario: After a restart
- **WHEN** the bot was restarted with a transactions file of 40 rows and the page is opened
- **THEN** the list shows those 40 transactions

#### Scenario: Nothing yet
- **WHEN** the page has got no transaction
- **THEN** the tab says that there are none

### Requirement: Transactions that did not go through are set apart
In the transaction list, a transaction of kind `failed` and one of kind `unconfirmed` SHALL be set apart from the others and say in words that the payment failed, or that the grid did not confirm it.

#### Scenario: Unconfirmed payment
- **WHEN** the page gets an `unconfirmed` record of L$25 to Jane Doe
- **THEN** its row says that the payment was sent and not confirmed, and looks different from a `paid` row

### Requirement: Pay button of a conversation
A conversation with an avatar on the IM tab SHALL have a Pay button that shows the Money tab with that avatar as the payee, by its UUID, with the amount box empty and nothing sent. A conversation with an object, and the grid's, SHALL have none.

#### Scenario: From a conversation
- **WHEN** the operator presses Pay in Jane Doe's conversation
- **THEN** the Money tab is shown with Jane Doe as the payee and the cursor in the amount box, and nothing is paid

#### Scenario: Object
- **WHEN** the conversation is with the object `Collar`
- **THEN** it has no Pay button

### Requirement: Pay button of the object scan
A row of the object scan whose flags have `pay` SHALL have a Pay button. Pressing it SHALL send `prices` with the object's UUID and show the answer in the row: a drop-down list with the amounts the object offers and, when it takes other amounts, an entry for another amount with a box that holds the suggested amount, if any; and a button that pays what is chosen. An error of `prices` SHALL be shown in the row.

#### Scenario: Vendor
- **WHEN** the operator presses Pay in the row of a vendor that offers 100 and 250 only
- **THEN** the row shows a list with L$100 and L$250, and no box for another amount

#### Scenario: Tip jar
- **WHEN** the operator presses Pay in the row of a tip jar that offers 1, 5, 10, 20 and other amounts
- **THEN** the list has the four amounts and an entry for another amount, which shows a box when it is chosen

#### Scenario: Object that takes no money
- **WHEN** a row's flags have no `pay`
- **THEN** the row has no Pay button

### Requirement: Paying from the object scan
The button that pays what is chosen in a row SHALL, after the confirmation (see "Confirmation before a payment"), send `pay` with the amount, `object` and the object's UUID, and the row SHALL show the answer or the error. Another amount SHALL be checked as the amount of the Send money form is.

#### Scenario: Buying from a vendor
- **WHEN** the operator chooses L$250 in the vendor's row, presses the button and confirms
- **THEN** the bot is sent `pay 250 object` with the vendor's UUID, and the row shows the answer

### Requirement: Confirmation before a payment
Before the page sends a `pay` from the Send money form or from the object scan, it SHALL show a dialog that names the amount and the payee and has a button that pays and one that cancels. Nothing SHALL be sent until the paying button is pressed; cancelling, the Escape key and a click outside the dialog SHALL send nothing. While the dialog is open nothing else on the page SHALL be usable.

#### Scenario: Cancel
- **WHEN** the operator presses the form's button, and then the dialog's cancel button
- **THEN** nothing is sent, and the form is as it was

#### Scenario: Enter in the form
- **WHEN** the operator presses the Enter key in the amount box
- **THEN** the dialog opens with the cancelling button focused, so that a second Enter pays nothing

### Requirement: What the confirmation names
For a payment to an avatar the dialog SHALL name the amount and the payee: a chosen avatar by its name and UUID, a typed one as typed. For a payment to an object it SHALL name the amount, the object's name and the owner's name, as the answer of `prices` gave them.

#### Scenario: Object
- **WHEN** the operator pays L$250 from the row of `Hat Vendor`, owned by Jane Doe
- **THEN** the dialog names L$250, `Hat Vendor` and Jane Doe as the owner

#### Scenario: Group's object
- **WHEN** the answer of `prices` gave `owner: group` and a UUID
- **THEN** the dialog says that a group owns the object and shows that UUID

#### Scenario: Typed payee
- **WHEN** the operator typed `jane.doe` as the payee and `25` as the amount
- **THEN** the dialog names L$25 and `jane.doe`

### Requirement: Typed payments are not confirmed
A `pay` line the operator types into the Console tab SHALL be sent as it is, without the dialog.

#### Scenario: Console
- **WHEN** the operator types `pay 25 avatar "Jane Doe"` into the Console tab and presses Enter
- **THEN** the line is sent at once, and the console prints the answer
