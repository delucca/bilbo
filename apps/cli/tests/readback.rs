use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use bilbo_store::entries::{Capture, Cursor, Entries, Origin, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::paths::{self, Layout};
use bilbo_store::segments::{Options, Store};

#[expect(
    clippy::disallowed_methods,
    reason = "the read-back is told its scratch store and target by the environment"
)]
fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

fn required(name: &str) -> OsString {
    var(name).unwrap_or_else(|| {
        panic!(
            "the read-back needs BILBO_HOME, XDG_STATE_HOME, XDG_CACHE_HOME, BILBO_CONFIG and READBACK_TARGET; {name} is not set"
        )
    })
}

fn markdown_files(folder: &Path, into: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(folder).expect("list a folder") {
        let entry = entry.expect("an entry");
        if entry.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        let kind = entry.file_type().expect("a file type");
        let path = entry.path();
        if kind.is_dir() {
            markdown_files(&path, into);
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            into.insert(path);
        }
    }
}

/// The `id:` of a leading `---` block when it is a canonical ULID; read here, not by the Markdown crate.
fn frontmatter_id(file: &[u8]) -> Option<String> {
    const DIGITS: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let text = std::str::from_utf8(file).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    if lines.next()? != "---" {
        return None;
    }
    let mut value = None;
    for line in lines {
        if line == "---" {
            let value: &str = value?;
            let canonical = value.len() == 26
                && value.starts_with(|c| ('0'..='7').contains(&c))
                && value.chars().all(|c| DIGITS.contains(c));
            return canonical.then(|| value.to_owned());
        }
        if value.is_none()
            && !line.starts_with([' ', '-'])
            && let Some(("id", rest)) = line.split_once(':')
        {
            value = Some(rest.strip_prefix(' ').unwrap_or("?"));
        }
    }
    None
}

#[derive(Debug)]
struct Report {
    ids: usize,
    with_frontmatter_id: usize,
    origins: BTreeSet<PathBuf>,
}

fn check(store: &Store) -> Result<Report, String> {
    let (changes, _) = store.changes(&Cursor::start()).map_err(|e| e.to_string())?;
    let ids: BTreeSet<Id> = changes
        .iter()
        .map(bilbo_store::entries::Change::id)
        .collect();

    let mut origins = BTreeSet::new();
    let mut with_frontmatter_id = 0_usize;
    for id in &ids {
        let meta = store
            .meta(*id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("entry {id} has no meta"))?;
        let origin = meta
            .origin
            .clone()
            .ok_or_else(|| format!("entry {id} has no origin"))?;
        let path = origin
            .path()
            .ok_or_else(|| format!("entry {id} has a non-file origin {origin}"))?
            .to_path_buf();
        let file = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let head = store
            .by_origin(&origin)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{origin} has no head"))?;
        if head.id != *id {
            return Err(format!("{origin}: head id {} is not {id}", head.id));
        }
        let records = store
            .read(
                *id,
                head.generation,
                RecordRange {
                    first: 1,
                    count: head.records,
                },
            )
            .map_err(|e| format!("{origin}: {e}"))?
            .concat();
        if records != file {
            return Err(format!("{origin} differs from its entry {id}"));
        }

        if let Some(written) = frontmatter_id(&file) {
            if written != id.to_string() {
                return Err(format!(
                    "{origin}: the frontmatter id {written} is not {id}"
                ));
            }
            with_frontmatter_id += 1;
        }
        origins.insert(path);
    }
    Ok(Report {
        ids: ids.len(),
        with_frontmatter_id,
        origins,
    })
}

#[test]
#[ignore = "the done check's read-back: the lead runs it against a scratch store"]
fn every_entry_reads_back_as_its_origin_file_with_its_frontmatter_id() {
    for name in [
        "BILBO_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "BILBO_CONFIG",
        "READBACK_TARGET",
    ] {
        required(name);
    }
    let target = PathBuf::from(required("READBACK_TARGET"))
        .canonicalize()
        .expect("READBACK_TARGET exists");
    let layout = paths::layout(var).expect("a layout from the environment");
    let store = Store::open_read_only(&layout, &mut |_| {}).expect("open the store read-only");
    let report = check(&store).unwrap_or_else(|error| panic!("{error}"));

    let mut found = BTreeSet::new();
    markdown_files(&target, &mut found);
    let message = format!(
        "read back {} entries, {} with their frontmatter id, {} origins against {} files under {}",
        report.ids,
        report.with_frontmatter_id,
        report.origins.len(),
        found.len(),
        target.display()
    );
    assert_eq!(report.origins, found, "{message}");
    if let Some(expected) = var("READBACK_ENTRIES") {
        let expected: usize = expected
            .to_str()
            .and_then(|text| text.parse().ok())
            .unwrap_or_else(|| panic!("READBACK_ENTRIES is not a number: {message}"));
        assert_eq!(report.ids, expected, "{message}");
        assert_eq!(report.with_frontmatter_id, expected, "{message}");
    }
    assert!(report.ids > 0, "{message}");
}

fn scratch(test: &str) -> PathBuf {
    let base = var("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base
        .canonicalize()
        .expect("canonical scratch base")
        .join(format!("{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch");
    dir
}

fn last_frame_offset(open_seg: &Path) -> usize {
    let bytes = fs::read(open_seg).expect("read open.seg");
    let mut at = 12;
    let mut last = at;
    while at < bytes.len() {
        last = at;
        let length = u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        at += 4 + length as usize;
    }
    last
}

#[test]
fn an_entry_at_generation_0_fails_the_read_back_even_with_nothing_to_read() {
    let dir = scratch("readback_gen0");
    let layout = Layout {
        root: dir.join("store"),
        state: dir.join("state"),
        cache: dir.join("cache"),
    };
    let file = dir.join("empty.md");
    fs::write(&file, "").expect("an empty file");
    let origin = Origin::file(&file).expect("an origin");
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open");
    store
        .capture(&Capture {
            origin: &origin,
            kind: "markdown",
            id: None,
            labels: &[],
            bytes: b"",
            ends: &[],
        })
        .expect("capture");
    drop(store);
    let open_seg = layout.state.join("open.seg");
    let body = last_frame_offset(&open_seg);
    fs::OpenOptions::new()
        .write(true)
        .open(&open_seg)
        .expect("open open.seg")
        .set_len((body + 30) as u64)
        .expect("tear the body frame away");

    let store = Store::open_read_only(&layout, &mut |_| {}).expect("open read-only");
    let error = check(&store).expect_err("a generation 0 entry cannot be read back");
    assert!(error.contains(&file.display().to_string()), "{error}");
    let _ = fs::remove_dir_all(&dir);
}

const VALID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

#[test]
fn the_oracle_strips_exactly_one_carriage_return_per_line() {
    let read = |text: String| frontmatter_id(text.as_bytes());
    assert_eq!(
        read(format!("---\r\nid: {VALID}\r\n---\r\n")).as_deref(),
        Some(VALID)
    );
    assert_eq!(read(format!("---\nid: {VALID}\r\r\n---\n")), None);
    assert_eq!(read(format!("---\r\r\nid: {VALID}\n---\n")), None);
    assert_eq!(read(format!("---\nid: {VALID}\n---\r\r\n")), None);
}

#[test]
fn an_entry_whose_id_the_frontmatter_contradicts_fails_the_read_back() {
    let dir = scratch("readback_id");
    let layout = Layout {
        root: dir.join("store"),
        state: dir.join("state"),
        cache: dir.join("cache"),
    };
    let file = dir.join("note.md");
    fs::write(&file, format!("---\nid: {VALID}\n---\nbody\n")).expect("a note");
    let other = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("an id");
    let origin = Origin::file(&file).expect("an origin");
    let bytes = fs::read(&file).expect("read");
    let ends: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &byte)| byte == b'\n')
        .map(|(at, _)| at + 1)
        .collect();
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open");
    store
        .capture(&Capture {
            origin: &origin,
            kind: "markdown",
            id: Some(other),
            labels: &[],
            bytes: &bytes,
            ends: &ends,
        })
        .expect("capture");
    drop(store);

    let store = Store::open_read_only(&layout, &mut |_| {}).expect("open read-only");
    let error = check(&store).expect_err("the ids disagree");
    assert!(error.contains("frontmatter id"), "{error}");
    let _ = fs::remove_dir_all(&dir);
}
