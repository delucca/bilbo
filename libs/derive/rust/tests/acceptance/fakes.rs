use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use bilbo_derive::instruction::{self, Instruction, Origin};
use bilbo_derive::keys::{self, Section};
use bilbo_derive::log::Log;
use bilbo_derive::plan::{self, Asked, Plan, Ports, Rules};
use bilbo_derive::record::{self, Run};
use bilbo_derive::run::{self, Clock, Context, Replayed, Report, Sinks};
use bilbo_derive::scope::{Named, Scope, Standing};
use bilbo_derive::spool::{self, Block, Spool};
use bilbo_derive::tries::{Extent, Tries};
use bilbo_derive::view::{Counter, Format, Paragraph};
use bilbo_entries::reader::{Passage, Reader};
use bilbo_entries::tokens::{Estimate, Tokens};
use bilbo_llm::model::{Cause, Limits, Model, Outcome, Prompt, Usage, What};
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Kept, Mark, Meta,
    RecordRange, StoreError,
};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{CaptureLabel, StoredLabels, Tags};
use bilbo_store::sealed::Sealed;
use serde_json::{Value, json};

pub const EPOCH_AT: u64 = 1_700_000_000;
pub const NOW: u64 = 2_000_000_000;

pub fn id(n: u8) -> Id {
    let mut bytes = [0; 16];
    bytes[15] = n;
    Id::from_bytes(bytes)
}

fn minted(n: usize) -> Id {
    let mut bytes = [0; 16];
    bytes[0] = 0x10;
    bytes[15] = u8::try_from(n).unwrap_or(u8::MAX);
    Id::from_bytes(bytes)
}

pub fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

struct Item {
    id: Id,
    kind: String,
    capture: Option<CaptureLabel>,
    belongs: Option<Id>,
    generations: Vec<Vec<Vec<u8>>>,
}

#[derive(Default)]
pub struct Shelf {
    items: Vec<Item>,
    sealed: Vec<(Id, SystemTime)>,
    pub reads: RefCell<Vec<Id>>,
    pub touched: Rc<Cell<usize>>,
    pub captures: Vec<(String, String, Option<Id>, Vec<u8>)>,
    pub refuse_capture: bool,
}

impl Shelf {
    pub fn put(&mut self, id: Id, kind: &str, records: &[&str]) {
        self.items.push(Item {
            id,
            kind: kind.to_owned(),
            capture: None,
            belongs: None,
            generations: vec![records.iter().map(|text| line(text)).collect()],
        });
    }

    pub fn belong(&mut self, id: Id, to: Id) {
        self.item_mut(id).belongs = Some(to);
    }

    pub fn grow(&mut self, id: Id, records: &[&str]) {
        let item = self.item_mut(id);
        let newest = item.generations.last_mut().expect("a generation");
        newest.extend(records.iter().map(|text| line(text)));
    }

    pub fn regenerate(&mut self, id: Id, records: &[&str]) {
        self.item_mut(id)
            .generations
            .push(records.iter().map(|text| line(text)).collect());
    }

    pub fn put_notes(&mut self, id: Id, parent: Id, records: Vec<Vec<u8>>) {
        self.items.push(Item {
            id,
            kind: "summary".to_owned(),
            capture: CaptureLabel::new(format!("derive:{parent}")),
            belongs: Some(parent),
            generations: vec![
                records
                    .into_iter()
                    .map(|record| with_line(&record))
                    .collect(),
            ],
        });
    }

    pub fn seal(&mut self, id: Id, when: SystemTime) {
        self.sealed.retain(|(held, _)| *held != id);
        self.sealed.push((id, when));
    }

    pub fn unseal(&mut self, id: Id) {
        self.sealed.retain(|(held, _)| *held != id);
    }

    pub fn ids(&self) -> Vec<Id> {
        self.items.iter().map(|item| item.id).collect()
    }

    pub fn notes_of(&self, parent: Id) -> Option<&Vec<Vec<Vec<u8>>>> {
        let label = CaptureLabel::new(format!("derive:{parent}"));
        self.items
            .iter()
            .find(|item| item.capture == label)
            .map(|item| &item.generations)
    }

    pub fn reads_of(&self, id: Id) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|held| **held == id)
            .count()
    }

    fn item_mut(&mut self, id: Id) -> &mut Item {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .expect("an entry")
    }

    fn touch(&self) {
        self.touched.set(self.touched.get() + 1);
    }
}

fn line(text: &str) -> Vec<u8> {
    format!("{text}\n").into_bytes()
}

fn with_line(record: &[u8]) -> Vec<u8> {
    let mut bytes = record.to_vec();
    bytes.push(b'\n');
    bytes
}

impl Entries for Shelf {
    fn capture(&mut self, input: &Capture<'_>) -> Result<Captured, CaptureError> {
        if self.refuse_capture {
            return Err(CaptureError::TooLarge);
        }
        self.touch();
        self.captures.push((
            input.label.to_string(),
            input.kind.to_owned(),
            input.id_label,
            input.bytes.to_vec(),
        ));
        let mut records = Vec::new();
        let mut start = 0;
        for &end in input.ends {
            records.push(input.bytes[start..end].to_vec());
            start = end;
        }
        let found = self
            .items
            .iter()
            .position(|item| item.capture.as_ref() == Some(input.label));
        let Some(position) = found else {
            let made = minted(self.items.len());
            self.items.push(Item {
                id: made,
                kind: input.kind.to_owned(),
                capture: Some(input.label.clone()),
                belongs: input.id_label,
                generations: vec![records.clone()],
            });
            return Ok(Captured::New(kept(made, 1, 1, records.len())));
        };
        let item = &mut self.items[position];
        let newest = item.generations.last_mut().expect("a generation");
        let stored: Vec<u8> = newest.concat();
        let made = item.id;
        if input.bytes.starts_with(&stored) && !newest.is_empty() {
            if input.bytes.len() == stored.len() {
                return Ok(Captured::Unchanged(made));
            }
            let first = newest.len() + 1;
            let added = records.split_off(newest.len());
            let count = added.len();
            newest.extend(added);
            let generation = item.generations.len();
            return Ok(Captured::Appended(kept(made, generation, first, count)));
        }
        if newest.is_empty() {
            *newest = records.clone();
            let generation = item.generations.len();
            return Ok(Captured::Appended(kept(made, generation, 1, records.len())));
        }
        item.generations.push(records.clone());
        let generation = item.generations.len();
        Ok(Captured::NewGeneration(kept(
            made,
            generation,
            1,
            records.len(),
        )))
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
        self.touch();
        Ok(0)
    }

    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.touch();
        self.reads.borrow_mut().push(id);
        let item = self
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| StoreError::Missing(format!("no entry {id}")))?;
        let held = &item.generations[generation.0 as usize - 1];
        let from = records.first as usize - 1;
        Ok(held[from..from + records.count as usize].to_vec())
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        self.touch();
        Ok(self
            .items
            .iter()
            .find(|item| item.id == id)
            .map(|item| Meta {
                id,
                kind: item.kind.clone(),
                labels: StoredLabels {
                    capture: item.capture.clone(),
                    id: item.belongs,
                    tags: Tags::default(),
                },
                captured: SystemTime::UNIX_EPOCH,
            }))
    }

    fn changes(&self, _since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        self.touch();
        Ok((Vec::new(), Cursor::start()))
    }

    fn by_capture(&self, label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        self.touch();
        Ok(self
            .items
            .iter()
            .find(|item| item.capture.as_ref() == Some(label))
            .map(|item| Head {
                id: item.id,
                generation: Generation(item.generations.len() as u32),
                records: item.generations.last().map_or(0, Vec::len) as u64,
                mark: Mark { end: 0, hash: 0 },
                stamp: None,
            }))
    }

    fn extent(
        &self,
        id: Id,
        generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        self.touch();
        let Some(item) = self.items.iter().find(|item| item.id == id) else {
            return Ok(None);
        };
        let number = generation.map_or(item.generations.len(), |held| held.0 as usize);
        Ok(item
            .generations
            .get(number.wrapping_sub(1))
            .map(|held| (Generation(number as u32), held.len() as u64)))
    }

    fn captures_with(&self, _part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        Ok(Vec::new())
    }

    fn by_shorts(&self, _shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        Ok(Vec::new())
    }
}

fn kept(id: Id, generation: usize, first: usize, count: usize) -> Kept {
    Kept {
        id,
        generation: Generation(generation as u32),
        records: RecordRange {
            first: first as u64,
            count: count as u64,
        },
        raw_bytes: 0,
        stored_bytes: 0,
    }
}

impl Sealed for Shelf {
    fn sealed_at(&self, id: Id) -> Result<Option<SystemTime>, StoreError> {
        self.touch();
        Ok(self
            .sealed
            .iter()
            .find(|(held, _)| *held == id)
            .map(|(_, when)| *when))
    }
}

pub struct Plain {
    pub kind: &'static str,
    pub appends_only: bool,
    pub silent: &'static str,
    pub preface: Option<&'static str>,
}

impl Reader for Plain {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                let text = String::from_utf8_lossy(record).trim_end().to_owned();
                if !self.silent.is_empty() && text.starts_with(self.silent) {
                    return None;
                }
                let number = index as u64 + 1;
                Some(Passage {
                    range: RecordRange {
                        first: number,
                        count: 1,
                    },
                    pieces: Vec::new(),
                    ends: vec![text.len()],
                    text,
                    at: Some(at(EPOCH_AT + number)),
                    outline: None,
                })
            })
            .collect()
    }

    fn appends_only(&self) -> bool {
        self.appends_only
    }

    fn preface(&self, _records: &[&[u8]]) -> Option<String> {
        self.preface.map(str::to_owned)
    }
}

pub fn readers() -> Vec<Box<dyn Reader>> {
    vec![
        Box::new(Plain {
            kind: "plain",
            appends_only: true,
            silent: "",
            preface: None,
        }),
        Box::new(Plain {
            kind: "folded",
            appends_only: true,
            silent: "-",
            preface: None,
        }),
        Box::new(Plain {
            kind: "rewrite",
            appends_only: false,
            silent: "",
            preface: None,
        }),
        Box::new(Plain {
            kind: "headed",
            appends_only: true,
            silent: "",
            preface: Some("A header line."),
        }),
    ]
}

pub fn write_paragraph(paragraph: &Paragraph) -> Vec<u8> {
    let when = paragraph.at.map(|time| {
        time.duration_since(SystemTime::UNIX_EPOCH)
            .expect("after")
            .as_secs()
    });
    json!({
        "text": paragraph.text,
        "at": when,
        "from": [paragraph.source.first, paragraph.source.last],
        "of": paragraph.source.parent.to_string(),
        "gen": paragraph.source.generation.0,
    })
    .to_string()
    .into_bytes()
}

pub fn read_paragraph(bytes: &[u8]) -> Option<Paragraph> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let [first, last] = value.get("from")?.as_array()?.as_slice() else {
        return None;
    };
    Some(Paragraph {
        text: value.get("text")?.as_str()?.to_owned(),
        at: value.get("at")?.as_u64().map(at),
        source: Extent {
            parent: Id::parse(value.get("of")?.as_str()?)?,
            generation: Generation(u32::try_from(value.get("gen")?.as_u64()?).ok()?),
            first: first.as_u64()?,
            last: last.as_u64()?,
        },
    })
}

pub fn format() -> Format<'static> {
    Format {
        kind: "summary",
        write: &write_paragraph,
        read: &read_paragraph,
    }
}

#[derive(Default)]
pub struct Table {
    pub names: Vec<(String, Vec<Id>)>,
    pub several: Vec<(String, Vec<Id>)>,
    pub standing: Option<Standing>,
}

impl Scope for Table {
    fn named(&self, values: &[String]) -> Result<Named, StoreError> {
        let mut named = Named::default();
        for value in values {
            if let Some((_, ids)) = self.names.iter().find(|(name, _)| name == value) {
                named.entries.extend(ids);
            }
            if let Some((_, ids)) = self.several.iter().find(|(name, _)| name == value) {
                named.several.extend(ids);
            }
        }
        named.entries.sort();
        named.entries.dedup();
        named.several.sort();
        named.several.dedup();
        Ok(named)
    }

    fn standing(&self) -> Result<Standing, StoreError> {
        Ok(self
            .standing
            .clone()
            .unwrap_or_else(|| Standing::NotCurrent(Vec::new())))
    }
}

#[derive(Default)]
pub struct Held {
    pub charges: Vec<(Extent, u8)>,
    pub mark: Option<Id>,
}

impl Held {
    pub fn of(&self, extent: &Extent) -> u8 {
        self.charges
            .iter()
            .find(|(held, _)| held == extent)
            .map_or(0, |(_, charges)| *charges)
    }
}

impl Tries for Held {
    fn charges(&self, extent: &Extent) -> io::Result<u8> {
        Ok(self.of(extent))
    }

    fn charge(&mut self, extent: &Extent) -> io::Result<()> {
        match self.charges.iter_mut().find(|(held, _)| held == extent) {
            Some((_, charges)) => *charges += 1,
            None => self.charges.push((*extent, 1)),
        }
        Ok(())
    }

    fn forget(&mut self, extent: &Extent) -> io::Result<()> {
        self.charges.retain(|(held, _)| held != extent);
        Ok(())
    }

    fn mark(&self) -> io::Result<Option<Id>> {
        Ok(self.mark)
    }

    fn set_mark(&mut self, entry: Option<Id>) -> io::Result<()> {
        self.mark = entry;
        Ok(())
    }
}

#[derive(Default)]
pub struct Spooled {
    pub files: Vec<(String, Vec<u8>)>,
    pub made: usize,
    pub removals: usize,
    pub fail_removal: Option<usize>,
}

impl Spool for Spooled {
    fn keep(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.made += 1;
        self.files
            .push((format!("s{:04}", self.made), bytes.to_vec()));
        Ok(())
    }

    fn list(&self) -> io::Result<Vec<(String, Vec<u8>)>> {
        Ok(self.files.clone())
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        self.removals += 1;
        if self.fail_removal == Some(self.removals) {
            return Err(io::Error::other("a removal that fails"));
        }
        self.files.retain(|(held, _)| held != name);
        Ok(())
    }
}

impl Spooled {
    pub fn blocks(&self) -> Vec<Block> {
        self.files
            .iter()
            .map(|(_, bytes)| spool::read(bytes).expect("a block"))
            .collect()
    }
}

#[derive(Default)]
pub struct Lines(pub Vec<String>);

impl Log for Lines {
    fn append(&mut self, line: &str) {
        self.0.push(line.to_owned());
    }
}

pub struct Watch {
    pub now: Cell<SystemTime>,
    pub slept: RefCell<Vec<Duration>>,
}

impl Clock for Watch {
    fn now(&self) -> SystemTime {
        self.now.get()
    }

    fn sleep(&self, wait: Duration) {
        self.slept.borrow_mut().push(wait);
        self.now.set(self.now.get() + wait);
    }
}

pub type Rule = Box<dyn FnMut(&Prompt, usize) -> Outcome>;

pub struct Script {
    pub model: String,
    pub queue: VecDeque<Outcome>,
    pub rule: Option<Rule>,
    pub prompts: Vec<Prompt>,
    pub limits: Vec<Limits>,
    pub prepared: usize,
    pub refuse_prepare: Option<String>,
}

impl Script {
    pub fn new() -> Script {
        Script {
            model: "model-a".to_owned(),
            queue: VecDeque::new(),
            rule: None,
            prompts: Vec::new(),
            limits: Vec::new(),
            prepared: 0,
            refuse_prepare: None,
        }
    }

    pub fn then(mut self, outcome: Outcome) -> Script {
        self.queue.push_back(outcome);
        self
    }

    pub fn always(mut self, rule: impl FnMut(&Prompt, usize) -> Outcome + 'static) -> Script {
        self.rule = Some(Box::new(rule));
        self
    }
}

impl Model for Script {
    fn prepare(&mut self) -> Result<(), String> {
        self.prepared += 1;
        self.refuse_prepare.clone().map_or(Ok(()), Err)
    }

    fn call(&mut self, prompt: &Prompt, limits: &Limits) -> Outcome {
        self.prompts.push(prompt.clone());
        self.limits.push(*limits);
        let called = self.prompts.len();
        if let Some(outcome) = self.queue.pop_front() {
            return outcome;
        }
        match self.rule.as_mut() {
            Some(rule) => rule(prompt, called),
            None => panic!("the writer was called more often than the test scripted"),
        }
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn describe(&self) -> Vec<String> {
        vec!["a scripted writer".to_owned()]
    }
}

pub fn usage(read: u64, written: u64) -> Usage {
    Usage {
        read: Some(read),
        written: Some(written),
    }
}

pub fn answer(text: &str) -> Outcome {
    Outcome::Answer {
        text: text.to_owned(),
        usage: usage(100, 10),
        model: Some("model-answered".to_owned()),
    }
}

pub fn no_note() -> Outcome {
    answer("I could not write notes.")
}

pub fn cut() -> Outcome {
    Outcome::Cut(usage(100, 50))
}

pub fn failed(what: What) -> Outcome {
    Outcome::Failed {
        cause: Cause::new(what),
        usage: Usage::default(),
    }
}

pub fn refused(category: Option<&str>) -> Outcome {
    Outcome::Refused {
        category: category.map(str::to_owned),
        usage: usage(100, 5),
    }
}

pub fn busy(wait: Option<Duration>) -> Outcome {
    Outcome::Busy {
        wait,
        cause: Cause::new(What::RateLimit),
    }
}

pub fn stopped() -> Outcome {
    Outcome::Stopped(Cause::new(What::Unauthorized))
}

pub struct World {
    pub shelf: Shelf,
    pub table: Table,
    pub held: Held,
    pub spooled: Spooled,
    pub lines: Lines,
    pub watch: Watch,
    pub section: Section,
    pub limits: Limits,
    pub instructions: Vec<Instruction>,
    pub bound: usize,
    pub redo: bool,
    pub readers: Vec<Box<dyn Reader>>,
    pub tokens: Box<dyn Tokens>,
    next: u8,
}

pub struct Done {
    pub report: Report,
    pub replayed: Replayed,
}

pub fn instruction_for(kind: &str) -> Instruction {
    let text = format!("Write notes about {kind}.\n");
    Instruction {
        kind: kind.to_owned(),
        id: instruction::id(&text),
        text,
        origin: Origin::Shipped,
    }
}

impl World {
    pub fn new() -> World {
        let mut section = keys::defaults();
        section.enabled.value = true;
        section.named.value = vec!["in".to_owned()];
        section.left_out.value = vec!["out".to_owned()];
        section.quiet.value = Duration::from_secs(30 * 60);
        let mut table = Table::default();
        table.names.push(("in".to_owned(), Vec::new()));
        table.names.push(("out".to_owned(), Vec::new()));
        World {
            shelf: Shelf::default(),
            table,
            held: Held::default(),
            spooled: Spooled::default(),
            lines: Lines::default(),
            watch: Watch {
                now: Cell::new(at(NOW)),
                slept: RefCell::new(Vec::new()),
            },
            section,
            limits: Limits {
                answer_tokens: 16_000,
                timeout: Duration::from_secs(300),
                response: 4 << 20,
                poll: Duration::from_millis(5),
            },
            instructions: ["plain", "folded", "rewrite", "headed"]
                .into_iter()
                .map(instruction_for)
                .collect(),
            bound: 1 << 20,
            redo: false,
            readers: readers(),
            tokens: Box::new(Estimate),
            next: 0,
        }
    }

    pub fn starting_at(offset: u8) -> World {
        World {
            next: offset,
            ..World::new()
        }
    }

    pub fn entry(&mut self, kind: &str, records: &[&str]) -> Id {
        self.next += 1;
        let made = id(self.next);
        self.shelf.put(made, kind, records);
        self.shelf.seal(made, at(NOW - 86_400));
        self.table.names[0].1.push(made);
        made
    }

    pub fn note_entry(&mut self, parent: Id, records: Vec<Vec<u8>>) -> Id {
        let made = minted(self.shelf.ids().len());
        self.shelf.put_notes(made, parent, records);
        made
    }

    pub fn name(&mut self, list: &str, ids: &[Id]) {
        let row = self
            .table
            .names
            .iter_mut()
            .find(|(name, _)| name == list)
            .expect("a list");
        row.1.extend(ids);
    }

    pub fn run_record(&self, parent: Id, first: u64, last: u64) -> Run {
        let generation = self
            .shelf
            .extent(parent, None)
            .expect("extent")
            .expect("generation")
            .0;
        let instruction = instruction_for("plain");
        Run {
            kind: "plain".to_owned(),
            instruction: instruction.id,
            provider: "fake".to_owned(),
            model: "model-a".to_owned(),
            answered: Some("model-answered".to_owned()),
            same: true,
            made: NOW - 1000,
            entry: parent,
            generation,
            first,
            last,
            total: last,
            sent: 10,
            tokens_in: Some(100),
            tokens_out: Some(10),
            written: 1,
            kept: 1,
            range: 0,
            names: 0,
            passes: 0,
            passed: false,
        }
    }

    pub fn paragraph(&self, parent: Id, first: u64, last: u64, text: &str) -> Paragraph {
        let generation = self
            .shelf
            .extent(parent, None)
            .expect("extent")
            .expect("generation")
            .0;
        Paragraph {
            text: text.to_owned(),
            at: Some(at(EPOCH_AT + first)),
            source: Extent {
                parent,
                generation,
                first,
                last,
            },
        }
    }

    pub fn notes(&mut self, parent: Id, covers: u64, edit: impl FnOnce(&mut Run)) -> Id {
        self.notes_saying(parent, covers, "Earlier paragraph.", edit)
    }

    pub fn notes_saying(
        &mut self,
        parent: Id,
        covers: u64,
        text: &str,
        edit: impl FnOnce(&mut Run),
    ) -> Id {
        let mut run = self.run_record(parent, 1, covers);
        edit(&mut run);
        let paragraph = self.paragraph(parent, 1, covers, text);
        let records = vec![
            write_paragraph(&paragraph),
            record::write(&run).into_bytes(),
        ];
        let made = minted(self.shelf.ids().len());
        self.shelf.put_notes(made, parent, records);
        made
    }

    pub fn plan(&self, asked: Asked<'_>) -> Plan {
        let format = format();
        let readers: Vec<&dyn Reader> = self.readers.iter().map(|reader| &**reader).collect();
        plan::plan(
            &Ports {
                entries: &self.shelf,
                sealed: &self.shelf,
                scope: &self.table,
                tries: &self.held,
            },
            &Rules {
                readers: &readers,
                instructions: &self.instructions,
                section: &self.section,
                format: &format,
                tokens: &*self.tokens,
                bound: self.bound,
                model: "model-a",
                provider: "fake",
                redo: self.redo,
                now: self.watch.now.get(),
            },
            &self.shelf.ids(),
            asked,
        )
        .expect("a plan")
    }

    pub fn calls(&mut self, writer: &mut Script, plan: &Plan) -> Report {
        let format = format();
        let counter = Counter {
            tokens: &*self.tokens,
            bound: self.bound,
        };
        run::calls(
            &plan.due,
            Sinks {
                writer,
                spool: &mut self.spooled,
                tries: &mut self.held,
                log: &mut self.lines,
                clock: &self.watch,
            },
            &Context {
                section: &self.section,
                provider: "fake",
                limits: self.limits,
                format: &format,
                counter: &counter,
            },
        )
        .expect("calls")
    }

    pub fn replay(&mut self) -> Replayed {
        let format = format();
        run::replay(&mut self.shelf, &mut self.spooled, &format).expect("replay")
    }

    pub fn derive(&mut self, writer: &mut Script) -> Done {
        let plan = self.plan(Asked::Every);
        let report = self.calls(writer, &plan);
        let replayed = self.replay();
        Done { report, replayed }
    }

    pub fn extent_of(&self, parent: Id, first: u64, last: u64) -> Extent {
        let generation = self
            .shelf
            .extent(parent, None)
            .expect("extent")
            .expect("generation")
            .0;
        Extent {
            parent,
            generation,
            first,
            last,
        }
    }
}

pub struct Longest(pub std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Tokens for Longest {
    fn ends(&self, text: &str) -> Vec<usize> {
        self.see(text);
        Estimate.ends(text)
    }

    fn count(&self, text: &str) -> usize {
        self.see(text);
        Estimate.count(text)
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        self.see(text);
        Estimate.prefix(text, budget)
    }
}

impl Longest {
    fn see(&self, text: &str) {
        self.0
            .fetch_max(text.len(), std::sync::atomic::Ordering::Relaxed);
    }
}
