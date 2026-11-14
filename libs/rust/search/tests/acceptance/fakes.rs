use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bilbo_kinds::reader::{EntryLabels, Passage, Piece, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_search::analyze::{self, Detect, Language};
use bilbo_search::chunk;
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats, millis};
use bilbo_search::lookup::{Filter, Found, Lookup, Posting, Totals};
use bilbo_search::position::Position;
use bilbo_search::rank::{Candidate, Query, Ranked, Ranker};
use bilbo_search::recall::log::QueryLog;
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Meta, RecordRange,
    StoreError,
};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{CaptureLabel, Filed, Labels, Relabel, StoredLabels};

pub const CATALOG: [u8; 16] = [7; 16];

pub type Rows = BTreeMap<Id, Vec<(String, Vec<String>)>>;

pub type Events = Rc<RefCell<Vec<&'static str>>>;

pub fn id(n: u8) -> Id {
    let mut bytes = [0; 16];
    bytes[15] = n;
    Id::from_bytes(bytes)
}

pub fn wide(n: u16) -> Id {
    let mut bytes = [0; 16];
    bytes[14..].copy_from_slice(&n.to_be_bytes());
    Id::from_bytes(bytes)
}

pub fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

pub struct Kept {
    pub id: Id,
    pub kind: &'static str,
    pub captured: SystemTime,
    pub generations: Vec<Vec<&'static str>>,
    pub damaged: bool,
}

pub fn kept(n: u8, kind: &'static str, records: &[&'static str]) -> Kept {
    Kept {
        id: id(n),
        kind,
        captured: at(1000 + u64::from(n)),
        generations: vec![records.to_vec()],
        damaged: false,
    }
}

pub fn shelf(kept: Vec<Kept>) -> Log {
    let mut log = Log::new();
    for entry in kept {
        log.log.push(Change::Meta(entry.id));
        for (at, records) in entry.generations.iter().enumerate() {
            log.records(entry.id, at as u32 + 1, 1, records.len() as u64);
        }
        log.entries.push(Held {
            id: entry.id,
            kind: entry.kind.to_owned(),
            captured: entry.captured,
            capture: None,
            earlier: Vec::new(),
            id_label: None,
            generations: entry
                .generations
                .iter()
                .map(|records| {
                    records
                        .iter()
                        .map(|record| record.strip_suffix('\n').unwrap_or(record).to_owned())
                        .collect()
                })
                .collect(),
            damaged: entry.damaged,
        });
    }
    log
}

fn unused() -> StoreError {
    StoreError::Missing("not called".to_owned())
}

/// One passage a record, its text the record without its line feed. A record `-` renders nothing,
/// a record that starts with `@<seconds> ` carries that time, and a record `!panic` makes it panic.
pub struct Lines(pub &'static str);

impl Reader for Lines {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        let mut passages = Vec::new();
        for (record, bytes) in (1..).zip(records) {
            let line = String::from_utf8_lossy(bytes);
            let line = line.trim_end_matches('\n');
            assert!(line != "!panic", "a reader that panics");
            if line == "!bomb" {
                std::panic::panic_any(Bomb);
            }
            if line == "-" {
                continue;
            }
            let (time, text) = match line.strip_prefix('@').and_then(|rest| rest.split_once(' ')) {
                Some((seconds, text)) => (seconds.parse().ok().map(at), text),
                None => (None, line),
            };
            passages.push(Passage {
                range: RecordRange {
                    first: record,
                    count: 1,
                },
                text: text.to_owned(),
                pieces: vec![Piece {
                    start: 0,
                    label: "line:".to_owned(),
                }],
                ends: vec![text.len()],
                at: time,
                outline: None,
            });
        }
        passages
    }

    fn appends_only(&self) -> bool {
        true
    }
}

pub struct Foldered(pub &'static str);

impl Reader for Foldered {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        Lines(self.0).passages(records)
    }

    fn appends_only(&self) -> bool {
        true
    }

    fn entry_labels(&self, records: &[&[u8]]) -> Vec<String> {
        assert!(
            !records
                .iter()
                .any(|record| record.starts_with(b"!labels-panic")),
            "a reader that panics while it gives labels"
        );
        folders(
            records
                .iter()
                .map(|record| String::from_utf8_lossy(record).into_owned()),
        )
    }
}

pub fn folders(records: impl Iterator<Item = String>) -> Vec<String> {
    let mut labels = EntryLabels::default();
    for record in records {
        if let Some(text) = record.trim_end_matches('\n').strip_prefix("cwd:") {
            labels.add(text);
        }
    }
    labels.into_texts()
}

pub struct Joined(pub &'static str);

impl Reader for Joined {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        let mut text = String::new();
        let mut ends = Vec::new();
        for record in records {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(String::from_utf8_lossy(record).trim_end_matches('\n'));
            ends.push(text.len());
        }
        vec![Passage {
            range: RecordRange {
                first: 1,
                count: records.len() as u64,
            },
            text,
            pieces: vec![Piece {
                start: 0,
                label: "line:".to_owned(),
            }],
            ends,
            at: None,
            outline: None,
        }]
    }

    fn appends_only(&self) -> bool {
        true
    }
}

pub struct Whole(pub &'static str);

impl Reader for Whole {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        Lines(self.0).passages(records)
    }

    fn appends_only(&self) -> bool {
        false
    }
}

struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("a payload that panics when dropped");
    }
}

pub struct Bytes;

impl Tokens for Bytes {
    fn ends(&self, text: &str) -> Vec<usize> {
        (1..=text.len()).collect()
    }
}

#[derive(Default)]
pub struct Watched(pub std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Tokens for Watched {
    fn ends(&self, text: &str) -> Vec<usize> {
        self.0
            .fetch_max(text.len(), std::sync::atomic::Ordering::Relaxed);
        (1..=text.len()).collect()
    }

    fn count(&self, text: &str) -> usize {
        self.0
            .fetch_max(text.len(), std::sync::atomic::Ordering::Relaxed);
        text.len()
    }
}

/// A token a byte, except in a text that holds `PANIC`, which it panics on, or `UNSPLIT`, which it
/// gives no token.
pub struct Marked;

impl Tokens for Marked {
    fn ends(&self, text: &str) -> Vec<usize> {
        assert!(!text.contains("PANIC"), "a tokenizer that panics");
        if text.contains("UNSPLIT") {
            return Vec::new();
        }
        (1..=text.len()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Clear,
    Add {
        entry: Id,
        generation: u32,
        kind: String,
        date: SystemTime,
        first: u64,
        count: u64,
        part: Option<(u32, u32)>,
        text: String,
    },
    Commit(Position),
    Delete {
        entry: Id,
        from: u64,
    },
}

#[derive(Default)]
pub struct Recorded {
    pub rolled_back: Cell<u32>,
    pub calls: RefCell<Vec<Call>>,
    pub fail_at: Option<usize>,
    pub bytes: u64,
    pub events: Events,
}

impl Recorded {
    fn record(&self, call: Call) -> Result<(), IndexError> {
        self.events.borrow_mut().push(match call {
            Call::Clear => "clear",
            Call::Add { .. } => "add",
            Call::Commit(_) => "commit",
            Call::Delete { .. } => "delete",
        });
        let mut calls = self.calls.borrow_mut();
        if self.fail_at == Some(calls.len()) {
            return Err(IndexError::Failed {
                action: "write",
                path: PathBuf::from("/c/index"),
                error: "disk full".to_owned(),
            });
        }
        calls.push(call);
        Ok(())
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    pub fn added(&self) -> Vec<(Id, String)> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Add { entry, text, .. } => Some((entry, text)),
                _ => None,
            })
            .collect()
    }
}

impl Index for Recorded {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        panic!("reindex never asks where the index stands: it makes it again")
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.record(Call::Clear)
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.record(Call::Add {
            entry: doc.entry,
            generation: doc.generation.0,
            kind: doc.kind.to_owned(),
            date: doc.date,
            first: doc.chunk.records.first,
            count: doc.chunk.records.count,
            part: doc.chunk.part,
            text: doc.chunk.text.clone(),
        })
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
        self.record(Call::Commit(position.clone()))?;
        Ok(Stats { bytes: self.bytes })
    }

    fn extents(&mut self, _entry: Id) -> Result<Vec<Extent>, IndexError> {
        panic!("reindex never asks what the index holds of an entry")
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        self.record(Call::Delete { entry, from })
    }

    fn rollback(&mut self) {
        self.rolled_back.set(self.rolled_back.get() + 1);
    }
}

pub fn unread(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
    pairs
        .iter()
        .map(|(kind, entries)| ((*kind).to_owned(), *entries))
        .collect()
}

/// A store that keeps a log of its changes, as the catalog does: the sequence number of a change is
/// its place in the log from 1.
pub struct Log {
    pub catalog: [u8; 16],
    pub entries: Vec<Held>,
    pub log: Vec<Change>,
    pub changes_fail: bool,
    pub reads: RefCell<Vec<Id>>,
    pub shorts: RefCell<Vec<usize>>,
    pub events: Events,
    pub filed: RefCell<Rows>,
    pub relabels: RefCell<Vec<Vec<Filed>>>,
    pub refuse_relabel: Cell<Option<usize>>,
    pub asked_words: RefCell<Vec<Vec<String>>>,
    pub asked_belonging: RefCell<Vec<Vec<Id>>>,
    pub labels_fail: Cell<bool>,
}

pub struct Held {
    pub id: Id,
    pub kind: String,
    pub captured: SystemTime,
    pub capture: Option<String>,
    pub earlier: Vec<Option<String>>,
    pub id_label: Option<Id>,
    pub generations: Vec<Vec<String>>,
    pub damaged: bool,
}

impl Log {
    pub fn new() -> Log {
        Log {
            catalog: CATALOG,
            entries: Vec::new(),
            log: Vec::new(),
            changes_fail: false,
            reads: RefCell::new(Vec::new()),
            shorts: RefCell::new(Vec::new()),
            events: Events::default(),
            filed: RefCell::new(BTreeMap::new()),
            relabels: RefCell::new(Vec::new()),
            refuse_relabel: Cell::new(None),
            asked_words: RefCell::new(Vec::new()),
            asked_belonging: RefCell::new(Vec::new()),
            labels_fail: Cell::new(false),
        }
    }

    fn held(&mut self, entry: Id) -> &mut Held {
        self.entries
            .iter_mut()
            .find(|held| held.id == entry)
            .expect("an entry")
    }

    fn records(&mut self, id: Id, generation: u32, first: u64, count: u64) {
        self.log.push(Change::Records {
            id,
            generation: Generation(generation),
            records: RecordRange { first, count },
        });
    }

    pub fn put(&mut self, n: u8, kind: &str, records: &[&str]) {
        self.put_id(id(n), kind, records);
    }

    pub fn put_id(&mut self, entry: Id, kind: &str, records: &[&str]) {
        let n = entry.to_bytes()[15];
        self.entries.push(Held {
            id: entry,
            kind: kind.to_owned(),
            captured: at(1000 + u64::from(n)),
            capture: None,
            earlier: Vec::new(),
            id_label: None,
            generations: vec![records.iter().map(|record| (*record).to_owned()).collect()],
            damaged: false,
        });
        self.log.push(Change::Meta(entry));
        if !records.is_empty() {
            self.records(entry, 1, 1, records.len() as u64);
        }
    }

    pub fn crowd(count: u16) -> Log {
        let mut log = Log::new();
        for n in 1..=count {
            log.put_id(wide(n), "lines", &[&format!("record {n}")]);
            log.entries.last_mut().expect("an entry").capture =
                Some(format!("file:/home/me/src/place{n}"));
        }
        log
    }

    pub fn relabel_all(&mut self, text: impl Fn(Id) -> String) {
        for held in &mut self.entries {
            let before = held.capture.replace(text(held.id));
            if held.capture != before && before.is_some() {
                held.earlier.push(before);
            }
            self.log.push(Change::Meta(held.id));
        }
    }

    pub fn append_all(&mut self, record: &str) {
        let ids: Vec<Id> = self.entries.iter().map(|held| held.id).collect();
        for entry in ids {
            self.append_id(entry, &[record]);
        }
    }

    pub fn label(&mut self, n: u8, capture: &str) {
        self.retouch(n, |held| held.capture = Some(capture.to_owned()));
    }

    pub fn belong(&mut self, n: u8, to: u8) {
        self.retouch(n, |held| held.id_label = Some(id(to)));
    }

    pub fn append(&mut self, n: u8, records: &[&str]) {
        self.append_id(id(n), records);
    }

    pub fn append_id(&mut self, entry: Id, records: &[&str]) {
        let held = self.held(entry);
        let generation = held.generations.len();
        let newest = held.generations.last_mut().expect("a generation");
        let first = newest.len() as u64 + 1;
        newest.extend(records.iter().map(|record| (*record).to_owned()));
        self.records(entry, generation as u32, first, records.len() as u64);
    }

    pub fn rewrite(&mut self, n: u8, records: &[&str]) {
        let held = self.held(id(n));
        held.generations
            .push(records.iter().map(|record| (*record).to_owned()).collect());
        let generation = held.generations.len();
        self.records(id(n), generation as u32, 1, records.len() as u64);
    }

    pub fn retouch(&mut self, n: u8, change: impl FnOnce(&mut Held)) {
        let held = self.held(id(n));
        let before = held.capture.clone();
        change(held);
        if held.capture != before && before.is_some() {
            held.earlier.push(before);
        }
        self.log.push(Change::Meta(id(n)));
    }

    pub fn remove(&mut self, n: u8) {
        self.entries.retain(|held| held.id != id(n));
        self.log.push(Change::Meta(id(n)));
    }

    pub fn cursor(&self) -> Cursor {
        Cursor::at(self.catalog, self.log.len() as u64)
    }
}

impl Entries for Log {
    fn capture(&mut self, _input: &Capture<'_>) -> Result<Captured, CaptureError> {
        Err(CaptureError::Store(unused()))
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
        Err(unused())
    }

    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.reads.borrow_mut().push(id);
        let held = self
            .entries
            .iter()
            .find(|held| held.id == id)
            .ok_or_else(unused)?;
        if held.damaged {
            return Err(StoreError::Damaged {
                path: PathBuf::from("/s/a.seg"),
                offset: 12,
                what: "checksum mismatch".to_owned(),
            });
        }
        let all = &held.generations[generation.0 as usize - 1];
        assert_eq!(
            records,
            RecordRange {
                first: 1,
                count: all.len() as u64
            },
            "a whole generation is read"
        );
        Ok(all
            .iter()
            .map(|record| format!("{record}\n").into_bytes())
            .collect())
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        Ok(self
            .entries
            .iter()
            .find(|held| held.id == id)
            .map(|held| Meta {
                id,
                kind: held.kind.clone(),
                labels: StoredLabels {
                    capture: held.capture.clone().and_then(CaptureLabel::new),
                    id: held.id_label,
                },
                captured: held.captured,
            }))
    }

    fn changes(&self, since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        if self.changes_fail {
            return Err(StoreError::Poisoned);
        }
        let from = if since.catalog() == self.catalog {
            since.seq()
        } else {
            0
        };
        let from = usize::try_from(from)
            .unwrap_or(usize::MAX)
            .min(self.log.len());
        Ok((self.log[from..].to_vec(), self.cursor()))
    }

    fn by_capture(&self, _label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        Err(unused())
    }

    fn extent(
        &self,
        id: Id,
        generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        assert_eq!(generation, None, "the newest generation is asked for");
        Ok(self.entries.iter().find(|held| held.id == id).map(|held| {
            let newest = held.generations.len();
            (
                Generation(newest as u32),
                held.generations[newest - 1].len() as u64,
            )
        }))
    }

    fn captures_with(&self, _part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        Err(unused())
    }

    fn by_shorts(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        self.shorts.borrow_mut().push(shorts.len());
        let mut ids: Vec<Id> = self
            .entries
            .iter()
            .map(|held| held.id)
            .filter(|id| shorts.iter().any(|short| short.ends(*id)))
            .collect();
        ids.sort();
        Ok(ids)
    }
}

fn refused() -> StoreError {
    StoreError::Missing("the label tables refused".to_owned())
}

impl Labels for Log {
    fn with_words(&self, words: &[String]) -> Result<Vec<Id>, StoreError> {
        self.asked_words.borrow_mut().push(words.to_vec());
        if self.labels_fail.get() {
            return Err(StoreError::Poisoned);
        }
        if words.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .filed
            .borrow()
            .iter()
            .filter(|(_, labels)| {
                labels
                    .iter()
                    .any(|(_, held)| held.windows(words.len()).any(|run| run == words))
            })
            .map(|(entry, _)| *entry)
            .collect())
    }

    fn belonging_to(&self, entries: &[Id]) -> Result<Vec<Id>, StoreError> {
        self.asked_belonging.borrow_mut().push(entries.to_vec());
        if self.labels_fail.get() {
            return Err(StoreError::Poisoned);
        }
        let mut found: Vec<Id> = Vec::new();
        let mut above: Vec<Id> = entries.to_vec();
        while let Some(target) = above.pop() {
            for held in &self.entries {
                if held.id_label == Some(target)
                    && !entries.contains(&held.id)
                    && !found.contains(&held.id)
                {
                    found.push(held.id);
                    above.push(held.id);
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

impl Relabel for Log {
    fn relabel(&self, filed: &[Filed]) -> Result<(), StoreError> {
        self.events.borrow_mut().push("relabel");
        let call = self.relabels.borrow().len();
        self.relabels.borrow_mut().push(filed.to_vec());
        if self.refuse_relabel.get() == Some(call) {
            return Err(refused());
        }
        let mut rows = self.filed.borrow_mut();
        for each in filed {
            if each.labels.is_empty() {
                rows.remove(&each.entry);
            } else {
                rows.insert(
                    each.entry,
                    each.labels
                        .iter()
                        .map(|label| (label.text.clone(), label.words.clone()))
                        .collect(),
                );
            }
        }
        Ok(())
    }
}

/// Whether every entry the index holds and the store still has holds the labels of the generation
/// indexed, or of a newer one.
pub fn named(docs: &Docs, log: &Log) -> bool {
    let filed = log.filed.borrow();
    log.entries.iter().all(|held| {
        let rows: Vec<&Row> = docs
            .rows
            .iter()
            .filter(|row| row.entry == held.id)
            .collect();
        let Some(newest) = rows.iter().map(|row| row.generation).max() else {
            return true;
        };
        let reached = rows
            .iter()
            .filter(|row| row.generation == newest)
            .map(|row| row.first + row.count - 1)
            .max()
            .unwrap_or(0) as usize;
        let mut possible = Vec::new();
        for (at, generation) in held.generations.iter().enumerate() {
            let number = at as u32 + 1;
            if number < newest {
                continue;
            }
            let from = if number == newest { reached } else { 0 };
            for upto in from..=generation.len() {
                possible.push(folders(generation[..upto].iter().cloned()));
            }
        }
        let held_rows: BTreeMap<String, Vec<String>> = filed
            .get(&held.id)
            .map(|rows| rows.iter().cloned().collect())
            .unwrap_or_default();
        std::iter::once(&held.capture)
            .chain(&held.earlier)
            .any(|capture| {
                possible.iter().any(|texts| {
                    let expected: BTreeMap<String, Vec<String>> = capture
                        .iter()
                        .chain(texts)
                        .map(|text| (text.clone(), analyze::filed_words(text)))
                        .collect();
                    held_rows == expected
                })
            })
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub entry: Id,
    pub first: u64,
    pub part: Option<(u32, u32)>,
    pub count: u64,
    pub generation: u32,
    pub kind: String,
    pub date: i64,
    pub text: String,
}

enum Op {
    Clear,
    Add(Row),
    Delete(Id, u64),
}

/// An index that keeps whole documents. Operations apply at `commit`, in the order they were
/// made, so a delete takes only what was there before it.
#[derive(Default)]
pub struct Docs {
    pub rows: Vec<Row>,
    pub position: Option<Position>,
    pending: Vec<Op>,
    pub writes: u32,
    pub rolled_back: u32,
    pub commits: u32,
    pub fail_add: Option<u32>,
    pub fail_entry: Option<Id>,
    pub adds: u32,
    pub deletes: Vec<(Id, u64)>,
    pub events: Events,
    pub commit_fault: Fault,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Fault {
    #[default]
    None,
    Refused,
    AfterPublishing,
}

impl Docs {
    pub fn sorted(&self) -> Vec<Row> {
        let mut rows = self.rows.clone();
        rows.sort();
        rows
    }
}

impl Index for Docs {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        Ok(self.position.clone())
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.events.borrow_mut().push("clear");
        self.writes += 1;
        self.pending.push(Op::Clear);
        Ok(())
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.events.borrow_mut().push("add");
        self.writes += 1;
        if self.fail_add == Some(self.adds) || self.fail_entry == Some(doc.entry) {
            return Err(IndexError::Failed {
                action: "write",
                path: PathBuf::from("/c/index"),
                error: "disk full".to_owned(),
            });
        }
        self.adds += 1;
        self.pending.push(Op::Add(Row {
            entry: doc.entry,
            first: doc.chunk.records.first,
            part: doc.chunk.part,
            count: doc.chunk.records.count,
            generation: doc.generation.0,
            kind: doc.kind.to_owned(),
            date: millis(doc.date),
            text: doc.chunk.text.clone(),
        }));
        Ok(())
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
        self.events.borrow_mut().push("commit");
        self.writes += 1;
        if self.commit_fault == Fault::Refused {
            return Err(IndexError::Failed {
                action: "write",
                path: PathBuf::from("/c/index"),
                error: "the position cannot be written".to_owned(),
            });
        }
        self.commits += 1;
        for op in self.pending.drain(..) {
            match op {
                Op::Clear => self.rows.clear(),
                Op::Add(row) => self.rows.push(row),
                Op::Delete(entry, from) => {
                    self.rows
                        .retain(|row| !(row.entry == entry && row.first >= from));
                }
            }
        }
        self.position = Some(position.clone());
        if self.commit_fault == Fault::AfterPublishing {
            return Err(IndexError::Failed {
                action: "measure",
                path: PathBuf::from("/c/index"),
                error: "the folder cannot be measured".to_owned(),
            });
        }
        Ok(Stats { bytes: 0 })
    }

    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        let mut rows: Vec<&Row> = self.rows.iter().filter(|row| row.entry == entry).collect();
        rows.sort_by_key(|row| (row.first, row.part));
        Ok(rows
            .into_iter()
            .map(|row| Extent {
                generation: Generation(row.generation),
                records: RecordRange {
                    first: row.first,
                    count: row.count,
                },
                part: row.part,
                date: row.date,
            })
            .collect())
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        self.events.borrow_mut().push("delete");
        self.writes += 1;
        self.deletes.push((entry, from));
        self.pending.push(Op::Delete(entry, from));
        Ok(())
    }

    fn rollback(&mut self) {
        self.rolled_back += 1;
        self.pending.clear();
    }
}

fn unreadable(method: &str) -> IndexError {
    IndexError::Failed {
        action: "read",
        path: PathBuf::from("/c/index"),
        error: format!("{method} failed"),
    }
}

impl Docs {
    fn terms_of(row: &Row) -> Vec<String> {
        analyze::terms(&row.text)
    }
}

impl Lookup for Docs {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        Ok(self.position.clone())
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        let entries: std::collections::BTreeSet<Id> =
            self.rows.iter().map(|row| row.entry).collect();
        Ok(Totals {
            documents: self.rows.len() as u64,
            entries: entries.len() as u64,
            length: self
                .rows
                .iter()
                .map(|row| Docs::terms_of(row).len() as u64)
                .sum(),
            bound: self.rows.len() as u32,
        })
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        let mut postings = Vec::new();
        for (doc, row) in self.rows.iter().enumerate() {
            let terms = Docs::terms_of(row);
            let freq = terms.iter().filter(|candidate| *candidate == term).count();
            if freq > 0 {
                postings.push(Posting {
                    doc: doc as u32,
                    freq: freq as u32,
                    length: terms.len() as u32,
                });
            }
        }
        Ok(postings)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        Ok(self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match filter {
                Filter::Kind(kind) => row.kind == *kind,
                Filter::Entries(ids) => ids.binary_search(&row.entry).is_ok(),
            })
            .map(|(doc, _)| doc as u32)
            .collect())
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        Ok(docs
            .iter()
            .map(|doc| {
                let row = &self.rows[*doc as usize];
                Found {
                    entry: row.entry,
                    kind: row.kind.clone(),
                    extent: Extent {
                        generation: Generation(row.generation),
                        records: RecordRange {
                            first: row.first,
                            count: row.count,
                        },
                        part: row.part,
                        date: row.date,
                    },
                    digest: chunk::digest(&row.text),
                }
            })
            .collect())
    }
}

pub struct Failing<'a>(pub &'a Docs, pub &'static str);

impl Lookup for Failing<'_> {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        if self.1 == "position" {
            return Err(unreadable("position"));
        }
        self.0.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        if self.1 == "totals" {
            return Err(unreadable("totals"));
        }
        self.0.totals()
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        if self.1 == "postings" {
            return Err(unreadable("postings"));
        }
        self.0.postings(term)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        if self.1 == "holding" {
            return Err(unreadable("holding"));
        }
        self.0.holding(filter)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        if self.1 == "found" {
            return Err(unreadable("found"));
        }
        self.0.found(docs)
    }
}

pub struct Scripted {
    pub candidates: Vec<Candidate>,
    pub calls: Cell<u32>,
    pub limits: RefCell<Vec<usize>>,
}

impl Scripted {
    pub fn new(candidates: Vec<Candidate>) -> Scripted {
        Scripted {
            candidates,
            calls: Cell::new(0),
            limits: RefCell::new(Vec::new()),
        }
    }
}

impl Ranker for Scripted {
    fn rank(&self, _query: &Query, limit: usize) -> Result<Ranked, IndexError> {
        self.calls.set(self.calls.get() + 1);
        self.limits.borrow_mut().push(limit);
        Ok(Ranked {
            candidates: self.candidates.clone(),
            unscoped: Vec::new(),
        })
    }
}

#[derive(Default)]
pub struct Logged(pub Vec<String>);

impl QueryLog for Logged {
    fn append(&mut self, line: &str) -> std::io::Result<()> {
        self.0.push(line.to_owned());
        Ok(())
    }
}

pub struct Broken;

pub struct Says(pub Option<Language>);

impl Detect for Says {
    fn language(&self, _text: &str) -> Option<Language> {
        self.0
    }
}

pub struct Never;

impl Reader for Never {
    fn kind(&self) -> &'static str {
        unreachable!("no reader is asked its kind")
    }

    fn passages(&self, _records: &[&[u8]]) -> Vec<Passage> {
        unreachable!("no reader reads a record")
    }

    fn appends_only(&self) -> bool {
        unreachable!("no reader is asked")
    }
}

impl QueryLog for Broken {
    fn append(&mut self, _line: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("the disk is full"))
    }
}

pub struct Dated(pub &'static str);

impl Reader for Dated {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        Lines(self.0).passages(records)
    }

    fn appends_only(&self) -> bool {
        true
    }

    fn preface(&self, records: &[&[u8]]) -> Option<String> {
        Some(format!("{} records:", records.len()))
    }
}

/// A record `!close` ends the first passage with ` (closed)`, as a fence closed by a later record
/// turns earlier text into frontmatter.
pub struct Fenced(pub &'static str);

impl Reader for Fenced {
    fn kind(&self) -> &'static str {
        self.0
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        let mut passages = Lines(self.0).passages(records);
        if records.iter().any(|record| record.starts_with(b"!close"))
            && let Some(first) = passages.first_mut()
        {
            first.text.push_str(" (closed)");
            first.ends = vec![first.text.len()];
        }
        passages
    }

    fn appends_only(&self) -> bool {
        false
    }
}
