# Spec Delta

## MODIFIED Requirements

### Requirement: Tabs
The page SHALL have seven tabs, in this order: World, Local chat, IM, Groups, Inventory, Console and Settings, and an eighth, Money, between Inventory and Console, while the web interface may pay (see "Money tab"). One tab SHALL be shown at a time, World when the page is opened without naming one. The tab shown SHALL be part of the page's address, so that loading the page again, a bookmark, and the browser's back and forward buttons lead to the same tab. Switching tabs SHALL NOT lose what the other tabs hold: text typed and not sent, the results of scans, the folders opened, and the console's output.

#### Scenario: First visit
- **WHEN** the page is opened at `/`
- **THEN** the World tab is shown

#### Scenario: Reload
- **WHEN** the operator is on the IM tab and loads the page again
- **THEN** the IM tab is shown

#### Scenario: Order of the tabs
- **WHEN** the page is opened and the web interface may not pay
- **THEN** the tabs are World, Local chat, IM, Groups, Inventory, Console and Settings

#### Scenario: Half-typed message
- **WHEN** the operator types a message on the Local chat tab, looks at the World tab and comes back
- **THEN** the typed text is still in the box

### Requirement: Money tab
The page SHALL show the Money tab and every Pay button while the status it got last has a `limit` line, which is while the web interface may pay (see `avatar-commands`, "Money lines of status"), and SHALL show none of them otherwise. Without the tab, the address of the Money tab SHALL show the World tab.

#### Scenario: Web may pay
- **WHEN** `payments.perms` has `web` and the page is opened
- **THEN** the tabs are World, Local chat, IM, Groups, Inventory, Money, Console and Settings

#### Scenario: Web may not pay
- **WHEN** the config has no `[payments]` table, or `payments.perms` has no `web`
- **THEN** the page has no Money tab and no Pay button anywhere, and opening it at the Money tab's address shows the World tab

## ADDED Requirements

### Requirement: Groups tab
The Groups tab SHALL show, from top to bottom or side by side as the width allows: the invitations that wait (see "Invitations on the Groups tab"), the list of the avatar's groups, and the group chosen from that list with its chat and notices. While no invitation waits the tab SHALL show nothing in their place. The tab SHALL be there whatever the config says of payments.

#### Scenario: Opening the tab
- **WHEN** the avatar is in `Builders` and `Dancers`, no invitation waits, and the operator opens the Groups tab
- **THEN** the tab lists both groups and nothing about invitations

### Requirement: List of groups
The Groups tab SHALL list the avatar's groups, from `group list`: each with its name, in the order of the answer, the active group marked as such. Choosing a group SHALL show its chat and notices. A group the page has got a group message of and that the answer does not list SHALL stay in the list, set apart as one the avatar is not in, and SHALL be shown without the box for a message and without the Activate and Invite buttons. While the avatar is not connected the list SHALL say so and SHALL keep the groups the page has messages of.

#### Scenario: Active group
- **WHEN** `Builders` is the active group
- **THEN** its entry in the list is marked as active and the entry of `Dancers` is not

#### Scenario: Between logins
- **WHEN** the bot is waiting to log in again and the page has lines of `Builders`
- **THEN** the list says that the avatar is not connected, and the lines of `Builders` can still be read

### Requirement: Refreshing the groups and the invitations
The page SHALL send `group list` and `group invitations`, marked as its own (see `web-server`, "Requests the page makes by itself"), when the Groups tab is first shown, whenever the event stream says that the groups have changed (see `web-server`, "Changes of the groups on the event stream"), whether or not the tab is shown then, and when it has found the bot again; and, not marked so, when the operator presses the tab's refresh button. An error of either SHALL be shown in place of its list.

#### Scenario: Joined from elsewhere
- **WHEN** `rslbot ctl group join <uuid>` makes the avatar a member of `Dancers` while the Groups tab is shown
- **THEN** `Dancers` is in the list within two seconds, without the operator doing anything

#### Scenario: Quiet log
- **WHEN** a page stays open for an hour during which the groups do not change
- **THEN** the operator log has no line for requests of the Groups tab

### Requirement: Messages of a group
A group's view SHALL be headed by the group's name and SHALL show the group messages the page has got for it, chat and notices in one list, oldest first, each with its time and, for a line of chat, the speaker by display name and legacy name as the Local chat tab names a speaker, and the text. The lines the bot said (`chat_out`) SHALL be set apart from the ones it received. A new message SHALL appear without the operator doing anything, and the view SHALL follow the newest message as "Local chat follows the newest line" says.

#### Scenario: Chat
- **WHEN** Jane Doe, display name `Janey`, says `hello` in `Builders` and the bot then says `hi` there
- **THEN** the view of `Builders` shows `Janey (Jane Doe): hello` and then `hi`, set apart as the avatar's own, each with its time

#### Scenario: Blocked line
- **WHEN** RLV blocked a line of Jane Doe
- **THEN** the view shows `(blocked by RLV: recvim)` as her line

### Requirement: Notices are set apart
In a group's view a notice SHALL stand out from chat at a glance: in a bold face, on a tinted background or with a coloured border, in both themes, with the word `Notice`, the sender's name, and the subject as its heading above the text. A notice with an attachment SHALL name the attached item and say that the page cannot take it. A notice the grid had stored while the avatar was offline SHALL be marked so. Line breaks of a notice's text SHALL be kept.

#### Scenario: Notice among chat
- **WHEN** `Builders` gets two lines of chat, then a notice from Jane Doe with the subject `Meeting` and the text `Tonight at eight`, then another line of chat
- **THEN** the four are shown in that order, and the notice is bold on its own background with `Notice`, `Jane Doe`, `Meeting` and the text

#### Scenario: Attachment
- **WHEN** a notice carries the notecard `Agenda`
- **THEN** the notice names `Agenda` and says that it cannot be taken from the page

### Requirement: Speaking in a group
The view of a group the avatar is in SHALL have a box for a message: sending it, with a button or the Enter key, SHALL send `group chat` with the group's UUID and the text. The box SHALL be emptied on success; on an error the text SHALL stay and the error be shown. The line SHALL appear in the view when its `chat_out` record arrives, as one said from anywhere else does. Text typed for one group and not sent SHALL still be in that group's box after the operator has looked at another group.

#### Scenario: Saying a line
- **WHEN** the operator chooses `Builders`, types `see you` and presses Enter
- **THEN** the bot is sent `group chat` with the UUID of `Builders` and `see you`, the box is empty, and `see you` appears in the view as the avatar's own line

#### Scenario: Refused by RLV
- **WHEN** a collar holds `sendim` and the operator sends a line
- **THEN** the error naming `sendim` is shown and the text is still in the box

### Requirement: Activate button
The view of a group the avatar is in SHALL have a button that sends `group activate` with the group's UUID and shows the answer or the error; for the active group it SHALL say instead that the group is active. The list SHALL show the new active group when the bot says that the groups have changed.

#### Scenario: Activating
- **WHEN** the operator presses the button in the view of `Dancers` while `Builders` is active
- **THEN** the bot is sent `group activate` with the UUID of `Dancers`, and the list then marks `Dancers` and not `Builders` as active

### Requirement: Invite button
The view of a group whose line in `group list` has the flag `invite` SHALL have a button that opens a form with a box for an avatar, a name or a UUID as `group invite` takes one, and the friends as the IM tab lists them to choose from; choosing a friend SHALL invite by its UUID. Sending the form SHALL send `group invite` with the group's UUID and the avatar as one argument, and SHALL show the answer or the error; the page SHALL NOT send it, and SHALL mark the box, while the box is empty. A group without the flag SHALL have no such button.

#### Scenario: Inviting a friend
- **WHEN** the operator presses Invite in the view of `Builders`, chooses the friend Jane Doe and sends
- **THEN** the bot is sent `group invite` with the UUID of `Builders` and Jane Doe's UUID, and the form shows the answer

#### Scenario: No right to invite
- **WHEN** the line of `Dancers` has no `invite` flag
- **THEN** the view of `Dancers` has no Invite button

### Requirement: Info button
A group's view SHALL have a button that sends `group info` with the group's UUID and shows the answer in the view: each line as a name and its value, the charter with its line breaks, and the fee as the answer gives it. An error SHALL be shown in its place. The info SHALL stay until the operator closes it or asks again.

#### Scenario: Info
- **WHEN** the operator presses Info in the view of `Builders`
- **THEN** the view shows the group's name, founder, fee, enrollment, numbers of members and roles, the avatar's title and the charter, the charter on as many lines as it has

### Requirement: Invitations on the Groups tab
While invitations wait, the top of the Groups tab SHALL show a row for each line of the answer of `group invitations`, in its order: the group's name, or its UUID where the answer gives `?`, the inviter's name, the fee as `L$` and the amount when it is greater than zero, a button that accepts and a button that rejects. The Groups tab's label SHALL show that an invitation waits, on every tab, for as long as one does.

#### Scenario: Free and paid
- **WHEN** Jane Doe's invitation into `Builders`, with a fee of L$250, and John Roe's into the free group `Dancers` wait
- **THEN** the tab shows two rows on top, the first with `Builders`, `Jane Doe` and `L$250`, the second with `Dancers`, `John Roe` and no fee, each with its two buttons, and the tab's label is marked

#### Scenario: Invitation arrives while on another tab
- **WHEN** an invitation begins to wait while the World tab is shown
- **THEN** the Groups tab's label is marked within two seconds

### Requirement: Accepting and rejecting an invitation
The accepting button of an invitation without a fee SHALL send `accept group` with the group's UUID, and the rejecting button of any invitation SHALL send `decline group` with the group's UUID. The row SHALL show that the command is under way and then its answer or its error, and the page SHALL then ask for the invitations and the groups again.

#### Scenario: Accept
- **WHEN** the operator presses Accept in the row of `Dancers`, which has no fee
- **THEN** the bot is sent `accept group` with the UUID of `Dancers`, and once it is answered the row is gone and `Dancers` is in the list of groups

#### Scenario: Reject
- **WHEN** the operator presses Reject in the row of `Builders`
- **THEN** the bot is sent `decline group` with the UUID of `Builders`, nothing is paid, and the row is gone

### Requirement: Confirmation before a fee
Before the page accepts an invitation with a fee it SHALL show the dialog of "Confirmation before a payment", naming the fee, the group by name and UUID, and the inviter, with a button that pays and one that cancels and with the cancelling button focused. Only the paying button SHALL send anything: `accept group` with the group's UUID, `--pay` and the fee of the row. While the web interface may not pay (see "Money tab"), the row of an invitation with a fee SHALL have no accepting button and SHALL say that payments are not enabled for the web interface; its rejecting button SHALL stay.

#### Scenario: Paying the fee
- **WHEN** the web interface may pay, the operator presses Accept in the row of `Builders` with `L$250`, and then the dialog's paying button
- **THEN** the bot is sent `accept group` with the UUID of `Builders`, `--pay` and `250`, and the row shows the answer

#### Scenario: Cancel
- **WHEN** the operator presses Accept in that row and then the dialog's cancel button, or the Escape key
- **THEN** nothing is sent and the invitation still waits

#### Scenario: Web may not pay
- **WHEN** `payments.perms` has no `web` and an invitation with a fee of L$250 waits
- **THEN** its row shows `L$250`, says that payments are not enabled for the web interface, and has a Reject button and no Accept button

#### Scenario: Fee raised meanwhile
- **WHEN** the bot answers the acceptance with an error naming a fee higher than the sum
- **THEN** the error is shown in the row, nothing was paid, and the invitation still waits

### Requirement: New group messages are marked
A group that has got a line of chat or a notice the bot received while the operator was not looking at that group SHALL show how many, and the Groups tab's label SHALL show that there are some, until the operator opens that group. Lines the bot said, and messages kept from before the page was opened, SHALL NOT be marked.

#### Scenario: Notice while on another tab
- **WHEN** `Builders` gets a notice and a line of chat while the World tab is shown
- **THEN** the Groups tab's label is marked, and `Builders` shows 2 until it is opened

#### Scenario: History is not new
- **WHEN** the page is opened and gets fifty kept group messages
- **THEN** no group is marked
