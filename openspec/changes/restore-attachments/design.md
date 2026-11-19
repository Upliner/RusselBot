# Design

## Context

See proposal.md for the motivation. Sources below: K = `references/kokua/indra/newview`, O = `references/opensim/OpenSim`.

What was seen (README.md, Status; logs of 2026-11-19):

- Second Life describes an avatar's attachments in full and unasked within four seconds of a login. In six logins none was mentioned as a cached object, and an attempt to learn of missed attachments by asking about cached objects was built, found beside the point and taken out again.
- An object attached to `Right Hand` was shown at the next two logins. After another object had been attached to the same point and detached, it was not shown at the login after that, though the bot saw nothing take it off. Earlier that day the simulator attached such an object again when asked, so it was really off. Why the simulator loses it is not known; the simulator is closed.
- An `attach` took the simulator 21 seconds once and 15 seconds once.
- On the local OpenSim the same sequence loses nothing (`docs/test-rig.md`, item 105). OpenSim does lose an attachment made less than five seconds before a logout, which is its appearance save delay (O `Region/CoreModules/Avatar/AvatarFactory/AvatarFactoryModule.cs:58`, item 106); that is a different matter and not touched here.

What a viewer does: whenever it has the Current Outfit folder, at login first of all, it takes the folder's links to objects as what should be attached, compares them with the objects on the avatar and asks the simulator for those that are missing (K `llappearancemgr.cpp:2618-2686`, `llagentwearables.cpp:1299-1400`). It also takes off what is attached and not listed, but only once the avatar is fully loaded. It forgets a request the simulator has not carried out after 30 seconds (K `llattachmentsmgr.cpp:41`), and links attachments that arrive without a link.

What the bot has:

- `Commands` (`src/commands.rs`) starts inventory jobs for commands and, with `note`, for changes to the Current Outfit folder nobody waits for; `note` does nothing unless the simulator offers the outfit service (`caps::InventoryUrls::outfit_service`: the inventory API and the appearance capability, `src/caps.rs:81`). The rig's OpenSim offers neither.
- The inventory worker (`src/inventory.rs`) can find the Current Outfit folder, list its links (`outfit_folder`, `outfit_links`) and fetch items by id (`What::Items`, which leaves out what the grid does not know).
- `Session` waits `ATTACH_LIMIT` (15 s) for an attach and for a detach alike and then reports `AttachFailed` or `DetachFailed` (`src/session.rs:28`, `798-814`). The control client waits 90 s for an answer (`src/control.rs:30`).
- `main.rs` turns `SessionEvent::Attached` into `Commands::attached`, which answers a waiting `attach` and notes the link; an attachment nobody waits for is only logged (`src/main.rs:796-800`, `src/commands.rs:970-982`).

## Goals / Non-Goals

**Goals:**

- An avatar run by the bot alone keeps the attachments its Current Outfit folder lists across logins on Second Life.
- With the option off, or without the outfit service, the bot sends exactly what it sends today.
- The operator log tells what was restored and what could not be.

**Non-Goals:**

- Taking off what is attached and not listed. The bot would undo things scripts and experiences attach, for no observed problem.
- Linking attachments the bot did not ask for.
- Restoring after a teleport. No loss was seen there; it can follow if one is.
- Restoring clothing and body parts. The grid makes the appearance from the folder by itself.
- Finding out why the simulator loses the object, or avoiding it by attaching and detaching differently.
- Repeating a request the simulator does not carry out.

## Decisions

### The Current Outfit folder says what should be attached
It is the only record of intent there is, the bot already keeps it for its own attaches and detaches, and viewers treat it the same way, so bot and viewer agree on one avatar.

*Alternative: remember in a file of the bot's own what it attached.* Would not know what a viewer session changed in between and would disagree with the viewer about the same avatar.

*Alternative: avoid the loss.* Nothing in the logs says what triggers it on the simulator's side beyond the sequence of commands, and a viewer sends the same requests.

### Only where the outfit service is offered
On a grid without it the bot does not keep the folder, so a link there may be for something the bot itself has detached since, and restoring would put it back on. The gate is the same one `note` uses. On the rig's OpenSim the option therefore does nothing, which is also what item 105 says is needed there.

### Once per login, 20 seconds after the first region is entered
The simulator needs time to show what it does bring: four seconds at most in the logs. Asking earlier would request objects that are about to appear by themselves; what the simulator does with such a request is not known. Twenty seconds is five times the observed time and still short for a bot that runs for days. The moment is counted from the first `RegionEntered` after a login. If the avatar is between regions when it comes, the comparison runs at the next region entry. Nothing is compared again in that login.

*Alternative: wait until no attachment has been reported for some seconds.* Adapts to a slow simulator, but an avatar with no attachments left gives no signal at all, so it needs the fixed limit as well; the fixed delay alone is simpler and enough for what was seen.

*Alternative: at once, as a viewer does.* A viewer can afford duplicates of its own requests because it tracks them; the bot has no evidence of what a request for an object already on its way does.

### What is compared
A new inventory job returns the items the folder's links point to that are objects: the links are listed, those to objects picked by the link's own type fields, and their targets fetched by id, which drops links whose item is gone. When the answer arrives `Commands` takes from them what is neither on the avatar (`Grid::attached`) nor being waited for by an `attach` already under way, and sends each through the same `Grid::attach` the command uses, with point 0: the simulator's choice, which is the point it was last on, and with the add flag the bot always sets.

The job runs through the worker like any other, so it does not hold up simulator traffic, and it is dropped like a note when the avatar changes simulator.

### A restore is an attach nobody waits for
Each requested item goes into a list beside `attaching`. `Commands::attached` looks there after `attaching`: a hit logs `attachment: restored ...` with the point and adds no link, since the link is what asked for it. `Commands::attach_failed` looks there too and logs that the simulator did not restore the item. A command for the same item meanwhile takes the entry over, as a newer `attach` of the same item already supersedes an older one.

### Attach waits 30 seconds, detach 15
`ATTACH_LIMIT` splits into two constants. Thirty seconds is the viewer's own patience and covers the 21 seconds seen. Detaches were answered within a second, and nothing suggests they need more. The control client's 90 seconds leave room for a lookup and the wait.

### An attach that arrives after the answer still gets its link
`Commands` keeps the items whose `attach` or `wear` it answered as not attached, for the rest of the session, bounded at 64 with the oldest dropped. When `Attached` comes for one of them it logs `attachment: ... arrived late` and notes the link. The list is emptied when the session is lost. Without this a late object is on the avatar and unlisted: a viewer would take it off at its next login, and the restore could not bring it back.

### The option
`restore_attachments`, boolean, default `true`, parsed with the existing `flag` helper beside `fetch_offline_im` and handed to `Commands` when it is made. With `false` the timer is never set.

### Log lines
At info level: `attachment: restoring "Name" <id>, listed in the Current Outfit folder and not on the avatar`; `attachment: restored "Name" <id> to Right Hand`; `attachment: the simulator did not restore "Name" <id> within 30 s`; `attachments not restored: <reason>` when the folder cannot be read. At debug level, when nothing is missing: `attachments: all N objects of the Current Outfit folder are on the avatar`.

## Risks / Trade-offs

- [Something took an object off without updating the folder: a script, another tool, or a `detach` of the bot whose folder update failed] → It is back after the next login. That is what a viewer does too; the failed update is already logged when it happens, and `restore_attachments = false` switches the behaviour off.
- [The simulator was only slow and the object is still coming after 20 seconds] → The bot asks for an object that then arrives anyway. Not seen, and what Second Life does then is unknown; the hand check looks for a second copy or an alert.
- [The lost object is on the avatar for others and only hidden from the bot] → Then the request is one for an attached item. The first check of 2026-11-19 speaks against it: the simulator carried such a request out. The hand check records what the simulator answers.
- [A bot that logs in every few minutes after outages asks for two folders each time] → Two small requests per login; the offline-IM fetch already does as much per login where it is on.
- [`inventory -a` in the first 20 seconds, and until a restored object has arrived, shows the avatar without it] → As today after any login; README says when the restore happens.
- [The rig cannot show a restore] → Unit tests with the fake grid and its inventory, which has the folder and the service switch; on the rig only that nothing is asked. Proof is the hand check on Second Life.
- [Longer wait for a failing `attach`] → 30 seconds instead of 15 before the error; the spec says so.

## Migration Plan

No stored state. Operators who want the old behaviour set `restore_attachments = false`; README names the key in the config table and under Attachments. Rollback is reverting the commit.

## Open Questions

- Whether Second Life also loses attachments across a teleport. Not seen; it would add a second moment for the same comparison and no new mechanism.
