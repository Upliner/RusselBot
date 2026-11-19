RusselBot is a lightweight bot software for Second Life and OpenSimulator written in Rust,
replacement for heavy .NET/Mono bots. Requires just a few MBs of memory which makes it possible
to run even on cheapest VPS machines without requiring to run heavy Mono runtime.

Left alone, the bot follows one simple scenario:
* Connect to the grid, connect to the region
* If current region is not the target region, issue teleport command to the target destination
* Issue "sit" command on a target object.
* Keep the avatar alive, respond all necessary server queries to keep it online.

Through its [control socket](#control-socket) it can also be told to talk,
stand, sit elsewhere, teleport, exchange teleport offers and touch objects.

## Status

Verified by unit tests and against a local OpenSim, including kicks, region
restarts, simulator outages and being unseated or teleported away; see
`docs/test-rig.md` for the rig and the results. Chat and IM logging is
verified there too, except IMs from scripts and delivery of offline IMs,
which the rig cannot produce. The control socket and its commands are verified there as well,
and `ctl im` by avatar name was checked by hand on Second Life on 2026-11-19.
The movement, teleport-offer and touch commands have not been tried on
Second Life yet, and nothing has been tried on Windows. On Second Life, checked by hand on 2026-11-18:
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

The bot keeps working towards "logged in", and as far as the config names
them, "in the target region" and "seated as configured". It logs in again
after a kick, a region restart or a network outage, teleports back when
moved, and sits again when unseated, each with growing delays between
attempts. Everything it does is logged to standard error with UTC
timestamps.

What it holds depends on three optional settings:

* `region` and `position`: the bot logs in there and teleports back whenever
  the avatar is in another region. Without them it never teleports by
  itself; `start` then says where a login puts the avatar.
* `sit` with an object's UUID: the bot sits on that object and sits again
  when unseated. Without a region it tries wherever the avatar is, and when
  the object is not there it keeps trying, at most once a minute.
* `sit = "ground"`: the bot sits the avatar on the ground. The simulator
  does not confirm a ground sit, so the bot sends it once each time the
  avatar arrives somewhere, stands up from an object, or `home` is run, and
  cannot tell whether the avatar still sits.

With none of them the bot just stays online where it logged in, logs
messages and takes commands.

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
| `region` | no, but only with `position` | Target region name; the bot keeps the avatar in that region |
| `position` | no, but only with `region` | `[x, y, z]` inside the region; pick a spot close to the object to sit on |
| `start` | no, and only without `region` and `position` | Where a login puts the avatar: `"home"` (its home location) or `"latest"` (where it was last). Default: `"latest"` |
| `sit` | no | What to sit on: the UUID of an object, or `"ground"`. Default: the bot does not sit by itself |
| `object` | no | Deprecated spelling of `sit`, for a UUID only; the bot logs a warning. Not together with `sit` |
| `log_chat` | no | File that receives local chat, see [Chat and IM logs](#chat-and-im-logs) |
| `log_im` | no | File that receives instant messages |
| `fetch_offline_im` | no | `true` to also fetch IMs stored while the avatar was offline. Default: `false` |
| `socket` | no | `true` to listen on the control socket, see [Control socket](#control-socket). Default: `false` |
| `requesttp_timeout` | no | Minutes after `requesttp` during which the asked avatar's teleport offer is accepted. Default: `10` |
| `scan_distance` | no | Metres (1 to 1024) within which objects are learned from the start, for `touch`; see [Touching objects](#touching-objects). Default: learn within 32 m when `touch` is first used |

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
are recorded too, as kind `im_out`. Group chat, offers (teleport offers
and requests included; those appear on standard error only), invitations
and typing notifications are not recorded.

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
target/release/rslbot ctl shout over here
target/release/rslbot ctl im "Jane Doe" are you there?
target/release/rslbot ctl stand
target/release/rslbot ctl tp 128 128 25 Blue Lagoon
target/release/rslbot ctl status
target/release/rslbot ctl home
```

| Command | Does |
|---|---|
| `say MESSAGE...` | Local chat, read the way a viewer's chat bar reads it (below) |
| `shout MESSAGE...`, `whisper MESSAGE...` | Local chat on channel 0, shouted or whispered; the message is sent as it is |
| `im AVATAR MESSAGE...` | Instant message |
| `stand`, `unsit` | Stand up (two names for the same command) |
| `sit UUID` | Sit on that object. Answers when the simulator has confirmed, or after 15 s with an error |
| `sit ground` | Sit on the ground |
| `tp X Y [Z] [REGION...]` | Teleport. Without `REGION`, inside the current region; without `Z`, at the current height. Answers on arrival or with the reason it failed |
| `requesttp AVATAR [MESSAGE...]` | Ask the avatar to teleport the bot to them; when they agree, the bot goes |
| `accepttp AVATAR` | Accept a teleport offer the avatar has sent, and go |
| `sendtp AVATAR [MESSAGE...]` | Offer the avatar a teleport to the bot |
| `touch UUID` | Click that object once |
| `home` | Hold the configured place again: go back to the configured region and sit as configured. Without a configured region the avatar is not moved, also not with `start = "home"` |
| `status` | Print where the avatar is |
| `quit` | Log out and stop the bot, like Ctrl-C |

`AVATAR` is one argument: `"First Last"` (quoted), `first.last`, a single
word (last name `Resident`), or a UUID. A name is looked up on the grid and
must match an avatar's full name exactly, in any letter case; the bot does
not guess from partial matches. A UUID is used as it is.

### Talking

`say` reads its message the way a viewer's chat bar does:

| Message starts with | Meaning |
|---|---|
| `/` and a number | Said on that channel: `/1 test message`, `/42open` |
| `//` | Said on the channel the previous `say` used |
| `/shout ` or `/whisper ` | Shouted or whispered on channel 0 |
| anything else | Said on channel 0 as it is, `/me waves` included |

The words of a message are joined with single spaces; quote the message to
keep other spacing. A message can be 1023 bytes long. Everything after
`ctl` belongs to the command, so a message may start with `-`. Mind the
shell: quote what it would expand, and in Git Bash on Windows write `//1`
for a leading `/1`, which it would otherwise turn into a path.

### Moving, and staying put

Left alone the bot holds its place: it returns to the configured region and
seat, as far as the config names them, whenever it finds the avatar
elsewhere. `stand`, `unsit`, `sit`,
`tp`, `requesttp` and `accepttp` switch that off. From then on the bot is
in manual mode: it stays logged in, and logs in again after a kick or an
outage (at the avatar's last location), but it no longer teleports or sits
by itself. `home` switches holding back on, and the bot returns at once.
`home` is not the avatar's home location: with no region configured it
moves nothing and only sits the avatar again as `sit` says.
Manual mode ends with the bot process too; a restarted bot holds its place.
`sendtp`, `touch` and the talking commands leave the mode alone.

In `tp`, a third argument that is a number is `Z`, and anything else starts
the region's name, which may be several words. A region whose name starts
with a number therefore needs `Z` given: `tp 128 128 25 7 Seas`. Without
`Z` the bot uses the height the simulator last reported for the avatar,
on arrival in the region or after a teleport inside it; a seat that has
carried the avatar elsewhere since is not noticed, so give `Z` when in
doubt.

`status` prints, for example:

```
connected: yes
region: Target
position: 128, 128, 25
seat: target
mode: holding
```

`seat` is `target` (the object configured as `sit`), `object <uuid>` (an object the
avatar was seated on by command or by the simulator), `other` (attached to
something the simulator did not name), `none` or `unknown`. A ground sit,
by command or by `sit = "ground"`, shows as `none`: the bot does not track
it. `mode` is `holding` or `manual`.
While the avatar is not in a region, only `connected: no` and `mode` are
printed.

### Teleport offers

`requesttp AVATAR` sends the request a viewer sends with "Request
Teleport". If that avatar answers with an offer within `requesttp_timeout`
minutes (10 unless configured), the bot accepts it and goes; one request,
one acceptance. `home` cancels the wait.

Any other teleport offer is not accepted by itself. It is noted on
standard error and kept, the latest one per avatar, until the session
ends; `accepttp AVATAR` accepts it. Whether an old offer still works is up
to the grid, and its refusal is what the command prints.

When an avatar asks to be teleported to the bot, that is noted on standard
error as well, and `sendtp AVATAR` is the answer if you want to give one.

### Touching objects

The simulator takes a touch by its own short-lived number for the object,
not by UUID, so the bot can only touch objects it has been told about.
Much of that it picks up from what the simulator sends anyway. For the
rest it has to ask (a scan), and that costs traffic, so it asks only when
there is a reason:

- Without `scan_distance`, nothing extra is sent until `touch` names an
  object the bot does not know. That first `touch` starts a scan of the
  objects within 32 m and waits up to 10 seconds for the object to turn
  up; later ones answer at once. A new region starts over.
- With `scan_distance = N`, the bot scans from the moment it enters a
  region and asks the simulator to describe objects within N metres, so
  `touch` answers at once and reaches further, at the price of that
  traffic all the time.

An object the bot has not learned of is an error naming the distance. On
OpenSimulator the distance makes little difference: it describes objects
regardless.

### Exit status and limits

| Exit status of `rslbot ctl` | Meaning |
|---|---|
| 0 | Done: the message was handed to the simulator, the avatar arrived, sat, and so on. For `im`, `requesttp` and `sendtp` this does not mean the other avatar is online, has read it or agreed |
| 1 | The bot refused or the action failed, and the reason is printed: not in a region right now, unknown avatar or region, no answer to a name lookup within 10 s, a sit not confirmed within 15 s, a teleport without outcome after 60 s, an unknown object, a bad argument, an unknown command. Or no bot is listening on the socket |
| 2 | No command given |

A command is carried out at once or not at all: one sent while the bot is
logged out or teleporting fails and is not kept for later. `rslbot ctl`
waits up to 90 seconds for the answer, since a teleport may take a minute.

### The socket

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
