# Spec Delta

## MODIFIED Requirements

### Requirement: Avatar scans appear in the operator log
The bot SHALL write a line to its operator log for every `scan avatars` it answers, with its arguments and the number of lines listed, and for every `scan avatars` it refuses or fails, with the reason. A `scan avatars` the page of the web interface sent by itself (see `web-server`, "Requests the page makes by itself") SHALL get no line, whatever its answer.

#### Scenario: Reading the log after a scan
- **WHEN** `rslbot ctl scan avatars chat` has listed two avatars
- **THEN** the log shows a line for the command with `chat` and the count 2

#### Scenario: A page left open
- **WHEN** a page of the web interface shows its Local chat tab for an hour and refreshes its list of avatars by itself all the while
- **THEN** the log has no line for any of those scans
