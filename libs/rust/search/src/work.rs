use std::collections::BTreeMap;
use std::fmt;

use bilbo_store::entries::StoreError;
use bilbo_store::id::Id;

use crate::chunk;
use crate::index::IndexError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub chunk: chunk::Size,
    /// How the readers render passages: their versions and the settings they take.
    pub render: String,
    /// The threads that read and chunk entries; 0 is 1.
    pub workers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Entries whose newest generation is in the index, also when it gave no chunk.
    pub entries: u64,
    pub chunks: u64,
    /// Entries left out because no reader reads their kind, by kind.
    pub unread: BTreeMap<String, u64>,
    /// Entries left out because the store could not give their records, their reader or tokenizer
    /// failed on them, or the tokenizer could not split a text of them, with why.
    pub failed: Vec<(Id, String)>,
    pub index_bytes: u64,
}

#[derive(Debug)]
pub enum Error {
    Store(StoreError),
    Index(IndexError),
    /// A manifest name holds a line break or nothing.
    Manifest,
    /// The threads that chunk the entries all stopped before the entries were done.
    Workers,
    /// More entries failed than a position can record.
    Failed(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Index(error) => write!(f, "{error}"),
            Self::Manifest => f.write_str("the index's manifest cannot be written down"),
            Self::Workers => f.write_str("the threads that chunk the entries stopped early"),
            Self::Failed(count) => {
                write!(f, "{count} entries failed: more than a commit can record")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Index(error) => Some(error),
            Self::Manifest | Self::Workers | Self::Failed(_) => None,
        }
    }
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<IndexError> for Error {
    fn from(error: IndexError) -> Self {
        Self::Index(error)
    }
}
