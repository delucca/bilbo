use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bilbo_store::entries::{Capture, Captured, Entries, Generation, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct Scratch {
    dir: PathBuf,
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn base() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

impl Scratch {
    pub fn new(test: &str) -> Scratch {
        let base = base().canonicalize().expect("canonical scratch base");
        let dir = base.join(format!(
            "{test}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create scratch");
        Scratch { dir }
    }

    pub fn layout(&self) -> Layout {
        Layout {
            root: self.dir.join("root"),
            state: self.dir.join("state"),
            cache: self.dir.join("cache"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub fn open(layout: &Layout) -> Store {
    open_with(layout, Options::default())
}

pub fn open_with(layout: &Layout, options: Options) -> Store {
    Store::open(layout, options, &mut |_| {}).expect("open the store")
}

pub fn origin(name: &str) -> bilbo_store::entries::Origin {
    bilbo_store::entries::Origin::file(Path::new(&format!("/notes/{name}"))).expect("origin")
}

pub fn ends(bytes: &[u8]) -> Vec<usize> {
    let mut ends: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &byte)| byte == b'\n')
        .map(|(index, _)| index + 1)
        .collect();
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        ends.push(bytes.len());
    }
    ends
}

pub fn keep(store: &mut Store, name: &str, bytes: &[u8]) -> Captured {
    keep_as(store, name, bytes, None)
}

pub fn keep_as(store: &mut Store, name: &str, bytes: &[u8], id: Option<Id>) -> Captured {
    try_keep(store, name, bytes, id).expect("capture")
}

pub fn try_keep(
    store: &mut Store,
    name: &str,
    bytes: &[u8],
    id: Option<Id>,
) -> Result<Captured, bilbo_store::entries::CaptureError> {
    let origin = origin(name);
    store.capture(&Capture {
        origin: &origin,
        kind: "markdown",
        id,
        labels: &["p".to_string()],
        bytes,
        ends: &ends(bytes),
    })
}

pub fn id_of(captured: &Captured) -> Id {
    match captured {
        Captured::New(kept) | Captured::Appended(kept) | Captured::NewGeneration(kept) => kept.id,
        Captured::Unchanged(id) => *id,
    }
}

/// The first `count` records of a generation, concatenated.
pub fn read_records(store: &Store, id: Id, generation: u32, count: u64) -> Vec<u8> {
    store
        .read(id, Generation(generation), RecordRange { first: 1, count })
        .expect("read")
        .concat()
}

pub fn sealed(layout: &Layout) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(&layout.root)
        .expect("list root")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "seg")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect();
    found.sort();
    found
}

pub fn open_seg(layout: &Layout) -> PathBuf {
    layout.state.join("open.seg")
}

pub fn catalog(layout: &Layout) -> PathBuf {
    layout.cache.join("catalog.sqlite")
}

pub fn flip(path: &Path, at: usize) {
    let mut bytes = fs::read(path).expect("read");
    bytes[at] ^= 0xFF;
    fs::write(path, bytes).expect("write");
}

/// Every file and folder under `dir`, with the bytes of each file.
pub fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(dir: &Path, into: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                into.insert(path.clone(), None);
                walk(&path, into);
            } else {
                let bytes = fs::read(&path).expect("read");
                into.insert(path, Some(bytes));
            }
        }
    }
    let mut into = BTreeMap::new();
    walk(dir, &mut into);
    into
}

pub fn snapshot_of(layout: &Layout) -> Vec<BTreeMap<PathBuf, Option<Vec<u8>>>> {
    vec![
        snapshot(&layout.root),
        snapshot(&layout.state),
        snapshot(&layout.cache),
    ]
}

/// Runs `statement` against the catalog file, as a person with a database tool could.
pub fn sql(layout: &Layout, statement: &str) {
    rusqlite::Connection::open(catalog(layout))
        .expect("open the catalog")
        .execute_batch(statement)
        .expect("run the statement");
}

pub fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}
