# web-server Specification

## Purpose

Lets the operator use the bot from a web browser without anything else installed: the bot itself serves a page over HTTP, carries out the commands the page sends as the control socket carries them out, and sends the page what the avatar hears and where it is as it happens. What the page shows and does is described in `web-ui`.

## Requirements

### Requirement: Web server lifetime
When the config has a `[web]` table, the bot SHALL listen for HTTP connections on its `listen` address, SHALL start listening before its first login attempt, and SHALL keep listening until it exits, whether or not the avatar is connected to a grid. The page SHALL be served at the path `/` of that address. Without a `[web]` table the bot SHALL NOT listen on any web address.

#### Scenario: Web interface enabled
- **WHEN** the bot starts with `listen = "127.0.0.1:8080"` in its `[web]` table
- **THEN** a browser on the same machine gets the page at `http://127.0.0.1:8080/` before or during the first login

#### Scenario: Web interface not configured
- **WHEN** the config has no `[web]` table
- **THEN** the bot listens on no TCP port for a web interface

#### Scenario: Between logins
- **WHEN** the page is opened while the bot is waiting to log in again
- **THEN** the page loads and shows that the avatar is not connected

### Requirement: The web server stops with the bot
When the bot stops, it SHALL stop accepting web connections and close the ones that are open before it exits; an answer still owed to a page SHALL NOT delay the exit by more than 2 seconds.

#### Scenario: Bot stopped
- **WHEN** the bot is stopped while a page is open
- **THEN** the bot exits within the time a clean logout takes, and the page shows that it has lost the bot

### Requirement: The page is part of the bot
The bot SHALL serve the page, its script and its style from what is built into the program: it SHALL read no file to serve them, and the page SHALL load nothing, neither scripts, styles, fonts nor images, from any host but the bot. A path the bot does not serve SHALL be answered as not found, and a request method it does not serve as not allowed; neither SHALL be answered with anything of the bot's state.

#### Scenario: No other host
- **WHEN** the page is opened in a browser that can reach the bot and nothing else
- **THEN** the page works in full

#### Scenario: Binary moved
- **WHEN** the `rslbot` binary is copied alone to another machine and started with a `[web]` table
- **THEN** the page is served there

#### Scenario: Unknown path
- **WHEN** a browser asks for `/russelbot.toml` or `/../etc/passwd`
- **THEN** it is answered as not found, and no file is read

### Requirement: The web server never disturbs the bot
Accepting, reading and answering web connections SHALL NOT delay the bot's simulator traffic. A slow, silent or misbehaving browser SHALL NOT make the bot exit, lose its session, stop recording messages, or stop answering the control socket, the XMPP gateway and other pages. A failure on a web connection SHALL be reported in the operator log at most.

#### Scenario: Slow browser
- **WHEN** a browser sends its request one byte a second
- **THEN** the avatar stays connected, and the control socket, the XMPP gateway and other pages are answered as before

### Requirement: A page that does not read its events
A page that does not take the events sent to it SHALL have its event stream closed once 256 events wait for it; nothing SHALL be kept back from other pages on its account.

#### Scenario: Browser stops reading
- **WHEN** a browser with an open page stops reading while chat keeps arriving
- **THEN** the avatar stays connected and seated, chat is still logged, other pages still get every line, and that browser's stream is closed

### Requirement: Limits on web connections
The bot SHALL serve at most 32 web connections at once, at most 8 of them event streams, and SHALL answer a further one as busy and close it. It SHALL close a connection whose request is not complete within 10 seconds. It SHALL refuse a request whose head is longer than 8192 bytes or whose body is longer than 16384 bytes, and carry out nothing of it.

#### Scenario: Silent client
- **WHEN** a client connects and sends nothing
- **THEN** the bot closes the connection after 10 seconds and keeps serving other clients

#### Scenario: Many connections
- **WHEN** 32 clients hold connections open and a further one connects
- **THEN** the further one is answered as busy, the avatar stays connected, and `rslbot ctl status` is still answered

#### Scenario: Oversized request
- **WHEN** a client sends a command of 100,000 bytes
- **THEN** it is refused and nothing is sent to the grid

### Requirement: Password and sessions
When the `[web]` table has a `password`, the bot SHALL serve to a browser without a session only what the login box needs, and SHALL carry out no command, open no event stream and send nothing of the bot's state for it. A browser that gives the password SHALL get a session; the password SHALL be compared in full and in a time that does not depend on how much of it was right.

#### Scenario: No session
- **WHEN** a password is configured and a browser without a session opens the page
- **THEN** it gets the login box, and a command it sends all the same is refused and not carried out

#### Scenario: Right password
- **WHEN** the operator types the configured password into the login box
- **THEN** the page shows its tabs, and stays logged in when it is loaded again

### Requirement: How long a session lasts
A session SHALL last until the browser logs out, until it has not been used for 30 days, or until the bot stops; the bot SHALL keep at most 32 sessions and SHALL end the one unused longest for a further one. Logging out SHALL end that browser's session only, and SHALL close its event streams.

#### Scenario: Two browsers
- **WHEN** the operator is logged in on a phone and on a desktop and logs out on the phone
- **THEN** the desktop stays logged in

#### Scenario: Bot restarted
- **WHEN** the bot is restarted while a page is open and logged in
- **THEN** the page shows the login box once the bot is back

#### Scenario: Thirty-third session
- **WHEN** 32 browsers have sessions and a further one logs in
- **THEN** it gets a session, and the session that was unused longest is ended

### Requirement: The session cookie and the password stay out of reach
The session SHALL be held in a cookie that the page's scripts cannot read and that the browser does not send with requests another site makes. The password SHALL be sent to the bot only when logging in, and the bot SHALL NOT send it, or anything it can be worked out from, to any browser.

#### Scenario: Cookie not readable by scripts
- **WHEN** a script running in the page, or a page of another site, looks for the session cookie
- **THEN** it does not find it

#### Scenario: Password not sent back
- **WHEN** a logged-in page asks the bot whether a login is needed, and loads the page's files
- **THEN** nothing it gets holds the password or anything it can be worked out from

### Requirement: Web interface without a password
Without a `password` the bot SHALL ask for none: every browser that reaches the address SHALL be served as one with a session.

#### Scenario: No password configured
- **WHEN** the `[web]` table has no `password`
- **THEN** the page shows its tabs at once, without a login box and without a way to log out

### Requirement: Wrong passwords are slowed down
After 5 wrong passwords within 60 seconds, the bot SHALL refuse every login, the right password included, until 60 seconds have passed since the last wrong one, and the page SHALL say that there were too many attempts. A wrong password SHALL NOT end or change any session.

#### Scenario: Guessing
- **WHEN** a client sends six wrong passwords within ten seconds and then the right one
- **THEN** the sixth and the right one are refused as too many attempts, and the right one is accepted a minute later

#### Scenario: Logged in meanwhile
- **WHEN** someone sends wrong passwords while the operator's page is open and logged in
- **THEN** the operator's page keeps working

### Requirement: Requests of other sites are refused
A command, a console line, a login or a logout that a page of another web site makes a browser send to the bot SHALL NOT be carried out, with or without a password and whether or not that browser has a session. A page of another site SHALL NOT be able to read an answer of the bot or its event stream. The bot SHALL NOT allow any other site to use it from a browser.

#### Scenario: Form on another site
- **WHEN** the operator, logged in to the bot, opens a page elsewhere that submits a form to the bot's address asking for `say hello`
- **THEN** the avatar says nothing

#### Scenario: Script on another site
- **WHEN** a script on a page elsewhere asks the bot for its events or sends it a command
- **THEN** the script gets no events and no answer, and the command is not carried out

### Requirement: Names the bot answers to without a password
While no `password` is configured, the bot SHALL serve only requests that name it, in their `Host` header, by an IP address or as `localhost`. A request that names it by any other host name SHALL be refused with a text that says to open the interface by IP address or to set `password` in the `[web]` table. With a `password` configured the bot SHALL serve requests under any name.

#### Scenario: By address
- **WHEN** no password is configured and the page is opened as `http://127.0.0.1:8080/`, `http://localhost:8080/` or `http://192.168.1.5:8080/`
- **THEN** it is served

#### Scenario: Name that points at the bot
- **WHEN** no password is configured and a browser sends the bot a request for the host `bot.attacker.example`, a name made to resolve to `127.0.0.1`
- **THEN** the request is refused and nothing is carried out

#### Scenario: Behind a name, with a password
- **WHEN** a password is configured and a reverse proxy passes on requests for `bot.example.org`
- **THEN** they are served

### Requirement: Commands from the page
A command the page sends, as a command name and its arguments, SHALL be carried out as the control socket carries out the same command (see `control-socket` and the capabilities its commands belong to), with the same answers and the same effects, and SHALL be answered to the page with the command's answer: success with its text, if it has one, or an error with its text.

#### Scenario: Status
- **WHEN** the page sends `status`
- **THEN** it gets the same text `rslbot ctl status` prints

#### Scenario: Command refused
- **WHEN** the page sends `sit ground` while the bot is between logins
- **THEN** it gets the error the control socket would give

#### Scenario: Argument with spaces and quotes
- **WHEN** the page sends `tp` with the arguments `128`, `128`, `25` and `Joe's "Blue" Lagoon`
- **THEN** the bot teleports the avatar to the region of exactly that name

#### Scenario: Quit
- **WHEN** the page sends `quit`
- **THEN** it gets the success answer, and the bot logs out and exits

### Requirement: Malformed commands from the page
The bot SHALL answer a command from the page with an error and do nothing else when the command name is unknown or the number or form of its arguments is one the control socket refuses.

#### Scenario: Unknown command
- **WHEN** the page sends the command `fly`, or `tp` with the one argument `128`
- **THEN** it gets an error, and nothing is sent to the grid

### Requirement: Order and waiting of commands from the page
Commands from the page SHALL be carried out in the order they arrive. An answer that takes time, such as a teleport's, SHALL NOT hold back other commands, the event stream or other pages. At most 16 commands of the web interface SHALL wait for their answers at once, and a further one SHALL be answered with an error saying the bot is busy. A command the bot has not answered within 75 seconds SHALL be answered with an error saying so.

#### Scenario: Teleport while chatting
- **WHEN** the page sends `tp 128 128 25 Home` and, while the teleport goes on, `say hi`
- **THEN** `say` is carried out or refused at once as it would be during a teleport, and the teleport's answer arrives when it ends

#### Scenario: Too many waiting
- **WHEN** 16 commands of the web interface wait for their answers and the page sends a seventeenth
- **THEN** the seventeenth is answered with an error saying that the bot is busy, and is not carried out

### Requirement: Console lines
A console line the page sends SHALL be read as one command line and carried out as the control contact of the XMPP gateway carries out the same line (see `xmpp-gateway`, "Control contact"): split into the command name and its arguments at spaces, tabs and line feeds, with a part between double quotes as one argument and `\"` and `\\` inside it; and `help` answered with the list of commands. The answer SHALL be the command's answer as "Commands from the page" gives it.

#### Scenario: Name with a space
- **WHEN** the console line is `im "Jane Doe" see you later`
- **THEN** Jane Doe receives the IM `see you later` and the answer names Jane Doe

### Requirement: Console lines without XMPP
Console lines SHALL work whether or not the config has an `[xmpp]` table.

#### Scenario: Help without XMPP
- **WHEN** the config has no `[xmpp]` table and the console line is `help`
- **THEN** the answer lists the commands

### Requirement: Console lines that are refused
A console line that ends inside double quotes or is longer than 8192 bytes SHALL be refused with an error, and nothing of it SHALL be carried out.

#### Scenario: Unclosed quote
- **WHEN** the console line is `im "Jane Doe hi`
- **THEN** the answer is an error about the quote and nothing is sent to the grid

#### Scenario: Line too long
- **WHEN** the console line is `say` followed by 9000 letters
- **THEN** the answer is an error and nothing is said

### Requirement: Requests the page makes by itself
The page SHALL mark as its own the commands it sends to keep a list up to date without the operator having asked for that list then: the `scan avatars` for the avatars in chat range and the `friends` for the friend list that it sends when their tab is shown, when it has found the bot again, and on a timer (see `web-ui`). Only `scan avatars` and `friends` SHALL be accepted as the page's own; any other command marked so SHALL be refused and not carried out.

#### Scenario: Something else marked as the page's own
- **WHEN** a client sends `say hello` marked as the page's own
- **THEN** it is refused and nothing is said

### Requirement: The page's own requests leave the override and the log alone
The bot SHALL carry out a command the page marked as its own (see "Requests the page makes by itself") as any other, with these differences: it SHALL NOT be the exempt command of an RLV override and SHALL leave RLV in override (see `rlv-restrictions`, "Override exempts one command"); and the operator log SHALL get no line for it, whatever its answer.

#### Scenario: Override kept
- **WHEN** the operator sends `rlv mode override` from the Console tab and the page then refreshes its list of the avatars in chat range by itself
- **THEN** `rlv status` still prints `rlv: override`, and the next command the operator gives is the exempt one

#### Scenario: Quiet log
- **WHEN** a page stays open on its Local chat tab for an hour
- **THEN** the operator log has no line for the scans the page sent by itself

#### Scenario: Refresh button
- **WHEN** the operator presses the page's button that refreshes the avatars in chat range
- **THEN** that `scan avatars chat` is logged and is the exempt command of an override, as one from `rslbot ctl` is

### Requirement: Event stream
To every open page that asks for it, the bot SHALL send, as it happens and without the page asking again, every local chat message and every instant message the message log records (see `message-log`), of every kind, `im_out` and `im_auto` included, as soon as it is recorded and in the order of recording, with its time, its kind, the avatar's name, display name and UUID, the object's name and UUID, its text, and whether it was stored while the avatar was offline.

#### Scenario: Chat heard
- **WHEN** Jane Doe says `hi all` near the avatar while a page is open
- **THEN** the page has the line within a second of its being recorded, without being loaded again

#### Scenario: IM sent from elsewhere
- **WHEN** the operator runs `rslbot ctl im "Jane Doe" hi` while a page is open
- **THEN** the page gets the `im_out` record, with Jane Doe's UUID

### Requirement: Every page gets every event
Every page with an open event stream SHALL get every event the bot sends.

#### Scenario: Two pages
- **WHEN** two browsers have the page open and an instant message arrives
- **THEN** both get it

### Requirement: Status on the event stream
On every open event stream the bot SHALL send the text of `status` (see `avatar-commands`, "Status command") when the stream opens and whenever that text changes, within one second of the change.

#### Scenario: Teleport
- **WHEN** the avatar arrives in another region, by a command from anywhere or by the bot's own doing
- **THEN** every open page gets the new status within a second

### Requirement: Events do not depend on the log files
Sending events SHALL NOT depend on the log files of the config, and SHALL change nothing of what is written to them, to standard error or to the XMPP owners.

#### Scenario: No log files
- **WHEN** the config has neither `log_chat` nor `log_im`
- **THEN** the pages get chat and instant messages all the same

### Requirement: Recent messages are kept for pages that open later
The bot SHALL keep, in memory only, the newest 500 local chat messages and the newest 500 instant messages it has recorded since it started, each count on its own. A page that opens its event stream SHALL first get the kept messages, oldest first, and then the new ones, without a gap and without one twice. The kept messages SHALL NOT be written to disk for the web interface and SHALL be gone when the bot exits. With no `[web]` table the bot SHALL keep none.

#### Scenario: Page opened later
- **WHEN** Jane Doe sent the avatar two instant messages an hour ago and the page is opened now
- **THEN** the page shows both, with the times they arrived

#### Scenario: Busy region
- **WHEN** 2000 chat lines and 3 instant messages have been recorded since the bot started
- **THEN** a page that opens gets the newest 500 chat lines and all 3 instant messages

### Requirement: A page that lost its stream catches up
A page whose event stream was broken and that says which event it got last SHALL get those after it; when the bot no longer keeps the one after it, or has been restarted since, the page SHALL be told to drop what it has and SHALL get the kept messages (see "Recent messages are kept for pages that open later") anew.

#### Scenario: Short loss of the connection
- **WHEN** the browser loses its connection for a minute during which three chat lines are heard
- **THEN** after it has found the bot again the page has those three lines, each once, in order

#### Scenario: Bot restarted
- **WHEN** the bot is restarted while a page is open
- **THEN** the page, once it has found the bot again, shows only what was recorded since the restart

### Requirement: The web server appears in the operator log
The bot SHALL write a line to its operator log for every login, with the address it came from; for every wrong password, with the address it came from; when logins are refused for too many attempts, once for each such time; for every logout; and for every request it refuses under "Requests of other sites are refused" or "Names the bot answers to without a password", with the address it came from and the name it was sent to.

#### Scenario: Wrong password logged
- **WHEN** a browser at `192.168.1.5` sends a wrong password
- **THEN** the log has a line saying so with `192.168.1.5`

#### Scenario: Refused name
- **WHEN** no password is configured and a request for the host `bot.attacker.example` is refused
- **THEN** the log has a line with the address the request came from and `bot.attacker.example`

### Requirement: Commands of the page in the operator log
Commands of the page SHALL be logged as the capabilities they belong to say, but for those of "Requests the page makes by itself". The bot SHALL NOT log one line per request served or per event sent.

#### Scenario: Reading the log after a session at the page
- **WHEN** the operator has logged in, teleported from the World tab and logged out
- **THEN** the log shows the login, the teleport and its outcome as for `rslbot ctl tp`, and the logout, and no line for the page's files or its event stream

### Requirement: The web interface's start line
The bot SHALL write a line to its operator log when the web interface starts listening, with its address and whether a password is asked for. When the web interface listens on an address that is not a loopback address and no password is configured, that line SHALL say that anyone who can reach that address controls the avatar.

#### Scenario: Starting
- **WHEN** the bot starts with `listen = "127.0.0.1:8080"` and a password
- **THEN** the log has one line with `127.0.0.1:8080` saying that a password is asked for

#### Scenario: Open to the network
- **WHEN** the bot starts with `listen = "0.0.0.0:8080"` and no password
- **THEN** the log says that anyone who can reach port 8080 of this machine controls the avatar, and the bot starts all the same
