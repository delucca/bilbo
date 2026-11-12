pub mod log;
pub mod view;

use std::time::SystemTime;

use bilbo_search::bm25::Bm25;
use bilbo_search::position::Manifest;
use bilbo_search::rank::{Query, Ranker};
use bilbo_search::recall::{self, Access, Error, Finished, Request, Stand};
use bilbo_settings::file::File;
use bilbo_settings::key::{Flag, Source};
use bilbo_store::segments::{Cache, Notice, Store};
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::env::Env;
use crate::console::message::{Message, group, notice_message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::reading::Reading;
use crate::console::settings::Settings;
use crate::console::theme::Mark;

const SUMMARY: &str = "Search everything bilbo keeps and print the chunks that match best";
const ABOUT: &str = "bilbo recall: search what bilbo keeps and print the chunks that match best.";
const USAGE: &str = "bilbo recall <query>... [--kind <kind>]... [--budget <tokens>]";
const BUDGET_FLAG: &str = "--budget";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE, after_help = PAGE)]
pub struct Args {
    #[arg(
        value_name = "query",
        required = true,
        num_args = 1..,
        value_parser = NonEmptyStringValueParser::new()
    )]
    pub query: Vec<String>,
    #[arg(
        long,
        value_name = "kind",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Search entries of this kind only; repeat it for more"
    )]
    pub kind: Vec<String>,
    #[arg(
        long,
        value_name = "tokens",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Print at most this many tokens"
    )]
    pub budget: Option<String>,
}

const PAGE: &str = "\
Output:
  === <ref> <kind> <date> <n> lines  a hit's header, its fields tab-separated,
                                     then exactly <n> lines of text
  ... part <k>/<m>                   one piece of a record too long for a chunk
  ... cut                            the last hit, cut to fit the budget
The text is as it was kept, after a line its kind adds (a session's start).
<ref> is what bilbo read takes. stderr says how many entries were searched.

Each query that reaches the store is logged in the state folder's queries.log.

Exit:
  0  at least one hit
  1  nothing matched, no store, no search index, or a store or index failure
  2  usage or config error

Examples:
  bilbo recall how did we decide the index format
  bilbo recall --kind markdown --budget 1500 -- release checklist

Docs: https://github.com/delucca/bilbo/wiki/Commands#recall";

pub fn run(
    root: &Command,
    args: &Args,
    env: &mut Env,
    settings: &Settings,
    reading: &Reading<'_>,
) -> Result<Outcome, Failure> {
    let known: Vec<&str> = reading.readers.iter().map(|reader| reader.kind()).collect();
    if let Some(unknown) = args
        .kind
        .iter()
        .find(|kind| !known.contains(&kind.as_str()))
    {
        return Err(usage(
            root,
            sanitize(&format!(
                "unknown kind '{unknown}'; kinds: {}",
                known.join(", ")
            )),
            Some("recall"),
        ));
    }
    let none = File::default();
    let source = Source {
        var: &*env.var,
        file: &none,
    };
    let flag = args
        .budget
        .as_deref()
        .map(|text| Flag::parse(&recall::keys::BUDGET, &source, BUDGET_FLAG, text))
        .transpose()?;
    let budget = settings.recall.budget.clone().with_flag(flag);
    let tokenizer = (reading.tokenizer)()?;

    let notice = &mut env.notice;
    let store = Store::open_read_only(&settings.layout(), settings.store.options(), &mut |said| {
        if said == Notice::Waiting {
            notice(&notice_message(&Notice::Waiting));
        }
    })
    .map_err(|error| Failure::from(&error))?;
    let cache = store.reader_cache().clone();
    let index = match &cache {
        Cache::Own(path) => Some((reading.lookup)(path)),
        _ => None,
    };
    let bm25 = index.as_deref().map(|index| Bm25 {
        index,
        k1: settings.recall.bm25_k1.value,
        b: settings.recall.bm25_b.value,
        dropped: &settings.recall.question_words.value,
        batch: settings.recall.read_batch.value,
    });
    let rankers: Vec<&dyn Ranker> = bm25.iter().map(|ranker| ranker as &dyn Ranker).collect();
    let request = Request {
        query: Query {
            text: args.query.join(" "),
            kinds: args.kind.clone(),
        },
        manifest: Manifest::new(reading.chunk, tokenizer.id, reading.render),
        chunk: reading.chunk,
        candidates: settings.recall.candidates.value,
        cap: settings.recall.cap.value,
        budget: budget.value,
        at: SystemTime::now(),
    };
    let access = match (&cache, index.as_deref()) {
        (Cache::Own(_), Some(index)) => Access::Read {
            index,
            rankers: &rankers,
        },
        (Cache::Absent, _) => Access::NoIndex,
        _ => Access::Withheld,
    };
    let finished = recall::run(
        &request,
        &store,
        reading.readers,
        &*tokenizer.tokens,
        access,
        &view::hit,
        &mut log::File::at(store.state()),
    );
    drop(store);
    outcome(finished, &cache)
}

fn say(mark: Mark, text: String) -> Message {
    Message {
        mark,
        text,
        details: Vec::new(),
    }
}

fn entries(count: u64) -> String {
    format!(
        "{} {}",
        group(count),
        if count == 1 { "entry" } else { "entries" }
    )
}

fn withheld(cache: &Cache) -> Option<String> {
    match cache {
        Cache::Own(_) | Cache::Absent => None,
        Cache::Unlocked => Some(
            "no bilbo has written this store from this state folder, so nothing protects its search index while it is read; bilbo remember sets it up"
                .to_owned(),
        ),
        Cache::NoCatalog(path) => Some(format!(
            "the cache folder {} holds no catalog of this store, so its search index cannot be trusted; bilbo remember makes both again",
            path.display()
        )),
        Cache::OtherStore(path) => Some(format!(
            "the cache folder {} holds another store's catalog; set paths.cache to a folder of this store's own",
            path.display()
        )),
        Cache::Unsafe(path) => Some(format!(
            "{} is not this user's alone, or could not be checked; bilbo reads no search index under it",
            path.display()
        )),
    }
}

fn failure(error: &Error) -> Failure {
    match error {
        Error::Store(error) => Failure::from(error),
        Error::Index(
            bilbo_search::index::IndexError::Failed { .. }
            | bilbo_search::index::IndexError::Inconsistent(_),
        ) => Failure::Refused(sanitize(&format!(
            "{error}; bilbo reindex makes the index again"
        ))),
        Error::Index(_) => Failure::Refused(sanitize(&error.to_string())),
    }
}

pub fn outcome(finished: Finished, cache: &Cache) -> Result<Outcome, Failure> {
    let Finished { result, unlogged } = finished;
    let unlogged =
        unlogged.map(|error| say(Mark::Warning, format!("the query was not logged: {error}")));
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            let failure = failure(&error);
            let Some(unlogged) = unlogged else {
                return Err(failure);
            };
            let exit = if failure.exit_code() == 2 {
                Exit::Config
            } else {
                Exit::Refused
            };
            return Ok(Outcome::new("")
                .with_diagnostics(vec![failure.message(), unlogged])
                .with_exit(exit));
        }
    };
    let ran = matches!(
        output.stand,
        Stand::Current | Stand::Behind(_) | Stand::Unknown
    );
    let mut diagnostics = Vec::new();
    if ran {
        diagnostics.push(say(
            Mark::Info,
            format!("searched {}", entries(output.searched)),
        ));
    }
    match output.stand {
        Stand::Behind(count) => diagnostics.push(say(
            Mark::Info,
            format!(
                "the index is behind the store by {}; bilbo remember brings it up to date",
                entries(count)
            ),
        )),
        Stand::Unknown => diagnostics.push(say(
            Mark::Warning,
            "the index cannot be compared with the store's catalog, so each hit was checked against the store; bilbo remember brings both up to date"
                .to_owned(),
        )),
        _ => {}
    }
    if output.left_out > 0 {
        diagnostics.push(say(
            Mark::Warning,
            format!(
                "left out {} {} the store no longer gives as indexed; bilbo remember brings the index up to date",
                group(output.left_out),
                if output.left_out == 1 { "hit" } else { "hits" }
            ),
        ));
    }
    diagnostics.extend(unlogged);
    if ran && output.hits.is_empty() {
        diagnostics.push(say(Mark::Info, "nothing matched".to_owned()));
    }
    match output.stand {
        Stand::NoIndex => diagnostics.push(Message::error(
            "the store has no search index bilbo can use; bilbo reindex makes it",
        )),
        Stand::OtherManifest => diagnostics.push(Message::error(
            "the search index was made with other settings or by another bilbo; bilbo reindex makes it again",
        )),
        Stand::Withheld => diagnostics.extend(withheld(cache).map(Message::error)),
        Stand::Current | Stand::Behind(_) | Stand::Unknown => {}
    }
    let exit = if !ran {
        Exit::Refused
    } else if output.hits.is_empty() {
        Exit::NothingMatched
    } else {
        Exit::Success
    };
    let plain = view::plain(&output);
    Ok(Outcome::new(plain)
        .with_human(move |view| view::human(&output, view))
        .on_every_terminal()
        .with_diagnostics(diagnostics)
        .with_exit(exit))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_search::index::{Extent, IndexError};
    use bilbo_search::lookup::Found;
    use bilbo_search::recall::hit::Hit;
    use bilbo_search::recall::{Error, Finished, Output, Stand};
    use bilbo_store::entries::{Generation, RecordRange, StoreError};
    use bilbo_store::id::Id;
    use bilbo_store::segments::Cache;

    use super::outcome;
    use crate::console::outcome::Failure;
    use crate::console::view::View;

    fn hit() -> Hit {
        Hit {
            found: Found {
                entry: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("an id"),
                kind: "note".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange { first: 1, count: 2 },
                    part: None,
                    date: 0,
                },
                digest: 0,
            },
            preface: None,
            text: "text".to_owned(),
            cut: false,
        }
    }

    fn output(hits: usize, searched: u64, stand: Stand, left_out: u64) -> Output {
        Output {
            hits: vec![hit(); hits],
            searched,
            stand,
            left_out,
        }
    }

    fn finished(output: Output, unlogged: Option<&str>) -> Finished {
        Finished {
            result: Ok(output),
            unlogged: unlogged.map(str::to_owned),
        }
    }

    fn lines(finished: Finished, cache: &Cache) -> (u8, Vec<String>) {
        match outcome(finished, cache) {
            Ok(outcome) => (outcome.exit_code(), outcome.stderr_lines(&View::PLAIN)),
            Err(failure) => (failure.exit_code(), failure.stderr_lines(&View::PLAIN)),
        }
    }

    #[test]
    fn hits_exit_0_and_say_what_was_searched() {
        let done = outcome(
            finished(output(2, 1412, Stand::Current, 0), None),
            &Cache::Absent,
        )
        .expect("an outcome");
        assert_eq!(done.exit_code(), 0);
        assert_eq!(
            done.stderr_lines(&View::PLAIN),
            ["bilbo: searched 1,412 entries"]
        );
        assert!(done.stdout(&View::PLAIN).starts_with("=== bilbo:"));
    }

    #[test]
    fn no_hit_exits_1_and_says_nothing_matched() {
        let (code, stderr) = lines(
            finished(output(0, 3, Stand::Current, 0), None),
            &Cache::Absent,
        );
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            ["bilbo: searched 3 entries", "bilbo: nothing matched"]
        );
    }

    #[test]
    fn no_index_and_another_manifest_exit_1_and_name_reindex() {
        let (code, stderr) = lines(
            finished(output(0, 0, Stand::NoIndex, 0), None),
            &Cache::Absent,
        );
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            ["bilbo: the store has no search index bilbo can use; bilbo reindex makes it"]
        );
        let (code, stderr) = lines(
            finished(output(0, 0, Stand::OtherManifest, 0), None),
            &Cache::Absent,
        );
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            [
                "bilbo: the search index was made with other settings or by another bilbo; bilbo reindex makes it again"
            ]
        );
    }

    #[test]
    fn a_behind_index_left_out_hits_and_an_unwritten_log_each_get_their_line_in_order() {
        let (code, stderr) = lines(
            finished(output(1, 9, Stand::Behind(4), 3), Some("disk full")),
            &Cache::Absent,
        );
        assert_eq!(code, 0);
        assert_eq!(
            stderr,
            [
                "bilbo: searched 9 entries",
                "bilbo: the index is behind the store by 4 entries; bilbo remember brings it up to date",
                "bilbo: left out 3 hits the store no longer gives as indexed; bilbo remember brings the index up to date",
                "bilbo: the query was not logged: disk full",
            ]
        );
        let (_, stderr) = lines(
            finished(output(0, 9, Stand::Unknown, 0), None),
            &Cache::Absent,
        );
        assert_eq!(
            stderr[1],
            "bilbo: the index cannot be compared with the store's catalog, so each hit was checked against the store; bilbo remember brings both up to date"
        );
        assert_eq!(stderr[2], "bilbo: nothing matched");
    }

    #[test]
    fn one_entry_and_one_hit_are_singular() {
        let (_, stderr) = lines(
            finished(output(1, 1, Stand::Behind(1), 1), None),
            &Cache::Absent,
        );
        assert_eq!(stderr[0], "bilbo: searched 1 entry");
        assert!(
            stderr[1].contains("behind the store by 1 entry;"),
            "{stderr:?}"
        );
        assert!(
            stderr[2].starts_with("bilbo: left out 1 hit the store"),
            "{stderr:?}"
        );
    }

    #[test]
    fn each_reason_an_index_is_withheld_has_its_line_and_exits_1() {
        let path = PathBuf::from("/c/bilbo");
        for (cache, line) in [
            (
                Cache::Unlocked,
                "no bilbo has written this store from this state folder, so nothing protects its search index while it is read; bilbo remember sets it up".to_owned(),
            ),
            (
                Cache::NoCatalog(path.clone()),
                "the cache folder /c/bilbo holds no catalog of this store, so its search index cannot be trusted; bilbo remember makes both again".to_owned(),
            ),
            (
                Cache::OtherStore(path.clone()),
                "the cache folder /c/bilbo holds another store's catalog; set paths.cache to a folder of this store's own".to_owned(),
            ),
            (
                Cache::Unsafe(path),
                "/c/bilbo is not this user's alone, or could not be checked; bilbo reads no search index under it".to_owned(),
            ),
        ] {
            let (code, stderr) = lines(finished(output(0, 0, Stand::Withheld, 0), None), &cache);
            assert_eq!(code, 1, "{cache:?}");
            assert_eq!(stderr, [format!("bilbo: {line}")], "{cache:?}");
        }
    }

    fn failed(error: Error, unlogged: Option<&str>) -> Finished {
        Finished {
            result: Err(error),
            unlogged: unlogged.map(str::to_owned),
        }
    }

    #[test]
    fn a_damaged_index_names_reindex_and_a_foreign_folder_does_not() {
        let damaged = Error::Index(IndexError::Failed {
            action: "read",
            path: PathBuf::from("/c/index"),
            error: "its content could not be read".to_owned(),
        });
        let (code, stderr) = lines(failed(damaged, None), &Cache::Absent);
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            [
                "bilbo: cannot read the index /c/index: its content could not be read; bilbo reindex makes the index again"
            ]
        );
        let foreign = Error::Index(IndexError::Foreign(PathBuf::from("/c/index")));
        let (code, stderr) = lines(failed(foreign, None), &Cache::Absent);
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            ["bilbo: /c/index holds files that are not bilbo's index; bilbo will not touch it"]
        );
    }

    #[test]
    fn an_index_that_does_not_agree_with_itself_names_reindex() {
        let (code, stderr) = lines(
            failed(
                Error::Index(IndexError::Inconsistent(
                    "a posting names a document outside the index",
                )),
                None,
            ),
            &Cache::Absent,
        );
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            [
                "bilbo: the search index does not agree with itself: a posting names a document outside the index; bilbo reindex makes the index again"
            ]
        );
        let foreign = Error::Index(IndexError::Foreign(PathBuf::from("/c/index")));
        let (_, stderr) = lines(failed(foreign, None), &Cache::Absent);
        assert!(!stderr[0].contains("reindex"), "{stderr:?}");
    }

    #[test]
    fn an_error_and_an_unwritten_log_print_both_lines_and_keep_the_errors_exit() {
        let index = Error::Index(IndexError::Foreign(PathBuf::from("/c/index")));
        let (code, stderr) = lines(failed(index, Some("disk full")), &Cache::Absent);
        assert_eq!(code, 1);
        assert_eq!(
            stderr,
            [
                "bilbo: /c/index holds files that are not bilbo's index; bilbo will not touch it",
                "bilbo: the query was not logged: disk full",
            ]
        );
        let done = outcome(
            failed(
                Error::Store(StoreError::Older(PathBuf::from("/r/notes"))),
                Some("disk full"),
            ),
            &Cache::Absent,
        )
        .expect("both lines make an outcome");
        assert_eq!(done.exit_code(), 2);
        assert_eq!(done.stdout(&View::PLAIN), "");
        let alone = outcome(
            failed(
                Error::Store(StoreError::Older(PathBuf::from("/r/notes"))),
                None,
            ),
            &Cache::Absent,
        );
        assert!(matches!(alone, Err(Failure::Config(_))));
    }
}
