//! The control loop's decision logic. `decide` is a pure function of what is
//! observed and remembered; the event methods on `Reconciler` are the only
//! places that state changes. No I/O happens here.

use crate::backoff::Backoff;
use crate::login::{Failure, Start};
use std::time::{Duration, Instant};

const LOGIN_FIRST: Duration = Duration::from_secs(10);
const LOGIN_MAX: Duration = Duration::from_secs(5 * 60);
const REFUSED_LOGIN_MAX: Duration = Duration::from_secs(30 * 60);
const TELEPORT_FIRST: Duration = Duration::from_secs(10);
const TELEPORT_MAX: Duration = Duration::from_secs(5 * 60);
const SIT_FIRST: Duration = Duration::from_secs(5);
const SIT_MAX: Duration = Duration::from_secs(60);
/// How long a session or a seat must last before its backoff starts over.
const STABLE_AFTER: Duration = Duration::from_secs(5 * 60);
/// Consecutive failed sits before the avatar is repositioned.
const SIT_FAILURES_BEFORE_REPOSITION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seat {
    /// Not yet learned since entering the region.
    Unknown,
    NotSeated,
    SeatedOnTarget,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Login(Start),
    Teleport,
    Sit,
    /// Unlike the others this one is over as soon as it is sent: the
    /// simulator does not answer a ground sit.
    SitGround,
}

/// What the bot seats the avatar on by itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeatGoal {
    Object,
    Ground,
}

/// What the config asks the bot to hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    /// The region to stay in; without one every region will do.
    pub region: Option<String>,
    /// Where a login asks to be placed.
    pub start: Start,
    pub seat: Option<SeatGoal>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Nothing to do until the given time, or until something changes.
    Wait(Option<Instant>),
    Start(Action),
    Exit { code: i32, message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    /// The region the avatar is in; `None` while not connected.
    pub region: Option<String>,
    pub seat: Seat,
}

#[derive(Debug, Clone)]
pub struct Memory {
    in_flight: Option<Action>,
    login: Backoff,
    teleport: Backoff,
    sit: Backoff,
    next_login: Option<Instant>,
    next_teleport: Option<Instant>,
    next_sit: Option<Instant>,
    had_successful_login: bool,
    last_login_refused: bool,
    last_start: Start,
    connected_since: Option<Instant>,
    seated_since: Option<Instant>,
    sit_failures: u32,
    reposition: bool,
    /// The ground sit the goal asks for has been sent since the avatar last
    /// arrived, stood up from an object, or came back to holding mode.
    ground_done: bool,
    /// The avatar has been seen seated on an object since that ground sit.
    on_object: bool,
    fatal: Option<String>,
    /// The operator placed the avatar by hand: stay connected, and leave
    /// region and seat alone.
    manual: bool,
}

pub fn same_region(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn gate(not_before: Option<Instant>, action: Action, now: Instant) -> Decision {
    match not_before {
        Some(at) if now < at => Decision::Wait(Some(at)),
        _ => Decision::Start(action),
    }
}

/// Chooses the next step from the current state, the placement mode and
/// the goal alone, by the first rule that applies: log in, nothing (manual
/// mode), teleport, sit on the object, sit on the ground, or nothing.
pub fn decide(observed: &Observed, memory: &Memory, goal: &Goal, now: Instant) -> Decision {
    if let Some(message) = &memory.fatal {
        return Decision::Exit { code: 1, message: message.clone() };
    }
    if memory.in_flight.is_some() {
        return Decision::Wait(None);
    }
    let Some(region) = &observed.region else {
        let start = if memory.last_login_refused || memory.manual { Start::Last } else { goal.start };
        return gate(memory.next_login, Action::Login(start), now);
    };
    if memory.manual {
        return Decision::Wait(None);
    }
    if goal.region.as_deref().is_some_and(|target| !same_region(region, target)) {
        return gate(memory.next_teleport, Action::Teleport, now);
    }
    match (goal.seat, observed.seat) {
        (Some(SeatGoal::Object), Seat::SeatedOnTarget) => Decision::Wait(None),
        // Without a region there is no position to go back to.
        (Some(SeatGoal::Object), _) if memory.reposition && goal.region.is_some() => {
            gate(memory.next_teleport, Action::Teleport, now)
        }
        (Some(SeatGoal::Object), _) => gate(memory.next_sit, Action::Sit, now),
        (Some(SeatGoal::Ground), _) if !memory.ground_done => Decision::Start(Action::SitGround),
        _ => Decision::Wait(None),
    }
}

pub struct Reconciler {
    goal: Goal,
    observed: Observed,
    memory: Memory,
}

impl Reconciler {
    pub fn new(goal: Goal) -> Reconciler {
        let start = goal.start;
        Reconciler {
            goal,
            observed: Observed { region: None, seat: Seat::Unknown },
            memory: Memory {
                in_flight: None,
                login: Backoff::new(LOGIN_FIRST),
                teleport: Backoff::new(TELEPORT_FIRST),
                sit: Backoff::new(SIT_FIRST),
                next_login: None,
                next_teleport: None,
                next_sit: None,
                had_successful_login: false,
                last_login_refused: false,
                last_start: start,
                connected_since: None,
                seated_since: None,
                sit_failures: 0,
                reposition: false,
                ground_done: false,
                on_object: false,
                fatal: None,
                manual: false,
            },
        }
    }

    pub fn decide(&self, now: Instant) -> Decision {
        decide(&self.observed, &self.memory, &self.goal, now)
    }

    #[cfg(test)]
    pub fn observed(&self) -> &Observed {
        &self.observed
    }

    pub fn is_manual(&self) -> bool {
        self.memory.manual
    }

    /// Switches between holding the configured place (the default) and
    /// leaving the avatar where the operator put it. Going manual gives up
    /// a teleport or sit of the bot's own without counting it as failed;
    /// coming back starts the return at once.
    pub fn set_manual(&mut self, manual: bool) {
        self.memory.manual = manual;
        if manual {
            if matches!(self.memory.in_flight, Some(Action::Teleport | Action::Sit)) {
                self.memory.in_flight = None;
            }
        } else {
            // What happened to the avatar under the operator's hand says
            // nothing about how well the bot's own attempts go.
            self.memory.teleport.reset();
            self.memory.sit.reset();
            self.memory.next_teleport = None;
            self.memory.next_sit = None;
            self.memory.sit_failures = 0;
            self.memory.reposition = false;
            self.memory.ground_done = false;
        }
    }

    pub fn goal(&self) -> &Goal {
        &self.goal
    }

    /// Whether the avatar is in the region it is to stay in; without a
    /// configured region every region is.
    pub fn in_target_region(&self) -> bool {
        self.observed.region.as_deref().is_some_and(|r| match &self.goal.region {
            Some(target) => same_region(r, target),
            None => true,
        })
    }

    /// An action returned by `decide` is now in progress, or in the case of
    /// a ground sit already over.
    pub fn started(&mut self, action: Action) {
        match action {
            Action::SitGround => {
                self.memory.ground_done = true;
                self.memory.on_object = false;
            }
            Action::Login(start) => {
                self.memory.in_flight = Some(action);
                self.memory.last_start = start;
            }
            Action::Teleport | Action::Sit => self.memory.in_flight = Some(action),
        }
    }

    fn login_delay(&mut self, now: Instant) -> Duration {
        let max = if self.memory.last_login_refused && self.memory.had_successful_login {
            REFUSED_LOGIN_MAX
        } else {
            LOGIN_MAX
        };
        let delay = self.memory.login.next_delay(max);
        self.memory.next_login = Some(now + delay);
        delay
    }

    /// The grid accepted the login; the action stays in progress until the
    /// avatar has entered a region.
    pub fn login_accepted(&mut self) {
        self.memory.last_login_refused = false;
    }

    /// The grid demands something the bot cannot supply. Never retried.
    pub fn login_needs_operator(&mut self, failure: &Failure) {
        self.memory.in_flight = None;
        self.memory.fatal = Some(if failure.reason == "mfa_challenge" {
            "the grid asks for multi-factor authentication, which is not supported".to_string()
        } else {
            format!("the grid demands operator action ({}): {}", failure.reason, failure.message)
        });
    }

    /// The grid rejected the attempt with its credential-or-location reason.
    /// Returns the delay before the next attempt, or `None` when giving up.
    pub fn login_refused(&mut self, failure: &Failure, now: Instant) -> Option<Duration> {
        self.memory.in_flight = None;
        if !self.memory.had_successful_login && self.memory.last_start == Start::Last {
            self.memory.fatal = Some(format!("credentials rejected by the grid: {}", failure.message));
            return None;
        }
        self.memory.last_login_refused = true;
        Some(self.login_delay(now))
    }

    /// Any other login failure. Returns the delay before the next attempt.
    pub fn login_failed(&mut self, now: Instant) -> Duration {
        self.memory.in_flight = None;
        self.memory.last_login_refused = false;
        self.login_delay(now)
    }

    /// The avatar is connected and in the named region, after a login or a
    /// teleport, whether the bot asked for it or not.
    pub fn region_entered(&mut self, name: &str, now: Instant) {
        match self.memory.in_flight.take() {
            Some(Action::Login(_)) => {
                self.memory.had_successful_login = true;
                self.memory.connected_since = Some(now);
            }
            Some(Action::Teleport) => self.memory.teleport.reset(),
            _ => {}
        }
        self.observed.region = Some(name.to_string());
        self.observed.seat = Seat::Unknown;
        self.memory.seated_since = None;
        self.memory.sit_failures = 0;
        self.memory.reposition = false;
        self.memory.ground_done = false;
        self.memory.on_object = false;
    }

    /// The session ended, whatever the cause. Returns the delay before the
    /// next login.
    pub fn session_lost(&mut self, now: Instant) -> Duration {
        let stable = self
            .memory
            .connected_since
            .is_some_and(|since| now.duration_since(since) >= STABLE_AFTER);
        if stable {
            self.memory.login.reset();
        }
        self.observed = Observed { region: None, seat: Seat::Unknown };
        self.memory.in_flight = None;
        self.memory.connected_since = None;
        self.memory.seated_since = None;
        self.login_delay(now)
    }

    /// A teleport inside the current region finished.
    pub fn teleport_arrived(&mut self) {
        self.memory.in_flight = None;
        self.memory.teleport.reset();
        self.observed.seat = Seat::NotSeated;
        self.memory.seated_since = None;
        self.memory.sit_failures = 0;
        self.memory.reposition = false;
        self.memory.ground_done = false;
        self.memory.on_object = false;
    }

    /// Returns the delay before the next teleport attempt.
    pub fn teleport_failed(&mut self, now: Instant) -> Duration {
        self.memory.in_flight = None;
        let delay = self.memory.teleport.next_delay(TELEPORT_MAX);
        self.memory.next_teleport = Some(now + delay);
        delay
    }

    /// The avatar is seated on the target, by the bot's sit or otherwise.
    pub fn seated(&mut self, now: Instant) {
        if self.memory.in_flight == Some(Action::Sit) {
            self.memory.in_flight = None;
        }
        self.observed.seat = Seat::SeatedOnTarget;
        self.memory.on_object = true;
        self.memory.seated_since = Some(now);
        self.memory.sit_failures = 0;
        self.memory.reposition = false;
    }

    /// Returns the delay before the next sit attempt.
    pub fn sit_failed(&mut self, now: Instant) -> Duration {
        self.memory.in_flight = None;
        self.memory.sit_failures += 1;
        if self.memory.sit_failures >= SIT_FAILURES_BEFORE_REPOSITION {
            self.memory.reposition = true;
        }
        self.sit_delay(now)
    }

    /// The avatar is seated on an object that is not the target: as good
    /// as standing to the bot, except that getting up from it later undoes
    /// a ground sit.
    pub fn seated_elsewhere(&mut self, now: Instant) {
        self.standing(now);
        self.memory.on_object = true;
    }

    /// The simulator reports the avatar standing. Returns the delay before
    /// sitting again when this ended a seat on the target.
    pub fn standing(&mut self, now: Instant) -> Option<Duration> {
        // The first report after an arrival says "standing" too; only
        // getting up from an object calls for another ground sit.
        if std::mem::take(&mut self.memory.on_object) {
            self.memory.ground_done = false;
        }
        let was_seated = self.observed.seat == Seat::SeatedOnTarget;
        self.observed.seat = Seat::NotSeated;
        if !was_seated {
            return None;
        }
        let stable = self
            .memory
            .seated_since
            .take()
            .is_some_and(|since| now.duration_since(since) >= STABLE_AFTER);
        if stable {
            self.memory.sit.reset();
        }
        Some(self.sit_delay(now))
    }

    fn sit_delay(&mut self, now: Instant) -> Duration {
        let delay = self.memory.sit.next_delay(SIT_MAX);
        self.memory.next_sit = Some(now + delay);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "Target";

    /// The goal of a full config: a region and an object to sit on.
    fn holding() -> Goal {
        Goal { region: Some(TARGET.to_string()), start: Start::Target, seat: Some(SeatGoal::Object) }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn failure(reason: &str) -> Failure {
        Failure { reason: reason.to_string(), message: format!("{reason} message") }
    }

    /// Starts whatever `decide` returns and reports it.
    fn step(rec: &mut Reconciler, now: Instant) -> Decision {
        let decision = rec.decide(now);
        if let Decision::Start(action) = decision {
            rec.started(action);
        }
        decision
    }

    fn start(action: Action) -> Decision {
        Decision::Start(action)
    }

    /// A reconciler that has logged in to `region`.
    fn logged_in(region: &str, now: Instant) -> Reconciler {
        let mut rec = Reconciler::new(holding());
        assert_eq!(step(&mut rec, now), start(Action::Login(Start::Target)));
        rec.login_accepted();
        rec.region_entered(region, now);
        rec
    }

    fn seated(now: Instant) -> Reconciler {
        let mut rec = logged_in(TARGET, now);
        assert_eq!(step(&mut rec, now), start(Action::Sit));
        rec.seated(now);
        rec
    }

    #[test]
    fn login_landing_in_target_region_sits_without_teleporting() {
        let now = Instant::now();
        let mut rec = Reconciler::new(holding());
        assert_eq!(step(&mut rec, now), start(Action::Login(Start::Target)));
        // One action at a time.
        assert_eq!(rec.decide(now), Decision::Wait(None));
        rec.login_accepted();
        assert_eq!(rec.decide(now), Decision::Wait(None));
        rec.region_entered("target ", now);
        assert_eq!(step(&mut rec, now), start(Action::Sit));
        assert_eq!(rec.decide(now), Decision::Wait(None));
        rec.seated(now);
        assert_eq!(rec.decide(now + secs(3600)), Decision::Wait(None));
    }

    #[test]
    fn start_in_another_region_teleports_then_sits() {
        let now = Instant::now();
        let mut rec = logged_in("Home", now);
        assert_eq!(step(&mut rec, now), start(Action::Teleport));
        rec.region_entered(TARGET, now);
        assert_eq!(step(&mut rec, now), start(Action::Sit));
    }

    #[test]
    fn kicked_while_seated_logs_in_again_after_the_login_delay() {
        let now = Instant::now();
        let mut rec = seated(now);
        let lost = now + secs(60);
        assert_eq!(rec.session_lost(lost), secs(10));
        assert_eq!(rec.decide(lost + secs(9)), Decision::Wait(Some(lost + secs(10))));
        assert_eq!(step(&mut rec, lost + secs(10)), start(Action::Login(Start::Target)));
        rec.login_accepted();
        rec.region_entered(TARGET, lost + secs(11));
        assert_eq!(step(&mut rec, lost + secs(11)), start(Action::Sit));
    }

    #[test]
    fn moved_to_another_region_teleports_back() {
        let now = Instant::now();
        let mut rec = seated(now);
        rec.region_entered("Elsewhere", now + secs(5));
        assert_eq!(rec.observed().seat, Seat::Unknown);
        assert_eq!(step(&mut rec, now + secs(5)), start(Action::Teleport));
    }

    // ------------------------------------------------------- manual mode

    #[test]
    fn manual_mode_leaves_region_and_seat_alone() {
        let now = Instant::now();
        let mut rec = seated(now);
        rec.set_manual(true);
        assert!(rec.is_manual());
        assert_eq!(rec.standing(now + secs(1)), Some(secs(5)));
        assert_eq!(rec.decide(now + secs(100)), Decision::Wait(None));
        rec.region_entered("Elsewhere", now + secs(200));
        assert_eq!(rec.decide(now + secs(200)), Decision::Wait(None));
        assert_eq!(rec.decide(now + secs(4000)), Decision::Wait(None));
    }

    #[test]
    fn kicked_in_manual_mode_logs_in_at_the_last_location_and_stays() {
        let now = Instant::now();
        let mut rec = seated(now);
        rec.set_manual(true);
        rec.region_entered("Elsewhere", now + secs(5));
        let lost = now + secs(60);
        assert_eq!(rec.session_lost(lost), secs(10));
        assert_eq!(rec.decide(lost + secs(9)), Decision::Wait(Some(lost + secs(10))));
        assert_eq!(step(&mut rec, lost + secs(10)), start(Action::Login(Start::Last)));
        rec.login_accepted();
        rec.region_entered("Elsewhere", lost + secs(11));
        assert_eq!(rec.decide(lost + secs(11)), Decision::Wait(None));
    }

    #[test]
    fn going_manual_gives_up_the_bots_own_sit_or_teleport() {
        let now = Instant::now();
        let mut rec = logged_in(TARGET, now);
        assert_eq!(step(&mut rec, now), start(Action::Sit));
        rec.set_manual(true);
        // Not stuck waiting for the sit to end, and no failure counted.
        rec.set_manual(false);
        assert_eq!(step(&mut rec, now), start(Action::Sit));

        let mut rec = logged_in("Elsewhere", now);
        assert_eq!(step(&mut rec, now), start(Action::Teleport));
        rec.set_manual(true);
        rec.set_manual(false);
        assert_eq!(step(&mut rec, now), start(Action::Teleport));
    }

    #[test]
    fn going_manual_does_not_interrupt_a_login() {
        let now = Instant::now();
        let mut rec = Reconciler::new(holding());
        assert_eq!(step(&mut rec, now), start(Action::Login(Start::Target)));
        rec.set_manual(true);
        assert_eq!(rec.decide(now), Decision::Wait(None));
        rec.login_accepted();
        rec.region_entered(TARGET, now);
        assert_eq!(rec.decide(now), Decision::Wait(None));
    }

    #[test]
    fn leaving_manual_mode_returns_at_once() {
        let now = Instant::now();
        let mut rec = seated(now);
        // Backoff left over from being unseated and from failed sits.
        assert_eq!(rec.standing(now + secs(1)), Some(secs(5)));
        assert_eq!(step(&mut rec, now + secs(6)), start(Action::Sit));
        assert_eq!(rec.sit_failed(now + secs(21)), secs(10));
        rec.set_manual(true);
        rec.region_entered("Elsewhere", now + secs(30));
        rec.set_manual(false);
        assert!(!rec.is_manual());
        assert_eq!(step(&mut rec, now + secs(30)), start(Action::Teleport));
        rec.region_entered(TARGET, now + secs(35));
        assert_eq!(step(&mut rec, now + secs(35)), start(Action::Sit));

        // The delays start over as well: being unseated now waits 5 s again.
        rec.seated(now + secs(36));
        assert_eq!(rec.standing(now + secs(40)), Some(secs(5)));

        // In the target region already: the sit does not wait either.
        let mut rec = seated(now);
        rec.set_manual(true);
        assert_eq!(rec.standing(now + secs(1)), Some(secs(5)));
        rec.set_manual(false);
        assert_eq!(step(&mut rec, now + secs(1)), start(Action::Sit));
    }

    #[test]
    fn region_restart_keeps_teleporting_until_the_region_is_back() {
        let now = Instant::now();
        let mut rec = seated(now);
        let mut at = now + secs(600);
        rec.session_lost(at);
        at += secs(10);
        assert_eq!(step(&mut rec, at), start(Action::Login(Start::Target)));
        rec.login_accepted();
        rec.region_entered("Home", at);
        for expected in [10, 20, 40] {
            assert_eq!(step(&mut rec, at), start(Action::Teleport));
            assert_eq!(rec.teleport_failed(at), secs(expected));
            assert_eq!(rec.decide(at), Decision::Wait(Some(at + secs(expected))));
            at += secs(expected);
        }
        assert_eq!(step(&mut rec, at), start(Action::Teleport));
        rec.region_entered(TARGET, at);
        assert_eq!(step(&mut rec, at), start(Action::Sit));
    }

    #[test]
    fn login_delay_doubles_up_to_five_minutes_while_the_grid_is_down() {
        let mut at = Instant::now();
        let mut rec = Reconciler::new(holding());
        let mut delays = Vec::new();
        for _ in 0..8 {
            assert_eq!(step(&mut rec, at), start(Action::Login(Start::Target)));
            let delay = rec.login_failed(at);
            delays.push(delay.as_secs());
            at += delay;
        }
        assert_eq!(delays, [10, 20, 40, 80, 160, 300, 300, 300]);
        // The server is back: the next attempt works.
        assert_eq!(step(&mut rec, at), start(Action::Login(Start::Target)));
        rec.login_accepted();
        rec.region_entered(TARGET, at);
        assert_eq!(step(&mut rec, at), start(Action::Sit));
    }

    #[test]
    fn sessions_that_keep_dropping_slow_the_logins_down() {
        let mut at = Instant::now();
        let mut rec = Reconciler::new(holding());
        let mut delays = Vec::new();
        for _ in 0..7 {
            assert_eq!(step(&mut rec, at), start(Action::Login(Start::Target)));
            rec.login_accepted();
            rec.region_entered(TARGET, at);
            at += secs(30);
            let delay = rec.session_lost(at);
            delays.push(delay.as_secs());
            at += delay;
        }
        assert_eq!(delays, [10, 20, 40, 80, 160, 300, 300]);
    }

    #[test]
    fn login_delay_starts_over_only_after_five_minutes_connected() {
        let mut at = Instant::now();
        let mut rec = logged_in(TARGET, at);
        at += secs(299);
        assert_eq!(rec.session_lost(at), secs(10));
        at += secs(10);
        step(&mut rec, at);
        rec.login_accepted();
        rec.region_entered(TARGET, at);
        at += secs(299);
        assert_eq!(rec.session_lost(at), secs(20), "short session keeps the backoff growing");
        at += secs(20);
        step(&mut rec, at);
        rec.login_accepted();
        rec.region_entered(TARGET, at);
        at += secs(300);
        assert_eq!(rec.session_lost(at), secs(10), "stable session resets it");
    }

    #[test]
    fn loss_during_login_handshake_counts_as_a_failed_login() {
        let now = Instant::now();
        let mut rec = Reconciler::new(holding());
        step(&mut rec, now);
        rec.login_accepted();
        assert_eq!(rec.session_lost(now + secs(30)), secs(10));
        assert_eq!(step(&mut rec, now + secs(40)), start(Action::Login(Start::Target)));
    }

    #[test]
    fn repeatedly_ejected_from_the_seat_slows_down_to_one_sit_a_minute() {
        let mut at = Instant::now();
        let mut rec = logged_in(TARGET, at);
        let mut delays = Vec::new();
        for _ in 0..7 {
            assert_eq!(step(&mut rec, at), start(Action::Sit));
            rec.seated(at);
            at += secs(1);
            let delay = rec.standing(at).unwrap();
            assert_eq!(rec.decide(at), Decision::Wait(Some(at + delay)));
            delays.push(delay.as_secs());
            at += delay;
        }
        assert_eq!(delays, [5, 10, 20, 40, 60, 60, 60]);
    }

    #[test]
    fn sit_delay_starts_over_after_five_minutes_seated() {
        let mut at = Instant::now();
        let mut rec = seated(at);
        at += secs(1);
        assert_eq!(rec.standing(at), Some(secs(5)));
        at += secs(5);
        step(&mut rec, at);
        rec.seated(at);
        at += secs(300);
        assert_eq!(rec.standing(at), Some(secs(5)));
    }

    #[test]
    fn learning_the_avatar_stands_causes_no_delay() {
        let now = Instant::now();
        let mut rec = logged_in(TARGET, now);
        assert_eq!(rec.standing(now), None);
        assert_eq!(rec.observed().seat, Seat::NotSeated);
        assert_eq!(step(&mut rec, now), start(Action::Sit));
    }

    #[test]
    fn three_failed_sits_reposition_before_the_next_sit() {
        let mut at = Instant::now();
        let mut rec = logged_in(TARGET, at);
        for expected in [5, 10, 20] {
            assert_eq!(step(&mut rec, at), start(Action::Sit));
            assert_eq!(rec.sit_failed(at), secs(expected));
            at += secs(expected);
        }
        // Still in the target region, but the next action is a teleport.
        assert_eq!(step(&mut rec, at), start(Action::Teleport));
        rec.teleport_arrived();
        assert_eq!(rec.observed().seat, Seat::NotSeated);
        assert_eq!(step(&mut rec, at), start(Action::Sit));
        // The count starts again after repositioning.
        rec.sit_failed(at);
        at += secs(60);
        assert_eq!(step(&mut rec, at), start(Action::Sit));
    }

    #[test]
    fn failed_reposition_is_retried_with_the_teleport_delay() {
        let mut at = Instant::now();
        let mut rec = logged_in(TARGET, at);
        for _ in 0..3 {
            step(&mut rec, at);
            at += rec.sit_failed(at);
        }
        assert_eq!(step(&mut rec, at), start(Action::Teleport));
        assert_eq!(rec.teleport_failed(at), secs(10));
        assert_eq!(rec.decide(at), Decision::Wait(Some(at + secs(10))));
        assert_eq!(step(&mut rec, at + secs(10)), start(Action::Teleport));
    }

    #[test]
    fn wrong_password_at_startup_exits_after_two_attempts() {
        let now = Instant::now();
        let mut rec = Reconciler::new(holding());
        assert_eq!(step(&mut rec, now), start(Action::Login(Start::Target)));
        assert_eq!(rec.login_refused(&failure("key"), now), Some(secs(10)));
        // The second attempt asks for the last location.
        assert_eq!(step(&mut rec, now + secs(10)), start(Action::Login(Start::Last)));
        assert_eq!(rec.login_refused(&failure("key"), now + secs(10)), None);
        match rec.decide(now + secs(10)) {
            Decision::Exit { code: 1, message } => assert!(message.contains("credentials rejected")),
            other => panic!("expected exit, got {other:?}"),
        }
    }

    #[test]
    fn target_region_down_at_startup_logs_in_at_last_location() {
        let now = Instant::now();
        let mut rec = Reconciler::new(holding());
        step(&mut rec, now);
        rec.login_refused(&failure("key"), now);
        assert_eq!(step(&mut rec, now + secs(10)), start(Action::Login(Start::Last)));
        rec.login_accepted();
        rec.region_entered("Home", now + secs(11));
        assert_eq!(step(&mut rec, now + secs(11)), start(Action::Teleport));
        // Later logins ask for the target region again.
        rec.session_lost(now + secs(400));
        assert_eq!(step(&mut rec, now + secs(1000)), start(Action::Login(Start::Target)));
    }

    #[test]
    fn refusals_after_a_successful_login_are_retried_up_to_thirty_minutes_apart() {
        let mut at = Instant::now();
        let mut rec = logged_in(TARGET, at);
        at += secs(600);
        at += rec.session_lost(at);
        assert_eq!(step(&mut rec, at), start(Action::Login(Start::Target)));
        let mut delays = Vec::new();
        for _ in 0..10 {
            let delay = rec.login_refused(&failure("key"), at).expect("must not give up");
            delays.push(delay.as_secs());
            at += delay;
            assert_eq!(step(&mut rec, at), start(Action::Login(Start::Last)));
        }
        assert_eq!(delays, [20, 40, 80, 160, 320, 640, 1280, 1800, 1800, 1800]);
    }

    #[test]
    fn multi_factor_challenge_exits_without_retrying() {
        let now = Instant::now();
        let mut rec = Reconciler::new(holding());
        step(&mut rec, now);
        rec.login_needs_operator(&failure("mfa_challenge"));
        match rec.decide(now + secs(3600)) {
            Decision::Exit { code: 1, message } => {
                assert!(message.contains("multi-factor authentication"), "{message}")
            }
            other => panic!("expected exit, got {other:?}"),
        }
    }

    #[test]
    fn region_names_compare_without_case_or_padding() {
        assert!(same_region("Target", " target "));
        assert!(!same_region("Target", "Target 2"));
    }

    // ------------------------------------------------- partial goals

    fn goal(region: Option<&str>, start: Start, seat: Option<SeatGoal>) -> Goal {
        Goal { region: region.map(str::to_string), start, seat }
    }

    /// A reconciler with `goal` that has logged in to `region`.
    fn arrived(goal: Goal, region: &str, now: Instant) -> Reconciler {
        let start_at = goal.start;
        let mut rec = Reconciler::new(goal);
        assert_eq!(step(&mut rec, now), start(Action::Login(start_at)));
        rec.login_accepted();
        rec.region_entered(region, now);
        rec
    }

    #[test]
    fn without_region_and_seat_the_bot_only_stays_connected() {
        let now = Instant::now();
        let mut rec = arrived(goal(None, Start::Last, None), "Anywhere", now);
        assert!(rec.in_target_region());
        assert_eq!(rec.decide(now), Decision::Wait(None));
        assert_eq!(rec.standing(now + secs(1)), None);
        rec.region_entered("Elsewhere", now + secs(5));
        assert_eq!(rec.decide(now + secs(4000)), Decision::Wait(None));
        let lost = now + secs(5000);
        assert_eq!(rec.session_lost(lost), secs(10));
        assert_eq!(step(&mut rec, lost + secs(10)), start(Action::Login(Start::Last)));
    }

    #[test]
    fn start_at_home_falls_back_to_the_last_location_when_refused() {
        let now = Instant::now();
        let mut rec = Reconciler::new(goal(None, Start::Home, None));
        assert_eq!(step(&mut rec, now), start(Action::Login(Start::Home)));
        assert_eq!(rec.login_refused(&failure("key"), now), Some(secs(10)));
        assert_eq!(step(&mut rec, now + secs(10)), start(Action::Login(Start::Last)));
        rec.login_accepted();
        rec.region_entered("Last", now + secs(11));
        assert_eq!(rec.decide(now + secs(11)), Decision::Wait(None));
        // The next login asks for home again.
        let lost = now + secs(1000);
        let delay = rec.session_lost(lost);
        assert_eq!(step(&mut rec, lost + delay), start(Action::Login(Start::Home)));
    }

    #[test]
    fn first_refusal_at_the_last_location_is_fatal() {
        let now = Instant::now();
        let mut rec = Reconciler::new(goal(None, Start::Last, None));
        assert_eq!(step(&mut rec, now), start(Action::Login(Start::Last)));
        assert_eq!(rec.login_refused(&failure("key"), now), None);
        assert!(matches!(rec.decide(now), Decision::Exit { code: 1, .. }));
    }

    #[test]
    fn region_without_a_seat_teleports_and_then_rests() {
        let now = Instant::now();
        let mut rec = arrived(goal(Some(TARGET), Start::Target, None), "Home", now);
        assert_eq!(step(&mut rec, now), start(Action::Teleport));
        rec.region_entered(TARGET, now + secs(3));
        assert_eq!(rec.decide(now + secs(3)), Decision::Wait(None));
        assert_eq!(rec.standing(now + secs(4)), None);
        assert_eq!(rec.decide(now + secs(4000)), Decision::Wait(None));
    }

    #[test]
    fn seat_without_a_region_sits_anywhere_and_never_repositions() {
        let now = Instant::now();
        let mut rec = arrived(goal(None, Start::Last, Some(SeatGoal::Object)), "Anywhere", now);
        let mut at = now;
        for _ in 0..6 {
            assert_eq!(step(&mut rec, at), start(Action::Sit));
            at += rec.sit_failed(at);
        }
        // At the longest delay by now, and still sitting, not teleporting.
        assert_eq!(rec.sit_failed(at), secs(60));
        assert_eq!(step(&mut rec, at + secs(60)), start(Action::Sit));
        rec.seated(at + secs(61));
        assert_eq!(rec.decide(at + secs(4000)), Decision::Wait(None));
    }

    // ----------------------------------------------------- ground sit

    fn ground(region: Option<&str>) -> Goal {
        let start = if region.is_some() { Start::Target } else { Start::Last };
        goal(region, start, Some(SeatGoal::Ground))
    }

    #[test]
    fn ground_sit_is_sent_once_per_arrival() {
        let now = Instant::now();
        let mut rec = arrived(ground(None), "Anywhere", now);
        assert_eq!(step(&mut rec, now), start(Action::SitGround));
        // Over at once: nothing in flight, nothing more to do.
        assert_eq!(rec.decide(now), Decision::Wait(None));
        // The first report of the avatar after arriving says "standing".
        assert_eq!(rec.standing(now + secs(1)), None);
        assert_eq!(rec.decide(now + secs(4000)), Decision::Wait(None));

        rec.teleport_arrived();
        assert_eq!(step(&mut rec, now + secs(4001)), start(Action::SitGround));
        rec.region_entered("Elsewhere", now + secs(4002));
        assert_eq!(step(&mut rec, now + secs(4002)), start(Action::SitGround));
        assert_eq!(rec.decide(now + secs(4003)), Decision::Wait(None));
    }

    #[test]
    fn ground_sit_is_repeated_after_getting_up_from_an_object() {
        let now = Instant::now();
        let mut rec = arrived(ground(None), "Anywhere", now);
        assert_eq!(step(&mut rec, now), start(Action::SitGround));
        rec.seated_elsewhere(now + secs(10));
        assert_eq!(rec.decide(now + secs(10)), Decision::Wait(None));
        rec.standing(now + secs(20));
        assert_eq!(step(&mut rec, now + secs(20)), start(Action::SitGround));
        rec.standing(now + secs(21));
        assert_eq!(rec.decide(now + secs(21)), Decision::Wait(None));
    }

    #[test]
    fn ground_sit_waits_for_holding_mode_and_the_target_region() {
        let now = Instant::now();
        let mut rec = arrived(ground(Some(TARGET)), "Home", now);
        assert_eq!(step(&mut rec, now), start(Action::Teleport));
        rec.region_entered(TARGET, now + secs(3));
        assert_eq!(step(&mut rec, now + secs(3)), start(Action::SitGround));
        assert_eq!(rec.decide(now + secs(3)), Decision::Wait(None));

        rec.set_manual(true);
        rec.teleport_arrived();
        assert_eq!(rec.decide(now + secs(10)), Decision::Wait(None));
        rec.set_manual(false);
        assert_eq!(step(&mut rec, now + secs(20)), start(Action::SitGround));
        // Back in holding mode without having moved: sat down again once.
        rec.set_manual(true);
        rec.set_manual(false);
        assert_eq!(step(&mut rec, now + secs(30)), start(Action::SitGround));
        assert_eq!(rec.decide(now + secs(30)), Decision::Wait(None));
    }
}
