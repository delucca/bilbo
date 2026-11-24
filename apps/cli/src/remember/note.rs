use std::io::Read;
use std::time::SystemTime;

use bilbo_entries::kind::Kind;
use bilbo_entries::remember::{self, Tagging, Tally, Written as Request, WrittenAnswer};
use bilbo_entries::tag;
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tag};
use bilbo_store::segments::Store;

use super::follows;
use super::view::{Totals, human_totals, plain_totals};
use crate::console::closing::Closing;
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::message::{Message, new_tags, notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;

pub struct Written {
    pub kind: &'static str,
    pub record: fn(&str, SystemTime) -> Option<Vec<u8>>,
    pub label: fn(Id) -> CaptureLabel,
}

pub fn from_input(input: &mut dyn Read) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).map_err(|error| {
        Failure::Refused(sanitize(&format!("cannot read standard input: {error}")))
    })?;
    String::from_utf8(bytes)
        .map_err(|_| Failure::Refused("refused the note: it is not UTF-8".to_owned()))
}

pub fn keep(
    text: &str,
    tags: &Tagging,
    written: &Written,
    env: &mut Env,
    settings: &Settings,
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
) -> Result<Outcome, Failure> {
    if (written.record)(text, SystemTime::now()).is_none() {
        return Ok(nothing_to_keep());
    }
    let id = Id::mint().map_err(|error| Failure::Refused(sanitize(&error.to_string())))?;
    let label = (written.label)(id);
    let layout = settings.layout();
    Store::check(&layout)?;
    let notice = &mut env.notice;
    let mut store = Store::open(&layout, settings.store.options(), &mut |n| {
        notice(&notice_message(&n));
    })?;
    let fresh = if tags.tags.is_empty() {
        Vec::new()
    } else {
        match tag::unused(&tags.tags, &store) {
            Ok(fresh) => fresh,
            Err(error) => {
                store.release();
                return Err(store_failed(&error, None));
            }
        }
    };
    let Some(record) = (written.record)(text, SystemTime::now()) else {
        store.release();
        return Ok(nothing_to_keep());
    };
    let answer = remember::keep_written(
        &mut store,
        &Request {
            kind: written.kind,
            label: &label,
            id,
            record: &record,
            tags,
        },
    );
    let closing = match &answer {
        WrittenAnswer::Kept { .. } => follows::finish(
            store,
            &settings.follow.value,
            kinds,
            indexing,
            &mut *env.notice,
            1,
        ),
        WrittenAnswer::Refused(_) => follows::finish(
            store,
            &settings.follow.value,
            kinds,
            indexing,
            &mut *env.notice,
            0,
        ),
        _ => {
            store.release();
            Closing::default()
        }
    };
    answered(answer, &fresh, closing)
}

fn nothing_to_keep() -> Outcome {
    outcome(
        &Totals::default(),
        vec![Message::warning("nothing to keep: the note is empty")],
        Exit::Refused,
    )
}

fn store_failed(error: &dyn std::fmt::Display, tried: Option<Id>) -> Failure {
    let line = match tried {
        Some(id) => format!("{error}; the note may have been kept as {id}"),
        None => error.to_string(),
    };
    Failure::Refused(sanitize(&line))
}

fn answered(answer: WrittenAnswer, fresh: &[Tag], closing: Closing) -> Result<Outcome, Failure> {
    match answer {
        WrittenAnswer::Kept {
            id,
            records,
            raw_bytes,
            stored_bytes,
            tagged,
        } => {
            let totals = Totals {
                tally: Tally {
                    new: 1,
                    ..Tally::default()
                },
                claimed: 1,
                records,
                raw_bytes,
                stored_bytes,
            };
            let mut diagnostics = vec![Message::info(&format!("kept the note as {id}"))];
            if tagged {
                diagnostics.push(Message::info("tagged 1 entries"));
                diagnostics.extend(new_tags(fresh));
            }
            diagnostics.extend(closing.messages);
            let exit = if closing.failed || closing.refused {
                Exit::Refused
            } else {
                Exit::Success
            };
            Ok(outcome(&totals, diagnostics, exit))
        }
        WrittenAnswer::Refused(reason) => {
            let totals = Totals {
                tally: Tally {
                    refused: 1,
                    ..Tally::default()
                },
                claimed: 1,
                ..Totals::default()
            };
            let mut diagnostics = vec![Message::warning(&format!("refused the note: {reason}"))];
            diagnostics.extend(closing.messages);
            Ok(outcome(&totals, diagnostics, Exit::Refused))
        }
        WrittenAnswer::BeforeCapture(error) => Err(store_failed(&error, None)),
        WrittenAnswer::AtCapture { id, error } => Err(store_failed(&error, Some(id))),
    }
}

fn outcome(totals: &Totals, diagnostics: Vec<Message>, exit: Exit) -> Outcome {
    let human = totals.clone();
    Outcome::new(plain_totals(totals))
        .with_human(move |view| human_totals(&human, view))
        .with_diagnostics(diagnostics)
        .with_exit(exit)
}

#[cfg(test)]
mod tests {
    use bilbo_entries::remember::WrittenAnswer;
    use bilbo_store::entries::StoreError;
    use bilbo_store::id::Id;
    use bilbo_store::labels::Tag;

    use super::answered;
    use crate::console::closing::Closing;
    use crate::console::view::View;

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

    fn id() -> Id {
        Id::parse(ID).expect("an id")
    }

    #[test]
    fn a_kept_note_says_its_id_and_counts_one_new_entry() {
        let kept = WrittenAnswer::Kept {
            id: id(),
            records: 1,
            raw_bytes: 40,
            stored_bytes: 190,
            tagged: true,
        };
        let fresh = [Tag::new("garden").expect("a tag")];
        let outcome = answered(kept, &fresh, Closing::default()).expect("an outcome");
        assert_eq!(
            outcome.stdout(&View::PLAIN),
            "new 1, appended 0, new generation 0, unchanged 0, refused 0\nrecords 1, raw bytes 40, stored bytes 190\n"
        );
        assert_eq!(
            outcome.stderr_lines(&View::PLAIN),
            [
                format!("bilbo: kept the note as {ID}"),
                "bilbo: tagged 1 entries".to_owned(),
                "bilbo: tag:garden is new".to_owned(),
            ]
        );
        assert_eq!(outcome.exit_code(), 0);

        let refused = answered(
            WrittenAnswer::Refused("its id x is taken by another entry".to_owned()),
            &[],
            Closing::default(),
        )
        .expect("an outcome");
        assert_eq!(
            refused.stdout(&View::PLAIN),
            "new 0, appended 0, new generation 0, unchanged 0, refused 1\nrecords 0, raw bytes 0, stored bytes 0\n"
        );
        assert_eq!(
            refused.stderr_lines(&View::PLAIN),
            ["bilbo: refused the note: its id x is taken by another entry"]
        );
        assert_eq!(refused.exit_code(), 1);

        let before = answered(
            WrittenAnswer::BeforeCapture(StoreError::Missing("no store".to_owned())),
            &[],
            Closing::default(),
        )
        .expect_err("a failure");
        assert_eq!(before.stderr_lines(&View::PLAIN), ["bilbo: no store"]);
        assert_eq!(before.exit_code(), 1);

        let at = answered(
            WrittenAnswer::AtCapture {
                id: id(),
                error: StoreError::Missing("no store".to_owned()),
            },
            &[],
            Closing::default(),
        )
        .expect_err("a failure");
        assert_eq!(
            at.stderr_lines(&View::PLAIN),
            [format!(
                "bilbo: no store; the note may have been kept as {ID}"
            )]
        );
    }
}
