use bilbo_entries::read::refs::Locator;
use bilbo_search::digest::{Row, Said};
use bilbo_store::labels::CaptureLabel;

use crate::console::message::{entry_ref, line};
use crate::console::theme::{Style, paint};
use crate::console::view::View;

pub struct Shown<'a> {
    pub word: &'a str,
    pub rows: &'a [Row],
}

fn reference(row: &Row) -> String {
    let entry = entry_ref(row.entry, row.shared_short);
    match &row.said {
        Said::Entry { .. } => entry,
        Said::Passage { records, .. } => {
            let locator = Locator {
                generation: None,
                records: Some(*records),
            };
            format!("{entry}{locator}")
        }
    }
}

fn first_line(shown: &Shown<'_>, view: &View) -> String {
    let does = match shown.rows.first().map(|row| &row.said) {
        Some(Said::Passage { .. }) => "prints",
        _ => "outlines",
    };
    let one = shown.rows.len() == 1;
    let note = |text: &str| paint(view, Style::Note, text);
    let head = if one {
        format!("{}: 1 may bear on this prompt; ", line(view, shown.word))
    } else {
        format!(
            "{}: {} may bear on this prompt, best first; ",
            line(view, shown.word),
            shown.rows.len()
        )
    };
    format!(
        "{}{}{}",
        note(&head),
        paint(view, Style::Command, "bilbo read <ref>"),
        note(&format!(" {does} {}", if one { "it" } else { "one" })),
    )
}

fn field(view: &View, text: Option<&str>) -> String {
    text.map_or_else(|| "-".to_owned(), |text| line(view, text))
}

fn row(row: &Row, view: &View) -> String {
    let reference = paint(view, Style::Name, &reference(row));
    let (mut text, summary) = match &row.said {
        Said::Entry { title, summary } => {
            let capture = field(view, row.capture.as_ref().map(CaptureLabel::as_str));
            (
                format!("{reference}\t{capture}\t{}", field(view, title.as_deref())),
                summary,
            )
        }
        Said::Passage {
            day,
            folder,
            line: outline,
            written,
            summary,
            ..
        } => {
            let mut text = format!(
                "{reference}\t{}\t{}\t{}",
                line(view, day),
                field(view, folder.as_deref()),
                field(view, outline.as_deref())
            );
            if let Some(written) = written {
                text.push_str(&format!("\tnote: {}", line(view, written)));
            }
            (text, summary)
        }
    };
    if let Some(summary) = summary {
        text.push_str(&format!("\tsummary: {}", line(view, summary)));
    }
    text
}

pub fn lists(lists: &[Shown<'_>], view: &View) -> String {
    let mut text = String::new();
    for shown in lists.iter().filter(|shown| !shown.rows.is_empty()) {
        text.push_str(&first_line(shown, view));
        text.push('\n');
        for item in shown.rows {
            text.push_str(&row(item, view));
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use bilbo_entries::read::refs::{Ref, Target};
    use bilbo_search::digest::{Row, Said};
    use bilbo_store::entries::RecordRange;
    use bilbo_store::id::Id;
    use bilbo_store::labels::CaptureLabel;

    use super::{Shown, lists};
    use crate::console::view::View;

    const A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DG";
    const B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DH";

    fn entry(id: &str, capture: Option<&str>, title: Option<&str>, shared: bool) -> Row {
        Row {
            entry: Id::parse(id).expect("an id"),
            capture: capture.and_then(|text| CaptureLabel::new(text.to_owned())),
            score: 1.0,
            shared_short: shared,
            said: Said::Entry {
                title: title.map(str::to_owned),
                summary: None,
            },
        }
    }

    fn passage(
        id: &str,
        records: (u64, u64),
        folder: Option<&str>,
        line: Option<&str>,
        written: Option<&str>,
        shared: bool,
    ) -> Row {
        Row {
            entry: Id::parse(id).expect("an id"),
            capture: None,
            score: 1.0,
            shared_short: shared,
            said: Said::Passage {
                records: RecordRange {
                    first: records.0,
                    count: records.1,
                },
                day: "2026-11-07".to_owned(),
                folder: folder.map(str::to_owned),
                line: line.map(str::to_owned),
                written: written.map(str::to_owned),
                summary: None,
            },
        }
    }

    #[test]
    fn each_list_has_its_line_and_the_rows_of_its_kind() {
        let files = [
            entry(A, Some("file:/n/a.md"), Some("The garden log"), false),
            entry(B, Some("file:/n/b.md"), None, false),
        ];
        let sessions = [
            passage(
                A,
                (3, 5),
                Some("/src/garden"),
                Some("user: thin"),
                Some("a note"),
                false,
            ),
            passage(B, (18, 4), None, None, None, false),
        ];
        let notes = [passage(A, (1, 1), None, Some("kept text"), None, false)];
        let one = [entry(A, Some("file:/n/a.md"), Some("T"), false)];
        let printed = lists(
            &[Shown {
                word: "files",
                rows: &files,
            }],
            &View::PLAIN,
        );
        let rows: Vec<&str> = printed.lines().collect();
        assert_eq!(
            rows[0],
            "files: 2 may bear on this prompt, best first; bilbo read <ref> outlines one"
        );
        assert_eq!(
            rows[1],
            format!("{}\tfile:/n/a.md\tThe garden log", &A[16..])
        );
        assert_eq!(rows[2], format!("{}\tfile:/n/b.md\t-", &B[16..]));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            lists(
                &[Shown {
                    word: "files",
                    rows: &one
                }],
                &View::PLAIN
            )
            .lines()
            .next(),
            Some("files: 1 may bear on this prompt; bilbo read <ref> outlines it")
        );
        let printed = lists(
            &[Shown {
                word: "sessions",
                rows: &sessions,
            }],
            &View::PLAIN,
        );
        let rows: Vec<&str> = printed.lines().collect();
        assert_eq!(
            rows[0],
            "sessions: 2 may bear on this prompt, best first; bilbo read <ref> prints one"
        );
        assert_eq!(
            rows[1],
            format!(
                "{}#r3-7\t2026-11-07\t/src/garden\tuser: thin\tnote: a note",
                &A[16..]
            )
        );
        assert_eq!(rows[2], format!("{}#r18-21\t2026-11-07\t-\t-", &B[16..]));
        let all = lists(
            &[
                Shown {
                    word: "files",
                    rows: &files,
                },
                Shown {
                    word: "sessions",
                    rows: &sessions,
                },
                Shown {
                    word: "notes",
                    rows: &notes,
                },
            ],
            &View::PLAIN,
        );
        let rows: Vec<&str> = all.lines().collect();
        assert_eq!(rows.len(), 3 + 3 + 2);
        assert!(rows[0].starts_with("files: 2"));
        assert!(rows[3].starts_with("sessions: 2"));
        assert_eq!(
            rows[6],
            "notes: 1 may bear on this prompt; bilbo read <ref> prints it"
        );
        assert!(all.lines().all(|line| !line.is_empty()));
        let empty: [Row; 0] = [];
        assert_eq!(
            lists(
                &[Shown {
                    word: "files",
                    rows: &empty
                }],
                &View::PLAIN
            ),
            ""
        );
    }

    #[test]
    fn a_shared_short_id_prints_the_whole_id_and_read_takes_both() {
        let rows = [
            entry(A, Some("file:/n/a.md"), Some("T"), false),
            entry(B, Some("file:/n/b.md"), Some("T"), true),
        ];
        let printed = lists(
            &[Shown {
                word: "files",
                rows: &rows,
            }],
            &View::PLAIN,
        );
        let refs: Vec<&str> = printed
            .lines()
            .skip(1)
            .map(|row| row.split('\t').next().expect("a ref"))
            .collect();
        assert_eq!(refs, [&A[16..], B]);
        assert!(matches!(
            Ref::parse(refs[0]).expect("a ref").target,
            Target::Short(_)
        ));
        assert!(matches!(
            Ref::parse(refs[1]).expect("a ref").target,
            Target::Entry(_)
        ));
        let passages = [
            passage(A, (3, 5), None, None, None, true),
            passage(B, (4, 1), None, None, None, false),
        ];
        let printed = lists(
            &[Shown {
                word: "sessions",
                rows: &passages,
            }],
            &View::PLAIN,
        );
        let refs: Vec<&str> = printed
            .lines()
            .skip(1)
            .map(|row| row.split('\t').next().expect("a ref"))
            .collect();
        assert_eq!(refs, [format!("{A}#r3-7"), format!("{}#r4", &B[16..])]);
        for found in &refs {
            let parsed = Ref::parse(found).expect("a ref");
            assert!(parsed.locator.generation.is_none());
            assert!(parsed.locator.records.is_some());
        }
        assert_eq!(
            Ref::parse(refs[1]).expect("a ref").locator.records,
            Some(RecordRange { first: 4, count: 1 })
        );
    }

    #[test]
    fn a_label_with_a_tab_or_a_line_feed_stays_one_field() {
        let bad = "a\tb\nc";
        let files = [
            entry(A, Some(&format!("file:/{bad}")), Some(bad), false),
            entry(B, None, None, false),
        ];
        let sessions = [passage(A, (1, 1), Some(bad), Some(bad), Some(bad), false)];
        let printed = lists(
            &[Shown {
                word: "files",
                rows: &files,
            }],
            &View::PLAIN,
        );
        assert_eq!(printed.lines().count(), 3);
        let fields: Vec<&str> = printed.lines().nth(1).expect("a row").split('\t').collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[1], "file:/a\\tb\\nc");
        assert_eq!(fields[2], "a\\tb\\nc");
        let none: Vec<&str> = printed.lines().nth(2).expect("a row").split('\t').collect();
        assert_eq!(none[1..], ["-", "-"]);
        let printed = lists(
            &[Shown {
                word: "sessions",
                rows: &sessions,
            }],
            &View::PLAIN,
        );
        assert_eq!(printed.lines().count(), 2);
        let fields: Vec<&str> = printed.lines().nth(1).expect("a row").split('\t').collect();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[2..], ["a\\tb\\nc", "a\\tb\\nc", "note: a\\tb\\nc"]);
    }

    fn unpainted(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn a_persons_terminal_gets_the_ref_in_bold_and_the_same_text() {
        let files = [entry(
            A,
            Some("file:/n/a.md"),
            Some("The garden log"),
            false,
        )];
        let sessions = [passage(
            A,
            (3, 5),
            Some("/src"),
            Some("user: thin"),
            Some("a note"),
            false,
        )];
        let shown = [
            Shown {
                word: "files",
                rows: &files,
            },
            Shown {
                word: "sessions",
                rows: &sessions,
            },
        ];
        let plain = lists(&shown, &View::PLAIN);
        let human = lists(&shown, &View::HUMAN);
        assert_ne!(plain, human);
        assert_eq!(unpainted(&human), plain);
        assert!(human.contains(&format!("\u{1b}[1m{}\u{1b}[22m\t", &A[16..])));
        assert!(human.contains(&format!("\u{1b}[1m{}#r3-7\u{1b}[22m\t", &A[16..])));
        let bidi = [entry(
            A,
            Some("file:/n/\u{202e}a.md"),
            Some("t\u{2066}x"),
            false,
        )];
        let shown = [Shown {
            word: "files",
            rows: &bidi,
        }];
        let human = lists(&shown, &View::HUMAN);
        assert!(!human.contains('\u{202e}') && !human.contains('\u{2066}'));
        assert!(human.contains("\\u{202e}") && human.contains("\\u{2066}"));
        let passages = [passage(
            A,
            (3, 5),
            Some("/s\u{202e}rc"),
            Some("user: \u{2066}thin"),
            Some("a\u{2067} note"),
            false,
        )];
        let shown = [Shown {
            word: "sessions",
            rows: &passages,
        }];
        let human = lists(&shown, &View::HUMAN);
        for control in ['\u{202e}', '\u{2066}', '\u{2067}'] {
            assert!(!human.contains(control), "{control:?}");
        }
        for escape in [
            "/s\\u{202e}rc",
            "user: \\u{2066}thin",
            "note: a\\u{2067} note",
        ] {
            assert!(human.contains(escape), "{escape}");
        }
    }

    fn summarised(mut held: Row, text: &str) -> Row {
        match &mut held.said {
            Said::Entry { summary, .. } | Said::Passage { summary, .. } => {
                *summary = Some(text.to_owned());
            }
        }
        held
    }

    fn only_row(word: &'static str, row: &Row) -> String {
        let printed = lists(
            &[Shown {
                word,
                rows: std::slice::from_ref(row),
            }],
            &View::PLAIN,
        );
        printed.lines().nth(1).expect("a row").to_owned()
    }

    #[test]
    fn a_summary_is_a_rows_last_field_after_the_note() {
        let short = &A[16..];
        let file = entry(A, Some("file:/n/a.md"), Some("The garden log"), false);
        assert_eq!(
            only_row("files", &file),
            format!("{short}\tfile:/n/a.md\tThe garden log")
        );
        assert_eq!(
            only_row("files", &summarised(file, "The beds.")),
            format!("{short}\tfile:/n/a.md\tThe garden log\tsummary: The beds.")
        );
        let both = passage(
            A,
            (3, 5),
            Some("/src/garden"),
            Some("user: thin"),
            Some("a note"),
            false,
        );
        assert_eq!(
            only_row("sessions", &summarised(both, "The beds.")),
            format!(
                "{short}#r3-7\t2026-11-07\t/src/garden\tuser: thin\tnote: a note\tsummary: The beds."
            )
        );
        let alone = passage(
            A,
            (3, 5),
            Some("/src/garden"),
            Some("user: thin"),
            None,
            false,
        );
        assert_eq!(
            only_row("sessions", &alone),
            format!("{short}#r3-7\t2026-11-07\t/src/garden\tuser: thin")
        );
        assert_eq!(
            only_row("sessions", &summarised(alone, "The beds.")),
            format!("{short}#r3-7\t2026-11-07\t/src/garden\tuser: thin\tsummary: The beds.")
        );
        let note = passage(A, (1, 1), None, Some("kept text"), None, false);
        assert_eq!(
            only_row("notes", &note),
            format!("{short}#r1\t2026-11-07\t-\tkept text")
        );
        assert_eq!(
            only_row("notes", &summarised(note, "The beds.")),
            format!("{short}#r1\t2026-11-07\t-\tkept text\tsummary: The beds.")
        );
    }

    #[test]
    fn a_summary_is_written_as_any_field_is() {
        let file = summarised(entry(A, Some("file:/n/a.md"), Some("T"), false), "a\tb\nc");
        assert_eq!(
            only_row("files", &file),
            format!("{}\tfile:/n/a.md\tT\tsummary: a\\tb\\nc", &A[16..])
        );
    }
}
