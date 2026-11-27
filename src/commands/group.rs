//! The `group` commands, invitations into groups (requests of the kind
//! `group`), group chat and notices, and what waits for the grid: the list
//! of the avatar's groups, a group's profile, the confirmation of a join
//! or a leave, and entering a group's chat session.

use super::pay::FeeFor;
use super::*;
use crate::control::GroupRequest;
use crate::escape::escape;
use crate::grouplog::{self, GroupMessage};
use crate::groups::{self, Groups, Membership, Profile};
use crate::payments::Interface;

/// How long a command waits for the grid to list the avatar's groups.
pub const GROUPS_LIMIT: Duration = Duration::from_secs(10);
/// How long `group activate` waits for the grid to say the group is active.
pub const ACTIVATE_LIMIT: Duration = Duration::from_secs(10);
/// How long the grid has to answer for a group's profile.
pub const PROFILE_LIMIT: Duration = Duration::from_secs(10);
/// How long an answer for a profile is used again.
pub const PROFILE_FRESH: Duration = Duration::from_secs(30);
/// How long the grid has to name a group's founder for `group info`.
pub const FOUNDER_LIMIT: Duration = Duration::from_secs(5);
/// How long a join or an acceptance waits for the grid to list the group.
pub const JOIN_LIMIT: Duration = Duration::from_secs(15);
/// How long `group leave` waits for the grid to say the avatar left.
pub const LEAVE_LIMIT: Duration = Duration::from_secs(15);
/// How long after a confirmed join or leave a list or a drop that says
/// otherwise is taken as one the grid made before it, and held back.
pub const SETTLE_TIME: Duration = Duration::from_secs(5);
/// How long `group chat` waits for the grid to let the avatar into the
/// group's session.
pub const SESSION_LIMIT: Duration = Duration::from_secs(10);
/// How old a notice delivered as stored has to be to count as one, in
/// seconds.
const STORED_AGE: u64 = 60;
/// How long an invitation waits for the group's profile and the inviter's
/// UUID before it is decided.
pub const INVITATION_LIMIT: Duration = Duration::from_secs(10);

/// A command that came before the grid listed the avatar's groups.
struct Early {
    request: GroupRequest,
    origin: Origin,
    via: Interface,
    reply: Sender<Response>,
    deadline: Instant,
}

/// A `group activate` waiting for the grid to say which group is active.
struct Activating {
    /// The group, `None` for none.
    target: Option<Uuid>,
    words: String,
    origin: Origin,
    reply: Sender<Response>,
    deadline: Instant,
}

/// What waits for a group's profile.
enum ForProfile {
    Info {
        origin: Origin,
        reply: Sender<Response>,
    },
    Join {
        origin: Origin,
        via: Interface,
        pay: Option<u32>,
        reply: Sender<Response>,
    },
}

/// A `group info` with its profile, waiting for the founder's name.
struct InfoWait {
    profile: Profile,
    origin: Origin,
    reply: Sender<Response>,
    deadline: Instant,
}

/// What the grid reported of the fee a join was sent with.
#[derive(Debug, Clone, PartialEq)]
enum Report {
    /// Nothing yet.
    Pending,
    /// Made, leaving this balance.
    Paid(i64),
}

/// A join or an acceptance waiting for the grid to list the group.
struct Joining {
    group: Uuid,
    words: String,
    /// The command that waits; nobody for an invitation the bot accepted
    /// by itself.
    reply: Option<Sender<Response>>,
    origin: Origin,
    via: Interface,
    /// `group join` or `accept group`, for the log.
    command: &'static str,
    /// Who invited, for an acceptance.
    inviter: Option<String>,
    fee: Option<u32>,
    report: Report,
    /// The grid answered the `JoinGroupRequest` with success; the join is
    /// confirmed by the list that follows.
    granted: bool,
    deadline: Instant,
}

/// A join or a leave the grid confirmed a moment ago.
#[derive(Clone, Copy)]
struct Settled {
    group: Uuid,
    /// Whether the avatar is in the group since.
    member: bool,
    until: Instant,
}

/// A `group leave` waiting for the grid to say the avatar left.
struct Leaving {
    group: Uuid,
    words: String,
    origin: Origin,
    reply: Sender<Response>,
    deadline: Instant,
}

/// A line `group chat` waits to say until the avatar is in the session.
struct Line {
    text: String,
    origin: Origin,
    reply: Sender<Response>,
}

/// How far the avatar is in a group's chat session.
enum Chat {
    In,
    Entering { lines: Vec<Line>, deadline: Instant },
}

/// An invitation waiting for the group's profile and the inviter's UUID.
struct Inviting {
    request: perms::Request,
    /// The avatar search for the inviter's name, while it is under way.
    query: Option<Uuid>,
    deadline: Instant,
}

/// What groups need, for one session.
#[derive(Default)]
pub(super) struct GroupState {
    groups: Groups,
    early: Vec<Early>,
    activating: Vec<Activating>,
    /// Profiles the grid answered for, and when.
    profiles: HashMap<Uuid, (Profile, Instant)>,
    /// When a profile was last asked for and not answered.
    asked: HashMap<Uuid, Instant>,
    for_profile: Vec<(Uuid, ForProfile, Instant)>,
    infos: Vec<InfoWait>,
    joining: Vec<Joining>,
    leaving: Vec<Leaving>,
    /// Joins and leaves confirmed within `SETTLE_TIME`.
    settled: Vec<Settled>,
    /// When to ask the grid for the list again, having held one back.
    ask_again: Option<Instant>,
    sessions: HashMap<Uuid, Chat>,
    inviting: Vec<Inviting>,
    /// Group messages for the message log to take.
    records: Vec<GroupMessage>,
    /// The waiting invitations as the page was last told of them.
    invitations_seen: Vec<(Uuid, Uuid)>,
    /// The bot logged in or lost the session since the page was told.
    session_changed: bool,
}

fn yes_answer(text: String) -> Result<Done, String> {
    Ok(Done::Answer(Some(text)))
}

/// Answers a command that has come to an end later, as `finish` does.
fn answer_later(origin: Origin, command: &str, reply: &Sender<Response>, result: Result<Option<String>, String>) {
    match result {
        Ok(text) => {
            let _ = reply.send(Response::Ok(text));
        }
        Err(reason) => refuse_from(origin, command, reply, reason),
    }
}

impl Commands {
    // ------------------------------------------------------------ commands

    /// A `group` command.
    pub(super) fn group_command<G: Grid>(
        &mut self,
        request: GroupRequest,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.present()?;
        if request == GroupRequest::Invitations {
            return Ok(Done::Answer(self.invitations_text()));
        }
        if !self.group.groups.known() {
            self.group.early.push(Early {
                request,
                origin: self.origin,
                via: self.via,
                reply: reply.clone(),
                deadline: now + GROUPS_LIMIT,
            });
            return Ok(Done::Parked);
        }
        self.group_now(request, reply, grid, now)
    }

    /// A `group` command once the avatar's groups are known.
    fn group_now<G: Grid>(&mut self, request: GroupRequest, reply: &Sender<Response>, grid: &mut G, now: Instant) -> Result<Done, String> {
        let groups = &self.group.groups;
        match request {
            GroupRequest::List => {
                let text = groups.list_text();
                Ok(Done::Answer((!text.is_empty()).then_some(text)))
            }
            GroupRequest::Invitations => Ok(Done::Answer(self.invitations_text())),
            GroupRequest::Activate(given) => self.activate(&given, reply, grid, now),
            GroupRequest::Invite { group, avatar } => {
                let group = groups.named(&group).map_err(|e| e.to_string())?.clone();
                if !group.may_invite() {
                    return Err(format!("the avatar may not invite into {}", group.words()));
                }
                let then = After::Invite {
                    group: group.id,
                    words: group.words(),
                };
                self.with_avatar(&avatar, then, reply, Some(grid), now)
            }
            GroupRequest::Chat { group, text } => {
                let group = groups.named(&group).map_err(|e| e.to_string())?.clone();
                self.group_chat(&group, text, reply, grid, now)
            }
            GroupRequest::Info(given) => {
                let id = groups.any(&given).map_err(|e| e.to_string())?;
                let then = ForProfile::Info {
                    origin: self.origin,
                    reply: reply.clone(),
                };
                self.with_profile(id, then, grid, now)
            }
            GroupRequest::Join { group, pay } => {
                if let Some(member) = groups.get(group) {
                    return Err(format!("the avatar is in {} already", member.words()));
                }
                let then = ForProfile::Join {
                    origin: self.origin,
                    via: self.via,
                    pay,
                    reply: reply.clone(),
                };
                self.with_profile(group, then, grid, now)
            }
            GroupRequest::Leave(given) => {
                let group = groups.named(&given).map_err(|e| e.to_string())?;
                let (id, words) = (group.id, group.words());
                grid.leave_group(id, now)?;
                if !self.origin.quiet() {
                    self.tell(format!("{}: group leave {words}: sent", self.origin.tag()));
                }
                self.group.leaving.push(Leaving {
                    group: id,
                    words,
                    origin: self.origin,
                    reply: reply.clone(),
                    deadline: now + LEAVE_LIMIT,
                });
                Ok(Done::Parked)
            }
        }
    }

    /// `group activate`: asks for the group, or none, to be active, and
    /// waits for the grid to say so.
    fn activate<G: Grid>(&mut self, given: &str, reply: &Sender<Response>, grid: &mut G, now: Instant) -> Result<Done, String> {
        self.limits(&*grid).set_group().map_err(|refusal| self.refused(refusal))?;
        let (target, words) = match given.trim() {
            none if none.eq_ignore_ascii_case("none") => (None, "no group".to_string()),
            given => {
                let group = self.group.groups.named(given).map_err(|e| e.to_string())?;
                (Some(group.id), group.words())
            }
        };
        grid.activate_group(target.unwrap_or_default(), now)?;
        self.group.activating.push(Activating {
            target,
            words,
            origin: self.origin,
            reply: reply.clone(),
            deadline: now + ACTIVATE_LIMIT,
        });
        Ok(Done::Parked)
    }

    /// `group invite`, once the avatar is known: never the bot's own.
    pub(super) fn invite<G: Grid>(&mut self, grid: &mut G, group: Uuid, words: &str, id: Uuid, name: &str) -> Result<Done, String> {
        let who = whom(id, name);
        if id == grid.own_id() {
            return Err("the bot's own avatar cannot be invited into a group".to_string());
        }
        grid.invite_to_group(group, id, Instant::now())?;
        if !self.origin.quiet() {
            self.tell(format!("{}: group invite {who} into {words}: sent", self.origin.tag()));
        }
        yes_answer(format!(
            "invited {who} into {words}; the grid does not report whether the invitation arrived"
        ))
    }

    /// `group chat`: says the line now, or enters the session first.
    fn group_chat<G: Grid>(
        &mut self,
        group: &Membership,
        text: String,
        reply: &Sender<Response>,
        grid: &mut G,
        now: Instant,
    ) -> Result<Done, String> {
        let words = group.words();
        let checked = check_text(&text).and_then(|()| {
            self.limits(&*grid)
                .group_chat(group.id, &group.name)
                .map_err(|refusal| self.refused(refusal))
        });
        if let Err(reason) = checked {
            return Err(format!("nothing said in {words}: {reason}"));
        }
        let line = Line {
            text,
            origin: self.origin,
            reply: reply.clone(),
        };
        match self.group.sessions.get_mut(&group.id) {
            Some(Chat::In) => {
                let said = self.say_in_group(group.id, &line.text, line.origin, grid, now)?;
                Ok(Done::Answer(Some(said)))
            }
            Some(Chat::Entering { lines, .. }) => {
                lines.push(line);
                Ok(Done::Parked)
            }
            None => {
                grid.start_group_session(group.id, now)?;
                self.tell(format!("groups: entering the chat session of {words}"));
                let entering = Chat::Entering {
                    lines: vec![line],
                    deadline: now + SESSION_LIMIT,
                };
                self.group.sessions.insert(group.id, entering);
                Ok(Done::Parked)
            }
        }
    }

    /// Says a line in a group the avatar is in the session of, and records
    /// it. The answer's text.
    fn say_in_group<G: Grid>(&mut self, group: Uuid, text: &str, origin: Origin, grid: &mut G, now: Instant) -> Result<String, String> {
        let (name, words) = match self.group.groups.get(group) {
            Some(member) => (member.name.clone(), member.words()),
            None => (String::new(), format!("the group {group}")),
        };
        grid.group_chat(group, text, now)?;
        let time = self.wall();
        self.group.records.push(GroupMessage {
            time,
            kind: grouplog::Kind::ChatOut,
            group,
            group_name: name,
            avatar: Some(grid.own_id()),
            avatar_name: String::new(),
            display_name: String::new(),
            subject: String::new(),
            attachment: String::new(),
            text: text.to_string(),
            offline: false,
        });
        if !origin.quiet() {
            self.tell(format!("{}: group chat in {words}: sent", origin.tag()));
        }
        Ok(format!("said in {words}"))
    }

    // ------------------------------------------------------------ profiles

    /// Does `then` with `group`'s profile: one the grid answered for less
    /// than 30 seconds ago, or the one it is asked for now.
    fn with_profile<G: Grid>(&mut self, group: Uuid, then: ForProfile, grid: &mut G, now: Instant) -> Result<Done, String> {
        if let Some((profile, _)) = self.fresh_profile(group, now) {
            return self.profiled(profile, then, grid, now);
        }
        self.ask_profile(group, grid, now)?;
        self.group.for_profile.push((group, then, now + PROFILE_LIMIT));
        Ok(Done::Parked)
    }

    fn fresh_profile(&self, group: Uuid, now: Instant) -> Option<(Profile, Instant)> {
        self.group
            .profiles
            .get(&group)
            .filter(|(_, at)| now.duration_since(*at) < PROFILE_FRESH)
            .cloned()
    }

    /// Asks the grid for `group`'s profile unless it was asked a moment
    /// ago and has not answered: a simulator drops a repeated question.
    fn ask_profile<G: Grid>(&mut self, group: Uuid, grid: &mut G, now: Instant) -> Result<(), String> {
        let waiting = self
            .group
            .asked
            .get(&group)
            .is_some_and(|at| now.duration_since(*at) < PROFILE_LIMIT);
        if !waiting {
            grid.ask_group_profile(group, now)?;
            self.group.asked.insert(group, now);
        }
        Ok(())
    }

    /// What waited for a profile, with the profile.
    fn profiled<G: Grid>(&mut self, profile: Profile, then: ForProfile, grid: &mut G, now: Instant) -> Result<Done, String> {
        match then {
            ForProfile::Info { origin, reply } => {
                if self.named(profile.founder) || profile.founder.is_nil() {
                    return Ok(Done::Answer(Some(self.info_text(&profile))));
                }
                self.group.infos.push(InfoWait {
                    profile,
                    origin,
                    reply,
                    deadline: now + FOUNDER_LIMIT,
                });
                Ok(Done::Parked)
            }
            ForProfile::Join { origin, via, pay, reply } => self.join(&profile, origin, via, pay, &reply, grid, now),
        }
    }

    /// The text of `group info`, with the founder's name as far as known.
    fn info_text(&self, profile: &Profile) -> String {
        let founder = match self.owners.get(&profile.founder) {
            Some(Some(names)) => names.legacy.clone(),
            _ => profile.founder.to_string(),
        };
        let member = self.group.groups.get(profile.id).is_some();
        groups::info_text(profile, &founder, member)
    }

    /// The grid answered for a group's profile.
    pub fn group_profile<G: Grid>(&mut self, profile: Profile, mut grid: Option<&mut G>, now: Instant) {
        let id = profile.id;
        self.group.asked.remove(&id);
        self.group.profiles.insert(id, (profile.clone(), now));
        // The invitations into it know the group's name now.
        for inviting in &mut self.group.inviting {
            if let Ask::Group { group, name, .. } = &mut inviting.request.ask {
                if *group == id {
                    *name = Some(profile.name.clone());
                }
            }
        }
        let (ready, waiting) = std::mem::take(&mut self.group.for_profile)
            .into_iter()
            .partition(|(group, _, _)| *group == id);
        self.group.for_profile = waiting;
        for (_, then, _) in ready {
            let (origin, reply, command) = match &then {
                ForProfile::Info { origin, reply } => (*origin, reply.clone(), "group info"),
                ForProfile::Join { origin, reply, .. } => (*origin, reply.clone(), "group join"),
            };
            self.origin = origin;
            let done = match grid.as_deref_mut() {
                Some(grid) => self.profiled(profile.clone(), then, grid, now),
                None => Err(NOT_CONNECTED.to_string()),
            };
            self.origin = Origin::Operator;
            finish(origin, command, done, &reply);
        }
        self.invitations_due(grid, now);
    }

    // --------------------------------------------------------------- joins

    /// `group join` with the group's profile: refused, sent, or paid for.
    #[allow(clippy::too_many_arguments)]
    fn join<G: Grid>(
        &mut self,
        profile: &Profile,
        origin: Origin,
        via: Interface,
        pay: Option<u32>,
        reply: &Sender<Response>,
        grid: &mut G,
        now: Instant,
    ) -> Result<Done, String> {
        let words = format!("\"{}\" <{}>", profile.name, profile.id);
        if self.group.groups.get(profile.id).is_some() {
            return Err(format!("the avatar is in {words} already"));
        }
        if !profile.open {
            return Err(format!("joining {words} takes an invitation: its enrollment is closed"));
        }
        let fee = u32::try_from(profile.fee).unwrap_or(0);
        if fee > 0 {
            match pay {
                None => return Err(format!("joining {words} costs L${fee}; give `--pay {fee}` to pay it")),
                Some(sum) if sum < fee => {
                    return Err(format!(
                        "joining {words} costs L${fee}, more than the L${sum} of `--pay`; give `--pay {fee}` to pay it"
                    ))
                }
                Some(_) => {}
            }
            self.origin = origin;
            self.pay_fee(fee, profile.id, &profile.name, FeeFor::Join, via, reply, grid, now)?;
        } else {
            grid.join_group(profile.id, now)?;
            if !origin.quiet() {
                self.tell(format!("{}: group join {words}: sent", origin.tag()));
            }
        }
        self.group.joining.push(Joining {
            group: profile.id,
            words,
            reply: Some(reply.clone()),
            origin,
            via,
            command: "group join",
            inviter: None,
            fee: (fee > 0).then_some(fee),
            report: Report::Pending,
            granted: false,
            deadline: now + JOIN_LIMIT,
        });
        Ok(Done::Parked)
    }

    /// The grid answered a `JoinGroupRequest`. Success does not answer the
    /// command: the grid sends it before the list with the group, and a
    /// command naming the group right after the answer would not find it.
    /// The list that follows, which the session asks for, confirms it.
    pub fn joined(&mut self, group: Uuid, success: bool) {
        match success {
            true => {
                for join in self.group.joining.iter_mut().filter(|join| join.group == group) {
                    join.granted = true;
                }
            }
            false => {
                let reason = "the grid refused the join".to_string();
                self.fee_refused(group, &reason);
                let (refused, waiting) = std::mem::take(&mut self.group.joining)
                    .into_iter()
                    .partition(|join| join.group == group && join.command == "group join");
                self.group.joining = waiting;
                for join in refused {
                    let error = format!("the grid refused to let the avatar join {}", join_words(&join));
                    self.join_ended(join, Err(error));
                }
            }
        }
    }

    /// The avatar is in these groups: the joins and acceptances into them
    /// are confirmed.
    fn confirm_joins(&mut self, groups: &[Uuid], now: Instant) {
        let (done, waiting) = std::mem::take(&mut self.group.joining)
            .into_iter()
            .partition(|join| groups.contains(&join.group));
        self.group.joining = waiting;
        for join in done {
            self.settle_group(join.group, true, now);
            let mut answer = format!("joined {}", join.words);
            if let Some(inviter) = &join.inviter {
                answer.push_str(&format!(", invited by {inviter}"));
            }
            if let Some(fee) = join.fee {
                match join.report {
                    Report::Paid(balance) => answer.push_str(&format!("; paid L${fee}, the balance is L${balance}")),
                    Report::Pending => {
                        answer.push_str(&format!("; the fee of L${fee} counts as paid, as the grid has not reported it yet"))
                    }
                }
            }
            self.join_ended(join, Ok(answer));
        }
    }

    /// A join or an acceptance came to an end: logged and answered.
    fn join_ended(&mut self, join: Joining, result: Result<String, String>) {
        let what = match &join.inviter {
            Some(inviter) => format!("{}: invitation from {inviter} into {}", join.command, join.words),
            None => format!("{} {}", join.command, join.words),
        };
        let tag = match join.reply {
            None => "bot".to_string(),
            Some(_) => match join.fee {
                Some(_) => join.via.to_string(),
                None => join.origin.tag().to_string(),
            },
        };
        if !join.origin.quiet() || join.reply.is_none() {
            match &result {
                Ok(_) => self.tell(format!("{tag}: {what}: confirmed")),
                Err(reason) => self.tell(format!("{tag}: {what}: not confirmed: {reason}")),
            }
        }
        if let Some(reply) = join.reply {
            let _ = reply.send(match result {
                Ok(text) => Response::Ok(Some(text)),
                Err(reason) => Response::Error(reason),
            });
        }
    }

    /// The grid reported a join's fee as made.
    pub(super) fn fee_paid(&mut self, group: Uuid, balance: i64) {
        for join in self
            .group
            .joining
            .iter_mut()
            .filter(|join| join.group == group && join.fee.is_some())
        {
            join.report = Report::Paid(balance);
        }
    }

    /// The grid reported a join's fee as failed: the join is answered with
    /// its reason.
    pub(super) fn fee_failed(&mut self, group: Uuid, reason: &str) {
        let (failed, waiting) = std::mem::take(&mut self.group.joining)
            .into_iter()
            .partition(|join| join.group == group && join.fee.is_some());
        self.group.joining = waiting;
        for join in failed {
            let fee = join.fee.unwrap_or(0);
            let error = format!("the grid refused the fee of L${fee} to join {}: {reason}", join.words);
            self.join_ended(join, Err(error));
        }
    }

    // -------------------------------------------------------------- leaves

    /// The grid answered a `LeaveGroupRequest`. Success is taken as the
    /// drop it is, as a viewer takes it: the grid may answer before it
    /// sends the drop, and a `group list` right after the answer would
    /// still list the group.
    pub fn left(&mut self, group: Uuid, success: bool, now: Instant) {
        match success {
            true => self.group_dropped(group, now),
            false => {
                let (refused, waiting) = std::mem::take(&mut self.group.leaving)
                    .into_iter()
                    .partition(|leave| leave.group == group);
                self.group.leaving = waiting;
                for leave in refused {
                    let error = format!("the grid refused to let the avatar leave {}", leave.words);
                    self.leave_ended(leave, Err(error));
                }
            }
        }
    }

    /// The avatar is no longer in the groups `gone` picks: the leaves from
    /// them are confirmed.
    fn confirm_leaves(&mut self, gone: impl Fn(Uuid) -> bool, now: Instant) {
        let (done, waiting) = std::mem::take(&mut self.group.leaving)
            .into_iter()
            .partition(|leave| gone(leave.group));
        self.group.leaving = waiting;
        for leave in done {
            self.settle_group(leave.group, false, now);
            let answer = format!("left {}", leave.words);
            self.leave_ended(leave, Ok(answer));
        }
    }

    /// A `group leave` came to an end: logged and answered.
    fn leave_ended(&mut self, leave: Leaving, result: Result<String, String>) {
        if !leave.origin.quiet() {
            let tag = leave.origin.tag();
            match &result {
                Ok(_) => self.tell(format!("{tag}: group leave {}: confirmed", leave.words)),
                Err(reason) => self.tell(format!("{tag}: group leave {}: not confirmed: {reason}", leave.words)),
            }
        }
        let _ = leave.reply.send(match result {
            Ok(text) => Response::Ok(Some(text)),
            Err(reason) => Response::Error(reason),
        });
    }

    // ------------------------------------------ lists that come too late

    /// The grid confirmed that the avatar is in `group`, or is not.
    fn settle_group(&mut self, group: Uuid, member: bool, now: Instant) {
        self.group.settled.retain(|settled| settled.group != group);
        self.group.settled.push(Settled {
            group,
            member,
            until: now + SETTLE_TIME,
        });
    }

    /// What the bot confirmed of `group` a moment ago, when the grid now
    /// says otherwise: `in_group` is what it says. A group the bot is
    /// joining or leaving again is never held back, as what the grid says
    /// of it may be the answer.
    fn contradicted(&self, group: Uuid, in_group: bool, now: Instant) -> Option<Settled> {
        let busy = self.group.joining.iter().any(|join| join.group == group) || self.group.leaving.iter().any(|leave| leave.group == group);
        if busy {
            return None;
        }
        self.group
            .settled
            .iter()
            .find(|settled| settled.group == group && settled.until > now && settled.member != in_group)
            .copied()
    }

    /// The grid said of `settled`'s group what it said before it confirmed
    /// the join or the leave: a grid may send its lists and drops out of
    /// order when a leave and a join follow each other closely. That is
    /// held back, and the list asked for again when the moment is over.
    fn hold_back(&mut self, settled: Settled, what: &str) {
        let words = self
            .group
            .groups
            .get(settled.group)
            .map_or_else(|| format!("the group {}", settled.group), Membership::words);
        crate::debug!(
            "{what} from the grid has the avatar {} {words}, which the grid confirmed otherwise a moment ago; \
             asking for the list again",
            if settled.member { "not in" } else { "in" }
        );
        self.group.ask_again = Some(self.group.ask_again.map_or(settled.until, |at| at.max(settled.until)));
    }

    // --------------------------------------------- what the grid reports

    /// The grid listed the avatar's groups.
    pub fn groups_listed<G: Grid>(&mut self, mut list: Vec<Membership>, mut grid: Option<&mut G>, now: Instant) {
        self.group.settled.retain(|settled| settled.until > now);
        for settled in self.group.settled.clone() {
            let at = list.iter().position(|group| group.id == settled.group);
            let Some(settled) = self.contradicted(settled.group, at.is_some(), now) else {
                continue;
            };
            match at {
                Some(at) => {
                    list.remove(at);
                }
                None => match self.group.groups.get(settled.group) {
                    Some(member) => list.push(member.clone()),
                    None => continue,
                },
            }
            self.hold_back(settled, "a list");
        }
        let ids: Vec<Uuid> = list.iter().map(|group| group.id).collect();
        let changes = self.group.groups.replace(list);
        if changes.first {
            let count = self.group.groups.count();
            let active = match self.group.groups.active().and_then(|id| self.group.groups.get(id)) {
                Some(group) => format!("the active group is {}", group.words()),
                None => "no group is active".to_string(),
            };
            self.tell(format!(
                "groups: the grid lists {count} group{} of the avatar; {active}",
                if count == 1 { "" } else { "s" }
            ));
        }
        for group in &changes.joined {
            self.tell(format!("groups: the avatar is in {} now", group.words()));
        }
        for group in &changes.left {
            self.tell(format!("groups: the avatar is no longer in {}", group.words()));
            self.group.sessions.remove(&group.id);
        }
        // An invitation into a group the avatar is in is of no use.
        for gone in self.requests.forget_invitations(&ids) {
            self.tell(format!("{}: forgotten, the avatar is in the group", gone.describe()));
        }
        self.group.inviting.retain(|inviting| match inviting.request.ask {
            Ask::Group { group, .. } => !ids.contains(&group),
            _ => true,
        });
        self.confirm_joins(&ids, now);
        self.confirm_leaves(|group| !ids.contains(&group), now);
        // The commands that came too early.
        for early in std::mem::take(&mut self.group.early) {
            let command = early.request.name();
            self.origin = early.origin;
            self.via = early.via;
            let done = match grid.as_deref_mut() {
                Some(grid) => self.group_now(early.request, &early.reply, grid, now),
                None => Err(NOT_CONNECTED.to_string()),
            };
            self.origin = Origin::Operator;
            self.via = Interface::Ctl;
            finish(early.origin, command, done, &early.reply);
        }
    }

    /// The grid says the avatar is no longer in `group`.
    pub fn group_dropped(&mut self, group: Uuid, now: Instant) {
        if let Some(settled) = self.contradicted(group, false, now) {
            self.hold_back(settled, "a drop");
            return;
        }
        match self.group.groups.dropped(group) {
            Some(member) => self.tell(format!("groups: the avatar is no longer in {}", member.words())),
            None => crate::debug!("the grid dropped the avatar from the group {group}, which it did not list"),
        }
        self.group.sessions.remove(&group);
        self.confirm_leaves(|left| left == group, now);
    }

    /// The grid says which group is active, nil for none.
    pub fn active_group(&mut self, group: Uuid, title: &str, name: &str) {
        let before = self.group.groups.active();
        self.group.groups.set_active(group, title);
        let active = self.group.groups.active();
        let words = match active {
            Some(id) => match self.group.groups.get(id) {
                Some(member) => member.words(),
                None if !name.is_empty() => format!("\"{name}\" <{id}>"),
                None => format!("the group {id}"),
            },
            None => "no group".to_string(),
        };
        let (done, waiting) = std::mem::take(&mut self.group.activating)
            .into_iter()
            .partition(|wait| wait.target == active);
        self.group.activating = waiting;
        let mut told = false;
        for wait in done {
            if !wait.origin.quiet() {
                self.tell(format!(
                    "{}: group activate {}: the active group now",
                    wait.origin.tag(),
                    wait.words
                ));
                told = true;
            }
            let answer = match (active, title.is_empty()) {
                (None, _) => "no group is active now".to_string(),
                (Some(_), true) => format!("{words} is the active group now, without a title"),
                (Some(_), false) => format!("{words} is the active group now, with the title \"{}\"", escape(title)),
            };
            let _ = wait.reply.send(Response::Ok(Some(answer)));
        }
        if before != active && !told && self.group.groups.known() {
            self.tell(format!("groups: the active group is {words} now"));
        }
    }

    /// The grid's answer to entering a group's chat session.
    pub fn group_session<G: Grid>(&mut self, group: Uuid, ok: bool, error: &str, grid: Option<&mut G>, now: Instant) {
        let words = self
            .group
            .groups
            .get(group)
            .map_or_else(|| format!("the group {group}"), Membership::words);
        let entering = self.group.sessions.remove(&group);
        if ok {
            self.group.sessions.insert(group, Chat::In);
        }
        let Some(Chat::Entering { lines, .. }) = entering else {
            return;
        };
        if !ok {
            let reason = match error.trim() {
                "" => "the grid gave no reason".to_string(),
                reason => reason.to_string(),
            };
            self.tell(format!(
                "groups: the grid did not let the avatar into the chat session of {words}: {reason}"
            ));
            for line in lines {
                let reason = format!("nothing said in {words}: the grid did not let the avatar into its chat session: {reason}");
                refuse_from(line.origin, "group chat", &line.reply, reason);
            }
            return;
        }
        self.tell(format!("groups: in the chat session of {words}"));
        self.say_parked(group, lines, grid, now);
    }

    fn say_parked<G: Grid>(&mut self, group: Uuid, lines: Vec<Line>, mut grid: Option<&mut G>, now: Instant) {
        for line in lines {
            let said = match grid.as_deref_mut() {
                Some(grid) => self.say_in_group(group, &line.text, line.origin, grid, now),
                None => Err(NOT_CONNECTED.to_string()),
            };
            answer_later(line.origin, "group chat", &line.reply, said.map(Some));
        }
    }

    /// The grid offered the avatar a group's chat session with its first
    /// line: it is taken up, as a viewer does that shows the line, where
    /// the grid has a service for that.
    pub fn session_offered<G: Grid>(&mut self, session: Uuid, grid: Option<&mut G>) {
        let (Some(member), Some(grid)) = (self.group.groups.get(session), grid) else {
            return;
        };
        if grid.take_up_session(session) {
            crate::debug!("taking up the chat session of {}", member.words());
        }
    }

    /// A line of a chat session. One of a group the avatar is in is
    /// recorded, with its text hidden where RLV says so; any other is a
    /// conference's and is dropped.
    pub fn group_line<G: Grid>(&mut self, session: Uuid, from: Uuid, name: &str, text: &str, grid: Option<&mut G>) {
        let Some(member) = self.group.groups.get(session).cloned() else {
            return crate::debug!("dropping a line of the chat session {session}, which is none of the avatar's groups");
        };
        let Some(grid) = grid else {
            return;
        };
        // A line of the session means the avatar is in it; lines that
        // wait for the grid's answer to entering it go out with that.
        if !matches!(self.group.sessions.get(&session), Some(Chat::Entering { .. })) {
            self.group.sessions.insert(session, Chat::In);
        }
        let text = match Origin::Bot.limits(grid.restrictions()).recv_group(from, session, &member.name) {
            Ok(()) => text.to_string(),
            Err(refusal) => format!("(blocked by RLV: {})", refusal.rule),
        };
        self.group.records.push(GroupMessage {
            time: self.wall(),
            kind: grouplog::Kind::Chat,
            group: session,
            group_name: member.name.clone(),
            avatar: (!from.is_nil()).then_some(from),
            avatar_name: name.trim().to_string(),
            display_name: String::new(),
            subject: String::new(),
            attachment: String::new(),
            text,
            offline: false,
        });
    }

    /// A group notice: recorded, never answered.
    pub fn group_notice(&mut self, group: Uuid, sender: Option<Uuid>, name: &str, text: &str, bucket: &[u8], sent_at: Option<u32>) {
        let notice = grouplog::notice(text, bucket);
        let group_name = self.group.groups.get(group).map(|member| member.name.clone()).unwrap_or_default();
        // OpenSim marks every notice as stored, with the time it was sent:
        // one sent a moment ago is a live one.
        let wall = self.wall();
        let sent_at = sent_at.filter(|at| wall.saturating_sub(u64::from(*at)) > STORED_AGE);
        self.group.records.push(GroupMessage {
            time: sent_at.map_or(wall, u64::from),
            kind: grouplog::Kind::Notice,
            group,
            group_name,
            avatar: sender,
            avatar_name: name.trim().to_string(),
            display_name: String::new(),
            subject: notice.subject,
            attachment: notice.attachment,
            text: notice.text,
            offline: sent_at.is_some(),
        });
    }

    /// The group messages recorded since this was last asked.
    pub fn take_group_records(&mut self) -> Vec<GroupMessage> {
        std::mem::take(&mut self.group.records)
    }

    /// Whether the avatar's groups or the waiting invitations changed
    /// since this was last asked, or the session began or ended.
    pub fn take_groups_changed(&mut self) -> bool {
        let groups = self.group.groups.take_changed();
        let seen: Vec<(Uuid, Uuid)> = self
            .requests
            .all_invitations()
            .iter()
            .filter_map(|request| match request.ask {
                Ask::Group { group, id, .. } => Some((group, id)),
                _ => None,
            })
            .collect();
        let invitations = seen != self.group.invitations_seen;
        self.group.invitations_seen = seen;
        let session = std::mem::take(&mut self.group.session_changed);
        groups || invitations || session
    }

    /// A login succeeded: the page asks for the groups again.
    pub fn groups_session_began(&mut self) {
        self.group.session_changed = true;
    }

    // --------------------------------------------------------- invitations

    /// An invitation into `group` arrived. Before the rules decide, the
    /// group's name and the inviter's UUID are asked for.
    pub fn group_invitation<G: Grid>(&mut self, group: Uuid, id: Uuid, inviter: &str, fee: i32, grid: Option<&mut G>, now: Instant) {
        let name = self.group.groups.get(group).map(|member| member.name.clone());
        let inviter = inviter.split_whitespace().collect::<Vec<_>>().join(" ");
        let known = crate::names::full_name(&inviter).and_then(|key| self.known.get(&key).map(|(id, _)| *id));
        let sender = perms::Sender::Inviter {
            name: inviter.clone(),
            id: known,
            group,
        };
        let request = perms::Request::new(sender, Ask::Group { group, id, fee, name });
        // An invitation into the same group that waits to be decided is
        // replaced by this one.
        self.group
            .inviting
            .retain(|inviting| !matches!(inviting.request.ask, Ask::Group { group: g, .. } if g == group));
        let Some(grid) = grid else {
            return self.decide(request, None::<&mut G>, now);
        };
        if let Some((profile, _)) = self.fresh_profile(group, now) {
            self.group_profile_known(&profile);
        }
        let mut query = None;
        if known.is_none() {
            if let Some(full) = crate::names::full_name(&inviter) {
                self.queries += 1;
                let asked = Uuid::from_u128(self.queries);
                if grid.find_avatar(asked, search_text(&full), now).is_ok() {
                    query = Some(asked);
                }
            }
        }
        let mut request = request;
        if let (Ask::Group { name: slot @ None, .. }, Some((profile, _))) = (&mut request.ask, self.fresh_profile(group, now)) {
            *slot = Some(profile.name);
        }
        if !matches!(request.ask, Ask::Group { name: Some(_), .. }) {
            let _ = self.ask_profile(group, grid, now);
        }
        self.group.inviting.push(Inviting {
            request,
            query,
            deadline: now + INVITATION_LIMIT,
        });
        self.invitations_due(Some(grid), now);
    }

    /// A profile that is known already names the group of the invitations
    /// into it.
    fn group_profile_known(&mut self, profile: &Profile) {
        for inviting in &mut self.group.inviting {
            if let Ask::Group { group, name, .. } = &mut inviting.request.ask {
                if *group == profile.id {
                    *name = Some(profile.name.clone());
                }
            }
        }
    }

    /// The avatar search for an inviter's name answered.
    pub(super) fn inviter_found<G: Grid>(&mut self, query: Uuid, avatars: &[(Uuid, String)], grid: Option<&mut G>, now: Instant) {
        let mut found_any = false;
        for inviting in self.group.inviting.iter_mut().filter(|inviting| inviting.query == Some(query)) {
            inviting.query = None;
            found_any = true;
            if let perms::Sender::Inviter { name, id, .. } = &mut inviting.request.sender {
                let full = crate::names::full_name(name).unwrap_or_default();
                if let Some((found, proper)) = exact_match(&full, avatars) {
                    *id = Some(*found);
                    self.known.insert(full, (*found, proper.clone()));
                }
            }
        }
        if found_any {
            self.invitations_due(grid, now);
        }
    }

    /// The avatar search for an inviter's name came to nothing: the
    /// invitation is decided without the UUID when it is next looked at.
    pub(super) fn inviter_lookup_failed(&mut self, query: Uuid) {
        for inviting in self.group.inviting.iter_mut().filter(|inviting| inviting.query == Some(query)) {
            inviting.query = None;
        }
    }

    /// Decides the invitations that know what they waited for, and those
    /// whose time is up with what is known.
    fn invitations_due<G: Grid>(&mut self, mut grid: Option<&mut G>, now: Instant) {
        let (ready, waiting) = std::mem::take(&mut self.group.inviting).into_iter().partition(|inviting| {
            let named = matches!(inviting.request.ask, Ask::Group { name: Some(_), .. });
            (named && inviting.query.is_none()) || now >= inviting.deadline
        });
        self.group.inviting = waiting;
        for inviting in ready {
            self.decide(inviting.request, grid.as_deref_mut(), now);
        }
    }

    /// Accepts an invitation the rules accept: only one without a fee.
    pub(super) fn accept_invitation_by_itself<G: Grid>(
        &mut self,
        request: &perms::Request,
        grid: &mut G,
        now: Instant,
    ) -> Result<Done, String> {
        let Ask::Group { group, id, .. } = request.ask else {
            return Err("not an invitation".to_string());
        };
        grid.answer_invitation(group, id, true, now)?;
        self.group.joining.push(Joining {
            group,
            words: request.group_words().unwrap_or_default(),
            reply: None,
            origin: Origin::Bot,
            via: Interface::Ctl,
            command: "accepted by itself",
            inviter: Some(request.sender.to_string()),
            fee: None,
            report: Report::Pending,
            granted: false,
            deadline: now + JOIN_LIMIT,
        });
        Ok(Done::Answer(None))
    }

    /// The one waiting invitation `who` names, taken out.
    fn named_invitation(&mut self, who: Option<&str>) -> Result<perms::Request, String> {
        let named = self.requests.invitations(who);
        let group = match named[..] {
            [] => {
                return Err(match who {
                    None => "no request of kind `group` waits: no invitation to a group".to_string(),
                    Some(who) => format!("no request of kind `group` from {who} waits"),
                })
            }
            [one] => match one.ask {
                Ask::Group { group, .. } => group,
                _ => unreachable!("an invitation"),
            },
            ref several => {
                let listed: Vec<String> = several
                    .iter()
                    .map(|request| {
                        let Ask::Group { group, ref name, .. } = request.ask else {
                            unreachable!()
                        };
                        let name = name.as_deref().map_or("?".to_string(), escape);
                        format!("{group} \"{name}\" from {}", request.sender)
                    })
                    .collect();
                return Err(format!(
                    "{} invitations are named; give the UUID of one's group: {}",
                    several.len(),
                    listed.join("; ")
                ));
            }
        };
        Ok(self.requests.take_invitation(group).expect("named"))
    }

    /// `accept group [WHO] [--pay SUM]`.
    pub(super) fn accept_group<G: Grid>(
        &mut self,
        who: Option<&str>,
        pay: Option<u32>,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<Done, String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.present()?;
        let request = self.named_invitation(who)?;
        let Ask::Group { group, id, ref name, .. } = request.ask else {
            unreachable!("an invitation")
        };
        let words = request.group_words().unwrap_or_default();
        let sent = match request.fee() {
            Some(fee) => match pay {
                None => Err(format!("joining {words} costs L${fee}; give `--pay {fee}` to pay it")),
                Some(sum) if sum < fee => Err(format!(
                    "joining {words} costs L${fee}, more than the L${sum} of `--pay`; give `--pay {fee}` to pay it"
                )),
                Some(_) => {
                    let name = name.clone().unwrap_or_else(|| group.to_string());
                    self.pay_fee(fee, group, &name, FeeFor::Accept { id }, self.via, reply, grid, now)
                }
            },
            None => grid.answer_invitation(group, id, true, now),
        };
        if let Err(reason) = sent {
            // It keeps its place among the waiting ones.
            self.requests.push(request);
            return Err(reason);
        }
        if !self.origin.quiet() {
            self.tell(format!("{}: accept group: {}: sent", self.origin.tag(), request.describe()));
        }
        self.group.joining.push(Joining {
            group,
            words,
            reply: Some(reply.clone()),
            origin: self.origin,
            via: self.via,
            command: "accept group",
            inviter: Some(request.sender.to_string()),
            fee: request.fee(),
            report: Report::Pending,
            granted: false,
            deadline: now + JOIN_LIMIT,
        });
        Ok(Done::Parked)
    }

    /// `decline group [WHO]`.
    pub(super) fn decline_group<G: Grid>(&mut self, who: Option<&str>, grid: Option<&mut G>, now: Instant) -> Result<Done, String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.present()?;
        let request = self.named_invitation(who)?;
        let Ask::Group { group, id, .. } = request.ask else {
            unreachable!("an invitation")
        };
        if let Err(reason) = grid.answer_invitation(group, id, false, now) {
            self.requests.push(request);
            return Err(reason);
        }
        if !self.origin.quiet() {
            self.tell(format!("{}: decline group: {}: declined", self.origin.tag(), request.describe()));
        }
        yes_answer(format!(
            "declined the invitation of {} into {}",
            request.sender,
            request.group_words().unwrap_or_default()
        ))
    }

    /// The answer to `group invitations`.
    fn invitations_text(&self) -> Option<String> {
        let lines: Vec<String> = self
            .requests
            .all_invitations()
            .iter()
            .filter_map(|request| {
                let Ask::Group { group, fee, ref name, .. } = request.ask else {
                    return None;
                };
                let (inviter, id) = request.inviter()?;
                let id = id.map_or("?".to_string(), |id| id.to_string());
                let name = name.as_deref().map_or("?".to_string(), escape);
                Some(format!("{group}\t{}\t{id}\t{}\t{name}", fee.max(0), escape(inviter)))
            })
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    // ----------------------------------------------------------- deadlines

    /// Answers what waited for the grid too long.
    pub fn groups_due<G: Grid>(&mut self, mut grid: Option<&mut G>, now: Instant) {
        if self.group.ask_again.is_some_and(|at| now >= at) {
            self.group.ask_again = None;
            if let Some(grid) = grid.as_deref_mut() {
                if let Err(reason) = grid.ask_groups(now) {
                    crate::debug!("could not ask for the avatar's groups again: {reason}");
                }
            }
        }
        for early in std::mem::take(&mut self.group.early) {
            match now >= early.deadline {
                false => self.group.early.push(early),
                true => {
                    let reason = format!("the grid has not told the avatar's groups within {} s", GROUPS_LIMIT.as_secs());
                    refuse_from(early.origin, early.request.name(), &early.reply, reason);
                }
            }
        }
        for wait in std::mem::take(&mut self.group.activating) {
            match now >= wait.deadline {
                false => self.group.activating.push(wait),
                true => {
                    let reason = format!(
                        "the grid has not confirmed within {} s that {} is active",
                        ACTIVATE_LIMIT.as_secs(),
                        wait.words
                    );
                    if !wait.origin.quiet() {
                        self.tell(format!("{}: group activate {}: not confirmed", wait.origin.tag(), wait.words));
                    }
                    let _ = wait.reply.send(Response::Error(reason));
                }
            }
        }
        for (group, then, deadline) in std::mem::take(&mut self.group.for_profile) {
            if now < deadline {
                self.group.for_profile.push((group, then, deadline));
                continue;
            }
            let (origin, reply, command) = match then {
                ForProfile::Info { origin, reply } => (origin, reply, "group info"),
                ForProfile::Join { origin, reply, .. } => (origin, reply, "group join"),
            };
            let reason = format!(
                "the grid knows no group {group} or has not answered for it within {} s",
                PROFILE_LIMIT.as_secs()
            );
            refuse_from(origin, command, &reply, reason);
        }
        for wait in std::mem::take(&mut self.group.infos) {
            let named = self.owners.contains_key(&wait.profile.founder);
            if named || now >= wait.deadline {
                let text = self.info_text(&wait.profile);
                answer_later(wait.origin, "group info", &wait.reply, Ok(Some(text)));
            } else {
                self.group.infos.push(wait);
            }
        }
        for join in std::mem::take(&mut self.group.joining) {
            if now < join.deadline {
                self.group.joining.push(join);
                continue;
            }
            let limit = JOIN_LIMIT.as_secs();
            let error = match (join.fee, &join.report) {
                (Some(fee), Report::Paid(_)) => format!(
                    "the fee of L${fee} was paid, and the grid has not confirmed within {limit} s that the avatar is in {}",
                    join.words
                ),
                (Some(fee), Report::Pending) => format!(
                    "the join was sent, and the grid has not confirmed within {limit} s that the avatar is in {}; \
                     the fee of L${fee} counts as paid",
                    join.words
                ),
                (None, _) => format!("the grid has not confirmed within {limit} s that the avatar is in {}", join.words),
            };
            let error = match join.granted {
                true => format!("{error}; it accepted the join, and has not listed the group since"),
                false => error,
            };
            self.join_ended(join, Err(error));
        }
        for leave in std::mem::take(&mut self.group.leaving) {
            match now >= leave.deadline {
                false => self.group.leaving.push(leave),
                true => {
                    let error = format!(
                        "the grid has not confirmed within {} s that the avatar left {}",
                        LEAVE_LIMIT.as_secs(),
                        leave.words
                    );
                    self.leave_ended(leave, Err(error));
                }
            }
        }
        let mut late = Vec::new();
        for (group, chat) in &self.group.sessions {
            if let Chat::Entering { deadline, .. } = chat {
                if now >= *deadline {
                    late.push(*group);
                }
            }
        }
        for group in late {
            if let Some(Chat::Entering { lines, .. }) = self.group.sessions.remove(&group) {
                let words = self
                    .group
                    .groups
                    .get(group)
                    .map_or_else(|| format!("the group {group}"), Membership::words);
                self.tell(format!(
                    "groups: the grid has not let the avatar into the chat session of {words} within {} s",
                    SESSION_LIMIT.as_secs()
                ));
                for line in lines {
                    let reason = format!(
                        "nothing said in {words}: the grid has not let the avatar into its chat session within {} s",
                        SESSION_LIMIT.as_secs()
                    );
                    refuse_from(line.origin, "group chat", &line.reply, reason);
                }
            }
        }
        self.invitations_due(grid, now);
    }

    /// When `groups_due` next has something to do.
    pub(super) fn groups_deadline(&self) -> Option<Instant> {
        let group = &self.group;
        let early = group.early.iter().map(|early| early.deadline);
        let activating = group.activating.iter().map(|wait| wait.deadline);
        let profiles = group.for_profile.iter().map(|(_, _, deadline)| *deadline);
        let infos = group.infos.iter().map(|wait| wait.deadline);
        let joining = group.joining.iter().map(|join| join.deadline);
        let leaving = group.leaving.iter().map(|leave| leave.deadline);
        let inviting = group.inviting.iter().map(|inviting| inviting.deadline);
        let sessions = group.sessions.values().filter_map(|chat| match chat {
            Chat::Entering { deadline, .. } => Some(*deadline),
            Chat::In => None,
        });
        early
            .chain(group.ask_again)
            .chain(activating)
            .chain(profiles)
            .chain(infos)
            .chain(joining)
            .chain(leaving)
            .chain(inviting)
            .chain(sessions)
            .min()
    }

    /// The session is over: whatever waited for the grid fails, and the
    /// groups are forgotten.
    pub(super) fn groups_lost(&mut self) {
        let lost = "the session was lost".to_string();
        for early in std::mem::take(&mut self.group.early) {
            refuse_from(early.origin, early.request.name(), &early.reply, NOT_CONNECTED.to_string());
        }
        for wait in std::mem::take(&mut self.group.activating) {
            refuse_from(wait.origin, "group activate", &wait.reply, lost.clone());
        }
        for (_, then, _) in std::mem::take(&mut self.group.for_profile) {
            match then {
                ForProfile::Info { origin, reply } => refuse_from(origin, "group info", &reply, lost.clone()),
                ForProfile::Join { origin, reply, .. } => refuse_from(origin, "group join", &reply, lost.clone()),
            }
        }
        for wait in std::mem::take(&mut self.group.infos) {
            refuse_from(wait.origin, "group info", &wait.reply, lost.clone());
        }
        for join in std::mem::take(&mut self.group.joining) {
            let error = match join.fee {
                Some(fee) => format!(
                    "the join was sent, and the session was lost before the grid confirmed that the avatar is in {}; \
                     the fee of L${fee} counts as paid",
                    join.words
                ),
                None => format!(
                    "the session was lost before the grid confirmed that the avatar is in {}",
                    join.words
                ),
            };
            self.join_ended(join, Err(error));
        }
        for leave in std::mem::take(&mut self.group.leaving) {
            let error = format!(
                "the session was lost before the grid confirmed that the avatar left {}",
                leave.words
            );
            self.leave_ended(leave, Err(error));
        }
        for (_, chat) in std::mem::take(&mut self.group.sessions) {
            if let Chat::Entering { lines, .. } = chat {
                for line in lines {
                    refuse_from(line.origin, "group chat", &line.reply, lost.clone());
                }
            }
        }
        self.group.inviting.clear();
        self.group.settled.clear();
        self.group.ask_again = None;
        self.group.profiles.clear();
        self.group.asked.clear();
        self.group.groups.clear();
        self.group.session_changed = true;
    }
}

fn join_words(join: &Joining) -> &str {
    &join.words
}

#[cfg(test)]
mod tests;
