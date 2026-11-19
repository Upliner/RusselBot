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
Second Life yet, and neither has `persist` with the position tracking under
it: whether Second Life answers the bot's question about a seat, and the
seated position that results, are unverified there. Nothing has been tried
on Windows. On Second Life, checked by hand on 2026-11-18:
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

The bot keeps working towards "logged in", "in the region it holds" and
"seated as it holds". It logs in again after a kick, a region restart or a
network outage, teleports back when moved to another region, and sits again
when unseated, each with growing delays between attempts. Everything it
does is logged to standard error with UTC timestamps.

What it holds is a placement: a location (a region and coordinates in it)
and a sit state (an object, the ground, standing, or "leave it alone"). It
starts out with the placement the config describes:

* `region` and `position`: the bot logs in there and teleports there when
  the login lands in another region. Without them the first login goes
  where `start` says, and the place it arrives at is the location.
* `sit` with an object's UUID: the bot sits on that object and sits again
  when unseated. After three failed sits it teleports to the location and
  tries from there.
* `sit = "ground"`: the bot sits the avatar on the ground. The simulator
  does not confirm a ground sit, so the bot sends it once each time the
  avatar arrives somewhere, stands up from an object, or a placement is
  persisted, and cannot tell whether the avatar still sits.
* Without `sit` the bot does not sit the avatar.

Once the avatar is there (logged in, and seated when `sit` names an
object), `persist` decides what the bot holds from then on:

* `persist = "cur"`, the default: the position and the seat the simulator
  reported at that moment. The position is where the avatar really is,
  on its seat when it sits, and the seat is the prim the simulator sat it
  on, which in a linkset may not be the one `sit` names.
* `persist = "cfg"`: `position` and `sit` exactly as the config has them.
* `persist = "off"`: nothing. The bot goes to manual mode and leaves the
  avatar alone; it only keeps it logged in.

Inside the region it holds, the bot does not put the avatar back where it
was: it reacts to a change of region, and to being unseated from a held
object. `rslbot ctl persist` changes the held placement while the bot runs,
see [Holding a place](#holding-a-place). Nothing of it is saved: a
restarted bot starts from the config again.

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
| `region` | no, but only with `position` | Region the avatar starts out in; the bot logs in there and holds it until a persist names another |
| `position` | no, but only with `region` | `[x, y, z]` inside the region; pick a spot close to the object to sit on |
| `start` | no, and only without `region` and `position` | Where a login puts the avatar while the bot holds no location, which is the first login: `"home"` (its home location) or `"latest"` (where it was last). Default: `"latest"` |
| `sit` | no | What to sit on at the start: the UUID of an object, or `"ground"`. Default: the avatar stands, and the bot does not sit it |
| `object` | no | Deprecated spelling of `sit`, for a UUID only; the bot logs a warning. Not together with `sit` |
| `persist` | no | What the bot holds once the avatar is at the place above: `"cur"`, `"cfg"`, `"off"` or `false`, an array, or a table; see below. Default: `"cur"` |
| `log_chat` | no | File that receives local chat, see [Chat and IM logs](#chat-and-im-logs) |
| `log_im` | no | File that receives instant messages |
| `fetch_offline_im` | no | `true` to also fetch IMs stored while the avatar was offline. Default: `false` |
| `socket` | no | `true` to listen on the control socket, see [Control socket](#control-socket). Default: `false` |
| `requesttp_timeout` | no | Minutes after `requesttp` during which the asked avatar's teleport offer is accepted. Default: `10` |
| `scan_distance` | no | Metres (1 to 1024) within which objects are learned from the start, for `touch`; see [Touching objects](#touching-objects). Default: learn within 32 m when `touch` is first used |

`persist` takes these forms:

| Form | Meaning |
|---|---|
| `"cur"` | Hold the position and the seat the simulator reported |
| `"cfg"` | Keep holding `position` and `sit` as configured |
| `"off"` or `false` | Hold nothing: manual mode once the avatar is in place |
| `["cfg", "cur"]` | An array of two, each `"cur"` or `"cfg"`: the first for the position, the second for the seat |
| `[persist]` with `position` and `sit` | The same as a table, each `"cur"` or `"cfg"`; a key left out is `"cur"`. As a TOML table it has to come after all other keys |

### Upgrading

Configs written for earlier versions still load, but three things changed:

* The bot used to hold `position` and the `sit` UUID exactly as configured.
  It now holds what the simulator reported after it sat (`persist = "cur"`).
  Add `persist = "cfg"` to keep the old behaviour.
* A config without `region` used to mean "never teleport". Now the bot
  returns to where it settled when the avatar is moved to another region,
  and logs in there after a kick. Add `persist = "off"` for a bot that
  does not move the avatar by itself.
* `rslbot ctl home` returns to the last persisted placement, which need not
  be the configured one; `rslbot ctl persist cfg cfg` returns to the config.

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
target/release/rslbot ctl persist                   # stay here, like this
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
| `persist [LOCATION] [SIT]` | Hold a place from now on: return to `LOCATION` whenever the avatar is in another region, and sit as `SIT` says. Without arguments: where the avatar is and how it sits now. See [Holding a place](#holding-a-place) |
| `home` | The same as `persist last last`: hold the place last persisted again, or the configured one when nothing was persisted |
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

Left alone the bot holds its place: it returns to the region and the seat
it holds whenever it finds the avatar elsewhere. `stand`, `unsit`, `sit`,
`tp`, `requesttp` and `accepttp` switch that off. From then on the bot is
in manual mode: it stays logged in, and logs in again after a kick or an
outage (at the avatar's last location), but it no longer teleports or sits
by itself. `persist` and `home` switch holding back on, and the bot acts
at once. `home` is not the avatar's home location: it is the place the bot
held last. Manual mode ends with the bot process too; a restarted bot
starts from its config. `sendtp`, `touch` and the talking commands leave
the mode alone.

### Holding a place

`persist [LOCATION] [SIT]` tells the bot what to hold from now on, and
switches holding on. A typical use is to put the avatar somewhere by hand
and then make it stay:

```sh
target/release/rslbot ctl tp 128 128 25 Blue Lagoon
target/release/rslbot ctl sit 11111111-2222-3333-4444-555555555555
target/release/rslbot ctl persist
```

`LOCATION` is `REGION/X/Y/Z` as one argument; quote it when the region's
name has spaces: `"Blue Lagoon/128/128/25"`. `SIT` is one of:

| `SIT` | The bot |
|---|---|
| an object's UUID | keeps the avatar seated on that object, sitting again when it is unseated |
| `ground` | sits the avatar on the ground, once each time it arrives or gets up from an object |
| `stand` | stands the avatar up if it sits, and then leaves sitting and standing alone |
| `unchanged` | leaves sitting and standing alone |

Both arguments can also be one of these words; an argument left out is
`cur`, so plain `persist` is `persist cur cur`:

| Word | As `LOCATION` | As `SIT` |
|---|---|---|
| `cur` | Where the avatar is now | The prim it sits on now, `ground` if the bot last sat it on the ground, else `stand` |
| `cfg` | `region` and `position` from the config; without them, where the first login of this run arrived | `sit` from the config, or `stand` |
| `last` | The location last persisted, or as `cfg` | The seat last persisted, or as `cfg` |
| `last_cmd` | Where the last `tp` command went, or as `last` | What the last `sit` command named, or as `last` |

The command answers at once with what it now holds, for example
`Blue Lagoon/128.5/131/25.2 11111111-2222-3333-4444-555555555555`; the
teleport or sit that follows happens afterwards and is in the log.

What holding means:

* When the avatar is in another region than `LOCATION`, now or later, the
  bot teleports it to `LOCATION`. After a kick or an outage it logs in
  there.
* Inside that region the bot does not move the avatar, wherever it is:
  `persist Target/10/10/25 stand` given in `Target` changes nothing until
  the avatar leaves the region. Use `tp` to go to the spot first. The one
  exception is a held object the bot cannot sit on: after three failed sits
  it teleports to `LOCATION` and tries from there.
* `cur` needs the avatar in a region; between logins and during a teleport
  `persist` and `persist cur ...` fail. The other words and full values
  work at any time, so `home` and `persist cfg cfg` do too.
* `persist` refuses a seat it knows to be in another region than the
  location, such as `persist "Home/128/128/25" cur` while sitting in
  `Target`. It knows that of objects the avatar sat on in this run and of
  objects it has learned in the current region.
* `last_cmd` goes by the commands as given, whether or not the teleport or
  sit succeeded. `stand`, `requesttp` and `accepttp` do not count.
* Nothing is saved. A restarted bot starts from its config and the
  `persist` setting there.

In `tp`, a third argument that is a number is `Z`, and anything else starts
the region's name, which may be several words. A region whose name starts
with a number therefore needs `Z` given: `tp 128 128 25 7 Seas`. Without
`Z` the bot uses the avatar's current height.

The bot follows where the avatar is from what the simulator reports about
it: on arrival, after a teleport, and whenever it moves or is moved. While
the avatar sits on an object the simulator gives its place relative to the
seat, so the bot asks about the seat once and works the position out from
where the seat is and how it is turned. Until the simulator has answered,
the position is the last one from before the sit. That position is what
`status` prints, what `tp` without `Z` takes the height from, and what
`persist` means by the current location.

`status` prints, for example:

```
connected: yes
region: Target
position: 130, 128, 25.9
seat: target
persist: Target/130/128/25.9 11111111-2222-3333-4444-555555555555
mode: holding
```

`seat` is `target` (the object the bot holds as the seat), `object <uuid>`
(another object, named by the prim the simulator sat the avatar on),
`other` (attached to something the simulator did not name), `none` or
`unknown`. A ground sit, by command or held, shows as `none`: the bot
cannot see it. `persist` is the placement the bot holds, written the way
the `persist` command takes it; in manual mode it is what `home` goes back
to, and its location is `unknown` until the first login of a config
without `position`. `mode` is `holding` or `manual`.
While the avatar is not in a region, only `connected: no`, `persist` and
`mode` are printed.

### Teleport offers

`requesttp AVATAR` sends the request a viewer sends with "Request
Teleport". If that avatar answers with an offer within `requesttp_timeout`
minutes (10 unless configured), the bot accepts it and goes; one request,
one acceptance. `persist` and `home` cancel the wait.

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

One request is made regardless of all this: each time the avatar becomes
seated on an object, the bot asks the simulator about that one object, to
know where the avatar is.

### Exit status and limits

| Exit status of `rslbot ctl` | Meaning |
|---|---|
| 0 | Done: the message was handed to the simulator, the avatar arrived, sat, and so on. For `im`, `requesttp` and `sendtp` this does not mean the other avatar is online, has read it or agreed |
| 1 | The bot refused or the action failed, and the reason is printed: not in a region right now, unknown avatar or region, no answer to a name lookup within 10 s, a sit not confirmed within 15 s, a teleport without outcome after 60 s, an unknown object, a seat known to be in another region than the location to persist, a bad argument, an unknown command. Or no bot is listening on the socket |
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
