# Local OpenSim test rig

slbot is verified against a local OpenSimulator standalone with two regions:

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
3. Create `slbot.toml` from `slbot.example.toml`: keep `login_url`, `user` and
   `password`, set `object` to the UUID and `position` to a spot next to the
   prim (`[128.0, 128.0, 25.0]` for the helper's prim).

Run the bot with `cargo run -- slbot.toml`. Add `--verbose` to see every
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

Seen in a viewer as well: the avatar seated on the prim in `Target`. Not covered on this rig:
a region that stays offline while the
rest of the grid is up, and anything specific to Second Life.
