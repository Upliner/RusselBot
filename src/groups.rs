//! What the bot knows of the avatar's groups: the list the grid gives, the
//! active group, how a command's `<group>` argument names one, and the
//! texts of `group list` and `group info`. Nothing here talks to the grid.

use crate::escape::escape;
use std::fmt;
use uuid::Uuid;

/// The power to invite into a group (`GP_MEMBER_INVITE`), bit 1 of a
/// member's `GroupPowers`.
pub const POWER_INVITE: u64 = 1 << 1;

/// One group the avatar is in, as the grid lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Membership {
    pub id: Uuid,
    pub name: String,
    /// The avatar's powers in the group.
    pub powers: u64,
    /// The avatar gets the group's notices.
    pub notices: bool,
}

impl Membership {
    /// Whether the avatar may invite others into the group.
    pub fn may_invite(&self) -> bool {
        self.powers & POWER_INVITE != 0
    }

    /// The group as the operator log and the errors name it.
    pub fn words(&self) -> String {
        format!("\"{}\" <{}>", self.name, self.id)
    }
}

/// A group's profile, as the grid answers for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: Uuid,
    pub name: String,
    pub charter: String,
    /// The group shows in search.
    pub listed: bool,
    /// The asker's own title in the group; empty for none.
    pub title: String,
    pub founder: Uuid,
    /// What joining costs, in L$.
    pub fee: i32,
    /// Anyone may join, without an invitation.
    pub open: bool,
    pub members: i32,
    pub roles: i32,
    pub mature: bool,
}

/// Why a `<group>` argument names no group.
#[derive(Debug, Clone, PartialEq)]
pub enum Naming {
    /// No group of the avatar has this name.
    NoName(String),
    /// The avatar is in no group with this UUID.
    NotIn(Uuid),
    /// Several groups of the avatar have this name: their UUIDs and names.
    Ambiguous(String, Vec<(Uuid, String)>),
}

impl fmt::Display for Naming {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Naming::NoName(name) => write!(f, "the avatar is in no group named \"{name}\""),
            Naming::NotIn(id) => write!(f, "the avatar is not in the group {id}"),
            Naming::Ambiguous(name, groups) => {
                let listed: Vec<String> = groups.iter().map(|(id, name)| format!("{id} \"{}\"", escape(name))).collect();
                write!(
                    f,
                    "the avatar is in {} groups named \"{name}\"; give the UUID of one: {}",
                    groups.len(),
                    listed.join(", ")
                )
            }
        }
    }
}

/// What a new list changed, for the operator log.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Changes {
    /// The first list since the login.
    pub first: bool,
    pub joined: Vec<Membership>,
    pub left: Vec<Membership>,
}

/// The avatar's groups and the active one, as the grid last said, for one
/// session.
#[derive(Debug, Clone, Default)]
pub struct Groups {
    /// The grid has listed the groups since the login.
    known: bool,
    list: Vec<Membership>,
    active: Option<Uuid>,
    /// The title the avatar wears in the active group.
    title: String,
    /// Something the page shows changed since `take_changed`.
    changed: bool,
}

impl Groups {
    /// Whether the grid has listed the groups since the login.
    pub fn known(&self) -> bool {
        self.known
    }

    pub fn get(&self, id: Uuid) -> Option<&Membership> {
        self.list.iter().find(|group| group.id == id)
    }

    pub fn active(&self) -> Option<Uuid> {
        self.active
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    /// The grid listed the avatar's groups: they are these now.
    pub fn replace(&mut self, list: Vec<Membership>) -> Changes {
        let first = !self.known;
        let mut sorted_old = self.list.clone();
        let mut sorted_new = list.clone();
        sorted_old.sort_by_key(|group| group.id);
        sorted_new.sort_by_key(|group| group.id);
        if sorted_old != sorted_new || first {
            self.changed = true;
        }
        let joined = list.iter().filter(|group| self.get(group.id).is_none()).cloned().collect();
        let left = self
            .list
            .iter()
            .filter(|old| !list.iter().any(|group| group.id == old.id))
            .cloned()
            .collect();
        self.list = list;
        self.known = true;
        Changes {
            first,
            joined: if first { Vec::new() } else { joined },
            left,
        }
    }

    /// The grid says the avatar is no longer in this group.
    pub fn dropped(&mut self, id: Uuid) -> Option<Membership> {
        let at = self.list.iter().position(|group| group.id == id)?;
        self.changed = true;
        if self.active == Some(id) {
            self.active = None;
            self.title.clear();
        }
        Some(self.list.remove(at))
    }

    /// The grid says which group is active now, nil for none, and the
    /// title the avatar wears in it.
    pub fn set_active(&mut self, active: Uuid, title: &str) {
        let active = (!active.is_nil()).then_some(active);
        if self.active != active || self.title != title {
            self.changed = true;
        }
        self.active = active;
        self.title = title.to_string();
    }

    /// The session is over: nothing of it is known any more.
    pub fn clear(&mut self) {
        let had = self.known || !self.list.is_empty() || self.active.is_some();
        *self = Groups::default();
        self.changed = had;
    }

    /// Whether something the page shows changed since this was last asked.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The avatar's group a command names: by UUID, or by the whole name
    /// without regard to letter case.
    pub fn named(&self, given: &str) -> Result<&Membership, Naming> {
        let given = given.trim();
        if let Ok(id) = Uuid::parse_str(given) {
            return self.get(id).ok_or(Naming::NotIn(id));
        }
        let wanted = given.to_lowercase();
        let found: Vec<&Membership> = self.list.iter().filter(|group| group.name.to_lowercase() == wanted).collect();
        match found[..] {
            [] => Err(Naming::NoName(given.to_string())),
            [one] => Ok(one),
            _ => Err(Naming::Ambiguous(
                given.to_string(),
                found.iter().map(|group| (group.id, group.name.clone())).collect(),
            )),
        }
    }

    /// The group a command that takes any group names: a UUID, whatever
    /// the group, or the name of one of the avatar's.
    pub fn any(&self, given: &str) -> Result<Uuid, Naming> {
        match Uuid::parse_str(given.trim()) {
            Ok(id) => Ok(id),
            Err(_) => self.named(given).map(|group| group.id),
        }
    }

    /// The answer to `group list`: UUID, flags and name of each group,
    /// ordered by name without regard to letter case.
    pub fn list_text(&self) -> String {
        let mut groups: Vec<&Membership> = self.list.iter().collect();
        groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
        let line = |group: &&Membership| {
            let mut flags = Vec::new();
            if self.active == Some(group.id) {
                flags.push("active");
            }
            if group.may_invite() {
                flags.push("invite");
            }
            if group.notices {
                flags.push("notices");
            }
            let flags = match flags.is_empty() {
                true => "-".to_string(),
                false => flags.join(","),
            };
            format!("{}\t{flags}\t{}", group.id, escape(&group.name))
        };
        groups.iter().map(line).collect::<Vec<_>>().join("\n")
    }
}

fn yes(flag: bool) -> &'static str {
    match flag {
        true => "yes",
        false => "no",
    }
}

/// The answer to `group info`: `name: value` lines, each value on one
/// line. `founder` is the founder's name, or its UUID where the grid did
/// not name it; `member` whether the avatar is in the group.
pub fn info_text(profile: &Profile, founder: &str, member: bool) -> String {
    let fee = match profile.fee {
        fee if fee > 0 => format!("L${fee}"),
        _ => "none".to_string(),
    };
    let mut lines = vec![
        format!("name: {}", escape(&profile.name)),
        format!("group: {}", profile.id),
        format!("founder: {}", escape(founder)),
        format!("fee: {fee}"),
        format!("enrollment: {}", if profile.open { "open" } else { "closed" }),
        format!("members: {}", profile.members),
        format!("roles: {}", profile.roles),
        format!("listed: {}", yes(profile.listed)),
        format!("mature: {}", yes(profile.mature)),
        format!("member: {}", yes(member)),
    ];
    if member && !profile.title.is_empty() {
        lines.push(format!("title: {}", escape(&profile.title)));
    }
    lines.push(format!("charter: {}", escape(&profile.charter)));
    lines.join("\n")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn builders() -> Uuid {
        Uuid::from_bytes([0xB1; 16])
    }

    pub(crate) fn dancers() -> Uuid {
        Uuid::from_bytes([0xD1; 16])
    }

    pub(crate) fn member(id: Uuid, name: &str, powers: u64, notices: bool) -> Membership {
        Membership {
            id,
            name: name.to_string(),
            powers,
            notices,
        }
    }

    fn both() -> Vec<Membership> {
        vec![
            member(dancers(), "Dancers", 0, true),
            member(builders(), "Builders", POWER_INVITE | 1, true),
        ]
    }

    pub(crate) fn profile(id: Uuid, name: &str, fee: i32, open: bool) -> Profile {
        Profile {
            id,
            name: name.to_string(),
            charter: "We build.".to_string(),
            listed: true,
            title: String::new(),
            founder: Uuid::from_bytes([0x33; 16]),
            fee,
            open,
            members: 12,
            roles: 3,
            mature: false,
        }
    }

    #[test]
    fn after_login_the_list_and_the_active_group_are_known() {
        let mut groups = Groups::default();
        assert!(!groups.known());
        let changes = groups.replace(both());
        assert!(changes.first && changes.joined.is_empty() && changes.left.is_empty());
        groups.set_active(builders(), "Master builder");
        assert!(groups.known());
        assert_eq!(groups.active(), Some(builders()));
        let lines: Vec<String> = groups.list_text().lines().map(str::to_string).collect();
        assert_eq!(lines[0], format!("{}\tactive,invite,notices\tBuilders", builders()));
        assert_eq!(lines[1], format!("{}\tnotices\tDancers", dancers()));
    }

    #[test]
    fn an_ejection_and_a_new_list_change_the_groups() {
        let mut groups = Groups::default();
        groups.replace(both());
        let dropped = groups.dropped(dancers()).unwrap();
        assert_eq!(dropped.name, "Dancers");
        assert!(groups.named("Dancers").is_err());
        assert_eq!(groups.dropped(dancers()), None);
        // The list after the next login, joined from a viewer meanwhile.
        let changes = groups.replace(both());
        assert_eq!(changes.joined, vec![member(dancers(), "Dancers", 0, true)]);
        assert!(!changes.first);
        let changes = groups.replace(vec![member(builders(), "Builders", 3, true)]);
        assert_eq!(changes.left.len(), 1);
    }

    #[test]
    fn dropping_the_active_group_leaves_none_active() {
        let mut groups = Groups::default();
        groups.replace(both());
        groups.set_active(dancers(), "");
        groups.dropped(dancers());
        assert_eq!(groups.active(), None);
        groups.set_active(Uuid::nil(), "");
        assert_eq!(groups.active(), None);
    }

    #[test]
    fn everything_is_forgotten_with_the_session() {
        let mut groups = Groups::default();
        groups.replace(both());
        groups.set_active(builders(), "x");
        groups.take_changed();
        groups.clear();
        assert!(!groups.known() && groups.count() == 0 && groups.active().is_none());
        assert!(groups.take_changed());
        groups.clear();
        assert!(!groups.take_changed());
    }

    #[test]
    fn the_flag_is_raised_by_a_change_and_not_by_an_equal_list() {
        let mut groups = Groups::default();
        groups.replace(both());
        assert!(groups.take_changed());
        assert!(!groups.take_changed());
        // The same groups in another order.
        let mut again = both();
        again.reverse();
        groups.replace(again);
        assert!(!groups.take_changed());
        // A flag of one group.
        groups.replace(vec![
            member(dancers(), "Dancers", 0, false),
            member(builders(), "Builders", 3, true),
        ]);
        assert!(groups.take_changed());
        groups.set_active(builders(), "");
        assert!(groups.take_changed());
        groups.set_active(builders(), "");
        assert!(!groups.take_changed());
        groups.dropped(builders());
        assert!(groups.take_changed());
    }

    #[test]
    fn a_group_is_named_by_its_uuid_or_its_whole_name_in_any_case() {
        let mut groups = Groups::default();
        groups.replace(vec![
            member(builders(), "Builders", 0, true),
            member(dancers(), "Night Owls", 0, true),
        ]);
        assert_eq!(groups.named("builders").unwrap().id, builders());
        assert_eq!(groups.named("BUILDERS").unwrap().id, builders());
        assert_eq!(groups.named("Night Owls").unwrap().id, dancers());
        assert_eq!(groups.named(&builders().to_string()).unwrap().name, "Builders");
        let part = groups.named("Build").unwrap_err();
        assert_eq!(part, Naming::NoName("Build".to_string()));
        assert!(part.to_string().contains("\"Build\""));
        let other = Uuid::from_bytes([7; 16]);
        let not_in = groups.named(&other.to_string()).unwrap_err();
        assert!(not_in.to_string().contains("not in the group") && not_in.to_string().contains(&other.to_string()));
        // `group info` takes any group by UUID.
        assert_eq!(groups.any(&other.to_string()), Ok(other));
        assert_eq!(groups.any("night owls"), Ok(dancers()));
    }

    #[test]
    fn two_groups_of_one_name_are_listed_with_their_uuids() {
        let mut groups = Groups::default();
        let (a, b) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        groups.replace(vec![member(a, "Friends", 0, true), member(b, "friends", 0, true)]);
        let error = groups.named("Friends").unwrap_err().to_string();
        assert!(error.contains(&a.to_string()) && error.contains(&b.to_string()), "{error}");
        assert!(error.contains("2 groups named \"Friends\""), "{error}");
    }

    #[test]
    fn no_groups_list_as_no_text() {
        let mut groups = Groups::default();
        groups.replace(Vec::new());
        assert_eq!(groups.list_text(), "");
    }

    #[test]
    fn a_name_with_tabs_and_line_ends_stays_one_field() {
        let mut groups = Groups::default();
        groups.replace(vec![member(builders(), "A\tB\nC\rD\\E", 0, false)]);
        assert_eq!(groups.list_text(), format!("{}\t-\tA\\tB\\nC\\rD\\\\E", builders()));
    }

    #[test]
    fn info_of_a_group_the_avatar_is_not_in() {
        let profile = profile(builders(), "Builders", 250, true);
        let text = info_text(&profile, "Jane Doe", false);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "name: Builders".to_string(),
                format!("group: {}", builders()),
                "founder: Jane Doe".to_string(),
                "fee: L$250".to_string(),
                "enrollment: open".to_string(),
                "members: 12".to_string(),
                "roles: 3".to_string(),
                "listed: yes".to_string(),
                "mature: no".to_string(),
                "member: no".to_string(),
                "charter: We build.".to_string(),
            ]
        );
    }

    #[test]
    fn info_of_a_free_closed_group_with_a_title_and_a_long_charter() {
        let profile = Profile {
            title: "Officer".to_string(),
            charter: "One\nTwo\nThree\twith \\ tab".to_string(),
            ..profile(builders(), "Builders\nInc", 0, false)
        };
        let text = info_text(&profile, "Jane Doe", true);
        assert!(text.contains("\nfee: none\n"));
        assert!(text.contains("\nenrollment: closed\n"));
        assert!(text.contains("\nmember: yes\ntitle: Officer\n"));
        assert!(text.starts_with("name: Builders\\nInc\n"));
        assert!(text.ends_with("charter: One\\nTwo\\nThree\\twith \\\\ tab"));
        assert_eq!(text.lines().count(), 12);
    }
}
