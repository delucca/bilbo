use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

/// Every close due to seal: the default waits for `open.seg` to be 60 minutes old.
pub fn sealing() -> Options {
    Options {
        seal_after: Duration::ZERO,
        ..Options::default()
    }
}

pub fn open(layout: &Layout) -> Store {
    open_with(layout, sealing())
}

pub fn open_with(layout: &Layout, options: Options) -> Store {
    Store::open(layout, options, &mut |_| {}).expect("open the store")
}

pub fn capture_label(name: &str) -> bilbo_store::labels::CaptureLabel {
    bilbo_store::labels::CaptureLabel::file(Path::new(&format!("/notes/{name}"))).expect("label")
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
    let label = capture_label(name);
    store.capture(&Capture {
        label: &label,
        kind: "markdown",
        id,
        id_label: None,
        stamp: None,
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

pub fn filed(entry: Id, texts: &[&str]) -> bilbo_store::labels::Filed {
    bilbo_store::labels::Filed {
        entry,
        labels: texts
            .iter()
            .map(|text| bilbo_store::labels::Worded {
                text: (*text).into(),
                words: text.split(' ').map(String::from).collect(),
            })
            .collect(),
    }
}

pub fn count(layout: &Layout, query: &str) -> i64 {
    rusqlite::Connection::open(catalog(layout))
        .expect("open the catalog")
        .query_row(query, [], |row| row.get(0))
        .expect("count")
}

pub fn label_tables(layout: &Layout) -> Vec<String> {
    let conn = rusqlite::Connection::open(catalog(layout)).expect("open the catalog");
    let mut lines = Vec::new();
    for query in [
        "SELECT id, text FROM labels ORDER BY id",
        "SELECT word, label, at FROM label_words ORDER BY word, label, at",
        "SELECT entry, label FROM entry_labels ORDER BY entry, label",
    ] {
        let mut statement = conn.prepare(query).expect("prepare");
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get_ref(column).map(|value| format!("{value:?}")))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("query");
        for row in rows {
            lines.push(row.expect("row").join("|"));
        }
        lines.push(String::new());
    }
    lines
}

pub fn words(text: &str) -> Vec<String> {
    text.split(' ').map(String::from).collect()
}

pub fn label_rows(layout: &Layout) -> (i64, i64, i64) {
    (
        count(layout, "SELECT count(*) FROM labels"),
        count(layout, "SELECT count(*) FROM label_words"),
        count(layout, "SELECT count(*) FROM entry_labels"),
    )
}
