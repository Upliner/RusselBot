# Spec Delta

## MODIFIED Requirements

### Requirement: Script commands are the operator's
A script command SHALL be carried out as the control socket carries out the same command, with the same effects, and SHALL count as a command of the operator and of the interface `ctl` (see `payments`, "Interfaces that may pay"), whether or not the control socket is enabled: RLV restrictions apply to it and `rlv mode override` exempts it as they do the operator's, and it changes the placement mode as the operator's does. The script SHALL need no other way of sending commands to be enabled: it SHALL be carried out with and without the control socket.

#### Scenario: No socket
- **WHEN** the config has a script and no `socket = true`
- **THEN** the script is carried out all the same

#### Scenario: Command that moves the avatar
- **WHEN** the script's last command is `tp 128 128 25 Home` and it is carried out
- **THEN** the avatar is in `Home`, the bot is in manual mode, and it does not teleport the avatar back by itself

#### Scenario: Restricted command
- **WHEN** an attachment has said `@sendchat=n` to its owner before the script's command `say hello` is due, and RLV is on
- **THEN** the command is refused as `rslbot ctl say hello` would be

#### Scenario: Paying from the script
- **WHEN** `payments.perms` has `ctl`, the config has no `socket = true`, and the script's command `pay 10 avatar "Jane Doe"` is due
- **THEN** Jane Doe is paid L$10, within the limit and the lists as a payment from `rslbot ctl` is
