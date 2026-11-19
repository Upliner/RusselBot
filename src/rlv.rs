//! RLV, the RestrainedLove API: scripted objects say `@` commands to their
//! owner with `llOwnerSay`, and a viewer that implements the API carries
//! them out. This is the part of it the bot has: reading the commands,
//! answering the version queries, and carrying out the force commands that
//! have a control-socket command. Restrictions are not implemented, which
//! the version the bot reports says.

use crate::commands::{Commands, Grid, Origin};
use crate::config::RlvPersist;
use crate::control::{Request, Response};
use crate::inventory;
use crate::persist::SitState;
use crate::session::{Message, Source, NOT_CONNECTED};
use crate::udp::messages::Vector3;
use std::collections::VecDeque;
use std::fmt;
use std::sync::mpsc;
use std::time::Instant;
use uuid::Uuid;

/// The version of the RLV API the bot implements. Far below any viewer's,
/// so that a script that looks at it sees an incomplete implementation.
pub const VERSION: &str = "0.1.0";
/// `VERSION` as `@versionnum` gives it: X·10⁶ + Y·10⁴ + Z·10² + P.
pub const VERSION_NUM: u32 = 10_000;
/// How many messages are kept until the avatar can act.
pub const MAX_WAITING: usize = 64;
/// The chat type of the answer to a query: said at normal volume.
const SAY: u8 = 1;
/// A region's side in metres, for global coordinates.
const REGION_SIDE: f64 = 256.0;
const NOT_SUPPORTED: &str = "not supported";

/// The names RLV has for attachment points where the bot's own names
/// differ: the points of the heads-up display without `HUD`, and the root.
const RLV_POINTS: [(&str, u8); 9] = [
    ("center 2", 31),
    ("top right", 32),
    ("top", 33),
    ("top left", 34),
    ("center", 35),
    ("bottom left", 36),
    ("bottom", 37),
    ("bottom right", 38),
    ("root", 40),
];

// ---------------------------------------------------------------- parsing

/// What a command asks for, by its param.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    /// Do it once, now.
    Force,
    /// Answer on this chat channel; `None` for a number no channel can be.
    Channel(Option<i32>),
    /// Set or lift a restriction: `n`, `add`, `y` or `rem`.
    Restrict,
    /// None of these: `@clear` comes alone, or with a filter after its
    /// `=`.
    Absent,
}

/// One RLV command: `behaviour[:option]=param`.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// The command as it was written, for the log and for answers.
    pub text: String,
    /// In lower case.
    pub behaviour: String,
    /// As it was written: it carries names and UUIDs.
    pub option: Option<String>,
    pub param: Param,
}

/// A piece of a message that is no RLV command.
#[derive(Debug, Clone, PartialEq)]
pub struct Malformed {
    pub text: String,
    pub reason: &'static str,
}

/// Reads the commands of a message, with or without its leading `@`: the
/// pieces between commas, in order. Nothing is trimmed; RLV commands are
/// sensitive to spaces.
pub fn parse(text: &str) -> Vec<Result<Command, Malformed>> {
    let text = text.strip_prefix('@').unwrap_or(text);
    text.split(',').filter(|piece| !piece.is_empty()).map(parse_one).collect()
}

fn parse_one(piece: &str) -> Result<Command, Malformed> {
    let malformed = |reason| Malformed {
        text: piece.to_string(),
        reason,
    };
    let Some((left, param)) = piece.split_once('=') else {
        // `@clear` alone lifts everything its sender has set.
        if !piece.eq_ignore_ascii_case("clear") {
            return Err(malformed("it has no `=`"));
        }
        return Ok(Command {
            text: piece.to_string(),
            behaviour: "clear".to_string(),
            option: None,
            param: Param::Absent,
        });
    };
    let (behaviour, option) = match left.split_once(':') {
        Some((behaviour, option)) => (behaviour, Some(option.to_string())),
        None => (left, None),
    };
    if behaviour.is_empty() {
        return Err(malformed("it names no behaviour"));
    }
    let lowered = param.to_ascii_lowercase();
    let filter = behaviour.eq_ignore_ascii_case("clear") && option.is_none();
    let digits = lowered.strip_prefix(['-', '+']).unwrap_or(&lowered);
    let param = match lowered.as_str() {
        "force" => Param::Force,
        _ if filter => Param::Absent,
        "n" | "add" | "y" | "rem" => Param::Restrict,
        _ if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => Param::Channel(lowered.parse().ok()),
        _ => return Err(malformed("its param is not `force`, a channel number, `n`, `add`, `y` or `rem`")),
    };
    Ok(Command {
        text: piece.to_string(),
        behaviour: behaviour.to_ascii_lowercase(),
        option,
        param,
    })
}

// ----------------------------------------------------------------- senders

/// Who sent RLV commands: an object the avatar owns, or the operator
/// through `rlv cmd`, who stands in for the avatar itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Issuer {
    pub id: Uuid,
    pub name: String,
    pub operator: bool,
}

impl Issuer {
    /// The operator, with the avatar's own id as a viewer's console has it.
    pub fn operator(avatar: Uuid) -> Issuer {
        Issuer {
            id: avatar,
            name: String::new(),
            operator: true,
        }
    }
}

impl fmt::Display for Issuer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.operator {
            true => f.write_str("operator"),
            false => write!(f, "{} <{}>", self.name, self.id),
        }
    }
}

/// The sender of the RLV commands in a chat message, if it carries any: one
/// an object said to its owner and that starts with `@`.
pub fn sender(message: &Message) -> Option<Issuer> {
    let Source::Object { id: Some(id), name, .. } = &message.source else {
        return None;
    };
    (message.kind == "owner" && message.text.starts_with('@')).then(|| Issuer {
        id: *id,
        name: name.clone(),
        operator: false,
    })
}

// ------------------------------------------------------------ carrying out

/// A forced movement the commands have carried out or begun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Movement {
    Sit,
    Ground,
    Stand,
    Teleport,
}

/// What became of a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A query: this was said on its channel.
    Said(String),
    /// Carried out or begun; the placement is not concerned.
    Done,
    /// A forced movement, carried out or begun as the control-socket
    /// command is: the commands went manual.
    Moved(Movement),
    /// A forced sit that is to be followed: nothing was done yet. The bot
    /// persists this object as its seat, which seats the avatar.
    HoldSeat(Uuid),
    /// Not carried out, and why.
    Failed(String),
}

/// The reply to a version query.
fn version_reply(behaviour: &str) -> Option<String> {
    let bot = env!("CARGO_PKG_VERSION");
    match behaviour {
        // The old name is what the command has always answered with.
        "version" => Some(format!("RestrainedLife viewer v{VERSION} (RusselBot {bot})")),
        "versionnew" => Some(format!("RestrainedLove viewer v{VERSION} (RusselBot {bot})")),
        "versionnum" => Some(VERSION_NUM.to_string()),
        _ => None,
    }
}

/// The attachment point an RLV command names. A number is none: RLV has
/// no numbers here, and a folder may be called `5`.
fn point(option: &str) -> Option<u8> {
    if option.trim().parse::<u64>().is_ok() {
        return None;
    }
    let rlv = || RLV_POINTS.iter().find(|(name, _)| name.eq_ignore_ascii_case(option));
    inventory::parse_point(option).or_else(|| rlv().map(|(_, point)| *point))
}

/// Where a `@tpto` goes.
#[derive(Debug, PartialEq)]
enum Destination {
    /// The arguments of a `tp` command: x, y, z and the region.
    Named(Vec<String>),
    /// The handle of the region the point is in, and the point inside it.
    Global(u64, Vector3),
}

/// Reads the option of `@tpto`: `region/x/y/z`, or `X/Y/Z` in metres from
/// the grid's origin. What follows a `;` is a direction to look in.
fn destination(option: &str) -> Result<Destination, String> {
    let place = option.split(';').next().unwrap_or_default();
    let number = |text: &str, name: &str| match text.parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(format!("`{name}` must be a number that is not negative, not \"{text}\"")),
    };
    match place.split('/').collect::<Vec<_>>()[..] {
        [region, x, y, z] => {
            if region.trim().is_empty() {
                return Err("the destination names no region".to_string());
            }
            for (text, name) in [(x, "x"), (y, "y"), (z, "z")] {
                number(text, name)?;
            }
            Ok(Destination::Named([x, y, z, region].map(str::to_string).to_vec()))
        }
        [x, y, z] => {
            let (x, y, z) = (number(x, "X")?, number(y, "Y")?, number(z, "Z")?);
            let corner = |value: f64| (value / REGION_SIDE).floor() * REGION_SIDE;
            if x.max(y) >= f64::from(u32::MAX) {
                return Err("the destination is outside any grid".to_string());
            }
            let handle = ((corner(x) as u64) << 32) | corner(y) as u64;
            // No grid has a region there, and one asked for it never
            // answers. Most likely these are coordinates inside a region.
            if handle == 0 {
                return Err(format!(
                    "\"{place}\" are global coordinates in the region at 0, 0, where no grid has one; \
                     for a place inside a region write `region/x/y/z`"
                ));
            }
            Ok(Destination::Global(
                handle,
                [(x - corner(x)) as f32, (y - corner(y)) as f32, z as f32],
            ))
        }
        _ => Err(format!("\"{place}\" is neither `region/x/y/z` nor global `X/Y/Z`")),
    }
}

/// Carries out `request` as the control-socket command would be, for RLV.
/// Nobody waits for an answer that comes later; the log has it.
fn carry<G: Grid>(request: Request, commands: &mut Commands, grid: Option<&mut G>, now: Instant) -> Result<(), String> {
    let (reply, answer) = mpsc::channel();
    commands.handle_from(Origin::Rlv, request, reply, grid, now);
    match answer.try_recv() {
        Ok(Response::Error(reason)) => Err(reason),
        // Answered, or put aside until the simulator has spoken.
        _ => Ok(()),
    }
}

/// Deals with one command of `issuer`. `follow` is the RLV persist
/// setting: a forced sit is then left to the bot (`Step::HoldSeat`).
pub fn deal<G: Grid>(
    command: &Command,
    issuer: &Issuer,
    follow: bool,
    commands: &mut Commands,
    grid: Option<&mut G>,
    now: Instant,
) -> Step {
    let text = &command.text;
    let refused = |reason: String| {
        crate::info!("rlv: {issuer}: {text}: refused: {reason}");
        Step::Failed(reason)
    };
    let unsupported = || {
        crate::info!("rlv: {issuer}: {text}: {NOT_SUPPORTED}");
        Step::Failed(NOT_SUPPORTED.to_string())
    };
    // Hands a force command to the control-socket command it stands for.
    let mut as_command = |shown: String, request: Request, done: Step, grid: Option<&mut G>| {
        crate::info!("rlv: {issuer}: {text}: as `{shown}`");
        match carry(request, commands, grid, now) {
            Ok(()) => done,
            // The commands have logged why.
            Err(reason) => Step::Failed(reason),
        }
    };
    let option = command.option.as_deref();
    match (command.param, command.behaviour.as_str()) {
        (Param::Channel(channel), behaviour @ ("version" | "versionnew" | "versionnum")) => {
            let reply = version_reply(behaviour).unwrap_or_default();
            let channel = match channel {
                Some(0) => return refused("channel 0 cannot be answered on".to_string()),
                Some(channel) => channel,
                None => return refused("the channel is out of range".to_string()),
            };
            let Some(grid) = grid else {
                return refused(NOT_CONNECTED.to_string());
            };
            match grid.say(channel, SAY, &reply, now) {
                Ok(()) => {
                    crate::info!("rlv: {issuer}: {text}: said \"{reply}\" on channel {channel}");
                    Step::Said(reply)
                }
                Err(reason) => refused(reason),
            }
        }
        (Param::Force, "sit") => {
            let given = option.unwrap_or_default();
            let Ok(object) = Uuid::parse_str(given) else {
                return refused(format!("\"{given}\" is not an object's UUID"));
            };
            if follow {
                return Step::HoldSeat(object);
            }
            as_command(
                format!("sit {object}"),
                Request::Sit(object.to_string()),
                Step::Moved(Movement::Sit),
                grid,
            )
        }
        (Param::Force, "sitground") => {
            let request = Request::Sit("ground".to_string());
            as_command("sit ground".to_string(), request, Step::Moved(Movement::Ground), grid)
        }
        (Param::Force, "unsit") => as_command("stand".to_string(), Request::Stand, Step::Moved(Movement::Stand), grid),
        (Param::Force, "tpto") => match destination(option.unwrap_or_default()) {
            Err(reason) => refused(reason),
            Ok(Destination::Named(args)) => {
                let shown = format!("tp {}", args.join(" "));
                as_command(shown, Request::Tp(args), Step::Moved(Movement::Teleport), grid)
            }
            Ok(Destination::Global(handle, position)) => {
                crate::info!("rlv: {issuer}: {text}: as a teleport by global coordinates");
                match commands.tp_global(handle, position, grid, now) {
                    Ok(()) => Step::Moved(Movement::Teleport),
                    Err(reason) => refused(reason),
                }
            }
        },
        // Without an option these take everything off, which no command of
        // the bot does; with one that is neither a point nor a UUID they
        // name a folder.
        (Param::Force, "detach" | "remattach") => {
            if let Some(point) = option.and_then(point) {
                let name = inventory::point_name(point);
                return as_command(format!("detach {name}"), Request::Detach(name), Step::Done, grid);
            }
            let Some(given) = option.and_then(|option| Uuid::parse_str(option).ok()) else {
                return unsupported();
            };
            // The UUID of an attachment, as a script knows it: its object.
            // One that is no such object is taken for the inventory item,
            // which is what the bot's own commands show.
            let attached = grid.as_ref().and_then(|grid| grid.attached_item(given));
            let item = attached.unwrap_or(given);
            as_command(format!("detach {item}"), Request::Detach(item.to_string()), Step::Done, grid)
        }
        (Param::Force, "remoutfit") => match option.map(|kind| (kind, inventory::parse_kind(kind))) {
            Some((_, Some(kind))) => {
                let name = inventory::kind_name(kind);
                as_command(format!("unwear {name}"), Request::Detach(name), Step::Done, grid)
            }
            Some((given, None)) => refused(format!("\"{given}\" is not a kind of clothing")),
            None => unsupported(),
        },
        (Param::Force, "detachme") => {
            let Some(grid) = grid else {
                return refused(NOT_CONNECTED.to_string());
            };
            let attached = if issuer.operator { None } else { grid.attached_item(issuer.id) };
            match attached {
                Some(item) => as_command(format!("detach {item}"), Request::Detach(item.to_string()), Step::Done, Some(grid)),
                None => refused("the sender is not attached".to_string()),
            }
        }
        _ => unsupported(),
    }
}

// ------------------------------------------------------------------- state

/// RLV as the bot runs it: whether it is on, what a forced movement does to
/// the held placement, and the messages that wait for the avatar.
pub struct Rlv {
    on: bool,
    persist: RlvPersist,
    /// Messages that arrived while the avatar could not act, in order.
    waiting: VecDeque<(Issuer, String)>,
}

impl Rlv {
    pub fn new(on: bool, persist: RlvPersist) -> Rlv {
        if on {
            crate::info!("rlv: on");
        }
        Rlv {
            on,
            persist,
            waiting: VecDeque::new(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Switches RLV on or off. Off forgets what waits. Returns the line
    /// that says which it is now.
    pub fn set_on(&mut self, on: bool) -> String {
        let line = format!("rlv: {}", if on { "on" } else { "off" });
        if self.on != on {
            crate::info!("{line}");
        }
        self.on = on;
        if !on {
            self.forget_waiting();
        }
        line
    }

    /// Whether a forced movement is followed by a persist.
    pub fn follows(&self) -> bool {
        self.persist == RlvPersist::Follow
    }

    fn persist_word(&self) -> &'static str {
        match self.persist {
            RlvPersist::Drop => "drop",
            RlvPersist::Follow => "follow",
        }
    }

    /// Sets the RLV persist setting. Returns the line that says what it is.
    pub fn set_persist(&mut self, persist: RlvPersist) -> String {
        if self.persist != persist {
            self.persist = persist;
            crate::info!("rlv: persist {}", self.persist_word());
        }
        format!("rlv persist: {}", self.persist_word())
    }

    /// The answer to `rlv status`.
    pub fn status(&self) -> String {
        format!("rlv: {}\npersist: {}", if self.on { "on" } else { "off" }, self.persist_word())
    }

    /// Keeps a message until the avatar can act. A full queue drops it.
    pub fn keep(&mut self, issuer: Issuer, text: String) {
        if self.waiting.len() >= MAX_WAITING {
            crate::info!("rlv: {issuer}: dropped, {MAX_WAITING} messages are waiting already: {text}");
            return;
        }
        self.waiting.push_back((issuer, text));
        let count = self.waiting.len();
        let messages = if count == 1 { "message" } else { "messages" };
        crate::info!("rlv: keeping {count} {messages} until the avatar is in a region");
    }

    pub fn has_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// The message that has waited longest.
    pub fn next_waiting(&mut self) -> Option<(Issuer, String)> {
        self.waiting.pop_front()
    }

    pub fn forget_waiting(&mut self) {
        if !self.waiting.is_empty() {
            crate::info!("rlv: dropped {} waiting messages", self.waiting.len());
            self.waiting.clear();
        }
    }
}

// --------------------------------------------------------------- following

/// What the bot does about its placement after a step of a forced movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum After {
    Nothing,
    /// Carry out `persist cur <sit state>`.
    Persist(SitState),
    /// The movement came to nothing: back to the placement mode from
    /// before it.
    Restore {
        manual: bool,
    },
}

/// Follows forced movements that end later than they are accepted: a
/// teleport is persisted when the avatar has arrived. (A followed sit is a
/// persist from the start, and is the bot's own business.)
#[derive(Debug, Default)]
pub struct Follow {
    /// A followed teleport is under way; whether the bot was in manual
    /// mode before it.
    teleport: Option<bool>,
}

impl Follow {
    /// The commands accepted a forced movement while the bot was in the
    /// placement mode `was_manual`. An earlier teleport under way is given
    /// up; its mode is the one to go back to.
    pub fn moved(&mut self, movement: Movement, follow: bool, was_manual: bool) -> After {
        let before = self.teleport.take().unwrap_or(was_manual);
        match (follow, movement) {
            (false, _) | (true, Movement::Sit) => After::Nothing,
            (true, Movement::Stand) => After::Persist(SitState::Stand),
            (true, Movement::Ground) => After::Persist(SitState::Ground),
            (true, Movement::Teleport) => {
                self.teleport = Some(before);
                After::Nothing
            }
        }
    }

    /// The avatar has arrived somewhere.
    pub fn arrived(&mut self) -> After {
        match self.teleport.take() {
            Some(_) => After::Persist(SitState::Stand),
            None => After::Nothing,
        }
    }

    /// A teleport failed, or the session was lost.
    pub fn failed(&mut self) -> After {
        match self.teleport.take() {
            Some(manual) => After::Restore { manual },
            None => After::Nothing,
        }
    }

    /// The operator has taken over, or something else was persisted.
    pub fn give_up(&mut self) {
        self.teleport = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tests::{Call, FakeGrid};

    fn command(text: &str) -> Command {
        match &parse(text)[..] {
            [Ok(command)] => command.clone(),
            other => panic!("expected one command in {text}, got {other:?}"),
        }
    }

    fn parts(text: &str) -> (String, Option<String>, Param) {
        let command = command(text);
        (command.behaviour, command.option, command.param)
    }

    fn reason(text: &str) -> &'static str {
        match &parse(text)[..] {
            [Err(malformed)] => malformed.reason,
            other => panic!("expected one malformed piece in {text}, got {other:?}"),
        }
    }

    #[test]
    fn a_message_is_split_into_its_commands() {
        let texts = |message: &str| -> Vec<String> {
            parse(message)
                .into_iter()
                .map(|piece| match piece {
                    Ok(command) => command.text,
                    Err(malformed) => format!("!{}", malformed.text),
                })
                .collect()
        };
        assert_eq!(texts("@versionnum=5,unsit=force"), ["versionnum=5", "unsit=force"]);
        // The at-sign is the message's, and `rlv cmd` may leave it out.
        assert_eq!(texts("unsit=force"), ["unsit=force"]);
        // Empty pieces are nothing, a trailing comma included.
        assert_eq!(texts("@unsit=force,"), ["unsit=force"]);
        assert_eq!(texts("@,,a=n,,"), ["a=n"]);
        assert_eq!(texts("@"), Vec::<String>::new());
        // A bad piece does not spoil the others.
        assert_eq!(texts("@nonsense,versionnum=5"), ["!nonsense", "versionnum=5"]);
    }

    #[test]
    fn a_command_is_behaviour_option_and_param() {
        assert_eq!(parts("@unsit=force"), ("unsit".to_string(), None, Param::Force));
        assert_eq!(
            parts("@versionnum=2222"),
            ("versionnum".to_string(), None, Param::Channel(Some(2222)))
        );
        assert_eq!(
            parts("@versionnum=-873246"),
            ("versionnum".to_string(), None, Param::Channel(Some(-873246)))
        );
        assert_eq!(parts("@version=0"), ("version".to_string(), None, Param::Channel(Some(0))));
        assert_eq!(parts("@version=99999999999"), ("version".to_string(), None, Param::Channel(None)));
        for restriction in ["n", "add", "y", "rem"] {
            assert_eq!(parts(&format!("@detach={restriction}")).2, Param::Restrict, "{restriction}");
        }
        let (behaviour, option, param) = parts("@sit:11111111-2222-3333-4444-555555555555=force");
        assert_eq!(behaviour, "sit");
        assert_eq!(option.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
        assert_eq!(param, Param::Force);
        // An empty option is still an option.
        assert_eq!(parts("@detach:=force").1.as_deref(), Some(""));
        // `@clear` is the one command that may come without a param.
        assert_eq!(parts("@clear"), ("clear".to_string(), None, Param::Absent));
        assert_eq!(parts("@CLEAR"), ("clear".to_string(), None, Param::Absent));
        // And what follows its `=` is a filter, not one of the params.
        assert_eq!(parts("@clear=tp").2, Param::Absent);
        assert_eq!(parts("@clear=tplure:").2, Param::Absent);
    }

    #[test]
    fn behaviour_and_param_ignore_case_and_the_option_keeps_it() {
        assert_eq!(parts("@UnSit=FORCE"), ("unsit".to_string(), None, Param::Force));
        assert_eq!(parts("@Detach=N").2, Param::Restrict);
        let (behaviour, option, _) = parts("@TpTo:Blue Lagoon/1/2/3=Force");
        assert_eq!((behaviour.as_str(), option.as_deref()), ("tpto", Some("Blue Lagoon/1/2/3")));
    }

    #[test]
    fn the_first_equals_sign_and_the_first_colon_split() {
        let (behaviour, option, param) = parts("@tpto:A:B/1/2/3=force");
        assert_eq!(
            (behaviour.as_str(), option.as_deref(), param),
            ("tpto", Some("A:B/1/2/3"), Param::Force)
        );
        // What follows the first `=` is the param, whatever it holds.
        assert_eq!(
            reason("@notify:5;x=y=add"),
            "its param is not `force`, a channel number, `n`, `add`, `y` or `rem`"
        );
    }

    #[test]
    fn malformed_pieces_say_why() {
        assert_eq!(reason("@nonsense"), "it has no `=`");
        assert_eq!(reason("@clearance"), "it has no `=`");
        assert_eq!(reason("@=force"), "it names no behaviour");
        assert_eq!(reason("@:chest=force"), "it names no behaviour");
        assert!(reason("@unsit=maybe").starts_with("its param"));
        assert!(reason("@unsit=").starts_with("its param"));
        assert!(reason("@version=5.5").starts_with("its param"));
    }

    #[test]
    fn spaces_are_part_of_what_they_are_in() {
        // `@unsit = force`: the behaviour is `unsit ` and the param ` force`.
        assert!(reason("@unsit = force").starts_with("its param"));
        assert_eq!(parts("@unsit =force").0, "unsit ");
        // A second at-sign makes another behaviour of it.
        assert_eq!(command("@versionnum=5,@unsit=force".split(',').nth(1).unwrap()).behaviour, "unsit");
        match &parse("@versionnum=5,@unsit=force")[..] {
            [Ok(first), Ok(second)] => assert_eq!((first.behaviour.as_str(), second.behaviour.as_str()), ("versionnum", "@unsit")),
            other => panic!("{other:?}"),
        }
    }

    // ------------------------------------------------------------ dealing

    fn collar() -> Issuer {
        Issuer {
            id: Uuid::from_bytes([7; 16]),
            name: "Collar".to_string(),
            operator: false,
        }
    }

    fn chair() -> Uuid {
        Uuid::from_bytes([9; 16])
    }

    struct Rig {
        commands: Commands,
        grid: FakeGrid,
        follow: bool,
        now: Instant,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                commands: Commands::default(),
                grid: FakeGrid::default(),
                follow: false,
                now: Instant::now(),
            }
        }

        fn deal_from(&mut self, issuer: &Issuer, text: &str) -> Step {
            deal(
                &command(text),
                issuer,
                self.follow,
                &mut self.commands,
                Some(&mut self.grid),
                self.now,
            )
        }

        fn deal(&mut self, text: &str) -> Step {
            self.deal_from(&collar(), text)
        }

        fn calls(&mut self) -> Vec<Call> {
            std::mem::take(&mut self.grid.calls)
        }
    }

    fn failed(step: Step) -> String {
        match step {
            Step::Failed(reason) => reason,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn version_queries_are_answered_on_their_channel() {
        let mut rig = Rig::new();
        let bot = env!("CARGO_PKG_VERSION");
        let old = format!("RestrainedLife viewer v0.1.0 (RusselBot {bot})");
        let new = format!("RestrainedLove viewer v0.1.0 (RusselBot {bot})");
        assert_eq!(rig.deal("@version=2222"), Step::Said(old.clone()));
        assert_eq!(rig.deal("@versionnew=2222"), Step::Said(new.clone()));
        assert_eq!(rig.deal("@versionnum=2222"), Step::Said("10000".to_string()));
        assert_eq!(rig.deal("@versionnum=-873246"), Step::Said("10000".to_string()));
        // An option changes nothing.
        assert_eq!(rig.deal("@versionnum:whatever=7"), Step::Said("10000".to_string()));
        assert_eq!(
            rig.calls(),
            [
                Call::Say(2222, 1, old),
                Call::Say(2222, 1, new),
                Call::Say(2222, 1, "10000".to_string()),
                Call::Say(-873246, 1, "10000".to_string()),
                Call::Say(7, 1, "10000".to_string()),
            ]
        );
        // Answering moves nothing.
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn the_version_number_follows_the_version() {
        let parts: Vec<u32> = VERSION.split('.').map(|part| part.parse().unwrap()).collect();
        assert_eq!(VERSION_NUM, parts[0] * 1_000_000 + parts[1] * 10_000 + parts[2] * 100);
    }

    #[test]
    fn a_query_that_cannot_be_answered_is_refused() {
        let mut rig = Rig::new();
        assert_eq!(failed(rig.deal("@versionnew=0")), "channel 0 cannot be answered on");
        assert_eq!(failed(rig.deal("@versionnew=99999999999")), "the channel is out of range");
        assert_eq!(rig.calls(), []);
        let mut commands = Commands::default();
        let step = deal(
            &command("@versionnum=5"),
            &collar(),
            false,
            &mut commands,
            None::<&mut FakeGrid>,
            rig.now,
        );
        assert_eq!(failed(step), NOT_CONNECTED);
    }

    #[test]
    fn what_the_bot_does_not_have_is_not_supported() {
        let mut rig = Rig::new();
        for text in [
            "@detach=n",
            "@sit=y",
            "@fly=add",
            "@unsit=rem",
            "@clear=force",
            "@clear",
            "@clear,detachme=force".split(',').next().unwrap(),
            "@notify:5;detach=add",
            "@getstatus=2222",
            "@getsitid=2222",
            "@versionnumbl=2222",
            "@getblacklist=2222",
            "@version=force",
            "@versionnew=n",
            "@attach:Outfits/Beach=force",
            "@setrot:1.5=force",
            "@unsit=5",
            "@frobnicate=force",
            // Everything, and what names a folder.
            "@detach=force",
            "@remattach=force",
            "@remoutfit=force",
            "@detach:Outfits/Beach=force",
            "@detach:5=force",
        ] {
            assert_eq!(failed(rig.deal(text)), "not supported", "{text}");
        }
        assert_eq!(rig.calls(), []);
        assert!(rig.grid.jobs.is_empty());
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn forced_sits_and_stands_go_through_the_commands() {
        let mut rig = Rig::new();
        let sit = format!("@sit:{}=force", chair());
        assert_eq!(rig.deal(&sit), Step::Moved(Movement::Sit));
        assert_eq!(rig.calls(), [Call::Sit(chair())]);
        assert!(rig.commands.take_manual());
        assert_eq!(rig.deal("@sitground=force"), Step::Moved(Movement::Ground));
        assert_eq!(rig.deal("@unsit=force"), Step::Moved(Movement::Stand));
        // An option on these two changes nothing.
        assert_eq!(rig.deal("@unsit:now=force"), Step::Moved(Movement::Stand));
        assert_eq!(rig.calls(), [Call::SitGround, Call::Stand, Call::Stand]);
        assert!(rig.commands.take_manual());
        // None of it is the operator's last command.
        assert_eq!(rig.commands.last_sit(), None);
    }

    #[test]
    fn a_forced_sit_needs_a_uuid_and_a_region() {
        let mut rig = Rig::new();
        assert_eq!(failed(rig.deal("@sit:chair=force")), "\"chair\" is not an object's UUID");
        assert_eq!(failed(rig.deal("@sit=force")), "\"\" is not an object's UUID");
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal(&format!("@sit:{}=force", chair()))), NOT_CONNECTED);
        assert_eq!(failed(rig.deal("@unsit=force")), NOT_CONNECTED);
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn a_followed_sit_is_left_to_the_bot() {
        let mut rig = Rig::new();
        rig.follow = true;
        assert_eq!(rig.deal(&format!("@sit:{}=force", chair())), Step::HoldSeat(chair()));
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        assert_eq!(failed(rig.deal("@sit:chair=force")), "\"chair\" is not an object's UUID");
        // The others are carried out all the same.
        assert_eq!(rig.deal("@unsit=force"), Step::Moved(Movement::Stand));
        assert_eq!(rig.calls(), [Call::Stand]);
    }

    #[test]
    fn a_forced_sit_takes_the_place_of_a_waiting_one() {
        let mut rig = Rig::new();
        let (reply, answer) = mpsc::channel();
        let other = Uuid::from_bytes([8; 16]);
        rig.commands
            .handle(Request::Sit(other.to_string()), reply, Some(&mut rig.grid), rig.now);
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(other)));
        assert_eq!(rig.deal(&format!("@sit:{}=force", chair())), Step::Moved(Movement::Sit));
        assert_eq!(
            answer.try_recv(),
            Ok(Response::Error("superseded by a newer sit command".to_string()))
        );
        // The operator's sit stays what `last_cmd` stands for.
        assert_eq!(rig.commands.last_sit(), Some(SitState::Object(other)));
        // The simulator's verdict on the forced one goes to nobody.
        assert!(rig.commands.sit_failed("no such object"));
    }

    #[test]
    fn a_forced_teleport_names_a_region_or_a_point_of_the_grid() {
        let mut rig = Rig::new();
        assert_eq!(rig.deal("@tpto:Blue Lagoon/128/100/25=force"), Step::Moved(Movement::Teleport));
        assert_eq!(rig.deal("@tpto:Home/128/100.5/25;1.57=force"), Step::Moved(Movement::Teleport));
        // A region may be called by a number.
        assert_eq!(rig.deal("@tpto:42/1/2/3=force"), Step::Moved(Movement::Teleport));
        assert_eq!(
            rig.calls(),
            [
                Call::Teleport("Blue Lagoon".to_string(), [128.0, 100.0, 25.0]),
                Call::Teleport("Home".to_string(), [128.0, 100.5, 25.0]),
                Call::Teleport("42".to_string(), [1.0, 2.0, 3.0]),
            ]
        );
        assert!(rig.commands.take_manual());
        // Region 1000, 1000 has its corner at 256000, 256000.
        assert_eq!(rig.deal("@tpto:256128/256100/25=force"), Step::Moved(Movement::Teleport));
        assert_eq!(rig.deal("@tpto:256511.5/512000/30;0=force"), Step::Moved(Movement::Teleport));
        // One of the two below 256 is still a region of some grid.
        assert_eq!(rig.deal("@tpto:300/10/20=force"), Step::Moved(Movement::Teleport));
        let handle = |x: u64, y: u64| (x << 32) | y;
        assert_eq!(
            rig.calls(),
            [
                Call::TeleportTo(handle(256000, 256000), [128.0, 100.0, 25.0]),
                Call::TeleportTo(handle(256256, 512000), [255.5, 0.0, 30.0]),
                Call::TeleportTo(handle(256, 0), [44.0, 10.0, 20.0]),
            ]
        );
        assert!(rig.commands.take_manual());
        // Where RLV went is not where the operator's last `tp` went.
        assert_eq!(rig.commands.last_tp(), None);
        // The outcome is nobody's to read, but it is the commands' teleport.
        assert!(rig.commands.teleport_failed("no such region"));
    }

    #[test]
    fn a_malformed_destination_is_refused() {
        let mut rig = Rig::new();
        for (text, part) in [
            ("@tpto:north/128=force", "neither"),
            ("@tpto:128=force", "neither"),
            ("@tpto=force", "neither"),
            ("@tpto:Home/1/2/3/4=force", "neither"),
            ("@tpto:Home/128/-5/25=force", "`y`"),
            ("@tpto:Home/128/5/up=force", "`z`"),
            ("@tpto:Home/inf/5/25=force", "`x`"),
            ("@tpto:/128/5/25=force", "no region"),
            ("@tpto:-1/2/3=force", "`X`"),
            ("@tpto:1/nan/3=force", "`Y`"),
            ("@tpto:1e12/2/3=force", "outside any grid"),
            // Coordinates inside a region, given without its name.
            ("@tpto:219/102/611=force", "`region/x/y/z`"),
            ("@tpto:0/0/0=force", "region at 0, 0"),
            ("@tpto:255.9/255.9/4000;1.57=force", "region at 0, 0"),
        ] {
            let reason = failed(rig.deal(text));
            assert!(reason.contains(part), "{text}: {reason}");
        }
        assert_eq!(rig.calls(), []);
        assert!(!rig.commands.take_manual());
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal("@tpto:256/2/3=force")), NOT_CONNECTED);
        assert_eq!(failed(rig.deal("@tpto:Home/1/2/3=force")), NOT_CONNECTED);
    }

    #[test]
    fn rlv_names_for_attachment_points_are_read() {
        for (name, number) in RLV_POINTS {
            assert_eq!(point(name), Some(number), "{name}");
            assert_eq!(point(&name.to_uppercase()), Some(number), "{name}");
        }
        // They are the bot's HUD points and its `Avatar Center`.
        for (name, number) in RLV_POINTS {
            let own = inventory::point_name(number);
            let expected = if name == "root" {
                "Avatar Center".to_string()
            } else {
                format!("HUD {name}")
            };
            assert!(own.eq_ignore_ascii_case(&expected), "{name}: {own}");
        }
        assert_eq!(point("left hand"), Some(5));
        assert_eq!(point("Chest"), Some(1));
        assert_eq!(point("HUD Center"), Some(35));
        assert_eq!(point("avatar center"), Some(40));
        assert_eq!(point("5"), None);
        assert_eq!(point("Outfits/Beach"), None);
        assert_eq!(point(""), None);
    }

    #[test]
    fn a_forced_detach_takes_everything_off_a_point() {
        let mut rig = Rig::new();
        let (watch, meter) = (Uuid::from_bytes([3; 16]), Uuid::from_bytes([4; 16]));
        rig.grid.attached = vec![(watch, 5), (meter, 35)];
        assert_eq!(rig.deal("@detach:left hand=force"), Step::Done);
        assert_eq!(rig.deal("@remattach:center=force"), Step::Done);
        // The inventory is asked for the items' names first, as for `detach`.
        let asked: Vec<&What> = rig.grid.jobs.iter().map(|(_, _, what)| what).collect();
        assert_eq!(asked, [&What::Items(vec![watch]), &What::Items(vec![meter])]);
        // Nothing on the point: the reason `detach` gives.
        assert_eq!(failed(rig.deal("@detach:chest=force")), "nothing is attached to Chest");
        assert_eq!(rig.grid.jobs.len(), 2);
        // Taking things off moves nothing.
        assert!(!rig.commands.take_manual());
    }

    use crate::inventory::{Clothes, What};

    #[test]
    fn a_forced_detach_takes_an_attachment_off_by_its_uuid() {
        let mut rig = Rig::new();
        let (object, item) = (Uuid::from_bytes([3; 16]), Uuid::from_bytes([4; 16]));
        rig.grid.worn = vec![(object, item)];
        let found = |rig: &Rig| match rig.grid.jobs.last() {
            Some((_, _, What::Find(inventory::Ref::Id(id)))) => *id,
            other => panic!("expected a lookup of an item, got {other:?}"),
        };
        // By the attachment's own UUID, which is what a script has: the item
        // it was made from is detached, as `detach <item>` does it.
        assert_eq!(rig.deal(&format!("@remattach:{object}=force")), Step::Done);
        assert_eq!(found(&rig), item);
        assert_eq!(
            rig.deal(&format!("@detach:{}=force", object.to_string().to_uppercase())),
            Step::Done
        );
        assert_eq!(found(&rig), item);
        // A UUID that is no attachment's object is taken for the item.
        assert_eq!(rig.deal(&format!("@remattach:{item}=force")), Step::Done);
        assert_eq!(found(&rig), item);
        let other = Uuid::from_bytes([5; 16]);
        assert_eq!(rig.deal(&format!("@remattach:{other}=force")), Step::Done);
        assert_eq!(found(&rig), other);
        assert_eq!(rig.grid.jobs.len(), 4);
        // The operator has UUIDs of items only, and may give one.
        let operator = Issuer::operator(collar().id);
        assert_eq!(rig.deal_from(&operator, &format!("@remattach:{item}=force")), Step::Done);
        assert!(!rig.commands.take_manual());
        rig.grid.offline = true;
        assert_eq!(failed(rig.deal(&format!("@remattach:{object}=force"))), NOT_CONNECTED);
    }

    #[test]
    fn a_forced_remoutfit_takes_a_kind_of_clothing_off() {
        let mut rig = Rig::new();
        // Without the grid's outfit service clothing cannot be changed.
        assert_eq!(failed(rig.deal("@remoutfit:shirt=force")), inventory::NO_OUTFIT_SERVICE);
        rig.grid.outfit = true;
        assert_eq!(rig.deal("@remoutfit:shirt=force"), Step::Done);
        let shirt = inventory::parse_kind("shirt").unwrap();
        assert!(matches!(&rig.grid.jobs[..], [(_, _, What::TakeOff(Clothes::Kind(kind)))] if *kind == shirt));
        // A body part stays, for the reason `unwear` gives.
        let reason = failed(rig.deal("@remoutfit:skin=force"));
        assert!(reason.contains("skin"), "{reason}");
        assert_eq!(failed(rig.deal("@remoutfit:hat=force")), "\"hat\" is not a kind of clothing");
        assert_eq!(rig.grid.jobs.len(), 1);
        assert!(!rig.commands.take_manual());
    }

    #[test]
    fn an_attachment_detaches_itself() {
        let mut rig = Rig::new();
        let item = Uuid::from_bytes([5; 16]);
        rig.grid.worn = vec![(collar().id, item)];
        assert_eq!(rig.deal("@detachme=force"), Step::Done);
        assert!(matches!(&rig.grid.jobs[..], [(_, _, What::Find(found))] if *found == inventory::Ref::Id(item)));
        // An object that is not worn, and the operator, have nothing to detach.
        let rezzed = Issuer {
            id: Uuid::from_bytes([6; 16]),
            ..collar()
        };
        assert_eq!(failed(rig.deal_from(&rezzed, "@detachme=force")), "the sender is not attached");
        let operator = Issuer::operator(collar().id);
        assert_eq!(failed(rig.deal_from(&operator, "@detachme=force")), "the sender is not attached");
        assert_eq!(rig.grid.jobs.len(), 1);
    }

    // -------------------------------------------------------------- senders

    fn heard(kind: &str, source: Source, text: &str) -> Message {
        Message {
            kind: kind.to_string(),
            source,
            sent_at: None,
            text: text.to_string(),
        }
    }

    fn object() -> Source {
        Source::Object {
            id: Some(collar().id),
            name: "Collar".to_string(),
            owner: Uuid::from_bytes([1; 16]),
        }
    }

    #[test]
    fn only_what_an_object_says_to_its_owner_carries_commands() {
        assert_eq!(sender(&heard("owner", object(), "@unsit=force")), Some(collar()));
        // Not what it says for all to hear, and not an ordinary message.
        assert_eq!(sender(&heard("say", object(), "@unsit=force")), None);
        assert_eq!(sender(&heard("owner", object(), "Battery low")), None);
        assert_eq!(sender(&heard("owner", object(), " @unsit=force")), None);
        // Not an avatar or the grid, whatever kind the message claims.
        let avatar = Source::Avatar {
            id: Uuid::from_bytes([2; 16]),
            name: "Jane Doe".to_string(),
        };
        assert_eq!(sender(&heard("say", avatar.clone(), "@unsit=force")), None);
        assert_eq!(sender(&heard("owner", avatar.clone(), "@unsit=force")), None);
        assert_eq!(sender(&heard("im", avatar, "@version")), None);
        let grid = Source::System { name: "Grid".to_string() };
        assert_eq!(sender(&heard("owner", grid, "@unsit=force")), None);
        assert_eq!(collar().to_string(), format!("Collar <{}>", collar().id));
        assert_eq!(Issuer::operator(collar().id).to_string(), "operator");
    }

    // ---------------------------------------------------------------- state

    #[test]
    fn the_mode_and_the_persist_setting_are_switched_and_shown() {
        let mut rlv = Rlv::new(false, RlvPersist::Drop);
        assert!(!rlv.is_on() && !rlv.follows());
        assert_eq!(rlv.status(), "rlv: off\npersist: drop");
        assert_eq!(rlv.set_on(true), "rlv: on");
        assert_eq!(rlv.set_on(true), "rlv: on");
        assert_eq!(rlv.set_persist(RlvPersist::Follow), "rlv persist: follow");
        assert_eq!(rlv.set_persist(RlvPersist::Follow), "rlv persist: follow");
        assert!(rlv.is_on() && rlv.follows());
        assert_eq!(rlv.status(), "rlv: on\npersist: follow");
        assert_eq!(rlv.set_on(false), "rlv: off");
        assert_eq!(rlv.set_persist(RlvPersist::Drop), "rlv persist: drop");
        assert_eq!(rlv.status(), "rlv: off\npersist: drop");
        let rlv = Rlv::new(true, RlvPersist::Follow);
        assert_eq!(rlv.status(), "rlv: on\npersist: follow");
    }

    #[test]
    fn waiting_messages_keep_their_order_up_to_the_limit() {
        let mut rlv = Rlv::new(true, RlvPersist::Drop);
        assert!(!rlv.has_waiting());
        for number in 0..MAX_WAITING + 1 {
            rlv.keep(collar(), format!("@versionnum={}", number + 1));
        }
        // The 65th was dropped.
        let mut texts = Vec::new();
        while let Some((issuer, text)) = rlv.next_waiting() {
            assert_eq!(issuer, collar());
            texts.push(text);
        }
        assert_eq!(texts.len(), MAX_WAITING);
        assert_eq!(texts[0], "@versionnum=1");
        assert_eq!(texts[MAX_WAITING - 1], format!("@versionnum={MAX_WAITING}"));
        // Switching off forgets what waits; so does being told to.
        rlv.keep(collar(), "@unsit=force".to_string());
        rlv.set_on(false);
        assert!(!rlv.has_waiting());
        rlv.keep(collar(), "@unsit=force".to_string());
        rlv.forget_waiting();
        assert!(!rlv.has_waiting());
    }

    // ------------------------------------------------------------ following

    #[test]
    fn dropped_movements_are_followed_by_nothing() {
        let mut follow = Follow::default();
        for movement in [Movement::Sit, Movement::Ground, Movement::Stand, Movement::Teleport] {
            assert_eq!(follow.moved(movement, false, false), After::Nothing);
        }
        assert_eq!(follow.arrived(), After::Nothing);
        assert_eq!(follow.failed(), After::Nothing);
    }

    #[test]
    fn a_followed_stand_and_ground_sit_are_persisted_at_once() {
        let mut follow = Follow::default();
        assert_eq!(follow.moved(Movement::Stand, true, false), After::Persist(SitState::Stand));
        assert_eq!(follow.moved(Movement::Ground, true, true), After::Persist(SitState::Ground));
        // Nothing is left under way.
        assert_eq!(follow.arrived(), After::Nothing);
        assert_eq!(follow.failed(), After::Nothing);
    }

    #[test]
    fn a_followed_teleport_is_persisted_on_arrival() {
        let mut follow = Follow::default();
        assert_eq!(follow.moved(Movement::Teleport, true, false), After::Nothing);
        assert_eq!(follow.arrived(), After::Persist(SitState::Stand));
        // Once: a later arrival is not this teleport's.
        assert_eq!(follow.arrived(), After::Nothing);
    }

    #[test]
    fn a_followed_teleport_that_fails_gives_the_earlier_mode_back() {
        for was_manual in [false, true] {
            let mut follow = Follow::default();
            follow.moved(Movement::Teleport, true, was_manual);
            assert_eq!(follow.failed(), After::Restore { manual: was_manual });
            assert_eq!(follow.failed(), After::Nothing);
        }
    }

    #[test]
    fn a_followed_teleport_is_given_up_for_what_comes_next() {
        // The operator steps in.
        let mut follow = Follow::default();
        follow.moved(Movement::Teleport, true, false);
        follow.give_up();
        assert_eq!(follow.arrived(), After::Nothing);
        // A forced stand during it is persisted by itself.
        follow.moved(Movement::Teleport, true, false);
        assert_eq!(follow.moved(Movement::Stand, true, true), After::Persist(SitState::Stand));
        assert_eq!(follow.arrived(), After::Nothing);
        // A second teleport keeps the mode from before the first: the bot
        // was holding then, and is in manual mode only because of the first.
        follow.moved(Movement::Teleport, true, false);
        follow.moved(Movement::Teleport, true, true);
        assert_eq!(follow.failed(), After::Restore { manual: false });
        // A movement that is dropped ends a teleport that was followed.
        follow.moved(Movement::Teleport, true, false);
        assert_eq!(follow.moved(Movement::Teleport, false, true), After::Nothing);
        assert_eq!(follow.arrived(), After::Nothing);
    }
}
