# Proposal

## Why

RLV in the bot is half an API: objects can force the avatar to sit, stand, teleport and undress, but nothing they forbid holds. Every restriction (`@unsit=n`, `@tploc=n`, `@detach=n`, `@sendim=n`, ...) is noted in the log as not supported, `@clear` and `@getstatus` are not understood, and a script that checks its restrictions with `@getstatus` sees an avatar that obeys none. For furniture, cages, collars and relays, restrictions are the point of RLV, and the first RLV change (`add-rlv-support`) was shaped to have this one built on it.

## What Changes

- The bot keeps RLV restrictions and exceptions as a viewer does: per object that set them, added with `=n` or `=add`, lifted with `=y` or `=rem`, and cleared with `@clear` or `@clear=<text>`. Every restriction the RLV API names is kept, so that what scripts read back is what they set.
- Restrictions that concern something the bot does are enforced; the others are kept and have no effect, without a line in the log calling them unsupported. Enforced in this change: chat and instant messages the avatar sends, sitting and standing, teleports and teleport offers, touching, attaching and detaching, wearing and taking off clothing, script permission requests, and listing the inventory.
- While RLV is on, restrictions apply to everything the bot does for the avatar:
  - the operator's `rslbot ctl` commands, which are answered with an error that names the restriction and who holds it;
  - what the bot does by itself: the steps of holding mode (teleporting back, sitting, standing), restoring attachments after a login, and accepting offers and requests under the `perms` rules;
  - RLV force commands, except that an object's own restrictions do not stand in the way of its own force commands.
- A persist override setting names the sorts of the bot's own placement steps that restrictions do not hold back: `tp` for its teleports to the held location, `sit` for sitting on the held seat or on the ground and for the stand a held `stand` calls for. It is the `override` key of the config's `[persist]` table (`"sit"`, `"tp"` or an array of both; empty by default) and can be changed while the bot runs with `rslbot ctl rlv persist override none|sit|tp|sit tp`. A bot that has to get back from a hub to the region whose objects restrict it, after a region restart, sets it. It covers the bot's own steps only, never the operator's commands, force commands or offers.
- `@getstatus` and `@getstatusall` are answered. `@accepttp`, `@accepttprequest` and `@acceptpermission` make the bot accept teleport offers, answer teleport requests and grant attach and control permissions by itself, as they do in a viewer.
- A third RLV mode, `rlv mode override`: the next `rslbot ctl` command is carried out as if no restriction were stored, and RLV is `on` again as soon as that command has been taken up. Override changes nothing about how the bot reacts to what happens in-world, and nothing about `rlv cmd`.
- `rlv mode off` keeps what is stored: nothing is enforced and nothing new is read while RLV is off, and the stored restrictions apply again when RLV is switched on.
- `rslbot ctl rlv reset` clears every stored restriction, in any mode.
- `rslbot ctl rlv cmd` holds its restrictions apart from every object's, as the RLV console of a viewer does: `rlv cmd @clear` clears only what `rlv cmd` set, and no object's `@clear` touches it.
- An object's restrictions end with the object, as in a viewer: when it is detached or deleted, when the avatar leaves its region without wearing it, and when the session is lost. Restrictions set with `rlv cmd` last until they are lifted, `rlv reset`, or the end of the bot process. Nothing is saved across a restart of the bot.
- `rlv status` also prints the persist override setting and the restrictions that are stored, each with its holder and whether it has an effect in the bot.
- **BREAKING** for scripts and operators that relied on restrictions being ignored: with RLV on, `rslbot ctl` commands and the bot's own placement can now be refused. `rlv cmd <restriction>` is answered with success instead of "not supported", and `rlv status` prints more lines.

Not in this change, and said so where an operator would look: `@notify`, the blacklist queries, the shared `#RLV` folder with everything that names inventory folders, the other status queries (`@getsitid`, `@getoutfit`, `@getattach`, ...), `@standtp`, re-attaching a locked attachment that a script kicked off, and everything that restricts what the user of a viewer sees or receives (`@recvchat`, `@recvim`, `@shownames`, `@showloc`, the camera restrictions, ...). Restrictions of these kinds are kept and reported like the others; they have no effect. The message log and `status` stay the operator's instruments and are not censored. The RLV API version the bot reports stays 0.1.0.

## Capabilities

### New Capabilities

- `rlv-restrictions`: keeping RLV restrictions and exceptions per holder, how long they live, which of them have an effect and on what (operator commands, the bot's own actions, force commands), the override exemption, the `@getstatus` queries, the automatic acceptances RLV can switch on, and `rlv reset`.

### Modified Capabilities

- `rlv-commands`: RLV has three modes, with `override` between `on` and `off`, and `off` keeps what is stored; `rlv reset`, the persist override setting with its `rlv persist override` command, and the longer `rlv status`; restrictions, `@clear`, `@getstatus` and `@getstatusall` are no longer "not supported"; `rlv cmd` has a holder of its own and answers a restriction with `ok`; the operator log names restrictions as they are set, lifted and applied.
- `presence-reconciler`: a step towards the held placement that an RLV restriction forbids is not started, unless the persist override setting names its sort; the bot waits for the restriction to go and says so in the log.
- `bot-config`: the `[persist]` table takes the key `override`.
- `permission-requests`: RLV restrictions come before the auto-accept rules and the `accept` command: an offer or request a restriction forbids is dropped, and one RLV tells the bot to accept is accepted whatever the rules say.

## Impact

- `src/rlv.rs`: the restriction store (holders, restrictions, exceptions, secure variants), the table of known behaviours, the checks, `@clear`, `@getstatus`, the three-valued mode. The parser already tells restrictions apart and keeps the sender with every command.
- `src/commands.rs`: every command that a restriction can refuse asks the store before it acts, as do the automatic acceptances; `say` gains the changes of volume and channel that restrictions make.
- `src/main.rs`: `rlv mode override`, `rlv reset`, the override exemption and when it ends, checks before the reconciler's steps and before the stand of a `persist`, dropping an object's restrictions when it is gone.
- `src/session.rs`: for every object the simulator describes, its parent and where it is, so that the bot can tell an attachment from a rezzed object, a child prim from its root, a HUD from a body attachment, and how far an object is; an event when a described object is gone.
- `src/reconciler.rs`: its decisions stay as they are; a step it chooses is held back in `src/main.rs` before it is started, so that waiting is not a failed attempt.
- `src/control.rs`: `rlv mode override`, `rlv reset`, `rlv persist override`.
- `src/config.rs` and `russelbot.example.toml`: the `override` key of the `[persist]` table.
- `README.md`: the RLV section (the paragraph saying that no restriction is implemented goes, and the tables of what is enforced and what is kept come), the command table, Upgrading.
- `docs/test-rig.md` and `test-rig/`: scenarios for restrictions set through `rlv cmd`, and through the rig's relaying prim where it can say `llOwnerSay`.
- No new dependencies, one optional config key, no change to the wire format of the control socket.
