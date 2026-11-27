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

On macOS the simulator does not start without `libgdiplus`, which its
module for drawing on textures loads (Homebrew: `mono-libgdiplus`). The rig
needs none of what that module does; without the library, add this to the
end of `test-rig/opensim/bin/OpenSim.ini`, as was done for the run of
[Web interface](#web-interface):

```ini
[Modules]
    Setup_VectorRenderModule = disabled

[Map]
    GenerateMaptiles = false
```

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

## XMPP server (Prosody)

The XMPP gateway is tried against a local Prosody (tested with 13.0.7;
Arch/Manjaro: `sudo pacman -S prosody`). `test-rig/setup.sh` writes its
config to `test-rig/prosody/prosody.cfg.lua` (git-ignored) from
`test-rig/config/prosody.cfg.lua.in` and registers two accounts on the
virtual host `rig.test`, each with its name as password:

| Account | Role |
|---|---|
| `owner@rig.test` | owner of the bot |
| `stranger@rig.test` | anyone else |

and the component `bot.rig.test` with the secret `rigsecret` on
`127.0.0.1:5347`. Clients connect to `127.0.0.1:5222` without TLS. Start it
in the foreground, as your own user:

```sh
prosody --config test-rig/prosody/prosody.cfg.lua -F
```

`prosodyctl --config test-rig/prosody/prosody.cfg.lua check config` must end
with "All checks passed". (The full `check` also complains about DNS and
certificates, which the rig has none of.)

Prosody also offers STARTTLS, with a certificate for `rig.test` signed by
a CA of the rig's own, `test-rig/prosody/certs/rig-ca.crt`, which
`setup.sh` makes once. Desktop clients need it:

- Gajim: add the account `owner@rig.test` (password `owner`), and under
  the account's connection settings set the host `127.0.0.1`, port 5222.
  It asks once whether to trust the certificate.
- Dino insists on STARTTLS, accepts only certificates the system trusts and
  has no field for the server's address. So `rig.test` has to resolve and
  the CA has to be trusted, which takes root (undo with the same commands,
  `trust anchor --remove` and deleting the line):

  ```sh
  echo '127.0.0.1 rig.test' | sudo tee -a /etc/hosts
  sudo trust anchor test-rig/prosody/certs/rig-ca.crt
  ```

`test-rig/xmpp.py ACCOUNT [ACTION...]` is a minimal client for the
checklists: it logs in, sends presence, carries out actions such as
`msg:bot.rig.test:status` or `sub:bot.rig.test`, and prints every stanza it
receives. Any desktop client works too, with the account and server
`127.0.0.1`, port 5222, encryption off.

The bot's config gets the table:

```toml
[xmpp]
component = "bot.rig.test"
secret = "rigsecret"
owners = ["owner@rig.test"]
```

## Payments

OpenSimulator has no money of its own, so the rig brings a money module,
`test-rig/money/RigMoneyModule.cs`. `setup.sh` copies it with its
`prebuild.xml` into `test-rig/opensim/addon-modules/RigMoney/` and builds
it with the simulator, and the `OpenSim.ini` it creates selects it. An
`OpenSim.ini` from before needs the line by hand, under `[Economy]` (where
the example has `; economymodule = BetaGridLikeMoneyModule`):

```ini
[Economy]
    economymodule = RigMoneyModule
```

and `test-rig/setup.sh` has to be run again, with the simulator stopped: it
rebuilds the simulator when the module is missing from `bin` or has
changed. The start log then has `[RIG MONEY]: enabled; every avatar starts
with L$1000`.

The module keeps balances in memory only: every avatar has L$1000 when the
module first meets it, and a restart of the simulator forgets everything.
It answers `MoneyBalanceRequest` with the balance (a `MoneyBalanceReply`
whose `TransactionInfo` is all zeros) and `RequestPayPrice` with the root
prim's pay price, as OpenSimulator's sample module does. A
`MoneyTransferRequest` of type 5001 (gift to an avatar) or 5008 (pay an
object) moves the money, and both parties, when online, get a
`MoneyBalanceReply` with `TransactionInfo` filled in as Second Life does:
the type, the payer as source, the avatar paid as destination (for an
object, its owner), the amount, and as item the request's description
(empty: `Payment` for a gift, the object's name for an object). A payment
to an object goes to its owner and starts the `money` event of its
scripts. A payer with too little money, an unknown avatar or object, an
amount below L$1 or another type gets a `MoneyBalanceReply` with
`TransactionSuccess` false and a description saying why; nothing moves. The
simulator marks objects with a `money` event as taking money by itself.

Console commands of the module:

| Command | Does |
|---|---|
| `rigmoney show` | every balance the module knows, and whether it is silent |
| `rigmoney show Rig Alt` | one avatar's balance |
| `rigmoney set Rig Alt 50` | sets an avatar's balance |
| `rigmoney silent on` | transfers still move the money, but nobody gets a `MoneyBalanceReply` about them, nor about refused ones: a payment that is never confirmed |
| `rigmoney silent off` | back to telling both parties |

A third account, `Rig Alt` (password `rigalt`), stands in for the owner's
second avatar, so that money can go between two avatars other than the
bot. Create it on the OpenSim console:

```
create user Rig Alt rigalt rigalt@example.com 7e570000-0000-4000-8000-0000000000a1 default
```

`test-rig/payee.py` rezzes a box that takes money: it logs in as `Rig
Owner` (`--as alt` or `--as bot` for the others), uploads a script with
`llSetPayPrice` and a `money` event, puts it into a box named `Payee box`
(`--name`) at 130, 131 in `Target`, next to the bot's place, and prints
the box's UUID and position and the pay price the simulator gives for it.
The prices are arguments: `--price` for the price box and up to four
buttons, each L$, `hide` or `default`; buttons not given are hidden. When
paid, the box says in local chat (and with `--channel N` on channel N as
well) `paid L$<amount> by <name> (<UUID>)`. `--help` has the rest.

```sh
test-rig/payee.py --price hide 100 250   # no price box; buttons L$100 and L$250
```

A box from an earlier run stays: delete it on the OpenSim console with
`delete object name "Payee box"` and answer `y`.

`test-rig/pay.py` logs in as `Rig Owner` (`--as alt`, `--as bot`), asks for
its balance, pays and prints every `MoneyBalanceReply` and `PayPriceReply`
it gets and what is said in local chat, for three seconds (`--wait`).
`--hex` adds each money message sent and received as hex, from the message
number on.

```sh
test-rig/pay.py 100                     # Rig Owner gives Sit Bot L$100 (type 5001)
test-rig/pay.py 100 "Rig Alt"           # ... Rig Alt
test-rig/pay.py --as alt --object 36424416-7b9e-4620-a3a0-1ec0e53bff49 100
                                        # Rig Alt pays the box (type 5008), after RequestPayPrice
test-rig/pay.py --balance --wait 30     # Rig Owner's balance, then 30 s of what it is told
```

The box's UUID is the one `payee.py` printed. With the box owned by `Rig
Owner`, the third command printed for `Rig Alt` (after its balance; the
box's line and the reply come in either order)

```
PayPriceReply: object=36424416-7b9e-4620-a3a0-1ec0e53bff49 default=-1 buttons=[100, 250, -1, -1]
paid L$100 to 36424416-7b9e-4620-a3a0-1ec0e53bff49 (type 5008)
MoneyBalanceReply: success=True balance=L$1000 transaction=102b7c8e-... description='You paid Payee box (Rig Owner) L$100.'
  TransactionInfo: type=5008 (pay object) source=Rig Alt dest=Rig Owner amount=L$100 item='Payee box'
chat: Payee box: paid L$100 by Rig Alt (7e570000-0000-4000-8000-0000000000a1)
```

while the fourth, running alongside as `Rig Owner`, got the other side of
it, `description='Rig Alt paid you L$100.'` with the same transaction, and
`rigmoney show` had `Rig Owner` L$100 richer.

## Groups

The rig runs OpenSim's Groups V2 with a store of its own,
`test-rig/groups/RigGroupsData.cs`, which keeps the groups in memory (the
rig is on SQLite, for which OpenSim has no groups store): **groups are
gone when the simulator stops**, and `test-rig/group.py` makes them again.
`setup.sh` builds the store as an addon module next to the money module
and writes the lines below into a new `OpenSim.ini`. A rig set up before
needs `setup.sh` run again and these lines added by hand to
`test-rig/opensim/bin/OpenSim.ini`, which `setup.sh` never overwrites:
right under the line `[Groups]`,

```ini
    Enabled = true
    Module = "Groups Module V2"
    StorageProvider = "RigGroupsData.dll"
    ServicesConnectorModule = "Groups Local Service Connector"
    LocalService = local
    MessagingEnabled = true
    MessagingModule = "Groups Messaging Module V2"
    MessageOnlineUsersOnly = true
    NoticesEnabled = true
```

and at the end of the file (the groups service otherwise looks for the
grid-user store in the groups store):

```ini
[GridUserService]
    StorageProvider = "OpenSim.Data.SQLite.dll"
```

The start log then has `Initializing Groups Module V2` and
`GroupsMessagingModule enabled`. The money module takes a group's fee
when Groups V2 lets an avatar join (`ApplyCharge`), and reports it as
transaction type 1004 with the avatar as payer.

`test-rig/group.py` logs in as `Rig Owner` (`as=alt` for `Rig Alt`) and
does one thing; `group.py` alone prints its usage:

```sh
test-rig/group.py create Builders open             # prints the new group's UUID
test-rig/group.py create Dancers fee=250 open      # with a fee of L$250
test-rig/group.py invite Builders                  # invites the bot; `invite Builders alt` Rig Alt
test-rig/group.py say Builders hello all           # starts the session and says a line
test-rig/group.py notice Builders Meeting "Tonight at eight" item   # with the item Agenda attached
test-rig/group.py eject Builders                   # ejects the bot
test-rig/group.py as=alt join <uuid>               # Rig Alt joins, and prints the fee's MoneyBalanceReply
test-rig/group.py as=alt leave Builders            # Rig Alt leaves, and prints the LeaveGroupReply
test-rig/group.py profile Builders
test-rig/group.py listen 60                        # prints the group chat, notices and IMs it gets
```

Only one run per account at a time: a second login of the same account
is refused while the first lasts. `dump` prints the group messages as hex
and the events as LLSD; `test-rig/captures/groups.txt` holds such
captures, which the unit tests read.

What Groups V2 does not do, so the rig cannot show it: an invitation with
a fee (it always says there is no cost, and takes none on acceptance),
refusing a join for closed enrollment (it checks neither enrollment nor
an invitation; the bot refuses that itself), and stored notices. OpenSim
also sends every notice marked as stored, with the time it was sent (the
bot counts one that is less than a minute old as live), marks the lines
of a group's chat as from the group, delivers a speaker's own line back,
has no `ChatSessionRequest`, and makes a group the avatar joins its
active group. Every group member starts in the role Everyone, without the
right to invite.

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
| 48 | `ctl accept tp "Rig Owner"` with no offer waiting (`accepttp "Rig Owner"` is the same command under its older name) | `there is no teleport offer from Rig Owner`, status 1 | Pass (as `accepttp`; run again on 2026-11-20 as item 138) |
| 49 | The helper's unasked offer, then `ctl accept tp "Rig Owner"` twice | The log notes the offer and that `rslbot ctl accept tp` accepts it; the avatar stays; the first `accept tp` prints `arrived in Target`; the second is refused | Pass (as `accepttp`; run again on 2026-11-20 as items 138 and 139, with `accept tp` and with `accepttp`) |
| 50 | `kick user Sit Bot bye` in manual mode | Login after 10 s with `start: last`; no teleport and no sit afterwards; `mode: manual` | Pass |
| 51 | `ctl home` in the target region, and again after `ctl tp 128 128 25 Home` | `placement: holding`; the bot sits, or teleports to `Target` and sits; `seat: target`, `mode: holding` | Pass, both |
| 52 | `ctl shout over here`, `ctl whisper psst` | The helper hears the shout; `chat [shout]` and `chat [whisper]` rows from the echo | Pass; the whisper was out of the helper's earshot at 20 m |
| 53 | `ctl quit` | Status 0; `logged out`; the bot exits and the socket file is gone | Pass |
| 54 | `scan_distance = 64`, then `ctl touch` with a UUID that does not exist, 14 s after the start | `scanning for objects within 64 m` on entering the region; status 1 at once, naming 64 m | Pass |
| 55 | `scan_distance = 5000`; `requesttp_timeout = 0` | Error naming the key, status 2 | Pass, both |
| 56 | Items 3 and 9 again, with no command given since `home` | The bot sits again and teleports back as before | Pass |

Not covered on this rig: what the scan costs and finds. In these runs
OpenSim described every object in full and none as cached, so the bot had
nothing to ask about and sent no `RequestMultipleObjects` even after a scan
started; that path, and the effect of the scan distance, are covered by
unit tests only until tried on Second Life. (OpenSim does mention an object
as cached once it has stood unchanged for five minutes, though never an
attachment; no scan was run against such objects.) Also not covered: an avatar's request to be
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
| 105 | What loses an attachment on Second Life (README, Status): `attach "Bot gear/Radio" Right Hand`, 20 s, `quit`; log in; `attach "Bot gear/Watch" Right Hand`, 15 s, `detach "Bot gear/Watch"`, 20 s, `quit`; log in; `inventory -a` | `Radio` is listed on `Right Hand` at both logins | Pass: listed both times, and `attachment: item ... is on Right Hand` in each login's log. OpenSim does not lose it |
| 106 | `attach "Bot gear/Watch" Left Hand` and `quit` within the same second; log in; `inventory -a` | As in item 96: OpenSim has not saved it | `Watch` is not attached after the login, twice out of two tries (the other with `Radio`). OpenSim saves a changed appearance five seconds after the change (`DelayBeforeAppearanceSave`), and a logout before that loses it. Not a fault of the bot, and not what `restore_attachments` is for |
| 107 | `restore_attachments` at its default (on), `--verbose`: items 1, 3 and 9, half a minute without a command after the login, then items 88 to 94 and 96 | As before. OpenSim offers no outfit service, so nothing is restored: no line starting `inventory:` before the first command, and none with `restor` at all | Pass: seated at login, again 5 s after `stand user`, back from `Home` and seated 10 s after the teleport; the log's only word on the matter is `the simulator has no outfit service`. Attach and detach answers as in items 88 to 94 (`attach` of the attached radio, of an animation and to `elbow` refused; both objects on `Left Hand` detached by body part). After item 96's new login the radio was reported on `Right Hand` and 31 s passed without an `inventory:` line until `inventory -a` was run |

Not covered on this rig, because OpenSim has neither service: wearing and
taking off clothing and body parts, and the bookkeeping in the Current
Outfit folder after an attach or a detach (unit tests against a local HTTP
listener only; what the bot sends is taken from viewer source). For the
same reason the rig cannot show an attachment being restored after a login,
or an attach that arrives late getting its link: unit tests only until
tried on Second Life. Also not
covered: an attachment described in the compressed form or as a cached
object (OpenSim describes attachments in full; unit tests only for the
compressed form, and a cached one the bot would not learn of), an
attachment detached by its own script, an attach the simulator refuses
(the 30-second error and the simulator's alert in it are in unit tests
only), a listing that takes longer than 50 seconds, and a link in a path
(OpenSim's default inventory has links only in `Current Outfit`, where they
were listed as `link`). What was tried on Second Life is under Status in
`README.md`.

## RLV

`test-rig/relay.py` gives the bot account what nothing else on the rig
has: scripted objects of its own that say RLV commands to their owner. It
logs in as `Sit Bot`, so the bot has to be stopped while it runs, uploads
one script, rezzes the box `Relay box` with it two metres from the bot's
landing point and puts a second one, `Relay`, into the folder `RLV gear`
at the top of My Inventory (an `RLV gear` from an earlier run goes to the
Trash and is deleted first). The script says to its owner what its owner
says on channel 77, and repeats on channel 0, as `heard on 5: ...`, what
its owner says on channel 5:

```sh
test-rig/relay.py                 # prints the box's UUID and: stocked RLV gear/ (<uuid>) with Relay
rslbot ctl say /77 @versionnew=5  # the box tells the bot `@versionnew=5`; the bot answers on channel 5
```

A `Relay box` from an earlier run stays in the region and would say every
command a second time: delete it first on the OpenSim console with
`change region Target`, `delete object name "Relay box"` and `y`.

The items below were run with the config of item 1 plus `socket = true`,
`rlv = true`, `log_chat` and `--verbose`, the release build, a
`RUSSELBOT_SOCK` of the run's own, and a second prim `<bench>` rezzed with
`rezprim.py` beside the seat. `say /77 X` stands for
`rslbot ctl say /77 X`, which makes the box say `X` to the bot.

| # | Inject | Expected | Result (2026-11-19, OpenSim 9d00a55552) |
|---|---|---|---|
| 108 | `rlv status`; `say /77 @versionnew=5`; `say /77 @version=5,versionnum=5` | `rlv: on`, `persist: drop`; one `rlv: Relay box <uuid>: ...: said "..." on channel 5` line per query; the box repeats each reply | Pass: `RestrainedLove viewer v0.1.0 (RusselBot 0.1.0)`, `RestrainedLife viewer v0.1.0 (RusselBot 0.1.0)` and `10000`, each heard back from the box as `heard on 5: ...`. `chat.tsv` has the `@` lines as kind `owner` and the box's echoes as `say` |
| 109 | Holding the seat (`persist = "cfg"`): `say /77 @unsit=force`, `status`, `home` | `rlv: ...: as \`stand\``, `rlv: standing up`, `placement: manual`; `seat: none`, `mode: manual`, the seat still on the `persist` line; `home` sits again | Pass |
| 110 | `say /77 @sit:<bench>=force`; `say /77 @sitground=force` | Seated on the bench, `mode: manual`; the ground sit is sent | Pass. The ground sit changes nothing while the avatar sits on an object, as with `ctl sit ground` |
| 111 | `say /77 @detach=n,getstatus=5,nonsense` | Two `not supported` lines and one `malformed: it has no \`=\``; nothing said on channel 5, nothing changed | Pass |
| 112 | `say /77 @sit:<a UUID that is no object>=force` | `rlv: sit refused: the simulator did not confirm the sit within 15 s` and `sit failed: ...` 15 s later | Pass |
| 113 | `say "/77 @tpto:Home/128/100/25;1.57=force"`; in `Home`, `rlv cmd @tpto:256384/256128/25=force` | The avatar arrives in `Home` at 128, 100; then in `Target` at 128, 128 (`Target` is at 1001, 1000, so its corner is 256256, 256000); `mode: manual` | Pass, both; the second logs `rlv: teleporting to 128, 128, 25 in the region at 256256, 256000` |
| 114 | `rlv cmd @tpto:Nowhere/1/2/3=force,versionnum=5`; `rlv cmd @tpto:north/128=force`; `rlv cmd @detachme=force` | `ok` for the teleport and `the avatar is not connected to a region` for the query behind it, status 1; `rlv: tp refused: region "Nowhere" not found` seven seconds later; the other two refused with their reasons, status 1 | Pass |
| 115 | `rlv cmd @versionnum=5`; `rlv cmd @versionnum=5,detach=n` | `versionnum=5: 10000`, status 0; both lines on standard error, the second `detach=n: not supported`, status 1 | Pass |
| 116 | `attach "RLV gear/Relay" left hand`, `say "/77 @detach:left hand=force"`; `attach ... HUD Center`, `say "/77 @remattach:center=force"`; `attach ... chest`, `say "/77 @detachme=force,remoutfit:shirt=force,detach:chin=force"` | `Relay` is detached each time; from the worn relay `@detachme` is carried out as `detach <its item>`, from the box it is refused as `the sender is not attached`; `remoutfit` is refused for want of the outfit service; `nothing is attached to Chin` | Pass. Worn and rezzed relay both hear channel 77, so every command is dealt with twice; the second detach of the same item finds it gone already |
| 117 | `rlv mode off`; `say /77 @versionnew=5,unsit=force`; `rlv cmd @unsit=force`; `rlv mode on` | Nothing answered or done, no `rlv:` line but the two for the switch; `rlv cmd` refused naming `rlv mode on`; the `@` line is in `chat.tsv` | Pass |
| 118 | `rlv persist follow`; standing in manual mode, `say /77 @sit:<bench>=force`; `status` at once and ten seconds later | `rlv: ...: persist cur <bench>` and `placement: holding Target/<where it stood> <bench>`; the bot sits; `rlv: following: persist cur cur` and `placement: holding Target/<the seat> <bench>`; `mode: holding` | Pass: 128/128/26.03 first, 129.33/128/26.34 within the second |
| 119 | Then `stand`, `home`; `kick user Sit Bot bye` | Manual mode; `home` sits on the bench; after the kick the bot logs in and sits on the bench | Pass |
| 120 | `say /77 @unsit=force`; `say /77 @sitground=force` | `rlv: following: persist cur stand` and `... ground`; `mode: holding` after each, the `persist` line ending in `stand`, then `ground` | Pass |
| 121 | `say "/77 @tpto:Home/100/120/30=force"`; `rlv cmd @sit:<bench>=force`; `kick user Sit Bot bye` | `placement: holding Home/100/120/30 stand` on arrival; the sit refused naming `Target` and `Home`, nothing changed; the next login is in `Home` | Pass |
| 122 | `rlv cmd @tpto:Nowhere/1/2/3=force`; then `rlv cmd @tpto:256384/256128/25=force` | Manual mode while the teleport is under way; `rlv: not following: the teleport failed; back to holding as before` and the `Home` placement unchanged; then `holding Target/128/128/... stand` | Pass |
| 123 | `say /77 @sit:<a UUID that is no object>=force`; forty seconds; `home`; `persist cur stand` | The UUID is held at once and the sit is tried again after 5 and 10 seconds; `home` holds the same and tries again; `persist cur stand` ends it | Pass |
| 124 | The bot started with a `[persist]` table of `atstart = true` and `rlv = "follow"`; with only `rlv = "follow"`; with `atstart = false` and `sit = "cfg"` | `config: persist cur cur` after the first sit, `persist: follow`; manual mode after the first sit, `persist: follow`; rejected naming `persist.atstart` and `persist.sit` | Pass, all three |

Not covered here: RLV commands that arrive during a login or a teleport
(the relay only says what the bot says, and the bot cannot talk then; the
keeping and its limit are unit-tested), an attachment that asks for the
version by itself when it is rezzed, `@remoutfit` carried out (OpenSim
offers no outfit service), a region larger than 256 metres for the global
`@tpto`, and `@detachme` from a child prim. Nothing of RLV has been tried on
Second Life.

## Permission requests

`test-rig/asker.py` gives the rig what asks the bot for things. It logs in
as `Rig Owner`, uploads one script and rezzes two boxes with it next to
the seat, `Asker box` and `Asker box 2`, which stay in the region. Each
does what anyone tells it on its channel, 78 for the first box and 79 for
the second, to whoever spoke:

```sh
test-rig/asker.py              # prints the two boxes' UUIDs
rslbot ctl say /78 ask 16      # the first box asks the bot for permission to trigger animations
rslbot ctl say /79 give        # the second box gives the bot a copy of its script
test-rig/asker.py 14 give=4 nobox   # Rig Owner itself gives the bot the script, and prints the answer
```

The number after `ask` is the permission bits: 16 trigger animations, 4
take controls, 32 attach, 2 debit. A box says `granted: <bits>` on channel
0 when the bot has answered, which the bot hears and logs. `Rig Owner`
cannot be logged in twice, so `asker.py` and `listen.py` take turns. Boxes
from an earlier run have to go first, or every request is made twice: on
the OpenSim console `change region Target`, `delete object name "Asker
box"`, `y`, and the same for `"Asker box 2"`.

The items below were run with the config of item 1 plus `socket = true`,
`log_chat`, `--verbose`, the release build, a `RUSSELBOT_SOCK` of the
run's own, and these tables, `<box 2>` being the second box's UUID:

```toml
[perms.autoaccept]
tp = ["Rig Owner"]
anim = ["Rig Owner"]
inventory = ["Rig Owner"]

[perms.except]
anim = ["<box 2>"]
```

`say /78 X` stands for `rslbot ctl say /78 X`, and the other commands for
`rslbot ctl` with them.

| # | Inject | Expected | Result (2026-11-20, OpenSim 9d00a55552) |
|---|---|---|---|
| 125 | `perms status` | `tp: Rig Owner`, `inventory: Rig Owner`, `anim: Rig Owner except <box 2>`, `attach: none`, `controls: none`, `camera: none` | Pass |
| 126 | `say /78 ask 16` | With `--verbose`: `received ScriptQuestion`, `sending RequestObjectPropertiesFamily`, `received ObjectPropertiesFamily`, `sending ScriptAnswerYes`. `permission request (anim) from Asker box <uuid> of Rig Owner <uuid>: accepted by itself`; the box says `granted: 16`; the bot stays seated, `mode: holding` | Pass, all four messages within the second |
| 127 | `say /79 ask 16` (the excepted box); `perms status`; `accept anim` | `...: waits; \`rslbot ctl accept anim\` accepts it`; a `waiting: anim from Asker box 2 <uuid> of Rig Owner <uuid>` line; `granted Asker box 2 ...: trigger animations`, and the box says `granted: 16` | Pass |
| 128 | `say /78 ask 20` (animations and controls); `accept anim Nobody`; `accept controls "Rig Owner"`; `accept anim` | `permission request (anim, controls) ...: waits, as controls is not auto-accepted for it`; status 1 for `Nobody`; `granted ...: take controls, trigger animations` and `granted: 20` from the box; status 1, `no request of kind \`anim\` waits` | Pass |
| 129 | `say /78 ask 18` (debit and animations) | `... cannot be accepted: it asks for permission to take money from the avatar, which the bot never grants`; nothing is sent, and nothing waits | Pass |
| 130 | `perms autoaccept anim "Rig Owner"`; `perms revoke anim rig.owner`; `perms revoke anim <box 1>`; `perms autoaccept anim <box 1>`; `perms autoaccept money all`; `perms autoaccept anim all "Rig Owner"`; `perms revoke anim`; then `say /78 ask 16` | `warning: Rig Owner was auto-accepted for anim already`, status 0; `anim: rig.owner is not auto-accepted any more`; `warning: <box 1> was not auto-accepted for anim already`; `anim: <box 1> is auto-accepted`; status 1 naming the kinds; status 1, `\`all\` stands alone`; status 1 with the usage; the box, now listed by its UUID, is granted | Pass |
| 131 | `perms autoaccept controls all`; `perms autoaccept anim all`; `say /78 ask 20`; `say /79 ask 16` | `anim: everyone is auto-accepted except <box 2>`; the first box gets `granted: 20` by itself; the second box's request waits | Pass |
| 132 | `perms autoaccept anim <box 2>`; `say /79 ask 4`; `perms revoke controls <box 2>`; `say /79 ask 4`; `perms status` | The exception to `anim` is gone; `granted: 4` by itself; `controls: <box 2> is not auto-accepted any more (an exception to \`all\`)`; the second request waits; `controls: all except <box 2>` | Pass |
| 133 | With `controls` not accepted: `say /78 ask 4`, then `say /78 ask 16` | The first waits; the second is accepted by itself, and no request of that script waits any more | Pass. An earlier build of the same day kept a script's older request waiting when a newer one was accepted (seen after item 132, where `anim` from the second box still waited); fixed before this item was run |
| 134 | `say /78 give` | `inventory offer from the object "Asker box" of Rig Owner <uuid> (script 'Asker script'. ...): accepted by itself`; `inventory` lists `Scripts/Asker script` | Pass: the owner's name was looked up, the top level of the inventory read, and the answer sent, within the second |
| 135 | `test-rig/asker.py 14 give=4 nobox` | `inventory offer from Rig Owner <uuid> (script Asker script): accepted by itself`; the helper prints `answer 5 from Sit Bot`; a second `Scripts/Asker script` | Pass |
| 136 | `perms revoke inventory all`, twice; `say /78 give`; `say /79 give`; `accept inventory "Somebody Else"`; `accept inventory "Rig Owner"`; `accept inventory` | `inventory: nothing is auto-accepted any more`, then the warning; two `waiting: inventory from the object "..." of Rig Owner <uuid>` lines; status 1; the second box's offer accepted, then the first's | Pass, in two runs. With an earlier build of the same day the owner's name was looked up only where a rule had a name in it: both offers waited as `of the avatar <uuid>`, `accept inventory "Rig Owner"` found nothing, and `accept inventory <owner's uuid>` and `accept inventory` accepted them. The name is now always looked up; with that build one offer from the first box waited with its owner's name and was accepted with `accept inventory "Rig Owner"` |
| 137 | `test-rig/listen.py 16 offer=5`; `status`; `home` | `teleport offer from Rig Owner <uuid>: accepted by itself`, `placement: manual`, `teleport arrived`; the avatar is at the helper's position, `seat: none`, `mode: manual`, the seat still on the `persist` line; `home` sits again | Pass |
| 138 | `perms revoke tp all`; `accept tp`; `accepttp "Rig Owner"`; the helper's offer; `accept tp "Nobody Here"`; `accept tp`, twice | `no teleport offer waits`, status 1; `there is no teleport offer from Rig Owner`, status 1; the offer `waits`, the avatar stays seated; status 1 for `Nobody Here` at once; `arrived in Target`, `mode: manual`; the second is refused | Pass |
| 139 | `home`; the helper's offer; `accepttp rig.owner`; `accepttp` | `arrived in Target`; `no teleport offer waits`, status 1 | Pass |
| 140 | With a script's request and an inventory offer waiting: `kick user Sit Bot` on the console | The bot logs in again and sits; `perms status` shows the rules as the commands left them and no `waiting:` line | Pass |
| 141 | With a script's request and an inventory offer waiting: `tp 100 100 25 Home`; `perms status`; `accept inventory`; `home` | The script's request is gone; the inventory offer still waits and is accepted from `Home` | Pass |

Not covered here: an object owned by a group (no rig helper deeds one), a
simulator that does not answer the question about an object's owner, a
grid that does not know or does not give the owner's name, a simulator
without an inventory service, more than 100 requests waiting, and the
`attach` and `camera` kinds, which go the way of `anim` and `controls`.
All of these are unit-tested. OpenSim sends the debit request of item 129
to an avatar that is not the object's owner; whether Second Life does is
not known. Nothing of this has been tried on Second Life.

## RLV restrictions

Run on 2026-11-20 with the debug build, the config of item 1 plus
`socket = true`, `rlv = true`, `persist = "cfg"` and `log_chat`, a
`RUSSELBOT_SOCK` of the run's own, the `Relay box` and the wearable
`Relay` of `test-rig/relay.py`, and the prim `<bench>` of `rezprim.py`.
`say /77 X` makes every relay that hears it say `X` to the bot, so the box
and the worn relay each become the holder of what is said. `console: X` is
typed on the OpenSim console.

| # | Inject | Expected | Result (2026-11-20, OpenSim 9d00a55552) |
|---|---|---|---|
| 142 | Seated: `rlv status`; `say /77 @unsit=n`; `rlv status`; `stand`; `tp 128 128 25 Home` | Four lines ending in `restrictions: none`; then `restrictions: 1` and `Relay box <uuid>: unsit`; both commands refused with status 1, naming `unsit`, the box and `rlv mode override`; the avatar stays seated, `mode: holding` | Pass |
| 143 | `rlv cmd @sendim=n,getstatus=5`; `im "Rig Owner" hi`; `rlv cmd @clear`; `rlv status` | `sendim=n: ok` and `getstatus=5: /sendim`; the `im` refused naming `sendim, held by the operator`; `clear: ok`; the box's `unsit` is still listed | Pass |
| 144 | `rlv mode override`; `stand`; `rlv status` | `rlv: override`; the avatar stands; `rlv: on again; \`stand\` was exempt`; `placement: manual`; `rlv: on` | Pass |
| 145 | `say /77 @clear`; `say /77 @getstatus=5`; `say /77 @sendchat=n,chatshout=n,touchworld=n,fartouch=n`; `say /77 @getstatus=5` | `clear: lifted 1 restriction`; the reply `""` is said on channel 5; then `/sendchat/chatshout/touchworld/fartouch` | Pass. The simulator delivers the empty reply: the box says `heard on 5: ` with nothing after it, then `heard on 5: /sendchat/chatshout/touchworld/fartouch` |
| 146 | With those four held: `say hello there`; `say /me nods slowly. Then she speaks at length`; `touch <bench>`; `say /77 @touchworld:<bench>=add,fartouch=y`; `touch <bench>`; `say /77 @sendchat=y`; `shout over here`; `rlv reset` | Refused naming `sendchat`; `/me nods slowly` is said and the answer is `shortened to "/me nods slowly", because of sendchat, ...`; refused naming `touchworld`; touched; `over here` at normal volume with `said at normal volume instead of shouted, because of chatshout, ...`; `rlv reset: 3 restrictions cleared` | Pass |
| 147 | `home`; `attach "RLV gear/Relay" chest`; `say /77 @detach=n`; `detach "RLV gear/Relay"`; `detach chest` | The box and the worn relay each hold `detach`; the first refused naming `detach, held by Relay <uuid>`, the second with `"Relay" <item> stayed on: refused by RLV: detach, ...`; nothing is detached | Pass |
| 148 | `say /77 @detach:left hand=n,addattach=n`; `rlv mode override`; `detach "RLV gear/Relay"`; `rlv status`; `attach "RLV gear/Relay" chest` | The relay is detached; `rlv: dropped 3 restrictions of Relay <uuid>: the object is gone`; the box's three are still listed; the attach is refused naming `addattach` and the box | Pass |
| 149 | `rlv reset`; `attach "RLV gear/Relay" chest`; `say /77 @tploc=n`; `console: teleport user Sit Bot Home/128/128/25`; `rlv status`; `status`; `say /77 @tploc=y` | `rlv: dropped 1 restriction of Relay box <uuid>: the avatar left its region`; the worn relay keeps `tploc`; one line `rlv: holding back the teleport to "Target": tploc, held by Relay <uuid>`, no teleport, the avatar stays in `Home`; after the lift `no longer holding back ...`, the teleport, `seated` | Pass: the relay had the same UUID in `Home`; back and seated 4 s after the lift |
| 150 | `say /77 @tploc=n,sit=n`; `console: teleport ... Home`; `rlv persist override tp`; in `Target`, `say /77 @sit=y`; `rlv persist override none` | Held back; then `rlv: the teleport to "Target" goes ahead although tploc, held by Relay <uuid>: persist override tp`; in `Target`, `rlv: holding back the sit on <seat>: sit, held by Relay <uuid>`; after the lift the avatar sits at once | Pass |
| 151 | With `tploc` held by the worn relay: `console: teleport ... Home`; `rlv mode override`; `home`; when seated, `console: teleport ... Home` again; `say /77 @tploc=y` | Held back; `rlv: on again; \`home\` was exempt`, the teleport and the sit; the second time the teleport is held back again until the lift | Pass |
| 152 | Seated: `say /77 @unsit=n`; `persist cur stand`; `status`; `rlv persist override sit` | The persist is answered with status 0 and `... stand`; `rlv: holding back the stand: unsit, held by Relay <uuid>`, the avatar stays seated, `mode: holding`; then `rlv: standing up although unsit, ...: persist override sit` and `seat: none` | Pass |
| 153 | Config: a `[persist]` table with `position = "cfg"`, `sit = "cfg"` and `override = "tp"`. `rlv status`; `say /77 @tploc=n`; `console: teleport ... Home` | `rlv: persist override: tp` at startup and `persist override: tp` in the status; `goes ahead although tploc ...` at once, back in `Target` | Pass |
| 154 | Item 153 with `rlv cmd @sittp=n` given first; then `console: kick user Sit Bot bye`; `rlv status`; `rlv cmd @clear` | In `Target`: `rlv: holding back the sit on <seat>: sittp, held by the operator (the bot does not know how far the object is)`; after the kick `rlv: dropped 1 restriction of 1 object: the session ended`, a login 10 s later that nothing holds back, `operator: sittp` still listed and the sit still held back; after the clear the avatar sits | Pass, as built at the time. Without `scan_distance` the simulator did not describe the seat to the bot after an arrival, so `sittp` held the sit back for as long as it was in force (item 157). The bot now looks for the seat; see item 158 |
| 155 | The box alone: `say /77 @unsit=n`; `stand`; `say /77 @unsit=force` | The operator's `stand` is refused; the box's own force command is carried out: `rlv: Relay box <uuid>: unsit=force: as \`stand\``, the avatar stands, `placement: manual` | Pass |
| 156 | `say /77 @sendchat=n`; `say hello`; `rlv mode off`; `say hello`; `say /77 @sendchat=y`; `rlv cmd @clear`; `rlv status`; `rlv mode on`; `say hello` | Refused; said; the lift is not read while off; `rlv cmd` refused naming `rlv mode on`; the status lists `Relay box <uuid>: sendchat` under `rlv: off`; once on, refused again | Pass |
| 157 | Config of item 1 plus `scan_distance = 64`: `rlv cmd @sittp:20=n`; a login (see the result); then `rlv cmd @clear,sittp=n`; `stand`; `sit <seat>` | After the arrival `holding back the sit ... (the bot does not know how far the object is)`, and as soon as the scan has described the seat `no longer holding back ...` and `seated`; with the plain `sittp` the `sit` is refused with `(the object is 2.0 m away)` | Pass: held back and released within the same second. The teleport to `Home` that was to bring this about ended in `session lost: event queue reports no such session` 4 s after the bot started its teleport back, so the arrival was that of the next login. That loss was seen this once and not looked into; items 149 to 153 made the same teleport five times without `scan_distance` and without a loss |
| 158 | Config of item 1 with `socket`, `rlv` and `persist = "cfg"`, no `scan_distance`. Seated: `rlv cmd @sittp:20=n`; `console: teleport user Sit Bot Home/128/128/25` | On the return to `Target`: `scanning for objects within 32 m`, `rlv: holding back the sit on <seat>: sittp:20, held by the operator (the bot does not know how far the object is)`, then `no longer holding back ...` and `seated` | Pass: all four lines in the second of the arrival |
| 159 | `rlv cmd @clear,sittp=n`; `stand`; `tp 126 128 25`; `sit <seat>` | Refused with status 1: `sittp, held by the operator (the object is 4.0 m away)` | Pass. Right after the `stand` the avatar is within 1.5 m of the seat and the benches, and `sit` on them is carried out |
| 160 | `tp 128 128 25 Home`; `rlv cmd @clear,sittp:5=n`; `sit <Home prim>`; eleven seconds later `stand` and `sit <a UUID that is no object>` | The first `sit` starts the scan (`control: sit: looking for <prim> first: sittp:5, ...`), and once the prim is described the avatar sits and the command exits with status 0; the second is refused at once, saying that the bot does not know how far the object is | Pass: answered 0.08 s after it was given; the second within 0.01 s |
| 161 | `rlv cmd @clear,sittp:20=n`; `tp 128 128 25 Target`; `say /77 @sit:<bench>=force` (the `Relay box`) | `rlv: Relay box <uuid>: sit:<bench>=force: as \`sit <bench>\``, `scanning ...`, `rlv: sit: looking for <bench> first: ...`, then `rlv: sitting on <bench>` and `seated` | Pass, in the same second |
| 162 | In `Home`, wearing `Relay`: `persist cur stand`; `say /77 @sit:<Home prim>=force` | `mode: holding` until the prim is described; then the sit, `placement: manual` | Pass |
| 163 | `rlv persist follow`; in a fresh visit to `Home`, `persist cur stand`; `say /77 @sit:<Home prim>=force` | `rlv: Relay <uuid>: sit:<prim>=force: looking for <prim> first: ...`; nothing persisted until the prim is described; then `persist cur <prim>`, the sit, `following: persist cur cur`, `mode: holding` | Pass |
| 164 | `rlv persist drop`; `rlv cmd @clear,touchattach=n,sittp=n`; `tp 128 128 25 Target`; `touch <bench>`; `sit <a UUID that is no object>`; then `rlv reset`, a fresh visit to `Home` and `sit <Home prim>` | `control: touch: looking for <bench> first: touchattach, ...`, the scan, `control: touched <bench>`; the `sit` refused with status 1 when the scan has had its 10 seconds, saying that the bot does not know how far the object is; without restrictions the `sit` goes out at once and no scan starts | Pass: the touch answered in 0.08 s; the `sit` refused 9.9 s after it was given, the scan having started with the touch; no `scanning` line in `Home` |

Not covered on this rig: a region restart that leaves the avatar in a hub
(OpenSim's restart ends the session), teleport offers and requests under
`tplure`, `accepttp`, `tprequest` and `accepttprequest`, script permission
requests under `acceptpermission` and `denypermission`, clothing (no
outfit service), a restriction set from a child prim of an attachment, the
heads-up display and other avatars' attachments, and instant messages
under `startim`. All of these are unit-tested. Nothing of this has been
tried on Second Life.

## XMPP gateway

Run on 2026-11-20 with the debug build, Prosody 13.0.7 as described in
[XMPP server (Prosody)](#xmpp-server-prosody), the config of item 1 without
`sit` plus `socket = true`, `log_chat`, `log_im`, a `RUSSELBOT_SOCK` of the
run's own and the `[xmpp]` table given there. The owner's side is
`test-rig/xmpp.py owner ...`, the stranger's `test-rig/xmpp.py stranger
...`; `listen.py` answers the bot's IMs with `pong: <text>`, `talk.py`
speaks as `Rig Owner` (address `rig.owner@bot.rig.test`).

| # | Inject | Expected | Result (2026-11-20, OpenSim 9d00a55552, Prosody 13.0.7) |
|---|---|---|---|
| 165 | Prosody stopped; start the bot; start Prosody after about 40 s | the avatar logs in at once; `cannot connect` lines with `next attempt in` 5, 10, 20, 40 s; `component bot.rig.test connected` at the next attempt after Prosody is up | Pass: attempts at 03:30:46, :51, 03:31:01, :21; Prosody up at 03:31:28; connected at 03:32:01, the avatar in `Target` from 03:30:46 on |
| 166 | Owner: `sub:bot.rig.test sub:local@bot.rig.test` | `subscribed` for both, then the domain available with status `Target` and `local@` available | Pass |
| 167 | Owner: `msg:bot.rig.test:status` | one message from the domain with the text `rslbot ctl status` prints | Pass: `connected: yes`, `region: Target`, ... `mode: manual` |
| 168 | `listen.py 40`; owner: `msg:rig.owner@bot.rig.test:hello from xmpp` | Rig Owner gets the IM; `im_out` in the log; the `pong` comes back from `rig.owner@bot.rig.test` with the nickname `Rig Owner` | Pass |
| 169 | Owner to `local@bot.rig.test`: `hello everyone`, `/shout come here`, `/5 open` | said, shouted, and said on channel 5; the owner gets the bot's own lines from `local@` | Pass: `listen.py` heard say and shout; the two relays answered `heard on 5: open`, and all four lines came back from `local@` as `Sit Bot: hello everyone`, `Sit Bot shouts: come here`, `Relay: heard on 5: open`, `Relay box: heard on 5: open` |
| 170 | Owner to `obj-<uuid>@bot.rig.test`: `hi` | an error `feature-not-implemented` with a text; nothing to the grid | Pass |
| 171 | `talk.py` while the owner is online | `Rig Owner: hello from the rig`, `Rig Owner whispers: psst`, `Rig Owner shouts: HEY`, the line with a tab and a line break as it was, `are you there?` from `rig.owner@` with nickname, `Object: Script running` | Pass |
| 172 | Stranger: `msg:jane.doe@bot.rig.test:hello`, `msg:bot.rig.test:quit`, `sub:bot.rig.test` | two `forbidden` errors, `unsubscribed`; log lines naming `stranger@rig.test/rig`; the bot keeps running | Pass |
| 173 | Owner: `tp 128 128 25 Home`, then `tp 128 128 25 Target` | an answer to each when the teleport ends; the domain's status follows the region | Pass: `arrived in Home`, `arrived in Target`; during each teleport the domain shows away `not in a region` and `local@` offline for a moment, as the presence table has it for a bot between regions |
| 174 | Stop Prosody; `talk.py`; start Prosody, the owner online | the six messages after reconnecting, in order, each with a delay stamp of the time it was heard | Pass: all six with stamps 03:34:01 to 03:34:05. Prosody was down 35 s, so the delay had grown to 40 s and the component came back 40 s after Prosody |
| 175 | Prosody stopped and started again 7 s later | connected again within 10 s of Prosody accepting connections; the avatar stays online | Pass: Prosody up at 03:38:41, connected at 03:38:48 |
| 176 | Owner: `msg:bot.rig.test:quit` | `ok`, then unavailable presence from both addresses; the bot logs out and exits 0 | Pass (before a fix the same day, the unavailable presence went out twice) |
| 177 | Config with a wrong `secret` | `the server refused: not-authorized (...)` naming `bot.rig.test`, retried after 5 and 10 s; the secret nowhere in the log; `rslbot ctl status` answered | Pass |
| 178 | Owner: `im "Rig Owner" see you later` and `help` to the domain | `sent to Rig Owner <uuid>` once the name is found; the command list | Pass |

### Desktop clients

Tried on 2026-11-20 with Gajim 2.6.0 and Dino 0.5.1, both logged in as
`owner@rig.test` over STARTTLS at once, while the rig bot ran in `Target`
and `listen.py` answered IMs as Rig Owner. The owner, by hand: presence of
`bot.rig.test` and `local@`, `status`, `hi` to `rig.owner@bot.rig.test`,
`hi` to `no.body@bot.rig.test`, `test` to `local@`, `tp 128 128 25 Home`.
Then Prosody was stopped at 03:57:27, the bot sent Rig Owner two IMs
whose pongs were recorded at 03:57:27 and 03:57:30, and Prosody was started
again at 03:58:25; both clients were logged in again before the component
reconnected at 03:58:41.

| Client | What it showed |
|---|---|
| Gajim 2.6.0 | Everything as the specification describes, by the owner's account of it: presence, the answers, the nickname, the failed message, the delayed pongs |
| Dino 0.5.1 | Received the messages. Could not send: Dino starts every new conversation with OMEMO on (its default encryption) and refuses to send to an address that publishes no OMEMO devices, which no address of the gateway does. Switching the conversation to "Unencrypted" with the lock icon in the message box is Dino's way round it. After that, by the owner's account and the bot's log: an IM to Rig Owner (in Cyrillic) went out and its pong came back with the nickname `Rig Owner`, a line in local chat was said, and `hi` to `no.body@bot.rig.test` showed as failed with the reason (`im refused: the grid knows no avatar named "no.body"` at 04:03:37) |
| Conversations | Not tried (Android) |

Not covered on this rig: a hypergrid visitor's address, an IM from a
script, offline IMs (which the rig cannot produce), an avatar with a display name of its
own, a server that stops reading (unit tests only), and more than 500
messages waiting (unit tests only).


Not covered: Windows, where the socket is TCP. On Second Life, looking a
name up through `AvatarPickerRequest` found nobody (2026-11-18, an avatar
that had just messaged the bot); the capability is what viewers use there,
and with the bot using it too, `ctl im` by name reached that avatar
(checked by hand, 2026-11-19).

Seen in a viewer as well: the avatar seated on the prim in `Target`. Not covered on this rig:
a region that stays offline while the
rest of the grid is up, and anything specific to Second Life.

## Autoreplies

Run on 2026-11-20 with the debug build, the config of item 1 plus
`persist = "cfg"`, `socket = true`, `rlv = true`, `log_im = "im.tsv"` and

```toml
[autoreply]
message = "Hello, I'm bot. I won't respond to you in next 2 minutes"
pause = "2m"
```

and a `RUSSELBOT_SOCK` of the run's own. `test-rig/listen.py` with
`im=T1,T2,...` sends the bot `hello <T>` that many seconds after it starts
listening, prints the automatic answers it gets as `autoresponse` lines
(dialog 20), and never answers those. `ctl` is `rslbot ctl` on that socket.

| # | Inject | Expected | Result (2026-11-20, OpenSim 9d00a55552) |
|---|---|---|---|
| 179 | `listen.py 135 im=3,8,128` | `hello 3` answered with the text as an `autoresponse` in the conversation of a plain IM; `hello 8` not answered; `hello 128`, after the pause, answered; `im.tsv` has each `im` row and after the first and third an `im_auto` row | Pass. OpenSim passes dialog 20 on; the answer's conversation id is the two avatar ids combined (`...001a`), as for a plain IM |
| 180 | `ctl autoreply refresh`; an IM | `forgot the pauses of 0 avatars` (the pause of 179 had run out); the IM answered | Pass |
| 181 | `ctl im "Rig Owner" one moment`, then the helper's `pong` and an IM | Neither is answered | Pass. Checked again on its own (`refresh`, then `ctl im "Rig Owner" brb`, then an IM): no answer to the `pong` or the IM |
| 182 | `ctl autoreply off`; an IM | `autoreply: off`; not answered | Pass |
| 183 | `ctl autoreply on`; `ctl autoreply refresh`; `ctl rlv cmd @recvim=n`; an IM | The helper gets "The Resident you messaged is currently prevented from reading your instant messages at the moment, please try again later." as an `autoresponse`; `im.tsv` has `(blocked by RLV: recvim)` in place of the text, then the `im_auto` row | Pass |
| 184 | `ctl rlv cmd @recvim=y,sendim=n`; an IM | Recorded with its text, not answered; `autoreply: not sent to Rig Owner <uuid>: sendim is in force, held by the operator` | Pass. The `autoreply: not sent` line comes before the `im` line on standard error: the message waits the moment its names take, the log line does not |
| 185 | `ctl rlv cmd @sendim=y`; an IM; `ctl autoreply status` | Answered (the held-back answer started no pause); the status lists `on`, the text, `pause: 2m` and the avatars in their pause | Pass |

Not covered on this rig: a viewer showing the answer (no viewer was at
hand), stored offline IMs (the rig cannot produce them), IMs from
scripts, and two bots with autoreply on (both unit-tested: a bot drops
dialog 20 unread). Nothing of this has been tried on Second Life.

## Area scan

Run on 2026-11-21 with the debug build and `--verbose`: `rslbot ctl scan`
and the `seat name` line of `status`. The config is that of item 1 plus
`persist = "cfg"` and `socket = true`, without `scan_distance` unless a row
says so, and a `RUSSELBOT_SOCK` of the run's own. `ctl` is `rslbot ctl` on
that socket. The seat was rezzed with `rezprim.py Target name=Seat`.

`rezprim.py` rezzes what the rows name, beside the seat (give each run its
own `east=` so the prims do not stack):

```sh
test-rig/rezprim.py Target name="Park Bench" desc="Sit here" east=3            # <bench>
test-rig/rezprim.py Target name=Button desc=Teleporter touch east=5            # <button>
test-rig/rezprim.py Target name=Sofa child=Cushion childtouch east=8           # <sofa>, child <cushion>
test-rig/rezprim.py Target name="Far lamp" east=90                             # <lamp>
```

| # | Inject | Expected | Result (2026-11-21, OpenSim feccd9cc53) |
|---|---|---|---|
| 186 | `ctl scan`, the first command of the bot's stay | `scanning for objects within 32 m` in the log; answered no sooner than 3 s; one line each for the seat, `<bench>`, `<button>` and `<sofa>`, nearest first, each with seven tab-separated fields, owner `Rig Owner` and its UUID; no line for `<lamp>`, `<cushion>` or an avatar; with `--verbose`, `ObjectSelect` and `ObjectDeselect` once each and one `AgentThrottle` after the scan started | | Pass: answered after 3.0 s with the four lines. The seat was not selected: its name was known from the sit |
| 187 | `ctl scan` again | The same lines within a second; no `ObjectSelect` | | Pass: 0.01 s |
| 188 | `ctl scan 5`; `ctl scan 200` | Only what is within 5 m; then `scan distance raised to 200 m` in the log and `<lamp>` listed with a distance near 90 | | Pass: three lines; then 3.0 s and `<lamp>` at 84.7. Raising the distance made OpenSimulator offer the neighbouring region (`EnableSimulator`), which the bot ignores |
| 189 | `ctl touch <lamp>` after item 188; `ctl scan` | The touch is answered at once; the scan without a distance lists `<lamp>` | | Pass, both |
| 190 | `ctl scan -n bench`; `ctl scan --name "park b"`; `ctl scan -d tele` | `<bench>` alone, twice; `<button>` alone | | Pass, all three |
| 191 | `ctl scan -o "rig owner"`; `ctl scan -o <Rig Owner's UUID>`; `ctl scan -o "Nobody Here"` | Everything, twice; then nothing, status 0 | | Pass, all three |
| 192 | `ctl scan -t`; `ctl scan -s` | `<button>` with `scripted,touch`. Record whether `<sofa>` is listed too, that is, whether OpenSimulator marks the root prim for a script in the child | | Pass. OpenSimulator marks only the prim that holds the script: `<sofa>` is listed by neither, and `-c` (item 193) lists `<cushion>` with `scripted,touch,child` |
| 193 | `ctl scan -c -n cushion`; `ctl scan -n cushion`; `ctl scan -c -t` | `<cushion>` with the sofa's distance and `child` in its flags; nothing; `<button>` and `<cushion>` | | Pass, all three |
| 194 | `ctl attach "Bot gear/Radio"` (see `stock.py`), then `ctl scan -a` and `ctl scan` | The radio with distance `0.0` and `attachment`; not listed without `-a` | | Pass; with `-a -c` likewise |
| 195 | `ctl scan -x`; `ctl scan -n`; `ctl scan 0`; `ctl scan 10 20` | Status 1 each, naming the argument; with `--verbose` nothing is sent | | Pass, all four |
| 196 | `ctl scan 96 &` and at once `ctl tp 128 128 25 Home` | The scan exits with status 1 saying the avatar left the region; `ctl scan` in `Home` reaches 32 m again | | Pass, with the first `scan` of a stay given 18 s after arriving. Sooner than that the scan was answered first: a teleport by region name took about 10 s, as OpenSimulator sends the map's answer behind the terrain, which it sends at 1000 bytes a second. That is so without a scan too |
| 197 | Stop the bot's session with `kick user Sit Bot bye`, and `ctl scan` before the next login | Status 1, `the avatar is not connected to a region` | | Pass |
| 198 | `ctl rlv mode on`, `ctl rlv mode override`, `ctl scan`, `ctl rlv status` | The scan is answered; `rlv: override` still | | Pass |
| 199 | `ctl status` while seated on the configured seat, named `Seat` in a viewer or rezzed with `name=Seat` | `seat: target`, then `seat name: Seat`; with `--verbose`, one `RequestObjectPropertiesFamily` after the sit and no `ObjectSelect` | | Pass. A `status` in the moment after the sit has no `seat name` line yet |
| 200 | `ctl sit <cushion>`, `ctl status`; `ctl stand`, `ctl status` | `seat: object` with the prim OpenSimulator names and `seat name: Sofa`; then `seat: none` and no `seat name` line | | Pass. OpenSimulator names the root prim for a sit on the child: `seat: object <sofa>`. No request for the name: the scan had it |
| 201 | Rename `<bench>` in a viewer, `ctl scan -n bench`; then `ctl tp 128 128 25 Home`, `ctl home`, `ctl scan` | The old name still; the new one after the round trip | | Pass, renamed with `ObjectName` from a script instead of a viewer |
| 202 | `scan_distance = 64`, wait a minute, `ctl scan` | Answered within a second or two (only the properties are asked for); with `--verbose`, no `ObjectSelect` before the command | | Pass: asked 11 s after the login, answered in the second the properties were asked for |

Found on this run, and the same with the bot as it was before `scan`: with
`scan_distance = 64`, a teleport back into the region next door lost the
session. That is fixed since; see "Teleports next door" below.

Not covered on this rig: Second Life, where the distance decides what the
simulator describes at all; a region with more objects than fit one
`ObjectSelect`; the 30 second limit; an owner the name service does not
know and objects owned by a group (unit tests only; the rig has groups
now, see [Groups](#groups), but no helper deeds an object to one).

## Teleports next door

Run on 2026-11-21 with the debug build and `--verbose`, the config of item
1 plus `persist = "cfg"` and `socket = true`, and a `RUSSELBOT_SOCK` of the
run's own; `ctl` is `rslbot ctl` on that socket. `Home` and `Target` are
neighbours, and the avatar is moved between their middles.

With a draw distance of 64 m or more there, this OpenSimulator counts the
neighbouring region as in view (it adds 64 m to the draw distance) and
keeps the avatar's presence in the region it leaves as a child agent,
together with the client and the UDP address the bot had there. A bot that
came back from another port (`Handling UseCircuitCode request ... from IP
127.0.0.1:<new port>`) had that presence closed under it (`Close has been
called for Sit Bot attached to scene Target`, then an exception in
`HandleUseCircuitCode`) and lost the session: `event queue reports no such
session` or `simulator handshake timed out`, a login refused as "already
logged in", and about a minute offline. The bot now sends on one UDP port
for the whole session, keeps the connection to a region it left alive
until the simulator closes it, and returns over it.

A round trip is `ctl tp 128 128 25 Home`, 20 seconds, `ctl home`, then
waiting for `seated on`.

| # | Inject | Expected | Result (2026-11-21, OpenSim feccd9cc53) |
|---|---|---|---|
| 203 | `scan_distance = 64`: three round trips in a row | Each ends `seated`; one login in the whole log and no `session lost`; at each return the simulator logs `Reusing existing child scene presence for Sit Bot ... in Target` and neither `Handling UseCircuitCode request` for `Target` nor `Close has been called ... scene Target` | Pass, three of three. `Home` closes its own presence when the avatar is back in `Target` (`Close has been called ... scene Home`; the bot logs `the simulator at 127.0.0.1:9000 closed the circuit the avatar left`), and the next trip opens a new connection to it from the same port |
| 204 | `scan_distance = 128`: three round trips in a row | As 203 | Pass, three of three. At this distance both regions keep the presence: `Reusing existing child scene presence` at every arrival from the second on, one `Handling UseCircuitCode request` for `Home` in the whole run and no `Close has been called`; the bot logs `continuing the circuit to 127.0.0.1:9000` and `...:9001` |
| 205 | `scan_distance = 64`: `ctl tp 128 128 25 Home`, five minutes, `ctl home` | The simulator logs no `No packets received from child agent`; the return is as in 203 | Pass: the bot answered the pings of `Target` for the five minutes and was seated one second after `home` |
| 206 | `ctl status` and `ctl scan` after a return of 203 | `region: Target`, `seat: target`, `seat name: Seat`; the scan lists the objects by the seat | Pass. The owner's name came out as `?` in every line, and did so with the bot built from the commit before as well, without any teleport: the simulator had been started for this run and `Rig Owner` had not logged in since |
| 207 | Item 9 with `scan_distance = 64`: `teleport user Sit Bot Home/128/128/25` on the console while the bot holds its seat | `connected, in region "Home" (not the target region)`, a teleport back without a new login, `seated` | Pass: back in `Target` and seated 5 s later. This is the return that lost the session before |
| 208 | Items 44, 45 and 51 again, without `scan_distance` | As recorded there | Pass, all three. At 32 m neither region keeps the presence: each closes it once the avatar has arrived next door, as before |

Tried with builds that were not kept, to see what the simulator needs
(`scan_distance = 64`, one round trip): with one UDP port but packet
numbers starting again at 1, the simulator took the return for old packets
(`Received a duplicate (not marked as resend) of packet #2, type
CompleteAgentMovement`) and the session was lost; with the old connection's
numbering carried on but the connection left unanswered while the avatar
was away, the first round trip passed and the second lost the session,
because the bot had not heard `Home` close the connection.

Not covered on this rig: Second Life; a region that restarts while it
keeps the avatar's presence; walking or flying across a region border,
which the bot does not follow.

## Startup script

Run on 2026-11-21 with the debug build, the config of item 1 plus
`socket = true`, and a `RUSSELBOT_SOCK` of the run's own; `ctl` is
`rslbot ctl` on that socket. The script is given as `startup = '''...'''`;
its lines are shown here separated by `;`. The configs of items 209 to 216
have none of the deprecated keys unless the row says so.

| # | Inject | Expected | Result (2026-11-21, OpenSim feccd9cc53) |
|---|---|---|---|
| 209 | Script: `rlv mode on`; `rlv persist follow`; `perms autoaccept tp "Jane Doe"`; `perms autoaccept anim all`; `perms revoke anim <uuid>`; `autoreply message Away from the keyboard`; `autoreply pause 2h`; `persist`; `say in place`. `ctl rlv status`, `ctl perms status` and `ctl autoreply status` 0.3 s after the start | A `startup: line N: ...` and a `startup: line N answered: ...` line for each of the seven setting commands, all before `logging in`; the three answers show the settings while the login is under way | Pass: `rlv: on`, `persist: follow`; `tp: Jane Doe`, `anim: all except <uuid>`; `autoreply: on`, the text, `pause: 2h` |
| 210 | The same run, after `seated`; then `ctl status` | `startup: the avatar is in place; carrying out the rest of the startup script`, `placement: manual`, `startup: line 10: persist`, `placement: holding` with the seated position and the seat, `startup: line 11: say in place`, `startup: the script is done`; the same placement on the `persist` line, `mode: holding` | Pass: `Target/129.33/128/26.45 <uuid>`, in the second of the sit. A config with `persist = "cur"` in place of the script ended with the same `persist` line (item 65) |
| 211 | Script: `persist cfg cfg`; and, in its place, `persist = "cfg"` | Both end with `persist: Target/128/128/25 <uuid>` and `mode: holding` | Pass, both; the key also logged `` `persist` is deprecated; write the startup script command `persist cfg cfg` instead `` |
| 212 | Script: `rlv mode on`; `say first`; `sit <a UUID that is no object>`; `persist`; `say never`. Then `ctl status`, `ctl rlv status` | `say first` is carried out, the sit is refused after 15 s, and the log says that the script stops and two commands are left out; the bot stays connected, in manual mode, with RLV on | Pass: `startup: line 3 refused: the simulator did not confirm the sit within 15 s; the startup script stops here: 2 commands after it will not be carried out` |
| 213 | `sit` set to a UUID that is no object; script: `rlv mode on`; `say hello`; `persist`. After the first failed sit, `ctl tp 128 128 25 Home`; in another run, `ctl persist cur stand` | The rest is given up with a line that counts the commands; RLV stays on; the bot is where the command put it | Pass: `startup: the avatar was moved before it was in place; 2 commands of the startup script, from line 2 on, will not be carried out`, `region: Home`, `mode: manual`, `rlv: on`. With `persist`: `a placement was persisted before the avatar was in place; 1 command ...` |
| 214 | `rlv = true` and `persist = "cfg"` with the script `rlv mode off`; `say hello`. `ctl rlv status` 0.3 s after the start | The two deprecation lines, `rlv: on`, then `startup: line 1: rlv mode off` and `rlv: off` before `logging in`; after `seated`, `config: persist cfg cfg` and `placement: holding`, then `startup: line 2: say hello` | Pass, in that order; `rlv: off` from the socket during the login |
| 215 | Script: `rlv mode on`; `perms autoaccept money all`; `say hello` | Exit status 2, an error naming `startup`, line 2 and the kinds, and no `logging in` line | Pass: ``rslbot: h.toml: `startup`: line 2: `money` is no kind of request; the kinds are `tp`, `inventory`, `anim`, `attach`, `controls` and `camera` `` |
| 216 | Script: `rlv mode on`; `autoreply message "Away.` / `Back tomorrow."` (one command on two lines); `persist`; `say in place`. When it is done, `kick user Sit Bot` on the console | The autoreply text has two lines; after the kick the bot logs in and sits again, and the log has no further `startup:` line | Pass: ten `startup:` lines before the kick and none after it; `seated` again after the next login, `mode: holding` |
| 217 | No script: `persist = "cfg"`, `rlv = true` and an `[autoreply]` table, with the build from the commit before this change and with the new one. After `config: persist cfg cfg`: `ctl rlv status`, `ctl autoreply status`, `ctl status`, `kick user Sit Bot`, and `ctl status` once seated again | The same answers and the same log from both, but for three deprecation lines at the start of the new one | Pass: the answers are identical, and the new log has the three lines for `persist`, `rlv` and `autoreply` and is otherwise the old one, except that one `attachment: item ... is on Left Hand` line, which follows the simulator's timing, came before the second `seated` in one run and after it in the other |

Not covered on this rig: a script command that RLV restrictions refuse; a
`startup_file` (unit tests read one); the XMPP gateway together with a
script.

## Avatars, friends and profiles

Run on 2026-11-22 with the debug build and `--verbose`: `scan avatars`,
`friends`, `profile` and the RLV restrictions on the two scans. The config
is the account, `region = "Target"`, `position = [128.0, 128.0, 25.0]` and
`socket = true`, with a `RUSSELBOT_SOCK` of the run's own; `ctl` is
`rslbot ctl` on that socket. The bot stands at 128, 128, 26. The objects
are those the area scan rezzed. The second avatar is `Rig Owner`, through
`listen.py SECONDS at=X,Y,Z`, which logs in at that place in `Target`.

Profiles need the simulator's profile service: `setup.sh` turns it on
(`ProfileServiceURL` under `[UserProfiles]` in `OpenSim.ini`, `Enabled =
true` under `[UserProfilesService]` in `StandaloneCommon.ini`). A rig set
up before that answers no profile: its `[UserProfiles]` section, present
but without a URL, switches both of OpenSimulator's profile modules off.
Set the two by hand there.

`test-rig/friend.py` makes and changes the friendship between `Sit Bot`
and `Rig Owner`: `make` (with the bot offline) has the first offer and the
second accept, `end` ends it, `hide` and `show` take away and give back the
bot's right to see `Rig Owner` online.

| # | Inject | Expected | Result (2026-11-22, OpenSim feccd9cc53) |
|---|---|---|---|
| 218 | `friend.py make` | `Sit Bot`'s next login lists `Rig Owner` in `buddy-list`; the bot logs `the grid lists 1 friend of the avatar` | Pass, both |
| 219 | Alone in the region: `ctl scan avatars`; `ctl friends`; `ctl profile "Rig Owner"` | Nothing, status 0; one line `offline`; the profile with `online: no` and `friend: yes`, over UDP (`the simulator has no profile service`) | Pass. The names were `?` in both, and the profile had no `name` lines: right after a simulator start this OpenSimulator names nobody who has not logged in since (as in item 206) |
| 220 | `listen.py 600 at=131,128,25`; `ctl scan avatars` twice; `ctl friends` | One line: UUID, `3.0`, `131,128,26`, `Rig Owner`, `Rig Owner`; the second within a second; `online` | Pass: 0.26 s, then 0.01 s |
| 221 | `ctl scan avatars whisper`, `chat`, `shout`, `50`, `2` | Listed by the first four, not by `2` | Pass |
| 222 | `ctl profile "Rig Owner"`; `ctl profile <its UUID>` | `id`, `name`, `display name`, `online: yes`, `friend: yes`, `born` as `YYYY-MM-DD`; one `AvatarPropertiesRequest` each; no `name` search for the UUID | Pass: `born: 2026-11-21`; three requests for the three `profile` commands of the run, nothing else |
| 223 | `ctl profile Nobody Here` | Status 1 naming `Nobody Here`, no profile asked | Pass |
| 224 | `scan avatars` in items 219 to 221 | With `--verbose`, no `scanning for objects`, no `ObjectSelect`, no raised distance; only the circuit's own messages sent | Pass |
| 225 | Stop `listen.py`; `ctl friends`; `ctl scan avatars` | `offline` once the simulator's notice is in; the avatar gone from `scan avatars` with the next list | Pass: `OfflineNotification` when it logged out; `scan avatars` still listed it a moment from the list, by the next list (within 5 s) not |
| 226 | `listen.py 240 at=200,40,25`; `ctl scan avatars`, `shout`, `100`, `120`, `chat` | One line about 114 m away at `200,40,`*height*; listed by the bare scan and `120` only | Pass: `113.7`, `200,40,25`. OpenSimulator describes every avatar of the region to every agent (`SendOtherAgentsAvatarFullToMe`), so the position is the exact one, not the list's; also with `ObjectsCullingByDistance = true`, tried for item 233 |
| 227 | `ctl rlv mode on`, `rlv cmd @shownames=n`; `ctl scan avatars`; `ctl scan`; `ctl scan -o "Rig Owner"` | `?`, distance, position, `?`, `?`; the objects with `?` as owner UUID and name; `-o` refused naming `shownames` | Pass, all three |
| 228 | `rlv cmd @shownames:<Rig Owner>=add`; `ctl scan avatars`; `ctl scan 3`; `ctl scan -o "Rig Owner"` | `Rig Owner` in full in both scans; `-o` still refused | Pass |
| 229 | `rlv cmd @clear`, `@shownearby=n`, `ctl scan avatars chat`; `@clear`, `@showminimap=n`, `ctl scan avatars` | Refused naming `shownearby`; then `?` as the position, distance and names kept | Pass, both |
| 230 | `ctl scan objects`; `ctl scan` | The same lines | Pass: the five objects of the area scan |
| 231 | `rlv cmd @camdrawmax:5=n`; `ctl scan`; `ctl scan 96`; `ctl scan avatars`; `rlv cmd @camdrawalphamax:0.5=n`; `ctl scan` | Nothing beyond 5 m and `cut to 5 m by camdrawmax` in the log, for both; no raised distance; `scan avatars` lists `Rig Owner` at 113.7; then the five objects again | Pass, all of it |
| 232 | `rlv cmd @shownearby=n,shownames=n`, `ctl rlv mode override`, `ctl scan avatars`, `ctl rlv status`, `ctl scan avatars` | Listed in full; `rlv: on`; refused naming `shownearby` | Pass |
| 233 | `ObjectsCullingByDistance = true` in `OpenSim.ini`; `listen.py at=200,40,25`; start the bot and give `ctl scan avatars` the moment it logs `connected` | The command waits for the simulator's list of the region's avatars and lists `Rig Owner` | Pass: answered after 1.76 s, the first `CoarseLocationUpdate` having come 2 s after the arrival (52 bytes: both avatars with their ids). `Rig Owner` had dropped to the ground, `200,40,1`, a height the list cannot hold below 4 m; listed exactly, being described all the same (see 226) |
| 234 | `friend.py hide`; `ctl friends`; `friend.py show`; `ctl friends` | `?`; `offline` | Pass, each after one `ChangeUserRights` |
| 235 | `ctl profile "Rig Owner"` right after item 234's `hide`, with `Rig Owner` just logged out | What the simulator says | `online: yes`: this OpenSimulator answers from its friends cache, which still had `Rig Owner` online, and does not apply the right it took away. A minute later, after `show`, `online: no` |
| 236 | `friend.py end`; `ctl friends`; `ctl profile "Rig Owner"` | Nothing; `<uuid> ended the friendship with the avatar` in the log; `friend: no` | Pass. The profile said `online: yes` again for the moment after `friend.py` logged out, as in 235 |

The rig was left without the friendship, as it was before (item 236).

Not covered on this rig: an avatar known by the simulator's list only
(OpenSimulator describes every avatar in the region), and so the list's
heights, its 60-avatar limit and var regions; the `AgentProfile` service,
which OpenSimulator does not have; a friendship accepted while the bot
runs, and friends online when the bot logs in; Second Life.

## Web interface

Run on 2026-11-22 with the debug build on macOS, against the rig set up
anew there (`setup.sh`, the `OpenSim.ini` lines of
[Build and start](#build-and-start), `create user Sit Bot ...`,
`rezprim.py name="Park Bench" desc="Sit here" touch`, `stock.py`,
`friend.py make`), and a second prim rezzed to try hostile text:
`rezprim.py name='<img src=x onerror=alert(1)>' desc='<b>bold</b> & "quoted"' east=9 touch`.
The bot's config is that of item 1 with `socket = true`, the startup script
`persist cfg cfg` and

```toml
[web]
listen = "127.0.0.1:8088"
```

`test-rig/web.py [--at HOST:PORT] [--password P] [--wait SECONDS] ACTION...`
is a client for the web interface without a browser: it sends what the
page sends and prints the answers. `cmd:NAME:ARG:...` is a command,
`own:...` the same marked as the page's own refresh, `line:TEXT` a console
line, `events` and `events:LAST` the event stream, `raw:METHOD:PATH` a
request without the page's marker, `host:NAME` another `Host` header. Its
default address is `127.0.0.1:8080`; the items below ran with
`--at 127.0.0.1:8088`.

The server:

| # | Inject | Expected | Result (2026-11-22, OpenSim feccd9cc53, macOS) |
|---|---|---|---|
| 237 | `web.py session cmd:status` with the avatar seated | `200: open`; the text `rslbot ctl status` prints | Pass: `connected: yes`, `region: Target`, `seat: target`, `seat name: Park Bench`, `mode: holding` |
| 238 | `web.py "line:scan --position 20"`; `ctl scan --position 20`; `ctl scan avatars --position` | Eight fields a line, the position third; the same from both; the last refused | Pass: `130,128,26` for the bench and `133,128,26` for the other prim; `` `scan avatars` has no option `--position` `` |
| 239 | `web.py cmd:scan:avatars own:friends "cmd:inventory:-n"` | Nothing (alone in the region); the friend's line; the 19 folders of My Inventory and nothing below them | Pass |
| 240 | `web.py --wait 3 events "cmd:say:first line" "cmd:say:second line"`; `events:<id of the first>`; `events:5-1` | What was kept, the status, then each line as it is heard; only `second line` and the status; `reset` and everything kept | Pass: ids `<start>-12` and `-13`; the replay after `-12` was `-13` alone |
| 241 | `line:rlv mode on`, `line:rlv cmd @shownearby=n,showinv=n,tploc=n`; `own:scan:avatars:chat`; `line:rlv mode override`; `own:scan:avatars:chat`, `own:friends`; `line:rlv status`; `cmd:scan:avatars:chat`; `line:rlv status`; `cmd:scan:avatars:chat`, `cmd:inventory:-n`, `cmd:tp:128:128:25` | The page's own scan refused naming `shownearby`, before and after the override; `rlv: override` still; the operator's scan answered and `rlv: on`; then the three refused naming `shownearby`, `showinv` and `tploc` | Pass, all of it |
| 242 | `web.py own:say:hello own:scan:20` | Refused: only `scan avatars` and `friends` can be the page's own; nothing said | Pass, and `web: bad command` in the log for each |
| 243 | `web.py raw:POST:/api/command raw:POST:/api/console raw:OPTIONS:/api/command` | 403, 403, 405; nothing said; a log line for each 403 with the address and the path | Pass: `web: refused a request from 127.0.0.1 for "127.0.0.1:8088/api/command": it was not sent by the page of the web interface` |
| 244 | `web.py host:bot.attacker.example:8088 session cmd:say:hello` | 403 with the text about IP addresses and `password`; nothing said; a log line naming the host | Pass, both |
| 245 | Start a second bot with the same `listen` | It exits with status 2 naming `web.listen`, the address and the reason; the first keeps serving | Pass: `` `web.listen`: cannot listen on 127.0.0.1:8088: Address already in use (os error 48) `` |
| 246 | Leave the page open on its Local chat tab through items 250 to 258; count the `scan avatars chat` lines of the log | Only the ones the operator gave: the refresh button and the command of item 241 | Pass: 2 lines in that run of the bot; none for `friends` |
| 247 | `ctl attach "Bot gear/Radio"`; `ctl inventory -a`; `ctl touch <the item's UUID>`; `ctl detach <it>`; `ctl touch <it>` again | The attachment touched at once, the log naming its root prim; after the detach, status 1 saying the object is not known | Pass: `control: touched 56a51ecf-...`, the prim and not the item; then `object eac8f033-... is not known to the bot` |
| 248 | A bot with `password` in `[web]` (not logged in to the grid): a wrong password, the right one, a logout, from the page | `web: wrong password from 127.0.0.1`, `web: login from 127.0.0.1`, `web: logout from 127.0.0.1`; neither password in the log | Pass |
| 249 | Resident memory of the bot seated in `Target`, with `[web]` and a page open, and without `[web]` | About the same | 12,272 KB and 12,192 KB (`ps -o rss`, debug build) |
| 264 | Two event streams, one of which is never read; 1200 `say` commands of 1000 bytes each through the web interface, as fast as they are answered | None refused; the reading stream gets every line; the bot closes the other's stream; the avatar stays connected and seated | Pass: 1200 lines in 0.5 s, all 1200 on the reading stream; 840,314 bytes were waiting on the stream that was not read when the bot closed it; `ctl status` answered at once afterwards |

The page, in the built-in browser of the Claude desktop app (Chromium), at
its desktop width and with the viewport set to 360 by 740 pixels:

| # | Inject | Expected | Result (2026-11-22) |
|---|---|---|---|
| 250 | Open `/`, `/#console`, `/#inventory`; type in the chat box, switch tabs and come back; every tab at 360 pixels | World first; the tab of the address; the text still there; no tab wider than the screen | Pass: `scrollWidth` 360 on all six, also with scan rows listed |
| 251 | The status panel while seated, after "Stand up", after a teleport, after "Sit", and with the bot between logins | `Target`, the position, `Park Bench (the held seat)`, `holding`; each change without loading the page; `not connected` | Pass |
| 252 | Teleport form: `12o` as X; then `128`, `128` | The X box marked and nothing sent; `arrived in Target` | Pass |
| 253 | "Scan objects" within 20; "Touch", "Teleport" and "Sit" on the rows | Two rows, the bench first; `Touched.` from the prim in local chat; `arrived in Target`; `seated on <uuid>` and the seat in the status panel | Pass |
| 254 | The prim named `<img src=x onerror=alert(1)>` in the object scan, in local chat (it speaks when touched) and in the notices | The name and the description letter by letter; no element made of them; nothing loaded | Pass: no `img` and no `b` element under the list. Not seen in an instant message: the rig has no object that sends one |
| 255 | `listen.py 240 at=131,128,25`; "Scan avatars"; "Teleport" and "IM" on the row | `Rig Owner`, `1.8 m`, `at 131,128,26`; `arrived in Target`; the IM tab with the conversation `Rig Owner` open | Pass |
| 256 | Local chat while `talk.py` runs; then `hello everyone` and `/shout come here` from the box; scroll to the top and say another line | `Rig Owner: hello from the rig`, `Rig Owner whispers: psst`, `Rig Owner shouts: HEY`, the line with a tab and a line end as those characters, `Object: Script running`; the avatar's own lines as heard, the box emptied; `Rig Owner 6.7 m` in the chat-range list while it is there; the view stays at the top and "New lines" shows | Pass |
| 257 | IM: the message `talk.py` sends; open it; answer `see you`; "Open" with `rig.owner`; "Open" with `Nobody Here` and send `hi`; `autoreply message ...` in the Console and `talk.py` again | The tab marked and `1` at the conversation, gone when it is opened; `see you` set apart as sent; the same conversation, not a second one; the error naming `Nobody Here` with `hi` still in the box; the autoreply under the message, marked as sent by the bot itself | Pass. The friend list showed `Rig Owner` as `offline`; it was not looked at while `Rig Owner` was online |
| 258 | Inventory: open the tab; open `Bot gear`; "Attach" on `Radio`; "Touch"; "Attached only"; back; "Detach" | 19 entries, folders only; `Bell`, `Dance`, `Radio`, `Watch` with their types; `Radio` then `Left Hand`, `attached`, with "Detach" and "Touch"; `done`; the one entry `Bot gear/Radio`; the folder still open; "Attach" again | Pass |
| 259 | Items 252, 255, 256 and 258 with `rlv cmd @shownearby=n,showinv=n,tploc=n` held | The teleport and the avatar scan refused naming `tploc` and `shownearby`, the avatar list unchanged; the chat-range list empty naming `shownearby`, chat still said; the inventory tab with the error naming `showinv` in place of the tree | Pass |
| 260 | Console: `tp 128 128 25` and at once `status`; `scan 10`; with the bot between logins `sit ground`, `im "Jane Doe hi`, three spaces, `help`, the up arrow | Each answer under its own line; the listing's fields in columns (`white-space: pre`, tabs of 8); `error: ` and the reason; the error about the quote; nothing sent; the command list; `help` in the box | Pass |
| 261 | Settings: light, dark, as the browser asks for; then the browser's own scheme switched to light | The page follows each choice and keeps it over a reload; with the browser's, it turns light without being loaded again | Pass |
| 262 | The bot of item 248: open `/#inventory`; a wrong password; the right one; reload; "Log out" | The login box and nothing else; `The password is wrong.` and an empty box; the tabs; still logged in; the login box again, with nothing of the console's output left | Pass. Which tab was shown right after the login was not read off; after the reload it was the one of the address |
| 263 | Stop the bot with the page open; press "Stand up"; start the bot | The page says that the bot cannot be reached; the error says so too and nothing is sent later; the bar is gone by itself | Pass: found again 3 s after the start |

What the rig has nobody to send was looked at in the same browser with the
page's own files served by a stand-in that answers every command and the
event stream with fixed text (item 265, not on the rig):

| # | Inject | Expected | Result (2026-11-22) |
|---|---|---|---|
| 265 | Events: an instant message stored offline from `Jane Doe` (`Janey`), one from an object named `Collar <i>x</i>` with the text `locked <script>alert(1)</script>`, one with neither UUID, chat of kind `owner` and a `/me`. Answers: three friends, `online`, `offline` and `?`; an inventory with clothing, a body part, a link, an attached object named `Radio\/TV`; an error for `detach`; a `scan avatars` with a hidden avatar and one at `128,64,?` | Three conversations, newest first, none marked as new; the object's and the grid's without a box and saying that they cannot be answered; the stored one marked; the texts letter by letter; `Collar <i>x</i> [owner]: ready` and `Janey (Jane Doe) waves`; the friends in that order with their status, and a click sends `im` with the UUID; "Wear" and "Take off" on clothing and the link, "Wear" on the body part, "Detach" and "Touch" on `Radio/TV`; the error of "Take off" shown; `Someone (hidden)` without an IM button; `tp 103 100 25` and `tp 128 64` | Pass, all of it. The commands the page sent by itself, `friends` and `scan avatars chat`, carried its mark, and no other did |

`listen.py` printed nothing but `listening` in this run, on macOS, though
its avatar was in the region (item 255) and the bot saw it; its `im=`
message did not arrive. `talk.py` worked as described.

Not covered: a phone and browsers other than the one above; a
seventeenth waiting command, which only the unit tests produce; on the
rig itself, what item 265 stands in for; an inventory of thousands of
folders; two windows of one browser at a logout; a reverse proxy in front
of the bot, so the nginx and Caddy examples of the README are untried;
Second Life.

### Second run: Firefox, a phone, nginx and Caddy

Run on 2026-11-23 with the debug build on Linux (Manjaro), against the rig
of [Build and start](#build-and-start) as it was left there (the seat
`Seat` at 130, 128, `friend.py make` done, `stock.py`'s inventory), with
the hostile prim of the first run rezzed again at 133, 128. The bot's
config is the first run's with `sit` set to `Seat` and

```toml
[web]
listen = "192.168.0.70:8088"   # the machine's address on the home network
password = "..."
```

so that a phone on the same network reaches it. Firefox 160 ran headless
and was driven over WebDriver BiDi (`--remote-debugging-port`) from a
Python script, which clicked the page's own controls and read what they
showed; the viewport was 360 by 740 pixels except where noted.

| # | Inject | Expected | Result (2026-11-23, Firefox 160, Linux) |
|---|---|---|---|
| 266 | Open `/` without a session; a wrong password; the right one; `/#inventory` | The login box and nothing else; `The password is wrong.` and an empty box; the tabs; the Inventory tab | Pass, and `web: wrong password from 192.168.0.70`, `web: login from 192.168.0.70` |
| 267 | With the login box shown, change the address to `/#inventory`; log in | Nothing asked of the bot behind the box; after the login the 19 folders | **Failed first**: the tab asked for its folder behind the box, got 401, and after the login showed `the session has ended; log in again` and no tree until "Refresh". The page now asks nothing while the login box is up (`route` in `app.js`); after that change, pass. The same happens in every browser, not only Firefox |
| 268 | World: the status panel; `12o` as X; "Scan objects" within 20; on the rows "Touch" (the hostile prim), "Teleport" (`Garden Seat`), "Sit" (`Seat`); "Stand up"; `listen.py 240 at=131,128,25`, "Scan avatars", "Teleport" and "IM" on its row | `Target`, `Seat (the held seat)`, `holding`; the X box marked and nothing sent; six rows, the hostile name and description letter by letter; `done`, `arrived in Target`, `seated on <uuid>`, `done`; `Rig Owner`, `2.3 m`, `at 131,128,26`; `arrived in Target`; the IM tab with `Rig Owner` open | Pass: no `img`, `b` or `script` element under the list; `listen.py` heard `<img src=x onerror=alert(1)>: Touched.` |
| 269 | Local chat while `talk.py` runs; `hello everyone` and `/shout come here` from the box | The lines of item 256, the one with a tab and a line end as those characters; `Sit Bot: hello everyone`, `Sit Bot shouts: come here`, the box emptied; the IM tab marked | Pass |
| 270 | IM: open `Rig Owner`'s conversation; answer `see you`; "Open" with `rig.owner`; "Open" with `Nobody Here` and send `hi` | `are you there?` and the mark gone; `see you` set apart as sent; still one conversation; the error naming `Nobody Here` with `hi` still in the box; the friend list with `Rig Owner`, `offline` | Pass |
| 271 | Inventory: open `Bot gear`; on `Radio` (attached) "Touch", "Detach", "Attach"; "Attached only"; back | `done`; `detached "Radio"` and "Attach" in place of "Detach"; `is attached to Left Hand` and "Detach" and "Touch" again; the one entry `Bot gear/Radio`; the tree again | Pass |
| 272 | Console: `tp 128 128 25 Home` and at once `status`; `scan 10`; `im "Jane Doe hi`; three spaces; `help`; the up arrow | `status` answered with `region: Target` while the teleport was still under way, each answer under its own line; the listing in columns (`white-space: pre`, `tab-size: 8`, a monospace font); the error about the quote; nothing sent; the command list; `help` in the box | Pass |
| 273 | Settings with Firefox set to light: the browser's, light, dark; reload; Firefox restarted set to dark (`layout.css.prefers-color-scheme.content-override`); light; the browser's | Light, light, dark; dark kept over the reload; light; dark | Pass, and still logged in after the restart of Firefox |
| 274 | `rlv mode on`, `rlv cmd @shownearby=n,showinv=n,tploc=n`, `rlv mode override` from the Console; the Local chat tab for 6 s and the IM tab; `rlv status`; `rlv mode on`; the teleport form, "Scan avatars", the Inventory tab | The chat-range list refused naming `shownearby`; `rlv: override` still; the three refused naming `tploc`, `shownearby` and `showinv` | Pass |
| 275 | Stop the bot with the page open; "Stand up"; start the bot | The bar saying the bot cannot be reached; `the bot cannot be reached; nothing was sent`; the bar gone by itself | Pass: found again 8 s after the start; no `stand` in the bot's log |
| 276 | Every tab at 360 pixels | No tab wider than the screen | Pass: `scrollWidth` 348 on all six, the rest being headless Firefox's scroll bar |

No difference between Firefox and the Chromium of the first run showed,
in behaviour or in layout.

On an Android phone, on the same network, by the operator (its browser
was not noted), with the page open at `http://192.168.0.70:8088/`:

| # | Inject | Expected | Result (2026-11-23, Android phone) |
|---|---|---|---|
| 280 | Log in; World; Local chat while `talk.py` runs; IM: open `Rig Owner`'s conversation and answer `yes` | The tabs; the status panel and the teleport form within the screen; the lines of item 256 as they are said and `Rig Owner` in the chat-range list; the IM tab marked, `are you there?`, `yes` set apart as sent | Pass, from the operator's screenshots: no tab wider than the screen, the line with a tab and a line end shown as those characters; `web: login from 192.168.0.55` and `im [im_out] Rig Owner (Rig Owner): yes` in the log. **Partial**: a wrong password, the scans and their buttons, speaking from the chat box, the Inventory and Console tabs, the theme and losing and finding the bot were not tried on the phone; nothing of them reached the bot's log |

The README's reverse proxy examples, with nginx 1.30 and Caddy 2.11 run
as an ordinary user from a scratch directory, each with a self-signed
certificate for `bot.example.org` and `localhost` (`curl --resolve` sends
the name to 127.0.0.1). The nginx `server` block is the README's with
`listen 8443 ssl`, the two `ssl_` lines and the bot's address in
`proxy_pass`, inside a minimal `nginx.conf` (`pid`, the logs and the
temporary paths in the scratch directory). The Caddyfile is the README's
with the port, a `tls` line naming the certificate, and, in the global
options, `auto_https disable_redirects` and `admin off`, since an
ordinary user cannot take port 80 for the redirect:

| # | Inject | Expected | Result (2026-11-23) |
|---|---|---|---|
| 277 | Through each proxy with `curl`: `/`; `/api/session`; log in; `status`; a `POST` without `X-Rslbot`; the event stream while `say` runs | The page; `login`; a session cookie; the status; 403; the said line on the stream as it is said | Pass, both: the line arrived 0.03 s after the `say` was answered, so nginx's `proxy_buffering off` is enough and Caddy needs nothing |
| 278 | The page in Firefox at `https://localhost:8443/` and `https://localhost:8444/`: log in, say a line from Local chat | The line comes back on the page's stream; no bar | Pass, both |
| 279 | A second bot without `password`, `listen = "127.0.0.1:8090"` and a `login_url` nobody answers at, behind one more site of each proxy; `/api/session` as `bot.example.org` and as `localhost` | Behind nginx, `open` under both names (nginx sends the upstream's address as `Host`); behind Caddy, `open` for `localhost` and the refusal naming IP addresses and `password` for `bot.example.org`, as the README says | Pass, and `web: refused a request from 127.0.0.1 for "bot.example.org:8446"` |

Behind a proxy the bot's log names the proxy's address, not the
browser's: the logins of items 277 and 278 were all logged as from
`192.168.0.70`, the machine the proxies ran on.

The checks of the change on the same machine:

| # | Inject | Expected | Result (2026-11-23, Linux) |
|---|---|---|---|
| 281 | `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`, `openspec validate add-web-interface --strict` | All clean | Pass: 1117 tests passed, 1 ignored; the two control-socket tests that fail on macOS pass here |
| 282 | Resident memory (`ps -o rss`, debug build) of the bot seated in `Target` 185 s after its start, with `[web]` and two pages open (Firefox and the phone of item 280), and without `[web]` | About the same | 12,276 KB and 11,580 KB |

## Payments

Run on 2026-11-24 with the debug build on Linux, against the rig with the
money module of [Payments](#payments) (OpenSim 4b95147c48). In `Target`:
`Hat Vendor` of `Rig Owner` (`payee.py --name "Hat Vendor" --price hide
--east 4 --north 0 100 250`), `Tip Jar` of `Rig Alt` (`payee.py --name "Tip
Jar" --as alt --east 8 --north 0`), the `Payee box` of the section above,
and a plain prim (`rezprim.py name="Bench" east=2`). The bot's config, with
`RUSSELBOT_SOCK` set to a scratch path:

```toml
login_url = "http://127.0.0.1:9000/"
user = "Sit Bot"
password = "secret"
region = "Target"
position = [126.0, 126.0, 25.0]
socket = true

[web]
listen = "127.0.0.1:8088"

[logs]
transactions = "tx.tsv"

[payments]
limit = 1000
limit_renew = "3m"
perms = ["ctl", "web"]
```

Later items change `[payments]` as they say. `rigmoney` commands were
typed on the simulator's console.

| # | Inject | Expected | Result (2026-11-24, OpenSim 4b95147c48, Linux) |
|---|---|---|---|
| 283 | Start the bot | The start line naming `ctl` and `web`, L$1000 every 3m, L$1000 left; the warning naming `web.password`; with `--verbose`, one `MoneyBalanceRequest` after the login, and none with a config without `[payments]` | Pass: `payments: `ctl` and `web` may pay, with a limit of L$1000 every 3m, L$1000 left`; one request and its reply; none without the table |
| 284 | `ctl status` | `avatar: Sit Bot` second; `balance` as `rigmoney show` has it; `limit: L$1000` | Pass: `balance: L$1100`, the module's L$1100 |
| 285 | `ctl pay 25 avatar rig.alt`; `ctl pay 25 avatar <Rig Alt's UUID>`; `ctl pay 1 avatar Rig Owner` | Each paid, the answer naming the avatar and the new balance | Pass: `paid L$25 to Rig Alt <7e57...a1>; the balance is L$1075`, then L$1050; the name in two words read as one |
| 286 | `ctl pay 10 object <Tip Jar>` | Paid; the jar says it was paid by `Sit Bot`; the answer names the jar and its owner | Pass: `paid L$10 to Tip Jar <b459...> of Rig Alt <7e57...a1>`; `Tip Jar: paid L$10 by Sit Bot (7e57...b0)` in the log |
| 287 | `ctl pay 120 object <Hat Vendor>`; `ctl pay 250 object <Hat Vendor>` | Refused naming L$100 and L$250; then paid, the vendor saying so | Pass: `... does not take L$120: it offers L$100, L$250 and no other amount`; `paid L$250 to Hat Vendor ... of Rig Owner ...` |
| 288 | `ctl prices` of the vendor and of the jar | The four lines | Pass: `object: Hat Vendor`, `owner: Rig Owner`, `prices: 100, 250`, `other amounts: no`; the jar `prices: 1, 5, 10, 20`, `other amounts: yes` |
| 289 | `ctl scan --pay` | The three objects that take money, flags `scripted,pay`; not the Bench | Pass |
| 290 | `ctl pay 10 object <Bench>`, `ctl prices <Bench>`; an object that does not exist; `pay 10 avatar "Sit Bot"`; `pay 10 avatar "Nobody Here"`; `pay 10 avatar <Tip Jar>` | Refused: no money; not known, with the scan distance; cannot pay itself; no such avatar; no avatar by that UUID, pointing at `pay AMOUNT object UUID` | Pass, each, with nothing paid |
| 291 | `ctl pay 2000 avatar rig.alt` with L$790; `pay 0 ...`, `pay 10.5 ...`, `pay 10 rig.alt`, `pay 10 object "Tip Jar"` | Refused naming L$790; the amount; the two forms; the UUID | Pass |
| 292 | `ctl pay 700 avatar rig.alt` with L$690 left; the same after the renewal time | Refused naming L$690 and the renewal; then paid, and a new period | Pass: refused at 00:23:33 naming 00:25:01; paid at 00:25:03; `limit: L$300 (renews to L$1000 on 2026-11-24T00:28:03Z)` |
| 293 | `ctl quit` and start again before the renewal | The start line and `status` with what was left | Pass: `L$690 (renews to L$1000 on 2026-11-24T00:25:01Z) left` |
| 294 | `rigmoney silent on`; `ctl pay 15 avatar rig.alt` | After 15 s an error saying it was sent and counts as paid; an `unconfirmed` row; L$15 less left | Pass: exit after 15.0 s; `limit: L$675`; the row with an empty balance and counted 15 |
| 295 | `pay.py 50` (Rig Owner pays the bot) | A `received` row with the new balance; `status` shows it without asking | Pass: `received 50 825 0`, `Rig Owner paid you L$50.` |
| 296 | `rigmoney set Sit Bot 5000`; restart with `limit = 100`, `perms = ["web"]`, `whitelist = ["Rig Owner", "<Hat Vendor>", "<Rig Alt>"]`, `blacklist = ["rig.alt"]`, `unlimited_list = ["rig.owner"]` | The start line with L$0 left (L$1025 counted without a renewal); `ctl status` without money lines and `ctl pay` refused naming `ctl`; through `web.py`: `status` with them, `payees`, Rig Alt and its jar refused for the blacklist, L$500 to Rig Owner and L$250 to the vendor paid as unlimited | Pass, each; `payees` printed `?`, `Rig Owner`, `whitelist,unlimited` and Rig Alt's UUID line |
| 297 | `web.py --wait 3 events "cmd:pay:7:avatar:Rig Owner"` | The rows of the file as `money` events, then the payment's, then a status with the new balance | Pass: 12 kept, then `money [..-12] ... paid | 7 | 4238 | 0 ...` and `balance: L$4238` |
| 298 | Prosody running; `perms = ["xmpp"]` and an `[xmpp]` table; `xmpp.py owner "msg:bot.rig.test:status" "msg:bot.rig.test:pay 3 avatar \"Rig Owner\""`; `ctl pay 3 avatar rig.owner` | The status with its money lines; paid over XMPP; refused over `ctl` | Pass |
| 299 | A fresh transactions file; `ctl pay 5 ...`, at once `chmod 444` on the file, `ctl pay 6 ...`, `ctl pay 7 ...`, `chmod 644`, `ctl pay 8 ...` | The first row cannot be written: the next payments refused naming the file; then the waiting row written first and the payment made | Pass: the L$5 row was still waiting for its names when the file became read-only, so L$6 and L$7 were refused; `wrote 1 row(s) that waited`, L$8 paid; the file has the L$5 and L$8 rows |
| 300 | `rigmoney silent on`; `ctl pay 9 ...`; two seconds later `ctl quit` | The `pay` answered as not confirmed, the bot stopped; an `unconfirmed` row in the file | Pass: `... the bot stopped before the grid confirmed it; it counts as paid ...` and the row |

The page, in the built-in browser of the Claude desktop app, with the
first config and `whitelist = ["Rig Owner", "<Rig Alt's UUID>"]`, and
`friend.py make`:

| # | Inject | Expected | Result (2026-11-24) |
|---|---|---|---|
| 301 | Open `/#money` | The Money tab between Inventory and Console; the header with the avatar, the balance and what is left; the friend; `Rig Owner` (by name) and `Rig Alt` under Whitelist; the transactions of the file, newest first, the unconfirmed one saying so | Pass |
| 302 | Choose `Rig Alt`; type `5`, press Enter; "Pay L$5" | The dialog with `Pay L$5 to Rig Alt?` and the UUID, Cancel focused; paid; the amount box emptied; header and list updated without a reload; the renewal in local time | Pass: `paid L$5 to Rig Alt ...; the balance is L$4199`, `Left of the limit L$995`, `(renews to L$1000 on 24 листопада 2026 р. о 02:33)` |
| 303 | Type `Rig Owner` and `2000`; "Pay"; "Pay L$2000" | `As typed.` in the dialog; the refusal for the limit shown; both boxes kept | Pass. A click beside the dialog before that closed it and sent nothing |
| 304 | World: "Scan objects"; "Pay" on the vendor, `L$250`, "Pay the object", confirm; "Pay" on the jar | "Pay" on the three objects that take money only; `L$100`, `L$250` and no other amount; the dialog naming the object and its owner; the answer in the row; the jar's four amounts and "Another amount" with its box | Pass |
| 305 | `talk.py`; IM: open `Rig Owner`; "Pay" | The Money tab with `Rig Owner` as payee by UUID, the amount empty with the cursor in it | Pass |
| 306 | 360 by 740 pixels: World, Money, IM | No tab wider than the screen | Pass: `scrollWidth` 360 each |
| 307 | Restart with `perms = ["ctl"]`; open `/#money`; the console line `pay 5 avatar "Rig Owner"` | Six tabs, World shown, no "Pay" anywhere; the console's refusal naming `web` | Pass |

The rig's OpenSim names an avatar by UUID only after the region has looked
it up by name: after a restart of the bot, `payees` left `Rig Alt`'s UUID
entry out and `pay 1 avatar <Rig Alt's UUID>` was refused as no avatar,
until `profile rig.alt` had found it by name. Second Life's name service
knows every account; the bot reports what the service says.

On Second Life, by the operator, on 2026-11-24 with `--verbose`, in
Porten Hill: an account of the operator's with L$2, `[payments]` with
`limit = 1000`, `limit_renew = "1h"` and `perms = ["ctl", "web"]`, and the
three files under `[logs]`. The tip jar is a scripted object of the other
avatar's that says thanks in an IM when paid.

| # | Inject | Expected | Result (2026-11-24, Second Life) |
|---|---|---|---|
| 308 | Start the bot | The start line; one `MoneyBalanceRequest` after the login, answered | Pass |
| 309 | `ctl pay 1 avatar <the other avatar>` | Paid, confirmed by the grid; a `paid` row | Pass: confirmed in the second it was sent; `paid 1 1 1 5001`, text `You paid AnnaNovykova99 Resident L$1.` |
| 310 | `ctl pay 1 object <the tip jar>` | `RequestPayPrice` answered; paid, confirmed; the jar's script told; a `paid` row with the jar and its owner | Pass: confirmed in the second it was sent; `paid 1 0 1 5008` with the jar's name and UUID and its owner as the party; the jar's IM `Thanks for the L$1 tip!`. The grid's text names the owner, not the object: `You paid AnnaNovykova99 Resident L$1.` |
| 311 | The other avatar pays the bot L$20 | A `received` row with the new balance | Pass: `received 20 20 0 5001`, `AnnaNovykova99 Resident paid you L$20.` |

The verbose log names messages and their sizes only, so the destination
Second Life put into `TransactionInfo` for the payment to the jar was not
read off; the grid's text points at the owner, as on the rig, and the bot
does not compare it. A refused payment was not tried: the bot does not
send one larger than the balance it knows, so the grid never sees an
overdraft, and a refusal for another reason would end the payment as
`unconfirmed` after 15 s. Confirmation works as on the rig, and the
README's wording stands.

The description of a payment, on the rig, with the config of items 301 to
306 and `test-rig/pay.py --as alt --balance --wait 60` running alongside
as `Rig Alt`, which prints what the module tells the payee:

| # | Inject | Expected | Result (2026-11-24, OpenSim 4b95147c48, Linux) |
|---|---|---|---|
| 312 | The page: payee `rig.alt`, `3`, description `for the hat`; "Pay"; "Pay L$3" | `Description: for the hat` in the dialog; paid; the amount and description boxes emptied; the log line naming the description; `Rig Alt` told of it | Pass: `web: pay L$3 to Rig Alt <...>: sent, with the description "for the hat"`; `pay.py` printed `item='for the hat'` in the payee's `TransactionInfo` |
| 313 | `ctl rlv mode on`, `ctl rlv cmd @sendim=n`; `ctl status`; the page; `ctl pay 2 avatar rig.alt -d blocked text` | `pay description: blocked by RLV` last in `status`; the page shows `Payment description is blocked by RLV` in place of the box, without a reload; paid without the description, the answer beginning with the warning | Pass: `warning: Payment description is blocked by RLV` then `paid L$2 to Rig Alt ...`; the log line `sent, without its description, which RLV holds back`; `pay.py` printed `item='Payment'`, the module's word for none |
| 314 | `ctl rlv reset`, `ctl rlv mode off` | The line gone from `status`; the description box back on the page | Pass |
| 315 | After the record's text became the viewer's (task 11.7): `ctl pay 2 avatar rig.alt -d rig check` | The row's text `You paid Rig Alt L$2: rig check` | Pass |

Until task 11.7 the bot's own `paid` rows kept the grid's text alone
(`You paid Rig Alt L$3.`); since then the text is the viewer's, with the
description, as item 315 shows. Second Life: item 316.

On Second Life, by the operator, the same day, as items 308 to 311:

| # | Inject | Expected | Result (2026-11-24, Second Life) |
|---|---|---|---|
| 316 | `ctl pay 3 avatar <the other avatar> -d test payment` | Paid with the description; the log line naming it; the other avatar sees it with the payment | Pass: `sent, with the description "test payment"`, confirmed in a second; the payer's viewer showed `You paid AnnaNovykova99 Resident L$3: test payment.` The grid's text in the reply was `You paid AnnaNovykova99 Resident L$3.`, without the description, so the row of that run (before task 11.7) had only that. The verbose log names messages and sizes only, so whether the reply's `ItemDescription` carried `test payment` was not read off; the bot now composes the text from it, or from what it sent when it does not |

## Groups

On the rig with groups (see [Groups](#groups)), the bot `Sit Bot` with
`[logs]` including `group`, `[payments]` with `limit = 1000` and
`perms = ["ctl", "web"]`, and `[web]`. `Rig Owner` created `Builders` (open,
free) with `group.py`.

| # | Inject | Expected | Result (2026-11-24, rig) |
|---|---|---|---|
| 317 | Start the bot | The list after login in the log; `ctl group list` | Pass: `groups: the grid lists 0 groups of the avatar`; empty answer |
| 318 | `group.py invite Builders` | The invitation waits, with the group's name and the inviter's UUID; `group invitations`, `perms status` | Pass: `invitation from Rig Owner <...aa> into "Builders" <...>: waits; ...accept group <uuid>...`; the line `<uuid> 0 <...aa> Rig Owner Builders`; `group: none` and the `waiting:` line |
| 319 | `ctl accept group builders`; `group list` | Joined; the list | Pass: `joined "Builders" <...>, invited by Rig Owner <...aa>`; Builders `active,notices` (OpenSim made it active) |
| 320 | `ctl group activate Builders`; `ctl group info Builders` | Confirmed with the title; the profile lines | Pass: `... is the active group now, with the title "Member of Builders"`; `founder: Rig Owner`, `fee: none`, `enrollment: open`, `member: yes`, the charter (`members: 1`: the profile kept from the invitation) |
| 321 | `group.py listen`; `ctl group chat Builders hello from the bot`, then `second line` | The bot enters the session; the listener gets both lines; two `chat_out` rows | Pass: the first line reached the listener as `ChatterBoxInvitation`, the second over UDP; `chat_out` rows with `Sit Bot` |
| 322 | `group.py as=alt join` Builders; `as=alt say Builders anyone around?`, then `say hello` | Two `chat` rows of Rig Alt, the first line an event; nothing said back | Pass: both recorded; nothing said |
| 323 | `group.py notice Builders Meeting "Tonight at eight"`; then with `item` | Two `notice` rows, the second naming `Agenda`; nothing accepted | Pass: `group [notice] Builders: Rig Owner, subject "Meeting": Tonight at eight`, then `... attachment "Agenda": See the notecard`; not marked stored |
| 324 | `group.py create Dancers fee=250 open`; `as=alt join` it | `MoneyBalanceReply` type 1004, L$250, balance lower by the fee | Pass: `balance=L$750 description='You paid L$250 to join the group Dancers.'`, `type=1004 source=Rig Alt amount=L$250` |
| 325 | `ctl group join Dancers`; `ctl group join <uuid>`; `--pay 100`; `--pay 300` | By name refused; the fee and `--pay` named; too little refused; the last joins and pays | Pass: `a group is joined by its UUID`; `costs L$250; give --pay 250`; `more than the L$100`; `joined "Dancers" <...>; the fee of L$250 counts as paid, as the grid has not reported it yet`, the report arriving in the same second: a `paid` row `250 750 250 1004 <Dancers>`, `limit: L$750` |
| 326 | `ctl group join <Fishers, L$900> --pay 900`; `<Closed>`; `<Dancers>` | The balance refuses; enrollment refuses; already a member | Pass: `the avatar has L$750, less than L$900`; `takes an invitation`; `already` |
| 327 | `ctl perms autoaccept group "Rig Owner"`; `group.py invite Free` | Accepted by itself, confirmed by the list | Pass: `bot: accepted by itself: invitation from Rig Owner ... into "Free" <...>: confirmed` |
| 328 | `ctl perms revoke group all`; `group.py invite Closed`; `ctl decline group closed` | Waits; declined; none waits | Pass |
| 329 | `group.py eject Builders` | The list without Builders; the log line | Pass: `groups: the avatar is no longer in "Builders" <...>` |
| 330 | `ctl group invite Free "Rig Alt"`; `ctl group chat Nonexistent hi` | Refused for the right to invite; refused naming the group | Pass |
| 331 | `ctl quit`, start again; `group list` | The groups from the grid | Pass: Dancers `active,notices`, Free `notices` |
| 332 | The page at `/#groups` with an invitation waiting, a line `hello <b>bold</b> & co` and a two-line notice in Free | The tab between IM and Inventory, marked; the invitation row; the list with the active one and Builders set apart; Free's chat with the markup as text and the notice set apart with its lines | Pass |
| 333 | Type `hi from the page`, Enter; "Info"; "Reject" on the row; 360 px wide; "Activate" on Dancers | The line as the avatar's own; the info with the title; the row gone and the mark with it; no sideways scroll; Dancers marked active | Pass |

Not on the rig: an invitation with a fee, and the fee rows of the page
(tried against a stand-in server with the page); a join the grid refuses;
the limit refusing a fee (the bot's balance is the one the grid last
reported); the bot inviting someone, as no rig group gives it the right.

### Leaving groups

The same bot and rig, with a simulator started for this run; `Rig Owner`
created `Builders` and `Dancers` (open, free) and `Leavers` (open, free).

| # | Inject | Expected | Result (2026-11-27, rig) |
|---|---|---|---|
| 334 | `group.py as=alt join <Leavers>`; `group.py as=alt dump leave Leavers` | `LeaveGroupReply` with success, then the drop, no active group and an empty list | Pass: in that order (OpenSim had made `Leavers` active on the join); the reply is in `test-rig/captures/groups.txt` |
| 335 | `ctl group join` Builders and Dancers; the page at `/#groups`, Dancers active: "Leave", then Escape; "Leave", then "Cancel"; "Leave" and the dialog's "Leave" | The dialog names the group and its UUID, says that joining again may take an invitation or a fee, and has "Cancel" focused; Escape and "Cancel" send nothing; then the notice of the answer, Dancers set apart as not a member, without "Leave" | Pass: no `group leave` line in the log until the dialog's "Leave", then `control: group leave "Dancers" <...>: sent` and `confirmed` in the same second. The first build showed the group by its UUID once it was left, as the page knew its name only from messages; it now keeps the name from the list |
| 336 | `group.py invite Dancers`; `ctl accept group Dancers` | Joined again, by invitation | Pass: `joined "Dancers" <...>, invited by Rig Owner <...aa>` |
| 337 | The page 360 px wide in the light theme: "Leave" on Builders and the dialog's "Leave"; then the Money tab's form, L$5 to `Rig Alt`, and Escape | The dialog fits without sideways scroll; Builders kept by its name as not a member; the payment dialog with its own title and "Pay L$5", "Cancel" focused, and nothing paid | Pass: page width 360; `Confirm the payment`, `Pay L$5 to Rig Alt?`; no `pay` line in the log |
| 338 | `ctl group join <Builders>` (OpenSim makes it active); `group chat Builders before leaving`; `group leave Builders`; at once `group list` | `left "Builders" <...>`; the list without Builders and with no `active` | Pass. With the first build the list right after the answer still had Builders `active`: OpenSim sends `LeaveGroupReply` before the drop, and the bot now takes a successful reply as the drop, as a viewer does. `listen.py` does not show group tags; the grid's update with no active group (item 334) stands for the tag being gone |
| 339 | `group.py say Builders said after the leave`; `ctl group join <Builders>`; `ctl group chat Builders back again` | No row for the line in `group.tsv`; joined; the session entered anew | Pass: no row; `entering the chat session of "Builders"` a second time, then `in the chat session` |
| 340 | `ctl group leave Build`; `ctl group leave <a UUID of no group>`; `ctl group leave` | Status 1 each | Pass: `no group named "Build"`; `not in the group <uuid>`; `` `group leave` takes a group `` |
| 341 | `ctl rlv mode on`; `ctl rlv cmd @setgroup=n`; `ctl group activate none`; `ctl group leave Dancers` | The activation refused naming `setgroup`; the leave carried out | Pass |
| 342 | `ctl group join <Dancers> && ctl group activate Dancers`, a few seconds after the last group command | The activation finds the group | Pass. Before, `group join` was answered on the `JoinGroupReply`, which OpenSim sends before the new list, and an activation straight after a join (in a second run of item 338) was refused with `no group named "Builders"`; the join is now answered by the list |
| 343 | Three times in a row, with no pause: `ctl group join <Dancers>`; `group activate Dancers`; `group leave Dancers` | Each command finds the group | Fail at first, in the second round: the join was confirmed, then `groups: the avatar is no longer in "Dancers"`, `... is in ... now` and `... no longer in` again within the same second, and the activation and the leave were refused. A leave and a join given within milliseconds of each other make OpenSim send lists and drops that arrive out of order. The bot now keeps to a confirmed join or leave for 5 seconds and then asks for the list again: five rounds in a row with `--verbose` passed, with four late lists and drops held back (`... which the grid confirmed otherwise a moment ago; asking for the list again`), `AgentDataUpdateRequest` sent 5 s after the last leave, and `group list` then with Builders alone and nothing active, as on the grid |
