# Proposal

## Why

The bot sits in a region but is deaf: local chat and instant messages addressed to its avatar are received and thrown away. An operator running it unattended has no way to see what was said near the avatar or sent to it, including what the seat's own scripts tell it.

## What Changes

- The bot reads local chat heard by the avatar and instant messages sent to it, from avatars and from objects.
- Every such message is written to standard error as one log line, whether or not any file is configured.
- New optional config key `log_chat`: path of a TSV file that receives one row per local chat message.
- New optional config key `log_im`: path of a TSV file that receives one row per instant message.
- Both files share one layout: timestamp, kind, avatar name, display name, object name, object UUID, message text. For a message from an object the avatar columns hold the object's owner.
- The bot looks up display names (and owners' names) from the grid to fill those columns.
- New optional config key `fetch_offline_im`, off by default: when on, the bot asks the grid after each login for instant messages stored while the avatar was offline and logs them like any other. This removes them from the grid's store.
- The bot stays read-only: it does not chat, reply to IMs, or act on what it reads.

## Capabilities

### New Capabilities
- `message-log`: reading local chat and instant messages, resolving the names that go with them, and recording them on standard error and in the optional TSV files.

### Modified Capabilities
- `bot-config`: three new optional keys (`log_chat`, `log_im`, `fetch_offline_im`) and their validation.

## Impact

- `src/udp/messages.rs`: decode `ChatFromSimulator` and `ImprovedInstantMessage`; encode `RetrieveInstantMessages`.
- `src/session.rs`: new session events for chat and IMs; the offline-IM request after login.
- `src/caps.rs`, `src/net.rs`: the seed request also asks for the display-name capability and reports its URL.
- New modules for the name lookup (HTTP, on its own thread) and for the message recorder (queueing, formatting, files).
- `src/config.rs`, `src/main.rs`: new keys, opening the log files at startup, wiring the above into the main loop.
- `README.md`, `slbot.example.toml`: document the keys and the file layout.
- No new dependencies. No change to login, teleport or sit behaviour.
