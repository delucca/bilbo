use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bilbo_entries::files::{Files, Memory, Node};
use bilbo_entries::kind::{Claim, Claims, Kind, Proposal, Refusal};
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Kept, Mark, Meta,
    RecordRange, Stamp, StoreError,
};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{CaptureLabel, Labels, StoredLabels, Tag, Tagged, Tags};

/// Claims `*.fake` and `*.fake.z` as one entry; the `.z` file holds a `z` byte, then the bytes;
/// records end at `|`; `!` refuses; `id=` sets the id; `<name>.sub.fake` takes
/// `<name>.fake` as its id label.
pub struct FakeKind {
    pub refused: Vec<(PathBuf, String)>,
    pub overrun: bool,
    pub promises: bool,
}

impl FakeKind {
    pub fn new() -> FakeKind {
        FakeKind {
            refused: Vec::new(),
            overrun: false,
            promises: false,
        }
    }

    pub fn promising() -> FakeKind {
        FakeKind {
            promises: true,
            ..FakeKind::new()
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
            Ok(Node::File) => FakeKind::claim(path, claims),
            Ok(Node::Folder) => {
                for name in files.list(path).unwrap_or_default() {
                    FakeKind::walk(files, &path.join(name), claims);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if matches!(files.node(&sibling(path)), Ok(Node::File)) {
                    FakeKind::claim(path, claims);
                }
            }
            _ => {}
        }
    }

    fn claim(path: &Path, claims: &mut Claims) {
        let Some(label) = label(path) else {
            return;
        };
        if claims.entries.iter().all(|claim| claim.label() != label) {
            claims
                .entries
                .push(Claim::file(label.clone()).or(packed(&label)));
        }
    }
}

fn label(path: &Path) -> Option<PathBuf> {
    let extension = path.extension()?;
    if extension == "fake" {
        return Some(path.to_path_buf());
    }
    let unpacked = path.with_extension("");
    (extension == "z" && unpacked.extension().is_some_and(|e| e == "fake")).then_some(unpacked)
}

fn packed(label: &Path) -> PathBuf {
    let mut name = label.as_os_str().to_owned();
    name.push(".z");
    PathBuf::from(name)
}

fn sibling(path: &Path) -> PathBuf {
    match label(path) {
        Some(label) if label != path => label,
        _ => packed(path),
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

    fn unpack(&self, file: &Path, bytes: Vec<u8>) -> Result<Vec<u8>, Refusal> {
        if file.extension().is_some_and(|e| e == "z") {
            return match bytes.split_first() {
                Some((b'z', rest)) => Ok(rest.to_vec()),
                _ => Err(Refusal("the fake kind cannot unpack it".into())),
            };
        }
        Ok(bytes)
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

    fn withdraws_gone(&self) -> bool {
        self.promises
    }

    fn propose(&self, label: &Path, bytes: &[u8], kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal {
        let text = String::from_utf8_lossy(bytes);
        let id = text
            .split_once("id=")
            .and_then(|(_, rest)| rest.get(..26))
            .and_then(Id::parse);
        let id_label = label
            .to_str()
            .and_then(|path| path.strip_suffix(".sub.fake"))
            .and_then(|stem| kept(Path::new(&format!("{stem}.fake"))));
        Proposal { id, id_label }
    }
}

pub struct Counting {
    pub files: Memory,
    pub nodes: RefCell<Vec<PathBuf>>,
    pub stamps: RefCell<Vec<PathBuf>>,
    pub read_paths: RefCell<Vec<PathBuf>>,
}

impl Counting {
    pub fn new(files: Memory) -> Counting {
        Counting {
            files,
            nodes: RefCell::default(),
            stamps: RefCell::default(),
            read_paths: RefCell::default(),
        }
    }

    pub fn count(calls: &RefCell<Vec<PathBuf>>, path: &str) -> usize {
        calls.borrow().iter().filter(|seen| *seen == path).count()
    }
}

impl Files for Counting {
    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        self.files.canonical(path)
    }

    fn node(&self, path: &Path) -> io::Result<Node> {
        self.nodes.borrow_mut().push(path.to_path_buf());
        self.files.node(path)
    }

    fn list(&self, folder: &Path) -> io::Result<Vec<OsString>> {
        self.files.list(folder)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.read_paths.borrow_mut().push(path.to_path_buf());
        self.files.read(path)
    }

    fn stamp(&self, path: &Path) -> io::Result<Stamp> {
        self.stamps.borrow_mut().push(path.to_path_buf());
        self.files.stamp(path)
    }

    fn now(&self) -> SystemTime {
        self.files.now()
    }
}

/// What `FakeEntries` answers for a label.
#[derive(Clone)]
pub enum Script {
    New,
    Appended,
    NewGeneration,
    Unchanged,
    IdHeld(Option<CaptureLabel>),
    Records(String),
    Failure,
}

/// One `capture` call, as owned data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub label: String,
    pub kind: String,
    pub id: Option<Id>,
    pub id_label: Option<Id>,
    pub stamp: Option<Stamp>,
    pub bytes: Vec<u8>,
    pub ends: Vec<usize>,
    pub head: Option<Head>,
}

#[derive(Default)]
pub struct FakeEntries {
    pub script: HashMap<String, Script>,
    pub seen: Vec<Seen>,
    pub heads: HashMap<String, Head>,
    pub metas: HashMap<Id, Meta>,
    pub adopted: Vec<(Id, Option<Id>)>,
    pub labelled: Vec<(Id, Option<CaptureLabel>)>,
    pub resolving: bool,
}

impl FakeEntries {
    pub fn new() -> FakeEntries {
        FakeEntries::default()
    }

    #[must_use]
    pub fn on(mut self, label: &str, script: Script) -> FakeEntries {
        self.script.insert(label.to_string(), script);
        self
    }

    #[must_use]
    pub fn resolving(mut self) -> FakeEntries {
        self.resolving = true;
        self
    }

    #[must_use]
    pub fn kept_under(
        mut self,
        label: Option<&str>,
        id: Id,
        kind: &str,
        tags: Tags,
    ) -> FakeEntries {
        let capture = label.and_then(|label| CaptureLabel::new(label.to_string()));
        if let Some(label) = &capture {
            self.heads.insert(
                label.to_string(),
                Head {
                    id,
                    generation: Generation(1),
                    records: 1,
                    mark: Mark { end: 0, hash: 0 },
                    stamp: None,
                },
            );
        }
        self.metas.insert(
            id,
            Meta {
                id,
                kind: kind.to_string(),
                labels: StoredLabels {
                    capture,
                    id: None,
                    tags,
                },
                captured: std::time::SystemTime::UNIX_EPOCH,
            },
        );
        self
    }

    #[must_use]
    pub fn held(mut self, id: Id, kind: &str, tags: Tags) -> FakeEntries {
        self.metas.insert(
            id,
            Meta {
                id,
                kind: kind.to_string(),
                labels: StoredLabels {
                    capture: None,
                    id: None,
                    tags,
                },
                captured: std::time::SystemTime::UNIX_EPOCH,
            },
        );
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
            label: input.label.to_string(),
            kind: input.kind.to_string(),
            id: input.id,
            id_label: input.id_label,
            stamp: input.stamp,
            bytes: input.bytes.to_vec(),
            ends: input.ends.to_vec(),
            head: self.heads.get(input.label.as_str()).copied(),
        });
        let n = u8::try_from(self.seen.len()).unwrap_or(u8::MAX);
        let held = self.heads.get(input.label.as_str()).map(|head| head.id);
        let script = self.script.get(input.label.as_str()).cloned().unwrap_or(
            if input.ends.is_empty() && held.is_some() {
                Script::NewGeneration
            } else {
                Script::New
            },
        );
        let generation = match (&script, self.heads.get(input.label.as_str())) {
            (Script::NewGeneration, Some(head)) => Generation(
                head.generation
                    .0
                    .checked_add(1)
                    .ok_or(CaptureError::Store(StoreError::TooManyGenerations(head.id)))?,
            ),
            _ => Generation(1),
        };
        let kept = Kept {
            id: held.or(input.id).unwrap_or_else(|| id(n)),
            generation,
            records: RecordRange {
                first: 1,
                count: input.ends.len() as u64,
            },
            raw_bytes: input.bytes.len() as u64,
            stored_bytes: input.bytes.len() as u64 + 100,
        };
        if matches!(
            script,
            Script::New | Script::Appended | Script::NewGeneration | Script::Unchanged
        ) {
            self.metas.entry(kept.id).or_insert_with(|| Meta {
                id: kept.id,
                kind: input.kind.to_string(),
                labels: StoredLabels {
                    capture: Some(input.label.clone()),
                    id: input.id_label,
                    tags: input.tags.clone(),
                },
                captured: std::time::SystemTime::UNIX_EPOCH,
            });
            self.heads.insert(
                input.label.to_string(),
                Head {
                    id: kept.id,
                    generation: kept.generation,
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
        self.adopted.push((id, meta.labels.id));
        self.labelled.push((id, meta.labels.capture.clone()));
        let moved = self
            .metas
            .get(&id)
            .and_then(|held| held.labels.capture.clone())
            .filter(|old| Some(old) != meta.labels.capture.as_ref());
        if let (Some(old), Some(new)) = (moved, &meta.labels.capture)
            && let Some(mut head) = self.heads.remove(old.as_str())
        {
            head.stamp = None;
            self.heads.insert(new.to_string(), head);
        }
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

    fn by_capture(&self, label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        Ok(self.heads.get(label.as_str()).copied())
    }

    fn extent(
        &self,
        _id: Id,
        _generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        panic!("remember must not call extent")
    }

    fn captures_with(&self, part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        assert!(self.resolving, "remember must not call captures_with");
        let mut found: Vec<(Id, CaptureLabel)> = self
            .metas
            .values()
            .filter_map(|meta| {
                let label = meta.labels.capture.clone()?;
                label
                    .as_str()
                    .to_ascii_lowercase()
                    .contains(&part.to_ascii_lowercase())
                    .then_some((meta.id, label))
            })
            .collect();
        found.sort_by_key(|(id, _)| *id);
        Ok(found)
    }

    fn by_shorts(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        assert!(self.resolving, "remember must not call by_shorts");
        let mut found: Vec<Id> = self
            .metas
            .keys()
            .copied()
            .filter(|id| shorts.iter().any(|short| short.ends(*id)))
            .collect();
        found.sort();
        Ok(found)
    }
}

impl Labels for FakeEntries {
    fn with_words(&self, _words: &[String]) -> Result<Vec<Id>, StoreError> {
        panic!("remember must not call with_words")
    }

    fn belonging_to(&self, _entries: &[Id]) -> Result<Vec<Id>, StoreError> {
        panic!("remember must not call belonging_to")
    }

    fn tagged(&self, tag: &Tag) -> Result<Vec<Id>, StoreError> {
        let mut found: Vec<Id> = self
            .metas
            .values()
            .filter(|meta| {
                meta.labels
                    .tags
                    .carried()
                    .any(|(carried, _)| carried == tag)
            })
            .map(|meta| meta.id)
            .collect();
        found.sort_unstable();
        Ok(found)
    }

    fn tags(&self) -> Result<Vec<Tagged>, StoreError> {
        let mut counts: std::collections::BTreeMap<(Tag, String), u64> =
            std::collections::BTreeMap::new();
        for meta in self.metas.values() {
            for (tag, _) in meta.labels.tags.carried() {
                *counts.entry((tag.clone(), meta.kind.clone())).or_default() += 1;
            }
        }
        Ok(counts
            .into_iter()
            .map(|((tag, kind), entries)| Tagged { tag, kind, entries })
            .collect())
    }
}
