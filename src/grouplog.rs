//! The record of a group message — a line of a group's chat, received or
//! said by the bot, or a group notice — with its row in the group file and
//! its line on standard error, and the reading of a notice.

use crate::escape::escape;
use crate::log::format_timestamp;
use uuid::Uuid;

/// The columns of the group file.
pub const COLUMNS: [&str; 9] = [
    "time",
    "kind",
    "group_name",
    "group_uuid",
    "avatar_name",
    "display_name",
    "subject",
    "attachment",
    "text",
];

/// What a group message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A line of a group's chat the avatar received.
    Chat,
    /// A line the bot said in a group's chat.
    ChatOut,
    /// A group notice.
    Notice,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Chat => "chat",
            Kind::ChatOut => "chat_out",
            Kind::Notice => "notice",
        }
    }
}

/// A group message with its names settled, ready to be written.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMessage {
    /// Seconds since the Unix epoch: when it arrived or was sent, or for a
    /// stored notice when it was sent.
    pub time: u64,
    pub kind: Kind,
    pub group: Uuid,
    pub group_name: String,
    /// Who said or sent it, when the message gives the UUID.
    pub avatar: Option<Uuid>,
    pub avatar_name: String,
    pub display_name: String,
    /// A notice's subject; empty for chat.
    pub subject: String,
    /// The name of the item attached to a notice; empty for none.
    pub attachment: String,
    pub text: String,
    /// A notice stored while the avatar was offline.
    pub offline: bool,
}

fn line(fields: &[&str]) -> String {
    fields.iter().map(|field| escape(field)).collect::<Vec<_>>().join("\t") + "\n"
}

/// The header row of the group file.
pub fn header() -> String {
    line(&COLUMNS)
}

impl GroupMessage {
    /// The message as one row of the group file, with its line ending.
    pub fn row(&self) -> String {
        line(&[
            &format_timestamp(self.time),
            self.kind.name(),
            &self.group_name,
            &self.group.to_string(),
            &self.avatar_name,
            &self.display_name,
            &self.subject,
            &self.attachment,
            &self.text,
        ])
    }

    /// The message as a standard-error log entry (without its timestamp),
    /// on one line.
    pub fn log_line(&self) -> String {
        let group = match self.group_name.is_empty() {
            true => self.group.to_string(),
            false => self.group_name.clone(),
        };
        let mut who = self.avatar_name.clone();
        if !self.display_name.is_empty() {
            who = format!("{who} ({})", self.display_name).trim_start().to_string();
        }
        let mut about = Vec::new();
        if !who.is_empty() {
            about.push(who);
        }
        if !self.subject.is_empty() {
            about.push(format!("subject \"{}\"", self.subject));
        }
        if !self.attachment.is_empty() {
            about.push(format!("attachment \"{}\"", self.attachment));
        }
        let stored = if self.offline { " (stored)" } else { "" };
        escape(&format!(
            "group [{}] {group}{stored}: {}: {}",
            self.kind.name(),
            about.join(", "),
            self.text
        ))
    }
}

/// What a notice says, read from its text and its binary bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub subject: String,
    pub text: String,
    /// The name of the attached item; empty for none.
    pub attachment: String,
}

/// Reads a notice: its text is `subject|body`, or the body alone; its
/// bucket is whether something is attached, its asset type, the group's
/// UUID, and the attached item's name, which ends with a NUL.
pub fn notice(text: &str, bucket: &[u8]) -> Notice {
    let (subject, text) = match text.split_once('|') {
        Some((subject, body)) => (subject.to_string(), body.to_string()),
        None => (String::new(), text.to_string()),
    };
    let attachment = match bucket {
        [has, _asset_type, rest @ ..] if *has != 0 && rest.len() > 16 => {
            let name = &rest[16..];
            let end = name.iter().position(|byte| *byte == 0).unwrap_or(name.len());
            String::from_utf8_lossy(&name[..end]).into_owned()
        }
        _ => String::new(),
    };
    Notice { subject, text, attachment }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 2026-12-01T12:00:00Z.
    pub(crate) const T0: u64 = 1_796_126_400;

    pub(crate) fn builders() -> Uuid {
        Uuid::from_bytes([0xB1; 16])
    }

    fn jane() -> Uuid {
        Uuid::from_bytes([0x33; 16])
    }

    pub(crate) fn chat(text: &str) -> GroupMessage {
        GroupMessage {
            time: T0,
            kind: Kind::Chat,
            group: builders(),
            group_name: "Builders".to_string(),
            avatar: Some(jane()),
            avatar_name: "Jane Doe".to_string(),
            display_name: "JD".to_string(),
            subject: String::new(),
            attachment: String::new(),
            text: text.to_string(),
            offline: false,
        }
    }

    /// A notice's bucket: attached or not, the group, the item's name.
    pub(crate) fn bucket(item: Option<&str>) -> Vec<u8> {
        let mut bucket = vec![u8::from(item.is_some()), if item.is_some() { 7 } else { 0 }];
        bucket.extend(builders().as_bytes());
        bucket.extend(item.unwrap_or_default().as_bytes());
        bucket.push(0);
        bucket
    }

    #[test]
    fn a_notice_has_a_subject_and_a_text() {
        assert_eq!(
            notice("Meeting|Tonight at eight", &bucket(None)),
            Notice {
                subject: "Meeting".to_string(),
                text: "Tonight at eight".to_string(),
                attachment: String::new(),
            }
        );
    }

    #[test]
    fn a_notice_without_a_subject_keeps_its_whole_text() {
        let read = notice("Tonight at eight", &bucket(None));
        assert_eq!((read.subject.as_str(), read.text.as_str()), ("", "Tonight at eight"));
        // A bar in the body stays in it.
        assert_eq!(notice("Re|a|b", &[]).text, "a|b");
    }

    #[test]
    fn a_notice_names_its_attachment() {
        assert_eq!(notice("Meeting|Tonight", &bucket(Some("Agenda"))).attachment, "Agenda");
        // A bucket too short to name an item names none.
        assert_eq!(notice("Meeting|Tonight", &[1, 7, 0]).attachment, "");
        assert_eq!(notice("Meeting|Tonight", &[]).attachment, "");
    }

    #[test]
    fn a_chat_row_has_nine_fields() {
        assert_eq!(
            chat("hello").row(),
            format!("2026-12-01T12:00:00Z\tchat\tBuilders\t{}\tJane Doe\tJD\t\t\thello\n", builders())
        );
        assert_eq!(
            header(),
            "time\tkind\tgroup_name\tgroup_uuid\tavatar_name\tdisplay_name\tsubject\tattachment\ttext\n"
        );
    }

    #[test]
    fn the_readme_shows_the_header_row() {
        let readme = include_str!("../README.md");
        assert!(readme.contains(&format!("```\n{}```", header())), "the README's header row differs");
    }

    #[test]
    fn a_notice_row_is_one_line() {
        let notice = GroupMessage {
            kind: Kind::Notice,
            display_name: String::new(),
            subject: "Meeting".to_string(),
            attachment: "Agenda".to_string(),
            ..chat("Tonight\nat eight")
        };
        let row = notice.row();
        assert_eq!(row.lines().count(), 1);
        assert!(row.contains("\tnotice\t"), "{row}");
        assert!(row.ends_with("\tJane Doe\t\tMeeting\tAgenda\tTonight\\nat eight\n"), "{row}");
    }

    #[test]
    fn the_log_line_names_the_group_the_speaker_and_the_text() {
        let line = chat("hello").log_line();
        assert_eq!(line, "group [chat] Builders: Jane Doe (JD): hello");
        let notice = GroupMessage {
            kind: Kind::Notice,
            subject: "Meeting".to_string(),
            attachment: "Agenda".to_string(),
            offline: true,
            ..chat("One\nTwo\nThree\nFour")
        };
        let line = notice.log_line();
        assert!(!line.contains('\n'));
        assert!(line.contains("Builders") && line.contains("Meeting") && line.contains("Agenda") && line.contains("(stored)"));
        assert!(line.ends_with("One\\nTwo\\nThree\\nFour"), "{line}");
    }
}
