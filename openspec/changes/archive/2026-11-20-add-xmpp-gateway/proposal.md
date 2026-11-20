# Proposal

## Why

The operator reaches a running bot only through `rslbot ctl` on the same machine, and reads what the avatar hears only in stderr and the TSV logs. Talking with several friends, following local chat and steering the bot from a phone calls for an ordinary chat client: the bot as a Jabber/XMPP gateway, where every avatar is a contact of its own and the operator's XMPP server keeps the history, the offline messages and the push to every device. Several bots on several grids should be reachable from the same XMPP account, each under an address of its own.

## What Changes

- The bot can act as an XMPP external component (XEP-0114): it connects to an XMPP server on the same host, Prosody or ejabberd, and owns one domain, such as `russel.sl.example.org`. A new `[xmpp]` table in the config names that domain, the server's component port, the shared secret and the XMPP accounts that own the bot. Several bots, on one grid or several, are several components with domains of their own on the same server.
- Only the owners' accounts may talk to the component; everyone else is refused.
- **Instant messages** become one-to-one chats. An IM from an avatar arrives from a contact address made of the avatar's name, such as `jane.doe@russel.sl.example.org`, with the display name as nickname; a message to such an address is sent as an IM. Replies go to the UUID the address stands for, so avatars that the grid's search cannot find, such as hypergrid visitors, can be answered. An address the bot has not seen is resolved as the `im` command resolves a name. IMs from objects arrive from an address per object and cannot be answered. Offline IMs arrive with the time they were sent.
- **Local chat** is a plain contact, `local@<domain>`: every line the avatar hears arrives as `Name: text`, and every message to it is said as `say` says it, so `/shout`, `/5 text` and `//` work as in a viewer's chat bar. A group chat room for local chat is left for a later change.
- **Control**: a message to the bare domain is a command line, read as `rslbot ctl` reads its arguments, with double quotes around an argument that holds spaces. It is answered with the command's text, or `ok`, or the error. The domain's presence shows the bot's state, with the text of `status`.
- Messages recorded while the component is not connected wait, up to a bound, and are delivered with their time once it is connected again.
- The component connects at startup, before the first login, and connects again by itself when the connection is lost; it never delays the grid session, and its troubles never end the session or the bot.

Not in this change: friends' online status and the friend list, a group chat room for local chat or for SL groups, the avatars that are near, profile pictures, ad-hoc command forms, delivery receipts, copying IMs sent with `rslbot ctl` into the XMPP conversation, TLS on the component connection, and a component on another host than the server.

## Capabilities

### New Capabilities

- `xmpp-gateway`: the XMPP component: connecting and connecting again, who may use it, contact addresses for avatars and objects, instant messages both ways, the local chat contact, the control contact with its command lines and presence, the messages that wait while it is not connected, and what it never does to the grid session.

### Modified Capabilities

- `bot-config`: the `[xmpp]` table with `component`, `server`, `secret` and `owners`, its validation, and the secret kept out of output like the password.
- `message-log`: the bot sends chat and instant messages only on an operator's command, and a message from an owner through the XMPP gateway is such a command.

## Impact

- New `src/xmpp.rs` (or a small module tree): the component connection with its handshake, reader and writer threads, the stanzas it reads and writes, the address mapping, command-line splitting, the queue of waiting messages.
- `src/main.rs`: a new `Event` for what the component reads; commands from XMPP enter as `Event::Control` does, with an answer that goes back to XMPP; recorded messages are handed to the component where they are written to the logs; presence follows the reconciler and the session.
- `src/msglog.rs`: a `Record` keeps the sending avatar's UUID, which the session already knows, so that a reply can go to it. The TSV files do not change.
- `src/control.rs`: the part of `request_line` that turns arguments into a request is shared with XMPP command lines.
- `src/config.rs`, `russelbot.example.toml`, `README.md`: the `[xmpp]` table, a Prosody example, the addresses.
- New dependencies: an incremental XML reader (`quick-xml`) and SHA-1 for the component handshake (`sha1`); no XMPP library and no async runtime.
- `docs/test-rig.md` and `test-rig/`: a local Prosody beside the local OpenSim, and scenarios for IMs, local chat, commands, reconnection and refusal of strangers.
