//! Note history: one log of versions per note under `<root>/.bilbo/history/`, the content each version names, and
//! what `watch`, `history` and `restore` share around them: naming a note, scanning `notes/` for ids, the sweeps of
//! leftovers, retention and the probe of `watch.lock`.
//!
//! ```text
//! <root>/.bilbo/watch.lock         held by the running watcher
//! <root>/.bilbo/history/lock       held while anything writes history
//! <root>/.bilbo/history/blobs/ab/cdef…    one file per distinct content, named by its SHA-256
//! <root>/.bilbo/history/notes/<ULID>.jsonl   one JSON line per version, oldest first
//! ```

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::note::{parse_name, read_id};
use crate::shared::frontmatter::{is_ulid, mint_ulid};
use crate::shared::hash;
use crate::shared::store::{self, is_topic};

/// The first line of a version id's preimage.
const ID_FORMAT: &str = "bilbo-version-1";
/// A tombstone's `blob`, and the last line of its id's preimage.
pub const DELETED: &str = "deleted";
/// The largest note that is recorded.
pub const MAX_BYTES: usize = 1024 * 1024;
const HEAD_BYTES: u64 = 64 * 1024;
const RESTORE_PREFIX: &str = ".bilbo-restore-";
const TMP_PREFIX: &str = ".tmp-";

pub const ADDED: &str = "added";
pub const EDITED: &str = "edited";
pub const RENAMED: &str = "renamed";
pub const RESTORED: &str = "restored";

const AT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%:z";

/// One line of a note's log. Fields a reader does not know are ignored, and an event it does not know is kept as is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    #[serde(default)]
    pub parents: Vec<String>,
    pub file: String,
    /// The SHA-256 of the content, or `DELETED`.
    pub blob: String,
    pub event: String,
    /// When it was recorded, with the UTC offset it was recorded under.
    pub at: String,
}

impl Version {
    pub fn is_deleted(&self) -> bool {
        self.blob == DELETED
    }

    /// The first 12 characters of the id, as `bilbo history` lists it.
    pub fn short(&self) -> &str {
        self.version.get(..12).unwrap_or(&self.version)
    }

    /// `at` to the minute, in the `created` form, keeping the offset it was recorded under.
    pub fn minute(&self) -> String {
        match (self.at.get(..16), self.at.get(19..)) {
            (Some(head), Some(offset)) if self.at.len() == 25 => format!("{head}{offset}"),
            _ => self.at.clone(),
        }
    }

    fn time(&self) -> Option<jiff::Timestamp> {
        self.at.parse().ok()
    }
}

/// A record is readable when its hashes are hex, so a blob never names a path outside `blobs/`, and its file name is
/// a note name, so a restore never writes a stray one.
fn readable(v: &Version) -> bool {
    let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    hex(&v.version)
        && v.parents.iter().all(|p| hex(p))
        && (hex(&v.blob) || v.is_deleted())
        && parse_name(&v.file).is_ok()
}

/// The SHA-256 over the format line, the note's id, the sorted parents, an empty line, the file name and the blob,
/// each followed by a newline. Nothing else goes in: not the time, the event or the device.
pub fn version_id(note_id: &str, parents: &[String], file: &str, blob: &str) -> String {
    let mut parents: Vec<&str> = parents.iter().map(String::as_str).collect();
    parents.sort_unstable();
    let mut preimage = format!("{ID_FORMAT}\n{note_id}\n");
    for parent in parents {
        preimage.push_str(parent);
        preimage.push('\n');
    }
    preimage.push_str(&format!("\n{file}\n{blob}\n"));
    hash::sha256_hex(preimage.as_bytes())
}

/// The time for a new record, to the second, with the local UTC offset.
pub fn now_at() -> String {
    jiff::Zoned::now().strftime(AT_FORMAT).to_string()
}

fn io_message(what: &str, path: &Path, e: &std::io::Error) -> String {
    format!("cannot {what} {}: {e}", path.display())
}

fn notes_dir(root: &Path) -> PathBuf {
    root.join("notes")
}

fn log_dir(root: &Path) -> PathBuf {
    store::history_dir(root).join("notes")
}

fn blobs_dir(root: &Path) -> PathBuf {
    store::history_dir(root).join("blobs")
}

pub fn log_path(root: &Path, note_id: &str) -> PathBuf {
    log_dir(root).join(format!("{note_id}.jsonl"))
}

fn blob_path(root: &Path, blob: &str) -> PathBuf {
    blobs_dir(root).join(&blob[..2]).join(&blob[2..])
}

/// `<root>/.bilbo/watch.lock`, which the running watcher holds.
pub fn watch_lock_path(root: &Path) -> PathBuf {
    root.join(".bilbo/watch.lock")
}

fn ensure_dirs(root: &Path) -> Result<(), String> {
    for dir in [blobs_dir(root), log_dir(root)] {
        fs::create_dir_all(&dir).map_err(|e| io_message("create", &dir, &e))?;
    }
    Ok(())
}

/// `history/lock`, held exclusively: every write to history goes through a function that takes it.
pub struct Lock {
    root: PathBuf,
    _file: File,
}

impl Lock {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Waits for `history/lock`, creating `history/` when it is missing. When the folder is deleted while it waits, the
/// lock it got is on a file nobody else will see, so it opens the new one and takes that.
pub fn lock(root: &Path) -> Result<Lock, String> {
    loop {
        ensure_dirs(root)?;
        let path = store::history_dir(root).join("lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| io_message("open", &path, &e))?;
        file.lock().map_err(|e| io_message("lock", &path, &e))?;
        let held = file.metadata().map_err(|e| io_message("stat", &path, &e))?;
        if fs::metadata(&path).is_ok_and(|now| (now.dev(), now.ino()) == (held.dev(), held.ino())) {
            return Ok(Lock {
                root: root.to_path_buf(),
                _file: file,
            });
        }
    }
}

/// Whether a `bilbo watch` holds `watch.lock`: one `try_lock`, released at once. It creates nothing.
pub fn watcher_running(root: &Path) -> bool {
    let Ok(file) = File::open(watch_lock_path(root)) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => false,
    }
}

/// Stores `bytes` under their SHA-256 and returns it. A blob that exists stays as it is.
pub fn write_blob(lock: &Lock, bytes: &[u8]) -> Result<String, String> {
    ensure_dirs(lock.root())?;
    let blob = hash::sha256_hex(bytes);
    let path = blob_path(lock.root(), &blob);
    if path.is_file() {
        return Ok(blob);
    }
    let dir = path.parent().expect("a blob path has a folder");
    fs::create_dir_all(dir).map_err(|e| io_message("create", dir, &e))?;
    let tmp = dir.join(format!("{TMP_PREFIX}{}", temp_name()?));
    let written = File::create(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    written
        .and_then(|()| fs::rename(&tmp, &path))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io_message("write", &path, &e)
        })?;
    Ok(blob)
}

fn temp_name() -> Result<String, String> {
    mint_ulid().map_err(|e| format!("cannot mint a name: {e}"))
}

/// The bytes of a blob, or `None` when no file holds them (a prune took them).
fn read_blob(root: &Path, blob: &str) -> Result<Option<Vec<u8>>, String> {
    let path = blob_path(root, blob);
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_message("read", &path, &e)),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContentError {
    Deleted,
    Pruned,
    Io(String),
}

/// The bytes a version recorded.
pub fn content(root: &Path, version: &Version) -> Result<Vec<u8>, ContentError> {
    if version.is_deleted() {
        return Err(ContentError::Deleted);
    }
    match read_blob(root, &version.blob) {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(ContentError::Pruned),
        Err(message) => Err(ContentError::Io(message)),
    }
}

/// A note's log as read: its versions with the bytes of each one's line, and the numbers of the complete lines that
/// did not parse.
#[derive(Debug, Default)]
pub struct Log {
    pub versions: Vec<Version>,
    /// `raw[i]` is the line `versions[i]` was read from, so a rewrite never re-encodes a record.
    pub raw: Vec<Vec<u8>>,
    pub unreadable: Vec<usize>,
}

impl Log {
    pub fn latest(&self) -> Option<&Version> {
        self.versions.last()
    }
}

fn parse_line(line: &[u8]) -> Option<Version> {
    let version: Version = serde_json::from_slice(line).ok()?;
    readable(&version).then_some(version)
}

/// A last line without its newline counts when it parses and is cut when it does not.
fn parse_log(bytes: &[u8]) -> Log {
    let mut log = Log::default();
    let mut lines = bytes.split(|b| *b == b'\n').enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        let last = lines.peek().is_none();
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match parse_line(line) {
            Some(version) => {
                log.versions.push(version);
                log.raw.push(line.to_vec());
            }
            None if !last => log.unreadable.push(n + 1),
            None => {}
        }
    }
    log
}

/// Reads a note's log; a note with none has an empty one. Takes no lock.
pub fn load(root: &Path, note_id: &str) -> Result<Log, String> {
    let path = log_path(root, note_id);
    match fs::read(&path) {
        Ok(bytes) => Ok(parse_log(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Log::default()),
        Err(e) => Err(io_message("read", &path, &e)),
    }
}

/// The ids of every note that has a log.
pub fn note_ids(root: &Path) -> Result<Vec<String>, String> {
    let dir = log_dir(root);
    let items = match fs::read_dir(&dir) {
        Ok(items) => items,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_message("read", &dir, &e)),
    };
    let mut ids: Vec<String> = items
        .filter_map(Result::ok)
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".jsonl").map(str::to_string)
        })
        .filter(|id| is_ulid(id))
        .collect();
    ids.sort();
    Ok(ids)
}

/// Appends one complete line. A last line left without its newline is finished when it parses and cut when it does
/// not, so the new line never joins a half-written one.
pub fn append(lock: &Lock, note_id: &str, version: &Version) -> Result<(), String> {
    ensure_dirs(lock.root())?;
    let path = log_path(lock.root(), note_id);
    let existing = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(io_message("read", &path, &e)),
    };
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|e| io_message("open", &path, &e))?;
    let mut line = Vec::new();
    if !existing.is_empty() && existing.last() != Some(&b'\n') {
        let kept = existing
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |p| p + 1);
        if parse_line(&existing[kept..]).is_some() {
            line.push(b'\n');
        } else {
            file.set_len(kept as u64)
                .map_err(|e| io_message("truncate", &path, &e))?;
        }
    }
    line.extend(serde_json::to_vec(version).map_err(|e| format!("cannot encode a version: {e}"))?);
    line.push(b'\n');
    file.write_all(&line)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_message("append to", &path, &e))
}

/// Records `file` with `bytes` (`None` for a deletion) as the note's next version.
pub fn record(
    lock: &Lock,
    note_id: &str,
    file: &str,
    bytes: Option<&[u8]>,
    event: &str,
    at: &str,
) -> Result<Version, String> {
    let latest = load(lock.root(), note_id)?.versions.pop();
    let blob = match bytes {
        Some(bytes) => write_blob(lock, bytes)?,
        None => DELETED.to_string(),
    };
    let parents: Vec<String> = latest.iter().map(|v| v.version.clone()).collect();
    let version = Version {
        version: version_id(note_id, &parents, file, &blob),
        parents,
        file: file.to_string(),
        blob,
        event: event.to_string(),
        at: at.to_string(),
    };
    append(lock, note_id, &version)?;
    Ok(version)
}

/// The event a note's current file records against its latest version, or `None` when they agree. `current` is the
/// file's name and the SHA-256 of its bytes, `None` when no file holds the id.
pub fn event_for(latest: Option<&Version>, current: Option<(&str, &str)>) -> Option<&'static str> {
    match (latest, current) {
        (None, Some(_)) => Some(ADDED),
        (None, None) => None,
        (Some(v), None) => (!v.is_deleted()).then_some(DELETED),
        (Some(v), Some((file, blob))) => {
            if v.is_deleted() {
                Some(EDITED)
            } else if v.file != file {
                Some(RENAMED)
            } else {
                (v.blob != blob).then_some(EDITED)
            }
        }
    }
}

/// Compares the note's current `(file name, bytes)` with its latest version and records the difference, if any.
pub fn record_difference(
    lock: &Lock,
    note_id: &str,
    current: Option<(&str, &[u8])>,
    at: &str,
) -> Result<Option<Version>, String> {
    let latest = load(lock.root(), note_id)?.versions.pop();
    let digest = current.map(|(_, bytes)| hash::sha256_hex(bytes));
    let seen = current
        .zip(digest.as_deref())
        .map(|((file, _), digest)| (file, digest));
    let Some(event) = event_for(latest.as_ref(), seen) else {
        return Ok(None);
    };
    let version = record(
        lock,
        note_id,
        current.map_or_else(|| latest.as_ref().map_or("", |v| &v.file), |(file, _)| file),
        current.map(|(_, bytes)| bytes),
        event,
        at,
    )?;
    Ok(Some(version))
}

/// A note file the scan accepted.
#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub id: String,
    pub bytes: Vec<u8>,
}

/// A note file the scan did not accept. `id` is its id when the scan could read one: a note whose only file is
/// skipped is not deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip {
    pub name: String,
    pub reason: String,
    pub id: Option<String>,
    /// The file could not be read, so what it holds is unknown.
    pub unreadable: bool,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub notes: BTreeMap<String, Found>,
    pub skipped: Vec<Skip>,
}

impl Scan {
    /// Whether some file could not be read. A scan that did not see every file must record no deletions, as one that
    /// found no note at all does: the unread file may hold any note's id.
    pub fn has_unreadable(&self) -> bool {
        self.skipped.iter().any(|s| s.unreadable)
    }
}

/// The regular, non-hidden files directly in `notes_dir`, by name. The error is the reason it cannot be listed.
pub fn list(notes_dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let items = fs::read_dir(notes_dir).map_err(|e| e.to_string())?;
    let mut files: Vec<(String, PathBuf)> = items
        .filter_map(Result::ok)
        .filter(|item| {
            item.file_type().is_ok_and(|t| {
                t.is_file()
                    || (t.is_symlink() && fs::metadata(item.path()).is_ok_and(|m| m.is_file()))
            })
        })
        .map(|item| (item.file_name().to_string_lossy().into_owned(), item.path()))
        .filter(|(name, _)| !name.starts_with('.'))
        .collect();
    files.sort();
    Ok(files)
}

fn read_up_to(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn id_of(bytes: &[u8]) -> Option<String> {
    read_id(&String::from_utf8_lossy(bytes))
}

/// Reads one listed file: its note, or the reason it is not recorded.
pub fn classify(name: &str, path: &Path) -> Result<Found, Skip> {
    let skip = |reason: &str, id: Option<String>| Skip {
        name: name.to_string(),
        reason: reason.to_string(),
        id,
        unreadable: false,
    };
    let bytes = read_up_to(path, MAX_BYTES as u64 + 1).map_err(|e| Skip {
        unreadable: true,
        ..skip(&format!("cannot read: {e}"), None)
    })?;
    if parse_name(name).is_err() {
        return Err(skip("name is not <kind>-<topic>.md", id_of(&bytes)));
    }
    if bytes.len() > MAX_BYTES {
        let head = read_up_to(path, HEAD_BYTES).unwrap_or_default();
        return Err(skip("larger than 1 MiB", id_of(&head)));
    }
    match id_of(&bytes) {
        Some(id) => Ok(Found {
            name: name.to_string(),
            id,
            bytes,
        }),
        None => Err(skip("no valid id in the frontmatter", None)),
    }
}

/// Reads every file of `notes/` and sorts them into notes and skips. Files that share an id are all skipped.
pub fn scan(notes_dir: &Path) -> Result<Scan, String> {
    let items = list(notes_dir)?
        .into_iter()
        .map(|(name, path)| classify(&name, &path))
        .collect();
    Ok(group(items))
}

/// Sorts classified files into notes and skips. Files that share an id are all skipped.
pub fn group(items: Vec<Result<Found, Skip>>) -> Scan {
    let mut by_id: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    let mut skipped = Vec::new();
    for item in items {
        match item {
            Ok(found) => by_id.entry(found.id.clone()).or_default().push(found),
            Err(skip) => skipped.push(skip),
        }
    }
    let mut notes = BTreeMap::new();
    for (id, mut files) in by_id {
        if files.len() == 1 {
            notes.insert(id, files.remove(0));
            continue;
        }
        for file in &files {
            let others: Vec<&str> = files
                .iter()
                .filter(|f| f.name != file.name)
                .map(|f| f.name.as_str())
                .collect();
            skipped.push(Skip {
                name: file.name.clone(),
                reason: format!("shares id {id} with {}", others.join(", ")),
                id: Some(id.clone()),
                unreadable: false,
            });
        }
    }
    skipped.sort_by(|a, b| a.name.cmp(&b.name));
    Scan { notes, skipped }
}

/// Records every `.bilbo-restore-<id>` file an interrupted restore left in `notes/`: as an `edited` version under
/// the note's latest file name when no version holds its bytes, then deletes it. A file whose note has no history is
/// left in place. Returns the messages to print.
pub fn sweep_restore_leftovers(lock: &Lock, at: &str) -> Result<Vec<String>, String> {
    let dir = notes_dir(lock.root());
    let items = fs::read_dir(&dir).map_err(|e| io_message("read", &dir, &e))?;
    let mut leftovers: Vec<(String, PathBuf)> = items
        .filter_map(Result::ok)
        .filter(|item| item.file_type().is_ok_and(|t| t.is_file()))
        .map(|item| (item.file_name().to_string_lossy().into_owned(), item.path()))
        .filter(|(name, _)| name.strip_prefix(RESTORE_PREFIX).is_some_and(is_ulid))
        .collect();
    leftovers.sort();
    let mut messages = Vec::new();
    for (name, path) in leftovers {
        let id = &name[RESTORE_PREFIX.len()..];
        let bytes = fs::read(&path).map_err(|e| io_message("read", &path, &e))?;
        let log = load(lock.root(), id)?;
        let Some(latest) = log.latest() else {
            messages.push(format!(
                "notes/{name} left in place: no history for its note"
            ));
            continue;
        };
        let digest = hash::sha256_hex(&bytes);
        if log.versions.iter().all(|v| v.blob != digest) {
            record(lock, id, &latest.file, Some(&bytes), EDITED, at)?;
            messages.push(format!("recorded notes/{name} from an interrupted restore"));
        }
        fs::remove_file(&path).map_err(|e| io_message("remove", &path, &e))?;
    }
    Ok(messages)
}

/// Deletes the `.tmp-*` files an interrupted blob write or prune left under `history/`, and returns how many.
pub fn sweep_temporaries(lock: &Lock) -> Result<usize, String> {
    let history = store::history_dir(lock.root());
    let mut dirs = vec![history.clone(), log_dir(lock.root())];
    if let Ok(items) = fs::read_dir(blobs_dir(lock.root())) {
        dirs.extend(items.filter_map(Result::ok).map(|item| item.path()));
    }
    let mut removed = 0;
    for dir in dirs {
        let Ok(items) = fs::read_dir(&dir) else {
            continue;
        };
        for item in items.filter_map(Result::ok) {
            let is_tmp = item.file_name().to_string_lossy().starts_with(TMP_PREFIX);
            if is_tmp && item.file_type().is_ok_and(|t| t.is_file()) {
                fs::remove_file(item.path()).map_err(|e| io_message("remove", &item.path(), &e))?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// The note a `<note>` argument names, and its file in `notes/` now, if it has one.
#[derive(Debug, PartialEq, Eq)]
pub struct Named {
    pub id: String,
    pub file: Option<String>,
    /// Files of `notes/` that hold the id but were skipped (too large, or sharing the id): the note is on disk
    /// though `file` is `None`, so restore must refuse and a diff against now has a file to name.
    pub skipped: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    /// An exit-2 message.
    Usage(String),
    NoHistory,
    Failed(String),
}

/// Names a note by its id or by a topic: the file in `notes/` with that topic, else the most recently deleted note
/// whose last file had it. `scan` is the caller's one scan of `notes/`.
pub fn resolve(root: &Path, arg: &str, scan: &Scan) -> Result<Named, NameError> {
    let named = |id: &str| Named {
        id: id.to_string(),
        file: scan.notes.get(id).map(|f| f.name.clone()),
        skipped: scan
            .skipped
            .iter()
            .filter(|s| s.id.as_deref() == Some(id))
            .map(|s| s.name.clone())
            .collect(),
    };
    let has_history = |id: &str| -> Result<(), NameError> {
        let log = load(root, id).map_err(NameError::Failed)?;
        if log.versions.is_empty() {
            return Err(NameError::NoHistory);
        }
        Ok(())
    };
    if is_ulid(arg) {
        has_history(arg)?;
        return Ok(named(arg));
    }
    if !is_topic(arg) {
        return Err(NameError::Usage(format!(
            "'{arg}' is neither a note id nor a topic; a topic uses segments of a-z and 0-9 joined by single hyphens"
        )));
    }
    let named_arg = |name: &str| parse_name(name).is_ok_and(|n| n.topic == arg);
    let mut holders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let found = scan.notes.values().map(|f| (&f.name, Some(&f.id)));
    let skipped = scan.skipped.iter().map(|s| (&s.name, s.id.as_ref()));
    for (name, id) in found.chain(skipped) {
        if let (true, Some(id)) = (named_arg(name), id) {
            holders.entry(id).or_default().push(name);
        }
    }
    if holders.len() > 1 {
        let mut files: Vec<String> = holders
            .iter()
            .flat_map(|(id, names)| names.iter().map(move |name| format!("{name} ({id})")))
            .collect();
        files.sort();
        return Err(NameError::Usage(format!(
            "topic '{arg}' names more than one note: {}",
            files.join(", ")
        )));
    }
    if let Some((id, _)) = holders.into_iter().next() {
        has_history(id)?;
        return Ok(named(id));
    }
    let mut deleted: Vec<(Option<jiff::Timestamp>, String)> = Vec::new();
    for id in note_ids(root).map_err(NameError::Failed)? {
        let log = load(root, &id).map_err(NameError::Failed)?;
        if let Some(last) = log
            .latest()
            .filter(|v| v.is_deleted() && named_arg(&v.file))
        {
            deleted.push((last.time(), id));
        }
    }
    match deleted.into_iter().max() {
        Some((_, id)) => Ok(named(&id)),
        None => Err(NameError::NoHistory),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VersionError<'a> {
    /// Not 6 to 64 hexadecimal characters.
    Invalid,
    Ambiguous(Vec<&'a Version>),
    Missing,
}

/// The one version whose id starts with `prefix`.
pub fn find_version<'a>(
    versions: &'a [Version],
    prefix: &str,
) -> Result<&'a Version, VersionError<'a>> {
    let prefix = prefix.to_ascii_lowercase();
    if !(6..=64).contains(&prefix.len()) || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(VersionError::Invalid);
    }
    let mut matches: Vec<&Version> = versions
        .iter()
        .filter(|v| v.version.starts_with(&prefix))
        .collect();
    match matches.len() {
        0 => Err(VersionError::Missing),
        1 => Ok(matches.remove(0)),
        _ => Err(VersionError::Ambiguous(matches)),
    }
}

/// What a prune did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    /// The versions dropped from logs.
    pub versions: usize,
    /// The local date of the cutoff.
    pub cutoff: String,
    /// Messages to print: logs it could not read.
    pub warnings: Vec<String>,
}

/// The indexes of the versions retention keeps: those at or after `cutoff`, the newest one before it, and, for a
/// deleted note, the tombstone and the version before it. A version with an unreadable time stays.
fn kept(versions: &[Version], cutoff: jiff::Timestamp) -> Vec<bool> {
    let old = |v: &Version| v.time().is_some_and(|t| t < cutoff);
    let mut keep: Vec<bool> = versions.iter().map(|v| !old(v)).collect();
    if let Some(newest) = versions.iter().rposition(old) {
        keep[newest] = true;
    }
    if let Some(last) = versions
        .len()
        .checked_sub(1)
        .filter(|&n| versions[n].is_deleted())
    {
        keep[last] = true;
        if last > 0 {
            keep[last - 1] = true;
        }
    }
    keep
}

/// Drops the versions older than `keep_days` days before `now`, then removes the content no kept version names. A
/// log with an unreadable line is left as it is and blocks every content removal.
pub fn prune(lock: &Lock, keep_days: u32, now: jiff::Timestamp) -> Result<Pruned, String> {
    let root = lock.root();
    let span = jiff::SignedDuration::from_hours(24 * i64::from(keep_days));
    let cutoff = now
        .checked_sub(span)
        .map_err(|e| format!("cannot compute the retention cutoff: {e}"))?;
    let mut pruned = Pruned {
        cutoff: cutoff
            .to_zoned(jiff::tz::TimeZone::system())
            .date()
            .to_string(),
        ..Pruned::default()
    };
    let mut blocked = false;
    for id in note_ids(root)? {
        let log = load(root, &id)?;
        if let Some(line) = log.unreadable.first() {
            pruned.warnings.push(format!(
                "history/notes/{id}.jsonl line {line} is unreadable; no content removed"
            ));
            blocked = true;
            continue;
        }
        let keep = kept(&log.versions, cutoff);
        let dropped = keep.iter().filter(|k| !**k).count();
        if dropped == 0 {
            continue;
        }
        let kept_lines: Vec<&[u8]> = log
            .raw
            .iter()
            .zip(&keep)
            .filter_map(|(line, k)| k.then_some(line.as_slice()))
            .collect();
        rewrite(root, &id, &kept_lines)?;
        pruned.versions += dropped;
    }
    if !blocked {
        remove_unnamed_blobs(root)?;
    }
    Ok(pruned)
}

/// Writes the log beside the old one and renames it over, so a reader sees one or the other.
fn rewrite(root: &Path, id: &str, lines: &[&[u8]]) -> Result<(), String> {
    let mut text = Vec::new();
    for line in lines {
        text.extend_from_slice(line);
        text.push(b'\n');
    }
    let tmp = log_dir(root).join(format!("{TMP_PREFIX}{}", temp_name()?));
    let written = File::create(&tmp).and_then(|mut f| {
        f.write_all(&text)?;
        f.sync_all()
    });
    written
        .and_then(|()| fs::rename(&tmp, log_path(root, id)))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io_message("rewrite", &log_path(root, id), &e)
        })
}

fn remove_unnamed_blobs(root: &Path) -> Result<(), String> {
    let mut named: HashSet<String> = HashSet::new();
    for id in note_ids(root)? {
        named.extend(load(root, &id)?.versions.into_iter().map(|v| v.blob));
    }
    let dir = blobs_dir(root);
    let Ok(folders) = fs::read_dir(&dir) else {
        return Ok(());
    };
    for folder in folders.filter_map(Result::ok) {
        let prefix = folder.file_name().to_string_lossy().into_owned();
        let Ok(items) = fs::read_dir(folder.path()) else {
            continue;
        };
        for item in items.filter_map(Result::ok) {
            let name = item.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') && !named.contains(&format!("{prefix}{name}")) {
                fs::remove_file(item.path()).map_err(|e| io_message("remove", &item.path(), &e))?;
            }
        }
        let _ = fs::remove_dir(folder.path());
    }
    Ok(())
}

/// The ids that appear in `scan` as notes or as skipped files: the notes that still have a file.
pub fn present_ids(scan: &Scan) -> BTreeSet<&str> {
    let notes = scan.notes.keys().map(String::as_str);
    let skipped = scan.skipped.iter().filter_map(|s| s.id.as_deref());
    notes.chain(skipped).collect()
}

/// `<root>/notes/.bilbo-restore-<id>`, the one hidden file a restore works through.
pub fn restore_path(root: &Path, note_id: &str) -> PathBuf {
    notes_dir(root).join(format!("{RESTORE_PREFIX}{note_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-04T12:00:00-03:00";

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A store root with an empty `notes/`.
    fn scratch(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("bilbo-versions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("notes")).unwrap();
        Scratch(dir)
    }

    fn resolved(root: &Path, arg: &str) -> Result<Named, NameError> {
        resolve(root, arg, &scan(&root.join("notes")).unwrap())
    }

    fn id(n: u8) -> String {
        format!("01M3YJ7R6HK6NQ30DCDB1P4D{n:02}")
    }

    fn note_text(id: &str, body: &str) -> String {
        format!("---\nid: {id}\ncreated: 2026-10-02T14:23-03:00\n---\n\n# T\n\n{body}\n")
    }

    fn put(root: &Path, name: &str, id: &str, body: &str) {
        fs::write(root.join("notes").join(name), note_text(id, body)).unwrap();
    }

    fn days_ago(days: i64) -> String {
        let now: jiff::Timestamp = NOW.parse().unwrap();
        let then = now
            .checked_sub(jiff::SignedDuration::from_hours(24 * days))
            .unwrap();
        then.to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-3)))
            .strftime(AT_FORMAT)
            .to_string()
    }

    fn now() -> jiff::Timestamp {
        NOW.parse().unwrap()
    }

    fn events(root: &Path, note_id: &str) -> Vec<String> {
        load(root, note_id)
            .unwrap()
            .versions
            .iter()
            .map(|v| format!("{} {}", v.event, v.file))
            .collect()
    }

    fn blob_files(root: &Path) -> usize {
        let mut count = 0;
        for folder in fs::read_dir(blobs_dir(root)).unwrap() {
            count += fs::read_dir(folder.unwrap().path()).unwrap().count();
        }
        count
    }

    #[test]
    fn version_id_follows_the_rule() {
        let note = id(1);
        let (p1, p2) = ("a".repeat(64), "b".repeat(64));
        let blob = "c".repeat(64);
        let expected = hash::sha256_hex(
            format!("bilbo-version-1\n{note}\n{p1}\n{p2}\n\nplan-x.md\n{blob}\n").as_bytes(),
        );
        let parents = [p2.clone(), p1.clone()];
        assert_eq!(version_id(&note, &parents, "plan-x.md", &blob), expected);
        let first =
            hash::sha256_hex(format!("bilbo-version-1\n{note}\n\nplan-x.md\ndeleted\n").as_bytes());
        assert_eq!(version_id(&note, &[], "plan-x.md", DELETED), first);
    }

    #[test]
    fn identical_records_share_an_id_and_two_notes_never_do() {
        let blob = "c".repeat(64);
        let a = version_id(&id(1), &[], "plan-x.md", &blob);
        assert_eq!(a, version_id(&id(1), &[], "plan-x.md", &blob));
        assert_ne!(a, version_id(&id(2), &[], "plan-x.md", &blob));
        assert_ne!(a, version_id(&id(1), &[], "plan-y.md", &blob));
    }

    #[test]
    fn a_record_with_unknown_fields_and_events_is_read() {
        let scratch = scratch("unknown");
        let root = &scratch.0;
        let (v, b) = ("a".repeat(64), "b".repeat(64));
        fs::create_dir_all(log_dir(root)).unwrap();
        let line = format!(
            r#"{{"version":"{v}","parents":[],"file":"plan-x.md","blob":"{b}","event":"merged","at":"{NOW}","device":"d1","sig":"zz"}}"#
        );
        fs::write(log_path(root, &id(1)), format!("{line}\n")).unwrap();
        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.versions.len(), 1);
        assert_eq!(log.versions[0].event, "merged");
        assert!(log.unreadable.is_empty());
    }

    #[test]
    fn a_record_that_names_a_path_is_unreadable() {
        let scratch = scratch("path");
        let root = &scratch.0;
        let v = "a".repeat(64);
        fs::create_dir_all(log_dir(root)).unwrap();
        let line = format!(
            r#"{{"version":"{v}","parents":[],"file":"plan-x.md","blob":"../../x","event":"added","at":"{NOW}"}}"#
        );
        fs::write(log_path(root, &id(1)), format!("{line}\n")).unwrap();
        let log = load(root, &id(1)).unwrap();
        assert!(log.versions.is_empty());
        assert_eq!(log.unreadable, [1]);
    }

    #[test]
    fn missing_history_folders_are_recreated_before_a_write() {
        let scratch = scratch("recreate");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        fs::remove_dir_all(store::history_dir(root)).unwrap();
        record(&lock, &id(1), "plan-x.md", Some(b"text"), ADDED, NOW).unwrap();
        assert_eq!(events(root, &id(1)), ["added plan-x.md"]);
        assert_eq!(blob_files(root), 1);
    }

    #[test]
    fn a_complete_last_line_without_a_newline_is_kept_and_finished() {
        let scratch = scratch("tail-kept");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW).unwrap();
        let path = log_path(root, &id(1));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.trim_end_matches('\n')).unwrap();
        assert_eq!(load(root, &id(1)).unwrap().versions.len(), 1);

        record(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with('\n'));
        assert_eq!(load(root, &id(1)).unwrap().versions.len(), 2);
    }

    #[test]
    fn an_unparsable_last_line_is_cut() {
        let scratch = scratch("tail-cut");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW).unwrap();
        let path = log_path(root, &id(1));
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(r#"{"version":"abc"#);
        fs::write(&path, &text).unwrap();
        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.versions.len(), 1);
        assert!(log.unreadable.is_empty());

        record(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW).unwrap();

        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.versions.len(), 2);
        assert!(log.unreadable.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
    }

    #[test]
    fn a_log_keeps_each_version_after_the_one_before() {
        let scratch = scratch("chain");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let first = record(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW).unwrap();
        let second = record(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW).unwrap();
        assert!(first.parents.is_empty());
        assert_eq!(second.parents, std::slice::from_ref(&first.version));
        assert_eq!(
            second.version,
            version_id(&id(1), &[first.version], "plan-x.md", &second.blob)
        );
        assert_eq!(content(root, &second), Ok(b"two".to_vec()));
    }

    #[test]
    fn a_topic_names_the_current_file_first() {
        let scratch = scratch("resolve-current");
        let root = &scratch.0;
        put(root, "decision-release.md", &id(1), "now");
        let lock = lock(root).unwrap();
        record(&lock, &id(1), "decision-release.md", Some(b"x"), ADDED, NOW).unwrap();
        record(&lock, &id(2), "plan-release.md", Some(b"y"), ADDED, NOW).unwrap();
        record(&lock, &id(2), "plan-release.md", None, DELETED, NOW).unwrap();

        let named = resolved(root, "release").unwrap();

        assert_eq!(named.id, id(1));
        assert_eq!(named.file.as_deref(), Some("decision-release.md"));
    }

    #[test]
    fn a_topic_with_no_file_names_the_latest_deleted_note() {
        let scratch = scratch("resolve-deleted");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        for (n, at) in [(1, days_ago(30)), (2, days_ago(5)), (3, days_ago(1))] {
            let file = if n == 3 {
                "plan-other.md"
            } else {
                "plan-release.md"
            };
            record(&lock, &id(n), file, Some(b"x"), ADDED, &at).unwrap();
            record(&lock, &id(n), file, None, DELETED, &at).unwrap();
        }
        assert_eq!(resolved(root, "release").unwrap().id, id(2));
        assert_eq!(resolved(root, "wumpus"), Err(NameError::NoHistory));
    }

    #[test]
    fn a_note_is_named_by_id_whatever_its_file_is_called() {
        let scratch = scratch("resolve-id");
        let root = &scratch.0;
        put(root, "plan-renamed.md", &id(1), "x");
        let lock = lock(root).unwrap();
        record(&lock, &id(1), "plan-renamed.md", Some(b"x"), ADDED, NOW).unwrap();
        let named = resolved(root, &id(1)).unwrap();
        assert_eq!(named.file.as_deref(), Some("plan-renamed.md"));
        assert_eq!(resolved(root, &id(2)), Err(NameError::NoHistory));
    }

    #[test]
    fn two_notes_with_one_topic_are_refused() {
        let scratch = scratch("resolve-two");
        let root = &scratch.0;
        put(root, "decision-release.md", &id(1), "a");
        put(root, "plan-release.md", &id(2), "b");
        let Err(NameError::Usage(message)) = resolved(root, "release") else {
            panic!("expected a usage error");
        };
        assert!(
            message.contains(&format!("decision-release.md ({})", id(1))),
            "{message}"
        );
        assert!(
            message.contains(&format!("plan-release.md ({})", id(2))),
            "{message}"
        );
        assert!(matches!(
            resolved(root, "Release_Notes"),
            Err(NameError::Usage(_))
        ));
    }

    fn versions_with(ids: &[&str]) -> Vec<Version> {
        ids.iter()
            .map(|prefix| Version {
                version: format!("{prefix}{}", "0".repeat(64 - prefix.len())),
                parents: Vec::new(),
                file: "plan-x.md".into(),
                blob: "c".repeat(64),
                event: EDITED.into(),
                at: NOW.into(),
            })
            .collect()
    }

    #[test]
    fn a_version_is_found_by_prefix() {
        let versions = versions_with(&["a1b2c3", "a1b2d4", "ffeedd"]);
        assert_eq!(
            find_version(&versions, "ffeedd").unwrap().version,
            versions[2].version
        );
        assert_eq!(
            find_version(&versions, "FFEEDD").unwrap().version,
            versions[2].version
        );
        assert_eq!(
            find_version(&versions, "a1b2c3").unwrap().version,
            versions[0].version
        );
        assert!(matches!(
            find_version(&versions, "a1b2"),
            Err(VersionError::Invalid)
        ));
        assert!(matches!(
            find_version(&versions, "a1b2cg"),
            Err(VersionError::Invalid)
        ));
        assert!(matches!(
            find_version(&versions, &"a".repeat(65)),
            Err(VersionError::Invalid)
        ));
        assert!(matches!(
            find_version(&versions, "000000"),
            Err(VersionError::Missing)
        ));
        let shared = versions_with(&["a1b2c3d4e5", "a1b2c3d4e6"]);
        let Err(VersionError::Ambiguous(found)) = find_version(&shared, "a1b2c3") else {
            panic!("expected an ambiguous prefix");
        };
        assert_eq!(found.len(), 2);
        assert!(find_version(&shared, "a1b2c3d4e5").is_ok());
    }

    #[test]
    fn a_version_whose_blob_is_gone_is_pruned() {
        let scratch = scratch("gone");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let v = record(&lock, &id(1), "plan-x.md", Some(b"text"), ADDED, NOW).unwrap();
        fs::remove_file(blob_path(root, &v.blob)).unwrap();
        assert_eq!(content(root, &v), Err(ContentError::Pruned));
        let gone = record(&lock, &id(1), "plan-x.md", None, DELETED, NOW).unwrap();
        assert_eq!(content(root, &gone), Err(ContentError::Deleted));
    }

    #[test]
    fn the_scan_keeps_note_names_and_says_why_it_skips_the_rest() {
        let scratch = scratch("scan");
        let root = &scratch.0;
        let notes = root.join("notes");
        put(root, "decision-release.md", &id(1), "a");
        put(root, "plan-copy.md", &id(2), "b");
        put(root, "plan-twin.md", &id(2), "b");
        put(root, "Release.md", &id(3), "c");
        fs::write(notes.join("plan-x.md"), "no frontmatter\n").unwrap();
        fs::write(
            notes.join("plan-huge.md"),
            format!("---\nid: {}\n---\n{}", id(4), "x".repeat(MAX_BYTES)),
        )
        .unwrap();
        fs::write(notes.join(".plan-hidden.md"), note_text(&id(5), "h")).unwrap();
        fs::create_dir(notes.join("plan-folder.md")).unwrap();

        let scan = scan(&notes).unwrap();

        assert_eq!(scan.notes.keys().collect::<Vec<_>>(), [&id(1)]);
        let skips: Vec<(&str, &str, Option<&str>)> = scan
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason.as_str(), s.id.as_deref()))
            .collect();
        let copy = format!("shares id {} with plan-twin.md", id(2));
        let twin = format!("shares id {} with plan-copy.md", id(2));
        assert_eq!(
            skips,
            [
                (
                    "Release.md",
                    "name is not <kind>-<topic>.md",
                    Some(id(3).as_str())
                ),
                ("plan-copy.md", copy.as_str(), Some(id(2).as_str())),
                ("plan-huge.md", "larger than 1 MiB", Some(id(4).as_str())),
                ("plan-twin.md", twin.as_str(), Some(id(2).as_str())),
                ("plan-x.md", "no valid id in the frontmatter", None),
            ]
        );
        let present = present_ids(&scan);
        assert!(present.contains(id(2).as_str()) && present.contains(id(4).as_str()));
    }

    #[test]
    fn each_difference_records_its_event() {
        let blob = |c: char| c.to_string().repeat(64);
        let version = |file: &str, blob: String| Version {
            version: "f".repeat(64),
            parents: Vec::new(),
            file: file.into(),
            blob,
            event: EDITED.into(),
            at: NOW.into(),
        };
        let live = version("plan-x.md", blob('a'));
        let gone = version("plan-x.md", DELETED.into());
        let (a, b) = (blob('a'), blob('b'));
        assert_eq!(event_for(None, Some(("plan-x.md", &a))), Some(ADDED));
        assert_eq!(event_for(None, None), None);
        assert_eq!(event_for(Some(&live), Some(("plan-x.md", &a))), None);
        assert_eq!(
            event_for(Some(&live), Some(("plan-x.md", &b))),
            Some(EDITED)
        );
        assert_eq!(
            event_for(Some(&live), Some(("plan-y.md", &a))),
            Some(RENAMED)
        );
        assert_eq!(
            event_for(Some(&live), Some(("plan-y.md", &b))),
            Some(RENAMED)
        );
        assert_eq!(event_for(Some(&live), None), Some(DELETED));
        assert_eq!(event_for(Some(&gone), None), None);
        assert_eq!(
            event_for(Some(&gone), Some(("plan-x.md", &a))),
            Some(EDITED)
        );
    }

    #[test]
    fn record_difference_walks_a_note_through_its_life() {
        let scratch = scratch("difference");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let step = |current: Option<(&str, &[u8])>| {
            record_difference(&lock, &id(1), current, NOW)
                .unwrap()
                .map(|v| v.event)
        };
        assert_eq!(step(None), None);
        assert_eq!(step(Some(("plan-x.md", b"one"))).as_deref(), Some(ADDED));
        assert_eq!(step(Some(("plan-x.md", b"one"))), None);
        assert_eq!(step(Some(("plan-x.md", b"two"))).as_deref(), Some(EDITED));
        assert_eq!(step(Some(("plan-y.md", b"two"))).as_deref(), Some(RENAMED));
        assert_eq!(step(None).as_deref(), Some(DELETED));
        assert_eq!(step(None), None);
        assert_eq!(step(Some(("plan-y.md", b"two"))).as_deref(), Some(EDITED));
        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.versions[2].file, "plan-y.md");
        assert_eq!(log.versions[3].file, "plan-y.md");
        assert_eq!(blob_files(root), 2);
    }

    #[test]
    fn the_restore_sweep_records_unknown_bytes_and_deletes_the_rest() {
        let scratch = scratch("sweep");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record(&lock, &id(1), "decision-x.md", Some(b"known"), ADDED, NOW).unwrap();
        record(&lock, &id(2), "plan-y.md", Some(b"known"), ADDED, NOW).unwrap();
        fs::write(restore_path(root, &id(1)), "unrecorded").unwrap();
        fs::write(restore_path(root, &id(2)), "known").unwrap();
        fs::write(restore_path(root, &id(3)), "orphan").unwrap();
        fs::write(root.join("notes/.bilbo-restore-nope"), "not ours").unwrap();

        let messages = sweep_restore_leftovers(&lock, NOW).unwrap();

        assert_eq!(
            messages[0],
            format!(
                "recorded notes/.bilbo-restore-{} from an interrupted restore",
                id(1)
            )
        );
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[1].contains(&id(3)));
        assert_eq!(
            events(root, &id(1)),
            ["added decision-x.md", "edited decision-x.md"]
        );
        assert_eq!(events(root, &id(2)), ["added plan-y.md"]);
        assert!(!restore_path(root, &id(1)).exists());
        assert!(!restore_path(root, &id(2)).exists());
        assert!(restore_path(root, &id(3)).exists());
        assert!(root.join("notes/.bilbo-restore-nope").exists());
    }

    #[test]
    fn the_temporary_sweep_removes_only_tmp_files() {
        let scratch = scratch("tmp");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let v = record(&lock, &id(1), "plan-x.md", Some(b"text"), ADDED, NOW).unwrap();
        let blob_dir = blob_path(root, &v.blob).parent().unwrap().to_path_buf();
        let strays = [
            blob_dir.join(".tmp-AAA"),
            log_dir(root).join(".tmp-BBB"),
            store::history_dir(root).join(".tmp-CCC"),
        ];
        for stray in &strays {
            fs::write(stray, "half").unwrap();
        }
        assert_eq!(sweep_temporaries(&lock), Ok(3));
        assert!(strays.iter().all(|s| !s.exists()));
        assert!(blob_path(root, &v.blob).is_file());
        assert!(log_path(root, &id(1)).is_file());
    }

    fn record_at(lock: &Lock, note: u8, file: &str, body: &str, event: &str, days: i64) {
        let bytes = (event != DELETED).then_some(body.as_bytes());
        record(lock, &id(note), file, bytes, event, &days_ago(days)).unwrap();
    }

    #[test]
    fn old_edits_are_dropped_and_their_content_removed() {
        let scratch = scratch("retention-edits");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        for (body, days) in [("a", 200), ("b", 150), ("c", 100), ("d", 10)] {
            record_at(&lock, 1, "plan-x.md", body, EDITED, days);
        }
        assert_eq!(blob_files(root), 4);

        let pruned = prune(&lock, 90, now()).unwrap();

        assert_eq!(pruned.versions, 2);
        assert!(pruned.warnings.is_empty());
        let log = load(root, &id(1)).unwrap();
        let blobs: Vec<Vec<u8>> = log
            .versions
            .iter()
            .map(|v| content(root, v).unwrap())
            .collect();
        assert_eq!(blobs, [b"c".to_vec(), b"d".to_vec()]);
        assert_eq!(blob_files(root), 2);
        assert_eq!(prune(&lock, 90, now()).unwrap().versions, 0);
    }

    #[test]
    fn a_note_untouched_for_a_year_keeps_its_only_version() {
        let scratch = scratch("retention-year");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record_at(&lock, 1, "plan-x.md", "old", ADDED, 400);
        assert_eq!(prune(&lock, 90, now()).unwrap().versions, 0);
        assert_eq!(events(root, &id(1)), ["added plan-x.md"]);
        assert_eq!(blob_files(root), 1);
    }

    #[test]
    fn a_deleted_note_keeps_its_last_text() {
        let scratch = scratch("retention-deleted");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record_at(&lock, 1, "plan-x.md", "first", ADDED, 320);
        record_at(&lock, 1, "plan-x.md", "last", EDITED, 300);
        record_at(&lock, 1, "plan-x.md", "", DELETED, 200);

        let pruned = prune(&lock, 90, now()).unwrap();

        assert_eq!(pruned.versions, 1);
        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.versions.len(), 2);
        assert!(log.versions[1].is_deleted());
        assert_eq!(content(root, &log.versions[0]), Ok(b"last".to_vec()));
        assert_eq!(blob_files(root), 1);
    }

    #[test]
    fn an_unreadable_middle_line_stops_every_blob_removal() {
        let scratch = scratch("retention-garbled");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record_at(&lock, 1, "plan-x.md", "one", EDITED, 200);
        record_at(&lock, 1, "plan-x.md", "two", EDITED, 100);
        record_at(&lock, 1, "plan-x.md", "three", EDITED, 5);
        let path = log_path(root, &id(1));
        let mut lines: Vec<String> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        lines[1] = "{garbled".into();
        let garbled = lines.join("\n") + "\n";
        fs::write(&path, &garbled).unwrap();
        for body in ["a", "b"] {
            record_at(&lock, 2, "plan-y.md", body, EDITED, 200);
        }
        let before = blob_files(root);

        let pruned = prune(&lock, 90, now()).unwrap();

        assert_eq!(
            pruned.warnings,
            [format!(
                "history/notes/{}.jsonl line 2 is unreadable; no content removed",
                id(1)
            )]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), garbled);
        assert_eq!(blob_files(root), before);
        assert_eq!(pruned.versions, 1);
    }

    #[test]
    fn the_cutoff_is_a_local_date() {
        let scratch = scratch("retention-date");
        let lock = lock(&scratch.0).unwrap();
        let pruned = prune(&lock, 30, now()).unwrap();
        assert_eq!(pruned.cutoff.len(), 10);
        assert!(pruned.cutoff.starts_with("2026-09-"), "{}", pruned.cutoff);
    }

    #[test]
    fn the_probe_sees_a_held_lock_and_releases_a_free_one() {
        let scratch = scratch("probe");
        let root = &scratch.0;
        assert!(!watcher_running(root));
        assert!(!watch_lock_path(root).exists());

        fs::create_dir_all(root.join(".bilbo")).unwrap();
        let held = File::create(watch_lock_path(root)).unwrap();
        assert!(!watcher_running(root));
        let probe = File::open(watch_lock_path(root)).unwrap();
        assert!(probe.try_lock().is_ok(), "the probe left the lock held");
        probe.unlock().unwrap();

        held.lock().unwrap();
        assert!(watcher_running(root));
        held.unlock().unwrap();
        assert!(!watcher_running(root));
    }

    #[test]
    fn minute_keeps_the_recorded_offset() {
        let v = &versions_with(&["a1b2c3"])[0];
        assert_eq!(v.minute(), "2026-10-04T12:00-03:00");
        assert_eq!(v.short(), "a1b2c3000000");
    }

    #[test]
    fn a_prune_keeps_each_kept_record_byte_for_byte() {
        let scratch = scratch("retention-raw");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        for (body, days) in [("a", 200), ("b", 100), ("c", 5)] {
            record_at(&lock, 1, "plan-x.md", body, EDITED, days);
        }
        let path = log_path(root, &id(1));
        let text = fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        lines[1] = format!(
            "{},  \"device\":\"d1\",\"flags\":[1, 2] }}",
            lines[1].trim_end_matches('}')
        );
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        assert_eq!(prune(&lock, 90, now()).unwrap().versions, 1);

        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(after, format!("{}\n{}\n", lines[1], lines[2]));
    }

    #[test]
    fn an_unreadable_file_stops_deletions() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = scratch("unreadable");
        let root = &scratch.0;
        put(root, "plan-x.md", &id(1), "x");
        put(root, "plan-y.md", &id(2), "y");
        assert!(!scan(&root.join("notes")).unwrap().has_unreadable());
        let path = root.join("notes/plan-y.md");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&path).is_ok() {
            return;
        }

        let scan = scan(&root.join("notes")).unwrap();

        assert!(scan.has_unreadable());
        let skip = &scan.skipped[0];
        assert!(skip.unreadable && skip.id.is_none() && skip.reason.starts_with("cannot read"));
        assert!(scan.notes.contains_key(&id(1)));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn a_note_whose_only_file_is_skipped_is_still_on_disk() {
        let scratch = scratch("resolve-skipped");
        let root = &scratch.0;
        let name = "decision-release.md";
        fs::write(
            root.join("notes").join(name),
            format!("---\nid: {}\n---\n{}", id(1), "x".repeat(MAX_BYTES)),
        )
        .unwrap();
        let lock = lock(root).unwrap();
        record(&lock, &id(1), name, Some(b"small"), ADDED, NOW).unwrap();

        for arg in ["release".to_string(), id(1)] {
            let named = resolved(root, &arg).unwrap();
            assert_eq!(named.id, id(1));
            assert_eq!(named.file, None);
            assert_eq!(named.skipped, [name]);
        }
    }
}
