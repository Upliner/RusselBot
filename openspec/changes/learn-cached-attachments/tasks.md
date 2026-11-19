# Tasks

## 1. Reading the mention

- [ ] 1.1 Make the `ObjectUpdateCached` decoder in `src/udp/messages.rs` hand over each object's `UpdateFlags` with its local id, add the owner bit (`0x20`) as a named constant, and let the `messages::testing` builder set the flags per object with a helper for an owned one; verify unit tests for a message with owned and other objects, for an empty one and for a truncated one, and that the existing cached-update tests in `messages.rs` and `session.rs` pass with their objects built as not owned

## 2. Asking about the account's own objects

- [ ] 2.1 In `Session`, ask at once about every mentioned object that has the owner bit and no description, 255 ids to a request, and remember the ids asked about in a bounded set of their own that is emptied with the simulator state and loses an id when the object is killed; verify fake-network tests for: owned and other objects in one message (only the owned are asked about, the others wait in `undescribed`), more than 255 owned objects, an object mentioned twice (one request), an object already described (no request), an object killed and its local id mentioned again (asked again), and a new region after a teleport (asked again there)
- [ ] 2.2 Keep the scan from asking twice: at the start of a scan and during it, an id already asked about is not asked about again, and an owned id mentioned during a scan is remembered as asked; verify fake-network tests for the scenario "Scan after own objects were asked about" of `avatar-commands` and for an owned object mentioned twice during a scan, and that `nothing_is_asked_about_objects_before_a_scan` still holds for objects that are not owned
- [ ] 2.3 Verify the attachment table end to end with the fake network: an owned object is mentioned, asked about, and described as an attachment of the avatar in the full form and, in a second test, in the compressed form, and `Attached` is reported with item and point each time; an owned object described as standing on the ground reports nothing; and in `Commands` with the fake grid, `attach` of an item learned this way is refused as already attached
- [ ] 2.4 Log at debug level, for each `ObjectUpdateCached`, how many objects it mentioned, how many are the account's own and how many were asked about, in the wording of design.md; verify by a run against the rig with `--verbose` that the line is absent there (the rig sends no such message) and by a unit test of the counts where the session's tests can observe them, or else by reading the line in the log of task 4.2

## 3. Documentation

- [ ] 3.1 In README.md, rewrite the paragraph of the Attachments section that says nothing is sent to find out what is attached: what the bot asks about, why it cannot ask about attachments alone, and what that costs on land where the account owns many objects; in `docs/test-rig.md`, replace "a cached one the bot would not learn of" under "Not covered" with what is now covered by unit tests only; verify both texts against the behaviour of tasks 2.1 and 2.2

## 4. Integration check

- [ ] 4.1 Run `cargo test`, `cargo fmt --check`, clippy and a release build, and repeat items 1, 3 and 9 of `docs/test-rig.md` and the attach and detach items of its inventory section on the rig with `--verbose`; verify all succeed and that the rig bot sends no `RequestMultipleObjects` other than the one about its seat
- [ ] 4.2 On Second Life, by hand with `--verbose`, on an avatar wearing at least one unscripted attachment: log in twice a few minutes apart and compare the `attachment:` lines of each login with what a viewer shows as worn; note from the new `objects:` lines whether objects of the account's own were mentioned as cached and asked about, and in which form the simulator described them; then `inventory -a`, `attach` of an item already worn (expected: refused) and `detach` by body part; record the date and the results under Status in README.md, including the case that an attachment was still missed and what the log showed then, and for anything not tried say so there
