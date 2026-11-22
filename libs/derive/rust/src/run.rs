use bilbo_store::labels::Tags;
use std::time::{Duration, SystemTime};

use bilbo_store::entries::{Capture, Entries};
use bilbo_store::id::Id;
use bilbo_store::opaque;

use crate::answer;
use crate::check::check;
use crate::keys::Section;
use crate::log::{self, Call, Log, TagVerdict, Tagged, Verdict};
use crate::plan::{self, Due, Error, Planned};
use crate::record::{self, Run, next_same};
use crate::spool::{self, Block, Spool, bare};
use crate::tags::{self, asked::Asked as TagsAsked};
use crate::tries::{Extent, Tries};
use crate::view::{self, Counter, Format, Paragraph};
use bilbo_entries::tag as entry_tag;
use bilbo_llm::model::{Cause, Limits, Model, Outcome, Prompt, Usage, What, Whose};

pub trait Clock {
    fn now(&self) -> SystemTime;
    fn sleep(&self, wait: Duration);
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub entries: u64,
    pub calls: u64,
    pub notes: u64,
    pub dropped: u64,
    pub refused: u64,
    pub failed: u64,
    pub passed: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Writer,
    Tagger,
}

impl Which {
    fn at(self) -> usize {
        match self {
            Which::Writer => 0,
            Which::Tagger => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Done,
    Calls,
    Failures(Which),
    Busy(Cause, u32, Which),
    Wait(Duration, Cause, Which),
    Stopped(Cause, Which),
    Prepare(String, Which),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Refused {
        extent: Extent,
        category: Option<String>,
    },
    Failed {
        extent: Extent,
        cause: Cause,
    },
    NoNote {
        extent: Extent,
    },
    Passed {
        extent: Extent,
    },
    TagsRefused {
        entry: Id,
        category: Option<String>,
    },
    TagsFailed {
        entry: Id,
        cause: Cause,
    },
    NoTag {
        entry: Id,
        why: tags::answer::Why,
    },
    TagsPassed {
        entry: Id,
        charges: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub counts: Counts,
    pub ended: Ended,
    pub events: Vec<Event>,
}

pub struct Sinks<'a> {
    pub writer: &'a mut dyn Model,
    pub tagger: Option<&'a mut dyn Model>,
    pub asked: &'a mut dyn TagsAsked,
    pub spool: &'a mut dyn Spool,
    pub tries: &'a mut dyn Tries,
    pub log: &'a mut dyn Log,
    pub clock: &'a dyn Clock,
}

pub struct Context<'a> {
    pub section: &'a Section,
    pub provider: &'a str,
    pub limits: Limits,
    pub format: &'a Format<'a>,
    pub counter: &'a Counter<'a>,
    pub tags: Option<tags::Rules<'a>>,
    pub redo: bool,
}

impl std::fmt::Debug for Sinks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sinks").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("section", &self.section)
            .field("provider", &self.provider)
            .field("limits", &self.limits)
            .field("format", &self.format)
            .field("counter", &self.counter)
            .field("tags", &self.tags.is_some())
            .field("redo", &self.redo)
            .finish()
    }
}

enum Step {
    Worked(Vec<Paragraph>, Run),
    EndEntry,
    Stop(Ended),
}

enum Attempted {
    Answer {
        text: String,
        usage: Usage,
        model: Option<String>,
        taken: (Option<u32>, u64),
    },
    Cut {
        usage: Usage,
        taken: (Option<u32>, u64),
    },
    Failed {
        cause: Cause,
        usage: Usage,
        taken: (Option<u32>, u64),
    },
    Refused {
        category: Option<String>,
        usage: Usage,
        taken: (Option<u32>, u64),
    },
    Calls,
    GaveUp(Cause, Ended),
}

struct Made {
    answered: Option<String>,
    usage: Usage,
    kept: u64,
    range: u64,
    names: u64,
    passed: bool,
}

struct Line {
    verdict: Verdict,
    attempt: Option<u32>,
    answered: Option<String>,
    usage: Usage,
    tallies: Option<(u64, u64, u64, u64)>,
    cause: Option<Cause>,
    reason: Option<String>,
    millis: u64,
}

#[derive(Clone, Copy)]
enum Slot {
    Window(Extent),
    Asking(Id, u64),
}

struct Machine<'a> {
    writer: &'a mut dyn Model,
    tagger: Option<&'a mut dyn Model>,
    asked: &'a mut dyn TagsAsked,
    spool: &'a mut dyn Spool,
    tries: &'a mut dyn Tries,
    log: &'a mut dyn Log,
    clock: &'a dyn Clock,
    context: &'a Context<'a>,
    counts: Counts,
    attempts: usize,
    streak: [usize; 2],
    waiting: [Vec<Slot>; 2],
    answered: [bool; 2],
    prepared: [bool; 2],
    bound: [bool; 2],
    events: Vec<Event>,
    counted: Option<Id>,
    notes: Vec<Vec<u8>>,
}

fn size_of_window(planned: &Planned) -> u64 {
    u64::try_from(planned.window.sent).unwrap_or(u64::MAX)
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn reason(cause: &Cause) -> String {
    let mut text = cause.what.word().to_owned();
    if let Some(status) = cause.status {
        text.push_str(&format!(" {status}"));
    }
    for part in [&cause.kind, &cause.code].into_iter().flatten() {
        text.push(' ');
        text.push_str(part);
    }
    text
}

#[derive(Debug)]
pub struct Halted {
    pub error: Error,
    pub events: Vec<Event>,
}

impl std::fmt::Display for Halted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for Halted {}

#[derive(Debug)]
pub struct Interrupted {
    pub error: Error,
    pub notices: Vec<String>,
}

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for Interrupted {}

pub fn calls(due: &[Due], sinks: Sinks<'_>, context: &Context<'_>) -> Result<Report, Box<Halted>> {
    let Sinks {
        writer,
        tagger,
        asked,
        spool,
        tries,
        log,
        clock,
    } = sinks;
    let tagger = tagger.map(|tagger| -> &mut dyn Model { tagger });
    let writer_ran = due
        .iter()
        .any(|entry| entry.windows.iter().any(|planned| !planned.pass));
    if writer_ran && let Err(text) = writer.prepare() {
        return Ok(Report {
            counts: Counts::default(),
            ended: Ended::Prepare(text, Which::Writer),
            events: Vec::new(),
        });
    }
    let mut machine = Machine {
        writer,
        tagger,
        asked,
        spool,
        tries,
        log,
        clock,
        context,
        counts: Counts::default(),
        attempts: 0,
        streak: [0; 2],
        waiting: [Vec::new(), Vec::new()],
        answered: [false; 2],
        prepared: [writer_ran, false],
        bound: [false; 2],
        events: Vec::new(),
        counted: None,
        notes: Vec::new(),
    };
    let first = due
        .iter()
        .find(|entry| {
            entry.windows.iter().any(|planned| !planned.pass)
                || entry
                    .tags
                    .as_ref()
                    .is_some_and(|planned| planned.state == Some(tags::plan::State::Due))
        })
        .map(|entry| entry.parent.id);
    let ended = match machine.work(due).and_then(|ended| {
        if matches!(ended, Ended::Done | Ended::Calls) {
            for which in [Which::Writer, Which::Tagger] {
                if machine.answered[which.at()] {
                    machine.charge_waiting(which)?;
                }
            }
        }
        machine.mark(first)?;
        Ok(ended)
    }) {
        Ok(ended) => ended,
        Err(error) => {
            return Err(Box::new(Halted {
                error,
                events: machine.events,
            }));
        }
    };
    Ok(Report {
        counts: machine.counts,
        ended,
        events: machine.events,
    })
}

impl Machine<'_> {
    fn work(&mut self, due: &[Due]) -> Result<Ended, Error> {
        for entry in due {
            if let Some(ended) = self.entry(entry)? {
                return Ok(ended);
            }
        }
        Ok(Ended::Done)
    }

    fn entry(&mut self, entry: &Due) -> Result<Option<Ended>, Error> {
        self.notes.clear();
        let mut previous = entry.last.clone();
        let mut mine: Vec<Paragraph> = Vec::new();
        for planned in &entry.windows {
            let (paragraphs, run) = if planned.pass {
                let run = self.record(
                    entry,
                    planned,
                    previous.as_ref(),
                    Made {
                        answered: None,
                        usage: Usage::default(),
                        kept: 0,
                        range: 0,
                        names: 0,
                        passed: true,
                    },
                    0,
                );
                self.spool_records(entry, &[], &run)?;
                self.tries.forget(&planned.extent)?;
                self.counts.passed += 1;
                self.events.push(Event::Passed {
                    extent: planned.extent,
                });
                self.log(
                    entry,
                    planned,
                    Line {
                        verdict: Verdict::Passed,
                        attempt: None,
                        answered: None,
                        usage: Usage::default(),
                        tallies: None,
                        cause: None,
                        reason: None,
                        millis: 0,
                    },
                    0,
                );
                (Vec::new(), run)
            } else {
                match self.window(entry, planned, previous.as_ref(), &mine)? {
                    Step::Worked(paragraphs, run) => (paragraphs, run),
                    Step::EndEntry => return Ok(None),
                    Step::Stop(ended) => return Ok(Some(ended)),
                }
            };
            mine.extend(paragraphs);
            previous = Some(run);
        }
        self.tagging(entry, &mine)
    }

    fn record(
        &self,
        entry: &Due,
        planned: &Planned,
        previous: Option<&Run>,
        made: Made,
        sent: u64,
    ) -> Run {
        let model = if made.passed {
            String::new()
        } else {
            self.writer.id().to_owned()
        };
        let provider = self.context.provider.to_owned();
        Run {
            kind: entry.parent.kind.clone(),
            instruction: entry.instruction.id.clone(),
            same: next_same(previous, &entry.instruction.id, &provider, &model),
            provider,
            model,
            answered: made.answered,
            made: seconds(self.clock.now()),
            entry: entry.parent.id,
            generation: entry.parent.generation,
            first: planned.window.first,
            last: planned.window.last,
            total: entry.parent.records,
            sent,
            tokens_in: made.usage.read,
            tokens_out: made.usage.written,
            written: made.kept + made.range + made.names,
            kept: made.kept,
            range: made.range,
            names: made.names,
            passes: previous.map_or(0, |run| run.passes) + u64::from(made.passed),
            passed: made.passed,
        }
    }

    fn spool_records(
        &mut self,
        entry: &Due,
        paragraphs: &[Paragraph],
        run: &Run,
    ) -> Result<Vec<Paragraph>, Error> {
        let mut records: Vec<Vec<u8>> = paragraphs
            .iter()
            .map(|paragraph| (self.context.format.write)(paragraph))
            .collect();
        records.push(record::write(run).into_bytes());
        let records: Vec<Vec<u8>> = records
            .iter()
            .map(|record| opaque::filter(record).into_owned())
            .collect();
        let stored = records
            .iter()
            .filter_map(|record| (self.context.format.read)(bare(record)))
            .collect();
        for record in &records {
            if (self.context.format.read)(bare(record)).is_some() {
                self.notes.push(record.clone());
            }
        }
        let block = Block {
            parent: entry.parent.id,
            generation: entry.parent.generation,
            generation_before: entry.held.map(|(generation, _)| generation),
            records_before: entry.held.map_or(0, |(_, records)| records),
            opens: entry.opens,
            records,
        };
        self.spool.keep(&spool::write(&block))?;
        Ok(stored)
    }

    fn mark(&mut self, first: Option<Id>) -> Result<(), Error> {
        if self.answered.iter().any(|answered| *answered) {
            self.tries.set_mark(None)?;
        } else if self.counts.calls > 0 {
            self.tries.set_mark(first)?;
        }
        Ok(())
    }

    fn charge_waiting(&mut self, which: Which) -> Result<(), Error> {
        for slot in std::mem::take(&mut self.waiting[which.at()]) {
            self.charge(slot)?;
        }
        Ok(())
    }

    fn charge(&mut self, slot: Slot) -> Result<(), Error> {
        match slot {
            Slot::Window(extent) => self.tries.charge(&extent)?,
            Slot::Asking(entry, asking) => self.asked.charge(entry, asking)?,
        }
        Ok(())
    }

    fn answered(&mut self, which: Which) -> Result<(), Error> {
        self.answered[which.at()] = true;
        self.streak[which.at()] = 0;
        self.charge_waiting(which)
    }

    fn spent(&self) -> bool {
        self.attempts >= self.context.section.calls.value
    }

    fn model(&mut self, which: Which) -> &mut dyn Model {
        match (which, self.tagger.as_mut()) {
            (Which::Tagger, Some(tagger)) => &mut **tagger,
            _ => &mut *self.writer,
        }
    }

    fn log(&mut self, entry: &Due, planned: &Planned, line: Line, sent: u64) {
        let (written, kept, range, names) = match line.tallies {
            Some((written, kept, range, names)) => {
                (Some(written), Some(kept), Some(range), Some(names))
            }
            None => (None, None, None, None),
        };
        let text = log::line(&Call {
            at: seconds(self.clock.now()),
            verdict: line.verdict,
            kind: entry.parent.kind.clone(),
            parent: entry.parent.id,
            first: planned.window.first,
            last: planned.window.last,
            provider: self.context.provider.to_owned(),
            asked: if line.verdict == Verdict::Passed {
                String::new()
            } else {
                self.writer.id().to_owned()
            },
            answered: line.answered,
            attempt: line.attempt,
            sent,
            tokens_in: line.usage.read,
            tokens_out: line.usage.written,
            written,
            kept,
            range,
            names,
            millis: line.millis,
            reason: line.reason.or_else(|| line.cause.as_ref().map(reason)),
            request: line.cause.and_then(|cause| cause.request),
        });
        self.log.append(&text);
    }

    fn add(&mut self, usage: Usage) {
        self.counts.tokens_in = self
            .counts
            .tokens_in
            .saturating_add(usage.read.unwrap_or(0));
        self.counts.tokens_out = self
            .counts
            .tokens_out
            .saturating_add(usage.written.unwrap_or(0));
    }

    fn window(
        &mut self,
        entry: &Due,
        planned: &Planned,
        previous: Option<&Run>,
        mine: &[Paragraph],
    ) -> Result<Step, Error> {
        let section = self.context.section;
        let shown: Vec<Paragraph> = entry.stored.iter().chain(mine).cloned().collect();
        let earlier = view::earlier(&shown, section.earlier.value, self.context.counter);
        let prompt = Prompt {
            instruction: entry.instruction.text.clone(),
            material: view::material(earlier, entry.preface.as_deref(), &planned.window.text),
        };
        let grounds = view::grounds(earlier, entry.preface.as_deref(), &planned.window.text);
        let sent = size_of_window(planned);
        let context = self.context;
        let attempted = self.attempt(
            Which::Writer,
            entry.parent.id,
            &prompt,
            &context.limits,
            |machine, line| {
                machine.log(entry, planned, line, sent);
            },
        );
        match attempted {
            Attempted::Calls => Ok(Step::Stop(Ended::Calls)),
            Attempted::GaveUp(cause, ended) => self.not_asked_again(planned, &cause, ended),
            Attempted::Answer {
                text,
                usage,
                model,
                taken,
            } => {
                self.add(usage);
                self.answer(
                    entry,
                    planned,
                    previous,
                    &grounds,
                    (text, usage, model),
                    taken,
                )
            }
            Attempted::Cut { usage, taken } => {
                self.failure(entry, planned, Cause::new(What::Cut), usage, taken)
            }
            Attempted::Failed {
                cause,
                usage,
                taken,
            } => self.failure(entry, planned, cause, usage, taken),
            Attempted::Refused {
                category,
                usage,
                taken: (attempt, millis),
            } => {
                self.add(usage);
                self.counts.refused += 1;
                self.tries.charge(&planned.extent)?;
                self.answered(Which::Writer)?;
                self.events.push(Event::Refused {
                    extent: planned.extent,
                    category: category.clone(),
                });
                self.log(
                    entry,
                    planned,
                    Line {
                        verdict: Verdict::Refused,
                        attempt,
                        answered: None,
                        usage,
                        tallies: None,
                        cause: None,
                        reason: Some(category.unwrap_or_else(|| "unknown".to_owned())),
                        millis,
                    },
                    sent,
                );
                Ok(Step::EndEntry)
            }
        }
    }

    fn attempt(
        &mut self,
        which: Which,
        parent: Id,
        prompt: &Prompt,
        limits: &Limits,
        mut log: impl FnMut(&mut Self, Line),
    ) -> Attempted {
        let section = self.context.section;
        let mut retries = 0u32;
        loop {
            if self.spent() {
                return Attempted::Calls;
            }
            self.attempts += 1;
            self.counts.calls += 1;
            if self.counted != Some(parent) {
                self.counted = Some(parent);
                self.counts.entries += 1;
            }
            let attempt = Some(retries + 1);
            let started = self.clock.now();
            let outcome = self.model(which).call(prompt, limits);
            let millis = self.clock.now().duration_since(started).map_or(0, |taken| {
                u64::try_from(taken.as_millis()).unwrap_or(u64::MAX)
            });
            let taken = (attempt, millis);
            match outcome {
                Outcome::Answer { text, usage, model } => {
                    return Attempted::Answer {
                        text,
                        usage,
                        model,
                        taken,
                    };
                }
                Outcome::Cut(usage) => return Attempted::Cut { usage, taken },
                Outcome::Failed { cause, usage } => {
                    return Attempted::Failed {
                        cause,
                        usage,
                        taken,
                    };
                }
                Outcome::Refused { category, usage } => {
                    return Attempted::Refused {
                        category,
                        usage,
                        taken,
                    };
                }
                Outcome::Busy { wait, cause } => {
                    log(
                        self,
                        Line {
                            verdict: Verdict::Busy,
                            attempt,
                            answered: None,
                            usage: Usage::default(),
                            tallies: None,
                            cause: Some(cause.clone()),
                            reason: None,
                            millis,
                        },
                    );
                    if retries >= u32::try_from(section.retries.value).unwrap_or(u32::MAX) {
                        let ended = Ended::Busy(cause.clone(), retries + 1, which);
                        return Attempted::GaveUp(cause, ended);
                    }
                    if self.spent() {
                        return Attempted::Calls;
                    }
                    let times = u32::try_from(section.retry_factor.value)
                        .unwrap_or(u32::MAX)
                        .saturating_pow(retries);
                    let delay =
                        wait.unwrap_or_else(|| section.retry_wait.value.saturating_mul(times));
                    if delay > section.retry_limit.value {
                        let ended = Ended::Wait(delay, cause.clone(), which);
                        return Attempted::GaveUp(cause, ended);
                    }
                    self.clock.sleep(delay);
                    retries += 1;
                }
                Outcome::Stopped(cause) => {
                    log(
                        self,
                        Line {
                            verdict: Verdict::Stopped,
                            attempt,
                            answered: None,
                            usage: Usage::default(),
                            tallies: None,
                            cause: Some(cause.clone()),
                            reason: None,
                            millis,
                        },
                    );
                    let ended = Ended::Stopped(cause.clone(), which);
                    return Attempted::GaveUp(cause, ended);
                }
            }
        }
    }

    fn not_asked_again(
        &mut self,
        planned: &Planned,
        cause: &Cause,
        ended: Ended,
    ) -> Result<Step, Error> {
        if cause.what.whose() == Whose::Provider {
            return Ok(Step::Stop(ended));
        }
        self.counts.failed += 1;
        self.events.push(Event::Failed {
            extent: planned.extent,
            cause: cause.clone(),
        });
        self.fails(Slot::Window(planned.extent), Which::Writer, cause)
    }

    fn failure(
        &mut self,
        entry: &Due,
        planned: &Planned,
        cause: Cause,
        usage: Usage,
        (attempt, millis): (Option<u32>, u64),
    ) -> Result<Step, Error> {
        self.add(usage);
        self.counts.failed += 1;
        let failed_cause = cause.clone();
        self.events.push(Event::Failed {
            extent: planned.extent,
            cause: cause.clone(),
        });
        self.log(
            entry,
            planned,
            Line {
                verdict: Verdict::Failed,
                attempt,
                answered: None,
                usage,
                tallies: None,
                cause: Some(cause),
                reason: None,
                millis,
            },
            size_of_window(planned),
        );
        self.fails(Slot::Window(planned.extent), Which::Writer, &failed_cause)
    }

    fn fails(&mut self, slot: Slot, which: Which, cause: &Cause) -> Result<Step, Error> {
        match cause.what.whose() {
            Whose::Call => self.ends_charged(slot, which),
            Whose::Provider => Ok(Step::Stop(Ended::Stopped(cause.clone(), which))),
            Whose::Either => {
                let at = which.at();
                self.waiting[at].push(slot);
                self.streak[at] += 1;
                Ok(if self.streak[at] >= self.context.section.failures.value {
                    Step::Stop(Ended::Failures(which))
                } else {
                    Step::EndEntry
                })
            }
        }
    }

    fn ends_charged(&mut self, slot: Slot, which: Which) -> Result<Step, Error> {
        self.charge(slot)?;
        self.answered(which)?;
        Ok(Step::EndEntry)
    }

    fn answer(
        &mut self,
        entry: &Due,
        planned: &Planned,
        previous: Option<&Run>,
        grounds: &str,
        (text, usage, model): (String, Usage, Option<String>),
        (attempt, millis): (Option<u32>, u64),
    ) -> Result<Step, Error> {
        let section = self.context.section;
        let parsed = answer::blocks(&text);
        let sent = size_of_window(planned);
        let (verdict, checked) = if text.trim().is_empty() {
            (Verdict::Empty, None)
        } else {
            let checked = check(
                &parsed.blocks,
                &planned.window,
                &entry.passages,
                grounds,
                section.names.value,
                entry.parent.id,
                entry.parent.generation,
            );
            self.counts.dropped += checked.range + checked.names;
            if checked.kept.is_empty() {
                self.counts.failed += 1;
                self.events.push(Event::NoNote {
                    extent: planned.extent,
                });
                self.log(
                    entry,
                    planned,
                    Line {
                        verdict: Verdict::Failed,
                        attempt,
                        answered: model,
                        usage,
                        tallies: Some((
                            checked.range + checked.names,
                            0,
                            checked.range,
                            checked.names,
                        )),
                        cause: None,
                        reason: Some("no-note".to_owned()),
                        millis,
                    },
                    sent,
                );
                return self.ends_charged(Slot::Window(planned.extent), Which::Writer);
            }
            (Verdict::Run, Some(checked))
        };
        let (paragraphs, kept, range, names) = checked.map_or((Vec::new(), 0, 0, 0), |checked| {
            let kept = u64::try_from(checked.kept.len()).unwrap_or(u64::MAX);
            (checked.kept, kept, checked.range, checked.names)
        });
        self.counts.notes += kept;
        let run = self.record(
            entry,
            planned,
            previous,
            Made {
                answered: model.clone(),
                usage,
                kept,
                range,
                names,
                passed: false,
            },
            sent,
        );
        let paragraphs = self.spool_records(entry, &paragraphs, &run)?;
        self.tries.forget(&planned.extent)?;
        self.answered(Which::Writer)?;
        self.log(
            entry,
            planned,
            Line {
                verdict,
                attempt,
                answered: model,
                usage,
                tallies: Some((run.written, kept, range, names)),
                cause: None,
                reason: None,
                millis,
            },
            sent,
        );
        Ok(Step::Worked(paragraphs, run))
    }

    fn tags_log(
        &mut self,
        entry: &Due,
        asked: &str,
        sent: u64,
        verdict: TagVerdict,
        line: Line,
        counts: Option<(u64, u64)>,
    ) {
        let text = log::tags_line(&Tagged {
            at: seconds(self.clock.now()),
            verdict,
            kind: entry.parent.kind.clone(),
            entry: entry.parent.id,
            provider: self.context.provider.to_owned(),
            asked: if verdict == TagVerdict::Passed {
                String::new()
            } else {
                asked.to_owned()
            },
            answered: line.answered,
            attempt: line.attempt,
            sent: (verdict != TagVerdict::Passed).then_some(sent),
            tokens_in: line.usage.read,
            tokens_out: line.usage.written,
            picks: counts.map(|(picks, _)| picks),
            dropped: counts.map(|(_, dropped)| dropped),
            millis: line.millis,
            reason: line.reason.or_else(|| line.cause.as_ref().map(reason)),
            request: line.cause.and_then(|cause| cause.request),
        });
        self.log.append(&text);
    }

    fn tagging(&mut self, entry: &Due, mine: &[Paragraph]) -> Result<Option<Ended>, Error> {
        let context = self.context;
        let (Some(rules), Some(planned)) = (&context.tags, &entry.tags) else {
            return Ok(None);
        };
        if entry.stored.is_empty() && mine.is_empty() {
            return Ok(None);
        }
        let notes: Vec<Paragraph> = entry.stored.iter().chain(mine).cloned().collect();
        let Some(digest) = self.notes.iter().fold(planned.notes, |before, record| {
            Some(tags::asked::digest(before, record))
        }) else {
            return Ok(None);
        };
        let list = &rules.section.pick.value;
        let fit = tags::view::fitting(&notes, rules.section.notes.value, context.counter);
        let material = tags::view::material(fit, list, rules.section.most.value);
        let about = tags::asked::About {
            generation: entry.parent.generation,
            records: entry.parent.records,
            notes: digest,
        };
        let id = entry.parent.id;
        let asking = tags::asked::asking(
            id,
            &about,
            context.provider,
            rules.model,
            rules.id,
            &material,
        );
        let charges = self.asked.charges(id, asking)?;
        let seen = tags::plan::Seen {
            answered: planned.answered.as_ref(),
            passed: planned.passed,
            added: planned.added || !self.notes.is_empty(),
            asking,
        };
        let in_force = (rules.id, context.provider, rules.model);
        if tags::plan::state(&seen, list, in_force, context.redo) != tags::plan::State::Due {
            return Ok(None);
        }
        let sent = u64::try_from(context.counter.count(&material)).unwrap_or(u64::MAX);
        let block = tags::block::Block {
            entry: id,
            about,
            picks: Vec::new(),
            takes_back: false,
            names: std::collections::BTreeSet::new(),
            instruction: rules.id.to_owned(),
            provider: context.provider.to_owned(),
            model: rules.model.to_owned(),
            asking,
            passed: false,
        };
        let idle = |reason: Option<String>| Line {
            verdict: Verdict::Passed,
            attempt: None,
            answered: None,
            usage: Usage::default(),
            tallies: None,
            cause: None,
            reason,
            millis: 0,
        };
        if usize::from(charges) >= context.section.tries.value {
            self.spool.keep(&tags::block::write(&tags::block::Block {
                passed: true,
                ..block
            }))?;
            self.events.push(Event::TagsPassed { entry: id, charges });
            self.tags_log(entry, "", sent, TagVerdict::Passed, idle(None), None);
            return Ok(None);
        }
        if self.spent() {
            return Ok(Some(Ended::Calls));
        }
        let which = if self.tagger.is_some() {
            Which::Tagger
        } else {
            Which::Writer
        };
        let at = which.at();
        if !self.prepared[at] {
            if let Err(text) = self.model(which).prepare() {
                return Ok(Some(Ended::Prepare(text, which)));
            }
            self.prepared[at] = true;
        }
        if !self.bound[at] {
            if let Err(text) = self.model(which).bound() {
                return Ok(Some(Ended::Prepare(text, which)));
            }
            self.bound[at] = true;
        }
        let asked = self.model(which).id().to_owned();
        let prompt = Prompt {
            instruction: rules.instruction.to_owned(),
            material,
        };
        let attempted = self.attempt(which, id, &prompt, &context.limits, |machine, line| {
            let verdict = if line.verdict == Verdict::Busy {
                TagVerdict::Busy
            } else {
                TagVerdict::Stopped
            };
            machine.tags_log(entry, &asked, sent, verdict, line, None);
        });
        let slot = Slot::Asking(id, asking);
        let step = match attempted {
            Attempted::Calls => Step::Stop(Ended::Calls),
            Attempted::GaveUp(cause, ended) => {
                if cause.what.whose() == Whose::Provider {
                    Step::Stop(ended)
                } else {
                    self.counts.failed += 1;
                    self.events.push(Event::TagsFailed {
                        entry: id,
                        cause: cause.clone(),
                    });
                    self.fails(slot, which, &cause)?
                }
            }
            Attempted::Cut { usage, taken } => self.tags_failure(
                entry,
                (which, &asked, sent, slot),
                &Cause::new(What::Cut),
                usage,
                taken,
            )?,
            Attempted::Failed {
                cause,
                usage,
                taken,
            } => self.tags_failure(entry, (which, &asked, sent, slot), &cause, usage, taken)?,
            Attempted::Refused {
                category,
                usage,
                taken: (attempt, millis),
            } => {
                self.add(usage);
                self.counts.refused += 1;
                self.asked.charge(id, asking)?;
                self.answered(which)?;
                self.events.push(Event::TagsRefused {
                    entry: id,
                    category: category.clone(),
                });
                let line = Line {
                    reason: Some(category.unwrap_or_else(|| "unknown".to_owned())),
                    attempt,
                    usage,
                    millis,
                    ..idle(None)
                };
                self.tags_log(entry, &asked, sent, TagVerdict::Refused, line, None);
                Step::EndEntry
            }
            Attempted::Answer {
                text,
                usage,
                model,
                taken: (attempt, millis),
            } => {
                self.add(usage);
                let line = Line {
                    attempt,
                    answered: model,
                    usage,
                    millis,
                    ..idle(None)
                };
                match tags::answer::read(&text, list, rules.section.most.value) {
                    tags::answer::Read::NoTag(why) => {
                        self.counts.failed += 1;
                        self.events.push(Event::NoTag { entry: id, why });
                        self.asked.charge(id, asking)?;
                        self.answered(which)?;
                        let line = Line {
                            reason: Some(format!("no-tag {}", why.word())),
                            ..line
                        };
                        self.tags_log(entry, &asked, sent, TagVerdict::Failed, line, None);
                    }
                    read => {
                        let (picks, dropped, verdict) = match read {
                            tags::answer::Read::Picks { picks, dropped } => {
                                (picks, dropped, TagVerdict::Tagged)
                            }
                            tags::answer::Read::NoneFits { dropped } => {
                                (Vec::new(), dropped, TagVerdict::None)
                            }
                            tags::answer::Read::NoTag(_) => unreachable!(),
                        };
                        let counts = (u64::try_from(picks.len()).unwrap_or(u64::MAX), dropped);
                        let seen = tags::plan::Seen {
                            answered: planned.answered.as_ref(),
                            passed: planned.passed,
                            added: planned.added || !self.notes.is_empty(),
                            asking,
                        };
                        let names = tags::plan::names(&seen, list);
                        self.spool.keep(&tags::block::write(&tags::block::Block {
                            picks,
                            takes_back: context.redo,
                            names,
                            ..block
                        }))?;
                        self.answered(which)?;
                        self.tags_log(entry, &asked, sent, verdict, line, Some(counts));
                    }
                }
                Step::EndEntry
            }
        };
        Ok(match step {
            Step::Stop(ended) => Some(ended),
            _ => None,
        })
    }

    fn tags_failure(
        &mut self,
        entry: &Due,
        aim: (Which, &str, u64, Slot),
        cause: &Cause,
        usage: Usage,
        (attempt, millis): (Option<u32>, u64),
    ) -> Result<Step, Error> {
        let (which, asked, sent, slot) = aim;
        self.add(usage);
        self.counts.failed += 1;
        self.events.push(Event::TagsFailed {
            entry: entry.parent.id,
            cause: cause.clone(),
        });
        let line = Line {
            verdict: Verdict::Failed,
            attempt,
            answered: None,
            usage,
            tallies: None,
            cause: Some(cause.clone()),
            reason: None,
            millis,
        };
        self.tags_log(entry, asked, sent, TagVerdict::Failed, line, None);
        self.fails(slot, which, cause)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replayed {
    pub captured: u64,
    pub tagged: u64,
    pub notices: Vec<String>,
}

pub fn replay(
    entries: &mut dyn Entries,
    spool: &mut dyn Spool,
    format: &Format<'_>,
    asked: &mut dyn TagsAsked,
    cap: usize,
) -> Result<Replayed, Box<Interrupted>> {
    let mut replayed = Replayed::default();
    match replay_into(entries, spool, format, asked, cap, &mut replayed) {
        Ok(()) => Ok(replayed),
        Err(error) => Err(Box::new(Interrupted {
            error,
            notices: replayed.notices,
        })),
    }
}

fn replay_into(
    entries: &mut dyn Entries,
    spool: &mut dyn Spool,
    format: &Format<'_>,
    asked: &mut dyn TagsAsked,
    cap: usize,
    replayed: &mut Replayed,
) -> Result<(), Error> {
    let mut blocks: Vec<(String, Block)> = Vec::new();
    let mut tagged: Vec<(String, tags::block::Block)> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for (name, bytes) in spool.list()? {
        match spool::read(&bytes) {
            Ok(block) => blocks.push((name, block)),
            Err(_) => match tags::block::read(&bytes) {
                Ok(block) => tagged.push((name, block)),
                Err(_) => unreadable.push(name),
            },
        }
    }
    let mut parents: Vec<Id> = Vec::new();
    for parent in blocks
        .iter()
        .map(|(_, block)| block.parent)
        .chain(tagged.iter().map(|(_, block)| block.entry))
    {
        if !parents.contains(&parent) {
            parents.push(parent);
        }
    }
    for parent in parents {
        let mine: Vec<&(String, Block)> = blocks
            .iter()
            .filter(|(_, block)| block.parent == parent)
            .collect();
        let mut ours: Vec<&(String, tags::block::Block)> = tagged
            .iter()
            .filter(|(_, block)| block.entry == parent)
            .collect();
        let mut from = 0;
        while from < mine.len() {
            let start = &mine[from].1;
            let same = |block: &Block| {
                block.generation_before == start.generation_before
                    && block.records_before == start.records_before
                    && block.opens == start.opens
            };
            let to = mine[from..]
                .iter()
                .position(|(_, block)| !same(block))
                .map_or(mine.len(), |at| from + at);
            let group = &mine[from..to];
            match replay_group(entries, format, group)? {
                Group::Captured => replayed.captured += 1,
                Group::Held => {}
                Group::Moved => {
                    replayed.notices.push(format!(
                        "dropped a run kept for {parent}: the store holds other notes for it now"
                    ));
                    for (name, _) in std::mem::take(&mut ours) {
                        spool.remove(name)?;
                        replayed.notices.push(format!(
                            "dropped the tags kept for {parent}: its notes changed since they were asked"
                        ));
                    }
                }
            }
            for (name, _) in group {
                spool.remove(name)?;
            }
            from = to;
        }
        for (name, block) in ours {
            replay_tags(entries, format, asked, cap, block, replayed)?;
            spool.remove(name)?;
        }
    }
    for name in unreadable {
        spool.remove(&name)?;
        replayed
            .notices
            .push(format!("dropped a spool file bilbo cannot read: {name}"));
    }
    Ok(())
}

fn replay_tags(
    entries: &mut dyn Entries,
    format: &Format<'_>,
    asked: &mut dyn TagsAsked,
    cap: usize,
    block: &tags::block::Block,
    replayed: &mut Replayed,
) -> Result<(), Error> {
    let id = block.entry;
    let drop = |replayed: &mut Replayed, why: &str| {
        replayed
            .notices
            .push(format!("dropped the tags kept for {id}: {why}"));
    };
    let Some(parent) = plan::Parent::of(&*entries, id)? else {
        drop(replayed, "the store no longer holds it");
        return Ok(());
    };
    if (parent.generation, parent.records) != (block.about.generation, block.about.records) {
        drop(replayed, "the entry changed since they were asked");
        return Ok(());
    }
    let head = plan::held(&*entries, id)?;
    let digest = match &head {
        Some(head) => plan::digest_of(&*entries, head, format)?,
        None => None,
    };
    let Some(head) = head.filter(|_| digest == Some(block.about.notes)) else {
        drop(replayed, "its notes changed since they were asked");
        return Ok(());
    };
    if block.passed {
        asked.pass(id, block.asking)?;
        return Ok(());
    }
    let declared = match entry_tag::declare(
        id,
        &entry_tag::Change::Model {
            picks: &block.picks,
            takes_back: block.takes_back,
        },
        cap,
        entries,
    ) {
        Ok(declared) => declared,
        Err(entry_tag::Error::Store(error)) => return Err(Error::Store(error)),
        Err(_) => {
            drop(replayed, "the store no longer holds it");
            return Ok(());
        }
    };
    if declared.wrote {
        replayed.tagged += 1;
    }
    if declared.applied.left_off > 0 {
        replayed.notices.push(format!(
            "left {} tags off {id}: tag.cap is {cap}",
            declared.applied.left_off
        ));
    }
    asked.answer(
        &tags::asked::Answered {
            entry: id,
            notes: (head.generation, head.records),
            instruction: block.instruction.clone(),
            provider: block.provider.clone(),
            model: block.model.clone(),
            names: block.names.clone(),
        },
        block.asking,
    )?;
    Ok(())
}

enum Group {
    Captured,
    Held,
    Moved,
}

fn replay_group(
    entries: &mut dyn Entries,
    format: &Format<'_>,
    group: &[&(String, Block)],
) -> Result<Group, Error> {
    let first = &group[0].1;
    let ours: Vec<Vec<u8>> = group
        .iter()
        .flat_map(|(_, block)| block.records.iter().cloned())
        .collect();
    let label = plan::label(first.parent);
    let head = plan::held(&*entries, first.parent)?;
    let state = head.map(|head| (head.generation, head.records));
    let expected = first
        .generation_before
        .map(|generation| (generation, first.records_before));
    if state == expected {
        let mut kept = match &head {
            Some(head) if !first.opens => plan::records_of(entries, head)?,
            _ => Vec::new(),
        };
        kept.extend(ours);
        let mut bytes = Vec::new();
        let mut ends = Vec::new();
        for record in &kept {
            bytes.extend_from_slice(record);
            bytes.push(b'\n');
            ends.push(bytes.len());
        }
        entries.capture(&Capture {
            label: &label,
            kind: format.kind,
            id: None,
            id_label: Some(first.parent),
            stamp: None,
            bytes: &bytes,
            ends: &ends,
            tags: &Tags::default(),
        })?;
        return Ok(Group::Captured);
    }
    let Some(head) = head else {
        return Ok(Group::Moved);
    };
    let stored = plan::records_of(entries, &head)?;
    let holds = match expected {
        None => stored.ends_with(&ours),
        Some((generation, _)) if first.opens => {
            head.generation != generation && stored.ends_with(&ours)
        }
        Some((generation, _)) => {
            let at = usize::try_from(first.records_before).unwrap_or(usize::MAX);
            head.generation == generation
                && stored.get(at..).is_some_and(|tail| tail.ends_with(&ours))
        }
    };
    Ok(if holds { Group::Held } else { Group::Moved })
}
