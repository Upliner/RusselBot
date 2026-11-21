//! The avatar's friends and which of them are online, as the login and the
//! simulator's notices tell it, and the `friends` command's listing. No
//! session and no clock.

use crate::escape::escape;
use crate::names::Names;
use std::cmp::Ordering;
use std::collections::HashMap;
use uuid::Uuid;

/// The right a friend grants to see when it is online
/// (`LLRelationship::GRANT_ONLINE_STATUS`).
pub const SEE_ONLINE: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Friend {
    /// It lets the avatar see when it is online.
    sees_online: bool,
    /// The simulator's notices say it is online.
    online: bool,
}

/// What a friend's line says of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Online,
    Offline,
    /// It does not let the avatar see.
    Unknown,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Online => "online",
            Status::Offline => "offline",
            Status::Unknown => "?",
        }
    }
}

/// The friends of the avatar for one login.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Friends(HashMap<Uuid, Friend>);

impl Friends {
    /// The friends the login named, each with the rights it grants the
    /// avatar; none counts as online yet.
    pub fn new(list: &[(Uuid, i32)]) -> Friends {
        let friend = |rights: i32| Friend {
            sees_online: rights & SEE_ONLINE != 0,
            online: false,
        };
        Friends(list.iter().map(|(id, rights)| (*id, friend(*rights))).collect())
    }

    /// The simulator says these avatars came online; strangers are no
    /// concern.
    pub fn online(&mut self, ids: &[Uuid]) {
        self.set_online(ids, true);
    }

    /// The simulator says these avatars went offline.
    pub fn offline(&mut self, ids: &[Uuid]) {
        self.set_online(ids, false);
    }

    fn set_online(&mut self, ids: &[Uuid], online: bool) {
        for id in ids {
            if let Some(friend) = self.0.get_mut(id) {
                friend.online = online;
            }
        }
    }

    /// `id` ended the friendship. Returns whether it was a friend.
    pub fn ended(&mut self, id: Uuid) -> bool {
        self.0.remove(&id).is_some()
    }

    /// `id` accepted the avatar's offer of friendship: a friend that lets
    /// the avatar see when it is online. Returns whether it is new.
    pub fn accepted(&mut self, id: Uuid) -> bool {
        let new = !self.0.contains_key(&id);
        let friend = self.0.entry(id).or_insert(Friend {
            sees_online: true,
            online: false,
        });
        friend.sees_online = true;
        new
    }

    /// The friend `id` changed the rights it grants the avatar to `rights`.
    /// Whether it is online stays as the notices said, hidden or not.
    pub fn rights_changed(&mut self, id: Uuid, rights: i32) {
        if let Some(friend) = self.0.get_mut(&id) {
            friend.sees_online = rights & SEE_ONLINE != 0;
        }
    }

    pub fn is_friend(&self, id: Uuid) -> bool {
        self.0.contains_key(&id)
    }

    /// Whether `id` is a friend that lets the avatar see when it is online.
    pub fn sees_online(&self, id: Uuid) -> bool {
        self.0.get(&id).is_some_and(|friend| friend.sees_online)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Every friend, in no particular order.
    pub fn ids(&self) -> Vec<Uuid> {
        self.0.keys().copied().collect()
    }

    pub fn status(&self, id: Uuid) -> Option<Status> {
        self.0.get(&id).map(|friend| match friend {
            Friend { sees_online: false, .. } => Status::Unknown,
            Friend { online: true, .. } => Status::Online,
            Friend { online: false, .. } => Status::Offline,
        })
    }

    /// How many friends are listed as online.
    pub fn online_count(&self) -> usize {
        self.0.keys().filter(|id| self.status(**id) == Some(Status::Online)).count()
    }

    /// The listing: one line each of the UUID, the status, the name and
    /// the display name, separated by tabs, with `?` for a name not known.
    /// Online first, then offline, then unknown; within each by name
    /// without regard to case, the unnamed last, then by UUID. `None` when
    /// there are no friends.
    pub fn listing(&self, names: &HashMap<Uuid, Option<Names>>) -> Option<String> {
        let named = |id: &Uuid| names.get(id).and_then(Option::as_ref);
        let mut lines: Vec<(Status, Option<String>, Uuid)> = self
            .0
            .keys()
            .map(|id| {
                (
                    self.status(*id).unwrap_or(Status::Unknown),
                    named(id).map(|n| n.legacy.to_lowercase()),
                    *id,
                )
            })
            .collect();
        let by_name = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(a), Some(b)) => a.cmp(b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        lines.sort_by(|a, b| a.0.cmp(&b.0).then(by_name(&a.1, &b.1)).then(a.2.cmp(&b.2)));
        let text: Vec<String> = lines
            .iter()
            .map(|(status, _, id)| {
                let (name, display) = match named(id) {
                    Some(names) => (escape(&names.legacy), escape(&names.display)),
                    None => ("?".to_string(), "?".to_string()),
                };
                format!("{id}\t{}\t{name}\t{display}", status.word())
            })
            .collect();
        (!text.is_empty()).then(|| text.join("\n"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn jane() -> Uuid {
        Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap()
    }
    pub fn john() -> Uuid {
        Uuid::parse_str("22222222-3333-4444-5555-666666666666").unwrap()
    }
    pub fn ann() -> Uuid {
        Uuid::parse_str("33333333-4444-5555-6666-777777777777").unwrap()
    }
    pub fn zed() -> Uuid {
        Uuid::parse_str("44444444-5555-6666-7777-888888888888").unwrap()
    }

    fn stranger() -> Uuid {
        Uuid::from_bytes([0x99; 16])
    }

    pub fn names() -> HashMap<Uuid, Option<Names>> {
        let named = |legacy: &str, display: &str| {
            Some(Names {
                legacy: legacy.to_string(),
                display: display.to_string(),
            })
        };
        HashMap::from([
            (jane(), named("Jane Doe", "JD")),
            (john(), named("John Roe", "John Roe")),
            (ann(), named("Ann Poe", "Annie")),
            (zed(), named("Zed Low", "Zed")),
        ])
    }

    fn status(friends: &Friends, id: Uuid) -> Option<Status> {
        friends.status(id)
    }

    #[test]
    fn who_is_online() {
        let mut friends = Friends::new(&[(jane(), SEE_ONLINE), (john(), 3)]);
        assert_eq!(status(&friends, jane()), Some(Status::Offline));
        friends.online(&[jane()]);
        assert_eq!(status(&friends, jane()), Some(Status::Online));
        assert_eq!(friends.online_count(), 1);
        friends.offline(&[jane()]);
        assert_eq!(status(&friends, jane()), Some(Status::Offline));
        // A notice about a stranger adds nobody.
        friends.online(&[stranger()]);
        assert!(!friends.is_friend(stranger()));
        assert_eq!(friends.len(), 2);
        // A new login starts offline.
        friends.online(&[jane()]);
        let again = Friends::new(&[(jane(), SEE_ONLINE)]);
        assert_eq!(status(&again, jane()), Some(Status::Offline));
    }

    #[test]
    fn friendships_that_end_or_begin() {
        let mut friends = Friends::new(&[(jane(), SEE_ONLINE)]);
        friends.online(&[jane()]);
        assert!(friends.ended(jane()));
        assert!(!friends.is_friend(jane()) && friends.status(jane()).is_none());
        assert!(!friends.ended(jane()));
        // An accepted offer: a friend that shows its status, online once
        // the notice comes.
        assert!(friends.accepted(ann()));
        assert_eq!(status(&friends, ann()), Some(Status::Offline));
        friends.online(&[ann()]);
        assert_eq!(status(&friends, ann()), Some(Status::Online));
        assert!(!friends.accepted(ann()));
        assert_eq!(status(&friends, ann()), Some(Status::Online));
    }

    #[test]
    fn friends_that_hide_their_status() {
        let mut friends = Friends::new(&[(john(), 0), (jane(), SEE_ONLINE)]);
        assert_eq!(status(&friends, john()), Some(Status::Unknown));
        assert!(!friends.sees_online(john()) && friends.sees_online(jane()));
        // Whatever notices arrive.
        friends.online(&[john()]);
        assert_eq!(status(&friends, john()), Some(Status::Unknown));
        // Shown again: the notice that came meanwhile counts.
        friends.rights_changed(john(), SEE_ONLINE | 2);
        assert_eq!(status(&friends, john()), Some(Status::Online));
        // Hidden while the bot runs.
        friends.online(&[jane()]);
        friends.rights_changed(jane(), 2);
        assert_eq!(status(&friends, jane()), Some(Status::Unknown));
        assert_eq!(friends.online_count(), 1);
        // Rights of a stranger change nothing.
        friends.rights_changed(stranger(), SEE_ONLINE);
        assert!(!friends.is_friend(stranger()));
    }

    #[test]
    fn a_line_has_four_fields() {
        let mut friends = Friends::new(&[(jane(), SEE_ONLINE)]);
        friends.online(&[jane()]);
        assert_eq!(friends.listing(&names()).unwrap(), format!("{}\tonline\tJane Doe\tJD", jane()));
        // Not named: `?` twice.
        for names in [HashMap::new(), HashMap::from([(jane(), None)])] {
            assert_eq!(friends.listing(&names).unwrap(), format!("{}\tonline\t?\t?", jane()));
        }
        let odd = HashMap::from([(
            jane(),
            Some(Names {
                legacy: "Jane Doe".to_string(),
                display: "J\tD".to_string(),
            }),
        )]);
        assert_eq!(friends.listing(&odd).unwrap(), format!("{}\tonline\tJane Doe\tJ\\tD", jane()));
        assert_eq!(Friends::new(&[]).listing(&names()), None);
    }

    /// The README's example.
    pub fn example() -> Friends {
        let mut friends = Friends::new(&[(ann(), SEE_ONLINE), (john(), SEE_ONLINE), (jane(), SEE_ONLINE), (zed(), 0)]);
        friends.online(&[john(), jane()]);
        friends
    }

    #[test]
    fn online_first_then_by_name() {
        let text = example().listing(&names()).unwrap();
        assert_eq!(
            text,
            [
                format!("{}\tonline\tJane Doe\tJD", jane()),
                format!("{}\tonline\tJohn Roe\tJohn Roe", john()),
                format!("{}\toffline\tAnn Poe\tAnnie", ann()),
                format!("{}\t?\tZed Low\tZed", zed()),
            ]
            .join("\n")
        );
        // Without regard to case, the unnamed last, then by UUID.
        let mut friends = Friends::new(&[(jane(), SEE_ONLINE), (john(), SEE_ONLINE), (ann(), SEE_ONLINE), (zed(), SEE_ONLINE)]);
        friends.online(&[]);
        let lower = HashMap::from([
            (
                john(),
                Some(Names {
                    legacy: "anna bee".to_string(),
                    display: String::new(),
                }),
            ),
            (
                ann(),
                Some(Names {
                    legacy: "Ann Poe".to_string(),
                    display: String::new(),
                }),
            ),
        ]);
        let ids: Vec<String> = friends
            .listing(&lower)
            .unwrap()
            .lines()
            .map(|line| line.split('\t').next().unwrap().to_string())
            .collect();
        assert_eq!(ids, [ann(), john(), jane(), zed()].map(|id| id.to_string()));
    }

    #[test]
    fn the_readme_shows_this_listing() {
        let listing = example().listing(&names()).unwrap();
        assert!(include_str!("../README.md").contains(&format!("```\n{listing}\n```")));
    }
}
