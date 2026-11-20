# Tasks

## 1. Rules, entries and matching (`src/perms.rs`)

- [x] 1.1 Add `src/perms.rs` with `Kind` (`tp`, `inventory`, `anim`, `attach`, `controls`, `camera`; name, parsing, the fixed order) and the table from script permission bits to kinds (`0x4`, `0x10`, `0x20`, `0x400`, `0x800`, `0x8000`), with a function that turns a bit mask into its kinds or names the first bit that has none
- [x] 1.2 Move the name normalisation of `parse_recipient` (`src/commands.rs`) to where `perms.rs` can use it too, and add `Entry` (`Id`, `Name`) with parsing from one argument (UUID, `First Last`, `first.last`, single word; refusing empty text and names of more than two words) and display
- [x] 1.3 Add `Rule` (`All`, `List`) and `Rules` (rule and exceptions per kind), with the four changes of the spec's table (`autoaccept all`, `autoaccept who`, `revoke all`, `revoke who`), each returning what changed or the warning
- [x] 1.4 Add `Sender` (avatar; object with optional UUID, name, and owner as avatar with optional name, group, or unknown) and `Entry::matches`, following the table in "Whom an entry stands for"
- [x] 1.5 Add the decision: `Rules::accepts(kinds, sender)` is true when every kind is `All` or has a matching entry, and has no matching exception
- [x] 1.6 Add the status text: one line per kind (`all`, `none`, entries, `except ...`)
- [x] 1.7 Unit tests for 1.1 to 1.6: every row of the matching table, the group named like an avatar, name spellings, each row of the command table with and without its warning, all-or-nothing over two kinds, exceptions under `all` and under a list, the status lines of the spec's scenario

## 2. Config

- [x] 2.1 Parse the `perms` table in `src/config.rs` into `Rules`: `autoaccept` values `"all"`, `true`, `false` and arrays of entries; `except` arrays; duplicates count once
- [x] 2.2 Reject, naming the key (`perms.autoaccept.<kind>`, `perms.except.<kind>`): a `perms` that is no table, unknown keys at both levels, values of the wrong type, malformed entries, `"all"` inside an array, `"all"` or a boolean under `except`, and one entry in both arrays of a kind
- [x] 2.3 Unit tests for 2.1 and 2.2, one per scenario of the `bot-config` delta
- [x] 2.4 Add a commented `[perms.autoaccept]` and `[perms.except]` block to `russelbot.example.toml`

## 3. Reading the requests

- [x] 3.1 In `src/udp/messages.rs`, read `FromGroup` and the binary bucket of `ImprovedInstantMessage`, and decode `ScriptQuestion` (Low 188: `TaskID`, `ItemID`, `ObjectName`, `ObjectOwner`, `Questions`; the `Experience` block may be absent) and `ObjectPropertiesFamily` (Medium 10: `ObjectID`, `OwnerID`, `GroupID`)
- [x] 3.2 In `src/udp/messages.rs`, build `ScriptAnswerYes` (Low 132), `RequestObjectPropertiesFamily` (Medium 5), and an instant message with a given dialog, id and binary bucket (for dialogs 5 and 10)
- [x] 3.3 In `src/session.rs`, turn dialogs 4 and 9 into `SessionEvent::InventoryOffer` (giver or owner UUID, name, from-group flag, from-object flag, transaction id, asset type, offered item UUID when given, text) instead of ignoring them, and a malformed bucket into a debug line
- [x] 3.4 In `src/session.rs`, turn `ScriptQuestion` into `SessionEvent::PermissionRequest` and `ObjectPropertiesFamily` into `SessionEvent::ObjectOwner`, and add `Session` methods to ask for an object's owner, to answer a script question with a mask, and to answer an inventory offer with a destination folder
- [x] 3.5 Add those three methods to the `Grid` trait, to `Live` in `src/main.rs` and to `FakeGrid` (as recorded calls)
- [x] 3.6 Unit tests: wire round trips for the new messages, and session tests that an inventory offer from an avatar, one from an object, one from a group's object and a script question each produce their event; the "Inventory offer" scenario of `message-log` (nothing recorded) still passes

## 4. Waiting requests and deciding

- [x] 4.1 Add `Waiting` to `src/perms.rs`: teleport offers by avatar (lure id and name), script requests by object and script item, inventory offers in arrival order; each bounded at 100 dropping the oldest; lookup of the latest by kind and optional `Entry`; removal; `clear` and `clear_scripts`; the `waiting:` status lines
- [x] 4.2 Replace `Commands::offers` with the teleport part of `Waiting`, keeping the `requesttp` check first; forget everything where the offers are cleared today, and script requests on a region change
- [x] 4.3 Script requests: on `PermissionRequest`, refuse and log a mask with a bit that has no kind; otherwise ask for the object's owner and hold the request as undecided until `ObjectOwner` arrives or `LOOKUP_LIMIT` passes (through the existing deadline tick), then decide: grant the whole mask, or move it to `Waiting`
- [x] 4.4 Inventory offers: decide at once unless the offer is from an avatar's object; in that case add the owner to the display-name batch `Bot` builds from `Recorder::take_lookups`, pass the reply to both, and decide when the name arrives or `LOOKUP_LIMIT` passes
- [x] 4.5 Add `What::SystemFolder(asset_type)` to the inventory worker: one fetch of the top folder's folders, returning the child whose `type_default` is the asset type, else the top folder; unit test against the worker tests' fake grid (the recorded OpenSim reply holds no system folders)
- [x] 4.6 Accepting an inventory offer: start the `SystemFolder` job, and on its answer send dialog 5 or 10 with the transaction id and the folder; on failure (no inventory service, teleport under way, timeout) leave or put the offer into `Waiting` and log the reason
- [x] 4.7 A teleport offer accepted by rule goes the way of `accept tp`: manual mode first, then `accept_teleport`; when that cannot be done now the offer waits
- [x] 4.8 Operator log lines for every request read (who, what, and accepted by itself, waiting with its `rslbot ctl accept <kind>` command, or not acceptable and why) and for an acceptance that failed
- [x] 4.9 Unit tests with `FakeGrid`: each scenario of "Deciding a request", "Learning who is behind a request", "Accepting a request" and "Requests that wait" that needs no live session, including the owner lookup timing out, the name lookup timing out, two kinds with one rule missing, a repeated request replacing the waiting one (also when the newer one is accepted), and a rule change leaving a waiting offer alone
- [x] 4.10 Bot-level tests: a teleport offer accepted by rule puts a holding bot in manual mode (the offer is handed to the bot's commands with the tests' fake grid, as a bot under test has no session; the path through a live session is rig check 6.3); an offer that waits leaves it holding; granting a script request or accepting inventory while holding changes nothing

## 5. Commands

- [x] 5.1 In `src/control.rs`, add `Request::Perms(PermsRequest)` (`Autoaccept`, `Revoke`, `Status`) and `Request::Accept { kind, who }`, make `accepttp` decode to `accept tp` with an optional argument, give each malformed form its own error text, and extend `CTL_USAGE` (the `perms` and `accept` lines, a paragraph on them, `accepttp` named as deprecated)
- [x] 5.2 Answer the `perms` requests in `Bot::control` whether or not a session exists: one line per `who`, `warning:` lines for no-ops, the status text followed by the waiting lines
- [x] 5.3 `accept tp [who]` in `Commands`: the latest offer, or the latest the entry matches, without a grid lookup; the existing teleport wait and answers; errors `no teleport offer waits` and `there is no teleport offer from <who>`
- [x] 5.4 `accept inventory [who]`: through 4.6, answered when the answer has gone out, with an error that leaves the offer waiting when the folder cannot be found
- [x] 5.5 `accept anim|attach|controls|camera [who]`: the latest waiting script request that has the kind and that the entry matches; grant its whole mask; answer naming the object, the owner and every permission granted
- [x] 5.6 Log lines for every `perms` and `accept` command carried out or refused
- [x] 5.7 Unit tests: decoding of every new command form and its errors; each scenario of "Perms autoaccept and revoke commands", "Perms status command" and "Accept command"; the changed and added scenarios of the `avatar-commands` delta, among them `accept tp` without a name taking the latest offer, by UUID, with a name the grid does not know, and `accepttp` with and without an argument

## 6. Test rig

- [x] 6.1 Add `test-rig/asker.py`: logs in as `Rig Owner` and rezzes two boxes with one script that, on anyone's chat on the box's channel (78 and 79), asks the speaker for the permission bits the message gives and says on channel 0 what `run_time_permissions` reported, and on `give` gives the speaker a copy of the script (an object's inventory offer; a script instead of a notecard, since the box has it already); it prints the boxes' UUIDs for the rules that name an object
- [x] 6.2 Let `Rig Owner` give the bot an inventory item and print the accept or decline message it gets back: `asker.py SECONDS give=N nobox`, in `asker.py` and not in `listen.py`, because the item to give is the script `asker.py` uploads and the account cannot be logged in by two helpers at once
- [x] 6.3 Add a `## Permission requests` section to `docs/test-rig.md` with numbered checks from 125 on, and run them on the rig with a scratch config and a scratch `RUSSELBOT_SOCK`: rules from the config shown by `perms status`; a teleport offer from a listed avatar accepted and the bot left in manual mode; an unlisted one waiting and accepted with `accept tp` without a name; `accepttp` still working; an animation request from a listed owner granted (the box reports the permission), from an unlisted one waiting and granted with `accept anim`; a request for animations and controls with only `anim` allowed waiting whole; a debit request refused; `anim = "all"` with the box's UUID excepted; `perms autoaccept` and `revoke` with their warnings; an inventory offer from an avatar and one from the box accepted by rule (the giver is told, the notecard is in Notecards) and by `accept inventory`; a session loss emptying the waiting list
- [x] 6.4 Update check 48 and 49 of `docs/test-rig.md` to the new command and its answers

## 7. Documentation and checks

- [x] 7.1 `README.md`: the `perms` row of the config table and a section on the two tables with the example; a `### Permission requests` section (kinds, whom an entry stands for, why an object's UUID does not work for `inventory`, all-or-nothing, what waits); the control-socket rows for `perms autoaccept`, `perms revoke`, `perms status`, `accept KIND [WHO]`; `accepttp` marked deprecated; the passages that name `accepttp` (manual mode, `requesttp`) changed to `accept tp`
- [x] 7.2 Record in the README's status section what the rig run of 6.3 showed, and that the inventory answers and `ScriptAnswerYes` are untried on Second Life until someone has tried them there
- [x] 7.3 `cargo test`, `cargo clippy --all-targets` without warnings, `cargo build --release`, and `openspec validate add-permission-autoaccept --strict`
