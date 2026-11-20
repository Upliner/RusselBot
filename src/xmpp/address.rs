//! Contact addresses: the local parts made from avatars' names (with the
//! escapes of XEP-0106) and objects' UUIDs, the addresses a stanza is sent
//! to and from, the owners, and the UUIDs the bot has used addresses for.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The local part of the local chat contact.
pub const LOCAL: &str = "local";
/// The local parts of objects start with this.
pub const OBJECT_PREFIX: &str = "obj-";

/// The characters XEP-0106 escapes, with their escapes.
const ESCAPES: [(char, &str); 10] = [
    (' ', "\\20"),
    ('"', "\\22"),
    ('&', "\\26"),
    ('\'', "\\27"),
    ('/', "\\2f"),
    (':', "\\3a"),
    ('<', "\\3c"),
    ('>', "\\3e"),
    ('@', "\\40"),
    ('\\', "\\5c"),
];

/// Whether `rest` starts with one of the escapes.
fn starts_with_escape(rest: &str) -> bool {
    ESCAPES
        .iter()
        .any(|(_, escape)| rest.get(..3).is_some_and(|start| start.eq_ignore_ascii_case(escape)))
}

/// XEP-0106: the ten characters an address's local part cannot hold, as
/// escapes. A backslash is escaped only where it would read as an escape.
pub fn escape_local(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, ch) in text.char_indices() {
        match ESCAPES.iter().find(|(escaped, _)| *escaped == ch) {
            Some(('\\', escape)) if starts_with_escape(&text[index..]) => out.push_str(escape),
            Some(('\\', _)) => out.push('\\'),
            Some((_, escape)) => out.push_str(escape),
            None => out.push(ch),
        }
    }
    out
}

/// Undoes `escape_local`.
pub fn unescape_local(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        let unescaped = (ch == '\\')
            .then(|| {
                ESCAPES
                    .iter()
                    .find(|(_, escape)| rest.get(..3).is_some_and(|s| s.eq_ignore_ascii_case(escape)))
            })
            .flatten();
        match unescaped {
            Some((original, _)) => {
                out.push(*original);
                rest = &rest[3..];
            }
            None => {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

/// The local part of an avatar's address: `first`, or `first.last` when
/// the last name is not `Resident`, from its legacy name `First Last`, in
/// lower case and escaped; its UUID when the name is not known or would
/// give the local chat contact, a UUID or an object's address.
pub fn avatar_local(legacy: &str, id: Uuid) -> String {
    let fallback = || id.hyphenated().to_string();
    let legacy = legacy.trim();
    if legacy.is_empty() {
        return fallback();
    }
    let lower = legacy.to_lowercase();
    let name = match lower.split_once(' ') {
        Some((first, last)) if last.trim() == "resident" => first.to_string(),
        Some((first, last)) => format!("{first}.{}", last.trim()),
        None => lower.clone(),
    };
    let local = escape_local(&name);
    if local == LOCAL || local.starts_with(OBJECT_PREFIX) || Uuid::parse_str(&local).is_ok() || local.len() > 1023 {
        return fallback();
    }
    local
}

pub fn object_local(id: Uuid) -> String {
    format!("{OBJECT_PREFIX}{}", id.hyphenated())
}

/// Who a stanza is addressed to, within the component's domain.
#[derive(Debug, Clone, PartialEq)]
pub enum Contact {
    /// The bare domain: commands.
    Control,
    /// `local@`: local chat.
    Local,
    /// An object's address.
    Object,
    /// Any other local part: an avatar.
    Avatar(String),
}

/// The contact `to` names, when it is an address of `domain`; the resource
/// does not matter.
pub fn contact(to: &str, domain: &str) -> Option<Contact> {
    let bare = to.split_once('/').map_or(to, |(bare, _)| bare);
    let (local, host) = match bare.split_once('@') {
        Some((local, host)) => (Some(local), host),
        None => (None, bare),
    };
    if !host.eq_ignore_ascii_case(domain) {
        return None;
    }
    Some(match local {
        None => Contact::Control,
        Some(local) => {
            let local = local.to_lowercase();
            match local.as_str() {
                "" => return None,
                LOCAL => Contact::Local,
                _ if local.starts_with(OBJECT_PREFIX) => Contact::Object,
                _ => Contact::Avatar(local),
            }
        }
    })
}

/// The bare address of a stanza's `from`, in lower case: `None` when it is
/// missing or not `user@domain[/resource]`.
pub fn sender(from: Option<&str>) -> Option<String> {
    let from = from?;
    let bare = from.split_once('/').map_or(from, |(bare, _)| bare);
    let (user, domain) = bare.split_once('@')?;
    if user.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }
    Some(bare.to_lowercase())
}

/// Whether a stanza from `from` comes from one of `owners` (bare, lower
/// case).
pub fn is_owner(owners: &[String], from: Option<&str>) -> bool {
    sender(from).is_some_and(|bare| owners.contains(&bare))
}

/// The address of an avatar's contact, as the owner's message to it means
/// it.
#[derive(Debug, Clone, PartialEq)]
pub enum Recipient {
    /// The avatar of this UUID.
    Id(Uuid),
    /// The avatar the grid finds for this name, as `im` takes it.
    Name(String),
}

/// How long an address the bot has used is kept once the book is large.
const BOOK_LIFETIME: Duration = Duration::from_secs(3600);
/// Old addresses are dropped once the book has grown this large.
const BOOK_PRUNE_SIZE: usize = 10_000;

/// The UUIDs behind the avatar addresses the bot has used, while it runs.
#[derive(Default)]
pub struct AddressBook {
    ids: HashMap<String, (Uuid, Instant)>,
}

impl AddressBook {
    pub fn learn(&mut self, local: &str, id: Uuid, now: Instant) {
        if id.is_nil() {
            return;
        }
        if self.ids.len() >= BOOK_PRUNE_SIZE {
            self.ids.retain(|_, (_, at)| now.duration_since(*at) < BOOK_LIFETIME);
        }
        self.ids.insert(local.to_lowercase(), (id, now));
    }

    /// Who an avatar address's local part stands for: the UUID it is, the
    /// UUID it was used for, or else the name it is made of.
    pub fn recipient(&self, local: &str) -> Recipient {
        let local = local.to_lowercase();
        if let Ok(id) = Uuid::parse_str(&local) {
            return Recipient::Id(id);
        }
        if let Some((id, _)) = self.ids.get(&local) {
            return Recipient::Id(*id);
        }
        Recipient::Name(unescape_local(&local))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.ids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    #[test]
    fn xep_0106_escapes() {
        assert_eq!(escape_local("ann.lee.@osgrid.org:8002"), "ann.lee.\\40osgrid.org\\3a8002");
        assert_eq!(
            escape_local("a b\"c&d'e/f:g<h>i@j"),
            "a\\20b\\22c\\26d\\27e\\2ff\\3ag\\3ch\\3ei\\40j"
        );
        // A backslash is escaped only where it would read as an escape.
        assert_eq!(escape_local("a\\b"), "a\\b");
        assert_eq!(escape_local("a\\20b"), "a\\5c20b");
        for text in ["ann.lee.@osgrid.org:8002", "a b\"c&d'e/f:g<h>i@j", "a\\b", "a\\20b", "\\5c", "ünï"] {
            assert_eq!(unescape_local(&escape_local(text)), text);
        }
        assert_eq!(unescape_local("ann.lee.\\40OSGRID.org\\3A8002"), "ann.lee.@OSGRID.org:8002");
        assert_eq!(unescape_local("trailing\\"), "trailing\\");
    }

    #[test]
    fn ordinary_name() {
        assert_eq!(avatar_local("Jane Doe", id(1)), "jane.doe");
    }

    #[test]
    fn resident() {
        assert_eq!(avatar_local("bob Resident", id(1)), "bob");
        assert_eq!(avatar_local("Bob", id(1)), "bob");
    }

    #[test]
    fn hypergrid_visitor() {
        assert_eq!(avatar_local("Ann.Lee @osgrid.org:8002", id(1)), "ann.lee.\\40osgrid.org\\3a8002");
    }

    #[test]
    fn name_not_known() {
        assert_eq!(avatar_local("", id(0xAB)), id(0xAB).to_string());
        assert_eq!(avatar_local("  ", id(0xAB)), id(0xAB).to_string());
    }

    #[test]
    fn names_that_would_be_another_contact() {
        assert_eq!(avatar_local("local Resident", id(2)), id(2).to_string());
        assert_eq!(avatar_local("Local", id(2)), id(2).to_string());
        assert_eq!(avatar_local("obj-x Resident", id(2)), id(2).to_string());
        assert_eq!(avatar_local("Obj-1 Smith", id(2)), id(2).to_string());
        assert_eq!(avatar_local(&format!("{} Resident", id(9)), id(2)), id(2).to_string());
        // Only exactly `local` is the chat contact.
        assert_eq!(avatar_local("local Doe", id(2)), "local.doe");
    }

    #[test]
    fn objects() {
        assert_eq!(object_local(id(0xC0)), format!("obj-{}", id(0xC0)));
    }

    #[test]
    fn contacts_of_the_domain() {
        let domain = "russel.sl.example.org";
        assert_eq!(contact("russel.sl.example.org", domain), Some(Contact::Control));
        assert_eq!(contact("Russel.SL.example.org/x", domain), Some(Contact::Control));
        assert_eq!(contact("local@russel.sl.example.org", domain), Some(Contact::Local));
        assert_eq!(contact("LOCAL@russel.sl.example.org/r", domain), Some(Contact::Local));
        assert_eq!(
            contact(&format!("obj-{}@russel.sl.example.org", id(3)), domain),
            Some(Contact::Object)
        );
        assert_eq!(
            contact("Jane.Doe@russel.sl.example.org", domain),
            Some(Contact::Avatar("jane.doe".into()))
        );
        assert_eq!(contact("jane.doe@other.example.org", domain), None);
        assert_eq!(contact("@russel.sl.example.org", domain), None);
    }

    #[test]
    fn owner_from_any_device() {
        let owners = vec!["me@example.org".to_string()];
        assert!(is_owner(&owners, Some("Me@Example.org/phone")));
        assert!(is_owner(&owners, Some("me@example.org")));
        assert!(is_owner(&owners, Some("me@example.org/a/b")));
    }

    #[test]
    fn strangers_and_malformed_senders_are_not_owners() {
        let owners = vec!["me@example.org".to_string()];
        assert!(!is_owner(&owners, Some("someone@elsewhere.org/x")));
        assert!(!is_owner(&owners, Some("me@example.org.evil")));
        assert!(!is_owner(&owners, Some("example.org")));
        assert!(!is_owner(&owners, Some("@example.org")));
        assert!(!is_owner(&owners, Some("me@")));
        assert!(!is_owner(&owners, Some("me@x@example.org")));
        assert!(!is_owner(&owners, Some("")));
        assert!(!is_owner(&owners, None));
        assert_eq!(sender(Some("Me@Example.org/phone")), Some("me@example.org".into()));
    }

    #[test]
    fn replies_go_to_the_uuid_the_address_stands_for() {
        let now = Instant::now();
        let mut book = AddressBook::default();
        // 1. A UUID address.
        assert_eq!(book.recipient(&id(5).to_string()), Recipient::Id(id(5)));
        // 3. An address not used yet stands for its name.
        assert_eq!(book.recipient("jane.doe"), Recipient::Name("jane.doe".into()));
        assert_eq!(
            book.recipient("ann.lee.\\40osgrid.org\\3a8002"),
            Recipient::Name("ann.lee.@osgrid.org:8002".into())
        );
        // 2. Once used, the UUID behind it, whatever the case.
        book.learn("ann.lee.\\40osgrid.org\\3a8002", id(6), now);
        book.learn("jane.doe", id(7), now);
        assert_eq!(book.recipient("Ann.Lee.\\40osgrid.org\\3a8002"), Recipient::Id(id(6)));
        assert_eq!(book.recipient("jane.doe"), Recipient::Id(id(7)));
        // The newest use wins.
        book.learn("jane.doe", id(8), now);
        assert_eq!(book.recipient("jane.doe"), Recipient::Id(id(8)));
        book.learn("nobody", Uuid::nil(), now);
        assert_eq!(book.recipient("nobody"), Recipient::Name("nobody".into()));
    }

    #[test]
    fn the_book_is_pruned_of_old_addresses() {
        let start = Instant::now();
        let mut book = AddressBook::default();
        for n in 0..BOOK_PRUNE_SIZE {
            book.learn(&format!("a{n}"), Uuid::from_u128(n as u128 + 1), start);
        }
        book.learn("fresh", id(1), start + BOOK_LIFETIME / 2);
        assert_eq!(book.len(), BOOK_PRUNE_SIZE + 1);
        book.learn("later", id(2), start + BOOK_LIFETIME + Duration::from_secs(1));
        assert_eq!(book.len(), 2);
        assert_eq!(book.recipient("fresh"), Recipient::Id(id(1)));
    }
}
