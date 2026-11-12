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
