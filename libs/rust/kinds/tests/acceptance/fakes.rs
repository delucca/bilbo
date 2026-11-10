use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bilbo_kinds::files::{Files, Memory, Node};
use bilbo_kinds::kind::{Claims, Kind, Proposal, Refusal};
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Kept, Mark, Meta,
    Origin, RecordRange, Stamp, StoreError,
};
use bilbo_store::id::Id;

/// Claims `*.fake`; records end at `|`; `!` refuses; `id=` sets the id; `<name>.sub.fake` takes
/// `<name>.fake` as parent.
pub struct FakeKind {
    pub refused: Vec<(PathBuf, String)>,
    pub overrun: bool,
}

impl FakeKind {
    pub fn new() -> FakeKind {
        FakeKind {
            refused: Vec::new(),
            overrun: false,
        }
    }

    pub fn overrunning() -> FakeKind {
        FakeKind {
            overrun: true,
            ..FakeKind::new()
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
        if self.overrun {
            return Ok(vec![bytes.len() + 1]);
        }
        Ok(bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'|')
            .map(|(at, _)| at + 1)
            .collect())
    }

    fn propose(&self, path: &Path, bytes: &[u8], kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal {
        let text = String::from_utf8_lossy(bytes);
        let id = text
            .split_once("id=")
            .and_then(|(_, rest)| rest.get(..26))
            .and_then(Id::parse);
        let parent = path
            .to_str()
            .and_then(|path| path.strip_suffix(".sub.fake"))
            .and_then(|stem| kept(Path::new(&format!("{stem}.fake"))));
        Proposal { id, parent }
    }
}

pub struct Counting {
    pub files: Memory,
    pub reads: Cell<usize>,
}

impl Counting {
    pub fn new(files: Memory) -> Counting {
        Counting {
            files,
            reads: Cell::new(0),
        }
    }
}

impl Files for Counting {
    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        self.files.canonical(path)
    }

    fn node(&self, path: &Path) -> io::Result<Node> {
        self.files.node(path)
    }

    fn list(&self, folder: &Path) -> io::Result<Vec<OsString>> {
        self.files.list(folder)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.reads.set(self.reads.get() + 1);
        self.files.read(path)
    }

    fn stamp(&self, path: &Path) -> io::Result<Stamp> {
        self.files.stamp(path)
    }

    fn now(&self) -> SystemTime {
        self.files.now()
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
    pub parent: Option<Id>,
    pub stamp: Option<Stamp>,
    pub bytes: Vec<u8>,
    pub ends: Vec<usize>,
}

/// Every method `remember` has no use for panics.
#[derive(Default)]
pub struct FakeEntries {
    pub script: HashMap<String, Script>,
    pub seen: Vec<Seen>,
    pub heads: HashMap<String, Head>,
    pub metas: HashMap<Id, Meta>,
    pub adopted: Vec<(Id, Option<Id>)>,
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

pub const ADOPTED: u64 = 64;

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
            parent: input.parent,
            stamp: input.stamp,
            bytes: input.bytes.to_vec(),
            ends: input.ends.to_vec(),
        });
        let n = u8::try_from(self.seen.len()).unwrap_or(u8::MAX);
        let held = self.heads.get(input.origin.as_str()).map(|head| head.id);
        let kept = Kept {
            id: held.or(input.id).unwrap_or_else(|| id(n)),
            generation: Generation(1),
            records: RecordRange {
                first: 1,
                count: input.ends.len() as u64,
            },
            raw_bytes: input.bytes.len() as u64,
            stored_bytes: input.bytes.len() as u64 + 100,
        };
        let script = self
            .script
            .get(input.origin.as_str())
            .cloned()
            .unwrap_or(Script::New);
        if matches!(
            script,
            Script::New | Script::Appended | Script::NewGeneration | Script::Unchanged
        ) {
            self.metas.entry(kept.id).or_insert_with(|| Meta {
                id: kept.id,
                kind: input.kind.to_string(),
                origin: Some(input.origin.clone()),
                captured: std::time::SystemTime::UNIX_EPOCH,
                parent: input.parent,
            });
            self.heads.insert(
                input.origin.to_string(),
                Head {
                    id: kept.id,
                    generation: Generation(1),
                    records: kept.records.count,
                    mark: Mark { end: 0, hash: 0 },
                    stamp: input.stamp,
                },
            );
        }
        match script {
            Script::New => Ok(Captured::New(kept)),
            Script::Appended => Ok(Captured::Appended(kept)),
            Script::NewGeneration => Ok(Captured::NewGeneration(kept)),
            Script::Unchanged => Ok(Captured::Unchanged(kept.id)),
            Script::IdHeld(by) => Err(CaptureError::IdHeld { id: kept.id, by }),
            Script::Records(what) => Err(CaptureError::Records(what)),
            Script::Failure => Err(CaptureError::Store(StoreError::Stopped("Appended".into()))),
        }
    }

    fn set_meta(&mut self, id: Id, meta: &Meta) -> Result<u64, StoreError> {
        self.adopted.push((id, meta.parent));
        self.metas.insert(id, meta.clone());
        Ok(ADOPTED)
    }

    fn read(
        &self,
        _id: Id,
        _generation: Generation,
        _records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        panic!("remember must not call read")
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        Ok(self.metas.get(&id).cloned())
    }

    fn changes(&self, _since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        panic!("remember must not call changes")
    }

    fn by_origin(&self, origin: &Origin) -> Result<Option<Head>, StoreError> {
        Ok(self.heads.get(origin.as_str()).copied())
    }

    fn extent(
        &self,
        _id: Id,
        _generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        panic!("remember must not call extent")
    }

    fn origins_with(&self, _part: &str) -> Result<Vec<(Id, Origin)>, StoreError> {
        panic!("remember must not call origins_with")
    }
}
