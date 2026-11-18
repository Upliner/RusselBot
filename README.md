RusselBot is a lightweight bot software for Second Life and OpenSimulator written in Rust,
replacement for heavy .NET/Mono bots. Requires just a few MBs of memory which makes it possible
to run even on cheapest VPS machines without requiring to run heavy Mono runtime.

First version follows only one single simple scenario:
* Connect to the grid, connect to the region
* If current region is not the target region, issue teleport command to the target destination
* Issue "sit" command on a target object.
* Keep the avatar alive, respond all necessary server queries to keep it online.

## Status

Verified by unit tests and against a local OpenSim, including kicks, region
restarts, simulator outages and being unseated or teleported away; see
`docs/test-rig.md` for the rig and the results. Chat and IM logging is
verified there too, except IMs from scripts and delivery of offline IMs,
which the rig cannot produce. The control socket is verified there as well,
and `ctl im` by avatar name was checked by hand on Second Life on 2026-11-19;
it has not been tried on Windows. On Second Life, checked by hand on 2026-11-18:
login, sitting, logging of chat and IMs from avatars and from objects, and
fetching offline IMs (delivered with the time they were sent). Multi-factor login is not supported: an account that has it enabled
makes the bot exit with an error.

## Build

Needs Rust 1.100 (pinned in `rust-toolchain.toml`; rustup picks it up).

```sh
cargo build --release      # binary: target/release/rslbot
cargo test
```

## Run

```sh
cp russelbot.example.toml russelbot.toml   # then edit it
target/release/rslbot               # reads ./russelbot.toml
target/release/rslbot path/to/other.toml
target/release/rslbot --verbose     # also log every simulator message
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

One TOML file; see `russelbot.example.toml`.

| Key | Required | Meaning |
|---|---|---|
| `login_url` | no | Grid login URL. Default: `https://login.agni.lindenlab.com/cgi-bin/login.cgi` (Second Life) |
| `user` | yes | `First Last`, or a single word (last name `Resident`) |
| `password` | one of the two | Plain password |
| `password_hash` | one of the two | Output of `rslbot hash` |
| `region` | yes | Target region name |
| `position` | yes | `[x, y, z]` inside the region; pick a spot close to the object |
| `object` | yes | UUID of the object to sit on |
| `log_chat` | no | File that receives local chat, see [Chat and IM logs](#chat-and-im-logs) |
| `log_im` | no | File that receives instant messages |
| `fetch_offline_im` | no | `true` to also fetch IMs stored while the avatar was offline. Default: `false` |
| `socket` | no | `true` to listen on the control socket, see [Control socket](#control-socket). Default: `false` |

To keep the plain password out of the file:

```sh
target/release/rslbot hash          # type the password; prints $1$...
printf 'secret\n' | target/release/rslbot hash
```

Put the printed value in `password_hash` and leave `password` out.

## Chat and IM logs

The bot reads local chat the avatar hears and instant messages sent to it,
from avatars and from objects, and writes each one to standard error. It
never answers by itself; it talks only when told to through the
[control socket](#control-socket). The instant messages it sends that way
are recorded too, as kind `im_out`. Group chat, offers, invitations and
typing notifications are not recorded.

With `log_chat` and/or `log_im` set, each message is also appended to that
file as one tab-separated line (UTF-8). The file is created when missing and
gets a header line when it is empty; relative paths are relative to the
directory the bot is started in. A file that cannot be opened makes the bot
exit with status 2 before it logs in.

| Column | Content |
|---|---|
| `time` | UTC, `YYYY-MM-DDTHH:MM:SSZ`: when the bot received the message, or when it was sent for an IM stored offline |
| `kind` | Chat: `whisper`, `say`, `shout`, `region`, `owner`, `direct`, `debug`. IM: `im` (from an avatar), `object` (from a script), `im_out` (sent by the bot) |
| `avatar_name` | The speaker; for an object, its owner; for `im_out`, the recipient |
| `display_name` | That avatar's display name |
| `object_name` | The object's name; empty for an avatar |
| `object_uuid` | The object's UUID; empty for an avatar |
| `text` | The message |

Inside a field, tab, line feed, carriage return and backslash are written as
`\t`, `\n`, `\r` and `\\`. Display names and owners' names are looked up on
the grid; a name the grid does not supply within 5 seconds is left empty, as
are the owner columns of a group-owned object.

An instant message the bot sends is one `im_out` row whose avatar columns
name the recipient, for example (tabs shown as `<TAB>`):

```
2026-11-18T17:04:23Z<TAB>im_out<TAB>Jane Doe<TAB>JD<TAB><TAB><TAB>are you there?
```

Chat the bot says on channel 0 comes back from the simulator and is logged
as heard, like anyone else's. Chat on other channels is not heard by
avatars, so it appears on standard error only.

`fetch_offline_im = true` makes the bot ask, after each login, for the IMs
stored while the avatar was offline. Fetching removes them from the grid: a
viewer logging in as this avatar afterwards will not see them.

The bot logs in with `agree_to_tos` set, so it accepts a grid's terms of
service on the account's behalf, including changed terms, without showing
them to anyone.

## Control socket

With `socket = true` in the config, a running bot takes commands on a local
socket, and `rslbot ctl` sends them:

```sh
target/release/rslbot ctl say hello there          # local chat, channel 0
target/release/rslbot ctl say /1 test message      # channel 1
target/release/rslbot ctl say /shout over here
target/release/rslbot ctl im "Jane Doe" are you there?
target/release/rslbot ctl im jane.doe are you there?
```

`say` reads its message the way a viewer's chat bar does:

| Message starts with | Meaning |
|---|---|
| `/` and a number | Said on that channel: `/1 test message`, `/42open` |
| `//` | Said on the channel the previous `say` used |
| `/shout ` or `/whisper ` | Shouted or whispered on channel 0 |
| anything else | Said on channel 0 as it is, `/me waves` included |

`im` takes the recipient as its first argument and the rest as the message.
The recipient is `"First Last"` (quoted, so that it is one argument),
`first.last`, a single word (last name `Resident`), or a UUID. A name is
looked up on the grid and must match an avatar's full name exactly, in any
letter case; the bot does not guess from partial matches. A UUID is used as
it is.

The words of a message are joined with single spaces; quote the message to
keep other spacing. A message can be 1023 bytes long. Everything after
`ctl` belongs to the command, so a message may start with `-`. Mind the
shell: quote what it would expand, and in Git Bash on Windows write `//1`
for a leading `/1`, which it would otherwise turn into a path.

| Exit status of `rslbot ctl` | Meaning |
|---|---|
| 0 | The message was handed to the simulator. For `im` this does not mean the recipient is online or has read it |
| 1 | The bot refused (the reason is printed): not in a region right now, unknown avatar, no answer to the name lookup within 10 s, empty or over-long message, unknown command. Or no bot is listening on the socket |
| 2 | No command given |

A command is carried out at once or not at all: one sent while the bot is
logged out or teleporting fails and is not kept for later.

The socket is the Unix domain socket `~/russelbot-ctl`, accessible only to
the user running the bot, and is removed when the bot exits. Set the
environment variable `RUSSELBOT_SOCK` to another path, for the bot and for
`rslbot ctl` alike, to put it elsewhere or to run several bots as one user.
A second bot on the same socket exits with status 2, as does a bot that
cannot create it.

On Windows the socket is TCP on `127.0.0.1:47115` instead, and
`RUSSELBOT_SOCK` takes `host:port`. Any user of the machine can connect to
it and speak as the avatar, so enable it only on a machine you do not
share. The Windows side has not been run or compiled: this project is
built and tested on Linux.
