use std::collections::BTreeMap;
use std::fmt;

use bilbo_store::entries::StoreError;
use bilbo_store::id::Id;

use crate::chunk;
use crate::index::IndexError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub chunk: chunk::Size,
    pub render: String,
    /// 0 is 1.
    pub workers: usize,
    pub label_batch: usize,
    pub kept_unread: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Counts an entry that gave no chunk too.
    pub entries: u64,
    pub chunks: u64,
    pub unread: BTreeMap<String, u64>,
    pub failed: Vec<(Id, String)>,
    pub index_bytes: u64,
}

#[derive(Debug)]
pub enum Error {
    Store(StoreError),
    Index(IndexError),
    /// A manifest name holds a line break or nothing.
    Manifest,
    Workers,
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
