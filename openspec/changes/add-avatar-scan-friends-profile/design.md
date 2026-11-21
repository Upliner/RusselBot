# Design

## Context

See `proposal.md` for the motivation. What the code has today, and what shapes the approach:

- `scan` is `Request::Scan(Vec<String>)` (`src/control.rs`), read by `scan::Query::parse` and parked in `Commands::scanning` until `scans_due` answers it (`src/commands.rs`). `src/scan.rs` is free of the session: it works on `Sighting`s that `Session::sightings()` reads out.
- The session already learns avatars: every `ObjectUpdate` with `PCODE_AVATAR` is stored like a prim (`objects`, `local_ids`, `kinds`, `places`, `parents`), moved by terse updates and dropped by `KillObject`. But the simulator only describes what is within the draw distance, which is `scan_distance` or 32 m (`DRAW_DISTANCE`), kept short on purpose.
- `CoarseLocationUpdate` (Medium 6) is sent to every agent unasked, every few seconds, for the whole region. The bot receives it today and drops it as `Incoming::Other`; the same goes for `OnlineNotification` (Low 322). The circuits of regions the avatar has left only answer pings and acks, so nothing a left region sends reaches the session's handlers.
- `Session` lasts for a login; `Sim` (inside it) for a stay in a region. `login::SessionInfo` carries what the login reply gave; `buddy-list` is in the reply (the rig's fixture has an empty one) and is not read.
- Names: `names.rs` resolves UUIDs through `GetDisplayNames` on a worker thread (legacy and display name); `Commands::wanted_names` / `names_resolved` feed it. `scan` keeps the owners' legacy names in `Commands::owners`, bounded by `MAX_OWNER_NAMES`. `im` resolves a name to a UUID through `Waiting` and `After`, with `AvatarPickerSearch` or the UDP picker, and a 10 second `LOOKUP_LIMIT`.
- Capabilities come from the seed request in `caps.rs`; a URL reaches the main loop as a `CapsEvent`. HTTP is done on worker threads that answer with a `net::Event`, never in the main loop.
- RLV: `src/rlv.rs` keeps every restriction; `ENFORCED` lists those with an effect and the rest show as `(no effect)` in `rlv status`. `Limits` answers the questions commands ask (`in_force`, `against(behaviour, Target::Id)` with the exception and `_sec` rules, `reach` for the smallest distance of `sittp` and `fartouch`); `Commands::limits(grid)` gives the limits for the command at hand, none for an exempt one. `Bot::control` has a list of commands that never use up an override, and `scan` is on it because no restriction applied to it.
- The reference viewer under `shownames`, `shownametags` or `shownearby` hides its Nearby list and writes dummy names for owners in area search (`llpanelpeople.cpp`, `fsareasearch.cpp`); under `shownearby` it drops the avatar dots from the minimap (`llnetmap.cpp`). Its `camdrawmax` is the smallest held, and blinds only when `camdrawalphamax` (the largest held, 1 when none) is at least 0.99 (`RRInterface::updateCameraLimits`).
- The reference viewer (`references/kokua`) reads profiles from the `AgentProfile` capability (`GET <cap>/<uuid>`, LLSD) and no longer sends the UDP `AvatarPropertiesRequest` for them; OpenSimulator (`references/opensim`) has no such capability and answers the UDP request with `AvatarPropertiesReply`.

## Goals / Non-Goals

**Goals:**
- A list of everyone in the region that costs the simulator nothing: no wider draw distance, no object scan.
- Exact distances where the simulator has given exact positions, so that `whisper` and `chat` are right at their edges.
- One place that knows friends and who is online, usable by `friends` and by `profile`.
- Profiles on Second Life and on OpenSimulator from one command and one answer format.
- Listing and parsing code free of the session, unit-tested with plain data, as `scan.rs` is.
- Scans that tell a held avatar no more than an RLV viewer's radar, minimap, area search and sight would.

**Non-Goals:**
- Filters on `scan avatars` other than the distance (name, friend, group); arguments to `friends`.
- Avatars of neighbouring regions, and telling which parcel an avatar is in.
- Raising the draw distance to get exact positions of far avatars.
- Treating an avatar that is in the region as online in `friends` and `profile`.
- A profile's picture, groups, picks, classifieds, notes, interests, payment and account type; the partner's name; changing any profile.
- Offering, accepting or ending friendships, changing friends' rights, tracking a friend on the map.
- Log lines or XMPP presence for friends coming online and going offline.
- RLV restrictions on `friends` and `profile`, and hiding names in the message log or in `status`.
- `showloc`, `camdrawmin`, `camavdist`, `showhovertext` and the other camera and display restrictions on the scans; a vision limit on `scan avatars`.
- Dummy names ("A resident") for hidden avatars: a hidden field is `?`, like any other the bot does not tell.

## Decisions

### 1. The region's avatars from `CoarseLocationUpdate`

Decode it into `Incoming::AvatarsHere(Vec<(Uuid, [u8; 3])>)`: `Location Variable { X, Y, Z U8 }`, `Index { You S16, Prey S16 }`, `AgentData Variable { AgentID }`, pairing the two variable blocks by index. A message without the `AgentData` block (older simulators) decodes to an empty list. `Sim` keeps the last one as `Option<Vec<..>>`: `None` until the first arrives in the region, which is what "the simulator has told who is in the region" means in the spec.

Alternatives: raising the draw distance to the region's size (heavy: the simulator then describes every object in the region; this is exactly what `DRAW_DISTANCE` avoids); the map's agent counts (`MapItemRequest` gives counts, not ids). Viewers' radars use the coarse list for the same reason.

### 2. Exact position first, coarse position otherwise

`Session::avatars()` returns one `Seen { id, position: [f32; 2], height: Option<f32>, exact: bool }` per avatar other than the bot's own, built as the union of:

- the described avatars (`kinds` says `PCODE_AVATAR`, not `own_local`): the place of the last element of `lineage(local_id)`, as `sightings()` does for a wearer, so a seated avatar counts at its seat's root; skipped when that place is not known;
- the coarse list, for ids not covered by the first: `x`, `y` as given, height `z * 4`, and `None` when `z` is 0 or 255. Second Life writes 255 for anything above 1020 m, OpenSimulator writes 0 above 1024 m (`LLClientView.SendCoarseLocationUpdate`).

When the coarse list has not arrived, the described avatars alone are the answer at the deadline. An avatar that is described but missing from a later coarse list is still listed: a `KillObject` removes it when it leaves.

Using the coarse values as given (not the middle of the cell) matches what viewers show and keeps the spec's example exact; the error is under 1 m sideways and 4 m in height.

### 3. `scan` dispatch stays inside `scan.rs`

`Request::Scan(Vec<String>)` and the wire format do not change. `scan::Command::parse(&[String])` returns `Objects(Query)` or `Avatars(Option<f32>)`: a first argument `avatars` selects the second, a first argument `objects` is dropped, anything else goes to `Query::parse` as today. One `distance(&str) -> Result<f32, String>` reads a number from 1 to 1024 or `shout`/`chat`/`whisper` for both, with one error text. `objects` in any other place falls through to the distance and is refused there, which needs no extra code.

Alternative: separate `Request` variants decoded in `control.rs`. Rejected: the startup script and the XMPP contact go through `Request::decode` too, and `scan`'s arguments are already "still to be read" there.

### 4. Friends live in the session, not in the region

`login.rs` reads `buddy-list` (`buddy_id`, `buddy_rights_has`, `buddy_rights_given`) into `SessionInfo::friends`; a missing or malformed entry is skipped, never a login failure. `Session` owns a `friends::Friends` (new `src/friends.rs`): a map from UUID to `{ sees_online: bool, online: bool }`, where `sees_online` is bit 1 of `buddy_rights_has`. It is outside `Sim`, so a teleport keeps it, and it dies with the `Session`, so a new login starts from the new reply.

Messages, all decoded anew:

- `OnlineNotification` / `OfflineNotification` (Low 322 / 323, `AgentBlock Variable { AgentID }`): set `online` for ids that are friends.
- `TerminateFriendship` (Low 300, `ExBlock { OtherID }`): remove.
- `ChangeUserRights` (Low 321, `AgentData { AgentID }`, `Rights Variable { AgentRelated, RelatedRights S32 }`): as the viewer's `LLAvatarTracker::processChange` reads it. When `AgentID` is another avatar, it is the friend who changed what it grants: set `sees_online` from bit 1. When `AgentID` is the bot's own, the rights are the ones the avatar gives; ignored.
- `ImprovedInstantMessage` with dialog 39 (friendship accepted): add the sender with `sees_online` true and `online` false; the simulator's notice follows (OpenSimulator sends it from `FriendsModule`).

The listed status is `?` when `!sees_online`, else `online`/`offline`. `online` is kept as the notices say even while hidden, so a right given back does not lose it.

### 5. Profiles: `AgentProfile` where offered, UDP otherwise

New `src/profile.rs` holds a `Profile { id, online, born, partner, about, first_life }`, two parsers into it, the text of the answer, and a worker thread like `names::spawn_search`:

- `caps.rs` asks the seed for `AgentProfile` and reports `CapsEvent::Profiles(Option<String>)`. With a URL, `Grid::ask_profile` sends the worker `GET <url>/<uuid>`; it answers `Event::Profile { id, result }`. Keys as the viewer reads them: `id` (must equal the asked UUID, else the avatar is unknown), `online`, `member_since` (an LLSD date; left out when `hide_age` is true), `partner_id`, `sl_about_text`, `fl_about_text`.
- Without a URL, the session sends `AvatarPropertiesRequest` (Low 169) and the answer is `AvatarPropertiesReply` (Low 171, zerocoded): `PartnerID`, `AboutText` (Variable 2), `FLAboutText`, `BornOn` (text, `M/D/YYYY` on both grids), `Flags` with online as `0x10`. It is matched to the waiting command by `AvatarID`.

A nil partner is no partner. `born` is normalised to `YYYY-MM-DD` when it parses as an LLSD date or as `M/D/YYYY`, and passed through otherwise.

The capability comes first because it is what current viewers use on Second Life, where the UDP path may be retired; UDP stays because OpenSimulator has nothing else. An unknown UUID fails at once on the capability (HTTP error, or a reply without the id) and by timeout on UDP, where the simulator just stays silent.

`online` in the answer follows the spec: the profile's flag gives `yes`; otherwise `friends` decides between `no` (a friend with `sees_online`) and `unknown`.

### 6. One name cache for the listings

`Commands::owners: HashMap<Uuid, Option<String>>` becomes a cache of `Option<names::Names>` (legacy and display name) used by the owners of `scan objects`, by `scan avatars`, by `friends` and by `profile`. `owner_lookups`, `wanted_names`, `names_resolved` and the `MAX_OWNER_NAMES` bound stay as they are; `scan::listing` and `Query::matches` keep taking legacy names, through a view of the cache. An avatar the name service did not name is forgotten once no listing waits, as today, so the next command asks again.

Alternative: a second map beside `owners`. Rejected: the same avatar would be looked up twice, and two bounds would have to be kept in step.

### 7. The three commands wait like `scan`

Each is parked in `Commands` with its reply and a deadline of `LOOKUP_LIMIT` (10 s), and looked at from `tick` on the `SCAN_POLL` rhythm that `scans_due` already sets:

- `scan avatars`: ready when `grid.avatars_known()` and every listed avatar is in the cache. Names are asked only for the avatars that pass the distance filter, which can be told without names.
- `friends`: ready when every friend is in the cache. Lookups go out in the worker's batches of 50; what has not come back at the deadline is `?`, and the next `friends` finds it cached.
- `profile`: `After::Profile` joins `im`'s name resolution (so the resolved-name cache and its error texts are shared), then `ask_profile` and a name lookup for the UUID. Ready when the profile is there and the name is cached or has failed. At the deadline: the profile without names, or an error when there is no profile.

All three fail with the texts `scan` uses when the avatar leaves the region or the session is lost (`fail_scans`' siblings). `friends` and `profile` go into the list of commands in `Bot::control` that never use up an RLV override; `scan` leaves that list (decision 9).

### 8. Where the pieces live

- `src/udp/messages.rs`: the six decoders and the one encoder, with `testing` builders.
- `src/login.rs`: `SessionInfo::friends`.
- `src/caps.rs`: `AgentProfile` in the seed, `CapsEvent::Profiles`.
- `src/session.rs`: the coarse list in `Sim`, `avatars()`, `avatars_known()`; `Friends` in `Session` and the handlers that feed it; `request_profile()`; a `SessionEvent` for a profile reply and for friendships that end or begin (for the operator log).
- `src/scan.rs`: `Command`, `distance`. New `src/avatars.rs`: `Seen` to lines (distance, order, `?`, escaping). `src/friends.rs`: the list and its lines. `src/profile.rs`: see 5.
- `src/rlv.rs`: `ENFORCED` and the four questions on `Limits` (decision 9).
- `src/control.rs`: `Request::Friends`, `Request::Profile(String)`, `CTL_USAGE`. `src/commands.rs`: the parked commands, `Grid` additions (`avatars`, `avatars_known`, `friends`, `ask_profile`). `src/main.rs`: `Live`, the worker, the events, the override list.

### 9. RLV restrictions on the scans

The seven behaviours (`shownames`, `shownames_sec`, `shownametags`, `shownearby`, `showminimap`, `camdrawmax`, `camdrawalphamax`) move into `ENFORCED`, so `rlv status` stops calling them `(no effect)`. `Limits` gets four questions, each a few lines over what is there:

- `hides(avatar: Uuid) -> bool`: `against("shownames", Id)` or `against("shownametags", Id)`. `against` already folds `shownames_sec` into `shownames` and applies the exception rules, so `shownames:<uuid>` excepts from `shownames` only and `shownametags:<uuid>` from `shownametags` only. The viewer pools both kinds of exception; the bot keeps its own rule ("When restrictions are in force") instead of a special case.
- `names_hidden() -> Option<Refusal>`: one of the three in force at all; refuses `-o`.
- `nearby() -> Result<(), Refusal>`: `in_force("shownearby")`; and `minimap() -> bool` for `showminimap`.
- `vision() -> Option<f32>`: the smallest readable option of `camdrawmax` (the loop of `reach`, without its default), `None` when the largest readable `camdrawalphamax` in force is below 0.99.

Refusals and the cut of the distance are decided when the command is given. Who is hidden and whether positions show are read when the command is answered, from the limits of the command's `Origin`, which the parked command (`Scanning` and its avatar sibling) keeps: an exempt scan stays exempt while it waits, and gets no limits and so none of this, which is all "Override exempts one command" asks.

Where each bites:

- `scan avatars`: refused up front by `nearby()`. At answer time each `Seen` is marked hidden by `hides(id)`; hidden ones are not queued for name lookups and `avatars::listing` writes `?` for their UUID and names. The minimap flag makes it write `?` for every position. Order and distance filter work on the real values.
- `scan objects`: `-o` with `names_hidden()` is refused before `grid.reach`. The distance handed to `grid.reach` is `min(requested or current, vision)`, so the reach is never raised past the limit; when the current reach is already wider, the listing is cut with the same value in `Query::reachable`. `scan::listing` writes `?` for owner UUID and name when the owner is an avatar, not the bot's own, and hidden; such owners are not queued for lookups. The log line adds `cut to N m by camdrawmax`.

Since restrictions now apply, `Request::Scan` leaves the `never` list in `Bot::control`: a `scan` under `rlv mode override` is the exempt command and uses it up, restricted or not, like `sendtp`.

Alternatives considered: refusing `scan avatars` under `shownames` as the viewer hides its list (chosen against: the count and distances are what the avatar could see anyway, and exceptions would mean nothing); hiding names but not UUIDs (a UUID is the name: `profile <uuid>` returns it); applying the vision limit to avatars too (a viewer's radar is not limited by fog; `shownearby` is the restriction for that).

## Risks / Trade-offs

- [**Behaviour change**: `scan` now uses up an RLV override, where it never did] → Called out as breaking in the proposal and the README's "Override" paragraph; it only matters between `rlv mode override` and the next command.
- [Seven restrictions that scripts already send start to bite on bots that run with RLV on] → Only the two scans change; nothing is refused but `scan avatars` under `shownearby` and `-o` under the name restrictions, and the override lets one through.
- [The message log, `status` and `profile` still tell names, so `shownames` does not blind the operator] → Deliberate and in the spec; the scans are the bot's radar and area search, the rest is not touched by this change.
- [`camdrawmin` alone, `camavdist` and `setsphere` also dim a viewer's sight and do nothing here] → `camdrawmax` with opaque fog is the blinding case; the others stay kept without an effect and can follow.
- [The viewer pools `shownames` and `shownametags` exceptions; the bot does not] → A script that excepts with one behaviour and restricts with the other hides more on the bot than in the viewer. Erring towards hiding is the safe side.

- [OpenSimulator sends at most 60 avatars in the coarse list] → A crowded region lists the 60 plus every described avatar; the nearest ones are described, so `chat` and `whisper` stay complete. Documented in the README.
- [Regions larger than 256 m (OpenSimulator var regions) do not fit the list's one-byte coordinates] → Far avatars there get wrong coarse positions; described ones are exact. Documented; not solved.
- [A height byte of 0 is read as unknown, so an avatar really standing below 4 m and not described shows `?`] → Rare (land is seldom that low), and the alternative is a wrong distance for every avatar above 1024 m on OpenSimulator.
- [The coarse list lags by a few seconds: an avatar that just left or arrived may be listed or missing] → Inherent; viewers' radars behave the same.
- [`friends` right after a login can show a friend as offline until the simulator's notices arrive] → Inherent in the protocol; documented.
- [A grid that never sends the right-to-see-online bit correctly makes statuses `?`] → The bit is what viewers rely on; on OpenSimulator the default grant includes it.
- [The `AgentProfile` reply is known only from the viewer's source, and the rig has no such capability] → The parser is unit-tested against the keys the viewer reads and tolerates missing ones; the UDP path is verified on the rig; a check on Second Life is a task, and the README's "Status" says which paths were verified.
- [Very long friend lists need many name requests and may not finish in 10 s] → The first `friends` shows `?` for the rest and a second one completes; nothing blocks.
- [Widening `owners` touches `scan objects`] → Its tests stay as they are and must pass unchanged.

## Migration Plan

No config keys, stored data or wire formats of the control socket change. With RLV off, or none of the seven restrictions held, `scan` without `avatars` behaves as before, so startup scripts keep working. Two things differ for a bot that runs with RLV on: `scan` uses up an override, and restrictions kept from before the upgrade (they are stored with the others) start to apply to the scans at once. Rollback is reverting the change.

## Open Questions

- Whether Second Life caps the coarse list as OpenSimulator does. It changes nothing in the approach: described avatars are merged in either way.
