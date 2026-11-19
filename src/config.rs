//! The single TOML config file: model, defaults and validation.

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
    /// The seat came from the old key `object`.
    pub deprecated_object: bool,
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
     [position, sit] of \"cur\" and \"cfg\", or a table with the keys `atstart`, `position`, `sit` and `rlv`";

fn source(value: &toml::Value) -> Option<Source> {
    match value.as_str() {
        Some("cur") => Some(Source::Cur),
        Some("cfg") => Some(Source::Cfg),
        _ => None,
    }
}

/// The config's `persist`: what the bot holds once the config placement is
/// reached, and the RLV persist setting, which only the table form can set.
fn persist(value: Option<toml::Value>) -> Result<(Persist, RlvPersist), ConfigError> {
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
    Ok((setting, RlvPersist::Drop))
}

/// The table form of `persist`. `atstart` says whether anything is held
/// once the config placement is reached; `position` and `sit` imply it.
fn persist_table(table: &toml::Table) -> Result<(Persist, RlvPersist), ConfigError> {
    let (mut position, mut sit, mut atstart, mut rlv) = (None, None, None, RlvPersist::Drop);
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
            _ => {
                return err(format!(
                    "`persist` has no key `{key}`; its keys are `atstart`, `position`, `sit` and `rlv`"
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
    Ok((setting, rlv))
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
        Config::parse(&text).map_err(|e| ConfigError(format!("{}: {e}", path.display())))
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

        let deprecated_object = raw.object.is_some();
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

        let (persist, rlv_persist) = persist(raw.persist)?;
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
        let rlv = flag(raw.rlv, "rlv")?;

        Ok(Config {
            login_url,
            first_name,
            last_name,
            password_hash,
            place,
            sit,
            deprecated_object,
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
        assert!(!cfg.deprecated_object);
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
        assert_eq!((cfg.place, cfg.sit, cfg.deprecated_object), (Place::Latest, None, false));
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
        assert!(old.deprecated_object);
        let new = Config::parse(&base()).unwrap();
        assert_eq!(
            Config {
                deprecated_object: false,
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
        assert!(!cfg.deprecated_object);
        assert_eq!(cfg.persist, Persist::Off);
        assert!(!cfg.rlv);
        assert_eq!(cfg.rlv_persist, RlvPersist::Drop);
    }

    #[test]
    fn example_file_is_valid_with_its_persist_table_enabled() {
        let text = include_str!("../russelbot.example.toml");
        let mut enabled = text.to_string();
        for line in [
            "# [persist]",
            "# atstart = true",
            "# position = \"cur\"",
            "# sit = \"cfg\"",
            "# rlv = \"follow\"",
            "# rlv = true",
        ] {
            assert_eq!(text.matches(&format!("\n{line}\n")).count(), 1, "{line} is not once in the example");
            enabled = enabled.replace(&format!("\n{line}\n"), &format!("\n{}\n", &line[2..]));
        }
        let cfg = Config::parse(&enabled).unwrap();
        assert_eq!(
            cfg.persist,
            Persist::On {
                position: Source::Cur,
                sit: Source::Cfg
            }
        );
        assert!(cfg.rlv);
        assert_eq!(cfg.rlv_persist, RlvPersist::Follow);
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
        let persist = "# persist = \"cfg\"";
        assert!(text.contains(persist), "persist is not in the example");
        enabled = enabled.replace(persist, "persist = \"cfg\"");
        let cfg = Config::parse(&enabled).unwrap();
        assert_eq!(
            cfg.persist,
            Persist::On {
                position: Source::Cfg,
                sit: Source::Cfg
            }
        );
        assert_eq!(cfg.log_chat, Some(PathBuf::from("chat.tsv")));
        assert_eq!(cfg.log_im, Some(PathBuf::from("im.tsv")));
        assert!(cfg.fetch_offline_im);
    }
}
