//! Timestamped stderr logging with an optional verbose level.

use std::fmt::Arguments;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(on: bool) {
    VERBOSE.store(on, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Formats seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days, proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// Reads `YYYY-MM-DDTHH:MM:SSZ` back into seconds since the Unix epoch.
pub fn parse_timestamp(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = text.get(range)?;
        digits.bytes().all(|b| b.is_ascii_digit()).then(|| digits.parse().ok())?
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Days-from-civil, proleptic Gregorian calendar.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    let secs = u64::try_from(secs).ok()?;
    // A day the month does not have does not read back as itself.
    (format_timestamp(secs) == text).then_some(secs)
}

pub fn write(args: Arguments) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    eprintln!("{} {}", format_timestamp(secs), one_line(&args.to_string()));
}

/// Grid-supplied text (kick reasons, login messages) may span lines; a log
/// entry must not.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::log::write(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::log::verbose() {
            $crate::log::write(format_args!($($arg)*))
        }
    };
}

#[cfg(test)]
mod tests {
    use super::{format_timestamp, one_line, parse_timestamp};

    #[test]
    fn multi_line_text_becomes_one_line() {
        assert_eq!(one_line("kicked: \nbye  now\n"), "kicked: bye now");
    }

    #[test]
    fn formats_known_epoch_values() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_timestamp(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn reads_back_what_it_formats() {
        for secs in [0, 951_782_400, 1_700_000_000, 1_796_126_400, 4_102_444_799] {
            assert_eq!(parse_timestamp(&format_timestamp(secs)), Some(secs));
        }
        for bad in [
            "",
            "2026-12-01 12:00:00Z",
            "2026-02-30T12:00:00Z",
            "2026-13-01T12:00:00Z",
            "+026-12-01T12:00:00Z",
        ] {
            assert_eq!(parse_timestamp(bad), None, "{bad}");
        }
    }
}
