//! The single TOML config file: model, defaults and validation.

use crate::perms::{Entry, Kind, Rule, Rules, KINDS};
use crate::startup::{self, Line};
use md5::{Digest, Md5};
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

pub const DEFAULT_LOGIN_URL: &str = "https://login.agni.lindenlab.com/cgi-bin/login.cgi";
pub const DEFAULT_CONFIG_PATH: &str = "russelbot.toml";
pub const DEFAULT_REQUESTTP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_SCAN_DISTANCE: f64 = 1024.0;

/// Where the avatar is to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// Held in this region; logins and teleports go to `position`.
    Region { name: String, position: [f32; 3] },
    /// Wherever it is; logins ask for the home location.
    Home,
    /// Wherever it is; logins ask for the location it was last at.
    Latest,
}

/// What the bot seats the avatar on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sit {
    Object(Uuid),
    Ground,
}

/// Where a persisted value comes from once the config placement is reached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// What the simulator reports at that moment.
    Cur,
    /// What the config says.
    Cfg,
}

/// What the bot holds once it has reached the config placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Persist {
    /// Nothing: it goes to manual mode.
    Off,
    On {
        position: Source,
        sit: Source,
    },
}

/// The persist override setting: the sorts of the bot's own steps towards
/// what it holds that RLV restrictions do not hold back.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PersistOverride {
    /// Sitting on the held seat or the ground, and standing up for a held
    /// `stand`.
    pub sit: bool,
    /// Teleporting to the held location.
    pub tp: bool,
}

impl PersistOverride {
    /// The setting as `rlv status` shows it.
    pub fn words(self) -> &'static str {
        match (self.sit, self.tp) {
            (false, false) => "none",
            (true, false) => "sit",
            (false, true) => "tp",
            (true, true) => "sit, tp",
        }
    }
}

/// What a forced RLV movement does to the placement the bot holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RlvPersist {
    /// The bot goes to manual mode and keeps what it held for `home`.
    Drop,
    /// The result of the movement becomes what the bot holds.
    Follow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub login_url: String,
    pub first_name: String,
    pub last_name: String,
    /// Always in `$1$<md5 hex>` form, whichever way it was configured.
    pub password_hash: String,
    pub place: Place,
    /// Without it the bot seats the avatar only on command.
    pub sit: Option<Sit>,
    /// A note for each deprecated key the config uses: that it is
    /// deprecated, and what to write instead.
    pub deprecated: Vec<String>,
    pub persist: Persist,
    /// TSV file for local chat, if any.
    pub log_chat: Option<PathBuf>,
    /// TSV file for instant messages, if any.
    pub log_im: Option<PathBuf>,
    pub fetch_offline_im: bool,
    /// Listen on the control socket.
    pub socket: bool,
    /// How long after a `requesttp` the asked avatar's teleport offer is
    /// accepted without the operator.
    pub requesttp_timeout: Duration,
    /// How far around the avatar objects are learned from the start, in
    /// metres; without it they are learned when first needed.
    pub scan_distance: Option<f32>,
    /// After each login, attach again what the Current Outfit folder lists
    /// and the simulator did not bring.
    pub restore_attachments: bool,
    /// Start with RLV on: read the `@` commands of the objects the avatar
    /// owns.
    pub rlv: bool,
    /// The RLV persist setting the bot starts with.
    pub rlv_persist: RlvPersist,
    /// The persist override setting the bot starts with.
    pub persist_override: PersistOverride,
    /// Which requests the bot accepts by itself when it starts.
    pub perms: Rules,
    /// The XMPP gateway, when the config has an `[xmpp]` table.
    pub xmpp: Option<XmppConfig>,
    /// Autoreplies to instant messages, when the config has an
    /// `[autoreply]` table.
    pub autoreply: Option<AutoreplyConfig>,
    /// The commands of the startup script, from `startup` or from the
    /// file `startup_file` names.
    pub startup: Vec<Line>,
    /// The file the startup script is in, when the config names one.
    /// `Config::load` reads it; `Config::parse` leaves `startup` empty.
    pub startup_file: Option<PathBuf>,
}

impl Config {
    /// The key the startup script is under, for messages about it.
    pub fn startup_key(&self) -> &'static str {
        match self.startup_file {
            Some(_) => "startup_file",
            None => "startup",
        }
    }
}

/// The `[autoreply]` table.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoreplyConfig {
    pub message: String,
    pub pause: Duration,
}

/// The default address of the XMPP server's component port.
pub const DEFAULT_XMPP_SERVER: &str = "127.0.0.1:5347";

/// A secret that never shows in debug output.
#[derive(Clone, PartialEq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Secret {
        Secret(text.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("***")
    }
}

/// The `[xmpp]` table: the bot as an external component of an XMPP server
/// on this host.
#[derive(Debug, Clone, PartialEq)]
pub struct XmppConfig {
    /// The component's domain, in lower case.
    pub component: String,
    /// `host:port` of the server's component port, on this host.
    pub server: String,
    pub secret: Secret,
    /// Bare addresses in lower case, each once.
    pub owners: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    login_url: Option<String>,
    user: Option<String>,
    password: Option<String>,
    password_hash: Option<String>,
    region: Option<String>,
    position: Option<Vec<f64>>,
    start: Option<String>,
    sit: Option<String>,
    object: Option<String>,
    // Kept untyped so that a wrong type is reported with the key's name.
    persist: Option<toml::Value>,
    log_chat: Option<toml::Value>,
    log_im: Option<toml::Value>,
    fetch_offline_im: Option<toml::Value>,
    socket: Option<toml::Value>,
    requesttp_timeout: Option<toml::Value>,
    scan_distance: Option<toml::Value>,
    restore_attachments: Option<toml::Value>,
    rlv: Option<toml::Value>,
    perms: Option<toml::Value>,
    xmpp: Option<toml::Value>,
    autoreply: Option<toml::Value>,
    startup: Option<toml::Value>,
    startup_file: Option<toml::Value>,
}

/// Hashes a plain password into the form grids accept: `$1$` + MD5 hex.
pub fn hash_password(password: &str) -> String {
    let digest = Md5::digest(password.as_bytes());
    let mut out = String::from("$1$");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn is_password_hash(s: &str) -> bool {
    s.strip_prefix("$1$")
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

fn required<T>(value: Option<T>, key: &str) -> Result<T, ConfigError> {
    value.ok_or_else(|| ConfigError(format!("`{key}` is required")))
}

fn flag(value: Option<toml::Value>, key: &str) -> Result<bool, ConfigError> {
    flag_or(value, key, false)
}

/// A boolean key that is `default` when left out.
fn flag_or(value: Option<toml::Value>, key: &str, default: bool) -> Result<bool, ConfigError> {
    match value {
        None => Ok(default),
        Some(toml::Value::Boolean(on)) => Ok(on),
        Some(_) => err(format!("`{key}` must be true or false")),
    }
}

/// A TOML integer or float.
fn number(value: &toml::Value) -> Option<f64> {
    match value {
        toml::Value::Integer(n) => Some(*n as f64),
        toml::Value::Float(n) => Some(*n),
        _ => None,
    }
}

fn log_path(value: Option<toml::Value>, key: &str) -> Result<Option<PathBuf>, ConfigError> {
    match value {
        None => Ok(None),
        Some(toml::Value::String(path)) if !path.is_empty() => Ok(Some(PathBuf::from(path))),
        Some(_) => err(format!("`{key}` must be a non-empty file path")),
    }
}

const PERSIST_FORMS: &str = "`persist` must be \"cur\", \"cfg\", \"off\" (or false), an array \
     [position, sit] of \"cur\" and \"cfg\", or a table with the keys `atstart`, `position`, `sit`, `rlv` and `override`";

fn source(value: &toml::Value) -> Option<Source> {
    match value.as_str() {
        Some("cur") => Some(Source::Cur),
        Some("cfg") => Some(Source::Cfg),
        _ => None,
    }
}

/// The config's `persist`: what the bot holds once the config placement is
/// reached, and the RLV persist setting and the persist override setting,
/// which only the table form can set.
fn persist(value: Option<toml::Value>) -> Result<(Persist, RlvPersist, PersistOverride), ConfigError> {
    let both = |source| Persist::On {
        position: source,
        sit: source,
    };
    let setting = match value {
        None => Persist::Off,
        Some(toml::Value::Boolean(false)) => Persist::Off,
        Some(toml::Value::String(word)) if word == "off" => Persist::Off,
        Some(word @ toml::Value::String(_)) => source(&word).map(both).ok_or(ConfigError(PERSIST_FORMS.to_string()))?,
        Some(toml::Value::Array(items)) => match &items[..] {
            [position, sit] => match (source(position), source(sit)) {
                (Some(position), Some(sit)) => Persist::On { position, sit },
                _ => return err(PERSIST_FORMS),
            },
            _ => return err(PERSIST_FORMS),
        },
        Some(toml::Value::Table(table)) => return persist_table(&table),
        Some(_) => return err(PERSIST_FORMS),
    };
    Ok((setting, RlvPersist::Drop, PersistOverride::default()))
}

/// The `override` key of the `persist` table: `"sit"`, `"tp"`, or an array
/// of these, each counting once.
fn persist_override(value: &toml::Value) -> Result<PersistOverride, ConfigError> {
    const FORMS: &str = "`persist.override` must be \"sit\", \"tp\" or an array of these";
    let words: Vec<&toml::Value> = match value {
        toml::Value::String(_) => vec![value],
        toml::Value::Array(items) => items.iter().collect(),
        _ => return err(FORMS),
    };
    let mut setting = PersistOverride::default();
    for word in words {
        match word.as_str() {
            Some("sit") => setting.sit = true,
            Some("tp") => setting.tp = true,
            _ => return err(FORMS),
        }
    }
    Ok(setting)
}

/// The entries of one array under `perms`, each of them once.
fn perms_entries(key: &str, values: &[toml::Value]) -> Result<Vec<Entry>, ConfigError> {
    let mut entries = Vec::new();
    for value in values {
        let Some(text) = value.as_str() else {
            return err(format!("`{key}` must hold avatar names and UUIDs as strings"));
        };
        if text.trim().eq_ignore_ascii_case("all") {
            let kind = key.rsplit('.').next().unwrap_or(key);
            return err(match key.starts_with("perms.autoaccept") {
                true => format!("`{key}` has \"all\" among its entries; write `{kind} = \"all\"`"),
                false => format!("`{key}` has the entry \"all\", which names nobody: exceptions are avatars and objects"),
            });
        }
        let entry = Entry::parse(text).map_err(|reason| ConfigError(format!("`{key}`: {reason}")))?;
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// The `perms` table: under `autoaccept` the rule of each kind of request,
/// under `except` its exceptions.
fn perms(value: Option<toml::Value>) -> Result<Rules, ConfigError> {
    let mut rules = Rules::default();
    let Some(value) = value else {
        return Ok(rules);
    };
    let toml::Value::Table(table) = value else {
        return err("`perms` must be a table with the tables `autoaccept` and `except`");
    };
    for (name, value) in &table {
        if name != "autoaccept" && name != "except" {
            return err(format!("`perms` has no key `{name}`; its keys are `autoaccept` and `except`"));
        }
        let toml::Value::Table(kinds) = value else {
            return err(format!("`perms.{name}` must be a table whose keys are kinds of request: {KINDS}"));
        };
        for (kind_name, value) in kinds {
            let key = format!("perms.{name}.{kind_name}");
            let Some(kind) = Kind::parse(kind_name) else {
                return err(format!("`{key}` is no kind of request; the kinds are {KINDS}"));
            };
            match (name.as_str(), value) {
                ("autoaccept", toml::Value::String(all)) if all == "all" => rules.set_rule(kind, Rule::All),
                ("autoaccept", toml::Value::Boolean(true)) => rules.set_rule(kind, Rule::All),
                ("autoaccept", toml::Value::Boolean(false)) => rules.set_rule(kind, Rule::List(Vec::new())),
                ("autoaccept", toml::Value::Array(values)) => rules.set_rule(kind, Rule::List(perms_entries(&key, values)?)),
                ("autoaccept", _) => {
                    return err(format!(
                        "`{key}` must be \"all\", true, false or an array of avatar names and UUIDs"
                    ))
                }
                (_, toml::Value::Array(values)) => rules.set_exceptions(kind, perms_entries(&key, values)?),
                _ => return err(format!("`{key}` must be an array of avatar names and UUIDs")),
            }
        }
    }
    for kind in Kind::ALL {
        let Rule::List(entries) = rules.rule(kind) else {
            continue;
        };
        if let Some(both) = entries.iter().find(|entry| rules.exceptions(kind).contains(entry)) {
            return err(format!(
                "`perms.autoaccept.{kind}` and `perms.except.{kind}` both have the entry \"{both}\"; leave it out of one"
            ));
        }
    }
    Ok(rules)
}

const XMPP_KEYS: &str = "`component`, `server`, `secret` and `owners`";

/// Whether `name` is a domain name: dot-separated labels of letters,
/// digits, `-` and `_`, none of them empty or starting or ending with `-`.
fn is_domain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1023
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

/// Whether `host` is this host: `localhost` or a loopback address, an
/// IPv6 one in brackets.
fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return v6.parse::<std::net::Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// The `[autoreply]` table.
fn autoreply(value: Option<toml::Value>) -> Result<Option<AutoreplyConfig>, ConfigError> {
    const KEYS: &str = "`message` and `pause`";
    let Some(value) = value else {
        return Ok(None);
    };
    let toml::Value::Table(table) = value else {
        return err(format!("`autoreply` must be a table with the keys {KEYS}"));
    };
    let (mut message, mut pause) = (None, None);
    for (key, value) in table {
        match key.as_str() {
            "message" => message = Some(value),
            "pause" => pause = Some(value),
            _ => return err(format!("`autoreply.{key}` is not a key of `autoreply`; its keys are {KEYS}")),
        }
    }
    let message = match required(message, "autoreply.message")? {
        toml::Value::String(text) => text,
        _ => return err("`autoreply.message` must be a string, the text sent back"),
    };
    crate::autoreply::check_message(&message).map_err(|why| ConfigError(format!("`autoreply.message`: {why}")))?;
    let pause = match pause {
        None => crate::autoreply::DEFAULT_PAUSE,
        Some(toml::Value::String(text)) => {
            crate::autoreply::parse_duration(&text).map_err(|why| ConfigError(format!("`autoreply.pause`: {why}")))?
        }
        Some(_) => {
            return err(format!(
                "`autoreply.pause` must be a string holding a duration: {}",
                crate::autoreply::DURATION_FORM
            ))
        }
    };
    Ok(Some(AutoreplyConfig { message, pause }))
}

/// The `[xmpp]` table.
fn xmpp(value: Option<toml::Value>) -> Result<Option<XmppConfig>, ConfigError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let toml::Value::Table(table) = value else {
        return err(format!("`xmpp` must be a table with the keys {XMPP_KEYS}"));
    };
    let (mut component, mut server, mut secret, mut owners) = (None, None, None, None);
    for (key, value) in table {
        match key.as_str() {
            "component" => component = Some(value),
            "server" => server = Some(value),
            "secret" => secret = Some(value),
            "owners" => owners = Some(value),
            _ => return err(format!("`xmpp.{key}` is not a key of `xmpp`; its keys are {XMPP_KEYS}")),
        }
    }

    let component = match required(component, "xmpp.component")? {
        toml::Value::String(name) => name.trim().to_lowercase(),
        _ => return err("`xmpp.component` must be a domain name, such as \"russel.sl.example.org\""),
    };
    if component.contains('@') || component.contains('/') {
        return err(format!(
            "`xmpp.component` is \"{component}\", which is an account or an address with a resource; \
             it must be a domain name of its own, such as \"russel.sl.example.org\""
        ));
    }
    if !is_domain(&component) {
        return err(format!("`xmpp.component` is \"{component}\", which is not a domain name"));
    }

    let server = match server {
        None => DEFAULT_XMPP_SERVER.to_string(),
        Some(toml::Value::String(address)) => address.trim().to_string(),
        Some(_) => return err("`xmpp.server` must be a string `host:port`, such as \"127.0.0.1:5347\""),
    };
    let Some((host, port)) = server.rsplit_once(':') else {
        return err(format!(
            "`xmpp.server` is \"{server}\"; it must be `host:port`, such as \"127.0.0.1:5347\""
        ));
    };
    if !port.parse::<u16>().is_ok_and(|port| port > 0) {
        return err(format!(
            "`xmpp.server` has the port \"{port}\"; it must be a number from 1 to 65535"
        ));
    }
    if !is_loopback(host) {
        return err(format!(
            "`xmpp.server` is on \"{host}\": the component connection is not encrypted and must stay \
             on this host, so the host must be `localhost` or a loopback address such as 127.0.0.1"
        ));
    }

    let secret = match required(secret, "xmpp.secret")? {
        toml::Value::String(secret) if !secret.is_empty() => Secret::new(secret),
        _ => return err("`xmpp.secret` must be a non-empty string"),
    };

    const OWNERS: &str = "`xmpp.owners` must be a non-empty array of bare XMPP addresses `user@domain`";
    let toml::Value::Array(entries) = required(owners, "xmpp.owners")? else {
        return err(OWNERS);
    };
    if entries.is_empty() {
        return err(OWNERS);
    }
    let mut list: Vec<String> = Vec::new();
    for entry in entries {
        let Some(text) = entry.as_str() else {
            return err(OWNERS);
        };
        let address = text.trim().to_lowercase();
        let bare = match address.split_once('@') {
            Some((user, domain)) => !user.is_empty() && !user.contains(['/', '"', '&', '\'', '<', '>', ':', ' ']) && is_domain(domain),
            None => false,
        };
        if !bare {
            return err(format!(
                "`xmpp.owners` has the entry \"{text}\", which is not a bare address; write `user@domain`, without a resource"
            ));
        }
        if !list.contains(&address) {
            list.push(address);
        }
    }

    Ok(Some(XmppConfig {
        component,
        server,
        secret,
        owners: list,
    }))
}

/// The table form of `persist`. `atstart` says whether anything is held
/// once the config placement is reached; `position` and `sit` imply it.
fn persist_table(table: &toml::Table) -> Result<(Persist, RlvPersist, PersistOverride), ConfigError> {
    let (mut position, mut sit, mut atstart, mut rlv) = (None, None, None, RlvPersist::Drop);
    let mut overridden = PersistOverride::default();
    for (key, value) in table {
        match key.as_str() {
            "position" | "sit" => {
                let source = source(value).ok_or_else(|| ConfigError(format!("`persist.{key}` must be \"cur\" or \"cfg\"")))?;
                *(if key == "position" { &mut position } else { &mut sit }) = Some(source);
            }
            "atstart" => match value {
                toml::Value::Boolean(on) => atstart = Some(*on),
                _ => return err("`persist.atstart` must be true or false"),
            },
            "rlv" => {
                rlv = match value.as_str() {
                    Some("drop") => RlvPersist::Drop,
                    Some("follow") => RlvPersist::Follow,
                    _ => return err("`persist.rlv` must be \"drop\" or \"follow\""),
                }
            }
            "override" => overridden = persist_override(value)?,
            _ => {
                return err(format!(
                    "`persist` has no key `{key}`; its keys are `atstart`, `position`, `sit`, `rlv` and `override`"
                ))
            }
        }
    }
    let valued = [("position", position), ("sit", sit)]
        .into_iter()
        .find(|(_, value)| value.is_some());
    let on = match (atstart, valued) {
        (Some(false), Some((key, _))) => {
            return err(format!(
                "`persist.atstart` is false, but `persist.{key}` says what to hold; leave one of them out"
            ))
        }
        (Some(on), _) => on,
        (None, valued) => valued.is_some(),
    };
    let setting = match on {
        true => Persist::On {
            position: position.unwrap_or(Source::Cur),
            sit: sit.unwrap_or(Source::Cur),
        },
        false => Persist::Off,
    };
    Ok((setting, rlv, overridden))
}

/// The startup script under `key`, read into its commands.
fn script(key: &str, text: &str) -> Result<Vec<Line>, ConfigError> {
    startup::parse(text).map_err(|e| ConfigError(format!("`{key}`: line {}: {}", e.line, e.reason)))
}

/// One argument as a startup script takes it: in double quotes when it
/// has to be.
fn script_word(text: &str) -> String {
    let plain = !text.is_empty() && !text.contains([' ', '\t', '\n', '\r', '"']) && !text.starts_with('#');
    match plain {
        true => text.to_string(),
        false => format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")),
    }
}

/// The note for a deprecated key that the startup script replaces: the
/// commands to write instead of what the config gives, or that the key
/// can simply go.
fn deprecation(key: &str, commands: &[String], more: &str) -> String {
    let instead = match commands {
        [] => "the bot does the same without it: leave it out".to_string(),
        [command] => format!("write the startup script command `{command}` instead (see `startup`)"),
        commands => format!(
            "write these startup script commands instead (see `startup`): `{}`",
            commands.join("`, `")
        ),
    };
    format!("`{key}` is deprecated; {instead}{more}")
}

/// The note for `persist`: its `rlv` and `override` keys as the setting
/// commands they are, and what is held at the start as a `persist`
/// command, which comes after the setting commands.
fn persist_note(given: &toml::Value, setting: Persist, rlv: RlvPersist, overridden: PersistOverride) -> String {
    let mut commands = Vec::new();
    let table = given.as_table();
    if table.is_some_and(|table| table.contains_key("rlv")) {
        let word = if rlv == RlvPersist::Follow { "follow" } else { "drop" };
        commands.push(format!("rlv persist {word}"));
    }
    if table.is_some_and(|table| table.contains_key("override")) {
        commands.push(format!("rlv persist override {}", overridden.words().replace(", ", " ")));
    }
    let mut more = "";
    if let Persist::On { position, sit } = setting {
        let word = |source| if source == Source::Cur { "cur" } else { "cfg" };
        commands.push(format!("persist {} {}", word(position), word(sit)));
        more = "; `persist` goes after the setting commands";
    }
    deprecation("persist", &commands, more)
}

/// The note for `perms`: each kind's rule as `perms autoaccept`, and its
/// exceptions as `perms revoke`, which only a kind whose rule is `all` has
/// a command for.
fn perms_note(rules: &Rules) -> String {
    let words = |entries: &[Entry]| {
        entries
            .iter()
            .map(|entry| script_word(&entry.to_string()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut commands = Vec::new();
    let mut lost = Vec::new();
    for kind in Kind::ALL {
        let exceptions = rules.exceptions(kind);
        match rules.rule(kind) {
            Rule::All => {
                commands.push(format!("perms autoaccept {kind} all"));
                if !exceptions.is_empty() {
                    commands.push(format!("perms revoke {kind} {}", words(exceptions)));
                }
            }
            Rule::List(entries) => {
                if !entries.is_empty() {
                    commands.push(format!("perms autoaccept {kind} {}", words(entries)));
                }
                if !exceptions.is_empty() {
                    lost.push(format!("`{kind}`"));
                }
            }
        }
    }
    let more = match lost.is_empty() {
        true => String::new(),
        false => format!(
            "; the exceptions for {} cannot be written in the startup script: \
             `perms revoke` makes an exception only while the kind's rule is `all`",
            lost.join(", ")
        ),
    };
    deprecation("perms", &commands, &more)
}

/// The note for `autoreply`: the text, and the pause when the table gives
/// one.
fn autoreply_note(given: &toml::Value, auto: &AutoreplyConfig) -> String {
    let mut commands = vec![format!("autoreply message {}", script_word(&auto.message))];
    if given.as_table().is_some_and(|table| table.contains_key("pause")) {
        commands.push(format!("autoreply pause {}", crate::autoreply::format_duration(auto.pause)));
    }
    deprecation("autoreply", &commands, "")
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
        let mut config = Config::parse(&text).map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
        if let Some(file) = &config.startup_file {
            config.startup = std::fs::read_to_string(file)
                .map_err(|e| ConfigError(format!("`startup_file`: cannot read {}: {e}", file.display())))
                .and_then(|text| script("startup_file", &text))
                .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
        }
        Ok(config)
    }

    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let raw: Raw = toml::from_str(text).map_err(|e| ConfigError(e.message().to_string()))?;

        let login_url = raw.login_url.unwrap_or_else(|| DEFAULT_LOGIN_URL.to_string());
        let rest = login_url.strip_prefix("http://").or_else(|| login_url.strip_prefix("https://"));
        if !rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/')) {
            return err("`login_url` must be an http or https URL");
        }

        let user = required(raw.user, "user")?;
        let words: Vec<&str> = user.split_whitespace().collect();
        let (first_name, last_name) = match words[..] {
            [first] => (first.to_string(), "Resident".to_string()),
            [first, last] => (first.to_string(), last.to_string()),
            [] => return err("`user` must not be empty"),
            _ => return err("`user` must be `First Last` or a single word"),
        };

        let password_hash = match (raw.password, raw.password_hash) {
            (Some(_), Some(_)) => return err("`password` and `password_hash` are both set; use only one"),
            (None, None) => return err("one of `password` or `password_hash` is required"),
            (Some(plain), None) => {
                if plain.is_empty() {
                    return err("`password` must not be empty");
                }
                hash_password(&plain)
            }
            (None, Some(hash)) => {
                if !is_password_hash(&hash) {
                    return err("`password_hash` must be `$1$` followed by 32 lowercase hex digits, \
                         as printed by `rslbot hash`");
                }
                hash
            }
        };

        let place = match (raw.region, raw.position, raw.start) {
            (Some(_), Some(_), Some(_)) | (Some(_), None, Some(_)) | (None, Some(_), Some(_)) => {
                return err("`start` cannot be combined with `region` and `position`; use one or the other")
            }
            (Some(_), None, None) => return err("`region` needs `position` as well"),
            (None, Some(_), None) => return err("`position` needs `region` as well"),
            (Some(region), Some(position), None) => {
                let name = region.trim().to_string();
                if name.is_empty() {
                    return err("`region` must not be empty");
                }
                let position = match position[..] {
                    [x, y, z] if position.iter().all(|v| v.is_finite()) => [x as f32, y as f32, z as f32],
                    _ => return err("`position` must be an array of three finite numbers [x, y, z]"),
                };
                Place::Region { name, position }
            }
            (None, None, start) => match start.as_deref() {
                None | Some("latest") => Place::Latest,
                Some("home") => Place::Home,
                Some(_) => return err("`start` must be \"home\" or \"latest\""),
            },
        };

        let mut deprecated = Vec::new();
        if raw.object.is_some() {
            deprecated.push("`object` is deprecated; write the same value as `sit`".to_string());
        }
        let sit = match (raw.sit, raw.object) {
            (Some(_), Some(_)) => return err("`sit` and `object` are both set; use only `sit`"),
            (None, None) => None,
            (Some(sit), None) => match sit.trim() {
                "ground" => Some(Sit::Ground),
                object => {
                    Some(Sit::Object(Uuid::parse_str(object).map_err(|_| {
                        ConfigError("`sit` must be the UUID of an object or \"ground\"".to_string())
                    })?))
                }
            },
            (None, Some(object)) => Some(Sit::Object(
                Uuid::parse_str(object.trim()).map_err(|_| ConfigError("`object` is not a valid UUID".to_string()))?,
            )),
        };

        let persist_given = raw.persist.clone();
        let (persist, rlv_persist, persist_override) = persist(raw.persist)?;
        if let Some(given) = &persist_given {
            deprecated.push(persist_note(given, persist, rlv_persist, persist_override));
        }
        let log_chat = log_path(raw.log_chat, "log_chat")?;
        let log_im = log_path(raw.log_im, "log_im")?;
        let fetch_offline_im = flag(raw.fetch_offline_im, "fetch_offline_im")?;
        let socket = flag(raw.socket, "socket")?;
        let requesttp_timeout = match raw.requesttp_timeout.as_ref().map(number) {
            None => DEFAULT_REQUESTTP_TIMEOUT,
            // Up to a year; beyond that the number is a mistake.
            Some(Some(minutes)) if minutes > 0.0 && minutes <= 525_600.0 => Duration::from_secs_f64(minutes * 60.0),
            Some(_) => return err("`requesttp_timeout` must be a number of minutes greater than zero"),
        };
        let scan_distance = match raw.scan_distance.as_ref().map(number) {
            None => None,
            Some(Some(metres)) if (1.0..=MAX_SCAN_DISTANCE).contains(&metres) => Some(metres as f32),
            Some(_) => return err("`scan_distance` must be a number of metres from 1 to 1024"),
        };
        let restore_attachments = flag_or(raw.restore_attachments, "restore_attachments", true)?;
        let rlv_given = raw.rlv.is_some();
        let rlv = flag(raw.rlv, "rlv")?;
        if rlv_given {
            let commands = if rlv { vec!["rlv mode on".to_string()] } else { Vec::new() };
            deprecated.push(deprecation("rlv", &commands, ""));
        }
        let perms_given = raw.perms.is_some();
        let perms = perms(raw.perms)?;
        if perms_given {
            deprecated.push(perms_note(&perms));
        }
        let xmpp = xmpp(raw.xmpp)?;
        let autoreply_given = raw.autoreply.clone();
        let autoreply = autoreply(raw.autoreply)?;
        if let (Some(given), Some(auto)) = (&autoreply_given, &autoreply) {
            deprecated.push(autoreply_note(given, auto));
        }
        let (startup, startup_file) = match (raw.startup, raw.startup_file) {
            (Some(_), Some(_)) => return err("`startup` and `startup_file` are both set; use only one"),
            (None, None) => (Vec::new(), None),
            (Some(toml::Value::String(text)), None) => (script("startup", &text)?, None),
            (Some(_), None) => return err("`startup` must be a string holding the startup script, one command per line"),
            (None, Some(toml::Value::String(path))) if !path.is_empty() => (Vec::new(), Some(PathBuf::from(path))),
            (None, Some(_)) => return err("`startup_file` must be a non-empty file path"),
        };

        Ok(Config {
            login_url,
            first_name,
            last_name,
            password_hash,
            place,
            sit,
            deprecated,
            persist,
            log_chat,
            log_im,
            fetch_offline_im,
            socket,
            requesttp_timeout,
            scan_distance,
            restore_attachments,
            rlv,
            rlv_persist,
            persist_override,
            perms,
            xmpp,
            autoreply,
            startup,
            startup_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT: &str = "11111111-2222-3333-4444-555555555555";

    fn base() -> String {
        format!(
            "user = \"Sit Bot\"\npassword = \"secret\"\nregion = \"Target\"\n\
             position = [128.0, 128.5, 25]\nsit = \"{OBJECT}\"\n"
        )
    }

    fn rejected(text: &str) -> String {
        Config::parse(text).unwrap_err().0
    }

    #[test]
    fn minimal_config_uses_agni() {
        let cfg = Config::parse(&base()).unwrap();
        assert_eq!(cfg.login_url, DEFAULT_LOGIN_URL);
        assert_eq!((cfg.first_name.as_str(), cfg.last_name.as_str()), ("Sit", "Bot"));
        assert_eq!(cfg.password_hash, "$1$5ebe2294ecd0e0f08eab7690d2a6ee69");
        assert_eq!(
            cfg.place,
            Place::Region {
                name: "Target".to_string(),
                position: [128.0, 128.5, 25.0]
            }
        );
        assert_eq!(cfg.sit, Some(Sit::Object(Uuid::parse_str(OBJECT).unwrap())));
        assert!(cfg.deprecated.is_empty());
    }

    /// `base()` without the lines of the given keys.
    fn without(keys: &[&str]) -> String {
        base()
            .lines()
            .filter(|l| !keys.iter().any(|k| l.starts_with(k)))
            .map(|l| format!("{l}\n"))
            .collect()
    }

    #[test]
    fn account_alone_is_a_config() {
        let cfg = Config::parse(&without(&["region", "position", "sit"])).unwrap();
        assert_eq!((cfg.place, cfg.sit, cfg.deprecated.is_empty()), (Place::Latest, None, true));
    }

    #[test]
    fn start_names_the_login_location() {
        let bare = without(&["region", "position"]);
        assert_eq!(
            Config::parse(&(bare.clone() + "start = \"latest\"\n")).unwrap().place,
            Place::Latest
        );
        assert_eq!(Config::parse(&(bare.clone() + "start = \"home\"\n")).unwrap().place, Place::Home);
        for bad in ["last", "Home", "", "uri:Target&1&2&3"] {
            let msg = rejected(&(bare.clone() + &format!("start = \"{bad}\"\n")));
            assert!(
                msg.contains("`start`") && msg.contains("\"home\"") && msg.contains("\"latest\""),
                "{msg}"
            );
        }
    }

    #[test]
    fn rejects_start_with_a_region() {
        for text in [base(), without(&["region"]), without(&["position"])] {
            let msg = rejected(&(text + "start = \"home\"\n"));
            assert!(msg.contains("`start`") && msg.contains("`region`"), "{msg}");
        }
    }

    #[test]
    fn region_and_position_come_as_a_pair() {
        for missing in ["region", "position"] {
            let msg = rejected(&without(&[missing]));
            assert!(msg.contains("`region`") && msg.contains("`position`"), "{msg}");
        }
        assert!(rejected(&base().replace("\"Target\"", "\"  \"")).contains("`region`"));
    }

    #[test]
    fn sit_is_an_object_or_the_ground() {
        let cfg = Config::parse(&base().replace(OBJECT, "ground")).unwrap();
        assert_eq!(cfg.sit, Some(Sit::Ground));
        for bad in ["not-a-uuid", "Ground", ""] {
            let msg = rejected(&base().replace(OBJECT, bad));
            assert!(msg.contains("`sit`") && !msg.contains("`object`"), "{msg}");
        }
    }

    #[test]
    fn object_is_the_old_spelling_of_sit() {
        let old = Config::parse(&base().replace("sit = ", "object = ")).unwrap();
        assert_eq!(old.deprecated, ["`object` is deprecated; write the same value as `sit`"]);
        let new = Config::parse(&base()).unwrap();
        assert_eq!(
            Config {
                deprecated: Vec::new(),
                ..old
            },
            new
        );
        // The new value belongs to the new key.
        let msg = rejected(&base().replace("sit = ", "object = ").replace(OBJECT, "ground"));
        assert!(msg.contains("`object`"), "{msg}");
    }

    #[test]
    fn rejects_both_seat_keys() {
        let msg = rejected(&(base() + &format!("object = \"{OBJECT}\"\n")));
        assert!(msg.contains("`sit`") && msg.contains("`object`"), "{msg}");
    }

    fn held(position: Source, sit: Source) -> Persist {
        Persist::On { position, sit }
    }

    #[test]
    fn persist_is_off_by_default() {
        assert_eq!(Config::parse(&base()).unwrap().persist, Persist::Off);
    }

    #[test]
    fn persist_takes_one_word_for_both_values() {
        let parsed = |value: &str| Config::parse(&(base() + &format!("persist = {value}\n"))).unwrap().persist;
        assert_eq!(parsed("\"cur\""), held(Source::Cur, Source::Cur));
        assert_eq!(parsed("\"cfg\""), held(Source::Cfg, Source::Cfg));
        assert_eq!(parsed("\"off\""), Persist::Off);
        assert_eq!(parsed("false"), Persist::Off);
    }

    #[test]
    fn persist_takes_an_array_of_position_and_sit() {
        let cfg = Config::parse(&(base() + "persist = [\"cfg\", \"cur\"]\n")).unwrap();
        assert_eq!(cfg.persist, held(Source::Cfg, Source::Cur));
    }

    #[test]
    fn persist_takes_a_table() {
        let cfg = Config::parse(&(base() + "[persist]\nposition = \"cur\"\nsit = \"cfg\"\n")).unwrap();
        assert_eq!(cfg.persist, held(Source::Cur, Source::Cfg));
        // A key that is left out is "cur".
        let cfg = Config::parse(&(base() + "[persist]\nsit = \"cfg\"\n")).unwrap();
        assert_eq!(cfg.persist, held(Source::Cur, Source::Cfg));
        let cfg = Config::parse(&(base() + "[persist]\nposition = \"cfg\"\n")).unwrap();
        assert_eq!(cfg.persist, held(Source::Cfg, Source::Cur));
    }

    fn table(keys: &str) -> Config {
        Config::parse(&(base() + "[persist]\n" + keys)).unwrap()
    }

    #[test]
    fn a_persist_table_holds_something_only_when_it_says_so() {
        // Nothing to hold at the start: as `persist = "off"`.
        assert_eq!(table("").persist, Persist::Off);
        assert_eq!(table("atstart = false\n").persist, Persist::Off);
        assert_eq!(table("rlv = \"follow\"\n").persist, Persist::Off);
        // `atstart` alone is "cur" for both.
        assert_eq!(table("atstart = true\n").persist, held(Source::Cur, Source::Cur));
        // `position` and `sit` imply it, and may say so as well.
        assert_eq!(table("atstart = true\nsit = \"cfg\"\n").persist, held(Source::Cur, Source::Cfg));
        assert_eq!(table("sit = \"cfg\"\nrlv = \"follow\"\n").persist, held(Source::Cur, Source::Cfg));
    }

    #[test]
    fn the_persist_override_setting_comes_from_the_table_only() {
        let setting = |sit, tp| PersistOverride { sit, tp };
        let table = |keys: &str| Config::parse(&(base() + "[persist]\n" + keys)).unwrap();
        // Not mentioned: empty, whatever the form of `persist`.
        assert_eq!(Config::parse(&base()).unwrap().persist_override, setting(false, false));
        for form in [
            "persist = \"cfg\"\n",
            "persist = [\"cfg\", \"cur\"]\n",
            "[persist]\nsit = \"cfg\"\n",
        ] {
            assert_eq!(
                Config::parse(&(base() + form)).unwrap().persist_override,
                setting(false, false),
                "{form}"
            );
        }
        assert_eq!(table("override = []\n").persist_override, setting(false, false));
        // One word, or an array in which each counts once.
        let teleports = table("sit = \"cfg\"\noverride = \"tp\"\n");
        assert_eq!(teleports.persist_override, setting(false, true));
        assert_eq!(teleports.persist, held(Source::Cur, Source::Cfg));
        assert_eq!(table("override = \"sit\"\n").persist_override, setting(true, false));
        assert_eq!(table("override = [\"sit\", \"tp\"]\n").persist_override, setting(true, true));
        assert_eq!(
            table("override = [\"tp\", \"sit\", \"tp\"]\n").persist_override,
            setting(true, true)
        );
        // It says nothing about `atstart`, and leaves the RLV persist setting alone.
        let alone = table("override = \"sit\"\n");
        assert_eq!((alone.persist, alone.rlv_persist), (Persist::Off, RlvPersist::Drop));
        assert_eq!(table("rlv = \"follow\"\noverride = \"tp\"\n").rlv_persist, RlvPersist::Follow);
        assert_eq!(setting(false, false).words(), "none");
        assert_eq!(setting(true, true).words(), "sit, tp");
    }

    #[test]
    fn a_bad_persist_override_is_rejected() {
        for value in ["\"stand\"", "true", "[\"tp\", \"home\"]", "[1]", "\"TP\"", "{ sit = true }"] {
            let reason = Config::parse(&(base() + &format!("[persist]\noverride = {value}\n")))
                .unwrap_err()
                .0;
            assert!(
                reason.contains("`persist.override`") && reason.contains("\"sit\"") && reason.contains("\"tp\""),
                "{value}: {reason}"
            );
        }
        // The message about an unknown key names the new one too.
        let reason = Config::parse(&(base() + "[persist]\nlocation = \"cur\"\n")).unwrap_err().0;
        assert!(reason.contains("`override`"), "{reason}");
    }

    #[test]
    fn the_rlv_persist_setting_comes_from_the_table_only() {
        assert_eq!(Config::parse(&base()).unwrap().rlv_persist, RlvPersist::Drop);
        for form in ["\"cfg\"", "\"off\"", "false", "[\"cur\", \"cfg\"]"] {
            let cfg = Config::parse(&(base() + &format!("persist = {form}\n"))).unwrap();
            assert_eq!(cfg.rlv_persist, RlvPersist::Drop, "{form}");
        }
        assert_eq!(table("sit = \"cfg\"\n").rlv_persist, RlvPersist::Drop);
        assert_eq!(table("rlv = \"drop\"\n").rlv_persist, RlvPersist::Drop);
        assert_eq!(table("rlv = \"follow\"\n").rlv_persist, RlvPersist::Follow);
        assert_eq!(table("sit = \"cfg\"\nrlv = \"follow\"\n").rlv_persist, RlvPersist::Follow);
        // It does not switch RLV on.
        assert!(!table("rlv = \"follow\"\n").rlv);
    }

    #[test]
    fn rejects_persist_tables_that_contradict_themselves() {
        let msg = rejected(&(base() + "[persist]\natstart = false\nsit = \"cfg\"\n"));
        assert!(msg.contains("`persist.atstart`") && msg.contains("`persist.sit`"), "{msg}");
        let msg = rejected(&(base() + "[persist]\natstart = false\nposition = \"cur\"\n"));
        assert!(msg.contains("`persist.atstart`") && msg.contains("`persist.position`"), "{msg}");
        for bad in ["\"yes\"", "1", "\"cur\""] {
            let msg = rejected(&(base() + &format!("[persist]\natstart = {bad}\n")));
            assert!(msg.contains("`persist.atstart`"), "{bad}: {msg}");
        }
        for bad in ["\"keep\"", "true", "\"Follow\"", "\"\""] {
            let msg = rejected(&(base() + &format!("[persist]\nrlv = {bad}\n")));
            assert!(
                msg.contains("`persist.rlv`") && msg.contains("\"drop\"") && msg.contains("\"follow\""),
                "{bad}: {msg}"
            );
        }
    }

    #[test]
    fn rlv_is_off_unless_enabled() {
        assert!(!Config::parse(&base()).unwrap().rlv);
        assert!(!Config::parse(&(base() + "rlv = false\n")).unwrap().rlv);
        assert!(Config::parse(&(base() + "rlv = true\n")).unwrap().rlv);
        for bad in ["\"on\"", "1", "\"follow\""] {
            let msg = rejected(&(base() + &format!("rlv = {bad}\n")));
            assert!(msg.contains("`rlv`") && !msg.contains("persist"), "{bad}: {msg}");
        }
    }

    #[test]
    fn rejects_persist_values_only_the_command_takes() {
        for bad in ["\"last\"", "\"last_cmd\"", "\"Target/128/128/25\"", "true", "5", "\"Cur\"", "\"\""] {
            let msg = rejected(&(base() + &format!("persist = {bad}\n")));
            assert!(
                msg.contains("`persist`") && msg.contains("\"cfg\"") && msg.contains("\"off\""),
                "{bad}: {msg}"
            );
        }
    }

    #[test]
    fn rejects_malformed_persist_arrays() {
        for bad in [
            "[]",
            "[\"cur\"]",
            "[\"cur\", \"cfg\", \"cur\"]",
            "[\"cur\", \"off\"]",
            "[\"cur\", false]",
        ] {
            let msg = rejected(&(base() + &format!("persist = {bad}\n")));
            assert!(msg.contains("`persist`"), "{bad}: {msg}");
        }
    }

    #[test]
    fn rejects_malformed_persist_tables() {
        let msg = rejected(&(base() + "[persist]\nlocation = \"cur\"\n"));
        assert!(msg.contains("`persist`") && msg.contains("`location`"), "{msg}");
        let msg = rejected(&(base() + "[persist]\nsit = \"ground\"\n"));
        assert!(msg.contains("`persist.sit`"), "{msg}");
        let msg = rejected(&(base() + "[persist]\nposition = \"off\"\n"));
        assert!(msg.contains("`persist.position`"), "{msg}");
    }

    #[test]
    fn single_word_user_gets_resident() {
        let cfg = Config::parse(&base().replace("Sit Bot", "sitbot")).unwrap();
        assert_eq!((cfg.first_name.as_str(), cfg.last_name.as_str()), ("sitbot", "Resident"));
    }

    #[test]
    fn password_hash_is_accepted_as_is() {
        let hash = "$1$5ebe2294ecd0e0f08eab7690d2a6ee69";
        let text = base().replace("password = \"secret\"", &format!("password_hash = '{hash}'"));
        assert_eq!(Config::parse(&text).unwrap().password_hash, hash);
    }

    #[test]
    fn rejects_both_password_forms() {
        let text = base() + "password_hash = '$1$5ebe2294ecd0e0f08eab7690d2a6ee69'\n";
        let msg = rejected(&text);
        assert!(msg.contains("`password`") && msg.contains("`password_hash`"), "{msg}");
    }

    #[test]
    fn rejects_neither_password_form() {
        let msg = rejected(&base().replace("password = \"secret\"\n", ""));
        assert!(msg.contains("`password`") && msg.contains("`password_hash`"), "{msg}");
    }

    #[test]
    fn rejects_malformed_password_hash() {
        for bad in ["5ebe2294ecd0e0f08eab7690d2a6ee69", "$1$5EBE2294ECD0E0F08EAB7690D2A6EE69", "$1$abc"] {
            let text = base().replace("password = \"secret\"", &format!("password_hash = '{bad}'"));
            assert!(rejected(&text).contains("`password_hash`"));
        }
    }

    #[test]
    fn rejects_unknown_key() {
        let msg = rejected(&(base() + "regoin = \"x\"\n"));
        assert!(msg.contains("regoin"), "{msg}");
    }

    #[test]
    fn only_the_user_is_required_by_name() {
        assert!(rejected(&without(&["user"])).contains("`user`"));
        assert_eq!(Config::parse(&without(&["sit"])).unwrap().sit, None);
    }

    #[test]
    fn rejects_three_word_user() {
        assert!(rejected(&base().replace("Sit Bot", "A B C")).contains("`user`"));
    }

    #[test]
    fn rejects_bad_object_uuid() {
        let msg = rejected(&base().replace("sit = ", "object = ").replace(OBJECT, "not-a-uuid"));
        assert!(msg.contains("`object`"), "{msg}");
    }

    #[test]
    fn rejects_bad_login_url() {
        for bad in ["ftp://example.com/", "login.example.com", "http://"] {
            let text = base() + &format!("login_url = \"{bad}\"\n");
            assert!(rejected(&text).contains("`login_url`"), "{bad}");
        }
    }

    #[test]
    fn rejects_bad_position() {
        for bad in ["[1.0, 2.0]", "[1.0, 2.0, 3.0, 4.0]", "[1.0, 2.0, inf]", "[1.0, nan, 3.0]"] {
            let text = base().replace("[128.0, 128.5, 25]", bad);
            assert!(rejected(&text).contains("`position`"), "{bad}");
        }
    }

    #[test]
    fn message_logging_is_off_by_default() {
        let cfg = Config::parse(&base()).unwrap();
        assert_eq!((cfg.log_chat, cfg.log_im, cfg.fetch_offline_im), (None, None, false));
    }

    #[test]
    fn accepts_message_logging_keys() {
        let text = base() + "log_chat = \"chat.tsv\"\nlog_im = \"/var/log/im.tsv\"\nfetch_offline_im = true\n";
        let cfg = Config::parse(&text).unwrap();
        assert_eq!(cfg.log_chat, Some(PathBuf::from("chat.tsv")));
        assert_eq!(cfg.log_im, Some(PathBuf::from("/var/log/im.tsv")));
        assert!(cfg.fetch_offline_im);
    }

    #[test]
    fn control_socket_is_off_unless_enabled() {
        assert!(!Config::parse(&base()).unwrap().socket);
        assert!(!Config::parse(&(base() + "socket = false\n")).unwrap().socket);
        assert!(Config::parse(&(base() + "socket = true\n")).unwrap().socket);
        assert!(rejected(&(base() + "socket = \"yes\"\n")).contains("`socket`"));
    }

    #[test]
    fn teleport_request_timeout_is_ten_minutes_unless_configured() {
        assert_eq!(Config::parse(&base()).unwrap().requesttp_timeout, Duration::from_secs(600));
        let cfg = Config::parse(&(base() + "requesttp_timeout = 30\n")).unwrap();
        assert_eq!(cfg.requesttp_timeout, Duration::from_secs(1800));
        let cfg = Config::parse(&(base() + "requesttp_timeout = 0.5\n")).unwrap();
        assert_eq!(cfg.requesttp_timeout, Duration::from_secs(30));
        for bad in ["0", "-5", "\"soon\"", "true", "inf", "nan", "1e9"] {
            let text = base() + &format!("requesttp_timeout = {bad}\n");
            assert!(rejected(&text).contains("`requesttp_timeout`"), "{bad}");
        }
    }

    #[test]
    fn restore_attachments_is_on_unless_switched_off() {
        assert!(Config::parse(&base()).unwrap().restore_attachments);
        assert!(
            Config::parse(&(base() + "restore_attachments = true\n"))
                .unwrap()
                .restore_attachments
        );
        assert!(
            !Config::parse(&(base() + "restore_attachments = false\n"))
                .unwrap()
                .restore_attachments
        );
        for bad in ["\"no\"", "0", "[false]"] {
            let text = base() + &format!("restore_attachments = {bad}\n");
            assert!(rejected(&text).contains("`restore_attachments`"), "{bad}");
        }
    }

    #[test]
    fn scan_distance_is_optional_and_bounded() {
        assert_eq!(Config::parse(&base()).unwrap().scan_distance, None);
        assert_eq!(Config::parse(&(base() + "scan_distance = 64\n")).unwrap().scan_distance, Some(64.0));
        assert_eq!(Config::parse(&(base() + "scan_distance = 1.5\n")).unwrap().scan_distance, Some(1.5));
        assert_eq!(
            Config::parse(&(base() + "scan_distance = 1024\n")).unwrap().scan_distance,
            Some(1024.0)
        );
        for bad in ["0", "0.5", "5000", "-32", "\"far\"", "nan"] {
            let text = base() + &format!("scan_distance = {bad}\n");
            assert!(rejected(&text).contains("`scan_distance`"), "{bad}");
        }
    }

    #[test]
    fn rejects_bad_message_logging_keys() {
        for key in ["log_chat", "log_im"] {
            assert!(rejected(&(base() + &format!("{key} = \"\"\n"))).contains(&format!("`{key}`")));
            assert!(rejected(&(base() + &format!("{key} = 5\n"))).contains(&format!("`{key}`")));
        }
        assert!(rejected(&(base() + "fetch_offline_im = \"yes\"\n")).contains("`fetch_offline_im`"));
    }

    #[test]
    fn rejects_invalid_toml() {
        assert!(Config::parse("user = ").is_err());
    }

    #[test]
    fn example_file_is_valid() {
        let cfg = Config::parse(include_str!("../russelbot.example.toml")).unwrap();
        assert_eq!(cfg.login_url, "http://127.0.0.1:9000/");
        assert_eq!(
            cfg.place,
            Place::Region {
                name: "Target".to_string(),
                position: [128.0, 128.0, 25.0]
            }
        );
        assert_eq!(cfg.sit, Some(Sit::Object(Uuid::nil())));
        assert!(cfg.deprecated.is_empty());
        assert_eq!(cfg.persist, Persist::Off);
        assert!(!cfg.rlv);
        assert_eq!(cfg.rlv_persist, RlvPersist::Drop);
        assert!(cfg.startup.is_empty() && cfg.startup_file.is_none());
    }

    #[test]
    fn example_file_is_valid_with_its_startup_script_enabled() {
        let text = include_str!("../russelbot.example.toml");
        // The block from `# startup = '''` to `# '''`, without its `# `.
        let start = text.find("\n# startup = '''\n").expect("the example has a startup script");
        let end = start + text[start + 1..].find("\n# '''\n").expect("the script ends") + "\n# '''\n".len();
        let block: Vec<&str> = text[start..=end]
            .lines()
            .map(|line| line.strip_prefix("# ").unwrap_or(line))
            .collect();
        let enabled = format!("{}{}\n{}", &text[..start], block.join("\n"), &text[end + 1..]);
        let cfg = Config::parse(&enabled).unwrap();
        let commands: Vec<&str> = cfg.startup.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            commands,
            [
                "rlv mode on",
                "rlv persist follow",
                "rlv persist override sit tp",
                "perms autoaccept tp \"Jane Doe\"",
                "perms autoaccept inventory \"Owner Resident\" 11111111-2222-3333-4444-555555555555",
                "perms autoaccept anim all",
                "perms revoke anim 01234567-9abc-defe-dcba-987654321000",
                "perms autoaccept attach all",
                "autoreply message Hello, I'm bot. I won't respond to you in next 2 hours",
                "autoreply pause 2h",
                "persist cur cfg",
            ]
        );
        // The setting commands are first, as the example says they should be.
        let (last, settings) = cfg.startup.split_last().unwrap();
        assert!(settings.iter().all(|line| startup::is_setting(&line.request)));
        assert!(!startup::is_setting(&last.request));
        // Nothing in the example is deprecated, with or without the script.
        assert!(cfg.deprecated.is_empty());
        assert_eq!((cfg.persist, cfg.rlv, cfg.autoreply), (Persist::Off, false, None));
        // The script in a file instead.
        let file = "# startup_file = \"startup.rsl\"";
        assert_eq!(text.matches(&format!("\n{file}\n")).count(), 1, "{file} is not once in the example");
        let cfg = Config::parse(&text.replace(&format!("\n{file}\n"), &format!("\n{}\n", &file[2..]))).unwrap();
        assert_eq!(cfg.startup_file, Some(PathBuf::from("startup.rsl")));
        assert_eq!(
            rejected(&enabled.replace(file, &file[2..])),
            "`startup` and `startup_file` are both set; use only one"
        );
    }

    fn perms_of(tables: &str) -> Rules {
        Config::parse(&format!("{}{tables}", base())).unwrap().perms
    }

    fn entry(text: &str) -> Entry {
        Entry::parse(text).unwrap()
    }

    #[test]
    fn without_a_perms_table_nothing_is_auto_accepted() {
        assert_eq!(perms_of(""), Rules::default());
        assert_eq!(perms_of("[perms]\n"), Rules::default());
        assert_eq!(perms_of("[perms.autoaccept]\n[perms.except]\n"), Rules::default());
    }

    #[test]
    fn perms_tables_give_the_rules_and_the_exceptions() {
        let rules = perms_of(&format!(
            "[perms.autoaccept]\ntp = [\"Avatar1\", \"Avatar2\"]\n\
             inventory = [\"Owner Resident\", \"{OBJECT}\"]\n"
        ));
        assert_eq!(*rules.rule(Kind::Tp), Rule::List(vec![entry("Avatar1"), entry("Avatar2")]));
        assert_eq!(
            *rules.rule(Kind::Inventory),
            Rule::List(vec![entry("Owner Resident"), entry(OBJECT)])
        );
        for kind in [Kind::Anim, Kind::Attach, Kind::Controls, Kind::Camera] {
            assert_eq!(*rules.rule(kind), Rule::List(vec![]), "{kind}");
        }

        // Everyone, with an exception.
        let rules = perms_of(&format!(
            "[perms.autoaccept]\nanim = \"all\"\n[perms.except]\nanim = [\"{OBJECT}\"]\n"
        ));
        assert_eq!(*rules.rule(Kind::Anim), Rule::All);
        assert_eq!(rules.exceptions(Kind::Anim), [entry(OBJECT)]);

        // Booleans are other spellings of "all" and of an empty list.
        let rules = perms_of("[perms.autoaccept]\nattach = true\ncontrols = false\ncamera = []\n");
        assert_eq!(*rules.rule(Kind::Attach), Rule::All);
        assert_eq!(*rules.rule(Kind::Controls), Rule::List(vec![]));
        assert_eq!(*rules.rule(Kind::Camera), Rule::List(vec![]));

        // Exceptions alone, and an entry written twice.
        let rules = perms_of("[perms.except]\ntp = [\"Jane Doe\", \"jane.doe\", \"John Roe\"]\n");
        assert_eq!(*rules.rule(Kind::Tp), Rule::List(vec![]));
        assert_eq!(rules.exceptions(Kind::Tp), [entry("Jane Doe"), entry("John Roe")]);

        // An exception to a list is fine as long as it is another entry.
        let rules = perms_of(&format!(
            "[perms.autoaccept]\nanim = [\"Jane Doe\"]\n[perms.except]\nanim = [\"{OBJECT}\"]\n"
        ));
        assert_eq!(*rules.rule(Kind::Anim), Rule::List(vec![entry("Jane Doe")]));
        assert_eq!(rules.exceptions(Kind::Anim), [entry(OBJECT)]);
    }

    #[test]
    fn a_malformed_perms_table_is_rejected_with_its_key() {
        let rejected = |tables: &str| rejected(&format!("{}{tables}", base()));
        assert!(rejected("perms = true\n").contains("`perms` must be a table"));
        assert!(rejected("[perms]\nautoaccept = \"all\"\n").contains("`perms.autoaccept` must be a table"));
        assert!(rejected("[perms.accept]\ntp = \"all\"\n").contains("`perms` has no key `accept`"));
        let unknown = rejected("[perms.autoaccept]\nmoney = \"all\"\n");
        assert!(unknown.contains("`perms.autoaccept.money` is no kind of request"), "{unknown}");
        assert!(
            unknown.contains("`tp`, `inventory`, `anim`, `attach`, `controls` and `camera`"),
            "{unknown}"
        );
        assert!(rejected("[perms.except]\nmoney = []\n").contains("`perms.except.money` is no kind"));

        let malformed = rejected("[perms.autoaccept]\ntp = [\"Jane van Doe\"]\n");
        assert!(malformed.contains("`perms.autoaccept.tp`"), "{malformed}");
        assert!(malformed.contains("\"Jane van Doe\" is neither a UUID nor"), "{malformed}");
        assert!(rejected("[perms.except]\ntp = [\"\"]\n").contains("`perms.except.tp`"));
        assert!(rejected("[perms.autoaccept]\ntp = [7]\n").contains("`perms.autoaccept.tp` must hold"));
        for wrong in ["\"everyone\"", "7", "\"Jane Doe\""] {
            let message = rejected(&format!("[perms.autoaccept]\ntp = {wrong}\n"));
            assert!(
                message.contains("`perms.autoaccept.tp` must be \"all\", true, false or an array"),
                "{message}"
            );
        }

        let inside = rejected("[perms.autoaccept]\nanim = [\"all\"]\n");
        assert!(inside.contains("`perms.autoaccept.anim`"), "{inside}");
        assert!(inside.contains("write `anim = \"all\"`"), "{inside}");
        assert!(rejected("[perms.except]\nanim = [\"all\"]\n").contains("`perms.except.anim` has the entry \"all\""));
        for wrong in ["\"all\"", "true", "false"] {
            let message = rejected(&format!("[perms.except]\nanim = {wrong}\n"));
            assert!(message.contains("`perms.except.anim` must be an array"), "{message}");
        }

        let both = rejected("[perms.autoaccept]\nanim = [\"Jane Doe\"]\n[perms.except]\nanim = [\"jane.doe\"]\n");
        assert!(both.contains("`perms.autoaccept.anim` and `perms.except.anim`"), "{both}");
        assert!(both.contains("\"Jane Doe\""), "{both}");
    }

    fn autoreply_table(lines: &str) -> String {
        base() + "[autoreply]\n" + lines
    }

    #[test]
    fn autoreply_configured() {
        let text = autoreply_table("message = \"Hello, I'm bot. I won't respond to you in next 2 hours\"\npause = \"2h\"\n");
        let autoreply = Config::parse(&text).unwrap().autoreply.unwrap();
        assert_eq!(autoreply.message, "Hello, I'm bot. I won't respond to you in next 2 hours");
        assert_eq!(autoreply.pause, Duration::from_secs(2 * 3600));
    }

    #[test]
    fn autoreply_without_a_pause() {
        let autoreply = Config::parse(&autoreply_table("message = \"Away\"\n")).unwrap().autoreply.unwrap();
        assert_eq!(autoreply.pause, Duration::from_secs(3600));
    }

    #[test]
    fn no_autoreply_table() {
        assert_eq!(Config::parse(&base()).unwrap().autoreply, None);
    }

    #[test]
    fn autoreply_without_a_message() {
        let msg = rejected(&autoreply_table("pause = \"2h\"\n"));
        assert!(msg.contains("`autoreply.message`"), "{msg}");
    }

    #[test]
    fn autoreply_with_a_malformed_pause() {
        for pause in ["\"2 hours\"", "\"30\"", "\"30m2h\"", "\"400d\"", "30"] {
            let msg = rejected(&autoreply_table(&format!("message = \"Away\"\npause = {pause}\n")));
            assert!(msg.contains("`autoreply.pause`") && msg.contains("`1h30m`"), "{msg}");
        }
    }

    #[test]
    fn autoreply_with_a_blank_or_wrong_message() {
        for message in ["\"  \"", "5", &format!("\"{}\"", "x".repeat(1024))] {
            let msg = rejected(&autoreply_table(&format!("message = {message}\n")));
            assert!(msg.contains("`autoreply.message`"), "{msg}");
        }
    }

    #[test]
    fn autoreply_not_a_table_or_with_another_key() {
        let msg = rejected(&(base() + "autoreply = \"Away\"\n"));
        assert!(msg.contains("`autoreply`"), "{msg}");
        let msg = rejected(&autoreply_table("message = \"Away\"\ntext = \"x\"\n"));
        assert!(msg.contains("`autoreply.text`"), "{msg}");
    }

    fn xmpp_table(lines: &str) -> String {
        base() + "[xmpp]\n" + lines
    }

    const XMPP_MINIMAL: &str = "component = \"Russel.SL.example.org\"\nsecret = \"s3cret\"\nowners = [\"me@example.org\"]\n";

    #[test]
    fn xmpp_gateway_configured() {
        let xmpp = Config::parse(&xmpp_table(XMPP_MINIMAL)).unwrap().xmpp.unwrap();
        assert_eq!(xmpp.component, "russel.sl.example.org");
        assert_eq!(xmpp.server, "127.0.0.1:5347");
        assert_eq!(xmpp.secret.expose(), "s3cret");
        assert_eq!(xmpp.owners, ["me@example.org"]);
    }

    #[test]
    fn xmpp_server_port_given() {
        for server in ["localhost:5348", "127.0.0.2:5348", "[::1]:5348", "LOCALHOST:5348"] {
            let text = xmpp_table(&format!("{XMPP_MINIMAL}server = \"{server}\"\n"));
            assert_eq!(Config::parse(&text).unwrap().xmpp.unwrap().server, server);
        }
    }

    #[test]
    fn xmpp_not_mentioned() {
        assert_eq!(Config::parse(&base()).unwrap().xmpp, None);
    }

    #[test]
    fn xmpp_owners_count_once_in_lower_case() {
        let text = xmpp_table(
            "component = \"bot.rig.test\"\nsecret = \"x\"\nowners = [\"Me@Example.org\", \"me@example.org\", \"you@rig.test\"]\n",
        );
        assert_eq!(
            Config::parse(&text).unwrap().xmpp.unwrap().owners,
            ["me@example.org", "you@rig.test"]
        );
    }

    #[test]
    fn xmpp_table_without_owners() {
        for owners in ["", "owners = []\n", "owners = \"me@example.org\"\n", "owners = [1]\n"] {
            let msg = rejected(&xmpp_table(&format!("component = \"bot.rig.test\"\nsecret = \"x\"\n{owners}")));
            assert!(msg.contains("`xmpp.owners`"), "{msg}");
        }
    }

    #[test]
    fn xmpp_owner_with_a_resource() {
        for owner in ["me@example.org/phone", "example.org", "@example.org", "me@", "me@exa mple.org"] {
            let msg = rejected(&xmpp_table(&format!(
                "component = \"bot.rig.test\"\nsecret = \"x\"\nowners = [\"{owner}\"]\n"
            )));
            assert!(
                msg.contains("`xmpp.owners`") && msg.contains(owner) && msg.contains("`user@domain`"),
                "{msg}"
            );
        }
    }

    #[test]
    fn xmpp_component_that_is_an_account() {
        for component in ["bot@example.org", "example.org/res", "", "bad..domain", "-x.org", "a b.org"] {
            let msg = rejected(&xmpp_table(&format!(
                "component = \"{component}\"\nsecret = \"x\"\nowners = [\"me@example.org\"]\n"
            )));
            assert!(msg.contains("`xmpp.component`"), "{msg}");
        }
        let msg = rejected(&xmpp_table("component = 5\nsecret = \"x\"\nowners = [\"me@example.org\"]\n"));
        assert!(msg.contains("`xmpp.component`"), "{msg}");
        let msg = rejected(&xmpp_table("secret = \"x\"\nowners = [\"me@example.org\"]\n"));
        assert!(msg.contains("`xmpp.component`"), "{msg}");
    }

    #[test]
    fn xmpp_server_on_another_host() {
        for server in ["xmpp.example.org:5347", "192.0.2.10:5347", "[2001:db8::1]:5347", "0.0.0.0:5347"] {
            let msg = rejected(&xmpp_table(&format!("{XMPP_MINIMAL}server = \"{server}\"\n")));
            assert!(
                msg.contains("`xmpp.server`") && msg.contains("not encrypted") && msg.contains("this host"),
                "{msg}"
            );
        }
        for server in ["localhost", "localhost:0", "localhost:65536", "localhost:x", "[::1]"] {
            let msg = rejected(&xmpp_table(&format!("{XMPP_MINIMAL}server = \"{server}\"\n")));
            assert!(msg.contains("`xmpp.server`"), "{msg}");
        }
        let msg = rejected(&xmpp_table(&format!("{XMPP_MINIMAL}server = 5347\n")));
        assert!(msg.contains("`xmpp.server`"), "{msg}");
    }

    #[test]
    fn xmpp_secret_must_be_given() {
        for secret in ["", "secret = \"\"\n", "secret = 1\n"] {
            let msg = rejected(&xmpp_table(&format!(
                "component = \"bot.rig.test\"\n{secret}owners = [\"me@example.org\"]\n"
            )));
            assert!(msg.contains("`xmpp.secret`"), "{msg}");
        }
    }

    #[test]
    fn unknown_key_in_the_xmpp_table() {
        let msg = rejected(&xmpp_table(&format!("{XMPP_MINIMAL}password = \"x\"\n")));
        assert!(msg.contains("`xmpp.password`"), "{msg}");
        for key in ["`component`", "`server`", "`secret`", "`owners`"] {
            assert!(msg.contains(key), "{msg}");
        }
        let msg = rejected(&(base() + "xmpp = \"bot.rig.test\"\n"));
        assert!(msg.contains("`xmpp`"), "{msg}");
    }

    #[test]
    fn the_xmpp_secret_stays_out_of_debug_output_and_errors() {
        let secret = "Very-Secret-42";
        let cfg = Config::parse(&xmpp_table(&format!(
            "component = \"bot.rig.test\"\nsecret = \"{secret}\"\nowners = [\"me@example.org\"]\n"
        )))
        .unwrap();
        let shown = format!("{cfg:?} {:?} {:#?}", cfg.xmpp, cfg);
        assert!(!shown.contains(secret), "{shown}");
        assert!(shown.contains("***"));
        for broken in [
            format!("component = \"bot@rig.test\"\nsecret = \"{secret}\"\nowners = [\"me@example.org\"]\n"),
            format!("component = \"bot.rig.test\"\nsecret = \"{secret}\"\nowners = [\"me\"]\n"),
            format!("component = \"bot.rig.test\"\nsecret = \"{secret}\"\nserver = \"10.0.0.1:1\"\nowners = [\"me@x.org\"]\n"),
            format!("component = \"bot.rig.test\"\nsecret = \"{secret}\"\nowners = [\"me@x.org\"]\nother = 1\n"),
        ] {
            let msg = rejected(&xmpp_table(&broken));
            assert!(!msg.contains(secret), "{msg}");
        }
    }

    #[test]
    fn example_file_is_valid_with_its_xmpp_table_enabled() {
        let text = include_str!("../russelbot.example.toml");
        let mut enabled = text.to_string();
        for line in [
            "# [xmpp]",
            "# component = \"russel.sl.example.org\"",
            "# server = \"127.0.0.1:5347\"",
            "# secret = \"the component_secret of the server\"",
            "# owners = [\"me@example.org\"]",
        ] {
            assert_eq!(text.matches(&format!("\n{line}\n")).count(), 1, "{line} is not once in the example");
            enabled = enabled.replace(&format!("\n{line}\n"), &format!("\n{}\n", &line[2..]));
        }
        let xmpp = Config::parse(&enabled).unwrap().xmpp.unwrap();
        assert_eq!(xmpp.component, "russel.sl.example.org");
        assert_eq!(xmpp.server, "127.0.0.1:5347");
        assert_eq!(xmpp.owners, ["me@example.org"]);
    }

    #[test]
    fn example_file_is_valid_with_its_optional_keys_enabled() {
        let text = include_str!("../russelbot.example.toml");
        let mut enabled = text.to_string();
        for key in ["log_chat", "log_im", "fetch_offline_im"] {
            let commented = format!("# {key} = ");
            assert!(text.contains(&commented), "{key} is not in the example");
            enabled = enabled.replace(&commented, &format!("{key} = "));
        }
        let cfg = Config::parse(&enabled).unwrap();
        assert_eq!(cfg.log_chat, Some(PathBuf::from("chat.tsv")));
        assert_eq!(cfg.log_im, Some(PathBuf::from("im.tsv")));
        assert!(cfg.fetch_offline_im);
    }

    // ------------------------------------------------- startup script

    fn script_texts(cfg: &Config) -> Vec<(usize, &str)> {
        cfg.startup.iter().map(|line| (line.number, line.text.as_str())).collect()
    }

    #[test]
    fn startup_is_a_script_of_command_lines() {
        let cfg = Config::parse(&format!(
            "{}startup = '''\n# settings\nrlv mode on\nperms autoaccept tp \"Jane Doe\"\n\npersist cur cfg\n'''\n",
            base()
        ))
        .unwrap();
        assert_eq!(
            script_texts(&cfg),
            [(2, "rlv mode on"), (3, "perms autoaccept tp \"Jane Doe\""), (5, "persist cur cfg")]
        );
        assert_eq!((cfg.startup_key(), &cfg.startup_file), ("startup", &None));
        assert!(cfg.deprecated.is_empty());
        // Without the keys there is no script, and an empty one is none.
        assert!(Config::parse(&base()).unwrap().startup.is_empty());
        assert!(Config::parse(&format!("{}startup = \"\"\n", base())).unwrap().startup.is_empty());
    }

    #[test]
    fn startup_and_startup_file_exclude_each_other() {
        let msg = rejected(&format!("{}startup = \"rlv mode on\"\nstartup_file = \"startup.rsl\"\n", base()));
        assert!(msg.contains("`startup`") && msg.contains("`startup_file`"), "{msg}");
    }

    #[test]
    fn startup_keys_of_the_wrong_type_are_rejected() {
        let msg = rejected(&format!("{}startup = [\"rlv mode on\"]\n", base()));
        assert!(msg.contains("`startup` must be a string"), "{msg}");
        for bad in ["\"\"", "true", "[\"startup.rsl\"]"] {
            let msg = rejected(&format!("{}startup_file = {bad}\n", base()));
            assert!(msg.contains("`startup_file` must be a non-empty file path"), "{bad}: {msg}");
        }
    }

    #[test]
    fn a_script_that_is_not_in_its_form_is_rejected_with_its_line() {
        let with = |script: &str| rejected(&format!("{}startup = '''\n{script}'''\n", base()));
        let msg = with("rlv mode on\nsay hi\nteleport 128 128 25\n");
        assert!(msg.contains("`startup`: line 3: unknown command `teleport`"), "{msg}");
        let msg = with("sit\n");
        assert!(msg.contains("`startup`: line 1: `sit` takes 1 argument"), "{msg}");
        let msg = with("\nrlv mode loud\n");
        assert!(msg.contains("`startup`: line 2: `rlv mode` takes"), "{msg}");
        let msg = with("say one\nim \"Jane Doe hi\nsay two\n");
        assert!(msg.contains("`startup`: line 2:") && msg.contains("quote"), "{msg}");
        let msg = with("help\n");
        assert!(msg.contains("line 1: unknown command `help`"), "{msg}");
        // What only the bot can judge is not judged here.
        assert!(Config::parse(&format!("{}startup = \"sit chair\\nautoreply pause soon\"\n", base())).is_ok());
    }

    /// A config file and, beside it, the file its `startup_file` names.
    struct Files {
        dir: PathBuf,
    }

    impl Files {
        fn new(name: &str) -> Files {
            let dir = std::env::temp_dir().join(format!("russelbot-config-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Files { dir }
        }

        fn script(&self) -> PathBuf {
            self.dir.join("startup.rsl")
        }

        /// Loads `base()` with `startup_file` naming the script file.
        fn load(&self) -> Result<Config, ConfigError> {
            let config = self.dir.join("russelbot.toml");
            std::fs::write(&config, format!("{}startup_file = '{}'\n", base(), self.script().display())).unwrap();
            Config::load(&config)
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_script_file_holds_what_startup_would() {
        const SCRIPT: &str = "# settings\nrlv mode on\n\npersist\n";
        let files = Files::new("file");
        std::fs::write(files.script(), SCRIPT).unwrap();
        let from_file = files.load().unwrap();
        let inline = Config::parse(&format!("{}startup = '''{SCRIPT}'''\n", base())).unwrap();
        assert_eq!(script_texts(&from_file), [(2, "rlv mode on"), (4, "persist")]);
        assert_eq!(from_file.startup_key(), "startup_file");
        // Apart from where the script is, the two configs are the same.
        assert_eq!(
            Config {
                startup_file: None,
                ..from_file
            },
            inline
        );
    }

    #[test]
    fn a_script_file_that_cannot_be_read_is_rejected() {
        let files = Files::new("missing");
        let msg = files.load().unwrap_err().0;
        assert!(msg.contains("russelbot.toml: `startup_file`: cannot read"), "{msg}");
        assert!(msg.contains("startup.rsl"), "{msg}");
        // Neither can one that is not text.
        std::fs::write(files.script(), [b's', b'a', b'y', b' ', 0xff, 0xfe]).unwrap();
        let msg = files.load().unwrap_err().0;
        assert!(msg.contains("`startup_file`: cannot read") && msg.contains("UTF-8"), "{msg}");
    }

    #[test]
    fn a_script_file_that_is_not_in_its_form_is_rejected_with_its_line() {
        let files = Files::new("form");
        std::fs::write(files.script(), "say one\nim \"Jane Doe hi\n").unwrap();
        let msg = files.load().unwrap_err().0;
        assert!(msg.contains("`startup_file`: line 2:") && msg.contains("quote"), "{msg}");
    }

    // --------------------------------------------------- deprecations

    fn notes(more: &str) -> Vec<String> {
        Config::parse(&format!("{}{more}", base())).unwrap().deprecated
    }

    #[test]
    fn a_config_without_deprecated_keys_has_no_notes() {
        assert_eq!(notes(""), Vec::<String>::new());
        assert_eq!(
            notes("socket = true\nstartup = \"rlv mode on\"\n[xmpp]\ncomponent = \"a.b\"\nsecret = \"s\"\nowners = [\"me@b\"]\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn deprecated_persist_is_named_with_the_persist_command() {
        let said = notes("persist = [\"cfg\", \"cur\"]\n");
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("`persist` is deprecated; "), "{}", said[0]);
        assert!(said[0].contains("`persist cfg cur`") && said[0].contains("startup"), "{}", said[0]);
        assert!(notes("persist = \"cur\"\n")[0].contains("`persist cur cur`"));
        assert!(notes("persist = \"cfg\"\n")[0].contains("`persist cfg cfg`"));
        assert!(notes("[persist]\nsit = \"cfg\"\n")[0].contains("`persist cur cfg`"));
        assert!(notes("[persist]\natstart = true\n")[0].contains("`persist cur cur`"));
    }

    #[test]
    fn a_persist_that_holds_nothing_can_be_left_out() {
        for off in [
            "persist = \"off\"\n",
            "persist = false\n",
            "[persist]\n",
            "[persist]\natstart = false\n",
        ] {
            let said = notes(off);
            assert_eq!(said.len(), 1, "{off}");
            assert!(
                said[0].contains("leave it out") && !said[0].contains("`persist "),
                "{off}: {}",
                said[0]
            );
        }
    }

    #[test]
    fn the_persist_tables_rlv_keys_are_named_with_their_commands() {
        let said = notes("[persist]\noverride = [\"sit\", \"tp\"]\nrlv = \"follow\"\n");
        assert_eq!(said.len(), 1);
        assert!(said[0].contains("`rlv persist follow`"), "{}", said[0]);
        assert!(said[0].contains("`rlv persist override sit tp`"), "{}", said[0]);
        assert!(!said[0].contains("`persist c"), "{}", said[0]);
        let said = notes("[persist]\nposition = \"cfg\"\nrlv = \"drop\"\noverride = []\n");
        assert!(said[0].contains("`rlv persist drop`"), "{}", said[0]);
        assert!(said[0].contains("`rlv persist override none`"), "{}", said[0]);
        // The setting commands come first.
        let (settings, persist) = (said[0].find("`rlv persist").unwrap(), said[0].find("`persist cfg cur`").unwrap());
        assert!(settings < persist, "{}", said[0]);
        // Every command named is one the script takes.
        for command in [
            "rlv persist follow",
            "rlv persist override sit tp",
            "rlv persist override none",
            "persist cfg cur",
        ] {
            assert!(startup::parse(command).is_ok(), "{command}");
        }
    }

    #[test]
    fn deprecated_rlv_is_named_with_rlv_mode_on() {
        assert_eq!(
            notes("rlv = true\n"),
            ["`rlv` is deprecated; write the startup script command `rlv mode on` instead (see `startup`)"]
        );
        let said = notes("rlv = false\n");
        assert!(
            said[0].starts_with("`rlv` is deprecated") && said[0].contains("leave it out"),
            "{}",
            said[0]
        );
    }

    #[test]
    fn deprecated_perms_are_named_with_the_perms_commands() {
        let said = notes(
            "[perms.autoaccept]\ntp = [\"Jane Doe\", \"sitbot\"]\nanim = \"all\"\nattach = true\ncontrols = false\n\
             [perms.except]\nanim = [\"01234567-9abc-defe-dcba-987654321000\", \"John Roe\"]\n",
        );
        assert_eq!(said.len(), 1);
        let said = &said[0];
        assert!(said.starts_with("`perms` is deprecated; "), "{said}");
        for command in [
            "perms autoaccept tp \"Jane Doe\" sitbot",
            "perms autoaccept anim all",
            "perms revoke anim 01234567-9abc-defe-dcba-987654321000 \"John Roe\"",
            "perms autoaccept attach all",
        ] {
            assert!(said.contains(&format!("`{command}`")), "{command}: {said}");
            assert!(startup::parse(command).is_ok(), "{command}");
        }
        assert!(!said.contains("controls") && !said.contains("cannot be written"), "{said}");
        // The exceptions come after the rule they are exceptions of.
        assert!(
            said.find("autoaccept anim all").unwrap() < said.find("revoke anim").unwrap(),
            "{said}"
        );
    }

    #[test]
    fn an_exception_under_a_list_rule_is_named_as_having_no_command() {
        let said = notes(
            "[perms.autoaccept]\nanim = [\"Jane Doe\"]\n[perms.except]\nanim = [\"01234567-9abc-defe-dcba-987654321000\"]\n\
             tp = [\"John Roe\"]\n",
        );
        assert!(said[0].contains("`perms autoaccept anim \"Jane Doe\"`"), "{}", said[0]);
        assert!(
            said[0].contains("the exceptions for `tp`, `anim` cannot be written in the startup script"),
            "{}",
            said[0]
        );
        assert!(!said[0].replace("`perms revoke` makes", "").contains("perms revoke"), "{}", said[0]);
    }

    #[test]
    fn deprecated_autoreply_is_named_with_the_autoreply_commands() {
        let said = notes("[autoreply]\nmessage = \"Away\"\npause = \"2h\"\n");
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("`autoreply` is deprecated; "), "{}", said[0]);
        assert!(
            said[0].contains("`autoreply message Away`") && said[0].contains("`autoreply pause 2h`"),
            "{}",
            said[0]
        );
        // Without a pause there is only the text, in quotes when it needs them.
        let said = notes("[autoreply]\nmessage = \"I am \\\"a bot\\\".  Away\"\n");
        assert!(!said[0].contains("autoreply pause"), "{}", said[0]);
        let command = "autoreply message \"I am \\\"a bot\\\".  Away\"";
        assert!(said[0].contains(&format!("`{command}`")), "{}", said[0]);
        let script = startup::parse(command).unwrap();
        assert_eq!(
            script[0].request,
            crate::control::Request::Autoreply(crate::control::AutoreplyRequest::Message("I am \"a bot\".  Away".to_string()))
        );
    }

    #[test]
    fn every_deprecated_key_has_its_own_note() {
        let said = notes("persist = \"cfg\"\nrlv = true\n[perms.autoaccept]\ntp = \"all\"\n[autoreply]\nmessage = \"Away\"\n").join("\n");
        for key in ["`persist` is", "`rlv` is", "`perms` is", "`autoreply` is"] {
            assert_eq!(said.matches(key).count(), 1, "{key}: {said}");
        }
        let old = Config::parse(&format!("{}rlv = true\n", base()).replace("sit = ", "object = ")).unwrap();
        assert_eq!(old.deprecated.len(), 2);
        assert!(old.deprecated[0].starts_with("`object`") && old.deprecated[1].starts_with("`rlv`"));
    }
}
