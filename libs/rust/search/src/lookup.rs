use bilbo_store::id::Id;

use crate::index::{Extent, IndexError};
use crate::position::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Posting {
    pub doc: u32,
    pub freq: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    pub documents: u64,
    pub entries: u64,
    pub length: u64,
    pub bound: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub entry: Id,
    pub kind: String,
    pub extent: Extent,
    pub digest: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter<'a> {
    Kind(&'a str),
    /// Sorted and distinct.
    Entries(&'a [Id]),
}

/// The last commit, read only: no method creates, repairs, locks or writes a file.
///
/// A `doc` is a number below `Totals.bound`, the same for one value of the port and meaningless
/// outside it. `postings` and `holding` give live documents only, by rising `doc`;
/// `Posting.length` is the document's exact number of terms, saturating at `u32::MAX`; `found`
/// answers in the order asked. `Totals` counts live documents of the last commit.
pub trait Lookup {
    /// `None`: no index there (the folder is absent or empty), or one that is broken or of another
    /// schema. A folder holding files that are not bilbo's index is `Err(Foreign)` from every
    /// method, `position` included. With no usable index every other method fails with
    /// `IndexError::Failed` saying it holds no index.
    fn position(&self) -> Result<Option<Position>, IndexError>;
    fn totals(&self) -> Result<Totals, IndexError>;
    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError>;
    /// Returns a list because filtering inside `postings` would need a column read for every
    /// posting, and would change the document frequency unless both counts crossed the port.
    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError>;
    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError>;
}
