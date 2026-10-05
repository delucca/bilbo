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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
pub const MERGED: &str = "merged";
/// A scope's record that a note moved out of it: its blob is `DELETED`, its file the followed version's (the writer
/// fills it from the log, and leaves it empty when that version is not held, which stays readable), and its id the
/// moving version's.
pub const LEFT: &str = "left";

const AT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%:z";

/// One line of a note's log. Fields a reader does not know are ignored, and an event it does not know is kept as is.
/// The fields after `at` stay out of the id and are written only when set, so a local version's line is as before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The device that recorded it, on a version that came through sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Parents a receiver must not wait for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outside: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    /// Each conflicting passage of a merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict: Vec<Conflict>,
    /// The lines a resolution dropped, per passage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<Dropped>,
}

/// A passage a merge left in conflict: its heading path and the versions of its sides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub passage: String,
    pub sides: Vec<String>,
}

/// The lines of a conflict's sides that the first version without its blocks no longer holds, for one passage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    pub passage: String,
    pub lines: Vec<String>,
}

impl Version {
    pub fn is_deleted(&self) -> bool {
        self.blob == DELETED
    }

    /// Whether it is a `left` record, which `is_deleted` covers too.
    pub fn is_left(&self) -> bool {
        self.event == LEFT
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

/// The second line type of a note's log: the conflict version a `bilbo sync declare` named, and why its drops are
/// intended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    pub declare: String,
    pub reason: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

fn is_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A record is readable when its hashes are hex, so a blob never names a path outside `blobs/`, and its file name is
/// a note name, so a restore never writes a stray one. A `left` record's id is the moving version's, so nothing
/// re-derives an id here.
pub fn readable(v: &Version) -> bool {
    is_hex(&v.version)
        && v.parents.iter().all(|p| is_hex(p))
        && (is_hex(&v.blob) || v.is_deleted())
        && (parse_name(&v.file).is_ok() || (v.is_left() && v.file.is_empty()))
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

/// A fresh `.tmp-<random>` path beside the files of `dir`, the form the watcher's start-up sweep removes.
pub fn temp_path(dir: &Path) -> Result<PathBuf, String> {
    Ok(dir.join(format!("{TMP_PREFIX}{}", temp_name()?)))
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

/// A note's log as read: its versions with the bytes of each one's line, its declarations, and the numbers of the
/// complete lines that did not parse.
#[derive(Debug, Default)]
pub struct Log {
    pub versions: Vec<Version>,
    /// `raw[i]` is the line `versions[i]` was read from, so a rewrite never re-encodes a record.
    pub raw: Vec<Vec<u8>>,
    pub declarations: Vec<Declaration>,
    /// `declaration_raw[i]` is the line `declarations[i]` was read from, with the number of versions before it.
    pub declaration_raw: Vec<(usize, Vec<u8>)>,
    pub unreadable: Vec<usize>,
}

impl Log {
    /// The last line, which is not the head the file holds once a log has several.
    pub fn latest(&self) -> Option<&Version> {
        self.versions.last()
    }
}

enum Line {
    Version(Version),
    Declaration(Declaration),
}

fn parse_line(line: &[u8]) -> Option<Line> {
    if let Ok(version) = serde_json::from_slice::<Version>(line) {
        return readable(&version).then_some(Line::Version(version));
    }
    let declaration: Declaration = serde_json::from_slice(line).ok()?;
    is_hex(&declaration.declare).then_some(Line::Declaration(declaration))
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
            Some(Line::Version(version)) => {
                log.versions.push(version);
                log.raw.push(line.to_vec());
            }
            Some(Line::Declaration(declaration)) => {
                log.declarations.push(declaration);
                log.declaration_raw
                    .push((log.versions.len(), line.to_vec()));
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
    append_line(lock, note_id, version)
}

/// Appends a declaration under the same rules as `append`.
pub fn append_declaration(
    lock: &Lock,
    note_id: &str,
    declaration: &Declaration,
) -> Result<(), String> {
    append_line(lock, note_id, declaration)
}

fn append_line(lock: &Lock, note_id: &str, entry: &impl Serialize) -> Result<(), String> {
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
    line.extend(serde_json::to_vec(entry).map_err(|e| format!("cannot encode a record: {e}"))?);
    line.push(b'\n');
    file.write_all(&line)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_message("append to", &path, &e))
}

/// The parent a change 1 log gives its next version: the id of the last line, none for an empty log. A log that
/// holds several heads has no such parent, so a caller that knows the head the file held passes that instead.
pub fn last_parents(root: &Path, note_id: &str) -> Result<Vec<String>, String> {
    Ok(load(root, note_id)?
        .versions
        .pop()
        .map(|v| v.version)
        .into_iter()
        .collect())
}

/// Records `file` with `bytes` (`None` for a deletion) as a version that follows `parents`: the versions the file
/// held before this change, none for a first version, every member of a head group for a version that follows it.
pub fn record(
    lock: &Lock,
    note_id: &str,
    parents: &[String],
    file: &str,
    bytes: Option<&[u8]>,
    event: &str,
    at: &str,
) -> Result<Version, String> {
    let blob = match bytes {
        Some(bytes) => write_blob(lock, bytes)?,
        None => DELETED.to_string(),
    };
    let version = Version {
        version: version_id(note_id, parents, file, &blob),
        parents: parents.to_vec(),
        file: file.to_string(),
        blob,
        event: event.to_string(),
        at: at.to_string(),
        ..Version::default()
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

/// Compares the note's current `(file name, bytes)` with the log's last line and records the difference, if any.
/// `record_difference_after` names the version the file held instead.
pub fn record_difference(
    lock: &Lock,
    note_id: &str,
    current: Option<(&str, &[u8])>,
    at: &str,
) -> Result<Option<Version>, String> {
    let latest = load(lock.root(), note_id)?.versions.pop();
    record_difference_after(lock, note_id, latest.as_ref(), current, at)
}

/// Compares the note's current `(file name, bytes)` with `latest`, the version the file held last, and records the
/// difference after it, if any.
pub fn record_difference_after(
    lock: &Lock,
    note_id: &str,
    latest: Option<&Version>,
    current: Option<(&str, &[u8])>,
    at: &str,
) -> Result<Option<Version>, String> {
    let digest = current.map(|(_, bytes)| hash::sha256_hex(bytes));
    let seen = current
        .zip(digest.as_deref())
        .map(|((file, _), digest)| (file, digest));
    let Some(event) = event_for(latest, seen) else {
        return Ok(None);
    };
    let parents: Vec<String> = latest.iter().map(|v| v.version.clone()).collect();
    let version = record(
        lock,
        note_id,
        &parents,
        current.map_or_else(|| latest.map_or("", |v| &v.file), |(file, _)| file),
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
/// left in place, and so is one whose bytes are a staged version of its note (`staged` holds note id and blob hash
/// pairs): the watcher's inbound write is still to land. Returns the messages to print.
pub fn sweep_restore_leftovers(
    lock: &Lock,
    at: &str,
    staged: &BTreeSet<(String, String)>,
) -> Result<Vec<String>, String> {
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
        let digest = hash::sha256_hex(&bytes);
        if staged.contains(&(id.to_string(), digest.clone())) {
            continue;
        }
        let Some(latest) = log.latest() else {
            messages.push(format!(
                "notes/{name} left in place: no history for its note"
            ));
            continue;
        };
        if log.versions.iter().all(|v| v.blob != digest) {
            let parents = [latest.version.clone()];
            record(lock, id, &parents, &latest.file, Some(&bytes), EDITED, at)?;
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

/// The heads of `versions`: the versions no other version lists as a parent, grouped by what they hold. A group is
/// the heads with one file and one blob (a `left` apart from a `deleted`), sorted by id, and counts as one head: the
/// next version follows every member. Groups are sorted by their first id. A record repeated in the log counts once,
/// and of all the `merged` versions with the same parents only the first id is a head, even after a version follows
/// it, so devices that hold the same versions compute the same heads.
pub fn heads(versions: &[Version]) -> Vec<Vec<&Version>> {
    let mut seen: HashSet<&str> = HashSet::new();
    let unique: Vec<&Version> = versions
        .iter()
        .filter(|v| seen.insert(v.version.as_str()))
        .collect();
    let followed: HashSet<&str> = unique
        .iter()
        .flat_map(|v| v.parents.iter().map(String::as_str))
        .collect();
    fn parent_set(v: &Version) -> Vec<&str> {
        let mut parents: Vec<&str> = v.parents.iter().map(String::as_str).collect();
        parents.sort_unstable();
        parents
    }
    let mut first_merge: HashMap<Vec<&str>, &str> = HashMap::new();
    for v in unique.iter().filter(|v| v.event == MERGED) {
        let first = first_merge.entry(parent_set(v)).or_insert(&v.version);
        *first = (*first).min(v.version.as_str());
    }
    let mut groups: BTreeMap<(&str, &str, bool), Vec<&Version>> = BTreeMap::new();
    for v in &unique {
        let loser = v.event == MERGED && first_merge[&parent_set(v)] != v.version;
        if !followed.contains(v.version.as_str()) && !loser {
            groups
                .entry((v.file.as_str(), v.blob.as_str(), v.is_left()))
                .or_default()
                .push(v);
        }
    }
    let mut groups: Vec<Vec<&Version>> = groups.into_values().collect();
    for group in &mut groups {
        group.sort_by(|a, b| a.version.cmp(&b.version));
    }
    groups.sort_by(|a, b| a[0].version.cmp(&b[0].version));
    groups
}

/// The lowest common versions of `a` and `b` (each counts as its own ancestor): the common ancestors no other common
/// ancestor follows. A criss-cross merge has several, and versions that share none, or an id `versions` lacks, have
/// none. Sorted by id.
pub fn lowest_common<'a>(versions: &'a [Version], a: &str, b: &str) -> Vec<&'a Version> {
    let by_id: HashMap<&str, &Version> = versions.iter().map(|v| (v.version.as_str(), v)).collect();
    let ancestors = |start: &str| -> HashSet<&'a str> {
        let mut seen = HashSet::new();
        let mut stack: Vec<&str> = by_id
            .get_key_value(start)
            .map(|(id, _)| *id)
            .into_iter()
            .collect();
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(
                    by_id[id]
                        .parents
                        .iter()
                        .map(String::as_str)
                        .filter(|p| by_id.contains_key(p)),
                );
            }
        }
        seen
    };
    let (of_a, of_b) = (ancestors(a), ancestors(b));
    let common: Vec<&str> = of_a.intersection(&of_b).copied().collect();
    let mut below: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = common
        .iter()
        .flat_map(|id| by_id[id].parents.iter().map(String::as_str))
        .filter(|p| by_id.contains_key(p))
        .collect();
    while let Some(id) = stack.pop() {
        if below.insert(id) {
            stack.extend(
                by_id[id]
                    .parents
                    .iter()
                    .map(String::as_str)
                    .filter(|p| by_id.contains_key(p)),
            );
        }
    }
    let mut lowest: Vec<&Version> = common
        .into_iter()
        .filter(|id| !below.contains(id))
        .map(|id| by_id[id])
        .collect();
    lowest.sort_by(|x, y| x.version.cmp(&y.version));
    lowest
}

/// What a prune must keep of one note on top of retention's own rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hold {
    /// These versions and every version that follows any of them: the newest versions known devices hold.
    After(BTreeSet<String>),
    /// The whole log: a device that holds no known version yet.
    All,
}

/// The holds per note id. A note with no entry, like an empty guard, is pruned by retention's rules alone.
pub type Guard = BTreeMap<String, Hold>;

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
/// deleted note, the tombstone and the version before it, and what the note's `hold` names. A version with an
/// unreadable time stays.
fn kept(versions: &[Version], cutoff: jiff::Timestamp, hold: Option<&Hold>) -> Vec<bool> {
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
    match hold {
        None => {}
        Some(Hold::All) => keep.fill(true),
        Some(Hold::After(heads)) => {
            let mut held: HashSet<&str> = versions
                .iter()
                .map(|v| v.version.as_str())
                .filter(|id| heads.contains(*id))
                .collect();
            loop {
                let before = held.len();
                for v in versions {
                    if v.parents.iter().any(|p| held.contains(p.as_str())) {
                        held.insert(&v.version);
                    }
                }
                if held.len() == before {
                    break;
                }
            }
            for (k, v) in keep.iter_mut().zip(versions) {
                *k |= held.contains(v.version.as_str());
            }
        }
    }
    keep
}

/// Drops the versions older than `keep_days` days before `now`, then removes the content no kept version names. A
/// log with an unreadable line is left as it is and blocks every content removal. A declaration goes with the
/// version it names. `staged` holds the blobs of versions that arrived and wait to reach a log, which no log names
/// yet and a removal must keep.
pub fn prune(
    lock: &Lock,
    keep_days: u32,
    now: jiff::Timestamp,
    guard: &Guard,
    staged: &BTreeSet<String>,
) -> Result<Pruned, String> {
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
        let keep = kept(&log.versions, cutoff, guard.get(&id));
        let dropped = keep.iter().filter(|k| !**k).count();
        if dropped == 0 {
            continue;
        }
        let gone: HashSet<&str> = log
            .versions
            .iter()
            .zip(&keep)
            .filter(|(_, k)| !**k)
            .map(|(v, _)| v.version.as_str())
            .collect();
        let mut kept_lines: Vec<&[u8]> = Vec::new();
        let mut declarations = log.declarations.iter().zip(&log.declaration_raw).peekable();
        for (n, line) in log.raw.iter().enumerate() {
            while let Some((d, (_, raw))) = declarations.next_if(|(_, (after, _))| *after <= n) {
                if !gone.contains(d.declare.as_str()) {
                    kept_lines.push(raw);
                }
            }
            if keep[n] {
                kept_lines.push(line);
            }
        }
        for (d, (_, raw)) in declarations {
            if !gone.contains(d.declare.as_str()) {
                kept_lines.push(raw);
            }
        }
        rewrite(root, &id, &kept_lines)?;
        pruned.versions += dropped;
    }
    if !blocked {
        remove_unnamed_blobs(root, staged)?;
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

fn remove_unnamed_blobs(root: &Path, staged: &BTreeSet<String>) -> Result<(), String> {
    let mut named: HashSet<String> = HashSet::new();
    for id in note_ids(root)? {
        named.extend(load(root, &id)?.versions.into_iter().map(|v| v.blob));
    }
    named.extend(staged.iter().cloned());
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

/// Writes the version's bytes to the hidden file, with the permissions of the file it will replace.
pub fn write_temp(temp: &Path, bytes: &[u8], current: Option<&str>) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", temp.display());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(fail)?;
    // The file is ours from here on, so a failure removes it.
    let filled = (|| {
        file.write_all(bytes)?;
        if let Some(name) = current
            && let Ok(meta) = fs::metadata(temp.with_file_name(name))
        {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()
    })();
    filled.map_err(|e| {
        let _ = fs::remove_file(temp);
        fail(e)
    })
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

    /// `record` after the log's last line, as change 1's linear logs did.
    fn rec(
        lock: &Lock,
        note_id: &str,
        file: &str,
        bytes: Option<&[u8]>,
        event: &str,
        at: &str,
    ) -> Version {
        let parents = last_parents(lock.root(), note_id).unwrap();
        record(lock, note_id, &parents, file, bytes, event, at).unwrap()
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
        rec(&lock, &id(1), "plan-x.md", Some(b"text"), ADDED, NOW);
        assert_eq!(events(root, &id(1)), ["added plan-x.md"]);
        assert_eq!(blob_files(root), 1);
    }

    #[test]
    fn a_complete_last_line_without_a_newline_is_kept_and_finished() {
        let scratch = scratch("tail-kept");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        rec(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW);
        let path = log_path(root, &id(1));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.trim_end_matches('\n')).unwrap();
        assert_eq!(load(root, &id(1)).unwrap().versions.len(), 1);

        rec(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW);

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
        rec(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW);
        let path = log_path(root, &id(1));
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(r#"{"version":"abc"#);
        fs::write(&path, &text).unwrap();
        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.versions.len(), 1);
        assert!(log.unreadable.is_empty());

        rec(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW);

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
        let first = rec(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW);
        let second = rec(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW);
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
        rec(&lock, &id(1), "decision-release.md", Some(b"x"), ADDED, NOW);
        rec(&lock, &id(2), "plan-release.md", Some(b"y"), ADDED, NOW);
        rec(&lock, &id(2), "plan-release.md", None, DELETED, NOW);

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
            rec(&lock, &id(n), file, Some(b"x"), ADDED, &at);
            rec(&lock, &id(n), file, None, DELETED, &at);
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
        rec(&lock, &id(1), "plan-renamed.md", Some(b"x"), ADDED, NOW);
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
                ..Version::default()
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
        let v = rec(&lock, &id(1), "plan-x.md", Some(b"text"), ADDED, NOW);
        fs::remove_file(blob_path(root, &v.blob)).unwrap();
        assert_eq!(content(root, &v), Err(ContentError::Pruned));
        let gone = rec(&lock, &id(1), "plan-x.md", None, DELETED, NOW);
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
            ..Version::default()
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
        rec(&lock, &id(1), "decision-x.md", Some(b"known"), ADDED, NOW);
        rec(&lock, &id(2), "plan-y.md", Some(b"known"), ADDED, NOW);
        fs::write(restore_path(root, &id(1)), "unrecorded").unwrap();
        fs::write(restore_path(root, &id(2)), "known").unwrap();
        fs::write(restore_path(root, &id(3)), "orphan").unwrap();
        fs::write(root.join("notes/.bilbo-restore-nope"), "not ours").unwrap();

        let messages = sweep_restore_leftovers(&lock, NOW, &BTreeSet::new()).unwrap();

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
        let v = rec(&lock, &id(1), "plan-x.md", Some(b"text"), ADDED, NOW);
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
        rec(lock, &id(note), file, bytes, event, &days_ago(days));
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

        let pruned = prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new()).unwrap();

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
        assert_eq!(
            prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new())
                .unwrap()
                .versions,
            0
        );
    }

    #[test]
    fn a_note_untouched_for_a_year_keeps_its_only_version() {
        let scratch = scratch("retention-year");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        record_at(&lock, 1, "plan-x.md", "old", ADDED, 400);
        assert_eq!(
            prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new())
                .unwrap()
                .versions,
            0
        );
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

        let pruned = prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new()).unwrap();

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

        let pruned = prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new()).unwrap();

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
        let pruned = prune(&lock, 30, now(), &Guard::new(), &BTreeSet::new()).unwrap();
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
            "{},  \"device\":\"d1\",\"later\":[1, 2] }}",
            lines[1].trim_end_matches('}')
        );
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        assert_eq!(
            prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new())
                .unwrap()
                .versions,
            1
        );

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
        rec(&lock, &id(1), name, Some(b"small"), ADDED, NOW);

        for arg in ["release".to_string(), id(1)] {
            let named = resolved(root, &arg).unwrap();
            assert_eq!(named.id, id(1));
            assert_eq!(named.file, None);
            assert_eq!(named.skipped, [name]);
        }
    }

    /// Writes a log of versions, each `(label, parent labels, event, days ago)`, with the label as its text, and
    /// returns the id of each label.
    fn dag(lock: &Lock, note: u8, specs: &[(&str, &[&str], &str, i64)]) -> HashMap<String, String> {
        let mut ids: HashMap<String, String> = HashMap::new();
        for (label, parents, event, days) in specs {
            let blob = write_blob(lock, label.as_bytes()).unwrap();
            let parents: Vec<String> = parents.iter().map(|p| ids[*p].clone()).collect();
            let version = Version {
                version: version_id(&id(note), &parents, "plan-x.md", &blob),
                parents,
                file: "plan-x.md".into(),
                blob,
                event: (*event).into(),
                at: days_ago(*days),
                ..Version::default()
            };
            append(lock, &id(note), &version).unwrap();
            ids.insert((*label).into(), version.version);
        }
        ids
    }

    fn labels(ids: &HashMap<String, String>, versions: &[&Version]) -> Vec<String> {
        let mut found: Vec<String> = versions
            .iter()
            .map(|v| {
                ids.iter()
                    .find(|(_, id)| **id == v.version)
                    .unwrap()
                    .0
                    .clone()
            })
            .collect();
        found.sort();
        found
    }

    fn logged(root: &Path, note: u8) -> Vec<Version> {
        load(root, &id(note)).unwrap().versions
    }

    fn held(root: &Path, note: u8, ids: &HashMap<String, String>) -> Vec<String> {
        let versions = logged(root, note);
        let mut found: Vec<String> = ids
            .iter()
            .filter(|(_, id)| versions.iter().any(|v| &v.version == *id))
            .map(|(label, _)| label.clone())
            .collect();
        found.sort();
        found
    }

    #[test]
    fn a_record_with_and_without_the_new_fields_has_one_id() {
        let blob = "c".repeat(64);
        let parents = ["a".repeat(64), "b".repeat(64)];
        let plain = Version {
            version: version_id(&id(1), &parents, "plan-x.md", &blob),
            parents: parents.to_vec(),
            file: "plan-x.md".into(),
            blob: blob.clone(),
            event: MERGED.into(),
            at: NOW.into(),
            ..Version::default()
        };
        let full = Version {
            device: Some("d1".into()),
            outside: vec!["e".repeat(64)],
            flags: vec!["stale-base".into()],
            conflict: vec![Conflict {
                passage: "# A".into(),
                sides: vec!["a".repeat(12)],
            }],
            dropped: vec![Dropped {
                passage: "# A".into(),
                lines: vec!["x".into()],
            }],
            ..plain.clone()
        };
        assert_eq!(
            version_id(&id(1), &full.parents, &full.file, &full.blob),
            plain.version
        );
        let line = serde_json::to_string(&plain).unwrap();
        for key in ["device", "outside", "flags", "conflict", "dropped"] {
            assert!(!line.contains(key), "{line}");
        }
        let back: Version = serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(back, full);
    }

    #[test]
    fn a_left_record_is_read_as_a_deletion_and_keeps_its_id() {
        let scratch = scratch("left");
        let root = &scratch.0;
        let (moving, parent) = ("a".repeat(64), "b".repeat(64));
        fs::create_dir_all(log_dir(root)).unwrap();
        let line = format!(
            r#"{{"version":"{moving}","parents":["{parent}"],"file":"plan-x.md","blob":"deleted","event":"left","at":"{NOW}"}}"#
        );
        fs::write(log_path(root, &id(1)), format!("{line}\n")).unwrap();

        let log = load(root, &id(1)).unwrap();

        assert!(log.unreadable.is_empty());
        let left = &log.versions[0];
        assert_eq!(left.version, moving);
        assert!(left.is_left() && left.is_deleted());
        assert_ne!(
            version_id(&id(1), &left.parents, &left.file, &left.blob),
            moving
        );
        assert_eq!(content(root, left), Err(ContentError::Deleted));
        assert_eq!(event_for(Some(left), None), None);
        assert_eq!(
            event_for(Some(left), Some(("plan-x.md", &"c".repeat(64)))),
            Some(EDITED)
        );
    }

    #[test]
    fn declarations_are_read_beside_versions_and_never_unreadable() {
        let scratch = scratch("declare");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let first = rec(&lock, &id(1), "plan-x.md", Some(b"one"), ADDED, NOW);
        let declaration = Declaration {
            declare: first.version.clone(),
            reason: "tidy-up".into(),
            at: NOW.into(),
            device: Some("d1".into()),
        };
        append_declaration(&lock, &id(1), &declaration).unwrap();
        rec(&lock, &id(1), "plan-x.md", Some(b"two"), EDITED, NOW);

        let log = load(root, &id(1)).unwrap();

        assert!(log.unreadable.is_empty());
        assert_eq!(log.versions.len(), 2);
        assert_eq!(log.declarations, std::slice::from_ref(&declaration));
        assert_eq!(log.declaration_raw[0].0, 1);
        assert_eq!(
            log.declaration_raw[0].1,
            serde_json::to_vec(&declaration).unwrap()
        );
        assert_eq!(log.latest().unwrap().event, EDITED);
    }

    #[test]
    fn a_declaration_with_an_unknown_field_is_read_and_one_naming_no_version_is_not() {
        let scratch = scratch("declare-shape");
        let root = &scratch.0;
        let v = "a".repeat(64);
        fs::create_dir_all(log_dir(root)).unwrap();
        let good =
            format!(r#"{{"declare":"{v}","reason":"r","at":"{NOW}","device":"d","sig":"zz"}}"#);
        let bad = format!(r#"{{"declare":"nope","reason":"r","at":"{NOW}"}}"#);
        fs::write(log_path(root, &id(1)), format!("{good}\n{bad}\n{good}\n")).unwrap();
        let log = load(root, &id(1)).unwrap();
        assert_eq!(log.declarations.len(), 2);
        assert_eq!(log.unreadable, [2]);
    }

    fn head_labels(ids: &HashMap<String, String>, groups: &[Vec<&Version>]) -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|group| {
                let mut found = labels(ids, group);
                found.sort();
                found
            })
            .collect()
    }

    #[test]
    fn heads_are_the_versions_nobody_follows() {
        let scratch = scratch("heads");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 5),
                ("b", &["a"], EDITED, 4),
                ("c", &["a"], EDITED, 3),
            ],
        );
        let versions = logged(&scratch.0, 1);
        let mut found = head_labels(&ids, &heads(&versions));
        found.sort();
        assert_eq!(found, [["b"], ["c"]]);
        assert_eq!(head_labels(&ids, &heads(&versions[..2])), [["b"]]);
        assert!(heads(&[]).is_empty());
    }

    #[test]
    fn heads_with_one_content_form_one_group_that_the_next_version_follows() {
        let scratch = scratch("heads-content");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 5),
                ("t", &["a"], EDITED, 4),
                ("u", &["a"], EDITED, 4),
            ],
        );
        let mut versions = logged(root, 1);
        let blob = versions[1].blob.clone();
        let twin = Version {
            version: version_id(&id(1), &[ids["u"].clone()], "plan-x.md", &blob),
            parents: vec![ids["u"].clone()],
            blob,
            ..versions[1].clone()
        };
        versions.push(twin);

        let found = heads(&versions);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].len(), 2);
        assert!(found[0][0].version < found[0][1].version);
        let parents: Vec<String> = found[0].iter().map(|v| v.version.clone()).collect();
        let next = Version {
            version: version_id(&id(1), &parents, "plan-x.md", &"d".repeat(64)),
            parents,
            blob: "d".repeat(64),
            ..versions[1].clone()
        };
        versions.push(next.clone());
        let found = heads(&versions);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].len(), 1);
        assert_eq!(found[0][0].version, next.version);
    }

    #[test]
    fn of_two_merges_with_the_same_parents_the_first_id_stays() {
        let scratch = scratch("heads-merged");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 5),
                ("b", &["a"], EDITED, 4),
                ("c", &["a"], EDITED, 4),
                ("m1", &["b", "c"], MERGED, 3),
                ("m2", &["c", "b"], MERGED, 3),
            ],
        );
        let versions = logged(&scratch.0, 1);
        let found = heads(&versions);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0][0].version,
            ids["m1"].clone().min(ids["m2"].clone())
        );
    }

    #[test]
    fn the_losing_merge_stays_out_after_a_version_follows_the_winner() {
        let scratch = scratch("heads-loser");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 5),
                ("b", &["a"], EDITED, 4),
                ("c", &["a"], EDITED, 4),
                ("m1", &["b", "c"], MERGED, 3),
                ("m2", &["c", "b"], MERGED, 3),
            ],
        );
        let (winner, loser) = if ids["m1"] < ids["m2"] {
            ("m1", "m2")
        } else {
            ("m2", "m1")
        };
        let x = record(
            &lock,
            &id(1),
            &[ids[winner].clone()],
            "plan-x.md",
            Some(b"x"),
            EDITED,
            NOW,
        )
        .unwrap();

        let versions = logged(&scratch.0, 1);
        let found = heads(&versions);

        assert_eq!(found.len(), 1, "{loser} must not come back as a head");
        assert_eq!(found[0][0].version, x.version);
    }

    #[test]
    fn a_record_repeated_in_a_log_counts_once() {
        let scratch = scratch("heads-dup");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(&lock, 1, &[("a", &[], ADDED, 5), ("b", &["a"], EDITED, 4)]);
        let mut versions = logged(&scratch.0, 1);
        let again = Version {
            event: LEFT.into(),
            blob: DELETED.into(),
            ..versions[1].clone()
        };
        versions.push(again);
        assert_eq!(head_labels(&ids, &heads(&versions)), [["b"]]);
    }

    #[test]
    fn a_left_head_is_not_grouped_with_a_deletion() {
        let v = |n: u8, event: &str| Version {
            version: format!("{n:02x}").repeat(32),
            parents: vec!["aa".repeat(32)],
            file: "plan-x.md".into(),
            blob: DELETED.into(),
            event: event.into(),
            at: NOW.into(),
            ..Version::default()
        };
        let versions = [
            Version {
                parents: vec![],
                ..v(0xaa, ADDED)
            },
            v(0xb1, DELETED),
            v(0xb2, LEFT),
        ];
        assert_eq!(heads(&versions).len(), 2);
    }

    #[test]
    fn a_left_whose_followed_version_is_unknown_stays_readable() {
        let scratch = scratch("left-nofile");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let first = rec(
            &lock,
            &id(2),
            "plan-y.md",
            Some(b"x"),
            ADDED,
            &days_ago(200),
        );
        rec(
            &lock,
            &id(2),
            "plan-y.md",
            Some(b"z"),
            EDITED,
            &days_ago(150),
        );
        rec(&lock, &id(2), "plan-y.md", Some(b"y"), EDITED, &days_ago(1));
        fs::create_dir_all(log_dir(root)).unwrap();
        let line = format!(
            r#"{{"version":"{}","parents":["{}"],"file":"","blob":"deleted","event":"left","at":"{NOW}"}}"#,
            "a".repeat(64),
            "b".repeat(64)
        );
        fs::write(log_path(root, &id(1)), format!("{line}\n")).unwrap();

        let log = load(root, &id(1)).unwrap();
        assert!(log.unreadable.is_empty());
        assert!(log.versions[0].is_left());
        let pruned = prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new()).unwrap();

        assert!(pruned.warnings.is_empty(), "{:?}", pruned.warnings);
        assert!(!blob_path(root, &first.blob).exists());
    }

    #[test]
    fn the_lowest_common_version_of_a_fork_is_its_root() {
        let scratch = scratch("lca-fork");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 5),
                ("b", &["a"], EDITED, 4),
                ("c", &["b"], EDITED, 3),
                ("d", &["b"], EDITED, 3),
            ],
        );
        let versions = logged(&scratch.0, 1);
        let found = lowest_common(&versions, &ids["c"], &ids["d"]);
        assert_eq!(labels(&ids, &found), ["b"]);
        let found = lowest_common(&versions, &ids["c"], &ids["b"]);
        assert_eq!(labels(&ids, &found), ["b"]);
    }

    #[test]
    fn a_criss_cross_has_several_lowest_common_versions() {
        let scratch = scratch("lca-criss");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 9),
                ("b", &["a"], EDITED, 8),
                ("c", &["a"], EDITED, 8),
                ("m1", &["b", "c"], MERGED, 7),
                ("m2", &["c", "b"], MERGED, 7),
                ("x", &["m1"], EDITED, 6),
                ("y", &["m2"], EDITED, 6),
            ],
        );
        let versions = logged(&scratch.0, 1);
        let found = lowest_common(&versions, &ids["x"], &ids["y"]);
        assert_eq!(labels(&ids, &found), ["b", "c"]);
    }

    #[test]
    fn versions_with_no_common_ancestor_have_no_lowest_common() {
        let scratch = scratch("lca-none");
        let lock = lock(&scratch.0).unwrap();
        let ids = dag(&lock, 1, &[("a", &[], ADDED, 5), ("b", &[], ADDED, 5)]);
        let versions = logged(&scratch.0, 1);
        assert!(lowest_common(&versions, &ids["a"], &ids["b"]).is_empty());
        assert!(lowest_common(&versions, &ids["a"], &"9".repeat(64)).is_empty());
    }

    #[test]
    fn the_restore_sweep_leaves_a_staged_version() {
        let scratch = scratch("sweep-staged");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        rec(&lock, &id(1), "decision-x.md", Some(b"known"), ADDED, NOW);
        rec(&lock, &id(2), "plan-y.md", Some(b"known"), ADDED, NOW);
        fs::write(restore_path(root, &id(1)), "inbound").unwrap();
        fs::write(restore_path(root, &id(2)), "inbound").unwrap();
        let staged = BTreeSet::from([(id(1), hash::sha256_hex(b"inbound"))]);

        let messages = sweep_restore_leftovers(&lock, NOW, &staged).unwrap();

        assert!(restore_path(root, &id(1)).exists());
        assert_eq!(events(root, &id(1)), ["added decision-x.md"]);
        assert!(!restore_path(root, &id(2)).exists());
        assert_eq!(
            events(root, &id(2)),
            ["added plan-y.md", "edited plan-y.md"]
        );
        assert_eq!(messages.len(), 1, "{messages:?}");
    }

    #[test]
    fn the_restore_sweep_leaves_a_staged_version_of_a_note_with_no_history() {
        let scratch = scratch("sweep-staged-new");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        fs::write(restore_path(root, &id(1)), "inbound").unwrap();
        fs::write(restore_path(root, &id(2)), "other").unwrap();
        let staged = BTreeSet::from([(id(1), hash::sha256_hex(b"inbound"))]);

        let messages = sweep_restore_leftovers(&lock, NOW, &staged).unwrap();

        assert!(restore_path(root, &id(1)).exists());
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains("no history"), "{messages:?}");
    }

    #[test]
    fn a_prune_keeps_a_declaration_with_its_conflict_version_only() {
        let scratch = scratch("prune-declare");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 300),
                ("b", &["a"], EDITED, 250),
                ("c", &["b"], EDITED, 100),
                ("d", &["c"], EDITED, 10),
            ],
        );
        let declare = |label: &str| Declaration {
            declare: ids[label].clone(),
            reason: label.into(),
            at: NOW.into(),
            device: None,
        };
        append_declaration(&lock, &id(1), &declare("a")).unwrap();
        append_declaration(&lock, &id(1), &declare("d")).unwrap();
        append_declaration(&lock, &id(1), &declare("b")).unwrap();

        let pruned = prune(&lock, 90, now(), &Guard::new(), &BTreeSet::new()).unwrap();

        assert_eq!(pruned.versions, 2);
        let log = load(root, &id(1)).unwrap();
        let reasons: Vec<&str> = log.declarations.iter().map(|d| d.reason.as_str()).collect();
        assert_eq!(reasons, ["d"]);
        assert!(log.unreadable.is_empty());
    }

    #[test]
    fn a_device_away_for_longer_than_the_window_holds_its_versions() {
        let scratch = scratch("guard-after");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 120),
                ("b", &["a"], EDITED, 100),
                ("c", &["b"], EDITED, 10),
            ],
        );
        let guard = Guard::from([(id(1), Hold::After(BTreeSet::from([ids["a"].clone()])))]);

        prune(&lock, 90, now(), &guard, &BTreeSet::new()).unwrap();

        assert_eq!(held(root, 1, &ids), ["a", "b", "c"]);
    }

    #[test]
    fn a_hold_keeps_what_follows_a_head_and_not_what_came_before() {
        let scratch = scratch("guard-between");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 300),
                ("b", &["a"], EDITED, 250),
                ("c", &["b"], EDITED, 200),
                ("d", &["c"], EDITED, 150),
                ("e", &["d"], EDITED, 100),
                ("f", &["e"], EDITED, 10),
            ],
        );
        let guard = Guard::from([(id(1), Hold::After(BTreeSet::from([ids["c"].clone()])))]);

        prune(&lock, 90, now(), &guard, &BTreeSet::new()).unwrap();

        assert_eq!(held(root, 1, &ids), ["c", "d", "e", "f"]);
    }

    #[test]
    fn a_device_that_never_acknowledged_holds_back_every_version() {
        let scratch = scratch("guard-all");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 200),
                ("b", &["a"], EDITED, 150),
                ("c", &["b"], EDITED, 10),
            ],
        );
        let other = dag(
            &lock,
            2,
            &[
                ("a", &[], ADDED, 200),
                ("b", &["a"], EDITED, 150),
                ("c", &["b"], EDITED, 10),
            ],
        );
        let guard = Guard::from([(id(1), Hold::All)]);

        prune(&lock, 90, now(), &guard, &BTreeSet::new()).unwrap();

        assert_eq!(held(root, 1, &ids), ["a", "b", "c"]);
        assert_eq!(held(root, 2, &other), ["b", "c"]);
        assert_eq!(blob_files(root), 3);
    }

    #[test]
    fn a_stale_device_holds_nothing_back() {
        let scratch = scratch("guard-none");
        let root = &scratch.0;
        let lock = lock(root).unwrap();
        let ids = dag(
            &lock,
            1,
            &[
                ("a", &[], ADDED, 200),
                ("b", &["a"], EDITED, 150),
                ("c", &["b"], EDITED, 100),
                ("d", &["c"], EDITED, 10),
            ],
        );

        prune(
            &lock,
            90,
            now(),
            &Guard::from([(id(9), Hold::All)]),
            &BTreeSet::new(),
        )
        .unwrap();

        assert_eq!(held(root, 1, &ids), ["c", "d"]);
    }
}
