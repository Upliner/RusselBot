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
   `Rig Owner`, rezzes a box at 130, 128 and prints its UUID and position
   (`test-rig/rezprim.py Home` rezzes one in the other region):

   ```sh
   test-rig/rezprim.py
   ```

   or log in with a viewer as `Rig Owner` / `rigowner` (login URI
   `http://127.0.0.1:9000/`), rez a prim in `Target` and copy its UUID
   (Build > Edit > General > Copy Keys).
3. Create `russelbot.toml` from `russelbot.example.toml`: keep `login_url`, `user` and
   `password`, set `sit` to the UUID and `position` to a spot next to the
   prim (`[128.0, 128.0, 25.0]` for the helper's prim), and add
   `persist = "cfg"`: the checklists below expect the bot to return to that
   place and sit again, which it does not do without `persist`.

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
avatar and what the avatar is attached to, what is attached to the bot's
avatar and on which point, the animations it plays and the sounds it
triggers, grants the bot's teleport
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

## Holding a place: `persist`

Since `add-persist-command` the bot holds a placement that `persist` in the
config and `rslbot ctl persist` choose. The items above were run with what
is now `persist = "cfg"`; items 58, 61 and 62 describe the behaviour before
this change and are superseded by items 71 and 72.

Items 65, 67, 71 and 72 were run without a `persist` key, when that meant
`persist = "cur"`; the key is written out below, as it has to be now. A
config without `persist` has since become the same as `persist = "off"`
(item 69). That default was not run on the rig again: the key being
absent is covered by unit tests only.

The configs here have `position = [120.0, 120.0, 25.0]`, ten metres from
the seat prim (at 130, 128, 25.55 in `Target`), so that a held position
shows whether it is the configured one or the seated one. `rezprim.py Home`
rezzed a second prim, at 130, 128, 25.2 in `Home`; `<home prim>` below.
Where a position is compared, the other side is `listen.py`'s
`bot at ...` line (the avatar's offset on its seat, or its place in the
region when it stands) and the prim's location from the console's
`show object id`.

| # | Inject | Expected | Result (2026-11-19, OpenSim 9d00a55552) |
|---|---|---|---|
| 65 | `region`, `position`, `sit`, `persist = "cur"`; then `ctl status` | After `seated`: `config: persist cur cur` and `placement: holding` with the seated position and the seat; the same on the `persist` line; `seat: target` | Pass: `Target/129.33/127.6/26.45 <uuid>`, persisted in the second of the sit. The simulator answered the request about the seat with an `ObjectUpdateCompressed` |
| 66 | `ctl stand`, `ctl sit <uuid>`, `ctl status`, with `listen.py` running | The bot's position is the prim's plus the offset the helper sees | Pass: the helper saw `-0.7 -0.2 0.9` on the prim; 130, 128, 25.55 plus that is the bot's `129.33, 127.85, 26.45`. Standing, the helper's one full update said z 27.1 and the bot, following the avatar down, 26.01 |
| 67 | `kick user Sit Bot bye` after item 65 | Login asking for the persisted location, then seated again | Pass: `start: uri:Target&129&128&26`, seated in the same second |
| 68 | `persist = "cfg"`; items 3, 5 and 9 again; then the same config with a `sit` UUID that does not exist (in place of item 8, which deletes the prim) | Holds `Target/120/120/25 <uuid>` before and after sitting; re-sit after 5 s; login at `uri:Target&120&120&25`; return from `Home` to 120, 120, 25; three failed sits, then a teleport to 120, 120, 25, and no `config: persist` line | Pass, all of it |
| 69 | `persist = "off"`; `stand user`, `kick user`, then `ctl home` | Sits once, `the avatar is in place and persist is off`, `mode: manual`; no re-sit; login with `start: last` and no sit; `home` sits on the configured object | Pass |
| 70 | `persist = ["cfg", "cur"]`; a `[persist]` table with `position = "cur"`, `sit = "cfg"` | `config: persist cfg cur` holding `Target/120/120/25 ...`; `config: persist cur cfg` holding the seated position | Pass, both; with the array, `teleport user ... Home/...` was answered by a teleport to 120, 120, 25 and a sit |
| 71 | Account and `persist = "cur"` only, avatar last in `Target` (was item 58) | `start: last`; holds the place it arrived at with `stand`; teleports back after `teleport user ... Home/...`; after a kick logs in asking for that place; never sits | Pass: `persist: Target/129.33/127.6/26.45 stand`. The location is the one the login reported; the avatar then settled 0.5 m lower |
| 72 | `sit = <uuid>` and `persist = "cur"` without `region`: avatar last in `Target` (was item 61), then last in `Home` (was item 62) | In `Target`: sits, holds the seated position, returns and sits again after `teleport user`. In `Home`: holds `Home/...`, three failed sits, then a teleport to that place, and sitting goes on | Pass, both: failures at 15 s with delays of 5, 10 and 20 s, then `teleporting to "Home" at 128, 128, 25.96` |
| 73 | During item 72 in `Home`: `ctl tp 128 128 25 Target`, `ctl sit <uuid>` | `mode: manual`, and no `config: persist` line afterwards although the avatar now sits on the configured object | Pass |
| 74 | `ctl tp 100 100 25 Home`, `ctl sit <home prim>`, `ctl persist`; then `teleport user Sit Bot Target/128/128/25` | The command prints the place in `Home` and the prim, exit status 0, no new sit request; the bot teleports back to `Home` and sits again | Pass: `Home/129.33/127.53/26.34 <home prim>` |
| 75 | From `Home`, seated: `ctl persist "Target/128/128/25" ground`, with `listen.py` running | Status 0 at once; `standing up`, a teleport to `Target`, `sitting on the ground`; `bot ground sit: yes` | Pass |
| 76 | In `Target` at 128, 128: `ctl persist Target/10/10/25 stand` | The avatar stands up from the ground and is not moved | Pass: `bot ground sit: no`, position still 128, 128 |
| 77 | Seated: `ctl persist cur stand`; seated again: `ctl persist cur unchanged`, then `stand user`; seated: `ctl persist cur ground` | Stands up; stays seated, and after the simulator unseats it no sit request follows; stands up and then sits on the ground | Pass: no `AgentRequestSit` after the unseat; the helper saw the avatar leave the prim and then `bot ground sit: yes` |
| 78 | `kick user`, then before the next login `ctl persist`, `ctl persist cfg cfg` | The first: `the avatar is not connected to a region`, status 1, nothing changed. The second: status 0, and the login that follows asks for the configured position and sits | Pass |
| 79 | After item 74: `ctl tp 128 128 25 Target`, `ctl home` | `home` prints the persisted place in `Home` and the bot goes there and sits, not to the configured `Target` | Pass |
| 80 | `ctl sit <uuid>`, `ctl tp 110 120 30`, `ctl persist "Home/100/100/25" stand`, then `ctl persist last_cmd last_cmd` | Prints `Target/110/120/30 <uuid>`; the bot teleports there and sits | Pass |
| 81 | Seated in `Target`: `ctl persist "Home/128/128/25" cur`; `ctl persist cur <home prim>`; with the last `tp` in `Home` and the last `sit` in `Target`: `ctl persist last_cmd last_cmd` | Status 1 naming both regions each time; `persist` line and `mode` unchanged | Pass, all three |
| 82 | `ctl persist Target/128/128 stand`, `ctl persist cur chair`, `ctl persist Blue Lagoon/128/128/25 stand` (unquoted), `ctl home now` | Status 1 naming the argument and its accepted forms; for the third, that a region name of several words has to be quoted | Pass |
| 83 | `--verbose` without `scan_distance`, over all the runs above | One `RequestMultipleObjects` each time the avatar becomes seated, none otherwise | Pass: 15 of 15, 3 of 3, 2 of 2, 2 of 2, 1 of 1, and none in the runs where the avatar never sat |

Not covered on this rig: a seat in a linkset (the prims here are single, so
the prim the simulator names is the one asked for; the difference is in
unit tests only), a seat that is turned or that moves, and an avatar pushed
by a script (unit tests only). OpenSim answered the request about the seat
at once; the 10-second wait and the fallback to the position before the
sit are covered by unit tests only. Not tried on Second Life: any of this,
in particular whether the simulator there answers the request about the
seat and in which form, and the positions in its terse updates.

## Inventory, attachments, animations and sounds

`test-rig/stock.py` gives the bot account something to work with. It logs
in as `Sit Bot`, so the bot has to be stopped while it runs, makes the
folder `Bot gear` at the top of My Inventory (a `Bot gear` from an earlier
run goes to the Trash and is deleted first) and puts four items into it:
the objects `Radio` and `Watch` (boxes it rezzes and takes), the animation
`Dance` (a copy of the library's `tpose2`) and the sound `Bell` (OpenSim's
click sound, uploaded as a new asset). The account also has what OpenSim
gives every new one, among it `Clothing/Default Shirt`.

```sh
test-rig/stock.py                 # prints: stocked Bot gear/ (<uuid>) with Radio, Watch, Dance and Bell
```

`listen.py` prints, besides what is described above,
`bot attachment <item> on point <N>` and `bot attachment <item> gone` from
the object updates and kills it gets, `bot plays <asset>` and
`bot stops <asset>` from the bot avatar's animation list, and
`bot sound <asset> at <x> <y> <z> gain <g>` for a sound the bot triggers.
OpenSim describes the bot's avatar to the helper only when something about
it changes, so the helper takes an attachment on any avatar but its own to
be the bot's: the rig has just the two.

The items below were run with the config of item 1 plus `socket = true`,
`--verbose`, the release build, and `listen.py` running four metres away.
`<radio>`, `<watch>` and `<gear>` are the UUIDs `inventory` prints for the
two objects and the folder.

| # | Inject | Expected | Result (2026-11-19, OpenSim 9d00a55552) |
|---|---|---|---|
| 84 | `ctl inventory` | One line of five tab-separated fields per entry, in the order of the paths, folders with a `/` at the end; the Library is not in it; status 0; one log line with the count and no line per entry | Pass: 39 entries, among them `Bot gear/` with its four items, `Clothing/Default Shirt` as `clothing shirt`, the body parts as `bodypart` with `shape`, `skin`, `hair`, `eyes`, and the six links of `Current Outfit/` as `link`. With `--verbose`: four requests, of 1, 10, 10 and 1 folders |
| 85 | `ctl inventory "Bot gear"`, `ctl inventory -n Clothing`, `ctl inventory <gear>`, `ctl inventory -n /` | The four items; the two pieces of clothing; the same four lines with their full paths; the folders at the top | Pass, all four |
| 86 | `ctl inventory -a` before and after `ctl attach "Bot gear/Radio" left hand`; `ctl inventory -a Clothing` | Nothing and status 0; then the one line `<radio> object Left Hand attached Bot gear/Radio`; nothing | Pass |
| 87 | `ctl inventory Nowhere`, `ctl inventory "Bot gear/Radio"`, `ctl inventory -x`, `ctl inventory Objects Clothing` | Status 1: `no folder named "Nowhere" was found in My Inventory`; `no folder named "Radio" was found in "Bot gear"`; the option named; `takes one folder`. Nothing asked of the grid for the last two | Pass |
| 88 | `ctl attach "Bot gear/Radio" left hand` | `"Radio" <uuid> is attached to Left Hand`, status 0, within a second; the helper prints `bot attachment <radio> on point 5`; the log has the request and the outcome | Pass |
| 89 | `ctl attach <watch> 5` while the radio is on the left hand | Attached to `Left Hand` as well; `inventory -a` lists both; the helper sees both on point 5 | Pass: nothing was taken off to make room |
| 90 | `ctl attach "Bot gear/Radio" Chest` while it is attached; `ctl attach "Bot gear/Dance"`; `ctl attach "Bot gear/Radio" elbow`; `ctl attach "Bot gear"`; `ctl attach /` | Status 1: `is attached to Left Hand already`; `is an animation, not an object`; `"elbow" is not an attachment point`; `no item named "Bot gear"`; `My Inventory itself is a folder`. Nothing sent to the simulator | Pass, all five |
| 91 | `ctl detach left hand` with both attached | One answer naming `Radio` and `Watch`, status 0; the helper prints `gone` for both; `inventory -a` prints nothing | Pass |
| 92 | `ctl attach "Bot gear/Radio"` (no body part) after it was on the left hand, and again after it was on the chest | Attached where it was last | Pass: `Left Hand`, then `Chest`. OpenSim keeps the last point with the object, not in the inventory item, so the listing shows `-` for it while it is not attached |
| 93 | `ctl detach "Bot gear/Radio"`; `ctl wear "Bot gear/Radio"`; `ctl unwear "Bot gear/Radio"` | Detached; attached as by `attach`; detached | Pass |
| 94 | `ctl detach "Bot gear/Watch"` (not attached), `ctl detach Chest` (nothing on it) | Status 1: `is not attached`; `nothing is attached to Chest` | Pass |
| 95 | `ctl attach "Bot gear/Radio" HUD Center`, `ctl tp 128 128 25 Home`, `ctl inventory -a`, `ctl home`, `ctl inventory -a` once seated again | `is attached to HUD Center`; in `Home` and back in `Target` the listing shows the radio on `HUD Center`; the log has `attachment: item <radio> is on HUD Center` after each arrival | Pass. OpenSim described the attachment in full after each teleport. Commands given while the teleport back was under way (it took 12 s) answered `the avatar is not connected to a region` |
| 96 | `ctl attach "Bot gear/Radio" right hand`, 20 s later `ctl quit`, then start the bot | Without any command: `attachment: item <radio> is on Right Hand` in the log, and `inventory -a` lists it | Pass. With the bot stopped within a second of the attach, OpenSim had not saved the attachment yet and the avatar came back without it (`attachments show Sit Bot` on the console: none), which the bot reported correctly |
| 97 | `ctl anim play "Bot gear/Dance"`, `ctl anim stop "Bot gear/Dance"`, and `stop` once more | `playing "Dance" <uuid>`, `stopped ...`, `stopped ...`, status 0 each; the helper prints `bot plays 01645eeb-...` (the animation's asset) and then `bot stops` it | Pass |
| 98 | `ctl anim play "Bot gear/Radio"`, `ctl anim pause "Bot gear/Dance"`, `ctl anim play` | Status 1: `is an object, not an animation`; the two forms of `anim` named, twice | Pass |
| 99 | `ctl sound "Bot gear/Bell"`; `ctl sound "Bot gear/Dance"` | `played "Bell" <uuid>`, status 0, and the helper prints `bot sound <asset> at <the bot's position> gain 1.0`; then status 1, `is an animation, not a sound` | Pass. A first version of `stock.py` made the sound item without an asset; `sound` refused that one with `the inventory does not give the bot the sound itself` |
| 100 | `ctl wear "Clothing/Default Shirt"`, `ctl unwear shirt`, `ctl unwear skin`, `ctl wear "Bot gear/Bell"` | Status 1: twice `this grid makes the avatar's appearance in the viewer, which the bot cannot do; ...`; `skin can only be replaced, with wear`; `is a sound, which cannot be worn`. `inventory "Current Outfit"` unchanged | Pass |
| 101 | Seated in holding mode: all of the above but item 95 | `ctl status` still shows `seat: target` and `mode: holding` | Pass |
| 102 | `kick user Sit Bot bye`, then before the next login `ctl inventory`, `attach`, `wear`, `detach` of an item and of a body part, `unwear shirt`, `anim play`, `sound` | Status 1, `the avatar is not connected to a region`, for each; nothing is done after the bot is back | Pass, all eight; `inventory -a` after the re-login printed nothing |
| 103 | Two folders `Bot gear` at the top (left by an earlier `stock.py`), `ctl inventory "Bot gear"` | Status 1; the error lists the folders' UUIDs | Pass, with three of them, while `stock.py` did not yet clear up after itself (debug build): `My Inventory holds 3 folders named "Bot gear": ...; name one by its UUID` |
| 104 | `--verbose`, no inventory command given: items 1, 3 and 9 | As before; no line starting `inventory:` in the log (each request to the inventory service writes one), and no message sent that was not sent before | Pass: login, seated; re-sit after `stand user`; back from `Home` and seated. Sent: `AgentUpdate`, `CompletePingCheck`, `UseCircuitCode`, `CompleteAgentMovement`, `RegionHandshakeReply`, `AgentThrottle`, `AgentRequestSit`, `AgentSit`, `RequestMultipleObjects` (one per sit), `MapNameRequest`, `TeleportLocationRequest` |

Not covered on this rig, because OpenSim has neither service: wearing and
taking off clothing and body parts, and the bookkeeping in the Current
Outfit folder after an attach or a detach (unit tests against a local HTTP
listener only; what the bot sends is taken from viewer source). Also not
covered: an attachment described in the compressed form or as a cached
object (OpenSim describes attachments in full; unit tests only for the
compressed form, and a cached one the bot would not learn of), an
attachment detached by its own script, an attach the simulator refuses
(the 15-second error and the simulator's alert in it are in unit tests
only), a listing that takes longer than 50 seconds, and a link in a path
(OpenSim's default inventory has links only in `Current Outfit`, where they
were listed as `link`). Not tried on Second Life: any of this.

Not covered: Windows, where the socket is TCP. On Second Life, looking a
name up through `AvatarPickerRequest` found nobody (2026-11-18, an avatar
that had just messaged the bot); the capability is what viewers use there,
and with the bot using it too, `ctl im` by name reached that avatar
(checked by hand, 2026-11-19).

Seen in a viewer as well: the avatar seated on the prim in `Target`. Not covered on this rig:
a region that stays offline while the
rest of the grid is up, and anything specific to Second Life.
