use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bilbo_kinds::reader::{Passage, Piece, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats, millis};
use bilbo_search::position::Position;
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Meta, Origin,
    RecordRange, StoreError,
};
use bilbo_store::id::Id;

pub const CATALOG: [u8; 16] = [7; 16];

pub fn id(n: u8) -> Id {
    let mut bytes = [0; 16];
    bytes[15] = n;
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
            parent: None,
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
}

impl Recorded {
    fn record(&self, call: Call) -> Result<(), IndexError> {
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
}

pub struct Held {
    pub id: Id,
    pub kind: String,
    pub captured: SystemTime,
    pub parent: Option<Id>,
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
        }
    }

    fn held(&mut self, n: u8) -> &mut Held {
        self.entries
            .iter_mut()
            .find(|held| held.id == id(n))
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
        self.entries.push(Held {
            id: id(n),
            kind: kind.to_owned(),
            captured: at(1000 + u64::from(n)),
            parent: None,
            generations: vec![records.iter().map(|record| (*record).to_owned()).collect()],
            damaged: false,
        });
        self.log.push(Change::Meta(id(n)));
        if !records.is_empty() {
            self.records(id(n), 1, 1, records.len() as u64);
        }
    }

    pub fn append(&mut self, n: u8, records: &[&str]) {
        let held = self.held(n);
        let generation = held.generations.len();
        let newest = held.generations.last_mut().expect("a generation");
        let first = newest.len() as u64 + 1;
        newest.extend(records.iter().map(|record| (*record).to_owned()));
        self.records(id(n), generation as u32, first, records.len() as u64);
    }

    pub fn rewrite(&mut self, n: u8, records: &[&str]) {
        let held = self.held(n);
        held.generations
            .push(records.iter().map(|record| (*record).to_owned()).collect());
        let generation = held.generations.len();
        self.records(id(n), generation as u32, 1, records.len() as u64);
    }

    pub fn retouch(&mut self, n: u8, change: impl FnOnce(&mut Held)) {
        change(self.held(n));
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
                origin: None,
                captured: held.captured,
                parent: held.parent,
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

    fn by_origin(&self, _origin: &Origin) -> Result<Option<Head>, StoreError> {
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

    fn origins_with(&self, _part: &str) -> Result<Vec<(Id, Origin)>, StoreError> {
        Err(unused())
    }
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
    adds: u32,
    pub deletes: Vec<(Id, u64)>,
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
        self.writes += 1;
        self.pending.push(Op::Clear);
        Ok(())
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.writes += 1;
        if self.fail_add == Some(self.adds) {
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
        self.writes += 1;
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
