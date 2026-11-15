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
pub enum Edits {
    One,
    Two,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Near {
    pub term: String,
    pub documents: u64,
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
    /// For a `term` of `a` to `z`: every term a live document holds within `edits` of it as
    /// `typo::distance` counts, `term` itself among them when held, each with its live documents, by
    /// rising term.
    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError>;
    /// The live documents where `second` stands at the place right after `first`, by rising `doc`:
    /// `freq` is how many times, `length` as `postings` gives it.
    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError>;
    /// Returns a list because filtering inside `postings` would need a column read for every
    /// posting, and would change the document frequency unless both counts crossed the port.
    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError>;
    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError>;
}

/// How many places of `firsts` have the next place in `seconds`; both rising.
pub fn adjacent(firsts: &[u32], seconds: &[u32]) -> u32 {
    debug_assert!(firsts.is_sorted() && seconds.is_sorted());
    let mut count = 0;
    let mut next = seconds.iter().peekable();
    for place in firsts {
        let Some(after) = place.checked_add(1) else {
            continue;
        };
        while next.next_if(|second| **second < after).is_some() {}
        if next.peek().is_some_and(|second| **second == after) {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::adjacent;

    #[test]
    fn adjacent_counts_the_places_whose_next_place_is_in_the_second_list() {
        assert_eq!(adjacent(&[1, 5, 9], &[2, 6, 11]), 2);
        assert_eq!(adjacent(&[3, 4, 5], &[3, 4, 5]), 2);
        assert_eq!(adjacent(&[2], &[1]), 0);
        assert_eq!(adjacent(&[], &[1]), 0);
        assert_eq!(adjacent(&[1], &[]), 0);
        assert_eq!(adjacent(&[u32::MAX], &[0]), 0);
    }
}
