use crate::console::theme::{Tone, paint};
use crate::console::view::View;

pub const PREFIX: &str = "bilbo: ";

const INDENT: &str = "   ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Error,
    Warning,
    Done,
    Info,
}

impl Mark {
    fn glyph(self, unicode: bool) -> &'static str {
        let (unicode_glyph, ascii) = match self {
            Self::Error => ("■", "x"),
            Self::Warning => ("▲", "!"),
            Self::Done => ("◆", "*"),
            Self::Info => ("●", "i"),
        };
        if unicode { unicode_glyph } else { ascii }
    }

    fn tone(self) -> Tone {
        match self {
            Self::Error => Tone::Red,
            Self::Warning => Tone::Yellow,
            Self::Done => Tone::Green,
            Self::Info => Tone::Blue,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    Text(String),
    Command { lead: &'static str, command: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub mark: Mark,
    pub text: String,
    pub details: Vec<Detail>,
}

impl Message {
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            mark: Mark::Error,
            text: text.into(),
            details: Vec::new(),
        }
    }

    pub fn lines(&self, view: &View) -> Vec<String> {
        if view.human {
            self.human_lines(view)
        } else {
            self.plain_lines()
        }
    }

    pub fn human_lines(&self, view: &View) -> Vec<String> {
        let mark = paint(view, self.mark.tone(), self.mark.glyph(view.unicode));
        let mut lines = Vec::new();
        let mut text = parts(&self.text);
        lines.push(format!(
            "{mark}  {}",
            sanitize(text.next().unwrap_or_default())
        ));
        for line in text {
            lines.push(indented(&sanitize(line)));
        }
        for detail in &self.details {
            match detail {
                Detail::Text(text) => {
                    for line in parts(text) {
                        lines.push(indented(&paint(view, Tone::Dim, &sanitize(line))));
                    }
                }
                Detail::Command { lead, command } => lines.push(format!(
                    "{INDENT}{} {}",
                    paint(view, Tone::Dim, lead),
                    paint(view, Tone::Cyan, &sanitize(command))
                )),
            }
        }
        lines
    }

    pub fn plain_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = parts(&self.text)
            .map(|line| format!("{PREFIX}{}", sanitize(line)))
            .collect();
        for detail in &self.details {
            match detail {
                Detail::Text(text) => {
                    lines.extend(parts(text).map(|line| format!("{PREFIX}{}", sanitize(line))));
                }
                Detail::Command { lead, command } => {
                    lines.push(format!("{PREFIX}{lead} '{}'", sanitize(command)));
                }
            }
        }
        lines
    }
}

fn parts(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
}

fn indented(line: &str) -> String {
    if line.is_empty() {
        String::new()
    } else {
        format!("{INDENT}{line}")
    }
}

pub fn sanitize(line: &str) -> String {
    line.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::console::view::View;

    use super::{Detail, Mark, Message};

    fn message(mark: Mark) -> Message {
        Message {
            mark,
            text: "it failed".to_owned(),
            details: vec![
                Detail::Text("why".to_owned()),
                Detail::Command {
                    lead: "see",
                    command: "bilbo --help".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn a_human_message_is_a_mark_its_text_and_indented_details() {
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(
            message(Mark::Error).human_lines(&unpainted),
            ["■  it failed", "   why", "   see bilbo --help"]
        );
    }

    #[test]
    fn each_mark_has_its_colour_details_are_dim_and_commands_cyan() {
        for (mark, painted) in [
            (Mark::Error, "\x1b[31m■\x1b[39m"),
            (Mark::Warning, "\x1b[33m▲\x1b[39m"),
            (Mark::Done, "\x1b[32m◆\x1b[39m"),
            (Mark::Info, "\x1b[34m●\x1b[39m"),
        ] {
            assert_eq!(
                message(mark).human_lines(&View::HUMAN),
                [
                    format!("{painted}  it failed"),
                    "   \x1b[2mwhy\x1b[22m".to_owned(),
                    "   \x1b[2msee\x1b[22m \x1b[36mbilbo --help\x1b[39m".to_owned(),
                ]
            );
        }
    }

    #[test]
    fn lines_follow_the_view() {
        assert_eq!(
            message(Mark::Error).lines(&View::PLAIN),
            message(Mark::Error).plain_lines()
        );
        assert_eq!(
            message(Mark::Error).lines(&View::HUMAN),
            message(Mark::Error).human_lines(&View::HUMAN)
        );
    }

    #[test]
    fn each_mark_has_an_ascii_form() {
        let ascii = View {
            paint: false,
            unicode: false,
            ..View::HUMAN
        };
        for (mark, glyph) in [
            (Mark::Error, "x"),
            (Mark::Warning, "!"),
            (Mark::Done, "*"),
            (Mark::Info, "i"),
        ] {
            let message = Message {
                mark,
                ..Message::error("text")
            };
            assert_eq!(message.human_lines(&ascii)[0], format!("{glyph}  text"));
        }
    }

    #[test]
    fn the_plain_view_prefixes_every_line_and_quotes_the_command() {
        assert_eq!(
            message(Mark::Error).plain_lines(),
            [
                "bilbo: it failed",
                "bilbo: why",
                "bilbo: see 'bilbo --help'"
            ]
        );
    }

    #[test]
    fn every_line_of_a_long_text_is_prefixed_or_indented() {
        let message = Message {
            mark: Mark::Error,
            text: "it failed\n\nbecause".to_owned(),
            details: vec![Detail::Text("bad\nvalue".to_owned())],
        };
        assert_eq!(
            message.plain_lines(),
            [
                "bilbo: it failed",
                "bilbo: ",
                "bilbo: because",
                "bilbo: bad",
                "bilbo: value"
            ]
        );
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(
            message.human_lines(&unpainted),
            ["■  it failed", "", "   because", "   bad", "   value"]
        );
    }

    #[test]
    fn a_carriage_return_before_a_newline_is_escaped_not_dropped() {
        let message = Message {
            mark: Mark::Error,
            text: "a\r\nb".to_owned(),
            details: vec![Detail::Text("c\r\nd".to_owned())],
        };
        assert_eq!(
            message.plain_lines(),
            ["bilbo: a\\r", "bilbo: b", "bilbo: c\\r", "bilbo: d"]
        );
        let unpainted = View {
            paint: false,
            ..View::HUMAN
        };
        assert_eq!(
            message.human_lines(&unpainted),
            ["■  a\\r", "   b", "   c\\r", "   d"]
        );
    }

    #[test]
    fn control_characters_in_outside_text_are_escaped() {
        let message = Message {
            mark: Mark::Error,
            text: "cannot read \u{1b}[2J".to_owned(),
            details: vec![
                Detail::Text("a\rb.md".to_owned()),
                Detail::Text("go\u{7f}d".to_owned()),
                Detail::Text("go\u{9b}d".to_owned()),
                Detail::Command {
                    lead: "see",
                    command: "bilbo \u{1b}[2J".to_owned(),
                },
            ],
        };
        for shown in [
            message.plain_lines().join("\n"),
            message.human_lines(&View::HUMAN).join("\n"),
        ] {
            assert!(!shown.contains("\u{1b}[2J"), "{shown:?}");
            for raw in ['\r', '\u{7f}', '\u{9b}'] {
                assert!(!shown.contains(raw), "{shown:?}");
            }
            assert!(shown.contains("cannot read \\u{1b}[2J"), "{shown:?}");
            assert!(shown.contains("a\\rb.md"), "{shown:?}");
            assert!(shown.contains("go\\u{7f}d"), "{shown:?}");
            assert!(shown.contains("go\\u{9b}d"), "{shown:?}");
            assert!(shown.contains("bilbo \\u{1b}[2J"), "{shown:?}");
        }
    }
}
