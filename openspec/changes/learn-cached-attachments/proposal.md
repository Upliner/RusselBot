# Proposal

## Why

On Second Life the bot can miss an attachment that is on its avatar. In the hand check of 2026-11-19 an object the bot had reported at one login, and that the Current Outfit folder still listed, was not reported at the next login seven minutes later; `attach` of that item was then carried out and confirmed instead of refused. Until this is fixed, `inventory -a`, `detach <body part>` and the refusal to attach a thing twice are only as good as what the bot happens to have seen there.

The likely cause is the risk `add-inventory-commands` named in its design: a simulator may mention an object as one the viewer has cached (`ObjectUpdateCached`) instead of describing it, and the bot asks about such objects only once a `touch` has started an object scan. That session's log shows 29 such messages, each of a size that holds about a hundred objects, no request about any object but the seat, and no object description the bot failed to read. The log cannot prove the missed attachment was among the objects mentioned, so this change also makes the bot say enough with `--verbose` to settle that on the next try.

## What Changes

- The bot asks the simulator to describe every object it mentions as cached and marks as owned by the bot's own account, as soon as it is mentioned, without waiting for a scan. An avatar's attachments always belong to its account, so this covers them; the mention itself does not say which objects are attachments, so everything of the account's own in view is asked about.
- Objects of other owners are treated as before: asked about only from the start of an object scan.
- **BREAKING** (traffic): a bot given no command now sends requests it did not send before, where the simulator mentions objects of the account's own as cached. On the local OpenSim, which describes attachments in full, nothing changes.
- With `--verbose` the bot logs, for each such message, how many objects it mentioned, how many of them are the account's own, and how many it asked about.
- README and `docs/test-rig.md` say what the bot now sends and why, and the Status paragraph is brought up to date after the check on Second Life.

This change builds on `add-inventory-commands`, which introduces the `avatar-outfit` capability and has to be archived before this one can be.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `avatar-outfit`: "Knowing what is attached" no longer says the bot sends nothing to learn what is attached; it asks about cached objects of the account's own.
- `avatar-commands`: "Object scan" gets a second exception to "nothing is sent for the sake of learning objects before a scan": the request about cached objects of the account's own.

## Impact

- `src/udp/messages.rs`: the decoder for `ObjectUpdateCached` keeps each object's flags instead of dropping them.
- `src/session.rs`: handling of cached mentions, the set of objects asked about, verbose lines; tests with the fake network.
- `README.md` (Attachments section, Status), `docs/test-rig.md`.
- No new command, config key or dependency. No change to what the control socket answers, except that answers depending on what is attached are right more often.
- Traffic on Second Life: one `RequestMultipleObjects` per 255 owned cached objects and the simulator's descriptions of them, once per region entered. Small where the account owns little besides its attachments; noticeable on land where the account owns thousands of objects.
