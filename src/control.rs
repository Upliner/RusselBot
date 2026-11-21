//! The control socket: a local socket on which a running bot takes one
//! command per connection, and the `ctl` subcommand that sends one. The
//! socket threads only pass requests to the main loop and write its answers.

use crate::escape::{escape, unescape};
use crate::net::Event;
use std::io::{self, ErrorKind, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Names the socket address for both the bot and `ctl`.
pub const ADDRESS_VARIABLE: &str = "RUSSELBOT_SOCK";
/// The socket file's name in the home directory.
#[cfg(unix)]
const DEFAULT_FILE: &str = "russelbot-ctl";
#[cfg(not(unix))]
const DEFAULT_TCP_ADDRESS: &str = "127.0.0.1:47115";

/// The longest request line, with its line ending.
const MAX_LINE: usize = 8192;
/// How long a client has to send its line.
const READ_LIMIT: Duration = Duration::from_secs(5);
/// How long a connection waits for the main loop's answer. A teleport may
/// take a minute to end one way or the other.
const ANSWER_LIMIT: Duration = Duration::from_secs(75);
/// How long `ctl` waits for the bot's answer.
const CLIENT_LIMIT: Duration = Duration::from_secs(90);
const MAX_CONNECTIONS: usize = 16;
/// How long a client turned away for that has to send its line.
const BUSY_READ_LIMIT: Duration = Duration::from_secs(1);

pub const CTL_USAGE: &str = "\
Usage: rslbot ctl say MESSAGE...
       rslbot ctl shout MESSAGE...
       rslbot ctl whisper MESSAGE...
       rslbot ctl im AVATAR MESSAGE...
       rslbot ctl stand | unsit
       rslbot ctl sit UUID | ground
       rslbot ctl tp X Y [Z] [REGION...]
       rslbot ctl requesttp AVATAR [MESSAGE...]
       rslbot ctl accept KIND [WHO]
       rslbot ctl accepttp [AVATAR]
       rslbot ctl sendtp AVATAR [MESSAGE...]
       rslbot ctl touch UUID
       rslbot ctl scan [DISTANCE] [FILTER...]
       rslbot ctl inventory [-a] [-n] [FOLDER]
       rslbot ctl attach ITEM [BODY PART...]
       rslbot ctl wear ITEM
       rslbot ctl detach | unwear ITEM | BODY PART... | KIND
       rslbot ctl anim play | stop ITEM
       rslbot ctl sound ITEM
       rslbot ctl persist [LOCATION] [SIT]
       rslbot ctl home
       rslbot ctl status
       rslbot ctl perms autoaccept KIND all | WHO...
       rslbot ctl perms revoke KIND all | WHO...
       rslbot ctl perms status
       rslbot ctl rlv mode on | off | override
       rslbot ctl rlv persist follow | drop
       rslbot ctl rlv persist override none | sit | tp | sit tp
       rslbot ctl rlv status
       rslbot ctl rlv reset
       rslbot ctl rlv cmd COMMAND...
       rslbot ctl autoreply message TEXT...
       rslbot ctl autoreply pause DURATION
       rslbot ctl autoreply refresh | on | off | status
       rslbot ctl quit

Sends a command to a running bot that has `socket = true` in its config.
`say` speaks in local chat; MESSAGE is read as a viewer's chat bar reads it,
so `/1 hello` goes to channel 1. `im` sends an instant message. AVATAR is
one argument: \"First Last\", first.last, a single word or a UUID.

`stand`, `sit`, `tp`, `requesttp` and `accept tp` move the avatar, and the
bot then leaves it where it is until `persist` or `home`. `tp` without
REGION stays in the current region, and without Z keeps the current height.
`requesttp` asks AVATAR to teleport the bot to them; `sendtp` offers AVATAR
a teleport to the bot. `touch` clicks an object. `status` shows where the
avatar is and what it sits on; `quit` logs out and stops the bot.

`scan` lists the objects within DISTANCE metres of the avatar, nearest
first: one line each with the UUID, the distance, flags, the owner's UUID
and name, the name and the description, separated by tabs, with `?` for
what the simulator has not told. Without DISTANCE it lists as far as the
bot looks for objects; a larger one makes it look that far until the avatar
leaves the region. FILTERs: `-n TEXT` (`--name`) and `-d TEXT` (`--desc`)
for a part of the name or the description; `-o WHO` (`--owner`) for the
objects of an AVATAR or of a group given by its UUID; `-s` (`--scripted`),
`-t` (`--touch`), `-p` (`--physical`), `--temporary` and `--phantom` for
prims the simulator flags so; `-a` (`--attachments`) to list what avatars
wear as well, and `-c` (`--children`) the child prims of linksets.

`accept` accepts a request that waits because the bot did not accept it by
itself. KIND is what was asked: `tp` (a teleport offer), `inventory` (an
inventory offer), or what a script asked permission for: `anim`, `attach`,
`controls`, `camera`. Without WHO the request received last is accepted,
with WHO the last one from that avatar, that object, or an object of that
avatar. WHO is one argument, an avatar as above or an object's UUID. A
script that asked for several permissions is granted all of them.
`accepttp [AVATAR]` is the older name of `accept tp [AVATAR]` and is
deprecated.

`perms autoaccept` and `perms revoke` change which requests of a KIND the
bot accepts by itself: `all`, or the avatars and objects named. An avatar
stands for its objects too. While everyone is accepted, revoking a WHO
makes it an exception. `perms status` prints what is accepted for each
kind, and the requests that wait. These changes are not saved; the config's
startup script (`startup`) is where the rules a bot starts with are written.

`persist` tells the bot what to hold from now on: it returns to LOCATION
when the avatar ends up in another region, and keeps it seated as SIT says.
LOCATION is REGION/X/Y/Z as one argument, such as \"Blue Lagoon/128/128/25\".
SIT is an object's UUID, `ground`, `stand` (stand up) or `unchanged` (leave
sitting and standing alone). Either can also be `cur` (where the avatar is
and what it sits on now; the default), `cfg` (what the config says), `last`
(what was last persisted) or `last_cmd` (where the last `tp` went, what the
last `sit` named). `home` is `persist last last`.

`inventory` lists what is under FOLDER, or all of My Inventory: one line
per entry with its UUID, type, attach or wear point, `attached` or `-`, and
path, separated by tabs. `-n` (`--no-recurse`) lists the folder's direct
children only, `-a` (`--attached`) only what is attached to the avatar now.
ITEM and FOLDER are one argument: a UUID as `inventory` prints it, or a
path from the top of My Inventory such as \"Objects/Bot gear/Radio\". In a
path, `\\/` is a `/` inside a name and `\\\\` a backslash; a `/` in front makes
a path of what would otherwise be read as an option, a UUID or a body part.

`attach` attaches an object to BODY PART (`Left Hand`, `HUD Center`, or a
number), or without one to the point it was last on. `wear` does the same
for an object, and puts on clothing or a body part (shape, skin, hair,
eyes) where the grid makes the avatar's appearance (Second Life). `detach`
takes off an item, everything on a body part, or all clothing of a KIND
(`shirt`, `pants`, ...). `anim play` and `anim stop` start and stop an
animation from the inventory, and `sound` plays a sound from it once. None
of these moves the avatar.

`rlv mode` switches RLV on and off: while it is on, the bot reads the `@`
commands that objects the avatar owns say to their owner, answers their
queries, carries out forced sits, stands, teleports and detaches, and holds
itself and these commands to the restrictions the objects set. `rlv mode
override` lets the next command through whatever the restrictions say, and
RLV is on again after it. `rlv mode off` keeps the restrictions and applies
none; `rlv reset` clears them all. `rlv persist` says what a forced movement
does to the place the bot holds: `drop` makes it stop holding, `follow`
makes the result the place it holds. `rlv persist override` names what the
bot does by itself whatever the restrictions say: `tp` its teleports to the
place it holds, `sit` its sits there, `none` neither. `rlv status` prints
all of this and the restrictions. `rlv cmd` carries out RLV commands as if
an object had sent them, such as `rlv cmd @versionnum=5` or
`rlv cmd @tpto:Blue Lagoon/128/128/25=force`, and prints what became of
each; its restrictions are its own, apart from every object's, and it is
refused while RLV is off. None of the switches is saved; the config's startup
script sets them at every start.

`autoreply message` sets the text the bot answers an avatar's instant
message with, and switches autoreply on. After answering an avatar, or
after `im` to it, the bot does not answer it again for the pause, which
`autoreply pause` sets: `90s`, `30m`, `2h`, `1h30m`, or `0` to answer every
message. `autoreply refresh` forgets every avatar's pause; `autoreply on`
and `off` switch it on with the text it has and off; `autoreply status`
prints all of this. None of it is saved; the config's startup script sets
it at every start.

The same commands, without `rslbot ctl` in front, are the lines of the
config's startup script (`startup`), which the bot carries out when it
starts. The socket address is taken from the environment variable
RUSSELBOT_SOCK when it is set.";

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Local chat, still to be read the way a chat bar reads it.
    Say(String),
    /// Local chat, shouted as it is.
    Shout(String),
    /// Local chat, whispered as it is.
    Whisper(String),
    Im {
        to: String,
        text: String,
    },
    Stand,
    /// A UUID or `ground`, still to be read.
    Sit(String),
    /// `x y [z] [region words]`, still to be read.
    Tp(Vec<String>),
    AskTp {
        to: String,
        message: String,
    },
    /// `accept KIND [WHO]`, both still to be read. `accepttp [AVATAR]` is
    /// `accept tp [AVATAR]`.
    Accept {
        kind: String,
        who: Option<String>,
    },
    SendTp {
        to: String,
        message: String,
    },
    /// A UUID, still to be read.
    Touch(String),
    /// `[DISTANCE] [FILTER...]`, still to be read.
    Scan(Vec<String>),
    /// `[-a] [-n] [folder]`, still to be read.
    Inventory(Vec<String>),
    /// An inventory item and, when given, the words of a body part.
    Attach {
        item: String,
        part: Option<String>,
    },
    /// An inventory item, still to be read.
    Wear(String),
    /// An inventory item, a body part or a kind of clothing, still to be
    /// read. `unwear` is the same request.
    Detach(String),
    /// `anim play` or `anim stop` and an inventory item.
    Anim {
        stop: bool,
        item: String,
    },
    /// An inventory item, still to be read.
    Sound(String),
    /// A location and a sit state, still to be read; what is left out
    /// stands for `cur`.
    Persist {
        location: Option<String>,
        sit: Option<String>,
    },
    Home,
    Status,
    Rlv(RlvRequest),
    Perms(PermsRequest),
    Autoreply(AutoreplyRequest),
    Quit,
}

impl Request {
    /// The command as the operator log names it.
    pub fn name(&self) -> &'static str {
        match self {
            Request::Say(_) => "say",
            Request::Shout(_) => "shout",
            Request::Whisper(_) => "whisper",
            Request::Im { .. } => "im",
            Request::Stand => "stand",
            Request::Sit(_) => "sit",
            Request::Tp(_) => "tp",
            Request::AskTp { .. } => "requesttp",
            Request::Accept { .. } => "accept",
            Request::Perms(_) => "perms",
            Request::Autoreply(_) => "autoreply",
            Request::SendTp { .. } => "sendtp",
            Request::Touch(_) => "touch",
            Request::Scan(_) => "scan",
            Request::Persist { .. } => "persist",
            Request::Home => "home",
            Request::Status => "status",
            Request::Rlv(_) => "rlv",
            Request::Quit => "quit",
            Request::Inventory(_) => "inventory",
            Request::Attach { .. } => "attach",
            Request::Wear(_) => "wear",
            Request::Detach(_) => "detach",
            Request::Anim { stop: false, .. } => "anim play",
            Request::Anim { stop: true, .. } => "anim stop",
            Request::Sound(_) => "sound",
        }
    }
}

/// The `perms` commands. Kinds and entries are still to be read.
#[derive(Debug, Clone, PartialEq)]
pub enum PermsRequest {
    /// `perms autoaccept KIND all|WHO...`
    Autoaccept {
        kind: String,
        who: Vec<String>,
    },
    /// `perms revoke KIND all|WHO...`
    Revoke {
        kind: String,
        who: Vec<String>,
    },
    Status,
}

/// The `autoreply` commands.
#[derive(Debug, Clone, PartialEq)]
pub enum AutoreplyRequest {
    /// `autoreply message TEXT...`: the words joined with spaces, still to
    /// be checked.
    Message(String),
    /// `autoreply pause DURATION`, still to be read.
    Pause(String),
    Refresh,
    On,
    Off,
    Status,
}

/// The modes of RLV. Override is on, with the operator's next command
/// exempt from the restrictions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RlvMode {
    Off,
    On,
    Override,
}

/// The `rlv` commands.
#[derive(Debug, Clone, PartialEq)]
pub enum RlvRequest {
    /// `rlv mode on`, `rlv mode off` or `rlv mode override`.
    Mode(RlvMode),
    /// `rlv persist follow` (true) or `rlv persist drop`.
    Persist(bool),
    /// `rlv persist override` with `none`, or with `sit`, `tp` or both.
    PersistOverride {
        sit: bool,
        tp: bool,
    },
    Status,
    /// `rlv reset`.
    Reset,
    /// RLV commands as an object would send them, still to be read.
    Cmd(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Ok(Option<String>),
    Error(String),
}

/// One line of tab-separated, escaped fields, with its line ending.
fn encode_line(fields: &[&str]) -> String {
    fields.iter().map(|field| escape(field)).collect::<Vec<_>>().join("\t") + "\n"
}

/// The fields of a line; its line ending is optional.
fn decode_line(line: &str) -> Vec<String> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.split('\t').map(unescape).collect()
}

impl Request {
    /// Reads a request line as a client sent it.
    pub fn decode(line: &[u8]) -> Result<Request, String> {
        let line = std::str::from_utf8(line).map_err(|_| "the command is not valid UTF-8".to_string())?;
        let mut fields = decode_line(line);
        let command = fields.remove(0);
        let given = fields.len();
        let plural = |count: usize| if count == 1 { "" } else { "s" };
        let arguments = |wanted: usize| match given == wanted {
            true => Ok(()),
            false if wanted == 0 => Err(format!("`{command}` takes no arguments")),
            false => Err(format!("`{command}` takes {wanted} argument{}, not {given}", plural(wanted))),
        };
        let at_least = |wanted: usize| match given >= wanted {
            true => Ok(()),
            false => Err(format!(
                "`{command}` takes at least {wanted} argument{}, not {given}",
                plural(wanted)
            )),
        };
        // An avatar, then the words of an optional message.
        let avatar_and_message = |mut fields: Vec<String>| {
            let to = fields.remove(0);
            (to, fields.join(" "))
        };
        match command.as_str() {
            "say" => {
                arguments(1)?;
                Ok(Request::Say(fields.remove(0)))
            }
            "shout" => {
                at_least(1)?;
                Ok(Request::Shout(fields.join(" ")))
            }
            "whisper" => {
                at_least(1)?;
                Ok(Request::Whisper(fields.join(" ")))
            }
            "im" => {
                arguments(2)?;
                let text = fields.remove(1);
                Ok(Request::Im {
                    to: fields.remove(0),
                    text,
                })
            }
            "stand" | "unsit" => arguments(0).map(|()| Request::Stand),
            "sit" => {
                arguments(1)?;
                Ok(Request::Sit(fields.remove(0)))
            }
            "tp" => {
                at_least(2)?;
                Ok(Request::Tp(fields))
            }
            "requesttp" => {
                at_least(1)?;
                let (to, message) = avatar_and_message(fields);
                Ok(Request::AskTp { to, message })
            }
            "accept" => {
                if given == 0 || given > 2 {
                    return Err(
                        "`accept` takes a kind of request and, if wanted, who sent it: `accept KIND [WHO]`; \
                                a name of two words has to be quoted"
                            .to_string(),
                    );
                }
                let mut fields = fields.into_iter();
                Ok(Request::Accept {
                    kind: fields.next().unwrap_or_default(),
                    who: fields.next(),
                })
            }
            // The older name of `accept tp`.
            "accepttp" => {
                if given > 1 {
                    return Err(format!(
                        "`accepttp` takes at most 1 argument, an avatar, not {given}; a name of two words has to be quoted"
                    ));
                }
                Ok(Request::Accept {
                    kind: "tp".to_string(),
                    who: fields.pop(),
                })
            }
            "perms" => {
                let usage = |command: &str| {
                    format!("`perms {command}` takes a kind of request and whom it is about: `perms {command} KIND all|WHO...`")
                };
                let mut fields = fields.into_iter();
                let perms = match fields.next().as_deref() {
                    Some("status") if given == 1 => PermsRequest::Status,
                    Some("status") => return Err("`perms status` takes no arguments".to_string()),
                    Some(command @ ("autoaccept" | "revoke")) => {
                        let (Some(kind), who) = (fields.next(), fields.collect::<Vec<_>>()) else {
                            return Err(usage(command));
                        };
                        if who.is_empty() {
                            return Err(usage(command));
                        }
                        match command {
                            "autoaccept" => PermsRequest::Autoaccept { kind, who },
                            _ => PermsRequest::Revoke { kind, who },
                        }
                    }
                    _ => return Err("`perms` takes `autoaccept KIND all|WHO...`, `revoke KIND all|WHO...` or `status`".to_string()),
                };
                Ok(Request::Perms(perms))
            }
            "sendtp" => {
                at_least(1)?;
                let (to, message) = avatar_and_message(fields);
                Ok(Request::SendTp { to, message })
            }
            "touch" => {
                arguments(1)?;
                Ok(Request::Touch(fields.remove(0)))
            }
            "scan" => Ok(Request::Scan(fields)),
            "inventory" => Ok(Request::Inventory(fields)),
            "attach" => {
                at_least(1)?;
                let item = fields.remove(0);
                // A body part's name may be several words.
                let part = (!fields.is_empty()).then(|| fields.join(" "));
                Ok(Request::Attach { item, part })
            }
            "wear" => {
                arguments(1)?;
                Ok(Request::Wear(fields.remove(0)))
            }
            // An item is one argument; the words of a body part are joined.
            "detach" | "unwear" => {
                at_least(1)?;
                Ok(Request::Detach(fields.join(" ")))
            }
            "anim" => {
                let stop = match (fields.first().map(String::as_str), given) {
                    (Some("play"), 2) => false,
                    (Some("stop"), 2) => true,
                    _ => return Err("`anim` takes `play ITEM` or `stop ITEM`; an item path with spaces has to be quoted".to_string()),
                };
                Ok(Request::Anim {
                    stop,
                    item: fields.remove(1),
                })
            }
            "sound" => {
                arguments(1)?;
                Ok(Request::Sound(fields.remove(0)))
            }
            "persist" => {
                if given > 2 {
                    return Err(format!(
                        "`persist` takes at most 2 arguments, a location and a sit state, not {given}; \
                         a region name of several words has to be quoted"
                    ));
                }
                let mut fields = fields.into_iter();
                Ok(Request::Persist {
                    location: fields.next(),
                    sit: fields.next(),
                })
            }
            "rlv" => {
                let words: Vec<&str> = fields.iter().map(String::as_str).collect();
                let rlv = match words[..] {
                    ["mode", "on"] => RlvRequest::Mode(RlvMode::On),
                    ["mode", "off"] => RlvRequest::Mode(RlvMode::Off),
                    ["mode", "override"] => RlvRequest::Mode(RlvMode::Override),
                    ["mode", ..] => return Err("`rlv mode` takes `on`, `off` or `override`".to_string()),
                    ["persist", "override", "none"] => RlvRequest::PersistOverride { sit: false, tp: false },
                    ["persist", "override", ref sorts @ ..]
                        if !sorts.is_empty() && sorts.iter().all(|sort| matches!(*sort, "sit" | "tp")) =>
                    {
                        RlvRequest::PersistOverride {
                            sit: sorts.contains(&"sit"),
                            tp: sorts.contains(&"tp"),
                        }
                    }
                    ["persist", "override", ..] => {
                        return Err("`rlv persist override` takes `none`, or one or both of `sit` and `tp`".to_string())
                    }
                    ["persist", "follow"] => RlvRequest::Persist(true),
                    ["persist", "drop"] => RlvRequest::Persist(false),
                    ["persist", ..] => return Err("`rlv persist` takes `follow`, `drop` or `override`".to_string()),
                    ["status"] => RlvRequest::Status,
                    ["status", ..] => return Err("`rlv status` takes no arguments".to_string()),
                    ["reset"] => RlvRequest::Reset,
                    ["reset", ..] => return Err("`rlv reset` takes no arguments".to_string()),
                    ["cmd"] => return Err("`rlv cmd` takes an RLV command, such as `@versionnum=5`".to_string()),
                    ["cmd", ..] => RlvRequest::Cmd(words[1..].join(" ")),
                    _ => {
                        return Err(
                            "`rlv` takes `mode on|off|override`, `persist follow|drop`, `persist override none|sit|tp`, \
                                    `status`, `reset` or `cmd COMMAND`"
                                .to_string(),
                        )
                    }
                };
                Ok(Request::Rlv(rlv))
            }
            "autoreply" => {
                const FORMS: &str = "`autoreply` takes `message TEXT...`, `pause DURATION`, `refresh`, `on`, `off` or `status`";
                let mut fields = fields.into_iter();
                let form = fields.next();
                let rest: Vec<String> = fields.collect();
                let bare = |request: AutoreplyRequest, name: &str| match rest.is_empty() {
                    true => Ok(Request::Autoreply(request)),
                    false => Err(format!("`autoreply {name}` takes no arguments")),
                };
                match form.as_deref() {
                    Some("message") if rest.is_empty() => Err("`autoreply message` takes the text to answer with".to_string()),
                    Some("message") => Ok(Request::Autoreply(AutoreplyRequest::Message(rest.join(" ")))),
                    Some("pause") => match <[String; 1]>::try_from(rest) {
                        Ok([duration]) => Ok(Request::Autoreply(AutoreplyRequest::Pause(duration))),
                        Err(_) => Err("`autoreply pause` takes 1 argument, a duration such as `30m` or `2h`".to_string()),
                    },
                    Some("refresh") => bare(AutoreplyRequest::Refresh, "refresh"),
                    Some("on") => bare(AutoreplyRequest::On, "on"),
                    Some("off") => bare(AutoreplyRequest::Off, "off"),
                    Some("status") => bare(AutoreplyRequest::Status, "status"),
                    _ => Err(FORMS.to_string()),
                }
            }
            "home" => arguments(0).map(|()| Request::Home),
            "status" => arguments(0).map(|()| Request::Status),
            "quit" => arguments(0).map(|()| Request::Quit),
            "" => Err("no command given".to_string()),
            other => Err(format!("unknown command `{other}`")),
        }
    }
}

impl Response {
    pub fn encode(&self) -> String {
        match self {
            Response::Ok(None) => encode_line(&["ok"]),
            Response::Ok(Some(text)) => encode_line(&["ok", text]),
            Response::Error(text) => encode_line(&["error", text]),
        }
    }

    pub fn decode(line: &str) -> Result<Response, String> {
        let fields = decode_line(line);
        match fields.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            ["ok"] => Ok(Response::Ok(None)),
            ["ok", text] => Ok(Response::Ok(Some(text.to_string()))),
            ["error", text] => Ok(Response::Error(text.to_string())),
            _ => Err("the bot's answer makes no sense".to_string()),
        }
    }
}

/// The request line for `ctl`'s arguments. The message words of `say` and
/// `im` become one argument; the bot judges everything else.
pub fn request_line(args: &[String]) -> String {
    let mut fields: Vec<String> = Vec::new();
    match args.first().map(String::as_str) {
        Some("say") if args.len() > 1 => fields.extend([args[0].clone(), args[1..].join(" ")]),
        Some("im") if args.len() > 2 => fields.extend([args[0].clone(), args[1].clone(), args[2..].join(" ")]),
        _ => fields.extend(args.iter().cloned()),
    }
    encode_line(&fields.iter().map(String::as_str).collect::<Vec<_>>())
}

/// The longest command line.
pub const MAX_COMMAND: usize = 8192;

/// Splits a command line, as the XMPP control contact and the startup
/// script take it, into arguments at spaces, tabs and line feeds. A
/// part between double quotes keeps its spaces; inside it `\"` is a double
/// quote and `\\` a backslash, and every other backslash stays as it is.
pub fn split(body: &str) -> Result<Vec<String>, String> {
    if body.len() > MAX_COMMAND {
        return Err(format!("the command is {} bytes long; the limit is {MAX_COMMAND}", body.len()));
    }
    let mut args = Vec::new();
    let mut current: Option<String> = None;
    let mut quoted = false;
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.get_or_insert_with(String::new);
            }
            '\\' if quoted && matches!(chars.peek(), Some('"' | '\\')) => {
                current.get_or_insert_with(String::new).push(chars.next().expect("peeked"));
            }
            ' ' | '\t' | '\n' | '\r' if !quoted => {
                if let Some(arg) = current.take() {
                    args.push(arg);
                }
            }
            _ => current.get_or_insert_with(String::new).push(ch),
        }
    }
    if quoted {
        return Err("the command ends inside double quotes; close the quote with \"".to_string());
    }
    args.extend(current);
    Ok(args)
}

/// The socket address from the environment: the override if there is one,
/// and the platform's default otherwise.
pub fn address_from(configured: Option<String>, home: Option<String>) -> Result<String, String> {
    if let Some(address) = configured.filter(|address| !address.is_empty()) {
        return Ok(address);
    }
    default_address(home)
}

#[cfg(unix)]
fn default_address(home: Option<String>) -> Result<String, String> {
    match home.filter(|home| !home.is_empty()) {
        Some(home) => Ok(format!("{}/{DEFAULT_FILE}", home.trim_end_matches('/'))),
        None => Err(format!("no home directory to put the socket in; set {ADDRESS_VARIABLE}")),
    }
}

#[cfg(not(unix))]
fn default_address(_home: Option<String>) -> Result<String, String> {
    Ok(DEFAULT_TCP_ADDRESS.to_string())
}

pub fn address() -> Result<String, String> {
    address_from(std::env::var(ADDRESS_VARIABLE).ok(), std::env::var("HOME").ok())
}

#[cfg(unix)]
mod sys {
    //! A Unix domain socket; the address is its file's path.
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

    pub fn bind(address: &str) -> io::Result<Listener> {
        match fs::symlink_metadata(address) {
            Ok(found) if found.file_type().is_socket() => {
                if Stream::connect(address).is_ok() {
                    return Err(io::Error::new(ErrorKind::AddrInUse, "another process is listening on it"));
                }
                // Left behind by a bot that is gone.
                fs::remove_file(address)?;
            }
            Ok(_) => return Err(io::Error::new(ErrorKind::AlreadyExists, "the file exists and is not a socket")),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = Listener::bind(address)?;
        fs::set_permissions(address, fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub fn connect(address: &str) -> io::Result<Stream> {
        Stream::connect(address)
    }

    pub fn remove(address: &str) {
        let _ = fs::remove_file(address);
    }
}

#[cfg(not(unix))]
mod sys {
    //! A TCP socket; the address is `host:port`.
    use std::io;
    pub use std::net::{TcpListener as Listener, TcpStream as Stream};

    pub fn bind(address: &str) -> io::Result<Listener> {
        Listener::bind(address)
    }

    pub fn connect(address: &str) -> io::Result<Stream> {
        Stream::connect(address)
    }

    pub fn remove(_address: &str) {}
}

/// The listening socket. Dropping it removes the socket file; the listener
/// thread lives until the process exits.
pub struct Server {
    address: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        sys::remove(&self.address);
    }
}

/// Starts listening on `address`. Every request arrives on `events` as an
/// `Event::Control`, to be answered on the sender it carries.
pub fn listen(address: &str, events: Sender<Event>) -> Result<Server, String> {
    let listener = sys::bind(address).map_err(|e| format!("cannot listen on {address}: {e}"))?;
    let connections = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    crate::info!("control socket: cannot accept a connection: {e}");
                    // Whatever is wrong, do not spin on it.
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            if connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                connections.fetch_sub(1, Ordering::SeqCst);
                // Hear the client out, briefly, so that the answer reaches it.
                read_line(&mut stream, BUSY_READ_LIMIT);
                let busy = Response::Error("too many control connections; try again".to_string());
                let _ = stream.write_all(busy.encode().as_bytes());
                continue;
            }
            let (events, connections) = (events.clone(), connections.clone());
            thread::spawn(move || {
                serve(stream, &events);
                connections.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(Server {
        address: address.to_string(),
    })
}

/// What became of reading a client's line.
enum Line {
    Complete(Vec<u8>),
    TooLong,
    /// The client hung up, went quiet or the read failed.
    Missing,
}

fn read_line(stream: &mut sys::Stream, limit: Duration) -> Line {
    let deadline = Instant::now() + limit;
    let mut line = Vec::new();
    let mut too_long = false;
    let mut chunk = [0; 1024];
    loop {
        if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
            line.truncate(end);
            return if too_long { Line::TooLong } else { Line::Complete(line) };
        }
        if line.len() >= MAX_LINE {
            // Read the rest of it all the same, and forget it: an answer
            // sent while the client is still writing would not reach it.
            too_long = true;
            line.clear();
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return Line::Missing;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Line::Missing,
            Ok(count) => line.extend(&chunk[..count]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Line::Missing,
        }
    }
}

/// Handles one connection: one request in, one answer out.
fn serve(mut stream: sys::Stream, events: &Sender<Event>) {
    let request = match read_line(&mut stream, READ_LIMIT) {
        Line::Complete(line) => Request::decode(&line),
        Line::TooLong => Err(format!("the command is longer than {MAX_LINE} bytes")),
        Line::Missing => return,
    };
    let response = match request {
        Ok(request) => {
            let (reply, answer) = mpsc::channel();
            match events.send(Event::Control { request, reply }) {
                Ok(()) => answer
                    .recv_timeout(ANSWER_LIMIT)
                    .unwrap_or_else(|_| Response::Error("the bot did not answer".to_string())),
                Err(_) => Response::Error("the bot is shutting down".to_string()),
            }
        }
        Err(reason) => {
            crate::info!("control: bad command: {reason}");
            Response::Error(reason)
        }
    };
    // The client may be gone already; the command has been dealt with.
    let _ = stream.write_all(response.encode().as_bytes());
}

/// Sends one request line to the bot at `address` and reads its answer.
fn exchange(address: &str, line: &str, limit: Duration) -> Result<Response, String> {
    let failed = |what: &str, e: io::Error| format!("{what} the bot at {address}: {e}");
    let mut stream = sys::connect(address).map_err(|e| failed("cannot reach", e))?;
    stream.write_all(line.as_bytes()).map_err(|e| failed("cannot write to", e))?;
    stream.set_read_timeout(Some(limit)).map_err(|e| failed("cannot read from", e))?;
    let mut answer = String::new();
    match stream.read_to_string(&mut answer) {
        Ok(_) if answer.is_empty() => Err(format!("the bot at {address} closed the connection without answering")),
        Ok(_) => Response::decode(&answer),
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Err(format!("no answer from the bot at {address} within {} s", limit.as_secs()))
        }
        Err(e) => Err(failed("cannot read from", e)),
    }
}

/// The `ctl` subcommand: sends `args` as one command to the bot listening
/// on `address` and reports the answer. Returns the exit status.
pub fn client(args: &[String], address: Result<String, String>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            let _ = writeln!(err, "rslbot: ctl needs a command\n\n{CTL_USAGE}");
            return 2;
        }
        Some("-h" | "--help") => {
            let _ = writeln!(out, "{CTL_USAGE}");
            return 0;
        }
        Some(_) => {}
    }
    let answer = address.and_then(|address| exchange(&address, &request_line(args), CLIENT_LIMIT));
    match answer {
        Ok(Response::Ok(None)) => 0,
        Ok(Response::Ok(Some(text))) => {
            let _ = writeln!(out, "{text}");
            0
        }
        Ok(Response::Error(text)) | Err(text) => {
            let _ = writeln!(err, "rslbot: {text}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn requests_are_read_from_their_lines() {
        assert_eq!(Request::decode(b"say\thello there"), Ok(Request::Say("hello there".into())));
        assert_eq!(
            Request::decode(b"say\t/1 test message\r"),
            Ok(Request::Say("/1 test message".into()))
        );
        assert_eq!(
            Request::decode(b"im\tJane Doe\tare you there?"),
            Ok(Request::Im {
                to: "Jane Doe".into(),
                text: "are you there?".into()
            })
        );
        // Escapes carry what a line cannot.
        assert_eq!(Request::decode(b"say\ta\\tb\\nc\\\\d"), Ok(Request::Say("a\tb\nc\\d".into())));
    }

    #[test]
    fn movement_requests_are_read_from_their_lines() {
        let text = |s: &str| s.to_string();
        assert_eq!(Request::decode(b"stand"), Ok(Request::Stand));
        assert_eq!(Request::decode(b"unsit"), Ok(Request::Stand));
        assert_eq!(Request::decode(b"sit\tground"), Ok(Request::Sit(text("ground"))));
        assert_eq!(
            Request::decode(b"tp\t128\t128\t25\tBlue\tLagoon"),
            Ok(Request::Tp(vec![
                text("128"),
                text("128"),
                text("25"),
                text("Blue"),
                text("Lagoon")
            ]))
        );
        assert_eq!(
            Request::decode(b"requesttp\tJane Doe"),
            Ok(Request::AskTp {
                to: text("Jane Doe"),
                message: String::new()
            })
        );
        assert_eq!(
            Request::decode(b"sendtp\tJane Doe\tcome\tand see"),
            Ok(Request::SendTp {
                to: text("Jane Doe"),
                message: text("come and see")
            })
        );
        assert_eq!(
            Request::decode(b"accepttp\tjane.doe"),
            Ok(Request::Accept {
                kind: text("tp"),
                who: Some(text("jane.doe"))
            })
        );
        assert_eq!(Request::decode(b"touch\tabc"), Ok(Request::Touch(text("abc"))));
        assert_eq!(Request::decode(b"home"), Ok(Request::Home));
        assert_eq!(Request::decode(b"persist"), Ok(Request::Persist { location: None, sit: None }));
        assert_eq!(
            Request::decode(b"persist\tcur"),
            Ok(Request::Persist {
                location: Some(text("cur")),
                sit: None
            })
        );
        assert_eq!(
            Request::decode(b"persist\tBlue Lagoon/128/128/25\tstand"),
            Ok(Request::Persist {
                location: Some(text("Blue Lagoon/128/128/25")),
                sit: Some(text("stand"))
            })
        );
        assert_eq!(Request::decode(b"status"), Ok(Request::Status));
        assert_eq!(Request::decode(b"quit"), Ok(Request::Quit));
        assert_eq!(Request::decode(b"shout\tover\there"), Ok(Request::Shout(text("over here"))));
        assert_eq!(Request::decode(b"whisper\tpsst"), Ok(Request::Whisper(text("psst"))));
    }

    #[test]
    fn movement_requests_with_the_wrong_arguments_are_refused() {
        assert!(Request::decode(b"stand\tup").unwrap_err().contains("`stand` takes no arguments"));
        assert!(Request::decode(b"quit\tnow").unwrap_err().contains("`quit`"));
        assert!(Request::decode(b"sit").unwrap_err().contains("`sit` takes 1 argument, not 0"));
        assert!(Request::decode(b"sit\ta\tb").unwrap_err().contains("`sit`"));
        assert!(Request::decode(b"tp\t128").unwrap_err().contains("at least 2 arguments, not 1"));
        assert!(Request::decode(b"requesttp").unwrap_err().contains("at least 1 argument, not 0"));
        assert!(Request::decode(b"sendtp").unwrap_err().contains("`sendtp`"));
        assert!(Request::decode(b"accepttp\ta\tb").unwrap_err().contains("`accepttp`"));
        assert!(Request::decode(b"touch").unwrap_err().contains("`touch`"));
        assert!(Request::decode(b"shout").unwrap_err().contains("`shout`"));
        assert!(Request::decode(b"home\tnow").unwrap_err().contains("`home` takes no arguments"));
        // An unquoted region name of two words makes three arguments.
        let reason = Request::decode(b"persist\tBlue\tLagoon/128/128/25\tstand").unwrap_err();
        assert!(
            reason.contains("`persist`") && reason.contains("not 3") && reason.contains("quoted"),
            "{reason}"
        );
    }

    #[test]
    fn scan_requests_keep_their_arguments_for_the_bot_to_read() {
        assert_eq!(Request::decode(b"scan"), Ok(Request::Scan(vec![])));
        assert_eq!(
            Request::decode(b"scan	96	-n	park bench	-s"),
            Ok(Request::Scan(strings(&["96", "-n", "park bench", "-s"])))
        );
        assert_eq!(Request::Scan(vec![]).name(), "scan");
        // The client passes options on as they are.
        assert_eq!(
            request_line(&strings(&["scan", "20", "--name", "park bench", "-a"])),
            "scan	20	--name	park bench	-a
"
        );
        assert!(CTL_USAGE.contains("rslbot ctl scan [DISTANCE] [FILTER...]"));
    }

    #[test]
    fn inventory_requests_are_read_from_their_lines() {
        let text = |s: &str| s.to_string();
        assert_eq!(Request::decode(b"inventory"), Ok(Request::Inventory(vec![])));
        assert_eq!(
            Request::decode(b"inventory\t-a\tObjects/Bot gear\t-n"),
            Ok(Request::Inventory(vec![text("-a"), text("Objects/Bot gear"), text("-n")]))
        );
        assert_eq!(
            Request::decode(b"attach\tObjects/Radio"),
            Ok(Request::Attach {
                item: text("Objects/Radio"),
                part: None
            })
        );
        // The words after the item are the body part.
        assert_eq!(
            Request::decode(b"attach\tObjects/Radio\tleft\thand"),
            Ok(Request::Attach {
                item: text("Objects/Radio"),
                part: Some(text("left hand"))
            })
        );
        assert_eq!(
            Request::decode(b"attach\tObjects/Radio\t5"),
            Ok(Request::Attach {
                item: text("Objects/Radio"),
                part: Some(text("5"))
            })
        );
        assert_eq!(
            Request::decode(b"wear\tClothing/Blue shirt"),
            Ok(Request::Wear(text("Clothing/Blue shirt")))
        );
        assert_eq!(
            Request::decode(b"detach\tObjects/Radio"),
            Ok(Request::Detach(text("Objects/Radio")))
        );
        assert_eq!(Request::decode(b"detach\tleft\thand"), Ok(Request::Detach(text("left hand"))));
        // `unwear` is `detach` under another name.
        assert_eq!(Request::decode(b"unwear\tObjects/Radio"), Request::decode(b"detach\tObjects/Radio"));
        assert_eq!(Request::decode(b"unwear\tshirt"), Ok(Request::Detach(text("shirt"))));
        assert_eq!(
            Request::decode(b"anim\tplay\tAnimations/Dance"),
            Ok(Request::Anim {
                stop: false,
                item: text("Animations/Dance")
            })
        );
        assert_eq!(
            Request::decode(b"anim\tstop\tAnimations/Dance"),
            Ok(Request::Anim {
                stop: true,
                item: text("Animations/Dance")
            })
        );
        assert_eq!(Request::decode(b"sound\tSounds/Bell"), Ok(Request::Sound(text("Sounds/Bell"))));
        // A path's escapes arrive as the client wrote them.
        assert_eq!(
            Request::decode(b"sound\tSounds/Ding\\\\/Dong"),
            Ok(Request::Sound(text("Sounds/Ding\\/Dong")))
        );
    }

    #[test]
    fn inventory_requests_with_the_wrong_arguments_are_refused() {
        assert!(Request::decode(b"attach")
            .unwrap_err()
            .contains("`attach` takes at least 1 argument"));
        assert!(Request::decode(b"wear").unwrap_err().contains("`wear` takes 1 argument, not 0"));
        assert!(Request::decode(b"wear\tBlue\tshirt").unwrap_err().contains("`wear`"));
        assert!(Request::decode(b"detach").unwrap_err().contains("`detach`"));
        assert!(Request::decode(b"unwear").unwrap_err().contains("`unwear`"));
        assert!(Request::decode(b"sound").unwrap_err().contains("`sound`"));
        assert!(Request::decode(b"sound\ta\tb").unwrap_err().contains("`sound`"));
        // `anim` is `play` or `stop` and one item, nothing else.
        for bad in [
            &b"anim"[..],
            b"anim\tplay",
            b"anim\tstop",
            b"anim\tpause\tAnimations/Dance",
            b"anim\tplay\tAnimations/Dance\tnow",
            b"anim\tAnimations/Dance\tplay",
            b"anim\tPlay\tAnimations/Dance",
        ] {
            let reason = Request::decode(bad).unwrap_err();
            assert!(reason.contains("`play ITEM`") && reason.contains("`stop ITEM`"), "{reason}");
        }
    }

    #[test]
    fn rlv_requests_are_read_from_their_lines() {
        let read = |line: &str| Request::decode(line.as_bytes());
        assert_eq!(read("rlv\tmode\ton"), Ok(Request::Rlv(RlvRequest::Mode(RlvMode::On))));
        assert_eq!(read("rlv\tmode\toff\n"), Ok(Request::Rlv(RlvRequest::Mode(RlvMode::Off))));
        assert_eq!(read("rlv\tmode\toverride"), Ok(Request::Rlv(RlvRequest::Mode(RlvMode::Override))));
        assert_eq!(read("rlv\treset"), Ok(Request::Rlv(RlvRequest::Reset)));
        let overridden = |sit, tp| Ok(Request::Rlv(RlvRequest::PersistOverride { sit, tp }));
        assert_eq!(read("rlv\tpersist\toverride\tnone"), overridden(false, false));
        assert_eq!(read("rlv\tpersist\toverride\tsit"), overridden(true, false));
        assert_eq!(read("rlv\tpersist\toverride\ttp"), overridden(false, true));
        assert_eq!(read("rlv\tpersist\toverride\ttp\tsit"), overridden(true, true));
        // A word given twice counts once.
        assert_eq!(read("rlv\tpersist\toverride\ttp\ttp"), overridden(false, true));
        assert_eq!(read("rlv\tpersist\tfollow"), Ok(Request::Rlv(RlvRequest::Persist(true))));
        assert_eq!(read("rlv\tpersist\tdrop"), Ok(Request::Rlv(RlvRequest::Persist(false))));
        assert_eq!(read("rlv\tstatus"), Ok(Request::Rlv(RlvRequest::Status)));
        assert_eq!(
            read("rlv\tcmd\t@versionnum=5"),
            Ok(Request::Rlv(RlvRequest::Cmd("@versionnum=5".to_string())))
        );
        // The words of a command are joined: a region name may have spaces.
        let line = request_line(&strings(&["rlv", "cmd", "@tpto:Blue", "Lagoon/128/100/25=force"]));
        assert_eq!(
            read(&line),
            Ok(Request::Rlv(RlvRequest::Cmd("@tpto:Blue Lagoon/128/100/25=force".to_string())))
        );
    }

    #[test]
    fn rlv_requests_with_the_wrong_arguments_are_refused() {
        let refused = |line: &str| Request::decode(line.as_bytes()).unwrap_err();
        for line in ["rlv", "rlv\trestrictions", "rlv\ton"] {
            let reason = refused(line);
            assert!(
                reason.contains("`mode on|off|override`") && reason.contains("`reset`") && reason.contains("`cmd COMMAND`"),
                "{line}: {reason}"
            );
        }
        for line in ["rlv\tmode", "rlv\tmode\tmaybe", "rlv\tmode\ton\toff", "rlv\tmode\tOn"] {
            assert_eq!(refused(line), "`rlv mode` takes `on`, `off` or `override`", "{line}");
        }
        for line in ["rlv\tpersist", "rlv\tpersist\tkeep", "rlv\tpersist\tfollow\tdrop"] {
            assert_eq!(refused(line), "`rlv persist` takes `follow`, `drop` or `override`", "{line}");
        }
        for line in [
            "rlv\tpersist\toverride",
            "rlv\tpersist\toverride\tstand",
            "rlv\tpersist\toverride\tnone\ttp",
            "rlv\tpersist\toverride\tsit\tnone",
            "rlv\tpersist\toverride\tTP",
        ] {
            let reason = refused(line);
            assert!(
                reason.contains("`none`") && reason.contains("`sit`") && reason.contains("`tp`"),
                "{line}: {reason}"
            );
        }
        assert_eq!(refused("rlv\treset\tall"), "`rlv reset` takes no arguments");
        assert_eq!(refused("rlv\tstatus\tnow"), "`rlv status` takes no arguments");
        assert!(refused("rlv\tcmd").contains("`rlv cmd` takes an RLV command"));
    }

    #[test]
    fn accept_and_perms_requests_are_read_from_their_lines() {
        let read = |line: &str| Request::decode(line.as_bytes());
        let accept = |kind: &str, who: Option<&str>| {
            Ok(Request::Accept {
                kind: kind.to_string(),
                who: who.map(str::to_string),
            })
        };
        assert_eq!(read("accept\ttp"), accept("tp", None));
        assert_eq!(read("accept\ttp\tJane Doe\n"), accept("tp", Some("Jane Doe")));
        assert_eq!(
            read("accept\tanim\t01234567-9abc-defe-dcba-987654321000"),
            accept("anim", Some("01234567-9abc-defe-dcba-987654321000"))
        );
        // The kind is read later, where the kinds are known.
        assert_eq!(read("accept\tmoney"), accept("money", None));
        // The older name of `accept tp`, with its avatar optional now.
        assert_eq!(read("accepttp"), accept("tp", None));
        assert_eq!(read("accepttp\tJane Doe"), accept("tp", Some("Jane Doe")));

        assert_eq!(read("perms\tstatus"), Ok(Request::Perms(PermsRequest::Status)));
        assert_eq!(
            read("perms\tautoaccept\tinventory\t01234567-9abc-defe-dcba-987654321000\tAvatar1"),
            Ok(Request::Perms(PermsRequest::Autoaccept {
                kind: "inventory".to_string(),
                who: strings(&["01234567-9abc-defe-dcba-987654321000", "Avatar1"]),
            }))
        );
        assert_eq!(
            read("perms\trevoke\tanim\tall"),
            Ok(Request::Perms(PermsRequest::Revoke {
                kind: "anim".to_string(),
                who: strings(&["all"]),
            }))
        );
    }

    #[test]
    fn autoreply_requests_are_read() {
        let read = |line: &str| Request::decode(line.as_bytes());
        let auto = |request| Ok(Request::Autoreply(request));
        assert_eq!(
            read("autoreply\tmessage\tNew autoreply from the bot"),
            auto(AutoreplyRequest::Message("New autoreply from the bot".to_string()))
        );
        assert_eq!(
            read("autoreply\tmessage\tback\tin\tan\thour"),
            auto(AutoreplyRequest::Message("back in an hour".to_string()))
        );
        assert_eq!(read("autoreply\tpause\t30m"), auto(AutoreplyRequest::Pause("30m".to_string())));
        assert_eq!(read("autoreply\trefresh"), auto(AutoreplyRequest::Refresh));
        assert_eq!(read("autoreply\ton"), auto(AutoreplyRequest::On));
        assert_eq!(read("autoreply\toff"), auto(AutoreplyRequest::Off));
        assert_eq!(read("autoreply\tstatus"), auto(AutoreplyRequest::Status));
        assert_eq!(read("autoreply\tstatus").unwrap().name(), "autoreply");
    }

    #[test]
    fn malformed_autoreply_requests_are_refused() {
        let refused = |line: &str| Request::decode(line.as_bytes()).unwrap_err();
        for line in ["autoreply", "autoreply\tclear", "autoreply\tOn"] {
            let reason = refused(line);
            assert!(
                reason.contains("`message TEXT...`") && reason.contains("`status`"),
                "{line}: {reason}"
            );
        }
        assert!(refused("autoreply\tmessage").contains("text"));
        for line in ["autoreply\tpause", "autoreply\tpause\t1h\t30m"] {
            assert!(refused(line).contains("`autoreply pause` takes 1 argument"), "{line}");
        }
        for form in ["refresh", "on", "off", "status"] {
            assert_eq!(
                refused(&format!("autoreply\t{form}\tnow")),
                format!("`autoreply {form}` takes no arguments")
            );
        }
    }

    #[test]
    fn malformed_accept_and_perms_requests_are_refused() {
        let refused = |line: &str| Request::decode(line.as_bytes()).unwrap_err();
        for line in ["accept", "accept\ttp\tJane\tDoe"] {
            assert!(refused(line).contains("`accept KIND [WHO]`"), "{line}");
        }
        assert!(refused("accepttp\ta\tb").contains("`accepttp` takes at most 1 argument"));
        for line in ["perms", "perms\tlist", "perms\tStatus"] {
            let reason = refused(line);
            assert!(
                reason.contains("`autoaccept KIND all|WHO...`") && reason.contains("`status`"),
                "{line}: {reason}"
            );
        }
        assert_eq!(refused("perms\tstatus\tnow"), "`perms status` takes no arguments");
        for line in ["perms\tautoaccept", "perms\tautoaccept\tanim"] {
            assert!(refused(line).contains("`perms autoaccept KIND all|WHO...`"), "{line}");
        }
        for line in ["perms\trevoke", "perms\trevoke\tanim"] {
            assert!(refused(line).contains("`perms revoke KIND all|WHO...`"), "{line}");
        }
    }

    #[test]
    fn bad_requests_are_refused_with_a_reason() {
        assert!(Request::decode(b"dance").unwrap_err().contains("`dance`"));
        assert!(Request::decode(b"dance\twith\tme").unwrap_err().contains("`dance`"));
        assert!(Request::decode(b"").unwrap_err().contains("no command"));
        assert!(Request::decode(b"say").unwrap_err().contains("1 argument,"));
        assert!(Request::decode(b"say\ta\tb").unwrap_err().contains("`say`"));
        assert!(Request::decode(b"im\tJane Doe").unwrap_err().contains("2 arguments, not 1"));
        assert!(Request::decode(b"im\ta\tb\tc").unwrap_err().contains("`im`"));
        assert!(Request::decode(b"say\t\xFF\xFE").unwrap_err().contains("UTF-8"));
    }

    #[test]
    fn responses_survive_the_wire() {
        let responses = [
            Response::Ok(None),
            Response::Ok(Some("sent to Jane Doe".into())),
            Response::Ok(Some(String::new())),
            Response::Error("line one\nline two\ttabbed \\ slashed".into()),
        ];
        for response in responses {
            let line = response.encode();
            assert_eq!(line.matches('\n').count(), 1, "{line:?}");
            assert_eq!(Response::decode(&line), Ok(response));
        }
        assert_eq!(Response::Ok(None).encode(), "ok\n");
        assert_eq!(Response::Error("no".into()).encode(), "error\tno\n");
        assert!(Response::decode("maybe\n").is_err());
        assert!(Response::decode("error\n").is_err());
    }

    #[test]
    fn ctl_arguments_become_one_request_line() {
        assert_eq!(request_line(&strings(&["say", "hello", "there"])), "say\thello there\n");
        assert_eq!(request_line(&strings(&["say", "-v", "/1", "x"])), "say\t-v /1 x\n");
        assert_eq!(
            request_line(&strings(&["im", "Jane Doe", "are", "you there?"])),
            "im\tJane Doe\tare you there?\n"
        );
        assert_eq!(request_line(&strings(&["say", "a\tb"])), "say\ta\\tb\n");
        // What is missing is for the bot to complain about.
        assert_eq!(request_line(&strings(&["say"])), "say\n");
        assert_eq!(request_line(&strings(&["im", "Jane Doe"])), "im\tJane Doe\n");
        assert_eq!(request_line(&strings(&["dance", "with", "me"])), "dance\twith\tme\n");
        // A quoted location stays one argument.
        assert_eq!(
            request_line(&strings(&["persist", "Blue Lagoon/128/128/25", "stand"])),
            "persist\tBlue Lagoon/128/128/25\tstand\n"
        );
        assert_eq!(request_line(&strings(&["persist"])), "persist\n");
        // Options and paths go to the bot as they are; a backslash is
        // escaped for the line and comes out of it again.
        assert_eq!(
            request_line(&strings(&["inventory", "-a", "Objects/Bot gear"])),
            "inventory\t-a\tObjects/Bot gear\n"
        );
        let line = request_line(&strings(&["attach", "Objects/Radio\\/TV", "left", "hand"]));
        assert_eq!(line, "attach\tObjects/Radio\\\\/TV\tleft\thand\n");
        assert_eq!(
            Request::decode(line.as_bytes()),
            Ok(Request::Attach {
                item: "Objects/Radio\\/TV".into(),
                part: Some("left hand".into())
            })
        );
    }

    #[test]
    fn address_comes_from_the_environment() {
        let set = |value: &str| Some(value.to_string());
        assert_eq!(address_from(set("/run/bot/ctl"), set("/home/bot")), Ok("/run/bot/ctl".to_string()));
        assert_eq!(address_from(set("/run/bot/ctl"), None), Ok("/run/bot/ctl".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn default_address_is_in_the_home_directory() {
        let set = |value: &str| Some(value.to_string());
        assert_eq!(address_from(None, set("/home/bot")), Ok("/home/bot/russelbot-ctl".to_string()));
        assert_eq!(address_from(set(""), set("/home/bot/")), Ok("/home/bot/russelbot-ctl".to_string()));
        for home in [None, set("")] {
            assert!(address_from(None, home).unwrap_err().contains("RUSSELBOT_SOCK"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_usage_points_at_the_startup_script_for_what_is_not_saved() {
        let flowed = CTL_USAGE.split_whitespace().collect::<Vec<_>>().join(" ");
        // Once each for `perms`, `rlv` and `autoreply`, and once for what
        // the script is.
        assert_eq!(flowed.matches("startup script").count(), 4, "{flowed}");
        assert!(flowed.contains("without `rslbot ctl` in front, are the lines of the config's startup script (`startup`)"));
        // The deprecated tables are no longer where the operator is sent.
        for table in ["[perms.autoaccept]", "[perms.except]", "[autoreply]", "[persist]"] {
            assert!(!CTL_USAGE.contains(table), "{table}");
        }
        assert!(
            CTL_USAGE.lines().all(|line| line.len() <= 79),
            "a usage line is too long for a terminal"
        );
    }

    mod command_line {
        use super::*;

        fn words(list: &[&str]) -> Vec<String> {
            list.iter().map(|word| word.to_string()).collect()
        }

        #[test]
        fn name_with_a_space() {
            assert_eq!(
                split("im \"Jane Doe\" see you later").unwrap(),
                words(&["im", "Jane Doe", "see", "you", "later"])
            );
            let line = request_line(&split("im \"Jane Doe\" see you later").unwrap());
            assert_eq!(
                Request::decode(line.as_bytes()),
                Ok(Request::Im {
                    to: "Jane Doe".into(),
                    text: "see you later".into()
                })
            );
        }

        #[test]
        fn quotes_and_backslashes() {
            assert_eq!(split(r#"say "a \"b\" c\\d""#).unwrap(), words(&["say", r#"a "b" c\d"#]));
            // Outside quotes, and before anything else, a backslash is itself.
            assert_eq!(
                split(r#"attach "Objects/Radio\/TV" left hand"#).unwrap(),
                words(&["attach", r"Objects/Radio\/TV", "left", "hand"])
            );
            assert_eq!(split(r"x a\b").unwrap(), words(&["x", r"a\b"]));
            assert_eq!(split(r#"x a"b c"d """#).unwrap(), words(&["x", "ab cd", ""]));
            assert_eq!(split("tp\t128 128\n25  Home ").unwrap(), words(&["tp", "128", "128", "25", "Home"]));
        }

        #[test]
        fn a_filter_text_of_two_words_is_one_argument() {
            let line = request_line(&split("scan 20 --name \"park bench\" -s").unwrap());
            assert_eq!(
                Request::decode(line.as_bytes()),
                Ok(Request::Scan(words(&["20", "--name", "park bench", "-s"])))
            );
        }

        #[test]
        fn unclosed_quote() {
            let error = split("im \"Jane Doe hi").unwrap_err();
            assert!(error.contains("quote"), "{error}");
        }

        #[test]
        fn empty_body() {
            assert_eq!(split("").unwrap(), Vec::<String>::new());
            assert_eq!(split(" \n\t").unwrap(), Vec::<String>::new());
        }

        #[test]
        fn too_long() {
            assert!(split(&"x".repeat(MAX_COMMAND)).is_ok());
            let error = split(&"x".repeat(MAX_COMMAND + 1)).unwrap_err();
            assert!(error.contains("8192"), "{error}");
        }

        #[test]
        fn words_of_a_message_are_joined_as_ctl_joins_them() {
            assert_eq!(request_line(&split("say a  b").unwrap()), "say\ta b\n");
            assert_eq!(request_line(&split("say a  b").unwrap()), request_line(&words(&["say", "a", "b"])));
        }
    }

    mod socket {
        use super::super::*;
        use super::strings;
        use std::fs;
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::os::unix::net::UnixStream;
        use std::path::PathBuf;
        use std::sync::mpsc::Receiver;

        /// A fresh directory for one test's socket.
        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("rslbot-ctl-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn socket_path(name: &str) -> String {
            temp_dir(name).join("ctl").to_str().unwrap().to_string()
        }

        /// A listening server whose requests are answered by `answer`, as
        /// the main loop would.
        fn serving(name: &str, answer: fn(Request) -> Option<Response>) -> (Server, String) {
            let path = socket_path(name);
            let (events, incoming) = mpsc::channel();
            let server = listen(&path, events).unwrap();
            thread::spawn(move || answer_events(&incoming, answer));
            (server, path)
        }

        fn answer_events(incoming: &Receiver<Event>, answer: fn(Request) -> Option<Response>) {
            for event in incoming {
                if let Event::Control { request, reply } = event {
                    if let Some(response) = answer(request) {
                        let _ = reply.send(response);
                    }
                }
            }
        }

        fn echo(request: Request) -> Option<Response> {
            Some(match request {
                Request::Say(text) if text == "fail" => Response::Error("refused".to_string()),
                Request::Say(_) => Response::Ok(None),
                Request::Im { to, text } => Response::Ok(Some(format!("sent to {to}: {text}"))),
                other => Response::Ok(Some(format!("{other:?}"))),
            })
        }

        fn run(args: &[&str], address: &str) -> (i32, String, String) {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let status = client(&strings(args), Ok(address.to_string()), &mut out, &mut err);
            (status, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
        }

        #[test]
        fn socket_file_is_private_and_removed_with_the_server() {
            let path = socket_path("private");
            let (events, _incoming) = mpsc::channel();
            let server = listen(&path, events).unwrap();
            let found = fs::symlink_metadata(&path).unwrap();
            assert!(found.file_type().is_socket());
            assert_eq!(found.permissions().mode() & 0o777, 0o600);
            drop(server);
            assert!(fs::symlink_metadata(&path).is_err());
        }

        #[test]
        fn stale_socket_file_is_replaced() {
            let path = socket_path("stale");
            // A listener that is gone leaves its file behind.
            drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
            assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
            let (server, path) = {
                let (events, incoming) = mpsc::channel();
                let server = listen(&path, events).unwrap();
                thread::spawn(move || answer_events(&incoming, echo));
                (server, path)
            };
            assert_eq!(run(&["say", "hello"], &path), (0, String::new(), String::new()));
            drop(server);
        }

        #[test]
        fn live_socket_is_not_taken_over() {
            let (_server, path) = serving("live", echo);
            let (events, _incoming) = mpsc::channel();
            let refused = listen(&path, events).err().unwrap();
            assert!(refused.contains(&path) && refused.contains("another process"), "{refused}");
            // The first one still answers, and still has its file.
            assert_eq!(run(&["say", "hello"], &path).0, 0);
        }

        #[test]
        fn other_files_are_left_alone() {
            let path = socket_path("file");
            fs::write(&path, "precious").unwrap();
            let (events, _incoming) = mpsc::channel();
            let refused = listen(&path, events).err().unwrap();
            assert!(refused.contains("not a socket"), "{refused}");
            assert_eq!(fs::read_to_string(&path).unwrap(), "precious");
            // A directory that does not exist is an error naming the address.
            let missing = temp_dir("missing").join("no/such/ctl");
            let (events, _incoming) = mpsc::channel();
            let refused = listen(missing.to_str().unwrap(), events).err().unwrap();
            assert!(refused.contains("no/such/ctl"), "{refused}");
        }

        #[test]
        fn client_reports_the_bots_answer() {
            let (_server, path) = serving("client", echo);
            assert_eq!(run(&["say", "hello", "there"], &path), (0, String::new(), String::new()));
            assert_eq!(
                run(&["im", "Jane Doe", "are", "you", "there?"], &path),
                (0, "sent to Jane Doe: are you there?\n".to_string(), String::new())
            );
            assert_eq!(run(&["say", "fail"], &path), (1, String::new(), "rslbot: refused\n".to_string()));
            let (status, out, err) = run(&["dance"], &path);
            assert_eq!((status, out.as_str()), (1, ""));
            assert!(err.contains("unknown command `dance`"), "{err}");
            let (status, _, err) = run(&["im", "Jane Doe"], &path);
            assert!(status == 1 && err.contains("2 arguments"), "{err}");
        }

        #[test]
        fn client_fails_without_a_bot_or_a_command() {
            let path = socket_path("nobody");
            let (status, out, err) = run(&["say", "hello"], &path);
            assert_eq!((status, out.as_str()), (1, ""));
            assert!(err.contains(&path), "{err}");

            let (status, _, err) = run(&[], &path);
            assert!(status == 2 && err.contains("Usage"), "{err}");
            let (status, out, _) = run(&["--help"], &path);
            assert!(status == 0 && out.contains("Usage"), "{out}");

            // No address to be had at all.
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let status = client(&strings(&["say", "x"]), Err("no home directory".to_string()), &mut out, &mut err);
            assert_eq!(
                (status, String::from_utf8(err).unwrap().as_str()),
                (1, "rslbot: no home directory\n")
            );
        }

        #[test]
        fn over_long_lines_are_refused() {
            let (_server, path) = serving("long", echo);
            let (status, _, err) = run(&["say", &"x".repeat(MAX_LINE)], &path);
            assert!(status == 1 && err.contains("8192"), "{err}");
            // Just under the limit is a request like any other.
            assert_eq!(run(&["say", &"x".repeat(MAX_LINE - 5)], &path).0, 0);
        }

        #[test]
        fn silent_client_is_dropped_and_others_are_served() {
            let (_server, path) = serving("silent", echo);
            let mut silent = UnixStream::connect(&path).unwrap();
            let mut partial = UnixStream::connect(&path).unwrap();
            partial.write_all(b"say\thalf a li").unwrap();
            // Others are served while these two say nothing.
            assert_eq!(run(&["say", "hello"], &path).0, 0);
            let started = Instant::now();
            let mut rest = Vec::new();
            for stream in [&mut silent, &mut partial] {
                stream.set_read_timeout(Some(READ_LIMIT * 2)).unwrap();
                assert_eq!(stream.read_to_end(&mut rest).unwrap(), 0);
            }
            let waited = started.elapsed();
            assert!(
                waited >= READ_LIMIT - Duration::from_secs(1) && waited < READ_LIMIT * 2,
                "{waited:?}"
            );
        }

        #[test]
        fn client_that_leaves_early_still_has_its_command_carried_out() {
            let path = socket_path("early");
            let (events, incoming) = mpsc::channel();
            let _server = listen(&path, events).unwrap();
            let mut stream = UnixStream::connect(&path).unwrap();
            stream.write_all(b"say\thello\n").unwrap();
            drop(stream);
            match incoming.recv_timeout(Duration::from_secs(5)) {
                Ok(Event::Control { request, reply }) => {
                    assert_eq!(request, Request::Say("hello".into()));
                    reply.send(Response::Ok(None)).unwrap();
                }
                _ => panic!("the request did not arrive"),
            }
            // The server is none the worse for it.
            thread::spawn(move || answer_events(&incoming, echo));
            assert_eq!(run(&["say", "again"], &path).0, 0);
        }

        #[test]
        fn connections_beyond_the_limit_are_turned_away() {
            let (_server, path) = serving("crowd", echo);
            let idle: Vec<UnixStream> = (0..MAX_CONNECTIONS).map(|_| UnixStream::connect(&path).unwrap()).collect();
            // Give the listener time to take them all in.
            thread::sleep(Duration::from_millis(300));
            let (status, _, err) = run(&["say", "hello"], &path);
            assert!(status == 1 && err.contains("too many"), "{err}");
            drop(idle);
            thread::sleep(Duration::from_millis(300));
            assert_eq!(run(&["say", "hello"], &path).0, 0);
        }
    }
}
