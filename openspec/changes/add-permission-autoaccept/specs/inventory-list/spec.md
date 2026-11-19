# Spec Delta

## MODIFIED Requirements

### Requirement: Inventory is read on demand
The bot SHALL ask the grid about the inventory only to carry out a command that needs it, to find the folder an inventory offer it accepts goes into (see `permission-requests`, "Accepting a request"), for which it reads the top level of the inventory and nothing below it, and, while `restore_attachments` is on, to restore attachments after a login (see `avatar-outfit`, "Restoring attachments after a login"). It SHALL NOT keep what it learned for a later command or a later offer: every command and every accepted offer sees the inventory as the grid has it then. The `inventory` command and every lookup of an item or folder SHALL be answered with an error, and SHALL ask the grid nothing, when the avatar is not connected to a region or is in the middle of a teleport or a logout, and when the simulator the avatar is on offers no inventory service. A command whose lookup is under way when the session is lost or the avatar changes simulator SHALL be answered with an error. Reading the inventory SHALL NOT delay the bot's simulator traffic and SHALL NOT change the bot's placement mode.

#### Scenario: No command, no inventory traffic
- **WHEN** the bot runs for an hour with `restore_attachments = false`, is given none of the commands that name an inventory item or folder, and accepts no inventory offer
- **THEN** it has asked the grid for no inventory folder and no inventory item

#### Scenario: No command and no outfit service
- **WHEN** the bot runs for an hour with `restore_attachments` on, on a grid whose simulator offers no outfit service, is given none of those commands, and accepts no inventory offer
- **THEN** it has asked the grid for no inventory folder and no inventory item

#### Scenario: Change made in a viewer
- **WHEN** an item is renamed with a viewer between two `rslbot ctl inventory` commands
- **THEN** the second listing shows the new name

#### Scenario: Between logins
- **WHEN** `rslbot ctl inventory` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: Session lost during a listing
- **WHEN** the session is lost while a listing is being collected
- **THEN** the command exits with status 1 saying the session was lost, and the bot logs in again as usual

#### Scenario: Listing while seated
- **WHEN** `rslbot ctl inventory` is run in holding mode while the avatar is seated
- **THEN** the avatar stays seated, stays connected, and the bot stays in holding mode

#### Scenario: Inventory offer that waits
- **WHEN** an avatar gives the bot's avatar an item and the offer is not accepted
- **THEN** the bot has asked the grid for no inventory folder

#### Scenario: Inventory offer accepted
- **WHEN** an inventory offer is accepted, by itself or with `rslbot ctl accept inventory`
- **THEN** the bot has asked the grid for the folders at the top of My Inventory and for nothing else of the inventory
