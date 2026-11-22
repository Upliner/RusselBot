# Proposal

## Why

The operator reaches a running bot through `rslbot ctl` on the same machine or, with an XMPP server set up beside it, through a chat client. Both are lines of text: the result of a scan is a table to read UUIDs from and type them back, the inventory is a listing of paths, and neither shows at a glance where the avatar is and who is around it. A page the bot serves itself gives the operator what a viewer's side panels give, in any browser, on a phone as well, with nothing to install beside the bot and no second daemon.

## What Changes

- The bot can serve a **web interface** over HTTP. A new `[web]` table in the config names the address to listen on (`listen`) and, optionally, a `password`. Without the table the bot listens on nothing. The page, its script and its style are part of the binary; the page loads nothing from anywhere else.
- The page has six tabs:
  - **World**: the current status (connected, region, position, seat, mode), kept up to date by the bot; a teleport form with a box each for X, Y, Z and the region name; a scan of the avatars in the region and a scan of the objects around the avatar, each row with buttons: teleport to an avatar, teleport to an object, sit on it, touch it. A button to stand up.
  - **Local chat**: the same status, the avatars within chat range, and local chat as it is heard, live, with a box to speak that reads its text as a viewer's chat bar does.
  - **IM**: every conversation the bot has had since it started, one per avatar or object, with the messages in both directions; a box to open a conversation with anyone by name; the friend list with who is online, to open one from.
  - **Inventory**: the inventory as a tree that reads a folder when it is opened, with attach, detach, wear, take off and, for what is attached, touch.
  - **Console**: a command line that takes what the XMPP control contact takes and prints the answers.
  - **Settings**: the theme of the page: as the browser asks for (light or dark, the default), always light, or always dark. Kept in the browser.
- **Password**: with `password` set, the page shows a login box and nothing else until the password is given, and no command is carried out and nothing is shown without it. Without it the interface is open to whoever reaches the address, which is meant for an address on the same machine.
- **Commands** from the page are carried out exactly as the control socket carries out the same commands, RLV restrictions, refusals and logging included. The page adds no ability the commands do not have.
- **Live updates**: the bot sends each open page every chat line and instant message as it is recorded, and the status whenever it changes. It keeps the newest 500 chat lines and 500 instant messages in memory, so a page that is opened later, or opened again, has the recent past. Nothing of it is written to disk.
- A page of another web site cannot make the operator's browser send commands to the bot or read from it.
- `scan` gets a switch `--position` that adds each object's position to its line, which the page needs to teleport to an object.
- `touch` also takes the UUID of an inventory item that is attached to the avatar, and then touches that attachment.
- Lists the page refreshes by itself (the avatars in chat range, the friends) do not use up an RLV override and do not fill the operator log.

Not in this change: TLS (a reverse proxy in front of the bot provides it), several accounts or read-only access, history kept across restarts, group chats, profile and texture pictures, a map, a panel for the teleport offers and permission requests that wait (the Console has `perms status` and `accept`), buttons for animations and sounds, drag and drop in the inventory, and browser notifications.

## Capabilities

### New Capabilities

- `web-server`: the HTTP side of the bot: listening and its limits, the password and the sessions, refusing other sites, carrying out the page's commands and console lines, the event stream with the messages kept for pages that open later, the requests the page makes by itself, and what the server never does to the grid session.
- `web-ui`: the page: its tabs and what each shows and does, how results and errors are shown, the login box, the theme, what happens when the bot cannot be reached, and that text from the grid is never read as markup.

### Modified Capabilities

- `bot-config`: the `[web]` table with `listen` and `password`, its validation, an address that cannot be listened on as a config error, and the password kept out of output.
- `message-log`: the bot sends chat and instant messages only on an operator's command, and a command from the web interface is such a command.
- `area-scan`: the `--position` switch and the eighth field it adds to a line.
- `avatar-commands`: `touch` with the UUID of an attached inventory item.
- `rlv-restrictions`: a command the page sends by itself is never the exempt command of an override and leaves the override in place.
- `avatar-scan`, `friend-list`: a `scan avatars` or `friends` the page sends by itself gets no line in the operator log.

## Impact

- New `src/web/`: the listener and its connection threads, a small HTTP/1.1 reader and writer, sessions and the login throttle, the hub that keeps recent records and feeds the event streams, and the page's three files (`index.html`, `app.js`, `style.css`) built into the binary with `include_str!`.
- `src/net.rs`, `src/main.rs`: new `Event`s for a page's command and for a page that subscribes; records are handed to the hub where they are written to the logs and to XMPP; the status text is published when it changes; `Bot::control` learns of commands that are the page's own.
- `src/config.rs`, `russelbot.example.toml`: the `[web]` table.
- `src/scan.rs`: the `--position` switch. `src/commands.rs`, `src/session.rs`: `touch` by attached item. `src/control.rs`: the usage text for both. `src/xmpp/command.rs`: `help` is shared with the web console.
- `README.md`: a "Web interface" section (setting it up, the tabs, the password, a reverse proxy for TLS), the config key table, the Status section. `docs/test-rig.md` and `test-rig/`: items for the web interface and a helper to drive it without a browser.
- Dependencies: `getrandom` for session tokens, which is already in the tree through the TLS stack of `ureq`. No HTTP server crate, no async runtime, no JSON crate; no JavaScript framework and no build step for the page.
