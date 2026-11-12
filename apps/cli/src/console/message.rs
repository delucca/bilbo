use bilbo_store::segments::Notice;

use crate::console::theme::{Mark, Style, paint};
use crate::console::view::View;

pub const PREFIX: &str = "bilbo: ";

const INDENT: &str = "   ";

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
        let mark = paint(view, Style::Mark(self.mark), self.mark.glyph(view.unicode));
        let mut lines = Vec::new();
        let mut text = parts(&self.text);
        lines.push(format!(
            "{mark}  {}",
            line(view, text.next().unwrap_or_default())
        ));
        for piece in text {
            lines.push(indented(&line(view, piece)));
        }
        for detail in &self.details {
            match detail {
                Detail::Text(text) => {
                    for piece in parts(text) {
                        lines.push(indented(&paint(view, Style::Note, &line(view, piece))));
                    }
                }
                Detail::Command { lead, command } => lines.push(format!(
                    "{INDENT}{} {}",
                    paint(view, Style::Note, lead),
                    paint(view, Style::Command, &line(view, command))
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

pub fn group(number: u64) -> String {
    let digits = number.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn reorders(c: char) -> bool {
    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
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

pub fn line(view: &View, text: &str) -> String {
    let escaped = sanitize(text);
    if !view.terminal {
        return escaped;
    }
    escaped
        .chars()
        .flat_map(|c| {
            if reorders(c) {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn notice_message(notice: &Notice) -> Message {
    match notice {
        Notice::Waiting => plain(
            Mark::Info,
            "waiting for another bilbo to finish writing the store",
        ),
        Notice::CutTornTail {
            path,
            offset,
            bytes,
        } => plain(
            Mark::Warning,
            &format!(
                "cut {bytes} bytes of a torn frame at byte {offset} of {}",
                path.display()
            ),
        ),
        Notice::RebuiltCatalog { entries, segments } => plain(
            Mark::Info,
            &format!(
                "rebuilt the catalog from the store: {} in {}",
                plural(*entries, "entry", "entries"),
                plural(*segments, "segment", "segments")
            ),
        ),
    }
}

fn plain(mark: Mark, text: &str) -> Message {
    Message {
        mark,
        text: sanitize(text),
        details: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::console::view::View;

    use crate::console::theme::Mark;

    use std::path::PathBuf;

    use bilbo_store::segments::Notice;

    use super::{Detail, Message, group, line, notice_message, sanitize};

    #[test]
    fn a_bidirectional_control_is_shown_escaped_and_a_joiner_is_kept() {
        let controls = [
            '\u{61c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
            '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ];
        let kept = ['\u{200d}', '\u{200c}', '\u{ad}', '\u{feff}'];
        for control in controls {
            assert_eq!(
                line(&View::HUMAN, &format!("a{control}b")),
                format!("a{}b", control.escape_default()),
                "{control:?}"
            );
        }
        for character in kept {
            assert_eq!(
                line(&View::HUMAN, &format!("a{character}b")),
                format!("a{character}b"),
                "{character:?}"
            );
        }
        for character in controls.into_iter().chain(kept) {
            let text = format!("a{character}b");
            assert_eq!(line(&View::PLAIN, &text), text, "{character:?}");
            assert_eq!(sanitize(&text), text, "{character:?}");
        }
    }

    #[test]
    fn numbers_on_a_terminal_are_grouped_by_thousands() {
        let grouped: Vec<String> = [0, 999, 1000, 1_234_567].into_iter().map(group).collect();
        assert_eq!(grouped, ["0", "999", "1,000", "1,234,567"]);
    }

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

    #[test]
    fn every_store_notice_is_a_message() {
        let cases = [
            (
                Notice::Waiting,
                Mark::Info,
                "waiting for another bilbo to finish writing the store",
            ),
            (
                Notice::CutTornTail {
                    path: PathBuf::from("/s/open.seg"),
                    offset: 12,
                    bytes: 20,
                },
                Mark::Warning,
                "cut 20 bytes of a torn frame at byte 12 of /s/open.seg",
            ),
            (
                Notice::RebuiltCatalog {
                    entries: 3,
                    segments: 1,
                },
                Mark::Info,
                "rebuilt the catalog from the store: 3 entries in 1 segment",
            ),
            (
                Notice::RebuiltCatalog {
                    entries: 1,
                    segments: 2,
                },
                Mark::Info,
                "rebuilt the catalog from the store: 1 entry in 2 segments",
            ),
        ];
        for (notice, mark, text) in cases {
            let message = notice_message(&notice);
            assert_eq!(message.mark, mark);
            assert_eq!(message.text, text);
        }
    }
}
