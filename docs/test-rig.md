# Local OpenSim test rig

RusselBot is verified against a local OpenSimulator standalone with two regions:

```
 OpenSim standalone (SQLite, http://127.0.0.1:9000)
 +------------------+   +------------------+
 | Region "Home"    |   | Region "Target"  |
 | default landing  |   | [prim to sit on] |
 +------------------+   +------------------+
```

## Build and start

Requirements: the .NET 8 SDK (`dotnet --list-sdks` must list an 8.x entry) and
an OpenSim source checkout at `references/opensim` (or anywhere, via
`OPENSIM_SRC`).

```sh
test-rig/setup.sh                       # clone, build, configure; safe to re-run
cd test-rig/opensim/bin && ./opensim.sh # starts the simulator and its console
```

`setup.sh` clones the checkout into `test-rig/opensim/` (git-ignored) so the
original stays untouched, builds it, creates `OpenSim.ini` and
`StandaloneCommon.ini` from the shipped examples, and installs
`test-rig/config/Regions.ini`. The first start creates an estate owned by the
account `Rig Owner` (password `rigowner`) without asking questions, and takes
about a minute.

On the OpenSim console, `show regions` must list `Home` and `Target`.

To drive the console from scripts, start the simulator with its input on a
named pipe instead:

```sh
mkfifo /tmp/sim.in; sleep infinity > /tmp/sim.in &
cd test-rig/opensim/bin && dotnet OpenSim.dll -console=basic < /tmp/sim.in > /tmp/sim.log 2>&1 &
echo 'show regions' > /tmp/sim.in
```

## Bot user and seat

1. On the OpenSim console create the bot account. With all six arguments it
   asks nothing (the unknown model name falls back to the default avatar):

   ```
   create user Sit Bot secret sitbot@example.com 7e570000-0000-4000-8000-0000000000b0 default
   ```

2. Rez the seat prim in `Target`. Either run the helper, which logs in as
   `Rig Owner`, rezzes a box at 130, 128 and prints its UUID and position:

   ```sh
   test-rig/rezprim.py
   ```

   or log in with a viewer as `Rig Owner` / `rigowner` (login URI
   `http://127.0.0.1:9000/`), rez a prim in `Target` and copy its UUID
   (Build > Edit > General > Copy Keys).
3. Create `russelbot.toml` from `russelbot.example.toml`: keep `login_url`, `user` and
   `password`, set `sit` to the UUID and `position` to a spot next to the
   prim (`[128.0, 128.0, 25.0]` for the helper's prim).

Run the bot with `cargo run -- russelbot.toml`. Add `--verbose` to see every
message name it receives and sends.

OpenSim seats an avatar on a prim from any distance in the region, so the
"too far to sit" case cannot be produced on this rig; the repositioning
teleport is exercised by deleting the prim instead (item 8).

## Failure-injection checklist

Console commands are typed on the OpenSim console. The console command
`restart` is disabled in current OpenSim; use `region restart notice`.

| # | Inject | Expected | Result (2026-11-18, OpenSim 9d00a55552) |
|---|---|---|---|
| 1 | Start the bot with the rig running | Login, lands in `Target`, `seated on <uuid>` | Pass: seated within 1 s of login |
| 2 | Leave it for 30 minutes with `--verbose` | Still online and seated; no errors; exactly one `AgentRequestSit` | Pass, and longer than planned: 8 h 52 min seated, one `AgentRequestSit`, 31927 agent updates, no resends or dropped packets, no errors; resident memory 8 MB (debug build); Ctrl-C then logged out with status 0 |
| 3 | `stand user name Sit Bot` (the same simulator call as a script's `llUnSit`) | `unseated; sitting again in 5 s`, then `seated` | Pass |
| 4 | Repeat 3 a few seconds after each re-sit | Delays grow 5, 10, 20, 40 s and stop at 60 s | Pass for 5, 10, 20, 40 s; the 60 s cap is covered by unit tests only |
| 5 | `kick user Sit Bot bye` | `session lost: kicked: ...` within 5 s, login after 10 s, seated again | Pass: loss logged in under 1 s |
| 6 | `change region Target`, then `region restart notice "restart" 5` | Session loss with its cause, login after the delay, seated again | Pass: `kicked: The simulator is going down.`, back and seated 10 s later |
| 7 | `shutdown`, wait, start OpenSim again | `login failed (network)` at 10, 20, 40, 80 ... s intervals; recovers after the restart | Pass: delays of 10 s after the loss, then 20, 40, 80 s; recovered on the first attempt after the restart |
| 8 | `change region Target`, then `delete object id <uuid>` | `unseated`, then `sit failed` at growing intervals; after three failures a teleport to the configured position; no re-login | Pass: failures at 10, 20, 40 s delays, then `teleport arrived at the configured position` |
| 9 | `teleport user Sit Bot Home/128/128/25` while the bot is seated | `connected, in region "Home" (not the target region)`, a teleport back to `Target` without a new login, then seated | Pass: back in `Target` and seated 5 s later |
| 10 | Config: `region = "Nowhere"` | Lands in `Home`; `teleport failed: region "Nowhere" not found` at growing intervals; avatar stays online | Pass: retried at 10, 20, 40 s |
| 11 | Config: wrong `password` | At most two login attempts, `giving up: credentials rejected ...`, exit status 1 | Pass: two attempts, exit 1, no password or hash in the log |
| 12 | Ctrl-C while seated | `stop requested; logging out`, `logged out`, exit status 0 within 6 s; an immediate restart logs in without "already logged in" | Pass: logged out in under 1 s; restart 3 s later logged in |
| 13 | Ctrl-C while waiting to retry (rig stopped) | Exits at once with status 0 | Pass |
| 14 | `kill -9` the OpenSim process while the bot is seated | Session loss no later than 60 s after the kill | Pass: `nothing received from the simulator for 60 s`, exactly 60 s after the kill |
| 15 | `alert-user Sit Bot hello`, then `alert hello` | `simulator alert: /hello` (OpenSim adds the slash), then `simulator alert: hello`; the bot stays seated | Pass |

## Chat and IM logging

`test-rig/talk.py` logs in as `Rig Owner` four metres from the bot and talks
to it: four lines of local chat (say, whisper, shout, and one containing a
tab, a line break and a backslash), one instant message, and one line from a
prim it rezzes with the simulator's default script. Run the bot with
`log_chat` and `log_im` set, wait for `seated`, then run the helper.

| # | Inject | Expected | Result (2026-11-18, OpenSim 9d00a55552) |
|---|---|---|---|
| 16 | `talk.py` with `log_chat = "chat.tsv"` and `log_im = "im.tsv"` | One standard-error line and one row per message; chat only in `chat.tsv`, the IM only in `im.tsv`; header row first; seven fields per row | Pass: 5 chat rows, 1 IM row, kinds `say`, `whisper`, `shout`, `im` |
| 17 | The avatar lines of 16 | `avatar_name` and `display_name` filled, object columns empty | Pass: `Rig Owner` in both (OpenSim's display name is the avatar name) |
| 18 | The scripted prim's line of 16 | Owner in the avatar columns, `Object` and the prim's UUID in the object columns | Pass |
| 19 | The line `one<TAB>two<LF>three \ four` of 16 | One row whose text is `one\ttwo\nthree \\ four`; one line on standard error | Pass |
| 20 | Ctrl-C the bot while `talk.py` is still talking | Every chat message the bot received before the signal is in `chat.tsv`; exit status 0 | Pass at three different moments: 2, 3 and 3 messages received, logged and written |
| 21 | `fetch_offline_im` absent, then `true`, with `--verbose` | No `RetrieveInstantMessages` sent; then exactly one, after entering the region, and the bot sits as usual | Pass |
| 22 | Start with `log_im` in a directory that does not exist | Error naming `log_im`, exit status 2, no login attempt | Pass |

Not covered on this rig: an instant message from a script (`llInstantMessage`;
the helper cannot upload a script of its own), and delivery of stored offline
IMs (OpenSim keeps them only with a MySQL/PostgreSQL store or an external
service, neither of which the rig has). Both were checked on Second Life
instead.

## Control socket

`test-rig/listen.py` logs in as `Rig Owner` four metres from the bot, prints
the local chat it hears and the instant messages it gets, and answers each
instant message with `pong: <text>`. It also prints where it sees the bot's
avatar and what the avatar is attached to, grants the bot's teleport
requests, prints the bot's teleport offers, and with its `prim` and
`offer=N` arguments rezzes a box that says `Touched.` when clicked and
sends the bot an unasked teleport offer. Run the bot with `socket = true`,
`log_chat` and `log_im` set and `--verbose`, wait for `seated`, start the
helper, then use `rslbot ctl`.

| # | Inject | Expected | Result (2026-11-18, OpenSim 9d00a55552) |
|---|---|---|---|
| 23 | `ctl say hello there`, `/shout over here`, `/whisper psst`, `/me waves`, `-v is not an option here` | Status 0; the helper hears each with its volume; one `control: said ...` line and one `chat` row each | Pass |
| 24 | `ctl say /1 test message`, then `ctl say //second on one` | Status 0; `control: said on channel 1: ...` twice; a `ChatFromViewer` sent for each; nothing heard by the helper and no `chat` row | Pass. What a script listening on channel 1 receives was not observed: the rig has no such script |
| 25 | `ctl say /5` | `there is no text to send`, status 1, nothing sent | Pass |
| 26 | `ctl im "Rig Owner" are you there?`, then `ctl im rig.owner second message` | `sent to Rig Owner <uuid>`, status 0; the helper gets both in one conversation (id = both agent ids combined); one name lookup for the two | Pass, both ways of looking a name up: through the `AvatarPickerSearch` capability, which the bot uses when the simulator offers it (OpenSim does), and through `AvatarPickerRequest`, checked with a build that did not ask for the capability |
| 27 | The helper's `pong` answers to 26 | `im.tsv` has each `im_out` row followed by its `im` row, all with `Rig Owner` in the avatar columns | Pass |
| 28 | `ctl im "Nobody Here" hello`, `ctl im Rig hello` | `the grid knows no avatar named ...`, status 1, no instant message, no row | Pass (`Rig` alone would be `Rig Resident`) |
| 29 | `ctl im <the bot's own UUID> note to self` | `sent to <uuid>`, status 0; an `im_out` row with the name filled in by the lookup | Pass |
| 30 | `ctl dance`, `ctl im "Rig Owner"`, `ctl` | `unknown command`, `takes 2 arguments`, status 1; usage and status 2 for the last | Pass |
| 31 | `kick user Sit Bot bye`, then `ctl say ...` and `ctl im ...` before the next login | `the avatar is not connected to a region`, status 1; nothing said after the bot is back | Pass; `ctl say` works again after the re-login |
| 32 | Start a second bot with the same socket | It exits with status 2 naming `socket`; the first keeps answering | Pass |
| 33 | Ctrl-C the bot, then `ctl say ...` | The socket file is gone; `cannot reach the bot at <path>`, status 1 | Pass |
| 34 | `RUSSELBOT_SOCK` in a directory that does not exist; no `HOME` and no `RUSSELBOT_SOCK`; `socket = "yes"` | Error naming `socket`, status 2, no login attempt | Pass, all three |
| 35 | No `socket` key | No socket file is created | Pass |

### Movement, teleport offers and touch

Run the helper as `test-rig/listen.py 120 prim offer=45`; `<prim>` below is
the UUID it prints.

| # | Inject | Expected | Result (2026-11-19, OpenSim 9d00a55552) |
|---|---|---|---|
| 36 | `ctl status` while seated | `connected: yes`, `region: Target`, a position, `seat: target`, `mode: holding` | Pass |
| 37 | `ctl touch <prim>`, twice | Status 0 each; the box says `Touched.` each time; with `--verbose`, no `RequestMultipleObjects` before the first `touch` | Pass |
| 38 | `ctl touch` with a UUID that does not exist | `scanning for objects within 32 m` in the log, then status 1 naming the object and 32 m | Pass |
| 39 | `ctl stand`, wait, `ctl status` | The helper sees the avatar detached; `placement: manual` in the log; `seat: none`, `mode: manual`; no sit follows | Pass |
| 40 | `ctl sit ground` | Status 0 | Pass as far as the rig shows: the command is accepted and sent. The helper does not decode animations, so the sitting itself was not seen |
| 41 | `ctl sit <prim>` | `seated on <prim>`, status 0; the helper sees the avatar attached; `seat: object <prim>` | Pass |
| 42 | `ctl sit` with a UUID that does not exist, while seated on the box | Status 1 after 15 s; the avatar stays on the box | Pass |
| 43 | `ctl sit chair`, `ctl tp north 1` | Status 1 naming the argument; nothing sent | Pass |
| 44 | `ctl tp 100 120`, then `ctl tp 110 120 40` | `arrived in Target` twice; `position: 100, 120, <height as before>` after the first | Pass |
| 45 | `ctl tp 128 128 25 Home`, `ctl tp 128 128 25 Nowhere`, `ctl tp 120 126 Target` | `arrived in Home` without a new login; `region "Nowhere" not found`, status 1; `arrived in Target` | Pass |
| 46 | `ctl requesttp "Rig Owner" please bring me over` | `asked Rig Owner <uuid> for a teleport`; the helper gets the request and offers; the log says the offer was accepted as requested; the avatar is at the helper's position | Pass |
| 47 | `ctl sendtp rig.owner come and see` | `offered Rig Owner <uuid> a teleport`; the helper prints the offer with the text | Pass |
| 48 | `ctl accepttp "Rig Owner"` with no offer waiting | `there is no teleport offer from Rig Owner`, status 1 | Pass |
| 49 | The helper's unasked offer, then `ctl accepttp "Rig Owner"` twice | The log notes the offer and that `accepttp` accepts it; the avatar stays; the first `accepttp` prints `arrived in Target`; the second is refused | Pass |
| 50 | `kick user Sit Bot bye` in manual mode | Login after 10 s with `start: last`; no teleport and no sit afterwards; `mode: manual` | Pass |
| 51 | `ctl home` in the target region, and again after `ctl tp 128 128 25 Home` | `placement: holding`; the bot sits, or teleports to `Target` and sits; `seat: target`, `mode: holding` | Pass, both |
| 52 | `ctl shout over here`, `ctl whisper psst` | The helper hears the shout; `chat [shout]` and `chat [whisper]` rows from the echo | Pass; the whisper was out of the helper's earshot at 20 m |
| 53 | `ctl quit` | Status 0; `logged out`; the bot exits and the socket file is gone | Pass |
| 54 | `scan_distance = 64`, then `ctl touch` with a UUID that does not exist, 14 s after the start | `scanning for objects within 64 m` on entering the region; status 1 at once, naming 64 m | Pass |
| 55 | `scan_distance = 5000`; `requesttp_timeout = 0` | Error naming the key, status 2 | Pass, both |
| 56 | Items 3 and 9 again, with no command given since `home` | The bot sits again and teleports back as before | Pass |

Not covered on this rig: what the scan costs and finds. OpenSim describes
every object in full and never as cached, so the bot had nothing to ask
about and sent no `RequestMultipleObjects` even after a scan started; that
path, and the effect of the scan distance, are covered by unit tests only
until tried on Second Life. Also not covered: an avatar's request to be
teleported to the bot (unit tests only). A ground sit as seen by others
is covered by item 57.

## Optional region, seat and start

These configs leave out `region`/`position` or the seat. `listen.py` prints
`bot ground sit: yes|no` from the animations the bot's avatar plays. The
bot account's home was never set on this rig; a login asking for `home`
then lands at 128, 128 in `Home`, the grid's default region.

| # | Inject | Expected | Result (2026-11-19, OpenSim 9d00a55552) |
|---|---|---|---|
| 57 | `ctl sit ground`, then `ctl stand`, with `listen.py` running | `bot ground sit: yes`, then `no` | Pass |
| 58 | Config with only the account and `socket = true`; `ctl tp 100 100 25 Home`, `ctl home`, then five minutes left alone | `start: last`; `connected, in region "Target"` without "target region"; no teleport or sit of the bot's own; `seat: none`, `mode: holding`; after `home` still in `Home` | Pass: `placement: holding; no region or seat is configured, the avatar stays where it is` |
| 59 | Avatar left in `Target`; `start = "latest"`, quit, then `start = "home"` | First `start: last` and `Target` at the position it left; then `start: home` and `Home` | Pass: 60, 70 in `Target`, then 128, 128 in `Home` |
| 60 | `region`, `position` and `sit = "ground"`, with `listen.py` running; then `ctl stand`, `ctl home`, `ctl tp 100 100 25 Home`, `ctl home` | `sitting on the ground` once after login and `bot ground sit: yes`; `no` after `stand`; `yes` after `home`; after the second `home` a teleport to `Target` and a ground sit there; `seat: none` while sitting on the ground | Pass. The ground sit sent in the same second as the region entry took effect both after the login and after the teleport |
| 61 | `sit = <uuid>` without `region`, avatar last in `Target` | `start: last`, no teleport, `seated on <uuid>`, `seat: target` | Pass: seated within 1 s |
| 62 | The same config, avatar last in `Home` | Sit attempts fail at growing intervals; never a teleport | Pass: four failures, delays 5, 10, 20, 40 s, still in `Home` |
| 63 | The config of item 1 with `object`; then with that key renamed to `sit` | Same behaviour; one line `` `object` is deprecated; write the same value as `sit` `` with the first only | Pass |
| 64 | `start = "last"`; `start = "home"` with `region`/`position`; `sit` with `object` | Error naming the keys, status 2 | Pass, all three |

Covered by unit tests only: a refused login with `start = "home"` falling
back to the last location, the single login attempt with a wrong password
and no `region`, the ground sit after standing up from an object, and the
sit backoff's one-minute cap without a region. Not tried on Second Life:
any of the above, in particular whether a ground sit sent right at region
entry takes effect there.

Not covered: Windows, where the socket is TCP. On Second Life, looking a
name up through `AvatarPickerRequest` found nobody (2026-11-18, an avatar
that had just messaged the bot); the capability is what viewers use there,
and with the bot using it too, `ctl im` by name reached that avatar
(checked by hand, 2026-11-19).

Seen in a viewer as well: the avatar seated on the prim in `Target`. Not covered on this rig:
a region that stays offline while the
rest of the grid is up, and anything specific to Second Life.
