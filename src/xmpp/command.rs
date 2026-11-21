//! The `help` text of the control contact. A command line is split into
//! arguments by `control::split`.

use crate::control::CTL_USAGE;

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

    #[test]
    fn help_lists_the_commands_without_the_ctl_prefix() {
        let text = help();
        assert!(text.contains("\nsay MESSAGE...\n"), "{text}");
        assert!(text.contains("\nquit"), "{text}");
        assert!(text.contains("\nrlv cmd COMMAND...\n"), "{text}");
        assert!(!text.contains("rslbot ctl"), "{text}");
        assert!(!text.contains("RUSSELBOT_SOCK"), "{text}");
        assert!(!text.contains("socket = true"), "{text}");
        // What is not saved belongs in the startup script, not in the
        // deprecated tables; how `rslbot ctl` relates to it is left out.
        let flowed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(flowed.matches("the config's startup script").count(), 3, "{text}");
        assert!(!text.contains("[autoreply]") && !text.contains("[perms."), "{text}");
        assert!(!text.contains("are the lines of"), "{text}");
        assert!(text.contains("`say` speaks in local chat"), "{text}");
    }
}
