use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use bilbo_store::entries::{Generation, RecordRange};
use bilbo_store::id::Id;

use crate::chunk::Chunk;
use crate::position::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Doc<'a> {
    pub entry: Id,
    pub generation: Generation,
    pub kind: &'a str,
    pub date: SystemTime,
    pub chunk: &'a Chunk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub generation: Generation,
    pub records: RecordRange,
    pub part: Option<(u32, u32)>,
    /// In milliseconds, as the index keeps it: compare a date just computed through `millis`.
    pub date: i64,
}

pub fn millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub bytes: u64,
}

#[derive(Debug)]
pub enum IndexError {
    /// The folder holds something that is not this index: nothing in it was touched.
    Foreign(PathBuf),
    Failed {
        action: &'static str,
        path: PathBuf,
        error: String,
    },
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreign(path) => write!(
                f,
                "{} holds files that are not bilbo's index; bilbo will not touch it",
                path.display()
            ),
            Self::Failed {
                action,
                path,
                error,
            } => write!(f, "cannot {action} the index {}: {error}", path.display()),
        }
    }
}

impl std::error::Error for IndexError {}

/// Nothing is visible to a reader, and nothing survives the process, before `commit`. After
/// `commit` or `rollback` the index holds no writer.
///
/// Only `clear` makes or repairs the folder of an index: `add`, `delete` and `commit` on a folder
/// that is absent or broken fail. A broken index has no position, so the next plan rebuilds it.
pub trait Index {
    /// What the last commit recorded. It reads only and takes no writer: a folder that is absent,
    /// empty, broken or of another schema has no position and is left as it is.
    fn position(&mut self) -> Result<Option<Position>, IndexError>;
    fn clear(&mut self) -> Result<(), IndexError>;
    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError>;
    /// The documents added and removed since the last commit and `position`, together or not at all.
    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError>;
    /// The documents of one entry in the last commit, by first record and then part. It reads only.
    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError>;
    /// Removes every document of the entry whose first record is `from` or later (`from` 1 removes
    /// them all) at the next commit, and only the documents added before this call.
    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError>;
    /// Drops what was added and deleted since the last commit and releases the writer.
    fn rollback(&mut self);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use super::{IndexError, millis};

    #[test]
    fn a_date_is_whole_milliseconds_from_1970_either_way() {
        assert_eq!(millis(UNIX_EPOCH), 0);
        assert_eq!(millis(UNIX_EPOCH + Duration::from_micros(1_999)), 1);
        assert_eq!(millis(UNIX_EPOCH + Duration::from_nanos(999_999)), 0);
        assert_eq!(millis(UNIX_EPOCH + Duration::from_secs(3)), 3_000);
        assert_eq!(millis(UNIX_EPOCH - Duration::from_secs(3)), -3_000);
        assert_eq!(millis(UNIX_EPOCH - Duration::from_micros(1_999)), -1);
        let far = Duration::from_secs(i64::MAX as u64);
        if let Some(after) = UNIX_EPOCH.checked_add(far) {
            assert_eq!(millis(after), i64::MAX);
        }
        if let Some(before) = UNIX_EPOCH.checked_sub(far) {
            assert_eq!(millis(before), i64::MIN);
        }
    }

    #[test]
    fn an_error_names_the_folder_and_what_failed() {
        assert_eq!(
            IndexError::Foreign(PathBuf::from("/c/index")).to_string(),
            "/c/index holds files that are not bilbo's index; bilbo will not touch it"
        );
        let failed = IndexError::Failed {
            action: "commit",
            path: PathBuf::from("/c/index"),
            error: "disk full".to_owned(),
        };
        assert_eq!(
            failed.to_string(),
            "cannot commit the index /c/index: disk full"
        );
    }
}
