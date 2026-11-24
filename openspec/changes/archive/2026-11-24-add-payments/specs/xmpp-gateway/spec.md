# Spec Delta

## MODIFIED Requirements

### Requirement: Control contact
A message with a body that an owner sends to the bare domain SHALL be read as one command line and carried out as the control socket carries out the same command (see `control-socket` and the capabilities its commands belong to), with the same answers and the same effects, but as a command of the interface `xmpp` (see `payments`, "Interfaces that may pay"): `payments.perms` says for `xmpp` on its own whether `pay`, `prices` and `payees` are carried out and whether `status` has its money lines. The body SHALL be split into the command name and its arguments at spaces, tabs and line feeds; a part between double quotes SHALL be one argument and keep its spaces, and inside it `\"` SHALL stand for a double quote and `\\` for a backslash. The arguments SHALL then be passed on as `rslbot ctl` passes on its own, so that the words of a `say` message or of an `im` message are joined with single spaces. The bot SHALL answer every command line with one message from the bare domain: the answer's text when it has one, `ok` when it has none, and `error: ` followed by the text for an error. A body that ends inside double quotes, or is longer than 8192 bytes, SHALL be answered with an error and carry out nothing. The body `help` SHALL be answered with the list of commands. Commands from several messages SHALL be carried out in the order they arrive, and an answer that takes time, such as a teleport's, SHALL NOT hold back the messages of the other contacts; at most 16 commands SHALL wait for their answers at once, and a further one SHALL be answered with an error saying the bot is busy.

#### Scenario: Status
- **WHEN** an owner sends `status` to `russel.sl.example.org`
- **THEN** the owner receives the text `rslbot ctl status` prints, with the `balance` and `limit` lines when `payments.perms` has `xmpp` and without them when it does not

#### Scenario: Name with a space
- **WHEN** an owner sends `im "Jane Doe" see you later`
- **THEN** Jane Doe receives the IM `see you later` and the owner receives the answer naming Jane Doe

#### Scenario: Command refused
- **WHEN** an owner sends `sit ground` while the bot is between logins
- **THEN** the owner receives `error: ` followed by the reason the control socket would give

#### Scenario: Unclosed quote
- **WHEN** an owner sends `im "Jane Doe hi`
- **THEN** the owner receives an error about the quote and nothing is sent to the grid

#### Scenario: Teleport while chatting
- **WHEN** an owner sends `tp 128 128 25 Home` and, while the teleport goes on, sends `hi` to `jane.doe@<domain>`
- **THEN** the IM is carried out or refused at once as the `im` command would be during a teleport, and the teleport's answer arrives when it ends

#### Scenario: Quit
- **WHEN** an owner sends `quit`
- **THEN** the owner receives `ok`, the bot logs out, and its addresses go offline

#### Scenario: Paying over XMPP
- **WHEN** `payments.perms` is `["xmpp"]` and an owner sends `pay 10 avatar "Jane Doe"`
- **THEN** Jane Doe is paid L$10 and the owner receives the answer, while `rslbot ctl pay 10 avatar "Jane Doe"` is refused
