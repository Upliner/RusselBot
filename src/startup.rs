//! The startup script: commands from the config that the bot carries out by
//! itself when it starts. This module reads the script and keeps track of
//! how far it has got; the main loop carries the commands out.

use crate::control::{request_line, split, Request, Response, RlvRequest};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};

/// One command of the script.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The line the command starts on, counted from 1.
    pub number: usize,
    /// The command as it is written, without the blanks around it.
    pub text: String,
    pub request: Request,
}

/// Why a script cannot be used, or why one of its commands was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptError {
    /// The line the command starts on.
    pub line: usize,
    pub reason: String,
}

/// Whether `text` ends inside double quotes, when it started inside them
/// or not as `quoted` says. Quotes are read as `control::split` reads them.
fn ends_quoted(text: &str, mut quoted: bool) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => quoted = !quoted,
            '\\' if quoted && matches!(chars.peek(), Some('"' | '\\')) => {
                chars.next();
            }
            _ => {}
        }
    }
    quoted
}

/// Reads a script: one command per line. A line end inside double quotes
/// belongs to the argument, and the command goes on in the next line.
/// Blank lines and lines that start with `#` are skipped, unless they are
/// inside double quotes. Every command must be one the control socket
/// takes, in its form.
pub fn parse(text: &str) -> Result<Vec<Line>, ScriptError> {
    let mut script = Vec::new();
    let mut lines = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).enumerate();
    while let Some((index, line)) = lines.next() {
        let start = line.trim_start_matches([' ', '\t']);
        if start.is_empty() || start.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let fail = |reason: String| ScriptError { line: number, reason };
        let mut command = line.to_string();
        let mut quoted = ends_quoted(line, false);
        while quoted {
            let Some((_, more)) = lines.next() else {
                return Err(fail("the command ends inside double quotes; close the quote with \"".to_string()));
            };
            command.push('\n');
            command.push_str(more);
            quoted = ends_quoted(more, true);
        }
        let args = split(&command).map_err(fail)?;
        let request = Request::decode(request_line(&args).as_bytes()).map_err(fail)?;
        script.push(Line {
            number,
            text: command.trim_matches([' ', '\t']).to_string(),
            request,
        });
    }
    Ok(script)
}

/// Whether a command is a setting command: one that works in every state
/// of the bot and moves nothing. Those at the head of the script are
/// carried out before the first login.
pub fn is_setting(request: &Request) -> bool {
    match request {
        Request::Perms(_) | Request::Autoreply(_) => true,
        Request::Rlv(rlv) => !matches!(rlv, RlvRequest::Cmd(_)),
        _ => false,
    }
}

/// How far the script has got.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// The setting commands at its head are still to be carried out.
    Head,
    /// The rest waits for the config placement to be reached.
    Waiting,
    /// The rest is being carried out.
    Running,
    /// Nothing is left of it, or what was left has been given up.
    Done,
}

/// The script of one run of the bot.
pub struct Script {
    /// The commands not handed out yet.
    lines: VecDeque<Line>,
    stage: Stage,
    /// The command whose answer is awaited.
    flight: Option<(Line, Receiver<Response>)>,
}

impl Script {
    pub fn new(lines: Vec<Line>) -> Script {
        let stage = if lines.is_empty() { Stage::Done } else { Stage::Head };
        Script {
            lines: lines.into(),
            stage,
            flight: None,
        }
    }

    /// The next setting command of the head. With the last of them handed
    /// out, the rest waits for the placement.
    pub fn next_head(&mut self) -> Option<Line> {
        if self.stage != Stage::Head {
            return None;
        }
        if self.lines.front().is_some_and(|line| is_setting(&line.request)) {
            return self.lines.pop_front();
        }
        self.stage = if self.lines.is_empty() { Stage::Done } else { Stage::Waiting };
        None
    }

    /// Whether a rest waits for the config placement to be reached.
    pub fn waiting(&self) -> bool {
        self.stage == Stage::Waiting
    }

    /// The config placement is reached: the rest may run. Returns whether
    /// there is one that waited for this.
    pub fn release(&mut self) -> bool {
        let waited = self.waiting();
        if waited {
            self.stage = Stage::Running;
        }
        waited
    }

    /// Whether the rest is being carried out.
    pub fn running(&self) -> bool {
        self.stage == Stage::Running
    }

    /// Gives up a rest that still waits for the placement. Returns the
    /// line of its first command and how many commands it had.
    pub fn give_up(&mut self) -> Option<(usize, usize)> {
        if !self.waiting() {
            return None;
        }
        self.stage = Stage::Done;
        let first = self.lines.front().map(|line| line.number)?;
        let left = self.lines.len();
        self.lines.clear();
        Some((first, left))
    }

    /// The next command of the running rest, when none is in flight.
    /// After the last one the script is done.
    pub fn next(&mut self) -> Option<Line> {
        if self.stage != Stage::Running || self.flight.is_some() {
            return None;
        }
        let line = self.lines.pop_front();
        if line.is_none() {
            self.stage = Stage::Done;
        }
        line
    }

    /// `line` has been handed to the bot; its answer comes on `answer`.
    pub fn sent(&mut self, line: Line, answer: Receiver<Response>) {
        self.flight = Some((line, answer));
    }

    /// The answer of the command in flight, once there is one. A command
    /// nobody will answer any more counts as refused.
    pub fn answered(&mut self) -> Option<(Line, Result<Option<String>, String>)> {
        let (_, answer) = self.flight.as_ref()?;
        let outcome = match answer.try_recv() {
            Ok(Response::Ok(text)) => Ok(text),
            Ok(Response::Error(reason)) => Err(reason),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the command got no answer".to_string()),
        };
        let (line, _) = self.flight.take()?;
        Some((line, outcome))
    }

    /// Stops the script where it is. Returns how many commands are left
    /// out.
    pub fn stop(&mut self) -> usize {
        self.stage = Stage::Done;
        self.flight = None;
        let left = self.lines.len();
        self.lines.clear();
        left
    }
}

/// `count` commands, in words for the log.
pub fn commands(count: usize) -> String {
    match count {
        1 => "1 command".to_string(),
        count => format!("{count} commands"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{AutoreplyRequest, PermsRequest, RlvMode};
    use std::sync::mpsc;

    fn requests(text: &str) -> Vec<Request> {
        parse(text).unwrap().into_iter().map(|line| line.request).collect()
    }

    fn refused(text: &str) -> ScriptError {
        parse(text).unwrap_err()
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let script = parse("# settings\nrlv mode on\n\n  \t\n  # more\npersist\n").unwrap();
        assert_eq!(
            script,
            [
                Line {
                    number: 2,
                    text: "rlv mode on".into(),
                    request: Request::Rlv(RlvRequest::Mode(RlvMode::On)),
                },
                Line {
                    number: 6,
                    text: "persist".into(),
                    request: Request::Persist { location: None, sit: None },
                },
            ]
        );
    }

    #[test]
    fn a_number_sign_inside_a_command_is_text() {
        assert_eq!(requests("say #1 is here"), [Request::Say("#1 is here".into())]);
        assert_eq!(requests("  say hi # there"), [Request::Say("hi # there".into())]);
    }

    #[test]
    fn a_line_end_inside_quotes_belongs_to_the_argument() {
        let script = parse("autoreply message \"I am a bot.\n# not a comment\n\nNobody reads this.\"\nsay hi\n").unwrap();
        assert_eq!(script.len(), 2);
        assert_eq!(
            script[0].request,
            Request::Autoreply(AutoreplyRequest::Message(
                "I am a bot.\n# not a comment\n\nNobody reads this.".into()
            ))
        );
        assert_eq!((script[0].number, script[1].number), (1, 5));
        // An escaped quote does not end the argument.
        assert_eq!(requests("say \"a \\\"b\nc\\\" d\""), [Request::Say("a \"b\nc\" d".into())]);
    }

    #[test]
    fn a_script_without_commands_is_valid() {
        assert_eq!(parse("").unwrap(), []);
        assert_eq!(parse("\n\n").unwrap(), []);
        assert_eq!(parse("# one\n  # two\n").unwrap(), []);
    }

    #[test]
    fn carriage_returns_before_line_feeds_are_dropped() {
        assert_eq!(
            requests("rlv mode on\r\nsay \"a\r\nb\"\r\n"),
            [Request::Rlv(RlvRequest::Mode(RlvMode::On)), Request::Say("a\nb".into())]
        );
    }

    #[test]
    fn arguments_are_split_as_the_control_contact_splits_them() {
        assert_eq!(
            requests("perms autoaccept tp \"Jane Doe\""),
            [Request::Perms(PermsRequest::Autoaccept {
                kind: "tp".into(),
                who: vec!["Jane Doe".into()],
            })]
        );
        assert_eq!(
            requests("autoreply message back in  an hour"),
            [Request::Autoreply(AutoreplyRequest::Message("back in an hour".into()))]
        );
        assert_eq!(
            requests("say \"she said \\\"hi\\\" and left\""),
            [Request::Say("she said \"hi\" and left".into())]
        );
        assert_eq!(requests("\tsay a\tb"), [Request::Say("a b".into())]);
    }

    #[test]
    fn every_command_of_the_control_socket_is_taken() {
        assert_eq!(
            requests("anim play \"Animations/Wave\"\nscan 20 -s\nquit"),
            [
                Request::Anim {
                    stop: false,
                    item: "Animations/Wave".into(),
                },
                Request::Scan(vec!["20".into(), "-s".into()]),
                Request::Quit,
            ]
        );
    }

    #[test]
    fn what_the_control_socket_does_not_take_is_refused_with_its_line() {
        let error = refused("rlv mode on\n\nhelp\n");
        assert_eq!(error.line, 3);
        assert!(error.reason.contains("unknown command `help`"), "{}", error.reason);
        let error = refused("rslbot ctl rlv mode on");
        assert!(error.reason.contains("unknown command `rslbot`"), "{}", error.reason);
        let error = refused("say hi\nsit\n");
        assert_eq!(error.line, 2);
        assert!(error.reason.contains("`sit` takes 1 argument"), "{}", error.reason);
        let error = refused("rlv mode loud");
        assert!(error.reason.contains("`rlv mode` takes"), "{}", error.reason);
    }

    #[test]
    fn an_unclosed_quote_is_refused_with_the_line_it_starts_on() {
        let error = refused("say one\nim \"Jane Doe hi\nsay two\n");
        assert_eq!(error.line, 2);
        assert!(error.reason.contains("quote"), "{}", error.reason);
    }

    #[test]
    fn a_command_that_is_too_long_is_refused() {
        let error = refused(&format!("say hi\nsay {}\n", "x".repeat(8192)));
        assert_eq!(error.line, 2);
        assert!(error.reason.contains("8192"), "{}", error.reason);
    }

    #[test]
    fn setting_commands_are_perms_autoreply_and_rlv_but_rlv_cmd() {
        for line in [
            "perms autoaccept anim all",
            "perms revoke anim all",
            "perms status",
            "autoreply message Away",
            "autoreply pause 2h",
            "autoreply off",
            "rlv mode on",
            "rlv persist follow",
            "rlv persist override sit tp",
            "rlv status",
            "rlv reset",
        ] {
            assert!(is_setting(&requests(line)[0]), "{line}");
        }
        for line in ["rlv cmd @versionnum=5", "persist cfg cfg", "home", "say hi", "status", "quit"] {
            assert!(!is_setting(&requests(line)[0]), "{line}");
        }
    }

    fn script(text: &str) -> Script {
        Script::new(parse(text).unwrap())
    }

    fn head(script: &mut Script) -> Vec<String> {
        std::iter::from_fn(|| script.next_head()).map(|line| line.text).collect()
    }

    #[test]
    fn a_script_of_settings_only_has_no_rest() {
        let mut script = script("rlv mode on\nperms autoaccept anim all\n");
        assert_eq!(head(&mut script), ["rlv mode on", "perms autoaccept anim all"]);
        assert!(!script.waiting() && !script.running());
        assert!(!script.release());
        assert_eq!(script.next(), None);
        assert_eq!(script.give_up(), None);
    }

    #[test]
    fn the_head_ends_at_the_first_other_command() {
        let mut late = script("say hello\nrlv mode on\n");
        assert_eq!(head(&mut late), Vec::<String>::new());
        assert!(late.waiting());
        let mut cmd = script("rlv mode on\nrlv cmd @versionnum=5\nrlv persist follow\n");
        assert_eq!(head(&mut cmd), ["rlv mode on"]);
        assert!(cmd.waiting());
        // The rest keeps its order, setting commands included.
        assert!(cmd.release());
        assert_eq!(cmd.next().map(|line| line.text).as_deref(), Some("rlv cmd @versionnum=5"));
        assert_eq!(cmd.next().map(|line| line.text).as_deref(), Some("rlv persist follow"));
        assert_eq!(cmd.next(), None);
        assert!(!cmd.running());
    }

    #[test]
    fn an_empty_script_is_done_from_the_start() {
        let mut script = script("# nothing\n");
        assert!(!script.waiting());
        assert_eq!(script.next_head(), None);
        assert!(!script.release());
    }

    #[test]
    fn the_rest_runs_one_command_at_a_time() {
        let mut script = script("say one\nsay two\n");
        // Nothing waits for the placement before the head has been dealt with.
        assert!(!script.waiting() && !script.release());
        assert_eq!(script.next_head(), None);
        // Nothing is handed out before the placement is reached.
        assert_eq!(script.next(), None);
        assert!(script.release());
        assert!(script.running());
        let first = script.next().unwrap();
        let (reply, answer) = mpsc::channel();
        script.sent(first, answer);
        assert_eq!(script.next(), None);
        assert_eq!(script.answered(), None);
        reply.send(Response::Ok(Some("said".into()))).unwrap();
        let (line, outcome) = script.answered().unwrap();
        assert_eq!((line.number, outcome), (1, Ok(Some("said".to_string()))));
        assert_eq!(script.next().map(|line| line.number), Some(2));
    }

    #[test]
    fn a_refusal_and_a_lost_answer_are_errors() {
        let mut script = script("say one\nsay two\nsay three\n");
        script.next_head();
        script.release();
        let first = script.next().unwrap();
        let (reply, answer) = mpsc::channel();
        script.sent(first, answer);
        reply.send(Response::Error("no".into())).unwrap();
        assert_eq!(script.answered().unwrap().1, Err("no".to_string()));
        let second = script.next().unwrap();
        let (reply, answer) = mpsc::channel();
        script.sent(second, answer);
        drop(reply);
        let (line, outcome) = script.answered().unwrap();
        assert_eq!(line.number, 2);
        assert!(outcome.unwrap_err().contains("no answer"));
        assert_eq!(script.stop(), 1);
        assert_eq!(script.next(), None);
        assert!(!script.running());
    }

    #[test]
    fn a_waiting_rest_can_be_given_up_and_a_running_one_cannot() {
        let mut waiting = script("rlv mode on\n\nsay one\nsay two\n");
        head(&mut waiting);
        assert_eq!(waiting.give_up(), Some((3, 2)));
        assert_eq!(waiting.give_up(), None);
        assert!(!waiting.release());
        let mut running = script("say one\nsay two\n");
        running.next_head();
        running.release();
        assert_eq!(running.give_up(), None);
        assert_eq!(running.next().map(|line| line.number), Some(1));
    }

    #[test]
    fn counts_are_said_in_words() {
        assert_eq!(commands(1), "1 command");
        assert_eq!(commands(3), "3 commands");
    }
}
