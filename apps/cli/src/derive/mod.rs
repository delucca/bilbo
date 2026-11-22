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
use bilbo_derive::plan::{self, Asked, Plan, Ports, Rules, State};
use bilbo_derive::run::{self, Clock, Context, Ended, Event, Sinks};
use bilbo_derive::scope::Untrusted;
use bilbo_derive::spool::bare;
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
use disk::{Blocks, Failed, Folder, Held};
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

pub type Finish<'a> = dyn Fn(Store, u64, &mut dyn FnMut(&Message)) -> Closing + 'a;

pub struct Hand<'a> {
    pub readers: &'a [Box<dyn Reader>],
    pub tokenizer: &'a dyn Fn() -> Result<Tokenizer, Failure>,
    pub lookup: &'a dyn Fn(&Path) -> Box<dyn Lookup>,
    pub chunk: Size,
    pub render: &'a str,
    pub format: Format<'a>,
    pub writer: &'a dyn Fn(&Var) -> Result<Built, Failure>,
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
        && let Err(text) = section.gate()
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
    let job = Job {
        settings,
        hand,
        section: &section,
        instructions: &instructions,
        redo: args.redo,
        dry: args.dry_run,
    };
    if args.dry_run {
        let built = if section.provider.value.is_empty() {
            None
        } else {
            Some((hand.writer)(&env.var)?)
        };
        return dry(env, &job, built.as_ref(), &given);
    }
    let mut built = (hand.writer)(&env.var)?;
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
    replay(env, settings, hand, &mut spool, said, &mut failed)?;
    let tokenizer = (hand.tokenizer)()?;
    let model = built.writer.id().to_owned();
    let mut tries = held.failed().map_err(|error| refused(&error))?;
    let looked = look(
        env,
        job,
        &tokenizer,
        &model,
        given,
        Some(&tries),
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
        if let Some(text) = state.and_then(|state| not_sent(text, state, kind)) {
            said.lines.push(Message::warning(&text));
            exit = Exit::Refused;
        }
    }
    tries
        .keep_only(&looked.plan.due)
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
            spool: &mut spool,
            tries: &mut tries,
            log: &mut log,
            clock: &Wall,
        },
        &Context {
            section,
            format: &hand.format,
            counter: &counter,
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
    if let Ended::Prepare(text) = &report.ended {
        return Err(Failure::Config(sanitize(text)));
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
    tail.extend(ended_line(
        &report.ended,
        built.api,
        section,
        why.as_deref(),
    ));
    tail.extend(view::why_after(&report.ended, why.as_deref()).map(|text| Message::warning(&text)));
    if let Some(error) = log.error() {
        tail.push(Message::warning(&format!(
            "the calls were not logged: {error}"
        )));
    }
    let listed = spool.is_empty();
    finish(listed, tail, said, |said| {
        replay(env, settings, hand, &mut spool, said, &mut failed)
    })?;
    let hard = matches!(
        report.ended,
        Ended::Failures | Ended::Busy(..) | Ended::Wait(..) | Ended::Stopped(_)
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

fn replay(
    env: &mut Env,
    settings: &Settings,
    hand: &Hand<'_>,
    spool: &mut Blocks,
    said: &mut Said,
    failed: &mut bool,
) -> Result<(), Failure> {
    spool.clear_parts().map_err(|error| refused(&error))?;
    if spool.is_empty().map_err(|error| refused(&error))? {
        return Ok(());
    }
    let layout = settings.layout();
    Store::check(&layout)?;
    let notice = &mut env.notice;
    let mut store = Store::open(&layout, settings.store.options(), &mut |heard| {
        notice(&notice_message(&heard));
    })?;
    let replayed = match run::replay(&mut store, spool, &hand.format) {
        Ok(replayed) => replayed,
        Err(interrupted) => {
            said.lines.extend(
                interrupted
                    .notices
                    .iter()
                    .map(|text| Message::warning(text)),
            );
            store.release();
            return Err(refused(&interrupted));
        }
    };
    said.lines
        .extend(replayed.notices.iter().map(|text| Message::warning(text)));
    if replayed.captured > 0 {
        let closing = (hand.finish)(store, replayed.captured, &mut *env.notice);
        *failed |= closing.failed || closing.refused;
        said.lines.extend(closing.messages);
    } else {
        store.release();
    }
    Ok(())
}

fn look(
    env: &mut Env,
    job: &Job<'_>,
    tokenizer: &Tokenizer,
    model: &str,
    given: &[Given],
    held: Option<&Failed>,
    lines: &mut Vec<Message>,
) -> Result<Looked, Failure> {
    let Job {
        settings,
        hand,
        section,
        instructions,
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
    let tries: &Failed = match held {
        Some(held) => held,
        None => {
            let (state, folder) = state_folder(&store)?;
            own = Folder::at(&state, folder)
                .charges()
                .map_err(|error| refused(&error))?;
            &own
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
        },
        &Rules {
            readers: &readers,
            instructions,
            section,
            format: &hand.format,
            tokens: &*tokenizer.tokens,
            bound: MAX_ENCODE_BYTES,
            model,
            redo,
            now: SystemTime::now(),
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
    for id in &only {
        if let Some(meta) = store.meta(*id)? {
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
    drop(store);
    Ok(Looked {
        plan,
        ids,
        problems,
        charges,
        noted,
        kinds,
    })
}

fn dry(
    env: &mut Env,
    job: &Job<'_>,
    built: Option<&Built>,
    given: &[Given],
) -> Result<Outcome, Failure> {
    let Job {
        hand,
        section,
        instructions,
        ..
    } = *job;
    let mut off = Vec::new();
    if !section.enabled.value {
        off.push("derive.enabled is false".to_owned());
    }
    if section.provider.value.is_empty() {
        off.push("derive.provider is not set".to_owned());
    }
    if section.named.value.is_empty() {
        off.push("derive.in is empty".to_owned());
    }
    let model = built
        .map(|built| built.writer.id().to_owned())
        .unwrap_or_default();
    let tokenizer = (hand.tokenizer)()?;
    let mut diagnostics = Vec::new();
    let looked = look(env, job, &tokenizer, &model, given, None, &mut diagnostics)?;
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
    let mut left = section.calls.value;
    for due in &looked.plan.due {
        let mut any = false;
        for planned in due.windows.iter().filter(|planned| !planned.pass) {
            if left == 0 {
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
        }
    }
    Ok(Outcome::new(joined(&view::lines(&dry)))
        .with_diagnostics(diagnostics)
        .with_exit(exit))
}

fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn untrusted(why: Untrusted) -> String {
    match why {
        Untrusted::Behind => "the index is behind the store, so labels may be missing; bilbo remember brings it up to date",
        Untrusted::NoIndex => "there is no search index, so labels cannot be read; bilbo remember makes one",
        Untrusted::OtherBilbo => "the search index was made by another bilbo, so labels cannot be read; bilbo remember makes it again",
    }
    .to_owned()
}

fn not_sent(text: &str, state: State, kind: &str) -> Option<String> {
    match state {
        State::LabelsNotCurrent => Some(format!(
            "{text} has labels that are not current; bilbo remember brings the index up to date"
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
    }
}

fn ended_line(ended: &Ended, api: bool, section: &Section, why: Option<&str>) -> Option<Message> {
    let line = |text: String| Message {
        mark: Mark::Error,
        text,
        details: Vec::new(),
    };
    match ended {
        Ended::Done | Ended::Calls | Ended::Prepare(_) => None,
        Ended::Busy(cause, attempt) => Some(line(format!(
            "the provider is busy: {}; gave up after attempt {attempt}",
            view::what(cause, api)
        ))),
        Ended::Wait(wait, cause) => Some(line(format!(
            "the provider is busy: {}; the next attempt would wait {}, more than derive.retry_limit",
            view::what(cause, api),
            duration::show(*wait)
        ))),
        Ended::Stopped(cause) => Some(line(format!(
            "the provider stopped the run: {}",
            view::stopped(cause, api, why)
        ))),
        Ended::Failures => Some(line(format!(
            "stopped: {} calls failed in a row",
            section.failures.value
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use bilbo_derive::run::Event;
    use bilbo_derive::tries::Extent;
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{Said, event_line, finish};
    use crate::console::message::Message;
    use crate::console::outcome::Failure;

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
}
