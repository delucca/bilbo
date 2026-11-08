use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_kinds::kind::{Claims, Kind, Proposal, Refusal};
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Kept, Meta, Origin,
    RecordRange, StoreError,
};
use bilbo_store::id::Id;

/// Claims `*.fake`, splits on `|`, refuses content starting with `!`, proposes the id after `id=`.
pub struct FakeKind {
    pub refused: Vec<(PathBuf, String)>,
}

impl FakeKind {
    pub fn new() -> FakeKind {
        FakeKind {
            refused: Vec::new(),
        }
    }

    fn walk(files: &dyn Files, path: &Path, claims: &mut Claims) {
        match files.node(path) {
            Ok(Node::File) if path.extension().is_some_and(|e| e == "fake") => {
                claims.files.push(path.to_path_buf());
            }
            Ok(Node::Folder) => {
                for name in files.list(path).unwrap_or_default() {
                    FakeKind::walk(files, &path.join(name), claims);
                }
            }
            _ => {}
        }
    }
}

impl Kind for FakeKind {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn summary(&self) -> &'static str {
        "*.fake files, records split on |"
    }

    fn claims(&self, files: &dyn Files, target: &Path) -> Claims {
        let mut claims = Claims::default();
        FakeKind::walk(files, target, &mut claims);
        claims.refused.clone_from(&self.refused);
        claims
    }

    fn split(&self, bytes: &[u8]) -> Result<Vec<usize>, Refusal> {
        if bytes.starts_with(b"!") {
            return Err(Refusal("the fake kind refuses it".into()));
        }
        let mut ends: Vec<usize> = bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'|')
            .map(|(at, _)| at + 1)
            .collect();
        if !bytes.is_empty() && ends.last() != Some(&bytes.len()) {
            ends.push(bytes.len());
        }
        Ok(ends)
    }

    fn propose(&self, bytes: &[u8]) -> Proposal {
        let text = String::from_utf8_lossy(bytes);
        let id = text
            .split_once("id=")
            .and_then(|(_, rest)| rest.get(..26))
            .and_then(Id::parse);
        Proposal { id }
    }
}

/// What `FakeEntries` answers for an origin.
#[derive(Clone)]
pub enum Script {
    New,
    Appended,
    NewGeneration,
    Unchanged,
    IdHeld(Option<Origin>),
    Records(String),
    Failure,
}

/// One `capture` call, as owned data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub origin: String,
    pub kind: String,
    pub id: Option<Id>,
    pub labels: Vec<String>,
    pub bytes: Vec<u8>,
    pub ends: Vec<usize>,
}

/// Answers `capture` from a script keyed by origin text; every other method panics, because
/// `remember` must not call it.
#[derive(Default)]
pub struct FakeEntries {
    pub script: HashMap<String, Script>,
    pub seen: Vec<Seen>,
}

impl FakeEntries {
    pub fn new() -> FakeEntries {
        FakeEntries::default()
    }

    #[must_use]
    pub fn on(mut self, origin: &str, script: Script) -> FakeEntries {
        self.script.insert(origin.to_string(), script);
        self
    }
}

pub fn id(n: u8) -> Id {
    Id::from_bytes([n; 16])
}

/// The ids `FakeEntries` hands to an entry it creates: `id(n)` for the n-th capture.
impl Entries for FakeEntries {
    fn capture(&mut self, input: &Capture<'_>) -> Result<Captured, CaptureError> {
        self.seen.push(Seen {
            origin: input.origin.to_string(),
            kind: input.kind.to_string(),
            id: input.id,
            labels: input.labels.to_vec(),
            bytes: input.bytes.to_vec(),
            ends: input.ends.to_vec(),
        });
        let n = u8::try_from(self.seen.len()).unwrap_or(u8::MAX);
        let kept = Kept {
            id: input.id.unwrap_or_else(|| id(n)),
            generation: Generation(1),
            records: RecordRange {
                first: 1,
                count: input.ends.len() as u64,
            },
            raw_bytes: input.bytes.len() as u64,
            stored_bytes: input.bytes.len() as u64 + 100,
        };
        match self
            .script
            .get(input.origin.as_str())
            .cloned()
            .unwrap_or(Script::New)
        {
            Script::New => Ok(Captured::New(kept)),
            Script::Appended => Ok(Captured::Appended(kept)),
            Script::NewGeneration => Ok(Captured::NewGeneration(kept)),
            Script::Unchanged => Ok(Captured::Unchanged(kept.id)),
            Script::IdHeld(by) => Err(CaptureError::IdHeld { id: kept.id, by }),
            Script::Records(what) => Err(CaptureError::Records(what)),
            Script::Failure => Err(CaptureError::Store(StoreError::Stopped("Appended".into()))),
        }
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<(), StoreError> {
        panic!("remember must not call set_meta")
    }

    fn read(
        &self,
        _id: Id,
        _generation: Generation,
        _records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        panic!("remember must not call read")
    }

    fn meta(&self, _id: Id) -> Result<Option<Meta>, StoreError> {
        panic!("remember must not call meta")
    }

    fn changes(&self, _since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        panic!("remember must not call changes")
    }

    fn by_origin(&self, _origin: &Origin) -> Result<Option<Head>, StoreError> {
        panic!("remember must not call by_origin")
    }
}
