use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use bilbo_entries::kind::Kind;
use bilbo_entries::remember::{self, Tagging, Tally, Written as Request, WrittenAnswer};
use bilbo_entries::tag;
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tag};
use bilbo_store::segments::Store;

use super::follows;
use super::refused;
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

pub struct Draft<'a> {
    pub path: &'a Path,
    pub remove: &'a mut dyn FnMut() -> Option<Message>,
}

fn ending(draft: &Option<&mut Draft<'_>>) -> String {
    draft.as_ref().map_or_else(String::new, |draft| {
        format!("; what was saved stays in {}", draft.path.display())
    })
}

fn with_ending(failure: Failure, ending: &str) -> Failure {
    match failure {
        Failure::Config(line) => Failure::Config(sanitize(&format!("{line}{ending}"))),
        Failure::Refused(line) => refused(&format!("{line}{ending}")),
        other => other,
    }
}

pub fn from_input(input: &mut dyn Read) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .map_err(|error| refused(&format!("cannot read standard input: {error}")))?;
    String::from_utf8(bytes)
        .map_err(|_| Failure::Refused("refused the note: it is not UTF-8".to_owned()))
}

pub fn id_for(
    text: &str,
    written: &Written,
    brought: Option<Id>,
    mint: impl FnOnce() -> io::Result<Id>,
) -> Result<Option<Id>, Failure> {
    if (written.record)(text, SystemTime::now()).is_none() {
        return Ok(None);
    }
    match brought {
        Some(id) => Ok(Some(id)),
        None => mint().map(Some).map_err(|error| refused(&error)),
    }
}

pub struct Note<'a, 'b> {
    pub text: &'a str,
    pub id: Option<Id>,
    pub tags: &'a Tagging,
    pub draft: Option<&'a mut Draft<'b>>,
}

pub fn keep(
    note: Note<'_, '_>,
    written: &Written,
    env: &mut Env,
    settings: &Settings,
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
) -> Result<Outcome, Failure> {
    let Note {
        text,
        id,
        tags,
        draft,
    } = note;
    let Some(id) = id_for(text, written, id, Id::mint)? else {
        return Ok(nothing_to_keep(draft));
    };
    let after = ending(&draft);
    let label = (written.label)(id);
    let (mut store, fresh) =
        open_store(tags, settings, env).map_err(|failure| with_ending(failure, &after))?;
    let Some(record) = (written.record)(text, SystemTime::now()) else {
        store.release();
        return Ok(nothing_to_keep(draft));
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
        WrittenAnswer::Kept { .. } | WrittenAnswer::Refused(_) => follows::finish(
            store,
            &settings.follow.value,
            kinds,
            indexing,
            &mut *env.notice,
            u64::from(matches!(answer, WrittenAnswer::Kept { .. })),
        ),
        _ => {
            store.release();
            Closing::default()
        }
    };
    answered(answer, &fresh, closing, draft)
}

fn open_store(
    tags: &Tagging,
    settings: &Settings,
    env: &mut Env,
) -> Result<(Store, Vec<Tag>), Failure> {
    let layout = settings.layout();
    Store::check(&layout)?;
    let notice = &mut env.notice;
    let store = Store::open(&layout, settings.store.options(), &mut |n| {
        notice(&notice_message(&n));
    })?;
    if tags.tags.is_empty() {
        return Ok((store, Vec::new()));
    }
    match tag::unused(&tags.tags, &store) {
        Ok(fresh) => Ok((store, fresh)),
        Err(error) => {
            store.release();
            Err(refused(&error))
        }
    }
}

pub fn nothing_to_keep(draft: Option<&mut Draft<'_>>) -> Outcome {
    let mut diagnostics = vec![Message::warning("nothing to keep: the note is empty")];
    if let Some(draft) = draft {
        diagnostics.extend((draft.remove)());
    }
    outcome(&Totals::default(), diagnostics, Exit::Refused)
}

fn store_failed(error: &dyn std::fmt::Display, tried: Option<Id>, ending: &str) -> Failure {
    let line = match tried {
        Some(id) => format!("{error}; the note may have been kept as {id}{ending}"),
        None => format!("{error}{ending}"),
    };
    refused(&line)
}

pub fn answered(
    answer: WrittenAnswer,
    fresh: &[Tag],
    closing: Closing,
    draft: Option<&mut Draft<'_>>,
) -> Result<Outcome, Failure> {
    let after = ending(&draft);
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
            if let Some(draft) = draft {
                diagnostics.extend((draft.remove)());
            }
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
            let mut diagnostics = vec![Message::warning(&format!(
                "refused the note: {reason}{after}"
            ))];
            diagnostics.extend(closing.messages);
            Ok(outcome(&totals, diagnostics, Exit::Refused))
        }
        WrittenAnswer::BeforeCapture(error) => Err(store_failed(&error, None, &after)),
        WrittenAnswer::AtCapture { id, error } => Err(store_failed(&error, Some(id), &after)),
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
    use bilbo_entries::remember::{Attempt, WrittenAnswer};
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
        let outcome = answered(kept, &fresh, Closing::default(), None).expect("an outcome");
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
            None,
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
            None,
        )
        .expect_err("a failure");
        assert_eq!(before.stderr_lines(&View::PLAIN), ["bilbo: no store"]);
        assert_eq!(before.exit_code(), 1);

        let at = answered(
            WrittenAnswer::AtCapture {
                id: id(),
                error: Attempt::Store(StoreError::Missing("no store".to_owned())),
            },
            &[],
            Closing::default(),
            None,
        )
        .expect_err("a failure");
        assert_eq!(
            at.stderr_lines(&View::PLAIN),
            [format!(
                "bilbo: no store; the note may have been kept as {ID}"
            )]
        );

        let none = answered(
            WrittenAnswer::AtCapture {
                id: id(),
                error: Attempt::NoNewEntry,
            },
            &[],
            Closing::default(),
            None,
        )
        .expect_err("a failure");
        assert_eq!(
            none.stderr_lines(&View::PLAIN),
            [format!(
                "bilbo: the store did not make a new entry for the note; the note may have been kept as {ID}"
            )]
        );
        assert_eq!(none.exit_code(), 1);
    }

    #[test]
    fn a_note_with_no_text_asks_for_no_id() {
        use std::cell::Cell;
        use std::io;

        use super::{Written, id_for};
        use crate::console::outcome::Failure;

        let notes = crate::notes();
        let kind = Written {
            kind: notes.kind,
            record: notes.record,
            label: notes.label,
        };
        let asked = Cell::new(0);
        let failing = |text: &'static str| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                Err::<Id, _>(io::Error::other(text))
            }
        };
        for text in ["", "  \n\t "] {
            asked.set(0);
            let found = id_for(text, &kind, None, failing("no random source"));
            assert!(matches!(found, Ok(None)), "{text:?}");
            assert_eq!(asked.get(), 0, "{text:?}");
        }

        asked.set(0);
        let found = id_for("A sentence.", &kind, None, failing("no random source"));
        assert_eq!(
            found.expect_err("the minting's failure"),
            Failure::Refused("no random source".to_owned())
        );
        assert_eq!(asked.get(), 1);

        asked.set(0);
        let minted = id();
        let found = id_for("A sentence.", &kind, None, || {
            asked.set(asked.get() + 1);
            Ok(minted)
        });
        assert!(matches!(found, Ok(Some(found)) if found == minted));
        assert_eq!(asked.get(), 1);

        asked.set(0);
        let found = id_for("A sentence.", &kind, Some(minted), failing("not asked"));
        assert!(matches!(found, Ok(Some(found)) if found == minted));
        assert_eq!(asked.get(), 0);
    }
}
