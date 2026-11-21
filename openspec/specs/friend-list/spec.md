# friend-list Specification

## Purpose

Keeps the bot informed of who the avatar's friends are and which of them are online, as a viewer's friend list is, and lets the operator read that list through the `friends` command.

## Requirements

### Requirement: Friend list from the grid
The bot SHALL know the avatar's friends as the grid lists them in its answer to the login, each with whether that friend lets the avatar see when it is online. It SHALL take the list anew from every login, and SHALL NOT keep it or who was online from one login to the next.

#### Scenario: Friends at login
- **WHEN** the grid's answer to the login lists Jane Doe and John Roe as friends of the avatar
- **THEN** `rslbot ctl friends` has one line for each of the two and no other

#### Scenario: Friend made between two logins
- **WHEN** the session is lost, the avatar becomes friends with Ann Poe from a viewer, and the bot logs in again
- **THEN** `rslbot ctl friends` has a line for Ann Poe

#### Scenario: Grid that lists no friends
- **WHEN** the grid's answer to the login has no list of friends, or an empty one
- **THEN** the login succeeds as before and `rslbot ctl friends` prints nothing

### Requirement: Who is online
A friend SHALL count as online from a notice of the simulator that it is online until a notice that it is offline, the end of the friendship or the loss of the session, and as offline otherwise. A notice about an avatar that is not a friend SHALL change nothing. Entering another region SHALL NOT change who counts as online.

#### Scenario: Friend comes online
- **WHEN** Jane Doe, a friend, logs in to the grid while the bot is connected
- **THEN** her line in `rslbot ctl friends` has the status `online`

#### Scenario: Friend goes offline
- **WHEN** Jane Doe then logs out
- **THEN** her line has the status `offline`

#### Scenario: Friends online at login
- **WHEN** the bot logs in while Jane Doe is online and the simulator has sent its notice of that
- **THEN** her line has the status `online`

#### Scenario: Teleport
- **WHEN** Jane Doe counts as online and the bot's avatar teleports to another region
- **THEN** her line still has the status `online`

#### Scenario: After a new login
- **WHEN** the session is lost while Jane Doe counts as online, and the bot has logged in again without a notice about her
- **THEN** her line has the status `offline`

#### Scenario: Notice about a stranger
- **WHEN** the simulator sends a notice that an avatar who is not a friend is online
- **THEN** `rslbot ctl friends` has no line for that avatar

### Requirement: Friendships that end or begin while the bot runs
When the simulator tells the bot that a friend has ended the friendship, that avatar SHALL no longer be a friend. When an avatar's acceptance of a friendship offer the avatar made arrives, that avatar SHALL be a friend that lets the avatar see when it is online. The bot SHALL go on leaving friendship offers made to the avatar unanswered (see `permission-requests`).

#### Scenario: Friendship ended by the friend
- **WHEN** Jane Doe removes the avatar from her friends while the bot is connected
- **THEN** `rslbot ctl friends` has no line for her

#### Scenario: Offer accepted while the bot runs
- **WHEN** the avatar offered Ann Poe friendship from a viewer, and Ann Poe accepts while the bot is connected
- **THEN** `rslbot ctl friends` has a line for Ann Poe, with the status `online` once the simulator's notice of that has arrived

#### Scenario: Offer made to the avatar
- **WHEN** Ann Poe offers the avatar friendship while the bot is connected
- **THEN** the bot sends no answer, and `rslbot ctl friends` has no line for her

### Requirement: Friends that hide their status
The status of a friend that does not let the avatar see when it is online SHALL be unknown, whatever notices arrive. When the simulator tells the bot that a friend has changed what it lets the avatar do, whether that friend lets the avatar see when it is online SHALL be as the simulator says from then on.

#### Scenario: Friend that hides its status
- **WHEN** the grid's answer to the login lists John Roe as a friend that does not let the avatar see when he is online
- **THEN** his line in `rslbot ctl friends` has the status `?`

#### Scenario: Status hidden while the bot runs
- **WHEN** Jane Doe counts as online and takes away the avatar's right to see when she is online
- **THEN** her line has the status `?`

#### Scenario: Status shown again
- **WHEN** John Roe gives the avatar the right to see when he is online, and the simulator then sends a notice that he is online
- **THEN** his line has the status `online`

### Requirement: Friends command
`friends` SHALL be answered with success and a text of one line for each friend of the avatar, or with success and no text when it has none. It SHALL take no arguments. It SHALL be answered with an error when the avatar is not connected to a region or is in the middle of a teleport or a logout.

#### Scenario: Listing
- **WHEN** the avatar has three friends and `rslbot ctl friends` is run
- **THEN** it prints three lines and exits with status 0

#### Scenario: No friends
- **WHEN** the avatar has no friends and `rslbot ctl friends` is run
- **THEN** it prints nothing and exits with status 0

#### Scenario: Between logins
- **WHEN** `rslbot ctl friends` is run while the bot is waiting to log in again
- **THEN** it prints that the avatar is not connected to a region and exits with status 1

#### Scenario: With an argument
- **WHEN** `rslbot ctl friends online` is run
- **THEN** it exits with status 1 with an error about the arguments of `friends`, and lists nothing

### Requirement: Friend listing format
Each line SHALL consist of four fields separated by tab characters: the friend's UUID; its status, which is `online`, `offline`, or `?` when unknown; its name, `First Last`; its display name. A name or a display name the bot does not know SHALL be `?`. There SHALL be no header line. A tab, line feed, carriage return or backslash inside a name or a display name SHALL be written as `\t`, `\n`, `\r` and `\\`.

#### Scenario: A line of the listing
- **WHEN** Jane Doe, whose display name is `JD`, is a friend and counts as online
- **THEN** her line is her UUID, `online`, `Jane Doe` and `JD`, separated by tabs

#### Scenario: Name the name service does not give
- **WHEN** the name service does not name a friend, or the simulator offers no name service
- **THEN** that friend's line has its UUID and its status, and `?` as the third and as the fourth field

### Requirement: Friend listing order
The lines SHALL be ordered by status, `online` first, then `offline`, then `?`; within a status by name without regard to letter case, with the friends whose name the bot does not know last; and by UUID where that does not decide.

#### Scenario: Online first
- **WHEN** Ann Poe is offline, John Roe is online, Jane Doe is online and Zed Low hides his status
- **THEN** the lines come in the order Jane Doe, John Roe, Ann Poe, Zed Low

### Requirement: Names of friends
The bot SHALL ask for the friends' names through the name service the message log uses, only when a `friends` command is given and only for the friends whose names it does not have. It SHALL answer once it has the names, and no later than 10 seconds after the command was given, with `?` for the names it lacks.

#### Scenario: First listing
- **WHEN** `rslbot ctl friends` is the first to be run after a login and the avatar has 120 friends
- **THEN** the name service is asked for those 120 avatars, and the command is answered with their names within 11 seconds

#### Scenario: Second listing
- **WHEN** `rslbot ctl friends` has been answered with every name and is run again
- **THEN** it is answered within a second and the name service is asked nothing

#### Scenario: Name service that does not answer
- **WHEN** the name service does not answer and `rslbot ctl friends` is run
- **THEN** the command exits with status 0 within 11 seconds, with `?` as name and as display name in every line

#### Scenario: No command, no lookups
- **WHEN** the bot logs in with 120 friends and no `friends` command is given
- **THEN** the name service is asked for nobody on account of the friend list

### Requirement: Friends reads only
`friends` SHALL NOT send the simulator anything, SHALL NOT move the avatar and SHALL NOT change the bot's placement mode or the held placement. No RLV restriction SHALL be applied to it, and it SHALL NOT be the exempt command of an RLV override (see `rlv-restrictions`, "Override exempts one command"). The bot SHALL NOT offer, accept or end a friendship and SHALL NOT change what the avatar lets its friends do.

#### Scenario: During an override
- **WHEN** `rslbot ctl rlv mode override` has been carried out and `rslbot ctl friends` is run
- **THEN** the listing is printed and `rlv status` still prints `rlv: override`

#### Scenario: In holding mode
- **WHEN** `rslbot ctl friends` is run in holding mode
- **THEN** `status` still prints `mode: holding`

### Requirement: Friends appear in the operator log
The bot SHALL write a line to its operator log for every `friends` it answers, with the number of friends listed and how many of them are online; for every `friends` it refuses, with the reason; for the number of friends the grid listed at each login; and for every friendship that ends or begins while the bot runs, naming the avatar by its UUID.

#### Scenario: Reading the log after a listing
- **WHEN** `rslbot ctl friends` has listed five friends, two of them online
- **THEN** the log shows a line for the command with the counts 5 and 2

#### Scenario: Friendship ended
- **WHEN** a friend ends the friendship while the bot is connected
- **THEN** the log shows a line saying so with that avatar's UUID
