//! What avatars and scripted objects ask of the avatar and a viewer answers
//! with a dialog: teleport offers, inventory offers and script permission
//! requests. Here are the kinds of request, the rules that say which ones
//! the bot accepts by itself, who a rule's entry stands for, and the
//! requests that wait for the operator. Nothing here talks to the grid.

use crate::inventory;
use crate::names::full_name;
use std::fmt;
use uuid::Uuid;

/// How many waiting requests of one sort are kept.
pub const MAX_WAITING: usize = 100;

/// What a request asks for, as the rules and the commands name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Tp,
    Inventory,
    Anim,
    Attach,
    Controls,
    Camera,
    Group,
}

/// The kinds, as an error lists them.
pub const KINDS: &str = "`tp`, `inventory`, `anim`, `attach`, `controls`, `camera` and `group`";

impl Kind {
    /// In the order `perms status` prints them.
    pub const ALL: [Kind; 7] = [
        Kind::Tp,
        Kind::Inventory,
        Kind::Anim,
        Kind::Attach,
        Kind::Controls,
        Kind::Camera,
        Kind::Group,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Tp => "tp",
            Kind::Inventory => "inventory",
            Kind::Anim => "anim",
            Kind::Attach => "attach",
            Kind::Controls => "controls",
            Kind::Camera => "camera",
            Kind::Group => "group",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == text)
    }

    fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The script permissions the bot may grant (`PERMISSION_*` in LSL), each
/// with its kind and what it lets a script do.
const PERMISSIONS: [(i32, Kind, &str); 6] = [
    (0x4, Kind::Controls, "take controls"),
    (0x10, Kind::Anim, "trigger animations"),
    (0x20, Kind::Attach, "attach"),
    (0x400, Kind::Camera, "track the camera"),
    (0x800, Kind::Camera, "control the camera"),
    (0x8000, Kind::Anim, "override animations"),
];

/// Script permissions the bot never grants, for the log.
const REFUSED: [(i32, &str); 6] = [
    (0x2, "take money from the avatar"),
    (0x80, "link and unlink objects"),
    (0x1000, "teleport the avatar"),
    (0x4000, "manage the estate"),
    (0x1_0000, "return objects"),
    (0x2_0000, "change the environment"),
];

/// The kinds of a script permission request with these permission bits, or
/// the first permission among them that has no kind and is never granted.
pub fn script_kinds(mask: i32) -> Result<Vec<Kind>, String> {
    let known = PERMISSIONS.iter().fold(0, |bits, (bit, _, _)| bits | bit);
    let other = mask & !known;
    if other != 0 {
        let named = REFUSED.iter().find(|(bit, _)| other & bit != 0);
        return Err(match named {
            Some((_, what)) => format!("permission to {what}"),
            None => format!("a permission the bot does not know (bits {other:#x})"),
        });
    }
    Ok(Kind::ALL
        .into_iter()
        .filter(|kind| PERMISSIONS.iter().any(|(bit, of, _)| of == kind && mask & bit != 0))
        .collect())
}

/// What a script may do with these permission bits: "trigger animations,
/// take controls".
pub fn permission_names(mask: i32) -> String {
    let names: Vec<&str> = PERMISSIONS
        .iter()
        .filter(|(bit, _, _)| mask & bit != 0)
        .map(|(_, _, name)| *name)
        .collect();
    names.join(", ")
}

/// One line of a rule: an avatar by name, or an avatar or object by UUID.
#[derive(Debug, Clone)]
pub enum Entry {
    Id(Uuid),
    Name {
        /// The full name in lower case, which is what is compared.
        key: String,
        /// The name as it was written.
        shown: String,
    },
}

impl PartialEq for Entry {
    fn eq(&self, other: &Entry) -> bool {
        match (self, other) {
            (Entry::Id(a), Entry::Id(b)) => a == b,
            (Entry::Name { key: a, .. }, Entry::Name { key: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Entry::Id(id) => write!(f, "{id}"),
            Entry::Name { shown, .. } => f.write_str(shown),
        }
    }
}

impl Entry {
    /// Reads a UUID, `First Last`, `first.last` or a single word.
    pub fn parse(given: &str) -> Result<Entry, String> {
        let given = given.trim();
        if let Ok(id) = Uuid::parse_str(given) {
            return Ok(Entry::Id(id));
        }
        match full_name(given) {
            Some(key) => Ok(Entry::Name {
                key,
                shown: given.split_whitespace().collect::<Vec<_>>().join(" "),
            }),
            None => Err(format!(
                "\"{given}\" is neither a UUID nor an avatar's name (`First Last`, `first.last` or one word)"
            )),
        }
    }

    /// Whether the entry stands for whoever sent a request: the avatar, the
    /// object, or the avatar that owns the object.
    pub fn matches(&self, sender: &Sender) -> bool {
        let (avatar, object) = match sender {
            Sender::Avatar { id, name } => (Some((Some(*id), Some(name.as_str()))), None),
            // The group is entered by its UUID; its name matches nothing.
            Sender::Inviter { name, id, group } => (Some((*id, Some(name.as_str()))), Some(*group)),
            Sender::Object { id, owner, .. } => {
                let owner = match owner {
                    Owner::Avatar { id, name } => Some((*id, name.as_deref())),
                    Owner::Group | Owner::Unconfirmed(_) => None,
                };
                (owner, *id)
            }
        };
        match self {
            Entry::Id(wanted) => object == Some(*wanted) || avatar.is_some_and(|(id, _)| id == Some(*wanted)),
            Entry::Name { key, .. } => avatar
                .and_then(|(_, name)| name)
                .and_then(full_name)
                .is_some_and(|name| name == *key),
        }
    }
}

/// Who owns an object that asks for something.
#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    /// An avatar, as far as the request or the grid said who.
    Avatar {
        id: Option<Uuid>,
        name: Option<String>,
    },
    Group,
    /// The request named this owner and the simulator has not confirmed
    /// that it is an avatar: nothing matches it.
    Unconfirmed(String),
}

/// Who sent a request.
#[derive(Debug, Clone, PartialEq)]
pub enum Sender {
    Avatar {
        id: Uuid,
        name: String,
    },
    Object {
        /// An inventory offer does not say which object gives.
        id: Option<Uuid>,
        name: String,
        owner: Owner,
    },
    /// The member of `group` who invites the avatar into it. An
    /// invitation names the inviter by name only; the UUID is looked up.
    Inviter {
        name: String,
        id: Option<Uuid>,
        group: Uuid,
    },
}

impl fmt::Display for Sender {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Sender::Avatar { id, name } if name.is_empty() => write!(f, "{id}"),
            Sender::Avatar { id, name } => write!(f, "{name} <{id}>"),
            Sender::Inviter { name, id: Some(id), .. } => write!(f, "{name} <{id}>"),
            Sender::Inviter { name, id: None, .. } => f.write_str(name),
            Sender::Object { id, name, owner } => {
                match id {
                    Some(id) => write!(f, "{name} <{id}>")?,
                    None => write!(f, "the object \"{name}\"")?,
                }
                match owner {
                    Owner::Avatar {
                        id: Some(id),
                        name: Some(name),
                    } => write!(f, " of {name} <{id}>"),
                    Owner::Avatar { id: Some(id), name: None } => write!(f, " of the avatar {id}"),
                    Owner::Avatar {
                        id: None,
                        name: Some(name),
                    } => write!(f, " of {name}"),
                    Owner::Avatar { id: None, name: None } => Ok(()),
                    Owner::Group => f.write_str(" of a group"),
                    Owner::Unconfirmed(name) => write!(f, ", said to be of {name} (not confirmed)"),
                }
            }
        }
    }
}

/// Whose requests of one kind are accepted without the operator.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    All,
    List(Vec<Entry>),
}

/// What a `perms` command did, as the line it is answered with. A change
/// that was no change is a warning.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub line: String,
    pub warning: bool,
}

fn changed(line: String) -> Change {
    Change { line, warning: false }
}

fn warning(text: String) -> Change {
    Change {
        line: format!("warning: {text}"),
        warning: true,
    }
}

fn listed(entries: &[Entry]) -> String {
    entries.iter().map(Entry::to_string).collect::<Vec<_>>().join(", ")
}

/// The auto-accept rule and the exceptions of every kind.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    rules: [Rule; 7],
    exceptions: [Vec<Entry>; 7],
}

impl Default for Rules {
    /// Nothing is accepted by itself.
    fn default() -> Rules {
        Rules {
            rules: std::array::from_fn(|_| Rule::List(Vec::new())),
            exceptions: std::array::from_fn(|_| Vec::new()),
        }
    }
}

impl Rules {
    pub fn rule(&self, kind: Kind) -> &Rule {
        &self.rules[kind.index()]
    }

    pub fn exceptions(&self, kind: Kind) -> &[Entry] {
        &self.exceptions[kind.index()]
    }

    /// Sets a kind's rule as the config gives it.
    pub fn set_rule(&mut self, kind: Kind, rule: Rule) {
        self.rules[kind.index()] = rule;
    }

    /// Sets a kind's exceptions as the config gives them.
    pub fn set_exceptions(&mut self, kind: Kind, entries: Vec<Entry>) {
        self.exceptions[kind.index()] = entries;
    }

    /// Whether `kind` is auto-accepted for `sender`.
    fn covers(&self, kind: Kind, sender: &Sender) -> bool {
        let wanted = match self.rule(kind) {
            Rule::All => true,
            Rule::List(entries) => entries.iter().any(|entry| entry.matches(sender)),
        };
        wanted && !self.exceptions(kind).iter().any(|entry| entry.matches(sender))
    }

    /// Whether a request with these kinds from `sender` is accepted without
    /// the operator: every kind must be auto-accepted for it. Otherwise the
    /// first kind that is not.
    pub fn decide(&self, kinds: &[Kind], sender: &Sender) -> Result<(), Kind> {
        match kinds.iter().find(|kind| !self.covers(**kind, sender)) {
            Some(kind) => Err(*kind),
            None => Ok(()),
        }
    }

    /// `perms autoaccept KIND all`.
    pub fn autoaccept_all(&mut self, kind: Kind) -> Change {
        if *self.rule(kind) == Rule::All {
            return warning(format!("everyone was auto-accepted for {kind} already"));
        }
        self.set_rule(kind, Rule::All);
        match self.exceptions(kind) {
            [] => changed(format!("{kind}: everyone is auto-accepted")),
            exceptions => changed(format!("{kind}: everyone is auto-accepted except {}", listed(exceptions))),
        }
    }

    /// `perms autoaccept KIND WHO`.
    pub fn autoaccept(&mut self, kind: Kind, entry: Entry) -> Change {
        let exceptions = &mut self.exceptions[kind.index()];
        let was_exception = exceptions.contains(&entry);
        exceptions.retain(|other| *other != entry);
        let added = match &mut self.rules[kind.index()] {
            Rule::List(entries) if !entries.contains(&entry) => {
                entries.push(entry.clone());
                true
            }
            _ => false,
        };
        match was_exception || added {
            true => changed(format!("{kind}: {entry} is auto-accepted")),
            false => warning(format!("{entry} was auto-accepted for {kind} already")),
        }
    }

    /// `perms revoke KIND all`.
    pub fn revoke_all(&mut self, kind: Kind) -> Change {
        if *self.rule(kind) == Rule::List(Vec::new()) {
            return warning(format!("nothing was auto-accepted for {kind} already"));
        }
        self.set_rule(kind, Rule::List(Vec::new()));
        changed(format!("{kind}: nothing is auto-accepted any more"))
    }

    /// `perms revoke KIND WHO`.
    pub fn revoke(&mut self, kind: Kind, entry: Entry) -> Change {
        let index = kind.index();
        let done = match &mut self.rules[index] {
            Rule::List(entries) if entries.contains(&entry) => {
                entries.retain(|other| *other != entry);
                Some(format!("{kind}: {entry} is not auto-accepted any more"))
            }
            Rule::List(_) => None,
            Rule::All if self.exceptions[index].contains(&entry) => None,
            Rule::All => {
                self.exceptions[index].push(entry.clone());
                Some(format!("{kind}: {entry} is not auto-accepted any more (an exception to `all`)"))
            }
        };
        match done {
            Some(line) => changed(line),
            None => warning(format!("{entry} was not auto-accepted for {kind} already")),
        }
    }

    /// One line per kind, as `perms status` prints them.
    pub fn status(&self) -> String {
        let line = |kind: Kind| {
            let rule = match self.rule(kind) {
                Rule::All => "all".to_string(),
                Rule::List(entries) if entries.is_empty() => "none".to_string(),
                Rule::List(entries) => listed(entries),
            };
            match self.exceptions(kind) {
                [] => format!("{kind}: {rule}"),
                exceptions => format!("{kind}: {rule} except {}", listed(exceptions)),
            }
        };
        Kind::ALL.into_iter().map(line).collect::<Vec<_>>().join("\n")
    }
}

/// What a request asks for, with what it takes to answer it.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// A teleport offer; `lure` is what accepting it takes.
    Tp { lure: Uuid },
    Inventory {
        /// Whom the answer goes to: the giver, or the giving object's owner.
        to: Uuid,
        /// An object gives, which is answered in its own way.
        from_object: bool,
        transaction: Uuid,
        asset_type: i32,
        /// What the offer says is offered.
        text: String,
    },
    /// A script permission request for the permission bits `mask`.
    Script { object: Uuid, item: Uuid, mask: i32 },
    /// An invitation into `group`; `id` is what the answer names, `fee`
    /// what joining costs as the invitation says, `name` the group's name
    /// once the grid has told it.
    Group {
        group: Uuid,
        id: Uuid,
        fee: i32,
        name: Option<String>,
    },
}

/// A request as the bot read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub sender: Sender,
    pub ask: Ask,
    /// Its place among the waiting requests; 0 until it has waited.
    seq: u64,
}

impl Request {
    pub fn new(sender: Sender, ask: Ask) -> Request {
        Request { sender, ask, seq: 0 }
    }

    pub fn kinds(&self) -> Vec<Kind> {
        match &self.ask {
            Ask::Tp { .. } => vec![Kind::Tp],
            Ask::Inventory { .. } => vec![Kind::Inventory],
            // Requests with other permissions are never made into one.
            Ask::Script { mask, .. } => script_kinds(*mask).unwrap_or_default(),
            Ask::Group { .. } => vec![Kind::Group],
        }
    }

    /// The group of an invitation, as the log and the lists name it:
    /// `"Builders" <uuid>`, or the UUID alone while its name is not known.
    pub fn group_words(&self) -> Option<String> {
        match &self.ask {
            Ask::Group {
                group, name: Some(name), ..
            } => Some(format!("\"{name}\" <{group}>")),
            Ask::Group { group, name: None, .. } => Some(format!("the group {group}")),
            _ => None,
        }
    }

    /// The fee of an invitation, when it has one.
    pub fn fee(&self) -> Option<u32> {
        match self.ask {
            Ask::Group { fee, .. } if fee > 0 => Some(fee as u32),
            _ => None,
        }
    }

    /// The inviter of an invitation: name and UUID, as far as known.
    pub fn inviter(&self) -> Option<(&str, Option<Uuid>)> {
        match &self.sender {
            Sender::Inviter { name, id, .. } => Some((name, *id)),
            _ => None,
        }
    }

    /// What is offered, for an inventory offer: `notecard Rules`.
    pub fn offered(&self) -> Option<String> {
        match &self.ask {
            Ask::Inventory { asset_type, text, .. } => Some(format!("{} {text}", inventory::type_name(*asset_type))),
            _ => None,
        }
    }

    /// The request for the operator log: what it is, what it asks for and
    /// who sent it.
    pub fn describe(&self) -> String {
        let sender = &self.sender;
        match &self.ask {
            Ask::Tp { .. } => format!("teleport offer from {sender}"),
            Ask::Inventory { .. } => format!("inventory offer from {sender} ({})", self.offered().unwrap_or_default()),
            Ask::Script { .. } => format!("permission request ({}) from {sender}", self.kind_names()),
            Ask::Group { .. } => {
                let fee = self.fee().map(|fee| format!(", which costs L${fee} to join")).unwrap_or_default();
                format!("invitation from {sender} into {}{fee}", self.group_words().unwrap_or_default())
            }
        }
    }

    fn kind_names(&self) -> String {
        let names: Vec<&str> = self.kinds().into_iter().map(Kind::name).collect();
        names.join(", ")
    }

    /// The command that accepts the request while it waits.
    pub fn accept_command(&self) -> String {
        if let Ask::Group { group, .. } = &self.ask {
            let pay = self.fee().map(|fee| format!(" --pay {fee}")).unwrap_or_default();
            return format!("rslbot ctl accept group {group}{pay}");
        }
        let kind = self.kinds().first().map_or("", |kind| kind.name());
        format!("rslbot ctl accept {kind}")
    }

    /// The request as `perms status` shows it.
    fn status_line(&self) -> String {
        if let Some(group) = self.group_words() {
            let fee = self.fee().map(|fee| format!(", L${fee}")).unwrap_or_default();
            return format!("waiting: group from {}: {group}{fee}", self.sender);
        }
        let what = self.offered().map(|what| format!(": {what}")).unwrap_or_default();
        format!("waiting: {} from {}{what}", self.kind_names(), self.sender)
    }

    /// Teleport offer, inventory offer or script permission request.
    fn sort(&self) -> u8 {
        match self.ask {
            Ask::Tp { .. } => 0,
            Ask::Inventory { .. } => 1,
            Ask::Script { .. } => 2,
            Ask::Group { .. } => 3,
        }
    }

    /// Whether `newer` takes this request's place: an avatar's newer
    /// teleport offer, a script's newer permission request.
    fn replaced_by(&self, newer: &Request) -> bool {
        match (&self.ask, &newer.ask) {
            (Ask::Tp { .. }, Ask::Tp { .. }) => match (&self.sender, &newer.sender) {
                (Sender::Avatar { id: a, .. }, Sender::Avatar { id: b, .. }) => a == b,
                _ => false,
            },
            (
                Ask::Script { object, item, .. },
                Ask::Script {
                    object: other,
                    item: other_item,
                    ..
                },
            ) => object == other && item == other_item,
            (Ask::Group { group, .. }, Ask::Group { group: other, .. }) => group == other,
            _ => false,
        }
    }
}

/// The requests that were not accepted and can still be, oldest first.
#[derive(Debug, Default)]
pub struct Waiting {
    requests: Vec<Request>,
    last_seq: u64,
}

impl Waiting {
    /// Keeps a request, in place of an older one of the same avatar or
    /// script. Returns the oldest request of its sort when that had to go
    /// to make room.
    pub fn push(&mut self, mut request: Request) -> Option<Request> {
        self.requests.retain(|old| !old.replaced_by(&request));
        if request.seq == 0 {
            self.last_seq += 1;
            request.seq = self.last_seq;
        }
        let sort = request.sort();
        let at = self.requests.partition_point(|old| old.seq < request.seq);
        self.requests.insert(at, request);
        let of_sort = self.requests.iter().filter(|old| old.sort() == sort).count();
        if of_sort <= MAX_WAITING {
            return None;
        }
        let oldest = self.requests.iter().position(|old| old.sort() == sort)?;
        Some(self.requests.remove(oldest))
    }

    /// Takes out the request of `kind` received last, among those `who`
    /// stands for when one is named.
    pub fn take(&mut self, kind: Kind, who: Option<&Entry>) -> Option<Request> {
        let wanted = |request: &Request| request.kinds().contains(&kind) && who.is_none_or(|who| who.matches(&request.sender));
        let found = self.requests.iter().rposition(wanted)?;
        Some(self.requests.remove(found))
    }

    /// The invitations `who` names, oldest first: by the UUID of the group
    /// or of the inviter, by the group's whole name in any letter case, or
    /// by the inviter's name in one of the forms of an entry. Every
    /// invitation without `who`.
    pub fn invitations(&self, who: Option<&str>) -> Vec<&Request> {
        let named = |request: &&Request| {
            let (Ask::Group { group, name, .. }, Sender::Inviter { name: inviter, id, .. }) = (&request.ask, &request.sender) else {
                return false;
            };
            let Some(who) = who.map(str::trim) else {
                return true;
            };
            if let Ok(wanted) = Uuid::parse_str(who) {
                return *group == wanted || *id == Some(wanted);
            }
            let by_group = name.as_ref().is_some_and(|name| name.to_lowercase() == who.to_lowercase());
            let by_inviter = full_name(who).is_some_and(|key| full_name(inviter) == Some(key));
            by_group || by_inviter
        };
        self.requests.iter().filter(named).collect()
    }

    /// Takes out the invitation into `group`.
    pub fn take_invitation(&mut self, group: Uuid) -> Option<Request> {
        let at = self
            .requests
            .iter()
            .position(|request| matches!(request.ask, Ask::Group { group: g, .. } if g == group))?;
        Some(self.requests.remove(at))
    }

    /// The avatar is in these groups: their invitations are of no use.
    /// Returns the ones forgotten.
    pub fn forget_invitations(&mut self, groups: &[Uuid]) -> Vec<Request> {
        let (gone, kept) = std::mem::take(&mut self.requests)
            .into_iter()
            .partition(|request| matches!(request.ask, Ask::Group { group, .. } if groups.contains(&group)));
        self.requests = kept;
        gone
    }

    /// The waiting invitations, oldest first.
    pub fn all_invitations(&self) -> Vec<&Request> {
        self.invitations(None)
    }

    /// A script asks anew: what it asked before is no longer open.
    pub fn forget_script(&mut self, object: Uuid, item: Uuid) {
        let asked = |ask: &Ask| matches!(ask, Ask::Script { object: o, item: i, .. } if *o == object && *i == item);
        self.requests.retain(|request| !asked(&request.ask));
    }

    /// The session is over: nothing that waited can be answered any more.
    pub fn clear(&mut self) {
        self.requests.clear();
    }

    /// The avatar left the region: its scripts stay behind.
    pub fn clear_scripts(&mut self) {
        self.requests.retain(|request| !matches!(request.ask, Ask::Script { .. }));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// One line per request, oldest first, as `perms status` prints them.
    pub fn status(&self) -> Vec<String> {
        self.requests.iter().map(Request::status_line).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn jane() -> Uuid {
        Uuid::from_bytes([0x33; 16])
    }

    pub(crate) fn ball() -> Uuid {
        Uuid::from_bytes([0xBA; 16])
    }

    fn avatar(name: &str) -> Sender {
        Sender::Avatar {
            id: jane(),
            name: name.to_string(),
        }
    }

    fn object_of(owner: Owner) -> Sender {
        Sender::Object {
            id: Some(ball()),
            name: "Dance Ball".to_string(),
            owner,
        }
    }

    fn janes(name: Option<&str>) -> Owner {
        Owner::Avatar {
            id: Some(jane()),
            name: name.map(str::to_string),
        }
    }

    fn entry(text: &str) -> Entry {
        Entry::parse(text).unwrap()
    }

    fn rules(kind: Kind, rule: &[&str], exceptions: &[&str]) -> Rules {
        let mut rules = Rules::default();
        let entries = |texts: &[&str]| texts.iter().map(|text| entry(text)).collect::<Vec<_>>();
        rules.set_rule(
            kind,
            match rule {
                ["all"] => Rule::All,
                listed => Rule::List(entries(listed)),
            },
        );
        rules.set_exceptions(kind, entries(exceptions));
        rules
    }

    fn builders() -> Uuid {
        Uuid::from_bytes([0xB1; 16])
    }

    fn inviter(name: &str, id: Option<Uuid>) -> Sender {
        Sender::Inviter {
            name: name.to_string(),
            id,
            group: builders(),
        }
    }

    fn invitation(name: &str, id: Option<Uuid>, group_name: Option<&str>, fee: i32) -> Request {
        let ask = Ask::Group {
            group: builders(),
            id: Uuid::from_bytes([0x1D; 16]),
            fee,
            name: group_name.map(str::to_string),
        };
        Request::new(inviter(name, id), ask)
    }

    #[test]
    fn an_invitation_is_matched_by_its_group_or_its_inviter() {
        let john = Uuid::from_bytes([0x34; 16]);
        // The group by UUID, whoever invites.
        assert!(entry(&builders().to_string()).matches(&inviter("John Roe", Some(john))));
        assert!(entry(&builders().to_string()).matches(&inviter("John Roe", None)));
        // The inviter by UUID, once known, and by name.
        assert!(entry(&jane().to_string()).matches(&inviter("Jane Doe", Some(jane()))));
        assert!(!entry(&jane().to_string()).matches(&inviter("Jane Doe", None)));
        assert!(entry("jane.doe").matches(&inviter("Jane Doe", None)));
        assert!(entry("sitbot").matches(&inviter("sitbot Resident", None)));
        // A group named like an avatar matches nothing of that name.
        assert!(!entry("Jane Doe").matches(&inviter("John Roe", Some(john))));
        assert_eq!(invitation("Jane Doe", None, None, 0).kinds(), [Kind::Group]);
    }

    #[test]
    fn an_invitation_is_described_with_its_group_inviter_and_fee() {
        let request = invitation("Jane Doe", Some(jane()), Some("Builders"), 250);
        let described = request.describe();
        assert!(described.contains("Jane Doe") && described.contains("\"Builders\"") && described.contains("L$250"));
        assert_eq!(
            request.accept_command(),
            format!("rslbot ctl accept group {} --pay 250", builders())
        );
        assert_eq!(
            request.status_line(),
            format!("waiting: group from Jane Doe <{}>: \"Builders\" <{}>, L$250", jane(), builders())
        );
        let free = invitation("Jane Doe", None, None, 0);
        assert_eq!(free.accept_command(), format!("rslbot ctl accept group {}", builders()));
        assert_eq!(
            free.status_line(),
            format!("waiting: group from Jane Doe: the group {}", builders())
        );
    }

    #[test]
    fn invitations_are_found_by_what_names_them() {
        let mut waiting = Waiting::default();
        waiting.push(invitation("Jane Doe", Some(jane()), Some("Builders"), 0));
        let named = |waiting: &Waiting, who: Option<&str>| waiting.invitations(who).len();
        assert_eq!(named(&waiting, None), 1);
        assert_eq!(named(&waiting, Some("builders")), 1);
        assert_eq!(named(&waiting, Some("Build")), 0);
        assert_eq!(named(&waiting, Some(&builders().to_string())), 1);
        assert_eq!(named(&waiting, Some(&jane().to_string())), 1);
        assert_eq!(named(&waiting, Some("jane.doe")), 1);
        assert_eq!(named(&waiting, Some("John Roe")), 0);
        // A newer invitation into the same group takes its place.
        waiting.push(invitation("John Roe", None, Some("Builders"), 0));
        assert_eq!(named(&waiting, None), 1);
        assert_eq!(named(&waiting, Some("John Roe")), 1);
        assert_eq!(waiting.forget_invitations(&[builders()]).len(), 1);
        assert_eq!(waiting.len(), 0);
    }

    #[test]
    fn kinds_have_names_and_an_order() {
        let names: Vec<&str> = Kind::ALL.into_iter().map(Kind::name).collect();
        assert_eq!(names, ["tp", "inventory", "anim", "attach", "controls", "camera", "group"]);
        assert_eq!(Kind::parse("anim"), Some(Kind::Anim));
        assert_eq!(Kind::parse("money"), None);
        assert_eq!(Kind::parse("Anim"), None);
    }

    #[test]
    fn script_permissions_have_kinds_or_are_refused() {
        assert_eq!(script_kinds(0x10), Ok(vec![Kind::Anim]));
        assert_eq!(script_kinds(0x8000), Ok(vec![Kind::Anim]));
        assert_eq!(script_kinds(0x10 | 0x8000), Ok(vec![Kind::Anim]));
        assert_eq!(script_kinds(0x20), Ok(vec![Kind::Attach]));
        assert_eq!(script_kinds(0x4), Ok(vec![Kind::Controls]));
        assert_eq!(script_kinds(0x400), Ok(vec![Kind::Camera]));
        assert_eq!(script_kinds(0x800 | 0x400), Ok(vec![Kind::Camera]));
        // In the order of the kinds, whatever the order of the bits.
        assert_eq!(script_kinds(0x4 | 0x10), Ok(vec![Kind::Anim, Kind::Controls]));
        assert_eq!(permission_names(0x4 | 0x10), "take controls, trigger animations");
        // Money, a script's teleport and whatever comes later are refused.
        assert_eq!(
            script_kinds(0x2 | 0x10),
            Err("permission to take money from the avatar".to_string())
        );
        assert_eq!(script_kinds(0x1000), Err("permission to teleport the avatar".to_string()));
        assert!(script_kinds(0x4000_0000).unwrap_err().contains("0x40000000"));
    }

    #[test]
    fn entries_are_uuids_or_names() {
        assert_eq!(entry("33333333-3333-3333-3333-333333333333"), Entry::Id(jane()));
        for spelling in ["Jane Doe", "jane.doe", "  JANE   doe "] {
            assert_eq!(entry(spelling), entry("Jane Doe"), "{spelling}");
        }
        assert_eq!(entry("sitbot"), entry("Sitbot Resident"));
        assert_ne!(entry("Jane Doe"), entry("Jane Roe"));
        // A name and a UUID are never the same entry.
        assert_ne!(entry("Jane Doe"), Entry::Id(jane()));
        // Shown as written, with the spaces tidied.
        assert_eq!(entry("  Jane   Doe ").to_string(), "Jane Doe");
        assert_eq!(entry("jane.doe").to_string(), "jane.doe");
        assert_eq!(Entry::Id(jane()).to_string(), "33333333-3333-3333-3333-333333333333");
        for bad in ["", "  ", "Jane van Doe"] {
            assert!(Entry::parse(bad).unwrap_err().contains("neither a UUID nor"), "{bad}");
        }
    }

    #[test]
    fn an_entry_stands_for_an_avatar_and_its_objects() {
        let (by_name, by_id, by_object) = (entry("Jane Doe"), Entry::Id(jane()), Entry::Id(ball()));
        // An avatar: a teleport offer, or an inventory offer of its own.
        assert!(by_name.matches(&avatar("Jane Doe")));
        assert!(by_id.matches(&avatar("Jane Doe")));
        assert!(!by_object.matches(&avatar("Jane Doe")));
        assert!(!entry("Jane Roe").matches(&avatar("Jane Doe")));
        // The grid's spelling of a name without a last name of its own.
        assert!(entry("sitbot").matches(&avatar("sitbot Resident")));
        assert!(entry("sitbot Resident").matches(&avatar("Sitbot")));
        // A script's object whose owner the simulator confirmed.
        let owned = object_of(janes(Some("Jane Doe")));
        assert!(by_name.matches(&owned));
        assert!(by_id.matches(&owned));
        assert!(by_object.matches(&owned));
        // An object that gives inventory: no UUID of its own, and the
        // owner's name only once it has been looked up.
        let giving = |name| Sender::Object {
            id: None,
            name: "Vendor".to_string(),
            owner: janes(name),
        };
        assert!(by_id.matches(&giving(None)));
        assert!(!by_name.matches(&giving(None)));
        assert!(by_name.matches(&giving(Some("Jane Doe"))));
        assert!(!by_object.matches(&giving(Some("Jane Doe"))));
    }

    #[test]
    fn a_group_or_an_unconfirmed_owner_matches_by_the_object_only() {
        for owner in [Owner::Group, Owner::Unconfirmed("Jane Doe".to_string())] {
            let sender = object_of(owner);
            assert!(!entry("Jane Doe").matches(&sender));
            assert!(!Entry::Id(jane()).matches(&sender));
            assert!(Entry::Id(ball()).matches(&sender));
        }
        // An object of a group that gives inventory: nothing matches.
        let giving = Sender::Object {
            id: None,
            name: "Vendor".to_string(),
            owner: Owner::Group,
        };
        assert!(!entry("Jane Doe").matches(&giving));
        assert!(!Entry::Id(jane()).matches(&giving));
    }

    #[test]
    fn a_request_is_accepted_when_every_kind_is() {
        let owned = object_of(janes(Some("Jane Doe")));
        // Nothing by default.
        assert_eq!(Rules::default().decide(&[Kind::Tp], &avatar("Jane Doe")), Err(Kind::Tp));
        // A listed avatar, and nobody else.
        let listed = rules(Kind::Tp, &["Jane Doe"], &[]);
        assert_eq!(listed.decide(&[Kind::Tp], &avatar("Jane Doe")), Ok(()));
        assert_eq!(listed.decide(&[Kind::Tp], &avatar("John Roe")), Err(Kind::Tp));
        assert_eq!(listed.decide(&[Kind::Inventory], &avatar("Jane Doe")), Err(Kind::Inventory));
        // Two kinds: both, or the request waits naming the one missing.
        let mut both = rules(Kind::Anim, &["all"], &[]);
        assert_eq!(both.decide(&[Kind::Anim, Kind::Controls], &owned), Err(Kind::Controls));
        both.set_rule(Kind::Controls, Rule::All);
        assert_eq!(both.decide(&[Kind::Anim, Kind::Controls], &owned), Ok(()));
    }

    #[test]
    fn exceptions_win_over_the_rule() {
        let other = Sender::Object {
            id: Some(Uuid::from_bytes([0xBB; 16])),
            name: "Other Ball".to_string(),
            owner: janes(Some("Jane Doe")),
        };
        let owned = object_of(janes(Some("Jane Doe")));
        // Everyone but one object.
        let all = rules(Kind::Anim, &["all"], &[&ball().to_string()]);
        assert_eq!(all.decide(&[Kind::Anim], &owned), Err(Kind::Anim));
        assert_eq!(all.decide(&[Kind::Anim], &other), Ok(()));
        // One object of a listed owner.
        let listed = rules(Kind::Anim, &["Jane Doe"], &[&ball().to_string()]);
        assert_eq!(listed.decide(&[Kind::Anim], &owned), Err(Kind::Anim));
        assert_eq!(listed.decide(&[Kind::Anim], &other), Ok(()));
        // An avatar excepted from everyone.
        let all = rules(Kind::Tp, &["all"], &["Jane Doe"]);
        assert_eq!(all.decide(&[Kind::Tp], &avatar("Jane Doe")), Err(Kind::Tp));
        assert_eq!(all.decide(&[Kind::Tp], &avatar("John Roe")), Ok(()));
    }

    #[test]
    fn autoaccept_adds_and_warns_when_there_is_nothing_to_add() {
        let mut rules = Rules::default();
        let added = rules.autoaccept(Kind::Inventory, entry("Avatar1"));
        assert_eq!((added.line.as_str(), added.warning), ("inventory: Avatar1 is auto-accepted", false));
        assert!(!rules.autoaccept(Kind::Inventory, Entry::Id(ball())).warning);
        assert_eq!(*rules.rule(Kind::Inventory), Rule::List(vec![entry("Avatar1"), Entry::Id(ball())]));
        // Again, in another spelling.
        let again = rules.autoaccept(Kind::Inventory, entry("avatar1.resident"));
        assert!(again.warning);
        assert_eq!(again.line, "warning: avatar1.resident was auto-accepted for inventory already");
        assert_eq!(*rules.rule(Kind::Inventory), Rule::List(vec![entry("Avatar1"), Entry::Id(ball())]));

        // Everyone: the list is covered, the exceptions stay.
        rules.set_exceptions(Kind::Inventory, vec![entry("John Roe")]);
        let all = rules.autoaccept_all(Kind::Inventory);
        assert_eq!(all.line, "inventory: everyone is auto-accepted except John Roe");
        assert_eq!(*rules.rule(Kind::Inventory), Rule::All);
        assert_eq!(rules.exceptions(Kind::Inventory), [entry("John Roe")]);
        let again = rules.autoaccept_all(Kind::Inventory);
        assert_eq!(again.line, "warning: everyone was auto-accepted for inventory already");
        // Under `all`, a name is news only when it was an exception.
        assert!(rules.autoaccept(Kind::Inventory, entry("Avatar1")).warning);
        let back = rules.autoaccept(Kind::Inventory, entry("John Roe"));
        assert_eq!((back.line.as_str(), back.warning), ("inventory: John Roe is auto-accepted", false));
        assert!(rules.exceptions(Kind::Inventory).is_empty());
        assert_eq!(*rules.rule(Kind::Inventory), Rule::All);
    }

    #[test]
    fn revoke_takes_away_and_warns_when_there_is_nothing_to_take() {
        let mut rules = rules(Kind::Anim, &["Jane Doe", &ball().to_string()], &[]);
        let gone = rules.revoke(Kind::Anim, Entry::Id(ball()));
        assert_eq!(gone.line, format!("anim: {} is not auto-accepted any more", ball()));
        assert_eq!(*rules.rule(Kind::Anim), Rule::List(vec![entry("Jane Doe")]));
        // Not in the list: a warning, and no exception is made.
        let again = rules.revoke(Kind::Anim, Entry::Id(ball()));
        assert_eq!(again.line, format!("warning: {} was not auto-accepted for anim already", ball()));
        assert!(rules.exceptions(Kind::Anim).is_empty());

        // Everything.
        let all = rules.revoke_all(Kind::Anim);
        assert_eq!((all.line.as_str(), all.warning), ("anim: nothing is auto-accepted any more", false));
        assert_eq!(*rules.rule(Kind::Anim), Rule::List(vec![]));
        assert_eq!(
            rules.revoke_all(Kind::Anim).line,
            "warning: nothing was auto-accepted for anim already"
        );

        // Under `all`, revoking makes an exception, once.
        rules.autoaccept_all(Kind::Anim);
        let excepted = rules.revoke(Kind::Anim, Entry::Id(ball()));
        assert!(!excepted.warning);
        assert!(excepted.line.contains("an exception to `all`"));
        assert_eq!(rules.exceptions(Kind::Anim), [Entry::Id(ball())]);
        assert!(rules.revoke(Kind::Anim, Entry::Id(ball())).warning);
        assert_eq!(rules.exceptions(Kind::Anim).len(), 1);
        // Revoking everything leaves the exceptions where they are.
        rules.revoke_all(Kind::Anim);
        assert_eq!(rules.exceptions(Kind::Anim), [Entry::Id(ball())]);
        // Other kinds are untouched throughout.
        assert_eq!(*rules.rule(Kind::Tp), Rule::List(vec![]));
    }

    #[test]
    fn status_prints_one_line_per_kind() {
        let mut rules = rules(Kind::Tp, &["Jane Doe"], &[]);
        rules.set_rule(Kind::Anim, Rule::All);
        rules.set_exceptions(Kind::Anim, vec![entry("01234567-9abc-defe-dcba-987654321000")]);
        assert_eq!(
            rules.status(),
            "tp: Jane Doe\n\
             inventory: none\n\
             anim: all except 01234567-9abc-defe-dcba-987654321000\n\
             attach: none\n\
             controls: none\n\
             camera: none\ngroup: none"
        );
        rules.set_rule(Kind::Tp, Rule::List(vec![entry("Jane Doe"), Entry::Id(ball())]));
        rules.set_exceptions(Kind::Tp, vec![entry("John Roe"), entry("sitbot")]);
        assert!(rules
            .status()
            .starts_with(&format!("tp: Jane Doe, {} except John Roe, sitbot\n", ball())));
    }

    pub(crate) fn offer(from: u8, name: &str, lure: u8) -> Request {
        let sender = Sender::Avatar {
            id: Uuid::from_bytes([from; 16]),
            name: name.to_string(),
        };
        Request::new(
            sender,
            Ask::Tp {
                lure: Uuid::from_bytes([lure; 16]),
            },
        )
    }

    fn script(object: u8, item: u8, mask: i32) -> Request {
        let object = Uuid::from_bytes([object; 16]);
        let sender = Sender::Object {
            id: Some(object),
            name: "Dance Ball".to_string(),
            owner: janes(Some("Jane Doe")),
        };
        let item = Uuid::from_bytes([item; 16]);
        Request::new(sender, Ask::Script { object, item, mask })
    }

    fn gift(from: u8, name: &str, transaction: u8) -> Request {
        let id = Uuid::from_bytes([from; 16]);
        let sender = Sender::Avatar {
            id,
            name: name.to_string(),
        };
        let ask = Ask::Inventory {
            to: id,
            from_object: false,
            transaction: Uuid::from_bytes([transaction; 16]),
            asset_type: inventory::ASSET_NOTECARD,
            text: "Rules".to_string(),
        };
        Request::new(sender, ask)
    }

    #[test]
    fn requests_say_what_they_are() {
        let request = script(0xBA, 1, 0x10 | 0x4);
        assert_eq!(request.kinds(), [Kind::Anim, Kind::Controls]);
        assert_eq!(
            request.describe(),
            format!(
                "permission request (anim, controls) from Dance Ball <{}> of Jane Doe <{}>",
                ball(),
                jane()
            )
        );
        assert_eq!(request.accept_command(), "rslbot ctl accept anim");
        let request = gift(0x33, "Jane Doe", 7);
        assert_eq!(request.kinds(), [Kind::Inventory]);
        assert_eq!(
            request.describe(),
            format!("inventory offer from Jane Doe <{}> (notecard Rules)", jane())
        );
        assert_eq!(request.accept_command(), "rslbot ctl accept inventory");
        let request = offer(0x33, "Jane Doe", 1);
        assert_eq!(request.describe(), format!("teleport offer from Jane Doe <{}>", jane()));
        assert_eq!(request.accept_command(), "rslbot ctl accept tp");
    }

    #[test]
    fn senders_are_shown_with_what_is_known_of_them() {
        let shown = |owner| object_of(owner).to_string();
        assert_eq!(shown(Owner::Group), format!("Dance Ball <{}> of a group", ball()));
        assert_eq!(
            shown(Owner::Unconfirmed("Jane Doe".to_string())),
            format!("Dance Ball <{}>, said to be of Jane Doe (not confirmed)", ball())
        );
        let giving = Sender::Object {
            id: None,
            name: "Vendor".to_string(),
            owner: janes(None),
        };
        assert_eq!(giving.to_string(), format!("the object \"Vendor\" of the avatar {}", jane()));
        let nameless = Sender::Avatar {
            id: jane(),
            name: String::new(),
        };
        assert_eq!(nameless.to_string(), jane().to_string());
    }

    #[test]
    fn the_latest_request_of_a_kind_is_taken_first() {
        let mut waiting = Waiting::default();
        for request in [
            offer(1, "John Roe", 1),
            script(0xB1, 1, 0x10),
            offer(2, "Jane Doe", 2),
            script(0xB2, 1, 0x10 | 0x4),
            gift(1, "John Roe", 1),
        ] {
            assert_eq!(waiting.push(request), None);
        }
        assert_eq!(waiting.len(), 5);
        // The latest of the kind; the others stay.
        assert_eq!(waiting.take(Kind::Tp, None), Some(offer(2, "Jane Doe", 2)).map(seq(3)));
        assert_eq!(waiting.take(Kind::Tp, None), Some(offer(1, "John Roe", 1)).map(seq(1)));
        assert_eq!(waiting.take(Kind::Tp, None), None);
        // A request with two kinds is found under each, and once.
        assert_eq!(waiting.take(Kind::Controls, None), Some(script(0xB2, 1, 0x14)).map(seq(4)));
        assert_eq!(waiting.take(Kind::Controls, None), None);
        assert_eq!(waiting.take(Kind::Anim, None), Some(script(0xB1, 1, 0x10)).map(seq(2)));
        assert_eq!(waiting.take(Kind::Camera, None), None);
        assert_eq!(waiting.len(), 1);
    }

    fn seq(seq: u64) -> impl Fn(Request) -> Request {
        move |request| Request { seq, ..request }
    }

    #[test]
    fn a_request_can_be_taken_by_who_sent_it() {
        let mut waiting = Waiting::default();
        waiting.push(script(0xB1, 1, 0x10));
        waiting.push(script(0xB2, 1, 0x10));
        waiting.push(gift(1, "John Roe", 1));
        // By the object, by the owner, and not by somebody else.
        let first = Entry::Id(Uuid::from_bytes([0xB1; 16]));
        assert_eq!(waiting.take(Kind::Anim, Some(&entry("John Roe"))), None);
        assert_eq!(waiting.take(Kind::Anim, Some(&first)), Some(script(0xB1, 1, 0x10)).map(seq(1)));
        assert_eq!(waiting.take(Kind::Anim, Some(&first)), None);
        assert_eq!(
            waiting.take(Kind::Anim, Some(&entry("jane.doe"))),
            Some(script(0xB2, 1, 0x10)).map(seq(2))
        );
        assert_eq!(waiting.take(Kind::Inventory, Some(&entry("Jane Doe"))), None);
        assert_eq!(waiting.len(), 1);
        assert!(waiting.take(Kind::Inventory, Some(&entry("John Roe"))).is_some());
    }

    #[test]
    fn a_newer_request_of_the_same_avatar_or_script_takes_the_place_of_the_older() {
        let mut waiting = Waiting::default();
        waiting.push(offer(1, "John Roe", 1));
        waiting.push(offer(1, "John Roe", 2));
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting.take(Kind::Tp, None).unwrap().ask, offer(1, "John Roe", 2).ask);
        // The same script again; another script of the same object is another.
        waiting.push(script(0xB1, 1, 0x10));
        waiting.push(script(0xB1, 1, 0x20));
        waiting.push(script(0xB1, 2, 0x10));
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting.take(Kind::Anim, None), Some(script(0xB1, 2, 0x10)).map(seq(5)));
        assert_eq!(waiting.take(Kind::Anim, None), None);
        assert_eq!(waiting.take(Kind::Attach, None), Some(script(0xB1, 1, 0x20)).map(seq(4)));
        // A script that asks anew has nothing open from before, whatever
        // becomes of the new request.
        waiting.push(script(0xB3, 1, 0x10));
        waiting.forget_script(Uuid::from_bytes([0xB3; 16]), Uuid::from_bytes([2; 16]));
        assert_eq!(waiting.len(), 1);
        waiting.forget_script(Uuid::from_bytes([0xB3; 16]), Uuid::from_bytes([1; 16]));
        assert_eq!(waiting.len(), 0);
        // Inventory offers all wait.
        for transaction in 1..=3 {
            waiting.push(gift(1, "John Roe", transaction));
        }
        assert_eq!(waiting.len(), 3);
    }

    #[test]
    fn a_request_put_back_keeps_its_place() {
        let mut waiting = Waiting::default();
        waiting.push(gift(1, "John Roe", 1));
        waiting.push(gift(2, "Jane Doe", 2));
        let first = waiting.take(Kind::Inventory, Some(&entry("John Roe"))).unwrap();
        assert_eq!(waiting.push(first), None);
        // Jane Doe's is still the one received last.
        assert_eq!(waiting.take(Kind::Inventory, None), Some(gift(2, "Jane Doe", 2)).map(seq(2)));
    }

    #[test]
    fn the_oldest_of_a_sort_goes_when_too_many_wait() {
        let mut waiting = Waiting::default();
        waiting.push(offer(0xEE, "Early Bird", 1));
        for transaction in 0..MAX_WAITING as u8 {
            assert_eq!(waiting.push(gift(1, "John Roe", transaction)), None);
        }
        let forgotten = waiting.push(gift(1, "John Roe", 200));
        assert_eq!(forgotten, Some(gift(1, "John Roe", 0)).map(seq(2)));
        // The teleport offer, older still, is of another sort and stays.
        assert_eq!(waiting.len(), MAX_WAITING + 1);
        assert!(waiting.take(Kind::Tp, None).is_some());
    }

    #[test]
    fn waiting_requests_are_forgotten_with_the_session_and_scripts_with_the_region() {
        let mut waiting = Waiting::default();
        waiting.push(offer(1, "John Roe", 1));
        waiting.push(script(0xB1, 1, 0x10));
        waiting.push(gift(1, "John Roe", 1));
        waiting.clear_scripts();
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting.take(Kind::Anim, None), None);
        waiting.clear();
        assert_eq!(waiting.len(), 0);
    }

    #[test]
    fn status_lists_what_waits_oldest_first() {
        let mut waiting = Waiting::default();
        waiting.push(script(0xBA, 1, 0x10 | 0x4));
        waiting.push(offer(0x33, "Jane Doe", 1));
        waiting.push(gift(0x33, "Jane Doe", 1));
        assert_eq!(
            waiting.status(),
            [
                format!("waiting: anim, controls from Dance Ball <{}> of Jane Doe <{}>", ball(), jane()),
                format!("waiting: tp from Jane Doe <{}>", jane()),
                format!("waiting: inventory from Jane Doe <{}>: notecard Rules", jane()),
            ]
        );
    }
}
