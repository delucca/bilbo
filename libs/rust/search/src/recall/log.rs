use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use bilbo_kinds::read::refs::Locator;

use super::hit::Hit;

pub trait QueryLog {
    fn append(&mut self, line: &str) -> std::io::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Hits,
    Nothing,
    NoIndex,
    OtherManifest,
    Withheld,
    Failed,
}

impl Ended {
    fn word(self) -> &'static str {
        match self {
            Self::Hits => "hits",
            Self::Nothing => "nothing",
            Self::NoIndex => "no-index",
            Self::OtherManifest => "other-manifest",
            Self::Withheld => "withheld",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    pub at: SystemTime,
    pub ended: Ended,
    pub searched: u64,
    pub kinds: &'a [String],
    pub hits: &'a [Hit],
    pub query: &'a str,
}

fn joined(items: impl Iterator<Item = String>) -> String {
    let joined: Vec<String> = items.collect();
    if joined.is_empty() {
        "-".to_owned()
    } else {
        joined.join(",")
    }
}

fn escaped(query: &str) -> String {
    let mut text = String::with_capacity(query.len());
    for c in query.chars() {
        match c {
            '\\' => text.push_str("\\\\"),
            '\t' => text.push_str("\\t"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(text, "\\u{{{:x}}}", u32::from(c));
            }
            c => text.push(c),
        }
    }
    text
}

fn shown(hit: &Hit) -> String {
    let locator = Locator {
        generation: None,
        records: Some(hit.found.extent.records),
    };
    let cut = if hit.cut { "!" } else { "" };
    format!("{}{locator}{cut}", hit.found.entry)
}

pub fn line(record: &Record<'_>) -> String {
    let seconds = record
        .at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |after| after.as_secs());
    format!(
        "q1\t{seconds}\t{}\t{}\t{}\t{}\t{}\n",
        record.ended.word(),
        record.searched,
        joined(record.kinds.iter().cloned()),
        joined(record.hits.iter().map(shown)),
        escaped(record.query),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{Ended, Record, line};
    use crate::index::Extent;
    use crate::lookup::Found;
    use crate::recall::hit::Hit;

    const FIRST: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
    const SECOND: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DC";

    fn hit(id: &str, first: u64, count: u64, cut: bool) -> Hit {
        Hit {
            found: Found {
                entry: Id::parse(id).expect("an id"),
                kind: "note".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange { first, count },
                    part: None,
                    date: 0,
                },
                digest: 0,
            },
            preface: None,
            text: String::new(),
            cut,
        }
    }

    fn record<'a>(
        ended: Ended,
        kinds: &'a [String],
        hits: &'a [Hit],
        query: &'a str,
    ) -> Record<'a> {
        Record {
            at: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            ended,
            searched: 7,
            kinds,
            hits,
            query,
        }
    }

    #[test]
    fn a_line_has_its_seven_fields_in_order() {
        let kinds = ["markdown".to_owned(), "session".to_owned()];
        let hits = [hit(FIRST, 3, 1, false), hit(SECOND, 2, 5, true)];
        for (ended, word) in [
            (Ended::Hits, "hits"),
            (Ended::Nothing, "nothing"),
            (Ended::NoIndex, "no-index"),
            (Ended::OtherManifest, "other-manifest"),
            (Ended::Withheld, "withheld"),
            (Ended::Failed, "failed"),
        ] {
            assert_eq!(
                line(&record(ended, &kinds, &hits, "release checklist")),
                format!(
                    "q1\t1700000000\t{word}\t7\tmarkdown,session\t{FIRST}#r3,{SECOND}#r2-6!\trelease checklist\n"
                )
            );
        }
        assert_eq!(
            line(&record(Ended::Nothing, &[], &[], "x")),
            "q1\t1700000000\tnothing\t7\t-\t-\tx\n"
        );
        assert_eq!(
            line(&record(Ended::Hits, &kinds[..1], &hits[1..], "x")),
            format!("q1\t1700000000\thits\t7\tmarkdown\t{SECOND}#r2-6!\tx\n")
        );
    }

    #[test]
    fn a_query_cannot_end_the_line_or_add_a_field() {
        for (query, written) in [
            ("a\tb", "a\\tb"),
            ("a\nb", "a\\nb"),
            ("a\r\nb", "a\\r\\nb"),
            ("a\\b", "a\\\\b"),
            ("a\\nb", "a\\\\nb"),
            ("a\u{0}b", "a\\u{0}b"),
            ("a\u{1b}[31mb", "a\\u{1b}[31mb"),
            ("a\u{7f}b", "a\\u{7f}b"),
            ("a\u{85}b", "a\\u{85}b"),
            ("a\u{2028}b", "a\u{2028}b"),
            ("ação ünï", "ação ünï"),
        ] {
            let text = line(&record(Ended::Hits, &[], &[], query));
            assert_eq!(text, format!("q1\t1700000000\thits\t7\t-\t-\t{written}\n"));
            assert_eq!(text.matches('\n').count(), 1, "{query:?}");
            assert_eq!(text.split('\t').count(), 7, "{query:?}");
        }
        let long = "word ".repeat(10_000);
        let text = line(&record(Ended::Hits, &[], &[], &long));
        assert!(text.ends_with(&format!("\t{long}\n")));
    }

    #[test]
    fn a_time_before_1970_is_second_0() {
        let mut before = record(Ended::Nothing, &[], &[], "x");
        before.at = UNIX_EPOCH - Duration::from_secs(5);
        assert!(line(&before).starts_with("q1\t0\t"));
        before.at = UNIX_EPOCH;
        assert!(line(&before).starts_with("q1\t0\t"));
    }
}
