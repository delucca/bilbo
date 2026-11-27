use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use super::asked::About;
use super::content_of;
use crate::instruction::number_of;
use crate::log;

const MAGIC: &str = "m1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Answer,
    Pass,
    Retirement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub entry: Id,
    pub about: About,
    pub asking: u64,
    pub form: Form,
    pub instruction: String,
    pub provider: String,
    pub model: String,
    pub answered: Option<String>,
    pub made: u64,
    pub sent: u64,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub text: String,
}

#[must_use]
pub fn write(block: &Block) -> Vec<u8> {
    let answer = block.form == Form::Answer;
    let retirement = block.form == Form::Retirement;
    format!(
        "{MAGIC}\t{}\t{}\t{}\t{:032x}\t{}\t{:016x}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        block.entry,
        block.about.generation.0,
        block.about.records,
        block.about.content,
        log::number(block.about.notes.filter(|_| !retirement)),
        if retirement { 0 } else { block.asking },
        match block.form {
            Form::Answer => 0,
            Form::Pass => 1,
            Form::Retirement => 2,
        },
        block.instruction,
        block.provider,
        block.model,
        block.answered.as_deref().filter(|_| answer).unwrap_or(""),
        block.made,
        if retirement { 0 } else { block.sent },
        log::number(block.tokens_in.filter(|_| answer)),
        log::number(block.tokens_out.filter(|_| answer)),
        if answer { block.text.as_str() } else { "" }
    )
    .into_bytes()
}

fn number(text: &str, what: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|_| format!("its {what} is no number"))
}

fn tokens(text: &str, what: &str) -> Result<Option<u64>, String> {
    if text == "-" {
        Ok(None)
    } else {
        number(text, what).map(Some)
    }
}

pub fn read(bytes: &[u8]) -> Result<Block, String> {
    let body = bytes
        .strip_suffix(b"\n")
        .ok_or("it does not end in a line feed")?;
    let text = std::str::from_utf8(body).map_err(|_| "it is not UTF-8")?;
    if text.contains('\n') {
        return Err("it is more than one line".to_owned());
    }
    let fields: Vec<&str> = text.split('\t').collect();
    let [
        magic,
        entry,
        generation,
        records,
        content,
        notes,
        asking,
        form,
        instruction,
        provider,
        model,
        answered,
        made,
        sent,
        tokens_in,
        tokens_out,
        words,
    ] = fields.as_slice()
    else {
        return Err("it does not have seventeen fields".to_owned());
    };
    if *magic != MAGIC {
        return Err(format!("its first field is {magic}, not {MAGIC}"));
    }
    let form = match *form {
        "0" => Form::Answer,
        "1" => Form::Pass,
        "2" => Form::Retirement,
        other => return Err(format!("{other} is none of 0, 1 and 2")),
    };
    let block = Block {
        entry: Id::parse(entry).ok_or("its entry is no id")?,
        about: About {
            generation: Generation(
                generation
                    .parse()
                    .map_err(|_| "its generation is no number")?,
            ),
            records: number(records, "records")?,
            content: content_of(content).ok_or("its content is no 32 hex digits")?,
            notes: tokens(notes, "notes digest")?,
        },
        asking: number_of(asking).ok_or("its asking is no 16 hex digits")?,
        form,
        instruction: (*instruction).to_owned(),
        provider: (*provider).to_owned(),
        model: (*model).to_owned(),
        answered: (!answered.is_empty()).then(|| (*answered).to_owned()),
        made: number(made, "made")?,
        sent: number(sent, "sent")?,
        tokens_in: tokens(tokens_in, "tokens read")?,
        tokens_out: tokens(tokens_out, "tokens written")?,
        text: (*words).to_owned(),
    };
    holds_only_its_form(&block)?;
    Ok(block)
}

fn holds_only_its_form(block: &Block) -> Result<(), String> {
    let refuse = |sentence: &str| Err(sentence.to_owned());
    let tokens = block.tokens_in.is_some() || block.tokens_out.is_some();
    match block.form {
        Form::Answer if block.text.is_empty() => refuse("an answer holds no text"),
        Form::Answer => Ok(()),
        Form::Pass if block.answered.is_some() => refuse("a pass holds a model that answered"),
        Form::Pass if tokens => refuse("a pass holds tokens"),
        Form::Pass if !block.text.is_empty() => refuse("a pass holds a text"),
        Form::Pass => Ok(()),
        Form::Retirement if block.about.notes.is_some() => {
            refuse("a retirement holds a notes' digest")
        }
        Form::Retirement if block.asking != 0 => refuse("a retirement holds an asking"),
        Form::Retirement if block.answered.is_some() => {
            refuse("a retirement holds a model that answered")
        }
        Form::Retirement if block.sent != 0 => refuse("a retirement holds tokens sent"),
        Form::Retirement if tokens => refuse("a retirement holds tokens"),
        Form::Retirement if !block.text.is_empty() => refuse("a retirement holds a text"),
        Form::Retirement => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{About, Block, Form, read, write};
    use crate::spool;

    const ENTRY: &str = "01M6ZGE4KTBYYH07AS2KVMSCAF";
    const CONTENT: &str = "0000000000000000000000000000abcd";

    fn id() -> Id {
        Id::parse(ENTRY).expect("id")
    }

    fn answer() -> Block {
        Block {
            entry: id(),
            about: About {
                generation: Generation(2),
                records: 14,
                content: 0xabcd,
                notes: Some(99),
            },
            asking: 255,
            form: Form::Answer,
            instruction: "7fdf9f0d93f443e3".to_owned(),
            provider: "anthropic".to_owned(),
            model: "model-a".to_owned(),
            answered: Some("model-b".to_owned()),
            made: 1_794_107_564,
            sent: 412,
            tokens_in: Some(790),
            tokens_out: Some(61),
            text: "Thinning the carrots of the north bed.".to_owned(),
        }
    }

    fn of_text() -> Block {
        Block {
            about: About {
                notes: None,
                ..answer().about
            },
            answered: None,
            tokens_in: None,
            tokens_out: None,
            text: "The garden log.".to_owned(),
            ..answer()
        }
    }

    fn pass() -> Block {
        Block {
            form: Form::Pass,
            answered: None,
            tokens_in: None,
            tokens_out: None,
            text: String::new(),
            ..answer()
        }
    }

    fn retirement() -> Block {
        Block {
            about: About {
                generation: Generation(3),
                records: 0,
                content: 0xabcd,
                notes: None,
            },
            asking: 0,
            form: Form::Retirement,
            sent: 0,
            ..pass()
        }
    }

    fn text(block: &Block) -> String {
        String::from_utf8(write(block)).expect("text")
    }

    #[test]
    fn a_block_holds_what_this_plan_lists_and_nothing_else() {
        assert_eq!(
            text(&answer()),
            format!(
                "m1\t{ENTRY}\t2\t14\t{CONTENT}\t99\t00000000000000ff\t0\t7fdf9f0d93f443e3\tanthropic\tmodel-a\tmodel-b\t1794107564\t412\t790\t61\tThinning the carrots of the north bed.\n"
            )
        );
        assert_eq!(
            text(&of_text()),
            format!(
                "m1\t{ENTRY}\t2\t14\t{CONTENT}\t-\t00000000000000ff\t0\t7fdf9f0d93f443e3\tanthropic\tmodel-a\t\t1794107564\t412\t-\t-\tThe garden log.\n"
            )
        );
        assert_eq!(
            text(&pass()),
            format!(
                "m1\t{ENTRY}\t2\t14\t{CONTENT}\t99\t00000000000000ff\t1\t7fdf9f0d93f443e3\tanthropic\tmodel-a\t\t1794107564\t412\t-\t-\t\n"
            )
        );
        assert_eq!(
            text(&retirement()),
            format!(
                "m1\t{ENTRY}\t3\t0\t{CONTENT}\t-\t0000000000000000\t2\t7fdf9f0d93f443e3\tanthropic\tmodel-a\t\t1794107564\t0\t-\t-\t\n"
            )
        );
        assert_eq!(
            text(&Block {
                form: Form::Pass,
                ..answer()
            }),
            text(&pass())
        );
        assert_eq!(
            text(&Block {
                form: Form::Retirement,
                ..answer()
            }),
            format!(
                "m1\t{ENTRY}\t2\t14\t{CONTENT}\t-\t0000000000000000\t2\t7fdf9f0d93f443e3\tanthropic\tmodel-a\t\t1794107564\t0\t-\t-\t\n"
            )
        );
    }

    #[test]
    fn a_block_holds_only_what_its_form_holds() {
        let changed = |block: &Block, field: usize, to: &str| {
            let valid = text(block);
            let mut fields: Vec<&str> = valid.trim_end_matches('\n').split('\t').collect();
            fields[field - 1] = to;
            format!("{}\n", fields.join("\t")).into_bytes()
        };
        for (block, field, to, message) in [
            (answer(), 8, "1", "a pass holds a model that answered"),
            (answer(), 8, "2", "a retirement holds a notes' digest"),
            (of_text(), 8, "1", "a pass holds a text"),
            (of_text(), 8, "2", "a retirement holds an asking"),
            (pass(), 8, "0", "an answer holds no text"),
            (pass(), 8, "2", "a retirement holds a notes' digest"),
            (retirement(), 8, "0", "an answer holds no text"),
            (answer(), 17, "", "an answer holds no text"),
            (pass(), 12, "model-b", "a pass holds a model that answered"),
            (pass(), 15, "790", "a pass holds tokens"),
            (pass(), 16, "61", "a pass holds tokens"),
            (pass(), 17, "x", "a pass holds a text"),
            (retirement(), 6, "99", "a retirement holds a notes' digest"),
            (
                retirement(),
                7,
                "00000000000000ff",
                "a retirement holds an asking",
            ),
            (
                retirement(),
                12,
                "model-b",
                "a retirement holds a model that answered",
            ),
            (retirement(), 14, "412", "a retirement holds tokens sent"),
            (retirement(), 15, "790", "a retirement holds tokens"),
            (retirement(), 17, "x", "a retirement holds a text"),
        ] {
            assert_eq!(
                read(&changed(&block, field, to)),
                Err(message.to_owned()),
                "{message}"
            );
        }
        assert_eq!(
            read(&changed(&retirement(), 8, "1")),
            Ok(Block {
                form: Form::Pass,
                ..retirement()
            })
        );
    }

    #[test]
    fn a_summary_block_reads_back_and_is_neither_of_the_other_two() {
        for block in [answer(), of_text(), pass(), retirement()] {
            assert_eq!(read(&write(&block)), Ok(block.clone()));
            assert!(spool::read(&write(&block)).is_err());
            assert!(crate::tags::block::read(&write(&block)).is_err());
        }
        let dashed = Block {
            answered: Some("-".to_owned()),
            text: "-".to_owned(),
            ..answer()
        };
        let back = read(&write(&dashed)).expect("a block");
        assert_eq!(back.answered.as_deref(), Some("-"));
        assert_eq!(back.text, "-");
        assert_eq!(read(&write(&of_text())).expect("a block").answered, None);
        let notes = spool::write(&spool::Block {
            parent: id(),
            generation: Generation(1),
            generation_before: None,
            records_before: 0,
            opens: true,
            records: vec![b"x".to_vec()],
        });
        assert!(read(&notes).is_err());
        let tags = crate::tags::block::write(&crate::tags::block::Block {
            entry: id(),
            about: crate::tags::asked::About {
                generation: Generation(2),
                records: 14,
                notes: 99,
            },
            picks: Vec::new(),
            takes_back: false,
            names: std::collections::BTreeSet::new(),
            instruction: "9ea9aba2ae5ed554".to_owned(),
            provider: "anthropic".to_owned(),
            model: "model-a".to_owned(),
            asking: 255,
            passed: false,
        });
        assert!(read(&tags).is_err());
        let first = |bytes: &[u8]| {
            let text = std::str::from_utf8(bytes).expect("text");
            text.split('\t').next().expect("a field").to_owned()
        };
        let words = [first(&write(&answer())), first(&notes), first(&tags)];
        assert_eq!(words, ["m1", "s1", "g1"]);
    }

    #[test]
    fn a_block_that_is_not_one_says_why() {
        let valid = text(&answer());
        let fields: Vec<&str> = valid.trim_end_matches('\n').split('\t').collect();
        let with = |at: usize, to: &str| {
            let mut changed = fields.clone();
            changed[at] = to;
            format!("{}\n", changed.join("\t")).into_bytes()
        };
        let shorter = format!("{}\n", fields[..16].join("\t")).into_bytes();
        let longer = format!("{}\tx\n", fields.join("\t")).into_bytes();
        for (bytes, message) in [
            (
                valid.trim_end_matches('\n').as_bytes().to_vec(),
                "it does not end in a line feed",
            ),
            (b"\xff\n".to_vec(), "it is not UTF-8"),
            (shorter, "it does not have seventeen fields"),
            (longer, "it does not have seventeen fields"),
            (with(0, "g1"), "its first field is g1, not m1"),
            (with(1, "x"), "its entry is no id"),
            (with(2, "x"), "its generation is no number"),
            (with(4, "abcd"), "its content is no 32 hex digits"),
            (with(6, "ff"), "its asking is no 16 hex digits"),
            (with(7, "3"), "3 is none of 0, 1 and 2"),
            (with(16, "One.\n\nTwo."), "it is more than one line"),
            (with(8, "i\nj"), "it is more than one line"),
        ] {
            assert_eq!(read(&bytes), Err(message.to_owned()), "{message}");
        }
    }
}
