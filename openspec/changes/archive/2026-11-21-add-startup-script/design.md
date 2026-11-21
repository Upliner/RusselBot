# Design

## Context

See proposal.md for the motivation. What shapes the approach:

- Every command reaches the bot as a `control::Request` with a `Sender<Response>` for its answer, and is carried out by `Bot::control` in `src/main.rs`. The control socket and the XMPP gateway both feed it; the gateway splits a message body into arguments with `xmpp::command::split` and then goes through `control::request_line` and `Request::decode`, the same path `rslbot ctl` takes.
- Some answers come at once (`rlv`, `perms`, `autoreply`, `persist`, `status`), others when the simulator has answered or a deadline has passed (`tp` up to 60 s, `sit` up to 15 s, the inventory commands). The gateway waits for late answers on a thread per command (`Gateway::waited`).
- The settings the deprecated keys give are read once, in `Bot::new`: `Rlv::new(config.rlv, config.rlv_persist, config.persist_override)`, `commands.set_rules(config.perms)`, `Autoreply::new(...)`, and `startup: Some(config.persist)`.
- `Bot::apply_startup` applies the config's `persist` once the config placement is reached, using `startup_due` for "reached" and for the wait of up to 10 s for the seat's position. The same `startup` field is set again by a followed forced RLV sit (`hold_seat`, with `follow_seat`), so it fires more than once in a run. It is cleared, without being applied, in `go_manual` (a command moved the avatar) and in `persist` (something else was persisted).
- The XMPP gateway is started inside `Bot::new`, before the first login.
- Config validation is all in `Config::parse` (pure, text in) and `Config::load` (reads the file), with `run_bot` adding the checks that need the outside world (log files, the socket).

## Goals / Non-Goals

**Goals:**

- One way to say what the bot starts with: the commands themselves.
- An existing config behaves exactly as before; only the log gains deprecation lines.
- A mistake in the script shows as early as a mistake in the config does today, wherever that is possible without the grid.
- The startup script does not depend on the XMPP module: leaving `src/xmpp/` out of the build must not break it.
- The script replaces `persist = "cur"` faithfully, including the wait for the seat's position.

**Non-Goals:**

- Removing the deprecated keys or their code. That is a later change, and the reason the old code paths are left as they are here.
- Deprecating `region`, `position`, `start` and `sit`. They are not the same as a `tp` and a `sit` command: the login asks for that place, the reconciler retries the sit with backoff and repositions, a session loss on the way does not lose it, and `cfg` stands for it.
- Control flow in the script: no conditions, no waiting for events, no retries, no "ignore this command's failure" marker.
- Running the script, or part of it, again after a later login.
- Saving runtime changes back to the script.
- Changing what `perms revoke` does, or how an exception under a list rule is decided.

## Decisions

### Two keys, `startup` and `startup_file`, that exclude each other

`startup` is a TOML string; the docs show it as a multi-line literal string (`'''`), in which backslashes and double quotes are the script's own and need no TOML escaping. `startup_file` names a file with the same content, for scripts that are long or shared between bots. Giving both is an error rather than a concatenation: there is no obvious order, and no need. A relative `startup_file` is relative to the working directory, as `log_chat` and `log_im` are; relative to the config file would be as defensible, but two rules for paths in one config are worse than either.

`Config::parse` checks the keys' types and parses an inline script; `Config::load` reads the file `startup_file` names and parses it. Both end in the same `Config` field, a list of parsed lines, so nothing after loading knows where the script came from.

*Alternative considered:* one key that is either a string or `{ file = "..." }`. Shorter to list, harder to explain, and a second TOML shape for something that is meant to remove TOML shapes.

### The script is command lines, read by code that belongs to the commands

Each command is split into arguments and decoded by `Request::decode`, so the script takes exactly the commands `rslbot ctl` takes, and a new command is in the script for free. The quoting is the one the XMPP control contact already has, so the operator learns it once; but the script must not depend on the gateway, which is to become an optional feature.

The splitting therefore moves out of the XMPP module: `split` and `MAX_COMMAND` go from `src/xmpp/command.rs` to `src/control.rs`, next to `request_line` and `Request::decode`, with their tests. `src/control.rs` is where the command model lives and is what the gateway already builds on, so the dependency keeps its one direction: the gateway uses `control`, `src/startup.rs` uses `control`, and neither uses the other. `src/xmpp/command.rs` keeps only what is the gateway's own, the `help` text. Nothing the script needs stays under `src/xmpp/`, and nothing in `src/startup.rs`, or in the code of `src/main.rs` that drives the script, names the `xmpp` module; the tests of the script sit outside the gateway's test module. The spec follows the same line: `startup-script` states the splitting rule itself instead of pointing at `xmpp-gateway`.

*Alternative considered:* a module of its own for command lines (`src/cmdline.rs`). It would only pay off if the control socket became optional too; `Request` and its decoding would have to move with it, which is a larger reshuffle than this change needs.

On top of that the script needs only two things: lines, and comments. Commands are separated at line ends outside double quotes; a line end inside quotes stays in the argument. That is what lets `autoreply message` carry a text of several lines, which `[autoreply]` allows, without a new escape sequence that the gateway's splitting would then have to learn too. A line whose first non-blank character is `#` is a comment; a `#` further in is text, because chat text may hold one. Each parsed command keeps the number of the line it starts on, for errors and the log.

*Alternative considered:* a TOML array of strings, or of argument arrays. No quoting questions, but it cannot be an external file, and it is not what the operator types elsewhere.

### Setting commands at the head run before the first login; the rest when the config placement is reached

This is the decision the rest hangs on. The candidates:

1. *Everything before the first login.* Then `persist` cannot mean "where the avatar is now", and `persist = "cur"`, the most used form, has no replacement.
2. *Everything once the config placement is reached.* Then `rlv mode on` comes too late: attachments say their restrictions in the first second after the login, and a bot whose configured seat is gone would never get its auto-accept rules or its autoreply.
3. *Each command when it can run.* "Can run" depends on arguments (`persist cfg cfg` can run offline, `persist` cannot), and what the operator wants from `persist cfg cfg` is still "after sitting".
4. *Chosen: by class of command, in order.* The setting commands (`perms`, `autoreply`, `rlv` except `rlv cmd`) are the ones that work "in every state of the bot, connected or not" and do not touch the placement mode. Those at the head of the script run at once; the first other command, and everything after it, waits for the config placement. Order is kept, so a setting command further down runs when it is reached.

The rest is released at the very moment the config's `persist` is applied today, which keeps the definition of "the config placement is reached" in one place (`startup_due`) and makes `persist` in the script equal to `persist = "cur"`. For that equality the 10 s wait for the seat's position has to happen whenever a rest is waiting, not only when the deprecated setting's location is `cur`: `startup_due` is told that there is a rest. A deprecated `persist = "cfg"` together with a script is thereby applied up to 10 s later than before; nothing depends on when.

A config without `persist` goes to manual mode at that moment, as today, and the rest then runs; a `persist` command in it puts the bot back in holding mode. The log shows `placement: manual` followed by `placement: holding ...` in the same second. Suppressing the first line would need the bot to look ahead in the script; it is not worth it.

### A refused setting command at the head is a config error

The head runs before any network activity, so its refusals (`perms autoaccept money all`, `autoreply pause soon`, `rlv mode override` while RLV is off) can be reported the way config errors are: message, exit status 2, nothing sent. This keeps what the deprecated tables had, validation before the bot shows itself, without a second validator: the commands validate themselves by being carried out.

For this the head has to run before anything connects, which today includes the XMPP gateway. The gateway's start moves out of `Bot::new` into a step `run_bot` takes after the head has run, so the script's side of this is only "the head runs right after `Bot::new`" and knows nothing of the gateway; the order in `run_bot` becomes: load config, open logs, bind the socket, build the bot, run the head, start the gateway, enter the loop. Setting commands answer synchronously (they are handled in `Bot::control` and `Commands` without the session), so the head is a plain loop that reads each answer right after the call.

Everything else in the script is checked at load only for its form, which is what `Request::decode` refuses. Whether `sit <uuid>` will work cannot be known before it is due.

*Alternative considered:* log a refused head command and go on. A bot that silently runs without the rule or the autoreply its config asked for is the failure the existing validation exists to prevent.

### A refused command of the rest stops the script; no failure-tolerant marker

Commands in a script usually build on each other (`tp`, then `sit`, then `persist`): going on after a failed `tp` would sit on something in the wrong region and persist the wrong place. Stopping is the safe default, and the bot stays up in whatever state the script left. A marker for "go on if this fails" (as `-` in a Makefile) is easy to add later and is left out until someone needs it.

The script also does not wait for a reconnect. If the session is lost while the rest runs, the command in flight is answered with an error by its own deadline or by the session's end, and the script stops. Resuming would mean deciding which commands are safe to repeat.

### The rest is driven from the main loop, one command in flight

A small state in a new module `src/startup.rs`, owned by `Bot`: the remaining lines and one of *head pending*, *waiting for the placement*, *running* (with the receiver of the command in flight, if any), *done*. `run_loop` gives it a turn in every pass: when running and nothing is in flight, it hands the next command to `Bot::control` with a fresh channel; then it polls the receiver without blocking. All answers are sent from the main thread during a pass (from `control` itself, from `commands.tick`, or from an event handler), so polling once per pass sees every answer in the pass that produced it, and no thread or new `Event` is needed. When an answer is taken and commands remain, the loop goes round again at once instead of sleeping, as it does today after `apply_startup`. A receiver whose sender is gone without an answer counts as a refusal.

`Bot::control` returns `Flow`; a `quit` in the script returns `Flow::Exit` through the same path as the socket's.

*Alternative considered:* a thread that feeds `Event::Control` and blocks on each answer, as a `ctl` client would. It needs no polling, but the head must be synchronous anyway, and "the operator acted first" would have to reach the thread.

### Giving the rest up when the operator acts first

The rule is the existing one for the config's `persist`, so it sits where `startup` is cleared without being applied: `go_manual` and `Bot::persist`. There the script, if it is still *waiting for the placement*, goes to *done* and logs how many commands it drops. `apply_startup` moves the script to *running* before it calls `Bot::persist` for the deprecated setting, so the script's release and its own later `persist` commands do not give it up.

The script's state is separate from the `startup` field on purpose: that field is set again for a followed forced RLV sit, and the script must not be released, or given up, by those later rounds. A followed forced sit that happens before the config placement is reached ends in a persist, which gives the rest up under the rule above; that matches what happens to the config's `persist` today.

### Script commands are the operator's commands

They go through `Bot::control` with no origin of their own, so RLV restrictions, `rlv mode override`, the manual-mode rule and the message log treat them as the socket's. The only new thing is the `startup:` lines in the log around each command; the `control:` lines the commands write themselves stay.

*Alternative considered:* an exempt origin, so that restrictions never stop the script. The config's own teleports and sits are held back by restrictions unless `persist override` says otherwise; a script that ignored them would be a way around that.

### Deprecated keys keep their code and gain a log line

The four keys keep feeding `Bot::new` as they do now, so they are in place before the head runs, and the deprecated `persist` is applied in `apply_startup` before the rest is released. `Config` collects one note per deprecated key it met, built while parsing, where the values are at hand to spell out the replacing commands (`persist cfg cur`, `rlv persist override sit tp`, ...); `run_bot` logs them next to the existing line for `object`.

*Alternative considered:* translate the deprecated keys into script lines and delete their code now. One mechanism sooner, but it changes the log of every existing config, cannot express an exception under a list rule, and rewrites code that the removal will delete anyway.

`[perms.except]` for a kind whose rule is not `all` is the one thing the deprecated keys can say and commands cannot. It was never the table's purpose, so no command is added for it; the note for `perms` names such kinds so that the exception is not lost unnoticed when a config is rewritten.

## Risks / Trade-offs

- [A script that stops early leaves the bot half set up, and only the log says so] → The stop is logged with the line, the reason and the number of commands dropped; setting commands belong at the head, where a refusal stops the bot instead.
- [A setting command placed after a movement command runs late, or never if the config placement is never reached] → Documented as the one rule of the script: settings first. The example config and the README show that order.
- [A command that never answers would stall the script] → Every command that waits has its own deadline in `Commands` (60 s for `tp`, 15 s for `sit`, the inventory deadlines); a dropped sender counts as a refusal. No extra timeout is added.
- [The head runs commands before the bot has a session; a command that turns out to need one would panic or misbehave] → The setting commands are exactly those the specs already require to work between logins, and tests already run them on a bot without a session.
- [Moving the gateway's start out of `Bot::new` touches the tests that build a bot] → Only the gateway tests' `Rig::new` in `src/main.rs` relies on `Bot::new` starting the gateway; it calls the new step itself, and the existing gateway tests check the move.
- [Two ways to set the same thing while the deprecation lasts] → The order is fixed (keys first, then the script), stated in the spec, and covered by a scenario.
- [TOML basic strings (`"""`) eat backslashes, so `\"` in a script would arrive as `"`] → The docs and the example use literal strings (`'''`), and say why.

## Migration Plan

Nothing has to be done for an existing config: it behaves as before and logs what to write instead. To migrate, replace each deprecated key with the commands its deprecation line names, setting commands first, `persist` last. Rolling back is going back to the old keys; both forms are accepted for as long as the keys exist.

## Open Questions

- When the deprecated keys are removed (the next release, or after a period of use) is left to the change that removes them.
