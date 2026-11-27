//! The avatar's L$ transactions: the record of one, its row in the
//! transactions file and its line on standard error, what a report of the
//! grid makes of the payment in flight, and reading the file back.

use crate::escape::{escape, unescape};
use crate::log::{format_timestamp, parse_timestamp};
use crate::udp::messages::TransactionInfo;
use std::path::Path;
use uuid::Uuid;

pub const COLUMNS: [&str; 12] = [
    "time",
    "kind",
    "amount",
    "balance",
    "counted",
    "type",
    "party_uuid",
    "avatar_name",
    "display_name",
    "object_name",
    "object_uuid",
    "text",
];

/// How many rows of the file are kept for the web interface.
pub const KEPT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The bot's payment, confirmed by the grid.
    Paid,
    /// The bot's payment, neither confirmed nor refused in time.
    Unconfirmed,
    /// A transaction the grid reports as failed.
    Failed,
    /// A reported transaction with the avatar as payee.
    Received,
    /// A reported transaction with the avatar as payer that is no payment
    /// of the bot.
    Spent,
}

impl Kind {
    const ALL: [Kind; 5] = [Kind::Paid, Kind::Unconfirmed, Kind::Failed, Kind::Received, Kind::Spent];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Paid => "paid",
            Kind::Unconfirmed => "unconfirmed",
            Kind::Failed => "failed",
            Kind::Received => "received",
            Kind::Spent => "spent",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == text)
    }
}

/// One transaction, with its names settled or given up on.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    /// When the bot learned the outcome, in seconds since the Unix epoch.
    pub time: u64,
    pub kind: Kind,
    pub amount: u64,
    /// The balance the grid reported with it.
    pub balance: Option<i64>,
    /// How much of the amount counts against the limit.
    pub counted: u64,
    /// The grid's number for the kind of transaction.
    pub code: i32,
    /// The avatar or group that paid or was paid: for a payment to an
    /// object, its owner.
    pub party: Option<Uuid>,
    /// The party is a group: it has no names to look up. Not written.
    pub party_group: bool,
    pub avatar_name: String,
    pub display_name: String,
    /// For a payment of the bot to an object.
    pub object_name: String,
    pub object: Option<Uuid>,
    /// The description the grid gave.
    pub text: String,
}

fn line(fields: &[String]) -> String {
    fields.iter().map(|field| escape(field)).collect::<Vec<_>>().join("\t") + "\n"
}

/// The header row of the transactions file.
pub fn header() -> String {
    line(&COLUMNS.map(String::from))
}

fn uuid_text(id: Option<Uuid>) -> String {
    id.map(|id| id.to_string()).unwrap_or_default()
}

impl Transaction {
    /// The record as one row of the transactions file, with its line end.
    pub fn row(&self) -> String {
        line(&self.fields())
    }

    /// The twelve fields of the row, before escaping: what the web
    /// interface's `money` event carries.
    pub fn fields(&self) -> [String; 12] {
        [
            format_timestamp(self.time),
            self.kind.name().to_string(),
            self.amount.to_string(),
            self.balance.map(|b| b.to_string()).unwrap_or_default(),
            self.counted.to_string(),
            self.code.to_string(),
            uuid_text(self.party),
            self.avatar_name.clone(),
            self.display_name.clone(),
            self.object_name.clone(),
            uuid_text(self.object),
            self.text.clone(),
        ]
    }

    /// The other party as the log line names it.
    fn party_words(&self) -> String {
        let mut who = self.avatar_name.clone();
        if !self.display_name.is_empty() {
            who = format!("{who} ({})", self.display_name).trim_start().to_string();
        }
        if let Some(id) = self.party {
            let id = match self.party_group {
                true => format!("group <{id}>"),
                false => format!("<{id}>"),
            };
            who = format!("{who} {id}").trim_start().to_string();
        }
        who
    }

    /// The record as a standard-error log entry (without its timestamp).
    pub fn log_line(&self) -> String {
        let mut line = format!("transaction [{}] L${}", self.kind.name(), self.amount);
        let party = self.party_words();
        let to = match self.kind {
            Kind::Received => "from",
            _ => "to",
        };
        if !self.object_name.is_empty() || self.object.is_some() {
            let object = format!(
                "{} {}",
                self.object_name,
                self.object.map(|id| format!("<{id}>")).unwrap_or_default()
            );
            line.push_str(&format!(" {to} {}", object.trim()));
            if !party.is_empty() {
                line.push_str(&format!(", owner {party}"));
            }
        } else if !party.is_empty() {
            line.push_str(&format!(" {to} {party}"));
        }
        if let Some(balance) = self.balance {
            line.push_str(&format!(", balance L${balance}"));
        }
        if self.counted > 0 {
            line.push_str(&format!(", L${} counted", self.counted));
        }
        line.push_str(&format!(": {}", self.text));
        line
    }
}

/// Reads one row of the file, without its line end; `None` for one that
/// does not have twelve fields or whose time, amount or counted cannot be
/// read.
pub fn parse_row(row: &str) -> Option<Transaction> {
    let fields: Vec<&str> = row.split('\t').collect();
    let [time, kind, amount, balance, counted, code, party, avatar_name, display_name, object_name, object, text] = fields[..] else {
        return None;
    };
    let id = |text: &str| match text {
        "" => Some(None),
        text => Uuid::parse_str(text).ok().map(Some),
    };
    Some(Transaction {
        time: parse_timestamp(time)?,
        kind: Kind::parse(kind)?,
        amount: amount.parse().ok()?,
        balance: match balance {
            "" => None,
            balance => Some(balance.parse().ok()?),
        },
        counted: counted.parse().ok()?,
        code: code.parse().unwrap_or(0),
        party: id(party)?,
        party_group: false,
        avatar_name: unescape(avatar_name),
        display_name: unescape(display_name),
        object_name: unescape(object_name),
        object: id(object)?,
        text: unescape(text),
    })
}

/// The rows of a transactions file, and how many could not be read.
#[derive(Debug, Default, PartialEq)]
pub struct ReadBack {
    pub rows: Vec<Transaction>,
    pub skipped: usize,
}

impl ReadBack {
    /// `(time, counted)` of the rows that count against the limit, in file
    /// order, for `Ledger::replay`.
    pub fn counted(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.rows
            .iter()
            .filter(|tx| matches!(tx.kind, Kind::Paid | Kind::Unconfirmed))
            .map(|tx| (tx.time, tx.counted))
    }

    /// The newest `KEPT` rows, oldest first.
    pub fn newest(&self) -> &[Transaction] {
        &self.rows[self.rows.len().saturating_sub(KEPT)..]
    }
}

/// Reads the transactions file at startup. A file that does not exist, or
/// is empty, has no rows; one that cannot be read, or whose first line is
/// not the header row, is an error.
pub fn read(path: &Path) -> Result<ReadBack, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ReadBack::default()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    if bytes.is_empty() {
        return Ok(ReadBack::default());
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.split('\n');
    if lines.next().map(|first| format!("{first}\n")) != Some(header()) {
        return Err(format!(
            "{} is not a transactions file: its first line is not the header row `{}`",
            path.display(),
            header().trim_end().replace('\t', " ")
        ));
    }
    let mut back = ReadBack::default();
    for row in lines {
        let row = row.strip_suffix('\r').unwrap_or(row);
        if row.is_empty() {
            continue;
        }
        match parse_row(row) {
            Some(tx) => back.rows.push(tx),
            None => back.skipped += 1,
        }
    }
    Ok(back)
}

// ------------------------------------------------- what the grid reports

/// A payment of the bot that waits for the grid's answer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InFlight {
    pub amount: u32,
    /// What counts against the limit: the amount, or 0.
    pub counted: u32,
    /// The transaction type it was sent with.
    pub code: i32,
    /// The avatar paid, or the owner of the object paid.
    pub party: Option<Uuid>,
    pub party_group: bool,
    pub avatar_name: String,
    pub object_name: String,
    pub object: Option<Uuid>,
    /// The description sent with it; empty for none.
    pub description: String,
    /// The text of its record when neither the grid nor `description`
    /// gives one: `Fee to join Builders` for a group's fee.
    pub fallback: String,
}

impl InFlight {
    /// The record of the payment when the grid has said nothing of it.
    pub fn unconfirmed(&self, time: u64) -> Transaction {
        Transaction {
            time,
            kind: Kind::Unconfirmed,
            amount: u64::from(self.amount),
            balance: None,
            counted: u64::from(self.counted),
            code: self.code,
            party: self.party,
            party_group: self.party_group,
            avatar_name: self.avatar_name.clone(),
            display_name: String::new(),
            object_name: self.object_name.clone(),
            object: self.object,
            text: match self.description.is_empty() {
                true => self.fallback.clone(),
                false => self.description.clone(),
            },
        }
    }
}

/// The simulator's item description of a gift that has none of the payer's.
const NO_DESCRIPTION: &str = "Payment";

/// `text` with the payer's description, as a viewer shows a gift with one:
/// `You paid Jane Doe L$3: test payment`.
fn described(text: &str, description: &str) -> String {
    let text = text.trim_end();
    match text.is_empty() {
        true => description.to_string(),
        false => format!("{}: {description}", text.strip_suffix('.').unwrap_or(text)),
    }
}

/// What a report of the grid about the avatar's money is.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A balance and no transaction.
    Balance,
    /// The answer to the payment in flight: `paid` or `failed`.
    Ours(Transaction),
    /// Another transaction: `received`, `spent` or `failed`.
    Other(Transaction),
}

/// Reads a MoneyBalanceReply: `own` is the bot's avatar, `in_flight` the
/// payment that waits for the grid. A report is the answer to that
/// payment when it has the avatar as payer and the payment's amount; the
/// payee is not compared, since for an object the grid names its owner.
pub fn from_report(
    balance: i32,
    success: bool,
    description: &str,
    info: Option<&TransactionInfo>,
    own: Uuid,
    in_flight: Option<&InFlight>,
    time: u64,
) -> Outcome {
    let Some(info) = info.filter(|info| info.amount > 0 && !(info.source.is_nil() && info.dest.is_nil())) else {
        return Outcome::Balance;
    };
    // A gift's description of the payer's comes as the item description;
    // the grid's own text never holds it.
    let gift = info.kind == crate::udp::messages::TRANSACTION_GIFT;
    let payers = Some(info.item.trim()).filter(|item| gift && !item.is_empty() && *item != NO_DESCRIPTION);
    let grid_text = match (description.is_empty(), payers) {
        (true, Some(_)) => String::new(),
        (true, None) => info.item.clone(),
        (false, _) => description.to_string(),
    };
    let text = match payers {
        Some(payers) => described(&grid_text, payers),
        None => grid_text.clone(),
    };
    let amount = u64::from(info.amount.unsigned_abs());
    if let Some(ours) = in_flight.filter(|ours| info.source == own && u64::from(ours.amount) == amount) {
        // The description sent stands in for one the report does not carry.
        let text = match (payers, ours.description.is_empty()) {
            (None, false) => described(&grid_text, &ours.description),
            _ if text.trim().is_empty() => ours.fallback.clone(),
            _ => text,
        };
        return Outcome::Ours(Transaction {
            kind: if success { Kind::Paid } else { Kind::Failed },
            balance: Some(i64::from(balance)),
            counted: if success { u64::from(ours.counted) } else { 0 },
            code: info.kind,
            text,
            ..ours.unconfirmed(time)
        });
    }
    let received = info.dest == own && info.source != own;
    let (party, group) = match received {
        true => (info.source, info.source_group),
        false => (info.dest, info.dest_group),
    };
    let kind = match (success, received) {
        (false, _) => Kind::Failed,
        (true, true) => Kind::Received,
        (true, false) => Kind::Spent,
    };
    Outcome::Other(Transaction {
        time,
        kind,
        amount,
        balance: Some(i64::from(balance)),
        counted: 0,
        code: info.kind,
        party: (!party.is_nil()).then_some(party),
        party_group: group,
        avatar_name: String::new(),
        display_name: String::new(),
        object_name: String::new(),
        object: None,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::messages::{TRANSACTION_GIFT, TRANSACTION_PAY_OBJECT};

    /// 2026-12-01T12:00:00Z.
    const T0: u64 = 1_796_126_400;

    fn id(byte: u8) -> Uuid {
        Uuid::from_bytes([byte; 16])
    }

    fn own() -> Uuid {
        id(0xA1)
    }

    fn jane() -> Uuid {
        id(0x71)
    }

    fn paid_jane() -> Transaction {
        Transaction {
            time: T0,
            kind: Kind::Paid,
            amount: 25,
            balance: Some(975),
            counted: 25,
            code: TRANSACTION_GIFT,
            party: Some(jane()),
            party_group: false,
            avatar_name: "Jane Doe".into(),
            display_name: "JD".into(),
            object_name: String::new(),
            object: None,
            text: "You paid Jane Doe L$25.".into(),
        }
    }

    fn info(source: Uuid, dest: Uuid, amount: i32) -> TransactionInfo {
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

    fn to_jane(amount: u32, counted: u32) -> InFlight {
        InFlight {
            amount,
            counted,
            code: TRANSACTION_GIFT,
            party: Some(jane()),
            party_group: false,
            avatar_name: "Jane Doe".into(),
            object_name: String::new(),
            object: None,
            description: String::new(),
            fallback: String::new(),
        }
    }

    // -------------------------------------------------------------- rows

    #[test]
    fn row_of_a_payment() {
        assert_eq!(
            paid_jane().row(),
            format!(
                "2026-12-01T12:00:00Z\tpaid\t25\t975\t25\t5001\t{}\tJane Doe\tJD\t\t\tYou paid Jane Doe L$25.\n",
                jane()
            )
        );
        assert_eq!(
            header(),
            "time\tkind\tamount\tbalance\tcounted\ttype\tparty_uuid\tavatar_name\tdisplay_name\tobject_name\tobject_uuid\ttext\n"
        );
    }

    #[test]
    fn the_readme_shows_the_header_row() {
        let readme = include_str!("../README.md");
        assert!(readme.contains(&format!("```\n{}```", header())), "the README's header row differs");
    }

    #[test]
    fn an_unconfirmed_payment_has_no_balance() {
        let tx = to_jane(25, 25).unconfirmed(T0);
        assert_eq!((tx.kind, tx.amount, tx.counted, tx.balance), (Kind::Unconfirmed, 25, 25, None));
        assert_eq!(tx.row().split('\t').nth(3), Some(""));
    }

    #[test]
    fn rows_read_back_whatever_their_text() {
        let tricky = Transaction {
            avatar_name: "Tab\tby".into(),
            display_name: "Back\\slash".into(),
            object_name: "Line\nend\r".into(),
            object: Some(id(0xB0)),
            text: "a\tb\nc\\n".into(),
            balance: None,
            ..paid_jane()
        };
        let row = tricky.row();
        assert_eq!(row.matches('\t').count(), 11);
        assert_eq!(row.matches('\n').count(), 1);
        assert_eq!(parse_row(row.trim_end_matches('\n')), Some(tricky.clone()));
        assert_eq!(tricky.fields().len(), 12);
        let negative = Transaction {
            balance: Some(-3),
            ..paid_jane()
        };
        assert_eq!(parse_row(negative.row().trim_end()), Some(negative));
    }

    #[test]
    fn rows_that_cannot_be_read() {
        let row = paid_jane().row();
        let row = row.trim_end_matches('\n');
        let cut = &row[..row.len() - 30];
        assert_eq!(parse_row(cut), None);
        for (field, bad) in [(0, "yesterday"), (2, "ten"), (4, "-1"), (1, "stolen")] {
            let mut fields: Vec<&str> = row.split('\t').collect();
            fields[field] = bad;
            assert_eq!(parse_row(&fields.join("\t")), None, "{bad}");
        }
    }

    // ---------------------------------------------------------- log lines

    #[test]
    fn log_lines_name_what_is_known() {
        assert_eq!(
            paid_jane().log_line(),
            format!(
                "transaction [paid] L$25 to Jane Doe (JD) <{}>, balance L$975, L$25 counted: You paid Jane Doe L$25.",
                jane()
            )
        );
        let received = Transaction {
            kind: Kind::Received,
            amount: 50,
            balance: Some(1250),
            counted: 0,
            text: String::new(),
            display_name: String::new(),
            ..paid_jane()
        };
        let line = received.log_line();
        assert!(
            line.contains("received") && line.contains("L$50") && line.contains("Jane Doe"),
            "{line}"
        );
        assert_eq!(
            line,
            format!("transaction [received] L$50 from Jane Doe <{}>, balance L$1250: ", jane())
        );
        let object = Transaction {
            object_name: "Hat Vendor".into(),
            object: Some(id(0xB0)),
            ..paid_jane()
        };
        assert!(object.log_line().starts_with(&format!(
            "transaction [paid] L$25 to Hat Vendor <{}>, owner Jane Doe (JD) <{}>",
            id(0xB0),
            jane()
        )));
        let nameless = Transaction {
            avatar_name: String::new(),
            display_name: String::new(),
            party: None,
            balance: None,
            ..paid_jane()
        };
        assert_eq!(
            nameless.log_line(),
            "transaction [paid] L$25, L$25 counted: You paid Jane Doe L$25."
        );
    }

    // ------------------------------------------- what the grid reports

    #[test]
    fn a_balance_alone_is_no_transaction() {
        assert_eq!(from_report(1200, true, "", None, own(), None, T0), Outcome::Balance);
        let empty = TransactionInfo {
            kind: 0,
            ..info(Uuid::nil(), Uuid::nil(), 0)
        };
        assert_eq!(from_report(1200, true, "", Some(&empty), own(), None, T0), Outcome::Balance);
        // A balance while a payment waits is no answer to it either.
        assert_eq!(
            from_report(1200, true, "", None, own(), Some(&to_jane(25, 25)), T0),
            Outcome::Balance
        );
    }

    #[test]
    fn a_payment_received() {
        let report = info(jane(), own(), 50);
        let Outcome::Other(tx) = from_report(1250, true, "Jane Doe paid you L$50.", Some(&report), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!((tx.kind, tx.amount, tx.balance, tx.counted), (Kind::Received, 50, Some(1250), 0));
        assert_eq!((tx.party, tx.object), (Some(jane()), None));
        assert_eq!(tx.text, "Jane Doe paid you L$50.");
    }

    #[test]
    fn a_payment_of_the_bot_is_merged_with_its_confirmation() {
        let report = info(own(), jane(), 25);
        let ours = to_jane(25, 25);
        let outcome = from_report(975, true, "You paid Jane Doe L$25.", Some(&report), own(), Some(&ours), T0);
        assert_eq!(
            outcome,
            Outcome::Ours(Transaction {
                display_name: String::new(),
                ..paid_jane()
            })
        );
        // A refusal releases the count.
        let Outcome::Ours(failed) = from_report(5, false, "Insufficient funds", Some(&report), own(), Some(&ours), T0) else {
            panic!("not ours")
        };
        assert_eq!(
            (failed.kind, failed.counted, failed.text.as_str()),
            (Kind::Failed, 0, "Insufficient funds")
        );
    }

    #[test]
    fn a_payment_with_a_description_reads_as_a_viewer_shows_it() {
        // The grid's text lacks it; the item description holds it.
        let report = TransactionInfo {
            item: "test payment".into(),
            ..info(own(), jane(), 3)
        };
        let ours = InFlight {
            description: "test payment".into(),
            ..to_jane(3, 3)
        };
        let Outcome::Ours(tx) = from_report(17, true, "You paid Jane Doe L$3.", Some(&report), own(), Some(&ours), T0) else {
            panic!("not ours")
        };
        assert_eq!(tx.text, "You paid Jane Doe L$3: test payment");
        // A report without it: the description sent stands in.
        let bare = TransactionInfo {
            item: String::new(),
            ..report.clone()
        };
        let Outcome::Ours(tx) = from_report(17, true, "You paid Jane Doe L$3.", Some(&bare), own(), Some(&ours), T0) else {
            panic!("not ours")
        };
        assert_eq!(tx.text, "You paid Jane Doe L$3: test payment");
        // And so does an unconfirmed one.
        assert_eq!(ours.unconfirmed(T0).text, "test payment");
    }

    #[test]
    fn a_payment_received_with_a_description() {
        let report = TransactionInfo {
            item: "for the hat".into(),
            ..info(jane(), own(), 5)
        };
        let Outcome::Other(tx) = from_report(25, true, "Jane Doe paid you L$5.", Some(&report), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!(tx.text, "Jane Doe paid you L$5: for the hat");
    }

    #[test]
    fn a_gift_without_a_description_keeps_the_grids_text() {
        // As the rig's module reports a gift: the item is `Payment`.
        let report = TransactionInfo {
            item: "Payment".into(),
            ..info(jane(), own(), 30)
        };
        let Outcome::Other(tx) = from_report(1030, true, "Rig Alt paid you L$30.", Some(&report), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!(tx.text, "Rig Alt paid you L$30.");
        // An object's item description is its name, not a description.
        let object = TransactionInfo {
            kind: TRANSACTION_PAY_OBJECT,
            item: "Tip Jar".into(),
            ..info(own(), jane(), 10)
        };
        let Outcome::Other(tx) = from_report(990, true, "You paid Tip Jar L$10.", Some(&object), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!(tx.text, "You paid Tip Jar L$10.");
    }

    #[test]
    fn a_payment_to_an_object_keeps_the_object_and_its_owner() {
        let ours = InFlight {
            code: TRANSACTION_PAY_OBJECT,
            object_name: "Hat Vendor".into(),
            object: Some(id(0xB0)),
            ..to_jane(100, 100)
        };
        // The grid names the owner as the destination.
        let report = TransactionInfo {
            kind: TRANSACTION_PAY_OBJECT,
            ..info(own(), jane(), 100)
        };
        let Outcome::Ours(tx) = from_report(900, true, "", Some(&report), own(), Some(&ours), T0) else {
            panic!("not ours")
        };
        assert_eq!(
            (tx.party, tx.avatar_name.as_str(), tx.object_name.as_str(), tx.object),
            (Some(jane()), "Jane Doe", "Hat Vendor", Some(id(0xB0)))
        );
        assert_eq!(tx.code, TRANSACTION_PAY_OBJECT);
    }

    #[test]
    fn other_outgoing_transactions_are_spent() {
        // A fee for an upload a viewer made, while a payment of another
        // amount waits.
        let fee = TransactionInfo {
            kind: 1101,
            ..info(own(), Uuid::nil(), 10)
        };
        let Outcome::Other(tx) = from_report(990, true, "Upload charge", Some(&fee), own(), Some(&to_jane(25, 25)), T0) else {
            panic!("not a transaction")
        };
        assert_eq!((tx.kind, tx.party, tx.counted), (Kind::Spent, None, 0));
        // A confirmation that comes after the payment was given up on.
        let late = info(own(), jane(), 25);
        let Outcome::Other(tx) = from_report(975, true, "", Some(&late), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!((tx.kind, tx.counted, tx.party), (Kind::Spent, 0, Some(jane())));
    }

    #[test]
    fn a_failed_report_that_is_not_ours() {
        let report = info(own(), jane(), 40);
        let Outcome::Other(tx) = from_report(5, false, "no", Some(&report), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!(tx.kind, Kind::Failed);
    }

    #[test]
    fn a_payout_from_an_object_has_its_owner_as_the_party() {
        // The grid names the owner as the source of an object's payout.
        let report = TransactionInfo {
            kind: 5009,
            ..info(jane(), own(), 10)
        };
        let Outcome::Other(tx) = from_report(1010, true, "", Some(&report), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert_eq!(
            (tx.kind, tx.party, tx.object_name.as_str(), tx.object),
            (Kind::Received, Some(jane()), "", None)
        );
    }

    #[test]
    fn a_group_party_is_marked() {
        let report = TransactionInfo {
            source_group: true,
            ..info(id(0x66), own(), 10)
        };
        let Outcome::Other(tx) = from_report(1010, true, "", Some(&report), own(), None, T0) else {
            panic!("not a transaction")
        };
        assert!(tx.party_group);
        assert!(tx.log_line().contains("group <"));
    }

    // -------------------------------------------------------- reading back

    /// A fresh directory under the system's temporary directory.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rslbot-txlog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reading_a_file_back() {
        let dir = temp_dir("read");
        let path = dir.join("tx.tsv");
        assert_eq!(read(&path), Ok(ReadBack::default()), "a missing file has no rows");
        let received = Transaction {
            kind: Kind::Received,
            counted: 0,
            ..paid_jane()
        };
        let text = header() + &paid_jane().row() + &received.row() + "2026-12-01T12:00:00Z\tpaid\t2" + "\n";
        std::fs::write(&path, text).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.rows, [paid_jane(), received]);
        assert_eq!(back.skipped, 1);
        assert_eq!(back.counted().collect::<Vec<_>>(), [(T0, 25)]);
        std::fs::write(&path, "").unwrap();
        assert_eq!(read(&path), Ok(ReadBack::default()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn group_fees_read_back_count_as_payments_do() {
        let dir = temp_dir("fees");
        let path = dir.join("tx.tsv");
        let builders = id(0xB1);
        let fee = |kind, counted| Transaction {
            kind,
            amount: 250,
            balance: None,
            counted,
            code: 1004,
            party: Some(builders),
            party_group: true,
            avatar_name: String::new(),
            display_name: String::new(),
            object_name: String::new(),
            object: None,
            text: "Fee to join Builders".into(),
            ..paid_jane()
        };
        let rows = [fee(Kind::Paid, 250), fee(Kind::Unconfirmed, 250), fee(Kind::Failed, 0)];
        let text = header() + &rows.iter().map(Transaction::row).collect::<String>();
        std::fs::write(&path, text).unwrap();
        let back = read(&path).unwrap();
        let kinds: Vec<(Kind, u64, i32)> = back.rows.iter().map(|tx| (tx.kind, tx.counted, tx.code)).collect();
        assert_eq!(
            kinds,
            [(Kind::Paid, 250, 1004), (Kind::Unconfirmed, 250, 1004), (Kind::Failed, 0, 1004)]
        );
        // A paid and an unconfirmed fee count against the limit; a failed one does not.
        assert_eq!(back.counted().map(|(_, counted)| counted).sum::<u64>(), 500);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_fee_is_recorded_with_the_grids_text_or_its_own() {
        let builders = id(0xB1);
        let ours = InFlight {
            amount: 250,
            counted: 250,
            code: 1004,
            party: Some(builders),
            party_group: true,
            fallback: "Fee to join Builders".into(),
            ..InFlight::default()
        };
        let unconfirmed = ours.unconfirmed(T0);
        assert_eq!(
            (unconfirmed.kind, unconfirmed.counted, unconfirmed.code, unconfirmed.text.as_str()),
            (Kind::Unconfirmed, 250, 1004, "Fee to join Builders")
        );
        assert_eq!(unconfirmed.balance, None);
        let report = TransactionInfo {
            kind: 1004,
            dest_group: true,
            ..info(own(), builders, 250)
        };
        let Outcome::Ours(tx) = from_report(750, true, "", Some(&report), own(), Some(&ours), T0) else {
            panic!("not ours")
        };
        assert_eq!(
            (tx.kind, tx.balance, tx.text.as_str()),
            (Kind::Paid, Some(750), "Fee to join Builders")
        );
        assert_eq!((tx.party, tx.party_group, tx.avatar_name.as_str()), (Some(builders), true, ""));
        let Outcome::Ours(tx) = from_report(750, true, "You paid to join a group.", Some(&report), own(), Some(&ours), T0) else {
            panic!("not ours")
        };
        assert_eq!(tx.text, "You paid to join a group.");
        // A fee taken for a join the bot did not send.
        let Outcome::Other(tx) = from_report(750, true, "", Some(&report), own(), None, T0) else {
            panic!("not another's")
        };
        assert_eq!(tx.kind, Kind::Spent);
    }

    #[test]
    fn a_file_without_the_header_is_refused() {
        let dir = temp_dir("header");
        let path = dir.join("chat.tsv");
        std::fs::write(&path, crate::msglog::header()).unwrap();
        let error = read(&path).unwrap_err();
        assert!(error.contains("chat.tsv") && error.contains("header"), "{error}");
        assert!(read(&dir).is_err(), "a directory cannot be read");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_newest_rows_are_kept() {
        let back = ReadBack {
            rows: (0..KEPT as u64 + 40)
                .map(|n| Transaction {
                    time: T0 + n,
                    ..paid_jane()
                })
                .collect(),
            skipped: 0,
        };
        let newest = back.newest();
        assert_eq!(newest.len(), KEPT);
        assert_eq!(newest[0].time, T0 + 40);
    }
}
