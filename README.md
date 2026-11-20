RusselBot is a lightweight bot software for Second Life and OpenSimulator written in Rust,
replacement for heavy .NET/Mono bots. Requires just a few MBs of memory which makes it possible
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
names, and wait for a command otherwise.

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
object), `persist` decides whether the bot holds a placement from then on,
and which:

* Without `persist`, or with `persist = "off"`: none. The bot goes to
  manual mode and leaves the avatar alone; it only keeps it logged in. An
  avatar that is unseated or moved away stays that way, and after a kick
  the bot logs in at the avatar's last location.
* `persist = "cfg"`: `position` and `sit` exactly as the config has them.
* `persist = "cur"`: the position and the seat the simulator reported at
  that moment. The position is where the avatar really is, on its seat
  when it sits, and the seat is the prim the simulator sat it on, which in
  a linkset may not be the one `sit` names.

A bot that holds a placement keeps working towards "in the region it
holds" and "seated as it holds": it logs in there after a kick, teleports
back when the avatar is moved to another region, and sits again when
unseated, each with growing delays between attempts. Inside the region it
holds, it does not put the avatar back where it was: it reacts to a change
of region, and to being unseated from a held object. `rslbot ctl persist`
sets or changes the held placement while the bot runs, see
[Holding a place](#holding-a-place). Nothing of it is saved: a restarted
bot starts from the config again.

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
| `region` | no, but only with `position` | Region the avatar starts out in; the bot logs in there, and returns there when `persist` says to hold the configured place |
| `position` | no, but only with `region` | `[x, y, z]` inside the region; pick a spot close to the object to sit on |
| `start` | no, and only without `region` and `position` | Where the first login puts the avatar: `"home"` (its home location) or `"latest"` (where it was last). Default: `"latest"` |
| `sit` | no | What to sit on at the start: the UUID of an object, or `"ground"`. Default: the avatar stands, and the bot does not sit it |
| `object` | no | Deprecated spelling of `sit`, for a UUID only; the bot logs a warning. Not together with `sit` |
| `persist` | no | What the bot holds once the avatar is at the place above, if anything: `"off"` or `false`, `"cfg"`, `"cur"`, an array, or a table; see below. Default: `"off"` |
| `log_chat` | no | File that receives local chat, see [Chat and IM logs](#chat-and-im-logs) |
| `log_im` | no | File that receives instant messages |
| `fetch_offline_im` | no | `true` to also fetch IMs stored while the avatar was offline. Default: `false` |
| `socket` | no | `true` to listen on the control socket, see [Control socket](#control-socket). Default: `false` |
| `requesttp_timeout` | no | Minutes after `requesttp` during which the asked avatar's teleport offer is accepted. Default: `10` |
| `scan_distance` | no | Metres (1 to 1024) within which objects are learned from the start, for `touch`; see [Touching objects](#touching-objects). Default: learn within 32 m when `touch` is first used |
| `restore_attachments` | no | `false` to keep the bot from attaching again, after each login, what the Current Outfit folder lists and the simulator did not bring; see [Attachments](#attachments). Default: `true` |
| `perms` | no | Two tables, `[perms.autoaccept]` and `[perms.except]`, that say which teleport offers, inventory offers and scripts' permission requests the bot accepts by itself, see [Permission requests](#permission-requests). Default: none |
| `rlv` | no | `true` to start with RLV on: the bot then reads the `@` commands of scripted objects the avatar owns, see [RLV](#rlv). Default: `false` |

`persist` takes these forms:

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
* Inventory offers and scripts' requests for permissions, which the bot
  used to drop unread, are written to the log and wait for
  `rslbot ctl accept`. Nothing is accepted by itself without a `perms`
  table.

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

A bot that holds a place, because the config has `persist` or a `persist`
or `home` command was given, returns to the region and the seat it holds
whenever it finds the avatar elsewhere. `stand`, `unsit`, `sit`, `tp`,
`requesttp` and `accept tp` switch that off, and so does a teleport offer
the bot accepts by itself. From then on the bot is in
manual mode, as it is without `persist` in the config once the avatar has
got to the configured place: it stays logged in, and logs in again after a
kick or an outage (at the avatar's last location), but it no longer
teleports or sits by itself. `persist` and `home` switch holding on, and
the bot acts at once. `home` is not the avatar's home location: it is the place the bot
held last. Manual mode ends with the bot process too; a restarted bot
starts from its config. `sendtp`, `touch`, the talking commands and the
inventory commands (`inventory`, `attach`, `wear`, `detach`, `anim`,
`sound`) leave the mode alone.

A sit, stand or teleport that a scripted object forces through [RLV](#rlv)
counts as the command it is carried out as: it switches holding off too.
With `rlv persist follow` the bot holds the result instead; see
[Forced movement and the place the bot holds](#forced-movement-and-the-place-the-bot-holds).

With RLV on, the bot's own steps towards the place it holds are held to
the RLV restrictions as your commands are: it waits where a restriction
forbids the teleport or the sit, unless the `[persist]` table's `override`
says otherwise. See [What the bot does by itself](#what-the-bot-does-by-itself).

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

RLV is off unless the config has `rlv = true` or `rslbot ctl rlv mode on`
was given. While it is off the bot does not read such lines at all, and
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

The switch is not saved: a restarted bot starts from its config.

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
| `sendim`, `sendim_sec`, `sendimto:UUID` | `im` is refused, for everyone but those `sendim:UUID` names, or for the one avatar |
| `startim`, `startimto:UUID` | `im` is refused to an avatar the bot has not exchanged an instant message with since the login |
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
| `permissive` | Every restriction counts only the exceptions of its own holder, as the `_sec` ones always do |

A few notes on these. An exception counts whoever holds it, except against
a `_sec` restriction and under `permissive`, where it has to come from the
holder of the restriction. `detach POINT` takes off what it may and
answers with an error naming what stayed on. `attach` without a body part
puts an object where it was last; when the inventory records no such
point and some point is closed, it is refused, since the bot cannot tell
where the simulator would put it. Distances are measured to the object's
own position, where a viewer measures to the spot that was clicked, and an
object the simulator has not described to the bot counts as too far. That
matters for `sittp` and `fartouch`: without `scan_distance` in the config
the simulator describes few objects to the bot, so most sits and touches
are refused under them, and the bot's own sit on the seat it holds waits
after every arrival. Set `scan_distance`, with which the seat is known
within a second of arriving, or `override = "sit"` in the `[persist]`
table.
Nothing is sent to an avatar whose offer, request or message was dropped,
where a viewer would tell them.

All other restrictions of the API are kept and reported, and have no
effect: `rlv status` marks them `(no effect)`. They are about what a viewer
shows its user, or about things the bot does not do: `recvchat`, `recvim`,
`recvemote` and their variants, `showloc`, `shownames`, the map, hovertext
and camera restrictions, `fly`, `edit`, `rez`, `share`, `sendgesture`,
`tplm`, `standtp`, `defaultwear`, `setgroup`, `setdebug`, `setenv`, and
everything about the shared `#RLV` folder. The message log and `status`
are yours and are never censored. A restriction the API does not have,
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
concerns, uses the override up as well. Only `status`, `perms`, `quit` and
the `rlv` commands never do. For `requesttp` the exemption includes
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

```toml
[persist]
override = ["sit", "tp"]
```

```sh
target/release/rslbot ctl rlv persist override tp       # or: sit, sit tp, none
```

| Word | The bot does by itself, whatever is held |
|---|---|
| `tp` | Teleport to the place it holds |
| `sit` | Sit on the seat it holds or on the ground, and stand up when what it holds is `stand` |

It is empty unless the config or the command says otherwise, a value given
by command is not saved, and `rlv status` shows it as `persist override:`.
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

or in the config, as `rlv = "follow"` in the `[persist]` table. Like the
RLV switch, a setting given by command is not saved.

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

Without a `perms` table in the config the bot accepts nothing by itself,
except a teleport offer it asked for with `requesttp`. Every request is
written to the log and waits. The table says what is accepted without you:

```toml
[perms.autoaccept]
tp = ["Jane Doe"]
inventory = ["Owner Resident", "11111111-2222-3333-4444-555555555555"]
anim = "all"
attach = true

[perms.except]
anim = ["01234567-9abc-defe-dcba-987654321000"]
```

Under `[perms.autoaccept]` a kind is `"all"`, or a list of avatars and
objects; `true` is another spelling of `"all"` and `false` of an empty
list. Under `[perms.except]` a kind is a list of avatars and objects that
are never accepted by themselves, whatever `autoaccept` says: the
exceptions to `"all"`, or one object of an avatar that is listed. Both are
TOML tables and have to come after all other keys.

An entry is an avatar's name (`"First Last"`, `first.last` or a single
word) or a UUID:

| Request from | A name matches | A UUID matches |
|---|---|---|
| An avatar (teleport offer, inventory offer) | that avatar | that avatar |
| An object that gives inventory | its owner | its owner |
| An object whose script asks for permissions | its owner | the object, or its owner |

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
waiting: tp from John Roe <uuid>
```

These commands work whether or not the avatar is connected. Like `rlv
mode`, they are not saved: a restarted bot starts from its config. A
change of the rules does not accept what waits already, and it does not
take back what a script was granted.

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
