//! The `group` commands, invitations, group chat and group fees against a
//! fake grid.

use super::*;
use crate::commands::tests::{Call, FakeGrid};
use crate::groups::tests::{builders, dancers, member, profile};
use crate::groups::POWER_INVITE;
use crate::names::Names;
use crate::payments::{Ledger, PaymentsConfig};
use crate::perms::{Entry, Rule};
use crate::rlv::Restrictions;
use crate::txlog::Kind as TxKind;
use std::sync::mpsc::{self, Receiver};

/// 2026-12-01T12:00:00Z.
const T0: u64 = 1_796_126_400;

fn jane() -> Uuid {
    Uuid::from_bytes([0x33; 16])
}

fn john() -> Uuid {
    Uuid::from_bytes([0x34; 16])
}

fn invitation(byte: u8) -> Uuid {
    Uuid::from_bytes([0x1D, byte, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn night_owls() -> Uuid {
    Uuid::from_bytes([0x0E; 16])
}

/// A group the avatar is not in.
fn fishers() -> Uuid {
    Uuid::from_bytes([0xAB; 16])
}

fn ok(answer: &Receiver<Response>) -> Option<String> {
    match answer.try_recv() {
        Ok(Response::Ok(text)) => text,
        other => panic!("expected success, got {other:?}"),
    }
}

fn error(answer: &Receiver<Response>) -> String {
    match answer.try_recv() {
        Ok(Response::Error(reason)) => reason,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn waits(answer: &Receiver<Response>) {
    assert!(answer.try_recv().is_err(), "answered already");
}

fn config() -> PaymentsConfig {
    PaymentsConfig {
        limit: Some(1000),
        renew: None,
        perms: [true, true, true],
        whitelist: None,
        blacklist: Vec::new(),
        unlimited: Vec::new(),
    }
}

/// The groups the avatar is in on most of these tests: `Builders`,
/// active and with the right to invite, and `Dancers`.
fn usual() -> Vec<Membership> {
    vec![
        member(builders(), "Builders", POWER_INVITE, true),
        member(dancers(), "Dancers", 0, true),
        member(night_owls(), "Night Owls", POWER_INVITE, false),
    ]
}

struct Rig {
    commands: Commands,
    grid: FakeGrid,
    now: Instant,
}

impl Rig {
    /// Logged in; the grid has not listed the groups yet.
    fn new() -> Rig {
        let mut commands = Commands::default();
        commands.set_clock(|| T0);
        Rig {
            commands,
            grid: FakeGrid::default(),
            now: Instant::now(),
        }
    }

    /// Logged in, in the usual groups, `Builders` active.
    fn listed() -> Rig {
        let mut rig = Rig::new();
        rig.list(usual());
        rig.commands.active_group(builders(), "", "Builders");
        rig.commands.take_told();
        rig
    }

    fn paying(config: PaymentsConfig) -> Rig {
        let mut rig = Rig::listed();
        rig.commands.set_payments(Some(config.clone()), Ledger::from_config(&config));
        rig
    }

    fn list(&mut self, groups: Vec<Membership>) {
        self.commands.groups_listed(groups, Some(&mut self.grid), self.now);
    }

    fn run_via(&mut self, via: Interface, request: Request) -> Receiver<Response> {
        let (reply, answer) = mpsc::channel();
        self.commands
            .handle_through(Origin::Operator, via, request, reply, Some(&mut self.grid), self.now);
        answer
    }

    fn run(&mut self, request: Request) -> Receiver<Response> {
        self.run_via(Interface::Ctl, request)
    }

    fn group(&mut self, request: GroupRequest) -> Receiver<Response> {
        self.run(Request::Group(request))
    }

    fn line(&mut self, line: &str) -> Receiver<Response> {
        let request = Request::decode(line.replace(' ', "\t").as_bytes()).unwrap();
        self.run(request)
    }

    fn calls(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.grid.calls)
    }

    fn told(&mut self) -> Vec<String> {
        self.commands.take_told()
    }

    fn advance(&mut self, seconds: u64) {
        self.now += Duration::from_secs(seconds);
        self.commands.tick(self.now);
        self.commands.requests_due(Some(&mut self.grid), self.now);
        self.commands.money_due(Some(&mut self.grid), self.now);
    }

    fn profile(&mut self, profile: Profile) {
        self.commands.group_profile(profile, Some(&mut self.grid), self.now);
    }

    fn named(&mut self, id: Uuid, legacy: &str) {
        let names = Names {
            legacy: legacy.to_string(),
            display: String::new(),
        };
        self.commands.names_resolved(&[(id, names)], &[], Some(&mut self.grid), self.now);
    }

    fn records(&mut self) -> Vec<GroupMessage> {
        self.commands.take_group_records()
    }

    fn invited(&mut self, group: Uuid, id: Uuid, inviter: &str, fee: i32) {
        self.commands
            .group_invitation(group, id, inviter, fee, Some(&mut self.grid), self.now);
    }

    /// The query of the one avatar search sent since the last call.
    fn lookup(&mut self, name: &str) -> Uuid {
        match &self.calls()[..] {
            [Call::Find(query, asked)] if *asked == name.to_lowercase() => *query,
            other => panic!("expected one lookup of {name}, got {other:?}"),
        }
    }

    /// `pay 10 avatar JANE`, as payments are carried out.
    fn pay_jane(&mut self) -> Receiver<Response> {
        let (reply, answer) = mpsc::channel();
        let request = Request::Pay {
            amount: 10,
            payee: crate::control::PayTo::Avatar(jane().to_string()),
            description: None,
        };
        self.commands
            .money_command(Origin::Operator, Interface::Ctl, request, reply, Some(&mut self.grid), self.now);
        answer
    }

    fn set_rule(&mut self, rule: Rule) {
        let mut rules = crate::perms::Rules::default();
        rules.set_rule(Kind::Group, rule);
        self.commands.set_rules(rules);
    }
}

fn entries(texts: &[&str]) -> Rule {
    Rule::List(texts.iter().map(|text| Entry::parse(text).unwrap()).collect())
}

// ---------------------------------------------------- needing a region

#[test]
fn group_commands_need_the_avatar_in_a_region() {
    let mut rig = Rig::listed();
    rig.grid.offline = true;
    for line in [
        "group list",
        "group activate Builders",
        "group info Builders",
        "group invitations",
        "group chat Builders hello",
        "group leave Builders",
    ] {
        assert_eq!(error(&rig.line(line)), NOT_CONNECTED, "{line}");
    }
    let (reply, answer) = mpsc::channel();
    rig.commands
        .handle(Request::Group(GroupRequest::List), reply, None::<&mut FakeGrid>, rig.now);
    assert_eq!(error(&answer), NOT_CONNECTED);
    assert_eq!(rig.calls(), []);
}

#[test]
fn a_command_right_after_login_waits_for_the_groups() {
    let mut rig = Rig::new();
    let answer = rig.group(GroupRequest::List);
    waits(&answer);
    rig.advance(1);
    waits(&answer);
    rig.list(usual());
    let listed = ok(&answer).unwrap();
    assert_eq!(listed.lines().count(), 3);
    // One whose groups never come is told so.
    let mut late = Rig::new();
    let answer = late.group(GroupRequest::Activate("Builders".into()));
    late.advance(10);
    assert!(error(&answer).contains("has not told the avatar's groups within 10 s"));
    assert_eq!(late.calls(), []);
}

// ---------------------------------------------------------------- list

#[test]
fn group_list_lists_the_groups_with_their_flags() {
    let mut rig = Rig::listed();
    let listed = ok(&rig.group(GroupRequest::List)).unwrap();
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(
        lines,
        [
            format!("{}\tactive,invite,notices\tBuilders", builders()),
            format!("{}\tnotices\tDancers", dancers()),
            format!("{}\tinvite\tNight Owls", night_owls()),
        ]
    );
    rig.list(Vec::new());
    assert_eq!(ok(&rig.group(GroupRequest::List)), None);
    // Listing leaves no line in the log.
    rig.told();
    ok(&rig.group(GroupRequest::List));
    assert_eq!(rig.told(), Vec::<String>::new());
}

#[test]
fn the_login_and_the_changes_of_the_groups_are_logged() {
    let mut rig = Rig::new();
    rig.commands.active_group(dancers(), "", "Dancers");
    rig.list(usual());
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("3 groups") && line.contains("\"Dancers\"")),
        "{told:?}"
    );
    rig.commands.group_dropped(dancers(), rig.now);
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("Dancers") && line.contains("no longer in")),
        "{told:?}"
    );
    rig.list(usual());
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("Dancers") && line.contains("is in")),
        "{told:?}"
    );
}

// ------------------------------------------------------------ activate

#[test]
fn group_activate_waits_for_the_grid_to_say_so() {
    let mut rig = Rig::listed();
    let answer = rig.line("group activate dancers");
    assert_eq!(rig.calls(), [Call::Activate(dancers())]);
    waits(&answer);
    // Another group's word does not answer it.
    rig.commands.active_group(builders(), "", "Builders");
    waits(&answer);
    rig.commands.active_group(dancers(), "Member", "Dancers");
    let text = ok(&answer).unwrap();
    assert!(text.contains("\"Dancers\"") && text.contains("Member"), "{text}");
    let told = rig.told();
    assert!(
        told.iter()
            .any(|line| line.contains("Dancers") && line.contains(&dancers().to_string()) && line.contains("active group now")),
        "{told:?}"
    );
    let listed = ok(&rig.group(GroupRequest::List)).unwrap();
    assert!(listed.contains(&format!("{}\tactive,notices\tDancers", dancers())), "{listed}");
    assert!(listed.contains(&format!("{}\tinvite,notices\tBuilders", builders())), "{listed}");
}

#[test]
fn group_activate_none_and_a_name_of_two_words() {
    let mut rig = Rig::listed();
    let answer = rig.line("group activate none");
    assert_eq!(rig.calls(), [Call::Activate(Uuid::nil())]);
    rig.commands.active_group(Uuid::nil(), "", "");
    assert_eq!(ok(&answer).unwrap(), "no group is active now");
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("active"));
    let answer = rig.line("group activate Night Owls");
    assert_eq!(rig.calls(), [Call::Activate(night_owls())]);
    rig.advance(10);
    assert!(error(&answer).contains("has not confirmed"));
    assert!(rig.told().iter().any(|line| line.contains("not confirmed")));
}

#[test]
fn naming_a_group_that_is_not_one() {
    let mut rig = Rig::listed();
    assert!(error(&rig.line("group activate Build")).contains("\"Build\""));
    let other = Uuid::from_bytes([7; 16]);
    let reason = error(&rig.line(&format!("group chat {other} hello")));
    assert!(
        reason.contains("not in the group") && reason.contains(&other.to_string()),
        "{reason}"
    );
    let (a, b) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
    rig.list(vec![member(a, "Friends", 0, true), member(b, "Friends", 0, true)]);
    let reason = error(&rig.line("group chat Friends hello"));
    assert!(reason.contains(&a.to_string()) && reason.contains(&b.to_string()), "{reason}");
    assert_eq!(rig.calls(), []);
}

// -------------------------------------------------------------- invite

#[test]
fn group_invite_invites_an_avatar_by_name() {
    let mut rig = Rig::listed();
    let answer = rig.group(GroupRequest::Invite {
        group: "Builders".into(),
        avatar: "Jane Doe".into(),
    });
    let query = rig.lookup("Jane Doe");
    waits(&answer);
    rig.commands
        .avatars_found(query, &[(jane(), "Jane Doe".into())], Some(&mut rig.grid), rig.now);
    assert_eq!(rig.calls(), [Call::Invite(builders(), jane())]);
    let text = ok(&answer).unwrap();
    assert!(
        text.contains("Jane Doe") && text.contains("Builders") && text.contains("does not report"),
        "{text}"
    );
    assert!(rig
        .told()
        .iter()
        .any(|line| line.contains("group invite") && line.contains("Jane Doe")));
}

#[test]
fn group_invite_is_refused_without_the_right_or_for_the_bot_itself() {
    let mut rig = Rig::listed();
    let reason = error(&rig.group(GroupRequest::Invite {
        group: "Dancers".into(),
        avatar: "Jane Doe".into(),
    }));
    assert!(reason.contains("may not invite into") && reason.contains("Dancers"), "{reason}");
    let own = rig.grid.own;
    let reason = error(&rig.group(GroupRequest::Invite {
        group: "Builders".into(),
        avatar: own.to_string(),
    }));
    assert!(reason.contains("own avatar"), "{reason}");
    assert_eq!(rig.calls(), []);
    let answer = rig.group(GroupRequest::Invite {
        group: "Builders".into(),
        avatar: "Nobody Here".into(),
    });
    let query = rig.lookup("Nobody Here");
    rig.commands.avatars_found(query, &[], Some(&mut rig.grid), rig.now);
    assert!(error(&answer).contains("Nobody Here"));
    assert_eq!(rig.calls(), []);
}

// ---------------------------------------------------------------- info

#[test]
fn group_info_of_a_group_the_avatar_is_not_in() {
    let mut rig = Rig::listed();
    let other = Uuid::from_bytes([0xAB; 16]);
    let answer = rig.line(&format!("group info {other}"));
    assert_eq!(rig.calls(), [Call::GroupProfile(other)]);
    rig.profile(profile(other, "Builders", 250, true));
    // The founder is named first.
    assert!(rig.commands.take_name_lookups().contains(&jane()));
    waits(&answer);
    rig.named(jane(), "Jane Doe");
    let text = ok(&answer).unwrap();
    for wanted in [
        "name: Builders",
        "founder: Jane Doe",
        "fee: L$250",
        "enrollment: open",
        "member: no",
    ] {
        assert!(text.contains(wanted), "{wanted}: {text}");
    }
    assert!(!text.contains("title:"));
    assert!(text.ends_with("charter: We build."));
    assert_eq!(rig.told(), Vec::<String>::new());
}

#[test]
fn group_info_names_the_founder_by_uuid_when_the_grid_does_not() {
    let mut rig = Rig::listed();
    let answer = rig.line("group info Builders");
    rig.profile(Profile {
        charter: "One\nTwo\nThree".into(),
        title: "Officer".into(),
        ..profile(builders(), "Builders", 0, false)
    });
    rig.advance(5);
    let text = ok(&answer).unwrap();
    assert!(text.contains(&format!("founder: {}", jane())), "{text}");
    assert!(text.contains("fee: none") && text.contains("member: yes") && text.contains("title: Officer"));
    assert!(text.ends_with("charter: One\\nTwo\\nThree"), "{text}");
}

#[test]
fn a_profile_the_grid_does_not_answer_for_fails_after_ten_seconds() {
    let mut rig = Rig::listed();
    let other = Uuid::from_bytes([0xAB; 16]);
    let answer = rig.line(&format!("group info {other}"));
    rig.advance(9);
    waits(&answer);
    rig.advance(1);
    let reason = error(&answer);
    assert!(reason.contains("knows no group") && reason.contains(&other.to_string()), "{reason}");
}

#[test]
fn info_then_join_asks_for_the_profile_once() {
    let mut rig = Rig::listed();
    let other = Uuid::from_bytes([0xAB; 16]);
    let info = rig.line(&format!("group info {other}"));
    assert_eq!(rig.calls(), [Call::GroupProfile(other)]);
    rig.profile(profile(other, "Fishers", 0, true));
    rig.advance(5);
    ok(&info);
    let join = rig.line(&format!("group join {other}"));
    assert_eq!(rig.calls(), [Call::Join(other)]);
    waits(&join);
}

// ---------------------------------------------------------------- join

#[test]
fn group_join_of_a_free_open_group() {
    let mut rig = Rig::new();
    rig.list(vec![member(builders(), "Builders", 0, true)]);
    let answer = rig.line(&format!("group join {}", dancers()));
    assert_eq!(rig.calls(), [Call::GroupProfile(dancers())]);
    rig.profile(profile(dancers(), "Dancers", 0, true));
    assert_eq!(rig.calls(), [Call::Join(dancers())]);
    waits(&answer);
    // The grid's yes comes before the list with the group: the command
    // waits for the list, so that one naming the group right after the
    // answer finds it.
    rig.commands.joined(dancers(), true, rig.now);
    waits(&answer);
    rig.list(vec![member(builders(), "Builders", 0, true), member(dancers(), "Dancers", 0, true)]);
    assert!(ok(&answer).unwrap().contains("joined \"Dancers\""));
    let _ = rig.line("group activate Dancers");
    assert_eq!(rig.calls(), [Call::Activate(dancers())]);
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("group join") && line.contains("sent")),
        "{told:?}"
    );
    assert!(told.iter().any(|line| line.contains("confirmed")), "{told:?}");
}

#[test]
fn a_join_is_confirmed_by_the_list_too() {
    let mut rig = Rig::new();
    rig.list(vec![]);
    let answer = rig.line(&format!("group join {}", dancers()));
    rig.profile(profile(dancers(), "Dancers", 0, true));
    rig.list(vec![member(dancers(), "Dancers", 0, true)]);
    assert!(ok(&answer).unwrap().contains("Dancers"));
}

#[test]
fn a_join_refused_or_unconfirmed_is_an_error() {
    let mut rig = Rig::new();
    rig.list(vec![]);
    let answer = rig.line(&format!("group join {}", dancers()));
    rig.profile(profile(dancers(), "Dancers", 0, true));
    rig.commands.joined(dancers(), false, rig.now);
    // The list is asked for, and decides: the avatar is not in it.
    waits(&answer);
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::GroupProfile(dancers()), Call::Join(dancers()), Call::AskGroups]);
    rig.list(vec![]);
    let reason = error(&answer);
    assert!(reason.contains("refused") && reason.contains("Dancers"), "{reason}");
    // Without the list, the refusal after 10 seconds.
    let answer = rig.line(&format!("group join {}", dancers()));
    rig.commands.joined(dancers(), false, rig.now);
    rig.advance(10);
    let reason = error(&answer);
    assert!(reason.contains("refused") && reason.contains("has not told"), "{reason}");
    let answer = rig.line(&format!("group join {}", dancers()));
    rig.advance(14);
    waits(&answer);
    rig.advance(1);
    let reason = error(&answer);
    assert!(reason.contains("has not confirmed") && !reason.contains("accepted"), "{reason}");
    // The grid said yes and never listed the group.
    let answer = rig.line(&format!("group join {}", dancers()));
    rig.commands.joined(dancers(), true, rig.now);
    rig.advance(15);
    let reason = error(&answer);
    assert!(
        reason.contains("has not confirmed") && reason.contains("accepted the join"),
        "{reason}"
    );
}

#[test]
fn group_join_is_refused_for_a_closed_group_or_a_member() {
    let mut rig = Rig::listed();
    let other = Uuid::from_bytes([0xAB; 16]);
    let answer = rig.line(&format!("group join {other}"));
    rig.profile(profile(other, "Club", 0, false));
    assert!(error(&answer).contains("takes an invitation"));
    // A member by what the bot holds: the grid is asked first.
    let answer = rig.line(&format!("group join {}", builders()));
    waits(&answer);
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::GroupProfile(other), Call::AskGroups]);
    rig.list(usual());
    assert!(error(&answer).contains("already"));
    assert_eq!(rig.calls(), []);
}

// ------------------------------------------- groups the grid did not drop

/// What Second Life does: the avatar is ejected from `Dancers` and the bot
/// is told nothing but an IM.
fn ejected_unsaid() -> Vec<Membership> {
    usual().into_iter().filter(|group| group.id != dancers()).collect()
}

#[test]
fn a_leave_refused_for_a_group_the_avatar_was_ejected_from_succeeds() {
    let mut rig = Rig::listed();
    let answer = rig.line("group leave Dancers");
    rig.commands.left(dancers(), false, rig.now);
    waits(&answer);
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::Leave(dancers()), Call::AskGroups]);
    rig.list(ejected_unsaid());
    let text = ok(&answer).unwrap();
    assert!(text.contains("not in \"Dancers\"") && text.contains("any more"), "{text}");
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("no longer in") && line.contains("Dancers")),
        "{told:?}"
    );
    assert!(
        told.iter().any(|line| line.contains("group leave") && line.contains("confirmed")),
        "{told:?}"
    );
}

#[test]
fn a_join_into_a_group_the_avatar_was_ejected_from_asks_first() {
    let mut rig = Rig::listed();
    let answer = rig.line(&format!("group join {}", dancers()));
    waits(&answer);
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::AskGroups]);
    rig.list(ejected_unsaid());
    // Not a member: the join goes on, with the profile.
    assert_eq!(rig.calls(), [Call::GroupProfile(dancers())]);
    rig.profile(profile(dancers(), "Dancers", 0, true));
    assert_eq!(rig.calls(), [Call::Join(dancers())]);
    rig.list(usual());
    assert!(ok(&answer).unwrap().contains("joined \"Dancers\""));
    // No answer to the question: the command fails as an early one does.
    let answer = rig.line(&format!("group join {}", builders()));
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::AskGroups]);
    rig.advance(10);
    assert!(error(&answer).contains("has not told"));
}

#[test]
fn a_join_refused_for_a_group_the_avatar_is_in_succeeds() {
    let mut rig = Rig::paying(config());
    rig.list(usual());
    // The bot holds the avatar out of Fishers; the grid knows better.
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    rig.commands.joined(fishers(), false, rig.now);
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::GroupProfile(fishers()), Call::Join(fishers()), Call::AskGroups]);
    rig.list(with_fishers());
    let text = ok(&answer).unwrap();
    assert!(
        text.contains("already") && text.contains("Fishers") && text.contains("was not paid"),
        "{text}"
    );
    let records = rig.commands.take_transactions();
    assert_eq!((records[0].kind, records[0].counted), (TxKind::Failed, 0));
}

// --------------------------------------------------------------- leave

#[test]
fn group_leave_leaves_a_group_by_name() {
    let mut rig = Rig::listed();
    let answer = rig.line("group leave dancers");
    assert_eq!(rig.calls(), [Call::Leave(dancers())]);
    waits(&answer);
    let told = rig.told();
    assert!(
        told.iter()
            .any(|line| line.contains("group leave") && line.contains(&dancers().to_string()) && line.contains("sent")),
        "{told:?}"
    );
    // Another group's answer does not end it.
    rig.commands.left(builders(), true, rig.now);
    waits(&answer);
    rig.commands.left(dancers(), true, rig.now);
    assert_eq!(ok(&answer).unwrap(), format!("left \"Dancers\" <{}>", dancers()));
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("group leave") && line.contains("confirmed")),
        "{told:?}"
    );
    assert!(
        told.iter().any(|line| line.contains("no longer in") && line.contains("Dancers")),
        "{told:?}"
    );
    // The list right after the answer, before the grid's drop.
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    // The drop that follows changes nothing and says nothing.
    rig.commands.group_dropped(dancers(), rig.now);
    assert_eq!(rig.told(), Vec::<String>::new());
}

#[test]
fn a_leave_is_confirmed_by_the_drop_or_the_list() {
    let mut rig = Rig::listed();
    let answer = rig.line("group leave Night Owls");
    assert_eq!(rig.calls(), [Call::Leave(night_owls())]);
    rig.commands.group_dropped(night_owls(), rig.now);
    assert!(ok(&answer).unwrap().contains("Night Owls"));
    let answer = rig.line("group leave Dancers");
    // A list that still has the group does not confirm it.
    rig.list(usual().into_iter().filter(|group| group.id != night_owls()).collect());
    waits(&answer);
    rig.list(vec![member(builders(), "Builders", POWER_INVITE, true)]);
    assert!(ok(&answer).unwrap().contains("Dancers"));
}

#[test]
fn group_leave_of_a_group_the_avatar_is_not_in() {
    let mut rig = Rig::listed();
    let other = Uuid::from_bytes([7; 16]);
    let reason = error(&rig.line(&format!("group leave {other}")));
    assert!(
        reason.contains("not in the group") && reason.contains(&other.to_string()),
        "{reason}"
    );
    assert!(error(&rig.line("group leave Build")).contains("\"Build\""));
    assert_eq!(rig.calls(), []);
}

#[test]
fn leaving_the_active_group_leaves_none_active() {
    let mut rig = Rig::listed();
    let answer = rig.line("group leave Builders");
    assert_eq!(rig.calls(), [Call::Leave(builders())]);
    rig.commands.left(builders(), true, rig.now);
    assert!(ok(&answer).unwrap().contains("Builders"));
    // At once, and after the drop and the update the grid then sends.
    let listed = ok(&rig.group(GroupRequest::List)).unwrap();
    assert!(!listed.contains("Builders") && !listed.contains("active"), "{listed}");
    rig.commands.group_dropped(builders(), rig.now);
    rig.commands.active_group(Uuid::nil(), "", "");
    let listed = ok(&rig.group(GroupRequest::List)).unwrap();
    assert!(!listed.contains("Builders") && !listed.contains("active"), "{listed}");
}

#[test]
fn a_leave_refused_or_unconfirmed_is_an_error() {
    let mut rig = Rig::listed();
    let answer = rig.line("group leave Builders");
    rig.commands.left(builders(), false, rig.now);
    // The list is asked for, and decides: the avatar is still in it.
    waits(&answer);
    rig.advance(0);
    assert_eq!(rig.calls(), [Call::Leave(builders()), Call::AskGroups]);
    rig.list(usual());
    let reason = error(&answer);
    assert!(reason.contains("refused") && reason.contains("Builders"), "{reason}");
    assert!(ok(&rig.group(GroupRequest::List)).unwrap().contains("Builders"));
    assert!(rig
        .told()
        .iter()
        .any(|line| line.contains("group leave") && line.contains("not confirmed")));
    let answer = rig.line("group leave Builders");
    rig.advance(14);
    waits(&answer);
    rig.advance(1);
    assert!(error(&answer).contains("has not confirmed"));
    // And with the session.
    let answer = rig.line("group leave Builders");
    rig.commands.groups_lost();
    assert!(error(&answer).contains("session was lost"));
}

#[test]
fn a_leave_forgets_the_chat_session() {
    let mut rig = Rig::listed();
    let answer = rig.line("group chat Dancers hello");
    assert_eq!(rig.calls(), [Call::StartSession(dancers())]);
    rig.commands.group_session(dancers(), true, "", Some(&mut rig.grid), rig.now);
    ok(&answer);
    rig.calls();
    rig.line("group leave Dancers");
    rig.commands.group_dropped(dancers(), rig.now);
    assert_eq!(rig.calls(), [Call::Leave(dancers())]);
    // In again, from a viewer a while later: the session is entered anew.
    rig.advance(10);
    rig.list(vec![member(dancers(), "Dancers", 0, true)]);
    rig.line("group chat Dancers hello again");
    assert_eq!(rig.calls(), [Call::StartSession(dancers())]);
}

// ------------------------------------------------- lists that come late

/// `Dancers` left and joined again, with the profile the join needs.
fn left_and_joined_again(rig: &mut Rig) {
    let answer = rig.line("group leave Dancers");
    rig.commands.left(dancers(), true, rig.now);
    ok(&answer);
    let answer = rig.line(&format!("group join {}", dancers()));
    rig.profile(profile(dancers(), "Dancers", 0, true));
    rig.commands.joined(dancers(), true, rig.now);
    rig.list(usual());
    ok(&answer);
    assert_eq!(
        rig.calls(),
        [Call::Leave(dancers()), Call::GroupProfile(dancers()), Call::Join(dancers())]
    );
    rig.told();
}

#[test]
fn a_list_or_a_drop_from_before_a_rejoin_is_held_back() {
    let mut rig = Rig::listed();
    left_and_joined_again(&mut rig);
    // What the grid made of the leave arrives after the join.
    let without: Vec<Membership> = usual().into_iter().filter(|group| group.id != dancers()).collect();
    rig.list(without.clone());
    rig.commands.group_dropped(dancers(), rig.now);
    assert!(ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    assert_eq!(rig.told(), Vec::<String>::new());
    let answer = rig.line("group activate Dancers");
    assert_eq!(rig.calls(), [Call::Activate(dancers())]);
    rig.commands.active_group(dancers(), "", "Dancers");
    ok(&answer);
    // The moment over, the list is asked for again, and its answer holds.
    rig.advance(4);
    assert_eq!(rig.calls(), []);
    rig.advance(1);
    assert_eq!(rig.calls(), [Call::AskGroups]);
    rig.list(usual());
    assert!(ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    rig.advance(10);
    assert_eq!(rig.calls(), []);
}

#[test]
fn a_list_from_before_a_leave_is_held_back() {
    let mut rig = Rig::listed();
    let answer = rig.line("group leave Dancers");
    rig.commands.left(dancers(), true, rig.now);
    ok(&answer);
    rig.told();
    rig.list(usual());
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    assert_eq!(rig.told(), Vec::<String>::new());
    // Other changes in the same list are taken.
    let mut more = usual();
    more.push(member(fishers(), "Fishers", 0, true));
    rig.list(more);
    let listed = ok(&rig.group(GroupRequest::List)).unwrap();
    assert!(listed.contains("Fishers") && !listed.contains("Dancers"), "{listed}");
    rig.advance(5);
    assert_eq!(rig.calls(), [Call::Leave(dancers()), Call::AskGroups]);
}

#[test]
fn the_grid_is_believed_again_after_the_moment() {
    let mut rig = Rig::listed();
    left_and_joined_again(&mut rig);
    // An ejection a while after the join is taken at once.
    rig.advance(5);
    assert_eq!(rig.calls(), []);
    rig.commands.group_dropped(dancers(), rig.now);
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    rig.advance(10);
    assert_eq!(rig.calls(), []);
}

#[test]
fn an_ejection_right_after_a_join_is_taken_once_asked_again() {
    let mut rig = Rig::listed();
    left_and_joined_again(&mut rig);
    rig.commands.group_dropped(dancers(), rig.now);
    assert!(ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    rig.advance(5);
    assert_eq!(rig.calls(), [Call::AskGroups]);
    rig.list(usual().into_iter().filter(|group| group.id != dancers()).collect());
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
    assert!(rig
        .told()
        .iter()
        .any(|line| line.contains("no longer in") && line.contains("Dancers")));
}

#[test]
fn a_new_join_or_leave_is_not_held_back() {
    let mut rig = Rig::listed();
    left_and_joined_again(&mut rig);
    // Leaving again at once: the drop is the answer, not a late word.
    let answer = rig.line("group leave Dancers");
    rig.commands.group_dropped(dancers(), rig.now);
    ok(&answer);
    assert!(!ok(&rig.group(GroupRequest::List)).unwrap().contains("Dancers"));
}

#[test]
fn a_sum_for_a_free_group_pays_nothing() {
    let mut rig = Rig::new();
    rig.list(vec![]);
    let answer = rig.line(&format!("group join {} --pay 100", dancers()));
    rig.profile(profile(dancers(), "Dancers", 0, true));
    assert_eq!(rig.calls(), [Call::GroupProfile(dancers()), Call::Join(dancers())]);
    rig.commands.joined(dancers(), true, rig.now);
    rig.list(vec![member(dancers(), "Dancers", 0, true)]);
    ok(&answer);
    assert!(rig.commands.take_transactions().is_empty());
}

// ------------------------------------------------------------ group fees

/// The usual groups and `Fishers`, as the grid lists them after the join.
fn with_fishers() -> Vec<Membership> {
    let mut list = usual();
    list.push(member(fishers(), "Fishers", 0, true));
    list
}

/// Starts a join of `Fishers` with its fee of L$250 and `--pay`, via `via`.
fn join_with_fee(rig: &mut Rig, via: Interface, pay: &str) -> Receiver<Response> {
    let request = Request::decode(format!("group\tjoin\t{}\t--pay\t{pay}", fishers()).as_bytes()).unwrap();
    let answer = rig.run_via(via, request);
    rig.profile(profile(fishers(), "Fishers", 250, true));
    answer
}

fn report(rig: &mut Rig, success: bool, amount: i32, balance: i32) {
    let info = crate::udp::messages::TransactionInfo {
        kind: 1004,
        source: rig.grid.own,
        source_group: false,
        dest: fishers(),
        dest_group: true,
        amount,
        item: String::new(),
    };
    let description = if success { "" } else { "Insufficient funds" };
    let own = rig.grid.own;
    rig.commands
        .money_report(Uuid::nil(), balance, success, description, Some(&info), own);
}

#[test]
fn a_fee_is_paid_by_joining_with_pay() {
    let mut rig = Rig::paying(config());
    let answer = join_with_fee(&mut rig, Interface::Ctl, "300");
    assert_eq!(rig.calls(), [Call::GroupProfile(fishers()), Call::Join(fishers())]);
    report(&mut rig, true, 250, 750);
    rig.commands.joined(fishers(), true, rig.now);
    waits(&answer);
    rig.list(with_fishers());
    let text = ok(&answer).unwrap();
    assert!(
        text.contains("Fishers") && text.contains("L$250") && text.contains("L$750"),
        "{text}"
    );
    let records = rig.commands.take_transactions();
    assert_eq!(records.len(), 1);
    assert_eq!(
        (
            records[0].kind,
            records[0].amount,
            records[0].code,
            records[0].party,
            records[0].party_group
        ),
        (TxKind::Paid, 250, 1004, Some(fishers()), true)
    );
    assert_eq!(records[0].text, "Fee to join Fishers");
    assert!(rig.commands.limit_text().contains("750"), "{}", rig.commands.limit_text());
}

#[test]
fn a_fee_reported_twice_is_recorded_once() {
    // Second Life sends the report of a fee twice, with one transaction id.
    let mut rig = Rig::paying(config());
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    rig.calls();
    let info = crate::udp::messages::TransactionInfo {
        kind: 1004,
        source: rig.grid.own,
        source_group: false,
        dest: fishers(),
        dest_group: true,
        amount: 250,
        item: String::new(),
    };
    let (own, id) = (rig.grid.own, Uuid::from_bytes([0x7A; 16]));
    let text = "You paid Fishers L$250.";
    rig.commands.money_report(id, 750, true, text, Some(&info), own);
    rig.commands.money_report(id, 750, true, text, Some(&info), own);
    rig.list(with_fishers());
    assert!(ok(&answer).unwrap().contains("L$750"));
    let records = rig.commands.take_transactions();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!((records[0].kind, records[0].counted), (TxKind::Paid, 250));
    // Another transaction is recorded.
    rig.commands
        .money_report(Uuid::from_bytes([0x7B; 16]), 740, true, text, Some(&info), own);
    assert_eq!(rig.commands.take_transactions().len(), 1);
}

#[test]
fn a_fee_without_a_sum_or_with_too_little_sends_nothing() {
    let mut rig = Rig::paying(config());
    let answer = rig.line(&format!("group join {}", fishers()));
    rig.profile(profile(fishers(), "Fishers", 250, true));
    let reason = error(&answer);
    assert!(reason.contains("L$250") && reason.contains("--pay"), "{reason}");
    let answer = join_with_fee(&mut rig, Interface::Ctl, "100");
    assert!(error(&answer).contains("L$250"));
    assert_eq!(rig.calls(), [Call::GroupProfile(fishers())]);
}

#[test]
fn a_fee_is_paid_only_from_an_interface_that_may_pay() {
    let mut rig = Rig::paying(PaymentsConfig {
        perms: [true, false, false],
        ..config()
    });
    let answer = join_with_fee(&mut rig, Interface::Web, "250");
    assert!(error(&answer).contains("payments are not enabled for `web`"));
    let answer = join_with_fee(&mut rig, Interface::Xmpp, "250");
    assert!(error(&answer).contains("`xmpp`"));
    let mut none = Rig::listed();
    let answer = join_with_fee(&mut none, Interface::Ctl, "250");
    assert!(error(&answer).contains("no `[payments]` table"));
    assert!(!rig.calls().iter().chain(&none.calls()).any(|call| matches!(call, Call::Join(_))));
}

#[test]
fn a_fee_is_held_to_the_lists_the_limit_and_the_balance() {
    // Only the group's UUID matches it.
    let mut rig = Rig::paying(PaymentsConfig {
        whitelist: Some(vec![Entry::Id(builders())]),
        ..config()
    });
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    assert!(error(&answer).contains("whitelist"));
    let mut rig = Rig::paying(PaymentsConfig {
        blacklist: vec![Entry::Id(fishers())],
        ..config()
    });
    assert!(error(&join_with_fee(&mut rig, Interface::Ctl, "250")).contains("blacklist"));
    // Over the limit.
    let mut rig = Rig::paying(PaymentsConfig {
        limit: Some(100),
        ..config()
    });
    let reason = error(&join_with_fee(&mut rig, Interface::Ctl, "250"));
    assert!(reason.contains("L$100"), "{reason}");
    // Not enough money.
    let mut rig = Rig::paying(config());
    report_balance(&mut rig, 100);
    let reason = error(&join_with_fee(&mut rig, Interface::Ctl, "250"));
    assert!(reason.contains("has L$100"), "{reason}");
    assert!(!rig.calls().iter().any(|call| matches!(call, Call::Join(_))));
}

fn report_balance(rig: &mut Rig, balance: i32) {
    let own = rig.grid.own;
    rig.commands.money_report(Uuid::nil(), balance, true, "", None, own);
}

#[test]
fn one_payment_at_a_time_with_fees() {
    let mut rig = Rig::paying(config());
    let first = join_with_fee(&mut rig, Interface::Ctl, "250");
    waits(&first);
    let pay = rig.pay_jane();
    let reason = error(&pay);
    assert!(reason.contains("under way") && reason.contains("Fishers"), "{reason}");
    // Even once the join is confirmed, until the report comes.
    rig.commands.joined(fishers(), true, rig.now);
    rig.list(with_fishers());
    let text = ok(&first).unwrap();
    assert!(text.contains("counts as paid"), "{text}");
    let pay = rig.pay_jane();
    assert!(error(&pay).contains("under way"));
    // The 15 seconds over, the fee is unconfirmed and keeps counting.
    rig.advance(15);
    let records = rig.commands.take_transactions();
    assert_eq!(records.len(), 1);
    assert_eq!(
        (records[0].kind, records[0].counted, records[0].balance),
        (TxKind::Unconfirmed, 250, None)
    );
    assert_eq!(records[0].text, "Fee to join Fishers");
    assert!(rig.commands.limit_text().contains("750"));
}

#[test]
fn a_fee_reported_and_no_join_is_an_error_when_the_time_is_up() {
    let mut rig = Rig::paying(config());
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    report(&mut rig, true, 250, 750);
    waits(&answer);
    rig.advance(15);
    let reason = error(&answer);
    assert!(reason.contains("was paid") && reason.contains("has not confirmed"), "{reason}");
}

#[test]
fn a_refused_join_or_a_failed_fee_stops_counting() {
    let mut rig = Rig::paying(config());
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    rig.commands.joined(fishers(), false, rig.now);
    rig.list(usual());
    assert!(error(&answer).contains("refused"));
    let records = rig.commands.take_transactions();
    assert_eq!((records[0].kind, records[0].counted), (TxKind::Failed, 0));
    assert!(records[0].text.starts_with("Fee to join Fishers: "), "{}", records[0].text);
    assert!(rig.commands.limit_text().contains("1000"));
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    report(&mut rig, false, 250, 100);
    assert!(error(&answer).contains("Insufficient funds"));
    let records = rig.commands.take_transactions();
    assert_eq!((records[0].kind, records[0].counted), (TxKind::Failed, 0));
    assert!(rig.commands.limit_text().contains("1000"));
}

#[test]
fn nothing_heard_of_a_fee_leaves_it_counting() {
    let mut rig = Rig::paying(config());
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    rig.advance(15);
    let reason = error(&answer);
    assert!(reason.contains("was sent") && reason.contains("counts as paid"), "{reason}");
    assert_eq!(rig.commands.take_transactions()[0].kind, TxKind::Unconfirmed);
    // The session's loss and the bot's stop leave the same record.
    let answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    rig.commands.session_lost();
    assert!(error(&answer).contains("counts as paid"));
    assert_eq!(rig.commands.take_transactions()[0].kind, TxKind::Unconfirmed);
    let mut rig = Rig::paying(config());
    let _answer = join_with_fee(&mut rig, Interface::Ctl, "250");
    rig.commands.money_stop();
    assert_eq!(rig.commands.take_transactions()[0].kind, TxKind::Unconfirmed);
}

// ---------------------------------------------------------- group chat

#[test]
fn the_first_line_enters_the_session_and_later_ones_go_at_once() {
    let mut rig = Rig::listed();
    let answer = rig.line("group chat Builders see you at eight");
    assert_eq!(rig.calls(), [Call::StartSession(builders())]);
    waits(&answer);
    let second = rig.line("group chat Builders and again");
    assert_eq!(rig.calls(), []);
    rig.commands.group_session(builders(), true, "", Some(&mut rig.grid), rig.now);
    assert_eq!(
        rig.calls(),
        [
            Call::GroupChat(builders(), "see you at eight".into()),
            Call::GroupChat(builders(), "and again".into())
        ]
    );
    assert!(ok(&answer).unwrap().contains("Builders"));
    ok(&second);
    let records = rig.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        (records[0].kind, records[0].group, records[0].group_name.as_str(), records[0].time),
        (grouplog::Kind::ChatOut, builders(), "Builders", T0)
    );
    assert_eq!(records[0].avatar, Some(rig.grid.own));
    let answer = rig.line("group chat Builders now");
    assert_eq!(rig.calls(), [Call::GroupChat(builders(), "now".into())]);
    ok(&answer);
    let told = rig.told();
    assert!(told.iter().any(|line| line.contains("entering the chat session")), "{told:?}");
    assert!(
        told.iter().any(|line| line.contains("group chat") && line.contains("sent")),
        "{told:?}"
    );
}

#[test]
fn a_session_refused_or_unanswered_says_nothing() {
    let mut rig = Rig::listed();
    let answer = rig.line("group chat Builders hello");
    rig.calls();
    rig.commands
        .group_session(builders(), false, "You are not a member", Some(&mut rig.grid), rig.now);
    assert!(error(&answer).contains("You are not a member"));
    assert_eq!(rig.calls(), []);
    assert!(rig.told().iter().any(|line| line.contains("You are not a member")));
    let answer = rig.line("group chat Builders hello");
    rig.advance(10);
    assert!(error(&answer).contains("within 10 s"));
    // A late answer sends nothing.
    rig.commands.group_session(builders(), true, "", Some(&mut rig.grid), rig.now);
    assert_eq!(rig.calls(), [Call::StartSession(builders())]);
    assert!(rig.records().is_empty());
}

#[test]
fn parked_lines_fail_with_the_session() {
    let mut rig = Rig::listed();
    let answer = rig.line("group chat Builders hello");
    rig.commands.session_lost();
    assert!(error(&answer).contains("session was lost"));
    assert!(rig.records().is_empty());
}

#[test]
fn group_chat_refuses_what_is_empty_or_too_long() {
    let mut rig = Rig::listed();
    let long = "x".repeat(2000);
    let reason = error(&rig.group(GroupRequest::Chat {
        group: "Builders".into(),
        text: long,
    }));
    assert!(reason.contains("1023") && reason.contains("Builders"), "{reason}");
    assert!(error(&rig.group(GroupRequest::Chat {
        group: "Builders".into(),
        text: " ".into(),
    }))
    .contains("no text"));
    assert_eq!(rig.calls(), []);
    assert!(rig.records().is_empty());
    let refusals = crate::commands::take_refusals();
    assert!(
        refusals.iter().any(|line| line.contains("Builders") && line.contains("1023")),
        "{refusals:?}"
    );
}

#[test]
fn lines_of_the_avatars_groups_are_recorded_and_others_dropped() {
    let mut rig = Rig::listed();
    rig.commands.session_offered(builders(), Some(&mut rig.grid));
    assert_eq!(rig.calls(), [Call::TakeUp(builders())]);
    rig.commands
        .group_line(builders(), jane(), "Jane Doe", "anyone around?", Some(&mut rig.grid));
    rig.commands
        .group_line(builders(), jane(), "Jane Doe", "say hello", Some(&mut rig.grid));
    // A conference's line.
    let conference = Uuid::from_bytes([0xCC; 16]);
    rig.commands.session_offered(conference, Some(&mut rig.grid));
    rig.commands
        .group_line(conference, jane(), "Jane Doe", "hi both", Some(&mut rig.grid));
    let records = rig.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        (
            records[0].kind,
            records[0].group_name.as_str(),
            records[0].avatar,
            records[0].avatar_name.as_str()
        ),
        (grouplog::Kind::Chat, "Builders", Some(jane()), "Jane Doe")
    );
    assert_eq!(records[0].text, "anyone around?");
    // Nothing is said, and the session counts as entered.
    assert_eq!(rig.calls(), []);
    let answer = rig.line("group chat Builders hi");
    assert_eq!(rig.calls(), [Call::GroupChat(builders(), "hi".into())]);
    ok(&answer);
    // Without the service nothing is taken up.
    rig.grid.no_chat_sessions = true;
    rig.commands.session_offered(dancers(), Some(&mut rig.grid));
    assert_eq!(rig.calls(), []);
}

#[test]
fn notices_are_recorded_with_their_parts() {
    let mut rig = Rig::listed();
    let bucket = crate::grouplog::tests::bucket(Some("Agenda"));
    rig.commands
        .group_notice(builders(), Some(jane()), "Jane Doe", "Meeting|Tonight at eight", &bucket, None);
    rig.commands.group_notice(
        builders(),
        None,
        "Jane Doe",
        "Tonight",
        &crate::grouplog::tests::bucket(None),
        Some(1_796_122_800),
    );
    let records = rig.records();
    assert_eq!(records[0].kind, grouplog::Kind::Notice);
    assert_eq!(
        (
            records[0].subject.as_str(),
            records[0].text.as_str(),
            records[0].attachment.as_str()
        ),
        ("Meeting", "Tonight at eight", "Agenda")
    );
    assert_eq!(
        (records[0].group_name.as_str(), records[0].time, records[0].offline),
        ("Builders", T0, false)
    );
    assert_eq!(
        (records[1].subject.as_str(), records[1].time, records[1].offline),
        ("", 1_796_122_800, true)
    );
    assert_eq!(records[1].avatar, None);
    // One marked as stored that was sent a moment ago is live, as OpenSim
    // sends every notice.
    rig.commands
        .group_notice(builders(), None, "Jane Doe", "Now", &[], Some((T0 - 2) as u32));
    let records = rig.records();
    assert_eq!((records[0].time, records[0].offline), (T0, false));
    // Nothing is answered or taken.
    assert_eq!(rig.calls(), []);
}

// ------------------------------------------------------------------ RLV

/// A collar holds these restrictions.
fn hold(rig: &mut Rig, commands: &str) {
    let mut kept = Restrictions::default();
    let collar = crate::rlv::Who::Object(Uuid::from_bytes([0xC0; 16]));
    for text in commands.split(',') {
        let (behaviour, option) = match text.split_once(':') {
            Some((behaviour, option)) => (behaviour, Some(option)),
            None => (text, None),
        };
        kept.set(collar, "Collar", behaviour, option);
    }
    rig.grid.kept = kept;
}

#[test]
fn setgroup_refuses_group_activate_and_nothing_else() {
    let mut rig = Rig::listed();
    hold(&mut rig, "setgroup");
    let reason = error(&rig.line("group activate Dancers"));
    assert!(reason.contains("setgroup"), "{reason}");
    assert!(error(&rig.line("group activate none")).contains("setgroup"));
    assert_eq!(rig.calls(), []);
    ok(&rig.group(GroupRequest::List));
    // Leaving, the active group too, is not refused, as viewers allow it.
    rig.line("group leave Builders");
    assert_eq!(rig.calls(), [Call::Leave(builders())]);
}

#[test]
fn sendim_holds_back_group_chat_before_the_session() {
    let mut rig = Rig::listed();
    hold(&mut rig, "sendim,sendim:Builders");
    let answer = rig.line("group chat Builders hello");
    assert_eq!(rig.calls(), [Call::StartSession(builders())]);
    waits(&answer);
    let reason = error(&rig.line("group chat Dancers hello"));
    assert!(reason.contains("sendim"), "{reason}");
    assert_eq!(rig.calls(), []);
}

#[test]
fn recvim_hides_a_line_and_not_a_notice() {
    let mut rig = Rig::listed();
    hold(&mut rig, "recvim");
    rig.commands
        .group_line(builders(), jane(), "Jane Doe", "are you there?", Some(&mut rig.grid));
    rig.commands
        .group_notice(builders(), None, "Jane Doe", "Meeting|Tonight", &[], None);
    let records = rig.records();
    assert_eq!(records[0].text, "(blocked by RLV: recvim)");
    assert_eq!(records[1].text, "Tonight");
    assert_eq!(rig.calls(), []);
}

// ---------------------------------------------------------- invitations

#[test]
fn an_invitation_waits_for_the_group_and_the_inviter() {
    let mut rig = Rig::listed();
    rig.set_rule(entries(&[&jane().to_string()]));
    let other = Uuid::from_bytes([0xAB; 16]);
    rig.invited(other, invitation(1), "Jane Doe", 0);
    let calls = rig.calls();
    let query = match &calls[..] {
        [Call::Find(query, name), Call::GroupProfile(group)] if name == "jane doe" && *group == other => *query,
        other => panic!("{other:?}"),
    };
    rig.profile(profile(other, "Fishers", 0, true));
    assert_eq!(rig.calls(), []);
    rig.commands
        .avatars_found(query, &[(jane(), "Jane Doe".into())], Some(&mut rig.grid), rig.now);
    assert_eq!(rig.calls(), [Call::Answer(other, invitation(1), true)]);
    let told = rig.told();
    assert!(told.iter().any(|line| line.contains("accepted by itself")), "{told:?}");
    // The list confirms it.
    let mut groups = usual();
    groups.push(member(other, "Fishers", 0, true));
    rig.list(groups);
    let told = rig.told();
    assert!(told.iter().any(|line| line.contains("confirmed")), "{told:?}");
}

#[test]
fn an_inviter_the_grid_does_not_find_matches_by_name_only() {
    let mut rig = Rig::listed();
    rig.set_rule(entries(&[&jane().to_string()]));
    let other = Uuid::from_bytes([0xAB; 16]);
    rig.invited(other, invitation(1), "Jane Doe", 0);
    rig.calls();
    rig.profile(profile(other, "Fishers", 0, true));
    rig.advance(10);
    assert_eq!(rig.calls(), []);
    let told = rig.told();
    assert!(
        told.iter()
            .any(|line| line.contains("Fishers") && line.contains("Jane Doe") && line.contains("waits")),
        "{told:?}"
    );
    // By name, it would have been accepted.
    let mut rig = Rig::listed();
    rig.set_rule(entries(&["Jane Doe"]));
    rig.invited(other, invitation(2), "Jane Doe", 0);
    rig.calls();
    rig.advance(10);
    assert_eq!(rig.calls(), [Call::Answer(other, invitation(2), true)]);
}

#[test]
fn an_invitation_with_a_fee_is_never_accepted_by_itself() {
    let mut rig = Rig::listed();
    rig.set_rule(Rule::All);
    let other = Uuid::from_bytes([0xAB; 16]);
    rig.invited(other, invitation(1), "Jane Doe", 250);
    rig.calls();
    rig.profile(profile(other, "Fishers", 250, true));
    rig.advance(10);
    assert_eq!(rig.calls(), []);
    let told = rig.told();
    let line = told.iter().find(|line| line.contains("waits")).expect("a line");
    assert!(
        line.contains("L$250") && line.contains("Fishers") && line.contains(&other.to_string()),
        "{line}"
    );
    assert!(line.contains(&format!("rslbot ctl accept group {other} --pay 250")), "{line}");
    let status = ok(&rig.run(Request::Perms(crate::control::PermsRequest::Status))).unwrap();
    let waiting = status.lines().find(|line| line.starts_with("waiting:")).unwrap();
    assert!(
        waiting.contains("group") && waiting.contains("Fishers") && waiting.contains("L$250"),
        "{waiting}"
    );
}

#[test]
fn rlv_does_not_apply_to_invitations() {
    let mut rig = Rig::listed();
    hold(&mut rig, "recvim,sendim,setgroup");
    rig.grid.sitting = crate::rlv::Sitting::Object;
    rig.set_rule(Rule::All);
    let other = Uuid::from_bytes([0xAB; 16]);
    rig.invited(other, invitation(3), "Jane Doe", 0);
    rig.calls();
    rig.profile(profile(other, "Fishers", 0, true));
    rig.advance(10);
    assert_eq!(rig.calls(), [Call::Answer(other, invitation(3), true)]);
    assert!(!rig.commands.take_manual());
}

/// Three invitations wait: Jane Doe's into `Builders` with a fee of
/// L$250, John Roe's into the free `Dancers`, and Jane Doe's into the
/// free `Fishers`.
fn three_waiting() -> Rig {
    let mut rig = Rig::new();
    rig.list(vec![]);
    let fishers = Uuid::from_bytes([0xAB; 16]);
    for (group, name, byte, inviter, inviter_id, fee) in [
        (builders(), "Builders", 1, "Jane Doe", jane(), 250),
        (dancers(), "Dancers", 2, "John Roe", john(), 0),
        (fishers, "Fishers", 3, "Jane Doe", jane(), 0),
    ] {
        rig.invited(group, invitation(byte), inviter, fee);
        let calls = rig.calls();
        let query = calls
            .iter()
            .find_map(|call| match call {
                Call::Find(query, _) => Some(*query),
                _ => None,
            })
            .unwrap_or_default();
        rig.profile(profile(group, name, fee, false));
        rig.commands
            .avatars_found(query, &[(inviter_id, inviter.into())], Some(&mut rig.grid), rig.now);
    }
    rig.calls();
    rig.told();
    rig
}

#[test]
fn group_invitations_lists_what_waits() {
    let mut rig = three_waiting();
    let listed = ok(&rig.group(GroupRequest::Invitations)).unwrap();
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines[0], format!("{}\t250\t{}\tJane Doe\tBuilders", builders(), jane()));
    assert_eq!(lines[1], format!("{}\t0\t{}\tJohn Roe\tDancers", dancers(), john()));
    assert_eq!(rig.told(), Vec::<String>::new());
    let mut rig = Rig::listed();
    assert_eq!(ok(&rig.group(GroupRequest::Invitations)), None);
    // One whose inviter and group are not known yet.
    rig.invited(Uuid::from_bytes([0xAB; 16]), invitation(9), "Jane Doe", 0);
    rig.advance(10);
    let listed = ok(&rig.group(GroupRequest::Invitations)).unwrap();
    assert!(listed.ends_with("\t0\t?\tJane Doe\t?"), "{listed}");
}

#[test]
fn accept_group_takes_the_one_invitation_named() {
    let mut rig = three_waiting();
    let answer = rig.line("accept group dancers");
    assert_eq!(rig.calls(), [Call::Answer(dancers(), invitation(2), true)]);
    waits(&answer);
    rig.list(vec![member(dancers(), "Dancers", 0, true)]);
    let text = ok(&answer).unwrap();
    assert!(text.contains("Dancers") && text.contains("John Roe"), "{text}");
    // By the inviter, who sent two: refused, listing both.
    let reason = error(&rig.line("accept group jane.doe"));
    assert!(
        reason.contains("2 invitations") && reason.contains(&builders().to_string()),
        "{reason}"
    );
    assert!(reason.contains("Fishers"), "{reason}");
    // Without WHO while two wait.
    assert!(error(&rig.line("accept group")).contains("2 invitations"));
    assert_eq!(rig.calls(), []);
    // Nobody named.
    let reason = error(&rig.run(Request::Accept {
        kind: "group".into(),
        who: Some("John Roe".into()),
        pay: None,
    }));
    assert!(reason.contains("`group`") && reason.contains("John Roe"), "{reason}");
    let told = rig.told();
    assert!(
        told.iter().any(|line| line.contains("accept group") && line.contains("Dancers")),
        "{told:?}"
    );
}

#[test]
fn accept_group_by_inviter_and_only_one_waiting() {
    let mut rig = Rig::new();
    rig.list(vec![]);
    rig.invited(dancers(), invitation(2), "John Roe", 0);
    rig.calls();
    rig.profile(profile(dancers(), "Dancers", 0, true));
    rig.advance(10);
    rig.calls();
    let answer = rig.line("accept group");
    assert_eq!(rig.calls(), [Call::Answer(dancers(), invitation(2), true)]);
    rig.advance(15);
    assert!(error(&answer).contains("has not confirmed"));
    // Forgotten once answered.
    assert!(error(&rig.line("accept group")).contains("no request of kind `group`"));
}

#[test]
fn two_groups_of_one_name_are_listed_with_their_uuids() {
    let mut rig = Rig::new();
    rig.list(vec![]);
    let (a, b) = (Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
    for (group, byte) in [(a, 1), (b, 2)] {
        rig.invited(group, invitation(byte), "Jane Doe", 0);
        rig.profile(profile(group, "Friends", 0, true));
    }
    rig.advance(10);
    rig.calls();
    let reason = error(&rig.line("accept group Friends"));
    assert!(reason.contains(&a.to_string()) && reason.contains(&b.to_string()), "{reason}");
}

#[test]
fn accept_group_pays_the_fee_of_an_invitation() {
    let mut rig = three_waiting();
    rig.commands.set_payments(Some(config()), Ledger::from_config(&config()));
    // Without a sum it keeps waiting.
    let reason = error(&rig.line("accept group Builders"));
    assert!(reason.contains("L$250") && reason.contains("--pay"), "{reason}");
    assert_eq!(rig.calls(), []);
    let answer = rig.line("accept group --pay 250 Builders");
    assert_eq!(rig.calls(), [Call::Answer(builders(), invitation(1), true)]);
    let info = crate::udp::messages::TransactionInfo {
        kind: 1004,
        source: rig.grid.own,
        source_group: false,
        dest: builders(),
        dest_group: true,
        amount: 250,
        item: String::new(),
    };
    let own = rig.grid.own;
    rig.commands
        .money_report(Uuid::nil(), 750, true, "You paid L$250 to join a group.", Some(&info), own);
    rig.list(vec![member(builders(), "Builders", 0, true)]);
    let text = ok(&answer).unwrap();
    assert!(text.contains("Builders") && text.contains("L$250"), "{text}");
    let records = rig.commands.take_transactions();
    assert_eq!(records[0].text, "You paid L$250 to join a group.");
}

#[test]
fn a_fee_of_an_invitation_refused_for_the_limit_keeps_it_waiting() {
    let mut rig = three_waiting();
    let limited = PaymentsConfig {
        limit: Some(100),
        ..config()
    };
    rig.commands.set_payments(Some(limited.clone()), Ledger::from_config(&limited));
    let reason = error(&rig.line("accept group Builders --pay 250"));
    assert!(reason.contains("L$100"), "{reason}");
    assert!(ok(&rig.group(GroupRequest::Invitations)).unwrap().contains("Builders"));
    let blocked = PaymentsConfig {
        blacklist: vec![Entry::Id(builders())],
        ..config()
    };
    rig.commands.set_payments(Some(blocked.clone()), Ledger::from_config(&blocked));
    assert!(error(&rig.line("accept group Builders --pay 250")).contains("blacklist"));
    // The inviter on the unlimited list does not make the fee unlimited.
    let unlimited = PaymentsConfig {
        unlimited: vec![Entry::parse("Jane Doe").unwrap()],
        limit: Some(100),
        ..config()
    };
    rig.commands.set_payments(Some(unlimited.clone()), Ledger::from_config(&unlimited));
    assert!(error(&rig.line("accept group Builders --pay 250")).contains("L$100"));
    assert_eq!(rig.calls(), []);
}

#[test]
fn decline_group_declines_and_forgets() {
    let mut rig = three_waiting();
    let text = ok(&rig.line("decline group Dancers")).unwrap();
    assert!(text.contains("John Roe") && text.contains("Dancers"), "{text}");
    assert_eq!(rig.calls(), [Call::Answer(dancers(), invitation(2), false)]);
    assert!(!ok(&rig.group(GroupRequest::Invitations)).unwrap().contains("Dancers"));
    assert!(rig.told().iter().any(|line| line.contains("decline group")));
}

#[test]
fn invitations_that_wait_are_replaced_forgotten_and_bounded() {
    let mut rig = Rig::new();
    rig.list(vec![]);
    rig.invited(builders(), invitation(1), "Jane Doe", 0);
    rig.invited(builders(), invitation(2), "John Roe", 0);
    rig.advance(10);
    let listed = ok(&rig.group(GroupRequest::Invitations)).unwrap();
    assert_eq!(listed.lines().count(), 1);
    assert!(listed.contains("John Roe"));
    // Decided apart, the newer one still takes the place of the older.
    rig.invited(builders(), invitation(3), "Jane Doe", 0);
    rig.advance(10);
    let listed = ok(&rig.group(GroupRequest::Invitations)).unwrap();
    assert_eq!(listed.lines().count(), 1);
    assert!(listed.contains("Jane Doe"));
    // Joined another way.
    rig.list(vec![member(builders(), "Builders", 0, true)]);
    assert_eq!(ok(&rig.group(GroupRequest::Invitations)), None);
    // The bound of 100, the oldest going.
    for n in 0..101u8 {
        rig.invited(Uuid::from_bytes([n; 16]), invitation(n), "Jane Doe", 0);
    }
    rig.advance(10);
    let listed = ok(&rig.group(GroupRequest::Invitations)).unwrap();
    assert_eq!(listed.lines().count(), 100);
    assert!(!listed.contains(&Uuid::from_bytes([0; 16]).to_string()));
    // Lost with the session.
    rig.commands.session_lost();
    rig.list(vec![]);
    assert_eq!(ok(&rig.group(GroupRequest::Invitations)), None);
}

#[test]
fn the_page_is_told_when_the_groups_or_invitations_change() {
    let mut rig = Rig::new();
    rig.commands.groups_session_began();
    assert!(rig.commands.take_groups_changed());
    assert!(!rig.commands.take_groups_changed());
    rig.list(usual());
    assert!(rig.commands.take_groups_changed());
    rig.list(usual());
    assert!(!rig.commands.take_groups_changed());
    rig.invited(dancers(), invitation(1), "Jane Doe", 0);
    assert!(!rig.commands.take_groups_changed(), "not before it waits");
    rig.advance(10);
    assert!(rig.commands.take_groups_changed());
    rig.commands.active_group(builders(), "", "");
    assert!(rig.commands.take_groups_changed());
    rig.commands.session_lost();
    assert!(rig.commands.take_groups_changed());
}

#[test]
fn group_commands_leave_the_placement_alone() {
    let mut rig = Rig::listed();
    for line in [
        "group activate Dancers",
        "group list",
        "group invitations",
        "group chat Builders hi",
    ] {
        rig.line(line);
    }
    rig.commands.active_group(dancers(), "", "");
    assert!(!rig.commands.take_manual());
}
