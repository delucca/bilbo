use bilbo_store::entries::RecordRange;

use bilbo_entries::read::refs::Locator;

use crate::digest::sessions::field;
use crate::digest::{Chosen, HeldBack, Listed, Row, Said};
use crate::recall::Stand;
use crate::recall::log::joined;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Listed,
    Nothing,
    Floor,
    Later,
    Input,
    Unmarked,
    Busy,
    NoIndex,
    OtherManifest,
    Withheld,
    Failed,
}

impl End {
    pub fn word(self) -> &'static str {
        match self {
            End::Listed => "listed",
            End::Nothing => "nothing",
            End::Floor => "floor",
            End::Later => "later",
            End::Input => "input",
            End::Unmarked => "unmarked",
            End::Busy => "busy",
            End::NoIndex => "no-index",
            End::OtherManifest => "other-manifest",
            End::Withheld => "withheld",
            End::Failed => "failed",
        }
    }

    pub fn of(listed: &Listed) -> End {
        match listed.stand {
            Stand::NoIndex => return End::NoIndex,
            Stand::OtherManifest => return End::OtherManifest,
            Stand::Withheld => return End::Withheld,
            Stand::Current | Stand::Behind(_) | Stand::Unknown => {}
        }
        if listed.lists.iter().any(|chosen| !chosen.rows.is_empty()) {
            End::Listed
        } else if listed
            .lists
            .iter()
            .any(|chosen| !chosen.held_back.is_empty())
        {
            End::Floor
        } else {
            End::Nothing
        }
    }

    fn ranked(self) -> bool {
        matches!(self, End::Listed | End::Nothing | End::Floor)
    }
}

pub struct Record<'a> {
    pub at: u64,
    pub end: End,
    pub thread: Option<&'a str>,
    pub millis: u64,
    pub listed: Option<&'a Listed>,
}

fn stand(stand: Stand) -> &'static str {
    match stand {
        Stand::Current => "current",
        Stand::Behind(_) => "behind",
        Stand::Unknown => "unknown",
        Stand::NoIndex | Stand::OtherManifest | Stand::Withheld => "-",
    }
}

fn locator(range: RecordRange) -> String {
    Locator {
        generation: None,
        records: Some(range),
    }
    .to_string()
}

fn listed_item(row: &Row) -> String {
    match &row.said {
        Said::Entry { .. } => format!("{}={}", row.entry, row.score),
        Said::Passage {
            records: range,
            summary,
            ..
        } => format!(
            "{}{}={}{}",
            row.entry,
            locator(*range),
            row.score,
            if summary.is_some() { "+" } else { "" }
        ),
    }
}

fn held_item(held: &HeldBack) -> String {
    let range = held.records.map(locator).unwrap_or_default();
    format!("{}{range}={}", held.entry, held.score)
}

fn five(chosen: &Chosen) -> String {
    let passed: Vec<String> = [
        ("gone", chosen.gone),
        ("own", chosen.own),
        ("belongs", chosen.belongs),
        ("part", chosen.part),
        ("old", chosen.old),
        ("shown", chosen.shown),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(cause, count)| format!("{cause}={count}"))
    .collect();
    format!(
        "{}\t{}\t{}\t{}\t{}",
        chosen.kind,
        chosen.floor,
        joined(chosen.rows.iter().map(listed_item)),
        joined(chosen.held_back.iter().map(held_item)),
        joined(passed.into_iter())
    )
}

pub fn line(record: &Record<'_>) -> String {
    let thread = record.thread.map_or_else(|| "-".to_owned(), field);
    let standing = match (record.end.ranked(), record.listed) {
        (true, Some(listed)) => stand(listed.stand),
        _ => "-",
    };
    let mut text = format!(
        "d1\t{}\t{}\t{thread}\t{}\t{standing}",
        record.at,
        record.end.word(),
        record.millis
    );
    if let (true, Some(listed)) = (record.end.ranked(), record.listed) {
        for chosen in &listed.lists {
            text.push('\t');
            text.push_str(&five(chosen));
        }
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::RecordRange;
    use bilbo_store::id::Id;
    use bilbo_store::labels::CaptureLabel;

    use super::{End, Record, line};
    use crate::digest::{Chosen, HeldBack, Listed, Row, Said};
    use crate::recall::Stand;

    fn id(n: u8) -> Id {
        Id::from_bytes([n; 16])
    }

    fn row(n: u8, score: f32, said: Said) -> Row {
        Row {
            entry: id(n),
            capture: CaptureLabel::new(format!("file:/{n}")),
            score,
            shared_short: false,
            said,
        }
    }

    fn passage(first: u64, count: u64, summary: Option<&str>) -> Said {
        Said::Passage {
            records: RecordRange { first, count },
            day: "2026-11-07".to_owned(),
            folder: None,
            line: None,
            summary: summary.map(str::to_owned),
        }
    }

    fn empty(kind: &str, floor: f64) -> Chosen {
        Chosen {
            kind: kind.to_owned(),
            floor,
            rows: Vec::new(),
            held_back: Vec::new(),
            gone: 0,
            own: 0,
            belongs: 0,
            old: 0,
            part: 0,
            shown: 0,
        }
    }

    fn fields(text: &str) -> Vec<&str> {
        assert!(text.ends_with('\n') && text.matches('\n').count() == 1);
        text.trim_end_matches('\n').split('\t').collect()
    }

    fn made(end: End, thread: Option<&str>, listed: Option<&Listed>) -> String {
        line(&Record {
            at: 1_794_069_552,
            end,
            thread,
            millis: 171,
            listed,
        })
    }

    #[test]
    fn a_line_has_its_six_fields_and_five_for_each_list_in_order() {
        let mut files = empty("markdown", 8.0);
        files.rows.push(row(1, 14.25, Said::Entry { title: None }));
        files.held_back.push(HeldBack {
            entry: id(2),
            capture: None,
            score: 3.5,
            records: None,
        });
        let mut sessions = empty("session", 12.0);
        sessions
            .rows
            .push(row(3, 19.5, passage(3, 5, Some("a note"))));
        sessions.rows.push(row(4, 13.0, passage(18, 1, None)));
        sessions.held_back.push(HeldBack {
            entry: id(5),
            capture: None,
            score: 7.5,
            records: Some(RecordRange { first: 9, count: 4 }),
        });
        sessions.belongs = 3;
        sessions.part = 2;
        sessions.old = 1;
        sessions.shown = 4;
        sessions.own = 5;
        sessions.gone = 6;
        let notes = empty("note", 8.0);
        let listed = Listed {
            stand: Stand::Behind(2),
            lists: vec![files, sessions, notes],
        };
        let text = made(End::Listed, Some("3b9f"), Some(&listed));
        let all = fields(&text);
        assert_eq!(all.len(), 6 + 15);
        assert_eq!(
            all[..6],
            ["d1", "1794069552", "listed", "3b9f", "171", "behind"]
        );
        let (first, second, third) = (&all[6..11], &all[11..16], &all[16..21]);
        assert_eq!(first[..2], ["markdown", "8"]);
        assert_eq!(first[2], format!("{}=14.25", id(1)));
        assert_eq!(first[3], format!("{}=3.5", id(2)));
        assert_eq!(first[4], "-");
        assert_eq!(second[..2], ["session", "12"]);
        assert_eq!(second[2], format!("{}#r3-7=19.5+,{}#r18=13", id(3), id(4)));
        assert_eq!(second[3], format!("{}#r9-12=7.5", id(5)));
        assert_eq!(second[4], "gone=6,own=5,belongs=3,part=2,old=1,shown=4");
        assert_eq!(third, ["note", "8", "-", "-", "-"]);

        for (end, word) in [
            (End::Later, "later"),
            (End::Input, "input"),
            (End::Unmarked, "unmarked"),
            (End::Busy, "busy"),
            (End::NoIndex, "no-index"),
            (End::OtherManifest, "other-manifest"),
            (End::Withheld, "withheld"),
            (End::Failed, "failed"),
        ] {
            let text = made(end, None, Some(&listed));
            let all = fields(&text);
            assert_eq!(all.len(), 6, "{word}");
            assert_eq!(all[2], word);
            assert_eq!(all[3], "-");
            assert_eq!(all[5], "-", "{word}");
        }
        for (end, word) in [
            (End::Listed, "listed"),
            (End::Nothing, "nothing"),
            (End::Floor, "floor"),
        ] {
            assert_eq!(fields(&made(end, None, Some(&listed)))[2], word);
        }
        let mut held = empty("markdown", 8.0);
        held.held_back.push(HeldBack {
            entry: id(2),
            capture: None,
            score: 3.5,
            records: None,
        });
        let floor = Listed {
            stand: Stand::Behind(1),
            lists: vec![held, empty("session", 12.0)],
        };
        assert_eq!(
            fields(&made(End::Floor, Some("s"), Some(&floor))),
            [
                "d1".to_owned(),
                "1794069552".to_owned(),
                "floor".to_owned(),
                "s".to_owned(),
                "171".to_owned(),
                "behind".to_owned(),
                "markdown".to_owned(),
                "8".to_owned(),
                "-".to_owned(),
                format!("{}=3.5", id(2)),
                "-".to_owned(),
                "session".to_owned(),
                "12".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
            ]
        );
        assert_eq!(fields(&made(End::Input, Some("s"), None))[3], "s");

        let none = Listed {
            stand: Stand::Current,
            lists: Vec::new(),
        };
        let text = made(End::Nothing, Some("s"), Some(&none));
        assert_eq!(fields(&text).len(), 6);
        assert_eq!(fields(&text)[5], "current");
        let three = Listed {
            stand: Stand::Unknown,
            lists: vec![
                empty("markdown", 8.0),
                empty("session", 12.0),
                empty("note", 8.0),
            ],
        };
        let text = made(End::Nothing, Some("s"), Some(&three));
        let all = fields(&text);
        assert_eq!(all.len(), 21);
        assert_eq!(all[5], "unknown");
        for five in all[6..].chunks(5) {
            assert_eq!(five[2..], ["-", "-", "-"]);
        }
        assert_eq!(End::of(&three), End::Nothing);
        assert_eq!(End::of(&listed), End::Listed);
        let mut held = empty("markdown", 8.0);
        held.held_back.push(HeldBack {
            entry: id(1),
            capture: None,
            score: 1.0,
            records: None,
        });
        assert_eq!(
            End::of(&Listed {
                stand: Stand::Current,
                lists: vec![held]
            }),
            End::Floor
        );
        for (stand, end) in [
            (Stand::NoIndex, End::NoIndex),
            (Stand::OtherManifest, End::OtherManifest),
            (Stand::Withheld, End::Withheld),
        ] {
            assert_eq!(
                End::of(&Listed {
                    stand,
                    lists: Vec::new()
                }),
                end
            );
        }
    }

    #[test]
    fn a_session_cannot_end_the_line_or_add_a_field() {
        for (thread, escaped) in [
            ("tab\there", "tab\\there"),
            ("line\nfeed", "line\\nfeed"),
            ("back\\slash", "back\\\\slash"),
            ("a\rb", "a\\rb"),
            ("\u{1b}[0m", "\\u{1b}[0m"),
        ] {
            let text = made(End::Later, Some(thread), None);
            let all = fields(&text);
            assert_eq!(all.len(), 6, "{thread:?}");
            assert_eq!(all[3], escaped, "{thread:?}");
        }
        assert_eq!(fields(&made(End::Later, Some("-"), None))[3], "\\-");
        assert_eq!(fields(&made(End::Later, None, None))[3], "-");
        assert_eq!(
            fields(&made(End::Later, Some("tab\there"), None))[3],
            "tab\\there"
        );
    }

    #[test]
    fn a_score_reads_back_as_the_same_number() {
        let scores = [
            8.0_f32,
            f32::from_bits(8.0_f32.to_bits() - 1),
            f32::from_bits(8.0_f32.to_bits() + 1),
            14.250_732,
            0.123_456_79,
            1234.5677,
        ];
        for score in scores {
            let mut files = empty("markdown", 8.0);
            files.rows.push(row(1, score, Said::Entry { title: None }));
            let listed = Listed {
                stand: Stand::Current,
                lists: vec![files],
            };
            let text = made(End::Listed, None, Some(&listed));
            let item = fields(&text)[8].to_owned();
            let written = item
                .split_once('=')
                .map(|(_, text)| text)
                .unwrap_or_default();
            let read: f32 = written.parse().unwrap_or(f32::NAN);
            assert_eq!(read.to_bits(), score.to_bits(), "{written}");
        }
    }
}
