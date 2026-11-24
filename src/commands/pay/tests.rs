//! The payment commands against a fake grid.

use super::*;
use crate::commands::tests::{Call, FakeGrid};
use crate::payments::{Other, PaymentsConfig};
use crate::perms::Entry;
use crate::txlog::Kind;
use std::sync::mpsc::{self, Receiver};

/// 2026-12-01T12:00:00Z.
const T0: u64 = 1_796_126_400;

fn id(byte: u8) -> Uuid {
    Uuid::from_bytes([byte; 16])
}

fn jane() -> Uuid {
    id(0x71)
}

fn john() -> Uuid {
    id(0x72)
}

fn jar() -> Uuid {
    id(0xC0)
}

fn vendor() -> Uuid {
    id(0xC1)
}

fn names(legacy: &str, display: &str) -> Names {
    Names {
        legacy: legacy.into(),
        display: display.into(),
    }
}

fn entry(text: &str) -> Entry {
    Entry::parse(text).unwrap()
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

struct Rig {
    commands: Commands,
    grid: FakeGrid,
    now: Instant,
}

impl Rig {
    fn new(config: PaymentsConfig) -> Rig {
        let mut commands = Commands::default();
        commands.set_payments(Some(config.clone()), Ledger::from_config(&config));
        commands.set_clock(|| T0);
        // The tip jar and the vendor are described, and take money.
        let grid = FakeGrid {
            money: vec![(jar(), jar(), true), (vendor(), vendor(), true)],
            ..FakeGrid::default()
        };
        Rig {
            commands,
            grid,
            now: Instant::now(),
        }
    }

    fn run(&mut self, via: Interface, request: Request) -> Receiver<Response> {
        let (reply, answer) = mpsc::channel();
        self.commands
            .money_command(Origin::Operator, via, request, reply, Some(&mut self.grid), self.now);
        answer
    }

    fn pay(&mut self, amount: u32, payee: PayTo) -> Receiver<Response> {
        self.run(Interface::Ctl, Request::Pay { amount, payee })
    }

    fn pay_avatar(&mut self, amount: u32, who: &str) -> Receiver<Response> {
        self.pay(amount, PayTo::Avatar(who.to_string()))
    }

    fn calls(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.grid.calls)
    }

    fn named(&mut self, found: &[(Uuid, Names)], missing: &[Uuid]) {
        self.commands.names_resolved(found, missing, Some(&mut self.grid), self.now);
    }

    fn found(&mut self, query: Uuid, avatars: &[(Uuid, String)]) {
        self.commands.avatars_found(query, avatars, Some(&mut self.grid), self.now);
    }

    fn report(&mut self, balance: i32, success: bool, description: &str, info: Option<TransactionInfo>) {
        let own = self.grid.own;
        self.commands.money_report(balance, success, description, info.as_ref(), own);
    }

    /// The grid's confirmation of a payment of the bot.
    fn confirm(&mut self, to: Uuid, amount: i32, balance: i32) {
        let info = gift(self.grid.own, to, amount);
        self.report(balance, true, "You paid.", Some(info));
    }

    fn wait(&mut self, by: Duration) {
        self.now += by;
        self.commands.money_due(Some(&mut self.grid), self.now);
    }

    fn left(&self) -> String {
        self.commands.limit_text()
    }

    /// A payment to Jane by UUID, sent.
    fn paid_jane(&mut self, amount: u32) -> Receiver<Response> {
        let answer = self.pay_avatar(amount, &jane().to_string());
        self.named(&[(jane(), names("Jane Doe", "JD"))], &[]);
        assert_eq!(self.calls(), [Call::Pay(jane(), amount as i32, TRANSACTION_GIFT, String::new())]);
        answer
    }

    /// Answers what an object asked about offers and whose it is.
    fn describe(&mut self, root: Uuid, default: i32, buttons: &[i32], owner: Uuid, name: &str) {
        self.commands.pay_price(root, default, buttons, Some(&mut self.grid), self.now);
        self.commands
            .pay_owner(root, owner, Uuid::nil(), Some(name), Some(&mut self.grid), self.now);
    }
}

/// The query of the one avatar search sent since the last call.
fn find(rig: &mut Rig) -> Uuid {
    match &rig.calls()[..] {
        [Call::Find(query, _)] => *query,
        other => panic!("expected one search, got {other:?}"),
    }
}

fn gift(source: Uuid, dest: Uuid, amount: i32) -> TransactionInfo {
    TransactionInfo {
        kind: TRANSACTION_GIFT,
        source,
        source_group: false,
        dest,
        dest_group: false,
        amount,
        item: String::new(),
    }
}

fn ok(answer: &Receiver<Response>) -> String {
    match answer.try_recv() {
        Ok(Response::Ok(text)) => text.unwrap_or_default(),
        other => panic!("expected success, got {other:?}"),
    }
}

fn error(answer: &Receiver<Response>) -> String {
    match answer.try_recv() {
        Ok(Response::Error(reason)) => reason,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn waiting(answer: &Receiver<Response>) {
    assert_eq!(answer.try_recv(), Err(mpsc::TryRecvError::Empty));
}

// -------------------------------------------------------- paying an avatar

#[test]
fn by_name() {
    let mut rig = Rig::new(config());
    let answer = rig.pay_avatar(25, "jane.doe");
    let query = match &rig.calls()[..] {
        [Call::Find(query, name)] if name == "jane doe" => *query,
        other => panic!("{other:?}"),
    };
    waiting(&answer);
    rig.found(query, &[(id(0x7F), "Jane Doester".into()), (jane(), "Jane Doe".into())]);
    assert_eq!(rig.calls(), [Call::Pay(jane(), 25, TRANSACTION_GIFT, String::new())]);
    waiting(&answer);
    rig.confirm(jane(), 25, 975);
    let text = ok(&answer);
    assert!(
        text.contains("L$25") && text.contains("Jane Doe") && text.contains("L$975"),
        "{text}"
    );
    let records = rig.commands.take_transactions();
    assert_eq!(records.len(), 1);
    assert_eq!((records[0].kind, records[0].amount, records[0].counted), (Kind::Paid, 25, 25));
    assert_eq!(records[0].balance, Some(975));
    assert_eq!(rig.commands.balance(), Some(975));
}

#[test]
fn by_uuid_the_answer_names_her() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(25);
    rig.confirm(jane(), 25, 975);
    assert!(ok(&answer).contains(&format!("Jane Doe <{}>", jane())));
}

#[test]
fn unknown_name() {
    let mut rig = Rig::new(config());
    let answer = rig.pay_avatar(25, "Nobody Here");
    let query = find(&mut rig);
    rig.found(query, &[]);
    assert!(error(&answer).contains("Nobody Here"));
    assert_eq!(rig.calls(), []);
}

#[test]
fn the_avatar_itself_is_no_payee() {
    let mut rig = Rig::new(config());
    let own = rig.grid.own;
    assert!(error(&rig.pay_avatar(10, &own.to_string())).contains("cannot pay itself"));
    let answer = rig.pay_avatar(10, "Sit Bot");
    let query = find(&mut rig);
    rig.found(query, &[(own, "Sit Bot".into())]);
    assert!(error(&answer).contains("cannot pay itself"));
    assert_eq!(rig.calls(), []);
}

#[test]
fn the_uuid_of_an_object_is_no_avatar() {
    let mut rig = Rig::new(config());
    let answer = rig.pay_avatar(10, &jar().to_string());
    assert_eq!(rig.commands.take_name_lookups(), [jar()]);
    rig.named(&[], &[jar()]);
    let reason = error(&answer);
    assert!(
        reason.contains("knows no avatar") && reason.contains("pay AMOUNT object UUID"),
        "{reason}"
    );
    assert_eq!(rig.calls(), []);
}

#[test]
fn a_uuid_the_grid_says_nothing_about_within_five_seconds() {
    let mut rig = Rig::new(config());
    let answer = rig.pay_avatar(10, &john().to_string());
    rig.wait(NAME_LIMIT - Duration::from_millis(1));
    waiting(&answer);
    rig.wait(Duration::from_millis(1));
    assert!(error(&answer).contains("5 s"));
    assert_eq!(rig.calls(), []);
}

#[test]
fn paying_needs_the_avatar_in_a_region() {
    let mut rig = Rig::new(config());
    rig.grid.offline = true;
    assert_eq!(error(&rig.pay_avatar(10, "Jane Doe")), NOT_CONNECTED);
    assert_eq!(error(&rig.run(Interface::Ctl, Request::Prices(jar().to_string()))), NOT_CONNECTED);
    assert_eq!(error(&rig.run(Interface::Ctl, Request::Payees)), NOT_CONNECTED);
    let (reply, answer) = mpsc::channel();
    rig.commands
        .money_command::<FakeGrid>(Origin::Operator, Interface::Ctl, Request::Payees, reply, None, rig.now);
    assert_eq!(error(&answer), NOT_CONNECTED);
    assert_eq!(rig.calls(), []);
}

// ----------------------------------------------------------- the balance

#[test]
fn paying_more_than_the_avatar_has() {
    let mut rig = Rig::new(config());
    rig.report(5, true, "", None);
    assert_eq!(rig.commands.take_transactions(), []);
    let answer = rig.pay_avatar(25, &jane().to_string());
    rig.named(&[(jane(), names("Jane Doe", ""))], &[]);
    assert!(error(&answer).contains("L$5"));
    assert_eq!(rig.calls(), []);
    assert_eq!(rig.left(), "L$1000");
}

// --------------------------------------------------- one payment at a time

#[test]
fn one_payment_at_a_time() {
    let mut rig = Rig::new(config());
    let first = rig.paid_jane(10);
    let second = rig.pay_avatar(10, "John Roe");
    assert!(error(&second).contains("under way"));
    assert_eq!(rig.calls(), []);
    rig.confirm(jane(), 10, 990);
    ok(&first);
    // And then the next one goes.
    rig.commands.take_name_lookups();
    rig.pay_avatar(10, &john().to_string());
    assert_eq!(rig.commands.take_name_lookups(), [john()]);
}

// ------------------------------------------------------ outcome of a payment

#[test]
fn the_grid_refuses() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(400);
    assert_eq!(rig.left(), "L$600");
    rig.report(1000, false, "Insufficient funds", Some(gift(rig.grid.own, jane(), 400)));
    assert!(error(&answer).contains("Insufficient funds"));
    assert_eq!(rig.left(), "L$1000");
    let records = rig.commands.take_transactions();
    assert_eq!((records[0].kind, records[0].counted), (Kind::Failed, 0));
}

#[test]
fn the_grid_says_nothing() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(400);
    rig.wait(CONFIRM_LIMIT - Duration::from_millis(1));
    waiting(&answer);
    rig.wait(Duration::from_millis(1));
    let reason = error(&answer);
    assert!(reason.contains("was sent") && reason.contains("counts as paid"), "{reason}");
    assert_eq!(rig.left(), "L$600");
    let records = rig.commands.take_transactions();
    assert_eq!(
        (records[0].kind, records[0].amount, records[0].counted, records[0].balance),
        (Kind::Unconfirmed, 400, 400, None)
    );
    // A confirmation that comes later is a transaction of its own and
    // counts nothing more.
    rig.confirm(jane(), 400, 600);
    let late = rig.commands.take_transactions();
    assert_eq!((late[0].kind, late[0].counted), (Kind::Spent, 0));
    assert_eq!(rig.left(), "L$600");
}

#[test]
fn kicked_while_waiting() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(25);
    rig.commands.session_lost();
    let reason = error(&answer);
    assert!(reason.contains("was sent") && reason.contains("session was lost"), "{reason}");
    assert_eq!(rig.commands.take_transactions()[0].kind, Kind::Unconfirmed);
}

#[test]
fn stopped_while_a_payment_waits() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(25);
    rig.commands.money_stop();
    assert!(error(&answer).contains("stopped"));
    assert_eq!(rig.commands.take_transactions()[0].kind, Kind::Unconfirmed);
}

// ---------------------------------------------------------- the operator log

#[test]
fn payments_appear_in_the_operator_log() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(25);
    rig.confirm(jane(), 25, 975);
    ok(&answer);
    let told = rig.commands.take_told();
    let who = format!("Jane Doe <{}>", jane());
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(told[0].starts_with(&format!("ctl: pay L$25 to {who}: sent")), "{told:?}");
    assert_eq!(told[1], format!("ctl: pay L$25 to {who}: made, balance L$975"));
}

#[test]
fn a_refusal_for_the_limit_is_logged_with_the_interface() {
    let mut rig = Rig::new(PaymentsConfig {
        limit: Some(100),
        ..config()
    });
    let (reply, answer) = mpsc::channel();
    let request = Request::Pay {
        amount: 500,
        payee: PayTo::Avatar(jane().to_string()),
    };
    rig.commands
        .money_command(Origin::Operator, Interface::Web, request, reply, Some(&mut rig.grid), rig.now);
    rig.named(&[(jane(), names("Jane Doe", ""))], &[]);
    assert!(error(&answer).contains("L$100"));
    let told = rig.commands.take_told();
    assert_eq!(told.len(), 1);
    assert!(
        told[0].starts_with("web: pay L$500 to Jane Doe <") && told[0].contains("limit"),
        "{told:?}"
    );
}

// ----------------------------------------------------------------- limit

#[test]
fn within_and_over_the_limit() {
    let mut rig = Rig::new(config());
    let answer = rig.paid_jane(900);
    rig.confirm(jane(), 900, 100);
    ok(&answer);
    rig.report(5000, true, "", None);
    let answer = rig.pay_avatar(101, &jane().to_string());
    let reason = error(&answer);
    assert!(reason.contains("L$100"), "{reason}");
    assert_eq!(rig.calls(), []);
    let answer = rig.pay_avatar(100, &jane().to_string());
    assert_eq!(rig.calls(), [Call::Pay(jane(), 100, TRANSACTION_GIFT, String::new())]);
    waiting(&answer);
    assert_eq!(rig.left(), "L$0");
}

#[test]
fn the_renewal_is_named_when_refused() {
    let mut rig = Rig::new(PaymentsConfig {
        renew: Some(Duration::from_secs(7 * 86_400)),
        ..config()
    });
    let answer = rig.paid_jane(1000);
    rig.confirm(jane(), 1000, 0);
    ok(&answer);
    rig.report(5000, true, "", None);
    let reason = error(&rig.pay_avatar(1, &jane().to_string()));
    assert!(reason.contains("2026-12-08T12:00:00Z"), "{reason}");
}

#[test]
fn the_unlimited_list_with_the_limit_used_up() {
    let mut rig = Rig::new(PaymentsConfig {
        limit: Some(100),
        unlimited: vec![entry("Jane Doe")],
        ..config()
    });
    rig.report(10_000, true, "", None);
    let answer = rig.pay_avatar(100, &john().to_string());
    rig.named(&[(john(), names("John Roe", ""))], &[]);
    rig.calls();
    rig.confirm(john(), 100, 9900);
    ok(&answer);
    assert_eq!(rig.left(), "L$0");
    let answer = rig.paid_jane(5000);
    rig.confirm(jane(), 5000, 4900);
    ok(&answer);
    assert_eq!(rig.left(), "L$0");
    let records = rig.commands.take_transactions();
    assert_eq!(records.iter().map(|r| r.counted).collect::<Vec<_>>(), [100, 0]);
}

#[test]
fn a_limit_of_zero() {
    let mut rig = Rig::new(PaymentsConfig {
        limit: Some(0),
        unlimited: vec![entry("Jane Doe")],
        ..config()
    });
    let answer = rig.paid_jane(5);
    waiting(&answer);
    rig.confirm(jane(), 5, 995);
    ok(&answer);
    let answer = rig.pay_avatar(5, &john().to_string());
    rig.named(&[(john(), names("John Roe", ""))], &[]);
    assert!(error(&answer).contains("L$0"));
}

#[test]
fn unlimited_refuses_nothing_for_the_limit() {
    let mut rig = Rig::new(PaymentsConfig { limit: None, ..config() });
    let answer = rig.paid_jane(5000);
    waiting(&answer);
    assert_eq!(rig.left(), "unlimited");
}

// ---------------------------------------------------------------- lists

#[test]
fn blacklisted_avatar() {
    let mut rig = Rig::new(PaymentsConfig {
        blacklist: vec![entry("John Roe")],
        limit: None,
        ..config()
    });
    let answer = rig.pay_avatar(1, "John Roe");
    let query = find(&mut rig);
    rig.found(query, &[(john(), "John Roe".into())]);
    let reason = error(&answer);
    assert!(reason.contains("John Roe") && reason.contains("blacklist"), "{reason}");
    assert_eq!(rig.calls(), []);
}

#[test]
fn not_on_the_whitelist() {
    let mut rig = Rig::new(PaymentsConfig {
        whitelist: Some(vec![entry("Jane Doe")]),
        ..config()
    });
    let answer = rig.pay_avatar(1, &john().to_string());
    rig.named(&[(john(), names("John Roe", "Jane Doe"))], &[]);
    let reason = error(&answer);
    assert!(reason.contains("John Roe") && reason.contains("not on the whitelist"), "{reason}");
}

#[test]
fn the_grid_does_not_name_the_owner_a_list_needs() {
    let mut rig = Rig::new(PaymentsConfig {
        blacklist: vec![entry("John Roe")],
        ..config()
    });
    let answer = rig.pay(10, PayTo::Object(jar()));
    rig.calls();
    rig.describe(jar(), -2, &[-2; 4], john(), "Tip Jar");
    assert_eq!(rig.commands.take_name_lookups(), [john()]);
    rig.wait(NAME_LIMIT - Duration::from_millis(1));
    waiting(&answer);
    rig.wait(Duration::from_millis(1));
    assert!(error(&answer).contains("could not be checked"));
    assert_eq!(rig.calls(), []);
}

// -------------------------------------------------------- paying an object

#[test]
fn a_tip_jar() {
    let mut rig = Rig::new(config());
    let answer = rig.pay(10, PayTo::Object(jar()));
    assert_eq!(rig.calls(), [Call::PayPrice(jar()), Call::OwnerOf(jar())]);
    rig.describe(jar(), -2, &[-2; 4], jane(), "Tip Jar");
    rig.named(&[(jane(), names("Jane Doe", "JD"))], &[]);
    assert_eq!(rig.calls(), [Call::Pay(jar(), 10, TRANSACTION_PAY_OBJECT, "Tip Jar".into())]);
    // The grid names the owner as the payee.
    let info = TransactionInfo {
        kind: TRANSACTION_PAY_OBJECT,
        ..gift(rig.grid.own, jane(), 10)
    };
    rig.report(990, true, "You paid Tip Jar.", Some(info));
    let text = ok(&answer);
    assert!(text.contains("L$10") && text.contains("Tip Jar") && text.contains("Jane Doe") && text.contains("L$990"));
    let record = &rig.commands.take_transactions()[0];
    assert_eq!(
        (record.party, record.object_name.as_str(), record.object, record.code),
        (Some(jane()), "Tip Jar", Some(jar()), TRANSACTION_PAY_OBJECT)
    );
}

#[test]
fn an_object_out_of_range_or_unknown() {
    let mut rig = Rig::new(config());
    rig.grid.look = Ok(Look::GaveUp);
    let stray = id(0xEE);
    let reason = error(&rig.pay(10, PayTo::Object(stray)));
    assert!(reason.contains("not known"), "{reason}");
    // One the scan has yet to turn up, and does not.
    rig.grid.look = Ok(Look::Waiting);
    let answer = rig.pay(10, PayTo::Object(stray));
    assert_eq!(rig.calls(), [Call::LookFor(stray), Call::LookFor(stray)]);
    waiting(&answer);
    rig.commands.not_described(stray, Some(&mut rig.grid), rig.now);
    assert!(error(&answer).contains("not known"));
    assert_eq!(rig.calls(), []);
}

#[test]
fn an_object_described_after_a_look() {
    let mut rig = Rig::new(config());
    let late = id(0xED);
    let answer = rig.pay(10, PayTo::Object(late));
    rig.calls();
    rig.grid.money.push((late, late, true));
    rig.commands.described(late, Some(&mut rig.grid), rig.now);
    assert_eq!(rig.calls(), [Call::PayPrice(late), Call::OwnerOf(late)]);
    waiting(&answer);
}

#[test]
fn only_objects_that_take_money() {
    let mut rig = Rig::new(config());
    let bench = id(0xBE);
    rig.grid.money.push((bench, bench, false));
    assert!(error(&rig.pay(10, PayTo::Object(bench))).contains("does not take money"));
    assert!(error(&rig.run(Interface::Ctl, Request::Prices(bench.to_string()))).contains("does not take money"));
    // A child prim of a vendor whose root takes money.
    let child = id(0xC2);
    rig.grid.money.push((child, vendor(), true));
    let answer = rig.pay(100, PayTo::Object(child));
    assert_eq!(rig.calls(), [Call::PayPrice(vendor()), Call::OwnerOf(vendor())]);
    rig.describe(vendor(), -1, &[100, 250, -1, -1], jane(), "Hat Vendor");
    rig.named(&[(jane(), names("Jane Doe", ""))], &[]);
    assert_eq!(rig.calls(), [Call::Pay(child, 100, TRANSACTION_PAY_OBJECT, "Hat Vendor".into())]);
    waiting(&answer);
}

#[test]
fn the_simulator_does_not_say_what_the_object_offers() {
    let mut rig = Rig::new(config());
    let answer = rig.pay(10, PayTo::Object(jar()));
    rig.wait(OFFER_LIMIT - Duration::from_millis(1));
    waiting(&answer);
    rig.wait(Duration::from_millis(1));
    assert!(error(&answer).contains("10 s"));
    assert_eq!(rig.calls(), [Call::PayPrice(jar()), Call::OwnerOf(jar())]);
}

#[test]
fn only_an_amount_the_object_offers() {
    let mut rig = Rig::new(config());
    let answer = rig.pay(120, PayTo::Object(vendor()));
    rig.describe(vendor(), -1, &[100, 250, -1, -1], jane(), "Hat Vendor");
    rig.named(&[(jane(), names("Jane Doe", ""))], &[]);
    let reason = error(&answer);
    assert!(reason.contains("L$100") && reason.contains("L$250"), "{reason}");
    rig.calls();
    let answer = rig.pay(37, PayTo::Object(jar()));
    rig.describe(jar(), -2, &[-2; 4], jane(), "Tip Jar");
    assert_eq!(rig.calls()[2], Call::Pay(jar(), 37, TRANSACTION_PAY_OBJECT, "Tip Jar".into()));
    waiting(&answer);
}

// -------------------------------------------------------------- prices

fn prices(rig: &mut Rig, object: Uuid) -> Receiver<Response> {
    rig.run(Interface::Ctl, Request::Prices(object.to_string()))
}

#[test]
fn prices_of_a_vendor() {
    let mut rig = Rig::new(config());
    let answer = prices(&mut rig, vendor());
    rig.describe(vendor(), -1, &[100, 250, -1, -1], jane(), "Hat Vendor");
    waiting(&answer);
    rig.named(&[(jane(), names("Jane Doe", "JD"))], &[]);
    assert_eq!(
        ok(&answer),
        "object: Hat Vendor\nowner: Jane Doe\nprices: 100, 250\nother amounts: no"
    );
    assert!(!rig.calls().iter().any(|call| matches!(call, Call::Pay(..))));
}

#[test]
fn prices_with_a_suggested_amount_a_tab_and_a_group() {
    let mut rig = Rig::new(config());
    let answer = prices(&mut rig, jar());
    rig.commands.pay_price(jar(), 50, &[-1; 4], Some(&mut rig.grid), rig.now);
    let group = id(0x66);
    rig.commands
        .pay_owner(jar(), group, group, Some("Tip\tJar\n"), Some(&mut rig.grid), rig.now);
    assert_eq!(
        ok(&answer),
        format!("object: Tip\\tJar\\n\nowner: group {group}\nprices: none\nother amounts: yes, suggested 50")
    );
}

#[test]
fn prices_name_the_owner_by_uuid_when_the_grid_does_not_name_it() {
    let mut rig = Rig::new(config());
    let answer = prices(&mut rig, jar());
    rig.describe(jar(), -2, &[-2; 4], john(), "Tip Jar");
    rig.wait(NAME_LIMIT);
    assert!(ok(&answer).contains(&format!("owner: {john}", john = john())));
}

#[test]
fn prices_are_not_held_back_by_a_payment() {
    let mut rig = Rig::new(config());
    let paying = rig.paid_jane(10);
    let answer = prices(&mut rig, jar());
    rig.describe(jar(), -2, &[-2; 4], jane(), "Tip Jar");
    assert!(ok(&answer).contains("prices: 1, 5, 10, 20"));
    waiting(&paying);
}

#[test]
fn prices_take_a_uuid() {
    let mut rig = Rig::new(config());
    let answer = rig.run(Interface::Ctl, Request::Prices("Tip Jar".into()));
    assert!(error(&answer).contains("UUID"));
}

#[test]
fn offers_read_as_a_viewer_reads_them() {
    let offer = Offer::from_reply(-1, &[100, 250, -1, -1]);
    assert_eq!((offer.quick.clone(), offer.other), (vec![100, 250], Other::No));
}

// --------------------------------------------------------------- payees

#[test]
fn payees_by_name_and_uuid() {
    let mut rig = Rig::new(PaymentsConfig {
        whitelist: Some(vec![entry("Jane Doe"), Entry::Id(john()), Entry::Id(vendor())]),
        unlimited: vec![entry("jane.doe")],
        ..config()
    });
    let answer = rig.run(Interface::Ctl, Request::Payees);
    waiting(&answer);
    assert_eq!(rig.commands.take_name_lookups(), [john(), vendor()]);
    rig.named(&[(john(), names("John Roe", "Johnny"))], &[vendor()]);
    assert_eq!(
        ok(&answer),
        format!("?\tJane Doe\twhitelist,unlimited\n{}\tJohn Roe\twhitelist", john())
    );
}

#[test]
fn payees_without_lists() {
    let mut rig = Rig::new(config());
    assert_eq!(rig.run(Interface::Ctl, Request::Payees).try_recv(), Ok(Response::Ok(None)));
}

#[test]
fn payees_give_up_on_names_after_five_seconds() {
    let mut rig = Rig::new(PaymentsConfig {
        unlimited: vec![Entry::Id(john())],
        ..config()
    });
    let answer = rig.run(Interface::Ctl, Request::Payees);
    rig.wait(NAME_LIMIT);
    assert_eq!(answer.try_recv(), Ok(Response::Ok(None)));
}

#[test]
fn the_pages_own_payees_leave_no_log_line() {
    let mut rig = Rig::new(PaymentsConfig {
        whitelist: Some(vec![entry("Jane Doe")]),
        ..config()
    });
    let (reply, answer) = mpsc::channel();
    rig.commands
        .money_command(Origin::Page, Interface::Web, Request::Payees, reply, Some(&mut rig.grid), rig.now);
    assert_eq!(ok(&answer), "?\tJane Doe\twhitelist");
    assert_eq!(rig.commands.take_told(), Vec::<String>::new());
}

// ------------------------------------------------- the limit needs its record

#[test]
fn a_payment_that_counts_waits_for_the_record_of_the_last() {
    let mut rig = Rig::new(PaymentsConfig {
        unlimited: vec![entry("Jane Doe")],
        ..config()
    });
    rig.commands.set_unwritten(Some(PathBuf::from("/var/tx.tsv")));
    let answer = rig.pay_avatar(10, &john().to_string());
    rig.named(&[(john(), names("John Roe", ""))], &[]);
    assert!(error(&answer).contains("/var/tx.tsv"));
    assert_eq!(rig.calls(), []);
    // A payment that does not count goes.
    let answer = rig.paid_jane(10);
    waiting(&answer);
    rig.confirm(jane(), 10, 990);
    ok(&answer);
    // Once the row is written, so does one that counts.
    rig.commands.set_unwritten(None);
    rig.pay_avatar(10, &john().to_string());
    assert_eq!(rig.calls(), [Call::Pay(john(), 10, TRANSACTION_GIFT, String::new())]);
}

#[test]
fn received_payments_are_recorded() {
    let mut rig = Rig::new(config());
    rig.report(1250, true, "Jane Doe paid you L$50.", Some(gift(jane(), rig.grid.own, 50)));
    let records = rig.commands.take_transactions();
    assert_eq!((records[0].kind, records[0].party), (Kind::Received, Some(jane())));
    assert_eq!(rig.commands.balance(), Some(1250));
}
