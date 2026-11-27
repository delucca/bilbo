use std::cell::OnceCell;
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::time::SystemTime;

use bilbo_entries::forget::{self, Forgotten};
use bilbo_entries::reader::Reader;
use bilbo_entries::tokens::Tokens;
use bilbo_store::entries::{CaptureError, Entries, Generation, Head, RecordRange, StoreError};
use bilbo_store::id::Id;
use bilbo_store::labels::CaptureLabel;
use bilbo_store::sealed::Sealed;

use crate::instruction::Instruction;
use crate::keys::Section;
use crate::record::{self, Run};
use crate::scope::{Scope, Standing, Untrusted};
use crate::spool::bare;
use crate::summary;
use crate::tags;
use crate::tries::{Extent, Tries};
use crate::view::{Counter, Format, Paragraph};
use crate::window::{Sent, Sizes, Window, windows};

#[derive(Debug)]
pub enum Error {
    Store(StoreError),
    Capture(CaptureError),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Store(error) => error.fmt(f),
            Error::Capture(error) => error.fmt(f),
            Error::Io(error) => error.fmt(f),
        }
    }
}

impl StdError for Error {}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Error::Store(error)
    }
}

impl From<CaptureError> for Error {
    fn from(error: CaptureError) -> Self {
        Error::Capture(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    LabelsNotCurrent,
    NoInstruction,
    BelongsToAnother,
    NotNamed,
    LeftOut,
    StillGrowing,
    Covered,
    PassedOver,
    Due,
}

impl State {
    #[must_use]
    pub fn words(self) -> &'static str {
        match self {
            State::LabelsNotCurrent => "labels not current",
            State::NoInstruction => "no instruction",
            State::BelongsToAnother => "belongs to another",
            State::NotNamed => "not named",
            State::LeftOut => "left out",
            State::StillGrowing => "still growing",
            State::Covered => "covered",
            State::PassedOver => "passed over",
            State::Due => "due",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parent {
    pub id: Id,
    pub kind: String,
    pub belongs: bool,
    pub generation: Generation,
    pub records: u64,
    pub forgotten: bool,
}

impl Parent {
    pub fn of(entries: &dyn Entries, id: Id) -> Result<Option<Parent>, StoreError> {
        let Some(meta) = entries.meta(id)? else {
            return Ok(None);
        };
        let (generation, records) = entries.extent(id, None)?.unwrap_or((Generation(1), 0));
        Ok(Some(Parent {
            id,
            kind: meta.kind,
            belongs: meta.labels.id.is_some(),
            generation,
            records,
            forgotten: meta
                .labels
                .capture
                .as_ref()
                .is_some_and(|label| Forgotten::read(id, label).is_some()),
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub window: Window,
    pub extent: Extent,
    pub pass: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub parent: Parent,
    pub instruction: Option<Instruction>,
    pub preface: Option<String>,
    pub held: Option<(Generation, u64)>,
    pub opens: bool,
    pub last: Option<Run>,
    pub stored: Vec<Paragraph>,
    pub passages: Vec<Sent>,
    pub windows: Vec<Planned>,
    pub tags: Option<tags::Planned>,
    pub summary: Option<summary::Planned>,
}

impl Due {
    pub(crate) fn writing(&self) -> &Instruction {
        self.instruction
            .as_ref()
            .expect("an entry with a window has the instruction of its kind")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagsState {
    Due,
    Asked,
    PassedOver,
    NotComplete,
    NoNote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryState {
    Due,
    Written,
    PassedOver,
    NotComplete,
    NoNote,
    Short,
    NoReader,
    Out(State),
    Forgotten,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub untrusted: Option<Untrusted>,
    pub states: Vec<(Id, State)>,
    pub due: Vec<Due>,
    pub tags: Vec<(Id, TagsState)>,
    pub summaries: Vec<(Id, SummaryState)>,
}

pub struct Ports<'a> {
    pub entries: &'a dyn Entries,
    pub sealed: &'a dyn Sealed,
    pub scope: &'a dyn Scope,
    pub tries: &'a dyn Tries,
    pub asked: &'a dyn tags::asked::Asked,
    pub summarised: &'a dyn summary::asked::Asked,
}

pub struct Rules<'a> {
    pub readers: &'a [&'a dyn Reader],
    pub instructions: &'a [Instruction],
    pub section: &'a Section,
    pub format: &'a Format<'a>,
    pub tokens: &'a dyn Tokens,
    pub bound: usize,
    pub model: &'a str,
    pub provider: &'a str,
    pub redo: bool,
    pub now: SystemTime,
    pub tags: Option<tags::Rules<'a>>,
    pub summary: Option<summary::Rules<'a>>,
}

impl fmt::Debug for Ports<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ports").finish_non_exhaustive()
    }
}

impl fmt::Debug for Rules<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rules")
            .field("instructions", &self.instructions)
            .field("section", &self.section)
            .field("format", &self.format)
            .field("bound", &self.bound)
            .field("model", &self.model)
            .field("redo", &self.redo)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

struct Wanted {
    due: Due,
    raw: Vec<Window>,
    head: Option<Head>,
    eligible: Option<Eligible>,
    notes: OnceCell<Scan>,
}

struct Eligible {
    parent: Parent,
    head: Head,
}

enum Pre {
    Skip,
    State(SummaryState),
    Eligible(Box<Eligible>),
}

enum Side<'a> {
    Text,
    Notes {
        head: Option<Head>,
        opens: bool,
        last: Option<&'a Run>,
        complete: bool,
        windowed: bool,
        given_up: bool,
    },
}

enum Found {
    State(State),
    Due(Box<Wanted>),
}

#[derive(Default)]
struct Looked {
    parent: Option<Parent>,
    instruction: Option<Instruction>,
    head: Option<Head>,
    last: Option<Run>,
    opens: bool,
    complete: bool,
    given_up: bool,
}

struct Judged {
    found: Found,
    looked: Looked,
}

impl Judged {
    fn plain(found: Found) -> Judged {
        Judged {
            found,
            looked: Looked::default(),
        }
    }
}

struct Standings {
    stale: Vec<Id>,
    named: Vec<Id>,
    left_out: Vec<Id>,
    had: Vec<forget::Forgotten>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked<'a> {
    Every,
    Only(&'a [Id]),
}

impl Asked<'_> {
    fn names(self, id: Id) -> bool {
        matches!(self, Asked::Only(ids) if ids.contains(&id))
    }
}

pub fn plan(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    entries: &[Id],
    asked: Asked<'_>,
) -> Result<Plan, Error> {
    let stale = match ports.scope.standing()? {
        Standing::Untrusted(why) => {
            return Ok(Plan {
                untrusted: Some(why),
                ..Plan::default()
            });
        }
        Standing::NotCurrent(stale) => stale,
    };
    let left_out = ports.scope.named(&rules.section.left_out.value)?;
    let standings = Standings {
        stale,
        named: ports.scope.named(&rules.section.named.value)?.entries,
        left_out: [left_out.entries, left_out.several].concat(),
        had: forget::held(ports.entries)?,
    };
    let considered = match asked {
        Asked::Every => entries,
        Asked::Only(ids) => ids,
    };
    let mut plan = Plan::default();
    let mut wanted = Vec::new();
    let mut seen = Vec::new();
    for &id in considered {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let eligible = match &rules.summary {
            None => None,
            Some(_) => match summary_pre(ports, rules, &standings, asked, id)? {
                Pre::Skip => None,
                Pre::State(state) => {
                    plan.summaries.push((id, state));
                    None
                }
                Pre::Eligible(eligible) => Some(*eligible),
            },
        };
        let cache = OnceCell::new();
        match judge(ports, rules, &standings, asked, id)? {
            None => {}
            Some(Judged {
                found: Found::State(state),
                looked,
            }) => {
                plan.states.push((id, state));
                if let (Some(rules_tags), Some(parent), Some(instruction)) =
                    (&rules.tags, &looked.parent, &looked.instruction)
                {
                    if looked.complete {
                        let (planned, shown) = tagging(
                            ports,
                            rules,
                            rules_tags,
                            parent,
                            &Notes {
                                head: looked.head.as_ref(),
                                opens: looked.opens,
                                cache: &cache,
                            },
                            false,
                        )?;
                        plan.tags.push((id, shown));
                        if let (Some(planned), Some(head)) = (planned, looked.head) {
                            wanted.push(Wanted {
                                due: Due {
                                    parent: parent.clone(),
                                    instruction: Some(instruction.clone()),
                                    preface: None,
                                    held: Some((head.generation, head.records)),
                                    opens: false,
                                    last: looked.last.clone(),
                                    stored: Vec::new(),
                                    passages: Vec::new(),
                                    windows: Vec::new(),
                                    tags: Some(planned),
                                    summary: None,
                                },
                                raw: Vec::new(),
                                head: Some(head),
                                eligible: None,
                                notes: OnceCell::new(),
                            });
                        }
                    } else {
                        plan.tags.push((id, TagsState::NotComplete));
                    }
                }
                if let (Some(eligible), Some(sr)) = (eligible, &rules.summary) {
                    let side = match &looked.instruction {
                        Some(_) => Side::Notes {
                            head: looked.head,
                            opens: looked.opens,
                            last: looked.last.as_ref(),
                            complete: looked.complete,
                            windowed: false,
                            given_up: looked.given_up,
                        },
                        None => Side::Text,
                    };
                    let (state, planned) = summarising(ports, rules, sr, &eligible, &side, &cache)?;
                    plan.summaries.push((id, state));
                    if let Some(planned) = planned {
                        match wanted.last_mut() {
                            Some(entry) if entry.due.parent.id == id => {
                                entry.due.summary = Some(planned);
                            }
                            _ => wanted.push(Wanted {
                                due: Due {
                                    parent: eligible.parent.clone(),
                                    instruction: looked.instruction.clone(),
                                    preface: None,
                                    held: None,
                                    opens: false,
                                    last: None,
                                    stored: Vec::new(),
                                    passages: Vec::new(),
                                    windows: Vec::new(),
                                    tags: None,
                                    summary: Some(planned),
                                },
                                raw: Vec::new(),
                                head: None,
                                eligible: None,
                                notes: OnceCell::new(),
                            }),
                        }
                    }
                }
                if let Some(entry) = wanted.last_mut()
                    && entry.due.parent.id == id
                    && entry.due.tags.is_some()
                {
                    entry.due.stored = cache
                        .into_inner()
                        .map(|found| found.paragraphs)
                        .unwrap_or_default();
                }
            }
            Some(Judged {
                found: Found::Due(mut entry),
                ..
            }) => {
                plan.states.push((id, State::Due));
                entry.eligible = eligible;
                entry.notes = cache;
                wanted.push(*entry);
            }
        }
    }
    wanted.sort_by_key(|entry| std::cmp::Reverse(entry.due.parent.id));
    turn(
        &mut wanted,
        |entry| entry.due.parent.id,
        ports.tries.mark()?,
    );
    plan.due = schedule(ports, rules, wanted, &mut plan.tags, &mut plan.summaries)?;
    Ok(plan)
}

fn turn<T>(items: &mut [T], id: impl Fn(&T) -> Id, mark: Option<Id>) {
    let Some(mark) = mark else {
        return;
    };
    if let Some(at) = items.iter().position(|item| id(item) < mark) {
        items.rotate_left(at);
    }
}

fn quiet_for(rules: &Rules<'_>, sealed: Option<SystemTime>) -> bool {
    let threshold = rules
        .now
        .checked_sub(rules.section.quiet.value)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    sealed.is_some_and(|at| at <= threshold)
}

struct Gate<'a> {
    ports: &'a Ports<'a>,
    rules: &'a Rules<'a>,
    standings: &'a Standings,
    asked: Asked<'a>,
    id: Id,
}

impl Gate<'_> {
    fn labels(&self) -> Option<State> {
        self.standings
            .stale
            .contains(&self.id)
            .then_some(State::LabelsNotCurrent)
    }

    fn rest(&self, belongs: bool) -> Result<Option<State>, Error> {
        let id = self.id;
        Ok(if belongs {
            Some(State::BelongsToAnother)
        } else if !self.standings.named.contains(&id) {
            Some(State::NotNamed)
        } else if self.standings.left_out.contains(&id) {
            Some(State::LeftOut)
        } else if !self.asked.names(id) && !quiet_for(self.rules, self.ports.sealed.sealed_at(id)?)
        {
            Some(State::StillGrowing)
        } else {
            None
        })
    }
}

fn judge(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    standings: &Standings,
    asked: Asked<'_>,
    id: Id,
) -> Result<Option<Judged>, Error> {
    let gate = Gate {
        ports,
        rules,
        standings,
        asked,
        id,
    };
    if let Some(state) = gate.labels() {
        return Ok(Some(Judged::plain(Found::State(state))));
    }
    let Some(parent) = Parent::of(ports.entries, id)? else {
        return Ok(None);
    };
    let instruction = rules
        .instructions
        .iter()
        .find(|instruction| instruction.kind == parent.kind);
    let reader = rules
        .readers
        .iter()
        .find(|reader| reader.kind() == parent.kind);
    let (Some(instruction), Some(reader)) = (instruction, reader) else {
        return Ok(Some(Judged::plain(Found::State(State::NoInstruction))));
    };
    if let Some(state) = gate.rest(parent.belongs)? {
        return Ok(Some(Judged::plain(Found::State(state))));
    }
    cover(ports, rules, standings, parent, instruction, *reader).map(Some)
}

#[must_use]
pub fn label(parent: Id) -> CaptureLabel {
    CaptureLabel::new(format!("derive:{parent}"))
        .expect("a capture label of a derived entry is not empty")
}

fn head_under(entries: &dyn Entries, label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
    Ok(entries.by_capture(label)?.filter(|head| head.records > 0))
}

fn last_record<T>(
    entries: &dyn Entries,
    head: &Head,
    parse: impl Fn(&[u8]) -> Option<T>,
) -> Result<Option<T>, StoreError> {
    let range = RecordRange {
        first: head.records,
        count: 1,
    };
    Ok(entries
        .read(head.id, head.generation, range)?
        .first()
        .and_then(|bytes| parse(bare(bytes))))
}

pub fn held(entries: &dyn Entries, parent: Id) -> Result<Option<Head>, StoreError> {
    head_under(entries, &label(parent))
}

fn last_run(entries: &dyn Entries, head: &Head) -> Result<Option<Run>, StoreError> {
    last_record(entries, head, record::read)
}

fn cover(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    standings: &Standings,
    parent: Parent,
    instruction: &Instruction,
    reader: &dyn Reader,
) -> Result<Judged, Error> {
    let covered = |passes: u64| {
        Found::State(if passes > 0 {
            State::PassedOver
        } else {
            State::Covered
        })
    };
    let gave = !parent.forgotten
        && forget::given_up_by_lookup(ports.entries, &standings.had, &label(parent.id))?;
    if parent.records == 0 || parent.forgotten || gave {
        return Ok(Judged {
            found: covered(0),
            looked: Looked {
                parent: Some(parent),
                instruction: Some(instruction.clone()),
                given_up: gave,
                ..Looked::default()
            },
        });
    }
    let head = held(ports.entries, parent.id)?;
    let last = match &head {
        Some(head) => last_run(ports.entries, head)?,
        None => None,
    };
    let provider = rules.provider;
    let mut opens = head.is_some() && last.is_none();
    let mut frontier = 0;
    let mut passes = 0;
    if let Some(run) = &last {
        let changed = rules.redo
            && (run.instruction != instruction.id
                || run.provider != *provider
                || run.model != rules.model
                || !run.same
                || run.passes > 0);
        if run.generation == parent.generation && !changed {
            frontier = run.last;
            passes = run.passes;
        } else {
            opens = true;
        }
    }
    let complete = |last: &Option<Run>, opens: bool| {
        !opens
            && last
                .as_ref()
                .is_some_and(|run| run.generation == parent.generation)
    };
    let looked = |opens: bool| Looked {
        parent: Some(parent.clone()),
        instruction: Some(instruction.clone()),
        head,
        last: last.clone(),
        opens,
        complete: complete(&last, opens),
        given_up: false,
    };
    if !opens && frontier == parent.records {
        return Ok(Judged {
            found: covered(passes),
            looked: looked(opens),
        });
    }
    let range = RecordRange {
        first: 1,
        count: parent.records,
    };
    let held = ports.entries.read(parent.id, parent.generation, range)?;
    let records: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
    let grown = last.as_ref().is_some_and(|run| run.total != parent.records);
    if !opens && frontier > 0 && !reader.appends_only() && grown {
        opens = true;
        frontier = 0;
        passes = 0;
    }
    let counter = Counter {
        tokens: rules.tokens,
        bound: rules.bound,
    };
    let sizes = Sizes {
        window: rules.section.window.value,
        passage: rules.section.passage.value,
        head: rules.section.passage_head.value,
    };
    let passages = reader.passages(&records);
    let found = windows(&passages, frontier, parent.records, &counter, &sizes);
    if found.is_empty() {
        return Ok(Judged {
            found: covered(passes),
            looked: looked(opens),
        });
    }
    let stored = match &head {
        Some(head) if !opens => stored(ports.entries, head, rules.format)?,
        _ => Vec::new(),
    };
    Ok(Judged {
        found: Found::Due(Box::new(Wanted {
            due: Due {
                instruction: Some(instruction.clone()),
                preface: reader.preface(&records),
                held: head.map(|head| (head.generation, head.records)),
                opens,
                last: last.filter(|_| !opens),
                stored,
                passages: passages
                    .iter()
                    .map(|passage| Sent {
                        range: passage.range,
                        at: passage.at,
                    })
                    .collect(),
                windows: Vec::new(),
                tags: None,
                summary: None,
                parent,
            },
            raw: found,
            head,
            eligible: None,
            notes: OnceCell::new(),
        })),
        looked: Looked::default(),
    })
}

pub(crate) fn records_of(entries: &dyn Entries, head: &Head) -> Result<Vec<Vec<u8>>, StoreError> {
    let range = RecordRange {
        first: 1,
        count: head.records,
    };
    Ok(entries
        .read(head.id, head.generation, range)?
        .iter()
        .map(|bytes| bare(bytes).to_vec())
        .collect())
}

fn stored(
    entries: &dyn Entries,
    head: &Head,
    format: &Format<'_>,
) -> Result<Vec<Paragraph>, StoreError> {
    Ok(records_of(entries, head)?
        .iter()
        .filter_map(|bytes| (format.read)(bytes))
        .collect())
}

fn schedule(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    wanted: Vec<Wanted>,
    shown: &mut Vec<(Id, TagsState)>,
    summaries: &mut Vec<(Id, SummaryState)>,
) -> Result<Vec<Due>, Error> {
    let mut scheduled = Vec::new();
    for Wanted {
        mut due,
        raw,
        head,
        eligible,
        notes: cache,
    } in wanted
    {
        for window in raw {
            let extent = Extent {
                parent: due.parent.id,
                generation: due.parent.generation,
                first: window.first,
                last: window.last,
            };
            let pass = usize::from(ports.tries.charges(&extent)?) >= rules.section.tries.value;
            due.windows.push(Planned {
                window,
                extent,
                pass,
            });
        }
        let calls = due.windows.iter().any(|planned| !planned.pass);
        if let Some(tags_rules) = &rules.tags
            && !due.windows.is_empty()
        {
            let (planned, state) = tagging(
                ports,
                rules,
                tags_rules,
                &due.parent,
                &Notes {
                    head: head.as_ref(),
                    opens: due.opens,
                    cache: &cache,
                },
                calls,
            )?;
            shown.push((due.parent.id, state));
            if planned.is_some() {
                due.tags = planned;
            }
        }
        if let (Some(eligible), Some(sr)) = (&eligible, &rules.summary) {
            let (state, planned) = if calls {
                let record = held_summary(ports.entries, due.parent.id)?;
                let notes = match head.filter(|_| !due.opens) {
                    Some(notes_head) => {
                        cached(&cache, ports.entries, &notes_head, rules.format, false)?.digest
                    }
                    None => None,
                };
                (
                    SummaryState::Due,
                    Some(summary::Planned {
                        asking: None,
                        sent: 0,
                        made: None,
                        retire: false,
                        head: eligible.head,
                        notes,
                        record,
                    }),
                )
            } else {
                let side = Side::Notes {
                    head,
                    opens: due.opens,
                    last: due.last.as_ref(),
                    complete: false,
                    windowed: true,
                    given_up: false,
                };
                summarising(ports, rules, sr, eligible, &side, &cache)?
            };
            summaries.push((due.parent.id, state));
            due.summary = planned;
        }
        if due.tags.is_some() && !calls && !due.windows.is_empty() {
            due.stored = cache
                .into_inner()
                .map(|found| found.paragraphs)
                .unwrap_or_default();
        }
        if !due.windows.is_empty() || due.tags.is_some() || due.summary.is_some() {
            scheduled.push(due);
        }
    }
    Ok(scheduled)
}

struct Scan {
    digest: Option<u64>,
    last: Option<usize>,
    paragraphs: Vec<Paragraph>,
}

fn scan(
    entries: &dyn Entries,
    head: &Head,
    format: &Format<'_>,
    gather: bool,
) -> Result<Scan, StoreError> {
    let mut found = Scan {
        digest: None,
        last: None,
        paragraphs: Vec::new(),
    };
    for (at, record) in records_of(entries, head)?.iter().enumerate() {
        if let Some(paragraph) = (format.read)(record) {
            found.digest = Some(tags::asked::digest(found.digest, record));
            found.last = Some(at);
            if gather {
                found.paragraphs.push(paragraph);
            }
        }
    }
    Ok(found)
}

fn cached<'c>(
    cache: &'c OnceCell<Scan>,
    entries: &dyn Entries,
    head: &Head,
    format: &Format<'_>,
    gather: bool,
) -> Result<&'c Scan, StoreError> {
    if cache.get().is_none() {
        let _ = cache.set(scan(entries, head, format, gather)?);
    }
    Ok(cache.get().expect("the scan was just kept"))
}

pub(crate) fn digest_of(
    entries: &dyn Entries,
    head: &Head,
    format: &Format<'_>,
) -> Result<Option<u64>, StoreError> {
    Ok(scan(entries, head, format, false)?.digest)
}

struct Notes<'a> {
    head: Option<&'a Head>,
    opens: bool,
    cache: &'a OnceCell<Scan>,
}

fn tagging(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    tags_rules: &tags::Rules<'_>,
    parent: &Parent,
    look: &Notes<'_>,
    calls: bool,
) -> Result<(Option<tags::Planned>, TagsState), Error> {
    let (head, opens, cache) = (look.head, look.opens, look.cache);
    let none = |state| Ok((None, state));
    let answered = ports.asked.answered(parent.id)?;
    let passed = ports.asked.passed(parent.id)?;
    let list = &tags_rules.section.pick.value;
    let in_force = (tags_rules.id, rules.provider, tags_rules.model);
    if calls {
        let (added, notes) = match head {
            Some(head) if !opens => {
                let found = cached(cache, ports.entries, head, rules.format, false)?;
                let added = answered
                    .as_ref()
                    .is_some_and(|held| added_after(held, head, found.last));
                (added, found.digest)
            }
            _ => (false, None),
        };
        return Ok((
            Some(tags::Planned {
                answered,
                passed,
                added,
                notes,
                state: None,
                asking: None,
                sent: 0,
            }),
            TagsState::Due,
        ));
    }
    let Some(head) = head.filter(|_| !opens) else {
        return none(TagsState::NoNote);
    };
    if !tags::plan::unsettled(
        answered.as_ref(),
        passed,
        (head.generation, head.records),
        list,
        in_force,
        rules.redo,
    ) {
        return none(TagsState::Asked);
    }
    let found = cached(cache, ports.entries, head, rules.format, true)?;
    let Some(digest) = found.digest else {
        return none(TagsState::NoNote);
    };
    let paragraphs = &found.paragraphs;
    let added = answered
        .as_ref()
        .is_none_or(|held| added_after(held, head, found.last));
    let counter = Counter {
        tokens: rules.tokens,
        bound: rules.bound,
    };
    let known = tags::Known {
        id: parent.id,
        generation: parent.generation,
        records: parent.records,
        notes: paragraphs,
        digest,
        answered: answered.as_ref(),
        passed,
        added,
    };
    let made = tags::asking_of(&known, tags_rules, rules.provider, rules.redo, &counter);
    let asking = made.seen.asking;
    let state = made.state;
    let charges = ports.asked.charges(parent.id, asking)?;
    match state {
        tags::plan::State::Settled => none(TagsState::Asked),
        tags::plan::State::PassedOver => none(TagsState::PassedOver),
        tags::plan::State::Due => {
            let sent = counter.count(&made.material);
            Ok((
                Some(tags::Planned {
                    answered,
                    passed,
                    added,
                    notes: Some(digest),
                    state: Some(tags::plan::State::Due),
                    asking: Some((asking, charges)),
                    sent,
                }),
                TagsState::Due,
            ))
        }
    }
}

fn summary_pre(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    standings: &Standings,
    asked: Asked<'_>,
    id: Id,
) -> Result<Pre, Error> {
    let out = |state| Ok(Pre::State(SummaryState::Out(state)));
    let gate = Gate {
        ports,
        rules,
        standings,
        asked,
        id,
    };
    if let Some(state) = gate.labels() {
        return out(state);
    }
    let Some(meta) = ports.entries.meta(id)? else {
        return Ok(Pre::Skip);
    };
    if !rules
        .readers
        .iter()
        .any(|reader| reader.kind() == meta.kind)
    {
        return Ok(Pre::State(SummaryState::NoReader));
    }
    if let Some(state) = gate.rest(meta.labels.id.is_some())? {
        return out(state);
    }
    let Some(capture) = meta.labels.capture else {
        return Ok(Pre::Skip);
    };
    if Forgotten::read(id, &capture).is_some()
        || forget::given_up_by_lookup(ports.entries, &standings.had, &summary::label(id))?
    {
        return Ok(Pre::State(SummaryState::Forgotten));
    }
    let Some(head) = ports
        .entries
        .by_capture(&capture)?
        .filter(|head| head.id == id)
    else {
        return Ok(Pre::Skip);
    };
    let Some(parent) = Parent::of(ports.entries, id)? else {
        return Ok(Pre::Skip);
    };
    Ok(Pre::Eligible(Box::new(Eligible { parent, head })))
}

pub(crate) fn held_summary(
    entries: &dyn Entries,
    id: Id,
) -> Result<Option<summary::record::Summary>, StoreError> {
    match head_under(entries, &summary::label(id))? {
        Some(head) => last_record(entries, &head, summary::record::read),
        None => Ok(None),
    }
}

fn summarising(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    sr: &summary::Rules<'_>,
    eligible: &Eligible,
    side: &Side<'_>,
    cache: &OnceCell<Scan>,
) -> Result<(SummaryState, Option<summary::Planned>), Error> {
    let (parent, head) = (&eligible.parent, eligible.head);
    let retirement = |record: &Option<summary::record::Summary>, notes: Option<u64>| {
        record.as_ref().map(|_| summary::Planned {
            asking: None,
            sent: 0,
            made: None,
            retire: true,
            head,
            notes,
            record: record.clone(),
        })
    };
    if let Side::Notes {
        complete: false,
        windowed: false,
        given_up,
        ..
    } = side
    {
        let record = if parent.records == 0 && !given_up {
            held_summary(ports.entries, parent.id)?
        } else {
            None
        };
        return Ok((SummaryState::NotComplete, retirement(&record, None)));
    }
    let record = held_summary(ports.entries, parent.id)?;
    let in_force = (sr.id, rules.provider, rules.model);
    let mut about = summary::asked::About {
        generation: parent.generation,
        records: parent.records,
        content: head.mark.hash,
        notes: None,
    };
    let mut paragraphs: &[Paragraph] = &[];
    if let Side::Notes {
        head: Some(notes_head),
        opens: false,
        ..
    } = side
    {
        let shortcut = record
            .as_ref()
            .and_then(|held| held.notes)
            .filter(|held| {
                held.generation == notes_head.generation && held.records == notes_head.records
            })
            .map(|held| held.digest);
        if let (Some(digest), Some(held)) = (shortcut, &record) {
            about.notes = Some(digest);
            if summary::plan::written(held, &about, in_force, rules.redo) {
                return Ok((SummaryState::Written, None));
            }
        }
        let found = cached(cache, ports.entries, notes_head, rules.format, true)?;
        about.notes = found.digest;
        paragraphs = &found.paragraphs;
    }
    let mut passages = Vec::new();
    let mut covered = true;
    match side {
        Side::Notes {
            last,
            opens,
            windowed,
            ..
        } => {
            covered = *windowed
                || (!opens
                    && last.is_some_and(|run| {
                        run.generation == parent.generation && run.last == parent.records
                    }));
        }
        Side::Text => {
            if let Some(held) = &record
                && summary::plan::written(held, &about, in_force, rules.redo)
            {
                return Ok((SummaryState::Written, None));
            }
            if parent.records > 0 {
                let range = RecordRange {
                    first: 1,
                    count: parent.records,
                };
                let held = ports.entries.read(parent.id, parent.generation, range)?;
                let records: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
                if let Some(reader) = rules
                    .readers
                    .iter()
                    .find(|reader| reader.kind() == parent.kind)
                {
                    passages = reader.passages(&records);
                }
            }
        }
    }
    let counter = Counter {
        tokens: rules.tokens,
        bound: rules.bound,
    };
    let known = summary::plan::Known {
        id: parent.id,
        about,
        of: match side {
            Side::Notes { .. } => summary::view::Of::Notes(paragraphs),
            Side::Text => summary::view::Of::Text(&passages),
        },
        record: record.as_ref(),
        covered,
    };
    let force = summary::plan::Force {
        provider: rules.provider,
        model: rules.model,
        redo: rules.redo,
        head: rules.section.passage_head.value,
        counter: &counter,
    };
    let decided = summary::plan::decide(&known, sr, &force, ports.summarised)?;
    let notes = known.about.notes;
    Ok(match decided {
        summary::plan::Decided::Written => (SummaryState::Written, None),
        summary::plan::Decided::NoNote { retire } => (
            SummaryState::NoNote,
            retire.then(|| retirement(&record, notes)).flatten(),
        ),
        summary::plan::Decided::Short { retire } => (
            SummaryState::Short,
            retire.then(|| retirement(&record, notes)).flatten(),
        ),
        summary::plan::Decided::PassedOver => (SummaryState::PassedOver, None),
        summary::plan::Decided::Due(asking) => (
            SummaryState::Due,
            Some(summary::Planned {
                asking: Some((asking.number, asking.charges)),
                sent: asking.sent,
                made: Some(asking.made),
                retire: false,
                head,
                notes,
                record,
            }),
        ),
    })
}

fn added_after(answered: &tags::asked::Answered, head: &Head, last: Option<usize>) -> bool {
    if answered.notes.0 != head.generation {
        return true;
    }
    let at = usize::try_from(answered.notes.1).unwrap_or(usize::MAX);
    last.is_some_and(|last| last >= at)
}

#[cfg(test)]
mod tests {
    use bilbo_store::id::Id;

    use super::turn;

    fn id(n: u8) -> Id {
        Id::from_bytes([n; 16])
    }

    fn turned(mark: Option<Id>) -> Vec<u8> {
        let mut order: Vec<Id> = [5, 4, 3, 2, 1].map(id).to_vec();
        turn(&mut order, |entry| *entry, mark);
        order.iter().map(|entry| entry.to_bytes()[0]).collect()
    }

    #[test]
    fn the_order_begins_one_past_the_mark_and_wraps() {
        assert_eq!(turned(None), [5, 4, 3, 2, 1]);
        assert_eq!(turned(Some(id(3))), [2, 1, 5, 4, 3]);
        assert_eq!(turned(Some(Id::from_bytes([0; 16]))), [5, 4, 3, 2, 1]);
        assert_eq!(turned(Some(id(9))), [5, 4, 3, 2, 1]);
        let mut gone = id(3).to_bytes();
        gone[15] = 1;
        assert_eq!(turned(Some(Id::from_bytes(gone))), [2, 1, 5, 4, 3]);
    }
}
