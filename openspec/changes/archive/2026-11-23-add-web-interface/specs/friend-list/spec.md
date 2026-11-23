# Spec Delta

## MODIFIED Requirements

### Requirement: Friends appear in the operator log
The bot SHALL write a line to its operator log for every `friends` it answers, with the number of friends listed and how many of them are online; for every `friends` it refuses, with the reason, but for neither when the page of the web interface sent the command by itself (see `web-server`, "Requests the page makes by itself"); for the number of friends the grid listed at each login; and for every friendship that ends or begins while the bot runs, naming the avatar by its UUID.

#### Scenario: Reading the log after a listing
- **WHEN** `rslbot ctl friends` has listed five friends, two of them online
- **THEN** the log shows a line for the command with the counts 5 and 2

#### Scenario: Friendship ended
- **WHEN** a friend ends the friendship while the bot is connected
- **THEN** the log shows a line saying so with that avatar's UUID

#### Scenario: A page left open
- **WHEN** a page of the web interface shows its IM tab for an hour and refreshes its friend list by itself all the while
- **THEN** the log has no line for any of those listings
