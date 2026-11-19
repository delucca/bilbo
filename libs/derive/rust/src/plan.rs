use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::time::SystemTime;

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
    pub instruction: Instruction,
    pub preface: Option<String>,
    pub held: Option<(Generation, u64)>,
    pub opens: bool,
    pub last: Option<Run>,
    pub stored: Vec<Paragraph>,
    pub passages: Vec<Sent>,
    pub windows: Vec<Planned>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub untrusted: Option<Untrusted>,
    pub states: Vec<(Id, State)>,
    pub due: Vec<Due>,
}

pub struct Ports<'a> {
    pub entries: &'a dyn Entries,
    pub sealed: &'a dyn Sealed,
    pub scope: &'a dyn Scope,
    pub tries: &'a dyn Tries,
}

pub struct Rules<'a> {
    pub readers: &'a [&'a dyn Reader],
    pub instructions: &'a [Instruction],
    pub section: &'a Section,
    pub format: &'a Format<'a>,
    pub tokens: &'a dyn Tokens,
    pub bound: usize,
    pub model: &'a str,
    pub redo: bool,
    pub now: SystemTime,
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
}

enum Found {
    State(State),
    Due(Box<Wanted>),
}

struct Standings {
    stale: Vec<Id>,
    named: Vec<Id>,
    left_out: Vec<Id>,
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
        match judge(ports, rules, &standings, asked, id)? {
            None => {}
            Some(Found::State(state)) => plan.states.push((id, state)),
            Some(Found::Due(entry)) => {
                plan.states.push((id, State::Due));
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
    plan.due = schedule(ports.tries, rules.section, wanted)?;
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

fn judge(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    standings: &Standings,
    asked: Asked<'_>,
    id: Id,
) -> Result<Option<Found>, Error> {
    if standings.stale.contains(&id) {
        return Ok(Some(Found::State(State::LabelsNotCurrent)));
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
        return Ok(Some(Found::State(State::NoInstruction)));
    };
    let state = if parent.belongs {
        Some(State::BelongsToAnother)
    } else if !standings.named.contains(&id) {
        Some(State::NotNamed)
    } else if standings.left_out.contains(&id) {
        Some(State::LeftOut)
    } else if !asked.names(id) && !quiet_for(rules, ports.sealed.sealed_at(id)?) {
        Some(State::StillGrowing)
    } else {
        None
    };
    if let Some(state) = state {
        return Ok(Some(Found::State(state)));
    }
    cover(ports, rules, parent, instruction, *reader).map(Some)
}

#[must_use]
pub fn label(parent: Id) -> CaptureLabel {
    CaptureLabel::new(format!("derive:{parent}"))
        .expect("a capture label of a derived entry is not empty")
}

pub fn held(entries: &dyn Entries, parent: Id) -> Result<Option<Head>, StoreError> {
    Ok(entries
        .by_capture(&label(parent))?
        .filter(|head| head.records > 0))
}

fn last_run(entries: &dyn Entries, head: &Head) -> Result<Option<Run>, StoreError> {
    let range = RecordRange {
        first: head.records,
        count: 1,
    };
    Ok(entries
        .read(head.id, head.generation, range)?
        .first()
        .and_then(|bytes| record::read(bare(bytes))))
}

fn cover(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    parent: Parent,
    instruction: &Instruction,
    reader: &dyn Reader,
) -> Result<Found, Error> {
    let covered = |passes: u64| {
        Found::State(if passes > 0 {
            State::PassedOver
        } else {
            State::Covered
        })
    };
    if parent.records == 0 {
        return Ok(covered(0));
    }
    let head = held(ports.entries, parent.id)?;
    let last = match &head {
        Some(head) => last_run(ports.entries, head)?,
        None => None,
    };
    let provider = &rules.section.provider.value;
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
    if !opens && frontier == parent.records {
        return Ok(covered(passes));
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
        return Ok(covered(passes));
    }
    let stored = match &head {
        Some(head) if !opens => stored(ports.entries, head, rules.format)?,
        _ => Vec::new(),
    };
    Ok(Found::Due(Box::new(Wanted {
        due: Due {
            instruction: instruction.clone(),
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
            parent,
        },
        raw: found,
    })))
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

fn schedule(tries: &dyn Tries, section: &Section, wanted: Vec<Wanted>) -> Result<Vec<Due>, Error> {
    let mut scheduled = Vec::new();
    for Wanted { mut due, raw } in wanted {
        for window in raw {
            let extent = Extent {
                parent: due.parent.id,
                generation: due.parent.generation,
                first: window.first,
                last: window.last,
            };
            let pass = usize::from(tries.charges(&extent)?) >= section.tries.value;
            due.windows.push(Planned {
                window,
                extent,
                pass,
            });
        }
        if !due.windows.is_empty() {
            scheduled.push(due);
        }
    }
    Ok(scheduled)
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
