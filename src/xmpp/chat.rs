//! What the owners receive for a recorded message: the local chat line,
//! and the contact and nickname of an instant message.

use super::address::{avatar_local, object_local};
use crate::msglog::{Channel, Record};

/// The speaker as the local chat contact names it: the display name with
/// the legacy name in parentheses when they differ, else either one, and
/// the object's name for an object.
pub fn speaker(record: &Record) -> String {
    if !record.object_name.is_empty() || record.object_uuid.is_some() {
        return match record.object_name.trim() {
            "" => "(object)".to_string(),
            name => name.to_string(),
        };
    }
    let (legacy, display) = (record.avatar_name.trim(), record.display_name.trim());
    match (legacy, display) {
        ("", "") => record.avatar_uuid.map_or_else(|| "(someone)".to_string(), |id| id.to_string()),
        (legacy, "") => legacy.to_string(),
        ("", display) => display.to_string(),
        (legacy, display) if same_name(legacy, display) => legacy.to_string(),
        (legacy, display) => format!("{display} ({legacy})"),
    }
}

/// Whether a display name says no more than the legacy name: the same, or
/// the first name of a `Resident`, in any case.
fn same_name(legacy: &str, display: &str) -> bool {
    let first = legacy.strip_suffix(" Resident").unwrap_or(legacy);
    display.eq_ignore_ascii_case(legacy) || display.eq_ignore_ascii_case(first)
}

/// A line of local chat as the owners read it.
pub fn chat_line(record: &Record) -> String {
    let name = speaker(record);
    let text = &record.text;
    if let Some(rest) = text.strip_prefix("/me") {
        if rest.starts_with(' ') || rest.starts_with('\'') {
            return format!("{name}{rest}");
        }
    }
    let how = match record.kind.as_str() {
        "say" => String::new(),
        "whisper" => " whispers".to_string(),
        "shout" => " shouts".to_string(),
        kind => format!(" [{kind}]"),
    };
    format!("{name}{how}: {text}")
}

/// Where a recorded message goes to the owners.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    /// The local part of the contact it comes from; `None` for the bare
    /// domain.
    pub from: Option<String>,
    pub body: String,
    pub nick: Option<String>,
}

/// The delivery of a record, if it is one the owners receive: local chat,
/// and instant messages received (not `im_out`).
pub fn delivery(record: &Record) -> Option<Delivery> {
    match record.channel {
        Channel::Chat => Some(Delivery {
            from: Some(super::address::LOCAL.to_string()),
            body: chat_line(record),
            nick: None,
        }),
        Channel::Im if record.kind == "im_out" => None,
        Channel::Im => {
            let from_object = !record.object_name.is_empty() || record.object_uuid.is_some();
            Some(match (from_object, record.object_uuid, record.avatar_uuid) {
                (true, Some(object), _) => Delivery {
                    from: Some(object_local(object)),
                    body: record.text.clone(),
                    nick: Some(record.object_name.clone()).filter(|name| !name.is_empty()),
                },
                (false, _, Some(avatar)) => {
                    let nick = match record.display_name.trim() {
                        "" => record.avatar_name.trim(),
                        display => display,
                    };
                    Delivery {
                        from: Some(avatar_local(&record.avatar_name, avatar)),
                        body: record.text.clone(),
                        nick: Some(nick.to_string()).filter(|nick| !nick.is_empty()),
                    }
                }
                // An object the simulator did not name by UUID, and the
                // grid itself: from the control contact, with the name.
                _ => Delivery {
                    from: None,
                    body: format!("{}: {}", speaker(record), record.text),
                    nick: None,
                },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn heard(kind: &str, legacy: &str, display: &str, text: &str) -> Record {
        Record {
            channel: Channel::Chat,
            time: 0,
            kind: kind.into(),
            avatar_name: legacy.into(),
            display_name: display.into(),
            object_name: String::new(),
            object_uuid: None,
            text: text.into(),
            avatar_uuid: Some(id(1)),
            offline: false,
        }
    }

    fn object(kind: &str, name: &str, text: &str) -> Record {
        Record {
            object_name: name.into(),
            object_uuid: Some(id(0xC0)),
            avatar_uuid: None,
            ..heard(kind, "Jane Doe", "Janey", text)
        }
    }

    #[test]
    fn chat_heard() {
        assert_eq!(chat_line(&heard("say", "Jane Doe", "Janey", "hi all")), "Janey (Jane Doe): hi all");
    }

    #[test]
    fn names_known_and_not() {
        assert_eq!(chat_line(&heard("say", "Jane Doe", "", "x")), "Jane Doe: x");
        assert_eq!(chat_line(&heard("say", "Jane Doe", "Jane Doe", "x")), "Jane Doe: x");
        assert_eq!(chat_line(&heard("say", "bob Resident", "bob", "x")), "bob Resident: x");
        assert_eq!(chat_line(&heard("say", "bob Resident", "Bob", "x")), "bob Resident: x");
        assert_eq!(chat_line(&heard("say", "", "Janey", "x")), "Janey: x");
        assert_eq!(chat_line(&heard("say", "", "", "x")), format!("{}: x", id(1)));
    }

    #[test]
    fn shout_and_emote() {
        assert_eq!(chat_line(&heard("shout", "bob Resident", "", "HEY")), "bob Resident shouts: HEY");
        assert_eq!(chat_line(&heard("say", "bob Resident", "", "/me waves")), "bob Resident waves");
        assert_eq!(chat_line(&heard("say", "bob Resident", "", "/me's hat")), "bob Resident's hat");
        assert_eq!(
            chat_line(&heard("whisper", "bob Resident", "", "psst")),
            "bob Resident whispers: psst"
        );
        // Only `/me ` and `/me'` are emotes.
        assert_eq!(chat_line(&heard("say", "bob Resident", "", "/meow")), "bob Resident: /meow");
    }

    #[test]
    fn object_speaking_to_its_owner() {
        assert_eq!(chat_line(&object("owner", "Collar", "ready")), "Collar [owner]: ready");
        assert_eq!(chat_line(&object("say", "Collar", "/me clicks")), "Collar clicks");
    }

    #[test]
    fn chat_goes_to_local() {
        let delivery = delivery(&heard("say", "Jane Doe", "Janey", "hi all")).unwrap();
        assert_eq!(delivery.from.as_deref(), Some("local"));
        assert_eq!(delivery.body, "Janey (Jane Doe): hi all");
        assert_eq!(delivery.nick, None);
    }

    #[test]
    fn ims_go_to_their_contacts() {
        let im = Record {
            channel: Channel::Im,
            kind: "im".into(),
            ..heard("", "Jane Doe", "Janey", "hello")
        };
        assert_eq!(
            delivery(&im),
            Some(Delivery {
                from: Some("jane.doe".into()),
                body: "hello".into(),
                nick: Some("Janey".into())
            })
        );
        let plain = Record {
            display_name: String::new(),
            ..im.clone()
        };
        assert_eq!(delivery(&plain).unwrap().nick.as_deref(), Some("Jane Doe"));
        let unknown = Record {
            avatar_name: String::new(),
            display_name: String::new(),
            ..im.clone()
        };
        assert_eq!(delivery(&unknown).unwrap().from, Some(id(1).to_string()));
        let sent = Record {
            kind: "im_out".into(),
            ..im
        };
        assert_eq!(delivery(&sent), None);
    }

    #[test]
    fn object_ims() {
        let collar = Record {
            channel: Channel::Im,
            ..object("object", "Collar", "locked")
        };
        assert_eq!(
            delivery(&collar),
            Some(Delivery {
                from: Some(format!("obj-{}", id(0xC0))),
                body: "locked".into(),
                nick: Some("Collar".into())
            })
        );
        let unnamed = Record {
            object_uuid: None,
            ..collar
        };
        assert_eq!(
            delivery(&unnamed),
            Some(Delivery {
                from: None,
                body: "Collar: locked".into(),
                nick: None
            })
        );
        let grid = Record {
            channel: Channel::Im,
            kind: "im".into(),
            avatar_uuid: None,
            ..heard("", "Second Life", "", "notice")
        };
        assert_eq!(delivery(&grid).unwrap().from, None);
        assert_eq!(delivery(&grid).unwrap().body, "Second Life: notice");
    }
}
