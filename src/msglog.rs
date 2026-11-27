//! Recording of chat and instant messages, and of the avatar's
//! transactions: a queue that holds each until the names that go with it
//! are known (or given up on), and the standard-error line and TSV row it
//! is then written as.

use crate::config::{LogPath, LogPaths};
use crate::escape::escape;
use crate::grouplog::GroupMessage;
use crate::log::format_timestamp;
use crate::names::Names;
use crate::session::{Message, Source};
use crate::txlog::Transaction;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a message may wait for its names.
pub const NAME_WAIT: Duration = Duration::from_secs(5);
/// How long a looked-up name, or the grid's "no such avatar", is trusted.
const CACHE_LIFETIME: Duration = Duration::from_secs(3600);
/// With this many messages waiting, new ones no longer wait for names.
const MAX_PENDING: usize = 1000;
/// Expired cache entries are dropped once the cache has grown this large.
const CACHE_PRUNE_SIZE: usize = 10_000;

pub const COLUMNS: [&str; 7] = ["time", "kind", "avatar_name", "display_name", "object_name", "object_uuid", "text"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Channel {
    Chat,
    Im,
}

/// A message with its names settled, ready to be written.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub channel: Channel,
    /// Seconds since the Unix epoch.
    pub time: u64,
    pub kind: String,
    pub avatar_name: String,
    pub display_name: String,
    pub object_name: String,
    pub object_uuid: Option<Uuid>,
    pub text: String,
    /// The avatar who sent it, or the recipient of one the bot sent; not
    /// written to the logs.
    pub avatar_uuid: Option<Uuid>,
    /// It was stored while the avatar was offline, and `time` is when it
    /// was sent.
    pub offline: bool,
}

/// What waits in the queue.
enum Item {
    Message(Channel, Message),
    Transaction(Transaction),
    Group(GroupMessage),
}

/// What leaves the queue, in arrival order.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    Message(Record),
    Transaction(Transaction),
    Group(GroupMessage),
}

#[cfg(test)]
impl Recorded {
    /// The message this is; a test's mistake otherwise.
    pub fn message(&self) -> &Record {
        match self {
            Recorded::Message(record) => record,
            Recorded::Transaction(tx) => panic!("a transaction, not a message: {tx:?}"),
            Recorded::Group(group) => panic!("a group message, not a message: {group:?}"),
        }
    }
}

/// Lets the tests compare what leaves the queue with the messages they
/// expect.
#[cfg(test)]
impl PartialEq<Record> for Recorded {
    fn eq(&self, other: &Record) -> bool {
        matches!(self, Recorded::Message(record) if record == other)
    }
}

struct Entry {
    item: Item,
    time: u64,
    /// The avatar whose names the record needs: the sender, or the owner.
    needs: Option<Uuid>,
    deadline: Instant,
}

/// Holds messages in arrival order until their names are settled.
#[derive(Default)]
pub struct Recorder {
    pending: VecDeque<Entry>,
    cache: HashMap<Uuid, (Option<Names>, Instant)>,
    /// Ids to look up, not yet handed out by `take_lookups`.
    wanted: Vec<Uuid>,
    /// Ids handed out and not answered yet.
    in_flight: HashSet<Uuid>,
}

impl Recorder {
    fn cached(&self, id: Uuid, now: Instant) -> Option<&Option<Names>> {
        let (names, at) = self.cache.get(&id)?;
        (now.duration_since(*at) < CACHE_LIFETIME).then_some(names)
    }

    fn awaited(&self, id: Uuid) -> bool {
        self.in_flight.contains(&id) || self.wanted.contains(&id)
    }

    /// Queues a message received at wall-clock time `received` (seconds
    /// since the Unix epoch).
    pub fn push(&mut self, channel: Channel, message: Message, received: u64, now: Instant) {
        let needs = match &message.source {
            Source::Avatar { id, .. } => Some(*id),
            Source::Object { owner, .. } => Some(*owner),
            Source::System { .. } => None,
        };
        let needs = needs.filter(|id| !id.is_nil() && self.pending.len() < MAX_PENDING);
        if let Some(id) = needs {
            if self.cached(id, now).is_none() && !self.awaited(id) {
                self.wanted.push(id);
            }
        }
        let time = message.sent_at.map_or(received, u64::from);
        self.pending.push_back(Entry {
            item: Item::Message(channel, message),
            time,
            needs,
            deadline: now + NAME_WAIT,
        });
    }

    /// Queues a transaction; it waits for the names of its party, an
    /// avatar, as a message waits for its sender's.
    pub fn push_transaction(&mut self, transaction: Transaction, now: Instant) {
        let needs = transaction
            .party
            .filter(|id| !transaction.party_group && !id.is_nil() && self.pending.len() < MAX_PENDING);
        if let Some(id) = needs {
            if self.cached(id, now).is_none() && !self.awaited(id) {
                self.wanted.push(id);
            }
        }
        self.pending.push_back(Entry {
            time: transaction.time,
            item: Item::Transaction(transaction),
            needs,
            deadline: now + NAME_WAIT,
        });
    }

    /// Queues a group message; it waits for the display name of who said
    /// or sent it, when its UUID is known, as a message waits for its
    /// sender's.
    pub fn push_group(&mut self, message: GroupMessage, now: Instant) {
        let needs = message.avatar.filter(|id| !id.is_nil() && self.pending.len() < MAX_PENDING);
        if let Some(id) = needs {
            if self.cached(id, now).is_none() && !self.awaited(id) {
                self.wanted.push(id);
            }
        }
        self.pending.push_back(Entry {
            time: message.time,
            item: Item::Group(message),
            needs,
            deadline: now + NAME_WAIT,
        });
    }

    /// Whether any ids are waiting to be looked up.
    #[cfg(test)]
    pub fn has_lookups(&self) -> bool {
        !self.wanted.is_empty()
    }

    /// The ids to look up now; each is handed out once.
    pub fn take_lookups(&mut self) -> Vec<Uuid> {
        let ids = std::mem::take(&mut self.wanted);
        self.in_flight.extend(&ids);
        ids
    }

    /// The grid answered: `found` avatars, and `missing` ids it does not
    /// know as avatars (a group, for a group-owned object).
    pub fn names_resolved(&mut self, found: Vec<(Uuid, Names)>, missing: Vec<Uuid>, now: Instant) {
        if self.cache.len() >= CACHE_PRUNE_SIZE {
            self.cache.retain(|_, (_, at)| now.duration_since(*at) < CACHE_LIFETIME);
        }
        for (id, names) in found {
            self.in_flight.remove(&id);
            self.cache.insert(id, (Some(names), now));
        }
        for id in missing {
            self.in_flight.remove(&id);
            self.cache.insert(id, (None, now));
        }
    }

    /// The lookup got no answer; the messages waiting on it go out without
    /// names and the next message from these avatars tries again.
    pub fn lookup_failed(&mut self, ids: &[Uuid]) {
        for id in ids {
            self.in_flight.remove(id);
        }
    }

    fn settled(&self, entry: &Entry, now: Instant) -> bool {
        now >= entry.deadline || entry.needs.is_none_or(|id| !self.awaited(id))
    }

    fn record(&self, entry: Entry, now: Instant) -> Recorded {
        let names = entry.needs.and_then(|id| self.cached(id, now).cloned().flatten());
        let (channel, message) = match entry.item {
            Item::Message(channel, message) => (channel, message),
            Item::Transaction(mut transaction) => {
                if let Some(names) = names {
                    transaction.avatar_name = names.legacy;
                    transaction.display_name = names.display;
                }
                return Recorded::Transaction(transaction);
            }
            Item::Group(mut message) => {
                // The name the message carries; the display name looked up.
                if let Some(names) = names {
                    if message.avatar_name.is_empty() {
                        message.avatar_name = names.legacy;
                    }
                    message.display_name = names.display;
                }
                return Recorded::Group(message);
            }
        };
        let (legacy, display) = names.map(|n| (n.legacy, n.display)).unwrap_or_default();
        let avatar_uuid = match &message.source {
            Source::Avatar { id, .. } if !id.is_nil() => Some(*id),
            _ => None,
        };
        let offline = message.sent_at.is_some();
        let (avatar_name, object_name, object_uuid) = match message.source {
            // The simulator names the speaker itself; the recipient of an
            // IM sent by UUID is known by the looked-up name only.
            Source::Avatar { name, .. } if name.is_empty() => (legacy, String::new(), None),
            Source::Avatar { name, .. } | Source::System { name } => (name, String::new(), None),
            Source::Object { id, name, .. } => (legacy, name, id),
        };
        Recorded::Message(Record {
            channel,
            time: entry.time,
            kind: message.kind,
            avatar_name,
            display_name: display,
            object_name,
            object_uuid,
            text: message.text,
            avatar_uuid,
            offline,
        })
    }

    /// The messages and transactions that can be written now, in arrival
    /// order. None is released before one that arrived earlier.
    pub fn flush(&mut self, now: Instant) -> Vec<Recorded> {
        let mut records = Vec::new();
        while self.pending.front().is_some_and(|entry| self.settled(entry, now)) {
            let entry = self.pending.pop_front().expect("front was checked");
            records.push(self.record(entry, now));
        }
        records
    }

    /// Everything still waiting, with whatever names are known.
    pub fn flush_all(&mut self, now: Instant) -> Vec<Recorded> {
        let entries = std::mem::take(&mut self.pending);
        entries.into_iter().map(|entry| self.record(entry, now)).collect()
    }

    /// When `flush` next has something to release without further input.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.front().map(|entry| entry.deadline)
    }
}

// ------------------------------------------------------------------ output

fn line(fields: [&str; 7]) -> String {
    fields.map(escape).join("\t") + "\n"
}

pub fn header() -> String {
    line(COLUMNS)
}

impl Record {
    /// The record as one line of a TSV log, with its line ending.
    pub fn row(&self) -> String {
        let uuid = self.object_uuid.map(|id| id.to_string()).unwrap_or_default();
        line([
            &format_timestamp(self.time),
            &self.kind,
            &self.avatar_name,
            &self.display_name,
            &self.object_name,
            &uuid,
            &self.text,
        ])
    }

    /// The record as a standard-error log entry (without its timestamp).
    pub fn log_line(&self) -> String {
        let mut who = self.avatar_name.clone();
        if !self.display_name.is_empty() {
            who = format!("{who} ({})", self.display_name).trim_start().to_string();
        }
        let from_object = !self.object_name.is_empty() || self.object_uuid.is_some();
        if from_object {
            let mut object = self.object_name.clone();
            if let Some(id) = self.object_uuid {
                object = format!("{object} <{id}>").trim_start().to_string();
            }
            who = if who.is_empty() { object } else { format!("{object}, owner {who}") };
        }
        let channel = match self.channel {
            Channel::Chat => "chat",
            Channel::Im => "im",
        };
        format!("{channel} [{}] {who}: {}", self.kind, self.text)
    }
}

/// One TSV file, open for appending.
struct LogFile {
    path: PathBuf,
    file: File,
}

impl LogFile {
    /// Opens the file for appending, creating it, and writes `header` where
    /// it is new or empty.
    fn open(path: &Path, header: &str) -> io::Result<LogFile> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if file.metadata()?.len() == 0 {
            file.write_all(header.as_bytes())?;
        }
        Ok(LogFile {
            path: path.to_path_buf(),
            file,
        })
    }

    /// One `write` per row keeps rows whole for anyone reading along.
    fn append(&mut self, row: &str) {
        if let Err(e) = self.file.write_all(row.as_bytes()) {
            crate::info!("cannot write to {}: {e}", self.path.display());
        }
    }
}

/// The transactions file. It is opened for each row, so that a file that
/// could not be written to takes rows again once it can; the rows that
/// could not be written wait, and whether one of them counted against the
/// limit.
struct TransactionsFile {
    path: PathBuf,
    unwritten: Vec<(String, bool)>,
}

impl TransactionsFile {
    fn append(&self, rows: &str) -> io::Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        if file.metadata()?.len() == 0 {
            file.write_all(crate::txlog::header().as_bytes())?;
        }
        file.write_all(rows.as_bytes())
    }

    /// Writes the rows that wait, oldest first, in one `write`. `fresh` of
    /// them were added just now; the log names the ones that waited.
    fn retry(&mut self, fresh: usize) -> io::Result<()> {
        if self.unwritten.is_empty() {
            return Ok(());
        }
        let rows: String = self.unwritten.iter().map(|(row, _)| row.as_str()).collect();
        self.append(&rows)?;
        let waited = self.unwritten.len().saturating_sub(fresh);
        if waited > 0 {
            crate::info!("wrote {waited} row(s) that waited to {}", self.path.display());
        }
        self.unwritten.clear();
        Ok(())
    }
}

/// Where records go: standard error, and the configured files.
pub struct Logs {
    chat: Option<LogFile>,
    im: Option<LogFile>,
    group: Option<LogFile>,
    transactions: Option<TransactionsFile>,
}

/// A log file that could not be opened: the config key naming it and why.
#[derive(Debug)]
pub struct OpenError {
    pub key: &'static str,
    pub message: String,
}

impl Logs {
    /// Opens the configured files, creating them and writing the header row
    /// where they are new or empty.
    pub fn open(paths: &LogPaths) -> Result<Logs, OpenError> {
        let error = |log: &LogPath, e: io::Error| OpenError {
            key: log.key,
            message: format!("cannot open {}: {e}", log.path.display()),
        };
        let open_with = |log: &Option<LogPath>, header: &str| {
            log.as_ref()
                .map(|log| LogFile::open(&log.path, header).map_err(|e| error(log, e)))
                .transpose()
        };
        let open = |log: &Option<LogPath>| open_with(log, &header());
        let transactions = match &paths.transactions {
            None => None,
            Some(log) => {
                LogFile::open(&log.path, &crate::txlog::header()).map_err(|e| error(log, e))?;
                Some(TransactionsFile {
                    path: log.path.clone(),
                    unwritten: Vec::new(),
                })
            }
        };
        Ok(Logs {
            chat: open(&paths.chat)?,
            im: open(&paths.im)?,
            group: open_with(&paths.group, &crate::grouplog::header())?,
            transactions,
        })
    }

    /// Logs that write nothing but standard error.
    #[cfg(test)]
    pub fn none() -> Logs {
        Logs {
            chat: None,
            im: None,
            group: None,
            transactions: None,
        }
    }

    pub fn write(&mut self, record: &Record) {
        crate::info!("{}", record.log_line());
        let file = match record.channel {
            Channel::Chat => &mut self.chat,
            Channel::Im => &mut self.im,
        };
        if let Some(file) = file {
            file.append(&record.row());
        }
    }

    /// Writes a group message to standard error and, when one is
    /// configured, to the group file.
    pub fn write_group(&mut self, message: &GroupMessage) {
        crate::info!("{}", message.log_line());
        if let Some(file) = &mut self.group {
            file.append(&message.row());
        }
    }

    /// Writes a transaction to standard error and, when one is configured,
    /// to the transactions file, after the rows that wait. A row that
    /// cannot be written waits for the next try.
    pub fn write_transaction(&mut self, transaction: &Transaction) {
        crate::info!("{}", transaction.log_line());
        let Some(file) = &mut self.transactions else {
            return;
        };
        file.unwritten.push((transaction.row(), transaction.counted > 0));
        if let Err(e) = file.retry(1) {
            crate::info!(
                "cannot write to {}: {e}; {} row(s) wait to be written",
                file.path.display(),
                file.unwritten.len()
            );
        }
    }

    /// Tries again to write the rows that could not be written. Returns
    /// whether one that counted against the limit still waits, and the
    /// file's path then.
    pub fn retry_transactions(&mut self) -> Option<PathBuf> {
        let file = self.transactions.as_mut()?;
        if let Err(e) = file.retry(0) {
            crate::info!("cannot write to {}: {e}", file.path.display());
        }
        file.unwritten.iter().any(|(_, counted)| *counted).then(|| file.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_795_021_463; // 2026-11-18T17:04:23Z

    fn id(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn jane() -> Names {
        Names {
            legacy: "Jane Doe".into(),
            display: "JD".into(),
        }
    }

    fn said(by: u8, name: &str, text: &str) -> Message {
        let source = Source::Avatar {
            id: id(by),
            name: name.into(),
        };
        Message {
            kind: "say".into(),
            source,
            sent_at: None,
            text: text.into(),
        }
    }

    fn object_said(owner: u8, text: &str) -> Message {
        let source = Source::Object {
            id: Some(id(0xB0)),
            name: "Clock".into(),
            owner: id(owner),
        };
        Message {
            kind: "say".into(),
            source,
            sent_at: None,
            text: text.into(),
        }
    }

    fn record(avatar: &str, display: &str, text: &str) -> Record {
        Record {
            channel: Channel::Chat,
            time: T0,
            kind: "say".into(),
            avatar_name: avatar.into(),
            display_name: display.into(),
            object_name: String::new(),
            object_uuid: None,
            text: text.into(),
            avatar_uuid: None,
            offline: false,
        }
    }

    /// `record` as the avatar `by` sent it (or was sent it).
    fn by(by: u8, record: Record) -> Record {
        Record {
            avatar_uuid: Some(id(by)),
            ..record
        }
    }

    fn object_record(avatar: &str, display: &str) -> Record {
        Record {
            object_name: "Clock".into(),
            object_uuid: Some(id(0xB0)),
            ..record(avatar, display, "tick")
        }
    }

    fn texts(records: &[Record]) -> Vec<&str> {
        records.iter().map(|r| r.text.as_str()).collect()
    }

    /// The messages among what left the queue; a transaction is a test's
    /// mistake.
    fn messages(recorded: Vec<Recorded>) -> Vec<Record> {
        recorded.iter().map(|r| r.message().clone()).collect()
    }

    fn all_texts(records: &[Recorded]) -> Vec<&str> {
        records
            .iter()
            .map(|r| match r {
                Recorded::Message(record) => record.text.as_str(),
                Recorded::Transaction(tx) => tx.text.as_str(),
                Recorded::Group(message) => message.text.as_str(),
            })
            .collect()
    }

    fn paths(chat: Option<&Path>, im: Option<&Path>, transactions: Option<&Path>) -> LogPaths {
        let log = |path: Option<&Path>, key| {
            path.map(|path| LogPath {
                path: path.to_path_buf(),
                key,
            })
        };
        LogPaths {
            chat: log(chat, "logs.chat"),
            im: log(im, "logs.im"),
            group: None,
            transactions: log(transactions, "logs.transactions"),
        }
    }

    // ------------------------------------------------------------ formatting

    #[test]
    fn formats_the_spec_row() {
        assert_eq!(
            record("Jane Doe", "JD", "hello").row(),
            "2026-11-18T17:04:23Z\tsay\tJane Doe\tJD\t\t\thello\n"
        );
        assert_eq!(header(), "time\tkind\tavatar_name\tdisplay_name\tobject_name\tobject_uuid\ttext\n");
    }

    #[test]
    fn formats_an_object_row() {
        assert_eq!(
            object_record("Jane Doe", "JD").row(),
            format!("2026-11-18T17:04:23Z\tsay\tJane Doe\tJD\tClock\t{}\ttick\n", id(0xB0))
        );
    }

    #[test]
    fn escapes_what_would_break_a_row() {
        let row = record("Tab\tby", "Back\\slash", "a\tb\nc\r\nd\\n").row();
        assert_eq!(
            row,
            "2026-11-18T17:04:23Z\tsay\tTab\\tby\tBack\\\\slash\t\t\ta\\tb\\nc\\r\\nd\\\\n\n"
        );
        assert_eq!(row.matches('\t').count(), 6);
        assert_eq!(row.matches('\n').count(), 1);
    }

    #[test]
    fn formats_log_lines() {
        assert_eq!(record("Jane Doe", "JD", "hello").log_line(), "chat [say] Jane Doe (JD): hello");
        assert_eq!(record("Jane Doe", "", "hello").log_line(), "chat [say] Jane Doe: hello");
        assert_eq!(
            object_record("Jane Doe", "JD").log_line(),
            format!("chat [say] Clock <{}>, owner Jane Doe (JD): tick", id(0xB0))
        );
        assert_eq!(object_record("", "").log_line(), format!("chat [say] Clock <{}>: tick", id(0xB0)));
        let im = Record {
            channel: Channel::Im,
            kind: "object".into(),
            object_uuid: None,
            ..object_record("Jane Doe", "")
        };
        assert_eq!(im.log_line(), "im [object] Clock, owner Jane Doe: tick");
    }

    // ----------------------------------------------------------------- queue

    #[test]
    fn message_waits_for_its_display_name() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        assert!(rec.take_lookups().is_empty(), "an id is handed out once");
        assert_eq!(messages(rec.flush(now)), []);
        assert_eq!(rec.next_deadline(), Some(now + NAME_WAIT));

        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(messages(rec.flush(now)), [by(1, record("Jane Doe", "JD", "hello"))]);
        assert_eq!(rec.next_deadline(), None);
    }

    // ----------------------------------------------- instant messages sent

    fn sent(to: u8, name: &str, text: &str) -> Message {
        let source = Source::Avatar {
            id: id(to),
            name: name.into(),
        };
        Message {
            kind: "im_out".into(),
            source,
            sent_at: None,
            text: text.into(),
        }
    }

    fn im_record(kind: &str, avatar: &str, display: &str, text: &str) -> Record {
        Record {
            channel: Channel::Im,
            kind: kind.into(),
            ..record(avatar, display, text)
        }
    }

    #[test]
    fn sent_instant_message_carries_the_recipients_names() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Im, sent(1, "Jane Doe", "are you there?"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        let records = messages(rec.flush(now));
        assert_eq!(records, [by(1, im_record("im_out", "Jane Doe", "JD", "are you there?"))]);
        assert_eq!(records[0].row(), "2026-11-18T17:04:23Z\tim_out\tJane Doe\tJD\t\t\tare you there?\n");
        assert_eq!(records[0].log_line(), "im [im_out] Jane Doe (JD): are you there?");
    }

    #[test]
    fn sent_instant_message_to_a_uuid_takes_its_name_from_the_lookup() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Im, sent(1, "", "hello"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(messages(rec.flush(now)), [by(1, im_record("im_out", "Jane Doe", "JD", "hello"))]);

        // A recipient the grid cannot name still gets its row.
        rec.push(Channel::Im, sent(2, "", "anyone?"), T0, now);
        assert_eq!(rec.take_lookups(), [id(2)]);
        assert_eq!(messages(rec.flush(now + NAME_WAIT - Duration::from_millis(1))), []);
        let records = messages(rec.flush(now + NAME_WAIT));
        assert_eq!(records, [by(2, im_record("im_out", "", "", "anyone?"))]);
        assert_eq!(records[0].row(), "2026-11-18T17:04:23Z\tim_out\t\t\t\t\tanyone?\n");
    }

    #[test]
    fn a_reply_is_recorded_after_the_message_it_answers() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Im, sent(1, "Jane Doe", "are you there?"), T0, now);
        let reply = Message {
            kind: "im".into(),
            ..said(1, "Jane Doe", "yes")
        };
        rec.push(Channel::Im, reply, T0 + 2, now);
        assert_eq!(rec.take_lookups(), [id(1)], "one lookup serves both");
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        let expected = [
            by(1, im_record("im_out", "Jane Doe", "JD", "are you there?")),
            Record {
                time: T0 + 2,
                ..by(1, im_record("im", "Jane Doe", "JD", "yes"))
            },
        ];
        assert_eq!(messages(rec.flush(now)), expected);
    }

    #[test]
    fn arrival_order_is_kept_when_a_later_name_resolves_first() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "first"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        rec.push(Channel::Chat, said(2, "John Roe", "second"), T0, now);
        assert_eq!(rec.take_lookups(), [id(2)]);

        rec.names_resolved(
            vec![(
                id(2),
                Names {
                    legacy: "John Roe".into(),
                    display: "Johnny".into(),
                },
            )],
            vec![],
            now,
        );
        assert_eq!(messages(rec.flush(now)), []);
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(texts(&messages(rec.flush(now))), ["first", "second"]);
    }

    #[test]
    fn unanswered_lookup_times_out_after_five_seconds() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, now);
        // No lookup service is known, so nothing is even handed out.
        assert_eq!(messages(rec.flush(now + NAME_WAIT - Duration::from_millis(1))), []);
        assert_eq!(messages(rec.flush(now + NAME_WAIT)), [by(1, record("Jane Doe", "", "hello"))]);
    }

    #[test]
    fn failed_lookup_releases_the_message_and_is_retried_next_time() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, now);
        let ids = rec.take_lookups();
        rec.lookup_failed(&ids);
        assert_eq!(messages(rec.flush(now)), [by(1, record("Jane Doe", "", "hello"))]);

        rec.push(Channel::Chat, said(1, "Jane Doe", "again"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
    }

    #[test]
    fn cached_names_need_no_lookup_for_an_hour() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.names_resolved(vec![(id(1), jane())], vec![], now);

        let later = now + CACHE_LIFETIME - Duration::from_secs(1);
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, later);
        assert!(!rec.has_lookups());
        assert_eq!(messages(rec.flush(later)), [by(1, record("Jane Doe", "JD", "hello"))]);

        let expired = now + CACHE_LIFETIME;
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, expired);
        assert_eq!(rec.take_lookups(), [id(1)]);
    }

    #[test]
    fn one_lookup_serves_several_waiting_messages() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "one"), T0, now);
        rec.push(Channel::Chat, said(1, "Jane Doe", "two"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        rec.push(Channel::Chat, said(1, "Jane Doe", "three"), T0, now);
        assert!(!rec.has_lookups());
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(texts(&messages(rec.flush(now))), ["one", "two", "three"]);
    }

    #[test]
    fn grid_messages_need_no_lookup() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        let source = Source::System {
            name: "Second Life".into(),
        };
        let message = Message {
            kind: "im".into(),
            source,
            sent_at: None,
            text: "notice".into(),
        };
        rec.push(Channel::Im, message, T0, now);
        assert!(!rec.has_lookups());
        let expected = Record {
            channel: Channel::Im,
            kind: "im".into(),
            ..record("Second Life", "", "notice")
        };
        assert_eq!(messages(rec.flush(now)), [expected]);
    }

    #[test]
    fn object_messages_carry_the_owners_names() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, object_said(1, "tick"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(messages(rec.flush(now)), [object_record("Jane Doe", "JD")]);
    }

    #[test]
    fn group_owned_object_leaves_the_avatar_columns_empty() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, object_said(9, "tick"), T0, now);
        assert_eq!(rec.take_lookups(), [id(9)]);
        rec.names_resolved(vec![], vec![id(9)], now);
        assert_eq!(messages(rec.flush(now)), [object_record("", "")]);
        // The grid's "unknown" is remembered like a name.
        rec.push(Channel::Chat, object_said(9, "tick"), T0, now);
        assert!(!rec.has_lookups());
        assert_eq!(messages(rec.flush(now)).len(), 1);
    }

    #[test]
    fn records_carry_the_avatars_uuid_but_not_in_the_logs() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.names_resolved(vec![(id(1), jane())], vec![(id(9))], now);
        let im = Message {
            kind: "im".into(),
            ..said(1, "Jane Doe", "hi")
        };
        rec.push(Channel::Im, im, T0, now);
        rec.push(Channel::Im, sent(1, "Jane Doe", "hello"), T0, now);
        let object_im = Message {
            kind: "object".into(),
            ..object_said(9, "locked")
        };
        rec.push(Channel::Im, object_im, T0, now);
        let records = messages(rec.flush(now));
        assert_eq!(
            records.iter().map(|r| r.avatar_uuid).collect::<Vec<_>>(),
            [Some(id(1)), Some(id(1)), None]
        );
        assert_eq!(records[0].row(), "2026-11-18T17:04:23Z\tim\tJane Doe\tJD\t\t\thi\n");
        assert_eq!(records[0].log_line(), "im [im] Jane Doe (JD): hi");
        assert!(records.iter().all(|r| !r.offline));
    }

    #[test]
    fn stored_messages_keep_the_time_they_were_sent() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        let mut message = said(1, "Jane Doe", "earlier");
        message.sent_at = Some(1_700_000_000);
        rec.push(Channel::Im, message, T0, now);
        let record = &messages(rec.flush_all(now))[0];
        assert_eq!(record.time, 1_700_000_000);
        assert!(record.offline);
    }

    #[test]
    fn flush_all_releases_everything_with_the_names_known_so_far() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.names_resolved(vec![(id(2), jane())], vec![], now);
        rec.push(Channel::Chat, said(1, "John Roe", "first"), T0, now);
        rec.push(Channel::Chat, said(2, "Jane Doe", "second"), T0, now);
        assert_eq!(messages(rec.flush(now)), []);
        assert_eq!(
            messages(rec.flush_all(now)),
            [by(1, record("John Roe", "", "first")), by(2, record("Jane Doe", "JD", "second"))]
        );
        assert_eq!(messages(rec.flush_all(now)), []);
    }

    #[test]
    fn a_full_queue_stops_waiting_for_names() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        for _ in 0..MAX_PENDING {
            rec.push(Channel::Chat, said(1, "Jane Doe", "flood"), T0, now);
        }
        rec.take_lookups();
        rec.push(Channel::Chat, said(2, "John Roe", "late"), T0, now);
        assert!(!rec.has_lookups(), "no lookup for a message that will not wait");
        rec.lookup_failed(&[id(1)]);
        assert_eq!(messages(rec.flush(now)).len(), MAX_PENDING + 1);
    }

    // ----------------------------------------------------------------- files

    /// A fresh directory under the system's temporary directory.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rslbot-msglog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn creates_the_file_with_a_header_and_appends_across_restarts() {
        let dir = temp_dir("append");
        let path = dir.join("chat.tsv");
        let first = record("Jane Doe", "JD", "hello");
        Logs::open(&paths(Some(&path), None, None)).unwrap().write(&first);
        assert_eq!(read(&path), header() + &first.row());

        let second = record("Jane Doe", "JD", "again");
        Logs::open(&paths(Some(&path), None, None)).unwrap().write(&second);
        assert_eq!(read(&path), header() + &first.row() + &second.row());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_existing_empty_file_gets_the_header() {
        let dir = temp_dir("empty");
        let path = dir.join("im.tsv");
        File::create(&path).unwrap();
        drop(Logs::open(&paths(None, Some(&path), None)).unwrap());
        assert_eq!(read(&path), header());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chat_and_ims_go_only_to_their_own_files() {
        let dir = temp_dir("split");
        let (chat_path, im_path) = (dir.join("chat.tsv"), dir.join("im.tsv"));
        let chat = record("Jane Doe", "JD", "hello");
        let im = Record {
            channel: Channel::Im,
            kind: "im".into(),
            ..record("Jane Doe", "JD", "psst")
        };

        let mut logs = Logs::open(&paths(Some(&chat_path), Some(&im_path), None)).unwrap();
        logs.write(&chat);
        logs.write(&im);
        assert_eq!(read(&chat_path), header() + &chat.row());
        assert_eq!(read(&im_path), header() + &im.row());

        // With only the IM file configured, chat reaches no file.
        std::fs::remove_file(&chat_path).unwrap();
        let mut logs = Logs::open(&paths(None, Some(&im_path), None)).unwrap();
        logs.write(&chat);
        logs.write(&im);
        assert!(!chat_path.exists());
        assert_eq!(read(&im_path), header() + &im.row() + &im.row());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_cannot_be_opened_names_its_key() {
        let dir = temp_dir("missing");
        let path = dir.join("no-such-dir").join("im.tsv");
        let error = Logs::open(&paths(None, Some(&path), None)).err().unwrap();
        assert_eq!(error.key, "logs.im");
        let old = LogPaths {
            im: Some(LogPath {
                path: path.clone(),
                key: "log_im",
            }),
            ..LogPaths::default()
        };
        assert_eq!(Logs::open(&old).err().unwrap().key, "log_im");
        assert!(error.message.contains("no-such-dir"), "{}", error.message);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_write_is_survived() {
        // /dev/full accepts the open and fails every write.
        let full = Path::new("/dev/full");
        if !full.exists() {
            return;
        }
        let mut file = LogFile {
            path: full.to_path_buf(),
            file: OpenOptions::new().append(true).open(full).unwrap(),
        };
        file.append("row\n");
        file.append("row\n");
    }

    // ---------------------------------------------------------- transactions

    fn received_from(party: u8, amount: u64) -> Transaction {
        Transaction {
            time: T0,
            kind: crate::txlog::Kind::Received,
            amount,
            balance: Some(1250),
            counted: 0,
            code: 5001,
            party: Some(id(party)),
            party_group: false,
            avatar_name: String::new(),
            display_name: String::new(),
            object_name: String::new(),
            object: None,
            text: "paid you".into(),
        }
    }

    #[test]
    fn a_transaction_gets_its_partys_names() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push_transaction(received_from(1, 50), now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        assert_eq!(rec.flush(now), Vec::<Recorded>::new());
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        let Recorded::Transaction(tx) = &rec.flush(now)[0] else {
            panic!("not a transaction")
        };
        assert_eq!((tx.avatar_name.as_str(), tx.display_name.as_str()), ("Jane Doe", "JD"));
    }

    #[test]
    fn a_transaction_does_not_wait_longer_than_a_message_for_a_name() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push_transaction(received_from(1, 50), now);
        rec.take_lookups();
        assert_eq!(rec.next_deadline(), Some(now + NAME_WAIT));
        let Recorded::Transaction(tx) = &rec.flush(now + NAME_WAIT)[0] else {
            panic!("not a transaction")
        };
        assert_eq!((tx.party, tx.avatar_name.as_str()), (Some(id(1)), ""));
    }

    #[test]
    fn a_groups_transaction_waits_for_no_name() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push_transaction(
            Transaction {
                party_group: true,
                ..received_from(1, 50)
            },
            now,
        );
        assert!(!rec.has_lookups());
        assert_eq!(rec.flush(now).len(), 1);
    }

    #[test]
    fn messages_and_transactions_leave_in_arrival_order() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "before"), T0, now);
        rec.push_transaction(received_from(2, 50), now);
        rec.push(Channel::Chat, said(3, "John Roe", "after"), T0, now);
        rec.take_lookups();
        // The transaction's name is in; the first message's is not.
        rec.names_resolved(vec![(id(2), jane()), (id(3), jane())], vec![], now);
        assert_eq!(rec.flush(now), Vec::<Recorded>::new());
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(all_texts(&rec.flush(now)), ["before", "paid you", "after"]);
    }

    fn tx_log(path: &Path) -> Logs {
        Logs::open(&paths(None, None, Some(path))).unwrap()
    }

    #[test]
    fn the_transactions_file_gets_its_own_header_and_rows() {
        let dir = temp_dir("transactions");
        let path = dir.join("transactions.tsv");
        let tx = received_from(1, 50);
        tx_log(&path).write_transaction(&tx);
        tx_log(&path).write_transaction(&tx);
        assert_eq!(read(&path), crate::txlog::header() + &tx.row() + &tx.row());
        // Chat goes elsewhere, and a transaction to no chat file.
        let chat = dir.join("chat.tsv");
        let mut logs = Logs::open(&paths(Some(&chat), None, Some(&path))).unwrap();
        logs.write(&record("Jane Doe", "JD", "hello"));
        assert_eq!(read(&chat), header() + &record("Jane Doe", "JD", "hello").row());
        assert_eq!(crate::txlog::read(&path).unwrap().rows.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transactions_without_a_file_go_to_standard_error_only() {
        let mut logs = Logs::none();
        logs.write_transaction(&received_from(1, 50));
        assert_eq!(logs.retry_transactions(), None);
    }

    /// Whether this process may write where the permissions say it may
    /// not, as root may.
    fn permissions_are_ignored(dir: &Path) -> bool {
        let probe = dir.join("probe");
        File::create(&probe).unwrap();
        set_read_only(&probe, true);
        let ignored = OpenOptions::new().append(true).open(&probe).is_ok();
        set_read_only(&probe, false);
        std::fs::remove_file(probe).unwrap();
        ignored
    }

    fn set_read_only(path: &Path, read_only: bool) {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(read_only);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn rows_that_cannot_be_written_wait_and_are_written_later() {
        let dir = temp_dir("readonly");
        if permissions_are_ignored(&dir) {
            return;
        }
        let path = dir.join("transactions.tsv");
        let mut logs = tx_log(&path);
        let paid = Transaction {
            kind: crate::txlog::Kind::Paid,
            counted: 100,
            ..received_from(1, 100)
        };
        set_read_only(&path, true);
        logs.write_transaction(&paid);
        logs.write_transaction(&received_from(1, 50));
        assert_eq!(read(&path), crate::txlog::header(), "nothing written");
        assert_eq!(logs.retry_transactions(), Some(path.clone()), "a counted row waits");
        set_read_only(&path, false);
        assert_eq!(logs.retry_transactions(), None);
        assert_eq!(read(&path), crate::txlog::header() + &paid.row() + &received_from(1, 50).row());
        // A row that does not count waits too, but holds nothing back.
        set_read_only(&path, true);
        logs.write_transaction(&received_from(1, 5));
        assert_eq!(logs.retry_transactions(), None);
        set_read_only(&path, false);
        logs.write_transaction(&received_from(1, 6));
        assert_eq!(crate::txlog::read(&path).unwrap().rows.len(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn group_chat(by: u8, text: &str) -> GroupMessage {
        GroupMessage {
            avatar: Some(id(by)),
            display_name: String::new(),
            time: T0,
            ..crate::grouplog::tests::chat(text)
        }
    }

    #[test]
    fn messages_group_messages_and_transactions_leave_in_arrival_order() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "first"), T0, now);
        rec.push_group(group_chat(1, "second"), now);
        let tx = Transaction {
            time: T0,
            kind: crate::txlog::Kind::Received,
            amount: 5,
            balance: Some(10),
            counted: 0,
            code: 5001,
            party: Some(id(1)),
            party_group: false,
            avatar_name: String::new(),
            display_name: String::new(),
            object_name: String::new(),
            object: None,
            text: "third".into(),
        };
        rec.push_transaction(tx, now);
        assert_eq!(rec.take_lookups(), [id(1)], "one lookup serves all three");
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        let out = rec.flush(now);
        assert_eq!(all_texts(&out), ["first", "second", "third"]);
        let Recorded::Group(group) = &out[1] else {
            panic!("not a group message: {:?}", out[1])
        };
        // The name the message carries; the display name looked up.
        assert_eq!((group.avatar_name.as_str(), group.display_name.as_str()), ("Jane Doe", "JD"));
    }

    #[test]
    fn a_group_message_waits_at_most_five_seconds_for_its_name() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push_group(group_chat(2, "hello"), now);
        assert!(rec.flush(now + NAME_WAIT - Duration::from_millis(1)).is_empty());
        let out = rec.flush(now + NAME_WAIT);
        let Recorded::Group(group) = &out[0] else {
            panic!("not a group message")
        };
        assert_eq!((group.text.as_str(), group.display_name.as_str()), ("hello", ""));
        // A notice that does not give its sender's UUID waits for nothing.
        let notice = GroupMessage {
            avatar: None,
            ..group_chat(2, "notice")
        };
        rec.push_group(notice, now);
        assert_eq!(rec.flush(now).len(), 1);
    }

    #[test]
    fn group_messages_go_only_to_the_group_file() {
        let dir = temp_dir("group");
        let (group_path, im_path) = (dir.join("group.tsv"), dir.join("im.tsv"));
        let paths = LogPaths {
            group: Some(LogPath {
                path: group_path.clone(),
                key: "logs.group",
            }),
            ..LogPaths::default()
        };
        let mut logs = Logs::open(&paths).unwrap();
        let line = group_chat(1, "hello");
        logs.write_group(&line);
        logs.write(&Record {
            channel: Channel::Im,
            kind: "im".into(),
            ..record("Jane Doe", "JD", "psst")
        });
        assert_eq!(read(&group_path), crate::grouplog::header() + &line.row());
        assert!(!im_path.exists());
        // A restart keeps the rows and writes no second header.
        drop(logs);
        Logs::open(&paths).unwrap().write_group(&line);
        assert_eq!(read(&group_path), crate::grouplog::header() + &line.row() + &line.row());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_group_file_that_cannot_be_opened_names_its_key() {
        let dir = temp_dir("group-missing");
        let paths = LogPaths {
            group: Some(LogPath {
                path: dir.join("no-such-dir").join("group.tsv"),
                key: "logs.group",
            }),
            ..LogPaths::default()
        };
        assert_eq!(Logs::open(&paths).err().unwrap().key, "logs.group");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_transactions_file_that_cannot_be_opened_names_its_key() {
        let dir = temp_dir("tx-missing");
        let path = dir.join("no-such-dir").join("tx.tsv");
        let error = Logs::open(&paths(None, None, Some(&path))).err().unwrap();
        assert_eq!(error.key, "logs.transactions");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
