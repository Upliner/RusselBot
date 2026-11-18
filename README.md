A simple lightweight bot software for Second Life and OpenSimulator.
Lightweight replacement for .NET/Mono bot software.

First version follows only one single simple scenario:
* Connect to the grid, connect to the region
* If current region is not the target region, issue teleport command to the target destination
* Issue "sit" command on a target object.
* Keep the avatar alive, respond all necessary server queries to keep it online.

It requires one single config file to run with following parameters:
* Grid URL (Second Life Agni by default)
* User name
* Password or password hash if grid can sign in by hash. In case of hash we need a separate tool for hash generation
* Region name and coordinates for teleport command
* UUID of the object to sit on

## Status

Verified by unit tests and against a local OpenSim, including kicks, region
restarts, simulator outages and being unseated or teleported away; see
`docs/test-rig.md` for the rig and the results. Chat and IM logging is
verified there too, except IMs from scripts and delivery of offline IMs,
which the rig cannot produce. On Second Life, checked by hand on 2026-11-18:
login, sitting, logging of chat and IMs from avatars and from objects, and
fetching offline IMs (delivered with the time they were sent). Multi-factor login is not supported: an account that has it enabled
makes the bot exit with an error.

## Build

Needs Rust 1.100 (pinned in `rust-toolchain.toml`; rustup picks it up).

```sh
cargo build --release      # binary: target/release/slbot
cargo test
```

## Run

```sh
cp slbot.example.toml slbot.toml   # then edit it
target/release/slbot               # reads ./slbot.toml
target/release/slbot path/to/other.toml
target/release/slbot --verbose     # also log every simulator message
```

The bot keeps working towards "logged in, in the target region, seated on
the target object". It logs in again after a kick, a region restart or a
network outage, teleports back when moved, and sits again when unseated, each
with growing delays between attempts. Everything it does is logged to
standard error with UTC timestamps.

Stop it with Ctrl-C or SIGTERM: it logs out and exits. A second signal exits
immediately.

| Exit status | Meaning |
|---|---|
| 0 | Stopped by a signal |
| 1 | Gave up: the grid rejected the credentials, or demands something the bot cannot supply |
| 2 | Bad command line or config file |

## Config

One TOML file; see `slbot.example.toml`.

| Key | Required | Meaning |
|---|---|---|
| `login_url` | no | Grid login URL. Default: `https://login.agni.lindenlab.com/cgi-bin/login.cgi` (Second Life) |
| `user` | yes | `First Last`, or a single word (last name `Resident`) |
| `password` | one of the two | Plain password |
| `password_hash` | one of the two | Output of `slbot hash` |
| `region` | yes | Target region name |
| `position` | yes | `[x, y, z]` inside the region; pick a spot close to the object |
| `object` | yes | UUID of the object to sit on |
| `log_chat` | no | File that receives local chat, see [Chat and IM logs](#chat-and-im-logs) |
| `log_im` | no | File that receives instant messages |
| `fetch_offline_im` | no | `true` to also fetch IMs stored while the avatar was offline. Default: `false` |

To keep the plain password out of the file:

```sh
target/release/slbot hash          # type the password; prints $1$...
printf 'secret\n' | target/release/slbot hash
```

Put the printed value in `password_hash` and leave `password` out.

## Chat and IM logs

The bot reads local chat the avatar hears and instant messages sent to it,
from avatars and from objects, and writes each one to standard error. It
never answers. Group chat, offers, invitations and typing notifications are
not recorded.

With `log_chat` and/or `log_im` set, each message is also appended to that
file as one tab-separated line (UTF-8). The file is created when missing and
gets a header line when it is empty; relative paths are relative to the
directory the bot is started in. A file that cannot be opened makes the bot
exit with status 2 before it logs in.

| Column | Content |
|---|---|
| `time` | UTC, `YYYY-MM-DDTHH:MM:SSZ`: when the bot received the message, or when it was sent for an IM stored offline |
| `kind` | Chat: `whisper`, `say`, `shout`, `region`, `owner`, `direct`, `debug`. IM: `im` (from an avatar), `object` (from a script) |
| `avatar_name` | The speaker; for an object, its owner |
| `display_name` | That avatar's display name |
| `object_name` | The object's name; empty for an avatar |
| `object_uuid` | The object's UUID; empty for an avatar |
| `text` | The message |

Inside a field, tab, line feed, carriage return and backslash are written as
`\t`, `\n`, `\r` and `\\`. Display names and owners' names are looked up on
the grid; a name the grid does not supply within 5 seconds is left empty, as
are the owner columns of a group-owned object.

`fetch_offline_im = true` makes the bot ask, after each login, for the IMs
stored while the avatar was offline. Fetching removes them from the grid: a
viewer logging in as this avatar afterwards will not see them.

The bot logs in with `agree_to_tos` set, so it accepts a grid's terms of
service on the account's behalf, including changed terms, without showing
them to anyone.
