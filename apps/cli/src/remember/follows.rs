use bilbo_kinds::follows::{self, Follow, Planned};
use bilbo_kinds::kind::Kind;
use bilbo_kinds::remember::{self, Output};
use bilbo_search::index::Index;
use bilbo_store::entries::{Entries, StoreError};
use bilbo_store::segments::{Closed, Held, Options, Store};

use super::disk::Disk;
use super::index::update;
use super::messages::{info, refusals, warning};
use super::view;
use crate::console::closing::Closing;
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::message::{Message, notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;

/// Said instead of the store's Busy text, which says less.
const BUSY: &str = "another bilbo is writing the store; requested another pass over the follows";

#[derive(Default)]
pub struct Progress {
    /// Entries this invocation kept: a build of no more says nothing.
    pub kept: u64,
    /// An update failed for the whole index: none is tried again.
    pub fatal: bool,
}

pub struct Pass<'k> {
    planned: Vec<Planned<'k>>,
    pub messages: Vec<Message>,
    /// A follow could not run: the run exits 1.
    pub failed: bool,
}

impl<'k> Pass<'k> {
    pub fn plan(follows: &[Follow], kinds: &'k [Box<dyn Kind>]) -> Pass<'k> {
        let planned = follows::plan(follows, kinds, &Disk);
        Pass {
            messages: planned_messages(&planned),
            failed: planned
                .iter()
                .any(|planned| matches!(planned, Planned::Failed(..))),
            planned,
        }
    }

    pub fn keep(&self, entries: &mut dyn Entries) -> Result<Vec<Output>, remember::Error> {
        follows::keep(&self.planned, &Disk, entries)
    }
}

/// Every follow into one store. The store is opened even when nothing is claimed, so the timer's
/// pass seals and rebuilds; a bilbo that holds it is asked to rerun instead.
pub fn run(
    env: &mut Env,
    settings: &Settings,
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
) -> Result<Outcome, Failure> {
    let pass = Pass::plan(&settings.follow.value, kinds);
    let mut diagnostics = pass.messages.clone();
    let layout = settings.layout();
    if let Err(error) = Store::check(&layout) {
        return Ok(stopped(diagnostics, &Failure::from(&error)));
    }
    let options = Options {
        held: Held::Flag { readers: None },
        ..settings.store.options()
    };
    let notice = &mut env.notice;
    let mut store = match Store::open(&layout, options, &mut |n| notice(&notice_message(&n))) {
        Ok(store) => store,
        Err(StoreError::Busy) => {
            diagnostics.push(info(BUSY));
            return Ok(Outcome::new("").with_diagnostics(diagnostics));
        }
        Err(error) => return Ok(stopped(diagnostics, &Failure::from(&error))),
    };
    let outputs = match pass.keep(&mut store) {
        Ok(outputs) => outputs,
        Err(error) => {
            return Ok(stopped(
                diagnostics,
                &Failure::Refused(sanitize(&error.to_string())),
            ));
        }
    };
    let kept = view::Totals::of(&outputs).kept();
    let closing = finish(
        store,
        &settings.follow.value,
        kinds,
        indexing,
        &mut *env.notice,
        kept,
    );
    diagnostics.extend(refusals(&outputs));
    diagnostics.extend(closing.messages);
    Ok(outcome(
        &outputs,
        diagnostics,
        pass.failed || closing.failed || closing.refused,
    ))
}

fn stopped(mut diagnostics: Vec<Message>, failure: &Failure) -> Outcome {
    diagnostics.push(failure.message());
    let exit = if failure.exit_code() == 2 {
        Exit::Config
    } else {
        Exit::Refused
    };
    Outcome::new("")
        .with_diagnostics(diagnostics)
        .with_exit(exit)
}

pub fn finish(
    store: Store,
    follows: &[Follow],
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
    notice: &mut dyn FnMut(&Message),
    kept: u64,
) -> Closing {
    let mut index = (indexing.index)(store.cache());
    let updated = update(
        &store,
        index.as_mut(),
        indexing.tries,
        kept,
        indexing,
        notice,
    );
    let progress = Progress {
        kept,
        fatal: updated.fatal,
    };
    let mut closing = close(
        store,
        index.as_mut(),
        follows,
        kinds,
        indexing,
        notice,
        progress,
    );
    let mut messages = updated.messages;
    messages.append(&mut closing.messages);
    closing.messages = messages;
    closing.refused |= updated.refused;
    closing
}

/// A pass never retries an entry the invocation tried already, and after an update that failed for the whole index it tries none.
pub fn close(
    mut store: Store,
    index: &mut dyn Index,
    follows: &[Follow],
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
    notice: &mut dyn FnMut(&Message),
    mut progress: Progress,
) -> Closing {
    let mut closing = Closing::default();
    loop {
        match store.close() {
            Ok(Closed::Released) => return closing,
            Ok(Closed::Again(again)) => {
                store = *again;
                let mut pass = Pass::plan(follows, kinds);
                closing.messages.append(&mut pass.messages);
                closing.refused |= pass.failed;
                match pass.keep(&mut store) {
                    Ok(outputs) => {
                        let totals = view::Totals::of(&outputs);
                        closing.refused |= totals.tally.refused > 0;
                        closing.messages.extend(refusals(&outputs));
                        closing.messages.push(info(&format!(
                            "ran the follows again for another bilbo: {}",
                            view::counts(&totals)
                        )));
                        progress.kept += totals.kept();
                        if !progress.fatal {
                            let mut updated =
                                update(&store, index, 0, progress.kept, indexing, notice);
                            closing.messages.append(&mut updated.messages);
                            closing.refused |= updated.refused;
                            progress.fatal = updated.fatal;
                        }
                    }
                    Err(error) => {
                        closing
                            .messages
                            .push(Message::error(sanitize(&error.to_string())));
                        closing.failed = true;
                    }
                }
            }
            Err(error) => {
                closing
                    .messages
                    .push(Message::error(sanitize(&error.to_string())));
                closing.failed = true;
                return closing;
            }
        }
    }
}

/// Refusals are not here: each follow's output carries them, so they count in the totals.
fn planned_messages(planned: &[Planned<'_>]) -> Vec<Message> {
    let mut messages = Vec::new();
    for planned in planned {
        match planned {
            Planned::Missing(follow) => {
                messages.push(info(&format!(
                    "skipped {}: no such folder",
                    follow.path.display()
                )));
            }
            Planned::Failed(follow, error) => {
                messages.push(warning(&format!(
                    "follow {}: {error}",
                    follow.path.display()
                )));
            }
            Planned::Ready(follow, plan) => {
                if plan.files.is_empty() {
                    messages.push(info(&match plan.kind {
                        Some(kind) => format!(
                            "nothing to keep: {} claims no file under {}",
                            kind.name(),
                            follow.path.display()
                        ),
                        None => format!(
                            "nothing to keep: no kind claims a file under {}",
                            follow.path.display()
                        ),
                    }));
                }
            }
        }
    }
    messages
}

fn outcome(
    outputs: &[bilbo_kinds::remember::Output],
    diagnostics: Vec<Message>,
    failed: bool,
) -> Outcome {
    let totals = view::Totals::of(outputs);
    let refused = totals.tally.refused > 0;
    let human = totals.clone();
    Outcome::new(view::plain_totals(&totals))
        .with_human(move |out| view::human_totals(&human, out))
        .with_diagnostics(diagnostics)
        .with_exit(if failed || refused {
            Exit::Refused
        } else {
            Exit::Success
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_kinds::files::{Memory, Node};
    use bilbo_kinds::follows::{Follow, plan};

    use super::planned_messages;
    use crate::console::theme::Mark;

    #[test]
    fn a_follow_that_does_not_run_says_why_in_one_line() {
        let files = Memory::new()
            .file("/notes/a.md", "a\n")
            .folder("/empty")
            .unreadable("/locked", Node::Folder);
        let follow = |path: &str, kind: Option<&str>| Follow {
            path: PathBuf::from(path),
            kind: kind.map(str::to_owned),
        };
        let kinds = crate::kinds();
        let planned = plan(
            &[
                follow("/gone", Some("session")),
                follow("/notes", Some("pdf")),
                follow("/empty", Some("session")),
                follow("/locked", Some("markdown")),
                follow("/notes", None),
            ],
            &kinds,
            &files,
        );
        let lines: Vec<(Mark, String)> = planned_messages(&planned)
            .into_iter()
            .map(|message| (message.mark, message.text))
            .collect();
        assert_eq!(
            lines,
            [
                (Mark::Info, "skipped /gone: no such folder".to_owned()),
                (
                    Mark::Warning,
                    "follow /notes: unknown kind 'pdf'; kinds: markdown, session".to_owned()
                ),
                (
                    Mark::Info,
                    "nothing to keep: session claims no file under /empty".to_owned()
                ),
                (
                    Mark::Info,
                    "nothing to keep: markdown claims no file under /locked".to_owned()
                ),
            ]
        );
    }
}
