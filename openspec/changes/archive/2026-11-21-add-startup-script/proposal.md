# Proposal

## Why

Four settings can be given in two ways: as a config key that says what the bot starts with, and as a command that changes it while the bot runs (`persist`, `rlv mode`, `rlv persist`, `perms autoaccept`, `autoreply`, ...). Each of them has its own TOML shape, its own validation and its own wording in the docs next to the command that does the same thing, and every new setting adds another pair. A startup script, a list of the commands the operator would type, replaces the config half: one mechanism, one syntax, and anything a command can do can be done at startup.

## What Changes

- New config key `startup`: a string of command lines the bot carries out by itself when it starts, written as `rslbot ctl` takes them, without `rslbot ctl` in front:

  ```toml
  startup = '''
  # settings: carried out before the first login
  rlv mode on
  rlv persist follow
  perms autoaccept tp "Jane Doe"
  autoreply message Hello, I'm bot. I won't respond to you in next 2 hours
  autoreply pause 2h
  # the rest: carried out once the avatar is in place
  persist cur cfg
  '''
  ```

  `startup_file = "startup.rsl"` names a file with the same content instead; the two keys exclude each other.
- The script is one command per line, with blank lines and `#` lines skipped. A line end inside double quotes belongs to the argument, so an autoreply text may span lines.
- When the commands run:
  - The setting commands at the head of the script (`perms`, `autoreply`, and `rlv` other than `rlv cmd`) are carried out before the first login, so that RLV is on and the auto-accept rules are there when the avatar arrives, as they are with the config keys today.
  - The rest starts the first time the config placement is reached (logged in at `region`/`position`, seated when `sit` is set), at the moment the config's `persist` setting is applied today. Commands run in order, one at a time, each after the answer of the one before.
- A script is checked before any network activity: an unknown command, a wrong number of arguments or an unclosed quote is a config error (exit status 2) that names the line, and so is a setting command at the head that is refused. A command of the rest that is refused later stops the script there; the bot keeps running, and the operator log says what was not carried out.
- The script runs once in a run of the bot, not again after a session loss. As with the config's `persist` today, the rest is not run when the operator moves the avatar or persists something before the config placement is reached.
- The config keys the script replaces are deprecated: `persist` in all its forms (string, array and the `[persist]` table with `atstart`, `position`, `sit`, `rlv` and `override`), `rlv`, `[perms]` and `[autoreply]`. They keep working exactly as today and are applied before the script; for each one a config uses, the bot logs a line at startup that names the key as deprecated and the script commands that replace it, as it does for `object`. Removing them is a later change.
- Not deprecated: `region`, `position`, `start` and `sit` (the config placement: where the bot logs in, what it retries with backoff, and what `cfg` stands for), and the keys no command sets (`login_url`, `user`, the password keys, `log_chat`, `log_im`, `fetch_offline_im`, `socket`, `requesttp_timeout`, `scan_distance`, `restore_attachments`, `[xmpp]`).
- No command changes. `perms autoaccept` and `perms revoke` are enough for the rules: `[perms.except]` was meant for kinds whose rule is `all`, which `perms revoke` after `perms autoaccept KIND all` gives. An exception for a kind whose rule is a list, which the deprecated table still accepts, has no command; the deprecation line says so when a config has one.

## Capabilities

### New Capabilities
- `startup-script`: the commands the bot carries out by itself when it starts: where the script comes from, how it is written, which commands run before the first login and which wait for the config placement, the order, what a refused command does, and what the operator log shows.

### Modified Capabilities
- `bot-config`: new keys `startup` and `startup_file` and their validation, including the script's form and the setting commands at its head; the keys `persist`, `rlv`, `perms` and `autoreply` become deprecated, with a line in the log that names their replacement.

## Impact

- Code: a new module for reading the script and keeping track of where it is; `src/config.rs` (the two keys, reading the file, the deprecation notes); `src/main.rs` (carrying out the head before the XMPP gateway starts and before the first login, releasing the rest when the config placement is reached, giving it up when the operator acts first, logging); `src/control.rs` and `src/xmpp/command.rs` (the splitting of a command line into arguments moves out of the XMPP module into `src/control.rs`, so that the script does not depend on the gateway; usage text no longer points at the deprecated tables).
- Behaviour of an existing config is unchanged apart from the deprecation lines in the log.
- Docs: `README.md` (the config table, a section on the startup script with a migration table), `russelbot.example.toml` (rewritten around `startup`), `docs/test-rig.md` (new check items).
- No new dependencies.
