//! `pay`, `prices` and `payees`: finding out whom a payment goes to, the
//! checks right before it is sent, and waiting for the grid to confirm it.
//! Which interface may use them is decided before they get here.

use super::*;
use crate::control::PayTo;
use crate::escape::escape;
use crate::payments::{limit_text, Decision, Interface, Ledger, Offer, Payee, PaymentsConfig};
use crate::txlog::{self, InFlight, Outcome, Transaction};
use crate::udp::messages::{TransactionInfo, TRANSACTION_GIFT, TRANSACTION_PAY_OBJECT};
use std::path::PathBuf;

/// How long the grid has to name an avatar a payment needs the name of.
pub const NAME_LIMIT: Duration = Duration::from_secs(5);
/// How long the simulator has to say what an object offers and whose it is.
pub const OFFER_LIMIT: Duration = Duration::from_secs(10);
/// How long a payment that was sent waits for the grid to confirm it.
pub const CONFIRM_LIMIT: Duration = Duration::from_secs(15);

/// Seconds since the Unix epoch by the system clock.
fn system_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What payments need to know, across sessions: the settings, the limit,
/// the balance, and the commands that wait.
pub struct Money {
    config: Option<PaymentsConfig>,
    ledger: Ledger,
    /// The newest balance the grid reported.
    balance: Option<i64>,
    /// The transactions file, while a row that counted against the limit
    /// waits to be written to it.
    unwritten: Option<PathBuf>,
    /// The wall clock, in seconds since the Unix epoch.
    clock: Box<dyn Fn() -> u64>,
    /// The one `pay` that may wait.
    pay: Option<Wait>,
    /// `prices` commands that wait.
    prices: Vec<Wait>,
    /// `payees` commands that wait for names.
    payees: Vec<PayeesWait>,
    /// The avatars the grid said it does not know.
    not_avatars: HashSet<Uuid>,
    /// Transactions recorded here, for the message log to take.
    records: Vec<Transaction>,
    /// The ids of the transactions the grid reported last, newest last.
    reported: std::collections::VecDeque<Uuid>,
}

/// How many transaction ids are kept to tell a repeated report, as a
/// viewer keeps them.
const REPORTS_KEPT: usize = 30;

impl Default for Money {
    fn default() -> Money {
        Money {
            config: None,
            ledger: Ledger::new(None, None),
            balance: None,
            unwritten: None,
            clock: Box::new(system_clock),
            pay: None,
            prices: Vec::new(),
            payees: Vec::new(),
            not_avatars: HashSet::new(),
            records: Vec::new(),
            reported: std::collections::VecDeque::new(),
        }
    }
}

/// Who an object's owner is, as the simulator said.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OwnedBy {
    Avatar(Uuid),
    Group(Uuid),
}

/// What a payment or a `prices` command is about.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    Avatar {
        /// The avatar as the command gave it.
        given: String,
        id: Option<Uuid>,
        /// Its login name, once known.
        name: Option<String>,
    },
    Object {
        object: Uuid,
        root: Option<Uuid>,
        offer: Option<Offer>,
        owner: Option<OwnedBy>,
        /// The name of the root prim, the linkset's.
        name: Option<String>,
        /// The owner's login name, once known.
        owner_name: Option<String>,
        /// The grid had its chance to name the owner.
        owner_asked: bool,
        /// The object scan has looked for it.
        looked: bool,
    },
    /// The fee for joining a group, which the grid takes when it gets the
    /// join or the acceptance.
    Group { group: Uuid, name: String, how: FeeFor },
}

/// What the avatar pays a group's fee for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeeFor {
    /// `group join`.
    Join,
    /// `accept group`, of the invitation `id`.
    Accept { id: Uuid },
}

/// The transaction type of a fee for joining a group (`TRANS_GROUP_JOIN`).
pub const TRANSACTION_GROUP_JOIN: i32 = 1004;

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    /// Nothing asked yet.
    Start,
    /// Waiting for the avatar search for a name.
    Finding(Uuid),
    /// Waiting for the grid to name this avatar.
    Naming(Uuid),
    /// Waiting for the simulator to describe the object.
    Looking,
    /// Waiting for what the object offers and whose it is.
    Asking,
    /// Sent; waiting for the grid to confirm it.
    Sent(InFlight),
}

/// A `pay` or `prices` command that waits.
#[derive(Clone)]
struct Wait {
    /// The amount of a `pay`; `None` for `prices`.
    amount: Option<u32>,
    origin: Origin,
    via: Interface,
    reply: Sender<Response>,
    target: Target,
    stage: Stage,
    deadline: Instant,
    /// The description to send with a payment to an avatar.
    description: Option<String>,
    /// RLV held the description back: the answer warns about it.
    held_back: bool,
}

/// What a payment's answer begins with when RLV held its description back.
pub const DESCRIPTION_BLOCKED: &str = "warning: Payment description is blocked by RLV";

/// A `payees` command waiting for the names of the UUID entries.
struct PayeesWait {
    origin: Origin,
    via: Interface,
    reply: Sender<Response>,
    ids: Vec<Uuid>,
    deadline: Instant,
}

/// `Jane Doe` for the key `jane doe` of a name entry.
fn login_name(key: &str) -> String {
    key.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Commands {
    // ------------------------------------------------------------ settings

    /// The `[payments]` table, and the limit as the transactions file left
    /// it.
    pub fn set_payments(&mut self, config: Option<PaymentsConfig>, ledger: Ledger) {
        self.money.config = config;
        self.money.ledger = ledger;
    }

    /// Sets the wall clock payments are counted by.
    #[cfg(test)]
    pub fn set_clock(&mut self, clock: impl Fn() -> u64 + 'static) {
        self.money.clock = Box::new(clock);
    }

    /// The wall clock payments are counted by, in seconds since the Unix
    /// epoch.
    pub(super) fn wall(&self) -> u64 {
        (self.money.clock)()
    }

    pub fn balance(&self) -> Option<i64> {
        self.money.balance
    }

    /// The value of the `limit` line of `status`.
    pub fn limit_text(&self) -> String {
        limit_text(&self.money.ledger, (self.money.clock)())
    }

    /// Whether a row that counted against the limit waits to be written,
    /// and to which file.
    pub fn set_unwritten(&mut self, file: Option<PathBuf>) {
        self.money.unwritten = file;
    }

    /// The transactions recorded here since this was last asked.
    pub fn take_transactions(&mut self) -> Vec<Transaction> {
        std::mem::take(&mut self.money.records)
    }

    // ------------------------------------------------------------ commands

    /// Carries out `pay`, `prices` or `payees` for an interface that may
    /// use them.
    pub fn money_command<G: Grid>(
        &mut self,
        origin: Origin,
        via: Interface,
        request: Request,
        reply: Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) {
        let command = request.name();
        let result = match request {
            Request::Pay {
                amount,
                payee,
                description,
            } => self.pay(origin, via, amount, (payee, description), &reply, grid, now),
            Request::Prices(object) => self.prices(origin, via, &object, &reply, grid, now),
            Request::Payees => self.payees(origin, via, &reply, grid, now),
            _ => Err("not a payment command".to_string()),
        };
        if let Err(reason) = result {
            if !origin.quiet() {
                self.tell(format!("{via}: {command} refused: {reason}"));
            }
            let _ = reply.send(Response::Error(reason));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pay<G: Grid>(
        &mut self,
        origin: Origin,
        via: Interface,
        amount: u32,
        (payee, description): (PayTo, Option<String>),
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<(), String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.present()?;
        if let Some(waiting) = &self.money.pay {
            return Err(format!(
                "a payment is under way: L${} to {}; wait for its answer",
                waiting.amount.unwrap_or(0),
                self.target_words(&waiting.target)
            ));
        }
        let target = match payee {
            PayTo::Avatar(given) => Target::Avatar {
                given: given.trim().to_string(),
                id: None,
                name: None,
            },
            PayTo::Object(object) => Target::Object {
                object,
                root: None,
                offer: None,
                owner: None,
                name: None,
                owner_name: None,
                owner_asked: false,
                looked: false,
            },
        };
        let wait = Wait {
            amount: Some(amount),
            origin,
            via,
            reply: reply.clone(),
            target,
            stage: Stage::Start,
            deadline: now,
            description,
            held_back: false,
        };
        let mut wait = wait;
        if self.begin(&mut wait, grid, now)? {
            self.money.pay = Some(wait);
        }
        Ok(())
    }

    fn prices<G: Grid>(
        &mut self,
        origin: Origin,
        via: Interface,
        object: &str,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<(), String> {
        let object = Uuid::parse_str(object.trim())
            .map_err(|_| format!("`prices` takes an object's UUID, not \"{object}\"; `scan --pay` lists what takes money"))?;
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.present()?;
        let wait = Wait {
            amount: None,
            origin,
            via,
            reply: reply.clone(),
            target: Target::Object {
                object,
                root: None,
                offer: None,
                owner: None,
                name: None,
                owner_name: None,
                owner_asked: false,
                looked: false,
            },
            stage: Stage::Start,
            deadline: now,
            description: None,
            held_back: false,
        };
        let mut wait = wait;
        if self.begin(&mut wait, grid, now)? {
            self.money.prices.push(wait);
        }
        Ok(())
    }

    fn payees<G: Grid>(
        &mut self,
        origin: Origin,
        via: Interface,
        reply: &Sender<Response>,
        grid: Option<&mut G>,
        now: Instant,
    ) -> Result<(), String> {
        let grid = grid.ok_or(NOT_CONNECTED)?;
        grid.present()?;
        let ids: Vec<Uuid> = self
            .payee_entries()
            .into_iter()
            .filter_map(|(entry, _)| match entry {
                perms::Entry::Id(id) => Some(id),
                perms::Entry::Name { .. } => None,
            })
            .collect();
        for id in &ids {
            self.ask_name(*id);
        }
        let wait = PayeesWait {
            origin,
            via,
            reply: reply.clone(),
            ids,
            deadline: now + NAME_LIMIT,
        };
        self.money.payees.push(wait);
        self.payees_due(now);
        Ok(())
    }

    /// The entries of the whitelist and the unlimited list, each once, with
    /// the lists it is on.
    fn payee_entries(&self) -> Vec<(perms::Entry, &'static str)> {
        let Some(config) = &self.money.config else {
            return Vec::new();
        };
        let white = config.whitelist.as_deref().unwrap_or(&[]);
        let mut entries: Vec<(perms::Entry, &'static str)> = Vec::new();
        for entry in white {
            if !entries.iter().any(|(e, _)| e == entry) {
                let lists = match config.unlimited.contains(entry) {
                    true => "whitelist,unlimited",
                    false => "whitelist",
                };
                entries.push((entry.clone(), lists));
            }
        }
        for entry in &config.unlimited {
            if !entries.iter().any(|(e, _)| e == entry) {
                entries.push((entry.clone(), "unlimited"));
            }
        }
        entries
    }

    /// Answers the `payees` commands whose names are in, or whose time is up.
    fn payees_due(&mut self, now: Instant) {
        let entries = self.payee_entries();
        for wait in std::mem::take(&mut self.money.payees) {
            let pending = wait.ids.iter().any(|id| self.name_state(*id) == NameState::Waiting);
            if pending && now < wait.deadline {
                self.money.payees.push(wait);
                continue;
            }
            let mut lines = Vec::new();
            for (entry, lists) in &entries {
                match entry {
                    perms::Entry::Name { key, .. } => lines.push(format!("?\t{}\t{lists}", login_name(key))),
                    perms::Entry::Id(id) => {
                        if let NameState::Known(name) = self.name_state(*id) {
                            lines.push(format!("{id}\t{}\t{lists}", escape(&name)));
                        }
                    }
                }
            }
            if !wait.origin.quiet() {
                self.tell(format!("{}: payees: listed {}", wait.via, lines.len()));
            }
            let text = (!lines.is_empty()).then(|| lines.join("\n"));
            let _ = wait.reply.send(Response::Ok(text));
        }
    }

    // ------------------------------------------------------------- names

    /// Asks the grid for the login name of `id`, unless it is known, or
    /// known to be no avatar's, or asked for already. A lookup that came
    /// to nothing before is tried again.
    fn ask_name(&mut self, id: Uuid) {
        if matches!(self.name_state(id), NameState::Known(_) | NameState::NoAvatar) {
            return;
        }
        if self.owner_lookups.insert(id) {
            self.owners.remove(&id);
            self.wanted_names.push(id);
        }
    }

    fn name_state(&self, id: Uuid) -> NameState {
        match self.owners.get(&id) {
            Some(Some(names)) => NameState::Known(names.legacy.clone()),
            _ if self.money.not_avatars.contains(&id) => NameState::NoAvatar,
            Some(None) if !self.owner_lookups.contains(&id) => NameState::Failed,
            _ => NameState::Waiting,
        }
    }

    /// The grid does not know these as avatars.
    pub(super) fn not_avatars(&mut self, missing: &[Uuid]) {
        self.money.not_avatars.extend(missing);
    }

    // ------------------------------------------------------------- stages

    /// Takes a command as far as it goes now. Returns whether it waits.
    fn begin<G: Grid>(&mut self, wait: &mut Wait, grid: &mut G, now: Instant) -> Result<bool, String> {
        loop {
            match self.advance(wait, grid, now)? {
                Advance::Again => continue,
                Advance::Wait => return Ok(true),
                Advance::Done => return Ok(false),
            }
        }
    }

    /// One step of a command. Errors refuse it.
    fn advance<G: Grid>(&mut self, wait: &mut Wait, grid: &mut G, now: Instant) -> Result<Advance, String> {
        match &mut wait.target {
            Target::Avatar { given, id, name } => {
                if id.is_none() {
                    match parse_recipient(given)? {
                        Recipient::Id(uuid) => *id = Some(uuid),
                        Recipient::Name(full) => match self.known.get(&full).cloned() {
                            Some((uuid, proper)) => {
                                *id = Some(uuid);
                                *name = Some(proper);
                            }
                            None => {
                                if let Stage::Finding(_) = wait.stage {
                                    return Ok(Advance::Wait);
                                }
                                self.queries += 1;
                                let query = Uuid::from_u128(self.queries);
                                grid.find_avatar(query, search_text(&full), now)?;
                                wait.stage = Stage::Finding(query);
                                wait.deadline = now + LOOKUP_LIMIT;
                                return Ok(Advance::Wait);
                            }
                        },
                    }
                }
                let uuid = id.expect("set above");
                if uuid == grid.own_id() {
                    return Err("the avatar cannot pay itself".to_string());
                }
                if name.is_none() {
                    let asked = wait.stage == Stage::Naming(uuid);
                    match self.name_state(uuid) {
                        NameState::Known(known) => *name = Some(known),
                        NameState::NoAvatar => {
                            return Err(format!(
                                "the grid knows no avatar by the UUID {uuid}; an object is paid with {}",
                                "`pay AMOUNT object UUID`"
                            ))
                        }
                        NameState::Failed if asked => return Err(format!("the grid could not say whether {uuid} is an avatar; try again")),
                        NameState::Waiting if asked => return Ok(Advance::Wait),
                        NameState::Failed | NameState::Waiting => {
                            self.ask_name(uuid);
                            wait.stage = Stage::Naming(uuid);
                            wait.deadline = now + NAME_LIMIT;
                            return Ok(Advance::Wait);
                        }
                    }
                }
            }
            Target::Object {
                object,
                root,
                offer,
                owner,
                name,
                owner_name,
                owner_asked,
                looked,
            } => {
                if root.is_none() {
                    match grid.money_target(*object) {
                        Some((_, false)) => return Err(format!("the object {object} does not take money")),
                        Some((found, true)) => *root = Some(found),
                        None if *looked => return Err(grid.unknown_object(*object)),
                        None if wait.stage == Stage::Looking => return Ok(Advance::Wait),
                        None => match grid.look_for(*object, now)? {
                            Look::Waiting => {
                                wait.stage = Stage::Looking;
                                // The scan gives up by itself; this is for one that does not.
                                wait.deadline = now + OFFER_LIMIT + Duration::from_secs(1);
                                return Ok(Advance::Wait);
                            }
                            Look::Known | Look::GaveUp => {
                                *looked = true;
                                return Ok(Advance::Again);
                            }
                        },
                    }
                    let root = root.expect("set above");
                    grid.request_pay_price(root, now)?;
                    grid.object_owner(root, now)?;
                    wait.stage = Stage::Asking;
                    wait.deadline = now + OFFER_LIMIT;
                    return Ok(Advance::Wait);
                }
                let (Some(_), Some(owned_by), Some(_)) = (offer.as_ref(), *owner, name.as_ref()) else {
                    return Ok(Advance::Wait);
                };
                if let (OwnedBy::Avatar(owner_id), None) = (owned_by, owner_name.as_ref()) {
                    match self.name_state(owner_id) {
                        NameState::Known(known) => *owner_name = Some(known),
                        NameState::Waiting | NameState::Failed if !*owner_asked => {
                            *owner_asked = true;
                            self.ask_name(owner_id);
                            wait.stage = Stage::Naming(owner_id);
                            wait.deadline = now + NAME_LIMIT;
                            return Ok(Advance::Wait);
                        }
                        NameState::Waiting if now < wait.deadline => return Ok(Advance::Wait),
                        // Without a name, as far as the lists let it go.
                        _ => {}
                    }
                }
                if wait.amount.is_none() {
                    let answer = self.prices_answer(&wait.target);
                    if !wait.origin.quiet() {
                        self.tell(format!("{}: prices of {}", wait.via, self.target_words(&wait.target)));
                    }
                    let _ = wait.reply.send(Response::Ok(Some(answer)));
                    return Ok(Advance::Done);
                }
            }
            Target::Group { .. } => {}
        }
        self.send_payment(wait, grid, now)?;
        Ok(Advance::Wait)
    }

    /// The answer to `prices`.
    fn prices_answer(&self, target: &Target) -> String {
        let Target::Object {
            offer: Some(offer),
            owner: Some(owner),
            name,
            owner_name,
            ..
        } = target
        else {
            return String::new();
        };
        let owner = match (owner, owner_name) {
            (OwnedBy::Group(group), _) => format!("group {group}"),
            (OwnedBy::Avatar(_), Some(name)) => escape(name),
            (OwnedBy::Avatar(id), None) => id.to_string(),
        };
        format!(
            "object: {}\nowner: {owner}\nprices: {}\nother amounts: {}",
            escape(name.as_deref().unwrap_or_default()),
            offer.prices_text(),
            offer.other_text()
        )
    }

    /// The payee as the log and the answers name it.
    fn target_words(&self, target: &Target) -> String {
        match target {
            Target::Avatar { given, id: None, .. } => format!("\"{given}\""),
            Target::Avatar { id: Some(id), name, .. } => whom(*id, name.as_deref().unwrap_or_default()),
            Target::Object {
                object,
                name,
                owner,
                owner_name,
                ..
            } => {
                let object = match name {
                    Some(name) => format!("{name} <{object}>"),
                    None => format!("the object {object}"),
                };
                match (owner, owner_name) {
                    (Some(OwnedBy::Avatar(id)), name) => {
                        format!("{object} of {}", whom(*id, name.as_deref().unwrap_or_default()))
                    }
                    (Some(OwnedBy::Group(group)), _) => format!("{object} of the group {group}"),
                    (None, _) => object,
                }
            }
            Target::Group { group, name, .. } => format!("the group \"{name}\" <{group}>"),
        }
    }

    /// The checks right before a payment is sent, and the sending: the
    /// object's amounts, the lists, the balance, the record of the limit
    /// and the limit itself.
    fn send_payment<G: Grid>(&mut self, wait: &mut Wait, grid: &mut G, now: Instant) -> Result<(), String> {
        let amount = wait.amount.expect("a payment");
        let config = self
            .money
            .config
            .clone()
            .ok_or("payments are not enabled: the config has no `[payments]` table")?;
        let shown = self.target_words(&wait.target);
        let (payee, in_flight, dest, kind, description) = match &wait.target {
            Target::Avatar { id, name, .. } => {
                let id = id.expect("resolved");
                let payee = Payee {
                    avatar: Some((id, name.clone())),
                    object: None,
                };
                let flight = InFlight {
                    amount,
                    counted: 0,
                    code: TRANSACTION_GIFT,
                    party: Some(id),
                    party_group: false,
                    avatar_name: name.clone().unwrap_or_default(),
                    object_name: String::new(),
                    object: None,
                    description: String::new(),
                    fallback: String::new(),
                };
                // The description is a message to the payee: RLV holds it
                // back as it would an instant message.
                let description = match wait.description.clone() {
                    Some(text) => {
                        let talking = self.talking.contains(&id);
                        match Origin::Operator.limits(grid.restrictions()).im(id, talking) {
                            Ok(()) => text,
                            Err(_) => {
                                wait.held_back = true;
                                String::new()
                            }
                        }
                    }
                    None => String::new(),
                };
                let flight = InFlight {
                    description: description.clone(),
                    ..flight
                };
                (payee, flight, id, TRANSACTION_GIFT, description)
            }
            Target::Object {
                object,
                offer,
                owner,
                name,
                owner_name,
                ..
            } => {
                let offer = offer.as_ref().expect("asked");
                if !offer.allows(amount) {
                    let other = match offer.other {
                        crate::payments::Other::No => "no other amount",
                        _ => "other amounts",
                    };
                    let quick = match offer.quick.is_empty() {
                        true => "no quick-pay amount".to_string(),
                        false => offer.quick.iter().map(|n| format!("L${n}")).collect::<Vec<_>>().join(", "),
                    };
                    return Err(format!("{shown} does not take L${amount}: it offers {quick} and {other}"));
                }
                let (party, group) = match owner.expect("asked") {
                    OwnedBy::Avatar(id) => (Some(id), false),
                    OwnedBy::Group(group) => (Some(group), true),
                };
                let payee = Payee {
                    avatar: (!group).then(|| (party.expect("an owner"), owner_name.clone())),
                    object: Some(*object),
                };
                let flight = InFlight {
                    amount,
                    counted: 0,
                    code: TRANSACTION_PAY_OBJECT,
                    party,
                    party_group: group,
                    avatar_name: owner_name.clone().unwrap_or_default(),
                    object_name: name.clone().unwrap_or_default(),
                    object: Some(*object),
                    description: String::new(),
                    fallback: String::new(),
                };
                let description = name.clone().unwrap_or_default();
                (payee, flight, *object, TRANSACTION_PAY_OBJECT, description)
            }
            Target::Group { group, name, .. } => {
                // Only the group's UUID matches an entry of the lists.
                let payee = Payee {
                    avatar: None,
                    object: Some(*group),
                };
                let flight = InFlight {
                    amount,
                    code: TRANSACTION_GROUP_JOIN,
                    party: Some(*group),
                    party_group: true,
                    fallback: format!("Fee to join {name}"),
                    ..InFlight::default()
                };
                (payee, flight, *group, TRANSACTION_GROUP_JOIN, String::new())
            }
        };
        let named = payee.avatar.as_ref().is_none_or(|(_, name)| name.is_some());
        if config.needs_name() && !named {
            return Err(format!(
                "the lists of `[payments]` could not be checked: the grid did not name {shown} within {} s",
                NAME_LIMIT.as_secs()
            ));
        }
        let counts = match config.decide(&payee) {
            Decision::Refused(refusal) => return Err(format!("{shown} {}", refusal.words())),
            Decision::Unlimited => false,
            Decision::Counted => true,
        };
        if let Some(balance) = self.money.balance {
            if i64::from(amount) > balance {
                return Err(format!("the avatar has L${balance}, less than L${amount}"));
            }
        }
        let wall = (self.money.clock)();
        let limited = counts && config.limit.is_some();
        if limited {
            if let Some(file) = &self.money.unwritten {
                return Err(format!(
                    "an earlier payment could not be written to {}, which keeps the limit; \
                     no payment that counts is made until it is",
                    file.display()
                ));
            }
        }
        if counts {
            self.money.ledger.reserve(amount, wall).map_err(|left| {
                let renews = match self.money.ledger.renews_at(wall) {
                    Some(at) => format!("; the limit renews on {}", crate::log::format_timestamp(at)),
                    None => String::new(),
                };
                format!("L${amount} is more than what is left of the limit: L${left}{renews}")
            })?;
        }
        let in_flight = InFlight {
            counted: if counts { amount } else { 0 },
            ..in_flight
        };
        let sent = match &wait.target {
            // The grid takes the fee itself: the bot sends the join or the
            // acceptance and no payment of its own.
            Target::Group { group, how, .. } => match how {
                FeeFor::Join => grid.join_group(*group, now),
                FeeFor::Accept { id } => grid.answer_invitation(*group, *id, true, now),
            },
            _ => grid.pay(dest, amount as i32, kind, &description, now),
        };
        if let Err(e) = sent {
            if counts {
                self.money.ledger.release(amount);
            }
            return Err(e);
        }
        let counted = match counts {
            true => format!(", counted against the limit (left: {})", limit_text(&self.money.ledger, wall)),
            false => ", not counted against the limit".to_string(),
        };
        let described = match (&wait.description, wait.held_back) {
            (Some(_), true) => ", without its description, which RLV holds back".to_string(),
            (Some(text), false) => format!(", with the description \"{}\"", escape(text)),
            (None, _) => String::new(),
        };
        let verb = match wait.target {
            Target::Group { how: FeeFor::Join, .. } => "join, paying",
            Target::Group { .. } => "accept the invitation, paying",
            _ => "pay",
        };
        self.tell(format!("{}: {verb} L${amount} to {shown}: sent{described}{counted}", wait.via));
        wait.stage = Stage::Sent(in_flight);
        wait.deadline = now + CONFIRM_LIMIT;
        Ok(())
    }

    // -------------------------------------------- what the grid reports

    /// Goes on with the commands that waited for something that has come.
    pub(super) fn money_again<G: Grid>(&mut self, mut grid: Option<&mut G>, now: Instant) {
        let pay = self.money.pay.take();
        let prices = std::mem::take(&mut self.money.prices);
        for (is_pay, mut wait) in pay.into_iter().map(|w| (true, w)).chain(prices.into_iter().map(|w| (false, w))) {
            let keep = match (&wait.stage, grid.as_deref_mut()) {
                (Stage::Sent(_), _) => true,
                (_, None) => {
                    self.refuse_wait(&wait, NOT_CONNECTED.to_string());
                    false
                }
                (_, Some(grid)) => match self.begin(&mut wait, grid, now) {
                    Ok(keep) => keep,
                    Err(reason) => {
                        self.refuse_wait(&wait, reason);
                        false
                    }
                },
            };
            if keep {
                match is_pay {
                    true => self.money.pay = Some(wait),
                    false => self.money.prices.push(wait),
                }
            }
        }
        self.payees_due(now);
    }

    fn refuse_wait(&mut self, wait: &Wait, reason: String) {
        if !wait.origin.quiet() {
            let what = match wait.amount {
                Some(amount) if matches!(wait.target, Target::Group { .. }) => {
                    format!("fee of L${amount} to {}", self.target_words(&wait.target))
                }
                Some(amount) => format!("pay L${amount} to {}", self.target_words(&wait.target)),
                None => format!("prices of {}", self.target_words(&wait.target)),
            };
            self.tell(format!("{}: {what} refused: {reason}", wait.via));
        }
        let _ = wait.reply.send(Response::Error(reason));
    }

    /// The avatar search answered: a payment that waited for it goes on.
    pub(super) fn money_found<G: Grid>(&mut self, query: Uuid, avatars: &[(Uuid, String)], grid: Option<&mut G>, now: Instant) {
        let Some(wait) = self.money.pay.as_mut().filter(|wait| wait.stage == Stage::Finding(query)) else {
            return;
        };
        let Target::Avatar { given, id, name } = &mut wait.target else {
            return;
        };
        let full = crate::names::full_name(given).unwrap_or_default();
        match exact_match(&full, avatars) {
            Some((found, proper)) => {
                self.known.insert(full, (*found, proper.clone()));
                *id = Some(*found);
                *name = Some(proper.clone());
            }
            None => {
                let wait = self.money.pay.take().expect("checked");
                let Target::Avatar { given, .. } = &wait.target else {
                    unreachable!()
                };
                let reason = format!("the grid knows no avatar named \"{given}\"");
                return self.refuse_wait(&wait, reason);
            }
        }
        self.money_again(grid, now);
    }

    /// The avatar search failed: a payment that waited for it is refused.
    pub(super) fn money_lookup_failed(&mut self, query: Uuid, reason: &str) {
        if let Some(wait) = self.money.pay.take() {
            match wait.stage == Stage::Finding(query) {
                true => {
                    let Target::Avatar { given, .. } = &wait.target else {
                        unreachable!()
                    };
                    let reason = format!("the lookup of \"{given}\" failed: {reason}");
                    self.refuse_wait(&wait, reason);
                }
                false => self.money.pay = Some(wait),
            }
        }
    }

    /// The simulator has described `object`, or given up: the commands
    /// that looked for it go on, with what is known now.
    pub(super) fn money_looked<G: Grid>(&mut self, object: Uuid, grid: Option<&mut G>, now: Instant) {
        let looked = |wait: &mut Wait| {
            if let Target::Object {
                object: wanted, looked, ..
            } = &mut wait.target
            {
                if *wanted == object && wait.stage == Stage::Looking {
                    *looked = true;
                    wait.stage = Stage::Start;
                }
            }
        };
        self.money.pay.iter_mut().for_each(looked);
        self.money.prices.iter_mut().for_each(looked);
        self.money_again(grid, now);
    }

    /// The simulator said what `object` offers to be paid.
    pub fn pay_price<G: Grid>(&mut self, object: Uuid, default: i32, buttons: &[i32], grid: Option<&mut G>, now: Instant) {
        let offer = Offer::from_reply(default, buttons);
        let mut changed = false;
        let mut set = |wait: &mut Wait| {
            if let Target::Object {
                root: Some(root),
                offer: slot @ None,
                ..
            } = &mut wait.target
            {
                if *root == object {
                    *slot = Some(offer.clone());
                    changed = true;
                }
            }
        };
        self.money.pay.iter_mut().for_each(&mut set);
        self.money.prices.iter_mut().for_each(&mut set);
        if changed {
            self.money_again(grid, now);
        }
    }

    /// The simulator said whose `object` is, and its name.
    pub fn pay_owner<G: Grid>(&mut self, object: Uuid, owner: Uuid, group: Uuid, name: Option<&str>, grid: Option<&mut G>, now: Instant) {
        let owned_by = match owner.is_nil() || owner == group {
            true => OwnedBy::Group(group),
            false => OwnedBy::Avatar(owner),
        };
        let mut changed = false;
        let mut set = |wait: &mut Wait| {
            if let Target::Object {
                root: Some(root),
                owner: slot @ None,
                name: named,
                ..
            } = &mut wait.target
            {
                if *root == object {
                    *slot = Some(owned_by);
                    *named = Some(name.unwrap_or_default().to_string());
                    changed = true;
                }
            }
        };
        self.money.pay.iter_mut().for_each(&mut set);
        self.money.prices.iter_mut().for_each(&mut set);
        if changed {
            self.money_again(grid, now);
        }
    }

    /// The grid reported the avatar's balance, alone or with a
    /// transaction. Answers the payment it confirms or refuses, and
    /// records what is a transaction.
    pub fn money_report(
        &mut self,
        transaction: Uuid,
        balance: i32,
        success: bool,
        description: &str,
        info: Option<&TransactionInfo>,
        own: Uuid,
    ) {
        self.money.balance = Some(i64::from(balance));
        // Second Life reports a group's fee twice with one transaction id;
        // a viewer shows such a report once, and the bot records it once.
        if !transaction.is_nil() {
            if self.money.reported.contains(&transaction) {
                crate::debug!("the grid reported the transaction {transaction} again; it is recorded once");
                return;
            }
            if self.money.reported.len() >= REPORTS_KEPT {
                self.money.reported.pop_front();
            }
            self.money.reported.push_back(transaction);
        }
        let wall = (self.money.clock)();
        let in_flight = match self.money.pay.as_ref().map(|wait| &wait.stage) {
            Some(Stage::Sent(in_flight)) => Some(in_flight.clone()),
            _ => None,
        };
        match txlog::from_report(balance, success, description, info, own, in_flight.as_ref(), wall) {
            Outcome::Balance => {}
            Outcome::Other(tx) => self.money.records.push(tx),
            Outcome::Ours(tx) => {
                let wait = self.money.pay.take().expect("a payment in flight");
                if let Target::Group { group, .. } = wait.target {
                    return self.fee_reported(wait, group, tx, balance, description);
                }
                let shown = self.target_words(&wait.target);
                let amount = wait.amount.unwrap_or(0);
                match tx.kind {
                    txlog::Kind::Paid => {
                        self.tell(format!("{}: pay L${amount} to {shown}: made, balance L${balance}", wait.via));
                        let answer = format!("paid L${amount} to {shown}; the balance is L${balance}");
                        let _ = wait.reply.send(Response::Ok(Some(warned(&wait, answer))));
                    }
                    _ => {
                        if let Some(Stage::Sent(flight)) = Some(&wait.stage) {
                            if flight.counted > 0 {
                                self.money.ledger.release(flight.counted);
                            }
                        }
                        let reason = match description.trim() {
                            "" => "the grid gave no reason".to_string(),
                            reason => reason.to_string(),
                        };
                        self.tell(format!("{}: pay L${amount} to {shown}: failed: {reason}", wait.via));
                        let answer = format!("the grid refused the payment of L${amount} to {shown}: {reason}");
                        let _ = wait.reply.send(Response::Error(warned(&wait, answer)));
                    }
                }
                self.money.records.push(tx);
            }
        }
    }

    /// A payment the grid has not confirmed, for `why`: answered, logged
    /// and recorded as unconfirmed; it keeps counting.
    fn unconfirmed(&mut self, wait: Wait, why: &str) {
        let Stage::Sent(in_flight) = &wait.stage else {
            return;
        };
        let shown = self.target_words(&wait.target);
        let amount = wait.amount.unwrap_or(0);
        if let Target::Group { .. } = wait.target {
            // The command is answered by what the grid says of the join.
            self.tell(format!(
                "{}: fee of L${amount} to {shown}: not reported: {why}; it counts as paid",
                wait.via
            ));
            let wall = (self.money.clock)();
            self.money.records.push(in_flight.unconfirmed(wall));
            return;
        }
        self.tell(format!("{}: pay L${amount} to {shown}: not confirmed: {why}", wait.via));
        let answer = format!(
            "the payment of L${amount} to {shown} was sent, but {why}; it counts as paid. \
             Check the balance with `status` before paying again"
        );
        let _ = wait.reply.send(Response::Error(warned(&wait, answer)));
        let wall = (self.money.clock)();
        self.money.records.push(in_flight.unconfirmed(wall));
    }

    /// Gives up on what waits too long.
    pub fn money_due<G: Grid>(&mut self, grid: Option<&mut G>, now: Instant) {
        if let Some(wait) = self.money.pay.take() {
            match (&wait.stage, now >= wait.deadline) {
                (_, false) => self.money.pay = Some(wait),
                (Stage::Sent(_), true) => {
                    let why = match wait.target {
                        Target::Group { .. } => format!("the grid has not reported it within {} s", CONFIRM_LIMIT.as_secs()),
                        _ => format!("the grid has not confirmed it within {} s", CONFIRM_LIMIT.as_secs()),
                    };
                    self.unconfirmed(wait, &why);
                }
                (stage, true) => {
                    let reason = self.late_reason(&wait, stage);
                    match reason {
                        Some(reason) => self.refuse_wait(&wait, reason),
                        // A name that did not come: go on without it.
                        None => self.money.pay = Some(wait),
                    }
                }
            }
        }
        for wait in std::mem::take(&mut self.money.prices) {
            match now >= wait.deadline {
                false => self.money.prices.push(wait),
                true => match self.late_reason(&wait, &wait.stage) {
                    Some(reason) => self.refuse_wait(&wait, reason),
                    None => self.money.prices.push(wait),
                },
            }
        }
        self.money_again(grid, now);
    }

    /// Why a command that waited too long is refused; `None` when it goes
    /// on without what it waited for.
    fn late_reason(&self, wait: &Wait, stage: &Stage) -> Option<String> {
        match stage {
            Stage::Finding(_) => {
                let Target::Avatar { given, .. } = &wait.target else { return None };
                Some(format!(
                    "the grid did not answer the lookup of \"{given}\" within {} s",
                    LOOKUP_LIMIT.as_secs()
                ))
            }
            Stage::Naming(id) => match &wait.target {
                Target::Avatar { .. } => Some(format!(
                    "the grid has not said within {} s whether it knows an avatar {id}",
                    NAME_LIMIT.as_secs()
                )),
                Target::Object { .. } | Target::Group { .. } => None,
            },
            Stage::Looking => {
                let Target::Object { object, .. } = &wait.target else { return None };
                Some(format!("the object {object} is not known to the bot"))
            }
            Stage::Asking => {
                let Target::Object { object, .. } = &wait.target else { return None };
                Some(format!(
                    "the simulator did not say within {} s what the object {object} offers and whose it is",
                    OFFER_LIMIT.as_secs()
                ))
            }
            Stage::Sent(_) | Stage::Start => None,
        }
    }

    /// When `money_due` next has something to do.
    pub(super) fn money_deadline(&self) -> Option<Instant> {
        let waits = self.money.pay.iter().chain(&self.money.prices).map(|wait| wait.deadline);
        waits.chain(self.money.payees.iter().map(|wait| wait.deadline)).min()
    }

    /// The session is gone: a payment that was sent is not confirmed, and
    /// everything else that waited is refused.
    pub(super) fn money_lost(&mut self) {
        if let Some(wait) = self.money.pay.take() {
            match wait.stage {
                Stage::Sent(_) => self.unconfirmed(wait, "the session was lost before the grid confirmed it"),
                _ => self.refuse_wait(&wait, NOT_CONNECTED.to_string()),
            }
        }
        for wait in std::mem::take(&mut self.money.prices) {
            self.refuse_wait(&wait, NOT_CONNECTED.to_string());
        }
        for wait in std::mem::take(&mut self.money.payees) {
            if !wait.origin.quiet() {
                self.tell(format!("{}: payees refused: {NOT_CONNECTED}", wait.via));
            }
            let _ = wait.reply.send(Response::Error(NOT_CONNECTED.to_string()));
        }
    }

    // ------------------------------------------------------- group fees

    /// Pays `fee` to `group` by sending the join or the acceptance `how`
    /// names, with every check of a payment first. Errors refuse it, and
    /// nothing is sent then.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pay_fee<G: Grid>(
        &mut self,
        fee: u32,
        group: Uuid,
        name: &str,
        how: FeeFor,
        via: Interface,
        reply: &Sender<Response>,
        grid: &mut G,
        now: Instant,
    ) -> Result<(), String> {
        let config = match &self.money.config {
            None => return Err("payments are not enabled: the config has no `[payments]` table".to_string()),
            Some(config) => config,
        };
        if !config.may(via) {
            return Err(format!(
                "payments are not enabled for `{via}`: `perms` in the config's `[payments]` table does not list it"
            ));
        }
        if let Some(waiting) = &self.money.pay {
            return Err(format!(
                "a payment is under way: L${} to {}; wait for its answer",
                waiting.amount.unwrap_or(0),
                self.target_words(&waiting.target)
            ));
        }
        // A row the limit needs is written first, if it can be now; the
        // bot says whether one still waits before every command.
        let mut wait = Wait {
            amount: Some(fee),
            origin: self.origin,
            via,
            reply: reply.clone(),
            target: Target::Group {
                group,
                name: name.to_string(),
                how,
            },
            stage: Stage::Start,
            deadline: now,
            description: None,
            held_back: false,
        };
        self.send_payment(&mut wait, grid, now)?;
        self.money.pay = Some(wait);
        Ok(())
    }

    /// The grid reported the fee of a join it was sent: recorded, and the
    /// command that waits for the join is told.
    fn fee_reported(&mut self, wait: Wait, group: Uuid, tx: Transaction, balance: i32, description: &str) {
        let shown = self.target_words(&wait.target);
        let amount = wait.amount.unwrap_or(0);
        match tx.kind {
            txlog::Kind::Paid => {
                self.tell(format!("{}: fee of L${amount} to {shown}: made, balance L${balance}", wait.via));
                self.fee_paid(group, i64::from(balance));
            }
            _ => {
                if let Stage::Sent(flight) = &wait.stage {
                    if flight.counted > 0 {
                        self.money.ledger.release(flight.counted);
                    }
                }
                let reason = match description.trim() {
                    "" => "the grid gave no reason".to_string(),
                    reason => reason.to_string(),
                };
                self.tell(format!("{}: fee of L${amount} to {shown}: failed: {reason}", wait.via));
                self.fee_failed(group, &reason);
            }
        }
        self.money.records.push(tx);
    }

    /// The grid refused the join a fee was sent with: the fee is recorded
    /// as failed and stops counting.
    pub(super) fn fee_refused(&mut self, group: Uuid, reason: &str) {
        let ours =
            matches!(&self.money.pay, Some(Wait { target: Target::Group { group: g, .. }, stage: Stage::Sent(_), .. }) if *g == group);
        if !ours {
            return;
        }
        let wait = self.money.pay.take().expect("checked");
        let Stage::Sent(flight) = &wait.stage else { return };
        if flight.counted > 0 {
            self.money.ledger.release(flight.counted);
        }
        let shown = self.target_words(&wait.target);
        self.tell(format!(
            "{}: fee of L${} to {shown}: failed: {reason}",
            wait.via,
            wait.amount.unwrap_or(0)
        ));
        let wall = (self.money.clock)();
        let record = Transaction {
            kind: txlog::Kind::Failed,
            counted: 0,
            text: format!("{}: {reason}", flight.fallback),
            ..flight.unconfirmed(wall)
        };
        self.money.records.push(record);
    }

    /// The bot stops: a payment that was sent is recorded as unconfirmed.
    pub fn money_stop(&mut self) {
        if let Some(wait) = self.money.pay.take() {
            match wait.stage {
                Stage::Sent(_) => self.unconfirmed(wait, "the bot stopped before the grid confirmed it"),
                _ => self.refuse_wait(&wait, "the bot is stopping".to_string()),
            }
        }
    }
}

/// The answer to a payment that was sent, with the warning first when
/// RLV held its description back.
fn warned(wait: &Wait, answer: String) -> String {
    match wait.held_back {
        true => format!("{DESCRIPTION_BLOCKED}\n{answer}"),
        false => answer,
    }
}

/// Whether the RLV restrictions in force hold back every payment's
/// description: `sendim` is held, with or without exceptions.
pub fn descriptions_blocked(restrictions: Option<&crate::rlv::Restrictions>) -> bool {
    Limits::new(restrictions, None).in_force("sendim").is_some()
}

/// What the grid has said about an avatar's name.
#[derive(Debug, Clone, PartialEq)]
enum NameState {
    Known(String),
    /// The grid knows no avatar by that UUID.
    NoAvatar,
    /// The lookup came to nothing.
    Failed,
    Waiting,
}

/// How far a step took a command.
enum Advance {
    /// It can go on at once.
    Again,
    /// It waits for something.
    Wait,
    /// It is answered.
    Done,
}

#[cfg(test)]
mod tests;
