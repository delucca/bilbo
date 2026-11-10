use std::time::SystemTime;

use bilbo_store::entries::RecordRange;

/// One labelled unit of an entry: a session's message, a note's heading section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub range: RecordRange,
    pub text: String,
    /// Each labelled piece of `text`, in order; the first starts at 0.
    pub pieces: Vec<Piece>,
    /// Where each record of `range` ends in `text`: rising char boundaries, the last `text.len()`.
    pub ends: Vec<usize>,
    pub at: Option<SystemTime>,
    /// The line an outline lists the passage by: a prompt's first line, a section's headings.
    /// `None` for a passage an outline leaves out.
    pub outline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// A byte offset in the passage's text.
    pub start: usize,
    /// The label line: `user:`, `tool_use Edit:`, a heading path.
    pub label: String,
}

/// Pure: no I/O, and nothing a record holds makes it fail or panic.
pub trait Reader {
    /// The kind an entry was kept with: `Kind::name` of the same crate.
    fn kind(&self) -> &'static str;
    /// `records` is one whole generation, record `n` at `records[n - 1]`. Passages come in record
    /// order and never share a record; a record that renders nothing is in none.
    fn passages(&self, records: &[&[u8]]) -> Vec<Passage>;
}
