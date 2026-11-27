use std::io;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use crate::instruction::number;
use crate::instruction::number_of;
use crate::tries;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct About {
    pub generation: Generation,
    pub records: u64,
    pub content: u128,
    pub notes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Passed { entry: Id, asking: u64 },
    Charged { entry: Id, asking: u64, charges: u8 },
}

#[must_use]
pub fn line(line: &Line) -> String {
    match line {
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
    part(&mut bytes, &about.content.to_le_bytes());
    match about.notes {
        None => part(&mut bytes, &[0]),
        Some(digest) => {
            let mut held = vec![1];
            held.extend_from_slice(&digest.to_le_bytes());
            part(&mut bytes, &held);
        }
    }
    part(&mut bytes, provider.as_bytes());
    part(&mut bytes, model.as_bytes());
    part(&mut bytes, instruction.as_bytes());
    part(&mut bytes, material.as_bytes());
    number(&bytes)
}

pub trait Asked {
    fn passed(&self, entry: Id) -> io::Result<Option<u64>>;
    fn charges(&self, entry: Id, asking: u64) -> io::Result<u8>;
    fn charge(&mut self, entry: Id, asking: u64) -> io::Result<()>;
    fn pass(&mut self, entry: Id, asking: u64) -> io::Result<()>;
    fn settle(&mut self, entry: Id) -> io::Result<()>;
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{About, Line, asking, line, parse};

    const ENTRY: &str = "01M6ZGE4KTBYYH07AS2KVMSCAF";
    const OTHER: &str = "01M6ZGE4KTBYYH07AS2KVMSCAG";

    fn id(text: &str) -> Id {
        Id::parse(text).expect("id")
    }

    #[test]
    fn each_line_form_reads_back() {
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
            (passed, format!("passed\t{ENTRY}\t00000000000000ff")),
            (charged, format!("charged\t{ENTRY}\t0000000000abcdef\t2")),
        ] {
            assert_eq!(line(&value), text);
            assert_eq!(parse(&text), Some(value));
        }
    }

    #[test]
    fn a_line_that_is_none_of_the_two_reads_as_none() {
        for text in [
            format!("answered\t{ENTRY}\t2\t14\t9ea9aba2ae5ed554\tanthropic\tmodel-a\tbilbo"),
            "passed\tnot-an-id\t00000000000000ff".to_owned(),
            format!("passed\t{ENTRY}\t0000000000000ff"),
            format!("charged\t{ENTRY}\t00000000000000ff\t0"),
            format!("charged\t{ENTRY}\t00000000000000ff\t10"),
            format!("charged\t{ENTRY}\t00000000000000ff"),
            String::new(),
        ] {
            assert_eq!(parse(&text), None, "{text:?}");
        }
    }

    #[test]
    fn an_asking_is_numbered_by_every_part_of_it() {
        let about = About {
            generation: Generation(2),
            records: 14,
            content: 0xabcd,
            notes: Some(1),
        };
        let number = |entry: &str, about: &About, parts: [&str; 4]| {
            asking(id(entry), about, parts[0], parts[1], parts[2], parts[3])
        };
        let parts = ["anthropic", "model-a", "7fdf9f0d93f443e3", "material"];
        let base = number(ENTRY, &about, parts);
        assert_eq!(base, number(ENTRY, &about, parts));
        let mut others = vec![
            number(OTHER, &about, parts),
            number(
                ENTRY,
                &About {
                    generation: Generation(3),
                    ..about
                },
                parts,
            ),
            number(
                ENTRY,
                &About {
                    records: 15,
                    ..about
                },
                parts,
            ),
            number(
                ENTRY,
                &About {
                    content: 0xabce,
                    ..about
                },
                parts,
            ),
            number(
                ENTRY,
                &About {
                    notes: None,
                    ..about
                },
                parts,
            ),
            number(
                ENTRY,
                &About {
                    notes: Some(0),
                    ..about
                },
                parts,
            ),
            number(
                ENTRY,
                &About {
                    notes: Some(2),
                    ..about
                },
                parts,
            ),
        ];
        for at in 0..4 {
            let mut changed = parts;
            changed[at] = "other";
            others.push(number(ENTRY, &about, changed));
        }
        others.push(base);
        let mut sorted = others.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), others.len());
    }
}
