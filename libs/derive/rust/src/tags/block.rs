use std::collections::BTreeSet;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;
use bilbo_store::labels::Tag;

use super::asked::About;

const MAGIC: &str = "g1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub entry: Id,
    pub about: About,
    pub picks: Vec<Tag>,
    pub takes_back: bool,
    pub names: BTreeSet<Tag>,
    pub instruction: String,
    pub provider: String,
    pub model: String,
    pub asking: u64,
    pub passed: bool,
}

fn names<'a>(tags: impl Iterator<Item = &'a Tag>) -> String {
    let joined = tags.map(Tag::as_str).collect::<Vec<_>>().join(",");
    if joined.is_empty() {
        "-".to_owned()
    } else {
        joined
    }
}

#[must_use]
pub fn write(block: &Block) -> Vec<u8> {
    format!(
        "{MAGIC}\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        block.entry,
        block.about.generation.0,
        block.about.records,
        block.about.notes,
        block.asking,
        u8::from(block.passed),
        u8::from(block.takes_back),
        block.instruction,
        block.provider,
        block.model,
        names(block.names.iter()),
        names(block.picks.iter())
    )
    .into_bytes()
}

fn flag(text: &str) -> Result<bool, String> {
    match text {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("{text} is neither 0 nor 1")),
    }
}

fn tags(text: &str) -> Result<Vec<Tag>, String> {
    if text == "-" {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|name| Tag::new(name).ok_or_else(|| format!("{name} is no tag's name")))
        .collect()
}

pub fn read(bytes: &[u8]) -> Result<Block, String> {
    let body = bytes
        .strip_suffix(b"\n")
        .ok_or("it does not end in a line feed")?;
    let text = std::str::from_utf8(body).map_err(|_| "it is not UTF-8")?;
    let fields: Vec<&str> = text.split('\t').collect();
    let [
        magic,
        entry,
        generation,
        records,
        notes,
        asking,
        passed,
        takes_back,
        instruction,
        provider,
        model,
        held,
        picks,
    ] = fields.as_slice()
    else {
        return Err("it does not have thirteen fields".to_owned());
    };
    if *magic != MAGIC {
        return Err(format!("its first field is {magic}, not {MAGIC}"));
    }
    let number = |text: &str, what: &str| {
        text.parse::<u64>()
            .map_err(|_| format!("its {what} is no number"))
    };
    Ok(Block {
        entry: Id::parse(entry).ok_or("its entry is no id")?,
        about: About {
            generation: Generation(
                generation
                    .parse()
                    .map_err(|_| "its generation is no number")?,
            ),
            records: number(records, "records")?,
            notes: number(notes, "notes digest")?,
        },
        picks: tags(picks)?,
        takes_back: flag(takes_back)?,
        names: tags(held)?.into_iter().collect(),
        instruction: (*instruction).to_owned(),
        provider: (*provider).to_owned(),
        model: (*model).to_owned(),
        asking: (asking.len() == 16 && asking.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| u64::from_str_radix(asking, 16).ok())
            .flatten()
            .ok_or("its asking is no 16 hex digits")?,
        passed: flag(passed)?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::Tag;

    use super::{Block, read, write};
    use crate::spool;
    use crate::tags::asked::About;

    const ENTRY: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";

    fn tags(list: &[&str]) -> Vec<Tag> {
        list.iter()
            .map(|name| Tag::new(name).expect("a tag"))
            .collect()
    }

    fn answer() -> Block {
        Block {
            entry: Id::parse(ENTRY).expect("id"),
            about: About {
                generation: Generation(2),
                records: 14,
                notes: 99,
            },
            picks: tags(&["garden", "bilbo"]),
            takes_back: true,
            names: tags(&["bilbo", "garden", "rust"]).into_iter().collect(),
            instruction: "9ea9aba2ae5ed554".to_owned(),
            provider: "anthropic".to_owned(),
            model: "model-a".to_owned(),
            asking: 255,
            passed: false,
        }
    }

    fn pass() -> Block {
        Block {
            picks: Vec::new(),
            takes_back: false,
            names: BTreeSet::new(),
            passed: true,
            ..answer()
        }
    }

    #[test]
    fn a_tags_block_reads_back_and_a_notes_block_is_not_one() {
        for block in [answer(), pass()] {
            assert_eq!(read(&write(&block)), Ok(block.clone()));
            assert!(spool::read(&write(&block)).is_err());
        }
        let notes = spool::write(&spool::Block {
            parent: Id::parse(ENTRY).expect("id"),
            generation: Generation(1),
            generation_before: None,
            records_before: 0,
            opens: true,
            records: vec![b"x".to_vec()],
        });
        assert!(read(&notes).is_err());
    }

    #[test]
    fn a_block_holds_what_this_plan_lists_and_nothing_else() {
        assert_eq!(
            String::from_utf8(write(&answer())).expect("text"),
            format!(
                "g1\t{ENTRY}\t2\t14\t99\t00000000000000ff\t0\t1\t9ea9aba2ae5ed554\tanthropic\tmodel-a\tbilbo,garden,rust\tgarden,bilbo\n"
            )
        );
        assert_eq!(
            String::from_utf8(write(&pass())).expect("text"),
            format!(
                "g1\t{ENTRY}\t2\t14\t99\t00000000000000ff\t1\t0\t9ea9aba2ae5ed554\tanthropic\tmodel-a\t-\t-\n"
            )
        );
    }
}
