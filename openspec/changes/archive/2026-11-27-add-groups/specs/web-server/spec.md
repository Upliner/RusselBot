# Spec Delta

## MODIFIED Requirements

### Requirement: Requests the page makes by itself
The page SHALL mark as its own the commands it sends to keep a list up to date without the operator having asked for that list then: the `scan avatars` for the avatars in chat range and the `friends` for the friend list that it sends when their tab is shown, when it has found the bot again, and on a timer, the `payees` it sends for the Money tab, and the `group list` and `group invitations` it sends for the Groups tab (see `web-ui`). Only `scan avatars`, `friends`, `payees`, `group list` and `group invitations` SHALL be accepted as the page's own; any other command marked so, another `group` command among them, SHALL be refused and not carried out.

#### Scenario: Something else marked as the page's own
- **WHEN** a client sends `say hello` marked as the page's own
- **THEN** it is refused and nothing is said

#### Scenario: Another group command marked as the page's own
- **WHEN** a client sends `group chat Builders hello` marked as the page's own
- **THEN** it is refused and nothing is said

## ADDED Requirements

### Requirement: Group messages on the event stream
To every open page that asks for it, the bot SHALL send every group message `group-chat` records, of every kind, `chat_out` included, as soon as it is recorded and in the order of recording, with its time, its kind, the group's UUID and name, the avatar's UUID when the message gives it, the avatar's name and display name, the subject, the attachment's name, the text, and whether it was stored while the avatar was offline. A line that RLV blocked SHALL be sent as it was recorded, without the text that was said. Sending them SHALL NOT depend on the `group` file of `[logs]`.

#### Scenario: Line heard
- **WHEN** Jane Doe says `hello` in `Builders` while a page is open
- **THEN** the page has the line within a second of its being recorded, with the UUID of `Builders` and Jane Doe's UUID and names

#### Scenario: Line said from elsewhere
- **WHEN** the operator runs `rslbot ctl group chat Builders hi` while a page is open
- **THEN** the page gets the `chat_out` record

#### Scenario: No group file
- **WHEN** the config has no `group` under `[logs]`
- **THEN** the pages get group chat and notices all the same

### Requirement: Group messages are kept for pages that open later
While the config has a `[web]` table the bot SHALL keep, in memory only, the newest 500 group messages it has recorded since it started, counted apart from local chat, instant messages and transactions. A page SHALL get them when it opens its stream and when it catches up, under the rules for kept messages (see "Recent messages are kept for pages that open later" and "A page that lost its stream catches up"). With no `[web]` table the bot SHALL keep none.

#### Scenario: Page opened later
- **WHEN** a notice was sent to `Builders` an hour ago and the page is opened now
- **THEN** the page shows the notice, with the time it arrived

#### Scenario: Busy group
- **WHEN** 2000 lines of group chat and 3 instant messages have been recorded since the bot started
- **THEN** a page that opens gets the newest 500 lines of group chat and all 3 instant messages

### Requirement: Changes of the groups on the event stream
On every open event stream the bot SHALL send an event that says that the groups have changed, within one second, whenever the list of the avatar's groups, one of their flags, the active group or the list of waiting invitations changes, and when the session ends or a login succeeds. The event SHALL carry no list: the page asks for what it shows (see "Requests the page makes by itself"). These events SHALL NOT be kept for pages that open later.

#### Scenario: Invitation arrives
- **WHEN** Jane Doe invites the avatar into `Builders` while a page is open and the invitation waits
- **THEN** the page gets the event within a second of the invitation's waiting

#### Scenario: Activated from elsewhere
- **WHEN** `rslbot ctl group activate Dancers` is carried out while a page is open
- **THEN** the page gets the event
