//! Grid login over XML-RPC: request, reply parsing and outcome classification.

use crate::config::{Config, Place};
use crate::xml::{self, Value};
use md5::{Digest, Md5};
use std::net::{IpAddr, SocketAddr};
use uuid::Uuid;

pub const CHANNEL: &str = "RusselBot";
const MAX_REDIRECTS: usize = 5;

/// Where the login asks the grid to place the avatar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Start {
    /// The configured region and position.
    Target,
    /// The avatar's home location.
    Home,
    /// Wherever the avatar was last.
    Last,
}

/// What a successful login hands over to the simulator session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub circuit_code: u32,
    pub sim_addr: SocketAddr,
    pub seed_capability: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Success(SessionInfo),
    /// The grid demands something the bot cannot supply.
    NeedsOperator(Failure),
    /// The grid rejected the attempt with its credential-or-location reason.
    Refused(Failure),
    /// Anything else; worth retrying.
    Temporary(Failure),
}

#[derive(Debug, PartialEq)]
enum Reply {
    Success(SessionInfo),
    Redirect(String),
    Failure(Failure),
}

fn md5_hex(text: &str) -> String {
    Md5::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn start_location(cfg: &Config, start: Start) -> String {
    match start {
        Start::Last => "last".to_string(),
        Start::Home => "home".to_string(),
        Start::Target => match &cfg.place {
            Place::Region { name, position } => {
                let [x, y, z] = position.map(|v| v.round() as i32);
                format!("uri:{name}&{x}&{y}&{z}")
            }
            // Nothing to aim for: the reconciler asks for this only with a
            // configured region.
            Place::Home | Place::Latest => "last".to_string(),
        },
    }
}

pub fn request_xml(cfg: &Config, start: Start) -> String {
    let str = |s: &str| Value::Str(s.to_string());
    // Stable per-account hardware identifiers, so the bot presents the same
    // identity to the grid on every login.
    let identity = format!("{} {}", cfg.first_name, cfg.last_name).to_lowercase();
    xml::write_xmlrpc_call(
        "login_to_simulator",
        &[
            ("first", str(&cfg.first_name)),
            ("last", str(&cfg.last_name)),
            ("passwd", str(&cfg.password_hash)),
            ("start", Value::Str(start_location(cfg, start))),
            ("channel", str(CHANNEL)),
            ("version", str(env!("CARGO_PKG_VERSION"))),
            ("platform", str("Lnx")),
            ("mac", Value::Str(md5_hex(&format!("rslbot-mac:{identity}")))),
            ("id0", Value::Str(md5_hex(&format!("rslbot-id0:{identity}")))),
            ("agree_to_tos", Value::Bool(true)),
            ("read_critical", Value::Bool(true)),
            ("options", Value::Array(Vec::new())),
        ],
    )
}

fn parse_reply(body: &str) -> Result<Reply, String> {
    let reply = xml::parse_xmlrpc_response(body).map_err(|e| e.to_string())?;
    let text = |key: &str| reply.str_field(key).unwrap_or("").to_string();
    // `login` is a string on every grid seen so far, but tolerate a boolean.
    let login = match reply.get("login") {
        Some(Value::Bool(true)) => "true",
        Some(Value::Bool(false)) => "false",
        Some(other) => other.as_str().unwrap_or(""),
        None => return Err("reply has no `login` field".to_string()),
    };
    match login {
        "true" => {
            let missing = |key: &str| format!("successful reply has no usable `{key}`");
            let uuid = |key: &str| reply.get(key).and_then(Value::as_uuid).ok_or(missing(key));
            let int = |key: &str| reply.get(key).and_then(Value::as_i64).ok_or(missing(key));
            let ip: IpAddr = text("sim_ip").parse().map_err(|_| missing("sim_ip"))?;
            let port = u16::try_from(int("sim_port")?).map_err(|_| missing("sim_port"))?;
            let seed_capability = text("seed_capability");
            if seed_capability.is_empty() {
                return Err(missing("seed_capability"));
            }
            Ok(Reply::Success(SessionInfo {
                agent_id: uuid("agent_id")?,
                session_id: uuid("session_id")?,
                // Circuit codes are unsigned on the wire but some grids send
                // them as signed XML-RPC ints.
                circuit_code: int("circuit_code")? as u32,
                sim_addr: SocketAddr::new(ip, port),
                seed_capability,
            }))
        }
        "indeterminate" => match reply.str_field("next_url") {
            Some(url) if !url.is_empty() => Ok(Reply::Redirect(url.to_string())),
            _ => Err("indeterminate reply has no `next_url`".to_string()),
        },
        _ => Ok(Reply::Failure(Failure { reason: text("reason"), message: text("message") })),
    }
}

fn classify(failure: Failure) -> Outcome {
    match failure.reason.as_str() {
        "mfa_challenge" => Outcome::NeedsOperator(failure),
        "key" => Outcome::Refused(failure),
        _ => Outcome::Temporary(failure),
    }
}

fn temporary(reason: &str, message: impl Into<String>) -> Outcome {
    Outcome::Temporary(Failure { reason: reason.to_string(), message: message.into() })
}

/// Turns one HTTP exchange into an outcome, or the URL to continue at.
fn interpret(http: Result<(u16, String), String>) -> Result<Outcome, String> {
    let (status, body) = match http {
        Ok(pair) => pair,
        Err(e) => return Ok(temporary("network", e)),
    };
    if status != 200 {
        return Ok(temporary("http", format!("login server answered HTTP {status}")));
    }
    Ok(match parse_reply(&body) {
        Ok(Reply::Success(info)) => Outcome::Success(info),
        Ok(Reply::Redirect(url)) => return Err(url),
        Ok(Reply::Failure(failure)) => classify(failure),
        Err(e) => temporary("malformed", e),
    })
}

/// Logs in, following the login server's redirects.
pub fn login(agent: &ureq::Agent, cfg: &Config, start: Start) -> Outcome {
    let request = request_xml(cfg, start);
    let mut url = cfg.login_url.clone();
    for _ in 0..=MAX_REDIRECTS {
        let http = agent
            .post(&url)
            .content_type("text/xml")
            .send(request.as_str())
            .map_err(|e| e.to_string())
            .and_then(|mut response| {
                let status = response.status().as_u16();
                let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
                Ok((status, body))
            });
        match interpret(http) {
            Ok(outcome) => return outcome,
            Err(next_url) => url = next_url,
        }
    }
    temporary("redirect", "login server kept redirecting")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::parse(
            "user = \"Sit Bot\"\npassword = \"secret\"\nregion = \"Target Region\"\n\
             position = [128.4, 60.6, 25]\nsit = \"11111111-2222-3333-4444-555555555555\"\n",
        )
        .unwrap()
    }

    fn member(name: &str, typed: &str) -> String {
        format!("<member><name>{name}</name><value>{typed}</value></member>")
    }

    fn reply(members: &[String]) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><methodResponse><params><param><value><struct>{}\
             </struct></value></param></params></methodResponse>",
            members.concat()
        )
    }

    /// Shaped like OpenSim's LLLoginResponse: strings for ids, ints for port
    /// and circuit code.
    fn success_reply() -> String {
        reply(&[
            member("login", "<string>true</string>"),
            member("agent_id", "<string>aaaaaaaa-0000-0000-0000-000000000001</string>"),
            member("session_id", "<string>bbbbbbbb-0000-0000-0000-000000000002</string>"),
            member("secure_session_id", "<string>cccccccc-0000-0000-0000-000000000003</string>"),
            member("circuit_code", "<int>123456789</int>"),
            member("sim_ip", "<string>127.0.0.1</string>"),
            member("sim_port", "<int>9000</int>"),
            member("region_x", "<int>256000</int>"),
            member("region_y", "<int>256000</int>"),
            member("seed_capability", "<string>http://127.0.0.1:9000/CAPS/abc0000/</string>"),
            member("message", "<string>Welcome</string>"),
        ])
    }

    fn failure_reply(reason: &str, message: &str) -> String {
        reply(&[
            member("login", "<string>false</string>"),
            member("reason", &format!("<string>{reason}</string>")),
            member("message", &format!("<string>{message}</string>")),
        ])
    }

    fn outcome(body: String) -> Outcome {
        interpret(Ok((200, body))).unwrap()
    }

    #[test]
    fn request_carries_credentials_and_start_location() {
        let xml = request_xml(&config(), Start::Target);
        let value = |name: &str| {
            let tail = xml.split(&format!("<name>{name}</name><value>")).nth(1).unwrap();
            tail.split("</value>").next().unwrap().to_string()
        };
        assert!(xml.contains("<methodName>login_to_simulator</methodName>"));
        assert_eq!(value("first"), "<string>Sit</string>");
        assert_eq!(value("last"), "<string>Bot</string>");
        assert_eq!(value("passwd"), "<string>$1$5ebe2294ecd0e0f08eab7690d2a6ee69</string>");
        assert_eq!(value("start"), "<string>uri:Target Region&amp;128&amp;61&amp;25</string>");
        assert_eq!(value("channel"), "<string>RusselBot</string>");
        assert_eq!(value("agree_to_tos"), "<boolean>1</boolean>");
        assert!(!xml.contains("secret"));
    }

    #[test]
    fn request_for_last_location() {
        assert!(request_xml(&config(), Start::Last).contains("<string>last</string>"));
        assert!(request_xml(&config(), Start::Home).contains("<string>home</string>"));
        // Without a region there is nothing else to ask for.
        let bare = Config::parse("user = \"Sit Bot\"\npassword = \"secret\"\n").unwrap();
        assert!(request_xml(&bare, Start::Target).contains("<string>last</string>"));
    }

    #[test]
    fn identity_is_stable_per_account() {
        assert_eq!(request_xml(&config(), Start::Last), request_xml(&config(), Start::Last));
    }

    #[test]
    fn parses_success_reply() {
        let Outcome::Success(info) = outcome(success_reply()) else { panic!("not a success") };
        assert_eq!(info.agent_id.to_string(), "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(info.session_id.to_string(), "bbbbbbbb-0000-0000-0000-000000000002");
        assert_eq!(info.circuit_code, 123_456_789);
        assert_eq!(info.sim_addr, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(info.seed_capability, "http://127.0.0.1:9000/CAPS/abc0000/");
    }

    /// A reply recorded from the local OpenSim rig. It carries a second,
    /// nested `agent_id` (the library owner's) that must not be picked up.
    #[test]
    fn parses_recorded_opensim_reply() {
        let body = include_str!("testdata/opensim_login_reply.xml").to_string();
        let Outcome::Success(info) = outcome(body) else { panic!("not a success") };
        assert_eq!(info.agent_id.to_string(), "7e570000-0000-4000-8000-0000000000aa");
        assert_eq!(info.session_id.to_string(), "e62a055a-78db-4007-b506-dc4c2742c694");
        assert_eq!(info.circuit_code, 304_247_186);
        assert_eq!(info.sim_addr, "127.0.0.1:9001".parse().unwrap());
        assert_eq!(
            info.seed_capability,
            "http://127.0.0.1:9000/CAPS/97f4ce1f-1dc5-4399-a41a-eaadb2f2609c0000/"
        );
    }

    #[test]
    fn key_is_refused() {
        let out = outcome(failure_reply("key", "Could not authenticate your avatar."));
        assert!(matches!(out, Outcome::Refused(f) if f.reason == "key" && f.message.contains("authenticate")));
    }

    #[test]
    fn presence_is_temporary() {
        let out = outcome(failure_reply("presence", "You appear to be already logged in."));
        assert!(matches!(out, Outcome::Temporary(f) if f.reason == "presence"));
    }

    #[test]
    fn mfa_challenge_needs_operator() {
        let out = outcome(failure_reply("mfa_challenge", "Enter your token"));
        assert!(matches!(out, Outcome::NeedsOperator(_)));
    }

    #[test]
    fn indeterminate_redirects_to_next_url() {
        let body = reply(&[
            member("login", "<string>indeterminate</string>"),
            member("next_url", "<string>https://login2.example/cgi-bin/login.cgi</string>"),
        ]);
        assert_eq!(
            interpret(Ok((200, body))),
            Err("https://login2.example/cgi-bin/login.cgi".to_string())
        );
    }

    #[test]
    fn malformed_reply_is_temporary() {
        for body in ["<html>Bad gateway</html>".to_string(), reply(&[]), String::new()] {
            assert!(matches!(outcome(body), Outcome::Temporary(f) if f.reason == "malformed"));
        }
        // A "success" that lacks session parameters is unusable, not fatal.
        let partial = reply(&[member("login", "<string>true</string>")]);
        assert!(matches!(outcome(partial), Outcome::Temporary(f) if f.reason == "malformed"));
    }

    #[test]
    fn connection_and_http_errors_are_temporary() {
        let refused = interpret(Err("connection refused".to_string())).unwrap();
        assert!(matches!(refused, Outcome::Temporary(f) if f.reason == "network"));
        let busy = interpret(Ok((503, String::new()))).unwrap();
        assert!(matches!(busy, Outcome::Temporary(f) if f.reason == "http"));
    }
}
