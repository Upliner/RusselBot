//! The single TOML config file: model, defaults and validation.

use md5::{Digest, Md5};
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const DEFAULT_LOGIN_URL: &str = "https://login.agni.lindenlab.com/cgi-bin/login.cgi";
pub const DEFAULT_CONFIG_PATH: &str = "slbot.toml";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub login_url: String,
    pub first_name: String,
    pub last_name: String,
    /// Always in `$1$<md5 hex>` form, whichever way it was configured.
    pub password_hash: String,
    pub region: String,
    pub position: [f32; 3],
    pub object: Uuid,
    /// TSV file for local chat, if any.
    pub log_chat: Option<PathBuf>,
    /// TSV file for instant messages, if any.
    pub log_im: Option<PathBuf>,
    pub fetch_offline_im: bool,
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
    object: Option<String>,
    // Kept untyped so that a wrong type is reported with the key's name.
    log_chat: Option<toml::Value>,
    log_im: Option<toml::Value>,
    fetch_offline_im: Option<toml::Value>,
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
    s.strip_prefix("$1$").is_some_and(|hex| {
        hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

fn required<T>(value: Option<T>, key: &str) -> Result<T, ConfigError> {
    value.ok_or_else(|| ConfigError(format!("`{key}` is required")))
}

fn log_path(value: Option<toml::Value>, key: &str) -> Result<Option<PathBuf>, ConfigError> {
    match value {
        None => Ok(None),
        Some(toml::Value::String(path)) if !path.is_empty() => Ok(Some(PathBuf::from(path))),
        Some(_) => err(format!("`{key}` must be a non-empty file path")),
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
        Config::parse(&text).map_err(|e| ConfigError(format!("{}: {e}", path.display())))
    }

    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let raw: Raw = toml::from_str(text).map_err(|e| ConfigError(e.message().to_string()))?;

        let login_url = raw.login_url.unwrap_or_else(|| DEFAULT_LOGIN_URL.to_string());
        let rest = login_url
            .strip_prefix("http://")
            .or_else(|| login_url.strip_prefix("https://"));
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
            (Some(_), Some(_)) => {
                return err("`password` and `password_hash` are both set; use only one")
            }
            (None, None) => return err("one of `password` or `password_hash` is required"),
            (Some(plain), None) => {
                if plain.is_empty() {
                    return err("`password` must not be empty");
                }
                hash_password(&plain)
            }
            (None, Some(hash)) => {
                if !is_password_hash(&hash) {
                    return err(
                        "`password_hash` must be `$1$` followed by 32 lowercase hex digits, \
                         as printed by `slbot hash`",
                    );
                }
                hash
            }
        };

        let region = required(raw.region, "region")?.trim().to_string();
        if region.is_empty() {
            return err("`region` must not be empty");
        }

        let position = required(raw.position, "position")?;
        let position = match position[..] {
            [x, y, z] if position.iter().all(|v| v.is_finite()) => [x as f32, y as f32, z as f32],
            _ => return err("`position` must be an array of three finite numbers [x, y, z]"),
        };

        let object = required(raw.object, "object")?;
        let object = Uuid::parse_str(object.trim())
            .map_err(|_| ConfigError("`object` is not a valid UUID".to_string()))?;

        let log_chat = log_path(raw.log_chat, "log_chat")?;
        let log_im = log_path(raw.log_im, "log_im")?;
        let fetch_offline_im = match raw.fetch_offline_im {
            None => false,
            Some(toml::Value::Boolean(on)) => on,
            Some(_) => return err("`fetch_offline_im` must be true or false"),
        };

        Ok(Config {
            login_url,
            first_name,
            last_name,
            password_hash,
            region,
            position,
            object,
            log_chat,
            log_im,
            fetch_offline_im,
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
             position = [128.0, 128.5, 25]\nobject = \"{OBJECT}\"\n"
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
        assert_eq!(cfg.position, [128.0, 128.5, 25.0]);
        assert_eq!(cfg.object, Uuid::parse_str(OBJECT).unwrap());
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
    fn rejects_missing_required_keys() {
        for key in ["user", "region", "position", "object"] {
            let text: String = base()
                .lines()
                .filter(|l| !l.starts_with(key))
                .map(|l| format!("{l}\n"))
                .collect();
            assert!(rejected(&text).contains(&format!("`{key}`")), "{key}");
        }
    }

    #[test]
    fn rejects_three_word_user() {
        assert!(rejected(&base().replace("Sit Bot", "A B C")).contains("`user`"));
    }

    #[test]
    fn rejects_bad_object_uuid() {
        assert!(rejected(&base().replace(OBJECT, "not-a-uuid")).contains("`object`"));
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
        let cfg = Config::parse(include_str!("../slbot.example.toml")).unwrap();
        assert_eq!(cfg.login_url, "http://127.0.0.1:9000/");
    }

    #[test]
    fn example_file_is_valid_with_its_optional_keys_enabled() {
        let text = include_str!("../slbot.example.toml");
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
}
