use std::time::SystemTime;

use bilbo_store::entries::RecordRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub range: RecordRange,
    pub text: String,
    /// Each labelled piece of `text`, in order; the first starts at 0.
    pub pieces: Vec<Piece>,
    /// Where each record of `range` ends in `text`: rising char boundaries, the last `text.len()`.
    pub ends: Vec<usize>,
    pub at: Option<SystemTime>,
    pub outline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// A byte offset in the passage's text.
    pub start: usize,
    pub label: String,
}

/// Pure: no I/O, and nothing a record holds makes it fail or panic.
pub trait Reader: Sync {
    fn kind(&self) -> &'static str;
    /// `records` is one whole generation, record `n` at `records[n - 1]`. Passages come in record
    /// order and never share a record; a record that renders nothing is in none.
    fn passages(&self, records: &[&[u8]]) -> Vec<Passage>;
    /// Whether appending records to a generation changes no existing passage: the passages of the
    /// shorter generation are the first passages of the longer one.
    fn appends_only(&self) -> bool;
    fn preface(&self, _records: &[&[u8]]) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Passage, Reader};

    struct Silent;

    impl Reader for Silent {
        fn kind(&self) -> &'static str {
            "silent"
        }

        fn passages(&self, _records: &[&[u8]]) -> Vec<Passage> {
            Vec::new()
        }

        fn appends_only(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_reader_that_does_not_say_shows_no_preface() {
        let records: [&[u8]; 2] = [b"one\n", b"two\n"];
        assert_eq!(Silent.preface(&records), None);
        assert_eq!(Silent.preface(&[]), None);
    }
}
