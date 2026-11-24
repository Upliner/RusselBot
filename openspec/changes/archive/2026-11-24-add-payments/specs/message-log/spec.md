# Spec Delta

## MODIFIED Requirements

### Requirement: Messages are logged to standard error
The bot SHALL write every recorded chat message and instant message to standard error as one timestamped log line, at the normal log level and regardless of whether a log file is configured. The line SHALL say whether it is chat or an instant message and SHALL contain the kind, the names and UUID described above that are not empty, and the text. Text spanning several lines SHALL be written on one line.

#### Scenario: Chat without any log file
- **WHEN** the config names no log file and an avatar says `hello` nearby
- **THEN** standard error receives one line containing the avatar's name and `hello`

#### Scenario: Multi-line text
- **WHEN** a message's text contains line breaks
- **THEN** it still produces exactly one line on standard error

### Requirement: TSV log files
When the config names a chat file, as `chat` in its `[logs]` table or with the deprecated key `log_chat` (see `bot-config`), the bot SHALL append one row per recorded chat message to that file; when it names an IM file, as `im` in `[logs]` or with the deprecated key `log_im`, it SHALL append one row per recorded instant message to that file. Where a requirement or a scenario of this or another capability says that `log_chat` or `log_im` is set or configured, it means that the config names that file, by either key. A row SHALL be one line of seven tab-separated fields in this order: `time`, `kind`, `avatar_name`, `display_name`, `object_name`, `object_uuid`, `text`. `time` SHALL be UTC in the form `YYYY-MM-DDTHH:MM:SSZ`: the time the bot received the message, or for an instant message stored while the avatar was offline the time it was sent. Inside a field, a tab SHALL be written as `\t`, a line feed as `\n`, a carriage return as `\r` and a backslash as `\\`, so that every row is exactly one line with exactly six tabs. The file SHALL be UTF-8. An existing file SHALL be appended to, never truncated. When the bot creates the file or finds it empty it SHALL first write a header row holding the seven field names. Each row SHALL be on disk as soon as the message is recorded, without waiting for more messages or for the bot to exit.

#### Scenario: Chat row
- **WHEN** `chat` is set under `[logs]` and `Jane Doe` (display name `JD`) says `hello` at 2026-11-18 17:04:23 UTC
- **THEN** the file gains the line `2026-11-18T17:04:23Z`, `say`, `Jane Doe`, `JD`, empty, empty, `hello`, separated by tabs

#### Scenario: Text with a tab and a line break
- **WHEN** a recorded message's text is `a`, a tab, `b`, a line feed, `c`
- **THEN** its row's last field is `a\tb\nc` and the row is a single line

#### Scenario: Restart keeps earlier rows
- **WHEN** the bot is restarted with the same chat file
- **THEN** the rows written before are still there, new rows follow them, and no second header row is written

#### Scenario: IMs go to their own file
- **WHEN** `im` is set under `[logs]`, `chat` is not, and both a chat message and an instant message arrive
- **THEN** the IM file gains one row for the instant message and no file receives the chat message

#### Scenario: Deprecated key
- **WHEN** the config has `log_im = "im.tsv"` and no `[logs]` table, and an instant message arrives
- **THEN** `im.tsv` gains its row, exactly as with `im = "im.tsv"` under `[logs]`
