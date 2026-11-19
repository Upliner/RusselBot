# Spec Delta

## MODIFIED Requirements

### Requirement: Control client subcommand
The bot SHALL provide a `ctl` subcommand that takes a command name and its arguments, sends them to the control socket and waits for the answer. Arguments after `ctl` SHALL be passed on as given, including arguments that start with `-` or `/`. On a success answer it SHALL print the answer's text, if any, on standard output and exit with status 0. On an error answer it SHALL print the text on standard error and exit with status 1. When it cannot connect or gets no answer within 90 seconds it SHALL print the address and the reason on standard error and exit with status 1. When no command name is given it SHALL print usage and exit with status 2. The `ctl` subcommand SHALL NOT read the config file.

#### Scenario: Command accepted
- **WHEN** `rslbot ctl say hello` is run while the bot is connected to a region
- **THEN** it exits with status 0

#### Scenario: Command refused
- **WHEN** `rslbot ctl say hello` is run while the bot is between logins
- **THEN** it prints the bot's error on standard error and exits with status 1

#### Scenario: No bot running
- **WHEN** `rslbot ctl say hello` is run and nothing listens on the socket address
- **THEN** it prints an error naming the address and exits with status 1

#### Scenario: Message that looks like an option
- **WHEN** `rslbot ctl say -v is not an option here` is run
- **THEN** the avatar says `-v is not an option here`

#### Scenario: No command
- **WHEN** `rslbot ctl` is run without further arguments
- **THEN** it prints usage and exits with status 2

#### Scenario: Command that takes a while
- **WHEN** `rslbot ctl tp 128 128 25 Home` is run and the teleport takes 40 seconds
- **THEN** the command waits and exits with status 0 when the avatar has arrived
