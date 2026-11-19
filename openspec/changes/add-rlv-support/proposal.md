# Proposal

## Why

Scripted objects on Second Life and OpenSim steer avatars through the RestrainedLove API (RLV): the object says `@command` lines to its owner with `llOwnerSay`, and a viewer that implements the API carries them out. Collars, furniture, relays and many games first ask `@version` or `@versionnew` and do nothing for an avatar that does not answer. The bot hears these lines today and only writes them to its chat log, so an avatar run by the bot is invisible to all such objects. Most of what the simplest RLV commands ask for (sit, stand, teleport, take something off) the bot can already do on the operator's command through `rslbot ctl`; what is missing is reading the commands and answering the version questions.

This change is the first step: reading RLV commands, answering the version queries, and carrying out the force commands that have a `ctl` counterpart. Restrictions, the heart of RLV, come in a later change. The bot therefore reports API version 0.1.0 (`@versionnum`: 10000), far below any real viewer's, so that scripts that check the number see an incomplete implementation.

## What Changes

- New config key `rlv`, a boolean, off by default. While RLV is off the bot behaves exactly as today.
- New control-socket commands:
  - `rslbot ctl rlv mode on` and `rslbot ctl rlv mode off` switch RLV on and off while the bot runs. Like `persist`, the switch is not saved: a restarted bot starts from its config.
  - `rslbot ctl rlv persist follow` and `rslbot ctl rlv persist drop` set what a forced movement does to the place the bot holds (below). Not saved either.
  - `rslbot ctl rlv status` prints `rlv: on` or `rlv: off`, and `persist: drop` or `persist: follow`. The list of restrictions will be added here later.
  - `rslbot ctl rlv cmd COMMAND...` carries out RLV commands as if an object had sent them, with the avatar itself as the sender, and prints what became of each. It is refused while RLV is off.
- While RLV is on, a chat line of kind `owner` (an object's `llOwnerSay`) that starts with `@` is read as RLV commands: `@behaviour[:option]=param`, several separated by commas after the one `@`. The line is still written to the chat log as before.
- Version queries are answered on the channel the command names: `@version=<channel>` with `RestrainedLife viewer v0.1.0 (RusselBot <bot version>)`, `@versionnew=<channel>` with `RestrainedLove viewer v0.1.0 (RusselBot <bot version>)`, `@versionnum=<channel>` with `10000`.
- These force commands are carried out the way the named `ctl` command is:

  | RLV command | Carried out as |
  |---|---|
  | `@sit:<uuid>=force` | `sit <uuid>` |
  | `@sitground=force` | `sit ground` |
  | `@unsit=force` | `stand` |
  | `@tpto:<region>/<x>/<y>/<z>[;lookat]=force` | `tp <x> <y> <z> <region>` |
  | `@tpto:<X>/<Y>/<Z>=force` (global coordinates) | a teleport to that point of the grid; `ctl tp` has no such form, the teleport underneath is the same |
  | `@detach:<attachment point>=force`, `@remattach:<attachment point>=force` | `detach <body part>` |
  | `@detach:<uuid>=force`, `@remattach:<uuid>=force` | `detach` of the item the attachment with that UUID was made from |
  | `@remoutfit:<kind>=force` | `unwear <kind>` |
  | `@detachme=force` | `detach` of the item the sending attachment was made from |

- What `@sit`, `@sitground`, `@unsit` and `@tpto` do to a place the bot holds is a setting of its own, the RLV persist setting, since a bot that kept holding its place would undo each of them within seconds:
  - `drop`, the default: like the `ctl` commands they stand for, they put the bot in manual mode. It stops returning to its place until `persist` or `home`.
  - `follow`: the result of the movement becomes what the bot holds, also for a bot that held nothing.
    - `@sit:<uuid>` is carried out as `persist cur <uuid>`: the named object becomes the held seat at once, with the place the avatar is at, and the bot sits on it as it does on any held seat, trying again when a sit fails. When the avatar is seated and its seated position is known, `persist cur cur` follows, so that the place the bot returns to is the seat itself.
    - `@sitground`, `@unsit` and `@tpto` are carried out first and persisted when they have ended well (`persist cur ground`, `persist cur stand`). A forced teleport that fails leaves everything as it was.
- The `[persist]` table of the config gets two keys:
  - `rlv = "drop"` or `rlv = "follow"` is the RLV persist setting the bot starts with.
  - `atstart`, a boolean, says whether the bot holds anything once it has reached the configured place. It is true when the table has `position` or `sit`, and false otherwise; `atstart = true` alone holds what the simulator reports (`"cur"` for both). The `rlv` key does not affect it, so a table can set the RLV persist setting and hold nothing at the start.
- **BREAKING** (edge): a `[persist]` table with neither `position`, `sit` nor `atstart = true` now holds nothing at the start. An empty `[persist]` table used to mean `"cur"` for both; such a config needs `atstart = true`. `atstart = false` together with `position` or `sit` is rejected.
- Every other RLV command is read, noted in the operator log as not supported, and otherwise ignored: all restrictions (`=n`, `=add`, `=y`, `=rem`), `@clear`, `@notify`, the other queries (`@getstatus`, `@getsitid`, `@getoutfit`, `@getattach`, `@getinv`, ...), the shared-folder commands (`@attach:...`, `@detachall:...`), `@detach=force` and `@remoutfit=force` without an option, and the blacklist commands.
- RLV commands that arrive while the avatar cannot act yet (during login or a teleport) are kept, up to 64 lines, and carried out when it has arrived, as viewers do.
- The operator log gets a line for every RLV command: who sent it and what was done, or why not.
- README, `russelbot.example.toml`, the `ctl` usage text and `docs/test-rig.md` are brought in line.

Apart from the empty `[persist]` table, nothing changes for a bot whose config does not set `rlv` and that is not told `rlv mode on`.

## Capabilities

### New Capabilities

- `rlv-commands`: RLV mode and how it is switched, the `rlv` control-socket commands, which chat lines are read as RLV commands and how they are parsed, the version queries, the force commands and the `ctl` commands they are carried out as, the RLV persist setting and what a forced movement does to the held placement, what happens to commands that are not supported, and the operator log lines.

### Modified Capabilities

- `bot-config`: the key `rlv` and the keys `atstart` and `rlv` of the `[persist]` table in "Single TOML config file", and their validation in "Config validation before any network activity".
- `avatar-commands`: "Commands that suspend automatic placement" also covers the movement commands carried out for RLV, and the return to holding mode after a followed one.
- `placement-persist`: "Held placement" also changes through a persist carried out after a forced RLV movement; "Config persist setting applied after startup" counts a table whose `atstart` is false as off.

## Impact

- New module `src/rlv.rs`: the parser, the version texts, the translation of force commands, the buffer.
- `src/config.rs`: the new key, and the two new keys of the `[persist]` table.
- `src/control.rs`: the `rlv` requests and the usage text.
- `src/main.rs`: holds the RLV state, hands `owner` chat to it, carries out what it asks for through the existing commands, and persists or returns to the earlier mode when a followed movement has ended.
- `src/commands.rs`: carrying out a command nobody waits for and that does not count as the operator's last `tp` or `sit`; a teleport by global coordinates; `detach` of the item behind an attached object.
- `src/session.rs`: a teleport by region handle; which inventory item an attached object was made from.
- `README.md`, `russelbot.example.toml`, `docs/test-rig.md`; a rig helper that puts a relaying script into an object the bot owns, since the rig has nothing that says `llOwnerSay` to the bot today.
- No new dependency. No traffic while RLV is off; while it is on, one chat message per answered query.
- Security: while RLV is on, any object the avatar owns, worn or rezzed in the region, can move the avatar and take off attachments and clothing without the operator. That is the purpose of RLV, and the reason it is off by default. With `follow` such an object also decides where the bot returns to after a kick or a region restart, until the operator persists another place.
