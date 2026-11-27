# web-ui Specification

## Purpose

The page of the web interface: what the operator sees and can do in a browser. It has a tab each for the world around the avatar, local chat, instant messages, the inventory, a command console and the page's own settings, and does everything through the commands and the event stream of `web-server`.

## Requirements

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

### Requirement: The page fits a phone
The page SHALL be usable on a screen 360 CSS pixels wide without scrolling sideways.

#### Scenario: Phone
- **WHEN** the page is opened on a phone held upright
- **THEN** every tab can be reached and used without scrolling sideways

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

### Requirement: Teleport form
The World tab SHALL have a teleport form with a text box each for X, Y, Z and the region name, and a button. The button SHALL send `tp` with X and Y, with Z when its box is filled, and with the region's name as one argument when its box is filled, so that an empty Z keeps the current height and an empty region stays in the current region as `tp` says (see `avatar-commands`, "Teleport command").

#### Scenario: Another region
- **WHEN** the operator fills in `128`, `128`, `25` and `Blue Lagoon` and presses the button
- **THEN** the bot is sent `tp 128 128 25` with `Blue Lagoon` as one argument, and the form shows the answer once the avatar has arrived

#### Scenario: Within the region, keeping the height
- **WHEN** the operator fills in `40` and `200` only
- **THEN** the bot is sent `tp 40 200`

### Requirement: Teleport form checks and answers
The page SHALL NOT send the teleport form's command, and SHALL mark the box, when X or Y is empty or a filled coordinate is not a number. While the teleport goes on the form SHALL show that it does, and SHALL then show the command's answer or its error.

#### Scenario: Not a number
- **WHEN** the operator fills in `12o` as X
- **THEN** nothing is sent and the X box is marked

#### Scenario: Teleport refused
- **WHEN** a collar holds `tploc` and the operator presses the button
- **THEN** the form shows the error naming `tploc`

### Requirement: Standing up
The World tab SHALL have a button that sends `stand`, and SHALL show the command's answer or its error.

#### Scenario: Stand
- **WHEN** the avatar is seated and the operator presses the button
- **THEN** the avatar stands up and the status panel shows no seat

### Requirement: Avatar scan
The World tab SHALL have a button that lists the avatars in the region with `scan avatars`, with a box for a distance that is passed on when filled. The list SHALL have a row for each line of the answer, in its order, with the avatar's name, its display name where it differs, its distance and its position, and SHALL write what the answer gives as `?` as unknown. The list SHALL stay as it is until the operator scans again, and SHALL say when it was made.

#### Scenario: Listing
- **WHEN** two avatars are in the region and the operator presses the button
- **THEN** the list has two rows, the nearer first

#### Scenario: Scan refused
- **WHEN** a collar holds `shownearby` and the operator presses the scan button
- **THEN** the tab shows the error naming `shownearby` and the list is not changed

### Requirement: Buttons of the avatar scan
Each row of the avatar scan SHALL have a button that teleports the avatar to that avatar: `tp` with the row's X and Y, and with its Z unless that is unknown, in the current region. A row whose avatar's UUID is known SHALL have a button that opens the conversation with that avatar on the IM tab.

#### Scenario: Teleport to an avatar
- **WHEN** the operator presses the teleport button of a row whose position is `103,100,25`
- **THEN** the bot is sent `tp 103 100 25`

#### Scenario: Unknown height
- **WHEN** the operator presses the teleport button of a row whose position is `128,64,?`
- **THEN** the bot is sent `tp 128 64`

#### Scenario: Open the conversation
- **WHEN** the operator presses the IM button of the row of Jane Doe
- **THEN** the IM tab is shown with the conversation with Jane Doe open

### Requirement: Object scan
The World tab SHALL have a button that lists the objects around the avatar with `scan --position`, with a box for a distance and a box for a part of the name, passed on as the distance and as `-n` when filled. The list SHALL have a row for each line of the answer, in its order, with the object's name, description, distance, owner's name and flags. The list SHALL stay as it is until the operator scans again, and SHALL say when it was made.

#### Scenario: Listing with a filter
- **WHEN** the operator fills in `20` and `bench` and presses the button
- **THEN** the bot is sent `scan --position 20 -n bench` and the list shows the objects of the answer, nearest first

#### Scenario: Nothing nearby
- **WHEN** the scan's answer has no lines
- **THEN** the tab says that nothing was found

### Requirement: Buttons of the object scan
Each row of the object scan SHALL have three buttons: one that teleports the avatar to the object, `tp` with the row's position in the current region; one that sends `sit` with the object's UUID; and one that sends `touch` with the object's UUID.

#### Scenario: Sit
- **WHEN** the operator presses the sit button of the row of `Park Bench`
- **THEN** the bot is sent `sit` with the bench's UUID, and the row shows the answer or the error

#### Scenario: Touch
- **WHEN** the operator presses the touch button of a row
- **THEN** the object is touched once

#### Scenario: Teleport to an object
- **WHEN** the operator presses the teleport button of a row whose position is `50,60,22`
- **THEN** the bot is sent `tp 50 60 22`

### Requirement: Results and errors are shown
Every command the operator sends from a control of the page SHALL be followed by its outcome where the operator looks: the control SHALL show that the command is under way until it is answered, and the page SHALL then show the answer's text or, set apart from it, the error's text, next to the control or in a place for messages that is visible on every tab. An error SHALL stay until the operator dismisses it or sends the same control's command again.

#### Scenario: Refused sit
- **WHEN** the operator presses a sit button and the bot answers with an error
- **THEN** the error's text is shown and stays when the operator switches tabs and comes back

### Requirement: A command under way is not sent twice
A control whose command is under way SHALL NOT send it a second time.

#### Scenario: Double click
- **WHEN** the operator presses the teleport button twice in a second
- **THEN** one `tp` is sent

### Requirement: Local chat tab
The Local chat tab SHALL show, beside the status panel, the local chat the message log records, as the event stream gives it and with what the bot kept from before the page was opened: one line per message, oldest first, each with its time and with the speaker and the text put together as the local chat contact of the XMPP gateway puts them (see `xmpp-gateway`, "Local chat contact").

#### Scenario: Chat heard
- **WHEN** Jane Doe, display name `Janey`, says `hi all` near the avatar
- **THEN** the tab shows `Janey (Jane Doe): hi all` with its time, within a second

#### Scenario: Whispers, shouts, kinds and /me
- **WHEN** Jane Doe, display name `Janey`, whispers `psst`, shouts `HEY` and says `/me waves`, and an object named `Collar` says `ready` to its owner
- **THEN** the tab shows `Janey (Jane Doe) whispers: psst`, `Janey (Jane Doe) shouts: HEY`, `Janey (Jane Doe) waves` and `Collar [owner]: ready`, as the local chat contact of the XMPP gateway shows them

### Requirement: Local chat follows the newest line
A new line of local chat SHALL appear without the operator doing anything. The view SHALL follow the newest line while it shows the end of the chat, and SHALL stay where it is, saying that there are new lines, while the operator has scrolled back.

#### Scenario: Reading back
- **WHEN** the operator has scrolled back and three new lines arrive
- **THEN** the view does not move, and the tab says that there are new lines

### Requirement: Speaking in local chat
The Local chat tab SHALL have a box for a message: sending it, with a button or the Enter key, SHALL send `say` with the text as it was typed, so that the chat-bar forms of `message-send` apply. The box SHALL be emptied when the command is answered with success; on an error the text SHALL stay and the error be shown. The avatar's own lines SHALL appear when the simulator delivers them back as chat, as every other line does.

#### Scenario: Speaking
- **WHEN** the operator types `hello everyone` and presses Enter
- **THEN** the avatar says `hello everyone` on channel 0, the box is empty, and the avatar's line appears in the chat

#### Scenario: Chat bar forms
- **WHEN** the operator sends `/shout come here` and then `/7 open`
- **THEN** the avatar shouts `come here` on channel 0 and then says `open` on channel 7

#### Scenario: Too long
- **WHEN** the operator sends a message of 2000 bytes
- **THEN** the error is shown, nothing is said, and the text is still in the box

### Requirement: Avatars in chat range
The Local chat tab SHALL list the avatars within chat range, from `scan avatars chat`: each with its name, its display name where it differs, and its distance. While the avatar is not connected, or a refresh is refused, the list SHALL be empty and say why.

#### Scenario: Nearby list closed
- **WHEN** a collar holds `shownearby`
- **THEN** the list is empty and names `shownearby`, and chat is still shown and can still be sent

### Requirement: Refreshing the avatars in chat range
The page SHALL refresh the list of the avatars in chat range by itself when the Local chat tab is shown, when the status panel's region changes, every 15 seconds while the tab is shown and the browser's window is visible, and when it has found the bot again; and when the operator presses the list's refresh button. Every refresh but the button's SHALL be marked as the page's own (see `web-server`, "Requests the page makes by itself").

#### Scenario: Someone walks up
- **WHEN** an avatar comes within 20 metres while the tab is shown
- **THEN** it is in the list within 15 seconds

#### Scenario: Tab not shown
- **WHEN** the World tab is shown, or the browser's window is hidden
- **THEN** the page sends no `scan avatars chat`

### Requirement: IM tab
The IM tab SHALL show a list of conversations and the conversation chosen from it. There SHALL be one conversation for each avatar and one for each object that an instant message the page has got was from or to, and one, named as the grid's, for the messages that carry neither an avatar's nor an object's UUID. The list SHALL be ordered by the time of each conversation's newest message, newest first.

#### Scenario: Message arrives
- **WHEN** Jane Doe sends the avatar `hello` while the IM tab shows another conversation
- **THEN** Jane Doe's conversation is first in the list and holds `hello`

### Requirement: Messages in a conversation
A conversation SHALL be named by the avatar's display name and legacy name, or the object's name, and SHALL show its messages oldest first, each with its time; the ones the bot sent SHALL be set apart from the ones it received, those of kind `im_auto` marked as sent by the bot itself, and those stored while the avatar was offline marked so.

#### Scenario: Autoreply
- **WHEN** the bot answers John Roe's message with its autoreply
- **THEN** John Roe's conversation holds his message and then the autoreply, marked as sent by the bot itself

### Requirement: Answering in a conversation
A conversation with an avatar SHALL have a box for a message: sending it, with a button or the Enter key, SHALL send `im` with the avatar's UUID and the text. The box SHALL be emptied on success; on an error the text SHALL stay and the error be shown. The message SHALL appear in the conversation when its `im_out` record arrives, as one sent from anywhere else does. A conversation with an object, and the grid's, SHALL have no box and SHALL say that it cannot be answered.

#### Scenario: Reply
- **WHEN** the operator opens Jane Doe's conversation, types `see you` and presses Enter
- **THEN** Jane Doe receives the IM `see you`, and it appears in the conversation as sent

#### Scenario: Sent from the control socket
- **WHEN** the operator runs `rslbot ctl im "Jane Doe" hi` while the page is open
- **THEN** `hi` appears in Jane Doe's conversation as sent

#### Scenario: Object
- **WHEN** an object named `Collar` sends the avatar an IM
- **THEN** there is a conversation named `Collar` that has no box for a message

#### Scenario: Not connected
- **WHEN** the operator sends a message while the bot is between logins
- **THEN** the error is shown, the text stays in the box, and nothing is sent later

### Requirement: Opening a conversation
The IM tab SHALL have a box that takes an avatar as `im` takes one, a name or a UUID, and opens a conversation with it. A conversation opened by a UUID SHALL be that avatar's. A conversation opened by a name SHALL send its first message as `im` with that name; when the bot answers with success, the conversation SHALL become that of the avatar the message went to, together with the messages the page already has of that avatar, and later messages SHALL go to its UUID.

#### Scenario: By name
- **WHEN** the operator types `jane.doe`, then `hi` in the conversation that opens, and the grid knows Jane Doe
- **THEN** Jane Doe receives `hi`, and the conversation is named `Jane Doe` and holds `hi` as sent

#### Scenario: Already talking
- **WHEN** Jane Doe has sent the avatar messages before, and the operator opens a conversation by typing `Jane Doe` and sends `hi`
- **THEN** there is one conversation with Jane Doe, holding her messages and `hi`

### Requirement: Opening a conversation with an unknown name
When the bot answers the first message of a conversation opened by a name with an error saying that it knows no such avatar, the error SHALL be shown and the conversation SHALL stay open with the text, to be corrected or closed.

#### Scenario: Unknown name
- **WHEN** the operator opens a conversation with `Nobody Here` and sends `hi`
- **THEN** the error naming `Nobody Here` is shown, nothing is sent, and `hi` is still in the box

### Requirement: Empty conversations are not kept
A conversation the operator opened and has sent nothing in SHALL be gone when the page is loaded again.

#### Scenario: Opened and left
- **WHEN** the operator opens a conversation with John Roe, who has sent the avatar nothing, sends nothing, and loads the page again
- **THEN** there is no conversation with John Roe

### Requirement: Friend list
The IM tab SHALL list the avatar's friends, from `friends`: each with its name, its display name where it differs, and whether it is online, offline or does not show it, in the order of the answer, the friends that are online first. Choosing a friend SHALL open the conversation with it, by its UUID. While the avatar is not connected the list SHALL say so.

#### Scenario: Who is online
- **WHEN** John Roe is online and Ann Poe is offline
- **THEN** John Roe is listed before Ann Poe, each marked with its status

#### Scenario: Conversation with a friend
- **WHEN** the operator chooses John Roe and sends `hello`
- **THEN** `im` is sent with John Roe's UUID, without a name lookup

### Requirement: Refreshing the friend list
The page SHALL refresh the friend list by itself when the IM tab is shown, every 60 seconds while the tab is shown and the browser's window is visible, and when it has found the bot again; and when the operator presses the list's refresh button. Every refresh but the button's SHALL be marked as the page's own.

#### Scenario: Friend comes online
- **WHEN** Ann Poe logs in while the IM tab is shown
- **THEN** she is shown as online within a minute

### Requirement: New messages are marked
A conversation that has got a message the bot received while the operator was not looking at it SHALL show how many, and the IM tab's label SHALL show that there are some, until the operator opens that conversation. The Local chat tab's label SHALL show that there are new lines while another tab is shown. Messages the bot sent, and messages kept from before the page was opened, SHALL NOT be marked.

#### Scenario: IM while on another tab
- **WHEN** Jane Doe sends two messages while the World tab is shown
- **THEN** the IM tab's label is marked, and her conversation shows 2 until it is opened

#### Scenario: History is not new
- **WHEN** the page is opened and gets twenty kept messages
- **THEN** no tab and no conversation is marked

### Requirement: Inventory tab
The Inventory tab SHALL show the avatar's inventory as a tree that starts with the entries of My Inventory. The page SHALL list a folder, with `inventory -n` and the folder's UUID, when the operator opens it, and My Inventory itself when the tab is first shown; it SHALL NOT ask for the whole inventory at once. Folders SHALL come before items, each kind in the order of the answer. A folder that was opened SHALL have a way to list it again.

#### Scenario: Opening a folder
- **WHEN** the operator opens the folder `Objects`, which holds `Radio` and the folder `Bot gear`
- **THEN** the bot is sent `inventory -n` with the folder's UUID, and the tree shows `Bot gear` and then `Radio` under `Objects`

#### Scenario: Large inventory
- **WHEN** the avatar's inventory holds 20,000 items in 800 folders and the tab is opened
- **THEN** the bot is asked for the entries of My Inventory only

#### Scenario: Inventory closed by RLV
- **WHEN** a collar holds `showinv` and the tab is opened
- **THEN** the tab shows the error naming `showinv` in place of the tree

### Requirement: Inventory entries
An entry of the Inventory tab SHALL show its name, its type, its attachment point or kind of clothing where the listing gives one, and whether it is attached now.

#### Scenario: Attached object
- **WHEN** the folder shown holds the object `Radio`, attached to the left hand
- **THEN** its entry shows `Radio`, its type, `Left Hand` and that it is attached

### Requirement: Attached view
The Inventory tab SHALL have a switch that shows, in place of the tree, what is attached to the avatar now, from `inventory -a`, each entry with its path.

#### Scenario: What is worn
- **WHEN** the operator switches to the attached view while the avatar wears three objects
- **THEN** three entries are listed with their paths, each with Detach and Touch

### Requirement: Inventory actions
An entry of the Inventory tab SHALL offer these actions, each a command with the entry's UUID, and no others:

| Entry | Actions |
|---|---|
| object that is not attached | Attach (`attach`) |
| object that is attached | Detach (`detach`), Touch (`touch`) |
| clothing | Wear (`wear`), Take off (`detach`) |
| body part | Wear (`wear`) |
| link | Wear (`wear`), Take off (`detach`) |
| folder, and every other type | none |

#### Scenario: Touch an attachment
- **WHEN** the operator presses Touch on the attached `Radio`
- **THEN** the bot is sent `touch` with the radio's inventory UUID, and the radio's script gets one touch

#### Scenario: Take off clothing
- **WHEN** the operator presses Take off on the shirt `Blue shirt`
- **THEN** the bot is sent `detach` with the shirt's UUID, and its answer or its error, as on a grid without an outfit service, is shown

### Requirement: Inventory listed again after an action
After an inventory action is answered with success, the page SHALL list the entry's folder, or the attached view, again, so that the entry shows what is attached now.

#### Scenario: Attach
- **WHEN** the operator presses Attach on `Radio`, last worn on the left hand
- **THEN** the bot is sent `attach` with the radio's UUID, the answer naming the point is shown, and `Radio` then shows `Left Hand`, attached, with Detach and Touch

### Requirement: Console tab
The Console tab SHALL have a box for a command line and, above it, what was sent and answered since the page was loaded: each line the operator sent, followed by the answer's text, `ok` for an answer without a text, or `error: ` and the error's text, in a fixed-width font with line ends and tab characters kept. Sending, with a button or the Enter key, SHALL send the line as a console line (see `web-server`, "Console lines"). The page SHALL send an empty line nowhere.

#### Scenario: Status
- **WHEN** the operator sends `status`
- **THEN** the tab shows `status` and under it the lines `rslbot ctl status` prints

#### Scenario: A listing
- **WHEN** the operator sends `scan 10`
- **THEN** the fields of each line stand in columns as their tab characters put them

#### Scenario: Command refused
- **WHEN** the operator sends `sit ground` while the bot is between logins
- **THEN** the tab shows `error: ` and the bot's reason

#### Scenario: Help
- **WHEN** the operator sends `help`
- **THEN** the tab shows the list of commands

#### Scenario: Empty line
- **WHEN** the operator presses Enter in a box holding three spaces
- **THEN** nothing is sent

### Requirement: Console answers in their places
Lines sent from the Console tab SHALL NOT wait for the answers of earlier ones, and an answer SHALL appear under the line it answers.

#### Scenario: Teleport and status
- **WHEN** the operator sends `tp 128 128 25 Home` and at once `status`
- **THEN** the answer of `status` appears under `status` before the teleport has ended, and the teleport's answer appears under its own line when it arrives

### Requirement: Console history
The up and down arrow keys in the Console tab's box SHALL bring back the lines sent before.

#### Scenario: Again
- **WHEN** the operator presses the up arrow in the empty box
- **THEN** the box holds the line sent last

### Requirement: Settings tab and theme
The Settings tab SHALL let the operator choose the page's theme: as the browser asks for, light, or dark. As the browser asks for SHALL be the choice until the operator makes another; with it the page SHALL be light or dark as the browser's preferred colour scheme says, and SHALL change with it while the page is open. Both themes SHALL be readable: text and controls SHALL stand out from their background in each.

#### Scenario: Default
- **WHEN** the page is opened for the first time in a browser set to prefer dark
- **THEN** the page is dark

#### Scenario: Browser changes its mind
- **WHEN** the theme is as the browser asks for and the operating system switches from light to dark in the evening
- **THEN** the open page turns dark

### Requirement: The theme is kept in the browser
The choice of theme SHALL be kept in that browser, for that address, and SHALL be in force when the page is opened there again, the login box included. The bot SHALL keep nothing of it: another browser has its own choice.

#### Scenario: Always light
- **WHEN** the operator chooses light in a browser set to prefer dark, and opens the page again the next day
- **THEN** the page is light

#### Scenario: Another browser
- **WHEN** the operator has chosen dark on the desktop and opens the page on a phone for the first time
- **THEN** the phone's page follows the phone's browser

### Requirement: Log out button
While a password is configured, the Settings tab SHALL have a button to log out.

#### Scenario: Logging out
- **WHEN** a password is configured and the operator presses the Settings tab's button to log out
- **THEN** the page shows the login box

### Requirement: Login box
While the bot asks for a password and the browser has no session, the page SHALL show a box for the password with a button and nothing of the bot: no tab, no status, no message. A wrong password SHALL be answered on the page, as SHALL too many attempts. After the right one the page SHALL show the tab its address names.

#### Scenario: Wrong password
- **WHEN** the operator types a wrong password
- **THEN** the page says that the password is wrong and shows the box again, empty

#### Scenario: Deep link
- **WHEN** the operator opens the address of the Inventory tab without a session and logs in
- **THEN** the Inventory tab is shown

### Requirement: A page whose session has ended
A page whose session has ended, by a logout in the same browser, by its age or by a restart of the bot, SHALL show the login box in place of its tabs as soon as it learns of it, and SHALL keep nothing of what it showed.

#### Scenario: Logged out in another window
- **WHEN** the operator has the page open in two windows of one browser and logs out in one
- **THEN** the other shows the login box, and no chat or message stays on screen

### Requirement: Losing and finding the bot
When the page cannot reach the bot, because the bot was stopped or the connection is gone, it SHALL say so in a place visible on every tab, SHALL keep showing what it has, and SHALL try again by itself, at first after a second and then at growing intervals of at most 30 seconds. A command sent meanwhile SHALL be answered on the page with an error saying that the bot cannot be reached, and SHALL NOT be sent later.

#### Scenario: Command while the bot is away
- **WHEN** the operator presses a sit button while the page cannot reach the bot
- **THEN** an error says so, and the avatar does not sit when the bot is found again

### Requirement: Taking up again after finding the bot
When the page has found the bot again it SHALL take up the event stream as `web-server` says ("A page that lost its stream catches up"), showing each message once, and SHALL refresh the status panel and the lists it refreshes by itself.

#### Scenario: Bot restarted
- **WHEN** the bot is stopped and started again a minute later while the page is open and no password is configured
- **THEN** the page says that it has lost the bot, and within 30 seconds of the bot's start it shows the new status and only the messages recorded since

#### Scenario: Train tunnel
- **WHEN** a phone loses its network for a minute during which Jane Doe sends two messages
- **THEN** both are in her conversation, once each, when the phone is back

### Requirement: Text from the grid is shown as text
Everything the page shows that comes from the grid or from the operator's typing, names, display names, chat, instant messages, object names and descriptions, inventory names, regions, answers and errors, SHALL be shown as the characters it consists of and SHALL NOT be read as markup, as script or as an address to load. The `\t`, `\n`, `\r`, `\\` and `\/` forms of the listings SHALL be shown as the characters they stand for.

#### Scenario: Object with a hostile name
- **WHEN** an object named `<img src=x onerror=alert(1)>` is in a scan, speaks in local chat and sends an IM
- **THEN** the name is shown letter by letter in all three places, no script runs, and nothing is loaded

#### Scenario: Escaped name
- **WHEN** an inventory item is named `Radio/TV` and the listing gives it as `Radio\/TV`
- **THEN** the tree shows `Radio/TV`

### Requirement: Money tab
The page SHALL show the Money tab and every Pay button while the status it got last has a `limit` line, which is while the web interface may pay (see `avatar-commands`, "Money lines of status"), and SHALL show none of them otherwise. Without the tab, the address of the Money tab SHALL show the World tab.

#### Scenario: Web may pay
- **WHEN** `payments.perms` has `web` and the page is opened
- **THEN** the tabs are World, Local chat, IM, Groups, Inventory, Money, Console and Settings

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
The Money tab SHALL have a form with a box for the payee, an avatar's name or UUID as `pay` takes one, a box for the amount, a box for an optional description, and a button. While the status the page got last has the line `pay description: blocked by RLV` (see `avatar-commands`, "Money lines of status"), the form SHALL show the text `Payment description is blocked by RLV` in place of the description box and SHALL send no description. After the confirmation (see "Confirmation before a payment") the button SHALL send `pay` with the amount, `avatar` and the payee as one argument, and, when the description box is not empty, `-d` and the description as one argument. The amount and description boxes SHALL be emptied on success; on an error all boxes SHALL keep their text and the error be shown.

#### Scenario: With a description
- **WHEN** the operator types `jane.doe`, `25` and `for the hat`, presses the button and confirms
- **THEN** the bot is sent `pay 25 avatar jane.doe -d "for the hat"`

#### Scenario: Description blocked
- **WHEN** the status has the line `pay description: blocked by RLV`
- **THEN** the form shows `Payment description is blocked by RLV` and no description box, and a payment sends no description

#### Scenario: Typed name
- **WHEN** the operator types `jane.doe` and `25`, presses the button and confirms
- **THEN** the bot is sent `pay 25 avatar jane.doe`, and the form shows the answer once the grid has confirmed the payment

#### Scenario: Refused for the limit
- **WHEN** the bot answers the form's payment with an error saying that L$10 are left
- **THEN** the error is shown, and the payee and the amount are still in their boxes

### Requirement: Send money form checks
The page SHALL NOT open the confirmation, and SHALL mark the box, when the payee is empty, the amount is not a whole number greater than zero written in digits, or the description is longer than 127 bytes. While the payment is under way the form SHALL show that it is and SHALL NOT send another.

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
For a payment to an avatar the dialog SHALL name the amount and the payee: a chosen avatar by its name and UUID, a typed one as typed; and the description, when there is one. For a payment to an object it SHALL name the amount, the object's name and the owner's name, as the answer of `prices` gave them.

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

### Requirement: Groups tab
The Groups tab SHALL show, from top to bottom or side by side as the width allows: the invitations that wait (see "Invitations on the Groups tab"), the list of the avatar's groups, and the group chosen from that list with its chat and notices. While no invitation waits the tab SHALL show nothing in their place. The tab SHALL be there whatever the config says of payments.

#### Scenario: Opening the tab
- **WHEN** the avatar is in `Builders` and `Dancers`, no invitation waits, and the operator opens the Groups tab
- **THEN** the tab lists both groups and nothing about invitations

### Requirement: List of groups
The Groups tab SHALL list the avatar's groups, from `group list`: each with its name, in the order of the answer, the active group marked as such. Choosing a group SHALL show its chat and notices. A group the page has got a group message of and that the answer does not list SHALL stay in the list, set apart as one the avatar is not in, and SHALL be shown without the box for a message and without the Activate, Invite and Leave buttons. While the avatar is not connected the list SHALL say so and SHALL keep the groups the page has messages of.

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

### Requirement: Leave button
The view of a group the avatar is in SHALL have a button that leaves the group. Pressing it SHALL first show a dialog that asks whether to leave the group, names it by name and UUID, and says that joining it again may take an invitation or a fee, with a button that leaves and one that cancels, the cancelling button focused. Cancelling, the Escape key and closing the dialog SHALL send nothing. Only the leaving button SHALL send `group leave` with the group's UUID; the view SHALL show that the command is under way and then its answer or its error, and the list SHALL show the group as one the avatar is not in, or not at all, when the bot says that the groups have changed.

#### Scenario: Leaving
- **WHEN** the operator presses Leave in the view of `Dancers` and then the dialog's leaving button
- **THEN** the bot is sent `group leave` with the UUID of `Dancers`, the view shows the answer, and the list no longer has `Dancers` as a group the avatar is in

#### Scenario: Cancel
- **WHEN** the operator presses Leave in the view of `Dancers` and then the dialog's cancel button, or the Escape key
- **THEN** nothing is sent and the avatar is still in `Dancers`

#### Scenario: Group the avatar is no longer in
- **WHEN** the list keeps `Builders` as a group the avatar is not in
- **THEN** the view of `Builders` has no Leave button

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
