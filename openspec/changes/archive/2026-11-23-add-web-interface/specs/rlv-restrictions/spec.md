# Spec Delta

## MODIFIED Requirements

### Requirement: Override exempts one command
While RLV is in override (see `rlv-commands`, "RLV mode"), the next control-socket command the bot receives other than `rlv`, `status`, `perms` and `quit` SHALL be the exempt command. For the exempt command no restriction SHALL be in force: it SHALL be carried out exactly as it would be with no restriction kept. RLV SHALL be `on` again at the moment the exempt command is taken up, before anything is known of its outcome and whatever its outcome is, also when it is refused for a reason that has nothing to do with RLV. A line the bot cannot read as a command SHALL NOT be the exempt command. A command the page of the web interface sent by itself (see `web-server`, "Requests the page makes by itself") SHALL NOT be the exempt command either, whatever command it is: it SHALL be carried out as while RLV is on and SHALL leave RLV in override.

The exemption SHALL cover everything that command does until it is answered, and in addition:

- for `requesttp`, the acceptance of the one offer that answers the request;
- for `persist` and `home`, the bot's steps towards the placement the command made the held one (see `presence-reconciler`), until the avatar is in that placement for the first time, or until the placement mode or the held placement is changed by anything else.

Override SHALL change nothing else. While RLV is in override, the commands objects send SHALL be read, kept and answered as while it is on; force commands SHALL be refused as "Force commands under restrictions" says; the acceptances and the drops of offers and requests SHALL be made as while it is on; the bot's own steps that do not belong to an exempt command SHALL be held back as while it is on; and `rlv cmd` SHALL be carried out as while it is on, SHALL NOT be the exempt command, and SHALL leave RLV in override.

#### Scenario: One command through
- **WHEN** a collar holds `unsit`, the avatar sits on a chair, and `rslbot ctl rlv mode override` and then `rslbot ctl stand` are run
- **THEN** the avatar stands up, and `rlv status` then prints `rlv: on`

#### Scenario: Only one
- **WHEN** the avatar is then seated on the chair again and `rslbot ctl stand` is run once more
- **THEN** the command is refused naming `unsit`

#### Scenario: Status does not use it up
- **WHEN** `rslbot ctl rlv mode override`, `rslbot ctl status` and `rslbot ctl rlv status` are run
- **THEN** the last prints `rlv: override`

#### Scenario: Used up by a command that fails
- **WHEN** `rslbot ctl rlv mode override` is run, and then `rslbot ctl sit <uuid>` naming no object in the region
- **THEN** the sit fails with the simulator's reason, and RLV is `on`

#### Scenario: Unrestricted command uses it up
- **WHEN** `rslbot ctl rlv mode override` and `rslbot ctl sendtp "Jane Doe"` are run
- **THEN** RLV is `on` after the second

#### Scenario: Force commands are not exempt
- **WHEN** a collar holds `unsit`, the avatar sits, `rslbot ctl rlv mode override` is run, and a bench the avatar owns says `@sit:<bench>=force` to its owner
- **THEN** the forced sit is refused because of `unsit`, and RLV is still in override

#### Scenario: rlv cmd is not exempt
- **WHEN** a collar holds `tploc`, and `rslbot ctl rlv mode override` and `rslbot ctl rlv cmd @tpto:Home/128/100/25=force` are run
- **THEN** the second is refused naming `tploc`, and `rlv status` prints `rlv: override`

#### Scenario: Offers are not exempt
- **WHEN** a collar holds `tplure`, `rslbot ctl rlv mode override` is run, and Jane Doe offers the bot a teleport
- **THEN** the offer is dropped, and RLV is still in override

#### Scenario: Requesting a teleport under override
- **WHEN** a collar holds `tplure`, and `rslbot ctl rlv mode override` and `rslbot ctl requesttp "Jane Doe"` are run, and Jane Doe then offers a teleport
- **THEN** the bot accepts that offer and goes; a second offer of hers is dropped

#### Scenario: Home under override
- **WHEN** the persist override setting is empty, the bot holds a chair in `Target`, the avatar is in `Home`, a collar holds `tploc` and `sittp`, and `rslbot ctl rlv mode override` and `rslbot ctl home` are run
- **THEN** the bot teleports the avatar to `Target` and seats it on the chair; when the avatar is unseated afterwards, the bot's next sit is held to `sittp`

#### Scenario: The page's own refresh does not use it up
- **WHEN** a collar holds `shownearby`, a page of the web interface shows its Local chat tab, `rslbot ctl rlv mode override` is run, and the page then refreshes its list of avatars by itself
- **THEN** that refresh is refused naming `shownearby`, and `rlv status` then still prints `rlv: override`
