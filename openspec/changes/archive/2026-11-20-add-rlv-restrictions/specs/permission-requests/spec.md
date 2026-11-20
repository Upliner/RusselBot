# Spec Delta

## ADDED Requirements

### Requirement: RLV restrictions come before the rules
While RLV restrictions are in force (see `rlv-restrictions`, "When restrictions are in force"), they SHALL be looked at before the auto-accept rules, before the wait for the operator and before the `accept` command, whatever "Deciding a request", "Requests that wait" and "Accept command" say:

- A teleport offer that `tplure` forbids SHALL be dropped when it arrives: it SHALL NOT be accepted, also not as the answer to a `requesttp`, and SHALL NOT wait for the operator. An offer that waits already SHALL NOT be accepted with `accept tp` while `tplure` forbids it, or while the avatar sits and `unsit` is in force, and SHALL keep waiting. An offer that the rules, a `requesttp` or `accepttp` would have the bot accept by itself while the avatar sits and `unsit` is in force SHALL wait for the operator instead.
- A script permission request with the kind `attach` or `controls` SHALL be dropped while `denypermission` is in force: it SHALL NOT be accepted, by itself or with `accept`, and SHALL NOT wait.
- A teleport offer for which `accepttp` is held SHALL be accepted by the bot itself, and the kinds `attach` and `controls` SHALL count as auto-accepted while `acceptpermission` is in force, whatever the rules and exceptions of `tp`, `attach` and `controls` are (see `rlv-restrictions`, "Acceptances that RLV switches on").

The bot SHALL send no refusal for a request it drops. The rules and exceptions themselves SHALL NOT be changed by any restriction, and `perms status` SHALL show them as they are. Inventory offers SHALL be decided and accepted as they are without RLV. Restrictions SHALL be looked at when a request arrives and when it is to be accepted: a request that waits is not dropped or accepted because a restriction is set or lifted later. While RLV is off none of this SHALL apply.

#### Scenario: Auto-accepted avatar under tplure
- **WHEN** the rule for `tp` is `all`, a collar holds `tplure`, and Jane Doe offers the bot a teleport
- **THEN** the avatar stays where it is, the offer does not wait, and the operator log says it was dropped because of `tplure`

#### Scenario: Accept on command
- **WHEN** Jane Doe's offer waits, a collar then holds `tplure`, and `rslbot ctl accept tp "Jane Doe"` is run
- **THEN** the command exits with status 1 naming `tplure`, and the offer still waits

#### Scenario: Seated and held
- **WHEN** the rule for `tp` is `all`, a chair holds `unsit` while the avatar sits on it, and Jane Doe offers the bot a teleport
- **THEN** the avatar stays seated and the offer waits for the operator

#### Scenario: Accepted for RLV
- **WHEN** the rule for `controls` is an empty list, a collar holds `acceptpermission`, and the collar's script asks for permission to take controls
- **THEN** the bot grants it, and `rslbot ctl perms status` shows the rule for `controls` as an empty list

#### Scenario: Denied for RLV
- **WHEN** the rule for `attach` is `all`, a collar holds `denypermission`, and a script asks for permission to attach its object
- **THEN** the bot sends no answer and `rslbot ctl accept attach` finds no request

#### Scenario: Inventory offers are untouched
- **WHEN** a collar holds `tplure` and `denypermission`, the rule for `inventory` is `all`, and Jane Doe offers the bot a notecard
- **THEN** the bot accepts the notecard

#### Scenario: Set while a request waits
- **WHEN** a script's request for controls waits for the operator and a collar then says `@acceptpermission=add` to its owner
- **THEN** the request still waits, and the same script's next request is granted
