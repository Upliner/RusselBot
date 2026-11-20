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
| `persist` | no | What the bot holds once it has reached the config placement after startup (see `placement-persist`), and in its table form also what a forced RLV movement does to the held placement and which of the bot's own steps RLV restrictions do not hold back (see `rlv-commands`); the forms are given below. Defaults to `"off"` |
| `log_chat` | no | Path of the TSV file that receives local chat; no chat file is written when absent |
| `log_im` | no | Path of the TSV file that receives instant messages; no IM file is written when absent |
| `fetch_offline_im` | no | Boolean; when true the bot fetches instant messages stored while the avatar was offline. Defaults to false |
| `socket` | no | Boolean; when true the bot listens on the control socket. Defaults to false |
| `requesttp_timeout` | no | Number of minutes, greater than zero, during which the bot accepts a teleport offer from an avatar it has asked with `requesttp`. Defaults to 10 |
| `scan_distance` | no | Number of metres, from 1 to 1024. When present the bot learns the objects within that distance from the moment it enters a region, for the `touch` command. When absent it learns them only when `touch` is first used, within 32 metres |
| `restore_attachments` | no | Boolean; when true the bot attaches again, after each login, the objects the avatar's Current Outfit folder lists and the simulator did not bring (see `avatar-outfit`, "Restoring attachments after a login"). Defaults to true |
| `rlv` | no | Boolean; when true the bot starts with RLV on and reads the RLV commands of the objects the avatar owns (see `rlv-commands`). Defaults to false |
| `perms` | no | Table saying which requests the bot accepts by itself (see `permission-requests`); its form is given below. Without it the bot accepts none by itself |

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

A relative `log_chat` or `log_im` path SHALL be taken relative to the directory the bot was started in. A config that uses `object` SHALL behave exactly as the same config with that value under `sit`, and the bot SHALL write a line to standard error at startup that names `object` as deprecated and `sit` as its replacement.

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
- **WHEN** the config contains none of `log_chat`, `log_im` and `fetch_offline_im`
- **THEN** the bot starts, writes no log file and does not fetch offline instant messages

#### Scenario: Chat log configured
- **WHEN** the config contains `log_chat = "chat.tsv"`
- **THEN** the bot creates `chat.tsv` in the directory it was started in if it does not exist, and appends chat rows to it

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

### Requirement: Config validation before any network activity
The bot SHALL validate the whole config before contacting any server. It SHALL reject a missing or unreadable file, invalid TOML, an unknown key, a missing required key, both or neither of `password` and `password_hash`, a `password_hash` that is not in the hash format, a `user` with more than two words, a `sit` that is neither a UUID nor the word `ground`, an `object` that is not a UUID, both `sit` and `object`, a `login_url` that is not an `http` or `https` URL, a `region` without `position` or a `position` without `region`, a `region` that is empty, a `position` that is not three finite numbers, a `start` that is neither `home` nor `latest`, a `start` together with `region` and `position`, a `persist` that is not in one of its forms (a string other than `cur`, `cfg` and `off`, the boolean `true`, an array that is not exactly two of `"cur"` and `"cfg"`, a table with a key other than `atstart`, `position`, `sit`, `rlv` and `override`, with a `position` or `sit` other than `"cur"` and `"cfg"`, with an `atstart` that is not a boolean, with an `rlv` other than `"drop"` and `"follow"`, with an `override` that is neither `"sit"`, `"tp"` nor an array of these two strings, or with `atstart = false` together with `position` or `sit`, or a value of any other type), a `log_chat` or `log_im` that is not a non-empty string, a `log_chat` or `log_im` file that cannot be opened for appending, a `fetch_offline_im` that is not a boolean, a `socket` that is not a boolean, a `requesttp_timeout` that is not a number greater than zero, a `scan_distance` that is not a number from 1 to 1024, a `restore_attachments` that is not a boolean, an `rlv` that is not a boolean, a `perms` that is not in its form (a value that is not a table, a key other than `autoaccept` and `except`, an `autoaccept` or `except` that is not a table, a key in either that is not a kind of request, a value under `autoaccept` that is neither `"all"`, a boolean nor an array of strings, a value under `except` that is not an array of strings, an entry that is neither a UUID nor a name of one or two words, the entry `"all"` inside an array, or one entry both in a kind's `autoaccept` array and in its `except` array), and an enabled control socket that cannot be set up: its address cannot be determined, is malformed, is in use by another process, or cannot be listened on. On rejection it SHALL print a message naming the offending key and the reason, and exit with status 2 without sending anything.

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
- **WHEN** `log_im` names a file in a directory that does not exist
- **THEN** the bot prints an error naming `log_im` and the reason, and exits with status 2 without contacting the login server

#### Scenario: Empty log path
- **WHEN** `log_chat` is an empty string
- **THEN** the bot prints an error naming `log_chat` and exits with status 2

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
The bot SHALL NOT write the plain password or the password hash to its log or to any error message.

#### Scenario: Login failure logging
- **WHEN** a login attempt is rejected
- **THEN** the logged message contains the user name and the grid's reason but neither the password nor its hash
