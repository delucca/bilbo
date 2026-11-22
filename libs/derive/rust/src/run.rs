use std::time::{Duration, SystemTime};

use bilbo_store::entries::{Capture, Entries};
use bilbo_store::id::Id;
use bilbo_store::opaque;

use crate::answer;
use crate::check::check;
use crate::keys::Section;
use crate::log::{self, Call, Log, Verdict};
use crate::plan::{self, Due, Error, Planned};
use crate::record::{self, Run, next_same};
use crate::spool::{self, Block, Spool, bare};
use crate::tries::{Extent, Tries};
use crate::view::{self, Counter, Format, Paragraph};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Done,
    Calls,
    Failures,
    Busy(Cause, u32),
    Wait(Duration, Cause),
    Stopped(Cause),
    Prepare(String),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub counts: Counts,
    pub ended: Ended,
    pub events: Vec<Event>,
}

pub struct Sinks<'a> {
    pub writer: &'a mut dyn Model,
    pub spool: &'a mut dyn Spool,
    pub tries: &'a mut dyn Tries,
    pub log: &'a mut dyn Log,
    pub clock: &'a dyn Clock,
}

pub struct Context<'a> {
    pub section: &'a Section,
    pub format: &'a Format<'a>,
    pub counter: &'a Counter<'a>,
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
            .field("format", &self.format)
            .field("counter", &self.counter)
            .finish()
    }
}

enum Step {
    Worked(Vec<Paragraph>, Run),
    EndEntry,
    Stop(Ended),
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

struct Machine<'a> {
    writer: &'a mut dyn Model,
    spool: &'a mut dyn Spool,
    tries: &'a mut dyn Tries,
    log: &'a mut dyn Log,
    clock: &'a dyn Clock,
    context: &'a Context<'a>,
    counts: Counts,
    attempts: usize,
    streak: usize,
    waiting: Vec<Extent>,
    answered: bool,
    events: Vec<Event>,
    counted: Option<Id>,
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
        spool,
        tries,
        log,
        clock,
    } = sinks;
    if due
        .iter()
        .any(|entry| entry.windows.iter().any(|planned| !planned.pass))
        && let Err(text) = writer.prepare()
    {
        return Ok(Report {
            counts: Counts::default(),
            ended: Ended::Prepare(text),
            events: Vec::new(),
        });
    }
    let mut machine = Machine {
        writer,
        spool,
        tries,
        log,
        clock,
        context,
        counts: Counts::default(),
        attempts: 0,
        streak: 0,
        waiting: Vec::new(),
        answered: false,
        events: Vec::new(),
        counted: None,
    };
    let first = due
        .iter()
        .find(|entry| entry.windows.iter().any(|planned| !planned.pass))
        .map(|entry| entry.parent.id);
    let ended = match machine.work(due).and_then(|ended| {
        if matches!(ended, Ended::Done | Ended::Calls) && machine.answered {
            machine.charge_waiting()?;
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
        Ok(None)
    }

    fn record(
        &self,
        entry: &Due,
        planned: &Planned,
        previous: Option<&Run>,
        made: Made,
        sent: u64,
    ) -> Run {
        let section = self.context.section;
        let model = if made.passed {
            String::new()
        } else {
            self.writer.id().to_owned()
        };
        let provider = section.provider.value.clone();
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
        if self.answered {
            self.tries.set_mark(None)?;
        } else if self.counts.calls > 0 {
            self.tries.set_mark(first)?;
        }
        Ok(())
    }

    fn charge_waiting(&mut self) -> Result<(), Error> {
        for extent in std::mem::take(&mut self.waiting) {
            self.tries.charge(&extent)?;
        }
        Ok(())
    }

    fn answered(&mut self) -> Result<(), Error> {
        self.answered = true;
        self.streak = 0;
        self.charge_waiting()
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
            provider: self.context.section.provider.value.clone(),
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
        let limits = Limits {
            answer_tokens: section.answer_tokens.value,
            timeout: section.timeout.value,
            response: section.response.value,
            poll: section.poll.value,
        };
        let sent = size_of_window(planned);
        let mut retries = 0u32;
        loop {
            if self.attempts >= section.calls.value {
                return Ok(Step::Stop(Ended::Calls));
            }
            self.attempts += 1;
            self.counts.calls += 1;
            if self.counted != Some(entry.parent.id) {
                self.counted = Some(entry.parent.id);
                self.counts.entries += 1;
            }
            let attempt = Some(retries + 1);
            let started = self.clock.now();
            let outcome = self.writer.call(&prompt, &limits);
            let millis = self.clock.now().duration_since(started).map_or(0, |taken| {
                u64::try_from(taken.as_millis()).unwrap_or(u64::MAX)
            });
            match outcome {
                Outcome::Answer { text, usage, model } => {
                    self.add(usage);
                    return self.answer(
                        entry,
                        planned,
                        previous,
                        &grounds,
                        (text, usage, model),
                        (attempt, millis),
                    );
                }
                Outcome::Cut(usage) => {
                    return self.failure(
                        entry,
                        planned,
                        Cause::new(What::Cut),
                        usage,
                        (attempt, millis),
                    );
                }
                Outcome::Failed { cause, usage } => {
                    return self.failure(entry, planned, cause, usage, (attempt, millis));
                }
                Outcome::Refused { category, usage } => {
                    self.add(usage);
                    self.counts.refused += 1;
                    self.tries.charge(&planned.extent)?;
                    self.answered()?;
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
                    return Ok(Step::EndEntry);
                }
                Outcome::Busy { wait, cause } => {
                    self.log(
                        entry,
                        planned,
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
                        sent,
                    );
                    if retries >= u32::try_from(section.retries.value).unwrap_or(u32::MAX) {
                        let ended = Ended::Busy(cause.clone(), retries + 1);
                        return self.not_asked_again(planned, &cause, ended);
                    }
                    if self.attempts >= section.calls.value {
                        return Ok(Step::Stop(Ended::Calls));
                    }
                    let times = u32::try_from(section.retry_factor.value)
                        .unwrap_or(u32::MAX)
                        .saturating_pow(retries);
                    let delay =
                        wait.unwrap_or_else(|| section.retry_wait.value.saturating_mul(times));
                    if delay > section.retry_limit.value {
                        let ended = Ended::Wait(delay, cause.clone());
                        return self.not_asked_again(planned, &cause, ended);
                    }
                    self.clock.sleep(delay);
                    retries += 1;
                }
                Outcome::Stopped(cause) => {
                    self.log(
                        entry,
                        planned,
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
                        sent,
                    );
                    return self.not_asked_again(planned, &cause, Ended::Stopped(cause.clone()));
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
        self.fails(planned, cause)
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
        self.fails(planned, &failed_cause)
    }

    fn fails(&mut self, planned: &Planned, cause: &Cause) -> Result<Step, Error> {
        match cause.what.whose() {
            Whose::Call => self.ends_charged(planned),
            Whose::Provider => Ok(Step::Stop(Ended::Stopped(cause.clone()))),
            Whose::Either => {
                self.waiting.push(planned.extent);
                self.streak += 1;
                Ok(if self.streak >= self.context.section.failures.value {
                    Step::Stop(Ended::Failures)
                } else {
                    Step::EndEntry
                })
            }
        }
    }

    fn ends_charged(&mut self, planned: &Planned) -> Result<Step, Error> {
        self.tries.charge(&planned.extent)?;
        self.answered()?;
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
                return self.ends_charged(planned);
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
        self.answered()?;
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replayed {
    pub captured: u64,
    pub notices: Vec<String>,
}

pub fn replay(
    entries: &mut dyn Entries,
    spool: &mut dyn Spool,
    format: &Format<'_>,
) -> Result<Replayed, Box<Interrupted>> {
    let mut replayed = Replayed::default();
    match replay_into(entries, spool, format, &mut replayed) {
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
    replayed: &mut Replayed,
) -> Result<(), Error> {
    let mut blocks: Vec<(String, Block)> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for (name, bytes) in spool.list()? {
        match spool::read(&bytes) {
            Ok(block) => blocks.push((name, block)),
            Err(_) => unreadable.push(name),
        }
    }
    let mut parents: Vec<Id> = Vec::new();
    for (_, block) in &blocks {
        if !parents.contains(&block.parent) {
            parents.push(block.parent);
        }
    }
    for parent in parents {
        let mine: Vec<&(String, Block)> = blocks
            .iter()
            .filter(|(_, block)| block.parent == parent)
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
                Group::Moved => replayed.notices.push(format!(
                    "dropped a run kept for {parent}: the store holds other notes for it now"
                )),
            }
            for (name, _) in group {
                spool.remove(name)?;
            }
            from = to;
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
