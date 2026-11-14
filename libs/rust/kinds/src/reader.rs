use std::time::SystemTime;

use bilbo_store::entries::RecordRange;
use bilbo_store::opaque;

pub const MAX_ENTRY_LABELS: usize = 256;
pub const MAX_ENTRY_LABEL_BYTES: usize = 4096;

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

#[derive(Debug, Default)]
pub struct EntryLabels {
    texts: Vec<String>,
}

impl EntryLabels {
    pub fn add(&mut self, text: &str) {
        if text.is_empty()
            || text.len() > MAX_ENTRY_LABEL_BYTES
            || self.texts.len() >= MAX_ENTRY_LABELS
            || opaque::is_placeholder(text)
            || self.texts.iter().any(|held| held == text)
        {
            return;
        }
        self.texts.push(text.to_owned());
    }

    pub fn into_texts(self) -> Vec<String> {
        self.texts
    }
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
    /// Distinct, in the order first seen, at most `MAX_ENTRY_LABELS`: what an `EntryLabels` gives.
    fn entry_labels(&self, _records: &[&[u8]]) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryLabels, MAX_ENTRY_LABEL_BYTES, MAX_ENTRY_LABELS, Passage, Reader};

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

    #[test]
    fn a_reader_that_does_not_say_gives_no_entry_label() {
        let records: [&[u8]; 2] = [b"/home/me/src/garden\n", b"two\n"];
        assert!(Silent.entry_labels(&records).is_empty());
        assert!(Silent.entry_labels(&[]).is_empty());
    }

    #[test]
    fn the_collector_ignores_the_stores_placeholder() {
        let mut labels = EntryLabels::default();
        labels.add("[opaque: 300 chars, xxh3 0123456789abcdef]");
        labels.add("/home/me/src/garden");
        assert_eq!(labels.into_texts(), ["/home/me/src/garden"]);
    }

    #[test]
    fn the_collector_keeps_256_distinct_texts_of_4096_bytes_at_most() {
        let mut labels = EntryLabels::default();
        labels.add("b");
        labels.add("a");
        labels.add("b");
        labels.add("");
        labels.add(&"x".repeat(MAX_ENTRY_LABEL_BYTES + 1));
        labels.add("c");
        let long = "y".repeat(MAX_ENTRY_LABEL_BYTES);
        labels.add(&long);
        for n in 0..MAX_ENTRY_LABELS {
            labels.add(&format!("/f/{n}"));
        }
        let texts = labels.into_texts();
        assert_eq!(texts.len(), MAX_ENTRY_LABELS);
        assert_eq!(texts[..4], ["b", "a", "c", long.as_str()]);
        assert_eq!(texts[4], "/f/0");
        assert!(!texts.contains(&format!("/f/{}", MAX_ENTRY_LABELS - 1)));
    }
}
