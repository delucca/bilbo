pub mod hook;
pub mod log;
pub mod seen;
pub mod view;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bilbo_search::digest::log::{End, Record, line as logged};
use bilbo_search::digest::{self, First, List, Prompt, Request, Turn, Way};
use bilbo_search::position::Manifest;
use bilbo_search::recall::Stand;
use bilbo_store::entries::StoreError;
use bilbo_store::segments::{Cache, Options, Store};
use clap::Command;

use crate::console::args::usage;
use crate::console::env::{Env, Who};
use crate::console::message::{Message, unsearched};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::reading::Reading;
use crate::console::settings::Settings;
use crate::console::view::View;

const SUMMARY: &str = "Answer a session's first prompt with what bilbo keeps on it";
const ABOUT: &str = "bilbo digest: answer a session's first prompt with what bilbo keeps on it.";
const USAGE: &str = "bilbo digest";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {}

const PAGE: &str = "\
A prompt hook runs it and hands it one JSON object on standard input, with
session_id and prompt. Every prompt after a session's first prints nothing.

Output:
  A list for each kind with an entry at its floor or over, each under its line:
  files: <n> may bear on this prompt, best first; bilbo read <ref> outlines one
  <ref> <capture> <title>      a file's row, by tabs
  <ref> <day> <folder> <line>  a row of a session or a note: its passage's
                               ref and day, its folder, and its outline's
                               line or the note's start. A fifth field,
                               note: <text>, is a note derive wrote of it
  summary: <text>              a row's last field: the summary derive wrote
                               of its entry, where there is one
  digest.log                   in the state folder: one line a run

Exit:
  0  always, so its exit code blocks no prompt; stderr says what went wrong

Examples:
  echo '{\"session_id\":\"s1\",\"prompt\":\"thin the carrot rows\"}' | bilbo digest

Docs: https://github.com/delucca/bilbo/wiki/Commands#digest";

pub struct Shelf<'a> {
    pub word: &'static str,
    pub list: List<'a>,
}

pub struct Hand<'a> {
    pub tokenizer: &'a str,
    pub shelves: &'a [Shelf<'a>],
    pub way: Way<'a>,
}

pub fn excused(failure: &Failure) -> Outcome {
    Outcome::new("")
        .with_diagnostics(vec![failure.message()])
        .with_exit(Exit::Always)
}

pub fn options(settings: &Settings) -> Options {
    Options {
        wait: settings.digest.wait.value,
        ..settings.store.options()
    }
}

struct Ended {
    outcome: Result<Outcome, Failure>,
    messages: Vec<Message>,
    end: End,
    listed: Option<digest::Listed>,
}

fn ended(outcome: Result<Outcome, Failure>, end: End) -> Ended {
    Ended {
        outcome,
        messages: Vec::new(),
        end,
        listed: None,
    }
}

fn said(message: Message, end: End) -> Ended {
    Ended {
        outcome: Ok(Outcome::new("")),
        messages: vec![message],
        end,
        listed: None,
    }
}

fn finished(
    outcome: Result<Outcome, Failure>,
    mut messages: Vec<Message>,
    unlogged: Option<Message>,
) -> Result<Outcome, Failure> {
    match (outcome, unlogged) {
        (Ok(outcome), unlogged) => {
            messages.extend(unlogged);
            Ok(if messages.is_empty() {
                outcome
            } else {
                outcome.with_diagnostics(messages)
            })
        }
        (Err(failure), None) => Err(failure),
        (Err(failure), Some(unlogged)) => {
            Ok(Outcome::new("").with_diagnostics(vec![failure.message(), unlogged]))
        }
    }
}

pub fn run(
    root: &Command,
    env: &mut Env,
    settings: &Settings,
    reading: &Reading<'_>,
    hand: &Hand<'_>,
) -> Result<Outcome, Failure> {
    let began = Instant::now();
    let now = SystemTime::now();
    if env.who == Who::Person {
        return Err(usage(
            root,
            "digest reads a prompt hook's JSON on standard input".to_owned(),
            Some("digest"),
        ));
    }
    let given = hook::read(&mut *env.stdin, settings.digest.input.value);
    let checked = Store::checked(&settings.layout(), options(settings))
        .map_err(|error| Failure::from(&error))?;
    let Some(state) = checked.state() else {
        return Err(Failure::Refused(
            "this store has no state folder that bilbo checked".to_owned(),
        ));
    };
    let state = state.try_clone();
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (done, thread) = match given {
        Err(refused) => (
            said(Message::warning(&refused.sentence()), End::Input),
            refused.session().map(str::to_owned),
        ),
        Ok(given) => {
            let thread = Some(given.session.clone());
            let done = match Prompt::new(&given.session, &given.prompt) {
                None => said(
                    Message::warning(&hook::Refused::NoPrompt(given.session).sentence()),
                    End::Input,
                ),
                Some(prompt) => {
                    let mut marks = seen::Seen::new(&state, seconds);
                    match digest::turn(prompt, &mut marks) {
                        Turn::Later => ended(Ok(Outcome::new("")), End::Later),
                        Turn::Unmarked(error) => said(
                            Message::warning(&format!(
                                "the session was not marked, so nothing was listed: {error}"
                            )),
                            End::Unmarked,
                        ),
                        Turn::First(first) => listed(checked, settings, reading, hand, &first, now),
                    }
                }
            };
            (done, thread)
        }
    };
    if !settings.digest.log_enabled.value {
        return finished(done.outcome, done.messages, None);
    }
    let millis = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    let line = logged(&Record {
        at: seconds,
        end: done.end,
        thread: thread.as_deref(),
        millis,
        listed: done.listed.as_ref(),
    });
    let unlogged = log::append(&state, &line)
        .err()
        .map(|error| Message::warning(&format!("the digest was not logged: {error}")));
    finished(done.outcome, done.messages, unlogged)
}

fn listed(
    checked: bilbo_store::segments::Checked,
    settings: &Settings,
    reading: &Reading<'_>,
    hand: &Hand<'_>,
    first: &First,
    now: SystemTime,
) -> Ended {
    let store = match checked.open(&mut |_| {}) {
        Ok(store) => store,
        Err(error) => {
            let end = if matches!(error, StoreError::Busy) {
                End::Busy
            } else {
                End::Failed
            };
            return ended(Err(Failure::from(&error)), end);
        }
    };
    let cache = store.reader_cache().clone();
    let lists: Vec<List<'_>> = hand
        .shelves
        .iter()
        .map(|shelf| shelf.list.clone())
        .collect();
    let manifest = Manifest::new(reading.chunk, hand.tokenizer, reading.render);
    let listed = match &cache {
        Cache::Own(path) => {
            let index = (reading.lookup)(path);
            let request = Request {
                first,
                lists: &lists,
                manifest: &manifest,
                digest: &settings.digest_search,
                ranker: &settings.recall,
                readers: reading.readers,
                width: settings.read_outline_width.value,
                labels: &store,
                at: now,
                way: hand.way,
            };
            match digest::list(&request, index.as_ref(), &store, reading.detect) {
                Ok(listed) => listed,
                Err(error) => return ended(Err(Failure::from(&error)), End::Failed),
            }
        }
        Cache::Absent => none(Stand::NoIndex),
        _ => none(Stand::Withheld),
    };
    drop(store);
    let end = End::of(&listed);
    if let Some(message) = unsearched(listed.stand, &cache) {
        return Ended {
            outcome: Ok(Outcome::new("")),
            messages: vec![message],
            end,
            listed: Some(listed),
        };
    }
    let owned: Vec<(&'static str, Vec<digest::Row>)> = listed
        .lists
        .iter()
        .filter_map(|chosen| {
            hand.shelves
                .iter()
                .find(|shelf| shelf.list.kind == chosen.kind)
                .map(|shelf| (shelf.word, chosen.rows.clone()))
        })
        .collect();
    let text = move |view: &View| {
        let shown: Vec<view::Shown<'_>> = owned
            .iter()
            .map(|(word, rows)| view::Shown { word, rows })
            .collect();
        view::lists(&shown, view)
    };
    Ended {
        outcome: Ok(Outcome::new(text(&View::PLAIN))
            .with_human(move |view| text(view))
            .on_every_terminal()),
        messages: Vec::new(),
        end,
        listed: Some(listed),
    }
}

fn none(stand: Stand) -> digest::Listed {
    digest::Listed {
        stand,
        lists: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{excused, options};
    use crate::console::env::Env;
    use crate::console::outcome::Failure;
    use crate::console::settings::Settings;
    use crate::console::view::View;

    fn configured(text: &str) -> Settings {
        let text = text.to_owned();
        crate::console::settings::load(
            &|name| match name {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_CONFIG" => Some(OsString::from("/c.toml")),
                _ => None,
            },
            &move |path| {
                if path == std::path::Path::new("/c.toml") {
                    Ok(text.clone().into_bytes())
                } else {
                    Err(std::io::ErrorKind::NotFound.into())
                }
            },
            &crate::kind_keys(),
            &crate::provider_keys(),
        )
        .expect("settings")
    }

    #[test]
    fn digest_takes_no_argument_and_exits_0_with_the_usage() {
        for (list, reason) in [
            (
                &["bilbo", "digest", "x"][..],
                "bilbo: unexpected argument 'x'",
            ),
            (
                &["bilbo", "digest", "--all"],
                "bilbo: unknown option '--all'",
            ),
        ] {
            let args: Vec<OsString> = list.iter().map(OsString::from).collect();
            let outcome = crate::run(
                &args,
                &mut Env::quiet(),
                Err(Failure::Config("no setting is read".to_owned())),
            )
            .expect("an outcome");
            assert_eq!(outcome.exit_code(), 0, "{list:?}");
            assert_eq!(outcome.stdout(&View::PLAIN), "", "{list:?}");
            let lines = outcome.stderr_lines(&View::PLAIN);
            assert_eq!(lines[0], reason, "{list:?}");
            assert_eq!(lines[1], "bilbo: usage: bilbo digest", "{list:?}");
            assert_eq!(lines[2], "bilbo: see 'bilbo digest --help'", "{list:?}");
            assert_eq!(lines.len(), 3, "{list:?}");
        }
    }

    #[test]
    fn a_failure_becomes_an_outcome_that_exits_0_with_its_lines() {
        let usage = Failure::Usage {
            reason: "unknown option '--all'".to_owned(),
            usage: vec!["usage: bilbo digest".to_owned()],
            page: "bilbo digest --help".to_owned(),
        };
        for failure in [
            usage,
            Failure::Config("digest.wait must be between 0s and 1m, got \"2m\"".to_owned()),
            Failure::Refused("no store at /s".to_owned()),
        ] {
            let outcome = excused(&failure);
            assert_eq!(outcome.exit_code(), 0, "{failure:?}");
            assert_eq!(outcome.write_failure_code(), 0, "{failure:?}");
            assert_eq!(outcome.stdout(&View::PLAIN), "", "{failure:?}");
            assert_eq!(
                outcome.stderr_lines(&View::PLAIN),
                failure.stderr_lines(&View::PLAIN),
                "{failure:?}"
            );
            assert_eq!(
                outcome.stderr_lines(&View::HUMAN),
                failure.stderr_lines(&View::HUMAN),
                "{failure:?}"
            );
        }
    }

    #[test]
    fn the_store_is_opened_with_digest_wait_and_the_stores_poll() {
        for wait in ["3s", "0s"] {
            let settings = configured(&format!(
                "digest.wait = \"{wait}\"\n[store]\nwait = \"7s\"\npoll = \"50ms\"\nseal_at = \"2 MiB\"\n"
            ));
            let own = settings.store.options();
            let given = options(&settings);
            let wanted = if wait == "3s" {
                std::time::Duration::from_secs(3)
            } else {
                std::time::Duration::ZERO
            };
            assert_eq!(given.wait, wanted);
            assert_eq!(own.wait, std::time::Duration::from_secs(7));
            assert_eq!(given.poll, std::time::Duration::from_millis(50));
            assert_eq!(given.seal_at, 2 * 1024 * 1024);
            assert_eq!(given.seal_at, own.seal_at);
            assert_eq!(given.seal_after, own.seal_after);
            assert_eq!(given.compression, own.compression);
            assert_eq!(given.tear_budget, own.tear_budget);
            assert_eq!(given.held, own.held);
            assert_eq!(given.rebuild, own.rebuild);
            assert!(given.hook.is_none());
        }
    }
}
