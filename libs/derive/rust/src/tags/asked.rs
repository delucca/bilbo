use std::collections::BTreeSet;
use std::io;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;
use bilbo_store::labels::Tag;

use crate::instruction::{number, number_of};
use crate::tries;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct About {
    pub generation: Generation,
    pub records: u64,
    pub notes: u64,
}

#[must_use]
pub fn digest(before: Option<u64>, record: &[u8]) -> u64 {
    let mut bytes = vec![u8::from(before.is_some())];
    if let Some(before) = before {
        bytes.extend_from_slice(&before.to_le_bytes());
    }
    bytes.extend_from_slice(record);
    number(&bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub entry: Id,
    pub notes: (Generation, u64),
    pub instruction: String,
    pub provider: String,
    pub model: String,
    pub names: BTreeSet<Tag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Answered(Answered),
    Passed { entry: Id, asking: u64 },
    Charged { entry: Id, asking: u64, charges: u8 },
}

fn joined(names: &BTreeSet<Tag>) -> String {
    names.iter().map(Tag::as_str).collect::<Vec<_>>().join(",")
}

#[must_use]
pub fn line(line: &Line) -> String {
    match line {
        Line::Answered(answered) => format!(
            "answered\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            answered.entry,
            answered.notes.0.0,
            answered.notes.1,
            answered.instruction,
            answered.provider,
            answered.model,
            joined(&answered.names)
        ),
        Line::Passed { entry, asking } => format!("passed\t{entry}\t{asking:016x}"),
        Line::Charged {
            entry,
            asking,
            charges,
        } => format!("charged\t{entry}\t{asking:016x}\t{charges}"),
    }
}

#[must_use]
pub fn parse(text: &str) -> Option<Line> {
    let fields: Vec<&str> = text.split('\t').collect();
    match fields.as_slice() {
        [
            "answered",
            entry,
            generation,
            records,
            instruction,
            provider,
            model,
            names,
        ] => {
            let names: BTreeSet<Tag> = names.split(',').map(Tag::new).collect::<Option<_>>()?;
            Some(Line::Answered(Answered {
                entry: Id::parse(entry)?,
                notes: (Generation(generation.parse().ok()?), records.parse().ok()?),
                instruction: (*instruction).to_owned(),
                provider: (*provider).to_owned(),
                model: (*model).to_owned(),
                names,
            }))
        }
        ["passed", entry, asking] => Some(Line::Passed {
            entry: Id::parse(entry)?,
            asking: number_of(asking)?,
        }),
        ["charged", entry, asking, charges] => {
            let charges: u8 = charges.parse().ok()?;
            (1..=tries::MOST).contains(&charges).then_some(())?;
            Some(Line::Charged {
                entry: Id::parse(entry)?,
                asking: number_of(asking)?,
                charges,
            })
        }
        _ => None,
    }
}

fn part(bytes: &mut Vec<u8>, text: &[u8]) {
    bytes.extend_from_slice(&u64::try_from(text.len()).unwrap_or(u64::MAX).to_le_bytes());
    bytes.extend_from_slice(text);
}

#[must_use]
pub fn asking(
    entry: Id,
    about: &About,
    provider: &str,
    model: &str,
    instruction: &str,
    material: &str,
) -> u64 {
    let mut bytes = Vec::new();
    part(&mut bytes, entry.to_string().as_bytes());
    part(&mut bytes, &about.generation.0.to_le_bytes());
    part(&mut bytes, &about.records.to_le_bytes());
    part(&mut bytes, &about.notes.to_le_bytes());
    part(&mut bytes, provider.as_bytes());
    part(&mut bytes, model.as_bytes());
    part(&mut bytes, instruction.as_bytes());
    part(&mut bytes, material.as_bytes());
    number(&bytes)
}

pub trait Asked {
    fn answered(&self, entry: Id) -> io::Result<Option<Answered>>;
    fn passed(&self, entry: Id) -> io::Result<Option<u64>>;
    fn charges(&self, entry: Id, asking: u64) -> io::Result<u8>;
    fn charge(&mut self, entry: Id, asking: u64) -> io::Result<()>;
    fn answer(&mut self, answered: &Answered, asking: u64) -> io::Result<()>;
    fn pass(&mut self, entry: Id, asking: u64) -> io::Result<()>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::Tag;

    use super::{About, Answered, Line, asking, digest, line, parse};

    const ENTRY: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
    const OTHER: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DC";

    fn id(text: &str) -> Id {
        Id::parse(text).expect("id")
    }

    fn names(list: &[&str]) -> BTreeSet<Tag> {
        list.iter()
            .map(|name| Tag::new(name).expect("a tag"))
            .collect()
    }

    #[test]
    fn each_line_form_reads_back() {
        let answered = Line::Answered(Answered {
            entry: id(ENTRY),
            notes: (Generation(2), 14),
            instruction: "9ea9aba2ae5ed554".to_owned(),
            provider: "anthropic".to_owned(),
            model: "model-a".to_owned(),
            names: names(&["garden", "bilbo"]),
        });
        let passed = Line::Passed {
            entry: id(ENTRY),
            asking: 255,
        };
        let charged = Line::Charged {
            entry: id(ENTRY),
            asking: 0xabcdef,
            charges: 2,
        };
        for (value, text) in [
            (
                answered,
                format!(
                    "answered\t{ENTRY}\t2\t14\t9ea9aba2ae5ed554\tanthropic\tmodel-a\tbilbo,garden"
                ),
            ),
            (passed, format!("passed\t{ENTRY}\t00000000000000ff")),
            (charged, format!("charged\t{ENTRY}\t0000000000abcdef\t2")),
        ] {
            assert_eq!(line(&value), text);
            assert_eq!(parse(&text), Some(value));
        }
    }

    #[test]
    fn a_line_that_does_not_parse_is_no_line() {
        let asked = "00000000000000ff";
        for text in [
            format!("answer\t{ENTRY}\t2\t14\ti\tp\tm\ta"),
            format!("answered\t{ENTRY}\t2\t14\ti\tp\ta"),
            format!("answered\t{ENTRY}\t2\t14\ti\tp\tm\ta\tb"),
            format!("passed\tnot-an-id\t{asked}"),
            "passed\t".to_owned() + ENTRY + "\t0000000000000ff",
            format!("charged\t{ENTRY}\t{asked}\t0"),
            format!("charged\t{ENTRY}\t{asked}\t10"),
            format!("answered\t{ENTRY}\t2\t14\ti\tp\tm\t"),
        ] {
            assert_eq!(parse(&text), None, "{text}");
        }
    }

    fn base() -> (Id, About) {
        (
            id(ENTRY),
            About {
                generation: Generation(1),
                records: 10,
                notes: 7,
            },
        )
    }

    #[test]
    fn an_askings_number_changes_with_each_of_its_parts() {
        let (entry, about) = base();
        let first = asking(
            entry,
            &about,
            "anthropic",
            "model-a",
            "instruction",
            "material",
        );
        assert_eq!(
            first,
            asking(
                entry,
                &about,
                "anthropic",
                "model-a",
                "instruction",
                "material"
            )
        );
        let changed = [
            asking(
                id(OTHER),
                &about,
                "anthropic",
                "model-a",
                "instruction",
                "material",
            ),
            asking(
                entry,
                &About {
                    generation: Generation(2),
                    ..about
                },
                "anthropic",
                "model-a",
                "instruction",
                "material",
            ),
            asking(
                entry,
                &About {
                    records: 11,
                    ..about
                },
                "anthropic",
                "model-a",
                "instruction",
                "material",
            ),
            asking(
                entry,
                &About { notes: 8, ..about },
                "anthropic",
                "model-a",
                "instruction",
                "material",
            ),
            asking(
                entry,
                &about,
                "openai",
                "model-a",
                "instruction",
                "material",
            ),
            asking(
                entry,
                &about,
                "anthropic",
                "model-b",
                "instruction",
                "material",
            ),
            asking(
                entry,
                &about,
                "anthropic",
                "model-a",
                "instructioN",
                "material",
            ),
            asking(
                entry,
                &about,
                "anthropic",
                "model-a",
                "instruction",
                "materiaL",
            ),
        ];
        let mut all: Vec<u64> = changed.to_vec();
        all.push(first);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 9);
    }

    #[test]
    fn no_two_lists_of_parts_give_the_same_bytes() {
        let (entry, about) = base();
        assert_ne!(
            asking(entry, &about, "ab", "c", "i", "m"),
            asking(entry, &about, "a", "bc", "i", "m")
        );
        assert_ne!(
            asking(entry, &about, "p", "m", "ab", "c"),
            asking(entry, &about, "p", "m", "a", "bc")
        );
        assert_ne!(
            asking(entry, &about, "p", "m", "i", "ab"),
            asking(entry, &about, "p", "m", "ia", "b")
        );
    }

    fn folded(records: &[&str]) -> u64 {
        let mut before = None;
        for record in records {
            before = Some(digest(before, record.as_bytes()));
        }
        before.expect("a record")
    }

    #[test]
    fn the_notes_digest_is_of_every_note_in_order() {
        let mut all = vec![
            folded(&["a", "b"]),
            folded(&["c", "b"]),
            folded(&["b", "a"]),
            folded(&["a"]),
            folded(&["ab"]),
        ];
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 5);
        assert_eq!(folded(&["a", "b"]), folded(&["a", "b"]));
        assert_ne!(digest(None, b"a\n"), digest(None, b"a"));
        let body: &[u8] = br#"{"text":"x","from":[1,1]}"#;
        let ended = [body, b"\n"].concat();
        assert_ne!(digest(None, body), digest(None, &ended));
    }
}
