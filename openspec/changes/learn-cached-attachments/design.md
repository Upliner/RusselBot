# Design

## Context

See proposal.md for the motivation. Sources below: K = `references/kokua/indra`, O = `references/opensim/OpenSim`.

What the protocol carries:

- `ObjectUpdateCached` (High 14) has, per object, `ID` (the region-local id, U32), `CRC` (U32) and `UpdateFlags` (U32), and nothing else: no object UUID, no parent, no attachment point (K `../scripts/messages/message_template.msg:3374-3389`). So a mention does not say whether the object is on the avatar.
- The viewer looks each mention up in its object cache by local id and CRC. On a miss it asks with `RequestMultipleObjects`; on a hit it takes the mention's `UpdateFlags` as the object's flags (K `newview/llviewerobjectlist.cpp:771-815`, `newview/llviewerregion.cpp:2905-2965`, `newview/llviewerobject.cpp:2485-2493`). So the flags in a mention are the object's flags as they stand for the receiving agent, the same as in a full update.
- `FLAGS_OBJECT_YOU_OWNER` is bit 5, `0x20` (K `llprimitive/object_flags.h:36`). OpenSim fills a mention's flags from the same per-agent function as a full update's, which sets that bit for the owner (O `Region/ClientStack/Linden/UDP/LLClientView.cs:5319-5350`, `Region/CoreModules/World/Permissions/PermissionsModule.cs:715`).
- OpenSim mentions an object as cached only if it is not an attachment and has not changed for five minutes (O `Region/Framework/Scenes/SceneObjectGroup.cs:192-203`), and only for a viewer that announced a cache. The rig's bot never gets such a message (`docs/test-rig.md`, "Not covered on this rig"). Second Life's simulator is closed; that OpenSim's rule is "unchanged for a while" suggests, and does not show, why an unscripted attachment would be mentioned there while scripted ones, which change themselves when rezzed, are described.

What the bot does today:

- The decoder keeps only the local ids of a mention (`src/udp/messages.rs:931-942`).
- `Session` puts ids it has no description for into `undescribed` (at most 20,000) and asks about them, 255 to a `RequestMultipleObjects`, only from the start of a scan (`src/session.rs:1062-1070`, `633-646`). A scan starts at region entry with `scan_distance`, else at the first `touch` of an unknown object.
- An object's description is read in both forms, full and compressed, including the attachment point and the `AttachItemID` name-value (`add-inventory-commands`, task 1.3). Which form Second Life answers a request in is not known: the 2026-11-19 log has one request (the seat) and no compressed update at all.

What the 2026-11-19 log says: 29 `ObjectUpdateCached` messages of 1194 bytes in the first nine seconds and four more later; five attachments reported at login, each right after a full `ObjectUpdate`; the sixth, reported at the previous login, not reported; no `dropping ...` line, so no update was received and found unreadable. That leaves two explanations: the object was only mentioned as cached, or the simulator said nothing about it at all.

## Goals / Non-Goals

**Goals:**

- The bot learns of an attachment the simulator only mentions as cached, without a command having to ask for it.
- A bot whose simulator mentions nothing of the account's own as cached sends exactly what it sends today.
- The next verbose log from Second Life shows whether cached mentions were the cause.

**Non-Goals:**

- Scanning the region by default, or changing when a scan starts.
- A cache of object descriptions between sessions, or use of the CRC to tell whether a description is still good.
- Making commands wait for the attachment table to be complete. There is no signal that it is.
- A config key to switch the new requests off.

## Decisions

### Ask about cached objects the account owns, and only those
`Session` asks at once about every mentioned object whose flags have `0x20` set and which it has no description of. An avatar's attachments are owned by its account, so they are among these. Nothing narrower can be read from a mention.

*Alternative: ask about every cached object (start the scan at entry).* Certain to cover attachments, but it is what `scan_distance` exists to make optional: the log shows some 2,800 mentions in one region, and the descriptions asked for are the bulk of a region's object traffic.

*Alternative: ask only when an outfit command needs the table.* Keeps a silent bot silent, as `touch` does for the scan. But `touch` waits for one named object, and here nothing says when the answers are complete, so every `inventory -a`, `attach` and `detach <body part>` would need a wait with a guessed length; and the log's lines for attachments that come and go by themselves would be wrong until the first command. The owner filter keeps the cost small enough not to need this.

*Alternative: none, and document the gap.* Leaves three answers of the control socket unreliable on the grid the bot is mainly for.

### Ask once per object in a region
`Session` keeps the local ids it has asked about in a set of their own (not in `undescribed`), bounded like `undescribed`, emptied with the rest of the simulator state. An id leaves the set when the simulator reports the object killed, since local ids are reused. An id is not asked about when it is in the set or already described. When a scan starts, it asks about `undescribed` as now; ids in the new set are not in `undescribed`, so they are not asked about twice. During a scan every mention is asked about as now, and owned ids go through the same path so that a later mention is not asked about again either.

The CRC is not kept. The viewer asks again when a mention's CRC differs from its cached copy's; the bot holds what the simulator last described and relies on changes arriving as updates, which is how it already treats every object. If Second Life turns out to announce a changed attachment only by a mention with a new CRC, that is a follow-up.

### The decoder hands over the flags
`Incoming::ObjectsCached` carries, per object, the local id and the `UpdateFlags`; `Session` tests the owner bit. Keeping the raw flags rather than a boolean leaves the decoder a plain reading of the message and lets the verbose line count by flag. The `messages::testing` builder takes the flags too, with a helper for "owned".

### Requests go out as the mentions come in
One `RequestMultipleObjects` per 255 ids per message handled, reliable, as `request_objects` already sends them. No batching across messages and no delay: a login mentions own objects in a handful of messages, and waiting would only widen the time in which the table is incomplete.

### Commands do not wait
An attachment asked about and not yet described is not known yet, the same as one the new simulator has not shown yet after a teleport, which the existing scenario already allows for. In practice the request leaves in the first seconds after region entry.

### Verbose lines
For each `ObjectUpdateCached`: `objects: N mentioned as cached, M of them the account's own, asking about K` (debug level, so only with `--verbose`). The existing `attachment: item ... is on ...` line then shows what came of the answers. Together they answer the open question from Second Life: if a login misses an attachment while `M` is 0 throughout, cached mentions are not the cause.

## Risks / Trade-offs

- [The missed attachment was not a cached mention: Second Life said nothing about it at all] → The verbose line shows it (no own object mentioned, attachment still missing). Then this change is harmless and insufficient, and the cause has to be looked for in what the simulator sends at login; task 4.2 records which it is.
- [Second Life does not set the owner bit in a mention] → Same symptom as above, `M` always 0 with attachments missing. The remedy would be asking about every mention, which is the rejected alternative and a change to this change's specs, to be decided then with the log in hand.
- [Second Life answers the request with a compressed update the bot cannot read, or one without the `AttachItemID` name-value] → An unreadable one leaves a `dropping ObjectUpdateCompressed` line; one without the name-value leaves the object learned and no attachment reported. Both are visible in the verbose log of task 4.2. The compressed layout has unit tests but has never met Second Life.
- [An account that owns thousands of objects in the region makes the bot ask about all of them at every entry] → Bounded by the 20,000 cap; some 4 bytes of request and one description per object, once per region. Stated in README. A switch can be added if it turns out to matter.
- [The bot now learns of the account's own objects in the world, so `touch` on one of them succeeds without a scan] → Consistent with "has learned of it"; no spec text depends on it failing.
- [The rig cannot produce the case] → Unit tests with the fake network cover the session logic; the proof is the hand check on Second Life.

## Migration Plan

None: no config or stored state changes. Rollback is reverting the commit.

## Open Questions

- In which form Second Life answers `RequestMultipleObjects` for an attachment. Either is read; task 4.2 notes which was seen.
