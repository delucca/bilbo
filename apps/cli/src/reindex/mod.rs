pub mod view;

use std::time::Instant;

use bilbo_search::reindex;
use bilbo_search::work::{Error, Output};
use bilbo_store::segments::{Notice, Options, Rebuild, Store};

use crate::console::closing::{Closer, Closing};
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::message::{notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;

const SUMMARY: &str = "Rebuild the catalog and the search index from the store";
const ABOUT: &str = "bilbo reindex: rebuild the catalog and the search index from the store.";
const USAGE: &str = "bilbo reindex";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {}

const PAGE: &str = "\
Both hold nothing the store does not: reindex makes them again from it.
It waits for another bilbo that is writing the store.

Output:
  entries <n>, chunks <n>, unread <n>, failed <n>
  index bytes <n>, milliseconds <n>

Exit:
  0  success, also when entries of a kind no reader reads were left out
  1  an entry failed or could not be read back, or a store or index failure
  2  usage or config error

Examples:
  bilbo reindex

Docs: https://github.com/delucca/bilbo/wiki/Commands#reindex";

/// The store's lock is held from the catalog's rebuild to the index's commit, then handed to
/// `closing`, which serves the again flag, grows the index after each pass and seals when due.
pub fn run(
    _args: &Args,
    env: &mut Env,
    settings: &Settings,
    indexing: &Indexing<'_>,
    closing: &Closer<'_>,
) -> Result<Outcome, Failure> {
    let started = Instant::now();
    let tokenizer = (indexing.tokenizer)()?;
    let layout = settings.layout();
    Store::check(&layout).map_err(|error| Failure::from(&error))?;
    let options = Options {
        rebuild: Rebuild::Always,
        ..settings.store.options()
    };
    let notice = &mut env.notice;
    let store = Store::open(&layout, options, &mut |heard| {
        if !matches!(heard, Notice::RebuiltCatalog { .. }) {
            notice(&notice_message(&heard));
        }
    })
    .map_err(|error| Failure::from(&error))?;
    let mut index = (indexing.index)(store.cache());
    let ran = reindex::run(
        &indexing.request,
        &store,
        &store,
        indexing.readers,
        &tokenizer,
        index.as_mut(),
    );
    let output = match ran {
        Ok(output) => output,
        Err(error) => {
            store.release();
            return Err(match error {
                Error::Store(error) => Failure::from(&error),
                other => Failure::Refused(sanitize(&other.to_string())),
            });
        }
    };
    let closed = closing(store, index.as_mut(), &mut *env.notice);
    drop(index);
    Ok(outcome(output, closed, started.elapsed().as_millis()))
}

fn outcome(output: Output, closing: Closing, milliseconds: u128) -> Outcome {
    let exit = if output.failed.is_empty() && !closing.failed && !closing.refused {
        Exit::Success
    } else {
        Exit::Refused
    };
    let mut diagnostics = view::left_out(&output);
    diagnostics.extend(closing.messages);
    Outcome::new(view::plain(&output, milliseconds))
        .with_human(move |view| view::human(&output, milliseconds, view))
        .with_diagnostics(diagnostics)
        .with_exit(exit)
}

#[cfg(test)]
mod tests {
    use bilbo_search::work::Output;
    use bilbo_store::id::Id;

    use super::outcome;
    use crate::console::closing::Closing;
    use crate::console::view::View;

    const STDOUT: &str =
        "entries 3, chunks 5, unread 0, failed 0\nindex bytes 10, milliseconds 7\n";

    fn output() -> Output {
        Output {
            entries: 3,
            chunks: 5,
            index_bytes: 10,
            ..Output::default()
        }
    }

    #[test]
    fn a_run_with_no_failed_entry_exits_0_and_says_nothing_on_stderr() {
        let outcome = outcome(output(), Closing::default(), 7);
        assert_eq!(outcome.exit_code(), 0);
        assert_eq!(outcome.stderr_lines(&View::PLAIN), Vec::<String>::new());
        assert_eq!(outcome.stdout(&View::PLAIN), STDOUT);
    }

    #[test]
    fn a_failed_entry_exits_1_and_is_named_on_stderr() {
        let id = Id::from_bytes([7; 16]);
        let mut output = output();
        output.unread.insert("pdf".to_owned(), 2);
        output.failed.push((id, "cannot split".to_owned()));
        let outcome = outcome(output, Closing::default(), 7);
        assert_eq!(outcome.exit_code(), 1);
        assert_eq!(
            outcome.stderr_lines(&View::PLAIN),
            [
                "bilbo: left out 2 entries of kind pdf: no reader reads it".to_owned(),
                format!("bilbo: left out {id}: cannot split"),
            ]
        );
        assert_eq!(
            outcome.stdout(&View::PLAIN),
            "entries 3, chunks 5, unread 2, failed 1\nindex bytes 10, milliseconds 7\n"
        );
    }

    #[test]
    fn a_pass_of_the_close_loop_that_was_refused_exits_1() {
        let closing = Closing {
            refused: true,
            ..Closing::default()
        };
        assert_eq!(outcome(output(), closing, 7).exit_code(), 1);
    }

    #[test]
    fn a_store_that_failed_in_the_close_loop_exits_1() {
        let closing = Closing {
            failed: true,
            ..Closing::default()
        };
        assert_eq!(outcome(output(), closing, 7).exit_code(), 1);
    }

    #[test]
    fn an_unread_kind_alone_exits_0() {
        let mut output = output();
        output.unread.insert("pdf".to_owned(), 1);
        let outcome = outcome(output, Closing::default(), 7);
        assert_eq!(outcome.exit_code(), 0);
        assert_eq!(
            outcome.stderr_lines(&View::PLAIN),
            ["bilbo: left out 1 entry of kind pdf: no reader reads it"]
        );
    }
}
