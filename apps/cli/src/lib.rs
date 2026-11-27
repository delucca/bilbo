pub mod config;
pub mod console;
pub mod derive;
pub mod digest;
pub mod forget;
pub mod help;
#[cfg(test)]
mod probe;
pub mod read;
pub mod recall;
pub mod refresh;
pub mod reindex;
pub mod remember;
pub mod tag;

use std::ffi::OsString;
use std::rc::Rc;

use bilbo_entries::kind::Kind;
use bilbo_entries::reader::Reader;
use bilbo_search::analyze::Detect;
use bilbo_search::index::Index;
use bilbo_search::lookup::Lookup;
use bilbo_search::tokenizer::Tokenizer;
use clap::{Command, CommandFactory, Parser, Subcommand};

use crate::console::args::{Parsed, parse, typed, usage, verbs, with_verbs};
use crate::console::env::{Env, Var};
use crate::console::indexing::Indexing;
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::reading;
use crate::console::settings::{Group, KindKey, Settings};
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
    Refresh(refresh::Args),
    Recall(recall::Args),
    Digest(digest::Args),
    Read(read::Args),
    Tag(tag::Args),
    Forget(forget::Args),
    Reindex(reindex::Args),
    Derive(derive::Args),
    Config(config::Args),
}

/// A page that only names the kinds reads no setting.
pub(crate) fn listed() -> Vec<Box<dyn Kind>> {
    vec![
        Box::new(bilbo_entries::markdown::Markdown::default()),
        Box::new(bilbo_entries::session::Session::default()),
    ]
}

pub fn kinds(settings: &Settings) -> Result<Vec<Box<dyn Kind>>, Failure> {
    Ok(vec![
        Box::new(bilbo_entries::markdown::Markdown::default()),
        Box::new(session(settings)?),
    ])
}

pub fn kind_keys() -> Vec<&'static KindKey> {
    vec![
        &bilbo_entries::markdown::HEADING_WIDTH,
        &bilbo_entries::session::UNPACK_FLOOR,
        &bilbo_entries::session::UNPACK_RATIO,
        &bilbo_entries::session::UNPACK_WINDOW,
        &bilbo_entries::session::WIDE_FIELDS,
    ]
}

static ANTHROPIC: Group = Group {
    about: &bilbo_llm::anthropic::ABOUT,
    resolve: bilbo_llm::anthropic::shown,
};

static OPENAI: Group = Group {
    about: &bilbo_llm::openai::ABOUT,
    resolve: bilbo_llm::openai::shown,
};

pub fn provider_keys() -> Vec<&'static Group> {
    vec![&ANTHROPIC, &OPENAI]
}

pub fn providers() -> Vec<(&'static str, &'static str)> {
    vec![
        (bilbo_llm::anthropic::NAME, bilbo_llm::anthropic::SUMMARY),
        (bilbo_llm::openai::NAME, bilbo_llm::openai::SUMMARY),
    ]
}

pub fn model(
    settings: &Settings,
    var: &Var,
    named: Option<(&'static str, &str)>,
) -> Result<derive::Built, Failure> {
    let state = settings.state.value.clone();
    let timeout = settings.llm.timeout.value;
    let response = usize::try_from(settings.llm.response.value).unwrap_or(usize::MAX);
    let poll = settings.llm.poll.value;
    let reading = |names: Vec<String>| -> bilbo_llm::model::Variables {
        let var = Rc::clone(var);
        Box::new(move |asked: &str| {
            if names.iter().any(|name| name == asked) {
                var(asked).and_then(|text| text.into_string().ok())
            } else {
                None
            }
        })
    };
    let source = settings.source(&**var);
    let name = settings.llm.provider.value.as_str();
    let (writer, api): (Box<dyn bilbo_llm::model::Model>, bool) = match name {
        bilbo_llm::anthropic::NAME => {
            let section = bilbo_llm::anthropic::section(&source)?;
            let section = match named {
                Some((key, id)) => section.asking(key, id),
                None => section,
            };
            let api = section.via.value == bilbo_llm::model::Way::Api;
            let host = bilbo_llm::model::Host {
                variables: reading(section.reads()),
                work: derive::disk::work(&state),
                state,
                timeout,
                response,
                poll,
            };
            (bilbo_llm::anthropic::model(section, host), api)
        }
        bilbo_llm::openai::NAME => {
            let section = bilbo_llm::openai::section(&source)?;
            let section = match named {
                Some((key, id)) => section.asking(key, id),
                None => section,
            };
            let api = section.via.value == bilbo_llm::model::Way::Api;
            let host = bilbo_llm::model::Host {
                variables: reading(section.reads()),
                work: derive::disk::work(&state),
                state,
                timeout,
                response,
                poll,
            };
            (bilbo_llm::openai::model(section, host), api)
        }
        other => {
            let names: Vec<&str> = providers().iter().map(|(name, _)| *name).collect();
            return Err(Failure::Config(format!(
                "llm.provider must be one of {}, got \"{other}\"",
                names.join(", ")
            )));
        }
    };
    Ok(derive::Built { writer, api })
}

fn registered(settings: &Settings, key: &'static KindKey) -> Result<usize, Failure> {
    settings.kind_value(key).ok_or_else(|| {
        Failure::Config(format!(
            "{} was not registered with the settings",
            key.about.name
        ))
    })
}

fn session(settings: &Settings) -> Result<bilbo_entries::session::Session, Failure> {
    Ok(bilbo_entries::session::Session::new(
        registered(settings, &bilbo_entries::session::UNPACK_FLOOR)?,
        registered(settings, &bilbo_entries::session::UNPACK_RATIO)?,
        registered(settings, &bilbo_entries::session::UNPACK_WINDOW)?,
        registered(settings, &bilbo_entries::session::WIDE_FIELDS)?,
    ))
}

pub fn apart() -> Vec<&'static str> {
    vec![notes().kind]
}

pub fn labelled(id: bilbo_store::id::Id) -> Vec<bilbo_store::labels::CaptureLabel> {
    vec![
        bilbo_derive::plan::label(id),
        bilbo_derive::summary::label(id),
    ]
}

pub struct Notes {
    pub kind: &'static str,
    pub record: fn(&str, std::time::SystemTime) -> Option<Vec<u8>>,
    pub label: fn(bilbo_store::id::Id) -> bilbo_store::labels::CaptureLabel,
    pub write: fn(&bilbo_entries::note::record::Record) -> Vec<u8>,
    pub line: fn(&bilbo_derive::view::Paragraph) -> Vec<u8>,
    pub paragraph: fn(&[u8]) -> Option<bilbo_derive::view::Paragraph>,
}

pub fn notes() -> Notes {
    Notes {
        kind: bilbo_entries::note::NAME,
        record: bilbo_entries::note::written::record,
        label: bilbo_entries::note::written::label,
        write: bilbo_entries::note::record::write,
        line: note_line,
        paragraph: note_paragraph,
    }
}

fn note_line(paragraph: &bilbo_derive::view::Paragraph) -> Vec<u8> {
    let record = bilbo_entries::note::record::Record {
        text: paragraph.text.clone(),
        at: paragraph.at.and_then(|time| {
            time.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|since| since.as_secs())
        }),
        from: vec![bilbo_entries::note::record::Source {
            entry: paragraph.source.parent,
            generation: paragraph.source.generation.0,
            first: paragraph.source.first,
            last: paragraph.source.last,
        }],
    };
    let mut bytes = bilbo_entries::note::record::write(&record);
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    bytes
}

fn note_paragraph(bytes: &[u8]) -> Option<bilbo_derive::view::Paragraph> {
    let record = bilbo_entries::note::record::read(bytes)?;
    let [source] = record.from.as_slice() else {
        return None;
    };
    Some(bilbo_derive::view::Paragraph {
        text: record.text.clone(),
        at: record
            .at
            .map(|at| std::time::UNIX_EPOCH + std::time::Duration::from_secs(at)),
        source: bilbo_derive::tries::Extent {
            parent: source.entry,
            generation: bilbo_store::entries::Generation(source.generation),
            first: source.first,
            last: source.last,
        },
    })
}

fn collapsed(text: &str, width: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    bilbo_entries::read::outline::shown(&one, width)
}

fn note_record(bytes: &[u8], width: usize) -> Option<(Vec<bilbo_search::digest::Source>, String)> {
    let record = bilbo_entries::note::record::read(bytes)?;
    let from = record
        .from
        .iter()
        .map(|source| bilbo_search::digest::Source {
            entry: source.entry,
            generation: bilbo_store::entries::Generation(source.generation),
            records: bilbo_store::entries::RecordRange {
                first: source.first,
                count: source.last - source.first + 1,
            },
        })
        .collect();
    Some((from, collapsed(&record.text, width)))
}

pub type NoteLine = Box<dyn Fn(&[u8]) -> Option<String>>;
pub type NoteWritten = Box<dyn Fn(&[u8]) -> Option<bilbo_search::digest::Written>>;

pub struct NoteWays {
    pub line: NoteLine,
    pub written: NoteWritten,
}

pub fn note_ways(width: usize) -> NoteWays {
    NoteWays {
        line: Box::new(move |bytes| note_record(bytes, width).map(|(_, text)| text)),
        written: Box::new(move |bytes| {
            note_record(bytes, width)
                .map(|(from, text)| bilbo_search::digest::Written { from, text })
        }),
    }
}

fn summary_text(bytes: &[u8]) -> Option<String> {
    let record = bilbo_derive::summary::record::read(bytes)?;
    Some(record.text.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub type SummaryShown = Box<dyn Fn(&[u8]) -> Option<String>>;

#[must_use]
pub fn summary_shown(settings: &Settings) -> Option<SummaryShown> {
    if !settings.digest.summary_enabled.value {
        return None;
    }
    Some(Box::new(summary_text))
}

pub fn hands<'a>(
    settings: &Settings,
    readers: &[Box<dyn Reader>],
    ways: &'a NoteWays,
    summaries: Option<&'a SummaryShown>,
) -> Result<(Vec<digest::Shelf<'a>>, bilbo_search::digest::Way<'a>), Failure> {
    let shelves = lists(settings, readers, &*ways.line)?;
    let way = bilbo_search::digest::Way {
        label: &bilbo_derive::plan::label,
        written: &*ways.written,
        summary: summaries.map(|text| bilbo_search::digest::Summary {
            label: &bilbo_derive::summary::label,
            text: &**text,
        }),
    };
    Ok((shelves, way))
}

pub fn lists<'a>(
    settings: &Settings,
    readers: &[Box<dyn Reader>],
    shows: bilbo_search::digest::Line<'a>,
) -> Result<Vec<digest::Shelf<'a>>, Failure> {
    let markdown = bilbo_entries::markdown::Markdown::default().kind();
    let session = bilbo_entries::session::Session::default().kind();
    let shown = &settings.digest;
    readers
        .iter()
        .map(|reader| {
            let kind = reader.kind().to_owned();
            let (word, list) = if kind == markdown {
                (
                    "files",
                    bilbo_search::digest::List {
                        kind,
                        most: shown.markdown_most.value,
                        floor: shown.markdown_floor.value,
                        line: None,
                        old_after: None,
                        lists_belonging: true,
                        lists_parts: true,
                    },
                )
            } else if kind == session {
                (
                    "sessions",
                    bilbo_search::digest::List {
                        kind,
                        most: shown.session_most.value,
                        floor: shown.session_floor.value,
                        line: None,
                        old_after: Some(shown.session_old_after.value),
                        lists_belonging: shown.session_subagents.value,
                        lists_parts: shown.session_parts.value,
                    },
                )
            } else if kind == bilbo_entries::note::NAME {
                (
                    "notes",
                    bilbo_search::digest::List {
                        kind,
                        most: shown.note_most.value,
                        floor: shown.note_floor.value,
                        line: Some(shows),
                        old_after: None,
                        lists_belonging: true,
                        lists_parts: true,
                    },
                )
            } else {
                return Err(Failure::Config(format!(
                    "no list is declared for the kind {kind}"
                )));
            };
            Ok(digest::Shelf { word, list })
        })
        .collect()
}

pub fn readers(settings: &Settings) -> Result<Vec<Box<dyn Reader>>, Failure> {
    let width = registered(settings, &bilbo_entries::markdown::HEADING_WIDTH)?;
    Ok(vec![
        Box::new(bilbo_entries::markdown::Markdown::new(width)),
        Box::new(session(settings)?),
        Box::new(bilbo_entries::note::Note),
    ])
}

pub fn rendering(settings: &Settings) -> Result<String, Failure> {
    let width = registered(settings, &bilbo_entries::markdown::HEADING_WIDTH)?;
    let last = |name: &'static str| name.rsplit('.').next().unwrap_or(name);
    Ok([
        format!(
            "{}/{}/{}={width}",
            bilbo_entries::markdown::Markdown::new(width).kind(),
            bilbo_entries::markdown::RENDER,
            last(bilbo_entries::markdown::HEADING_WIDTH.about.name)
        ),
        format!(
            "{}/{}",
            bilbo_entries::session::Session::default().kind(),
            bilbo_entries::session::RENDER
        ),
        format!(
            "{}/{}",
            bilbo_entries::note::NAME,
            bilbo_entries::note::RENDER
        ),
    ]
    .join(" "))
}

/// Loading it costs a tenth of a second: a verb that does not chunk never calls this.
pub fn tokenizer() -> Result<Tokenizer, Failure> {
    let qwen = bilbo_search::qwen::Qwen::load()
        .map_err(|error| Failure::Refused(format!("cannot load the tokenizer: {error}")))?;
    Ok(Tokenizer {
        tokens: Box::new(qwen),
        id: bilbo_search::qwen::ID,
    })
}

/// Opens and makes nothing before its first use.
pub fn index(cache: &std::path::Path, search: &bilbo_search::keys::Section) -> Box<dyn Index> {
    Box::new(bilbo_search::tantivy::Tantivy::at(
        bilbo_search::tantivy::folder(cache),
        bilbo_search::tantivy::Options {
            memory: search.memory(),
            merge_deleted_share: search.merge_deleted_share(),
            list_limit: search.list_limit.value,
        },
    ))
}

pub fn detect() -> &'static dyn Detect {
    &bilbo_search::whichlang::Whichlang
}

pub fn lookup(cache: &std::path::Path, search: &bilbo_search::keys::Section) -> Box<dyn Lookup> {
    Box::new(bilbo_search::tantivy::Committed::at(
        bilbo_search::tantivy::folder(cache),
        search.list_limit.value,
    ))
}

pub fn kept_unread() -> Vec<String> {
    vec![bilbo_derive::summary::KIND.to_owned()]
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
            kept_unread: kept_unread(),
        },
        tries: settings.search.tries(),
        tokenizer_id: bilbo_search::qwen::ID,
        tokenizer,
        index,
    }
}

pub fn bilbo() -> Command {
    let root = Cli::command();
    verbs(&root).into_iter().fold(root, |root, name| {
        let page = match name.as_str() {
            "remember" => Some(remember::page(&listed())),
            "derive" => {
                let shipped = bilbo_derive::instruction::shipped();
                let kinds: Vec<&str> = shipped.iter().map(|(kind, _)| *kind).collect();
                Some(derive::page(&kinds, &providers()))
            }
            _ => None,
        };
        root.mut_subcommand(name, |verb| {
            shape(match page {
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
    let digest = args.get(1).is_some_and(|word| word == "digest");
    let result = dispatch(root, args, env, settings, tune);
    if !digest {
        return result;
    }
    Ok(match result {
        Ok(outcome) => outcome.with_exit(Exit::Always),
        Err(failure) => digest::excused(&failure),
    })
}

fn dispatch(
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
            if let Verb::Config(args) = &verb
                && let Some(config::Sub::Set(set)) = &args.set
            {
                return config::set::run(root, set, &*env.var, &kind_keys(), &provider_keys());
            }
            let settings = settings?;
            match verb {
                Verb::Remember(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    let note = notes();
                    let written = remember::note::Written {
                        kind: note.kind,
                        record: note.record,
                        label: note.label,
                    };
                    remember::run(
                        root,
                        args,
                        env,
                        &settings,
                        &kinds(&settings)?,
                        &indexing,
                        &remember::Handed {
                            written: &written,
                            labelled: &labelled,
                        },
                    )
                }
                Verb::Refresh(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let kinds = kinds(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    refresh::run(&args, env, &|env| {
                        remember::follows::run(env, &settings, &kinds, &indexing)
                    })
                }
                Verb::Recall(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    let committed = |cache: &std::path::Path| lookup(cache, &settings.search);
                    recall::run(
                        root,
                        &args,
                        env,
                        &settings,
                        &reading::of(&indexing, &committed, detect(), &apart()),
                    )
                }
                Verb::Digest(_) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    let committed = |cache: &std::path::Path| lookup(cache, &settings.search);
                    let width = settings.digest.note_width.value;
                    let ways = note_ways(width);
                    let summaries = summary_shown(&settings);
                    let (shelves, way) = hands(&settings, &readers, &ways, summaries.as_ref())?;
                    let apart = apart();
                    digest::run(
                        root,
                        env,
                        &settings,
                        &reading::of(&indexing, &committed, detect(), &apart),
                        &digest::Hand {
                            tokenizer: indexing.tokenizer_id,
                            shelves: &shelves,
                            way,
                        },
                    )
                }
                Verb::Read(args) => {
                    let readers = readers(&settings)?;
                    read::run(
                        root,
                        &args,
                        env,
                        &settings,
                        &readers,
                        &read::Summaries {
                            label: &bilbo_derive::summary::label,
                            text: &summary_text,
                        },
                    )
                }
                Verb::Tag(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let kinds = kinds(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    tag::run(
                        root,
                        &args,
                        env,
                        &settings,
                        &indexing,
                        &|store, index, notice| {
                            remember::follows::close(
                                store,
                                index,
                                &settings.follow.value,
                                &kinds,
                                &indexing,
                                notice,
                                remember::follows::Progress::default(),
                            )
                        },
                    )
                }
                Verb::Forget(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let kinds = kinds(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    forget::run(
                        root,
                        &args,
                        env,
                        &settings,
                        &readers,
                        &|store, notice| {
                            remember::follows::finish(
                                store,
                                &settings.follow.value,
                                &kinds,
                                &indexing,
                                notice,
                                0,
                            )
                        },
                        &labelled,
                    )
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
                Verb::Derive(args) => {
                    let readers = readers(&settings)?;
                    let rendering = rendering(&settings)?;
                    let kinds = kinds(&settings)?;
                    let disk = |cache: &std::path::Path| index(cache, &settings.search);
                    let indexing =
                        tune(indexing(&settings, &readers, &rendering, &tokenizer, &disk));
                    let committed = |cache: &std::path::Path| lookup(cache, &settings.search);
                    let note = notes();
                    let hand = derive::Hand {
                        readers: &readers,
                        tokenizer: &tokenizer,
                        lookup: &committed,
                        chunk: indexing.request.chunk,
                        render: &rendering,
                        format: bilbo_derive::view::Format {
                            kind: note.kind,
                            write: &note.line,
                            read: &note.paragraph,
                        },
                        writer: &|var, named| model(&settings, var, named),
                        finish: &|store, kept, notice| {
                            if kept == 0 {
                                let mut index = (indexing.index)(store.cache());
                                return remember::follows::close(
                                    store,
                                    index.as_mut(),
                                    &settings.follow.value,
                                    &kinds,
                                    &indexing,
                                    notice,
                                    remember::follows::Progress::default(),
                                );
                            }
                            remember::follows::finish(
                                store,
                                &settings.follow.value,
                                &kinds,
                                &indexing,
                                notice,
                                kept,
                            )
                        },
                    };
                    derive::run(root, &args, env, &settings, &hand)
                }
                Verb::Config(args) => config::run(root, args, &settings),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::rc::Rc;

    use super::{apart, bilbo, kinds, listed, notes, readers, rendering, run, run_command};
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
            &super::provider_keys(),
        )
        .expect("the defaults")
    }

    fn with_provider(name: &str) -> Settings {
        let name = name.to_owned();
        crate::console::settings::load(
            &move |variable| match variable {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_LLM_PROVIDER" => Some(OsString::from(name.clone())),
                _ => None,
            },
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &super::kind_keys(),
            &super::provider_keys(),
        )
        .expect("settings")
    }

    #[test]
    fn every_shipped_instruction_is_for_a_kind_some_reader_reads() {
        let read: Vec<&str> = readers(&defaults())
            .expect("the readers")
            .iter()
            .map(|reader| reader.kind())
            .collect();
        let shipped = bilbo_derive::instruction::shipped();
        assert!(!shipped.is_empty());
        for (kind, _) in shipped {
            assert!(read.contains(&kind), "{kind}");
            assert_ne!(kind, notes().kind);
        }
    }

    #[test]
    fn the_writer_is_the_provider_the_settings_name() {
        for (name, _) in super::providers() {
            let settings = with_provider(name);
            let var: crate::console::env::Var = Rc::new(|_| None);
            let built = super::model(&settings, &var, None).expect("a writer");
            assert!(
                built.writer.describe()[0].starts_with(&format!("provider: {name}")),
                "{name}"
            );
            assert!(built.api, "{name}");
        }
    }

    #[test]
    fn the_root_builds_another_model_from_the_key_and_the_id_it_is_given() {
        let settings = with_provider("anthropic");
        let var: crate::console::env::Var =
            Rc::new(|name| (name == "BILBO_LLM_ANTHROPIC_MODEL").then(|| OsString::from("m1")));
        let own = super::model(&settings, &var, None).expect("the provider's model");
        assert_eq!(own.writer.id(), "m1");
        let other = super::model(&settings, &var, Some(("derive.tags.model", "m2")))
            .expect("another model");
        assert_eq!(other.writer.id(), "m2");
    }

    fn recording(
        set: Vec<(&'static str, &'static str)>,
    ) -> (crate::console::env::Var, Rc<RefCell<Vec<String>>>) {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&asked);
        let var: crate::console::env::Var = Rc::new(move |name| {
            sink.borrow_mut().push(name.to_owned());
            set.iter()
                .find(|(held, _)| *held == name)
                .map(|(_, value)| OsString::from(*value))
        });
        (var, asked)
    }

    #[test]
    fn writer_builds_an_adapter_and_reads_no_variable_of_its_own() {
        for (name, key_env) in [
            ("anthropic", "BILBO_LLM_ANTHROPIC_API_KEY_ENV"),
            ("openai", "BILBO_LLM_OPENAI_API_KEY_ENV"),
        ] {
            let settings = with_provider(name);
            let (var, asked) = recording(vec![(key_env, "PLUM_KEY"), ("PLUM_KEY", "sk-test")]);
            let built = super::model(&settings, &var, None).expect("a writer");
            assert!(
                asked.borrow().iter().all(|name| name.starts_with("BILBO_")),
                "{name}: {:?}",
                asked.borrow()
            );
            asked.borrow_mut().clear();
            built.writer.describe();
            assert_eq!(*asked.borrow(), ["PLUM_KEY"], "{name}");
        }
        let settings = crate::console::settings::load(
            &|variable| match variable {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_LLM_PROVIDER" => Some(OsString::from("anthropic")),
                _ => None,
            },
            &|_| {
                Ok(b"[llm.anthropic]\nvia = \"tool\"\n\n[llm.anthropic.tool]\ninherit = [\"PLUM_TOKEN\"]\n".to_vec())
            },
            &super::kind_keys(),
            &super::provider_keys(),
        )
        .expect("settings");
        let (var, asked) = recording(vec![("PLUM_TOKEN", "t"), ("HOME", "/h")]);
        super::model(&settings, &var, None).expect("a writer");
        assert!(
            asked
                .borrow()
                .iter()
                .all(|name| name != "HOME" && name != "PLUM_TOKEN"),
            "{:?}",
            asked.borrow()
        );
    }

    #[test]
    fn the_tools_reader_is_given_the_names_of_inherit() {
        let state = std::env::temp_dir().join(format!("tool-names-{}", std::process::id()));
        std::fs::create_dir_all(&state).expect("a folder");
        let settings = crate::console::settings::load(
            &|variable| match variable {
                "HOME" => Some(OsString::from("/h")),
                "XDG_STATE_HOME" => Some(state.clone().into_os_string()),
                "BILBO_LLM_PROVIDER" => Some(OsString::from("anthropic")),
                _ => None,
            },
            &|_| {
                Ok(b"[llm.anthropic]\nvia = \"tool\"\nmodel = \"m\"\n\n[llm.anthropic.tool]\nprogram = \"/bin/sh\"\ninherit = [\"HOME\", \"PLUM_TOKEN\"]\n".to_vec())
            },
            &super::kind_keys(),
            &super::provider_keys(),
        )
        .expect("settings");
        let (var, asked) = recording(vec![("PLUM_TOKEN", "t"), ("HOME", "/h"), ("OTHER", "o")]);
        let mut built = super::model(&settings, &var, None).expect("a writer");
        asked.borrow_mut().clear();
        built.writer.prepare().expect("a prepared writer");
        let mut names = asked.borrow().clone();
        names.sort();
        names.dedup();
        assert_eq!(names, ["HOME", "PLUM_TOKEN"]);
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn an_unknown_provider_is_a_config_error_that_lists_the_providers() {
        let var: crate::console::env::Var = Rc::new(|_| None);
        match super::model(&with_provider("gemini"), &var, None) {
            Err(Failure::Config(text)) => assert_eq!(
                text,
                "llm.provider must be one of anthropic, openai, got \"gemini\""
            ),
            other => panic!("{:?}", other.map(|built| built.api)),
        }
    }

    #[test]
    fn the_providers_keys_are_registered_in_the_providers_order() {
        let names: Vec<&str> =
            crate::console::settings::keys(&super::kind_keys(), &super::provider_keys())
                .iter()
                .map(|about| about.name)
                .collect();
        let from = names
            .iter()
            .position(|name| *name == "llm.anthropic.model")
            .expect("llm.anthropic.model");
        let tail: Vec<&str> = names[from..from + 28].to_vec();
        assert_eq!(
            tail,
            [
                "llm.anthropic.model",
                "llm.anthropic.via",
                "llm.anthropic.api.endpoint",
                "llm.anthropic.api.version",
                "llm.anthropic.api.key_env",
                "llm.anthropic.api.key_command",
                "llm.anthropic.api.headers",
                "llm.anthropic.api.extra",
                "llm.anthropic.tool.program",
                "llm.anthropic.tool.effort",
                "llm.anthropic.tool.arguments",
                "llm.anthropic.tool.inherit",
                "llm.anthropic.tool.environment",
                "llm.anthropic.tool.project_files",
                "llm.openai.model",
                "llm.openai.via",
                "llm.openai.api.endpoint",
                "llm.openai.api.key_env",
                "llm.openai.api.key_command",
                "llm.openai.api.headers",
                "llm.openai.api.extra",
                "llm.openai.tool.program",
                "llm.openai.tool.effort",
                "llm.openai.tool.arguments",
                "llm.openai.tool.inherit",
                "llm.openai.tool.environment",
                "llm.openai.tool.home",
                "llm.openai.tool.system_files",
            ]
        );
        let order: Vec<&str> = super::providers().iter().map(|(name, _)| *name).collect();
        assert_eq!(order, ["anthropic", "openai"]);
    }

    #[test]
    fn claude_is_no_provider() {
        let var: crate::console::env::Var = Rc::new(|_| None);
        match super::model(&with_provider("claude"), &var, None) {
            Err(Failure::Config(text)) => assert_eq!(
                text,
                "llm.provider must be one of anthropic, openai, got \"claude\""
            ),
            other => panic!("{:?}", other.map(|built| built.api)),
        }
    }

    #[test]
    fn the_way_is_the_one_via_names() {
        let built = |provider: &'static str, file: &'static [u8]| {
            let settings = crate::console::settings::load(
                &move |variable| match variable {
                    "HOME" => Some(OsString::from("/h")),
                    "BILBO_LLM_PROVIDER" => Some(OsString::from(provider)),
                    _ => None,
                },
                &move |_| Ok(file.to_vec()),
                &super::kind_keys(),
                &super::provider_keys(),
            )
            .expect("settings");
            let var: crate::console::env::Var = Rc::new(|_| None);
            super::model(&settings, &var, None).expect("a writer")
        };
        let tool = built("anthropic", b"[llm.anthropic]\nvia = \"tool\"\n");
        assert!(tool.writer.describe()[0].starts_with("provider: anthropic, via tool, "));
        assert!(!tool.api);
        let api = built("anthropic", b"");
        assert!(api.writer.describe()[0].starts_with("provider: anthropic, via api, "));
        assert!(api.api);
        let openai = built("openai", b"");
        assert!(openai.writer.describe()[0].starts_with("provider: openai, via api, "));
        assert!(openai.api);
        let codex = built("openai", b"[llm.openai]\nvia = \"tool\"\n");
        assert!(codex.writer.describe()[0].starts_with("provider: openai, via tool, "));
        assert!(!codex.api);
    }

    #[test]
    fn the_kinds_kept_with_no_reader_are_handed_to_every_index_run() {
        let kept = super::kept_unread();
        assert_eq!(kept, [bilbo_derive::summary::KIND]);
        let settings = defaults();
        for reader in readers(&settings).expect("the readers") {
            assert!(!kept.iter().any(|kind| kind == reader.kind()));
        }
        let disk = |cache: &std::path::Path| super::index(cache, &settings.search);
        let indexing = super::indexing(&settings, &[], "lines/1", &super::tokenizer, &disk);
        assert_eq!(indexing.request.kept_unread, kept);
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
                &super::provider_keys(),
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
            &super::provider_keys(),
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
            &super::provider_keys(),
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
        let mut kept = names.clone();
        kept.push(notes().kind);
        assert_eq!(
            read, kept,
            "a kind that is kept and cannot be read, or the reverse"
        );
        let rendered = rendering(&defaults()).expect("the rendering");
        let named: Vec<&str> = rendered
            .split(' ')
            .map(|item| item.split('/').next().unwrap_or_default())
            .collect();
        assert_eq!(named, kept, "{rendered:?}");
    }

    #[test]
    fn the_kind_bilbo_writes_is_read_and_listed_apart_and_claims_no_file() {
        let written = notes();
        let last = readers(&defaults())
            .expect("the readers")
            .last()
            .map(|reader| reader.kind());
        assert_eq!(last, Some(written.kind));
        assert!(apart().contains(&written.kind));
        let claimed: Vec<&str> = kinds(&defaults())
            .expect("the kinds")
            .iter()
            .map(|kind| kind.name())
            .collect();
        assert!(!claimed.contains(&written.kind));
        let source = bilbo_derive::tries::Extent {
            parent: bilbo_store::id::Id::from_bytes([4; 16]),
            generation: bilbo_store::entries::Generation(2),
            first: 3,
            last: 9,
        };
        let paragraph = bilbo_derive::view::Paragraph {
            text: "a paragraph".to_owned(),
            at: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(86_400)),
            source,
        };
        let line = (written.line)(&paragraph);
        assert!(!line.ends_with(b"\n"));
        let back = (written.paragraph)(&line).expect("a paragraph");
        assert_eq!(
            (back.text, back.at, back.source),
            (paragraph.text, paragraph.at, source)
        );
        let one = bilbo_entries::note::record::Source {
            entry: source.parent,
            generation: 1,
            first: 1,
            last: 1,
        };
        for from in [vec![one.clone(), one], Vec::new()] {
            let record = bilbo_entries::note::record::Record {
                text: "a note".to_owned(),
                at: None,
                from,
            };
            assert!((written.paragraph)(&(written.write)(&record)).is_none());
        }
    }

    #[test]
    fn the_rendering_names_each_reader_its_version_and_its_settings() {
        assert_eq!(
            rendering(&defaults()).expect("the rendering"),
            "markdown/1/heading_width=100 session/3 note/1"
        );
        let wide = crate::console::settings::load(
            &|name| match name {
                "HOME" => Some(OsString::from("/h")),
                "BILBO_MARKDOWN_HEADING_WIDTH" => Some(OsString::from("40")),
                _ => None,
            },
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &super::kind_keys(),
            &super::provider_keys(),
        )
        .expect("settings");
        assert_eq!(
            rendering(&wide).expect("the rendering"),
            "markdown/1/heading_width=40 session/3 note/1"
        );
    }

    #[test]
    fn a_rendering_whose_key_was_not_registered_is_a_config_error() {
        let settings = crate::console::settings::load(
            &|name| (name == "HOME").then(|| OsString::from("/h")),
            &|_| Err(std::io::ErrorKind::NotFound.into()),
            &[],
            &super::provider_keys(),
        )
        .expect("settings with no kind key");
        assert!(matches!(rendering(&settings), Err(Failure::Config(_))));
    }

    #[test]
    fn the_remember_page_lists_every_kind() {
        let page = stdout(&bilbo(), &["bilbo", "remember", "--help"]);
        let names: Vec<&str> = listed().iter().map(|kind| kind.name()).collect();
        let line = format!("\nKinds: {}\n", names.join(", "));
        assert!(page.contains(&line), "{line:?} missing in\n{page}");
    }

    #[test]
    fn a_bare_remember_is_a_usage_error_that_names_refresh() {
        let failure = run(
            &args(&["bilbo", "remember"]),
            &mut Env::quiet(),
            Ok(defaults()),
        )
        .expect_err("a usage error");
        let Failure::Usage {
            reason,
            usage,
            page,
        } = &failure
        else {
            panic!("{failure:?}");
        };
        assert_eq!(
            reason,
            "missing <file|folder>, - or --edit; bilbo refresh keeps every follow"
        );
        assert_eq!(
            usage,
            &[
                "usage: bilbo remember <file|folder> [--kind <kind>] [--tag <tag>]...",
                "       [--follow]",
                "       bilbo remember - [--tag <tag>]...",
                "       bilbo remember --edit [--tag <tag>]...",
                "       bilbo remember"
            ]
        );
        assert_eq!(page, "bilbo remember --help");
        assert_eq!(failure.exit_code(), 2);
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
            "unknown verb 'a\\nb'; the verbs are remember, refresh, recall, digest, read, tag, forget, reindex, derive, config"
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
            &super::kind_keys(),
            &super::provider_keys(),
        )
        .expect("settings")
    }

    #[test]
    fn every_kind_a_reader_reads_has_a_list_with_its_word_and_its_two_keys() {
        let settings = configured(
            "digest.markdown.most = 1\ndigest.markdown.floor = 2.5\ndigest.session.most = 3\n\
             digest.session.floor = 4\ndigest.session.old_after = \"5d\"\n\
             digest.session.subagents = true\ndigest.session.parts = false\ndigest.note.most = 7\n\
             digest.note.floor = 9\ndigest.note.width = 50\n",
        );
        let readers = readers(&settings).expect("the readers");
        let ways = super::note_ways(50);
        let shelves = super::lists(&settings, &readers, &*ways.line).expect("the lists");
        let kinds: Vec<&str> = readers.iter().map(|reader| reader.kind()).collect();
        let listed: Vec<&str> = shelves
            .iter()
            .map(|shelf| shelf.list.kind.as_str())
            .collect();
        assert_eq!(listed, kinds);
        let words: Vec<&str> = shelves.iter().map(|shelf| shelf.word).collect();
        assert!(words.iter().all(|word| !word.is_empty()));
        let mut apart = words.clone();
        apart.sort_unstable();
        apart.dedup();
        assert_eq!(apart.len(), words.len());
        let registered: Vec<&str> =
            crate::console::settings::keys(&super::kind_keys(), &super::provider_keys())
                .iter()
                .map(|about| about.name)
                .collect();
        for kind in &kinds {
            for key in ["most", "floor"] {
                let name = format!("digest.{kind}.{key}");
                assert!(registered.contains(&name.as_str()), "{name}");
            }
        }
        let by = |at: usize| &shelves[at].list;
        assert_eq!((by(0).most, by(1).most, by(2).most), (1, 3, 7));
        assert!((by(0).floor - 2.5).abs() < f64::EPSILON);
        assert!((by(1).floor - 4.0).abs() < f64::EPSILON);
        assert!((by(2).floor - 9.0).abs() < f64::EPSILON);
        assert_eq!(by(1).old_after, Some(5 * bilbo_settings::duration::DAY));
        assert_eq!((by(0).old_after, by(2).old_after), (None, None));
        assert_eq!((by(1).lists_belonging, by(1).lists_parts), (true, false));
        assert_eq!((by(0).lists_belonging, by(0).lists_parts), (true, true));
        assert_eq!((by(2).lists_belonging, by(2).lists_parts), (true, true));
        assert!(by(0).line.is_none() && by(1).line.is_none());
        let line = by(2).line.expect("the notes' line");
        let note = bilbo_entries::note::record::write(&bilbo_entries::note::record::Record {
            text: "x".repeat(60),
            at: None,
            from: Vec::new(),
        });
        assert_eq!(line(&note).map(|text| text.chars().count()), Some(50));
        let crossed = configured(
            "digest.session.old_after = \"5d\"\ndigest.session.subagents = false\n\
             digest.session.parts = true\n",
        );
        let ways = super::note_ways(crossed.digest.note_width.value);
        let shelves = super::lists(&crossed, &readers, &*ways.line).expect("the lists");
        let by = |at: usize| &shelves[at].list;
        assert_eq!((by(1).lists_belonging, by(1).lists_parts), (false, true));
        assert_eq!((by(0).lists_belonging, by(0).lists_parts), (true, true));
        assert_eq!((by(2).lists_belonging, by(2).lists_parts), (true, true));
        let wider = configured("digest.note.width = 80\n");
        let ways = super::note_ways(wider.digest.note_width.value);
        let shelves = super::lists(&wider, &readers, &*ways.line).expect("the lists");
        let line = shelves[2].list.line.expect("the notes' line");
        assert_eq!(line(&note).map(|text| text.chars().count()), Some(60));
    }

    #[test]
    fn no_reader_reads_the_summarys_kind_and_its_record_is_no_note() {
        use bilbo_derive::summary::{KIND, record};
        use bilbo_store::entries::Generation;
        let settings = defaults();
        for reader in readers(&settings).expect("the readers") {
            assert_ne!(reader.kind(), KIND);
        }
        for kind in listed() {
            assert_ne!(kind.name(), KIND);
        }
        assert_ne!(notes().kind, KIND);
        let summary = record::Summary {
            text: "Thinning the carrots of the north bed: 142 seedlings counted in four rows."
                .to_owned(),
            instruction: "7fdf9f0d93f443e3".to_owned(),
            provider: "anthropic".to_owned(),
            model: "claude-haiku-5-5".to_owned(),
            answered: Some("claude-haiku-5-5".to_owned()),
            made: 1_794_107_564,
            entry: bilbo_store::id::Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("an id"),
            generation: Generation(1),
            records: 23,
            content: 0xabcd,
            notes: Some(record::Notes {
                generation: Generation(1),
                records: 8,
                digest: 0xff,
            }),
            sent: 412,
            tokens_in: Some(790),
            tokens_out: Some(61),
        };
        let bytes = format!("{}\n", record::write(&summary)).into_bytes();
        assert!((notes().paragraph)(&bytes).is_none());
        assert!(bilbo_derive::record::read(&bytes).is_none());
        assert_eq!(
            bilbo_store::opaque::filter(&bytes).as_ref(),
            bytes.as_slice()
        );
    }

    fn summary_record(text: &str) -> Vec<u8> {
        use bilbo_derive::summary::record;
        use bilbo_store::entries::Generation;
        let summary = record::Summary {
            text: text.to_owned(),
            instruction: "7fdf9f0d93f443e3".to_owned(),
            provider: "anthropic".to_owned(),
            model: "claude-haiku-5-5".to_owned(),
            answered: None,
            made: 1_794_107_564,
            entry: bilbo_store::id::Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("an id"),
            generation: Generation(1),
            records: 23,
            content: 0xabcd,
            notes: None,
            sent: 412,
            tokens_in: None,
            tokens_out: None,
        };
        format!("{}\n", record::write(&summary)).into_bytes()
    }

    #[test]
    fn the_way_to_a_summary_is_derives_label_and_its_records_text_whole() {
        const EXAMPLE: &str = r#"{"summary":{"text":"Thinning the carrots of the north bed: 142 seedlings counted in four rows.","instruction":"7fdf9f0d93f443e3","by":{"provider":"anthropic","model":"claude-haiku-5-5"},"answered":"claude-haiku-5-5","made":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":1,"records":23,"content":"0000000000000000000000000000abcd","notes":{"generation":1,"records":8,"digest":"00000000000000ff"}},"sent":412,"in":790,"out":61}}"#;
        const RUN: &str = r#"{"run":{"kind":"session","instruction":"9f86d081884c7d65","by":{"provider":"anthropic","model":"claude-haiku-5-5"},"answered":"claude-haiku-5-5","same":true,"made":1794107564,"of":{"entry":"01M6ZGE4KTBYYH07AS2KVMSCAF","generation":1,"records":[1,811],"total":1204},"sent":19874,"in":29411,"out":2210,"written":14,"kept":12,"range":1,"names":1,"passes":0,"passed":false}}"#;
        let readers = readers(&defaults()).expect("the readers");
        let ways = super::note_ways(300);
        let id = bilbo_store::id::Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("an id");
        let settings = defaults();
        let shown = super::summary_shown(&settings).expect("a reader while the switch is true");
        let (_, way) = super::hands(&settings, &readers, &ways, Some(&shown)).expect("the hands");
        let summary = way.summary.expect("a way to a summary");
        assert_eq!((summary.label)(id), bilbo_derive::summary::label(id));
        assert_eq!((summary.label)(id).as_str(), format!("summary:{id}"));
        let text = |bytes: &[u8]| (summary.text)(bytes);
        assert_eq!(
            text(&summary_record("Thinning the carrots.")),
            Some("Thinning the carrots.".to_owned())
        );
        assert_eq!(
            text(&summary_record("one  two\tthree")),
            Some("one two three".to_owned())
        );
        let note = bilbo_entries::note::record::write(&bilbo_entries::note::record::Record {
            text: "a note".to_owned(),
            at: None,
            from: Vec::new(),
        });
        assert_eq!(text(&note), None);
        assert_eq!(
            text(EXAMPLE.as_bytes()).as_deref(),
            Some("Thinning the carrots of the north bed: 142 seedlings counted in four rows.")
        );
        assert_eq!(text(RUN.as_bytes()), None);
        assert_eq!(text(b"not a record"), None);
        let long = "a".repeat(1001);
        assert_eq!(text(&summary_record(&long)), Some(long));
        assert_eq!(
            text(&summary_record("one\n two\tthree")),
            Some("one two three".to_owned())
        );
        let off = configured("digest.summary.enabled = false\n");
        assert!(super::summary_shown(&off).is_none());
        let (_, way) = super::hands(&off, &readers, &ways, None).expect("the hands");
        assert!(way.summary.is_none());
    }

    #[test]
    fn the_way_to_a_passages_note_is_derives_label_and_what_a_notes_record_says_and_shows() {
        use bilbo_entries::note::record::{Record, Source, write};
        use bilbo_search::digest::Written;
        use bilbo_store::entries::{Generation, RecordRange};
        let id = bilbo_store::id::Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("an id");
        let settings = defaults();
        let readers = readers(&settings).expect("the readers");
        let ways = super::note_ways(300);
        let (shelves, way) = super::hands(&settings, &readers, &ways, None).expect("the hands");
        let notes_line = shelves[2].list.line.expect("the notes' line");
        let label = (way.label)(id);
        assert_eq!(label, bilbo_derive::plan::label(id));
        assert_eq!(label.as_str(), format!("derive:{id}"));

        let source = |generation: u32, first: u64, last: u64| bilbo_search::digest::Source {
            entry: id,
            generation: Generation(generation),
            records: RecordRange {
                first,
                count: last - first + 1,
            },
        };
        let note = |from: Vec<bilbo_search::digest::Source>, text: &str| {
            Some(Written {
                from,
                text: text.to_owned(),
            })
        };
        let derived = include_str!("../../../libs/entries/rust/fixtures/note/derived.jsonl");
        let wanted = [
            note(
                vec![source(1, 3, 19)],
                "The user asked to thin the carrot rows in the north bed and was told to wait for the second leaves.",
            ),
            note(
                vec![source(1, 20, 31), source(1, 40, 41)],
                "The user planted the beans along the south fence and noted that the soil there stayed wet. Still open: whether to add a second row.",
            ),
            note(
                vec![source(1, 42, 42)],
                "Nothing was decided about the compost bin; the question is still unresolved.",
            ),
            None,
            note(
                vec![source(1, 50, 60)],
                "The tomato cages were moved to the greenhouse.",
            ),
            note(
                vec![source(2, 61, 70)],
                "The watering schedule was skipped for a week because of the rain.",
            ),
            None,
        ];
        let lines: Vec<&str> = derived.lines().collect();
        assert_eq!(lines.len(), wanted.len());
        for (line, wanted) in lines.iter().zip(&wanted) {
            assert_eq!((way.written)(line.as_bytes()), *wanted, "{line}");
            assert_eq!(
                notes_line(line.as_bytes()),
                wanted.as_ref().map(|note| note.text.clone()),
                "{line}"
            );
        }
        let by_hand = include_str!("../../../libs/entries/rust/fixtures/note/by-hand.jsonl");
        let texts = [
            "The beet seeds are in the drawer by the door.",
            "The hose needs a new washer; ordered on Monday.",
            "The fence is mended.",
        ];
        assert_eq!(by_hand.lines().count(), texts.len());
        for (line, text) in by_hand.lines().zip(texts) {
            assert_eq!((way.written)(line.as_bytes()), note(Vec::new(), text));
            assert_eq!(notes_line(line.as_bytes()).as_deref(), Some(text));
        }
        for bytes in [&b"not a record"[..], b""] {
            assert_eq!((way.written)(bytes), None);
            assert_eq!(notes_line(bytes), None);
        }

        let of = |text: &str| {
            write(&Record {
                text: text.to_owned(),
                at: None,
                from: vec![Source {
                    entry: id,
                    generation: 1,
                    first: 1,
                    last: 1,
                }],
            })
        };
        let spaced = (way.written)(&of("  one \t two\n\n   three  ")).expect("a note");
        assert_eq!(spaced.text, "one two three");
        let narrow_settings = configured("digest.note.width = 40\n");
        let narrow = super::note_ways(narrow_settings.digest.note_width.value);
        let (narrow_shelves, narrow_way) =
            super::hands(&narrow_settings, &readers, &narrow, None).expect("the hands");
        let narrow_line = narrow_shelves[2].list.line.expect("the notes' line");
        let exact = "a".repeat(40);
        let over = "a".repeat(41);
        let cut = format!("{}\u{2026}", "a".repeat(39));
        assert_eq!(narrow_line(&of(&exact)).as_deref(), Some(exact.as_str()));
        assert_eq!(
            (narrow_way.written)(&of(&exact)).map(|note| note.text),
            Some(exact.clone())
        );
        assert_eq!(narrow_line(&of(&over)).as_deref(), Some(cut.as_str()));
        assert_eq!(
            (narrow_way.written)(&of(&over)).map(|note| note.text),
            Some(cut)
        );
    }

    #[test]
    fn every_run_whose_verb_word_is_digest_exits_0_and_no_other_does() {
        let all = |list: &[&str], settings: Result<Settings, Failure>| {
            let args: Vec<OsString> = list.iter().map(OsString::from).collect();
            run(&args, &mut Env::quiet(), settings).expect("an outcome")
        };
        for list in [
            &["bilbo", "digest", "--help"][..],
            &["bilbo", "digest", "-h"],
            &["bilbo", "digest", "x"],
            &["bilbo", "digest", "--all"],
        ] {
            let outcome = all(list, Ok(defaults()));
            assert_eq!(outcome.exit_code(), 0, "{list:?}");
            assert_eq!(outcome.write_failure_code(), 0, "{list:?}");
        }
        let unloaded = all(
            &["bilbo", "digest"],
            Err(Failure::Config("the settings did not load".to_owned())),
        );
        assert_eq!(unloaded.exit_code(), 0);
        assert_eq!(unloaded.write_failure_code(), 0);
        assert_eq!(
            unloaded.stderr_lines(&View::PLAIN),
            ["bilbo: the settings did not load"]
        );
        for list in [
            &["bilbo", "help", "digest"][..],
            &["bilbo", "--help", "digest"],
            &["bilbo", "--version"],
            &["bilbo", "recall", "--help"],
        ] {
            let outcome = all(list, Ok(defaults()));
            assert_eq!(outcome.write_failure_code(), 1, "{list:?}");
        }
    }
}
