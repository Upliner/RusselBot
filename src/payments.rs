//! Who may pay and how much: the interfaces of `payments.perms`, the lists
//! a payee is matched against, the limit and its period, and the amounts an
//! object offers. Everything here is pure; the clock is passed in, in
//! wall-clock seconds since the Unix epoch.

use crate::names::full_name;
use crate::perms::Entry;
use std::fmt;
use std::time::Duration;
use uuid::Uuid;

/// The largest amount the grid takes: a signed 32-bit number.
pub const MAX_AMOUNT: u32 = i32::MAX as u32;

/// The way a command reached the bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    /// The control socket and the startup script.
    Ctl,
    /// The control contact of the XMPP gateway.
    Xmpp,
    /// The page of the web interface and its console.
    Web,
}

impl Interface {
    pub const ALL: [Interface; 3] = [Interface::Ctl, Interface::Xmpp, Interface::Web];

    pub fn name(self) -> &'static str {
        match self {
            Interface::Ctl => "ctl",
            Interface::Xmpp => "xmpp",
            Interface::Web => "web",
        }
    }

    pub fn parse(text: &str) -> Option<Interface> {
        Interface::ALL.into_iter().find(|i| i.name() == text)
    }

    fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Interface {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The `[payments]` table.
#[derive(Debug, Clone, PartialEq)]
pub struct PaymentsConfig {
    /// What may be paid in one period; `None` is `"unlimited"`.
    pub limit: Option<u32>,
    /// The length of a period; without it the limit never renews.
    pub renew: Option<Duration>,
    /// Which interfaces may pay, by `Interface::index`.
    pub perms: [bool; 3],
    /// `None` when the key is left out, which differs from an empty array.
    pub whitelist: Option<Vec<Entry>>,
    pub blacklist: Vec<Entry>,
    pub unlimited: Vec<Entry>,
}

impl PaymentsConfig {
    pub fn may(&self, via: Interface) -> bool {
        self.perms[via.index()]
    }

    /// The interfaces that may pay, as the config names them.
    pub fn interfaces(&self) -> Vec<Interface> {
        Interface::ALL.into_iter().filter(|i| self.may(*i)).collect()
    }

    /// Whether a list has a name among its entries, so that the login name
    /// of the payee is needed to match it.
    pub fn needs_name(&self) -> bool {
        let named = |entries: &[Entry]| entries.iter().any(|e| matches!(e, Entry::Name { .. }));
        named(self.whitelist.as_deref().unwrap_or(&[])) || named(&self.blacklist) || named(&self.unlimited)
    }

    /// Whether the payment may be made, and whether it counts against the
    /// limit: blacklist first, then whitelist, then unlimited list. The
    /// limit itself is the ledger's.
    pub fn decide(&self, payee: &Payee) -> Decision {
        let on = |entries: &[Entry]| entries.iter().any(|entry| payee.matched_by(entry));
        if on(&self.blacklist) {
            return Decision::Refused(Refusal::Blacklisted);
        }
        let unlimited = on(&self.unlimited);
        if let Some(whitelist) = &self.whitelist {
            if !unlimited && !on(whitelist) {
                return Decision::Refused(Refusal::NotWhitelisted);
            }
        }
        match unlimited {
            true => Decision::Unlimited,
            false => Decision::Counted,
        }
    }
}

/// Whom a payment goes to, as far as the lists are concerned.
#[derive(Debug, Clone, PartialEq)]
pub struct Payee {
    /// The avatar paid, or the avatar that owns the object paid, with its
    /// login name when known; `None` for an object a group owns.
    pub avatar: Option<(Uuid, Option<String>)>,
    /// The object paid.
    pub object: Option<Uuid>,
}

impl Payee {
    fn matched_by(&self, entry: &Entry) -> bool {
        match entry {
            Entry::Id(wanted) => self.object == Some(*wanted) || self.avatar.as_ref().is_some_and(|(id, _)| id == wanted),
            // A login name, never a display name: only the grid's account
            // name is ever put here.
            Entry::Name { key, .. } => self
                .avatar
                .as_ref()
                .and_then(|(_, name)| name.as_deref())
                .and_then(full_name)
                .is_some_and(|name| name == *key),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Refusal {
    Blacklisted,
    NotWhitelisted,
}

impl Refusal {
    /// The reason, after the payee's name: "Jane Doe is on the blacklist".
    pub fn words(self) -> &'static str {
        match self {
            Refusal::Blacklisted => "is on the blacklist of `[payments]`",
            Refusal::NotWhitelisted => "is not on the whitelist of `[payments]`",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Refused(Refusal),
    /// May be paid and does not count against the limit.
    Unlimited,
    /// May be paid within the limit.
    Counted,
}

// ------------------------------------------------------------------ limit

/// What is counted against the limit in the current period.
#[derive(Debug, Clone, PartialEq)]
pub struct Ledger {
    limit: Option<u32>,
    renew: Option<u64>,
    /// When the current period started; `None` while none runs.
    period_start: Option<u64>,
    counted: u64,
}

impl Ledger {
    pub fn new(limit: Option<u32>, renew: Option<Duration>) -> Ledger {
        Ledger {
            limit,
            renew: renew.map(|d| d.as_secs().max(1)),
            period_start: None,
            counted: 0,
        }
    }

    pub fn from_config(config: &PaymentsConfig) -> Ledger {
        Ledger::new(config.limit, config.renew)
    }

    pub fn limit(&self) -> Option<u32> {
        self.limit
    }

    /// When the period that started at `start` ends, if it ends.
    fn end(&self, start: u64) -> Option<u64> {
        self.renew.map(|renew| start.saturating_add(renew))
    }

    /// Drops a period that is over by `now`. A clock set back only makes
    /// the period last longer: it ends at its start plus the renewal.
    fn expire(&mut self, now: u64) {
        if let Some(end) = self.period_start.and_then(|start| self.end(start)) {
            if now >= end {
                self.period_start = None;
                self.counted = 0;
            }
        }
    }

    /// What is counted in the period running at `now`.
    pub fn counted(&self, now: u64) -> u64 {
        let mut ledger = self.clone();
        ledger.expire(now);
        ledger.counted
    }

    /// What is left at `now`; `None` without a limit. Never less than zero,
    /// also when the limit was lowered.
    pub fn left(&self, now: u64) -> Option<u64> {
        let limit = u64::from(self.limit?);
        Some(limit.saturating_sub(self.counted(now)))
    }

    /// When the whole limit is back: the end of the period running at
    /// `now`, while part of the limit is counted in it and it renews.
    pub fn renews_at(&self, now: u64) -> Option<u64> {
        self.limit?;
        let mut ledger = self.clone();
        ledger.expire(now);
        match ledger.counted {
            0 => None,
            _ => ledger.period_start.and_then(|start| ledger.end(start)),
        }
    }

    /// Counts `amount` at `now`, starting a period when none runs, or says
    /// what is left when it is more than that.
    pub fn reserve(&mut self, amount: u32, now: u64) -> Result<(), u64> {
        self.expire(now);
        if let Some(left) = self.left(now) {
            if u64::from(amount) > left {
                return Err(left);
            }
        }
        if self.period_start.is_none() {
            self.period_start = Some(now);
        }
        self.counted += u64::from(amount);
        Ok(())
    }

    /// Takes back what a failed payment had counted. The period it started
    /// goes on: the renewal stays where it was announced.
    pub fn release(&mut self, amount: u32) {
        self.counted = self.counted.saturating_sub(u64::from(amount));
    }

    /// Restores the period from the rows of the transactions file that
    /// count, `(time, counted)` in file order.
    pub fn replay(&mut self, rows: impl IntoIterator<Item = (u64, u64)>, now: u64) {
        for (time, counted) in rows {
            if counted == 0 {
                continue;
            }
            let over = match self.period_start {
                None => true,
                Some(start) => self.end(start).is_some_and(|end| time >= end),
            };
            if over {
                self.period_start = Some(time);
                self.counted = 0;
            }
            self.counted += counted;
        }
        self.expire(now);
    }
}

/// `L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)`, `L$1000` or
/// `unlimited`: the value of the `limit` line of `status`.
pub fn limit_text(ledger: &Ledger, now: u64) -> String {
    let (Some(limit), Some(left)) = (ledger.limit(), ledger.left(now)) else {
        return "unlimited".to_string();
    };
    match ledger.renews_at(now) {
        Some(at) => format!("L${left} (renews to L${limit} on {})", crate::log::format_timestamp(at)),
        None => format!("L${left}"),
    }
}

// ----------------------------------------------------------------- amounts

/// Whether an object takes amounts other than its quick-pay ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Other {
    No,
    Any,
    Suggested(u32),
}

/// The amounts an object offers, as a viewer's Pay dialog shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub quick: Vec<u32>,
    pub other: Other,
}

/// A pay price the object hides.
const PAY_HIDE: i32 = -1;
/// A pay price the object leaves at the default.
const PAY_DEFAULT: i32 = -2;
/// The quick-pay amounts of a viewer, by position.
const DEFAULT_BUTTONS: [u32; 4] = [1, 5, 10, 20];

impl Offer {
    /// Reads a `PayPriceReply`: the default price and the four buttons.
    pub fn from_reply(default: i32, buttons: &[i32]) -> Offer {
        let quick = buttons
            .iter()
            .take(4)
            .enumerate()
            .filter_map(|(i, &price)| match price {
                PAY_DEFAULT => Some(DEFAULT_BUTTONS[i]),
                price if price > 0 => Some(price as u32),
                _ => None,
            })
            .collect();
        let other = match default {
            PAY_HIDE => Other::No,
            price if price > 0 => Other::Suggested(price as u32),
            _ => Other::Any,
        };
        Offer { quick, other }
    }

    pub fn allows(&self, amount: u32) -> bool {
        self.other != Other::No || self.quick.contains(&amount)
    }

    /// The value of the `prices` line: `100, 250` or `none`.
    pub fn prices_text(&self) -> String {
        match self.quick.is_empty() {
            true => "none".to_string(),
            false => self.quick.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
        }
    }

    /// The value of the `other amounts` line.
    pub fn other_text(&self) -> String {
        match self.other {
            Other::No => "no".to_string(),
            Other::Any => "yes".to_string(),
            Other::Suggested(n) => format!("yes, suggested {n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    /// 2026-12-01T12:00:00Z.
    const DEC1_NOON: u64 = 1_796_126_400;

    fn id(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn entry(text: &str) -> Entry {
        Entry::parse(text).unwrap()
    }

    fn config() -> PaymentsConfig {
        PaymentsConfig {
            limit: Some(1000),
            renew: None,
            perms: [true, false, false],
            whitelist: None,
            blacklist: Vec::new(),
            unlimited: Vec::new(),
        }
    }

    fn avatar(byte: u8, name: &str) -> Payee {
        Payee {
            avatar: Some((id(byte), Some(name.to_string()))),
            object: None,
        }
    }

    fn object_of(byte: u8, owner: Option<(u8, &str)>) -> Payee {
        Payee {
            avatar: owner.map(|(b, name)| (id(b), Some(name.to_string()))),
            object: Some(id(byte)),
        }
    }

    #[test]
    fn timestamps_in_the_tests_are_what_they_say() {
        assert_eq!(crate::log::format_timestamp(DEC1_NOON), "2026-12-01T12:00:00Z");
    }

    #[test]
    fn interfaces_are_named_and_switched_on_one_by_one() {
        let cfg = PaymentsConfig {
            perms: [true, false, true],
            ..config()
        };
        assert!(cfg.may(Interface::Ctl) && !cfg.may(Interface::Xmpp) && cfg.may(Interface::Web));
        assert_eq!(cfg.interfaces(), [Interface::Ctl, Interface::Web]);
        assert_eq!(Interface::parse("xmpp"), Some(Interface::Xmpp));
        assert_eq!(Interface::parse("socket"), None);
    }

    // ------------------------------------------------ whom a payment goes to

    #[test]
    fn an_avatar_on_a_list_stands_for_its_objects() {
        let cfg = PaymentsConfig {
            blacklist: vec![entry("Jane Doe")],
            ..config()
        };
        let jar = object_of(0xB0, Some((1, "Jane Doe")));
        assert_eq!(cfg.decide(&jar), Decision::Refused(Refusal::Blacklisted));
        let by_uuid = PaymentsConfig {
            blacklist: vec![Entry::Id(id(1))],
            ..config()
        };
        assert_eq!(by_uuid.decide(&jar), Decision::Refused(Refusal::Blacklisted));
    }

    #[test]
    fn an_object_on_a_list_stands_for_itself_only() {
        let cfg = PaymentsConfig {
            whitelist: Some(vec![Entry::Id(id(0xB0))]),
            ..config()
        };
        assert_eq!(cfg.decide(&object_of(0xB0, Some((1, "Jane Doe")))), Decision::Counted);
        assert_eq!(cfg.decide(&avatar(1, "Jane Doe")), Decision::Refused(Refusal::NotWhitelisted));
        assert_eq!(
            cfg.decide(&object_of(0xB1, Some((1, "Jane Doe")))),
            Decision::Refused(Refusal::NotWhitelisted)
        );
    }

    #[test]
    fn a_groups_object_is_matched_by_its_own_uuid_only() {
        let cfg = PaymentsConfig {
            blacklist: vec![entry("Jane Doe"), Entry::Id(id(1))],
            ..config()
        };
        assert_eq!(cfg.decide(&object_of(0xB0, None)), Decision::Counted);
        let listed = PaymentsConfig {
            blacklist: vec![Entry::Id(id(0xB0))],
            ..config()
        };
        assert_eq!(listed.decide(&object_of(0xB0, None)), Decision::Refused(Refusal::Blacklisted));
    }

    // ------------------------------------------- login names, not display names

    #[test]
    fn names_are_login_names_in_any_spelling() {
        let cfg = PaymentsConfig {
            whitelist: Some(vec![entry("jane.doe")]),
            ..config()
        };
        assert_eq!(cfg.decide(&avatar(1, "Jane Doe")), Decision::Counted);
        // John Roe whose display name is "Jane Doe": the payee carries the
        // login name only, so the entry does not match.
        assert_eq!(cfg.decide(&avatar(2, "John Roe")), Decision::Refused(Refusal::NotWhitelisted));
        let single = PaymentsConfig {
            whitelist: Some(vec![entry("jane")]),
            ..config()
        };
        assert_eq!(single.decide(&avatar(3, "Jane Resident")), Decision::Counted);
    }

    #[test]
    fn a_name_entry_does_not_match_a_payee_without_a_name() {
        let cfg = PaymentsConfig {
            whitelist: Some(vec![entry("Jane Doe")]),
            ..config()
        };
        let nameless = Payee {
            avatar: Some((id(1), None)),
            object: None,
        };
        assert_eq!(cfg.decide(&nameless), Decision::Refused(Refusal::NotWhitelisted));
    }

    // ------------------------------------------------------------ the lists

    #[test]
    fn the_blacklist_wins_over_everything() {
        let cfg = PaymentsConfig {
            limit: None,
            blacklist: vec![entry("John Roe")],
            whitelist: Some(vec![entry("John Roe")]),
            unlimited: vec![entry("John Roe")],
            ..config()
        };
        assert_eq!(cfg.decide(&avatar(2, "John Roe")), Decision::Refused(Refusal::Blacklisted));
    }

    #[test]
    fn the_whitelist_lets_through_who_is_on_it_or_on_the_unlimited_list() {
        let cfg = PaymentsConfig {
            whitelist: Some(vec![entry("Jane Doe")]),
            ..config()
        };
        assert_eq!(cfg.decide(&avatar(1, "Jane Doe")), Decision::Counted);
        assert_eq!(cfg.decide(&avatar(2, "John Roe")), Decision::Refused(Refusal::NotWhitelisted));

        let empty = PaymentsConfig {
            whitelist: Some(Vec::new()),
            unlimited: vec![entry("Jane Doe")],
            ..config()
        };
        assert_eq!(empty.decide(&avatar(1, "Jane Doe")), Decision::Unlimited);
        assert_eq!(empty.decide(&avatar(2, "John Roe")), Decision::Refused(Refusal::NotWhitelisted));
    }

    #[test]
    fn without_lists_anybody_can_be_paid() {
        let cfg = config();
        assert_eq!(cfg.decide(&avatar(2, "John Roe")), Decision::Counted);
        assert_eq!(cfg.decide(&object_of(0xB0, None)), Decision::Counted);
    }

    #[test]
    fn names_are_needed_only_when_a_list_has_one() {
        assert!(!config().needs_name());
        let uuids = PaymentsConfig {
            whitelist: Some(vec![Entry::Id(id(1))]),
            blacklist: vec![Entry::Id(id(2))],
            ..config()
        };
        assert!(!uuids.needs_name());
        for cfg in [
            PaymentsConfig {
                blacklist: vec![entry("John Roe")],
                ..config()
            },
            PaymentsConfig {
                whitelist: Some(vec![entry("John Roe")]),
                ..config()
            },
            PaymentsConfig {
                unlimited: vec![entry("John Roe")],
                ..config()
            },
        ] {
            assert!(cfg.needs_name());
        }
    }

    // ----------------------------------------------------------------- limit

    #[test]
    fn a_payment_within_what_is_left_is_counted() {
        let mut ledger = Ledger::new(Some(1000), None);
        ledger.reserve(900, DEC1_NOON).unwrap();
        ledger.reserve(100, DEC1_NOON + 60).unwrap();
        assert_eq!(ledger.left(DEC1_NOON + 60), Some(0));
    }

    #[test]
    fn a_payment_over_what_is_left_is_refused_with_what_is_left() {
        let mut ledger = Ledger::new(Some(1000), None);
        ledger.reserve(900, DEC1_NOON).unwrap();
        assert_eq!(ledger.reserve(101, DEC1_NOON), Err(100));
        assert_eq!(ledger.left(DEC1_NOON), Some(100));
    }

    #[test]
    fn a_limit_of_zero_refuses_whatever_counts() {
        let mut ledger = Ledger::new(Some(0), None);
        assert_eq!(ledger.reserve(1, DEC1_NOON), Err(0));
    }

    #[test]
    fn unlimited_refuses_nothing() {
        let mut ledger = Ledger::new(None, Some(Duration::from_secs(DAY)));
        ledger.reserve(MAX_AMOUNT, DEC1_NOON).unwrap();
        ledger.reserve(MAX_AMOUNT, DEC1_NOON).unwrap();
        assert_eq!(ledger.left(DEC1_NOON), None);
        assert_eq!(ledger.renews_at(DEC1_NOON), None);
        assert_eq!(limit_text(&ledger, DEC1_NOON), "unlimited");
    }

    // --------------------------------------------------------- the period

    #[test]
    fn the_limit_renews_when_the_period_is_over() {
        let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(7 * DAY)));
        ledger.reserve(1000, DEC1_NOON).unwrap();
        let renewal = DEC1_NOON + 7 * DAY;
        assert_eq!(ledger.reserve(1, renewal - 60), Err(0));
        assert_eq!(ledger.renews_at(renewal - 60), Some(renewal));
        assert_eq!(crate::log::format_timestamp(renewal), "2026-12-08T12:00:00Z");
        ledger.reserve(1, renewal).unwrap();
        // That payment started a new period.
        assert_eq!(ledger.renews_at(renewal), Some(renewal + 7 * DAY));
        assert_eq!(ledger.left(renewal), Some(999));
    }

    #[test]
    fn later_payments_do_not_move_the_renewal() {
        let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(7 * DAY)));
        ledger.reserve(100, DEC1_NOON).unwrap();
        ledger.reserve(100, DEC1_NOON + 4 * DAY).unwrap();
        assert_eq!(ledger.renews_at(DEC1_NOON + 4 * DAY), Some(DEC1_NOON + 7 * DAY));
    }

    #[test]
    fn without_renewal_the_period_never_ends() {
        let mut ledger = Ledger::new(Some(1000), None);
        ledger.reserve(1000, DEC1_NOON).unwrap();
        let year_later = DEC1_NOON + 365 * DAY;
        assert_eq!(ledger.reserve(1, year_later), Err(0));
        assert_eq!(ledger.renews_at(year_later), None);
        assert_eq!(limit_text(&ledger, year_later), "L$0");
    }

    #[test]
    fn a_clock_set_back_never_shortens_a_period() {
        let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(DAY)));
        ledger.reserve(1000, DEC1_NOON).unwrap();
        // The clock jumps back an hour: the period still ends a day after
        // its start, which is now further away.
        let back = DEC1_NOON - 3600;
        assert_eq!(ledger.reserve(1, back), Err(0));
        assert_eq!(ledger.renews_at(back), Some(DEC1_NOON + DAY));
        // And a row written before the jump stays in the period on replay.
        let mut replayed = Ledger::new(Some(1000), Some(Duration::from_secs(DAY)));
        replayed.replay([(DEC1_NOON, 600), (back, 400)], back);
        assert_eq!(replayed.left(back), Some(0));
        assert_eq!(replayed.renews_at(back), Some(DEC1_NOON + DAY));
    }

    #[test]
    fn limit_lines() {
        let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(7 * DAY)));
        assert_eq!(limit_text(&ledger, DEC1_NOON), "L$1000");
        ledger.reserve(250, DEC1_NOON).unwrap();
        assert_eq!(
            limit_text(&ledger, DEC1_NOON + 60),
            "L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)"
        );
        let mut plain = Ledger::new(Some(1000), None);
        plain.reserve(250, DEC1_NOON).unwrap();
        assert_eq!(limit_text(&plain, DEC1_NOON), "L$750");
    }

    // ---------------------------------------------- when a payment counts

    #[test]
    fn a_failed_payment_is_released() {
        let mut ledger = Ledger::new(Some(1000), None);
        ledger.reserve(400, DEC1_NOON).unwrap();
        assert_eq!(ledger.left(DEC1_NOON), Some(600));
        ledger.release(400);
        assert_eq!(ledger.left(DEC1_NOON), Some(1000));
    }

    // ------------------------------------------------ the limit outlasts the bot

    #[test]
    fn replay_restores_what_is_counted() {
        let mut ledger = Ledger::new(Some(1000), None);
        ledger.replay([(DEC1_NOON - 3600, 900)], DEC1_NOON);
        assert_eq!(ledger.left(DEC1_NOON), Some(100));
        assert_eq!(ledger.reserve(101, DEC1_NOON), Err(100));
    }

    #[test]
    fn a_lowered_limit_leaves_nothing_not_less() {
        let mut ledger = Ledger::new(Some(500), None);
        ledger.replay([(DEC1_NOON - 3600, 900)], DEC1_NOON);
        assert_eq!(ledger.left(DEC1_NOON), Some(0));
        assert_eq!(limit_text(&ledger, DEC1_NOON), "L$0");
    }

    #[test]
    fn a_period_over_while_the_bot_was_down_is_dropped() {
        let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(12 * 3600)));
        ledger.replay([(DEC1_NOON - DAY, 1000)], DEC1_NOON);
        assert_eq!(ledger.left(DEC1_NOON), Some(1000));
        assert_eq!(ledger.renews_at(DEC1_NOON), None);
    }

    #[test]
    fn replay_starts_a_period_at_the_first_row_after_the_last_one_ended() {
        let mut ledger = Ledger::new(Some(1000), Some(Duration::from_secs(7 * DAY)));
        let rows = [
            (DEC1_NOON, 600),
            (DEC1_NOON + DAY, 300),
            // The first period ended on 8 December; this row starts one.
            (DEC1_NOON + 9 * DAY, 200),
            (DEC1_NOON + 10 * DAY, 0),
            (DEC1_NOON + 11 * DAY, 50),
        ];
        let now = DEC1_NOON + 12 * DAY;
        ledger.replay(rows, now);
        assert_eq!(ledger.left(now), Some(750));
        assert_eq!(ledger.renews_at(now), Some(DEC1_NOON + 16 * DAY));
    }

    // --------------------------------------------------------------- offers

    #[test]
    fn a_vendor_with_two_prices() {
        let offer = Offer::from_reply(PAY_HIDE, &[100, 250, PAY_HIDE, PAY_HIDE]);
        assert_eq!(
            offer,
            Offer {
                quick: vec![100, 250],
                other: Other::No
            }
        );
        assert!(offer.allows(250) && !offer.allows(120));
        assert_eq!((offer.prices_text(), offer.other_text()), ("100, 250".into(), "no".into()));
    }

    #[test]
    fn an_object_that_sets_nothing_offers_the_viewers_defaults() {
        let offer = Offer::from_reply(PAY_DEFAULT, &[PAY_DEFAULT; 4]);
        assert_eq!(
            offer,
            Offer {
                quick: vec![1, 5, 10, 20],
                other: Other::Any
            }
        );
        assert!(offer.allows(37));
        assert_eq!((offer.prices_text(), offer.other_text()), ("1, 5, 10, 20".into(), "yes".into()));
    }

    #[test]
    fn default_buttons_keep_their_place() {
        let offer = Offer::from_reply(PAY_HIDE, &[PAY_HIDE, PAY_DEFAULT, 0, PAY_DEFAULT]);
        assert_eq!(offer.quick, [5, 20]);
    }

    #[test]
    fn a_suggested_amount_with_all_buttons_hidden() {
        let offer = Offer::from_reply(50, &[PAY_HIDE; 4]);
        assert_eq!(offer.other, Other::Suggested(50));
        assert_eq!(
            (offer.prices_text(), offer.other_text()),
            ("none".into(), "yes, suggested 50".into())
        );
        assert!(offer.allows(1));
    }
}
