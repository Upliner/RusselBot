# Proposal

## Why

Avatars and scripted objects ask an avatar for things a viewer answers with a dialog: a teleport offer, an inventory offer, a script's request to animate the avatar or to attach itself to it. The bot answers only one of them today, the teleport offer, and only by hand (`rslbot ctl accepttp <avatar>`) or right after a `requesttp`. Inventory offers and script permission requests are dropped unread, so a dance ball never animates the bot's avatar and nobody who gives the bot an item ever sees it accepted. A bot that runs unattended needs to answer the requests of the avatars and objects its operator trusts by itself, and needs a command for the rest.

## What Changes

- The bot reads two kinds of request it ignores today: inventory offers (from avatars and from objects) and script permission requests. With the teleport offers it already reads, each request is of one or more **kinds**:

  | Kind | Request |
  |---|---|
  | `tp` | an avatar's teleport offer |
  | `inventory` | an inventory offer from an avatar or an object |
  | `anim` | a script asks to animate the avatar (trigger or override animations) |
  | `attach` | a script asks to attach its object to the avatar |
  | `controls` | a script asks to take the avatar's controls |
  | `camera` | a script asks to track or control the camera |

  A script may ask for several permissions at once; such a request has all their kinds. A request for a permission that has no kind here (debit, script teleport, anything else) is never accepted, by itself or on command.
- New config table `perms` with two tables in it:
  - `[perms.autoaccept]` says per kind whose requests the bot accepts by itself: a list of avatar names and UUIDs, or `"all"`. `true` means the same as `"all"`, `false` the same as an empty list. An avatar's name or UUID stands for the avatar and for every object it owns; an object's UUID stands for that object. Nothing is accepted by itself for a kind that has no key, which is the bot's behaviour today.
  - `[perms.except]` lists per kind avatars and objects whose requests are never accepted by themselves, whatever `autoaccept` says. This is how `"all"` gets its exceptions, and how one object of a trusted avatar is left out.

  ```toml
  [perms.autoaccept]
  tp = ["Jane Doe"]
  inventory = ["Owner Resident", "11111111-2222-3333-4444-555555555555"]
  anim = "all"
  attach = true

  [perms.except]
  anim = ["01234567-9abc-defe-dcba-987654321000"]
  ```
- A request with several kinds is accepted by itself only when every one of its kinds is auto-accepted for its sender. It is then accepted whole; the bot never grants part of what a script asked for.
- A request that is not accepted by itself waits for the operator. The log says who sent it, what it asks for, and which command accepts it.
- New control-socket commands:
  - `rslbot ctl perms autoaccept KIND all | WHO...` adds to what is auto-accepted; `rslbot ctl perms revoke KIND all | WHO...` takes away from it. Both answer with a warning, and still succeed, for a WHO that was already in that state. While a kind accepts `all`, revoking a WHO makes it an exception. Like `rlv mode`, these changes are not saved: a restarted bot starts from its config.
  - `rslbot ctl perms status` prints the auto-accept rules of every kind and the requests that wait.
  - `rslbot ctl accept KIND [WHO]` accepts the latest waiting request of that kind, or with WHO the latest one from that avatar, that object, or an object of that avatar.
- `rslbot ctl accept tp [AVATAR]` takes the place of `accepttp AVATAR`. The avatar is optional now: without it the latest offer is accepted. `accepttp` stays as a deprecated alias with the same optional argument.
- A teleport offer accepted by itself moves the avatar as `accept tp` does, and like it puts the bot in manual mode, so that the bot does not teleport straight back to the place it holds.
- An accepted inventory offer is filed into the system folder for its type (Objects, Notecards, ...), as a viewer does. To find that folder the bot reads the top level of the inventory when it accepts an offer, the one new exception to "inventory is read on demand".

Two points of the request as it was put need an answer here:

- **`attach = true`: only true and false?** No. An attach request is a script permission request like `anim`, with an object and its owner behind it, so a list works for it as for the others. `true` and `false` are accepted for every kind as other spellings of `"all"` and an empty list.
- **An object's UUID in `inventory`.** The grid does not tell the receiver which object an inventory offer came from: the offer carries the owner's UUID and the object's name only. An object's UUID in `inventory` therefore can never match an offer. The config accepts it (the bot cannot tell an object's UUID from an avatar's), and the documentation says to name the owner. An avatar's name or UUID covers what its objects give.

## Capabilities

### New Capabilities
- `permission-requests`: the kinds of request the bot answers, the auto-accept rules and their exceptions, how a request's sender is matched against them, what accepting each kind does, the requests that wait, and the `perms` and `accept` commands.

### Modified Capabilities
- `bot-config`: new key `perms` with the tables `autoaccept` and `except`, and their validation.
- `avatar-commands`: `accepttp` becomes a deprecated alias of `accept tp` and its avatar becomes optional; an unsolicited teleport offer is accepted by itself when the rules say so; `accept tp` and a teleport offer accepted by itself put the bot in manual mode; the log names `accept tp`.
- `inventory-list`: the bot also reads the top level of the inventory to accept an inventory offer.

## Impact

- Code: a new module for the rules, the matching and the waiting requests; `src/config.rs` (the `perms` table); `src/control.rs` (the `perms` and `accept` commands, usage text); `src/commands.rs` (teleport offers move under the common rules; `accept`); `src/session.rs` and `src/udp/messages.rs` (reading inventory offers and `ScriptQuestion`, sending the inventory answers, `ScriptAnswerYes` and `RequestObjectPropertiesFamily`, reading `ObjectPropertiesFamily`); `src/inventory.rs` (finding the system folder for a type); `src/main.rs` (wiring).
- Behaviour without a `perms` table is as today, except that inventory offers and script permission requests now appear in the operator log and wait for `accept`.
- Docs: `README.md`, `russelbot.example.toml`, `docs/test-rig.md`; new rig helpers that ask for script permissions and give inventory.
- No new dependencies.
