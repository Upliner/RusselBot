//! The escaping that keeps a text field on one line and free of tabs, used
//! by the TSV logs and by the control socket's lines.

/// Writes backslash, tab, line feed and carriage return as `\\`, `\t`, `\n`
/// and `\r`.
pub fn escape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    for c in field.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// The inverse of `escape`. A backslash before anything else, or at the end,
/// stands for itself.
pub fn unescape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_what_it_escapes() {
        for text in ["plain", "", "a\tb\nc\r\nd", "back\\slash", "\\t is not a tab", "\\\\", "tab\t\\n"] {
            let escaped = escape(text);
            assert!(!escaped.contains(['\t', '\n', '\r']), "{escaped:?}");
            assert_eq!(unescape(&escaped), text);
        }
        assert_eq!(escape("a\tb\\"), "a\\tb\\\\");
    }

    #[test]
    fn stray_backslashes_stand_for_themselves() {
        assert_eq!(unescape("dangling\\"), "dangling\\");
        assert_eq!(unescape("\\x"), "\\x");
    }
}
