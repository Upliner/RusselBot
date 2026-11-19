//! Recording of chat and instant messages: a queue that holds each message
//! until the names that go with it are known (or given up on), and the
//! standard-error line and TSV row it is then written as.

use crate::escape::escape;
use crate::log::format_timestamp;
use crate::names::Names;
use crate::session::{Message, Source};
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
}

struct Entry {
    channel: Channel,
    message: Message,
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
            channel,
            message,
            time,
            needs,
            deadline: now + NAME_WAIT,
        });
    }

    /// Whether any ids are waiting to be looked up.
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

    fn record(&self, entry: Entry, now: Instant) -> Record {
        let names = entry.needs.and_then(|id| self.cached(id, now).cloned().flatten());
        let (legacy, display) = names.map(|n| (n.legacy, n.display)).unwrap_or_default();
        let (avatar_name, object_name, object_uuid) = match entry.message.source {
            // The simulator names the speaker itself; the recipient of an
            // IM sent by UUID is known by the looked-up name only.
            Source::Avatar { name, .. } if name.is_empty() => (legacy, String::new(), None),
            Source::Avatar { name, .. } | Source::System { name } => (name, String::new(), None),
            Source::Object { id, name, .. } => (legacy, name, id),
        };
        Record {
            channel: entry.channel,
            time: entry.time,
            kind: entry.message.kind,
            avatar_name,
            display_name: display,
            object_name,
            object_uuid,
            text: entry.message.text,
        }
    }

    /// The messages that can be written now, in arrival order. A message is
    /// never released before one that arrived earlier.
    pub fn flush(&mut self, now: Instant) -> Vec<Record> {
        let mut records = Vec::new();
        while self.pending.front().is_some_and(|entry| self.settled(entry, now)) {
            let entry = self.pending.pop_front().expect("front was checked");
            records.push(self.record(entry, now));
        }
        records
    }

    /// Everything still waiting, with whatever names are known.
    pub fn flush_all(&mut self, now: Instant) -> Vec<Record> {
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
    fn open(path: &Path) -> io::Result<LogFile> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if file.metadata()?.len() == 0 {
            file.write_all(header().as_bytes())?;
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

/// Where records go: standard error, and the configured files.
pub struct Logs {
    chat: Option<LogFile>,
    im: Option<LogFile>,
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
    pub fn open(chat: Option<&Path>, im: Option<&Path>) -> Result<Logs, OpenError> {
        let open = |path: Option<&Path>, key| {
            path.map(LogFile::open).transpose().map_err(|e| OpenError {
                key,
                message: format!("cannot open {}: {e}", path.unwrap_or(Path::new("")).display()),
            })
        };
        Ok(Logs {
            chat: open(chat, "log_chat")?,
            im: open(im, "log_im")?,
        })
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
        assert_eq!(rec.flush(now), []);
        assert_eq!(rec.next_deadline(), Some(now + NAME_WAIT));

        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(rec.flush(now), [record("Jane Doe", "JD", "hello")]);
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
        let records = rec.flush(now);
        assert_eq!(records, [im_record("im_out", "Jane Doe", "JD", "are you there?")]);
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
        assert_eq!(rec.flush(now), [im_record("im_out", "Jane Doe", "JD", "hello")]);

        // A recipient the grid cannot name still gets its row.
        rec.push(Channel::Im, sent(2, "", "anyone?"), T0, now);
        assert_eq!(rec.take_lookups(), [id(2)]);
        assert_eq!(rec.flush(now + NAME_WAIT - Duration::from_millis(1)), []);
        let records = rec.flush(now + NAME_WAIT);
        assert_eq!(records, [im_record("im_out", "", "", "anyone?")]);
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
            im_record("im_out", "Jane Doe", "JD", "are you there?"),
            Record {
                time: T0 + 2,
                ..im_record("im", "Jane Doe", "JD", "yes")
            },
        ];
        assert_eq!(rec.flush(now), expected);
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
        assert_eq!(rec.flush(now), []);
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(texts(&rec.flush(now)), ["first", "second"]);
    }

    #[test]
    fn unanswered_lookup_times_out_after_five_seconds() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, now);
        // No lookup service is known, so nothing is even handed out.
        assert_eq!(rec.flush(now + NAME_WAIT - Duration::from_millis(1)), []);
        assert_eq!(rec.flush(now + NAME_WAIT), [record("Jane Doe", "", "hello")]);
    }

    #[test]
    fn failed_lookup_releases_the_message_and_is_retried_next_time() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, said(1, "Jane Doe", "hello"), T0, now);
        let ids = rec.take_lookups();
        rec.lookup_failed(&ids);
        assert_eq!(rec.flush(now), [record("Jane Doe", "", "hello")]);

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
        assert_eq!(rec.flush(later), [record("Jane Doe", "JD", "hello")]);

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
        assert_eq!(texts(&rec.flush(now)), ["one", "two", "three"]);
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
        assert_eq!(rec.flush(now), [expected]);
    }

    #[test]
    fn object_messages_carry_the_owners_names() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, object_said(1, "tick"), T0, now);
        assert_eq!(rec.take_lookups(), [id(1)]);
        rec.names_resolved(vec![(id(1), jane())], vec![], now);
        assert_eq!(rec.flush(now), [object_record("Jane Doe", "JD")]);
    }

    #[test]
    fn group_owned_object_leaves_the_avatar_columns_empty() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.push(Channel::Chat, object_said(9, "tick"), T0, now);
        assert_eq!(rec.take_lookups(), [id(9)]);
        rec.names_resolved(vec![], vec![id(9)], now);
        assert_eq!(rec.flush(now), [object_record("", "")]);
        // The grid's "unknown" is remembered like a name.
        rec.push(Channel::Chat, object_said(9, "tick"), T0, now);
        assert!(!rec.has_lookups());
        assert_eq!(rec.flush(now).len(), 1);
    }

    #[test]
    fn stored_messages_keep_the_time_they_were_sent() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        let mut message = said(1, "Jane Doe", "earlier");
        message.sent_at = Some(1_700_000_000);
        rec.push(Channel::Im, message, T0, now);
        assert_eq!(rec.flush_all(now)[0].time, 1_700_000_000);
    }

    #[test]
    fn flush_all_releases_everything_with_the_names_known_so_far() {
        let now = Instant::now();
        let mut rec = Recorder::default();
        rec.names_resolved(vec![(id(2), jane())], vec![], now);
        rec.push(Channel::Chat, said(1, "John Roe", "first"), T0, now);
        rec.push(Channel::Chat, said(2, "Jane Doe", "second"), T0, now);
        assert_eq!(rec.flush(now), []);
        assert_eq!(
            rec.flush_all(now),
            [record("John Roe", "", "first"), record("Jane Doe", "JD", "second")]
        );
        assert_eq!(rec.flush_all(now), []);
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
        assert_eq!(rec.flush(now).len(), MAX_PENDING + 1);
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
        Logs::open(Some(&path), None).unwrap().write(&first);
        assert_eq!(read(&path), header() + &first.row());

        let second = record("Jane Doe", "JD", "again");
        Logs::open(Some(&path), None).unwrap().write(&second);
        assert_eq!(read(&path), header() + &first.row() + &second.row());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_existing_empty_file_gets_the_header() {
        let dir = temp_dir("empty");
        let path = dir.join("im.tsv");
        File::create(&path).unwrap();
        drop(Logs::open(None, Some(&path)).unwrap());
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

        let mut logs = Logs::open(Some(&chat_path), Some(&im_path)).unwrap();
        logs.write(&chat);
        logs.write(&im);
        assert_eq!(read(&chat_path), header() + &chat.row());
        assert_eq!(read(&im_path), header() + &im.row());

        // With only the IM file configured, chat reaches no file.
        std::fs::remove_file(&chat_path).unwrap();
        let mut logs = Logs::open(None, Some(&im_path)).unwrap();
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
        let error = Logs::open(None, Some(&path)).err().unwrap();
        assert_eq!(error.key, "log_im");
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
}
