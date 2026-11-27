RusselBot is a lightweight bot software for Second Life and OpenSimulator written in Rust,
replacement for heavy .NET/Mono bots. Requires just about a dozen MBs of memory which makes it possible
to run even on cheapest VPS machines without requiring to run heavy Mono runtime.

Left alone, the bot follows one simple scenario:
* Connect to the grid, connect to the region
* If current region is not the target region, issue teleport command to the target destination
* Issue "sit" command on a target object.
* Keep the avatar alive, respond all necessary server queries to keep it online.

Through its [control socket](#control-socket) it can also be told to talk,
stand, sit elsewhere, teleport, exchange teleport offers and touch objects,
and to use what the avatar owns: list its inventory, attach and detach
objects, play animations and sounds, and (on Second Life) change clothing.
With [RLV](#rlv) switched on, scripted objects the avatar owns can ask for
some of the same: sit, stand, teleport, take things off. They can also
hold the avatar to restrictions, which then bind the bot and your commands
alike. And it can answer
what a viewer answers with a dialog: teleport offers, inventory offers and
scripts' requests for permissions are [accepted by
itself](#permission-requests) from the avatars and objects the config
names, and wait for a command otherwise. As an [XMPP gateway](#xmpp-gateway)
it puts the avatar's instant messages, local chat and commands into an
ordinary Jabber client, and as a [web interface](#web-interface) it serves
a page of its own, with the world around the avatar, chat, instant messages
and the inventory, to any browser.

## Status

Verified by unit tests and against a local OpenSim, including kicks, region
restarts, simulator outages and being unseated or teleported away; see
`docs/test-rig.md` for the rig and the results. Chat and IM logging is
verified there too, except IMs from scripts and delivery of offline IMs,
which the rig cannot produce. The control socket and its commands are verified there as well,
and `ctl im` by avatar name was checked by hand on Second Life on 2026-11-19.
The `scan` command and the `seat name` line of `status` are verified there
too (2026-11-21); nothing of them has been tried on Second Life, where the
distance decides what the simulator describes at all.
`scan avatars`, `friends` and `profile`, and the RLV restrictions on the
two scans, are verified there too (2026-11-22), with profiles read over the
simulator connection. The three commands were tried once on Second Life
too (2026-11-22, a busy region, the avatar on a platform at 610 m):
`scan avatars` listed 20 avatars with their names, those nearby at the
positions the simulator described and those farther away at the list's
whole metres and steps of 4 m, and eleven on platforms above 1020 m with
`?` as distance and height; `friends` listed the two friends with their
names; `profile` printed a stranger's profile with `online: unknown`. That
profile came through the profile service (`AgentProfile`) as far as can be
told: its about text was 516 bytes, more than the simulator connection
carries, and the run was not logged verbosely. The RLV restrictions on the
scans have not been tried on Second Life.
The movement, teleport-offer and touch commands have not been tried on
Second Life yet, and neither has `persist` with the position tracking under
it: whether Second Life answers the bot's question about a seat, and the
seated position that results, are unverified there. Nothing has been tried
on Windows. On Second Life, checked by hand on 2026-11-18:
login, sitting, logging of chat and IMs from avatars and from objects, and
fetching offline IMs (delivered with the time they were sent). Multi-factor login is not supported: an account that has it enabled
makes the bot exit with an error.

[RLV](#rlv) is verified by unit tests and on the local OpenSim
(`docs/test-rig.md`, items 108 to 124, 2026-11-19), with a script of the
bot's own that says to it what it is told: the version queries, the force
commands, `drop` and `follow`, and the `[persist]` table. On Second Life,
checked by hand on 2026-11-19 in three logins of two to fifteen minutes,
with `rlv = true`, wearing a collar and a relay, near objects named
`corestat` that seat an avatar and put handcuffs on it, and with `rlv cmd`:

- The version queries, at every login. The collar asked `@versionnew` ten
  seconds after the region was entered, the bot answered, and the collar
  reported "RLV ready!" four seconds later. Each `corestat` asked
  `@versionnew` and went on to force a sit. The handcuffs asked the old
  `@version`, got the `RestrainedLife viewer v0.1.0 (...)` text, and set
  their restrictions in the same second, so that text was taken as well;
  the specification does not quote it.
- `@sit:UUID=force` with the setting `drop`, five times from such objects:
  carried out as `sit`, and four of them confirmed by the simulator within
  a second, on another prim than the one named, as happens in a linkset.
  The fifth came in the same second as another from a second object, which
  took its place. The first of all switched a holding bot to manual mode.
- `rlv cmd @sitground=force` and `rlv cmd @unsit=force`: handed to the
  simulator. The log does not show what the avatar then did.
- `rlv cmd @tpto:Polasido/219/112/612=force`, inside the region the avatar
  was in: arrived within the second. A teleport to another region was not
  tried.
- `rlv cmd @tpto:219/102/611=force`, three numbers that were coordinates
  inside the region and not global ones: the bot asked for a region at
  0, 0, the grid did not answer, and the teleport ended after 60 seconds,
  during which the bot refused other commands as it does during any
  teleport. Such coordinates are refused since, before anything is sent.
  The global form with global coordinates was not tried.
- The setting `follow`, with a forced sit: `persist cur UUID` at once, the
  avatar seated within the second on another prim than the one named, and
  `persist cur cur` in the same second with the seat's position, 16 metres
  from where the avatar had stood. So Second Life answers the bot's
  question about a seat fast enough. The object then unseated the avatar
  three times within a minute, by itself and not with `@unsit=force`, and
  the bot sat it down again after 5, 10 and 20 seconds, as it does on any
  held seat: an object that lets the avatar go that way does not get rid
  of a following bot. `rlv persist drop` given meanwhile changed nothing,
  as it applies to later movements only; `rslbot ctl stand` would have.
- `rlv cmd @remattach:skull=force`: the two attachments on the skull were
  detached within a second. `@remattach` with a UUID was answered as not
  supported; that form was added afterwards and has unit tests only.
- `rlv cmd @remoutfit:shoes=force`: the shoes the avatar wore were taken
  off in two seconds. With an item's UUID in place of the kind it was
  refused.
- What is not supported yet. Over the three logins the objects sent 1,337
  commands the bot noted as not supported, nearly all of them restrictions
  and their exceptions; the handcuffs alone sent 71 messages of three
  exceptions each within two seconds. The `corestat` objects asked
  `@getstatus` eight times, after `@unsit=n`, and got no answer. A bare
  `@clear` was noted as
  malformed in these runs, wrongly; it is read as a command since.

RLV restrictions are verified by unit tests and on the local OpenSim
(`docs/test-rig.md`, items 142 to 157, 2026-11-20): restrictions held by a
rezzed and by a worn object and by `rlv cmd`, commands refused for them,
`@getstatus`, a locked attachment, the three modes, `rlv reset`, the bot's
own teleport and sit held back and released, and the persist override
setting from the config and by command. Nothing about restrictions has
been tried on Second Life: the collar, the handcuffs and the `corestat`
objects of the runs below now hold the avatar to what they send.

Not tried there: `@detach` and `@detachme` (on the local OpenSim only,
`docs/test-rig.md`, item 116), `@tpto` to another region and by global
coordinates, the setting `follow` with a forced stand, ground sit or
teleport, and commands that arrive while the avatar logs in: the first
ones came two to four seconds after the region was entered, and none had
to wait.

The inventory commands (`inventory`, `attach`, `wear`, `detach`, `anim`,
`sound`) are verified by unit tests and on the local OpenSim, where the
clothing commands can only be refused. On Second Life, checked by hand on
2026-11-19 in one region, in two runs, the second with `--verbose`:

- `attach` of three objects, by UUID and by path, without a body part: each
  answered within a second with the point the simulator chose. All three
  went to `Right Hand` and stayed there together.
- `detach` of two of them by item: each answered within a second.
- The Current Outfit folder: a link was added after each `attach` and
  removed after each `detach`, and the grid took both. An item that already
  had a link there got no second one.
- `wear` and `unwear` of a piece of clothing (shoes), three times: the grid
  took the link and the request for the new appearance and answered in two
  seconds. The log does not show what the avatar then looked like.
- What is attached: at login the bot learned of the avatar's attachments
  from the simulator, five or six of them with their points. But one that
  it had reported at one login, and that the Current Outfit folder still
  listed, it did not report at the next login seven minutes later, and an
  `attach` of that item was then carried out and confirmed, not refused. So
  on Second Life an attachment can be missing after a login, and
  `inventory -a`, `detach BODY PART` and the refusal to attach a thing twice
  are only as good as what the simulator shows.
- Six more logins in the same region later that day, all with `--verbose`,
  to find out why. They were made with a build that asked the simulator at
  once about objects it only mentions, as ones a viewer has cached, and
  marks as the account's own; the miss was first taken for such a mention.
  At each login the simulator mentioned more than 2,100 objects that way
  and marked none of them as the account's own, so that was not the cause,
  and the bot does not ask so any more.
- In the first two of them, a minute and a half apart, the bot reported
  the same five attachments within four seconds each time, all described
  by the simulator without being asked. The object missed in the first
  check had been taken off in between, so its absence was right; beyond
  that the five were not compared with what a viewer shows. `inventory`
  listed 338 entries in three seconds and `inventory -a` the five
  attachments; `attach` of one of them was refused as attached already;
  `detach` by body part answered within a second, and the link was removed
  from the Current Outfit folder.
- The other four show the gap again and what goes before it. An object was
  attached to `Right Hand` and was reported at the next two logins. Then
  another object was attached to the same point and detached again, and at
  the login after that the first one was not reported. The miss in the
  first check had come about the same way. Most likely the simulator
  itself no longer has the object on the avatar after such a login: in the
  first check it attached that object again when asked. Why it loses it is
  not known. A viewer attaches at every login whatever the Current Outfit
  folder lists and the simulator did not bring, which would hide this. The
  bot now does the same (`restore_attachments`, see
  [Attachments](#attachments)).
- Two logins with that, the same evening, `--verbose`. At the first the
  object lost before was still missing; 20 seconds after the region was
  entered the bot read the Current Outfit folder, asked for it, and the
  simulator showed it on `Right Hand` a second later. An `attach` of it
  was then refused as attached already. Meanwhile another object had been
  attached to the same point by command, and it was detached before the
  logout. At the second login the simulator brought the restored object by
  itself, so this time the same sequence lost nothing; whether that was the
  order of the attaches or chance, two logins do not tell.
- One thing in those two logins is not understood. The folder also lists an
  object that the simulator showed at no login that day. The bot asked for
  it both times, the simulator showed it on `Chin` within the second, where
  another object already was, and at the next login it was missing again.
  The avatar looked right afterwards, with nothing doubled. Whether it
  then wears two things there or one under two inventory items the log
  does not tell, and `inventory -a` was not run to see. So the bot sends
  one request at every login that changes nothing to the eye; a viewer
  would ask for the same object at every login.
- Also seen there: at the first of the two logins the simulator took 12
  seconds to show the avatar's attachments, not four as before, which
  leaves 8 of the 20 seconds the bot waits.
- In those four also: the simulator took 21 and 15 seconds to carry out an
  `attach`. The bot waits 15, so the first was answered as refused; the
  object came six seconds later and was reported, and got no link in the
  Current Outfit folder. The bot now waits 30 seconds and gives an object
  that comes later still its link; unit tests only so far.

Not tried there, or not recorded: the listing of `inventory` against what a
viewer shows, `attach` with a body part named, `unwear` by kind of
clothing, `wear` of a body part, a grid that refuses the new outfit,
`anim`, `sound`, the points the bot reported compared with a viewer, a
viewer login afterwards to see whether it finds the avatar as the bot left
it, whether an object missing after a login is on the avatar for other
people, a login with `restore_attachments = false`, a login at which the
folder lists nothing that is missing, and an `attach` the simulator takes
longer than 15 seconds over, now that the bot waits 30.

[Permission requests](#permission-requests) are verified by unit tests and
on the local OpenSim (`docs/test-rig.md`, items 125 to 141, 2026-11-20),
with scripted boxes of another avatar that ask the bot for script
permissions and give it inventory: the rules from the config and from the
`perms` commands, exceptions, a request for two permissions at once, a
request for money that is never granted, inventory offers from an object
and from an avatar accepted by rule and by hand, teleport offers accepted
by rule, with `accept tp` and with `accepttp`, and what a lost session and
a change of region do to the requests that wait. Not produced on the rig
and covered by unit tests only: an object owned by a group, a simulator
that does not say who owns an object, a grid that does not know an owner's
name, a simulator without an inventory service, and more than 100 requests
waiting. Nothing of this has been tried on Second Life: that it takes the
bot's answers to an inventory offer and to a script's permission request
as the bot builds them is known from the viewer's source only.

The [web interface](#web-interface) is verified by unit tests, among them
the server over real connections and the bot with its pages' event
streams, and on the local OpenSim (`docs/test-rig.md`, items 237 to 265,
2026-11-22) with `test-rig/web.py` and in one browser, at desktop width
and at 360 pixels: the six tabs, teleports, both scans with their buttons,
local chat and instant messages as they happen, the inventory with attach,
touch and detach, the console, the theme, the login box, the page losing
and finding the bot, the RLV refusals `tploc`, `shownearby` and `showinv`
on the page, and an override left in place by the page's own refreshes.
`scan --position` and `touch` with the UUID of an attached item were tried
there too. On 2026-11-23 (items 266 to 282) the same went through Firefox
on Linux without a difference; on an Android phone, the login, the World
tab, local chat and an instant message answered; and the nginx and Caddy
examples below, over TLS with the page and its live updates. Not tried:
the rest of the page on a phone, Safari, and anything of it on Second
Life.

The [XMPP gateway](#xmpp-gateway) is verified by unit tests, against a fake
component server in the tests, and on the local OpenSim with a local
Prosody 13 (`docs/test-rig.md`, items 165 to 178, 2026-11-20): starting
before the server, IMs and local chat both ways, commands, a stranger
refused, messages kept while Prosody was down and delivered with their
times, a restart, a wrong secret and `quit`. The rig's client is a minimal
script of the project's own; Gajim 2.6.0 and Dino 0.5.1 were tried on the
rig too. Gajim showed everything as described. Dino showed everything as
well, but sends only once a conversation is switched to unencrypted (see
"Encryption" under [XMPP gateway](#xmpp-gateway)). Conversations,
ejabberd and Second Life have not been tried.

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

The bot logs in, takes the avatar to the place the config describes, and
keeps it online: it logs in again after a kick, a region restart or a
network outage, with growing delays between attempts. Everything it does is
logged to standard error with UTC timestamps.

The place is a placement: a location (a region and coordinates in it) and a
sit state (an object, the ground, standing, or "leave it alone"). The
config describes it like this:

* `region` and `position`: the bot logs in there and teleports there when
  the login lands in another region. Without them the first login goes
  where `start` says, and the place it arrives at is the location.
* `sit` with an object's UUID: the bot sits on that object, trying again
  with growing delays when a sit fails. After three failed sits it
  teleports to the location and tries from there.
* `sit = "ground"`: the bot sits the avatar on the ground. The simulator
  does not confirm a ground sit, so the bot sends it once each time the
  avatar arrives somewhere, stands up from an object, or a placement is
  persisted, and cannot tell whether the avatar still sits.
* Without `sit` the bot does not sit the avatar.

Once the avatar is there (logged in, and seated when `sit` names an
object), the bot carries out the rest of its [startup script](#startup-script),
and a `persist` command in it decides whether the bot holds a placement
from then on, and which:

* Without one: none. The bot goes to manual mode and leaves the avatar
  alone; it only keeps it logged in. An avatar that is unseated or moved
  away stays that way, and after a kick the bot logs in at the avatar's
  last location.
* `persist cfg cfg`: `position` and `sit` exactly as the config has them.
* `persist` (which is `persist cur cur`): the position and the seat the
  simulator reported at that moment. The position is where the avatar
  really is, on its seat when it sits, and the seat is the prim the
  simulator sat it on, which in a linkset may not be the one `sit` names.

A bot that holds a placement keeps working towards "in the region it
holds" and "seated as it holds": it logs in there after a kick, teleports
back when the avatar is moved to another region, and sits again when
unseated, each with growing delays between attempts. Inside the region it
holds, it does not put the avatar back where it was: it reacts to a change
of region, and to being unseated from a held object. `rslbot ctl persist`
sets or changes the held placement while the bot runs, see
[Holding a place](#holding-a-place). Nothing of it is saved: a restarted
bot starts from the config and its startup script again.

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
| `region` | no, but only with `position` | Region the avatar starts out in; the bot logs in there, and returns there when a `persist cfg` says to hold the configured place |
| `position` | no, but only with `region` | `[x, y, z]` inside the region; pick a spot close to the object to sit on |
| `start` | no, and only without `region` and `position` | Where the first login puts the avatar: `"home"` (its home location) or `"latest"` (where it was last). Default: `"latest"` |
| `sit` | no | What to sit on at the start: the UUID of an object, or `"ground"`. Default: the avatar stands, and the bot does not sit it |
| `object` | no | Deprecated spelling of `sit`, for a UUID only; the bot logs a warning. Not together with `sit` |
| `logs` | no | A table, `[logs]`, with the paths of the log files: `chat`, `im` and `transactions`, see [Chat and IM logs](#chat-and-im-logs). Default: no log file |
| `fetch_offline_im` | no | `true` to also fetch IMs stored while the avatar was offline. Default: `false` |
| `socket` | no | `true` to listen on the control socket, see [Control socket](#control-socket). Default: `false` |
| `requesttp_timeout` | no | Minutes after `requesttp` during which the asked avatar's teleport offer is accepted. Default: `10` |
| `scan_distance` | no | Metres (1 to 1024) within which objects are learned from the start, for `touch` and `scan`; see [Touching objects](#touching-objects). Default: learn within 32 m when `touch` or `scan` is first used, or when an RLV restriction such as `sittp` needs an object the bot does not know |
| `restore_attachments` | no | `false` to keep the bot from attaching again, after each login, what the Current Outfit folder lists and the simulator did not bring; see [Attachments](#attachments). Default: `true` |
| `startup` | no, and not with `startup_file` | The [startup script](#startup-script): commands the bot carries out by itself when it starts, one per line. Default: none |
| `startup_file` | no, and not with `startup` | A file that holds the startup script; a relative path is relative to the working directory |
| `xmpp` | no | A table, `[xmpp]`, that makes the bot an XMPP component, see [XMPP gateway](#xmpp-gateway). Default: no XMPP connection |
| `web` | no | A table, `[web]`, that makes the bot serve a web page to use it from, see [Web interface](#web-interface). Default: no web interface |
| `payments` | no | A table, `[payments]`, that lets the bot pay, see [Payments](#payments). Default: the bot pays nothing |
| `persist`, `rlv`, `perms`, `autoreply` | no | Deprecated: what the startup script says with commands, see [Deprecated keys](#deprecated-keys) |
| `log_chat`, `log_im` | no | Deprecated spellings of `chat` and `im` in the `[logs]` table, see [Deprecated keys](#deprecated-keys) |

### Startup script

What `rslbot ctl` changes in a running bot is not saved. What the bot is
to start with goes into the startup script: the same commands, without
`rslbot ctl` in front, which the bot carries out by itself when it starts.

```toml
startup = '''
# Settings: before the first login.
rlv mode on
rlv persist follow
perms autoaccept tp "Jane Doe"
perms autoaccept anim all
perms revoke anim 01234567-9abc-defe-dcba-987654321000
autoreply message Hello, I'm bot. I won't respond to you in next 2 hours
autoreply pause 2h
# The rest: once the avatar is in place.
persist cur cfg
'''
```

Write it as a TOML literal string, between `'''` and `'''`: there a
backslash and a double quote are the script's own, where a string between
`"""` would want them escaped. Like every key, `startup` has to come
before the tables (`[xmpp]`) at the end of the file. `startup_file =
"startup.rsl"` names a file with the same content instead.

* One command per line, any command of the [control socket](#control-socket).
  An argument with spaces goes in double quotes; inside them `\"` is a
  double quote and `\\` a backslash. A line end inside double quotes
  belongs to the argument, so an autoreply text can have several lines.
  Blank lines and lines that start with `#` are skipped.
* The **setting commands** at the head of the script are carried out
  before the first login: `perms`, `autoreply`, and `rlv` other than
  `rlv cmd`. RLV is then on, and the rules are there, when the avatar
  arrives. The head ends at the first other command, so put the setting
  commands first: one further down waits with the rest.
* **The rest** starts once the avatar is at the place the config describes
  (logged in, and seated when `sit` names an object; after sitting the bot
  waits up to 10 seconds to learn where the seat is). Without `persist` in
  it, the bot is in manual mode from then on, as described under [Run](#run).
  The commands are carried out in order, one at a time: each after the
  answer of the one before, so a `sit` after a `tp` is sent when the avatar
  has arrived.
* The script is checked before the bot contacts any server. An unknown
  command, a wrong number of arguments, an unclosed quote, and a setting
  command at the head that is refused (`perms autoaccept money all`) are
  config errors: the bot names the line and exits with status 2.
* A command of the rest that is refused when it is due (`sit` on an object
  that is not there) stops the script: the commands after it are not
  carried out, the bot keeps running, and the log says how many were left
  out. The script does not wait for a lost connection to come back.
* The script is carried out once. It does not start again after a kick, a
  region restart or a network outage.
* If you move the avatar (`tp`, `sit`, `stand`, an accepted teleport
  offer) or run `persist` or `home` before the avatar is in place, the
  rest is not carried out, as the bot then no longer goes to that place.
  The setting commands have been carried out by then. Once the rest has
  started, your commands do not stop it.
* The script's commands count as yours: [RLV restrictions](#restrictions)
  apply to them, and a `tp`, `sit` or `stand` in it puts the bot in manual
  mode. `quit` at the end makes a bot that logs in, does its commands and
  leaves.

The log shows each command with the line it starts on, and what came of it:

```
startup: line 2: rlv mode on
startup: line 2 answered: rlv: on
...
startup: the avatar is in place; carrying out the rest of the startup script
startup: line 10: persist cur cfg
startup: line 10 answered: Target/129.33/127.6/26.45 11111111-2222-3333-4444-555555555555
startup: the script is done
```

### Deprecated keys

`persist`, `rlv`, `perms` and `autoreply` are the older way to say what
the startup script says. They still work as they did, and what they set is
in place before the script's first command; the `persist` setting is
applied when the avatar is in place, before the rest of the script. For
each of them a config uses, the bot logs at startup what to write instead:

| Deprecated | In the startup script |
|---|---|
| `persist = "cur"`, `persist = "cfg"`, `persist = ["cfg", "cur"]`, and `position` and `sit` under `[persist]` | `persist cur cur`, `persist cfg cfg`, `persist cfg cur`: the same two words, after the setting commands |
| `persist = "off"`, `persist = false` | Nothing: leave the key out |
| `rlv = "follow"` under `[persist]` | `rlv persist follow` |
| `override = ["sit", "tp"]` under `[persist]` | `rlv persist override sit tp` |
| `rlv = true` | `rlv mode on` |
| `anim = "all"`, `tp = ["Jane Doe"]` under `[perms.autoaccept]` | `perms autoaccept anim all`, `perms autoaccept tp "Jane Doe"` |
| `anim = [...]` under `[perms.except]` | `perms revoke anim ...`, after `perms autoaccept anim all` |
| `[autoreply]` with `message` and `pause` | `autoreply message ...` and `autoreply pause ...` |

`log_chat` and `log_im` are the older spellings of `chat` and `im` in the
`[logs]` table. They still work, and the bot logs at startup the line to
write under `[logs]` instead:

```toml
# was: log_chat = "chat.tsv"
[logs]
chat = "chat.tsv"
```

A config may not name one file both ways: `log_chat` together with `chat`
under `[logs]` is an error. Like every table, `[logs]` goes after the keys
that are not in a table.

`[perms.except]` was meant for kinds whose rule is `"all"`. The table also
takes an exception for a kind whose rule is a list (one object of an
avatar that is listed), and no command sets that; the bot says so when a
config has one.

The deprecated `persist` takes these forms:

| Form | Meaning |
|---|---|
| `"off"` or `false` | Hold nothing: manual mode once the avatar is in place. The same as leaving `persist` out |
| `"cfg"` | Keep holding `position` and `sit` as configured |
| `"cur"` | Hold the position and the seat the simulator reported |
| `["cfg", "cur"]` | An array of two, each `"cur"` or `"cfg"`: the first for the position, the second for the seat |
| `[persist]` | A table with any of the keys below. As a TOML table it has to come after all other keys |

| Key of `[persist]` | Meaning |
|---|---|
| `atstart` | `true` or `false`: whether the bot holds anything once the avatar is at the place above. Default: `true` when the table has `position` or `sit`, `false` otherwise. `atstart = true` alone is `persist = "cur"`; `atstart = false` together with `position` or `sit` is an error |
| `position`, `sit` | Each `"cur"` or `"cfg"`, as in the array; one that is left out is `"cur"` |
| `rlv` | `"drop"` or `"follow"`: what a sit, stand or teleport forced through [RLV](#rlv) does to the place the bot holds. Default: `"drop"`. It says nothing about `atstart`, so a table with only `rlv = "follow"` holds nothing at the start |
| `override` | `"sit"`, `"tp"` or an array of both: what the bot does by itself towards the place it holds whatever RLV restrictions say, see [What the bot does by itself](#what-the-bot-does-by-itself). Default: nothing. Like `rlv`, it says nothing about `atstart` |

### Upgrading

Configs written for earlier versions still load, but these things changed:

* With `rlv = true`, the restrictions that objects set are obeyed now.
  Earlier versions noted every one of them as not supported; a collar or a
  piece of furniture that sent them all along now holds the avatar, your
  `rslbot ctl` commands and the bot's own return to its place. See
  [Restrictions](#restrictions); `rslbot ctl rlv status` shows what is
  held, `rlv reset` and `rlv mode off` are the ways out, and
  `override = "tp"` in the `[persist]` table keeps the bot returning to
  its place.

* The bot used to hold `position` and the `sit` UUID for as long as it
  ran: it teleported back when the avatar was moved to another region and
  sat again when unseated. Now it goes there and sits once, and then
  leaves the avatar alone unless `persist` is set. Add `persist = "cfg"`
  to keep the old behaviour.
* A config without `region` used to mean "never teleport". With `persist`
  set, the bot now returns to the place the first login arrived at when
  the avatar is moved to another region, and logs in there after a kick.
  Without `persist` it still does not move the avatar by itself, but it
  does not sit it again either.
* `rslbot ctl home` returns to the last persisted placement, which need not
  be the configured one; `rslbot ctl persist cfg cfg` returns to the config.
* A `[persist]` table holds something at the start only when it has
  `position`, `sit` or `atstart = true`. An empty `[persist]` table used to
  mean `"cur"` for both; write `atstart = true` into it to keep that.
* `rslbot ctl accepttp AVATAR` is now `rslbot ctl accept tp [AVATAR]`. The
  old name still works. Under either name the avatar is no longer looked
  up on the grid: it is compared with the avatars whose offers wait, so a
  name nobody offered under is answered with "there is no teleport offer
  from ..." where it used to be "the grid knows no avatar named ...".
* With `rlv = true`, `recvim`, `recvim_sec` and `recvimfrom`, which the
  bot used to keep without an effect, now hide the instant messages they
  name and answer their senders, as an RLV viewer does. A restriction an
  object holds takes effect as soon as the new version runs.
* Inventory offers and scripts' requests for permissions, which the bot
  used to drop unread, are written to the log and wait for
  `rslbot ctl accept`. Nothing is accepted by itself without `perms
  autoaccept` commands in the startup script.
* `persist`, `rlv`, `[perms]` and `[autoreply]` are deprecated in favour of
  the [startup script](#startup-script). They still work; the bot logs
  what to write instead, see [Deprecated keys](#deprecated-keys).
* `log_chat` and `log_im` are deprecated in favour of the `[logs]` table,
  which also takes the new transactions file. They still work; the bot
  logs what to write instead.
* `status` has a second line, `avatar`, with the avatar's name, and, for
  an interface that may pay, two lines `balance` and `limit` at its end. A
  script that reads `status` by line number needs adjusting.

To keep the plain password out of the file:

```sh
target/release/rslbot hash          # type the password; prints $1$...
printf 'secret\n' | target/release/rslbot hash
```

Put the printed value in `password_hash` and leave `password` out.

## Chat and IM logs

The bot reads local chat the avatar hears and instant messages sent to it,
from avatars and from objects, and writes each one to standard error. It
talks only when told to through the [control socket](#control-socket) or
the [XMPP gateway](#xmpp-gateway), and answers instant messages by itself
only with an [autoreply](#autoreplies) and when RLV `recvim` hides them
(see [Restrictions](#restrictions)). The instant messages it sends on
command are recorded too, as kind `im_out`, and those automatic answers as
kind `im_auto`. The chat and the notices of the avatar's groups are
recorded apart, as [group messages](#group-chat-and-notices). Offers
(teleport offers and requests included; those appear on standard error
only), invitations, conferences and typing notifications are not
recorded.

The `[logs]` table names the files the bot appends to:

```toml
[logs]
chat = "chat.tsv"
im = "im.tsv"
group = "group.tsv"
transactions = "transactions.tsv"
```

With `chat` and/or `im` set, each message is also appended to that file as
one tab-separated line (UTF-8). The file is created when missing and gets a
header line when it is empty; relative paths are relative to the directory
the bot is started in. A file that cannot be opened makes the bot exit with
status 2 before it logs in. `group` and `transactions` are described
[below](#group-chat-and-notices).

| Column | Content |
|---|---|
| `time` | UTC, `YYYY-MM-DDTHH:MM:SSZ`: when the bot received the message, or when it was sent for an IM stored offline |
| `kind` | Chat: `whisper`, `say`, `shout`, `region`, `owner`, `direct`, `debug`. IM: `im` (from an avatar), `object` (from a script), `im_out` (sent by the bot on command), `im_auto` (an automatic answer the bot sent) |
| `avatar_name` | The speaker; for an object, its owner; for `im_out` and `im_auto`, the recipient |
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

An automatic answer follows the row of the message it answers:

```
2026-11-20T10:00:00Z<TAB>im<TAB>Jane Doe<TAB>JD<TAB><TAB><TAB>hi
2026-11-20T10:00:00Z<TAB>im_auto<TAB>Jane Doe<TAB>JD<TAB><TAB><TAB>back later
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

### Group chat and notices

Every line of chat in the session of a group the avatar is in, every line
the bot says there with [`group chat`](#groups), and every group notice is
a group message. Each one is written to standard error as one line, and,
with `group` set under `[logs]`, appended to that file as a row of nine
tab-separated columns, with the same time, escaping and header as the
other files:

```
time	kind	group_name	group_uuid	avatar_name	display_name	subject	attachment	text
```

| Column | Content |
|---|---|
| `time` | UTC: when the bot received the message or said the line; for a notice stored while the avatar was offline, when it was sent |
| `kind` | `chat` (heard), `chat_out` (said by the bot on command) or `notice` |
| `group_name`, `group_uuid` | The group |
| `avatar_name`, `display_name` | Who said or sent it; for `chat_out` the bot's own avatar. A notice that does not give its sender's UUID has no display name |
| `subject`, `attachment` | A notice's subject and the name of the item attached to it; empty for chat |
| `text` | The line, or the notice's text |

A notice's attachment is named, never taken. A line the grid delivers back
that the bot said itself, as OpenSim and, by all signs, Second Life do, is
not recorded twice. Lines of conferences
(ad-hoc sessions) are not recorded. The bot answers no group message by
itself, and none is passed to the owners of the
[XMPP gateway](#xmpp-gateway). Under [RLV](#restrictions) `recvim`, a
line is recorded as `(blocked by RLV: recvim)`.

### Transactions

The bot records the avatar's L$ transactions: every payment it makes (see
[Payments](#payments)) and every transaction the grid reports to the avatar
while it is online: a payment received, an object's payout, a fee. Each one
is written to standard error, with or without a `[payments]` table, and,
with `transactions` set under `[logs]`, appended to that file as one row of
twelve tab-separated fields, with the same escaping, header line and
appending as the chat and IM files:

```
time	kind	amount	balance	counted	type	party_uuid	avatar_name	display_name	object_name	object_uuid	text
```

| Column | Content |
|---|---|
| `time` | UTC, `YYYY-MM-DDTHH:MM:SSZ`: when the bot learned how it ended |
| `kind` | `paid` (the bot's payment, confirmed by the grid), `unconfirmed` (the bot's payment, neither confirmed nor refused within 15 s), `failed` (the grid reports it failed), `received` (the avatar was paid), `spent` (the avatar paid, and it was no payment of the bot's: a fee, a purchase in a viewer, a confirmation that came too late) |
| `amount` | Whole L$ |
| `balance` | The balance the grid reported with it; empty for `unconfirmed` |
| `counted` | How much of the amount counts against the limit: the amount for a `paid` or `unconfirmed` payment that counts, 0 for everything else |
| `type` | The grid's number for the kind of transaction (5001 a gift to an avatar, 5008 paying an object, ...) |
| `party_uuid` | The avatar or group that paid or was paid; for a payment to an object, its owner |
| `avatar_name`, `display_name` | That avatar's names, looked up as for messages; empty for a group |
| `object_name`, `object_uuid` | For a payment of the bot to an object: the object |
| `text` | What the grid said about it. For a gift with a description, as a viewer shows it: `You paid Jane Doe L$3: test payment`, `Jane Doe paid you L$5: for the hat`; for the bot's own payment, the description it sent when the grid does not repeat it |

A payment of the bot and the grid's confirmation of it are one row. The
grid has no request by which a viewer, or the bot, can fetch past
transactions: what happens while the avatar is offline shows in the
balance only.

The transactions file is also the record of the [spending
limit](#payments): at startup the bot reads it, and the rows with
`counted` greater than zero give what was spent in the current period, so
a restart does not renew the limit. Moving, truncating or editing the file
does: that is the operator's decision to make. A row that is cut short or
cannot be read is skipped, and the bot says how many it skipped; a file
whose first line is not the header above is a config error. When a row
cannot be written (a full disk, a file made read-only), it waits, and the
bot refuses every further payment that counts until it is written; it
tries again at each `pay`.

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
target/release/rslbot ctl scan 20 -n bench          # what is around, by name
target/release/rslbot ctl scan avatars chat         # who hears local chat
target/release/rslbot ctl friends                   # the friends, and who is online
target/release/rslbot ctl profile "Jane Doe"        # who an avatar is
target/release/rslbot ctl persist                   # stay here, like this
target/release/rslbot ctl home
target/release/rslbot ctl inventory "Bot gear"      # what the avatar owns there
target/release/rslbot ctl attach "Bot gear/Radio" left hand
target/release/rslbot ctl anim play "Bot gear/Dance"
target/release/rslbot ctl rlv mode on               # let owned objects steer the avatar
target/release/rslbot ctl rlv cmd @versionnum=5     # an RLV command, by hand
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
| `accept KIND [WHO]` | Accept a request that waits: `accept tp` a teleport offer (and go), `accept inventory` an inventory offer, `accept anim`, `attach`, `controls` or `camera` a script's request for permissions. Without `WHO`, the one that came last. See [Permission requests](#permission-requests) |
| `accepttp [AVATAR]` | The older name of `accept tp [AVATAR]`; deprecated |
| `sendtp AVATAR [MESSAGE...]` | Offer the avatar a teleport to the bot |
| `touch UUID` | Click that object once |
| `scan [objects] [DISTANCE] [FILTER...]` | List the objects around the avatar with their UUID, distance, owner and name. `DISTANCE` is metres, or `shout` (100), `chat` (20) or `whisper` (10). See [Scanning for objects](#scanning-for-objects) |
| `scan avatars [DISTANCE]` | List the other avatars in the region, or those within `DISTANCE`, with their UUID, distance, position and names. See [Scanning for avatars](#scanning-for-avatars) |
| `friends` | List the avatar's friends with their UUID, whether they are online, and their names. See [Friends](#friends) |
| `profile AVATAR` | Print an avatar's profile. See [Profiles](#profiles) |
| `inventory [-a] [-n] [FOLDER]` | List the avatar's inventory: everything, or what is under `FOLDER`. See [Inventory](#inventory) |
| `attach ITEM [BODY PART...]` | Attach an object from the inventory, to the named attachment point or else to the one it was last on. See [Attachments](#attachments) |
| `wear ITEM` | For an object, the same as `attach ITEM`. For clothing or a body part, put it on; only where the grid makes the avatar's appearance. See [Clothing](#clothing) |
| `detach WHAT...`, `unwear WHAT...` | Take off an item, everything on a body part, or all clothing of one kind (two names for the same command) |
| `anim play ITEM`, `anim stop ITEM` | Start or stop an animation from the inventory. See [Animations and sounds](#animations-and-sounds) |
| `sound ITEM` | Play a sound from the inventory once, where the avatar is |
| `persist [LOCATION] [SIT]` | Hold a place from now on: return to `LOCATION` whenever the avatar is in another region, and sit as `SIT` says. Without arguments: where the avatar is and how it sits now. See [Holding a place](#holding-a-place) |
| `home` | The same as `persist last last`: hold the place last persisted again, or the configured one when nothing was persisted |
| `perms autoaccept KIND WHO...`, `perms revoke KIND WHO...` | Add to, or take away from, the requests of a kind the bot accepts by itself; `WHO` is `all`, or avatars and objects. See [Permission requests](#permission-requests) |
| `perms status` | Print what is accepted by itself for each kind, and the requests that wait |
| `rlv mode on`, `rlv mode off` | Switch [RLV](#rlv) on or off. Off keeps the restrictions and applies none |
| `rlv mode override` | Let the next command through whatever the [restrictions](#restrictions) say; see [Override](#override) |
| `rlv persist follow`, `rlv persist drop` | What a sit, stand or teleport forced through RLV does to the place the bot holds |
| `rlv persist override none`, `sit`, `tp`, `sit tp` | What the bot does by itself whatever the restrictions say; see [What the bot does by itself](#what-the-bot-does-by-itself) |
| `rlv status` | Print the RLV mode, the two `rlv persist` settings, and the restrictions that are kept |
| `rlv reset` | Forget every restriction |
| `rlv cmd COMMAND...` | Carry out RLV commands as if an object had sent them, and print what became of each. Restrictions given this way are held apart from every object's |
| `status` | Print where the avatar is and what it sits on |
| `pay AMOUNT avatar AVATAR... [-d DESCRIPTION...]`, `pay AMOUNT object UUID` | Pay an avatar, with a description if you like, or an object nearby. Only with a `[payments]` table; see [Payments](#payments) |
| `prices UUID` | Print what an object offers to be paid |
| `payees` | List the avatars the whitelist and the unlimited list name |
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

A bot that holds a place, because the config has `persist` or a `persist`
or `home` command was given, returns to the region and the seat it holds
whenever it finds the avatar elsewhere. `stand`, `unsit`, `sit`, `tp`,
`requesttp` and `accept tp` switch that off, and so does a teleport offer
the bot accepts by itself. From then on the bot is in
manual mode, as it is without a `persist` in the startup script once the
avatar has got to the configured place: it stays logged in, and logs in again after a
kick or an outage (at the avatar's last location), but it no longer
teleports or sits by itself. `persist` and `home` switch holding on, and
the bot acts at once. `home` is not the avatar's home location: it is the place the bot
held last. Manual mode ends with the bot process too; a restarted bot
starts from its config and startup script. `sendtp`, `touch`, `scan`, `friends`, `profile`, the talking commands and
the inventory commands (`inventory`, `attach`, `wear`, `detach`, `anim`,
`sound`) leave the mode alone.

A sit, stand or teleport that a scripted object forces through [RLV](#rlv)
counts as the command it is carried out as: it switches holding off too.
With `rlv persist follow` the bot holds the result instead; see
[Forced movement and the place the bot holds](#forced-movement-and-the-place-the-bot-holds).

With RLV on, the bot's own steps towards the place it holds are held to
the RLV restrictions as your commands are: it waits where a restriction
forbids the teleport or the sit, unless `rlv persist override` says
otherwise. See [What the bot does by itself](#what-the-bot-does-by-itself).

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
  sit succeeded. `stand`, `requesttp` and `accept tp` do not count, and
  neither does anything carried out for [RLV](#rlv).
* With `rlv persist follow`, a sit, stand or teleport forced through RLV
  is a persist of its own: it replaces what the bot holds, and `home`
  returns to it.
* Nothing is saved. A restarted bot starts from its config and the
  `persist` command in its startup script.

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
avatar: Sit Bot
region: Target
position: 130, 128, 25.9
seat: target
seat name: Park Bench
persist: Target/130/128/25.9 11111111-2222-3333-4444-555555555555
mode: holding
balance: L$1200
limit: L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)
```

`avatar` is the avatar's name as the config gives it (a single word with
`Resident`).

`seat` is `target` (the object the bot holds as the seat), `object <uuid>`
(another object, named by the prim the simulator sat the avatar on),
`other` (attached to something the simulator did not name), `none` or
`unknown`. A ground sit, by command or held, shows as `none`: the bot
cannot see it. `seat name` is what the object the avatar sits on is called:
the name of its linkset's root prim, also when the avatar sits on a child
prim. The bot asks the simulator for it each time the avatar sits down, so
the line is missing for the moment that takes, and whenever the avatar
does not sit on an object. `persist` is the placement the bot holds, written the way
the `persist` command takes it; in manual mode it is what `home` goes back
to, and its location is `unknown` until the first login of a config
without `position`. `mode` is `holding` or `manual`.
`balance` and `limit` are there only for an interface that may pay (see
[Payments](#payments)): `balance` is the newest balance the grid reported,
`unknown` before it has; `limit` is what is left of the limit, `unlimited`,
and, while part of a renewing limit is used, when it renews. Without a
`[payments]` table, and for `rslbot ctl` when `perms` does not list
`ctl`, neither line is printed. With them, while RLV holds `sendim`, a
third line `pay description: blocked by RLV` follows, which is what the
web page goes by.
While the avatar is not in a region, only `connected: no`, `avatar`,
`persist`, `mode` and the money lines are printed.

### Teleport offers

`requesttp AVATAR` sends the request a viewer sends with "Request
Teleport". If that avatar answers with an offer within `requesttp_timeout`
minutes (10 unless configured), the bot accepts it and goes; one request,
one acceptance. `persist` and `home` cancel the wait.

Any other teleport offer is for the rules of [Permission
requests](#permission-requests) to decide. From an avatar that `tp` is
auto-accepted for, the bot accepts it and goes, and is in manual mode from
then on, as after `accept tp`: it stays where it was brought until
`persist` or `home`. Any other offer is noted on standard error and kept,
the latest one per avatar, until the session ends. `accept tp` accepts the
offer that came last, `accept tp AVATAR` that avatar's; the avatar is
compared with the names the offers came with and is not looked up on the
grid. Whether an old offer still works is up to the grid, and its refusal
is what the command prints. `accepttp` is the older name of `accept tp`.

When an avatar asks to be teleported to the bot, that is noted on standard
error as well, and `sendtp AVATAR` is the answer if you want to give one.

### Touching objects

The simulator takes a touch by its own short-lived number for the object,
not by UUID, so the bot can only touch objects it has been told about.
Much of that it picks up from what the simulator sends anyway. For the
rest it has to ask (a scan), and that costs traffic, so it asks only when
there is a reason:

- Without `scan_distance`, nothing extra is sent until `touch` names an
  object the bot does not know, a [`scan`](#scanning-for-objects) command
  is given, or an RLV restriction such as `sittp` or `fartouch` has to be
  applied to an object the bot does not know (see
  [Restrictions](#restrictions)). The first of these starts a scan of the
  objects within 32 m; a `touch` waits up to 10 seconds for its object to
  turn up, and later ones answer at once. A new region starts over.
- With `scan_distance = N`, the bot scans from the moment it enters a
  region and asks the simulator to describe objects within N metres, so
  `touch` answers at once and reaches further, at the price of that
  traffic all the time.

`touch` also takes the UUID of an inventory item, as
[`inventory`](#inventory) prints it, when that item is attached to the
avatar: it then clicks the attachment, with no scan and no wait, and under
the RLV restrictions that hold a touch of that attachment.

```sh
rslbot ctl inventory -a             # what is attached, with the items' UUIDs
rslbot ctl touch <item uuid>        # click that attachment
```

An object the bot has not learned of is an error naming the distance. On
OpenSimulator the distance makes little difference: it describes objects
regardless. `scan` with a larger distance widens the scan until the avatar
leaves the region, and `touch` then reaches that far too.

Two requests are made regardless of all this: each time the avatar becomes
seated on an object, the bot asks the simulator about that one object, to
know where the avatar is, and for its name, which `status` prints.

### Scanning for objects

```sh
rslbot ctl scan                     # everything within 32 m, or `scan_distance`
rslbot ctl scan 20 -n bench         # within 20 m, with "bench" in the name
rslbot ctl scan -t                  # what handles touches
rslbot ctl scan --pay               # what takes money
rslbot ctl scan 96 -o "Jane Doe"    # Jane Doe's objects within 96 m
rslbot ctl scan -a -c -o <uuid>     # an avatar's or a group's, worn ones and child prims too
rslbot ctl scan --position 20       # within 20 m, each with where it is
```

`scan [objects] [DISTANCE] [FILTER...]` lists the objects around the avatar
the way a viewer's area search does, nearest first, one line each. The word
`objects` changes nothing: `scan objects 20 -n bench` is `scan 20 -n bench`
(and `scan avatars` is [another command](#scanning-for-avatars)).

```
0c1b2f6e-...   3.2   scripted,touch   7d9a41c0-...   Jane Doe   Park Bench   Sit here
```

The seven fields are separated by tabs, and there is no header line:

| Field | |
|---|---|
| 1 | The prim's UUID, which is what `touch`, `sit` and `persist` take |
| 2 | Its distance from the avatar in metres |
| 3 | Its flags, separated by commas: `scripted`, `touch` (it handles touches), `pay` (it takes money), `physical`, `temporary`, `phantom`, `attachment` (an avatar wears it), `child` (not the root prim of its linkset); `-` when it has none |
| 4 | Its owner's UUID; the group's when a group owns it |
| 5 | Its owner's name, or `(group)` |
| 6 | Its name |
| 7 | Its description, which is often empty |

A tab, line feed, carriage return or backslash in a name or description is
written as `\t`, `\n`, `\r` and `\\`. Where the bot does not know a field
from the fourth on, it prints `?`.

| Filter | Lists only |
|---|---|
| `-n TEXT`, `--name TEXT` | prims with `TEXT` in their name, in any letter case |
| `-d TEXT`, `--desc TEXT` | prims with `TEXT` in their description |
| `-o WHO`, `--owner WHO` | prims of that avatar (`"First Last"`, `first.last`, one word, or a UUID) or of the group with that UUID |
| `-s`, `--scripted` | scripted prims |
| `-t`, `--touch` | prims that handle touches |
| `--pay` | prims that take money: what `pay ... object` and `prices` take |
| `-p`, `--physical` | physical prims |
| `--temporary` | temporary prims |
| `--phantom` | phantom prims |

Two switches list more: `-a` (`--attachments`) adds what avatars wear, the
bot's own avatar included, and `-c` (`--children`) the child prims of
linksets, each with its own name. Without them a linkset is one line, for
its root prim, and nothing worn is listed. Avatars, trees and grass are
never listed. Filters and switches combine in any order, and a prim has to
meet all the filters.

`--position` adds a field to every line, between the distance and the
flags: where the object is in the region, as `X,Y,Z` in whole metres, which
is what `tp` takes to go there. It is the position the distance is
measured to, so a child prim has its root's. The lines then have eight
fields; without the switch they are the seven above.

```
0c1b2f6e-...   3.2   103,100,25   scripted,touch   7d9a41c0-...   Jane Doe   Park Bench   Sit here
```

The distance is measured from the avatar to the object's root prim; a
child prim counts as being where its root is, and what an avatar wears as
being where that avatar is. Without `DISTANCE` the listing reaches as far
as the bot looks for objects: `scan_distance`, or 32 m. `DISTANCE` is a
number of metres from 1 to 1024, or one of the words `shout`, `chat` and
`whisper`, in lower case, for 100, 20 and 10 metres. A larger `DISTANCE`
makes the bot look that far from then on, until the avatar leaves the
region; a smaller one only shortens the listing.

On OpenSimulator a distance that reaches towards a neighbouring region (64
m or more from the middle of a region) makes the simulator count that
region as in view, and it then keeps the avatar's presence in a region it
leaves for the one next door, together with the connection the bot had
there. The bot keeps that connection up, as a viewer does: it answers the
region's pings and acknowledges what it sends, without reading it, until
the region closes the connection, and a teleport back into the region goes
over it. That is so for `scan_distance` as well; see "Teleports next door"
in `docs/test-rig.md`. The bot still opens no connection to a region the
avatar has not been in.

A scan asks the simulator what each object in reach is called and whose it
is, which it does the way the viewer does: by selecting the object and
ending the selection at once. A physical object the avatar owns stops for
that moment. The owners' names come from the grid's name service. Both are
asked for once: a second `scan` of the same objects answers at once, and a
renamed object keeps its old name in the listing until the avatar has left
the region and come back.

The answer comes when the simulator has described nothing new for 3
seconds and everything in reach is named: after about 3 seconds for the
first scan in a region or after a larger `DISTANCE`, at once later on. It
comes after 30 seconds at the latest, with `?` for what is still missing; a
second `scan` then goes on where the first stopped. A scan that waits when
the avatar leaves the region fails.

`-s` and `-t` go by what the simulator says of the listed prim itself. Where
it marks only the child prim that holds the script, the root prim's line
lacks the flag: add `-c` when an object you expect is missing.

`scan` moves nothing. While [RLV](#rlv) is on, it is held to what limits
a viewer's area search and sight, and to nothing else:

* Under `shownames`, `shownames_sec` or `shownametags` the owner's UUID and
  name are `?` for an object of an avatar those restrictions hide; an
  exception (`shownames:UUID`, `shownametags:UUID`) shows that avatar, and
  the bot's own avatar and groups are always shown. `-o` is refused while
  any of the three is held, whoever it names.
* `camdrawmax:DISTANCE` is a fog the avatar cannot see through: nothing
  farther away is listed, the bot does not look farther for objects on
  account of a larger `DISTANCE`, and the operator log says the scan was cut
  to that distance by `camdrawmax`. The smallest held counts, and none while
  a `camdrawalphamax` below 0.99 is held (the largest such counts), which
  makes the fog thin enough to see through.

Under an [override](#override) a `scan` is listed as with no restriction
kept, and uses the override up like any other command.

### Scanning for avatars

```sh
rslbot ctl scan avatars             # everyone in the region
rslbot ctl scan avatars chat        # who hears local chat: within 20 m
rslbot ctl scan avatars 50          # within 50 m
```

`scan avatars [DISTANCE]` lists the other avatars in the avatar's region,
nearest first, one line each:

```
7d9a41c0-...   3.2   103,100,25   Jane Doe   JD
```

The five fields are separated by tabs, and there is no header line: the
avatar's UUID, its distance from the bot's avatar in metres, its position
in the region as `X,Y,Z` in whole metres, its name (`First Last`) and its
display name. A tab, line feed, carriage return or backslash in a name is
written as `\t`, `\n`, `\r` and `\\`. Without `DISTANCE` everyone in the
region is listed, however far; with it (metres, or `shout`, `chat` or
`whisper` as for `scan`) only those known to be within it. Avatars in
neighbouring regions are not listed.

The bot asks the simulator nothing for this: it uses what the simulator
tells every viewer unasked, so `scan avatars` does not start the object
scan or widen it, and the only traffic is the name lookups. An avatar the
simulator has described to the bot is where the simulator says, and a
seated one where its seat is; Second Life describes what is within the
distance the bot looks for objects (`scan_distance`, or 32 m), and
OpenSimulator every avatar in the region. Every other avatar is where the
simulator's list of the region's avatars puts it, as a viewer's minimap
does: in whole metres, and in steps of 4 m in height. That list cannot hold
a height above about 1020 m: such an avatar is listed with `?` as its
distance and its height, and is left out when a `DISTANCE` is given. So is
one at a height of 0 to 4 m that is not described, which is rare. A name
the name service does not give is `?`.

On OpenSimulator the list holds at most 60 avatars, and regions larger
than 256 m (var regions) do not fit its coordinates; as OpenSimulator
describes every avatar in the region, neither limit shows in the listing.

The answer comes once the simulator has sent its list since the avatar
entered the region, which it does every few seconds, and the names are
there: at once for a second `scan avatars`. It comes after 10 seconds at
the latest, with what is known by then. A `scan avatars` that waits when
the avatar leaves the region fails.

`scan avatars` moves nothing. While [RLV](#rlv) is on, it is held to what
limits a viewer's radar and minimap:

* Under `shownames`, `shownames_sec` or `shownametags` an avatar those
  restrictions hide has a line with its distance and position, and `?` as
  its UUID, name and display name; the bot does not ask the name service
  about it. An exception (`shownames:UUID`, `shownametags:UUID`) shows that
  avatar.
* `shownearby` refuses `scan avatars`.
* `showminimap` makes every position `?`; distances, the order and
  `DISTANCE` are as without it.

`camdrawmax` does not cut `scan avatars`. Under an [override](#override) it
lists as with no restriction kept, and uses the override up.

### Friends

```sh
rslbot ctl friends
```

`friends` lists the avatar's friends, one line each, online ones first,
then by name:

```
11111111-2222-3333-4444-555555555555	online	Jane Doe	JD
22222222-3333-4444-5555-666666666666	online	John Roe	John Roe
33333333-4444-5555-6666-777777777777	offline	Ann Poe	Annie
44444444-5555-6666-7777-888888888888	?	Zed Low	Zed
```

The four fields are separated by tabs: the friend's UUID, its status, its
name and its display name, `?` for a name the name service does not give.
The status is `online` or `offline`, or `?` for a friend that does not let
the avatar see whether it is online (it has not given the right to see
it).

The list is the one the grid gives at login, kept up to date while the bot
runs: from the simulator's notices that friends come online and go
offline, from a friend ending the friendship, and from an avatar
accepting a friendship the avatar offered from a viewer. The bot itself
never offers, accepts or ends a friendship, and leaves offers made to the
avatar unanswered. Right after a login a friend that is online may show as
`offline` for a moment, until the simulator's notice has come. The names
are looked up at the first `friends`, which with many friends can take
until its 10-second limit; what is missing then is `?`, and the next
`friends` has it. `friends` is not held to RLV restrictions and does not
use up an [override](#override).

### Profiles

```sh
rslbot ctl profile "Jane Doe"
rslbot ctl profile 11111111-2222-3333-4444-555555555555
```

`profile AVATAR` prints the profile of an avatar given as for `im`:

```
id: 11111111-2222-3333-4444-555555555555
name: Jane Doe
display name: JD
online: yes
friend: yes
born: 2010-03-14
partner: 22222222-3333-4444-5555-666666666666
about: Builder.
first life: Not telling.
```

`id`, `online` and `friend` are always there. `name` and `display name`
are left out when the name service does not give them, `born` when the
grid does not tell it, `partner` when there is none, `about` and `first
life` when they are empty. A line feed in a text is written as `\n`, as
in the listings. `born` is `YYYY-MM-DD`, or as the grid wrote it when it
is not a day.

`online` is `yes` when the grid says the avatar is online, and otherwise
`no` for a [friend](#friends) that lets the avatar see whether it is
online, and `unknown` for everyone else: the grid answers for an avatar
that hides its status as it does for one that is offline.

The grid is asked anew for every `profile`: through its profile service
where the simulator offers one (Second Life), and otherwise on the
simulator connection (OpenSimulator). The answer comes within 10 seconds
of the avatar being found, or the command fails naming the avatar. An
avatar the grid does not know fails at once where there is a profile
service, and after the 10 seconds otherwise. `profile` is not held to RLV
restrictions, `shownames` included, and does not use up an
[override](#override).

### Groups

```sh
rslbot ctl group list
rslbot ctl group activate Builders           # or: group activate none
rslbot ctl group invite Builders "Jane Doe"
rslbot ctl group chat "Night Owls" see you at eight
rslbot ctl group info 01234567-9abc-defe-dcba-987654321000
rslbot ctl group join 01234567-9abc-defe-dcba-987654321000 --pay 250
rslbot ctl group leave Dancers
rslbot ctl group invitations
```

The bot asks the grid for the avatar's groups after each login and
follows every change the grid reports: a group joined, one the avatar was
ejected from, the active group. `group list` prints one line per group,
by name: the UUID, the flags `active` (the group whose tag the avatar
wears), `invite` (the avatar may invite into it) and `notices` (it gets
the group's notices), or `-`, and the name, separated by tabs. A command
that comes right after a login, before the grid has told the groups,
waits for them for up to 10 seconds.

`GROUP` is one argument: the group's UUID, or the name of one of the
avatar's groups in any letter case, as a whole. When two of the avatar's
groups have that name the command does nothing and lists both with their
UUIDs:

```
$ rslbot ctl group chat Friends hello
error: the avatar is in 2 groups named "Friends"; give the UUID of one: 11111111-... "Friends", 22222222-... "Friends"
```

`group activate` makes a group the active one, and is answered when the
grid says so (within 10 seconds) with the title the avatar wears there;
the words of a name may be separate arguments. `group invite` invites an
avatar, as `im` names one, into the group's role Everyone; the avatar
needs the right to invite. Neither grid tells the inviter whether the
invitation arrived, and the answer says so. `group chat` says a line in a
group's chat; the first line after a login enters the group's session
first, as a viewer does when its chat window opens, and fails with the
grid's reason when the grid refuses that. `group info` prints a group's
profile as `name: value` lines (`name`, `group`, `founder`, `fee`,
`enrollment`, `members`, `roles`, `listed`, `mature`, `member`, `title`,
`charter`), of any group by its UUID.

`group join` joins a group by its UUID, when its enrollment is open. A
group that costs L$ to join is joined only with `--pay` and a sum not
below its fee; the fee, not the sum, is paid, and only as
[Payments](#payments) allows: from an interface `payments.perms` lists,
within the limit and the lists. Without `--pay`, or with too little,
nothing is sent and the error names the fee:

```
$ rslbot ctl group join 01234567-9abc-defe-dcba-987654321000
error: joining "Builders" <01234567-...> costs L$250; give `--pay 250` to pay it
```

The join is answered when the grid lists the group, within 15 seconds.

`group leave` leaves a group, as a viewer's Leave does, and is answered
when the grid says the avatar has left it (within 15 seconds); the words
of a name may be separate arguments. Leaving the active group leaves
none active. When a leave and a join of one group follow each other
closely, the grid's lists can arrive out of order: for 5 seconds after a
join or a leave is confirmed the bot keeps to it, and then asks the grid
for the groups again. Second Life tells an avatar it ejected only by an
instant message, so the bot may still hold it in the group: when the grid
refuses a leave or a join, the bot asks for the groups and answers by
them (a leave from a group no longer listed succeeds, saying the avatar
was not in it any more), and `group join` of a group the bot holds the
avatar in asks the grid before it refuses. The bot does not check, as a viewer does, whether the
avatar is the group's only owner: the grid refuses that, and the
command fails with that refusal. Nothing is paid or refunded, and joining
again may take an invitation or the fee once more. `setgroup` does not
hold it back, as it does not in viewers.

`group invitations` lists the [invitations that wait](#invitations-to-groups).
`group list`, `group info` and `group invitations` only read: no RLV
restriction applies to them, and they never use up an
[override](#override). No group command moves the avatar.

Not there: creating a group, ejecting members, roles and
titles, sending notices, taking a notice's attachment, conferences,
searching groups by name, and `@setgroup:NAME=force` and `@getgroup`.

### Inventory

```sh
target/release/rslbot ctl inventory                    # all of My Inventory
target/release/rslbot ctl inventory "Bot gear"         # everything under that folder
target/release/rslbot ctl inventory -n Clothing        # its direct children only
target/release/rslbot ctl inventory -a                 # what is attached now
```

`inventory [-a] [-n] [FOLDER]` prints one line for each entry under
`FOLDER`, or under My Inventory when none is given, with five fields
separated by tabs and no header:

```
b14608d3-f78e-4bf5-adde-d27c3c286c04	object	Left Hand	attached	Bot gear/Radio
```

| Field | Is |
|---|---|
| 1 | The entry's UUID |
| 2 | Its type: `folder`, `object`, `clothing`, `bodypart`, `animation`, `sound`, `gesture`, `texture`, `notecard`, `script`, `landmark`, `callingcard`, `link` (a link to another item or folder), or `other` |
| 3 | For an object, the attachment point it is on now, or else the one it was last on, or `-` when the inventory records none (OpenSimulator's does not: there the point shows only while the object is attached). For clothing and body parts, the kind (`shirt`, `skin`, ...). `-` for everything else |
| 4 | `attached` when the item is attached to the avatar now, else `-` |
| 5 | Its path from the top of My Inventory, with a `/` at the end for a folder |

The lines are in the order of their paths. `-n` (or `--no-recurse`) lists
only what is directly in the folder; without it, everything below is listed
too. `-a` (or `--attached`) keeps only the items attached to the avatar now.
The flags can stand before or after the folder. A folder with nothing to
list prints nothing, with exit status 0. The folder itself is not listed,
and neither is the Library: My Inventory is the only tree these commands
see.

`ITEM` and `FOLDER` are always one argument, in one of two forms:

* a UUID, as `inventory` prints it in the first field;
* a path from the top of My Inventory, such as `"Bot gear/Radio"`: names
  separated by `/`. Quote a path with spaces. Names have to match exactly,
  letter case included.

In a path, `\/` is a `/` inside a name and `\\` is a backslash; `inventory`
prints paths that way, along with `\t`, `\n` and `\r` for a tab and line
breaks in a name, and reads them back the same. A `/` at the start or end
of a path is ignored, which is how to name something that would otherwise
be read differently: `/-n` is a folder called `-n`, and `detach /Chest`
means an item called `Chest`, not the body part. When two entries in one
folder have the same name, a path through them is refused and the error
lists their UUIDs, which always work. A link stands for the item it points
to: `attach "Outfits/Work/Radio"` attaches the item the link leads to. The
last name of a path means an item for every command but `inventory`, where
it means a folder, so an item and a folder with one name do not get in each
other's way.

Nothing is kept: every command asks the grid for the folders it needs, one
folder for each name of a path, and a bot that is never given one of these
commands asks the grid nothing about the inventory, except once after each
login for the Current Outfit folder where `restore_attachments` applies
(see [Attachments](#attachments)). A change made with a
viewer is seen by the next command. A folder given by UUID is looked for
from the top of My Inventory, to print the paths, so on a large inventory a
path is the quicker way to name one.

The grid has 50 seconds to answer everything one command needs. A whole
inventory of many thousands of items may not be listed in that time, and
the error then says to name a folder; `inventory -a` looks through the same
folders and has the same limit. The simulator's inventory service is
needed: on a grid without it the commands say so.

### Attachments

```sh
target/release/rslbot ctl attach "Bot gear/Radio" left hand
target/release/rslbot ctl detach "Bot gear/Radio"
target/release/rslbot ctl attach "Bot gear/Radio"      # where it was last
target/release/rslbot ctl detach left hand             # everything on that point
target/release/rslbot ctl wear "Bot gear/Radio"        # an object: the same as attach
```

`attach ITEM [BODY PART...]` attaches an object to the named attachment
point. Without one the simulator puts it where the inventory says it was
attached last, and chooses for itself when the inventory records nothing.
The command answers with the item and the point once the simulator shows
the object on the avatar, or with an error after 30 seconds. (An object
that arrives later still is noted in the log, and in the Current Outfit
folder like any other the bot attached.) Nothing is taken off to make room: several objects can share a point, so repeating an
`attach` with other items piles them up; `inventory -a` shows what is on
and `detach BODY PART` clears a point. An item that is attached already is
refused, with the point it is on. `wear ITEM` on an object does exactly what
`attach ITEM` does.

`detach WHAT...` (or `unwear`, the same command) takes off, in this order
of reading: the item with that UUID; everything attached to that body part;
all clothing of that kind (see [Clothing](#clothing)); the item at that
path. It answers with what was detached once the simulator shows the
objects gone, or with an error after 15 seconds; taking everything off a
point is answered once, naming each item.

`BODY PART` is the name viewers show for an attachment point, in any letter
case, with `HUD ` in front for the points of the heads-up display, or the
point's number from 1 to 127. Its words need no quotes:

| | | | |
|---|---|---|---|
| 1 `Chest` | 2 `Skull` | 3 `Left Shoulder` | 4 `Right Shoulder` |
| 5 `Left Hand` | 6 `Right Hand` | 7 `Left Foot` | 8 `Right Foot` |
| 9 `Spine` | 10 `Pelvis` | 11 `Mouth` | 12 `Chin` |
| 13 `Left Ear` | 14 `Right Ear` | 15 `Left Eyeball` | 16 `Right Eyeball` |
| 17 `Nose` | 18 `R Upper Arm` | 19 `R Forearm` | 20 `L Upper Arm` |
| 21 `L Forearm` | 22 `Right Hip` | 23 `R Upper Leg` | 24 `R Lower Leg` |
| 25 `Left Hip` | 26 `L Upper Leg` | 27 `L Lower Leg` | 28 `Stomach` |
| 29 `Left Pec` | 30 `Right Pec` | 31 `HUD Center 2` | 32 `HUD Top Right` |
| 33 `HUD Top` | 34 `HUD Top Left` | 35 `HUD Center` | 36 `HUD Bottom Left` |
| 37 `HUD Bottom` | 38 `HUD Bottom Right` | 39 `Neck` | 40 `Avatar Center` |
| 41 `Left Ring Finger` | 42 `Right Ring Finger` | 43 `Tail Base` | 44 `Tail Tip` |
| 45 `Left Wing` | 46 `Right Wing` | 47 `Jaw` | 48 `Alt Left Ear` |
| 49 `Alt Right Ear` | 50 `Alt Left Eye` | 51 `Alt Right Eye` | 52 `Tongue` |
| 53 `Groin` | 54 `Left Hind Foot` | 55 `Right Hind Foot` | |

A point without a name is printed as its number.

The bot knows what is attached from what the simulator tells it about the
objects on the avatar, which it does without being asked; nothing is sent
to find out. So the bot also knows of things attached before it started, or
attached and detached by a script, and notes on standard error whatever
comes or goes without a command of its own. After a teleport to another
region or a new login it learns it again from the new simulator, which may
take a moment.

On Second Life an attachment that was there at one login can be missing at
the next: seen each time after another object had been attached to the same
point and detached again (see [Status](#status)). A viewer makes up for
that at every login, and so does the bot. Once after each login, when the
avatar has been in a region for 20 seconds, it reads the Current Outfit
folder and asks the simulator to attach every object the folder lists that
is not on the avatar, to the point it was last on. It takes nothing off,
whether the folder lists it or not, and it does this once: not again after
a teleport, and not a second time for an object the simulator does not
bring within 30 seconds. The log says what it asked for and what came of
it:

```
attachment: restoring "Radio" <UUID>, listed in the Current Outfit folder and not on the avatar
attachment: restored "Radio" <UUID> to Left Hand
```

So the folder is what counts: an object taken off by something that does
not update it, a script for instance, is back after the next login.
`restore_attachments = false` in the config switches all of this off; the
bot then asks the grid nothing about the folder unless a command needs it.
On a grid without the outfit service (see [Clothing](#clothing)) the option
does nothing either way, because the bot does not keep the folder there.
That is the case on the local OpenSimulator, where the loss was not seen
either (`docs/test-rig.md`, item 105), so there it makes no difference
whether the option is on or off.

Until that moment, and until a restored object has arrived, the bot treats
an attachment the simulator does not show as not attached.

None of these commands moves the avatar or changes whether the bot is
holding its place.

### Clothing

Second Life makes an avatar's appearance on its servers from the avatar's
Current Outfit folder. There, `wear ITEM` puts on a piece of clothing or a
body part by adding it to that folder and asking the grid for the new
appearance, and `unwear` takes clothing off the same way:

| Command | Does |
|---|---|
| `wear "Clothing/Blue shirt"` | Adds the shirt to what the avatar wears; nothing is taken off |
| `wear "Body Parts/Tanned skin"` | Replaces the skin: an avatar wears exactly one shape, skin, hair and eyes |
| `unwear "Clothing/Blue shirt"` | Takes that piece off |
| `unwear shirt` | Takes off every piece of that kind: `shirt`, `pants`, `shoes`, `socks`, `jacket`, `gloves`, `undershirt`, `underpants`, `skirt`, `alpha`, `tattoo`, `physics`, `universal` |

They answer when the grid has accepted the new outfit. When it refuses, the
bot puts the folder back as it was and prints the grid's reason; should
that fail too, the error says the folder now differs from what the avatar
wears, and `unwear` or a viewer puts it right. A body part cannot be taken
off, only replaced: `unwear skin` and `unwear` of a body part item are
refused.

On the same grid the bot also notes in the Current Outfit folder what its
`attach`, `wear` and `detach` commands attached and detached, as viewers
do, so that a viewer logging in to the account later finds the avatar as
the bot left it. That happens after the command has answered; if the grid
refuses it, a line on standard error says so and the attachment itself is
not affected.

OpenSimulator has no such service: it expects the viewer to paint the
avatar's skin and clothing into textures and upload them, which the bot
cannot do. There `wear` and `unwear` of clothing and body parts fail with
`this grid makes the avatar's appearance in the viewer, which the bot
cannot do; ...`, and the Current Outfit folder is left alone. Objects are
not affected: `attach`, `wear` on an object and `detach` work on both.

On Second Life `wear` and `unwear` of one piece of clothing and the notes
in the Current Outfit folder have been tried; the rest of this section has
not. See [Status](#status).

### Animations and sounds

```sh
target/release/rslbot ctl anim play "Bot gear/Dance"
target/release/rslbot ctl anim stop "Bot gear/Dance"
target/release/rslbot ctl sound "Bot gear/Bell"
```

`anim play ITEM` starts an animation from the inventory on the avatar, on
top of whatever else it is playing, and `anim stop ITEM` stops it; stopping
one that is not playing is not an error. `sound ITEM` plays a sound from
the inventory once, at full volume, where the avatar is, for those nearby
to hear. All three answer as soon as the request has been handed to the
simulator connection. `ITEM` has to be in My Inventory: an animation or
sound cannot be played by its asset's UUID, and the built-in animations
have no names here.

An animation plays until it is stopped, ends by itself, or the session
ends. The bot does not remember it: after a kick or an outage it logs in
again without starting the animation again.

### RLV

RLV, the RestrainedLove API, is how scripted objects steer an avatar:
the object says a line starting with `@` to its owner (`llOwnerSay`), and
a viewer that implements the API carries it out. Collars, furniture,
relays and many games work this way, and most of them first ask for the
version and do nothing for an avatar that does not answer.

RLV is off unless the [startup script](#startup-script) has `rlv mode on`
or `rslbot ctl rlv mode on` was given. While it is off the bot does not read such lines at all, and
nothing below applies. **While it is on, every object the avatar owns,
worn or rezzed in the region, can make the avatar sit, stand and teleport,
can take attachments and clothing off it, and can forbid it, the bot and
you most of what the bot does, without the operator.**
That is what RLV is for; switch it on only for an avatar whose objects you
trust.

```sh
target/release/rslbot ctl rlv mode on
target/release/rslbot ctl rlv status       # rlv: on / persist: drop / persist override: none / restrictions: none
target/release/rslbot ctl rlv mode off
```

The switch is not saved: a restarted bot starts from its config, with RLV
on when the startup script has `rlv mode on`. There it belongs among the
setting commands at the head, which are carried out before the first
login: attachments say their restrictions in the first seconds after it.

What the bot implements is a part of the API, and it says so: it reports
version 0.1.0, far below any viewer's, so that a script that looks at the
number sees an incomplete implementation. Restrictions are kept, and those
that concern something the bot does are enforced; see
[Restrictions](#restrictions). The others are kept and have no effect, and
some queries, `@notify` and the shared `#RLV` folder are missing. A script
that reads back its restrictions with `@getstatus` gets what it set.

A message is read as RLV commands when an object says it to its owner and
it starts with `@`. Several commands follow the one `@`, separated by
commas, each as `behaviour[:option]=param`. They are dealt with in order
and each on its own. The behaviour and the param may be in any letter
case; a space anywhere makes another command of it, as in a viewer. The
line stays in the chat log like any other chat of kind `owner`.

These queries are answered, by saying the reply on the channel named
(any channel but 0, negative ones included):

| Command | Reply |
|---|---|
| `@version=CHANNEL` | `RestrainedLife viewer v0.1.0 (RusselBot 0.1.0)`, with the bot's own version in the parentheses |
| `@versionnew=CHANNEL` | `RestrainedLove viewer v0.1.0 (RusselBot 0.1.0)` |
| `@versionnum=CHANNEL` | `10000` |
| `@getstatus=CHANNEL` | The restrictions the asking object holds, each with a `/` in front: `/detach/sendim`. `@getstatus:PART;SEPARATOR=CHANNEL` leaves out those that do not contain `PART` and puts `SEPARATOR` in place of the `/` |
| `@getstatusall=CHANNEL` | The same for the restrictions of every holder |

These commands are carried out, each the way the `rslbot ctl` command
beside it is, with the same checks and refusals:

| RLV command | Carried out as |
|---|---|
| `@sit:UUID=force` | `sit UUID` |
| `@sitground=force` | `sit ground` |
| `@unsit=force` | `stand` |
| `@tpto:REGION/X/Y/Z=force` | `tp X Y Z REGION` |
| `@tpto:X/Y/Z=force` | A teleport to that point of the grid, X and Y in metres from the grid's origin. `rslbot ctl tp` has no such form |
| `@detach:POINT=force`, `@remattach:POINT=force` | `detach POINT` |
| `@detach:UUID=force`, `@remattach:UUID=force` | `detach` of the attachment with that UUID |
| `@remoutfit:KIND=force` | `unwear KIND` |
| `@detachme=force` | `detach` of the item the object that says it was attached from |

`POINT` is an attachment point by the names of [Attachments](#attachments)
or by the names RLV has for some of them: `center 2`, `top right`, `top`,
`top left`, `center`, `bottom left`, `bottom`, `bottom right` for the
points of the heads-up display, and `root` for `Avatar Center`. A number
is not taken as a point. `UUID` for `@detach` and `@remattach` is the
attached object's own, which is what a script knows; the bot also takes
the UUID of the inventory item, as `inventory -a` prints it, since the RLV
specification does not say which is meant. What follows a `;` in `@tpto` (a direction to
look in) is ignored. Nobody waits for the outcome of these commands: it is
in the log, as it is for the commands they stand for. None of them counts
as the last `tp` or `sit` command for `persist last_cmd`.

What is left is noted in the log as not supported and has no effect:
restrictions the API does not have, `@notify`, the other queries
(`@getsitid`, `@getoutfit`, `@getattach`, `@getinv`, `@versionnumbl`, ...),
everything that names inventory folders (`@attach:...`, `@detachall:...`,
`@detach:FOLDER`), and `@detach=force` and `@remoutfit=force` without an
option, which would take everything off. An instant message that says
`@version` is not answered either. Two limits of what is carried out:

* `@tpto:X/Y/Z` takes regions to be 256 metres square, which they are on
  Second Life. On a grid with larger regions use the form with the
  region's name. X and Y are global, counted from the grid's origin; on
  Second Life they run into the hundreds of thousands. Numbers that are
  both below 256 are refused, as they are most likely meant for a place
  inside a region, which is `@tpto:REGION/X/Y/Z`.
* Commands that arrive while the avatar cannot act, during the login or a
  teleport, are kept (up to 64 messages) and carried out in order once it
  is in a region. They are dropped when the session is lost and when RLV
  is switched off.

#### Restrictions

A restriction is an RLV command that ends in `=n` or `=add`; `=y` or `=rem`
lifts it again. `@sendim=n` forbids instant messages, `@sendim:UUID=add` is
an exception to that for one avatar, and `@clear` lifts everything its
sender set (`@clear=tp` everything with `tp` in it). The bot keeps
restrictions the way a viewer does:

* Each belongs to the object that set it, its *holder*. What two objects
  both set holds until both have lifted it, and no object can lift or
  clear another's.
* An object's restrictions end with the object: when it is detached or
  deleted, when the avatar leaves the region without wearing it, and when
  the session is lost. Scripts set theirs again when they are rezzed or
  attached, as the API tells them to. An attachment keeps its restrictions
  through a teleport.
* Nothing is saved: a restarted bot keeps no restriction.

While RLV is on, the restrictions apply to everything the bot does for the
avatar: to your `rslbot ctl` commands, to what the bot does by itself, and
to RLV force commands. A command that a restriction forbids does nothing
and exits with status 1:

```
$ rslbot ctl stand
refused by RLV: unsit, held by Collar <2f1a...>; `rslbot ctl rlv mode override` lets one command through
```

`rslbot ctl rlv status` lists what is kept, each restriction with its
holder; see [When the bot is held](#when-the-bot-is-held) for the ways out.

These restrictions have an effect:

| Restriction | Effect |
|---|---|
| `sendchat` | `say`, `shout` and `whisper` on channel 0 are refused. An emote (`/me ...`) and other text starting with `/` still go out, cut off at the first `.` and to 30 and 15 characters, and are refused when they contain one of `()"-*=_^`. `emote` held by anyone leaves emotes whole |
| `chatshout`, `chatwhisper`, `chatnormal` | On channel 0 a shout, or a whisper, is said at normal volume; with `chatnormal` everything is whispered. The command succeeds and its answer says what was changed |
| `redirchat:CHANNEL`, `rediremote:CHANNEL` | Text, or an emote, for channel 0 is said on that channel instead |
| `sendchannel`, `sendchannel_sec`, `sendchannel_except:CHANNEL` | Chat on other channels than 0 is refused, except on a channel that `sendchannel:CHANNEL` names; or on the one channel named. The bot's own replies to RLV queries always go out |
| `sendim`, `sendim_sec`, `sendimto:UUID` | `im` is refused, for everyone but those `sendim:UUID` names, or for the one avatar. [`group chat`](#groups) is refused too, unless an exception names the group (`sendim:GROUP`, by its UUID or its name without `,` and `;`) or is `sendim:allgroups`; `sendimto:GROUP` and `sendimto:allgroups` refuse it for that group or all |
| `startim`, `startimto:UUID` | `im` is refused to an avatar the bot has not exchanged an instant message with since the login |
| `recvim`, `recvim_sec`, `recvimfrom:UUID` | An instant message from an avatar, from everyone but those `recvim:UUID` names or from the one avatar, is hidden: it is recorded, and passed to the XMPP owners, as `(blocked by RLV: recvim)`, and the sender gets the answer RLV viewers send, "The Resident you messaged is currently prevented from reading your instant messages at the moment, please try again later.", in place of any [autoreply](#autoreplies). A message stored while the avatar was offline is hidden too, but not answered. Messages from objects are not hidden. A line of group chat is hidden the same way, without an answer, unless an exception names the group or is `recvim:allgroups`, or names the speaker (and no `recvimfrom:UUID` does); `recvimfrom:GROUP` and `recvimfrom:allgroups` hide a group's lines. Group notices are never hidden |
| `setgroup` | [`group activate`](#groups) is refused. The avatar's groups are still read, invitations accepted, joins made and groups left |
| `unsit` | While the avatar sits: `stand`, `sit` (which leaves the seat), `tp` and `accept tp` are refused |
| `sit` | `sit UUID` and `sit ground` are refused |
| `sittp`, `sittp:DISTANCE` | `sit UUID` is refused when the object is farther than 1.5 m, or the distance given, from the avatar |
| `interact` | `sit UUID` and every `touch` are refused |
| `tploc` | `tp` is refused |
| `tplocal`, `tplocal:DISTANCE` | `tp` inside the region the avatar is in is refused, unless every holder gives a distance and the destination is within the smallest |
| `tplure`, `tplure_sec` | Teleport offers are dropped as they arrive, also the answer to a `requesttp`, and one that waits cannot be accepted; `tplure:UUID` names avatars whose offers are still taken |
| `accepttp`, `accepttp:UUID` | The bot accepts teleport offers by itself, from anyone or from that avatar, whatever the `perms` rules say |
| `tprequest`, `tprequest_sec` | A request to be teleported to the avatar is dropped; `tprequest:UUID` names exceptions |
| `accepttprequest`, `accepttprequest:UUID` | The bot answers such a request with a teleport offer by itself |
| `touchall`, `touchworld`, `touchthis:UUID`, `touchattach`, `touchattachself`, `touchattachother`, `touchhud`, `fartouch`, `touchfar` | `touch` is refused according to what the object is: rezzed in the region, attached to the avatar or to another, on the HUD, or farther than 1.5 m (or the distance given). `touchworld:UUID` and `touchme`, held by the object itself, are exceptions |
| `detach`, held by an attachment | That attachment cannot be detached |
| `detach:POINT`, `remattach`, `remattach:POINT`, `addattach`, `addattach:POINT` | Nothing can be detached from, or attached to, that attachment point, or any |
| `addoutfit`, `addoutfit:KIND`, `remoutfit`, `remoutfit:KIND` | No clothing, or none of that kind, can be put on, or taken off |
| `acceptpermission`, `denypermission` | A script's request to attach or to take controls is granted by itself, or dropped, whatever the `perms` rules say |
| `showinv` | `inventory` is refused; items can still be named in other commands |
| `shownames`, `shownames_sec`, `shownametags` | [`scan avatars`](#scanning-for-avatars) writes `?` for the UUID and names of the avatars they hide, and [`scan`](#scanning-for-objects) for such an owner; `scan -o` is refused. `shownames:UUID` and `shownametags:UUID` are exceptions, each to its own restriction |
| `shownearby` | `scan avatars` is refused |
| `showminimap` | `scan avatars` writes `?` for every position |
| `camdrawmax:DISTANCE`, `camdrawalphamax:ALPHA` | `scan` lists nothing farther than the smallest `camdrawmax`, unless the largest `camdrawalphamax` held is below 0.99 |
| `permissive` | Every restriction counts only the exceptions of its own holder, as the `_sec` ones always do |

A few notes on these. An exception counts whoever holds it, except against
a `_sec` restriction and under `permissive`, where it has to come from the
holder of the restriction. `detach POINT` takes off what it may and
answers with an error naming what stayed on. `attach` without a body part
puts an object where it was last; when the inventory records no such
point and some point is closed, it is refused, since the bot cannot tell
where the simulator would put it. Distances are measured to the object's
own position, where a viewer measures to the spot that was clicked.

Under `sittp` and the touch restrictions the bot has to know how far an
object is and what it is, which it knows only of objects the simulator has
described to it; without `scan_distance` in the config that is few of
them. So when one of these restrictions has to be applied to an object the
bot does not know, it looks for the object first: it starts the scan of
[Touching objects](#touching-objects), if none is running in the region,
and gives the object until 10 seconds after the start of that scan. A
`sit` or `touch` command, and a forced `@sit`, wait for that and are then
decided by what the object turns out to be; `rlv cmd` answers a forced
`@sit` with `ok` at once. An object that is not described in that time,
because it does not exist or is beyond the scan's 32 m (or
`scan_distance`), counts as too far, and once the scan has had its 10
seconds such an object is decided at once. The bot's own sit on the seat
it holds is held back until the seat is described, which after an arrival
takes about a second, and has no time limit. Without such a restriction
nothing is looked for, and a `sit` goes out at once as it always has.
Nothing is sent to an avatar whose offer, request or message was dropped,
where a viewer would tell them.

All other restrictions of the API are kept and reported, and have no
effect: `rlv status` marks them `(no effect)`. They are about what a viewer
shows its user, or about things the bot does not do: `recvchat`,
`recvemote` and their variants, `showloc`, the world map, hovertext
and the other camera restrictions, `fly`, `edit`, `rez`, `share`, `sendgesture`,
`tplm`, `standtp`, `defaultwear`, `setdebug`, `setenv`, and
everything about the shared `#RLV` folder. Apart from what `recvim` hides,
the message log, `status`, `friends` and `profile` are yours and are never
censored; the name restrictions hide names from the two scans only. A restriction the API does not have,
and `@notify`, are not kept and are noted in the log as not supported.

Two things a viewer does for restrictions the bot does not: it does not
put a locked attachment back on when a script kicks it off, and it does
not take off again what a script attaches to a locked point.

RLV force commands are held to restrictions too: `@sit` by `sit`,
`interact`, `sittp` and a seated `unsit`; `@sitground` by `sit`; `@unsit`
by `unsit`; `@tpto` by `tploc` and a seated `unsit`; `@detach`,
`@remattach` and `@detachme` leave locked attachments on; `@remoutfit` by
`remoutfit`. **An object's own restrictions never stand in the way of its
own force commands**: a chair that holds `unsit` can still unseat the
avatar, and a cage that holds `sittp` can still seat it from afar.

#### Override

```sh
target/release/rslbot ctl rlv mode override
target/release/rslbot ctl stand              # carried out, whatever is held
```

`rlv mode override` exempts your next command from every restriction. RLV
is on again the moment that command arrives, whatever becomes of it: a
command that fails for a reason of its own, or that no restriction
concerns, uses the override up as well. Only `status`, `perms`,
`friends`, `profile`, `quit` and the `rlv` commands never do. (`scan`
used to be among them; now that restrictions apply to it, it uses the
override up like any other command.) For `requesttp` the exemption includes
accepting the one offer that answers it; for `persist` and `home` it
includes the bot's own way to the place they name, until the avatar is
there for the first time.

Override changes nothing else. What objects send is read as before, their
force commands and `rlv cmd` are held to the restrictions, offers are
dropped and accepted as before. It has no time limit: `rlv status` shows
`rlv: override` until a command has used it, or `rlv mode on` takes it
back. It cannot be had while RLV is off.

#### What the bot does by itself

A bot that [holds a place](#holding-a-place) is held to the restrictions
like you are: it does not teleport back while `tploc` is in force, does not
sit down again under `sit` or beyond `sittp`, and does not stand up under
`unsit`. It stays in holding mode and waits; a step that is held back is
not a failed attempt, so no delay grows. The log says once what waits and
for which restriction, and once when it no longer does:

```
rlv: holding back the teleport to "Target": tploc, held by Cage <8c40...>
rlv: no longer holding back the teleport to "Target"
```

`persist` and `home` are never refused: they change what the bot holds,
and its way there is held back or not as above. A login is never held
back. [Restoring attachments](#attachments) leaves out what an `attach`
would be refused for.

That is what restrictions are for, and it is often not what a bot is for.
After a region restart the avatar finds itself in a hub, still wearing a
collar that holds `tploc`, and the place it is to hold is the region whose
objects restrict it. The **persist override setting** takes a sort of step
out of the restrictions for good:

```sh
target/release/rslbot ctl rlv persist override tp       # or: sit, sit tp, none
```

or, to have it from the start, `rlv persist override sit tp` in the
[startup script](#startup-script).

| Word | The bot does by itself, whatever is held |
|---|---|
| `tp` | Teleport to the place it holds |
| `sit` | Sit on the seat it holds or on the ground, and stand up when what it holds is `stand` |

It is empty unless the startup script or a command says otherwise, a value
given by `rslbot ctl` is not saved, and `rlv status` shows it as `persist override:`.
It covers the bot's own steps only: your `tp` is still refused under
`tploc`, and so is a forced `@tpto`. The log names each restriction a step
went past. Do not confuse it with `rlv mode override`, which exempts one
command of yours, once.

#### When the bot is held

```sh
target/release/rslbot ctl rlv status          # what is held, and by whom
target/release/rslbot ctl rlv mode override   # one command through...
target/release/rslbot ctl home                # ...for instance back to the held place
target/release/rslbot ctl rlv reset           # forget every restriction
target/release/rslbot ctl rlv mode off        # or apply none, and keep them
```

`rlv reset` clears everything, in any mode, and tells the objects nothing:
one that sets a restriction afterwards holds it. `rlv mode off` keeps what
is stored, applies none of it, and reads no further commands; when RLV is
switched on again the stored restrictions are in force as they are, which
may no longer be what the objects think.

#### Forced movement and the place the bot holds

A bot that holds a place puts the avatar back there within seconds, which
would undo a forced stand or teleport at once. What happens instead is a
setting, `drop` or `follow`:

```sh
target/release/rslbot ctl rlv persist follow
target/release/rslbot ctl rlv persist drop
```

or, to have it from the start, as `rlv persist follow` in the
[startup script](#startup-script). Like the RLV switch, a setting given by
`rslbot ctl` is not saved.

With `drop`, the default, `@sit`, `@sitground`, `@unsit` and `@tpto`
switch holding off, as the `rslbot ctl` commands they stand for do. The
bot is in manual mode from then on and keeps what it held for `home`.

With `follow`, the result of the forced movement becomes what the bot
holds, whether or not it held anything before:

| Forced movement | The bot carries out | When |
|---|---|---|
| `@sit:UUID=force` | `persist cur UUID` | at once, in place of the `sit` |
| | `persist cur cur` | when the avatar sits on that object and its seated position is known, or 10 seconds after it sat down |
| `@sitground=force` | `persist cur ground` | when the ground sit has gone out |
| `@unsit=force` | `persist cur stand` | when the stand has gone out |
| `@tpto:...=force` | `persist cur stand` | when the avatar has arrived |

So a forced seat is held from the moment it is forced, with the place the
avatar was at: the bot sits there the way it sits on any held seat, tries
again with growing delays when the sit fails, and what it held before is
gone, also when the object does not exist or cannot be sat on. A
`persist` of something else (`persist cur stand`, `persist cfg cfg`) or
any `rslbot ctl` command that moves the avatar ends that; `home` does not,
since the forced seat is what it returns to. Once
the avatar sits, the seat's own position becomes the place the bot returns
to. A forced sit is refused, and changes nothing, when the avatar is not
in a region or the object is known to be in another region.

A forced teleport that fails leaves everything as it was: the bot goes
back to holding, or to manual mode, whichever it was in before. When the
operator gives a command that moves the avatar, or `persist` or `home`,
before a forced movement has been persisted, the operator's command
decides and nothing more is persisted for the forced one. `@detach`,
`@remattach`, `@remoutfit`, `@detachme` and the queries never change what
the bot holds.

With `follow`, an object the avatar owns decides where the bot returns to
after a kick or a region restart, for as long as the bot runs or until the
operator persists another place.

#### RLV commands by hand

`rlv cmd` carries out RLV commands as if an object had sent them, to try
what an object would get. The leading `@` may be left out, and the words of
the command are joined with single spaces:

```sh
target/release/rslbot ctl rlv cmd @versionnum=5
target/release/rslbot ctl rlv cmd @tpto:Blue Lagoon/128/128/25=force
target/release/rslbot ctl rlv cmd @sendim=n,getstatus=5
target/release/rslbot ctl rlv cmd @clear
```

It prints one line per command: the command, and the reply that was said
for a query, `ok` for a restriction and for a force command that was
carried out or begun, or the reason it was not. The first example prints
`versionnum=5: 10000` and says `10000` on channel 5 as well; the third
prints `sendim=n: ok` and `getstatus=5: /sendim`. The exit status is 0 when
every command was carried out and 1 otherwise. `rlv cmd` does not wait for
a sit, a teleport or a detach to end; that is in the log.

Like the RLV console of a viewer, `rlv cmd` is a holder of its own:
restrictions given with it are held by `operator`, apart from every
object's. `rlv cmd @clear` clears those and no object's, no object's
`@clear` touches them, and they last until they are lifted, `rlv reset`,
or the end of the bot process, across logins. Its force commands are held
to the restrictions of the objects and not to its own. It is refused while
RLV is off, and `@detachme` has no meaning in it.

### Permission requests

Avatars and scripted objects ask an avatar for things that a viewer
answers with a dialog. The bot reads three sorts of them, and each request
has one or more kinds:

| Kind | Request |
|---|---|
| `tp` | An avatar offers a teleport |
| `inventory` | An avatar or an object offers an inventory item or a folder |
| `anim` | A script asks for permission to animate the avatar (trigger or override animations) |
| `attach` | A script asks for permission to attach its object to the avatar |
| `controls` | A script asks for permission to take the avatar's controls |
| `camera` | A script asks for permission to track or control the camera |
| `group` | A member of a group invites the avatar into it |

A script may ask for several permissions at once, and such a request has
all their kinds. A script that asks for anything else (to take money from
the avatar, to teleport it, ...) is never granted anything, by itself or
on command; the log says which permission stood in the way.

With [RLV](#rlv) on, restrictions come before all of this: `tplure` drops
teleport offers, `denypermission` drops a script's request to attach or to
take controls, `accepttp` and `acceptpermission` have the bot accept what
the rules below would leave waiting, and a seated avatar under `unsit`
goes nowhere. The rules themselves are not changed by any of it. See
[Restrictions](#restrictions).

Without `perms` commands the bot accepts nothing by itself, except a
teleport offer it asked for with `requesttp`. Every request is written to
the log and waits. `perms autoaccept` and `perms revoke`, in the
[startup script](#startup-script) or through `rslbot ctl`, say what is
accepted without you:

```toml
startup = '''
perms autoaccept tp "Jane Doe"
perms autoaccept inventory "Owner Resident" 11111111-2222-3333-4444-555555555555
perms autoaccept anim all
perms revoke anim 01234567-9abc-defe-dcba-987654321000
perms autoaccept attach all
'''
```

`perms autoaccept KIND` takes `all`, or avatars and objects, which are
added to the kind's list. `perms revoke KIND` takes them off the list
again, `all` for the whole list; for a kind that accepts `all` it names the
exceptions: avatars and objects that are never accepted by themselves.

An entry is an avatar's name (`"First Last"`, `first.last` or a single
word) or a UUID:

| Request from | A name matches | A UUID matches |
|---|---|---|
| An avatar (teleport offer, inventory offer) | that avatar | that avatar |
| An object that gives inventory | its owner | its owner |
| An object whose script asks for permissions | its owner | the object, or its owner |
| A member who invites into a group | the inviter | the group, or the inviter |

So an avatar's name or UUID stands for the avatar and for everything it
owns, and an object's UUID for that object. Names are compared, in any
letter case, with the name the grid gives with the request; they are not
looked up, so a misspelt name simply matches nobody. An avatar that
changes its name keeps its UUID.

Two things follow from what the grid tells the bot:

* An inventory offer from an object does not say which object it came
  from, only who owns it and what the object is called. An object's UUID
  under `inventory` therefore never matches; name the owner.
* A script's request names the owner as text, and for an object deeded to
  a group that text is the group's name. Before the bot decides, it asks
  the simulator who owns the object, and only an avatar counts: a group
  that calls itself like an avatar you trust gets nothing. An object owned
  by a group is matched by the object's own UUID, or by `"all"`. When the
  simulator does not answer within 10 seconds the same holds.

A request with several kinds is accepted by itself only when every one of
them is auto-accepted for its sender, and then whole. The bot never grants
part of what a script asked for: a dance ball that asks to animate the
avatar and to take its controls needs `anim` and `controls`.

What accepting does:

* A teleport offer: the bot goes, and is in manual mode from then on (see
  [Teleport offers](#teleport-offers)).
* An inventory offer: the giver is told it was accepted, and the item is
  filed in the folder for its type (Objects, Notecards, Scripts, ...) at
  the top of My Inventory, as a viewer does. The grids put the item into
  the inventory when it is offered, so one that is never accepted is there
  all the same; accepting is the answer to the giver. The bot reads the
  top level of the inventory for this, so a simulator without an inventory
  service leaves the offer waiting.
* A script's request: the script is granted every permission it asked for.

Neither of the last two moves the avatar or changes what the bot holds.

#### Invitations to groups

An invitation names its group by UUID and its inviter by name only (on
Second Life the username, such as `annanovykova99`). Before
the rules decide it, the bot asks the grid for the group's profile, for
its name, and looks the inviter's name up for the UUID; it waits for both
for up to 10 seconds and then decides with what it has. A group's name
matches no entry: enter a group in a rule by its UUID. An invitation into
a group that costs L$ to join is never accepted by itself, also under
`all`; it waits, and the log gives the command with `--pay`:

```
invitation from Jane Doe <uuid> into "Builders" <uuid>, which costs L$250 to join: waits, as joining costs L$250; `rslbot ctl accept group <uuid> --pay 250` accepts it
```

Accepting answers the invitation as a viewer's Join does; the bot takes
it as accepted when the grid lists the group. No RLV restriction applies
to an invitation. The latest invitation into each group waits, at most
100 of them; one is forgotten when a newer one into its group comes, when
it is answered, when the avatar joins the group another way, and with the
session.

```sh
rslbot ctl group invitations                 # group, fee, inviter's UUID or ?, inviter, group's name or ?
rslbot ctl accept group Builders             # by the group's name or UUID,
rslbot ctl accept group "Jane Doe"           # or by the inviter
rslbot ctl accept group Builders --pay 250   # one with a fee
rslbot ctl decline group Builders
```

Unlike the other kinds, `accept group` and `decline group` take the one
invitation that `WHO` names: when it names two, or `WHO` is left out while
two wait, the command answers nothing and lists them with their groups'
UUIDs. An acceptance joins a group and may cost money, so the latest is
not good enough. `accept group` is answered when the grid lists the group
(within 15 seconds); a fee is paid as for `group join`, and one the bot
refuses to pay leaves the invitation waiting. `decline` takes `group`
only.

#### Requests that wait

A request that was not accepted by itself is kept until the session ends:
the latest teleport offer of each avatar, every inventory offer, and the
latest request of each script, which is also forgotten when the script
asks again or the avatar leaves the region. The log names the command
that accepts it:

```
permission request (anim, controls) from Dance Ball <uuid> of Jane Doe <uuid>: waits, as controls is not auto-accepted for it; `rslbot ctl accept anim` accepts it
```

```sh
rslbot ctl accept anim                    # the latest request that asks for anim
rslbot ctl accept anim "Jane Doe"         # the latest from an object of Jane Doe's
rslbot ctl accept attach 01234567-9abc-defe-dcba-987654321000   # from that object
rslbot ctl accept inventory               # the latest inventory offer
rslbot ctl accept tp                      # the latest teleport offer
```

`WHO` is one argument, and matches as an entry does. A script's request
is found under each of its kinds, and accepting it grants everything it
asked for; the answer says what that was. There is no command to refuse a
request: one that is left alone ends with the session.

#### Changing the rules while the bot runs

```sh
rslbot ctl perms autoaccept anim all
rslbot ctl perms autoaccept inventory 01234567-9abc-defe-dcba-987654321000 Avatar1
rslbot ctl perms revoke inventory all
rslbot ctl perms revoke anim 01234567-9abc-defe-dcba-987654321000
rslbot ctl perms status
```

`perms autoaccept KIND` adds avatars and objects to a kind's list, or with
`all` accepts everyone; an entry that was an exception stops being one.
`perms revoke KIND` takes entries off the list, or with `all` empties it.
While a kind accepts everyone, revoking an entry makes it an exception.
Each is answered with one line per entry. An entry that was in that state
already gets a line starting with `warning:`, and the command still
succeeds:

```
$ rslbot ctl perms autoaccept inventory Avatar1
warning: Avatar1 was auto-accepted for inventory already
```

An exception to a list (one object of an avatar that is listed) can be
written in the config only. `perms status` prints one line per kind, and
then the requests that wait, oldest first:

```
tp: Jane Doe
inventory: none
anim: all except 01234567-9abc-defe-dcba-987654321000
attach: none
controls: none
camera: none
group: none
waiting: tp from John Roe <uuid>
```

These commands work whether or not the avatar is connected. Like `rlv
mode`, they are not saved: a restarted bot starts from its config and the
`perms` commands in its startup script. A
change of the rules does not accept what waits already, and it does not
take back what a script was granted.

### Autoreplies

With an autoreply text the bot answers an avatar that sends it an instant
message, and then leaves that avatar alone for the pause. In the
[startup script](#startup-script):

```toml
startup = '''
autoreply message Hello, I'm bot. I won't respond to you in next 2 hours
autoreply pause 2h
'''
```

The pause is a whole number and a unit for each of `d`, `h`, `m` and `s`
that is needed, in that order and without spaces: `90s`, `30m`, `2h`,
`1h30m`, at most 365 days. `0` answers every message. Without `autoreply
pause` it is one hour. A text of several lines goes in double quotes.

* Only instant messages from avatars are answered. Objects, the grid,
  group chats, and messages stored while the avatar was offline get no
  answer.
* An avatar's pause starts when the bot answers it, and when the bot sends
  it an instant message on command: `rslbot ctl im`, or an owner's message
  through the [XMPP gateway](#xmpp-gateway). An avatar you are talking to
  is not told the bot will not answer. Messages from the avatar during its
  pause do not make the pause longer.
* The answer goes out the way a viewer's Do Not Disturb autoresponse does:
  viewers show it as automatic, and neither viewers nor this bot answer
  one, so two bots cannot answer each other forever.
* It is recorded as kind `im_auto`, see [Chat and IM logs](#chat-and-im-logs),
  and not passed to the XMPP owners.
* With RLV on, `sendim` and `sendimto` hold an answer back; the log says
  so, and the avatar's pause does not start. A message `recvim` hides gets
  the RLV viewers' answer instead, see [Restrictions](#restrictions).

While the bot runs:

```sh
rslbot ctl autoreply message "New autoreply from the bot"
rslbot ctl autoreply pause 30m
rslbot ctl autoreply refresh    # forget every avatar's pause
rslbot ctl autoreply off
rslbot ctl autoreply on         # with the text it had
rslbot ctl autoreply status
```

`autoreply message` also switches autoreply on; `on` is refused while
there is no text. A new pause applies at once, also to the avatars already
answered. `status` prints:

```
autoreply: on
message: New autoreply from the bot
pause: 30m
avatars in their pause: 2
```

These commands work whether or not the avatar is connected, no RLV
restriction refuses them, and they are not saved: a restarted bot starts
from its config and the `autoreply` commands in its startup script. The
pauses are kept across logins until the bot stops.

### Payments

The bot can pay avatars and objects in L$, as a viewer's Pay does, and say
what an object offers to be paid. Nothing is paid without a `[payments]`
table in the config:

```toml
[logs]
transactions = "transactions.tsv"

[payments]
limit = 1000
limit_renew = "7d"
perms = ["ctl", "web"]
whitelist = ["Jane Doe", "11111111-2222-3333-4444-555555555555"]
blacklist = ["john.roe"]
unlimited_list = ["Owner Resident"]
```

| Key of `[payments]` | Required | Meaning |
|---|---|---|
| `limit` | yes | What the bot may pay in one period: a whole number of L$ from 0 to 2147483647, or `"unlimited"`. A number needs `transactions` under `[logs]` |
| `perms` | yes | The interfaces that may pay: any of `"ctl"` (`rslbot ctl` and the startup script), `"xmpp"` (the [XMPP gateway](#xmpp-gateway)'s control contact) and `"web"` (the [web interface](#web-interface) and its console). `[]` lets none pay |
| `limit_renew` | no | The length of a period: `7d`, `12h`, `1d12h`, up to 365 days. Without it the limit never renews |
| `whitelist` | no | With it, only who is on it, or on `unlimited_list`, can be paid. `[]` lets only `unlimited_list` be paid |
| `blacklist` | no | Who is on it is never paid, whatever the other lists say |
| `unlimited_list` | no | A payment to who is on it does not count against the limit, and goes through also when nothing is left |

The **limit** is what the bot may pay in one period. A period starts with
the first payment that counts while none runs, and ends `limit_renew`
later: then the whole limit is back, whatever was paid in between. A
payment counts from the moment it is sent, and stops counting only when
the grid reports that it failed. The bot keeps what was paid in the
[transactions file](#transactions) and reads it back when it starts, so
stopping and starting the bot does not renew the limit; this is why a
`limit` that is a number needs that file. The limit is against the
interfaces, not against whoever can edit the config or the file.

An entry of the **lists** is an avatar's UUID or login name (`First Last`,
`first.last`, or one word for a `Resident`), or an object's UUID. A name is
compared with the account's name the grid gives, never with a display
name, which anybody can set to anything. An avatar on a list stands for
the objects it owns too; an object a group owns is matched by its own UUID
only. Login names change only by the grid's hand, but a UUID is the stable
form. When a list holds a name and the grid does not say within 5 seconds
whose the payee or the object is, the payment is refused.

```sh
rslbot ctl pay 25 avatar jane.doe          # an avatar, by name or UUID
rslbot ctl pay 25 avatar Jane Doe          # the words of a name may be separate
rslbot ctl pay 25 avatar Jane Doe -d for the hat   # with a description
rslbot ctl prices <uuid>                   # what an object offers
rslbot ctl pay 250 object <uuid>           # pay a vendor one of its prices
rslbot ctl scan --pay                      # what takes money nearby
rslbot ctl payees                          # who the lists name
```

`pay AMOUNT avatar AVATAR` pays an avatar, found as `im` finds one; the
bot's own avatar is not a payee. `pay AMOUNT object UUID` pays an object
in the region, as a viewer's Pay on it does: the object's script is told
that the bot's avatar paid it. The object has to be one the bot can find,
as for `touch`, and one that takes money: the simulator marks it (or the
root prim of its linkset) so, and `scan` shows it with the flag `pay`. It
takes only an amount it offers: one of its quick-pay amounts, or any
amount when it has a box for one. `AMOUNT` is a whole number of L$, at
least 1.

`-d DESCRIPTION...` (or `--desc`) after the avatar sends a description
with the payment, as a viewer's Pay Resident dialog does: everything after
it, the words joined by single spaces, one line of at most 127 bytes. The
payee sees it with the payment. It is for avatars only: for an object,
that field carries the object's name. The payee reads it as a message, so
while the [RLV restrictions](#restrictions) hold back an instant message to
that avatar (`sendim`, `sendimto`, `startim` for a new conversation), the
payment is made without the description, and the answer begins with
`warning: Payment description is blocked by RLV`. Nothing else of RLV
applies to payments.

Before it sends a payment the bot checks, in this order: the object's
amounts, the lists, the balance (it does not pay more than the avatar has,
as a viewer does not), the record of the limit, and the limit. One payment
waits for the grid at a time: a second `pay` meanwhile is refused.

`pay` is answered when the grid reports the payment: with the amount, the
payee and the new balance, or with the grid's reason when it failed. When
the grid says nothing within 15 seconds, or the session ends first, the
answer is an error saying that the payment was sent and is **not
confirmed**, and it counts as paid. Do not simply pay again: look at the
balance in `status` first, which shows whether the L$ left. The bot never
pays anything again by itself. A grid that reports nothing about payments,
such as OpenSim with its stock money module, makes every payment end
unconfirmed.

`prices UUID` prints what an object offers, for example:

```
object: Hat Vendor
owner: Jane Doe
prices: 100, 250
other amounts: no
```

`owner` is the owner's login name, its UUID when the grid does not name it
within 5 seconds, or `group` and the group's UUID. `other amounts` is
`yes`, `yes, suggested 50` or `no`.

`payees` lists the avatars of `whitelist` and `unlimited_list`, one line
each, tab-separated: the UUID (`?` for an entry by name), the login name,
and the lists (`whitelist`, `unlimited` or `whitelist,unlimited`). Objects
and UUIDs the grid does not know as avatars are left out.

Every `pay` is written to the bot's log with the interface it came through,
the amount, the payee, and what became of it. When the bot starts with a
`[payments]` table it logs who may pay, the limit and what is left, and
warns when `web` may pay and the `[web]` table has no `password`.

None of the three commands moves the avatar or touches the place the bot
holds, no RLV restriction applies to them, and `rlv mode override` is not
used up by them.

#### Fees for joining a group

The fee of a group, paid with [`group join --pay`](#groups) or
[`accept group --pay`](#invitations-to-groups), is a payment of the fee
to the group: the interface has to be in `perms`, the lists are matched
against the group's UUID only (neither its name nor the inviter's matches
an entry), the balance and the limit are checked, and it is the one
payment that may be under way. The grid takes the fee when it gets the
join or the acceptance; the bot sends no payment of its own. The fee
counts against the limit from the moment the join is sent. The bot then
waits up to 15 seconds for two things: the grid's word that the avatar is
in the group, which answers the command, and the grid's report of the
fee, which settles the transaction:

| Within 15 seconds the grid | The fee | The command |
|---|---|---|
| reports the fee and lists the group | `paid`, counts | succeeds, with the new balance when the report came first |
| lists the group, reports no fee | `unconfirmed`, counts | succeeds, saying the fee was not reported and counts as paid |
| reports the fee, does not list the group | `paid`, counts | fails when the time is up, saying the fee was paid |
| refuses the join, or reports the fee as failed | `failed`, stops counting | fails with the grid's reason |
| says nothing, or the session is lost | `unconfirmed`, counts | fails, saying the join was sent and the fee counts as paid |

The record has the group's UUID as `party_uuid`, type 1004, and the grid's
text (on Second Life `You paid <group> L$<fee>.`) or `Fee to join` and the
group's name. Second Life reports a fee twice, with one transaction id;
as a viewer shows such a report once, the bot records any transaction
whose id it saw among the last 30 once. Should the fee change between
the profile and the join, the grid takes what it takes: the bot's record
says `unconfirmed` for the fee it knew and the grid's report is recorded
as `spent`.

Not there: paying a group other than its fee, buying objects, land or
passes, a description with a payment to an object, letting a script take money from the
avatar (always refused), the transaction history of the Second Life web
site, transactions over XMPP, and changing the limit or the lists while
the bot runs.

### Exit status and limits

| Exit status of `rslbot ctl` | Meaning |
|---|---|
| 0 | Done: the message was handed to the simulator, the avatar arrived, sat, and so on. For `im`, `requesttp` and `sendtp` this does not mean the other avatar is online, has read it or agreed |
| 1 | The bot refused or the action failed, and the reason is printed: not in a region right now, unknown avatar or region, no answer to a name lookup within 10 s, a sit not confirmed within 15 s, a teleport without outcome after 60 s, an unknown object, a seat known to be in another region than the location to persist, an inventory item or folder that was not found or is of the wrong type, an inventory that did not answer within 50 s, an attach or detach the simulator did not show within 15 s, clothing on a grid without the outfit service, a bad argument, an unknown command. Or no bot is listening on the socket |
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

## XMPP gateway

The bot can be an XMPP (Jabber) gateway: it connects to an XMPP server on
the same host as an external component (XEP-0114) and owns a domain there,
such as `russel.sl.example.org`. The owners' XMPP accounts then use the
avatar from any XMPP client, on any device: every avatar it exchanges
instant messages with is a contact, local chat is a contact, and commands
go to the domain itself. The XMPP server keeps the history and the offline
messages and pushes to phones, as for any other contact. Several bots, on
one grid or several, are several components with domains of their own on
the same server, and can share one owner account.

### Setting it up

1. Add the component to the XMPP server. For Prosody:

   ```lua
   component_interfaces = { "127.0.0.1" }
   Component "russel.sl.example.org"
       component_secret = "a long random secret"
   ```

   For ejabberd, a `ejabberd_service` listener does the same:

   ```yaml
   listen:
     -
       port: 5347
       ip: "127.0.0.1"
       module: ejabberd_service
       hosts:
         "russel.sl.example.org":
           password: "a long random secret"
   ```

   The component connection is plain TCP authenticated by the shared
   secret, without encryption: keep the component port on `127.0.0.1`. The
   bot refuses a `server` that is not on this host.

2. Add the `[xmpp]` table at the end of the bot's config:

   ```toml
   [xmpp]
   component = "russel.sl.example.org"
   secret = "a long random secret"
   owners = ["me@example.org"]
   ```

   | Key of `[xmpp]` | Required | Meaning |
   |---|---|---|
   | `component` | yes | The domain the component owns; letter case does not matter |
   | `server` | no | `host:port` of the server's component port; the host must be `localhost` or a loopback address. Default: `127.0.0.1:5347` |
   | `secret` | yes | The server's secret for the component. It never appears in the bot's log or errors |
   | `owners` | yes | Bare addresses (`user@domain`, no resource) of the accounts that may use the gateway |

3. Restart the bot, and add `russel.sl.example.org` and
   `local@russel.sl.example.org` to the owner's contacts. Messages arrive
   without that too, but most clients show presence, and some show
   messages, only for contacts on the list.

The component connects when the bot starts, before the first login, and
stays connected whether or not the avatar is on a grid. When the server is
not there, refuses the secret or drops the connection, the bot logs why and
tries again after 5 seconds, doubling the delay up to 5 minutes, and back
to 5 seconds after a connection the server accepted. None of this ever
holds up the grid session.

### Addresses

| Address | What it is |
|---|---|
| `russel.sl.example.org` | The bot: commands, and the bot's state as presence |
| `local@russel.sl.example.org` | Local chat |
| `jane.doe@russel.sl.example.org` | The avatar Jane Doe; `bob@...` for `bob Resident` |
| `<uuid>@russel.sl.example.org` | An avatar by UUID: one whose name is not known, or would give one of the other addresses (`local Resident`, say) |
| `obj-<uuid>@russel.sl.example.org` | An object that sent an IM |

An avatar's address is made from its legacy name in lower case, `first` or
`first.last`, with the characters an address cannot hold escaped as
XEP-0106 says: a hypergrid visitor `Ann.Lee @osgrid.org:8002` is
`ann.lee.\40osgrid.org\3a8002@...`.

**Instant messages.** An IM to the avatar comes from the sender's address,
with the display name as nickname, as soon as the bot records it (see
[Chat and IM logs](#chat-and-im-logs)). Offline IMs fetched at login carry
the time they were sent. A message to an avatar's address is sent to it as
`im` sends it, and the reply goes to the UUID the address came from, so
avatars the grid's search does not know, such as hypergrid visitors, can be
answered. To start a conversation, write to `first.last@...`: the name is
looked up as `im` looks it up. A refused or failed IM comes back as an
error with the reason, which most clients show on the message. IMs from
objects come from `obj-<uuid>@...` with the object's name as nickname and
cannot be answered; an IM from the grid itself comes from the domain.

**Local chat.** Every line the avatar hears arrives from `local@...` as
`Name: text`: the display name with the legacy name in parentheses,
` shouts` or ` whispers` after the name, `[owner]` and the like for other
kinds (`Collar [owner]: ready`), and `/me` emotes as `Name waves`. A
message to `local@...` is said as `say` says it, so `/shout`, `/whisper`,
`/7 text` and `//` work as in a viewer's chat bar. The avatar's own lines
come back as the simulator delivers them.

**Commands.** A message to the domain is one command line, read as
`rslbot ctl` reads its arguments: `status`, `tp 128 128 25 Home`,
`im "Jane Doe" see you later`. An argument with spaces goes in double
quotes; inside them `\"` is a double quote and `\\` a backslash. Every
command is answered with one message: its text, `ok`, or `error: ` and the
reason. `help` lists the commands. A command that takes time, such as a
teleport, is answered when it ends and does not hold up the other
contacts; at most 16 commands wait for their answers at once. Commands
here are those of the interface `xmpp`: `status` has its `balance` and
`limit` lines, and `pay`, `prices` and `payees` are carried out, only when
`perms` in `[payments]` lists `xmpp` (see [Payments](#payments)).

**Presence.** The domain is online with the region's name as status while
the avatar is in a region, away with `not in a region` otherwise, and
offline once the bot stops; `local@...` is online while the avatar is in a
region. Avatars' addresses always show as offline.

**Who may use it.** Only the `owners`, from any of their devices. Anyone
else gets an error for every message, a refusal for every subscription,
and a line in the bot's log. Service discovery and ping are answered to
anyone.

**Encryption.** The gateway cannot take OMEMO-encrypted messages: it is
not an end of the conversation that holds keys, so its addresses publish
none. Clients that turn OMEMO on by themselves must be told to send
unencrypted to these addresses. Dino starts every new conversation with
OMEMO and then refuses to send; switch the conversation to "Unencrypted"
with the lock icon in the message box (or make that Dino's default). The
messages travel between the owner's client and the server over TLS as
usual, and between the server and the bot over the loopback interface.

Messages for the owners that come while the component is not connected
wait, the newest 500 of them, and are delivered with their time once it is
connected again; the log says how many were dropped.

Not supported yet: the friends list and friends' online status, a group
chat room for local chat or for groups, the avatars nearby, profile
pictures, ad-hoc command forms, delivery receipts, IMs sent with
`rslbot ctl` in the XMPP conversation, TLS on the component connection,
and a server on another host.

## Web interface

The bot can serve a web page to use it from: the status of the avatar,
teleports, scans of the avatars and objects around it with buttons to go
there, sit and touch, local chat and instant messages as they happen, the
inventory as a tree, and a console for everything else. The page is part
of the `rslbot` binary and loads nothing from anywhere else; there is
nothing to install beside the bot, and it works on a phone.

Everything the page does is one of the commands of the
[control socket](#control-socket), carried out the same way: the same
answers, the same RLV restrictions, the same lines in the log. The page can
do nothing `rslbot ctl` cannot.

### Setting it up

```toml
[web]
listen = "127.0.0.1:8080"
password = "choose one"
```

`listen` is the address the page is served at, `host:port`, here
`http://127.0.0.1:8080/`. The host is `localhost` or an IP address; several
bots on one machine need a port each. The bot starts listening before its
first login, and an address it cannot listen on, such as one another
program has, is a config error. Like every table, `[web]` comes after the
plain keys of the config.

`password` is optional. With it, the page shows a login box and nothing
else, and the bot carries out nothing and shows nothing, until the password
is given. A browser that gave it stays logged in until you log out in the
Settings tab, for 30 days without use at most, and until the bot is
restarted. After five wrong passwords within a minute every login is
refused for a minute. Without `password`, whoever can reach the address
controls the avatar: that is meant for `127.0.0.1` on a machine of your
own, and the bot says so in its log when it is asked to listen elsewhere
without one.

**The connection is not encrypted.** Keep `listen` on `127.0.0.1`. To use
the page from another machine, either forward the port over SSH
(`ssh -L 8080:127.0.0.1:8080 yourserver`, then open
`http://127.0.0.1:8080/` at home), or put a web server with TLS in front
of it and set a password:

```nginx
server {
    listen 443 ssl;
    server_name bot.example.org;
    # ssl_certificate ...; ssl_certificate_key ...;
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_buffering off;          # the page's live updates are one long answer
        proxy_read_timeout 1h;
    }
}
```

```
bot.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

The first is for nginx, the second a whole Caddyfile.

Two things keep other web sites away from the bot while your browser has
them open. A request that changes anything is carried out only when the
page itself sent it, which a form or a script on another site cannot
imitate. And without a password the bot answers only when it is opened by
IP address or as `localhost`, never under a host name, since a name can be
made to point at your machine by whoever owns it; behind a proxy that
passes your own host name on, as Caddy does, set a password. Both
refusals are logged with the address they came from, which behind a proxy
is the proxy's.

### The tabs

**World.** The status: the avatar's name, region, position, what it sits on, and
whether the bot holds a place or leaves the avatar where commands put it
(see [Holding a place](#holding-a-place)). It follows the bot by itself. A
teleport form with a box each for X, Y, Z and the region; an empty Z keeps
the height and an empty region stays in this one. "Stand up". "Scan
avatars" lists everyone in the region, or within the distance you give,
with a button to teleport to each and one to open an IM. "Scan objects"
lists the objects around the avatar, within a distance and with a part of
the name if you like, each with "Teleport", "Sit" and "Touch", and "Pay"
for an object that takes money (see the Money tab). A list stays as it is
until you scan again, and says when it was made.

**Local chat.** The same status, the avatars within chat range (kept up to
date every 15 seconds while you look at the tab), and local chat as it is
heard, one line each as the [XMPP gateway](#xmpp-gateway) writes them.
What you type is said as a viewer's chat bar would say it: `/shout`,
`/whisper`, `/7 text` and `//` work. The avatar's own lines appear when
the simulator delivers them back.

**IM.** One conversation for each avatar and each object the bot has
exchanged instant messages with, the newest first, with a count of the
messages you have not seen. Messages the bot sent, from here or with
`rslbot ctl im` or by its [autoreply](#autoreplies), are in the
conversation too. To write to someone new, type a name (`Jane Doe`,
`jane.doe`, one word) or a UUID into the box and press "Open"; the name is
looked up when the first message is sent. The friend list shows who is
online, and a click on a friend opens the conversation. Objects cannot be
answered. A conversation with an avatar has "Pay", which shows the Money
tab with that avatar as the payee.

**Groups.** On top, the invitations that wait, each with its group, the
inviter, the fee, "Accept" and "Reject"; one with a fee is confirmed in
the dialog of the Money tab first, and has no "Accept" while the web
interface may not pay. Below, the avatar's groups, the active one marked,
each with the number of messages you have not seen, and the chosen
group's chat and notices in one list. A notice stands out on its own
background, with its subject, its sender and the name of what is attached
to it, which the page cannot take. The box says a line in the group;
"Activate", "Invite" (for a group you may invite into: a name, a UUID or a
friend), "Info" and "Leave" do what `group activate`, `group invite`,
`group info` and `group leave` do; "Leave" asks first, in a dialog that
names the group, with "Cancel" chosen. A group you have messages of and are no longer in stays in the
list, set apart. The tab's label is marked while an invitation waits or a
group has new messages.

**Inventory.** My Inventory as a tree. A folder is read when you open it,
so a large inventory costs nothing until you look into it. An object has
"Attach", and once attached "Detach" and "Touch"; clothing has "Wear" and
"Take off", a body part "Wear". "Attached only" shows what the avatar
wears now, in place of the tree.

**Money.** There only while the web interface may pay: with a
`[payments]` table whose `perms` lists `web` (see [Payments](#payments));
otherwise the page has no Money tab and no "Pay" button anywhere. At the
top: the avatar, its "Balance", and what is "Left of the limit", with when
it renews, in your time zone. "Send money" takes a "Payee", a name or a
UUID as `pay` takes one, an "Amount L$", and an optional "Description
(optional)" sent with the payment; while RLV holds `sendim` the form says
"Payment description is blocked by RLV" in its place. Beside it, the "Friends" and
the avatars of the config's "Whitelist" and "Unlimited list" (what
`payees` lists); choosing one makes it the payee, paid by its UUID, or by
name for a list entry given by name. Below, the "Transactions": the ones
recorded since the bot started and the newest 500 of the [transactions
file](#transactions), newest first, a failed or unconfirmed payment set
apart and saying so.

"Pay" on a row of the object scan asks the object what it offers and
shows its amounts in a list, with "Another amount" and a box when it takes
other amounts, and "Pay the object". Every payment a button sends is
confirmed first in a dialog that names the amount and the payee, for an
object its name and its owner: nothing is sent until "Pay L$..." is
pressed, and "Cancel", Escape or a click beside the dialog send nothing.
`pay` typed into the Console is sent at once, as every typed command is.
Without a `password` in `[web]`, whoever reaches the page can pay, within
the limit; the bot warns about that when it starts.

**Console.** A command line that takes what `rslbot ctl` takes, without
`rslbot ctl` in front and with double quotes around an argument that holds
spaces, as the XMPP control contact does: `status`, `scan 20 -n bench`,
`im "Jane Doe" see you later`, `help`. The page and its console are the
interface `web`: their `status` has the `balance` and `limit` lines, and
`pay`, `prices` and `payees` are carried out, only when `perms` in
`[payments]` lists `web`. The arrow keys bring back earlier
lines. Everything the tabs have no button for is here: `persist` and
`home`, `accept` and `perms`, `rlv`, `autoreply`, `anim` and `sound`,
`profile`, `quit`.

**Settings.** The theme of the page: as the browser asks for (the
default), light, or dark. The choice is kept in that browser. With a
password, the button to log out.

What a button did, or why it could not, is shown under the tabs on every
tab; an error stays until you dismiss it.

### What the page keeps

The bot keeps the newest 500 lines of local chat, the newest 500
instant messages and the newest 500 group messages in memory, from the moment it starts, so that a page you
open later, or open again, has them. A page that lost its connection gets
what it missed when it is back, and says meanwhile that it cannot reach
the bot. Nothing of this is written to disk for the page: after a restart
of the bot the IM tab is empty. `log_chat` and `log_im` are the record
that lasts (see [Chat and IM logs](#chat-and-im-logs)).

The lists the page keeps up to date by itself, the avatars in chat range,
the friends, and the groups and invitations of the Groups tab, are
ordinary `scan avatars chat`, `friends`, `group list` and `group
invitations` commands, with two differences: they get no line in the bot's log, and they never
use up an RLV [override](#override), which stays for the next command you
give yourself. Their "Refresh" buttons are commands of yours like any
other.

At most 32 connections are served at once, 8 of them open pages; a ninth
page says that the bot cannot be reached until one of the others is
closed.

Not there yet: a panel for the teleport offers and permission requests
that wait (the console has `perms status` and `accept`; invitations to
groups are on the Groups tab), conferences,
pictures, a map, history from before the bot started, several accounts or
read-only access, browser notifications, and TLS in the bot itself.
