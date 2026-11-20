//! Messages for the owners that wait while the component is not connected.

use super::stanza::chat_message;
use std::collections::VecDeque;

/// The most messages that wait; older ones are dropped.
pub const MAX_WAITING: usize = 500;

/// Something for the owners.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    /// A chat message from `from` (a full address of the component) to
    /// `to`, or to every owner when `to` is `None`.
    Chat {
        from: String,
        to: Option<String>,
        body: String,
        nick: Option<String>,
        /// When it was recorded, or for an offline IM sent, in seconds
        /// since the Unix epoch.
        time: u64,
        /// An offline IM: it carries its time even when it goes out at once.
        offline: bool,
    },
    /// Any other stanza, as it is.
    Raw(String),
}

impl Outgoing {
    /// The stanzas to write: one per owner for a message to them all.
    /// `waited` says whether it waited, and so carries its time.
    pub fn stanzas(&self, owners: &[String], waited: bool) -> Vec<String> {
        match self {
            Outgoing::Chat {
                from,
                to,
                body,
                nick,
                time,
                offline,
            } => {
                let delay = (waited || *offline).then_some(*time);
                let one = |to: &str| chat_message(from, to, body, nick.as_deref(), delay);
                match to {
                    Some(to) => vec![one(to)],
                    None => owners.iter().map(|owner| one(owner)).collect(),
                }
            }
            Outgoing::Raw(stanza) => vec![stanza.clone()],
        }
    }
}

/// The waiting messages, oldest first, at most `MAX_WAITING` of them.
#[derive(Default)]
pub struct WaitQueue {
    waiting: VecDeque<Outgoing>,
    /// How many were dropped since the last `take_dropped`.
    dropped: usize,
}

impl WaitQueue {
    pub fn push(&mut self, outgoing: Outgoing) {
        if self.waiting.len() >= MAX_WAITING {
            self.waiting.pop_front();
            self.dropped += 1;
        }
        self.waiting.push_back(outgoing);
    }

    /// Puts back a message taken out but not sent, as the oldest.
    pub fn push_front(&mut self, outgoing: Outgoing) {
        if self.waiting.len() >= MAX_WAITING {
            self.dropped += 1;
            return;
        }
        self.waiting.push_front(outgoing);
    }

    pub fn pop(&mut self) -> Option<Outgoing> {
        self.waiting.pop_front()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    /// How many were dropped since this was last asked.
    pub fn take_dropped(&mut self) -> usize {
        std::mem::take(&mut self.dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_795_021_463; // 2026-11-18T17:04:23Z

    fn line(n: usize) -> Outgoing {
        Outgoing::Chat {
            from: "local@bot.rig.test".into(),
            to: None,
            body: format!("line {n}"),
            nick: None,
            time: T + n as u64,
            offline: false,
        }
    }

    fn owners() -> Vec<String> {
        vec!["owner@rig.test".into(), "other@rig.test".into()]
    }

    #[test]
    fn xmpp_server_restart_during_chat() {
        let mut queue = WaitQueue::default();
        let im = |text: &str, time| Outgoing::Chat {
            from: "jane.doe@bot.rig.test".into(),
            to: None,
            body: text.into(),
            nick: Some("Janey".into()),
            time,
            offline: false,
        };
        queue.push(im("one", T));
        queue.push(im("two", T + 30));
        let mut sent = Vec::new();
        while let Some(outgoing) = queue.pop() {
            sent.extend(outgoing.stanzas(&owners(), true));
        }
        assert_eq!(sent.len(), 4);
        assert!(sent[0].contains("to='owner@rig.test'") && sent[0].contains("<body>one</body>"));
        assert!(sent[0].contains("stamp='2026-11-18T17:04:23Z'"));
        assert!(sent[1].contains("to='other@rig.test'") && sent[1].contains("<body>one</body>"));
        assert!(sent[2].contains("<body>two</body>") && sent[2].contains("stamp='2026-11-18T17:04:53Z'"));
        assert_eq!(queue.take_dropped(), 0);
    }

    #[test]
    fn long_outage() {
        let mut queue = WaitQueue::default();
        for n in 0..600 {
            queue.push(line(n));
        }
        assert_eq!(queue.len(), 500);
        assert_eq!(queue.take_dropped(), 100);
        assert_eq!(queue.take_dropped(), 0, "said once");
        assert_eq!(queue.pop(), Some(line(100)));
        let rest: Vec<Outgoing> = std::iter::from_fn(|| queue.pop()).collect();
        assert_eq!(rest.last(), Some(&line(599)));
        assert_eq!(rest.len(), 499);
    }

    #[test]
    fn live_messages_carry_no_time_but_offline_ims_do() {
        let live = line(0).stanzas(&owners(), false);
        assert!(!live[0].contains("<delay"));
        let mut offline = line(0);
        if let Outgoing::Chat { offline: stored, .. } = &mut offline {
            *stored = true;
        }
        assert!(offline.stanzas(&owners(), false)[0].contains("stamp='2026-11-18T17:04:23Z'"));
        let answer = Outgoing::Chat {
            from: "bot.rig.test".into(),
            to: Some("owner@rig.test/phone".into()),
            body: "ok".into(),
            nick: None,
            time: T,
            offline: false,
        };
        assert_eq!(
            answer.stanzas(&owners(), false),
            ["<message type='chat' from='bot.rig.test' to='owner@rig.test/phone'><body>ok</body></message>"]
        );
        assert_eq!(Outgoing::Raw("<x/>".into()).stanzas(&owners(), true), ["<x/>"]);
    }

    #[test]
    fn a_message_put_back_goes_first() {
        let mut queue = WaitQueue::default();
        queue.push(line(1));
        queue.push_front(line(0));
        assert_eq!(queue.pop(), Some(line(0)));
    }
}
