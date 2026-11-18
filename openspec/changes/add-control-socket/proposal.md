# Proposal

## Why

Once started, the bot can only be stopped: there is no way to make the avatar say something or message someone without logging in with a viewer, which kicks the bot off. A local control socket lets the operator, a script or a cron job drive the running bot from the command line.

## What Changes

- New config key `socket` (boolean, default false). When true the bot listens on a local control socket for the whole run.
- The socket is a Unix domain socket at `~/russelbot-ctl` on Unix and a loopback TCP socket at `127.0.0.1:47115` on Windows. The environment variable `RUSSELBOT_SOCK` overrides the address for both the bot and the client: a file path on Unix, `host:port` on Windows.
- New subcommand `rslbot ctl <command> ...` that connects to the socket, sends one command, prints the bot's answer and exits with a status that says whether the command worked.
- First two commands:
  - `rslbot ctl say <message>` sends local chat. The message is parsed the way a viewer's chat bar parses it: `/1 test message` goes to channel 1, `//text` reuses the last channel, `/shout` and `/whisper` change the chat type, anything else (including `/me ...`) is ordinary chat on channel 0.
  - `rslbot ctl im <avatar name> <message>` sends an instant message to an avatar given by name or UUID. The bot looks the name up on the grid first.
- Instant messages the bot sends are recorded like the ones it receives, on standard error and in the `log_im` file, with the new kind `im_out` and the recipient's names in the avatar columns.
- A command that cannot be carried out (not connected, unknown avatar, message too long, unknown command) is answered with an error and changes nothing.

Assumptions recorded here because the request left them open:

- The avatar name is one command-line argument: `"John Doe"` in quotes, `john.doe`, a single word (last name `Resident`) or a UUID. Everything after it is the message; several message arguments are joined with single spaces.
- The chat the avatar says on channel 0 is echoed by the simulator and reaches `log_chat` as any other chat does; chat on other channels is not echoed and appears in the operator log only.
- No authentication beyond the operating system's: the Unix socket file is accessible to its owner only; the Windows TCP socket is reachable by every local user.

## Capabilities

### New Capabilities
- `control-socket`: the listening socket, its address and lifetime, the request/response exchange, and the `rslbot ctl` client.
- `message-send`: what the `say` and `im` commands send to the grid, including chat-bar parsing and avatar name resolution.

### Modified Capabilities
- `bot-config`: adds the `socket` key and its validation, including a socket that cannot be set up.
- `message-log`: records outgoing instant messages as kind `im_out`; the rule that the bot never sends messages becomes "only on an operator's command".

## Impact

- `src/config.rs`: `socket` key.
- `src/main.rs`: `ctl` subcommand, usage text, control events in the main loop, argument handling that leaves everything after `ctl` untouched.
- New `src/control.rs`: socket address, listener thread, wire format, client.
- `src/net.rs`: a new `Event` variant carrying a control request and its reply channel.
- `src/session.rs`, `src/udp/messages.rs`: sending `ChatFromViewer` and `ImprovedInstantMessage`, sending `AvatarPickerRequest` and decoding `AvatarPickerReply`.
- `src/msglog.rs`: `im_out` records for sent instant messages.
- `README.md`, `russelbot.example.toml`: document the key, the environment variable and the commands.
- No new dependencies. Windows support is compiled conditionally and can only be build-checked from this machine.
