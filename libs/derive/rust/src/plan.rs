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
    pub tags: Option<tags::Planned>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagsState {
    Due,
    Asked,
    PassedOver,
    NotComplete,
    NoNote,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub untrusted: Option<Untrusted>,
    pub states: Vec<(Id, State)>,
    pub due: Vec<Due>,
    pub tags: Vec<(Id, TagsState)>,
}

pub struct Ports<'a> {
    pub entries: &'a dyn Entries,
    pub sealed: &'a dyn Sealed,
    pub scope: &'a dyn Scope,
    pub tries: &'a dyn Tries,
    pub asked: &'a dyn tags::asked::Asked,
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
            Some(Judged {
                found: Found::State(state),
                looked,
            }) => {
                plan.states.push((id, state));
                if let (Some(rules_tags), Some(parent), Some(instruction)) =
                    (&rules.tags, &looked.parent, &looked.instruction)
                {
                    if !looked.complete {
                        plan.tags.push((id, TagsState::NotComplete));
                        continue;
                    }
                    {
                        let (planned, shown, notes) = tagging(
                            ports,
                            rules,
                            rules_tags,
                            parent,
                            looked.head.as_ref(),
                            looked.opens,
                            false,
                        )?;
                        plan.tags.push((id, shown));
                        if let (Some(planned), Some(head)) = (planned, looked.head) {
                            wanted.push(Wanted {
                                due: Due {
                                    parent: parent.clone(),
                                    instruction: instruction.clone(),
                                    preface: None,
                                    held: Some((head.generation, head.records)),
                                    opens: false,
                                    last: looked.last.clone(),
                                    stored: notes,
                                    passages: Vec::new(),
                                    windows: Vec::new(),
                                    tags: Some(planned),
                                },
                                raw: Vec::new(),
                                head: Some(head),
                            });
                        }
                    }
                }
            }
            Some(Judged {
                found: Found::Due(entry),
                ..
            }) => {
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
    plan.due = schedule(ports, rules, wanted, &mut plan.tags)?;
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
) -> Result<Option<Judged>, Error> {
    if standings.stale.contains(&id) {
        return Ok(Some(Judged::plain(Found::State(State::LabelsNotCurrent))));
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
        return Ok(Some(Judged::plain(Found::State(state))));
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
) -> Result<Judged, Error> {
    let covered = |passes: u64| {
        Found::State(if passes > 0 {
            State::PassedOver
        } else {
            State::Covered
        })
    };
    if parent.records == 0 {
        return Ok(Judged {
            found: covered(0),
            looked: Looked {
                parent: Some(parent),
                instruction: Some(instruction.clone()),
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
    let kept_head = head;
    Ok(Judged {
        found: Found::Due(Box::new(Wanted {
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
                tags: None,
                parent,
            },
            raw: found,
            head: kept_head,
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
) -> Result<Vec<Due>, Error> {
    let mut scheduled = Vec::new();
    for Wanted { mut due, raw, head } in wanted {
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
        if due.tags.is_none()
            && let Some(tags_rules) = &rules.tags
            && !due.windows.is_empty()
        {
            let calls = due.windows.iter().any(|planned| !planned.pass);
            let (planned, state, notes) = tagging(
                ports,
                rules,
                tags_rules,
                &due.parent,
                head.as_ref(),
                due.opens,
                calls,
            )?;
            shown.push((due.parent.id, state));
            if planned.is_some() {
                due.tags = planned;
                if !calls {
                    due.stored = notes;
                }
            }
        }
        if !due.windows.is_empty() || due.tags.is_some() {
            scheduled.push(due);
        }
    }
    Ok(scheduled)
}

type Records = Vec<Vec<u8>>;

fn notes_of(
    entries: &dyn Entries,
    head: &Head,
    format: &Format<'_>,
) -> Result<(Records, Records), StoreError> {
    let all = records_of(entries, head)?;
    let notes = all
        .iter()
        .filter(|record| (format.read)(record).is_some())
        .cloned()
        .collect();
    Ok((all, notes))
}

fn fold(notes: &[Vec<u8>]) -> Option<u64> {
    notes.iter().fold(None, |before, record| {
        Some(tags::asked::digest(before, record))
    })
}

pub(crate) fn digest_of(
    entries: &dyn Entries,
    head: &Head,
    format: &Format<'_>,
) -> Result<Option<u64>, StoreError> {
    Ok(fold(&notes_of(entries, head, format)?.1))
}

fn tagging(
    ports: &Ports<'_>,
    rules: &Rules<'_>,
    tags_rules: &tags::Rules<'_>,
    parent: &Parent,
    head: Option<&Head>,
    opens: bool,
    calls: bool,
) -> Result<(Option<tags::Planned>, TagsState, Vec<Paragraph>), Error> {
    let none = |state| Ok((None, state, Vec::new()));
    let answered = ports.asked.answered(parent.id)?;
    let passed = ports.asked.passed(parent.id)?;
    let list = &tags_rules.section.pick.value;
    let in_force = (tags_rules.id, rules.provider, tags_rules.model);
    let held = |planned: tags::Planned| Ok((Some(planned), TagsState::Due, Vec::new()));
    if calls {
        let (added, notes) = match head {
            Some(head) if !opens => {
                let (all, notes) = notes_of(ports.entries, head, rules.format)?;
                let added = answered
                    .as_ref()
                    .is_some_and(|held| added_after(held, head, &all, rules.format));
                (added, fold(&notes))
            }
            _ => (false, None),
        };
        return held(tags::Planned {
            answered,
            passed,
            added,
            notes,
            state: None,
            asking: None,
            sent: 0,
        });
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
    let (all, notes) = notes_of(ports.entries, head, rules.format)?;
    if notes.is_empty() {
        return none(TagsState::NoNote);
    }
    let paragraphs: Vec<Paragraph> = notes
        .iter()
        .filter_map(|record| (rules.format.read)(record))
        .collect();
    let added = answered
        .as_ref()
        .is_none_or(|held| added_after(held, head, &all, rules.format));
    let counter = Counter {
        tokens: rules.tokens,
        bound: rules.bound,
    };
    let fit = tags::view::fitting(&paragraphs, tags_rules.section.notes.value, &counter);
    let material = tags::view::material(fit, list, tags_rules.section.most.value);
    let Some(digest) = fold(&notes) else {
        return none(TagsState::NoNote);
    };
    let about = tags::asked::About {
        generation: parent.generation,
        records: parent.records,
        notes: digest,
    };
    let asking = tags::asked::asking(
        parent.id,
        &about,
        rules.provider,
        tags_rules.model,
        tags_rules.id,
        &material,
    );
    let charges = ports.asked.charges(parent.id, asking)?;
    let seen = tags::plan::Seen {
        answered: answered.as_ref(),
        passed,
        added,
        asking,
    };
    match tags::plan::state(&seen, list, in_force, rules.redo) {
        tags::plan::State::Settled => none(TagsState::Asked),
        tags::plan::State::PassedOver => none(TagsState::PassedOver),
        tags::plan::State::Due => Ok((
            Some(tags::Planned {
                answered,
                passed,
                added,
                notes: Some(digest),
                state: Some(tags::plan::State::Due),
                asking: Some((asking, charges)),
                sent: counter.count(&material),
            }),
            TagsState::Due,
            paragraphs,
        )),
    }
}

fn added_after(
    answered: &tags::asked::Answered,
    head: &Head,
    all: &[Vec<u8>],
    format: &Format<'_>,
) -> bool {
    if answered.notes.0 != head.generation {
        return true;
    }
    let at = usize::try_from(answered.notes.1).unwrap_or(usize::MAX);
    all.get(at..)
        .is_some_and(|tail| tail.iter().any(|record| (format.read)(record).is_some()))
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
