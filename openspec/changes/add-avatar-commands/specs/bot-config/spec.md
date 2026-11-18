# Spec Delta

## MODIFIED Requirements

### Requirement: Single TOML config file
The bot SHALL read all of its settings from one TOML file. The file path SHALL be taken from the first command-line argument and SHALL default to `russelbot.toml` in the current directory when no argument is given. The file SHALL support these keys:

| Key | Required | Meaning |
|---|---|---|
| `login_url` | no | Grid login URL; defaults to the Second Life Agni login URL `https://login.agni.lindenlab.com/cgi-bin/login.cgi` |
| `user` | yes | Avatar name, either `First Last` or a single word (the last name is then `Resident`) |
| `password` | one of the two | Plain password |
| `password_hash` | one of the two | Password hash as produced by the `hash` subcommand |
| `region` | yes | Target region name |
| `position` | yes | Target coordinates inside the region, as an array of three numbers `[x, y, z]` |
| `object` | yes | UUID of the object to sit on |
| `log_chat` | no | Path of the TSV file that receives local chat; no chat file is written when absent |
| `log_im` | no | Path of the TSV file that receives instant messages; no IM file is written when absent |
| `fetch_offline_im` | no | Boolean; when true the bot fetches instant messages stored while the avatar was offline. Defaults to false |
| `socket` | no | Boolean; when true the bot listens on the control socket. Defaults to false |
| `requesttp_timeout` | no | Number of minutes, greater than zero, during which the bot accepts a teleport offer from an avatar it has asked with `requesttp`. Defaults to 10 |
| `scan_distance` | no | Number of metres, from 1 to 1024. When present the bot learns the objects within that distance from the moment it enters a region, for the `touch` command. When absent it learns them only when `touch` is first used, within 32 metres |

A relative `log_chat` or `log_im` path SHALL be taken relative to the directory the bot was started in.

#### Scenario: Minimal config for Second Life
- **WHEN** the config contains `user`, `password_hash`, `region`, `position` and `object` and no `login_url`
- **THEN** the bot starts and uses the Second Life Agni login URL

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

### Requirement: Config validation before any network activity
The bot SHALL validate the whole config before contacting any server. It SHALL reject a missing or unreadable file, invalid TOML, an unknown key, a missing required key, both or neither of `password` and `password_hash`, a `password_hash` that is not in the hash format, a `user` with more than two words, an `object` that is not a UUID, a `login_url` that is not an `http` or `https` URL, a `position` that is not three finite numbers, a `log_chat` or `log_im` that is not a non-empty string, a `log_chat` or `log_im` file that cannot be opened for appending, a `fetch_offline_im` that is not a boolean, a `socket` that is not a boolean, a `requesttp_timeout` that is not a number greater than zero, a `scan_distance` that is not a number from 1 to 1024, and an enabled control socket that cannot be set up: its address cannot be determined, is malformed, is in use by another process, or cannot be listened on. On rejection it SHALL print a message naming the offending key and the reason, and exit with status 2 without sending anything.

#### Scenario: Both password forms given
- **WHEN** the config contains both `password` and `password_hash`
- **THEN** the bot prints an error naming both keys and exits with status 2 without contacting the login server

#### Scenario: Malformed object UUID
- **WHEN** `object` is not a valid UUID
- **THEN** the bot prints an error naming `object` and exits with status 2

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
