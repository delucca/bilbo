use bilbo_store::id::Id;
use serde_json::Value;

use super::render::locator;
use crate::read::refs::Ref;

// A time past the year 9999 has no four-digit day.
const LAST_SECOND: u64 = 253_402_300_799;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub entry: Id,
    pub generation: u32,
    pub first: u64,
    pub last: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub text: String,
    pub at: Option<u64>,
    pub from: Vec<Source>,
}

fn quoted(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

pub fn write(record: &Record) -> Vec<u8> {
    let mut line = format!("{{\"text\":{}", quoted(&record.text));
    if let Some(at) = record.at {
        line.push_str(&format!(",\"at\":{at}"));
    }
    if !record.from.is_empty() {
        let sources: Vec<String> = record
            .from
            .iter()
            .map(|source| {
                format!(
                    "{{\"entry\":{},\"generation\":{},\"records\":[{},{}]}}",
                    quoted(&source.entry.to_string()),
                    source.generation,
                    source.first,
                    source.last
                )
            })
            .collect();
        line.push_str(&format!(",\"from\":[{}]", sources.join(",")));
    }
    line.push_str("}\n");
    line.into_bytes()
}

pub fn read(bytes: &[u8]) -> Option<Record> {
    let Value::Object(fields) = serde_json::from_slice(bytes).ok()? else {
        return None;
    };
    let text = fields
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())?
        .to_owned();
    let at = fields
        .get("at")
        .and_then(Value::as_u64)
        .filter(|at| *at <= LAST_SECOND);
    let from = fields
        .get("from")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(source).collect())
        .unwrap_or_default();
    Some(Record { text, at, from })
}

fn source(item: &Value) -> Option<Source> {
    let entry = Id::parse(item.get("entry")?.as_str()?)?;
    let generation = u32::try_from(item.get("generation")?.as_u64()?)
        .ok()
        .filter(|generation| *generation >= 1)?;
    let [first, last] = item.get("records")?.as_array()?.as_slice() else {
        return None;
    };
    let (first, last) = (first.as_u64()?, last.as_u64()?);
    let source = Source {
        entry,
        generation,
        first,
        last,
    };
    (first >= 1 && last >= first && Ref::parse(&locator(&source)).is_ok()).then_some(source)
}

#[cfg(test)]
mod tests {
    use bilbo_store::id::Id;

    use super::{Record, Source, read, write};
    use crate::note::render::locator;
    use crate::read::refs::Ref;

    const ENTRY: &str = "01M6ZGE4KTBYYH07AS2KVMSCAF";

    fn source(first: u64, last: u64) -> Source {
        Source {
            entry: Id::parse(ENTRY).expect("an id"),
            generation: 1,
            first,
            last,
        }
    }

    #[test]
    fn a_note_record_is_one_json_line_text_then_at_then_from() {
        let record = Record {
            text: "The user asked.".to_owned(),
            at: Some(1_794_039_249),
            from: vec![source(3, 19)],
        };
        assert_eq!(
            String::from_utf8(write(&record)).expect("utf-8"),
            format!(
                "{{\"text\":\"The user asked.\",\"at\":1794039249,\"from\":[{{\"entry\":\"{ENTRY}\",\"generation\":1,\"records\":[3,19]}}]}}\n"
            )
        );
        let bare = Record {
            text: "a".to_owned(),
            at: None,
            from: Vec::new(),
        };
        assert_eq!(write(&bare), b"{\"text\":\"a\"}\n");
    }

    #[test]
    fn a_written_record_reads_back_the_same() {
        for text in [
            "a \"quote\"",
            "back\\slash",
            "line\nfeed",
            "tab\there",
            "nul\u{0}byte",
            "caf\u{e9}",
            "sep\u{2028}arator",
            "emoji \u{1F331}",
        ] {
            let record = Record {
                text: text.to_owned(),
                at: Some(0),
                from: vec![source(1, 1), source(2, 9)],
            };
            let line = write(&record);
            assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1);
            assert_eq!(read(&line), Some(record), "{text:?}");
        }
    }

    #[test]
    fn a_record_without_text_is_no_note() {
        for bytes in [
            &b"{\"run\":{}}"[..],
            b"{}",
            b"{\"text\":\"\"}",
            b"{\"text\":5}",
            b"[1]",
            b"x",
            b"",
            b"{\"text\":\"\xff\"}",
        ] {
            assert_eq!(read(bytes), None, "{bytes:?}");
        }
    }

    #[test]
    fn a_field_the_reader_does_not_know_is_ignored() {
        assert_eq!(
            read(b"{\"text\":\"a\",\"author\":\"me\",\"x\":[1]}"),
            Some(Record {
                text: "a".to_owned(),
                at: None,
                from: Vec::new(),
            })
        );
    }

    fn note(at: Option<u64>, from: Vec<Source>) -> Option<Record> {
        Some(Record {
            text: "a".to_owned(),
            at,
            from,
        })
    }

    #[test]
    fn a_bad_time_or_source_is_dropped_and_the_note_stays() {
        for at in [
            "\"x\"",
            "-1",
            "1.5",
            "253402300800",
            "99999999999999999999999",
        ] {
            let found = read(format!("{{\"text\":\"a\",\"at\":{at}}}").as_bytes());
            assert_eq!(found, note(None, Vec::new()), "{at}");
        }
        assert_eq!(read(b"{\"text\":\"carrot\",\"at\":1e400}"), None);
        let nested = format!(
            "{{\"text\":\"carrot\",\"extra\":{}{}}}",
            "[".repeat(200),
            "]".repeat(200)
        );
        assert_eq!(read(nested.as_bytes()), None);
        assert_eq!(
            read(b"{\"text\":\"a\",\"at\":253402300799}"),
            note(Some(253_402_300_799), Vec::new())
        );
        let good = format!("{{\"entry\":\"{ENTRY}\",\"generation\":2,\"records\":[1,4]}}");
        let bad = [
            "{\"entry\":\"nope\",\"generation\":1,\"records\":[1,2]}".to_owned(),
            format!("{{\"entry\":\"{ENTRY}\",\"generation\":0,\"records\":[1,2]}}"),
            format!("{{\"entry\":\"{ENTRY}\",\"generation\":1,\"records\":[5,3]}}"),
            format!("{{\"entry\":\"{ENTRY}\",\"generation\":1,\"records\":[0,1]}}"),
            format!("{{\"entry\":\"{ENTRY}\",\"generation\":1,\"records\":[1]}}"),
        ];
        let mut items = bad.to_vec();
        items.insert(2, good);
        let line = format!("{{\"text\":\"a\",\"from\":[{}]}}", items.join(","));
        assert_eq!(
            read(line.as_bytes()),
            note(
                None,
                vec![Source {
                    generation: 2,
                    ..source(1, 4)
                }]
            )
        );
    }

    #[test]
    fn a_source_read_cannot_name_is_dropped_and_the_note_stays() {
        let from = |last: &str| {
            format!(
                "{{\"text\":\"a\",\"at\":5,\"from\":[{{\"entry\":\"{ENTRY}\",\"generation\":1,\"records\":[1,{last}]}},{{\"entry\":\"{ENTRY}\",\"generation\":2,\"records\":[2,3]}}]}}"
            )
        };
        let kept = source(1, 18_446_744_073_709_551_614);
        let found = read(from("18446744073709551614").as_bytes());
        assert_eq!(
            found,
            Some(Record {
                text: "a".to_owned(),
                at: Some(5),
                from: vec![
                    kept.clone(),
                    Source {
                        generation: 2,
                        ..source(2, 3)
                    }
                ],
            })
        );
        assert!(Ref::parse(&locator(&kept)).is_ok());
        assert_eq!(
            read(from("18446744073709551615").as_bytes()),
            Some(Record {
                text: "a".to_owned(),
                at: Some(5),
                from: vec![Source {
                    generation: 2,
                    ..source(2, 3)
                }],
            })
        );
    }
}
