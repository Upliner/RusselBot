# control-socket Specification

## Purpose

Lets the operator send commands to a running bot from the same machine through a local socket and the `ctl` subcommand, without stopping the bot or logging in with a viewer.

## Requirements

### Requirement: Control socket address
The control socket address SHALL be the value of the environment variable `RUSSELBOT_SOCK` when it is set and not empty, and the platform default otherwise. On Unix the address is a file path of a Unix domain socket and the default is `russelbot-ctl` in the home directory of the user running the bot. On Windows the address is `host:port` of a TCP socket and the default is `127.0.0.1:47115`. The bot and the `ctl` subcommand SHALL determine the address the same way.

#### Scenario: Default address on Unix
- **WHEN** the bot runs on Unix with `socket = true`, `HOME=/home/bot` and `RUSSELBOT_SOCK` unset
- **THEN** it listens on the Unix domain socket `/home/bot/russelbot-ctl`

#### Scenario: Address from the environment
- **WHEN** `RUSSELBOT_SOCK=/run/bot/ctl` is set for both the bot and `rslbot ctl`
- **THEN** the bot listens on `/run/bot/ctl` and `rslbot ctl` connects to `/run/bot/ctl`

#### Scenario: Default address on Windows
- **WHEN** the bot runs on Windows with `socket = true` and `RUSSELBOT_SOCK` unset
- **THEN** it listens on TCP `127.0.0.1:47115`

### Requirement: Control socket lifetime
When the config enables the control socket, the bot SHALL start listening before its first login attempt and SHALL keep listening until it exits, whether or not it is connected to a grid. When the config does not enable it, the bot SHALL NOT create or listen on any control socket. On Unix the socket file SHALL be accessible only to the user running the bot, and the bot SHALL remove it when it exits. A socket file left behind by a bot that is no longer running SHALL be replaced. The bot SHALL NOT take over an address another process is listening on, and SHALL NOT remove a file at the address that is not a socket.

#### Scenario: Socket disabled
- **WHEN** the config has no `socket` key
- **THEN** no socket file is created and `rslbot ctl say hello` fails because nothing is listening

#### Scenario: Socket file permissions
- **WHEN** the bot listens on a Unix domain socket
- **THEN** other users of the machine cannot connect to it

#### Scenario: Stale socket file
- **WHEN** the bot starts and the socket file exists but no process is listening on it
- **THEN** the bot replaces the file and listens

#### Scenario: Second bot on the same address
- **WHEN** the bot starts while another bot is listening on the same address
- **THEN** it exits with status 2 and the first bot keeps answering commands

#### Scenario: Socket removed on exit
- **WHEN** the bot exits after an operator signal
- **THEN** the socket file no longer exists

### Requirement: One command per connection
A client SHALL send one command per connection as a single line, and the bot SHALL answer with a single line and close the connection. The answer SHALL be either success, optionally with a text, or an error with a text that says why. A command line SHALL consist of the command name and its arguments separated by tab characters, with backslash, tab, line feed and carriage return inside an argument written as `\\`, `\t`, `\n` and `\r`. The bot SHALL answer with an error and do nothing else when the command name is unknown, the number of arguments is wrong, the line is not valid UTF-8, or the line is longer than 8192 bytes. The bot SHALL close a connection that has not sent a complete line within 5 seconds.

#### Scenario: Unknown command
- **WHEN** a client sends the line `dance`
- **THEN** the bot answers with an error naming `dance` and sends nothing to the grid

#### Scenario: Wrong number of arguments
- **WHEN** a client sends `im` with one argument
- **THEN** the bot answers with an error and sends nothing to the grid

#### Scenario: Silent client
- **WHEN** a client connects and sends nothing
- **THEN** the bot closes the connection after 5 seconds and keeps serving other clients

### Requirement: The control socket never disturbs the bot
Handling control connections SHALL NOT delay the bot's simulator traffic, and a misbehaving client SHALL NOT make the bot exit, lose its session or stop accepting other connections. A failure on a control connection SHALL be reported in the operator log at most.

#### Scenario: Client disconnects before the answer
- **WHEN** a client sends `say hello` and closes the connection before reading the answer
- **THEN** the message is still said and the bot keeps running

#### Scenario: Many idle connections
- **WHEN** several clients hold connections open without sending anything
- **THEN** the avatar stays connected and a further client's command is still answered

### Requirement: Control client subcommand
The bot SHALL provide a `ctl` subcommand that takes a command name and its arguments, sends them to the control socket and waits for the answer. Arguments after `ctl` SHALL be passed on as given, including arguments that start with `-` or `/`. On a success answer it SHALL print the answer's text, if any, on standard output and exit with status 0. On an error answer it SHALL print the text on standard error and exit with status 1. When it cannot connect or gets no answer within 30 seconds it SHALL print the address and the reason on standard error and exit with status 1. When no command name is given it SHALL print usage and exit with status 2. The `ctl` subcommand SHALL NOT read the config file.

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
