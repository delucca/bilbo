use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::SystemTime;

use bilbo_kinds::reader::{Passage, Piece, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Meta, Origin,
    RecordRange, StoreError,
};
use bilbo_store::id::Id;

use crate::index::{Doc, Extent, Index, IndexError, Stats};
use crate::lookup::{Filter, Found, Lookup, Posting, Totals};
use crate::position::Position;

pub(crate) fn id(n: u8) -> Id {
    Id::from_bytes([n; 16])
}

pub(crate) struct Four;

fn unused() -> StoreError {
    StoreError::Missing("not called".to_owned())
}

impl Entries for Four {
    fn capture(&mut self, _input: &Capture<'_>) -> Result<Captured, CaptureError> {
        Err(CaptureError::Store(unused()))
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
        Err(unused())
    }

    fn read(
        &self,
        id: Id,
        _generation: Generation,
        _records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        if id == Id::from_bytes([2; 16]) {
            return Ok(vec![b"fine\n".to_vec()]);
        }
        Err(StoreError::Damaged {
            path: PathBuf::from("/s/a.seg"),
            offset: 1,
            what: "checksum mismatch".to_owned(),
        })
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        Ok(Some(Meta {
            id,
            kind: "lines".to_owned(),
            origin: None,
            captured: SystemTime::UNIX_EPOCH,
            parent: None,
        }))
    }

    fn changes(&self, _since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        Ok((
            (1..=4).map(|n| Change::Meta(id(n))).collect(),
            Cursor::at([1; 16], 4),
        ))
    }

    fn by_origin(&self, _origin: &Origin) -> Result<Option<Head>, StoreError> {
        Err(unused())
    }

    fn extent(
        &self,
        _id: Id,
        _generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        Ok(Some((Generation(1), 1)))
    }

    fn origins_with(&self, _part: &str) -> Result<Vec<(Id, Origin)>, StoreError> {
        Err(unused())
    }
}

pub(crate) struct OnePassage;

impl Reader for OnePassage {
    fn kind(&self) -> &'static str {
        "lines"
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        records
            .iter()
            .zip(1..)
            .map(|(record, first)| Passage {
                range: RecordRange { first, count: 1 },
                text: String::from_utf8_lossy(record).trim_end().to_owned(),
                pieces: vec![Piece {
                    start: 0,
                    label: "line:".to_owned(),
                }],
                ends: vec![record.len() - 1],
                at: None,
                outline: None,
            })
            .collect()
    }

    fn appends_only(&self) -> bool {
        true
    }
}

pub(crate) struct Bytes;

impl Tokens for Bytes {
    fn ends(&self, text: &str) -> Vec<usize> {
        (1..=text.len()).collect()
    }
}

#[derive(Default)]
pub(crate) struct Commits {
    pub(crate) commits: Vec<Position>,
    pub(crate) rollbacks: usize,
}

impl Index for Commits {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        Ok(None)
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        Ok(())
    }

    fn add(&mut self, _doc: &Doc<'_>) -> Result<(), IndexError> {
        Ok(())
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
        self.commits.push(position.clone());
        Ok(Stats { bytes: 0 })
    }

    fn extents(&mut self, _entry: Id) -> Result<Vec<Extent>, IndexError> {
        Ok(Vec::new())
    }

    fn delete(&mut self, _entry: Id, _from: u64) -> Result<(), IndexError> {
        Ok(())
    }

    fn rollback(&mut self) {
        self.rollbacks += 1;
    }
}

struct Row {
    kind: String,
    entry: Id,
    records: RecordRange,
    part: Option<(u32, u32)>,
    terms: Vec<String>,
    live: bool,
}

/// A document's number is its place in the list, so a test numbers them in reverse by listing them so.
#[derive(Default)]
pub(crate) struct Table {
    rows: Vec<Row>,
    failing: bool,
    posting_at: Option<u32>,
    holding_at: Option<u32>,
    pub(crate) asked: RefCell<Vec<usize>>,
}

impl Table {
    pub(crate) fn new() -> Table {
        Table::default()
    }

    pub(crate) fn doc(mut self, kind: &str, entry: u8, first: u64, terms: &str) -> Table {
        self.rows.push(Row {
            kind: kind.to_owned(),
            entry: id(entry),
            records: RecordRange { first, count: 1 },
            part: None,
            terms: terms.split_whitespace().map(str::to_owned).collect(),
            live: true,
        });
        self
    }

    pub(crate) fn part(mut self, part: u32, of: u32) -> Table {
        if let Some(row) = self.rows.last_mut() {
            row.part = Some((part, of));
        }
        self
    }

    pub(crate) fn dead(mut self) -> Table {
        if let Some(row) = self.rows.last_mut() {
            row.live = false;
        }
        self
    }

    pub(crate) fn failing(mut self) -> Table {
        self.failing = true;
        self
    }

    pub(crate) fn posting_at(mut self, doc: u32) -> Table {
        self.posting_at = Some(doc);
        self
    }

    pub(crate) fn holding_at(mut self, doc: u32) -> Table {
        self.holding_at = Some(doc);
        self
    }

    fn check(&self) -> Result<(), IndexError> {
        if self.failing {
            return Err(IndexError::Failed {
                action: "read",
                path: PathBuf::from("/c/index"),
                error: "broken".to_owned(),
            });
        }
        Ok(())
    }

    fn live(&self) -> impl Iterator<Item = (usize, &Row)> {
        self.rows.iter().enumerate().filter(|(_, row)| row.live)
    }
}

impl Lookup for Table {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        Ok(None)
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.check()?;
        Ok(Totals {
            documents: self.live().count() as u64,
            entries: self
                .live()
                .map(|(_, row)| row.entry)
                .collect::<BTreeSet<_>>()
                .len() as u64,
            length: self.live().map(|(_, row)| row.terms.len() as u64).sum(),
            bound: u32::try_from(self.rows.len()).expect("a small table"),
        })
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.check()?;
        let mut postings: Vec<Posting> = self
            .live()
            .filter_map(|(doc, row)| {
                let freq = row.terms.iter().filter(|each| *each == term).count();
                (freq > 0).then(|| Posting {
                    doc: u32::try_from(doc).expect("a small table"),
                    freq: u32::try_from(freq).expect("a small count"),
                    length: u32::try_from(row.terms.len()).expect("a small count"),
                })
            })
            .collect();
        if let Some(doc) = self.posting_at {
            postings.push(Posting {
                doc,
                freq: 1,
                length: 1,
            });
        }
        Ok(postings)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.check()?;
        let Filter::Kind(kind) = filter;
        let mut held: Vec<u32> = self
            .live()
            .filter(|(_, row)| row.kind == *kind)
            .map(|(doc, _)| u32::try_from(doc).expect("a small table"))
            .collect();
        held.extend(self.holding_at);
        Ok(held)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.check()?;
        self.asked.borrow_mut().push(docs.len());
        Ok(docs
            .iter()
            .map(|doc| {
                let row = &self.rows[*doc as usize];
                Found {
                    entry: row.entry,
                    kind: row.kind.clone(),
                    extent: Extent {
                        generation: Generation(1),
                        records: row.records,
                        part: row.part,
                        date: 0,
                    },
                    digest: u64::from(*doc),
                }
            })
            .collect())
    }
}
