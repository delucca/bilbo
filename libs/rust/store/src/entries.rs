use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::id::Id;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin(String);

impl Origin {
    /// `file:` and the path; `None` unless the path is absolute and UTF-8. No I/O: callers pass a canonical path.
    #[must_use]
    pub fn file(path: &Path) -> Option<Origin> {
        if !path.is_absolute() {
            return None;
        }
        Some(Origin(format!("file:{}", path.to_str()?)))
    }

    #[must_use]
    pub fn new(text: String) -> Option<Origin> {
        if text.is_empty() {
            None
        } else {
            Some(Origin(text))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.0.strip_prefix("file:").map(Path::new)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub id: Id,
    pub kind: String,
    pub origin: Option<Origin>,
    /// Whole milliseconds: the store truncates before writing.
    pub captured: SystemTime,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u32);

/// Records are numbered from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordRange {
    pub first: u64,
    pub count: u64,
}

/// The origin mark: the origin's byte offset and a hash of its bytes up to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub end: u64,
    pub hash: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    pub id: Id,
    pub generation: Generation,
    pub records: u64,
    pub mark: Mark,
}

pub struct Capture<'a> {
    pub origin: &'a Origin,
    /// Used at first capture only.
    pub kind: &'a str,
    /// Used at first capture only; `None` mints one.
    pub id: Option<Id>,
    /// Used at first capture only.
    pub labels: &'a [String],
    pub bytes: &'a [u8],
    /// Each record's end offset: strictly rising, the last equal to `bytes.len()`.
    pub ends: &'a [usize],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kept {
    pub id: Id,
    pub generation: Generation,
    /// The records written by this capture.
    pub records: RecordRange,
    /// Input bytes captured.
    pub raw_bytes: u64,
    /// Bytes appended to `open.seg`, frame headers and the meta frame included.
    pub stored_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Captured {
    New(Kept),
    Appended(Kept),
    NewGeneration(Kept),
    Unchanged(Id),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Meta(Id),
    Records {
        id: Id,
        generation: Generation,
        records: RecordRange,
    },
}

impl Change {
    #[must_use]
    pub fn id(&self) -> Id {
        match self {
            Change::Meta(id) | Change::Records { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    catalog: [u8; 16],
    seq: u64,
}

impl Cursor {
    #[must_use]
    pub fn start() -> Cursor {
        Cursor::at([0; 16], 0)
    }

    #[must_use]
    pub fn at(catalog: [u8; 16], seq: u64) -> Cursor {
        Cursor { catalog, seq }
    }

    #[must_use]
    pub fn catalog(&self) -> [u8; 16] {
        self.catalog
    }

    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }
}

#[derive(Debug)]
pub enum CaptureError {
    /// Refused per file.
    IdHeld { id: Id, by: Option<Origin> },
    /// Refused per file: the kind's records do not cover its bytes.
    Records(String),
    /// Refused per file: a frame holds at most 4 GiB.
    TooLarge,
    /// Stops the run.
    Store(StoreError),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdHeld {
                id,
                by: Some(origin),
            } => {
                write!(f, "its id {id} is held by {origin}")
            }
            Self::IdHeld { id, by: None } => write!(f, "its id {id} is held by another entry"),
            Self::Records(what) => write!(f, "its records do not cover it: {what}"),
            Self::TooLarge => f.write_str("it is too large to keep: a frame holds at most 4 GiB"),
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for CaptureError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io {
        action: &'static str,
        path: PathBuf,
        error: io::Error,
    },
    Damaged {
        path: PathBuf,
        offset: u64,
        what: String,
    },
    Newer {
        path: PathBuf,
        version: u32,
    },
    Catalog {
        path: PathBuf,
        error: String,
    },
    Missing(String),
    ReadOnly,
    /// An earlier write failed or was stopped half way: the store must be opened again.
    Poisoned,
    TooManyGenerations(Id),
    MetaTooLarge,
    /// A minted id is already held by an entry.
    IdTaken(Id),
    MetaMismatch {
        meta: Id,
        given: Id,
    },
    OriginHeld {
        origin: Origin,
        by: Id,
    },
    /// A folder that group or others can write: bilbo will not use it.
    Unsafe(PathBuf),
    /// An id could not be made: the clock or `/dev/urandom` failed.
    Id(io::Error),
    /// The state folder was first opened for another store root.
    OtherRoot {
        state: PathBuf,
        recorded: PathBuf,
        asked: PathBuf,
    },
    /// A test hook stopped the store at this step (the step's `Debug` name).
    Stopped(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                error,
            } => write!(f, "cannot {action} {}: {error}", path.display()),
            Self::Damaged { path, offset, what } => {
                write!(f, "{} is damaged at byte {offset}: {what}", path.display())
            }
            Self::Newer { path, version } => write!(
                f,
                "{} is segment format {version}; this bilbo reads format 1",
                path.display()
            ),
            Self::Catalog { path, error } => {
                write!(f, "cannot use the catalog {}: {error}", path.display())
            }
            Self::Missing(what) => f.write_str(what),
            Self::ReadOnly => f.write_str("the store is open read-only"),
            Self::Unsafe(path) => write!(
                f,
                "{} can be written by others; bilbo will not use it",
                path.display()
            ),
            Self::Id(error) => write!(f, "cannot make an id: {error}"),
            Self::Poisoned => f.write_str("the store stopped on an error; open it again"),
            Self::TooManyGenerations(id) => write!(f, "entry {id} has too many generations"),
            Self::MetaTooLarge => f.write_str("a meta field is larger than 4 GiB"),
            Self::IdTaken(id) => write!(f, "minted id {id} is already held"),
            Self::MetaMismatch { meta, given } => write!(f, "meta for {meta} given to {given}"),
            Self::OriginHeld { origin, by } => write!(f, "origin {origin} is held by {by}"),
            Self::OtherRoot {
                state,
                recorded,
                asked,
            } => write!(
                f,
                "the state folder {} belongs to the store at {}, not {}",
                state.display(),
                recorded.display(),
                asked.display()
            ),
            Self::Stopped(step) => write!(f, "stopped at the test hook {step}"),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } | Self::Id(error) => Some(error),
            _ => None,
        }
    }
}

pub trait Entries {
    fn capture(&mut self, input: &Capture<'_>) -> Result<Captured, CaptureError>;
    fn set_meta(&mut self, id: Id, meta: &Meta) -> Result<(), StoreError>;
    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError>;
    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError>;
    fn changes(&self, since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError>;
    fn by_origin(&self, origin: &Origin) -> Result<Option<Head>, StoreError>;
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use super::{CaptureError, Cursor, Origin, StoreError};
    use crate::id::Id;

    #[test]
    fn a_file_origin_is_its_absolute_utf8_path() {
        let origin = Origin::file(Path::new("/notes/a.md")).expect("absolute");
        assert_eq!(origin.as_str(), "file:/notes/a.md");
        assert_eq!(origin.to_string(), "file:/notes/a.md");
        assert_eq!(origin.path(), Some(Path::new("/notes/a.md")));
        assert_eq!(Origin::file(Path::new("notes/a.md")), None);
        let bytes = PathBuf::from(OsString::from_vec(b"/notes/\xFF.md".to_vec()));
        assert_eq!(Origin::file(&bytes), None);
        assert_eq!(Origin::new(String::new()), None);
        assert_eq!(
            Origin::new("stdin".into()).and_then(|o| o.path().map(Path::to_path_buf)),
            None
        );
    }

    #[test]
    fn every_error_says_what_failed_and_where() {
        let id = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DB").expect("id");
        let origin = Origin::file(Path::new("/n/a.md")).expect("origin");
        let path = PathBuf::from("/s/open.seg");
        let cases = [
            (
                StoreError::Io {
                    action: "lock",
                    path: path.clone(),
                    error: io::Error::other("busy"),
                }
                .to_string(),
                "cannot lock /s/open.seg: busy",
            ),
            (
                StoreError::Damaged {
                    path: path.clone(),
                    offset: 12,
                    what: "checksum mismatch".into(),
                }
                .to_string(),
                "/s/open.seg is damaged at byte 12: checksum mismatch",
            ),
            (
                StoreError::Newer {
                    path: path.clone(),
                    version: 2,
                }
                .to_string(),
                "/s/open.seg is segment format 2; this bilbo reads format 1",
            ),
            (
                StoreError::Catalog {
                    path: PathBuf::from("/c/catalog.sqlite"),
                    error: "locked".into(),
                }
                .to_string(),
                "cannot use the catalog /c/catalog.sqlite: locked",
            ),
            (
                StoreError::Missing("no entry X".into()).to_string(),
                "no entry X",
            ),
            (
                StoreError::ReadOnly.to_string(),
                "the store is open read-only",
            ),
            (
                StoreError::Stopped("Appended".into()).to_string(),
                "stopped at the test hook Appended",
            ),
            (
                CaptureError::IdHeld {
                    id,
                    by: Some(origin),
                }
                .to_string(),
                "its id 01JC5Q3V7XKM4P8R2T6W9YB0DB is held by file:/n/a.md",
            ),
            (
                CaptureError::IdHeld { id, by: None }.to_string(),
                "its id 01JC5Q3V7XKM4P8R2T6W9YB0DB is held by another entry",
            ),
            (
                CaptureError::Records("ends rise".into()).to_string(),
                "its records do not cover it: ends rise",
            ),
            (
                CaptureError::TooLarge.to_string(),
                "it is too large to keep: a frame holds at most 4 GiB",
            ),
            (
                StoreError::Unsafe(PathBuf::from("/s")).to_string(),
                "/s can be written by others; bilbo will not use it",
            ),
            (
                StoreError::Id(io::Error::other("the clock is behind")).to_string(),
                "cannot make an id: the clock is behind",
            ),
            (
                StoreError::Poisoned.to_string(),
                "the store stopped on an error; open it again",
            ),
            (
                StoreError::TooManyGenerations(id).to_string(),
                "entry 01JC5Q3V7XKM4P8R2T6W9YB0DB has too many generations",
            ),
            (
                StoreError::MetaTooLarge.to_string(),
                "a meta field is larger than 4 GiB",
            ),
            (
                StoreError::IdTaken(id).to_string(),
                "minted id 01JC5Q3V7XKM4P8R2T6W9YB0DB is already held",
            ),
            (
                StoreError::MetaMismatch {
                    meta: id,
                    given: id,
                }
                .to_string(),
                "meta for 01JC5Q3V7XKM4P8R2T6W9YB0DB given to 01JC5Q3V7XKM4P8R2T6W9YB0DB",
            ),
            (
                StoreError::OriginHeld {
                    origin: Origin::file(Path::new("/n/a.md")).expect("origin"),
                    by: id,
                }
                .to_string(),
                "origin file:/n/a.md is held by 01JC5Q3V7XKM4P8R2T6W9YB0DB",
            ),
            (
                StoreError::OtherRoot {
                    state: PathBuf::from("/s"),
                    recorded: PathBuf::from("/r1"),
                    asked: PathBuf::from("/r2"),
                }
                .to_string(),
                "the state folder /s belongs to the store at /r1, not /r2",
            ),
            (
                CaptureError::Store(StoreError::ReadOnly).to_string(),
                "the store is open read-only",
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn a_cursor_starts_at_the_zero_catalog_and_sequence() {
        let start = Cursor::start();
        assert_eq!(start.catalog(), [0; 16]);
        assert_eq!(start.seq(), 0);
        assert_eq!(Cursor::at([1; 16], 7).seq(), 7);
    }

    #[test]
    fn an_error_names_its_cause() {
        use std::error::Error;
        let io_error = StoreError::Io {
            action: "read",
            path: PathBuf::from("/s"),
            error: io::Error::other("busy"),
        };
        assert_eq!(
            io_error.source().map(ToString::to_string),
            Some("busy".into())
        );
        let id_error = StoreError::Id(io::Error::other("clock"));
        assert_eq!(
            id_error.source().map(ToString::to_string),
            Some("clock".into())
        );
        assert!(StoreError::ReadOnly.source().is_none());
        let wrapped = CaptureError::Store(StoreError::ReadOnly);
        assert_eq!(
            wrapped.source().map(ToString::to_string),
            Some("the store is open read-only".into())
        );
        assert!(CaptureError::TooLarge.source().is_none());
    }
}
