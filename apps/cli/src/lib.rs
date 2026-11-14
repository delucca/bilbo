pub mod config;
pub mod console;
pub mod help;
#[cfg(test)]
mod probe;
pub mod read;
pub mod recall;
pub mod reindex;
pub mod remember;

use std::ffi::OsString;

use bilbo_kinds::kind::Kind;
use bilbo_kinds::reader::Reader;
use bilbo_search::analyze::Detect;
use bilbo_search::index::Index;
use bilbo_search::lookup::Lookup;
use bilbo_search::tokenizer::Tokenizer;
use clap::{Command, CommandFactory, Parser, Subcommand};

use crate::console::args::{Parsed, parse, typed, usage, verbs, with_verbs};
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::outcome::{Failure, Outcome};
use crate::console::reading;
use crate::console::settings::{KindKey, Settings};
use crate::help::overview;
use crate::help::page::shape;

#[derive(Debug, Parser)]
#[command(
    name = "bilbo",
    version,
    about = overview::DESCRIPTION,
    override_usage = overview::USAGE,
    after_help = overview::AFTER,
    help_template = overview::TEMPLATE,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub verb: Option<Verb>,
}

#[derive(Debug, Subcommand)]
pub enum Verb {
    Remember(remember::Args),
    Recall(recall::Args),
    Read(read::Args),
    Reindex(reindex::Args),
    Config(config::Args),
}

/// A page that only names the kinds reads no setting.
pub(crate) fn listed() -> Vec<Box<dyn Kind>> {
    vec![
        Box::new(bilbo_markdown::Markdown::default()),
        Box::new(bilbo_session::Session::default()),
    ]
}

pub fn kinds(settings: &Settings) -> Result<Vec<Box<dyn Kind>>, Failure> {
    Ok(vec![
        Box::new(bilbo_markdown::Markdown::default()),
        Box::new(session(settings)?),
    ])
}

pub fn kind_keys() -> Vec<&'static KindKey> {
    vec![
        &bilbo_markdown::HEADING_WIDTH,
        &bilbo_session::UNPACK_FLOOR,
        &bilbo_session::UNPACK_RATIO,
    ]
}

fn registered(settings: &Settings, key: &'static KindKey) -> Result<usize, Failure> {
    settings.kind_value(key).ok_or_else(|| {
        Failure::Config(format!(
            "{} was not registered with the settings",
            key.about.name
        ))
    })
}

fn session(settings: &Settings) -> Result<bilbo_session::Session, Failure> {
    Ok(bilbo_session::Session::new(
        registered(settings, &bilbo_session::UNPACK_FLOOR)?,
        registered(settings, &bilbo_session::UNPACK_RATIO)?,
    ))
}

pub fn readers(settings: &Settings) -> Result<Vec<Box<dyn Reader>>, Failure> {
    let width = registered(settings, &bilbo_markdown::HEADING_WIDTH)?;
    Ok(vec![
        Box::new(bilbo_markdown::Markdown::new(width)),
        Box::new(session(settings)?),
    ])
}

/// One item for each reader, in the order of `readers()`.
pub fn rendering(settings: &Settings) -> Result<String, Failure> {
    let width = registered(settings, &bilbo_markdown::HEADING_WIDTH)?;
    let last = |name: &'static str| name.rsplit('.').next().unwrap_or(name);
    Ok([
        format!(
            "{}/{}/{}={width}",
            bilbo_markdown::Markdown::new(width).kind(),
            bilbo_markdown::RENDER,
            last(bilbo_markdown::HEADING_WIDTH.about.name)
        ),
        format!(
            "{}/{}",
            bilbo_session::Session::default().kind(),
            bilbo_session::RENDER
        ),
    ]
    .join(" "))
}

/// Loading it costs a tenth of a second: a verb that does not chunk never calls this.
pub fn tokenizer() -> Result<Tokenizer, Failure> {
    let qwen = bilbo_qwen::Qwen::load()
        .map_err(|error| Failure::Refused(format!("cannot load the tokenizer: {error}")))?;
    Ok(Tokenizer {
        tokens: Box::new(qwen),
        id: bilbo_qwen::ID,
    })
}

/// The index of a store's cache folder: nothing is opened or made before its first use.
pub fn index(cache: &std::path::Path, search: &bilbo_search::keys::Section) -> Box<dyn Index> {
    Box::new(bilbo_tantivy::Tantivy::at(
        bilbo_tantivy::folder(cache),
        bilbo_tantivy::Options {
            memory: search.memory(),
            merge_deleted_share: search.merge_deleted_share(),
        },
    ))
}

pub fn detect() -> &'static dyn Detect {
    &bilbo_whichlang::Whichlang
}

pub fn lookup(cache: &std::path::Path) -> Box<dyn Lookup> {
    Box::new(bilbo_tantivy::Committed::at(bilbo_tantivy::folder(cache)))
}

pub fn indexing<'a>(
    settings: &Settings,
    readers: &'a [Box<dyn Reader>],
    rendering: &str,
    tokenizer: &'a dyn Fn() -> Result<Tokenizer, Failure>,
    index: &'a dyn Fn(&std::path::Path) -> Box<dyn Index>,
) -> Indexing<'a> {
    Indexing {
        readers,
        request: bilbo_search::work::Request {
            chunk: settings.search.chunk(),
            render: rendering.to_owned(),
            workers: settings
                .search
                .workers(std::thread::available_parallelism().map_or(1, usize::from)),
            label_batch: settings.search.label_batch.value,
        },
        tries: settings.search.tries(),
        tokenizer_id: bilbo_qwen::ID,
        tokenizer,
        index,
    }
}

pub fn bilbo() -> Command {
    let root = Cli::command();
    verbs(&root).into_iter().fold(root, |root, name| {
        let kinds = (name == "remember").then(|| remember::page(&listed()));
        root.mut_subcommand(name, |verb| {
            shape(match kinds {
                Some(page) => verb.after_help(page),
                None => verb,
            })
        })
    })
}

/// `settings` is what `main` resolved; help, the version and a usage error never read it.
pub fn run(
    args: &[OsString],
    env: &mut Env,
    settings: Result<Settings, Failure>,
) -> Result<Outcome, Failure> {
    run_command(&bilbo(), args, env, settings)
}

/// What a test can change of the indexing a verb builds: the tokenizer's loader, the workers.
pub type Tune = dyn for<'a> Fn(Indexing<'a>) -> Indexing<'a>;

pub fn run_command(
    root: &Command,
    args: &[OsString],
    env: &mut Env,
    settings: Result<Settings, Failure>,
) -> Result<Outcome, Failure> {
    run_tuned(root, args, env, settings, &|indexing| indexing)
}

pub fn run_tuned(
    root: &Command,
    args: &[OsString],
    env: &mut Env,
    settings: Result<Settings, Failure>,
    tune: &Tune,
) -> Result<Outcome, Failure> {
    match parse(root, args)? {
        Parsed::Help(topic) => help::run(root, &topic),
        Parsed::Version => Ok(Outcome::new(format!(
            "bilbo {}\n",
            env!("CARGO_PKG_VERSION")
        ))),
        Parsed::Matches(matches) => {
            let Some(verb) = typed::<Cli>(root, &matches)?.verb else {
                return Err(usage(root, with_verbs(root, "missing verb"), None));
            };
            let settings = settings?;
            match verb {
                Verb::Remember(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    remember::run(root, args, env, &settings, &kinds(&settings)?, &indexing)
                }
                Verb::Recall(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    recall::run(
                        root,
                        &args,
                        env,
                        &settings,
                        &reading::of(&indexing, &lookup, detect()),
                    )
                }
                Verb::Read(args) => {
                    let readers = readers(&settings)?;
                    read::run(root, &args, env, &settings, &readers)
                }
                Verb::Reindex(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let kinds = kinds(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    reindex::run(&args, env, &settings, &indexing, &|store, index, notice| {
                        remember::follows::close(
                            store,
                            index,
                            &settings.follow.value,
                            &kinds,
                            &indexing,
                            notice,
                            remember::follows::Progress::default(),
                        )
                    })
                }
                Verb::Config(args) => config::run(root, args, &settings),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{bilbo, kinds, listed, readers, rendering, run, run_command};
    use crate::console::env::Env;
    use crate::console::outcome::Failure;
    use crate::console::settings::Settings;
    use crate::console::view::View;
    use crate::probe::{self, with_verb};

    fn defaults() -> Settings {
        crate::console::settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &super::kind_keys(),
        )
        .expect("the defaults")
    }

    #[test]
    fn the_label_batch_key_reaches_the_index_request() {
        let batch_of = |variable: Option<&str>| {
            let settings = crate::console::settings::load(
                &|name| match name {
                    "HOME" => Some(OsString::from("/h")),
                    "BILBO_SEARCH_LABEL_BATCH" => variable.map(OsString::from),
                    _ => None,
                },
                &|_| Err(std::io::ErrorKind::NotFound.into()),
                &super::kind_keys(),
            )
            .expect("the settings");
            let disk = |cache: &std::path::Path| super::index(cache, &settings.search);
            super::indexing(&settings, &[], "lines/1", &super::tokenizer, &disk)
                .request
                .label_batch
        };
        assert_eq!(batch_of(None), 512);
        assert_eq!(batch_of(Some("7")), 7);
    }

    #[test]
    fn a_reader_whose_key_was_not_registered_is_a_config_error_not_a_default() {
        let settings = crate::console::settings::load(
            &|name| match name {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_MARKDOWN_HEADING_WIDTH" => Some(OsString::from("20")),
                _ => None,
            },
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &[],
        )
        .expect("settings with no kind key");
        match readers(&settings) {
            Err(Failure::Config(text)) => {
                assert_eq!(
                    text,
                    "markdown.heading_width was not registered with the settings"
                );
            }
            other => panic!("{:?}", other.map(|list| list.len())),
        }
    }

    #[test]
    fn a_kind_whose_key_was_not_registered_is_a_config_error_not_a_default() {
        let settings = crate::console::settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &[],
        )
        .expect("settings with no kind key");
        match kinds(&settings) {
            Err(Failure::Config(text)) => {
                assert_eq!(
                    text,
                    "session.unpack_floor was not registered with the settings"
                );
            }
            other => panic!("{:?}", other.map(|list| list.len())),
        }
    }

    fn unread() -> Result<Settings, Failure> {
        Err(Failure::Config(
            "these tests never read a setting".to_owned(),
        ))
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn stdout(root: &clap::Command, list: &[&str]) -> String {
        run_command(root, &args(list), &mut Env::quiet(), unread())
            .unwrap_or_else(|failure| panic!("{list:?}: {failure:?}"))
            .stdout(&View::PLAIN)
    }

    #[test]
    fn every_kind_has_its_own_lowercase_name_and_its_reader() {
        let kinds = kinds(&defaults()).expect("the kinds");
        let names: Vec<&str> = kinds.iter().map(|kind| kind.name()).collect();
        assert!(!names.is_empty());
        let listed = listed();
        assert_eq!(
            listed.iter().map(|kind| kind.name()).collect::<Vec<_>>(),
            names
        );
        for name in &names {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()),
                "{name:?}"
            );
        }
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        let read: Vec<&str> = readers(&defaults())
            .expect("the readers")
            .iter()
            .map(|reader| reader.kind())
            .collect();
        assert_eq!(
            read, names,
            "a kind that is kept and cannot be read, or the reverse"
        );
        let rendered = rendering(&defaults()).expect("the rendering");
        let named: Vec<&str> = rendered
            .split(' ')
            .map(|item| item.split('/').next().unwrap_or_default())
            .collect();
        assert_eq!(named, names, "{rendered:?}");
    }

    #[test]
    fn the_rendering_names_each_reader_its_version_and_its_settings() {
        assert_eq!(
            rendering(&defaults()).expect("the rendering"),
            "markdown/1/heading_width=100 session/2"
        );
        let wide = crate::console::settings::load(
            &|name| match name {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_MARKDOWN_HEADING_WIDTH" => Some(OsString::from("40")),
                _ => None,
            },
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &super::kind_keys(),
        )
        .expect("settings");
        assert_eq!(
            rendering(&wide).expect("the rendering"),
            "markdown/1/heading_width=40 session/2"
        );
    }

    #[test]
    fn a_rendering_whose_key_was_not_registered_is_a_config_error() {
        let settings = crate::console::settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &[],
        )
        .expect("settings with no kind key");
        assert!(matches!(rendering(&settings), Err(Failure::Config(_))));
    }

    #[test]
    fn the_remember_page_lists_every_kind() {
        let page = stdout(&bilbo(), &["bilbo", "remember", "--help"]);
        let at = page.find("\nKinds:\n").expect("a Kinds section");
        for kind in listed() {
            let row = format!("  {:<8}  {}\n", kind.name(), kind.summary());
            assert!(page[at..].contains(&row), "{row:?} missing in\n{page}");
        }
    }

    #[test]
    fn a_newline_in_a_help_topic_cannot_cut_the_reason() {
        let reason =
            |list: &[&str]| match run_command(&bilbo(), &args(list), &mut Env::quiet(), unread()) {
                Err(Failure::Usage { reason, .. }) => reason,
                other => panic!("{list:?}: {other:?}"),
            };
        assert_eq!(
            reason(&["bilbo", "help", "a\nb"]),
            "unknown verb 'a\\nb'; the verbs are remember, recall, read, reindex, config"
        );
        assert_eq!(
            reason(&["bilbo", "help", "--a\nb"]),
            "unknown option '--a\\nb'"
        );
        assert_eq!(
            reason(&["bilbo", "help", "x", "y\nz"]),
            "unexpected argument 'y\\nz'"
        );
    }

    #[test]
    fn help_and_version_are_results_that_exit_0() {
        for list in [&["bilbo", "--help"][..], &["bilbo", "--version"]] {
            let outcome = run(&args(list), &mut Env::quiet(), unread()).expect("a result");
            assert_eq!(outcome.exit_code(), 0);
            assert!(outcome.stderr_lines(&View::PLAIN).is_empty());
        }
    }

    #[test]
    fn a_top_level_usage_error_is_two_lines() {
        for list in [
            &["bilbo"][..],
            &["bilbo", "frob"],
            &["bilbo", "--frobnicate"],
            &["bilbo", "help", "frob"],
            &["bilbo", "help", "frob", "x"],
        ] {
            let failure = run(&args(list), &mut Env::quiet(), unread()).expect_err("a usage error");
            assert_eq!(failure.exit_code(), 2, "{list:?}");
            for view in [View::PLAIN, View::HUMAN] {
                let lines = failure.stderr_lines(&view);
                assert_eq!(lines.len(), 2, "{list:?}: {lines:?}");
            }
        }
    }

    #[test]
    fn a_verb_page_is_the_same_from_every_help_form() {
        let root = with_verb(bilbo(), probe::verb());
        let page = stdout(&root, &["bilbo", "probe", "--help"]);
        for list in [
            &["bilbo", "probe", "-h"][..],
            &["bilbo", "help", "probe"],
            &["bilbo", "help", "probe", "--help"],
        ] {
            assert_eq!(stdout(&root, list), page, "{list:?}");
        }
    }

    #[test]
    fn a_verb_page_has_its_sections_in_order_within_80_columns() {
        let root = with_verb(bilbo(), probe::verb());
        let page = stdout(&root, &["bilbo", "probe", "--help"]);
        let at = |needle: &str| {
            page.find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing in\n{page}"))
        };
        assert!(page.starts_with("bilbo probe: shows the shape of a verb page.\n\nUsage:\n  "));
        assert!(
            at("Usage:\n  bilbo probe [options] <word>\n  bilbo probe --all <word>\n")
                < at("\nOptions:\n")
        );
        assert!(at("\nOptions:\n") < at("\nOutput:\n"));
        assert!(at("\nOutput:\n") < at("\nExit:\n  0  "));
        assert!(at("\nExit:\n  0  ") < at("\nExamples:\n"));
        assert!(at("\nExamples:\n") < at("\nDocs: "));
        assert!(!page.contains("--help"), "{page}");
        assert!(!page.contains("Only the long help says this."), "{page}");
        for line in page.lines() {
            assert!(line.chars().count() <= 80, "{line:?}");
        }
    }

    #[test]
    fn a_verb_page_on_a_terminal_is_styled() {
        let root = with_verb(bilbo(), probe::verb());
        let human = run_command(
            &root,
            &args(&["bilbo", "probe", "--help"]),
            &mut Env::quiet(),
            unread(),
        )
        .expect("a page")
        .stdout(&View::HUMAN);
        for styled in [
            "\x1b[1mUsage:\x1b[22m\n  \x1b[36mbilbo probe",
            "\x1b[36mbilbo probe [options] \x1b[39m\x1b[2m<word>\x1b[22m",
            "\x1b[36mbilbo probe --all \x1b[39m\x1b[2m<word>\x1b[22m",
            "\x1b[2mOne line per word.\x1b[22m",
            "\x1b[1mOptions:\x1b[22m\n",
            "\x1b[1m-k, --kind \x1b[22m\x1b[2m<kind>\x1b[22m",
            "\x1b[1m--all\x1b[22m",
            "\x1b[1m--times \x1b[22m\x1b[2m<n>\x1b[22m",
            "\x1b[1mOutput:\x1b[22m\n",
            "\n  \x1b[2m<word>\x1b[22m\n",
            "\x1b[1mExit:\x1b[22m\n  \x1b[1m0\x1b[22m  success\n",
            "\x1b[1mExamples:\x1b[22m\n",
            "\x1b[1mDocs:\x1b[22m \x1b[2mhttps://",
        ] {
            assert!(human.contains(styled), "{styled:?} missing in\n{human}");
        }
    }
}
