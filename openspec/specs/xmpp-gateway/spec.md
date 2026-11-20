# xmpp-gateway Specification

## Purpose

Lets the operator use the bot from an ordinary XMPP (Jabber) client: the bot connects to an XMPP server on the same host as an external component, every avatar it exchanges instant messages with becomes a contact, local chat is a contact of its own, and a further contact takes the commands that `rslbot ctl` takes.

## Requirements

### Requirement: Component connection
When the config has an `[xmpp]` table, the bot SHALL connect to the XMPP server at its `server` address as an external component (XEP-0114) for the domain `component`, authenticating with `secret`. The bot SHALL start connecting before its first login attempt and SHALL keep the component connected until it exits, whether or not the avatar is connected to a grid. When the connection cannot be made, is refused, or is lost, the bot SHALL write one line to its operator log with the reason and SHALL try again after a delay that starts at 5 seconds and doubles with every failure in a row up to 5 minutes; the delay SHALL start again at 5 seconds after a connection the server accepted. A refusal of the handshake, such as a wrong secret or another component already holding the domain, SHALL be treated the same way and SHALL NOT end the bot. When the bot stops, a connected component SHALL send the owners unavailable presence for its addresses and close the stream before the bot exits. Without an `[xmpp]` table the bot SHALL NOT open any XMPP connection.

#### Scenario: Server up at startup
- **WHEN** the bot starts with an `[xmpp]` table and the XMPP server accepts the component
- **THEN** the operator log says the component is connected, with its domain, before or during the first login

#### Scenario: Server comes up later
- **WHEN** the bot starts while nothing listens on the `server` address, and the XMPP server is started two minutes later
- **THEN** the avatar logs in and stays online meanwhile, the operator log shows the failed attempts at growing intervals, and the component is connected within 5 minutes of the server starting

#### Scenario: Wrong secret
- **WHEN** the XMPP server refuses the handshake
- **THEN** the operator log names the domain and the server's reason without the secret, the bot keeps running, and it tries again after the delay

#### Scenario: Server restarted
- **WHEN** the XMPP server is restarted while the bot runs
- **THEN** the bot connects again within 10 seconds of the server accepting connections, without the avatar logging out

#### Scenario: Bot stopped
- **WHEN** the bot is stopped while its component is connected
- **THEN** the owners see the bot's addresses go offline

### Requirement: The gateway never disturbs the bot
Connecting, reading and writing the component connection SHALL NOT delay the bot's simulator traffic. A slow, silent or misbehaving XMPP server or client SHALL NOT make the bot exit, lose its session, or stop recording messages and answering the control socket. A failure on the component connection SHALL be reported in the operator log at most.

#### Scenario: XMPP server stalls
- **WHEN** the XMPP server stops reading from the component connection while chat keeps arriving
- **THEN** the avatar stays connected and seated, chat is still logged, and `rslbot ctl status` is still answered

#### Scenario: Malformed stanza
- **WHEN** the server sends something the bot cannot read as XML
- **THEN** the bot closes that connection, logs the reason, connects again after the delay, and the avatar stays connected

### Requirement: Only owners use the gateway
The bot SHALL carry out messages and subscription requests only from an entity whose bare address, compared without regard to letter case, is one of `owners`; the resource of the sender SHALL NOT matter. A message from anyone else SHALL be answered with a `forbidden` error and SHALL NOT reach the grid, a subscription request from anyone else SHALL be answered with `unsubscribed`, and the operator log SHALL get one line naming the sender. Service discovery and ping requests SHALL be answered to anyone. An error stanza SHALL never be answered.

#### Scenario: Stranger writes to a contact
- **WHEN** `someone@elsewhere.org` sends `hello` to `jane.doe@russel.sl.example.org`
- **THEN** they get a `forbidden` error, Jane Doe receives nothing, and the operator log names `someone@elsewhere.org`

#### Scenario: Stranger sends a command
- **WHEN** `someone@elsewhere.org` sends `quit` to `russel.sl.example.org`
- **THEN** they get a `forbidden` error and the bot keeps running

#### Scenario: Owner from any device
- **WHEN** `Me@Example.org/phone` sends a message and `me@example.org` is an owner
- **THEN** the message is carried out

### Requirement: Contact addresses
Each avatar SHALL have a contact address in the component's domain whose local part is made from its legacy name, in lower case: `first` when the last name is `Resident`, and `first.last` otherwise, with the characters that XMPP does not allow in a local part escaped as XEP-0106 escapes them. The local part SHALL instead be the avatar's UUID, in lower case with hyphens, when the avatar's name is not known or when the name would give `local`, a UUID, or a local part that starts with `obj-`. An object SHALL have the address `obj-<uuid>` with the object's UUID. The bare domain SHALL be the control contact and `local` the local chat contact.

The bot SHALL remember, while it runs, the UUID behind every avatar address it has used. A message an owner sends to an avatar address SHALL go to:
1. the avatar of the UUID, when the local part is a UUID;
2. the remembered UUID, when the bot has used that address;
3. otherwise the avatar the `im` command finds for the name the local part stands for, after undoing the XEP-0106 escapes, with the rules and answers of `message-send` ("Instant message recipients").

#### Scenario: Ordinary name
- **WHEN** an IM arrives from `Jane Doe`
- **THEN** it comes from `jane.doe@<domain>`

#### Scenario: Resident
- **WHEN** an IM arrives from `bob Resident`
- **THEN** it comes from `bob@<domain>`

#### Scenario: Hypergrid visitor
- **WHEN** an IM arrives from an avatar whose first name is `Ann.Lee` and last name `@osgrid.org:8002`
- **THEN** it comes from `ann.lee.\40osgrid.org\3a8002@<domain>`, and the owner's reply to that address reaches the same avatar though the grid's search does not know the name

#### Scenario: Name not known
- **WHEN** an IM arrives from an avatar whose name the grid did not give within 5 seconds
- **THEN** it comes from `<uuid>@<domain>` and replies to that address reach the avatar

#### Scenario: Avatar called local
- **WHEN** an IM arrives from `local Resident`
- **THEN** it comes from the avatar's UUID address, not from `local@<domain>`

#### Scenario: Owner starts a conversation
- **WHEN** an owner sends `hi` to `jane.doe@<domain>`, an address the bot has not used, and the grid knows Jane Doe
- **THEN** Jane Doe receives the IM `hi`

### Requirement: Instant messages to the owners
While the component is connected, every instant message the message log records as received, of kind `im` or `object`, SHALL be sent to every owner's bare address as a message of type `chat` with the text as body, as soon as it is recorded, in the order of recording. A message from an avatar SHALL come from that avatar's contact address and carry as nickname (XEP-0172) the display name, or the legacy name when there is no display name. A message from an object SHALL come from the object's address and carry the object's name as nickname; one from an object whose UUID the simulator did not give, and one from the grid itself, SHALL come from the control contact with the sender's name and `: ` in front of the text. An instant message stored while the avatar was offline SHALL carry the time it was sent as a delay (XEP-0203). Instant messages the bot sent (`im_out`) SHALL NOT be sent to the owners.

#### Scenario: IM from an avatar
- **WHEN** Jane Doe, display name `Janey`, sends the avatar `hello`
- **THEN** each owner receives `hello` from `jane.doe@<domain>` with the nickname `Janey`

#### Scenario: IM from an object
- **WHEN** an object named `Collar` sends the avatar an IM with `locked`
- **THEN** each owner receives `locked` from `obj-<uuid>@<domain>` with the nickname `Collar`

#### Scenario: Offline IM
- **WHEN** the bot fetches an IM that Jane Doe sent at 09:00 while the avatar was offline
- **THEN** the owners receive it from `jane.doe@<domain>` marked as sent at 09:00

#### Scenario: IM sent through the control socket
- **WHEN** the operator runs `rslbot ctl im "Jane Doe" hi`
- **THEN** no message is sent to the owners

### Requirement: Instant messages from the owners
A message with a body that an owner sends to an avatar address SHALL be sent as an instant message to the avatar that address stands for, as the `im` command sends it, with the body as text; it SHALL be recorded as kind `im_out` as `message-log` describes. When the `im` command would answer with an error, the bot SHALL answer the owner's message with an error stanza holding that error's text and send nothing. A message to an object's address SHALL be answered with an error saying that objects cannot receive instant messages. Messages without a body, such as chat state notifications, SHALL be ignored.

#### Scenario: Reply to an avatar
- **WHEN** an owner sends `see you` to `jane.doe@<domain>` while the avatar is in a region
- **THEN** Jane Doe receives the IM `see you` and the IM log gets an `im_out` row for Jane Doe

#### Scenario: Avatar not in a region
- **WHEN** an owner sends a message to `jane.doe@<domain>` while the bot is between logins
- **THEN** the owner's client shows the message as failed with the reason, and nothing is sent

#### Scenario: Unknown name
- **WHEN** an owner sends a message to `no.body@<domain>` and the grid knows no such avatar
- **THEN** the message is answered with an error naming `no.body` and nothing is sent

#### Scenario: Reply to an object
- **WHEN** an owner sends a message to `obj-<uuid>@<domain>`
- **THEN** the message is answered with an error and nothing is sent to the grid

#### Scenario: Typing notification
- **WHEN** an owner's client sends a chat state without a body to `jane.doe@<domain>`
- **THEN** nothing is sent to the grid and nothing is answered

### Requirement: Local chat contact
While the component is connected, every local chat message the message log records SHALL be sent to every owner's bare address from `local@<domain>`, as a message of type `chat`, as soon as it is recorded and in order. The body SHALL be the speaker's name, `: ` and the text. The speaker's name SHALL be the display name followed by the legacy name in parentheses when both are known and differ, the legacy name otherwise, and the object's name for an object. A message of kind `whisper` or `shout` SHALL have ` whispers` or ` shouts` after the name; a message of another kind than `say` SHALL have the kind in square brackets after the name, such as `Collar [owner]: text`. A text that starts with `/me ` or `/me'` SHALL be written as the name followed by what comes after `/me`, without `: `. Chat the avatar itself said SHALL be sent like any other chat it hears.

A message with a body that an owner sends to `local@<domain>` SHALL be carried out as the `say` command with the body as its message, so that the chat-bar forms of `message-send` apply. When `say` would answer with an error, the owner's message SHALL be answered with an error stanza holding that text.

#### Scenario: Chat heard
- **WHEN** Jane Doe, display name `Janey`, says `hi all` near the avatar
- **THEN** the owners receive `Janey (Jane Doe): hi all` from `local@<domain>`

#### Scenario: Shout and emote
- **WHEN** `bob Resident` shouts `HEY` and then says `/me waves`
- **THEN** the owners receive `bob Resident shouts: HEY` and then `bob Resident waves`

#### Scenario: Object speaking to its owner
- **WHEN** an object named `Collar` sends the avatar `llOwnerSay` text `ready`
- **THEN** the owners receive `Collar [owner]: ready` from `local@<domain>`

#### Scenario: Owner speaks in local chat
- **WHEN** an owner sends `hello everyone` to `local@<domain>`
- **THEN** the avatar says `hello everyone` on channel 0, and the owners then receive the avatar's own line as the simulator delivers it

#### Scenario: Chat bar forms
- **WHEN** an owner sends `/shout come here` and then `/7 open` to `local@<domain>`
- **THEN** the avatar shouts `come here` on channel 0 and then says `open` on channel 7

#### Scenario: Too long
- **WHEN** an owner sends a message of 2000 bytes to `local@<domain>`
- **THEN** the message is answered with an error and nothing is said

### Requirement: Control contact
A message with a body that an owner sends to the bare domain SHALL be read as one command line and carried out as the control socket carries out the same command (see `control-socket` and the capabilities its commands belong to), with the same answers and the same effects. The body SHALL be split into the command name and its arguments at spaces, tabs and line feeds; a part between double quotes SHALL be one argument and keep its spaces, and inside it `\"` SHALL stand for a double quote and `\\` for a backslash. The arguments SHALL then be passed on as `rslbot ctl` passes on its own, so that the words of a `say` message or of an `im` message are joined with single spaces. The bot SHALL answer every command line with one message from the bare domain: the answer's text when it has one, `ok` when it has none, and `error: ` followed by the text for an error. A body that ends inside double quotes, or is longer than 8192 bytes, SHALL be answered with an error and carry out nothing. The body `help` SHALL be answered with the list of commands. Commands from several messages SHALL be carried out in the order they arrive, and an answer that takes time, such as a teleport's, SHALL NOT hold back the messages of the other contacts; at most 16 commands SHALL wait for their answers at once, and a further one SHALL be answered with an error saying the bot is busy.

#### Scenario: Status
- **WHEN** an owner sends `status` to `russel.sl.example.org`
- **THEN** the owner receives the same text `rslbot ctl status` prints

#### Scenario: Name with a space
- **WHEN** an owner sends `im "Jane Doe" see you later`
- **THEN** Jane Doe receives the IM `see you later` and the owner receives the answer naming Jane Doe

#### Scenario: Command refused
- **WHEN** an owner sends `sit ground` while the bot is between logins
- **THEN** the owner receives `error: ` followed by the reason the control socket would give

#### Scenario: Unclosed quote
- **WHEN** an owner sends `im "Jane Doe hi`
- **THEN** the owner receives an error about the quote and nothing is sent to the grid

#### Scenario: Teleport while chatting
- **WHEN** an owner sends `tp 128 128 25 Home` and, while the teleport goes on, sends `hi` to `jane.doe@<domain>`
- **THEN** the IM is carried out or refused at once as the `im` command would be during a teleport, and the teleport's answer arrives when it ends

#### Scenario: Quit
- **WHEN** an owner sends `quit`
- **THEN** the owner receives `ok`, the bot logs out, and its addresses go offline

### Requirement: Presence and subscriptions
The bot SHALL approve a subscription request from an owner to any address of its domain with `subscribed`, and SHALL answer an owner's unsubscription with `unsubscribed`. It SHALL send presence to every owner for the bare domain and for `local@<domain>` when the component connects, when it stops, and whenever the state below changes, and SHALL answer an owner's presence probe for them with the current state:

| Bot state | Bare domain | `local@<domain>` |
|---|---|---|
| The avatar is in a region | available, with the region's name as status | available |
| Any other state while the bot runs | away, with status `not in a region` | unavailable |
| The bot stopping | unavailable | unavailable |

Avatar and object addresses SHALL be answered as unavailable.

#### Scenario: Adding the bot
- **WHEN** an owner adds `russel.sl.example.org` to their contacts while the avatar is in region `Home`
- **THEN** the subscription is approved and the contact shows as online with the status `Home`

#### Scenario: Teleport
- **WHEN** the avatar teleports from `Home` to `Away`
- **THEN** the bare domain's status changes to `Away` once the avatar has arrived

#### Scenario: Grid lost
- **WHEN** the grid session is lost and the bot waits to log in again
- **THEN** the bare domain shows as away with `not in a region` and `local@<domain>` as offline

### Requirement: Messages wait while the component is disconnected
Messages for the owners that arise while the component is not connected, recorded instant messages, local chat and answers to commands alike, SHALL be kept, up to the newest 500, and SHALL be sent in their order as soon as the component is connected again, each carrying as a delay (XEP-0203) the time it was recorded, or for an offline IM the time it was sent. When messages are dropped for the bound, the operator log SHALL say how many, once for each time the connection was down.

#### Scenario: XMPP server restart during chat
- **WHEN** the XMPP server is down for a minute during which Jane Doe sends two IMs
- **THEN** after the component is connected again the owners receive both IMs in order, each marked with the time it arrived

#### Scenario: Long outage
- **WHEN** 600 chat lines are heard while the XMPP server is down
- **THEN** the owners receive the newest 500 once it is up, and the operator log says 100 were dropped

### Requirement: Service discovery and ping
The bot SHALL answer a service discovery information request (XEP-0030) to the bare domain with an identity of category `gateway` and the features it supports, a ping (XEP-0199) to any of its addresses with a result, and any other request of type `get` or `set` with a `service-unavailable` error.

#### Scenario: Client discovers the gateway
- **WHEN** an XMPP client asks the bare domain for its disco information
- **THEN** it receives an identity of category `gateway` and a feature list that includes ping

#### Scenario: Unknown request
- **WHEN** an owner's client sends a vCard request to `jane.doe@<domain>`
- **THEN** it receives a `service-unavailable` error
