# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- The session already delivers what RLV needs to be read. `llOwnerSay` arrives as `ChatFromSimulator` with chat type 8, which `src/session.rs` reports as `SessionEvent::Chat` with kind `owner` and `Source::Object { id, name, owner }`. `src/main.rs` (`Bot::apply`) passes it to the message recorder and does nothing else with it.
- Everything the force commands of this change ask for exists behind `Commands::handle` in `src/commands.rs`, which takes a `control::Request`, a `Sender<Response>` for the answer and the `Grid` trait. `sit`, `stand` and `tp` set `went_manual`, which `Bot::follow_commands` turns into manual mode; `sit <uuid>`, `tp` and `detach` park their reply until the simulator or the inventory worker has spoken, and the outcome is logged whether or not a client still waits.
- A control-socket command is one line in, one line out (`control-socket`, "One command per connection"). `status`, `persist`, `home` and `quit` are handled in `Bot::control` because they are the bot's own business; the rest goes to `Commands`.
- RLV is specified by the wiki page "LSL Protocol/RestrainedLoveAPI" (read for this change on 2026-11-19, version 2.9 of the specification). The Kokua checkout under `references/` is a build without RLV and has no code to compare with.
- The rig has no object that says `llOwnerSay` to the bot: its helpers rez prims as `Rig Owner`, and the only scripts they add are the simulator's default script and a library script.

## Goals / Non-Goals

**Goals:**

- A parser that is a pure function from a chat line to commands, testable without a session.
- Force commands carried out by the code that carries out the `ctl` commands, so that checks, refusals, timeouts and log lines stay in one place.
- A shape that the next change (restrictions) can build on: the sender of every command is kept with it, and the mode has one owner.

**Non-Goals:**

- Restrictions, `@clear`, `@notify` and the status queries. Until they exist there is nothing to clear and nothing to report, so `@clear` is logged as not supported rather than answered as done.
- The shared `#RLV` folder and everything that names inventory folders.
- `@detach=force` and `@remoutfit=force` without an option. `ctl` has no command that takes everything off, and a first version should not gain one through RLV alone.
- `@getsitid`, `@getattach`, `@getoutfit`: the bot knows the answers, but they are not version queries and not `ctl` commands; they belong with the other queries.
- The blacklist (`@versionnumbl`, `@getblacklist`): whether the bot gets one is a question for the restrictions change.
- The automatic answer to an instant message that says `@version`. It is a different channel with its own rules (the message and the answer are hidden from the user), and it would change what the message log records.
- Hiding `@` lines from the chat log, which viewers do for their chat window. The log is the operator's only view of what objects send.

## Decisions

### A module of its own, `src/rlv.rs`

It holds the parser, the API version and reply texts, the translation of a force command into a `control::Request`, and a struct `Rlv` with the mode and the lines kept for later. `Bot` owns one `Rlv`. Parsing is a function `parse(text) -> Vec<Result<Command, Malformed>>` with `Command { behaviour, option, param }` and `Param::{Force, Channel(i32), Restrict(bool)}`; everything after it works on those values. *Alternative:* fold it into `commands.rs`. That file is 3,700 lines and is about carrying commands out; what RLV adds is a second way of asking, and the restrictions will add state that has nothing to do with `ctl`.

### Reading: kind `owner`, object source, leading `@`

`Bot::apply` looks at each `SessionEvent::Chat` before recording it. No change to the session. The bot does not compare the object's owner with the avatar: the simulator delivers `llOwnerSay` to the owner only, viewers rely on that too, and the owner field of the message is not filled the same way by every simulator. Requiring an object as the source keeps out an avatar that sends chat with a forged type, should a simulator let one through. *Alternative:* interpret `@` lines of any chat kind from objects the avatar owns. That is not RLV, and it would let an object answer on a public channel what was meant for its owner.

### Parsing follows the viewers, not a stricter grammar

Split at commas, then at the first `=`, then the left part at the first `:`. Only the behaviour and the param are lowered; the option keeps its case, because it carries region names and UUIDs. Nothing is trimmed: the specification calls the commands "spacing-sensitive", and a region name may start or end with a space only by mistake, which then fails at the lookup with a clear reason. Empty pieces are skipped so that a trailing comma is not an error.

### Version texts

`RLV_VERSION = "0.1.0"` and `RLV_VERSION_NUM = 10000` are constants of `rlv.rs`, apart from the crate version, which goes into the parentheses through `CARGO_PKG_VERSION`. The number follows the specification's formula, X·10⁶ + Y·10⁴ + Z·10² + P. `@versionnew` answers `RestrainedLove viewer v…`, as the specification says. `@version` answers `RestrainedLife viewer v…`: the specification keeps that command "for ascending compatibility" without quoting its text, and the old name is what RLV viewers send and old scripts match. That wording is from the viewers' behaviour and not from the page; task 7.2 checks it against a script on Second Life. The reply is said through `Grid::say` with the say volume, which is how a viewer's chat bar would send `/<channel> text`.

### Force commands go through `Commands::handle`

`rlv.rs` turns a force command into the `control::Request` the operator's command would have made (`Request::Sit`, `Request::Stand`, `Request::Tp`, `Request::Detach`) and hands it to `Commands::handle` with a reply channel of its own. After the call it looks into that channel once: an error is the refusal, an answer is `ok`, and an empty channel means the command is parked and under way, which is also `ok`. The receiving end is then dropped; `Commands` already copes with clients that went away, and it logs the outcome itself. *Alternative:* give `Commands` a second entry point without a reply. That would repeat the bookkeeping of parked sits and teleports (a newer one supersedes an older one, a stand ends a waiting sit) for no gain.

`handle` gets one more argument, who is asking: the operator or RLV. For RLV it leaves `last_tp` and `last_sit` alone, so that `persist last_cmd` keeps meaning what the operator last asked for. It also changes the prefix of the log lines from `control:` to `rlv:`. *Alternative:* let RLV commands count for `last_cmd`. The global form of `@tpto` has no region name to remember, so the two forms would behave differently, and an object could change what an operator's later `persist last_cmd` does.

### Forced movement: drop or follow

A bot in holding mode puts the avatar back within seconds, which would undo `@unsit=force` and `@tpto` for exactly the bots that are most likely to wear a collar. So a forced movement has to change what the bot holds, or stop it holding. Which of the two is the RLV persist setting, as the user decided on review.

**`drop`** (the default). Every forced movement goes to manual mode when it starts. That falls out of the decision above (`went_manual` is set by the same code). Nothing more happens: the operator sees it in `status` and in the log, and `home` undoes it.

**`follow`, `@sit`.** The command is not handed to `Commands` at all. `Bot` carries out `persist cur <uuid>` through its existing `persist` function, which makes the object the held seat, switches to holding and lets the reconciler do the sitting, as it does after `rslbot ctl persist cur <uuid>`. A refusal of that persist (not in a region, object known to be elsewhere) is the refusal of the command. Then `Bot` arms the mechanism that already exists for the config's setting: `apply_startup` and `startup_due` wait until the avatar is seated on the held object, give the seat's position up to `SEAT_POSITION_WAIT` to become known, and carry out a persist of `cur` and `cur`. Today they look at the config's sit state and run once per run; they are changed to look at the held sit state and to be armed again by a followed `@sit`, with a log prefix that says which of the two it was. Arming survives a session loss, as it does at startup, and is cleared by the same things: a persist, `home`, or anything that goes manual.

This follows the user's review of the first draft, which carried the sit out as a `sit` command and persisted on confirmation. Persisting first means the forced seat is held from the moment it is asked for: a sit that fails is tried again with the reconciler's backoff, including its repositioning teleport after three failures, and the earlier held placement is gone whether or not the sit ever succeeds. That is what "persistent sit target" means, and it is the difference from `drop`, where a forced sit is one attempt.

**`follow`, the other three.** `@sitground`, `@unsit` and `@tpto` are carried out through `Commands::handle` and so go to manual mode first, which keeps the reconciler out of the way. `Bot` remembers the one under way in `following: Option<Following>`, with what it is and whether the bot was in manual mode before. A stand and a ground sit end when `handle` returns without a refusal, and are persisted at once (`persist cur stand`, `persist cur ground`). A teleport ends well with `RegionEntered` or `TeleportArrived`, where `persist cur stand` follows, and badly with `TeleportFailed` or `Lost`, where `Bot` calls `set_manual` with the remembered mode and persists nothing: a failed teleport leaves the avatar where it was, so dropping what the bot held would be a side effect of a command that did nothing.

`following` and the armed second persist are cleared, without persisting, when an operator's request reaches `Commands` and sets `went_manual`, by `persist` and `home`, and by the next forced movement. A forced movement that replaces a teleport under way keeps the teleport's remembered mode.

The sit state after a stand and after a teleport is given as `stand`, not `cur`: right after the stand request the simulator still shows the avatar seated, and `cur` would hold the seat it was just told to leave. After `@sitground` the persist makes the reconciler send the ground sit once more (`object-sit`, "Sit on the ground when configured"), which changes nothing for an avatar that already sits.

*Alternatives:* (1) Treat `@tpto` like `@sit` and persist the destination first. The global form has no region name to persist, and the reconciler's teleport retries would turn one forced teleport into many; the user asked for the change for `@sit` only. (2) Leave a failed followed teleport in manual mode: rejected above.

### The `[persist]` table: `atstart` and `rlv`

`config::Persist` keeps its two cases; the table is read into them. `atstart` true (given, or implied by `position` or `sit`) gives `Persist::On` with `cur` for a missing key; `atstart` false gives `Persist::Off`. `Config` gets a field `rlv_persist` with the two values, taken from the table's `rlv` key and `drop` for every other form. An explicit `atstart = false` beside `position` or `sit` is an error and not a silent override: the file would otherwise say two things, and every other contradiction in the config is rejected too.

This changes one thing for existing configs: an empty `[persist]` table, which today means `cur` for both, holds nothing. That follows from "`atstart = false` is the default" and is the price of letting a table carry only `rlv`. README's Upgrading section gets a line.

The key is `[persist] rlv`, a string, next to the top-level `rlv`, a boolean. They are told apart by where they stand; error messages name them `rlv` and `persist.rlv`.

### `@tpto` by global coordinates: the handle is computed

`TeleportLocationRequest` takes a region handle, which is the region's south-west corner in metres, and a position inside the region. For global X, Y the bot takes the corner as `floor(X / 256) * 256`, `floor(Y / 256) * 256` and the remainder as the position, and sends the request at once, without the map lookup the named form needs. The session gets `start_teleport_to(handle, position)` next to `start_teleport`, sharing its end states; the `Grid` trait gets `teleport_global`. *Alternative:* ask the map for the region at those grid coordinates first. One more round trip and one more failure mode, to learn a name the teleport does not need.

A point whose X and Y are both below 256 gives the handle 0, which is refused before anything is sent. On Second Life (2026-11-19) such a request got no answer at all, so the teleport ran into its 60-second limit with the avatar counted as on its way for all that time; the session already takes a map entry at 0, 0 for "no such region". The numbers were coordinates inside the region, typed without its name, which is the mistake the reason points at.

The four-part form and the three-part form are told apart by the number of `/`-separated parts alone; a region name cannot contain `/`. Everything from the first `;` on is cut off before that (the look-at direction, which the bot's teleport request sets to a fixed value).

### Attachment points by their RLV names

`@detach:<point>` is first read with `inventory::parse_point`, minus its numbers, and then against a table of nine RLV-only names: the eight HUD points without `HUD ` and `root`. Numbers are left out because RLV never sends them and a folder may be named `5`. An option that names no point is "not supported" rather than "refused", because in RLV it is a folder of the shared inventory; the log line should not call a valid command wrong.

### `@remattach` by UUID

Added after the first run on Second Life, where the operator tried `@remattach:<uuid>=force` and found it not supported. The specification lists the form (`@remattach[:attachpt or :uuid]=force`, the UUID since v2.9.20) and does not say whose UUID it is. A script can only know the attached object's, so that is what is looked up first, with the lookup `@detachme` uses. A UUID that is no attachment's object is passed on as the inventory item's, which is the only UUID `rslbot ctl` shows an operator and what was tried by hand; `detach <uuid>` then refuses what is not there. The same is accepted for `@detach`, which `@remattach` is an alias of.

### `@clear` is well-formed without a param

The same run showed objects sending a bare `@clear`, which the parser called malformed. The specification gives `@clear` and `@clear=<string>` as its two forms, so the parser reads both, with whatever follows the `=` as a filter and not as a param. It stays not supported, as there is nothing to clear.

### `@detachme`: from the sender to its inventory item

The session knows the avatar's attachments by inventory item, each with its region-local id, and every described object by UUID and local id. A new `Session::attached_item(object) -> Option<Uuid>` joins the two, and the `Grid` trait passes it on. The item then goes through the ordinary `Request::Detach` by UUID. A script in a child prim of an attachment sends the child's UUID, which is not the attachment's; that case is refused with a log line that says the sender is not attached (see Risks).

### What is not supported is logged, and nothing else

Viewers tell their user "fails command" in the chat window. The bot's chat window is the operator log, so that is where the line goes, at the ordinary level and one per command. Nothing is said in-world: an answer on a channel is reserved for queries, and a script that asked `@getstatus` must hear nothing rather than something it would misread.

### Lines kept until the avatar can act

`Rlv` holds a queue of `(sender, name, text)`. `Bot::apply` puts a line there instead of dealing with it when the session is not `present()` (not yet in the region, or teleporting), and `Bot::service_session` drains the queue whenever the session is present. The queue is emptied in `Bot::drop_session`, on `rlv mode off`, and when the logout starts. Sixty-four lines is several times what a login with a dozen scripted attachments produces. *Alternative:* drop what arrives early. The specification points out that attachments ask for the version halfway through the login and that viewers buffer; a script that gets no answer treats the avatar as having no RLV for the rest of the session.

`rlv cmd` does not use the queue: the operator gets an answer on the same connection, and "kept for later" is not an answer.

### The `rlv` control-socket commands

`control::Request` gets `Rlv(RlvRequest)` with `Mode(bool)`, `Persist(follow or drop)`, `Status` and `Cmd(String)`. `Request::decode` checks the subcommand and its arguments and joins the words of `cmd` with single spaces, as it does for `shout`; `request_line` on the client needs no change. `Bot::control` handles all four itself, next to `status`, since the mode and the persist setting live in `Bot`. `rlv persist` only sets the value: a movement under way carries the value it started with in `following`, so switching does not change how it ends.

`rlv status` prints the persist setting on a second line. The request asked for the mode only, but a setting that can be switched while the bot runs has to be readable somewhere, and the line is in the `name: value` form the future restriction list can follow. `rlv cmd` builds its answer from the outcomes `rlv.rs` returns for each command; an answer is one field on the wire, and line feeds inside it are escaped already.

`rlv cmd` both says a query's reply on the channel and prints it. Saying it is what makes the command behave "as if an object had sent it" and lets the operator test a listening script; printing it is what an operator at a terminal wants to see. The sender is the avatar's own id, as in the console of an RLV viewer.

`rlv cmd` is refused while RLV is off so that off has one meaning: no RLV command is carried out, wherever it comes from.

### Log lines

All start with `rlv:`. For example:

```
rlv: on
rlv: Collar <uuid>: versionnew=5: said "RestrainedLove viewer v0.1.0 (RusselBot 0.1.0)" on channel 5
rlv: Collar <uuid>: sit:<uuid>=force: sitting on <uuid>
rlv: Collar <uuid>: detach=n: not supported
rlv: operator: tpto:north/128=force: refused: ...
rlv: keeping 3 messages until the avatar is in a region
rlv: persist follow
rlv: Collar <uuid>: sit:<uuid>=force: persist cur <uuid>
rlv: following: persist cur cur
rlv: following: persist cur stand
rlv: not following: the teleport failed; holding <placement> as before
```

The persist itself logs the placement that results, as it does for the command.

## Risks / Trade-offs

- [Any object the avatar owns can move it and undress it once RLV is on] → Off by default; the README section says so in its first paragraph; `rlv mode off` stops it at once. The restrictions change is where per-object trust belongs.
- [With `follow`, an owned object decides where the bot returns to after a kick or a restart, for as long as the bot runs] → `drop` is the default; the log names every followed persist with its placement; `persist` or `home` by the operator replaces it, and a restart goes back to the config.
- [An empty `[persist]` table changes meaning] → Named as breaking in the proposal and in README's Upgrading section. The form is unlikely in practice: the table was documented with its keys, and a missing key was the documented way to say `cur`.
- [A followed sit is confirmed but the seated position never becomes known] → After 10 seconds the persist is carried out with the position the bot has, as at startup, and the log says so.
- [With `follow`, a forced sit on an object that does not exist, or cannot be sat on, is retried for as long as nothing else is asked, and replaces what the bot held] → Intended: the seat is persistent from the moment it is forced. The reconciler's backoff bounds the traffic, the log shows each attempt, and a `persist` of something else or any operator command that moves the avatar ends it; `home` does not, since the forced seat is what it returns to (seen on the rig, item 123). README says so beside the table.
- [A script concludes from an answer to `@versionnew` that restrictions work, without looking at the number] → The version is 0.1.0 for that reason, and every ignored restriction is in the log. README states that this version cannot hold the avatar to anything.
- [The `@version` text is not quoted by the specification] → Checked on Second Life in task 7.2; a different text is a one-line change.
- [Global `@tpto` assumes 256 m regions] → True on Second Life. On an OpenSim grid with larger regions a point beyond the first 256 m gives a handle that is not a region's corner; OpenSim's teleport code looks the region up by position, but that is not verified here. README notes it, and the named form works everywhere.
- [`@detachme` from a child prim is refused] → Scripts that manage a worn object usually sit in its root prim. Resolving a child to its root needs the parent ids the session does not keep; left for when a real object needs it.
- [`@unsit=force` straight after `@sit` in one line may fail, as the specification warns for viewers too] → Nothing to do; the two requests go out in order and the simulator decides.
- [A chatty object fills the operator log, one line per restriction it sets] → Accepted for now: the lines are what tells the operator which commands the next change has to cover.
- [The rig cannot say `llOwnerSay` to the bot today] → Task 7.1 builds a helper that logs in as the bot's account while the bot is stopped and leaves a prim with a relaying script; if OpenSim's script upload cannot be driven from it, the owner-say path is verified by unit tests and on Second Life only, and `docs/test-rig.md` says so.

## Migration Plan

None needed: without `rlv = true` and without `rlv mode on` the bot behaves as before. Switching it off again is `rlv mode off` or removing the key.

## Open Questions

- Whether `status` should also show the RLV mode. Left out because the request asked for `rlv status`; adding a line later breaks nothing.
