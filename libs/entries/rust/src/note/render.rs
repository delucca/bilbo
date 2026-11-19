use std::time::{Duration, UNIX_EPOCH};

use crate::day;
use crate::note::record::{Record, Source};
use crate::read::refs::Locator;
use crate::reader::{Passage, Piece};
use bilbo_store::entries::{Generation, RecordRange};

pub(super) fn locator(source: &Source) -> String {
    let found = Locator {
        generation: Some(Generation(source.generation)),
        records: Some(RecordRange {
            first: source.first,
            count: source.last.saturating_sub(source.first).saturating_add(1),
        }),
    };
    format!("{}{found}", source.entry)
}

fn source_line(record: &Record) -> String {
    let mut parts: Vec<String> = record.from.iter().map(locator).collect();
    if let Some(at) = record.at {
        parts.push(day::utc(UNIX_EPOCH + Duration::from_secs(at)));
    }
    parts.join(" ")
}

pub fn passage(record: &Record, number: u64) -> Passage {
    let outline = record
        .text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .to_owned();
    let source = source_line(record);
    let text = format!("{source}\n{}", record.text);
    Passage {
        range: RecordRange {
            first: number,
            count: 1,
        },
        pieces: vec![Piece {
            start: source.len() + 1,
            label: source,
        }],
        ends: vec![text.len()],
        text,
        at: record.at.map(|at| UNIX_EPOCH + Duration::from_secs(at)),
        outline: Some(outline),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use bilbo_store::entries::RecordRange;
    use bilbo_store::id::Id;

    use super::passage;
    use crate::note::record::read;
    use crate::note::record::{Record, Source};
    use crate::reader::{Passage, Piece};

    const ENTRY: &str = "01M6ZGE4KTBYYH07AS2KVMSCAF";
    const OTHER: &str = "01M6ZGE4KTBYYH07AS2KVMSCAG";
    const DAY: u64 = 1_794_039_249;

    fn source(entry: &str, first: u64, last: u64) -> Source {
        Source {
            entry: Id::parse(entry).expect("an id"),
            generation: 1,
            first,
            last,
        }
    }

    fn record(at: Option<u64>, from: Vec<Source>) -> Record {
        Record {
            text: "First line.\nSecond line.".to_owned(),
            at,
            from,
        }
    }

    #[test]
    fn a_passage_opens_with_its_sources_and_its_day() {
        let day = "2026-11-07";
        for (given, expected) in [
            (
                record(Some(DAY), vec![source(ENTRY, 3, 19)]),
                format!("{ENTRY}#g1:r3-19 {day}\nFirst line.\nSecond line."),
            ),
            (
                record(Some(DAY), vec![source(ENTRY, 5, 5)]),
                format!("{ENTRY}#g1:r5 {day}\nFirst line.\nSecond line."),
            ),
            (
                record(None, vec![source(ENTRY, 1, 2), source(OTHER, 4, 4)]),
                format!("{ENTRY}#g1:r1-2 {OTHER}#g1:r4\nFirst line.\nSecond line."),
            ),
            (
                record(Some(DAY), Vec::new()),
                format!("{day}\nFirst line.\nSecond line."),
            ),
            (
                record(None, Vec::new()),
                "\nFirst line.\nSecond line.".to_owned(),
            ),
        ] {
            assert_eq!(passage(&given, 1).text, expected);
        }
        let by_hand = include_str!("../../fixtures/note/by-hand.jsonl");
        let held = by_hand
            .lines()
            .map(|line| read(line.as_bytes()).expect("a note"));
        let texts: Vec<String> = held.map(|held| passage(&held, 1).text).collect();
        assert_eq!(texts[0], "\nThe beet seeds are in the drawer by the door.");
        assert_eq!(texts[2], "\nThe fence is mended.");
    }

    #[test]
    fn the_source_line_is_the_passages_one_label() {
        let with = passage(&record(Some(DAY), vec![source(ENTRY, 3, 19)]), 4);
        let line = format!("{ENTRY}#g1:r3-19 2026-11-07");
        let text = format!("{line}\nFirst line.\nSecond line.");
        assert_eq!(
            with,
            Passage {
                range: RecordRange { first: 4, count: 1 },
                pieces: vec![Piece {
                    start: line.len() + 1,
                    label: line,
                }],
                ends: vec![text.len()],
                text,
                at: Some(UNIX_EPOCH + Duration::from_secs(DAY)),
                outline: Some("First line.".to_owned()),
            }
        );
        assert_eq!(
            passage(&record(None, Vec::new()), 1),
            Passage {
                range: RecordRange { first: 1, count: 1 },
                pieces: vec![Piece {
                    start: 1,
                    label: String::new(),
                }],
                ends: vec![25],
                text: "\nFirst line.\nSecond line.".to_owned(),
                at: None,
                outline: Some("First line.".to_owned()),
            }
        );
    }

    #[test]
    fn the_outline_is_the_first_line_of_the_paragraph_that_is_not_blank() {
        let outline = |text: &str| {
            let given = Record {
                text: text.to_owned(),
                at: None,
                from: Vec::new(),
            };
            passage(&given, 1).outline
        };
        assert_eq!(
            outline("\n  \nThe fence is mended."),
            Some("The fence is mended.".to_owned())
        );
        assert_eq!(outline("\n\n "), Some(String::new()));
        assert_eq!(outline("One line."), Some("One line.".to_owned()));
    }
}
