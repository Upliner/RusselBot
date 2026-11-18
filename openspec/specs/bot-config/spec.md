# bot-config Specification

## Purpose

Defines the single configuration file the bot runs from, how it is validated, and the helper command that produces a password hash so a plain password need not be stored.

## Requirements

### Requirement: Single TOML config file
The bot SHALL read all of its settings from one TOML file. The file path SHALL be taken from the first command-line argument and SHALL default to `slbot.toml` in the current directory when no argument is given. The file SHALL support these keys:

| Key | Required | Meaning |
|---|---|---|
| `login_url` | no | Grid login URL; defaults to the Second Life Agni login URL `https://login.agni.lindenlab.com/cgi-bin/login.cgi` |
| `user` | yes | Avatar name, either `First Last` or a single word (the last name is then `Resident`) |
| `password` | one of the two | Plain password |
| `password_hash` | one of the two | Password hash as produced by the `hash` subcommand |
| `region` | yes | Target region name |
| `position` | yes | Target coordinates inside the region, as an array of three numbers `[x, y, z]` |
| `object` | yes | UUID of the object to sit on |

#### Scenario: Minimal config for Second Life
- **WHEN** the config contains `user`, `password_hash`, `region`, `position` and `object` and no `login_url`
- **THEN** the bot starts and uses the Second Life Agni login URL

#### Scenario: Config path from the command line
- **WHEN** the bot is started with a file path as its first argument
- **THEN** it reads that file and not `slbot.toml`

#### Scenario: Single-word user name
- **WHEN** `user` is a single word such as `sitbot`
- **THEN** the bot logs in with first name `sitbot` and last name `Resident`

### Requirement: Config validation before any network activity
The bot SHALL validate the whole config before contacting any server. It SHALL reject a missing or unreadable file, invalid TOML, an unknown key, a missing required key, both or neither of `password` and `password_hash`, a `password_hash` that is not in the hash format, a `user` with more than two words, an `object` that is not a UUID, a `login_url` that is not an `http` or `https` URL, and a `position` that is not three finite numbers. On rejection it SHALL print a message naming the offending key and the reason, and exit with status 2 without sending anything.

#### Scenario: Both password forms given
- **WHEN** the config contains both `password` and `password_hash`
- **THEN** the bot prints an error naming both keys and exits with status 2 without contacting the login server

#### Scenario: Malformed object UUID
- **WHEN** `object` is not a valid UUID
- **THEN** the bot prints an error naming `object` and exits with status 2

#### Scenario: Unknown key
- **WHEN** the config contains a key the bot does not define, such as `regoin`
- **THEN** the bot prints an error naming that key and exits with status 2

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
