//! Answering instant messages by itself: the autoreply setting (whether it
//! is on, the text, the pause), the pause each avatar is in after it was
//! answered or written to, and the duration form the config and the
//! `autoreply` command take. Nothing here talks to the grid.

use crate::udp::messages::MAX_TEXT;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// The pause of a config whose `[autoreply]` table gives none.
pub const DEFAULT_PAUSE: Duration = Duration::from_secs(HOUR);
/// The longest pause.
pub const MAX_PAUSE: Duration = Duration::from_secs(365 * DAY);
/// The duration form, as an error describes it.
pub const DURATION_FORM: &str = "a whole number and a unit for each of d, h, m and s that is needed, in that order and without spaces \
     (`90s`, `30m`, `2h`, `1h30m`), or `0`, at most 365 days";

/// The units of a duration, in the order they are written.
const UNITS: [(char, u64); 4] = [('d', DAY), ('h', HOUR), ('m', MINUTE), ('s', 1)];

/// Reads a duration: `90s`, `30m`, `1h30m`, `0`.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || format!("`{text}` is not a duration: write {DURATION_FORM}");
    if text == "0" {
        return Ok(Duration::ZERO);
    }
    let mut rest = text;
    // The units still allowed: each comes after the one before it.
    let mut units = &UNITS[..];
    let mut total: u64 = 0;
    if rest.is_empty() {
        return Err(bad());
    }
    while !rest.is_empty() {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return Err(bad());
        }
        let number: u64 = rest[..digits].parse().map_err(|_| bad())?;
        let unit = rest[digits..].chars().next().ok_or_else(bad)?;
        let at = units.iter().position(|(name, _)| *name == unit).ok_or_else(bad)?;
        let seconds = number.checked_mul(units[at].1).ok_or_else(bad)?;
        total = total.checked_add(seconds).ok_or_else(bad)?;
        units = &units[at + 1..];
        rest = &rest[digits + unit.len_utf8()..];
    }
    let duration = Duration::from_secs(total);
    if duration > MAX_PAUSE {
        return Err(format!("`{text}` is longer than 365 days; write {DURATION_FORM}"));
    }
    Ok(duration)
}

/// Writes a duration in the form `parse_duration` reads, largest units first.
pub fn format_duration(duration: Duration) -> String {
    let mut left = duration.as_secs();
    if left == 0 {
        return "0".to_string();
    }
    let mut text = String::new();
    for (unit, seconds) in UNITS {
        if left >= seconds {
            text.push_str(&format!("{}{unit}", left / seconds));
            left %= seconds;
        }
    }
    text
}

/// Checks an autoreply text: not blank, and short enough for one instant
/// message.
pub fn check_message(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("the autoreply text is blank".to_string());
    }
    if text.len() > MAX_TEXT {
        return Err(format!("the autoreply text is {} bytes long; the limit is {MAX_TEXT}", text.len()));
    }
    Ok(())
}

/// The autoreply setting and the avatars' pauses.
#[derive(Debug)]
pub struct Autoreply {
    on: bool,
    text: Option<String>,
    pause: Duration,
    /// When each avatar's pause started: its last autoreply, or the last
    /// instant message the bot sent it on command.
    started: HashMap<Uuid, Instant>,
}

impl Autoreply {
    /// On when there is a text, as a config with an `[autoreply]` table.
    pub fn new(text: Option<String>, pause: Duration) -> Autoreply {
        Autoreply {
            on: text.is_some(),
            text,
            pause,
            started: HashMap::new(),
        }
    }

    fn in_pause(&self, who: Uuid, now: Instant) -> bool {
        self.started
            .get(&who)
            .is_some_and(|&start| now.saturating_duration_since(start) < self.pause)
    }

    /// The text to answer `from` with, if autoreply is on and `from` is not
    /// in its pause. The pause starts only with `sent`, once the answer has
    /// gone out.
    pub fn answer(&self, from: Uuid, now: Instant) -> Option<&str> {
        match &self.text {
            Some(text) if self.on && !self.in_pause(from, now) => Some(text),
            _ => None,
        }
    }

    /// The bot sent `to` an instant message: its pause starts again.
    /// Pauses that have run out are forgotten on the way.
    pub fn sent(&mut self, to: Uuid, now: Instant) {
        let pause = self.pause;
        self.started.retain(|_, start| now.saturating_duration_since(*start) < pause);
        self.started.insert(to, now);
    }

    /// Forgets every pause; returns how many avatars were in theirs.
    pub fn refresh(&mut self, now: Instant) -> usize {
        let paused = self.paused(now);
        self.started.clear();
        paused
    }

    /// Sets the text and switches autoreply on.
    pub fn set_text(&mut self, text: String) {
        self.text = Some(text);
        self.on = true;
    }

    pub fn set_pause(&mut self, pause: Duration) {
        self.pause = pause;
    }

    /// Switches autoreply on or off; returns whether that changed anything.
    /// It cannot be switched on without a text.
    pub fn switch(&mut self, on: bool) -> Result<bool, String> {
        if on && self.text.is_none() {
            return Err("there is no autoreply text; `autoreply message TEXT` sets one and switches autoreply on".to_string());
        }
        let changed = self.on != on;
        self.on = on;
        Ok(changed)
    }

    #[cfg(test)]
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// How many avatars are in their pause.
    pub fn paused(&self, now: Instant) -> usize {
        self.started.keys().filter(|&&who| self.in_pause(who, now)).count()
    }

    /// What `autoreply status` answers.
    pub fn status(&self, now: Instant) -> String {
        let state = if self.on { "on" } else { "off" };
        let text = self.text.as_deref().unwrap_or("(none)");
        format!(
            "autoreply: {state}\nmessage: {text}\npause: {}\navatars in their pause: {}",
            format_duration(self.pause),
            self.paused(now)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn avatar(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    const JANE: u8 = 1;
    const JOHN: u8 = 2;

    fn two_hours() -> Autoreply {
        Autoreply::new(Some("Away".to_string()), secs(2 * HOUR))
    }

    #[test]
    fn durations_are_read() {
        assert_eq!(parse_duration("90s"), Ok(secs(90)));
        assert_eq!(parse_duration("30m"), Ok(secs(30 * MINUTE)));
        assert_eq!(parse_duration("2h"), Ok(secs(2 * HOUR)));
        assert_eq!(parse_duration("1h30m"), Ok(secs(90 * MINUTE)));
        assert_eq!(parse_duration("1d2h3m4s"), Ok(secs(DAY + 2 * HOUR + 3 * MINUTE + 4)));
        assert_eq!(parse_duration("0"), Ok(Duration::ZERO));
        assert_eq!(parse_duration("0m"), Ok(Duration::ZERO));
        assert_eq!(parse_duration("365d"), Ok(MAX_PAUSE));
    }

    #[test]
    fn malformed_durations_are_refused() {
        for text in ["2 hours", "30", "30m2h", "1h1h", "", "h", "-1h", "1.5h", "2H", " 2h", "2h ", "1w"] {
            let error = parse_duration(text).unwrap_err();
            assert!(error.contains("not a duration"), "{text}: {error}");
        }
        for text in ["400d", "366d", "365d1s", "99999999999999999999d", "18446744073709551615d"] {
            assert!(parse_duration(text).is_err(), "{text}");
        }
        assert!(parse_duration("400d").unwrap_err().contains("365 days"));
    }

    #[test]
    fn durations_are_written_largest_units_first() {
        assert_eq!(format_duration(Duration::ZERO), "0");
        assert_eq!(format_duration(secs(90)), "1m30s");
        assert_eq!(format_duration(secs(2 * HOUR)), "2h");
        assert_eq!(format_duration(secs(DAY + 30 * MINUTE)), "1d30m");
        for text in ["90s", "30m", "1h30m", "1d2h3m4s"] {
            assert_eq!(
                parse_duration(&format_duration(parse_duration(text).unwrap())),
                parse_duration(text)
            );
        }
    }

    #[test]
    fn texts_are_checked() {
        assert!(check_message("Away").is_ok());
        assert!(check_message("Away.\nBack tonight.").is_ok());
        assert!(check_message("  \n").unwrap_err().contains("blank"));
        assert!(check_message(&"x".repeat(MAX_TEXT)).is_ok());
        assert!(check_message(&"x".repeat(MAX_TEXT + 1)).unwrap_err().contains("1023"));
    }

    #[test]
    fn first_message_is_answered_and_the_second_within_the_pause_is_not() {
        let mut auto = two_hours();
        let t = Instant::now();
        assert_eq!(auto.answer(avatar(JANE), t), Some("Away"));
        auto.sent(avatar(JANE), t);
        assert_eq!(auto.answer(avatar(JANE), t + secs(HOUR)), None);
        assert_eq!(auto.answer(avatar(JANE), t + secs(3 * HOUR)), Some("Away"));
    }

    #[test]
    fn messages_within_the_pause_do_not_extend_it() {
        let mut auto = two_hours();
        let t = Instant::now();
        let mut answered = Vec::new();
        for half_hour in 0..=6 {
            let now = t + secs(half_hour * 30 * MINUTE);
            if auto.answer(avatar(JANE), now).is_some() {
                auto.sent(avatar(JANE), now);
                answered.push(half_hour);
            }
        }
        assert_eq!(answered, [0, 4]);
    }

    #[test]
    fn pauses_are_per_avatar() {
        let mut auto = two_hours();
        let t = Instant::now();
        auto.sent(avatar(JANE), t);
        assert_eq!(auto.answer(avatar(JOHN), t + secs(MINUTE)), Some("Away"));
    }

    #[test]
    fn a_shorter_pause_applies_at_once() {
        let mut auto = two_hours();
        let t = Instant::now();
        auto.sent(avatar(JANE), t);
        let later = t + secs(40 * MINUTE);
        assert_eq!(auto.answer(avatar(JANE), later), None);
        auto.set_pause(secs(30 * MINUTE));
        assert_eq!(auto.answer(avatar(JANE), later), Some("Away"));
    }

    #[test]
    fn pause_zero_answers_every_message() {
        let mut auto = Autoreply::new(Some("Away".to_string()), Duration::ZERO);
        let t = Instant::now();
        for _ in 0..3 {
            assert_eq!(auto.answer(avatar(JANE), t), Some("Away"));
            auto.sent(avatar(JANE), t);
        }
        assert_eq!(auto.paused(t), 0);
    }

    #[test]
    fn off_answers_nothing_and_keeps_the_pauses() {
        let mut auto = two_hours();
        let t = Instant::now();
        auto.sent(avatar(JANE), t);
        assert_eq!(auto.switch(false), Ok(true));
        assert_eq!(auto.switch(false), Ok(false));
        assert_eq!(auto.answer(avatar(JOHN), t), None);
        assert_eq!(auto.switch(true), Ok(true));
        assert_eq!(auto.answer(avatar(JOHN), t), Some("Away"));
        assert_eq!(auto.answer(avatar(JANE), t), None);
    }

    #[test]
    fn refresh_forgets_every_pause() {
        let mut auto = two_hours();
        let t = Instant::now();
        auto.sent(avatar(JANE), t);
        auto.sent(avatar(JOHN), t);
        let later = t + secs(10 * MINUTE);
        assert_eq!(auto.refresh(later), 2);
        assert_eq!(auto.answer(avatar(JANE), later), Some("Away"));
        assert_eq!(auto.refresh(later), 0);
    }

    #[test]
    fn on_needs_a_text() {
        let mut auto = Autoreply::new(None, DEFAULT_PAUSE);
        assert!(!auto.is_on());
        assert!(auto.switch(true).unwrap_err().contains("autoreply message"));
        assert!(!auto.is_on());
        assert_eq!(auto.answer(avatar(JANE), Instant::now()), None);
        auto.set_text("Back soon".to_string());
        assert!(auto.is_on());
        assert_eq!(auto.answer(avatar(JANE), Instant::now()), Some("Back soon"));
    }

    #[test]
    fn run_out_pauses_are_forgotten() {
        let mut auto = two_hours();
        let t = Instant::now();
        auto.sent(avatar(JANE), t);
        auto.sent(avatar(JOHN), t + secs(3 * HOUR));
        assert_eq!(auto.started.len(), 1);
    }

    #[test]
    fn status_says_everything() {
        let mut auto = two_hours();
        let t = Instant::now();
        auto.sent(avatar(JANE), t);
        assert_eq!(auto.status(t), "autoreply: on\nmessage: Away\npause: 2h\navatars in their pause: 1");
        let none = Autoreply::new(None, DEFAULT_PAUSE);
        assert_eq!(
            none.status(t),
            "autoreply: off\nmessage: (none)\npause: 1h\navatars in their pause: 0"
        );
    }
}
