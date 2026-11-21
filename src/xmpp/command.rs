//! Command lines sent to the control contact: split into arguments as a
//! shell would for `rslbot ctl`, and the `help` text.

use crate::control::CTL_USAGE;

/// The longest command line.
pub const MAX_COMMAND: usize = 8192;

/// Splits a message body into arguments at spaces, tabs and line feeds. A
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

/// The answer to `help`: the command list of `rslbot ctl` as commands to
/// send here.
pub fn help() -> String {
    let mut out = String::from("Commands (an argument with spaces goes in double quotes):\n");
    let mut paragraphs = CTL_USAGE.split("\n\n");
    let usage = paragraphs.next().unwrap_or_default();
    for line in usage.lines() {
        let line = line.trim_start_matches("Usage:").trim_start();
        out.push_str(line.strip_prefix("rslbot ctl ").unwrap_or(line));
        out.push('\n');
    }
    let rest: Vec<&str> = paragraphs.collect();
    // The last paragraph is about the socket's address, of no use here;
    // the first starts with a sentence about the socket.
    for (index, paragraph) in rest.iter().enumerate().take(rest.len().saturating_sub(1)) {
        let paragraph = match index {
            0 => paragraph.split_once(".\n").map_or(*paragraph, |(_, after)| after),
            _ => paragraph,
        };
        out.push('\n');
        out.push_str(&paragraph.replace("rslbot ctl ", ""));
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{request_line, Request};

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

    #[test]
    fn help_lists_the_commands_without_the_ctl_prefix() {
        let text = help();
        assert!(text.contains("\nsay MESSAGE...\n"), "{text}");
        assert!(text.contains("\nquit"), "{text}");
        assert!(text.contains("\nrlv cmd COMMAND...\n"), "{text}");
        assert!(!text.contains("rslbot ctl"), "{text}");
        assert!(!text.contains("RUSSELBOT_SOCK"), "{text}");
        assert!(!text.contains("socket = true"), "{text}");
        assert!(text.contains("`say` speaks in local chat"), "{text}");
    }
}
