//! What the pages are sent: the recorded messages, kept for pages that
//! open later, and the status. The main loop owns the hub and hands it
//! every record; each open page is a bounded channel that is never waited
//! for. No sockets and no clock here.

use crate::escape::escape;
use crate::msglog::{Channel, Record};
use std::collections::VecDeque;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

/// How many messages of each channel are kept.
pub const KEPT: usize = 500;
/// How many events may wait for a page before its stream is closed.
pub const QUEUE: usize = 256;

/// A line that keeps a quiet stream alive: a comment, which pages ignore.
pub const KEEPALIVE: &str = ": \n\n";

/// Tells a page to drop every message it holds; the kept ones follow.
const RESET: &str = "event: reset\ndata: -\n\n";

/// The newest messages of one channel, as the events they were sent as.
#[derive(Default)]
struct Ring {
    events: VecDeque<(u64, Arc<str>)>,
    /// The number of the newest event that was dropped; 0 for none.
    dropped: u64,
}

impl Ring {
    fn push(&mut self, number: u64, event: Arc<str>) {
        self.events.push_back((number, event));
        while self.events.len() > KEPT {
            if let Some((number, _)) = self.events.pop_front() {
                self.dropped = number;
            }
        }
    }
}

pub struct Hub {
    /// When the bot started, in seconds since the epoch: part of every
    /// event's id, so that a page can tell a restart.
    start: u64,
    /// The number of the last message recorded.
    last: u64,
    chat: Ring,
    im: Ring,
    subscribers: Vec<SyncSender<Arc<str>>>,
    /// The status as it was last sent to everyone.
    status: Option<String>,
}

/// The fields of a line of an event, escaped and separated by tabs: a
/// line that holds no line end, whatever the fields hold.
fn data(fields: &[&str]) -> String {
    fields.iter().map(|field| escape(field)).collect::<Vec<_>>().join("\t")
}

/// A recorded message as an event.
fn message_event(id: &str, record: &Record) -> String {
    let kind = match record.channel {
        Channel::Chat => "chat",
        Channel::Im => "im",
    };
    let uuid = |id: Option<uuid::Uuid>| id.map(|id| id.to_string()).unwrap_or_default();
    let (time, avatar, object) = (record.time.to_string(), uuid(record.avatar_uuid), uuid(record.object_uuid));
    let fields: [&str; 9] = [
        &time,
        &record.kind,
        &avatar,
        &record.avatar_name,
        &record.display_name,
        &object,
        &record.object_name,
        if record.offline { "1" } else { "0" },
        &record.text,
    ];
    format!("event: {kind}\nid: {id}\ndata: {}\n\n", data(&fields))
}

/// The status text as an event.
fn status_event(status: &str) -> String {
    format!("event: status\ndata: {}\n\n", data(&[status]))
}

impl Hub {
    pub fn new(start: u64) -> Hub {
        Hub {
            start,
            last: 0,
            chat: Ring::default(),
            im: Ring::default(),
            subscribers: Vec::new(),
            status: None,
        }
    }

    /// Sends `event` to every page; one that does not take it is dropped,
    /// which closes its stream.
    fn publish(&mut self, event: &Arc<str>) {
        self.subscribers.retain(|page| page.try_send(event.clone()).is_ok());
    }

    /// A message was recorded: it is kept, and sent to every page.
    pub fn record(&mut self, record: &Record) {
        self.last += 1;
        let id = format!("{}-{}", self.start, self.last);
        let event: Arc<str> = message_event(&id, record).into();
        match record.channel {
            Channel::Chat => self.chat.push(self.last, event.clone()),
            Channel::Im => self.im.push(self.last, event.clone()),
        }
        self.publish(&event);
    }

    /// The number a page goes on after, when `last` is the id of an event
    /// of this run and nothing after it was dropped.
    fn after(&self, last: Option<&str>) -> Option<u64> {
        let (start, number) = last?.split_once('-')?;
        let (start, number) = (start.parse::<u64>().ok()?, number.parse::<u64>().ok()?);
        let kept = number >= self.chat.dropped && number >= self.im.dropped;
        (start == self.start && number <= self.last && kept).then_some(number)
    }

    /// What a page that got `last` as its last event is sent first: the
    /// messages after it, or, when that cannot be told or some of them are
    /// gone, the word to start over and every kept message; then the
    /// status. All of it is one piece, so that it fits a page's queue
    /// however much it is.
    fn replay(&self, last: Option<&str>, status: &str) -> Arc<str> {
        let after = self.after(last);
        let mut text = String::new();
        if after.is_none() {
            text.push_str(RESET);
        }
        let after = after.unwrap_or(0);
        let (mut chat, mut im) = (self.chat.events.iter().peekable(), self.im.events.iter().peekable());
        // Both are in order; merged, they are in the order of recording.
        loop {
            let next = match (chat.peek(), im.peek()) {
                (Some((a, _)), Some((b, _))) if a < b => chat.next(),
                (Some(_), None) => chat.next(),
                (_, Some(_)) => im.next(),
                (None, None) => break,
            };
            if let Some((number, event)) = next {
                if *number > after {
                    text.push_str(event);
                }
            }
        }
        text.push_str(&status_event(status));
        text.into()
    }

    /// A page opens its stream: it gets what it missed and the status as
    /// it is now, and from then on every event.
    pub fn subscribe(&mut self, last: Option<&str>, page: SyncSender<Arc<str>>, status: &str) {
        // The pages that are open get this status too if they have not
        // had it, so that it is what everyone was sent last.
        match self.subscribers.is_empty() {
            true => self.status = Some(status.to_string()),
            false => self.status(status),
        }
        match page.try_send(self.replay(last, status)) {
            Ok(()) => self.subscribers.push(page),
            // A queue that is full before anything was sent, or a page
            // that is gone already.
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {}
        }
    }

    /// Whether anyone would see a status.
    pub fn watched(&self) -> bool {
        !self.subscribers.is_empty()
    }

    /// The status is `status` now: sent to every page when it is not what
    /// they were sent last.
    pub fn status(&mut self, status: &str) {
        if self.subscribers.is_empty() || self.status.as_deref() == Some(status) {
            return;
        }
        self.status = Some(status.to_string());
        let event: Arc<str> = status_event(status).into();
        self.publish(&event);
    }

    /// Closes every page's stream; for use on the way out.
    pub fn close(&mut self) {
        self.subscribers.clear();
    }

    #[cfg(test)]
    pub fn pages(&self) -> usize {
        self.subscribers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{sync_channel, Receiver};
    use uuid::Uuid;

    const START: u64 = 1_700_000_000;

    fn id(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn chat(text: &str) -> Record {
        Record {
            channel: Channel::Chat,
            time: START + 5,
            kind: "say".to_string(),
            avatar_name: "Jane Doe".to_string(),
            display_name: "Janey".to_string(),
            object_name: String::new(),
            object_uuid: None,
            text: text.to_string(),
            avatar_uuid: Some(id(1)),
            offline: false,
        }
    }

    fn im(text: &str) -> Record {
        Record {
            channel: Channel::Im,
            kind: "im".to_string(),
            ..chat(text)
        }
    }

    fn page() -> (SyncSender<Arc<str>>, Receiver<Arc<str>>) {
        sync_channel(QUEUE)
    }

    /// Everything the page has been sent, as one text.
    fn got(page: &Receiver<Arc<str>>) -> String {
        page.try_iter().map(|event| event.to_string()).collect()
    }

    /// The texts of the messages in what a page was sent, in order.
    fn texts(sent: &str) -> Vec<String> {
        let lines = sent
            .lines()
            .filter(|line| line.starts_with("data: ") && line.matches('\t').count() == 8);
        lines.map(|line| line.rsplit('\t').next().unwrap_or_default().to_string()).collect()
    }

    /// The id of the last message in what a page was sent.
    fn last_id(sent: &str) -> String {
        let mut ids = sent.lines().filter_map(|line| line.strip_prefix("id: "));
        ids.next_back().expect("an event with an id").to_string()
    }

    #[test]
    fn a_message_is_an_event_of_one_data_line() {
        let jane = id(1);
        assert_eq!(
            message_event("1700000000-7", &chat("hi all")),
            format!("event: chat\nid: 1700000000-7\ndata: 1700000005\tsay\t{jane}\tJane Doe\tJaney\t\t\t0\thi all\n\n")
        );
        let object = Record {
            kind: "object".to_string(),
            avatar_name: "Owner Resident".to_string(),
            display_name: String::new(),
            object_name: "Collar".to_string(),
            object_uuid: Some(id(9)),
            avatar_uuid: None,
            ..im("locked")
        };
        assert_eq!(
            message_event("1700000000-8", &object),
            format!(
                "event: im\nid: 1700000000-8\ndata: 1700000005\tobject\t\tOwner Resident\t\t{}\tCollar\t0\tlocked\n\n",
                id(9)
            )
        );
        let sent = Record {
            kind: "im_out".to_string(),
            ..im("see you")
        };
        assert!(message_event("1-1", &sent).contains(&format!("\tim_out\t{jane}\tJane Doe\t")));
        let stored = Record {
            offline: true,
            time: START - 3600,
            ..im("while you were out")
        };
        assert!(message_event("1-1", &stored).contains(&format!("data: {}\tim\t", START - 3600)));
        assert!(message_event("1-1", &stored).contains("\t1\twhile you were out\n"));
    }

    #[test]
    fn nothing_a_message_holds_breaks_its_line() {
        let odd = Record {
            avatar_name: "Tab\tby".to_string(),
            ..chat("one\ntwo\r\nthree \\ four\tfive")
        };
        let event = message_event("1-1", &odd);
        assert_eq!(event.lines().count(), 4, "{event}");
        assert!(event.ends_with("\tone\\ntwo\\r\\nthree \\\\ four\\tfive\n\n"), "{event}");
        assert!(event.contains("\tTab\\tby\t"), "{event}");
        assert_eq!(event.lines().nth(2).unwrap().matches('\t').count(), 8);
        // A status of several lines is one field.
        assert_eq!(
            status_event("connected: yes\nregion: Target\nmode: holding"),
            "event: status\ndata: connected: yes\\nregion: Target\\nmode: holding\n\n"
        );
    }

    #[test]
    fn a_page_opened_later_gets_what_was_kept_and_then_what_comes() {
        let mut hub = Hub::new(START);
        hub.record(&im("first"));
        hub.record(&chat("a line"));
        hub.record(&im("second"));
        let (sender, page) = page();
        hub.subscribe(None, sender, "connected: no");
        let sent = got(&page);
        assert!(sent.starts_with(RESET), "{sent}");
        assert_eq!(texts(&sent), ["first", "a line", "second"]);
        assert!(sent.ends_with("event: status\ndata: connected: no\n\n"), "{sent}");
        hub.record(&chat("live"));
        let sent = got(&page);
        assert_eq!(texts(&sent), ["live"]);
        assert_eq!(last_id(&sent), format!("{START}-4"));
        assert!(!sent.contains("reset"));
    }

    #[test]
    fn a_page_that_lost_its_stream_gets_what_it_missed_once() {
        let mut hub = Hub::new(START);
        let (sender, page) = page();
        hub.subscribe(None, sender, "s");
        hub.record(&chat("before"));
        let last = last_id(&got(&page));
        drop(page);
        for text in ["one", "two", "three"] {
            hub.record(&chat(text));
        }
        // The dead page is forgotten with the first event it does not take.
        assert_eq!(hub.pages(), 0);
        let (sender, page) = self::page();
        hub.subscribe(Some(&last), sender, "s");
        let sent = got(&page);
        assert!(!sent.contains("reset"), "{sent}");
        assert_eq!(texts(&sent), ["one", "two", "three"]);
        // Nothing missed: only the status.
        let (sender, again) = self::page();
        hub.subscribe(Some(&last_id(&sent)), sender, "s");
        assert_eq!(got(&again), "event: status\ndata: s\n\n");
    }

    #[test]
    fn each_channel_keeps_its_own_five_hundred() {
        let mut hub = Hub::new(START);
        hub.record(&im("im one"));
        for n in 0..2000 {
            hub.record(&chat(&format!("line {n}")));
            if n == 700 || n == 1400 {
                hub.record(&im(&format!("im at {n}")));
            }
        }
        let (sender, page) = page();
        hub.subscribe(None, sender, "s");
        let texts = texts(&got(&page));
        assert_eq!(texts.len(), KEPT + 3);
        // Chat cannot push instant messages out, and the order is that of
        // recording.
        assert_eq!(texts[0], "im one");
        assert_eq!(texts[1], "im at 700");
        assert_eq!(texts[2], "im at 1400");
        assert_eq!(texts[3], "line 1500");
        assert_eq!(texts[KEPT + 2], "line 1999");
    }

    #[test]
    fn a_gap_or_another_run_starts_the_page_over() {
        let mut hub = Hub::new(START);
        for n in 0..KEPT + 10 {
            hub.record(&chat(&format!("line {n}")));
        }
        let all = |hub: &mut Hub, last: Option<&str>| -> String {
            let (sender, page) = page();
            hub.subscribe(last, sender, "s");
            got(&page)
        };
        // The page's last event is one of those dropped: more was missed
        // than is kept.
        let sent = all(&mut hub, Some(&format!("{START}-5")));
        assert!(sent.starts_with(RESET));
        assert_eq!(texts(&sent).len(), KEPT);
        // The newest dropped one itself is fine: all after it are kept.
        let sent = all(&mut hub, Some(&format!("{START}-10")));
        assert!(!sent.contains("reset"));
        assert_eq!(texts(&sent).len(), KEPT);
        // An id of another run of the bot, of the future, or no id at all.
        for last in [
            format!("{}-3", START - 100),
            format!("{START}-99999"),
            "nonsense".to_string(),
            "-".to_string(),
            String::new(),
        ] {
            let sent = all(&mut hub, Some(&last));
            assert!(sent.starts_with(RESET), "{last}");
            assert_eq!(texts(&sent).len(), KEPT, "{last}");
        }
        // After a restart the page gets only what the new run recorded.
        let mut restarted = Hub::new(START + 60);
        restarted.record(&chat("since the restart"));
        let sent = all(&mut restarted, Some(&format!("{START}-400")));
        assert!(sent.starts_with(RESET));
        assert_eq!(texts(&sent), ["since the restart"]);
    }

    #[test]
    fn every_page_gets_every_event() {
        let mut hub = Hub::new(START);
        let (first, one) = page();
        let (second, two) = page();
        hub.subscribe(None, first, "s");
        hub.subscribe(None, second, "s");
        let _ = (got(&one), got(&two));
        hub.record(&im("hello"));
        assert_eq!(texts(&got(&one)), ["hello"]);
        assert_eq!(texts(&got(&two)), ["hello"]);
    }

    #[test]
    fn a_page_that_does_not_read_is_dropped_and_holds_nobody_up() {
        let mut hub = Hub::new(START);
        let (stalled, _kept_but_never_read) = page();
        let (reading, page) = self::page();
        hub.subscribe(None, stalled, "s");
        hub.subscribe(None, reading, "s");
        let _ = got(&page);
        // The replay is one piece, so 255 more events fill the queue.
        for n in 0..QUEUE + 20 {
            hub.record(&chat(&format!("line {n}")));
            assert_eq!(texts(&got(&page)), [format!("line {n}")]);
            assert_eq!(hub.pages(), if n < QUEUE - 1 { 2 } else { 1 }, "{n}");
        }
    }

    #[test]
    fn a_long_replay_fits_a_pages_queue() {
        let mut hub = Hub::new(START);
        for n in 0..2 * KEPT {
            hub.record(&chat(&format!("line {n}")));
            hub.record(&im(&format!("im {n}")));
        }
        let (sender, page) = page();
        hub.subscribe(None, sender, "s");
        assert_eq!(hub.pages(), 1);
        assert_eq!(texts(&got(&page)).len(), 2 * KEPT);
    }

    #[test]
    fn the_status_is_sent_when_it_changes_and_someone_watches() {
        let mut hub = Hub::new(START);
        assert!(!hub.watched());
        // Nobody watches: nothing is kept of it.
        hub.status("connected: no");
        let (sender, page) = page();
        hub.subscribe(None, sender, "connected: yes\nregion: Home");
        assert!(hub.watched());
        assert!(got(&page).ends_with("event: status\ndata: connected: yes\\nregion: Home\n\n"));
        // What the page was sent with its replay is not sent again.
        hub.status("connected: yes\nregion: Home");
        assert_eq!(got(&page), "");
        hub.status("connected: yes\nregion: Away");
        hub.status("connected: yes\nregion: Away");
        hub.status("connected: yes\nregion: Away");
        assert_eq!(got(&page), "event: status\ndata: connected: yes\\nregion: Away\n\n");
        // A page that opens meanwhile gets the status with its replay, and
        // is not sent it again.
        let (sender, second) = self::page();
        hub.subscribe(None, sender, "connected: yes\nregion: Away");
        let _ = got(&second);
        hub.status("connected: yes\nregion: Away");
        assert_eq!(got(&second), "");
        assert_eq!(got(&page), "");
        // One that opens with a status the others have not had brings it
        // to them.
        let (sender, third) = self::page();
        hub.subscribe(None, sender, "connected: no");
        assert_eq!(got(&page), "event: status\ndata: connected: no\n\n");
        assert_eq!(got(&second), "event: status\ndata: connected: no\n\n");
        assert!(got(&third).ends_with("event: status\ndata: connected: no\n\n"));
        hub.status("connected: no");
        assert_eq!(got(&page) + &got(&second) + &got(&third), "");
        hub.close();
        assert!(!hub.watched());
        assert!(second.recv().is_err());
    }
}
