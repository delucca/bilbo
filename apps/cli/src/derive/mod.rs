pub mod disk;
pub mod scope;
pub mod view;

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use bilbo_derive::instruction::{self, Instruction};
use bilbo_derive::keys::{CALLS, Section};
use bilbo_derive::plan::{self, Asked, Plan, Ports, Rules, State, SummaryState};
use bilbo_derive::run::{self, Clock, Context, Ended, Event, Sinks, Which};
use bilbo_derive::scope::Untrusted;
use bilbo_derive::spool::bare;
use bilbo_derive::summary;
use bilbo_derive::tags;
use bilbo_derive::tries::{Extent, Tries};
use bilbo_derive::view::{Counter, Format};
use bilbo_entries::read::refs::{Ref, Target};
use bilbo_entries::read::{self, Error as ReadError};
use bilbo_entries::reader::Reader;
use bilbo_llm::model::Model;
use bilbo_search::chunk::{MAX_ENCODE_BYTES, Size};
use bilbo_search::lookup::Lookup;
use bilbo_search::position::Manifest;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_settings::duration;
use bilbo_settings::file::File;
use bilbo_settings::key::{Flag, Source};
use bilbo_store::entries::{Cursor, Entries, RecordRange, StoreError};
use bilbo_store::id::Id;
use bilbo_store::segments::{Cache, Notice, Store};
use clap::Command;
use clap::builder::NonEmptyStringValueParser;

use crate::console::args::usage;
use crate::console::closing::Closing;
use crate::console::env::{Env, Var};
use crate::console::message::{Message, notice_message, sanitize, unnamed_value};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::settings::Settings;
use crate::console::theme::Mark;
use disk::{Blocks, Failed, Folder, Held, Summaries, Tagged};
use scope::Labelled;
use view::Noted;

const SUMMARY: &str = "Write notes about entries that stopped growing, with a model";
const ABOUT: &str = "bilbo derive: write notes about entries that stopped growing, with a model.";
const USAGE: &str = "bilbo derive [--dry-run] [--redo] [--calls <n>] [<entry>...]";
const CALLS_FLAG: &str = "--calls";

#[derive(Debug, clap::Args)]
#[command(about = SUMMARY, before_help = ABOUT, override_usage = USAGE)]
pub struct Args {
    #[arg(long, help = "Say what a run would send, and send nothing")]
    pub dry_run: bool,
    #[arg(long, help = "Redo what other settings wrote and what was passed over")]
    pub redo: bool,
    #[arg(
        long,
        value_name = "n",
        value_parser = NonEmptyStringValueParser::new(),
        help = "Make at most this many calls"
    )]
    pub calls: Option<String>,
    #[arg(value_name = "entry", value_parser = NonEmptyStringValueParser::new())]
    pub entries: Vec<String>,
}

pub fn page(kinds: &[&str], providers: &[(&str, &str)]) -> String {
    let kind_width = kinds
        .iter()
        .map(|kind| kind.len())
        .max()
        .unwrap_or_default();
    let kind_rows: String = kinds
        .iter()
        .map(|kind| format!("  {kind:<kind_width$}  bilbo ships an instruction for it\n"))
        .collect();
    let width = providers
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or_default();
    let provider_rows: String = providers
        .iter()
        .map(|(name, line)| format!("  {name:<width$}  {line}\n"))
        .collect();
    format!("Kinds:\n{kind_rows}Providers:\n{provider_rows}\n{PAGE}")
}

const PAGE: &str = "\
Output:
  entries <n>, calls <n>, notes <n>, dropped <n>, refused <n>, failed <n>
  tokens in <n>, tokens out <n>, seconds <n>

Exit:
  0  done, also when derive is off, nothing is due or another derive runs
  1  a failed call, a refused entry, a busy or stopped provider, a store failure
  2  usage or config error

Examples:
  bilbo derive --dry-run

Docs: https://github.com/delucca/bilbo/wiki/Commands#derive";

pub struct Built {
    pub writer: Box<dyn Model>,
    pub api: bool,
}

pub type Named<'a> = Option<(&'static str, &'a str)>;

pub type Finish<'a> = dyn Fn(Store, u64, &mut dyn FnMut(&Message)) -> Closing + 'a;

pub struct Hand<'a> {
    pub readers: &'a [Box<dyn Reader>],
    pub tokenizer: &'a dyn Fn() -> Result<Tokenizer, Failure>,
    pub lookup: &'a dyn Fn(&Path) -> Box<dyn Lookup>,
    pub chunk: Size,
    pub render: &'a str,
    pub format: Format<'a>,
    pub writer: &'a dyn Fn(&Var, Named<'_>) -> Result<Built, Failure>,
    pub finish: &'a Finish<'a>,
}

struct Wall;

impl Clock for Wall {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep(&self, wait: Duration) {
        thread::sleep(wait);
    }
}

struct Job<'a> {
    settings: &'a Settings,
    hand: &'a Hand<'a>,
    section: &'a Section,
    instructions: &'a [Instruction],
    tags: Option<&'a tags::instruction::Instruction>,
    summary: Option<&'a summary::instruction::Instruction>,
    redo: bool,
    dry: bool,
}

struct Given {
    text: String,
    target: Target,
}

#[derive(Default)]
struct Looked {
    plan: Plan,
    ids: Vec<(String, Id)>,
    problems: bool,
    charges: Vec<(Extent, u8)>,
    noted: Vec<Noted>,
    kinds: Vec<(Id, String)>,
    held_tags: Vec<(Id, Vec<String>)>,
}

fn tags_instruction(
    settings: &Settings,
) -> Result<Option<tags::instruction::Instruction>, Failure> {
    if !settings.tags.enabled.value || settings.tags.pick.value.is_empty() {
        return Ok(None);
    }
    tags::instruction::in_force(settings.tags.instruction.value.as_deref(), &|path| {
        fs::read(path)
    })
    .map(Some)
    .map_err(Failure::Config)
}

fn summary_instruction(
    settings: &Settings,
) -> Result<Option<summary::instruction::Instruction>, Failure> {
    if !settings.summary.enabled.value {
        return Ok(None);
    }
    summary::instruction::in_force(settings.summary.instruction.value.as_deref(), &|path| {
        fs::read(path)
    })
    .map(Some)
    .map_err(Failure::Config)
}

fn summary_rules<'a>(
    settings: &'a Settings,
    instruction: Option<&'a summary::instruction::Instruction>,
) -> Option<summary::Rules<'a>> {
    instruction.map(|held| summary::Rules {
        section: &settings.summary,
        instruction: &held.text,
        id: &held.id,
    })
}

fn tags_rules<'a>(
    settings: &'a Settings,
    instruction: Option<&'a tags::instruction::Instruction>,
    model: &'a str,
) -> Option<tags::Rules<'a>> {
    instruction.map(|held| tags::Rules {
        section: &settings.tags,
        instruction: &held.text,
        id: &held.id,
        model,
        cap: settings.tag_cap.value,
    })
}

#[derive(Default)]
struct Said {
    lines: Vec<Message>,
    plain: String,
}

fn refs(root: &Command, texts: &[String]) -> Result<Vec<Given>, Failure> {
    texts
        .iter()
        .map(|text| {
            let parsed = Ref::parse(text).map_err(|malformed| {
                usage(root, sanitize(&malformed.to_string()), Some("derive"))
            })?;
            if parsed.locator.generation.is_some() || parsed.locator.records.is_some() {
                let bare = text.split('#').next().unwrap_or(text);
                return Err(usage(
                    root,
                    sanitize(&format!(
                        "{text} names records; bilbo derive takes an entry: {bare}"
                    )),
                    Some("derive"),
                ));
            }
            Ok(Given {
                text: text.clone(),
                target: parsed.target,
            })
        })
        .collect()
}

fn refused(error: &dyn std::fmt::Display) -> Failure {
    Failure::Refused(sanitize(&error.to_string()))
}

pub fn run(
    root: &Command,
    args: &Args,
    env: &mut Env,
    settings: &Settings,
    hand: &Hand<'_>,
) -> Result<Outcome, Failure> {
    let started = Instant::now();
    let given = refs(root, &args.entries)?;
    let none = File::default();
    let flag = args
        .calls
        .as_deref()
        .map(|text| {
            Flag::parse(
                &CALLS,
                &Source {
                    var: &*env.var,
                    file: &none,
                },
                CALLS_FLAG,
                text,
            )
        })
        .transpose()?;
    let mut section = settings.derive.clone();
    section.calls = section.calls.with_flag(flag);
    if !args.dry_run
        && let Err(text) = section.gate(&settings.llm.provider.value)
    {
        return Ok(Outcome::new("")
            .with_diagnostics(vec![Message::info(&format!("derive is off: {text}"))]));
    }
    let shipped = instruction::shipped();
    let notes_kind = hand.format.kind;
    let kinds: Vec<&str> = hand
        .readers
        .iter()
        .map(|reader| reader.kind())
        .filter(|kind| *kind != notes_kind)
        .collect();
    let instructions = instruction::in_force(
        &shipped,
        &section.instructions.value,
        &|path| fs::read(path),
        &kinds,
    )
    .map_err(Failure::Config)?;
    let in_force = tags_instruction(settings)?;
    let summary_in_force = summary_instruction(settings)?;
    let job = Job {
        settings,
        hand,
        section: &section,
        instructions: &instructions,
        tags: in_force.as_ref(),
        summary: summary_in_force.as_ref(),
        redo: args.redo,
        dry: args.dry_run,
    };
    if args.dry_run {
        let built = if settings.llm.provider.value.is_empty() {
            None
        } else {
            Some((hand.writer)(&env.var, None)?)
        };
        return dry(env, &job, built.as_ref(), &given);
    }
    let mut built = (hand.writer)(&env.var, None)?;
    let (state, state_folder) = state_of(env, settings)?;
    let folder = Folder::at(&state, state_folder);
    let held = match folder.lock() {
        Ok(Some(held)) => held,
        Ok(None) => {
            return Ok(Outcome::new("")
                .with_diagnostics(vec![Message::info("another bilbo derive is running")]));
        }
        Err(error) => return Err(refused(&error)),
    };
    let mut said = Said::default();
    match go(env, &job, &mut built, &held, &given, &mut said, started) {
        Ok(outcome) => Ok(outcome),
        Err(failure) => Ok(Outcome::stopped(said.lines, &failure, said.plain)),
    }
}

fn state_of(env: &mut Env, settings: &Settings) -> Result<(PathBuf, fs::File), Failure> {
    let notice = &mut env.notice;
    let store = Store::open_read_only(&settings.layout(), settings.store.options(), &mut |said| {
        if said == Notice::Waiting {
            notice(&notice_message(&Notice::Waiting));
        }
    })?;
    state_folder(&store)
}

fn state_folder(store: &Store) -> Result<(PathBuf, fs::File), Failure> {
    let state = store.state().ok_or_else(|| {
        Failure::Refused("this store has no state folder that bilbo checked".to_owned())
    })?;
    let folder = state
        .folder()
        .try_clone()
        .map_err(|error| refused(&error))?;
    Ok((state.path().to_owned(), folder))
}

fn go(
    env: &mut Env,
    job: &Job<'_>,
    built: &mut Built,
    held: &Held,
    given: &[Given],
    said: &mut Said,
    started: Instant,
) -> Result<Outcome, Failure> {
    let Job {
        settings,
        hand,
        section,
        ..
    } = *job;
    let mut failed = false;
    let mut spool = held.blocks().map_err(|error| refused(&error))?;
    let mut summary_spool = held.summary_blocks().map_err(|error| refused(&error))?;
    let mut asked = held.tagged();
    let mut summarised = held.summaries();
    replay(
        env,
        settings,
        hand,
        Spools {
            spool: &mut spool,
            summary_spool: &mut summary_spool,
            asked: &mut asked,
            summarised: &mut summarised,
        },
        said,
        &mut failed,
    )?;
    let tokenizer = (hand.tokenizer)()?;
    let model = built.writer.id().to_owned();
    let mut tries = held.failed().map_err(|error| refused(&error))?;
    let tags_model = model_that_tags(settings, job.tags, &model);
    let mut tagger = if tags_model == model {
        None
    } else {
        Some((hand.writer)(
            &env.var,
            Some((tags::keys::MODEL.about.name, &tags_model)),
        )?)
    };
    let looked = look(
        env,
        job,
        &tokenizer,
        (&model, &tags_model),
        given,
        Some((&tries, &asked, &summarised)),
        &mut said.lines,
    )?;
    let mut exit = if looked.problems || failed {
        Exit::Refused
    } else {
        Exit::Success
    };
    if let Some(why) = looked.plan.untrusted {
        said.lines.push(Message::error(untrusted(why)));
        return Ok(Outcome::new("")
            .with_diagnostics(std::mem::take(&mut said.lines))
            .with_exit(Exit::Refused));
    }
    for (text, id) in &looked.ids {
        let state = looked
            .plan
            .states
            .iter()
            .find(|(held, _)| held == id)
            .map(|(_, state)| *state);
        let kind = looked
            .kinds
            .iter()
            .find(|(held, _)| held == id)
            .map_or("", |(_, kind)| kind.as_str());
        let summary_state = job.summary.and_then(|_| {
            looked
                .plan
                .summaries
                .iter()
                .find(|(held, _)| held == id)
                .map(|(_, state)| *state)
        });
        if let Some(text) = state.and_then(|state| not_sent(text, state, kind, summary_state)) {
            said.lines.push(Message::warning(&text));
            exit = Exit::Refused;
        }
    }
    let windowed: Vec<&bilbo_derive::plan::Due> = looked
        .plan
        .due
        .iter()
        .filter(|due| !due.windows.is_empty())
        .collect();
    tries
        .keep_only(&windowed)
        .map_err(|error| refused(&error))?;
    let mut log = held.log().map_err(|error| refused(&error))?;
    let counter = Counter {
        tokens: &*tokenizer.tokens,
        bound: MAX_ENCODE_BYTES,
    };
    let report = run::calls(
        &looked.plan.due,
        Sinks {
            writer: &mut *built.writer,
            tagger: tagger
                .as_mut()
                .map(|built| -> &mut dyn Model { &mut *built.writer }),
            asked: &mut asked,
            spool: &mut spool,
            summary_spool: &mut summary_spool,
            summarised: &mut summarised,
            tries: &mut tries,
            log: &mut log,
            clock: &Wall,
        },
        &Context {
            section,
            provider: &settings.llm.provider.value,
            limits: settings.llm.limits(section.answer_tokens.value),
            format: &hand.format,
            counter: &counter,
            tags: tags_rules(settings, job.tags, &tags_model),
            summary: summary_rules(settings, job.summary),
            redo: job.redo,
        },
    )
    .map_err(|halted| {
        said.lines.extend(
            halted
                .events
                .iter()
                .map(|event| event_line(event, built.api, &looked.charges)),
        );
        refused(&halted)
    })?;
    let prepare = match &report.ended {
        Ended::Prepare(text, _) => Some(sanitize(text)),
        _ => None,
    };
    if let Some(text) = &prepare
        && report.counts.calls == 0
        && report.events.is_empty()
    {
        return Err(Failure::Config(text.clone()));
    }
    let totals = report.counts;
    let seconds = started.elapsed().as_secs();
    said.plain = view::plain(&totals, seconds);
    let mut tail: Vec<Message> = report
        .events
        .iter()
        .map(|event| event_line(event, built.api, &looked.charges))
        .collect();
    let why = built.writer.why().map(str::to_owned);
    let tagger_why = tagger
        .as_ref()
        .and_then(|built| built.writer.why().map(str::to_owned));
    let whys = |which: Which| match which {
        Which::Writer => why.clone(),
        Which::Tagger => tagger_why.clone(),
    };
    let keys = tagger.as_ref().map(|_| {
        (
            format!("llm.{}.model", settings.llm.provider.value),
            tags::keys::MODEL.about.name.to_owned(),
        )
    });
    tail.extend(ended_line(
        &report.ended,
        built.api,
        section,
        &whys,
        keys.as_ref(),
    ));
    tail.extend(
        why_lines(&report.ended, why.as_deref(), tagger_why.as_deref())
            .iter()
            .map(|text| Message::warning(text)),
    );
    if let Some(error) = log.error() {
        tail.push(Message::warning(&format!(
            "the calls were not logged: {error}"
        )));
    }
    let listed = spool
        .is_empty()
        .and_then(|plain| Ok(plain && summary_spool.is_empty()?));
    let closed = finish(listed, tail, said, |said| {
        replay(
            env,
            settings,
            hand,
            Spools {
                spool: &mut spool,
                summary_spool: &mut summary_spool,
                asked: &mut asked,
                summarised: &mut summarised,
            },
            said,
            &mut failed,
        )
    });
    if let Some(text) = prepare {
        if let Err(failure) = closed {
            said.lines.push(failure.message());
        }
        return Err(Failure::Config(text));
    }
    closed?;
    let hard = matches!(
        report.ended,
        Ended::Failures(_) | Ended::Busy(..) | Ended::Wait(..) | Ended::Stopped(..)
    );
    if report.counts.failed > 0 || hard || failed {
        exit = Exit::Refused;
    }
    Ok(Outcome::new(view::plain(&totals, seconds))
        .with_human(move |out| view::human(&totals, seconds, out))
        .with_diagnostics(std::mem::take(&mut said.lines))
        .with_exit(exit))
}

fn finish(
    empty: io::Result<bool>,
    tail: Vec<Message>,
    said: &mut Said,
    replay: impl FnOnce(&mut Said) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let after = match empty {
        Ok(true) => Ok(()),
        Ok(false) => replay(said),
        Err(error) => Err(refused(&error)),
    };
    said.lines.extend(tail);
    after
}

struct Spools<'a> {
    spool: &'a mut Blocks,
    summary_spool: &'a mut Blocks,
    asked: &'a mut Tagged<'static>,
    summarised: &'a mut Summaries<'static>,
}

fn replay(
    env: &mut Env,
    settings: &Settings,
    hand: &Hand<'_>,
    spools: Spools<'_>,
    said: &mut Said,
    failed: &mut bool,
) -> Result<(), Failure> {
    let Spools {
        spool,
        summary_spool,
        asked,
        summarised,
    } = spools;
    spool.clear_parts().map_err(|error| refused(&error))?;
    if spool.is_empty().map_err(|error| refused(&error))?
        && summary_spool.is_empty().map_err(|error| refused(&error))?
    {
        return Ok(());
    }
    let layout = settings.layout();
    Store::check(&layout)?;
    let notice = &mut env.notice;
    let mut store = Store::open(&layout, settings.store.options(), &mut |heard| {
        notice(&notice_message(&heard));
    })?;
    let replayed = match run::replay(
        &mut store,
        spool,
        summary_spool,
        &hand.format,
        asked,
        summarised,
        settings.tag_cap.value,
    ) {
        Ok(replayed) => replayed,
        Err(interrupted) => {
            said.lines.extend(
                interrupted
                    .notices
                    .iter()
                    .map(|text| Message::warning(text)),
            );
            if interrupted.written > 0 {
                let closing = (hand.finish)(store, interrupted.written, &mut *env.notice);
                said.lines.extend(closing.messages);
            } else {
                store.release();
            }
            return Err(refused(&interrupted));
        }
    };
    said.lines
        .extend(replayed.notices.iter().map(|text| Message::warning(text)));
    if replayed.tagged > 0 {
        said.lines.push(Message::info(&format!(
            "tagged {} entries",
            replayed.tagged
        )));
    }
    if replayed.summarised > 0 {
        said.lines.push(Message::info(&format!(
            "summarised {} entries",
            replayed.summarised
        )));
    }
    let closing = (hand.finish)(store, replayed.written(), &mut *env.notice);
    *failed |= closing.failed || closing.refused;
    said.lines.extend(closing.messages);
    Ok(())
}

fn look(
    env: &mut Env,
    job: &Job<'_>,
    tokenizer: &Tokenizer,
    (model, tags_model): (&str, &str),
    given: &[Given],
    held: Option<(&Failed, &Tagged, &Summaries)>,
    lines: &mut Vec<Message>,
) -> Result<Looked, Failure> {
    let Job {
        settings,
        hand,
        section,
        instructions,
        tags,
        summary,
        redo,
        dry,
    } = *job;
    let notice = &mut env.notice;
    let store = Store::open_read_only(&settings.layout(), settings.store.options(), &mut |said| {
        if said == Notice::Waiting {
            notice(&notice_message(&Notice::Waiting));
        }
    });
    let store = match store {
        Ok(store) => store,
        Err(StoreError::Missing(_)) if dry => return Ok(Looked::default()),
        Err(error) => return Err(Failure::from(error)),
    };
    let cache = store.reader_cache().clone();
    let mut ids: Vec<(String, Id)> = Vec::new();
    let mut problems = false;
    let mut problem_lines = Vec::new();
    for one in given {
        let found = match &one.target {
            Target::Entry(id) => match store.meta(*id) {
                Ok(Some(_)) => Ok(*id),
                Ok(None) => Err(ReadError::NoEntry(one.target.clone())),
                Err(error) => Err(ReadError::Store(error)),
            },
            target => read::resolve(target, settings.read_matches_listed.value, &store),
        };
        match found {
            Ok(id) => {
                if !ids.iter().any(|(_, held)| *held == id) {
                    ids.push((one.text.clone(), id));
                }
            }
            Err(ReadError::Store(error)) => return Err(Failure::from(error)),
            Err(error) => {
                problems = true;
                problem_lines.push(Message::warning(&error.to_string()));
            }
        }
    }
    let (changes, _) = store.changes(&Cursor::start())?;
    let everything: BTreeSet<Id> = changes
        .iter()
        .map(bilbo_store::entries::Change::id)
        .collect();
    let everything: Vec<Id> = everything.into_iter().collect();
    let position = match &cache {
        Cache::Own(path) => (hand.lookup)(path)
            .position()
            .map_err(|error| Failure::from(&error))?,
        _ => None,
    };
    let labelled = Labelled {
        entries: &store,
        labels: &store,
        position,
        manifest: Manifest::new(hand.chunk, tokenizer.id, hand.render),
    };
    for (key, values, then) in [
        ("derive.in", &section.named.value, "give one id in full"),
        (
            "derive.not_in",
            &section.left_out.value,
            "all of them are left out",
        ),
    ] {
        for (value, why) in labelled.unnamed(values)? {
            lines.push(unnamed_value(key, &value, &why, then));
        }
    }
    lines.extend(problem_lines);
    let own;
    let mut own_asked = None;
    let mut own_summaries = None;
    let (tries, asked_port, summaries_port): (&Failed, &Tagged<'_>, &Summaries<'_>) = match held {
        Some((held, asked, summarised)) => (held, asked, summarised),
        None => {
            let (state, folder) = state_folder(&store)?;
            own = Folder::at(&state, folder)
                .charges()
                .map_err(|error| refused(&error))?;
            let owned = store.state().ok_or_else(|| {
                Failure::Refused("this store has no state folder that bilbo checked".to_owned())
            })?;
            (
                &own,
                &*own_asked.insert(disk::reader(owned.folder(), &state)),
                &*own_summaries.insert(disk::summaries_reader(owned.folder(), &state)),
            )
        }
    };
    let readers: Vec<&dyn Reader> = hand.readers.iter().map(|reader| &**reader).collect();
    let only: Vec<Id> = ids.iter().map(|(_, id)| *id).collect();
    let asked = if given.is_empty() {
        Asked::Every
    } else {
        Asked::Only(&only)
    };
    let plan = plan::plan(
        &Ports {
            entries: &store,
            sealed: &store,
            scope: &labelled,
            tries,
            asked: asked_port,
            summarised: summaries_port,
        },
        &Rules {
            readers: &readers,
            instructions,
            section,
            format: &hand.format,
            tokens: &*tokenizer.tokens,
            bound: MAX_ENCODE_BYTES,
            model,
            provider: &settings.llm.provider.value,
            redo,
            now: SystemTime::now(),
            tags: tags_rules(settings, tags, tags_model),
            summary: summary_rules(settings, summary),
        },
        &everything,
        asked,
    )
    .map_err(|error| refused(&error))?;
    let mut charges = Vec::new();
    for due in &plan.due {
        for planned in due.windows.iter().filter(|planned| planned.pass) {
            let held = tries
                .charges(&planned.extent)
                .map_err(|error| refused(&error))?;
            charges.push((planned.extent, held));
        }
    }
    let mut noted = Vec::new();
    let mut kinds = Vec::new();
    let mut held_tags = Vec::new();
    for id in &only {
        if let Some(meta) = store.meta(*id)? {
            if dry && settings.tags.enabled.value {
                held_tags.push((
                    *id,
                    meta.labels
                        .tags
                        .carried()
                        .filter(|(_, by)| *by == bilbo_store::labels::By::Model)
                        .map(|(tag, _)| tag.as_str().to_owned())
                        .collect(),
                ));
            }
            kinds.push((*id, meta.kind));
        }
        let head = plan::held(&store, *id)?;
        noted.push(match head {
            Some(head) => {
                let range = RecordRange {
                    first: 1,
                    count: head.records,
                };
                let notes = store
                    .read(head.id, head.generation, range)?
                    .iter()
                    .filter(|record| (hand.format.read)(bare(record)).is_some())
                    .count() as u64;
                Noted {
                    entry: *id,
                    note_entry: Some(head.id),
                    notes,
                }
            }
            None => Noted {
                entry: *id,
                note_entry: None,
                notes: 0,
            },
        });
    }
    drop(own_asked);
    drop(store);
    Ok(Looked {
        plan,
        ids,
        problems,
        charges,
        noted,
        kinds,
        held_tags,
    })
}

fn dry(
    env: &mut Env,
    job: &Job<'_>,
    built: Option<&Built>,
    given: &[Given],
) -> Result<Outcome, Failure> {
    let Job {
        settings,
        hand,
        section,
        instructions,
        ..
    } = *job;
    let mut off = Vec::new();
    if !section.enabled.value {
        off.push("derive.enabled is false".to_owned());
    }
    if settings.llm.provider.value.is_empty() {
        off.push("llm.provider is not set".to_owned());
    }
    if section.named.value.is_empty() {
        off.push("derive.in is empty".to_owned());
    }
    let model = built
        .map(|built| built.writer.id().to_owned())
        .unwrap_or_default();
    let tokenizer = (hand.tokenizer)()?;
    let mut diagnostics = Vec::new();
    let tags_model = model_that_tags(settings, job.tags, &model);
    let looked = look(
        env,
        job,
        &tokenizer,
        (&model, &tags_model),
        given,
        None,
        &mut diagnostics,
    )?;
    let mut dry = view::Dry {
        off,
        kinds: instructions
            .iter()
            .map(|held| view::KindRow {
                kind: held.kind.clone(),
                origin: held.origin.clone(),
                instruction: held.id.clone(),
            })
            .collect(),
        limit: section.calls.value,
        provider: built
            .map(|built| built.writer.describe())
            .unwrap_or_default(),
        ..view::Dry::default()
    };
    let exit = if looked.problems {
        Exit::Refused
    } else {
        Exit::Success
    };
    if let Some(why) = looked.plan.untrusted {
        let mut lines = view::lines(&dry);
        lines.truncate(dry.off.len() + 1);
        diagnostics.push(Message::error(untrusted(why)));
        return Ok(Outcome::new(joined(&lines))
            .with_diagnostics(diagnostics)
            .with_exit(Exit::Refused));
    }
    for (_, state) in &looked.plan.states {
        match state {
            State::LeftOut => {
                dry.named += 1;
                dry.left_out += 1;
            }
            State::StillGrowing => {
                dry.named += 1;
                dry.not_due.growing += 1;
            }
            State::Covered => {
                dry.named += 1;
                dry.not_due.covered += 1;
            }
            State::PassedOver => {
                dry.named += 1;
                dry.not_due.passed += 1;
            }
            State::Due => {
                dry.named += 1;
                dry.due += 1;
            }
            State::BelongsToAnother => dry.not_due.belongs += 1,
            State::LabelsNotCurrent => dry.not_due.labels += 1,
            State::NoInstruction | State::NotNamed => {}
        }
    }
    let binds = built.is_none_or(|built| built.writer.bound().is_ok());
    let tags_on = settings.tags.enabled.value;
    let mut tags_dry = view::TagsDry {
        pick: settings.tags.pick.value.len(),
        model: (tags_model != model).then(|| tags_model.clone()),
        instruction: job.tags.map(|held| (held.origin.clone(), held.id.clone())),
        due: looked
            .plan
            .tags
            .iter()
            .filter(|(_, state)| *state == bilbo_derive::plan::TagsState::Due)
            .count() as u64,
        ..view::TagsDry::default()
    };
    let mut summaries_dry = job.summary.map(|held| view::SummariesDry {
        due: looked
            .plan
            .summaries
            .iter()
            .filter(|(_, state)| *state == SummaryState::Due)
            .count() as u64,
        calls: 0,
        tokens: 0,
        instruction: (held.origin.clone(), held.id.clone()),
        lines: Vec::new(),
    });
    let mut left = section.calls.value;
    for due in &looked.plan.due {
        let mut any = false;
        let mut reached = true;
        for planned in due.windows.iter().filter(|planned| !planned.pass) {
            if left == 0 {
                reached = false;
                break;
            }
            left -= 1;
            any = true;
            dry.calls += 1;
            dry.tokens += planned.window.sent as u64;
        }
        if any {
            dry.entries += 1;
        }
        if !reached {
            break;
        }
        if let Some(planned) = &due.tags {
            let windowed = due.windows.iter().any(|planned| !planned.pass);
            let (call, tokens) = if windowed {
                (true, 0)
            } else if planned.state == Some(tags::plan::State::Due) {
                let spent = planned
                    .asking
                    .is_some_and(|(_, charges)| usize::from(charges) >= section.tries.value);
                (!spent, planned.sent as u64)
            } else {
                (false, 0)
            };
            if call {
                if left == 0 {
                    break;
                }
                left -= 1;
                tags_dry.calls += 1;
                tags_dry.tokens += tokens;
            }
        }
        if let (Some(summaries), Some(planned)) = (&mut summaries_dry, &due.summary) {
            let windowed = due.windows.iter().any(|planned| !planned.pass);
            let (call, tokens) = if !binds {
                (false, 0)
            } else if windowed {
                (true, 0)
            } else if let (Some((_, charges)), Some(_), false) =
                (planned.asking, &planned.made, planned.retire)
            {
                (
                    usize::from(charges) < section.tries.value,
                    planned.sent as u64,
                )
            } else {
                (false, 0)
            };
            if call {
                if left == 0 {
                    break;
                }
                left -= 1;
                summaries.calls += 1;
                summaries.tokens += tokens;
            }
        }
    }
    for noted in &looked.noted {
        let state = looked
            .plan
            .states
            .iter()
            .find(|(entry, _)| *entry == noted.entry)
            .map(|(_, state)| *state);
        if let Some(state) = state {
            dry.entry_lines.push(view::entry_line(state, noted));
            if tags_on {
                let shown = looked
                    .plan
                    .tags
                    .iter()
                    .find(|(entry, _)| *entry == noted.entry)
                    .map(|(_, shown)| *shown);
                let held = looked
                    .held_tags
                    .iter()
                    .find(|(entry, _)| *entry == noted.entry)
                    .map_or(&[][..], |(_, held)| held.as_slice());
                let words = if settings.tags.pick.value.is_empty() {
                    view::NONE_TO_PICK.to_owned()
                } else {
                    view::tags_state(shown, state)
                };
                tags_dry
                    .lines
                    .push(view::tags_ref_line(noted.entry, &words, held));
            }
        }
    }
    if let Some(summaries) = &mut summaries_dry {
        for noted in &looked.noted {
            let shown = looked
                .plan
                .summaries
                .iter()
                .find(|(entry, _)| *entry == noted.entry)
                .map(|(_, state)| *state);
            let has_line = looked
                .plan
                .states
                .iter()
                .any(|(entry, _)| *entry == noted.entry);
            if let (true, Some(state)) = (has_line, shown) {
                summaries.lines.push(view::summary_ref_line(
                    noted.entry,
                    &view::summary_state(state),
                ));
            }
        }
    }
    dry.tags = tags_on.then_some(tags_dry);
    dry.summaries = summaries_dry;
    Ok(Outcome::new(joined(&view::lines(&dry)))
        .with_diagnostics(diagnostics)
        .with_exit(exit))
}

fn model_that_tags(
    settings: &Settings,
    instruction: Option<&tags::instruction::Instruction>,
    writer: &str,
) -> String {
    if instruction.is_some() && !settings.tags.model.value.is_empty() {
        settings.tags.model.value.clone()
    } else {
        writer.to_owned()
    }
}

fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn untrusted(why: Untrusted) -> String {
    match why {
        Untrusted::Behind => "the index is behind the store, so labels may be missing; bilbo refresh brings it up to date",
        Untrusted::NoIndex => "there is no search index, so labels cannot be read; bilbo refresh makes one",
        Untrusted::OtherBilbo => "the search index was made by another bilbo, so labels cannot be read; bilbo refresh makes it again",
    }
    .to_owned()
}

fn not_sent(text: &str, state: State, kind: &str, summary: Option<SummaryState>) -> Option<String> {
    let state = match (state, summary) {
        (State::NoInstruction, Some(SummaryState::Out(out))) => out,
        (State::NoInstruction, Some(SummaryState::NoReader) | None) => State::NoInstruction,
        (State::NoInstruction, Some(_)) => return None,
        (state, _) => state,
    };
    match state {
        State::LabelsNotCurrent => Some(format!(
            "{text} has labels that are not current; bilbo refresh brings the index up to date"
        )),
        State::NoInstruction => Some(format!(
            "{text} is of the kind {kind}, which has no instruction"
        )),
        State::BelongsToAnother => Some(format!("{text} belongs to another entry")),
        State::NotNamed => Some(format!("{text} is not named by derive.in")),
        State::LeftOut => Some(format!("{text} is left out by derive.not_in")),
        State::StillGrowing | State::Covered | State::PassedOver | State::Due => None,
    }
}

fn event_line(event: &Event, api: bool, charges: &[(Extent, u8)]) -> Message {
    match event {
        Event::Refused { extent, category } => Message::warning(&format!(
            "refused {}: {}",
            view::place(extent),
            category.as_deref().unwrap_or("unknown")
        )),
        Event::Failed { extent, cause } => Message::warning(&format!(
            "call failed for {}: {}",
            view::place(extent),
            view::what(cause, api)
        )),
        Event::NoNote { extent } => Message::warning(&format!(
            "call failed for {}: {}",
            view::place(extent),
            view::NO_NOTE
        )),
        Event::Passed { extent } => {
            let held = charges
                .iter()
                .find(|(planned, _)| planned == extent)
                .map_or(0, |(_, held)| *held);
            Message::info(&format!(
                "passed over {}: refused or failed in {held} runs",
                view::place(extent)
            ))
        }
        Event::TagsRefused { entry, category } => Message::warning(&format!(
            "refused the tags of {entry}: {}",
            category.as_deref().unwrap_or("unknown")
        )),
        Event::TagsFailed { entry, cause } => Message::warning(&format!(
            "call failed for the tags of {entry}: {}",
            view::tags_what(cause, api)
        )),
        Event::NoTag { entry, why } => Message::warning(&format!(
            "call failed for the tags of {entry}: {}",
            if *why == tags::answer::Why::Both {
                view::BOTH
            } else {
                view::NO_TAG
            }
        )),
        Event::TagsPassed { entry, charges } => Message::info(&format!(
            "passed over the tags of {entry}: refused or failed in {charges} runs"
        )),
        Event::SummariesNotAsked { why } => Message::warning(&format!(
            "no summary is asked in this run: {}",
            sanitize(why)
        )),
        Event::SummaryRefused { entry, category } => Message::warning(&format!(
            "refused the summary of {entry}: {}",
            category.as_deref().unwrap_or("unknown")
        )),
        Event::SummaryFailed { entry, cause } => Message::warning(&format!(
            "call failed for the summary of {entry}: {}",
            view::summary_what(cause, api)
        )),
        Event::NoSummary { entry, .. } => Message::warning(&format!(
            "call failed for the summary of {entry}: {}",
            view::NO_SUMMARY
        )),
        Event::SummaryPassed { entry, charges } => Message::info(&format!(
            "passed over the summary of {entry}: refused or failed in {charges} runs"
        )),
    }
}

fn why_lines(ended: &Ended, writer: Option<&str>, tagger: Option<&str>) -> Vec<String> {
    match ended {
        Ended::Done | Ended::Calls => [writer, tagger]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect(),
        _ => {
            let own = match ended_which(ended) {
                Which::Writer => writer,
                Which::Tagger => tagger,
            };
            view::why_after(ended, own).into_iter().collect()
        }
    }
}

fn ended_which(ended: &Ended) -> Which {
    match ended {
        Ended::Done | Ended::Calls => Which::Writer,
        Ended::Failures(which)
        | Ended::Busy(_, _, which)
        | Ended::Wait(_, _, which)
        | Ended::Stopped(_, which)
        | Ended::Prepare(_, which) => *which,
    }
}

fn ended_line(
    ended: &Ended,
    api: bool,
    section: &Section,
    whys: &dyn Fn(Which) -> Option<String>,
    keys: Option<&(String, String)>,
) -> Option<Message> {
    let line = |text: String| Message {
        mark: Mark::Error,
        text,
        details: Vec::new(),
    };
    match ended {
        Ended::Done | Ended::Calls | Ended::Prepare(..) => None,
        Ended::Busy(cause, attempt, _) => Some(line(format!(
            "the provider is busy: {}; gave up after attempt {attempt}",
            view::what(cause, api)
        ))),
        Ended::Wait(wait, cause, _) => Some(line(format!(
            "the provider is busy: {}; the next attempt would wait {}, more than derive.retry_limit",
            view::what(cause, api),
            duration::show(*wait)
        ))),
        Ended::Stopped(cause, which) => Some(line(format!(
            "the provider stopped the run: {}",
            view::stopped(cause, api, whys(*which).as_deref())
        ))),
        Ended::Failures(which) => {
            let whose = keys.map_or(String::new(), |(writer, tagger)| {
                let key = if *which == Which::Writer {
                    writer
                } else {
                    tagger
                };
                format!(", to the model of {key}")
            });
            Some(line(format!(
                "stopped: {} calls failed in a row{whose}",
                section.failures.value
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use bilbo_derive::run::Event;
    use bilbo_derive::tries::Extent;
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{Said, ended_line, event_line, finish};
    use crate::console::message::Message;
    use crate::console::outcome::Failure;
    use crate::console::theme::Mark;

    #[test]
    fn an_answer_with_no_note_is_said_as_a_failed_call_with_its_sentence() {
        let extent = Extent {
            parent: Id::from_bytes([4; 16]),
            generation: Generation(2),
            first: 1,
            last: 23,
        };
        let line = event_line(&Event::NoNote { extent }, true, &[]);
        assert_eq!(
            line.plain_lines(),
            [format!(
                "bilbo: call failed for {} #r1-23: the answer holds no note bilbo can keep",
                extent.parent
            )]
        );
        assert_eq!(line.mark, Mark::Warning);
    }

    #[test]
    fn what_was_said_before_an_error_of_the_listing_is_printed() {
        let mut said = Said::default();
        said.lines.push(Message::warning("said before the calls"));
        let tail = vec![
            Message::warning("refused an entry"),
            Message::info("passed over"),
        ];
        let error = io::Error::other("cannot list /spool: Permission denied (os error 13)");
        let result = finish(Err(error), tail, &mut said, |_| Ok(()));
        let lines: Vec<String> = said.lines.iter().flat_map(Message::plain_lines).collect();
        assert_eq!(
            lines,
            [
                "bilbo: said before the calls",
                "bilbo: refused an entry",
                "bilbo: passed over"
            ]
        );
        assert_eq!(
            result,
            Err(Failure::Refused(
                "cannot list /spool: Permission denied (os error 13)".to_owned()
            ))
        );
    }

    #[test]
    fn each_line_about_tags_has_its_words() {
        use bilbo_derive::tags::answer::Why;
        use bilbo_llm::model::{Cause, What};
        let entry = Id::from_bytes([4; 16]);
        let said = |event: Event, api: bool| event_line(&event, api, &[]).plain_lines();
        assert_eq!(
            said(
                Event::TagsRefused {
                    entry,
                    category: None
                },
                true
            ),
            [format!("bilbo: refused the tags of {entry}: unknown")]
        );
        for (why, text) in [
            (Why::Both, "the answer picks a tag and says none fits"),
            (Why::Empty, "the answer names no tag bilbo can keep"),
            (Why::Marks, "the answer names no tag bilbo can keep"),
            (Why::Strays, "the answer names no tag bilbo can keep"),
        ] {
            assert_eq!(
                said(Event::NoTag { entry, why }, true),
                [format!(
                    "bilbo: call failed for the tags of {entry}: {text}"
                )]
            );
        }
        for (api, text) in [
            (
                true,
                "the answer was cut at derive.answer_tokens; raise it or lower derive.tags.notes",
            ),
            (
                false,
                "the answer was cut by the tool; lower derive.tags.notes",
            ),
        ] {
            assert_eq!(
                said(
                    Event::TagsFailed {
                        entry,
                        cause: Cause::new(What::Cut)
                    },
                    api
                ),
                [format!(
                    "bilbo: call failed for the tags of {entry}: {text}"
                )]
            );
        }
        assert_eq!(
            said(Event::TagsPassed { entry, charges: 3 }, true),
            [format!(
                "bilbo: passed over the tags of {entry}: refused or failed in 3 runs"
            )]
        );
    }

    #[test]
    fn each_line_about_a_summary_has_its_words() {
        use bilbo_derive::summary::answer::Why;
        use bilbo_llm::model::{Cause, What};
        let entry = Id::from_bytes([4; 16]);
        let said = |event: Event, api: bool| event_line(&event, api, &[]);
        let line = said(
            Event::SummariesNotAsked {
                why: "unbound".to_owned(),
            },
            true,
        );
        assert_eq!(
            line.plain_lines(),
            ["bilbo: no summary is asked in this run: unbound".to_owned()]
        );
        assert_eq!(line.mark, Mark::Warning);
        let line = said(
            Event::SummaryRefused {
                entry,
                category: None,
            },
            true,
        );
        assert_eq!(
            line.plain_lines(),
            [format!("bilbo: refused the summary of {entry}: unknown")]
        );
        assert_eq!(line.mark, Mark::Warning);
        let line = said(
            Event::SummaryRefused {
                entry,
                category: Some("policy".to_owned()),
            },
            true,
        );
        assert_eq!(
            line.plain_lines(),
            [format!("bilbo: refused the summary of {entry}: policy")]
        );
        let line = said(
            Event::NoSummary {
                entry,
                why: Why::Names,
            },
            true,
        );
        assert_eq!(
            line.plain_lines(),
            [format!(
                "bilbo: call failed for the summary of {entry}: the answer holds no summary bilbo can keep"
            )]
        );
        assert_eq!(line.mark, Mark::Warning);
        for (api, text) in [
            (
                true,
                "the answer was cut at derive.answer_tokens; raise it or lower derive.summary.window",
            ),
            (
                false,
                "the answer was cut by the tool; lower derive.summary.window",
            ),
        ] {
            let line = said(
                Event::SummaryFailed {
                    entry,
                    cause: Cause::new(What::Cut),
                },
                api,
            );
            assert_eq!(
                line.plain_lines(),
                [format!(
                    "bilbo: call failed for the summary of {entry}: {text}"
                )]
            );
            assert_eq!(line.mark, Mark::Warning);
        }
        let line = said(Event::SummaryPassed { entry, charges: 3 }, true);
        assert_eq!(
            line.plain_lines(),
            [format!(
                "bilbo: passed over the summary of {entry}: refused or failed in 3 runs"
            )]
        );
        assert_eq!(line.mark, Mark::Info);
    }

    #[test]
    fn after_an_ordinary_end_each_model_says_its_own_reason() {
        use super::why_lines;
        use bilbo_derive::run::{Ended, Which};
        use bilbo_llm::model::{Cause, What};
        for ended in [Ended::Done, Ended::Calls] {
            assert_eq!(
                why_lines(&ended, Some("w"), Some("t")),
                ["w".to_owned(), "t".to_owned()]
            );
            assert_eq!(why_lines(&ended, None, Some("t")), ["t".to_owned()]);
            assert_eq!(why_lines(&ended, Some("w"), None), ["w".to_owned()]);
        }
        assert_eq!(
            why_lines(&Ended::Failures(Which::Tagger), Some("w"), Some("t")),
            ["t".to_owned()]
        );
        let not_started = Cause::new(What::NotStarted);
        assert_eq!(
            why_lines(
                &Ended::Stopped(not_started, Which::Tagger),
                Some("w"),
                Some("t")
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_line_a_run_ends_with_names_a_model_only_when_there_are_two() {
        use bilbo_derive::keys::defaults;
        use bilbo_derive::run::{Ended, Which};
        let mut section = defaults();
        section.failures.value = 3;
        let no_why = |_: Which| None;
        let one = ended_line(
            &Ended::Failures(Which::Writer),
            true,
            &section,
            &no_why,
            None,
        )
        .expect("a line");
        assert_eq!(
            one.plain_lines(),
            ["bilbo: stopped: 3 calls failed in a row"]
        );
        let keys = (
            "llm.anthropic.model".to_owned(),
            "derive.tags.model".to_owned(),
        );
        for (which, key) in [
            (Which::Tagger, "derive.tags.model"),
            (Which::Writer, "llm.anthropic.model"),
        ] {
            let two = ended_line(
                &Ended::Failures(which),
                true,
                &section,
                &no_why,
                Some(&keys),
            )
            .expect("a line");
            assert_eq!(
                two.plain_lines(),
                [format!(
                    "bilbo: stopped: 3 calls failed in a row, to the model of {key}"
                )]
            );
        }
        let own = |which: Which| Some(if which == Which::Writer { "wr" } else { "tg" }.to_owned());
        for (which, reason) in [(Which::Tagger, "tg"), (Which::Writer, "wr")] {
            let stopped = ended_line(
                &Ended::Stopped(
                    bilbo_llm::model::Cause::new(bilbo_llm::model::What::NotStarted),
                    which,
                ),
                true,
                &section,
                &own,
                Some(&keys),
            )
            .expect("a line");
            assert_eq!(
                stopped.plain_lines(),
                [format!(
                    "bilbo: the provider stopped the run: its program could not be started: {reason}"
                )]
            );
        }
    }
}
