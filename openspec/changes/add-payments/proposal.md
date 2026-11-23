# Proposal

## Why

The bot's avatar cannot pay: a vendor, a tip jar, a rental box or another avatar needs L$ from it, and today the operator has to log in with a viewer for that, which throws the bot off the grid. The bot also says nothing about money it receives or about what the avatar has. Paying is the one thing an operator would not hand to every interface without a ceiling, so it comes with its own switch, a spending limit and lists of who may be paid. The record of what was paid needs a file, which is the moment to give the three log files one table in the config.

## What Changes

- **Payments are off by default.** A new `[payments]` table switches them on. It has two mandatory keys: `limit`, a number of L$ or `"unlimited"`, and `perms`, an array of the interfaces that may pay: `"ctl"` (the control socket and the startup script), `"xmpp"` (the control contact) and `"web"` (the page and its Console). Without the table, and for an interface that is not listed, every payment command is refused.
- **Limit.** `limit` is what the bot may pay in one period. `limit_renew`, a duration such as `"7d"` or `"12h"`, is the length of that period: it starts with the first payment that counts, and when it ends the whole limit is back. Without `limit_renew` the limit never renews. The limit survives restarts: the bot restores what was spent from the transaction log, so a `limit` that is a number needs `logs.transactions`.
- **Lists.** `whitelist`, `blacklist` and `unlimited_list` are optional arrays of avatars and object UUIDs. With a `whitelist` only who is on it (or on `unlimited_list`) can be paid; who is on `blacklist` is never paid; a payment to who is on `unlimited_list` does not count against the limit. An avatar is written as a UUID or as its login name (`First Last`, `first.last`, one word), never as a display name, which its owner can change at will. An avatar on a list stands for the objects it owns too.
- **Commands**, for an interface that may pay:
  - `pay <amount> avatar <name|uuid>` pays an avatar.
  - `pay <amount> object <uuid>` pays an object in the region, as a viewer's Pay does: only an object that takes money, and only an amount it offers.
  - `prices <object-uuid>` prints the object's name, its owner and the amounts it offers.
  - `payees` lists the avatars the `whitelist` and `unlimited_list` name, for the page to choose from.
  A payment is answered when the grid has confirmed or refused it, or after 15 seconds without either, and then counts as paid.
- **`status`** gets a line `avatar` with the avatar's name after `connected`, always, and at its end the lines `balance` and `limit`, the latter with `(renews to <amount> on <time>)` when part of a limit with `limit_renew` is used. The two money lines are shown only to an interface that may pay.
- **Transactions.** Every payment the bot sends and every transaction the grid reports to the avatar while it is online (a payment received, an object's payout, a fee) is recorded: on standard error, and in the file `logs.transactions` when set. A payment of the bot and the grid's confirmation of it are one record. The grid has no request by which a viewer can fetch past transactions, so what happened while the avatar was offline is not recorded; the balance the bot shows is the grid's.
- **Web interface**, when `"web"` may pay:
  - a **Money** tab between Inventory and Console: a header with the avatar's name, the balance and the limit with its renewal; a "Send money" form that takes any name or UUID, or one chosen from the friends or the whitelist; and the list of transactions, live, with the newest 500 of the transaction log after a restart;
  - a **Pay** button in an avatar's conversation on the IM tab, which leads to the Money tab with that avatar filled in;
  - a **Pay** button on the rows of the object scan that take money, with a drop-down of the amounts the object offers;
  - a **confirmation dialog** before every payment the page's buttons send, with the amount and the payee, and for an object its name and its owner's name. A `pay` typed into the Console, `rslbot ctl` or XMPP pays at once, as every typed command does.
- `scan` marks objects that take money with the flag `pay`.
- **Logs.** A new `[logs]` table holds the paths of the three files: `chat`, `im` and `transactions`. `log_chat` and `log_im` are deprecated: they still work, the bot says at startup what to write instead, and a config with both spellings of one file is rejected.
- The rig gets a money module for its OpenSim, whose own one moves no money and confirms nothing, and a helper that rezzes a paid object.

Not in this change: payments to groups, buying objects, land or passes, a message with a payment, accepting a script's request to take money from the avatar (still always refused), fetching the transaction history of the Second Life web site, transactions shown over XMPP, changing the limit or the lists while the bot runs, and a confirmation step for typed commands.

## Capabilities

### New Capabilities

- `payments`: who may pay and how much: the interfaces of `payments.perms`, the limit and its period, the three lists and how a payee is matched against them; the commands `pay`, `prices` and `payees`; how a payment is sent, confirmed, refused or left unconfirmed; the balance; what the operator log says.
- `transaction-log`: recording the avatar's transactions: which reports of the grid are transactions, the kinds and fields of a record, the names in it, the line on standard error, the TSV file, and what the bot reads back from that file when it starts.

### Modified Capabilities

- `bot-config`: the `[payments]` and `[logs]` tables, their validation, the deprecation of `log_chat` and `log_im`.
- `message-log`: the chat and IM files are named by `logs.chat` and `logs.im`, or by the deprecated keys.
- `avatar-commands`: `status` has the `avatar` line and, for an interface that may pay, the `balance` and `limit` lines.
- `area-scan`: the flag `pay` in a listing and the filter `--pay`.
- `xmpp-gateway`, `startup-script`: a command is carried out for the interface it came through, which decides whether it may pay and what `status` shows; the control contact is `xmpp`, the startup script counts as `ctl`.
- `web-server`: commands and console lines count as `web`; the status sent to the pages is the one `web` gets; transactions on the event stream, kept and replayed as messages are; `payees` as a request the page makes by itself.
- `web-ui`: the Money tab and when it is there, the Pay buttons on the IM tab and in the object scan, the confirmation dialog, the status panel's new line.

## Impact

- New `src/payments.rs`: the settings, the interface, matching a payee against the lists, the limit and its period (pure, with the clock passed in), the waiting payment. New `src/txlog.rs`: the transaction record, its row and log line, reading the file back; `src/msglog.rs`: the recorder holds transactions beside messages, `Logs` writes the third file.
- `src/config.rs`, `russelbot.example.toml`: the two tables, the deprecation notes, `Config::payments`, `Config::logs`.
- `src/udp/messages.rs`, `src/session.rs`: `MoneyTransferRequest`, `MoneyBalanceRequest`, `MoneyBalanceReply`, `RequestPayPrice`, `PayPriceReply`; the money flag of an object; session events for a balance and a transaction.
- `src/control.rs`, `src/commands.rs`, `src/main.rs`: the three commands and their usage; the interface passed along with every command; `status_text` with the avatar and the money lines; restoring the limit at startup; transactions handed to the log, the hub and standard error.
- `src/scan.rs`: the `pay` flag.
- `src/web/hub.rs`, `src/web/mod.rs`, `src/web/assets/*`: the `money` event and its ring, `payees` as the page's own, the Money tab, the Pay buttons, the dialog.
- `src/xmpp/gateway.rs`: commands marked as coming through XMPP.
- `README.md`: a "Payments" section, the `[logs]` table in "Chat and IM logs", the deprecated keys, the Money tab; `docs/test-rig.md`, `test-rig/`: the money module, a helper for a paid object, rig items.
- Dependencies: none new.
- Risk: the bot can now spend the avatar's L$. Everything is off without `[payments]`; the limit is checked before anything is sent and is not reset by a restart.
