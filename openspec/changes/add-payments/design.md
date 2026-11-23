# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- Every command is a `control::Request` that ends in `Bot::control_as(request, reply, own, now)` in `src/main.rs`. The control socket and the startup script send `Event::Control`, the XMPP gateway calls `Bot::control` from `xmpp_event`, the web interface sends `WebEvent::Command`. Nothing says today which of them a command came through, and `status_text` gives one text to all.
- A command that waits for the grid keeps its `Sender<Response>` in `Commands` (`pending_sit`, the teleport, the `waiting` list for name searches) and is answered from a session event. `Commands::find_avatar` resolves a name to a UUID with an exact match, as `im` needs it.
- The session learns objects from `ObjectUpdate` into sightings that carry `UpdateFlags` (`FLAG_SCRIPTED`, `FLAG_TOUCH`, ...); `Session::touch` and `look_for` start the scan and wait for an object up to 10 seconds. `RequestObjectPropertiesFamily` is already sent, for the owner of an object that asks for a permission.
- The grid's money messages, as the Kokua source and the message template have them:
  - `MoneyTransferRequest` (Low 311): source, destination, amount, a transaction type (5001 for a gift to an avatar, 5008 for paying an object) and a description. It carries no transaction id.
  - `MoneyBalanceReply` (Low 314): the balance, `TransactionSuccess`, a description, and a `TransactionInfo` block with type, source, destination, whether each is a group, the amount and an item description. The grid sends it in answer to `MoneyBalanceRequest` (Low 313) with an empty `TransactionInfo`, and by itself for every transaction of the avatar.
  - `RequestPayPrice` (Low 161) and `PayPriceReply` (Low 162): a default price and up to four button prices, with `-1` for hidden and `-2` for default.
  - There is no message that lists past transactions. Second Life shows them on its web site, behind a web login.
- OpenSim's stock money module (`SampleMoneyModule`) answers `RequestPayPrice` and `MoneyBalanceRequest` (always 0) and does nothing at all on `MoneyTransferRequest`. The rig cannot confirm a payment as it is.
- `msglog::Recorder` holds messages in arrival order until their names are settled, with one name cache and one path for lookups; `Logs` writes rows; `web::Hub` keeps two rings of 500 encoded events and replays them.
- `perms::Entry` already reads a UUID or a legacy name in three spellings and compares names in lower case; `config.rs` has the duration reader of `[autoreply]` and the deprecation notes.

## Goals / Non-Goals

**Goals:**

- Whether a payment may be made is decided in one place, inside the bot, before anything is sent: interface, form, balance, lists, limit. The page's dialog is a courtesy, not a safeguard.
- The limit cannot be reset by anything an interface can do, restarting the bot included.
- Everything that decides is pure code with the clock passed in: matching a payee, the period, replaying the file, reading a `MoneyBalanceReply` into a record.
- Payments are tried end to end on the rig, like every other feature.

**Non-Goals:**

- A ledger that survives the operator: whoever can edit the config or the transactions file can lift the limit. The limit is against the interfaces, not against the machine's owner.
- Exactly-once payments. The protocol has no id to make a payment idempotent; the design makes the uncertain case visible and never pays twice by itself.
- Reconciling the balance against the records.

## Decisions

### The interface travels with the command

A new `payments::Interface { Ctl, Xmpp, Web }` becomes a parameter of `Bot::control_as`. `Event::Control` (socket and startup script) passes `Ctl`, `xmpp_event` passes `Xmpp`, `web_event` passes `Web`. It is used in three places only: the gate of `pay`, `prices` and `payees`; the money lines of `status`; the operator log line of a payment. `Bot::status(via)` renders per interface, and the status published to the hub is `status(Web)`.

Alternative considered: put it into `commands::Origin`. `Origin` says who is behind a command for RLV and for the log tag (`Operator`, `Exempt`, `Page`, ...) and is rewritten on the way (an operator command becomes `Exempt`); the interface is orthogonal to that and would double the variants.

### `payments.rs`: settings, policy and the limit, without I/O

`PaymentsConfig { limit: Option<u32>, renew: Option<Duration>, perms: [bool; 3], whitelist: Option<Vec<Entry>>, blacklist, unlimited: Vec<Entry> }` is what `config.rs` produces; `whitelist` is an `Option` because an empty array and a missing key differ. The policy takes a `Payee { avatar: Option<(Uuid, Option<String>)>, object: Option<Uuid> }` (the avatar is the payee or the object's owner; `None` for a group's object) and answers `Refused(reason)`, `Unlimited` or `Counted`, in the order the spec gives: blacklist, whitelist, unlimited list, limit. `needs_name()` says whether any list has a name entry, which is the only case in which a missing name is a refusal.

The limit is `Ledger { period_start: Option<u64>, counted: u64 }` in wall-clock seconds, with `left(now)`, `renews_at(now)`, `reserve(amount, now)` and `release(amount)`. Wall-clock time, not `Instant`, because the period has to be the same before and after a restart and is printed as a date. A clock that jumps back makes a period longer, never shorter.

### The transactions file is the ledger

At startup `txlog::read(path)` returns the rows; `Ledger::replay(rows, renew, now)` walks the rows of kind `paid` and `unconfirmed` with `counted > 0` in file order: a row later than `period_start + renew` (or the first one) starts a period at its time, others add to it; at the end, a period that is over by `now` is dropped. Without `limit_renew` every such row adds to one period. The replay uses the `limit_renew` of the config the bot starts with, so changing it re-reads history under the new rule, which is predictable and needs no migration.

That is why a numeric `limit` requires `logs.transactions` (a config error otherwise), and why the `counted` column exists: whether a payment counted depends on `unlimited_list` at the time, which the file must remember.

Alternatives considered: a state file of its own (a second file to keep consistent with the log, a new key, and the web list would still want the log); memory only (a `quit` under a supervisor that restarts the bot hands out a fresh limit to whoever reached the web page).

A row that cannot be written would silently lift the limit after the next restart. `Logs` keeps such rows (they are few: one per payment) and `pay` retries them first; while one is unwritten, a payment that counts is refused. Payments to the unlimited list and with `limit = "unlimited"` go on, since they do not depend on the file.

### A payment is one slot in `Commands`, with three stages

`pending_pay: Option<Payment>` holds the reply channel, the interface, the amount, the payee as far as known, and a stage:

1. **Resolving.** For an avatar by name: `find_avatar`, as `im`. By UUID: a name lookup through the existing `wanted_names` path, 5 seconds; "the grid knows no avatar by that UUID" comes from the `missing` list of `names_resolved`. For an object: `Session::look_for` (scan and 10 seconds, shared with `touch`), then the money flag of the sighting or of its root, then `RequestPayPrice` and `RequestObjectPropertiesFamily` for the root prim together, 10 seconds for both; then the owner's name if a list needs it or it is not cached, 5 seconds.
2. **Checked and sent.** Balance, lists and limit are checked once everything is known, immediately before sending, so that a payment that waited cannot slip past a limit that another one used up. `Ledger::reserve` and the `MoneyTransferRequest` happen in the same step. For an object the destination is the UUID given and the description is the root prim's name, as the viewer's `LLFloaterPay::give` does.
3. **Waiting for the grid**, 15 seconds. Answered by the matching `MoneyBalanceReply`, by the deadline, or by the loss of the session.

One slot, not a queue: a second `pay` is refused while one waits. It makes the confirmation unambiguous (below), and the only caller that would queue is a script, which waits for each answer anyway.

`prices` uses stage 1 for an object and answers from it. `payees` asks the names of the UUID entries it has not cached and answers when they are in or 5 seconds have passed.

### Matching the grid's confirmation without an id

`MoneyTransferRequest` has no transaction id and the reply's id is the grid's own, so the confirmation is recognised by content: a `MoneyBalanceReply` whose `TransactionInfo` has the avatar as source and the amount of the payment in flight. The destination is not compared: for an object Second Life names the owner or a group there, not the object. With one payment in flight and one session per account, another outgoing transaction of exactly that amount in those 15 seconds is not a realistic collision; if it happened, the record would be right in amount and wrong in party.

`TransactionSuccess = false` on a match answers with the reply's description as the reason and releases the reservation. A reply after the deadline finds no payment in flight and is recorded as `spent`; the `unconfirmed` record keeps the count, so the limit errs on the side of having spent.

The balance check before sending mirrors the viewer's `can_afford_transaction`, which is why a viewer never sees how the grid refuses an overdraft; the bot does not depend on that answer either.

### Transactions go through the recorder

`msglog::Recorder` gets a second kind of entry: its queue holds `Message` or `Transaction`, `push_transaction(tx, now)` asks for the party's names through the same cache and lookup path, and `flush` returns `Recorded::Message(Record)` or `Recorded::Transaction(txlog::Transaction)` in arrival order. `Bot::service_messages` and `flush_messages` hand a transaction to `Logs::write_transaction` (standard error and the file) and to `Hub::transaction`.

Alternative considered: a second recorder in `txlog.rs`. It would need its own name cache and a second drain of lookups in `service_messages`, for a queue that holds a handful of entries a day.

`txlog.rs` owns `Transaction`, `Kind`, `COLUMNS` (twelve), `row()`, `log_line()`, `parse_row()` and `read()`, and the pure function that turns a `MoneyBalanceReply` and the payment in flight into `None` (balance only), a merged record, or a `received`/`spent`/`failed` one. The session only decodes: it emits `SessionEvent::Money { balance, success, description, info }` and keeps no money state; the balance lives in `Bot`, across sessions.

### Config: `logs` and `payments`

`Config::log_chat` and `log_im` become `Config::logs: LogPaths { chat, im, transactions }`, each with the key it was given under (`logs.chat` or `log_chat`) so that an error and `OpenError::key` name what the operator wrote. Both spellings of one file is an error; the deprecation note for the old keys prints the `[logs]` table to write. `Config::payments: Option<PaymentsConfig>` reuses `perms::Entry::parse` for the lists and the duration reader of `[autoreply]` for `limit_renew`. The cross-check "numeric limit needs `logs.transactions`" is in `Config::parse`; "the transactions file is readable and starts with the header" is where `Logs::open` runs in `run_bot`, before any network activity, and exits with `EXIT_CONFIG`.

### Objects that take money

`FLAG_MONEY = 1 << 9` joins the `FLAG_` constants of `udp/messages.rs`. `scan` prints it as `pay` after `touch` and filters on it with `--pay`. `pay object` and `prices` look at the sighting's flag and at the root's, as the viewer's pie menu does (`flagTakesMoney()` of the object or its parent).

`PayPriceReply` is read into `Offer { quick: Vec<u32>, other: Other }` with the viewer's rules: button `-1` dropped, `-2` replaced by 1, 5, 10, 20 by position; default price `-1` gives `Other::No`, `-2` gives `Other::Any`, a positive one `Other::Suggested(n)`.

### Web: a third ring, and the status decides what the page shows

`Hub` gets a ring for `money` events beside chat and IM, numbered in the same sequence so that `Last-Event-ID` keeps working, and is told at construction whether `web` may pay; if not, `Hub::transaction` drops what it is given. The rows read at startup are pushed into the ring before the server accepts. A `money` event's data is the twelve fields of the row, escaped like the others.

The page shows the Money tab and the Pay buttons when the last `status` event has a `limit` line. The status is already rendered per interface, reaches every page on connect and on every change, and carries the numbers the header shows, so there is exactly one place that says "web may pay". Alternative considered: a flag in the answer of `/api/session`; it would be a second source of the same fact and would not carry the balance.

`payees` joins `scan avatars` and `friends` in the list of commands accepted with `X-Rslbot-Own`, which keeps a page's own request out of the operator log and away from an RLV override. It is exempt from the override anyway.

The dialog is a `<dialog>` element opened with `showModal()`, which gives the modal behaviour, the Escape key and the focus trap the spec asks for without a library; its text is set with `textContent` like everything else. Focus starts on Cancel. The page parses the `limit` line's time with `Date` and prints it with `toLocaleString()`.

### A money module for the rig

`test-rig/money/RigMoneyModule.cs`, an `ISharedRegionModule` implementing `IMoneyModule`, selected with `economymodule = RigMoneyModule` under `[Economy]`. `setup.sh` copies it with a `prebuild.xml` into `test-rig/opensim/addon-modules/RigMoney/` before the build and sets the key in `OpenSim.ini` when it creates it. It keeps balances in memory, L$1000 for an avatar it has not seen, and:

- answers `MoneyBalanceRequest` with the balance;
- on `MoneyTransferRequest` moves the amount, or fails when the payer has too little, and sends both parties a `MoneyBalanceReply` with `TransactionInfo` filled in as Second Life does (type, source, destination, amount), through `IClientAPI.SendMoneyBalance`, which has those parameters;
- for type 5008 resolves the object, pays its owner and raises `OnObjectPaid`, so that a script's `money` event fires;
- answers `RequestPayPrice` from the root part's `PayPrice`, as the stock module does;
- has console commands `rigmoney set <first> <last> <amount>` and `rigmoney silent on|off`, the latter to move money without reporting it, for the unconfirmed case.

`test-rig/payee.py` uploads a script (as `relay.py` does) with `llSetPayPrice` and a `money` event that says what it got on a channel, and rezzes a box with it; it takes the prices as arguments. `test-rig/pay.py` logs in as another avatar, as `talk.py` does, and pays the bot an amount, so that a received payment can be tried.

Alternative considered: unit tests against the fake grid plus one manual L$1 on Second Life. It leaves the decode of the real messages, the object flag and the price reply untried on a simulator, and every later change to payments would need a live grid and real money.

## Risks / Trade-offs

- [A grid that reports nothing about payments, such as OpenSim with its stock module or some third-party ones] → every payment ends `unconfirmed` after 15 seconds and counts. The error says so in words; the README names it. The operator can check the balance in `status`.
- [A confirmation later than 15 seconds makes two rows, `unconfirmed` and `spent`] → the amount counts once; the web list shows both. Accepted over holding a payment open indefinitely.
- [The operator retries an `unconfirmed` payment that did go through] → the bot never retries by itself, the answer says the payment counts as paid, and the balance in `status` shows whether it left.
- [The transactions file is moved, truncated or edited] → the limit starts afresh. Documented as what the file is: the ledger. Log rotation of that file is the operator's decision to renew the limit.
- [Reading a long file at every start] → one linear pass over a few bytes per row; 100 000 rows are a few MB. The ring keeps 500.
- [Web interface without a password and `web` in `perms`] → allowed, as the page already controls the avatar there, but the start line warns, and the limit still holds.
- [A page of another site makes the browser pay] → already closed by `web-server` ("Requests of other sites are refused"); `pay` is one more `POST` under the same checks.
- [Login names change] → rare and by the grid's hand only; an entry by UUID is the stable form, and the README says so. Display names are never compared.
- [Wall clock set back] → a period lasts longer; never shorter.
- [`status` differs by interface] → scripts that parse `rslbot ctl status` see one new line (`avatar`) in a fixed place and, only with `ctl` in `perms`, two at the end.

## Migration Plan

- `log_chat` and `log_im` keep working. A config that has them logs, at each start, the `[logs]` table to write instead; README "Upgrading" gets an entry. Since `[logs]` is a table, it goes after the top-level keys, like `[xmpp]` and `[web]`; the note says so.
- Nothing pays until the operator writes a `[payments]` table. Rolling back is removing the table; the transactions file can stay.
- A rig set up before this change needs `test-rig/setup.sh` run again to build the money module, and `economymodule = RigMoneyModule` added by hand to an existing `OpenSim.ini`, which `setup.sh` never overwrites; `docs/test-rig.md` says so.

## Open Questions

- Which destination and type Second Life puts into `TransactionInfo` for a payment to an object, and whether it reports a refused payment with `TransactionSuccess = false` or only as an alert. Neither changes the design (the match ignores the destination; an alert leads to `unconfirmed`); one L$1 payment each to an avatar and to a tip jar on Second Life settles the wording of the README.
