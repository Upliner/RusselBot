# Proposal

## Why

The bot can list the objects around its avatar, but not the people: who is in the region and how near, which of the avatar's friends are online, and who an avatar that turns up in the logs is. Today each of these means logging in with a viewer, which logs the bot out.

## What Changes

- `scan` gets two forms. `scan objects [DISTANCE] [FILTER...]` is today's `scan`; a `scan` whose first argument is neither `objects` nor `avatars` stays what it is, so `scan 20 -n bench`, the startup scripts and the README examples keep working. `scan avatars [DISTANCE]` is new.
- `scan avatars` lists the other avatars in the avatar's region, one line each, nearest first: UUID, distance, position, name and display name. Without a distance it lists every avatar in the region, also those far beyond the distance the bot asks the simulator to describe; with one, only those within it.
  - It uses what the simulator tells every viewer unasked: the coarse positions of all avatars in the region (whole metres, height in steps of 4 m), and the exact position of the avatars it has described. So it sends the simulator nothing, does not start the object scan and does not raise its distance; the only traffic is the name lookups.
  - An avatar whose height the simulator does not tell (above 1020 m and not described) is listed with `?` as its distance and is left out when a distance is given.
- `DISTANCE` takes the aliases `shout` (100 m), `chat` (20 m) and `whisper` (10 m) beside a number of metres, in both forms of `scan`.
- New command `friends`: the avatar's friend list, one line each, with UUID, status (`online`, `offline`, or `?` for a friend who does not let the avatar see it), name and display name; online friends first. The list is the one the grid gives at login, kept current from the simulator's online and offline notices and from friendships that end or begin while the bot runs.
- New command `profile AVATAR`: the profile of an avatar given as `im` takes one (name or UUID), as `name: value` lines like `status`: id, name, display name, online status, whether it is a friend, date of birth, partner, the about text and the first-life text. `online` is `yes`, `no` or `unknown`, because a grid reports an avatar that hides its status as offline.
- All three read only: they move nothing and do not change the placement mode. They work through the control socket, the startup script and the XMPP control contact like every other command.
- `scan` is held to RLV restrictions, as far as a viewer's radar, minimap, area search and sight are. These restrictions had no effect until now:
  - `shownames`, `shownames_sec` and `shownametags` hide who an avatar is. `scan avatars` still has a line for it, with distance and position, but with `?` as UUID, name and display name; `scan objects` writes `?` for an owner that is hidden and refuses `-o`. An avatar an exception names (`shownames:<uuid>`, `shownametags:<uuid>`) is shown in full, and so is the bot's own.
  - `shownearby` refuses `scan avatars`.
  - `showminimap` turns the position field of `scan avatars` into `?`; distances stay.
  - `camdrawmax` cuts `scan objects` to the smallest distance so held (the vision limit): nothing beyond it is listed and the object scan is not widened past it. As in the viewer, the limit holds only while the fog is opaque, which a `camdrawalphamax` below 0.99 undoes. `scan avatars` is not cut by it.
- **BREAKING**: because restrictions now apply to it, `scan` is the exempt command of `rlv mode override` like any other command and uses the override up. Until now `scan` never did.
- `friends` and `profile` are not held to any RLV restriction and do not use up an override: a viewer does not hide the friend list or a profile opened by name either.

Nothing is removed. With RLV off or none of these restrictions held, no existing command changes its answer: `scan` without `avatars` lists what it listed, and now also accepts `objects` and the three aliases, which it refused before.

## Capabilities

### New Capabilities
- `avatar-scan`: the `scan avatars` command: which avatars it lists, where their positions come from, the distance and its filter, the listing format, how long it waits, and what it asks of the simulator and the name service.
- `friend-list`: the bot's knowledge of the avatar's friends and of who is online, and the `friends` command that lists them.
- `avatar-profile`: the `profile` command: how the avatar is named, what the grid is asked, the lines of the answer and what `online` means.

### Modified Capabilities
- `area-scan`: "Scan command" (the optional `objects` word; `scan avatars` belongs to `avatar-scan`), "Scan distance" and "Combining and refusing arguments" (the aliases `shout`, `chat` and `whisper`), and "Scan reads only" (held to the restrictions on scans; exempt under an override like any other command).
- `rlv-restrictions`: "Restrictions and their holders" (`shownames`, `shownames_sec`, `shownametags`, `shownearby`, `showminimap`, `camdrawmax` and `camdrawalphamax` move to the restrictions with an effect), and new requirements for what they do to the two scans.

## Impact

- `src/udp/messages.rs`: decode `CoarseLocationUpdate`, `OnlineNotification`, `OfflineNotification`, `TerminateFriendship`, `ChangeUserRights` and `AvatarPropertiesReply`; encode `AvatarPropertiesRequest`.
- `src/login.rs`: read `buddy-list` from the login reply into `SessionInfo`.
- `src/caps.rs`: ask the seed capability for `AgentProfile` and report its URL.
- `src/session.rs`: the region's coarse avatar positions (per region); the friend list and who is online (per login); what `scan avatars` reads out; the profile request over UDP.
- `src/scan.rs`: the `objects`/`avatars` word and the distance aliases; new `src/avatars.rs` (listing of avatars), `src/friends.rs` (friend list and its listing) and `src/profile.rs` (profile fetch, parsing and text).
- `src/control.rs`: `Request::Friends`, `Request::Profile`, `CTL_USAGE`; `src/commands.rs` and `src/main.rs`: the waiting commands, the name lookups they share with `scan`, the wiring, and `scan` leaving the commands that never use up an override.
- `src/rlv.rs`: the seven restrictions move from the list without an effect to `ENFORCED`; new checks on `Limits` for hidden avatars, the nearby list, the minimap and the vision limit.
- `README.md`, `docs/test-rig.md`.
- Traffic: none added to the simulator by `scan avatars` and `friends` beyond name lookups; `profile` makes one request per command. The messages decoded anew already arrive today and are dropped.
- No new dependencies, no config keys, no change to stored data.
