# bot-config Specification

## Purpose

Defines the single configuration file the bot runs from, how it is validated, and the helper command that produces a password hash so a plain password need not be stored.

## Requirements

### Requirement: Single TOML config file
The bot SHALL read all of its settings from one TOML file. The file path SHALL be taken from the first command-line argument and SHALL default to `russelbot.toml` in the current directory when no argument is given. The file SHALL support these keys:

| Key | Required | Meaning |
|---|---|---|
| `login_url` | no | Grid login URL; defaults to the Second Life Agni login URL `https://login.agni.lindenlab.com/cgi-bin/login.cgi` |
| `user` | yes | Avatar name, either `First Last` or a single word (the last name is then `Resident`) |
| `password` | one of the two | Plain password |
| `password_hash` | one of the two | Password hash as produced by the `hash` subcommand |
| `region` | no, but only together with `position` | Region of the config placement. When present the bot logs in there at startup and holds that region until a persist changes the held placement |
| `position` | no, but only together with `region` | Coordinates of the config placement inside the region, as an array of three numbers `[x, y, z]` |
| `start` | no, and only without `region` and `position` | Where a login places the avatar while the bot holds no location: `"home"` for the avatar's home location, `"latest"` for the location it was last at. Defaults to `"latest"` |
| `sit` | no | Sit state of the config placement: the UUID of an object, or the word `ground`. When absent the config placement's sit state is `stand` and the bot does not seat the avatar at startup |
| `object` | no | Deprecated spelling of `sit`, accepting a UUID only |
| `persist` | no | Deprecated in favour of the startup script, as said below. What the bot holds once it has reached the config placement after startup (see `placement-persist`), and in its table form also what a forced RLV movement does to the held placement and which of the bot's own steps RLV restrictions do not hold back (see `rlv-commands`); the forms are given below. Defaults to `"off"` |
| `log_chat` | no, and not together with `chat` in the `logs` table | Deprecated spelling of `chat` in the `logs` table |
| `log_im` | no, and not together with `im` in the `logs` table | Deprecated spelling of `im` in the `logs` table |
| `logs` | no | Table with the paths of the log files (see `message-log` and `transaction-log`); its form is given below. Without it, and without the two deprecated keys, the bot writes no log file |
| `fetch_offline_im` | no | Boolean; when true the bot fetches instant messages stored while the avatar was offline. Defaults to false |
| `socket` | no | Boolean; when true the bot listens on the control socket. Defaults to false |
| `requesttp_timeout` | no | Number of minutes, greater than zero, during which the bot accepts a teleport offer from an avatar it has asked with `requesttp`. Defaults to 10 |
| `scan_distance` | no | Number of metres, from 1 to 1024. When present the bot learns the objects within that distance from the moment it enters a region, for the `touch` command. When absent it learns them only when `touch` is first used, within 32 metres |
| `restore_attachments` | no | Boolean; when true the bot attaches again, after each login, the objects the avatar's Current Outfit folder lists and the simulator did not bring (see `avatar-outfit`, "Restoring attachments after a login"). Defaults to true |
| `rlv` | no | Deprecated in favour of the startup script. Boolean; when true the bot starts with RLV on and reads the RLV commands of the objects the avatar owns (see `rlv-commands`). Defaults to false |
| `perms` | no | Deprecated in favour of the startup script. Table saying which requests the bot accepts by itself (see `permission-requests`); its form is given below. Without it the bot accepts none by itself |
| `xmpp` | no | Table that switches on the XMPP gateway (see `xmpp-gateway`); its form is given below. Without it the bot makes no XMPP connection |
| `web` | no | Table that switches on the web interface (see `web-server`); its form is given below. Without it the bot listens on no web address |
| `payments` | no | Table that switches on payments (see `payments`); its form is given below. Without it the bot pays nothing |
| `autoreply` | no | Deprecated in favour of the startup script. Table that switches on autoreplies to instant messages (see `im-autoreply`); its form is given below. Without it the bot starts with autoreply off and no autoreply text |
| `startup` | no, and not together with `startup_file` | The startup script, a string of one or more lines: commands the bot carries out by itself when it starts (see `startup-script`) |
| `startup_file` | no, and not together with `startup` | Path of a file that holds the startup script |

`persist` SHALL be accepted in these forms:

| Form | Meaning |
|---|---|
| `"cur"` | Location and sit state both `cur`: hold what the simulator reports once the config placement is reached |
| `"cfg"` | Location and sit state both `cfg`: keep holding the config placement |
| `"off"` or `false` | Hold nothing: switch to manual mode once the config placement is reached. The two spellings are equivalent, and a config without the key behaves the same |
| An array of two strings, each `"cur"` or `"cfg"` | The first is the value for the location, the second the value for the sit state |
| A table with any of the keys `atstart`, `position`, `sit`, `rlv` and `override` | As described below the table |

In the table form, `atstart` is a boolean that says whether the bot holds anything once the config placement is reached; `position` and `sit`, each `"cur"` or `"cfg"`, are the values for the location and for the sit state; `rlv`, `"drop"` or `"follow"`, is the RLV persist setting the bot starts with (see `rlv-commands`, "RLV persist setting"); and `override`, the string `"sit"` or `"tp"` or an array of these strings, is the persist override setting the bot starts with (see `rlv-commands`, "Persist override setting"). `atstart` SHALL default to true when the table has `position` or `sit`, and to false otherwise. While `atstart` is true, a `position` or `sit` that is left out of the table is `"cur"`, so `atstart = true` alone is the same as `persist = "cur"`. While `atstart` is false the bot holds nothing once the config placement is reached, as with `persist = "off"`. The `rlv` and `override` keys SHALL NOT affect `atstart`. The RLV persist setting SHALL be `"drop"` when the table has no `rlv` key, with every other form of `persist`, and without `persist`. The persist override setting SHALL be empty when the table has no `override` key or gives an empty array, with every other form of `persist`, and without `persist`; a string given twice in the array SHALL count once.

`perms` SHALL be a table with any of the keys `autoaccept` and `except`, each of them a table whose keys are kinds of request: `tp`, `inventory`, `anim`, `attach`, `controls` and `camera`. A value under `autoaccept` is the auto-accept rule the bot starts with for that kind: the string `"all"` or the boolean `true` for the rule `all`, an array of entries for a list, and the boolean `false` for an empty list, which is also the rule of a kind without a key. A value under `except` SHALL be an array of entries, the exceptions the bot starts with for that kind. An entry SHALL be a string holding a UUID or an avatar's name, written `First Last`, `first.last` or as a single word (see `permission-requests`, "Auto-accept rules"). An entry that is given twice in one array SHALL count once.

`xmpp` SHALL be a table with these keys:

| Key | Required | Meaning |
|---|---|---|
| `component` | yes | The domain the bot's component owns on the XMPP server, such as `russel.sl.example.org`. Letter case does not matter; the bot uses it in lower case |
| `server` | no | `host:port` of the XMPP server's component port. The host SHALL be `localhost` or a loopback address. Defaults to `127.0.0.1:5347` |
| `secret` | yes | The shared secret the XMPP server has for that component |
| `owners` | yes | A non-empty array of bare XMPP addresses, `user@domain`, of the accounts that may use the gateway. Letter case does not matter, and an address given twice SHALL count once |

`web` SHALL be a table with these keys:

| Key | Required | Meaning |
|---|---|---|
| `listen` | yes | `host:port` of the address the web interface listens on, such as `127.0.0.1:8080`. The host SHALL be `localhost` or an IP address, an IPv6 address in square brackets; the port a number from 1 to 65535 |
| `password` | no | The password the page of the web interface asks for, a non-empty string. Without it the web interface asks for none |

`logs` SHALL be a table with these keys:

| Key | Required | Meaning |
|---|---|---|
| `chat` | no | Path of the TSV file that receives local chat; no chat file is written when absent |
| `im` | no | Path of the TSV file that receives instant messages; no IM file is written when absent |
| `transactions` | no, but required when `payments.limit` is a number | Path of the TSV file that receives the avatar's transactions (see `transaction-log`); no transactions file is written, and none is read at startup, when absent |

`payments` SHALL be a table with these keys:

| Key | Required | Meaning |
|---|---|---|
| `limit` | yes | What the bot may pay in one period: a whole number of L$ from 0 to 2147483647, or the string `"unlimited"` |
| `perms` | yes | The interfaces that may pay: an array of the strings `"ctl"`, `"xmpp"` and `"web"`, which may be empty. A string given twice SHALL count once. An interface the config does not switch on may be listed and then has no effect |
| `limit_renew` | no | The length of a period of the limit, as a duration greater than zero. Without it the limit never renews. With `limit = "unlimited"` it has no effect |
| `whitelist` | no | Array of entries: with it, only who is on it or on `unlimited_list` can be paid |
| `blacklist` | no | Array of entries: who is on it is never paid |
| `unlimited_list` | no | Array of entries: a payment to who is on it does not count against the limit |

An entry of the three lists SHALL be a string holding a UUID, of an avatar or of an object, or an avatar's login name, written `First Last`, `first.last` or as a single word, in the forms `permission-requests`, "Auto-accept rules", gives for an entry; a display name is no entry (see `payments`, "Login names, not display names"). An entry that is given twice in one array SHALL count once.

`autoreply` SHALL be a table with these keys:

| Key | Required | Meaning |
|---|---|---|
| `message` | yes | The autoreply text: a string that is not blank, at most 1023 bytes long. It may span several lines |
| `pause` | no | How long the bot stays quiet towards an avatar after it has answered it, as a duration. Defaults to `"1h"` |

A duration SHALL be a string of one or more groups, each a whole number followed by one of the units `s` (seconds), `m` (minutes), `h` (hours) and `d` (days), written without spaces, such as `"90s"`, `"30m"`, `"2h"` or `"1h30m"`, or the string `"0"`. A unit SHALL appear at most once, and the units SHALL be in the order `d`, `h`, `m`, `s`. The duration is the sum of the groups and SHALL be at most 365 days. A config with an `autoreply` table SHALL start the bot with autoreply on.

A relative path under `logs`, and a relative `log_chat`, `log_im` or `startup_file` path, SHALL be taken relative to the directory the bot was started in. A config that uses `log_chat` or `log_im` SHALL behave exactly as the same config with that path under `chat` or `im` of the `logs` table, and for each of the two keys the config contains the bot SHALL write a line to standard error at startup that names the key as deprecated and what to write under `[logs]` instead. A config that uses `object` SHALL behave exactly as the same config with that value under `sit`, and the bot SHALL write a line to standard error at startup that names `object` as deprecated and `sit` as its replacement.

The keys `persist`, `rlv`, `perms` and `autoreply` are deprecated in favour of the startup script. A config that uses them SHALL behave as this requirement says, and what they set SHALL be in place before the first command of the startup script is carried out: the bot starts with what `rlv`, `perms`, `autoreply` and the `[persist]` keys `rlv` and `override` give, and it applies the `persist` setting before the rest of the script starts (see `startup-script`, "The rest waits for the config placement"). For each of these four keys the config contains, the bot SHALL write a line to standard error at startup that names the key as deprecated and the startup script commands that replace what the config gives for it:

| Deprecated | Commands in the startup script |
|---|---|
| `persist` as `"cur"`, `"cfg"` or an array, and the `[persist]` keys `atstart`, `position` and `sit` | `persist <location> <sit state>` with the same words `cur` and `cfg`, after the setting commands |
| `persist = "off"` or `persist = false` | None: the key is left out |
| `rlv` in the `[persist]` table | `rlv persist follow` or `rlv persist drop` |
| `override` in the `[persist]` table | `rlv persist override` with `sit`, `tp` or both |
| `rlv = true` | `rlv mode on` |
| A kind under `[perms.autoaccept]` | `perms autoaccept <kind> all`, or `perms autoaccept <kind>` with the entries |
| A kind under `[perms.except]` | `perms revoke <kind>` with the entries, after `perms autoaccept <kind> all` |
| `[autoreply]` | `autoreply message` with the text and, when the table has a `pause`, `autoreply pause` with it |

An exception under `[perms.except]` for a kind whose rule is not `all` has no command; when the config has one, the line for `perms` SHALL say that it cannot be written in the startup script.

#### Scenario: Minimal config for Second Life
- **WHEN** the config contains only `user` and `password_hash`
- **THEN** the bot starts, uses the Second Life Agni login URL, logs in at the avatar's last location, does not sit the avatar, and is in manual mode from then on

#### Scenario: Full placement
- **WHEN** the config contains `region`, `position` and `sit` with the UUID of an object
- **THEN** the bot logs in at that position, sits on that object, and is in manual mode from then on: it neither teleports nor sits again by itself

#### Scenario: Ground sit
- **WHEN** the config contains `sit = "ground"`
- **THEN** the bot sits the avatar on the ground

#### Scenario: Start at home
- **WHEN** the config contains `start = "home"` and neither `region` nor `position`
- **THEN** the first login asks for the avatar's home location, and later logins ask for the location the bot holds

#### Scenario: Old key
- **WHEN** the config contains `object` with a UUID and no `sit`
- **THEN** the bot behaves as with `sit` set to that UUID and its log names `object` as deprecated in favour of `sit`

#### Scenario: Persist not mentioned
- **WHEN** the config has no `persist` key
- **THEN** the bot behaves as with `persist = "off"`

#### Scenario: Persist as the simulator reports
- **WHEN** the config contains `region`, `position`, `sit` with the UUID of an object and `persist = "cur"`
- **THEN** the bot logs in at that position, sits on that object, and from then on holds the location and seat the simulator reported

#### Scenario: Persist as the config says
- **WHEN** the config contains `region`, `position`, `sit` and `persist = "cfg"`
- **THEN** the bot keeps holding the configured position and the configured `sit` UUID after it has sat

#### Scenario: Persist off
- **WHEN** the config contains `persist = "off"` or `persist = false`
- **THEN** the bot reaches the config placement once and then is in manual mode

#### Scenario: Persist as an array
- **WHEN** the config contains `persist = ["cfg", "cur"]`
- **THEN** the bot holds the configured position and the seat the simulator reported

#### Scenario: Persist as a table
- **WHEN** the config ends with a `[persist]` table containing `position = "cur"` and `sit = "cfg"`
- **THEN** the bot holds the location the simulator reported and the configured `sit`

#### Scenario: Persist table with one key
- **WHEN** the config ends with a `[persist]` table containing only `sit = "cfg"`
- **THEN** the location value is `cur` and the sit state value is `cfg`

#### Scenario: Config path from the command line
- **WHEN** the bot is started with a file path as its first argument
- **THEN** it reads that file and not `russelbot.toml`

#### Scenario: Single-word user name
- **WHEN** `user` is a single word such as `sitbot`
- **THEN** the bot logs in with first name `sitbot` and last name `Resident`

#### Scenario: No logging keys
- **WHEN** the config contains none of `logs`, `log_chat`, `log_im` and `fetch_offline_im`
- **THEN** the bot starts, writes no log file and does not fetch offline instant messages

#### Scenario: Chat log configured
- **WHEN** the `[logs]` table contains `chat = "chat.tsv"`
- **THEN** the bot creates `chat.tsv` in the directory it was started in if it does not exist, and appends chat rows to it

#### Scenario: All three log files
- **WHEN** the `[logs]` table contains `chat = "chat.tsv"`, `im = "im.tsv"` and `transactions = "transactions.tsv"`
- **THEN** local chat, instant messages and transactions each go to their own file

#### Scenario: Old log keys
- **WHEN** the config contains `log_chat = "chat.tsv"` and `log_im = "im.tsv"` and no `[logs]` table
- **THEN** the bot writes both files as with `chat` and `im` under `[logs]`, and its log names `log_chat` and `log_im` as deprecated and shows the `[logs]` table to write instead

#### Scenario: Old and new key for different files
- **WHEN** the config contains `log_chat = "chat.tsv"` and a `[logs]` table with `im = "im.tsv"` only
- **THEN** the bot writes both files, and its log names `log_chat` as deprecated

#### Scenario: Payments not mentioned
- **WHEN** the config has no `[payments]` table
- **THEN** the bot starts, pays nothing on any command, and does not ask the grid for the avatar's balance

#### Scenario: Payments table
- **WHEN** the config has a `[payments]` table with `limit = 1000`, `limit_renew = "7d"` and `perms = ["ctl", "web"]`, and `transactions` under `[logs]`
- **THEN** the bot starts, and `pay` is carried out from the control socket and from the web interface, within L$1000 in seven days

#### Scenario: Unlimited without a transactions file
- **WHEN** the `[payments]` table has `limit = "unlimited"` and `perms = ["ctl"]`, and the config has no `[logs]` table
- **THEN** the bot starts and `rslbot ctl pay` is carried out without a limit

#### Scenario: Interface that is not switched on
- **WHEN** the `[payments]` table has `perms = ["ctl", "xmpp"]` and the config has no `[xmpp]` table
- **THEN** the bot starts as with `perms = ["ctl"]`

#### Scenario: Control socket enabled
- **WHEN** the config contains `socket = true`
- **THEN** the bot listens on the control socket

#### Scenario: Control socket not mentioned
- **WHEN** the config has no `socket` key or has `socket = false`
- **THEN** the bot does not listen on any control socket

#### Scenario: Teleport request timeout configured
- **WHEN** the config contains `requesttp_timeout = 30`
- **THEN** a teleport offer from an avatar the bot asked 20 minutes ago is accepted

#### Scenario: Scan distance configured
- **WHEN** the config contains `scan_distance = 64`
- **THEN** the bot starts learning the objects within 64 metres as soon as it has entered a region

#### Scenario: Restoring attachments is on by default
- **WHEN** the config does not contain `restore_attachments`
- **THEN** the bot restores attachments after a login as `avatar-outfit` describes

#### Scenario: Restoring attachments switched off
- **WHEN** the config contains `restore_attachments = false`
- **THEN** the bot starts normally and never asks the grid about the Current Outfit folder unless a command needs it

#### Scenario: Persist table with atstart alone
- **WHEN** the config ends with a `[persist]` table containing only `atstart = true`
- **THEN** the bot behaves as with `persist = "cur"`

#### Scenario: Persist table with only the RLV setting
- **WHEN** the config ends with a `[persist]` table containing only `rlv = "follow"`
- **THEN** the bot is in manual mode once the config placement is reached, as with `persist = "off"`, and its RLV persist setting is `follow`

#### Scenario: Empty persist table
- **WHEN** the config ends with a `[persist]` table that has no keys
- **THEN** the bot behaves as with `persist = "off"`

#### Scenario: Persist table with a value and the RLV setting
- **WHEN** the config ends with a `[persist]` table containing `sit = "cfg"` and `rlv = "follow"`
- **THEN** the location value is `cur`, the sit state value is `cfg`, and the RLV persist setting is `follow`

#### Scenario: RLV persist setting not mentioned
- **WHEN** the config contains `persist = "cfg"`, or a `[persist]` table without `rlv`, or no `persist` at all
- **THEN** the RLV persist setting is `drop`

#### Scenario: RLV not mentioned
- **WHEN** the config has no `rlv` key or has `rlv = false`
- **THEN** the bot starts with RLV off, as `rlv-commands` describes

#### Scenario: RLV enabled
- **WHEN** the config contains `rlv = true`
- **THEN** the bot starts with RLV on

#### Scenario: Perms not mentioned
- **WHEN** the config has no `perms` table
- **THEN** the bot starts with an empty auto-accept list and no exceptions for every kind

#### Scenario: Auto-accept lists
- **WHEN** the config ends with a `[perms.autoaccept]` table containing `tp = ["Avatar1", "Avatar2"]` and `inventory = ["Owner Resident", "11111111-2222-3333-4444-555555555555"]`
- **THEN** the bot starts with those two entries as the rule for `tp`, those two as the rule for `inventory`, and empty lists for the other kinds

#### Scenario: All with an exception
- **WHEN** the config has `anim = "all"` under `[perms.autoaccept]` and `anim = ["01234567-9abc-defe-dcba-987654321000"]` under `[perms.except]`
- **THEN** the bot starts with the rule `all` for `anim` and that UUID as its one exception

#### Scenario: Booleans
- **WHEN** the config has `attach = true` and `controls = false` under `[perms.autoaccept]`
- **THEN** the bot starts with the rule `all` for `attach` and an empty list for `controls`

#### Scenario: Exceptions without a rule
- **WHEN** the config has a `[perms.except]` table and no `[perms.autoaccept]` table
- **THEN** the bot starts normally, with the exceptions and an empty list for every kind

#### Scenario: Persist override for teleports
- **WHEN** the config ends with a `[persist]` table containing `sit = "cfg"` and `override = "tp"`
- **THEN** the sit state value is `cfg`, and the persist override setting is `tp`

#### Scenario: Persist override as an array
- **WHEN** the config ends with a `[persist]` table containing `override = ["sit", "tp"]` or `override = ["tp", "sit", "tp"]`
- **THEN** the persist override setting is `sit` and `tp`

#### Scenario: Table with only an override
- **WHEN** the config ends with a `[persist]` table containing only `override = "sit"`
- **THEN** the bot is in manual mode once the config placement is reached, as with `persist = "off"`, and its persist override setting is `sit`

#### Scenario: Persist override not mentioned
- **WHEN** the config contains `persist = "cfg"`, a `[persist]` table without `override` or with `override = []`, or no `persist` at all
- **THEN** the persist override setting is empty

#### Scenario: XMPP gateway configured
- **WHEN** the config ends with an `[xmpp]` table containing `component = "Russel.SL.example.org"`, `secret = "s3cret"` and `owners = ["me@example.org"]`
- **THEN** the bot starts its component as `russel.sl.example.org`, connects to `127.0.0.1:5347`, and lets only `me@example.org` use it

#### Scenario: XMPP server port given
- **WHEN** the `[xmpp]` table also contains `server = "localhost:5348"`
- **THEN** the bot connects its component to port 5348 on the local host

#### Scenario: XMPP not mentioned
- **WHEN** the config has no `[xmpp]` table
- **THEN** the bot makes no XMPP connection

#### Scenario: Autoreply configured
- **WHEN** the config ends with an `[autoreply]` table containing `message = "Hello, I'm bot. I won't respond to you in next 2 hours"` and `pause = "2h"`
- **THEN** the bot starts with autoreply on, that text, and a pause of two hours

#### Scenario: Autoreply without a pause
- **WHEN** the `[autoreply]` table contains only `message`
- **THEN** the bot starts with autoreply on and a pause of one hour

#### Scenario: No autoreply table
- **WHEN** the config has no `autoreply` key
- **THEN** the bot starts with autoreply off and answers no instant message by itself

#### Scenario: Startup script in the config
- **WHEN** the config contains `startup` with the lines `rlv mode on` and `persist`
- **THEN** the bot starts with RLV on and, once the config placement is reached, holds where the avatar is, as `startup-script` describes

#### Scenario: Startup script in a file
- **WHEN** the config contains `startup_file = "startup.rsl"` and the directory the bot was started in has that file
- **THEN** the bot carries out the commands in that file as it would the same text under `startup`

#### Scenario: No startup script
- **WHEN** the config has neither `startup` nor `startup_file`
- **THEN** the bot starts and carries out no startup script

#### Scenario: Deprecated key named in the log
- **WHEN** the config contains `rlv = true`
- **THEN** the bot starts with RLV on, and its log names `rlv` as deprecated in favour of the startup script command `rlv mode on`

#### Scenario: Deprecated persist named in the log
- **WHEN** the config contains `persist = ["cfg", "cur"]`
- **THEN** the bot holds the configured position and the seat the simulator reported, and its log names `persist` as deprecated in favour of the startup script command `persist cfg cur`

#### Scenario: No deprecated key
- **WHEN** the config has none of `object`, `persist`, `rlv`, `perms` and `autoreply`
- **THEN** the bot's log has no line about a deprecated key

#### Scenario: Deprecated key and a script
- **WHEN** the config contains `rlv = true` and `startup` with the line `rlv mode off`
- **THEN** RLV is off when the bot logs in

#### Scenario: Deprecated persist and a script
- **WHEN** the config contains `region`, `position`, `sit`, `persist = "cfg"` and `startup` with the line `say hello`
- **THEN** once the avatar is seated the bot holds the config placement, and the avatar then says `hello`

#### Scenario: Exception that no command gives
- **WHEN** the config has `anim = ["Jane Doe"]` under `[perms.autoaccept]` and `anim = ["01234567-9abc-defe-dcba-987654321000"]` under `[perms.except]`
- **THEN** the bot starts with that rule and that exception, and the line that names `perms` as deprecated says that the exception for `anim` cannot be written in the startup script

#### Scenario: Web interface configured
- **WHEN** the config ends with a `[web]` table containing `listen = "127.0.0.1:8080"`
- **THEN** the bot serves its web interface at `http://127.0.0.1:8080/` and asks for no password

#### Scenario: Web interface with a password
- **WHEN** the `[web]` table also contains `password = "s3cret"`
- **THEN** the page at that address shows nothing of the bot until `s3cret` is given

#### Scenario: Web not mentioned
- **WHEN** the config has no `[web]` table
- **THEN** the bot listens on no web address

### Requirement: Config validation before any network activity
The bot SHALL validate the whole config before contacting any server. It SHALL reject a missing or unreadable file, invalid TOML, an unknown key, a missing required key, both or neither of `password` and `password_hash`, a `password_hash` that is not in the hash format, a `user` with more than two words, a `sit` that is neither a UUID nor the word `ground`, an `object` that is not a UUID, both `sit` and `object`, a `login_url` that is not an `http` or `https` URL, a `region` without `position` or a `position` without `region`, a `region` that is empty, a `position` that is not three finite numbers, a `start` that is neither `home` nor `latest`, a `start` together with `region` and `position`, a `persist` that is not in one of its forms (a string other than `cur`, `cfg` and `off`, the boolean `true`, an array that is not exactly two of `"cur"` and `"cfg"`, a table with a key other than `atstart`, `position`, `sit`, `rlv` and `override`, with a `position` or `sit` other than `"cur"` and `"cfg"`, with an `atstart` that is not a boolean, with an `rlv` other than `"drop"` and `"follow"`, with an `override` that is neither `"sit"`, `"tp"` nor an array of these two strings, or with `atstart = false` together with `position` or `sit`, or a value of any other type), a `log_chat` or `log_im` that is not a non-empty string, a `logs` that is not in its form (a value that is not a table, a key other than `chat`, `im` and `transactions`, or a path that is not a non-empty string), `log_chat` together with `logs.chat`, `log_im` together with `logs.im`, a log file that cannot be opened for appending, a transactions file that exists and cannot be read or whose first line is not the header row of `transaction-log`, a `fetch_offline_im` that is not a boolean, a `socket` that is not a boolean, a `requesttp_timeout` that is not a number greater than zero, a `scan_distance` that is not a number from 1 to 1024, a `restore_attachments` that is not a boolean, an `rlv` that is not a boolean, a `perms` that is not in its form (a value that is not a table, a key other than `autoaccept` and `except`, an `autoaccept` or `except` that is not a table, a key in either that is not a kind of request, a value under `autoaccept` that is neither `"all"`, a boolean nor an array of strings, a value under `except` that is not an array of strings, an entry that is neither a UUID nor a name of one or two words, the entry `"all"` inside an array, or one entry both in a kind's `autoaccept` array and in its `except` array), an `xmpp` that is not in its form (a value that is not a table, a key other than `component`, `server`, `secret` and `owners`, a missing `component`, `secret` or `owners`, a `component` that is not a domain name or that holds `@` or `/`, a `server` that is not `host:port` with a port from 1 to 65535 or whose host is neither `localhost` nor a loopback address, a `secret` that is not a non-empty string, an `owners` that is not a non-empty array of strings, or an owner that is not a bare address `user@domain`), a `web` that is not in its form (a value that is not a table, a key other than `listen` and `password`, a missing `listen`, a `listen` that is not `host:port` with a port from 1 to 65535 or whose host is neither `localhost` nor an IP address, or a `password` that is not a non-empty string), a `payments` that is not in its form (a value that is not a table, a key other than `limit`, `perms`, `limit_renew`, `whitelist`, `blacklist` and `unlimited_list`, a missing `limit` or `perms`, a `limit` that is neither a whole number from 0 to 2147483647 nor the string `"unlimited"`, a `perms` that is not an array of the strings `"ctl"`, `"xmpp"` and `"web"`, a `limit_renew` that is not a string in the duration form, is `"0"` or is longer than 365 days, a list that is not an array of strings, an entry that is neither a UUID nor a name of one or two words, or one entry both in `blacklist` and in `whitelist` or `unlimited_list`), a `payments.limit` that is a number without `logs.transactions`, an `autoreply` that is not in its form (a value that is not a table, a key other than `message` and `pause`, a missing `message`, a `message` that is not a string, is blank or is longer than 1023 bytes, or a `pause` that is not a string in the duration form or is longer than 365 days), both `startup` and `startup_file`, a `startup` that is not a string, a `startup_file` that is not a non-empty string, a `startup_file` file that cannot be read or is not valid UTF-8, a startup script that is not in its form (see `startup-script`: a command that ends inside double quotes, that is longer than 8192 bytes, whose name the control socket does not know, or that has a number or form of arguments for which the control socket refuses it without looking at the bot's state), a setting command at the head of the startup script that is refused when it is carried out (see `startup-script`, "Setting commands run before the first login"), an enabled control socket that cannot be set up: its address cannot be determined, is malformed, is in use by another process, or cannot be listened on, and a web interface whose `listen` address is in use by another process or cannot be listened on. On rejection it SHALL print a message naming the offending key and the reason, and exit with status 2 without sending anything. For a startup script the message SHALL name `startup` or `startup_file`, whichever the config has, and the line the offending command starts on. A command of the startup script that is in its form and is no setting command at the head SHALL NOT be judged before it is due: when it is refused then, the bot keeps running, as `startup-script` says.

#### Scenario: Both password forms given
- **WHEN** the config contains both `password` and `password_hash`
- **THEN** the bot prints an error naming both keys and exits with status 2 without contacting the login server

#### Scenario: Malformed sit value
- **WHEN** `sit` is neither a valid UUID nor `ground`
- **THEN** the bot prints an error naming `sit` and exits with status 2

#### Scenario: Malformed object UUID
- **WHEN** `object` is not a valid UUID
- **THEN** the bot prints an error naming `object` and exits with status 2

#### Scenario: Both seat keys
- **WHEN** the config contains both `sit` and `object`
- **THEN** the bot prints an error naming both keys and exits with status 2

#### Scenario: Region without position
- **WHEN** the config contains `region` and no `position`
- **THEN** the bot prints an error naming `region` and `position` and exits with status 2

#### Scenario: Start together with a region
- **WHEN** the config contains `start = "home"` together with `region` and `position`
- **THEN** the bot prints an error naming `start` and `region` and exits with status 2

#### Scenario: Unknown start value
- **WHEN** the config contains `start = "last"`
- **THEN** the bot prints an error naming `start` and the two accepted values, and exits with status 2

#### Scenario: Persist value that only the command takes
- **WHEN** the config contains `persist = "last"`, `persist = "Target/128/128/25"` or `persist = true`
- **THEN** the bot prints an error naming `persist` and its accepted forms, and exits with status 2

#### Scenario: Malformed persist array
- **WHEN** the config contains `persist = ["cur"]`, `persist = ["cur", "cfg", "cur"]` or `persist = ["cur", "off"]`
- **THEN** the bot prints an error naming `persist` and exits with status 2

#### Scenario: Malformed persist table
- **WHEN** the config has a `[persist]` table containing `location = "cur"` or `sit = "ground"`
- **THEN** the bot prints an error naming `persist` and the offending key, and exits with status 2

#### Scenario: Unknown key
- **WHEN** the config contains a key the bot does not define, such as `regoin`
- **THEN** the bot prints an error naming that key and exits with status 2

#### Scenario: Log file cannot be opened
- **WHEN** `im` under `[logs]` names a file in a directory that does not exist
- **THEN** the bot prints an error naming `logs.im` and the reason, and exits with status 2 without contacting the login server

#### Scenario: Old log key that cannot be opened
- **WHEN** `log_im` names a file in a directory that does not exist
- **THEN** the bot prints an error naming `log_im` and the reason, and exits with status 2 without contacting the login server

#### Scenario: Empty log path
- **WHEN** `log_chat` is an empty string, or `chat` under `[logs]` is an empty string or a number
- **THEN** the bot prints an error naming `log_chat` or `logs.chat` and exits with status 2

#### Scenario: Both spellings of one log file
- **WHEN** the config contains `log_chat = "chat.tsv"` and a `[logs]` table with `chat = "chat.tsv"`
- **THEN** the bot prints an error naming `log_chat` and `logs.chat`, and exits with status 2

#### Scenario: Unknown key in the logs table
- **WHEN** the `[logs]` table contains `payments = "pay.tsv"`
- **THEN** the bot prints an error naming `logs.payments` and the keys there are, and exits with status 2

#### Scenario: Transactions file that is something else
- **WHEN** `transactions` under `[logs]` names an existing file whose first line is not the header row
- **THEN** the bot prints an error naming `logs.transactions` and the file, and exits with status 2 without contacting the login server

#### Scenario: Socket key of the wrong type
- **WHEN** the config contains `socket = "yes"`
- **THEN** the bot prints an error naming `socket` and exits with status 2

#### Scenario: Socket cannot be created
- **WHEN** `socket = true` and `RUSSELBOT_SOCK` names a path in a directory that does not exist
- **THEN** the bot prints an error naming `socket`, the address and the reason, and exits with status 2 without contacting the login server

#### Scenario: No home directory
- **WHEN** `socket = true` on Unix and neither `RUSSELBOT_SOCK` nor `HOME` is set
- **THEN** the bot prints an error naming `socket` and `RUSSELBOT_SOCK` and exits with status 2

#### Scenario: Bad teleport request timeout
- **WHEN** the config contains `requesttp_timeout = 0` or `requesttp_timeout = "soon"`
- **THEN** the bot prints an error naming `requesttp_timeout` and exits with status 2

#### Scenario: Scan distance out of range
- **WHEN** the config contains `scan_distance = 5000`
- **THEN** the bot prints an error naming `scan_distance` and exits with status 2

#### Scenario: Restore option of the wrong type
- **WHEN** the config contains `restore_attachments = "no"`
- **THEN** the bot prints an error naming `restore_attachments` and exits with status 2

#### Scenario: Persist table that contradicts itself
- **WHEN** the config has a `[persist]` table containing `atstart = false` and `sit = "cfg"`
- **THEN** the bot prints an error naming `persist`, `atstart` and `sit`, and exits with status 2

#### Scenario: Bad atstart
- **WHEN** the config has a `[persist]` table containing `atstart = "yes"`
- **THEN** the bot prints an error naming `persist` and `atstart`, and exits with status 2

#### Scenario: Bad RLV persist setting
- **WHEN** the config has a `[persist]` table containing `rlv = "keep"` or `rlv = true`
- **THEN** the bot prints an error naming `persist`, `rlv` and the two accepted values, and exits with status 2

#### Scenario: RLV option of the wrong type
- **WHEN** the config contains `rlv = "on"`
- **THEN** the bot prints an error naming `rlv` and exits with status 2

#### Scenario: Unknown kind of request
- **WHEN** the config has `money = "all"` under `[perms.autoaccept]`
- **THEN** the bot prints an error naming `perms.autoaccept.money` and the kinds there are, and exits with status 2

#### Scenario: Malformed entry
- **WHEN** the config has `tp = ["Jane van Doe"]` under `[perms.autoaccept]`
- **THEN** the bot prints an error naming `perms.autoaccept.tp` and the entry, and exits with status 2

#### Scenario: All inside a list
- **WHEN** the config has `anim = ["all"]` under `[perms.autoaccept]`
- **THEN** the bot prints an error naming `perms.autoaccept.anim` that says to write `anim = "all"`, and exits with status 2

#### Scenario: Exceptions that are not a list
- **WHEN** the config has `anim = "all"` under `[perms.except]`
- **THEN** the bot prints an error naming `perms.except.anim` and exits with status 2

#### Scenario: Entry both accepted and excepted
- **WHEN** the config has `anim = ["Jane Doe"]` under `[perms.autoaccept]` and `anim = ["jane.doe"]` under `[perms.except]`
- **THEN** the bot prints an error naming both keys and the entry, and exits with status 2

#### Scenario: Bad persist override
- **WHEN** the config has a `[persist]` table containing `override = "stand"`, `override = true`, `override = ["tp", "home"]` or `override = [1]`
- **THEN** the bot prints an error naming `persist`, `override` and the two accepted strings, and exits with status 2

#### Scenario: XMPP table without owners
- **WHEN** the config has an `[xmpp]` table with `component` and `secret` and no `owners`, or with `owners = []`
- **THEN** the bot prints an error naming `xmpp.owners` and exits with status 2

#### Scenario: Owner with a resource
- **WHEN** the `[xmpp]` table contains `owners = ["me@example.org/phone"]` or `owners = ["example.org"]`
- **THEN** the bot prints an error naming `xmpp.owners` and the entry, and saying that a bare address `user@domain` is wanted, and exits with status 2

#### Scenario: Component that is an account
- **WHEN** the `[xmpp]` table contains `component = "bot@example.org"`
- **THEN** the bot prints an error naming `xmpp.component` and exits with status 2

#### Scenario: Server on another host
- **WHEN** the `[xmpp]` table contains `server = "xmpp.example.org:5347"` or `server = "192.0.2.10:5347"`
- **THEN** the bot prints an error naming `xmpp.server` that says the component connection is not encrypted and must stay on this host, and exits with status 2

#### Scenario: Unknown key in the XMPP table
- **WHEN** the `[xmpp]` table contains `password = "x"`
- **THEN** the bot prints an error naming `xmpp.password` and the keys there are, and exits with status 2

#### Scenario: XMPP server not running at startup
- **WHEN** the config has a valid `[xmpp]` table and nothing listens on its `server` address
- **THEN** the bot starts, logs in to the grid, and keeps trying to connect its component as `xmpp-gateway` describes

#### Scenario: Autoreply without a message
- **WHEN** the config has an `[autoreply]` table containing only `pause = "2h"`
- **THEN** the bot prints an error naming `autoreply.message` and exits with status 2

#### Scenario: Malformed pause
- **WHEN** the `[autoreply]` table contains `pause = "2 hours"`, `pause = "30"`, `pause = "30m2h"`, `pause = "400d"` or `pause = 30`
- **THEN** the bot prints an error naming `autoreply.pause` and the duration form, and exits with status 2

#### Scenario: Blank autoreply text
- **WHEN** the `[autoreply]` table contains `message = "  "`
- **THEN** the bot prints an error naming `autoreply.message` and exits with status 2

#### Scenario: Both startup keys
- **WHEN** the config contains both `startup` and `startup_file`
- **THEN** the bot prints an error naming both keys and exits with status 2

#### Scenario: Startup of the wrong type
- **WHEN** the config contains `startup = ["rlv mode on"]` or `startup_file = ""`
- **THEN** the bot prints an error naming that key and exits with status 2

#### Scenario: Startup file missing
- **WHEN** `startup_file` names a file that does not exist
- **THEN** the bot prints an error naming `startup_file`, the path and the reason, and exits with status 2 without contacting the login server

#### Scenario: Unknown command in the script
- **WHEN** the third line of `startup` is `teleport 128 128 25`
- **THEN** the bot prints an error naming `startup`, line 3 and the unknown command `teleport`, and exits with status 2

#### Scenario: Wrong number of arguments in the script
- **WHEN** a line of `startup` is `sit` or `rlv mode loud`
- **THEN** the bot prints an error naming `startup`, that line and what the command takes, and exits with status 2

#### Scenario: Unclosed quote in the script
- **WHEN** line 2 of the file `startup_file` names is `im "Jane Doe hi` and no later line has a double quote
- **THEN** the bot prints an error naming `startup_file`, line 2 and the quote, and exits with status 2

#### Scenario: Setting command refused
- **WHEN** `startup` starts with `rlv mode on` and has `perms autoaccept money all` as its second line
- **THEN** the bot prints an error naming `startup`, line 2 and the kinds there are, and exits with status 2 without contacting any server

#### Scenario: Command that is judged when it is due
- **WHEN** `startup` is the two lines `say hello` and `autoreply pause soon`
- **THEN** the bot starts and logs in; the refusal of `autoreply pause soon` is in the operator log once the avatar has said `hello`

#### Scenario: Web table without listen
- **WHEN** the config has a `[web]` table containing only `password = "s3cret"`, or an empty `[web]` table
- **THEN** the bot prints an error naming `web.listen` and exits with status 2

#### Scenario: Malformed listen
- **WHEN** the `[web]` table contains `listen = "8080"`, `listen = "127.0.0.1"`, `listen = "127.0.0.1:0"`, `listen = "127.0.0.1:70000"` or `listen = 8080`
- **THEN** the bot prints an error naming `web.listen` and the form `host:port`, and exits with status 2

#### Scenario: Listen on a host name
- **WHEN** the `[web]` table contains `listen = "bot.example.org:8080"`
- **THEN** the bot prints an error naming `web.listen` that says the host is `localhost` or an IP address, and exits with status 2

#### Scenario: Unknown key in the web table
- **WHEN** the `[web]` table contains `user = "me"`
- **THEN** the bot prints an error naming `web.user` and the keys there are, and exits with status 2

#### Scenario: Empty web password
- **WHEN** the `[web]` table contains `password = ""` or `password = 1234`
- **THEN** the bot prints an error naming `web.password` and exits with status 2

#### Scenario: Web address in use
- **WHEN** the `[web]` table contains `listen = "127.0.0.1:8080"` and another process listens on that address
- **THEN** the bot prints an error naming `web.listen`, the address and the reason, and exits with status 2 without contacting the login server

#### Scenario: Payments table without its mandatory keys
- **WHEN** the config has a `[payments]` table without `limit`, without `perms`, or with neither
- **THEN** the bot prints an error naming `payments.limit` or `payments.perms` as required, and exits with status 2

#### Scenario: Malformed limit
- **WHEN** the `[payments]` table contains `limit = -1`, `limit = 10.5`, `limit = "1000"`, `limit = "none"` or `limit = true`
- **THEN** the bot prints an error naming `payments.limit` that says it is a whole number of L$ or `"unlimited"`, and exits with status 2

#### Scenario: Malformed payment perms
- **WHEN** the `[payments]` table contains `perms = "web"`, `perms = ["ctl", "socket"]` or `perms = [1]`
- **THEN** the bot prints an error naming `payments.perms` and the three interfaces, and exits with status 2

#### Scenario: Malformed limit renewal
- **WHEN** the `[payments]` table contains `limit_renew = "7 days"`, `limit_renew = "0"`, `limit_renew = 7` or `limit_renew = "400d"`
- **THEN** the bot prints an error naming `payments.limit_renew` and the duration form, and exits with status 2

#### Scenario: Limit without a transactions file
- **WHEN** the `[payments]` table contains `limit = 1000` and the config has no `transactions` under `[logs]`
- **THEN** the bot prints an error naming `payments.limit` and `logs.transactions`, saying that the limit is kept in that file, and exits with status 2

#### Scenario: Malformed payment list
- **WHEN** the `[payments]` table contains `whitelist = "Jane Doe"`, `blacklist = [42]` or `unlimited_list = ["Jane Mary Doe"]`
- **THEN** the bot prints an error naming that key and exits with status 2

#### Scenario: Entry on the blacklist and on another list
- **WHEN** the `[payments]` table contains `blacklist = ["Jane Doe"]` and `whitelist = ["jane.doe"]`
- **THEN** the bot prints an error naming `payments.blacklist`, `payments.whitelist` and the entry, and exits with status 2

#### Scenario: Unknown key in the payments table
- **WHEN** the `[payments]` table contains `max = 100`
- **THEN** the bot prints an error naming `payments.max` and the keys there are, and exits with status 2

### Requirement: Password hash subcommand
The bot SHALL provide a `hash` subcommand that reads a password from standard input and prints the corresponding hash on standard output as `$1$` followed by the 32 lowercase hexadecimal digits of the MD5 digest of the password. The password SHALL NOT be accepted as a command-line argument. When standard input is a terminal the password SHALL NOT be echoed. A trailing line ending SHALL NOT be part of the hashed password. An empty password SHALL be rejected with a non-zero exit status.

#### Scenario: Hashing a known password
- **WHEN** `hash` is run with `secret` followed by a newline on standard input
- **THEN** it prints `$1$5ebe2294ecd0e0f08eab7690d2a6ee69` and exits with status 0

#### Scenario: Hash is usable as config value
- **WHEN** the printed value is placed in `password_hash` and `password` is absent
- **THEN** the bot logs in to a grid where the plain password would have been accepted

#### Scenario: Empty input
- **WHEN** `hash` receives empty input
- **THEN** it prints an error and exits with a non-zero status without printing a hash

### Requirement: Secrets stay out of output
The bot SHALL NOT write the plain password, the password hash, the XMPP component secret or the password of the web interface to its log or to any error message, and SHALL NOT send any of them to a page of the web interface.

#### Scenario: Login failure logging
- **WHEN** a login attempt is rejected
- **THEN** the logged message contains the user name and the grid's reason but neither the password nor its hash

#### Scenario: Component handshake refused
- **WHEN** the XMPP server refuses the component's handshake because the secret is wrong
- **THEN** the logged message names the component domain and the server's reason but not the secret

#### Scenario: Wrong web password
- **WHEN** someone gives the web interface a wrong password
- **THEN** the logged message names the address the attempt came from, and holds neither what was given nor the password of the config
