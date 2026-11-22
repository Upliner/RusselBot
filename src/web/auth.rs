//! Who may use the web interface: the password, the sessions of the
//! browsers that gave it, and the brake on wrong passwords. The clock is
//! passed in, so that all of it can be tested.

use crate::config::Secret;
use sha1::{Digest, Sha1};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How many sessions are kept; a further one ends the one unused longest.
pub const MAX_SESSIONS: usize = 32;
/// How long a session lasts without being used.
pub const IDLE_LIMIT: Duration = Duration::from_secs(30 * 24 * 3600);
/// This many wrong passwords within `WRONG_WINDOW` refuse every login
/// until `WRONG_WINDOW` has passed since the last of them.
pub const WRONG_LIMIT: usize = 5;
pub const WRONG_WINDOW: Duration = Duration::from_secs(60);

/// What became of a login.
#[derive(Debug, Clone, PartialEq)]
pub enum Login {
    /// The token of the new session.
    Session(String),
    Wrong,
    /// Refused unlooked at; `first` for the first refusal since logins
    /// were last looked at.
    TooMany {
        first: bool,
    },
    /// The system gave no random bytes for a token.
    Failed(String),
}

struct State {
    /// Token and when it was last used.
    sessions: Vec<(String, Instant)>,
    /// When the last wrong passwords were given, oldest first, at most
    /// `WRONG_LIMIT`.
    wrong: Vec<Instant>,
    /// A login has been refused for too many attempts since one was last
    /// looked at.
    refusing: bool,
}

pub struct Auth {
    password: Option<Secret>,
    cookie: String,
    state: Mutex<State>,
}

/// Whether two texts are the same, in a time that depends on neither:
/// their digests have one length and are compared to the end.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (Sha1::digest(a.as_bytes()), Sha1::digest(b.as_bytes()));
    a.iter().zip(b.iter()).fold(0, |differ, (x, y)| differ | (x ^ y)) == 0
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("no random bytes for a session: {e}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Auth {
    /// `port` goes into the cookie's name: browsers share cookies between
    /// the ports of a host, and two bots on one machine would otherwise
    /// log each other out.
    pub fn new(password: Option<Secret>, port: u16) -> Auth {
        Auth {
            password,
            cookie: format!("rslbot_{port}"),
            state: Mutex::new(State {
                sessions: Vec::new(),
                wrong: Vec::new(),
                refusing: false,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether a browser has to log in at all.
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// A browser gives `given` as the password.
    pub fn login(&self, given: &str, now: Instant) -> Login {
        self.login_with(given, now, random_token)
    }

    fn login_with(&self, given: &str, now: Instant, token: impl FnOnce() -> Result<String, String>) -> Login {
        let Some(password) = &self.password else {
            return Login::Wrong;
        };
        let mut state = self.state();
        let many = state.wrong.len() >= WRONG_LIMIT && state.wrong[WRONG_LIMIT - 1] - state.wrong[0] <= WRONG_WINDOW;
        let since = state.wrong.last().map(|last| now.saturating_duration_since(*last));
        if many && since.is_some_and(|since| since < WRONG_WINDOW) {
            // Not looked at: the right password is refused as well, and a
            // wrong one does not make the wait longer.
            let first = !std::mem::replace(&mut state.refusing, true);
            return Login::TooMany { first };
        }
        state.refusing = false;
        if !same(given, password.expose()) {
            state.wrong.push(now);
            if state.wrong.len() > WRONG_LIMIT {
                state.wrong.remove(0);
            }
            return Login::Wrong;
        }
        let token = match token() {
            Ok(token) => token,
            Err(reason) => return Login::Failed(reason),
        };
        state.sessions.retain(|(_, used)| now.saturating_duration_since(*used) < IDLE_LIMIT);
        if state.sessions.len() >= MAX_SESSIONS {
            let oldest = state
                .sessions
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(index, _)| index);
            if let Some(index) = oldest {
                state.sessions.remove(index);
            }
        }
        state.sessions.push((token.clone(), now));
        Login::Session(token)
    }

    /// Whether `token` is a session, which counts as using it. Without a
    /// password everyone has one.
    pub fn allows(&self, token: Option<&str>, now: Instant) -> bool {
        if self.password.is_none() {
            return true;
        }
        let Some(token) = token else {
            return false;
        };
        let mut state = self.state();
        state.sessions.retain(|(_, used)| now.saturating_duration_since(*used) < IDLE_LIMIT);
        match state.sessions.iter_mut().find(|(known, _)| known == token) {
            Some((_, used)) => {
                *used = now;
                true
            }
            None => false,
        }
    }

    /// Ends the session `token`, if it is one. Whether it was.
    pub fn logout(&self, token: &str) -> bool {
        let mut state = self.state();
        let before = state.sessions.len();
        state.sessions.retain(|(known, _)| known != token);
        state.sessions.len() < before
    }

    /// The session token in a `Cookie` header, if it has one.
    pub fn token<'a>(&self, cookies: Option<&'a str>) -> Option<&'a str> {
        let pairs = cookies?.split(';').filter_map(|pair| pair.trim().split_once('='));
        let token = pairs.filter(|(name, _)| *name == self.cookie).map(|(_, value)| value).next()?;
        // A token is 64 hex digits; anything else is not worth a lookup.
        (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(token)
    }

    /// The `Set-Cookie` value that gives a browser the session `token`:
    /// not readable by scripts, and not sent with what other sites ask
    /// the browser to send.
    pub fn set_cookie(&self, token: &str) -> String {
        format!(
            "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
            self.cookie,
            IDLE_LIMIT.as_secs()
        )
    }

    /// The `Set-Cookie` value that takes the session away again.
    pub fn clear_cookie(&self) -> String {
        format!("{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0", self.cookie)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "s3cret";

    fn auth() -> Auth {
        Auth::new(Some(Secret::new(PASSWORD)), 8080)
    }

    fn session(auth: &Auth, now: Instant) -> String {
        match auth.login(PASSWORD, now) {
            Login::Session(token) => token,
            other => panic!("no session: {other:?}"),
        }
    }

    #[test]
    fn the_right_password_gives_a_session() {
        let (auth, now) = (auth(), Instant::now());
        assert!(auth.has_password());
        assert!(!auth.allows(None, now));
        let token = session(&auth, now);
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(auth.allows(Some(&token), now));
        assert!(!auth.allows(Some("0000"), now));
        // Each login has a token of its own.
        assert_ne!(session(&auth, now), token);
    }

    #[test]
    fn passwords_are_compared_in_full() {
        let (auth, now) = (auth(), Instant::now());
        let wrong = ["", "s", "s3cre", "s3cret ", "S3CRET", "s3cret\0", "another password altogether"];
        for (n, wrong) in wrong.into_iter().enumerate() {
            // Far enough apart never to be too many.
            let then = now + WRONG_WINDOW * 2 * n as u32;
            assert_eq!(auth.login(wrong, then), Login::Wrong, "{wrong:?}");
        }
        assert!(same("", ""));
        assert!(!same("a", "b"));
    }

    #[test]
    fn without_a_password_everyone_is_let_in_and_nobody_logs_in() {
        let (auth, now) = (Auth::new(None, 8080), Instant::now());
        assert!(!auth.has_password());
        assert!(auth.allows(None, now));
        assert!(auth.allows(Some("anything"), now));
        assert_eq!(auth.login("", now), Login::Wrong);
    }

    #[test]
    fn logging_out_ends_one_session() {
        let (auth, now) = (auth(), Instant::now());
        let (phone, desktop) = (session(&auth, now), session(&auth, now));
        assert!(auth.logout(&phone));
        assert!(!auth.allows(Some(&phone), now));
        assert!(auth.allows(Some(&desktop), now));
        assert!(!auth.logout(&phone));
    }

    #[test]
    fn the_thirty_third_session_ends_the_one_unused_longest() {
        let (auth, start) = (auth(), Instant::now());
        let second = Duration::from_secs(1);
        let tokens: Vec<String> = (0..MAX_SESSIONS as u32).map(|n| session(&auth, start + second * n)).collect();
        // The first is used again, so the second is now the oldest.
        assert!(auth.allows(Some(&tokens[0]), start + second * 40));
        let further = session(&auth, start + second * 41);
        let now = start + second * 42;
        assert!(!auth.allows(Some(&tokens[1]), now));
        assert!(auth.allows(Some(&tokens[0]), now));
        assert!(auth.allows(Some(&tokens[2]), now));
        assert!(auth.allows(Some(&further), now));
    }

    #[test]
    fn a_session_ends_after_thirty_days_without_use() {
        let (auth, start) = (auth(), Instant::now());
        let day = Duration::from_secs(24 * 3600);
        let (used, unused) = (session(&auth, start), session(&auth, start));
        assert!(auth.allows(Some(&used), start + day * 20));
        assert!(auth.allows(Some(&used), start + day * 45));
        assert!(!auth.allows(Some(&unused), start + day * 31));
        assert!(!auth.allows(Some(&used), start + day * 76));
    }

    #[test]
    fn guessing_is_slowed_down() {
        let (auth, start) = (auth(), Instant::now());
        let second = Duration::from_secs(1);
        let operator = session(&auth, start);
        for n in 0..5 {
            assert_eq!(auth.login("guess", start + second * n), Login::Wrong);
        }
        // The sixth and the right one are refused unlooked at.
        assert_eq!(auth.login("guess", start + second * 6), Login::TooMany { first: true });
        assert_eq!(auth.login(PASSWORD, start + second * 10), Login::TooMany { first: false });
        assert_eq!(auth.login(PASSWORD, start + second * 63), Login::TooMany { first: false });
        // A session that was there keeps working all the while.
        assert!(auth.allows(Some(&operator), start + second * 10));
        // A minute after the last wrong one, which was the fifth.
        let token = match auth.login(PASSWORD, start + second * 64) {
            Login::Session(token) => token,
            other => panic!("{other:?}"),
        };
        assert!(auth.allows(Some(&token), start + second * 64));
        // The next time it happens, it is the first refusal again.
        for n in 0..5 {
            assert_eq!(auth.login("guess", start + second * (70 + n)), Login::Wrong);
        }
        assert_eq!(auth.login("guess", start + second * 76), Login::TooMany { first: true });
    }

    #[test]
    fn wrong_passwords_far_apart_are_not_too_many() {
        let (auth, start) = (auth(), Instant::now());
        let second = Duration::from_secs(1);
        for n in 0..20 {
            assert_eq!(auth.login("guess", start + second * 20 * n), Login::Wrong, "{n}");
        }
        assert!(matches!(auth.login(PASSWORD, start + second * 401), Login::Session(_)));
    }

    #[test]
    fn a_failed_token_is_no_session() {
        let (auth, now) = (auth(), Instant::now());
        let failed = auth.login_with(PASSWORD, now, || Err("no entropy".to_string()));
        assert_eq!(failed, Login::Failed("no entropy".to_string()));
        assert!(auth.state().sessions.is_empty());
    }

    #[test]
    fn the_cookie_is_named_by_the_port_and_kept_from_scripts_and_other_sites() {
        let auth = auth();
        let token = "ab".repeat(32);
        let set = auth.set_cookie(&token);
        assert_eq!(
            set,
            format!("rslbot_8080={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000")
        );
        assert_eq!(auth.clear_cookie(), "rslbot_8080=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
        // Read back among other cookies, whatever the spacing.
        for header in [
            format!("rslbot_8080={token}"),
            format!("theme=dark; rslbot_8080={token}; other=1"),
            format!("rslbot_8081={}; rslbot_8080={token}", "cd".repeat(32)),
            format!("a=b;rslbot_8080={token}"),
        ] {
            assert_eq!(auth.token(Some(&header)), Some(token.as_str()), "{header}");
        }
        for header in [
            "",
            "rslbot_8080=",
            "rslbot_8080=short",
            "xrslbot_8080=abc",
            &format!("rslbot_8081={token}"),
        ] {
            assert_eq!(auth.token(Some(header)), None, "{header}");
        }
        assert_eq!(auth.token(None), None);
        let other = Auth::new(Some(Secret::new(PASSWORD)), 8081);
        assert!(other.set_cookie(&token).starts_with("rslbot_8081="));
    }
}
