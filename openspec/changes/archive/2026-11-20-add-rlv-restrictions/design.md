# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- `src/rlv.rs` already parses a message into `Command { text, behaviour, option, param }` with `Param::Restrict` for `n`, `add`, `y` and `rem` and `Param::Absent` for `@clear`, and hands every command to `deal` together with its `Issuer` (an object's UUID and name, or the operator). `deal` answers `Step::Failed("not supported")` for all of them today. `Param::Restrict` does not say which of the four it was.
- `Rlv` (in `Bot`) holds a boolean mode, the persist setting and the queue of messages that wait for the avatar. `Bot::control` handles `rlv`, `status`, `persist`, `home` and `quit` itself and passes every other request to `Commands::handle`.
- Everything the avatar does on command goes through `Commands` (`src/commands.rs`), which already knows who is asking through `Origin::{Operator, Rlv}` and keeps that with a parked command (`pending_sit`, `pending_teleport`). Commands that first look something up (`im`, `requesttp`, `attach`, `wear`, `detach` by path) act later, from `avatars_found` and `inventory_answer`. Offers and permission requests arrive through `Commands::teleport_offer`, `inventory_offer` and `permission_request`; a request to be teleported to the bot is only logged in `Bot::apply`.
- The bot's own steps are chosen by `reconciler::decide` and started in `Bot::reconcile`, which calls `reconciler.started(action)` before acting. The one stand the bot does by itself is in `Bot::persist` (`needs_stand`). Attachments are restored from `Commands::restore_due` and `restore_missing`.
- The session (`src/session.rs`) keeps, for the objects the simulator has described, the UUID and the region-local id in both directions, and for attachments the item, the local id, the parent and the point. The parsed updates (`ObjectParent`, `CompressedObject`) carry the parent id and a `Transform`, which the session uses only for the avatar and its seat. `KillObject` removes an object from these tables without an event, and the tables are emptied when the region changes.
- The specification is the wiki page "LSL Protocol/RestrainedLoveAPI", read again for this change on 2026-11-20. It says what each restriction means for a viewer's user; it does not say how the checks are ordered where several apply, nor what a force command does under a restriction beyond `@sit`, `@sitground` and `@tpto`. The Kokua checkout under `references/` is a build without RLV.
- The rig has `test-rig/relay.py`: a rezzed `Relay box` and a wearable `Relay` that say to their owner what they hear on channel 77, so an object can be made the holder of a restriction with `rslbot ctl say /77 @...`.
- Decided by the user: the bot's own placement is held to restrictions like the operator's commands, and an object's restrictions end with the object, as in a viewer. On review of the first draft the user added the persist override setting: a region restart leaves the avatar in a hub with its attachments' restrictions intact, and a bot that holds a place has to be able to return to the region whose objects restrict it. The setting is `[persist] override` in the config and `rlv persist override` while the bot runs, and `sit` covers the stand the bot owes as well.

## Goals / Non-Goals

**Goals:**

- One store with one question interface, so that a command asks "may I do this to that?" in one line and a later change can make another restriction effective by adding a row and a call.
- Checks at the place where the bot acts, not where it reads the command, so that the operator's commands, the bot's own steps and force commands share them.
- The store, the chat rules and the exception rules as pure code, testable without a session.
- No restriction logic in the session: it reports facts about objects.

**Non-Goals:**

- Everything the proposal lists as not in this change.
- Telling scripts that a restriction is kept but has no effect. A viewer gives no such signal either; the operator sees it in `rlv status`.
- A time limit or a confirmation for override.
- Telling avatars that their offer, request or message was dropped. A viewer notifies them; the bot sends refusals for nothing by itself (`permission-requests`, "Deciding a request"), and a message in the avatar's name that the operator did not write would be the first.

## Decisions

### The store: holders in order, each with its restrictions in order

`rlv.rs` gets `Restrictions`: a `Vec<Holder>` in the order holders first set something, each `Holder { who: Who, name: String, rules: Vec<Rule> }` with `Who::{Operator, Object(Uuid)}` and `Rule { behaviour: String, option: Option<String> }`. A holder with no rules left is removed. Vectors, not maps: `@getstatus` and `rlv status` need the order, the numbers are small (bounded at 1024 rules), and every lookup is a scan of a few dozen entries. No change counter is kept: the one place that would have used it, the held-back log line, compares the refusal's text instead.

`Param::Restrict` becomes `Param::Restrict(bool)`, true for `n` and `add`.

*Alternative:* one map from behaviour to holders, as viewers have it (a multimap from UUID to rule). It makes "is `sendim` in force" one lookup and everything that is per holder (`@clear`, `@getstatus`, dropping a holder, exceptions that count only from the same holder) a scan with more bookkeeping.

### What is known is a table

Two static lists, `ENFORCED` and `KEPT`, hold every behaviour of the specification's restriction commands, as the spec's two lists have them, and `effect(behaviour)` says which a behaviour is in. `Restrictions::apply` keeps a restriction when its behaviour is in one of them; any other is left to `deal`, which answers "not supported". Setting, lifting and clearing are the store's alone and happen before `deal` is called: `deal` is handed the grid, which holds a shared view of the store, and so cannot change it. `Effect` is used by `rlv status` for `(no effect)` and by nothing else: whether a restriction has an effect is decided by whether a check asks for it.

Unknown behaviours are not kept, although Marine Kelley's viewer keeps any string: the user asked for known commands to be ignored silently, and an unknown one is more likely a typo or a command of a newer API, which the operator log should show. `@notify` is left out of the table on purpose: a script that sets it expects messages the bot does not send, and `@getstatus` should not claim it.

### Questions, not data: `Limits`

Nothing outside `rlv.rs` looks into the store. Callers get a `Limits<'a>`: the store, or nothing when no restriction is in force for this caller, plus the holder whose own restrictions do not count. It has one method per kind of question, each returning `Result<(), Refusal>` where `Refusal { rule: String, holder: String }` is what error texts and log lines are made from:

- `in_force(behaviour)`: any holder holds it without an option.
- `against(behaviour, target)`: the rule with exceptions of the spec ("When restrictions are in force"), including `_sec` and `permissive`. Targets are compared as UUIDs or as channel numbers, parsed from the options; an option that does not parse never matches.
- `names(behaviour, target)`: a holder holds `behaviour:<target>` (`sendimto`, `touchthis`, `sendchannel_except`, `accepttp:<uuid>`, ...).
- `distance(behaviour)`: the smallest distance of the holders that hold it, 1.5 where one gives none, `None` when nobody holds it. `tplocal` differs: without a distance it has no limit to be within, so its question is `local_teleport(distance)`.
- `chat(channel, volume, text) -> Result<Vec<Utterance>, Refusal>` with a note of what was changed: the four steps of "Chat the avatar sends".
- Compound questions that name what is being done: `stand(seated)`, `sit(object distance)`, `teleport(seated, same region, distance)`, `touch(target)`, `attach(point)`, `detach(attachment)`, `wear(kind)`, `unwear(kind)`, `im(to, in conversation)`, `inventory()`. These hold the tables of the spec, so that a table is in one function.

`Limits::none()` is what a caller gets while RLV is off and for the exempt command. So "no restriction is in force" is one value, and no call site asks about the mode.

*Alternative:* checks in `Bot::control` before a request reaches `Commands`. It cannot see what a command will act on: the recipient of an `im` is known after a lookup, the point of an `attach` after the inventory answered, the offer an `accept tp` means after `Commands` picked it. It would also miss the paths that do not start at the control socket.

### Who is asking travels with the command: `Origin` grows

`Origin` becomes `Operator`, `Exempt` (the operator's command that override exempts), `Rlv(Who)` and `Bot` (the bot's own acceptances and the restoring of attachments). `Commands` already stores the origin with what it parks; it now stores it with every job that acts later (`After::Im`, `After::RequestTp`, the `Then::*` of inventory lookups, the record of a `requesttp` in `asked`). When a job acts, the `Limits` it uses are built from the store and the job's origin (`Origin::limits`): nothing for `Exempt`, the store minus the sender for `Rlv(who)`, the whole store otherwise. The store reaches `Commands` with the grid: the `Grid` trait gets `restrictions()`, which `Live` answers from the `Rlv` it is built with and which is nothing while RLV is off. Every path that acts already has the grid, since nothing can be done without it, so no signature in `Commands` changes. The same trait gets the three facts the questions need: `sitting()`, `thing(object)` and `attachment_prims(item)`.

This is what makes the exemption last "until the command is answered" and makes "an object's own restrictions do not stand in the way of its own force commands" one rule instead of a special case per command.

*Alternative:* `Commands` owns the store. Then `Bot` would reach into `Commands` for `rlv status`, `rlv reset`, the reconciler's checks and the session events that drop holders, and `rlv.rs` would stop being the one place for RLV state.

### The sender's own restrictions never inhibit its force commands

The specification says this for `sittp` only ("this command DOES work if the sit-tping prevention is issued by the same object") and gives the reason: the script would otherwise have to lift, force, wait and set again. The reason holds for every pair: a chair that holds `unsit` and sends `@unsit=force`, a collar that holds `detach` and sends `@detachme=force`. So the rule is general, and it also defines `rlv cmd` without a special case: the operator's console is a sender like any other, held by every object's restrictions and by none of its own. Viewers differ here (RLVa is lenient in about this way, Marine Kelley's viewer for `sittp` only); a stricter reading would break scripts that work in the most common viewer.

Which restrictions inhibit which force command is otherwise taken from the page: `@sit` by `sit`, by `sittp` beyond the distance and by `unsit`; `@sitground` by `sit`; `@tpto` by `tploc` and by `unsit`. `tplocal` does not inhibit `@tpto`, which the page says of `tploc` and `unsit` only. For `@unsit=force` the page is silent; it is inhibited by `unsit` of other holders, since a force command that undoes another object's restriction would make that restriction worthless. `unsit` counts only while the avatar sits: the restriction is about leaving a seat.

A followed `@sit` is not carried out through `Commands` but through `Bot::hold_seat` and the reconciler. `deal` checks it before returning `Step::HoldSeat`, and `Bot` remembers the sender (`seat_sender: Option<Who>`) beside `follow_seat`; the reconciler's sits towards that seat ask with that sender left out. It is cleared where `follow_seat` is cleared and whenever something else is persisted.

### Three modes, and the exemption is decided in `Bot::control`

`Rlv.on: bool` becomes `mode: Mode::{Off, On, Override}`. `Rlv::is_on()` stays and is true for both `On` and `Override`, so the reading of chat, the queue and `rlv cmd` need no change. `Rlv::limits(origin)` gives `Limits::none()` when the mode is `Off`.

`Bot::control` looks at the request before the `match`: when the mode is `Override` and the request is none of `Rlv`, `Status`, `Perms` and `Quit`, it sets the mode to `On`, logs which command was exempt, and carries the request out with `Origin::Operator { exempt: true }`, or for `Persist` and `Home` with the flag described below. This is the "moment the command is taken up": before `Commands` reads the arguments, so a command with bad arguments uses the override up too. A line that does not decode never reaches `Bot::control`.

Which commands use the override up is a fixed list of four that never do, rather than "the commands a restriction can refuse": the operator can remember four names, and the second rule would change with every restriction a later change makes effective. `anim`, `sound` and `sendtp` use it up although nothing restricts them today.

`rlv mode override` is refused while RLV is off. It could mean "switch on for one command", but the request defines override as on with one exemption and as returning to on, and switching RLV on as a side effect of a command would be a surprise.

### The bot's own steps: held back in `Bot::reconcile`, not in the reconciler

`reconciler::decide` stays a function of observed state, memory and goal. `Bot::reconcile`, after `Decision::Start(action)` and before `reconciler.started(action)`, asks the `Limits` for `Origin::Bot` whether the action may be started: `teleport(...)` for `Action::Teleport`, `sit(...)` for `Action::Sit` and `Action::SitGround`. When it may not, `reconcile` returns "wait" without calling `started`, so no backoff runs and no failure is counted. Whether a step may start is one pure function, `rlv::own_step(limits, sitting, step, persist override) -> Verdict::{Go, LetThrough, HeldBack}`, which the unit tests walk through the scenarios; `Bot::held_back` gathers the facts for it from the session. `Bot` keeps `held_step: Option<(String, String)>`, what is held back and the refusal's text: the log line is written when this changes from nothing or to another text, and "no longer holding back" when a decision gets through or there is nothing to start. A bot under test has no session, so this glue (no `started` call, the two log lines) is exercised on the rig, not by unit tests. The main loop calls `reconcile` after every event, and every change of the store comes with one (a chat line, a control command, a session event), so nothing has to wake the bot up.

*Alternative:* give the reconciler a "blocked" memory and a `Decision::Blocked`. It would put a second notion of waiting beside backoff into the code that the project's most careful tests cover, for a condition the reconciler cannot evaluate itself.

The stand in `Bot::persist` is the one step that has no `Action`. It becomes a flag `stand_owed`, set by `persist` when the avatar is in a region and `needs_stand` is true at that moment (the bot does not keep an avatar standing that sits down later), and tried in `reconcile` before the decision: carried out when the limits allow it, held back otherwise with the same log line, and dropped when the avatar no longer sits on anything, when the bot goes manual and at the next `persist`. Without restrictions the stand goes out in the same pass of the loop as today.

The exemption of an exempt `persist` or `home` is a flag `exempt_placement` in `Bot`, set when such a command is carried out. While it is set, `reconcile` and the owed stand ask `Limits::none()`. It is cleared when the reconciler has nothing left to do for the held placement (`in_target_region`, and seated as the goal says), at the next `persist`, and when the bot goes manual. It survives a session loss: the command asked for a placement, and a login in between does not make the placement another one.

`persist` and `home` are never refused themselves. Refusing them would need the same checks twice (now, and at each step) and would make "hold this place when you can" impossible to say while a restriction is in force.

### The persist override setting: asked first, then let through

`Rlv` holds the setting as two booleans beside the RLV persist setting, read from `Config::persist_override`, which `config::persist_table` fills from the `override` key (a string, or an array of strings; anything else is an error naming `persist.override`). `control::RlvRequest` gets `PersistOverride { sit, tp }`, read from `["persist", "override", ..]` before the `follow` and `drop` forms.

Each step has a sort: `Action::Teleport` is `tp`; `Action::Sit`, `Action::SitGround` and the owed stand are `sit`. `Bot::reconcile` still asks the `Limits` first. When they refuse and the setting names the step's sort, the step is started and its log line names the restriction that was let through; when they refuse and it does not, the step is held back as before. Asking first, rather than skipping the question, is what makes the log say which restriction the setting overrode, which is what an operator reads when the bot left a cage.

A `tp` step under the setting ignores every restriction that refuses a teleport, `unsit` while seated among them: the case the setting exists for includes an avatar that was seated by the object that restricts it. `sit` likewise ignores `unsit` for a change of seat.

The setting is not consulted anywhere else. In particular it is not an input to `Limits`: the operator's `tp` is refused under `tploc` whatever it says, and so is a forced `@tpto`. Restoring attachments is not a placement step and stays held to restrictions.

The name clashes with `rlv mode override`. The two differ in what they exempt (one operator command against the bot's own steps of a sort) and in how long (once against standing); texts and log lines say "persist override" for the setting and never "override" alone.

*Alternatives:* (1) Exempt the bot's own steps from restrictions altogether, which was the other answer to the first question; the user chose restriction as the default and this as the exception per sort. (2) Exempt only the first return after a region restart. The bot cannot tell a restart from a script that moves the avatar: both arrive as a change of region without a command.

### Looking for an object before deciding

Added after the first rig run of the implementation (2026-11-20, `docs/test-rig.md` item 154): under `sittp` the bot's own sit on the seat it holds was held back after every arrival, because without `scan_distance` the simulator only mentions the seat and the bot never asks about it. A restriction that goes by distance was unusable in the bot's default configuration. The user chose to have the bot look.

The session can already do this for `touch`: `start_scan` asks about everything the simulator has only mentioned and keeps asking, and `touch` puts the object into `wanted` with a deadline of `TOUCH_WAIT` after the start of the scan. That is generalised:

- `Session::look_for(object, now) -> Result<Look, String>` with `Look::{Known, Waiting, GaveUp}`: known when described, waiting when the scan is within its 10 seconds (the object goes into a list `sought` beside `wanted`), gave up when the scan has had its time. Two events end a wait: `SessionEvent::Described(Uuid)`, pushed when the update that describes the object has been taken in whole, so that its parent and place are there when the event is read, and `SessionEvent::NotDescribed(Uuid)` at the deadline, at a change of region and with the session. `Session::scan(now)` starts the scan without waiting for anything, for the bot's own step.
- `Grid` gets `look_for`. In `Commands`, `sit` and `touch` ask the `Limits` as now. When they are refused and the object is one the simulator has not described (`Thing::placement` is `Unknown`), the command is parked in `looking: Vec<Looking { object, origin, reply, then }>` instead of answered, and `Commands::described(object, grid, now)` carries it out again from the start with its origin, which then finds the object known. `Commands::not_described(object)` answers with the refusal the first check gave. A `touch` under a restriction must not use the session's own wait, which sends the touch from inside `learn` and would pass the restriction: it looks first and touches after.
- A forced `@sit` that is not followed is a `sit` command with `Origin::Rlv(who)` and waits in `looking` with the others; the bot goes manual when it is carried out, through `follow_commands` after `Commands::described`.
- A followed `@sit` does not go through `Commands`. `deal` returns a new `Step::Sought(object)` when its check fails only for want of a description; `Bot` asks `look_for` and keeps `(issuer, command text, object)` in `seats_sought`. On `Described` it deals with the command again, which ends in `Step::HoldSeat` or `Step::Failed`; on `NotDescribed` it logs the refusal. This is the second waiting mechanism the user asked for; it is small because nothing is answered to anybody. `rlv cmd` reports the step as `ok`, as it does for a sit whose outcome comes later.
- `Bot::held_back` calls `session.scan(now)` when the verdict for `Action::Sit` is held back and the held object is not described. The step is looked at again with every pass of the main loop, so nothing else is needed, and there is no deadline: the bot's own sit has all the time there is.

Whether a refusal is for want of a description is asked of the `Thing` and not of the `Refusal`: `Limits` stays a function of plain values. No scan starts unless a check has refused something for an undescribed object, which is what keeps a bot with `rlv = true` and no such restriction at the traffic it has today.

As built (task group 9): `Limits::sit_on` and `Limits::touch_on` answer `Check::{Allowed, LookFirst, Refused}`. For an undescribed object a sit is looked for only when it would be allowed near at hand, and a touch only when some row of "Touching" is held and some kind of thing the object may turn out to be, near or far, would be touchable; what is refused whatever the object is (`sit`, `interact`) is refused at once without a scan. `Commands::described` and `Commands::not_described` both carry the parked command out again from the start; after a `NotDescribed` its `look_for` answers `GaveUp` (or the session is not in a region), so the command is refused by the restrictions in force then, a `touch` with `Grid::unknown_object`, the text the session gives an unknown object. `deal` asks `look_for` itself for a followed `@sit` and returns `Step::Sought` only while it waits; `Bot` deals with the kept command again on either event. Rig rows 158 to 164 record the run.

*Alternatives:* (1) Scan whenever RLV is on. Every bot with `rlv = true` would pay the traffic the scan was made optional to avoid. (2) Take an object of unknown place as near. A cage's `sittp` would then not keep the avatar from sitting through its wall, which is what the restriction is for. (3) Ask the simulator about the one object. It takes region-local ids, and the bot has only the UUID.

### Restoring attachments

`Commands::restore_missing` asks `attach(point)` for every object it is about to attach, with the point the inventory records, and leaves out what is refused. After a login the only restrictions there can be are the operator's and those attachments sent in the first 20 seconds, which is what a viewer's user would be held to as well.

### Chat: one pure function

`Limits::chat` takes what `Commands::say` has read (channel, volume, text) and returns the utterances to send, each with channel, volume and text, or a refusal. The order is that of Marine Kelley's viewer as far as the page shows it: volume first (`chatwhisper` and `chatshout` to normal, then `chatnormal` to whisper, so that all three together whisper), then redirection, then `sendchat`. Redirection comes before `sendchat` because a gag that holds both wants the text on its channel and nothing in public; the page's "does not supersede `sendchannel`" is read as: a redirection to a channel that `sendchannel` closes is not sent. An emote is `/me ` or `/me'`, as viewers have it. The lengths 30 and 15 are counted in characters, not bytes.

The volume rules apply on channel 0 only, as the page describes them as about public chat ("the avatar will chat normally even when the user tries to shout").

The bot's `say` reads `/shout` and `/whisper` before any of this, and `//` and `/<digits>` as a channel, so "text that starts with `/`" in step 3 is what is left: `/me...` and other slash words, which the bot says on channel 0 as they are.

### Instant messages: "in conversation" is a set in `Bot`

`startim` restricts starting a session and leaves open ones alone. The bot has no sessions, so one is defined: an avatar that sent the bot an instant message or was sent one since the login. `Commands` keeps them in a `HashSet<Uuid>`: it adds the recipient when it sends a message, `Bot::apply` tells it of every instant message an avatar sent (`im_received`), and `session_lost` empties the set. *Alternative:* treat every `im` as a new session. It would turn `startim` into `sendim`, and the difference is why scripts use it: the wearer may answer but not approach.

Nothing is sent in place of a refused message. A viewer sends the recipient a notice; see Non-Goals.

### What the session has to tell

Three additions, all facts:

1. For every described object, its parent's local id and its position, from the updates that carry them, beside the two id maps. A child prim's position is its root's; an attachment's is the avatar's. An update whose transform the bot does not read leaves the position unknown, which the checks take as too far.
2. Queries on top of that: `placement_of(object) -> Whereabouts::{Hud, Worn, WornBy(avatar), InWorld, Unknown}` with the root's UUID, following parents to the avatar's own local id (worn; a HUD when the root's point is one of 31 to 38), to another avatar (worn by it) or to nothing (in-world); and `distance_to(object) -> Option<f32>`. `attached_item` uses the same walk, so `@detachme` from a child prim finds its attachment, which the first RLV change left as a known gap.
3. Two events: `ObjectGone(Uuid)` when `KillObject` names a described object, and, when the avatar leaves a region, `RegionLeft { worn: Vec<Uuid> }` with the UUIDs of the described objects that were on the avatar, roots and children, taken before the tables are emptied.

`Bot::apply` turns these into `Restrictions::drop_holder` and `Restrictions::keep_only(worn)`, and `Bot::drop_session` drops every object holder. The store never asks the session whether an object exists: a holder the simulator never described (a rezzed object out of the interest range that still reaches its owner with `llOwnerSay`) cannot be told gone, and stays until the region is left.

*Alternative:* a garbage collector on a timer, as viewers have, asking for each holder whether the object is known. It would drop holders the simulator has merely not described, which for the bot is the normal state of far objects without `scan_distance`.

Whether a `detach` without an option locks something is asked when the lock is tested, not when the restriction is set: `Limits::detach(attachment)` gets the UUIDs of the attachment's prims from the session and looks for a holder among them. A collar that sends `@detach=n` during the login, before the simulator has described it, is locked as soon as it is described.

### Offers, requests and permissions

`Commands::teleport_offer` asks in this order: `tplure` against the offering avatar (drop and log; skipped when the offer answers a `requesttp` whose record is exempt); then whether it would accept by itself, now also when `accepttp` names the avatar or is held without an option; then, before accepting, `teleport(seated, ...)` for `unsit`, which makes the offer wait. `accept tp` asks `tplure` and `unsit` when it has found the offer. `tplure` wins over `accepttp`, as a restriction over a convenience.

`Bot::apply` passes `TeleportRequested` to a new `Commands::teleport_requested`: dropped under `tprequest`, answered with `Grid::offer_teleport(from, "")` when `accepttprequest` applies, logged as today otherwise.

`Commands::permission_request` drops a request with `attach` or `controls` under `denypermission`, and otherwise treats those two kinds as auto-accepted under `acceptpermission` when it decides. The rules in `perms.rs` are not touched.

A request that waits is looked at again only when it is accepted. Dropping waiting offers when a restriction is set would make the store reach into the queues of `Commands` for a case `accept` already refuses.

### Detaching from a point with locked attachments

`detach <point>` and `@detach:<point>=force` both end in `Commands::detach` for a point. It detaches what `Limits::detach` allows and reports what stayed. For the operator the answer is an error when anything stayed, since the command did not do what it said; for a force command it is `Step::Done` when anything was detached. Viewers skip locked attachments in a forced detach, and one rule for both callers keeps the code in one place.

### `@getstatus`

`deal` gets two more queries. The reply is built by `Restrictions::status(who, part, separator)`, with `who` absent for `@getstatusall`, and said through `Grid::say`, which sends an empty text as it is. The option is split at the first `;`. The reply is cut at the chat limit of 1023 bytes on a character boundary; a script that sets more than that cannot read it back in a viewer either. The operator is part of `@getstatusall`: a viewer keeps the console's restrictions under the avatar's own UUID in the same table.

### `rlv status`, `rlv reset` and the error text

`Rlv::status()` prints the mode, the RLV persist setting and the persist override setting, and appends the restriction lines. `control::RlvRequest` gets `Reset`, and `Mode(bool)` becomes `Mode(Mode)`; `Request::decode` reads `["mode", "override"]` and `["reset"]`.

A refusal reads, for example:

```
refused by RLV: unsit, held by Collar <uuid>; `rslbot ctl rlv mode override` lets one command through
refused by RLV: tploc, held by the operator; `rslbot ctl rlv mode override` lets one command through
```

One holder is named, the first in the store's order, although several may hold the restriction: the operator needs to know where to start, and `rlv status` has the rest.

### The API version stays 0.1.0

The number tells scripts how much of the API to expect, and the bot still lacks most queries, `@notify` and the shared folder. Raising it is a decision about which scripts should start to trust the bot, to be taken with a list of what they check; it is one constant.

### Log lines

All start with `rlv:`. For example:

```
rlv: override
rlv: on again; `stand` was exempt
rlv: Collar <uuid>: detach=n: set
rlv: Collar <uuid>: detach=n: set already
rlv: Collar <uuid>: clear=tp: lifted 2 restrictions
rlv: Collar <uuid>: sit:<uuid>=force: refused: unsit, held by Cuff <uuid>
rlv: stand refused: unsit, held by Collar <uuid>
rlv: say: whispered instead of said, because of chatnormal held by Gag <uuid>
rlv: dropped 2 restrictions of Collar <uuid>: the object is gone
rlv: dropped 3 restrictions of 2 objects: the session ended
rlv: teleport offer from Jane Doe <uuid> dropped: tplure, held by Collar <uuid>
rlv: holding back the teleport to "Target": tploc, held by Cage <uuid>
rlv: no longer holding back the teleport to "Target"
rlv: persist override: tp
rlv: teleporting to "Target" although tploc is held by Collar <uuid>: persist override tp
rlv: reset: 3 restrictions cleared
```

## Risks / Trade-offs

- [A holding bot stops holding: any owned object can pin the avatar somewhere else for as long as it holds `tploc`, `sit` or `unsit`] → That is the feature, and the user chose it as the default. The held-back line says what waits and for whom; `rlv status` lists the holders; the persist override setting takes a sort of step out of it for good, and `rlv reset`, `rlv mode off` and `rlv mode override` followed by `home` get the bot out once.
- [With the persist override setting naming `tp`, no object can keep a holding bot away from its held place: a cage that moves the avatar sees it teleport back] → That is what the setting is for, it is empty by default, and it leaves the operator's commands restricted. `rlv status` shows it and every step it lets through is in the log.
- [Two things called override] → Named "persist override" everywhere it appears; README explains both side by side.
- [The operator locks the console out with `rlv cmd`: `rlv cmd @sendim=n` then wants to send a message] → `rlv cmd @clear`, `rlv reset` and override are never restricted.
- [Override stays armed: a forgotten `rlv mode override` exempts a command given hours later, perhaps by a script] → `rlv status` shows `rlv: override`, the log has the switch, `rlv mode on` takes it back. No time limit, since the bot has no clock-driven state elsewhere and a limit would need its own setting.
- [A command that restrictions do not concern uses the override up (`anim play`, `sendtp`)] → Accepted for a rule that fits in a sentence; the answer of the next refused command names override again.
- [Restrictions of an object the simulator never described stay after the object is deleted] → They end when the region is left or the session is lost, and `rlv reset` clears them. With `scan_distance` configured more objects are described.
- [An attachment that is not yet described when the avatar leaves a region loses its restrictions] → Only within seconds of a login or an arrival; scripts set their restrictions again on `on_rez` and `attach`, as the API tells them to.
- [Positions go stale: an object that moves without the bot reading its updates is taken to be where it was described] → `sittp` and `fartouch` are about furniture and buttons, which rest. Full, compressed and terse updates all pass through `Session::moved`, which now keeps the place of every described object (task 3.3, tested); what is left is an object whose update carries its motion in a form the bot does not read, which counts as of unknown place.
- [Under `sittp` and the touch restrictions the bot has to learn about objects it would otherwise leave alone: a scan starts in every region where such a restriction is applied to an undescribed object, and a command waits up to 10 seconds for its answer] → The scan is the one `touch` starts, with the same traffic and the same limit, and it starts only when a check has refused something for want of a description. An object beyond the distance the simulator is asked to mention (32 m, or `scan_distance`) is never described and stays too far, which for the default reach of 1.5 m it is.
- [A forced `@sit` that waits is carried out up to 10 seconds after it was said, by restrictions as they are then] → The log has both moments. A script that changes its mind in between sends `@unsit=force`, which is dealt with in order.
- [Distance for `sittp` is measured to the object's origin, a viewer measures to where the user clicked or to the sit target] → A large object can be nearer or farther than a viewer would say. Named in README.
- [`sendchat`, emote truncation and redirection differ between RLV viewers, and the page calls its own numbers "likely to change"] → The page's text is implemented as written and tested as written; the functions are small and in one place.
- [An empty `@getstatus` reply may not be delivered by the simulator, so a script waits for its timeout] → Viewers send it the same way. Task 8.3 checks on the rig what the relay hears.
- [`tplure` drops offers without telling the avatar who offered, where a viewer tells them] → Non-goal above; the operator log has every dropped offer.
- [Restrictions that are kept without an effect make `@getstatus` say the avatar is held to something it is not] → `rlv status` marks them, README lists them, and the API version says the implementation is incomplete. Not keeping them would make scripts that verify with `@getstatus` send them again and again.
- [1024 restrictions is a guess] → A fully restrained avatar in a viewer has well under a hundred; the bound is against a script in a loop, and the refusal is logged.
- [`Origin` and the store reach many signatures in `commands.rs`] → Mechanical, and the compiler finds every call. Passing them together in one small struct keeps the signatures readable.

## Migration Plan

None needed for configs: the one new key is optional, and with RLV off the bot behaves as before. With `rlv = true`, objects that sent restrictions all along are now obeyed after the upgrade; README's Upgrading section says so and names `rlv reset` and `rlv mode off`. Rolling back is the previous binary: nothing is saved.

## Open Questions

- Whether the API version should rise once `@notify` and the remaining queries exist, and to what. Not needed for this change.
- Whether `status` should show the RLV mode and a count of restrictions. Left out as in the first RLV change; adding a line later breaks nothing.
