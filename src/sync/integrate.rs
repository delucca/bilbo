//! Applying the versions other devices wrote: the inbox, heads, merges, inbound writes, the stale-base rule, deletes,
//! topic collisions and scope moves.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::host::swap;
use crate::note::conflicts::{self, Left};
use crate::note::merge::{self, Side};
use crate::note::versions::{self, Conflict, ContentError, Declaration, Dropped, Lock, Version};
use crate::note::{self, ScopeKey};
use crate::shared::{frontmatter, hash, store};
use crate::sync::segment::{Blob, Record};

/// Rounds one note's write may take: each one that finds a save made during the swap records it and plans again.
const MAX_ROUNDS: usize = 5;
const INBOX: &str = "inbox.jsonl";
const STALE_BASE: &str = "stale-base.json";
const WRITTEN: &str = "written.json";
const TMP_PREFIX: &str = ".tmp-";
const SECONDS_PER_DAY: i64 = 86_400;
/// How long a move out of a scope stays in the open-conflict summary.
const LEFT_DAYS: i64 = 30;
/// The flag of a rename that settled a topic collision.
const TOPIC_TAKEN: &str = "topic-taken";

/// What a note's write is about to do when it calls its hook. A hook that fails stops the work where it is, without
/// cleanup, as a kill would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Read the note's file again, before anything is planned from it.
    Prepare,
    /// Write the incoming bytes to the hidden file.
    Write,
    /// Exchange the hidden file with the note's file, rename it into place, or move the file out of the way.
    Swap,
    /// Rename the note's file, now holding the incoming bytes, to the version's name.
    Rename,
    /// Read what the swap took out.
    Inspect,
    /// Delete the hidden file.
    Remove,
    /// Append the staged versions and the merges to the note's log.
    Record,
}

/// What a sync cycle knows besides the store.
pub struct Params<'a> {
    /// Whether this device syncs a scope, for a merge's scope clash.
    pub syncs: &'a dyn Fn(&str) -> bool,
    pub now: jiff::Timestamp,
    /// `sync.stale_days`: a version whose parents never arrived is applied after this long.
    pub stale_days: u32,
}

/// One line of the inbox: a record or a declaration that arrived and has not reached a log yet.
#[derive(Serialize, Deserialize)]
struct Staged {
    /// When it was staged (RFC 3339).
    seen: String,
    /// The scope it arrived through.
    scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record: Option<Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    declaration: Option<Declaration>,
}

/// The stale-base entry of a note: the version its file held just before the first sync write, and the version of
/// the latest one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub base: String,
    pub written: String,
}

fn io_message(what: &str, path: &Path, e: &std::io::Error) -> String {
    format!("cannot {what} {}: {e}", path.display())
}

/// Writes `bytes` to `<dir>/<name>` through a temporary file and a rename.
fn replace(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| io_message("create", dir, &e))?;
    let id = frontmatter::mint_ulid().map_err(|e| format!("cannot mint a name: {e}"))?;
    let tmp = dir.join(format!("{TMP_PREFIX}{id}"));
    let path = dir.join(name);
    let written = fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    written.and_then(|()| fs::rename(&tmp, &path)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_message("write", &path, &e)
    })
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_message("read", path, &e)),
    }
}

/// The inbox, in staging order. A line that does not parse is skipped: only a crash leaves one, and only last.
fn read_inbox(root: &Path) -> Result<Vec<Staged>, String> {
    let path = store::sync_dir(root).join(INBOX);
    let Some(bytes) = read_optional(&path)? else {
        return Ok(Vec::new());
    };
    Ok(bytes
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect())
}

fn write_inbox(root: &Path, entries: &[Staged]) -> Result<(), String> {
    let dir = store::sync_dir(root);
    if entries.is_empty() {
        return match fs::remove_file(dir.join(INBOX)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(io_message("remove", &dir.join(INBOX), &e))
            }
            _ => Ok(()),
        };
    }
    let mut bytes = Vec::new();
    for entry in entries {
        bytes.extend(serde_json::to_vec(entry).map_err(|e| format!("cannot encode: {e}"))?);
        bytes.push(b'\n');
    }
    replace(&dir, INBOX, &bytes)
}

fn append_inbox(root: &Path, entries: &[Staged]) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let dir = store::sync_dir(root);
    fs::create_dir_all(&dir).map_err(|e| io_message("create", &dir, &e))?;
    let path = dir.join(INBOX);
    let mut bytes = Vec::new();
    if read_optional(&path)?.is_some_and(|old| old.last().is_some_and(|b| *b != b'\n')) {
        bytes.push(b'\n');
    }
    for entry in entries {
        bytes.extend(serde_json::to_vec(entry).map_err(|e| format!("cannot encode: {e}"))?);
        bytes.push(b'\n');
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| io_message("open", &path, &e))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_message("append to", &path, &e))
}

fn read_map<T: DeserializeOwned>(root: &Path, name: &str) -> Result<BTreeMap<String, T>, String> {
    let path = store::sync_dir(root).join(name);
    let Some(bytes) = read_optional(&path)? else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_slice(&bytes).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn write_map<T: Serialize>(
    root: &Path,
    name: &str,
    map: &BTreeMap<String, T>,
) -> Result<(), String> {
    let dir = store::sync_dir(root);
    if map.is_empty() {
        return match fs::remove_file(dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(io_message("remove", &dir.join(name), &e))
            }
            _ => Ok(()),
        };
    }
    let bytes = serde_json::to_vec(map).map_err(|e| format!("cannot encode: {e}"))?;
    replace(&dir, name, &bytes)
}

fn read_bases(root: &Path) -> Result<BTreeMap<String, Base>, String> {
    read_map(root, STALE_BASE)
}

fn write_bases(root: &Path, bases: &BTreeMap<String, Base>) -> Result<(), String> {
    write_map(root, STALE_BASE, bases)
}

/// What this device has not recorded yet for a note, kept until the write that records it commits:
/// - the file name and blob it put into the file (empty when the file does not hold them), written before each swap, so
///   a kill in between never turns the bytes into a save;
/// - the local saves held back for that write, so a kill after the swap still records them, after the version they
///   were made against, with the `stale-base` flag.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Written {
    #[serde(default)]
    file: String,
    #[serde(default)]
    blob: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extras: Vec<Version>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    flagged: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    clears: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    absent: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// The stale-base entry of a note, if a sync write left one.
#[cfg(test)]
pub fn stale_base(root: &Path, note: &str) -> Result<Option<Base>, String> {
    Ok(read_bases(root)?.remove(note))
}

/// Keeps a note's stale-base entry and makes `written`, a version a verb recorded, its written version. Without an
/// entry it does nothing.
pub fn keep_base(lock: &Lock, note: &str, written: &str) -> Result<(), String> {
    let mut bases = read_bases(lock.root())?;
    if let Some(entry) = bases.get_mut(note) {
        entry.written = written.to_string();
        write_bases(lock.root(), &bases)?;
    }
    Ok(())
}

/// The note ids and blob hashes of what waits for a log, for `versions::sweep_restore_leftovers`: the versions in the
/// inbox, the local saves a killed write held back, and the bytes it was writing.
pub fn staged(root: &Path) -> Result<BTreeSet<(String, String)>, String> {
    let mut out: BTreeSet<(String, String)> = read_inbox(root)?
        .into_iter()
        .filter_map(|entry| entry.record)
        .filter(|r| !r.version.is_deleted())
        .map(|r| (r.note, r.version.blob))
        .collect();
    for (note, w) in read_map::<Written>(root, WRITTEN)? {
        out.extend(
            w.extras
                .iter()
                .filter(|v| !v.is_deleted())
                .map(|v| (note.clone(), v.blob.clone())),
        );
        if !w.blob.is_empty() {
            out.insert((note, w.blob));
        }
    }
    Ok(out)
}

/// The blobs of what waits for a log, for `versions::prune`: no log names them yet.
pub fn staged_blobs(root: &Path) -> Result<BTreeSet<String>, String> {
    Ok(staged(root)?.into_iter().map(|(_, blob)| blob).collect())
}

/// Deletes the `.tmp-*` files a crash left under `<root>/.bilbo/sync/` and each scope's folder under
/// `<root>/.bilbo/scopes/`, and returns how many.
pub fn sweep_temporaries(root: &Path) -> Result<usize, String> {
    let mut dirs = vec![store::sync_dir(root)];
    if let Ok(scopes) = fs::read_dir(store::scopes_dir(root)) {
        for scope in scopes.filter_map(Result::ok) {
            dirs.push(scope.path());
            dirs.push(scope.path().join("out"));
        }
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

fn says_scope(text: &str, scope: &str) -> bool {
    matches!(note::read(text).scope, ScopeKey::Valid(ref name) if name == scope)
}

/// The scope a note's text names, when it names a valid one.
fn scope_of(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    match note::read(text).scope {
        ScopeKey::Valid(name) => Some(name),
        _ => None,
    }
}

/// A log's versions without the `left` records a full version of the same id supersedes: a device that syncs both
/// the scope a note left and the one it moved to gets the moving version whole through the second log.
fn effective(versions: Vec<Version>) -> Vec<Version> {
    let whole: HashSet<String> = versions
        .iter()
        .filter(|v| !v.is_left())
        .map(|v| v.version.clone())
        .collect();
    versions
        .into_iter()
        .filter(|v| !(v.is_left() && whole.contains(&v.version)))
        .collect()
}

/// Puts verified records, their blobs and declarations into the inbox, as one pull handed them over. A version
/// whose text does not say `scope` is skipped with a line, and one already in the inbox or the note's log is left
/// alone, so staging the same pull twice changes nothing. Returns the lines to print.
pub fn stage(
    lock: &Lock,
    scope: &str,
    records: &[Record],
    blobs: &[Blob],
    declarations: &[Declaration],
    now: jiff::Timestamp,
) -> Result<Vec<String>, String> {
    let root = lock.root();
    let mut events = Vec::new();
    for blob in blobs {
        match blob.bytes() {
            Ok(bytes) => {
                versions::write_blob(lock, &bytes)?;
            }
            Err(e) => events.push(format!("sync {scope}: skipped a blob: {e}")),
        }
    }
    let seen = now.to_string();
    // Whether a version id is held only as a `left`: a full version of that id still comes in over it.
    let mut taken: HashMap<String, bool> = HashMap::new();
    for record in read_inbox(root)?.into_iter().filter_map(|e| e.record) {
        let left = record.version.is_left();
        let only = taken.entry(record.version.version).or_insert(true);
        *only &= left;
    }
    let mut logs: HashMap<String, HashMap<String, bool>> = HashMap::new();
    let mut entries = Vec::new();
    for record in records {
        let v = &record.version;
        let in_log = match logs.get(&record.note) {
            Some(ids) => ids,
            None => {
                let mut ids: HashMap<String, bool> = HashMap::new();
                for v in versions::load(root, &record.note)?.versions {
                    let only = ids.entry(v.version.clone()).or_insert(true);
                    *only &= v.is_left();
                }
                logs.entry(record.note.clone()).or_insert(ids)
            }
        };
        let held = |held: &HashMap<String, bool>| {
            held.get(&v.version)
                .is_some_and(|only_left| v.is_left() || !only_left)
        };
        if held(&taken) || held(in_log) {
            continue;
        }
        if !v.is_deleted() {
            match versions::content(root, v) {
                Ok(bytes) => {
                    let says = std::str::from_utf8(&bytes).is_ok_and(|t| says_scope(t, scope));
                    if !says {
                        events.push(format!(
                            "sync {scope}: skipped version {} of {}: its text does not say scope {scope}",
                            v.short(),
                            record.note
                        ));
                        continue;
                    }
                }
                // The text may still come: the version waits until it does, and is checked then.
                Err(ContentError::Pruned) => {}
                Err(e) => return Err(format!("cannot read blob {}: {e:?}", v.blob)),
            }
        }
        taken.insert(v.version.clone(), v.is_left());
        entries.push(Staged {
            seen: seen.clone(),
            scope: scope.to_string(),
            record: Some(record.clone()),
            declaration: None,
        });
    }
    for declaration in declarations {
        entries.push(Staged {
            seen: seen.clone(),
            scope: scope.to_string(),
            record: None,
            declaration: Some(declaration.clone()),
        });
    }
    append_inbox(root, &entries)?;
    Ok(events)
}

/// The lines of the passages `previous` left open that `bytes` no longer holds a block for: the text those blocks
/// held that `bytes` lacks. Empty when `previous` has no conflict.
pub fn dropped_after(
    root: &Path,
    previous: &Version,
    bytes: &[u8],
) -> Result<Vec<Dropped>, String> {
    dropped_blocks(root, previous, None, bytes)
}

fn same_block(a: &merge::Block, b: &merge::Block) -> bool {
    a.passage == b.passage
        && a.sides
            .iter()
            .map(|s| &s.version)
            .eq(b.sides.iter().map(|s| &s.version))
}

/// What `bytes` dropped from the blocks of the passages `carrier` recorded as conflicts, limited to the blocks in
/// `open` when it is given: the ones a later version still held.
fn dropped_blocks(
    root: &Path,
    carrier: &Version,
    open: Option<&[merge::Block]>,
    bytes: &[u8],
) -> Result<Vec<Dropped>, String> {
    if carrier.conflict.is_empty() {
        return Ok(Vec::new());
    }
    let before = match versions::content(root, carrier) {
        Ok(before) => before,
        Err(ContentError::Pruned | ContentError::Deleted) => return Ok(Vec::new()),
        Err(ContentError::Io(e)) => return Err(e),
    };
    let after = String::from_utf8_lossy(bytes);
    let kept = merge::blocks(&after);
    let resolved: Vec<merge::Block> = merge::blocks(&String::from_utf8_lossy(&before))
        .into_iter()
        .filter(|block| carrier.conflict.iter().any(|c| c.passage == block.passage))
        .filter(|block| open.is_none_or(|open| open.iter().any(|o| same_block(o, block))))
        .filter(|block| !kept.iter().any(|k| same_block(k, block)))
        .collect();
    Ok(merge::dropped(&resolved, &after))
}

/// What a save of `bytes` after `previous` dropped from the conflicts `previous` holds open. Besides the passages
/// `previous` recorded itself, that is each passage of its nearest ancestors that recorded one whose block
/// `previous` still holds, so a resolution that follows an edit which kept the blocks drops what it drops.
fn dropped_since(
    root: &Path,
    view: &[Version],
    previous: &Version,
    bytes: &[u8],
) -> Result<Vec<Dropped>, String> {
    if !previous.conflict.is_empty() {
        return dropped_after(root, previous, bytes);
    }
    let Ok(held) = versions::content(root, previous) else {
        return Ok(Vec::new());
    };
    let held = merge::blocks(&String::from_utf8_lossy(&held));
    if held.is_empty() {
        return Ok(Vec::new());
    }
    let by_id: HashMap<&str, &Version> = view.iter().map(|v| (v.version.as_str(), v)).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack = vec![previous];
    let mut dropped: Vec<Dropped> = Vec::new();
    while let Some(v) = stack.pop() {
        for parent in v.parents.iter().filter_map(|p| by_id.get(p.as_str())) {
            if !seen.insert(parent.version.as_str()) {
                continue;
            }
            if parent.conflict.is_empty() {
                stack.push(parent);
                continue;
            }
            for found in dropped_blocks(root, parent, Some(&held), bytes)? {
                match dropped.iter_mut().find(|d| d.passage == found.passage) {
                    Some(d) => {
                        for line in found.lines {
                            if !d.lines.contains(&line) {
                                d.lines.push(line);
                            }
                        }
                    }
                    None => dropped.push(found),
                }
            }
        }
    }
    Ok(dropped)
}

/// What the file of a note is now.
enum Cur {
    Absent,
    File(versions::Found),
    /// Another file holds the id and is not recorded, so nothing is written for the note.
    Blocked,
}

/// Local versions that reach the log only once the write that merges them succeeded.
#[derive(Default)]
struct Pending {
    /// Saves recorded at commit, after the write.
    extras: Vec<Version>,
    /// The extras whose merge is flagged `stale-base`.
    flagged: HashSet<String>,
    /// Commit replaces the stale-base entry.
    clears: bool,
    /// The save was a delete: no file is expected.
    absent: bool,
}

impl Pending {
    fn extend(&mut self, other: Pending) {
        self.extras.extend(other.extras);
        self.flagged.extend(other.flagged);
        self.clears |= other.clears;
        self.absent |= other.absent;
    }
}

/// The staged versions of one note that wait for the log.
struct Waiting {
    /// Each with when it was staged.
    versions: Vec<(Version, jiff::Timestamp)>,
    /// The scope each arrived through, by version id.
    scopes: HashMap<String, String>,
    /// The scope of the last one.
    scope: String,
}

/// What one round of a note's settle planned, and what it found.
struct Round {
    plan: Plan,
    applicable: Vec<Version>,
    in_log: HashSet<String>,
    cur: Cur,
    scope: String,
    scopes: HashMap<String, String>,
}

/// A note's heads as a merge consumes them.
struct Head {
    members: Vec<Version>,
    bytes: Option<Vec<u8>>,
}

impl Head {
    fn rep(&self) -> &Version {
        &self.members[0]
    }
}

/// What one note's inbound versions come to: the merges to record, and what the file ends up holding.
struct Plan {
    merges: Vec<Version>,
    bytes: HashMap<String, Vec<u8>>,
    file: String,
    /// The text of the file, none for a deletion.
    content: Option<Vec<u8>>,
    final_id: String,
    /// The final version is a `left`: the file goes and the history stays.
    left: bool,
    conflicts: usize,
}

enum Wrote {
    Done,
    /// A save landed during the swap and is recorded; plan again.
    Raced,
    /// Something else is in the way; try again.
    Retry,
    /// Two saves landed during the swap: the older one, for this file name, is to be recorded before the newer one,
    /// which the file holds, and its hidden file removed.
    Landed(String, Vec<u8>),
    /// Nothing was written, and a line says why.
    Skipped,
}

struct Engine<'a> {
    lock: &'a Lock,
    p: &'a Params<'a>,
    at: String,
    hook: &'a mut dyn FnMut(Step) -> Result<(), String>,
    exchange: &'a mut dyn FnMut(&Path, &Path) -> Result<(), String>,
    events: Vec<String>,
    /// The file name and blob the file last held that a version of the note names.
    seen: Option<(String, String)>,
    /// The notes this call recorded a version for, whose entry in the open-conflict summary is rewritten at the end.
    touched: BTreeSet<String>,
}

/// The parent ids of the live head group whose file name and blob the file holds, every member of it: what a version
/// recorded for that file follows. None when the file is no head group's, so it is a save.
pub fn held_group(view: &[Version], file: &str, blob: &str) -> Option<Vec<String>> {
    versions::heads(view)
        .into_iter()
        .filter(|g| !g[0].is_left())
        .find(|g| {
            g.iter()
                .any(|v| !v.is_deleted() && v.file == file && v.blob == blob)
        })
        .map(|g| g.iter().map(|v| v.version.clone()).collect())
}

/// The head group `version` belongs to, or the version alone when it is no head.
fn group_of(view: &[Version], version: &Version) -> Vec<Version> {
    versions::heads(view)
        .into_iter()
        .find(|g| g.iter().any(|m| m.version == version.version))
        .map(|g| g.into_iter().cloned().collect())
        .unwrap_or_else(|| vec![version.clone()])
}

fn digest(bytes: &[u8]) -> String {
    hash::sha256_hex(bytes)
}

impl Engine<'_> {
    fn root(&self) -> &Path {
        self.lock.root()
    }

    fn scan(&self) -> Result<versions::Scan, String> {
        let notes = self.root().join("notes");
        versions::scan(&notes).map_err(|e| format!("cannot read {}: {e}", notes.display()))
    }

    fn cur_of(scan: &versions::Scan, id: &str) -> Cur {
        if let Some(found) = scan.notes.get(id) {
            return Cur::File(found.clone());
        }
        if scan.skipped.iter().any(|s| s.id.as_deref() == Some(id)) {
            return Cur::Blocked;
        }
        Cur::Absent
    }

    fn current(&self, id: &str) -> Result<Cur, String> {
        Ok(Self::cur_of(&self.scan()?, id))
    }

    /// Appends a version to the note's log and notes that its summary entry needs a rewrite.
    fn append(&mut self, id: &str, version: &Version) -> Result<(), String> {
        versions::append(self.lock, id, version)?;
        self.touched.insert(id.to_string());
        Ok(())
    }

    fn make(
        &self,
        id: &str,
        parents: &[String],
        file: &str,
        bytes: Option<&[u8]>,
        event: &str,
    ) -> Result<Version, String> {
        let blob = match bytes {
            Some(bytes) => versions::write_blob(self.lock, bytes)?,
            None => versions::DELETED.to_string(),
        };
        Ok(Version {
            version: versions::version_id(id, parents, file, &blob),
            parents: parents.to_vec(),
            file: file.to_string(),
            blob,
            event: event.to_string(),
            at: self.at.clone(),
            ..Version::default()
        })
    }

    /// The head group a save follows, as its members: the one holding the file as the engine last saw it, else the
    /// one the stale-base entry names as written, else the single live group. Never the union of several.
    fn parent_group(&self, view: &[Version], written: Option<&str>) -> Vec<Version> {
        let groups: Vec<Vec<Version>> = versions::heads(view)
            .into_iter()
            .map(|g| g.into_iter().cloned().collect())
            .collect();
        let live: Vec<&Vec<Version>> = groups.iter().filter(|g| !g[0].is_left()).collect();
        let by_state = self.seen.as_ref().and_then(|(file, blob)| {
            live.iter().find(|g| {
                g.iter()
                    .any(|v| !v.is_deleted() && v.file == *file && v.blob == *blob)
            })
        });
        let by_entry = written.and_then(|w| live.iter().find(|g| g.iter().any(|v| v.version == w)));
        by_state
            .or(by_entry)
            .or(live.first())
            .copied()
            .or(groups.first())
            .cloned()
            .unwrap_or_default()
    }

    /// The version the file holds, as the entry's base: the one matching its bytes, else the head a save follows.
    fn held_id(&self, view: &[Version], cur: &Cur) -> Option<String> {
        let matching = match cur {
            Cur::File(f) => {
                let sum = digest(&f.bytes);
                view.iter()
                    .rev()
                    .find(|v| !v.is_deleted() && v.file == f.name && v.blob == sum)
                    .map(|v| v.version.clone())
            }
            _ => None,
        };
        matching.or_else(|| {
            self.parent_group(view, None)
                .first()
                .map(|v| v.version.clone())
        })
    }

    fn drop_entry(&self, id: &str) -> Result<(), String> {
        let mut bases = read_bases(self.root())?;
        if bases.remove(id).is_some() {
            write_bases(self.root(), &bases)?;
        }
        Ok(())
    }

    /// Forgets everything held for the note: its write committed.
    fn clear_written(&self, id: &str) -> Result<(), String> {
        let mut written: BTreeMap<String, Written> = read_map(self.root(), WRITTEN)?;
        if written.remove(id).is_some() {
            write_map(self.root(), WRITTEN, &written)?;
        }
        Ok(())
    }

    /// Updates the note's record, dropping it when it holds nothing.
    fn update_written(&self, id: &str, change: impl FnOnce(&mut Written)) -> Result<(), String> {
        let mut written: BTreeMap<String, Written> = read_map(self.root(), WRITTEN)?;
        let before = written.get(id).cloned();
        let entry = written.entry(id.to_string()).or_default();
        change(entry);
        if entry.file.is_empty() && entry.extras.is_empty() {
            written.remove(id);
        }
        if written.get(id) != before.as_ref() {
            write_map(self.root(), WRITTEN, &written)?;
        }
        Ok(())
    }

    /// The file does not hold the bytes this device wrote: forget them, keep the held saves.
    fn clear_marker(&self, id: &str) -> Result<(), String> {
        self.update_written(id, |w| {
            w.file.clear();
            w.blob.clear();
        })
    }

    /// Makes the held saves as durable as the write they wait for.
    fn hold(&self, id: &str, pending: &Pending) -> Result<(), String> {
        self.update_written(id, |w| {
            w.extras = pending.extras.clone();
            w.flagged = pending.flagged.iter().cloned().collect();
            w.clears = pending.clears;
            w.absent = pending.absent;
        })
    }

    /// The saves a killed call held back for the note.
    fn held(&self, id: &str) -> Result<Pending, String> {
        let written: BTreeMap<String, Written> = read_map(self.root(), WRITTEN)?;
        Ok(written.get(id).map_or_else(Pending::default, |w| Pending {
            extras: w.extras.clone(),
            flagged: w.flagged.iter().cloned().collect(),
            clears: w.clears,
            absent: w.absent,
        }))
    }

    /// Records the file as it is against the head group it follows, as the scan does, with the text it dropped
    /// from an open conflict.
    fn plain_save(
        &mut self,
        id: &str,
        file: &str,
        bytes: Option<&[u8]>,
        view: &[Version],
        written: Option<&str>,
    ) -> Result<Option<Version>, String> {
        if let Some(bytes) = bytes
            && held_group(view, file, &digest(bytes)).is_some()
        {
            // The file is a head group's text, whichever group that is: nothing changed.
            self.seen = Some((file.to_string(), digest(bytes)));
            return Ok(None);
        }
        let group = self.parent_group(view, written);
        let parents: Vec<String> = group.iter().map(|v| v.version.clone()).collect();
        let rep = group.first();
        let sum = bytes.map(digest);
        let seen = sum.as_deref().map(|sum| (file, sum));
        let Some(event) = versions::event_for(rep, seen) else {
            return Ok(None);
        };
        let name = match bytes {
            Some(_) => file,
            None => rep.map_or("", |v| v.file.as_str()),
        };
        let mut version = self.make(id, &parents, name, bytes, event)?;
        if let (Some(rep), Some(bytes)) = (rep, bytes) {
            version.dropped = dropped_since(self.root(), view, rep, bytes)?;
        }
        self.append(id, &version)?;
        self.seen = sum.map(|sum| (file.to_string(), sum));
        Ok(Some(version))
    }

    /// A save of `bytes` that follows `parent`, not yet appended.
    fn build_save(
        &self,
        id: &str,
        view: &[Version],
        parent: &Version,
        file: &str,
        bytes: &[u8],
        sum: &str,
    ) -> Result<Version, String> {
        let event =
            versions::event_for(Some(parent), Some((file, sum))).unwrap_or(versions::EDITED);
        // The save follows every member of the head group the version holds.
        let parents: Vec<String> = group_of(view, parent)
            .iter()
            .map(|v| v.version.clone())
            .collect();
        let mut version = self.make(id, &parents, file, Some(bytes), event)?;
        version.dropped = dropped_since(self.root(), view, parent, bytes)?;
        Ok(version)
    }

    /// A local change to the note's file: `bytes` is its text, none when the file is gone. `extras` are the saves
    /// an earlier round of this call holds back. A save that needs a write comes back as pending versions.
    fn local(
        &mut self,
        id: &str,
        file: &str,
        bytes: Option<&[u8]>,
        extras: &[Version],
    ) -> Result<Pending, String> {
        let mut view = effective(versions::load(self.root(), id)?.versions);
        view.extend(extras.iter().cloned());
        let entry = read_bases(self.root())?.remove(id);
        let find = |version: &str| view.iter().find(|v| v.version == version).cloned();
        let pair = entry
            .as_ref()
            .and_then(|b| Some((find(&b.base)?, find(&b.written)?)));
        if entry.is_some() && pair.is_none() {
            self.drop_entry(id)?;
        }
        let Some((base, written)) = pair else {
            self.plain_save(id, file, bytes, &view, None)?;
            return Ok(Pending::default());
        };
        let Some(bytes) = bytes else {
            return self.local_delete(id, &base, &written, &view);
        };
        let sum = digest(bytes);
        if written.file == file && written.blob == sum {
            self.seen = Some((file.to_string(), sum));
            return Ok(Pending::default());
        }
        let held = |v: &Version| versions::content(self.root(), v).ok();
        let (true, Some(theirs)) = (!written.is_deleted(), held(&written)) else {
            self.plain_save(id, file, Some(bytes), &view, Some(&written.version))?;
            self.drop_entry(id)?;
            return Ok(Pending::default());
        };
        let parent = self
            .seen
            .as_ref()
            .and_then(|(f, b)| extras.iter().find(|v| v.file == *f && v.blob == *b))
            .cloned()
            .unwrap_or_else(|| base.clone());
        let provisional = Version {
            version: versions::version_id(id, std::slice::from_ref(&parent.version), file, &sum),
            file: file.to_string(),
            blob: sum.clone(),
            ..Version::default()
        };
        let base_bytes = held(&base).unwrap_or_default();
        let merged = merge::merge(
            &[&base_bytes],
            Some(&base.file),
            &Side {
                version: &provisional,
                bytes,
            },
            &Side {
                version: &written,
                bytes: &theirs,
            },
            self.p.syncs,
        );
        let resolving = !written.conflict.is_empty();
        if resolving || (merged.bytes == bytes && merged.file == file) {
            let save = self.build_save(id, &view, &written, file, bytes, &sum)?;
            self.append(id, &save)?;
            self.drop_entry(id)?;
            self.seen = Some((file.to_string(), sum));
            return Ok(Pending::default());
        }
        let save = self.build_save(id, &view, &parent, file, bytes, &sum)?;
        self.seen = Some((file.to_string(), sum));
        Ok(Pending {
            flagged: HashSet::from([save.version.clone()]),
            extras: vec![save],
            clears: true,
            absent: false,
        })
    }

    /// A delete counts as a save: against a sync write that edited the note, the edit wins, once.
    fn local_delete(
        &mut self,
        id: &str,
        base: &Version,
        written: &Version,
        view: &[Version],
    ) -> Result<Pending, String> {
        let edited = !written.is_deleted() && written.blob != base.blob;
        if !edited {
            self.plain_save(id, "", None, view, Some(&written.version))?;
            self.drop_entry(id)?;
            return Ok(Pending::default());
        }
        let parents: Vec<String> = group_of(view, base)
            .iter()
            .map(|v| v.version.clone())
            .collect();
        let deleted = self.make(id, &parents, &base.file, None, versions::DELETED)?;
        Ok(Pending {
            extras: vec![deleted],
            clears: true,
            absent: true,
            ..Pending::default()
        })
    }

    /// The staged versions of `id` that can be applied: every parent known or declared outside, or waiting longer
    /// than `stale_days`.
    fn applicable(&self, log: &[Version], staged: &[(Version, jiff::Timestamp)]) -> Vec<Version> {
        let mut known: HashSet<String> = log.iter().map(|v| v.version.clone()).collect();
        let limit = i64::from(self.p.stale_days) * SECONDS_PER_DAY;
        let mut pending: Vec<&(Version, jiff::Timestamp)> = staged.iter().collect();
        let mut ready = Vec::new();
        loop {
            let before = pending.len();
            pending.retain(|(v, seen)| {
                let known_parents = v
                    .parents
                    .iter()
                    .all(|p| known.contains(p) || v.outside.contains(p));
                let stale = self.p.now.as_second() - seen.as_second() >= limit;
                if known_parents || stale {
                    known.insert(v.version.clone());
                    ready.push(v.clone());
                    false
                } else {
                    true
                }
            });
            if pending.len() == before {
                return ready;
            }
        }
    }

    fn head(&self, group: Vec<Version>) -> Result<Option<Head>, String> {
        let bytes = if group[0].is_deleted() {
            None
        } else {
            match versions::content(self.root(), &group[0]) {
                Ok(bytes) => Some(bytes),
                Err(ContentError::Pruned) => return Ok(None),
                Err(ContentError::Deleted) => None,
                Err(ContentError::Io(e)) => return Err(e),
            }
        };
        Ok(Some(Head {
            members: group,
            bytes,
        }))
    }

    /// Merges two heads into one `merged` version that follows every member of both.
    fn join(
        &self,
        id: &str,
        a: Head,
        b: Head,
        work: &mut Vec<Version>,
        plan: &mut Plan,
        flagged: &mut HashSet<String>,
    ) -> Result<Head, String> {
        let mut parents: Vec<String> = a
            .members
            .iter()
            .chain(&b.members)
            .map(|v| v.version.clone())
            .collect();
        parents.sort();
        parents.dedup();
        let mut flags: BTreeSet<String> = BTreeSet::new();
        if a.members
            .iter()
            .chain(&b.members)
            .any(|v| flagged.contains(&v.version))
        {
            flags.insert("stale-base".into());
        }
        let (file, bytes, conflict) = match (&a.bytes, &b.bytes) {
            (Some(left), Some(right)) => {
                let lowest = versions::lowest_common(work, &a.rep().version, &b.rep().version);
                let mut bases: Vec<Vec<u8>> = Vec::new();
                for v in &lowest {
                    let held = match plan.bytes.get(&v.version) {
                        Some(held) => Some(held.clone()),
                        None if v.is_deleted() => Some(Vec::new()),
                        None => versions::content(self.root(), v).ok(),
                    };
                    bases.extend(held);
                }
                let base_file = lowest
                    .first()
                    .map(|v| v.file.as_str())
                    .filter(|file| lowest.iter().all(|v| v.file == *file));
                let refs: Vec<&[u8]> = bases.iter().map(Vec::as_slice).collect();
                let merged = merge::merge(
                    &refs,
                    base_file,
                    &Side {
                        version: a.rep(),
                        bytes: left,
                    },
                    &Side {
                        version: b.rep(),
                        bytes: right,
                    },
                    self.p.syncs,
                );
                flags.extend(merged.flags);
                (merged.file, Some(merged.bytes), merged.conflict)
            }
            (Some(live), None) | (None, Some(live)) => {
                let kept = if a.bytes.is_some() { &a } else { &b };
                flags.insert("edit-beat-delete".into());
                let carried = merge::blocks(&String::from_utf8_lossy(live))
                    .into_iter()
                    .map(|b| Conflict {
                        passage: b.passage,
                        sides: b.sides.into_iter().map(|s| s.version).collect(),
                    })
                    .collect();
                (kept.rep().file.clone(), Some(live.clone()), carried)
            }
            (None, None) => (a.rep().file.clone(), None, Vec::new()),
        };
        let blob = bytes
            .as_deref()
            .map_or_else(|| versions::DELETED.to_string(), digest);
        let merged = Version {
            version: versions::version_id(id, &parents, &file, &blob),
            parents,
            file,
            blob,
            event: versions::MERGED.into(),
            at: self.at.clone(),
            flags: flags.into_iter().collect(),
            conflict,
            ..Version::default()
        };
        if let Some(bytes) = &bytes {
            plan.bytes.insert(merged.version.clone(), bytes.clone());
        }
        // The joined head stands for every save it holds, so the next join flags its merge too.
        if merged.flags.iter().any(|f| f == "stale-base") {
            flagged.insert(merged.version.clone());
        }
        work.push(merged.clone());
        plan.merges.push(merged.clone());
        Ok(Head {
            members: vec![merged],
            bytes,
        })
    }

    /// The versions to leave out of the fold: a head whose text is not held and that has waited `sync.stale_days`
    /// since it arrived (`aged`) drops out as if its children listed it in `outside`, and so on with what that
    /// uncovers. A head whose text is not held and that is younger holds the note back.
    fn unheld(
        &self,
        work: &[Version],
        aged: &HashMap<String, jiff::Timestamp>,
    ) -> Result<HashSet<String>, String> {
        let limit = i64::from(self.p.stale_days) * SECONDS_PER_DAY;
        let mut out: HashSet<String> = HashSet::new();
        loop {
            let rest: Vec<Version> = work
                .iter()
                .filter(|v| !out.contains(&v.version))
                .cloned()
                .collect();
            let mut grew = false;
            for group in versions::heads(&rest) {
                if group[0].is_deleted() {
                    continue;
                }
                match versions::content(self.root(), group[0]) {
                    Err(ContentError::Pruned) => {}
                    Err(ContentError::Io(e)) => return Err(e),
                    _ => continue,
                }
                let stale = aged
                    .get(&group[0].version)
                    .is_some_and(|at| self.p.now.as_second() - at.as_second() >= limit);
                if stale {
                    out.extend(group.iter().map(|v| v.version.clone()));
                    grew = true;
                }
            }
            if !grew {
                return Ok(out);
            }
        }
    }

    /// Plans a note's heads: merged two at a time in id order, or removed when every head is a `left` (a `left` carries
    /// no file name, so they are one head group). `None` when a head's text is not held. The versions in
    /// `excluded` are no heads but stay in `work`, so a merge still finds them as ancestors.
    fn plan(
        &self,
        id: &str,
        work: &mut Vec<Version>,
        flagged: &HashSet<String>,
        excluded: &HashSet<String>,
    ) -> Result<Option<Plan>, String> {
        let mut flagged = flagged.clone();
        let kept: Vec<Version> = work
            .iter()
            .filter(|v| !excluded.contains(&v.version))
            .cloned()
            .collect();
        let groups: Vec<Vec<Version>> = versions::heads(&kept)
            .into_iter()
            .map(|g| g.into_iter().cloned().collect())
            .collect();
        let (left, live): (Vec<_>, Vec<_>) = groups.into_iter().partition(|g| g[0].is_left());
        let mut plan = Plan {
            merges: Vec::new(),
            bytes: HashMap::new(),
            file: String::new(),
            content: None,
            final_id: String::new(),
            left: live.is_empty(),
            conflicts: 0,
        };
        let mut groups = if live.is_empty() { left } else { live }.into_iter();
        let Some(first) = groups.next() else {
            return Ok(None);
        };
        let Some(mut acc) = self.head(first)? else {
            return Ok(None);
        };
        for group in groups {
            let Some(next) = self.head(group)? else {
                return Ok(None);
            };
            acc = self.join(id, acc, next, work, &mut plan, &mut flagged)?;
        }
        plan.file = acc.rep().file.clone();
        plan.final_id = acc.rep().version.clone();
        // The conflicts of the final version, whether this call merged it or another device did.
        plan.conflicts = plan
            .merges
            .iter()
            .chain(work.iter())
            .find(|m| m.version == plan.final_id)
            .map_or(0, |m| m.conflict.len());
        plan.content = acc.bytes;
        Ok(Some(plan))
    }

    /// The staged versions of `id` that wait for the log, with the scope each arrived through, and the last of those
    /// scopes. A version whose text does not say that scope is skipped with a line and counts as done; one whose text
    /// is not held waits. A `left` whose id a full version holds, in the log or waiting, is ignored: the note moved to
    /// a scope this device syncs too, and the full version comes through that log.
    fn waiting(
        &mut self,
        id: &str,
        in_log: &HashSet<String>,
        left_only: &HashSet<String>,
        done: &mut HashSet<String>,
    ) -> Result<Waiting, String> {
        let mut waiting: Vec<(Version, jiff::Timestamp)> = Vec::new();
        let mut scopes = HashMap::new();
        let mut scope = String::new();
        for entry in read_inbox(self.root())? {
            let Some(record) = entry.record.filter(|r| r.note == id) else {
                continue;
            };
            let v = record.version;
            if done.contains(&v.version) {
                continue;
            }
            if in_log.contains(&v.version) && (v.is_left() || !left_only.contains(&v.version)) {
                done.insert(v.version);
                continue;
            }
            if !v.is_deleted() {
                match versions::content(self.root(), &v) {
                    Ok(bytes) => {
                        if !std::str::from_utf8(&bytes).is_ok_and(|t| says_scope(t, &entry.scope)) {
                            self.events.push(format!(
                                "sync {}: skipped version {} of {id}: its text does not say scope {}",
                                entry.scope,
                                v.short(),
                                entry.scope
                            ));
                            done.insert(v.version);
                            continue;
                        }
                    }
                    Err(ContentError::Pruned | ContentError::Deleted) => {}
                    Err(ContentError::Io(e)) => return Err(e),
                }
            }
            let seen = entry.seen.parse().unwrap_or(self.p.now);
            scopes.insert(v.version.clone(), entry.scope.clone());
            scope = entry.scope;
            waiting.push((v, seen));
        }
        let whole: HashSet<String> = waiting
            .iter()
            .filter(|(v, _)| !v.is_left())
            .map(|(v, _)| v.version.clone())
            .collect();
        waiting.retain(|(v, _)| !(v.is_left() && whole.contains(&v.version)));
        Ok(Waiting {
            versions: waiting,
            scopes,
            scope,
        })
    }

    /// Ends a note's settle without a commit. The record of what the file may hold and the held saves stay, since the
    /// file may still hold the bytes; only an entry this call set up goes.
    fn leave(&self, id: &str, created: bool) -> Result<(), String> {
        if created {
            self.drop_entry(id)?;
        }
        Ok(())
    }

    /// Writes one note's final text and settles its log, repeating while a save lands during the swap. Saves that
    /// need the write (`pending`) and the staged versions reach the log only after it succeeded.
    fn settle(
        &mut self,
        id: &str,
        mut pending: Pending,
        done: &mut HashSet<String>,
    ) -> Result<(), String> {
        let mut created = false;
        if !pending.extras.is_empty() {
            self.hold(id, &pending)?;
        }
        for round in 0..=MAX_ROUNDS {
            (self.hook)(Step::Prepare)?;
            let log = versions::load(self.root(), id)?.versions;
            let in_log: HashSet<String> = log.iter().map(|v| v.version.clone()).collect();
            let whole: HashSet<&str> = log
                .iter()
                .filter(|v| !v.is_left())
                .map(|v| v.version.as_str())
                .collect();
            let left_only: HashSet<String> = log
                .iter()
                .filter(|v| v.is_left() && !whole.contains(v.version.as_str()))
                .map(|v| v.version.clone())
                .collect();
            pending.extras.retain(|v| !in_log.contains(&v.version));
            let waiting = self.waiting(id, &in_log, &left_only, done)?;
            let applicable = self.applicable(&log, &waiting.versions);
            let mut view = effective(log);
            view.extend(pending.extras.iter().cloned());
            let mut work = view.clone();
            work.extend(applicable.iter().cloned());
            work = effective(work);
            let mut aged: HashMap<String, jiff::Timestamp> = work
                .iter()
                .filter_map(|v| Some((v.version.clone(), v.at.parse().ok()?)))
                .collect();
            aged.extend(
                waiting
                    .versions
                    .iter()
                    .map(|(v, seen)| (v.version.clone(), *seen)),
            );
            let excluded = self.unheld(&work, &aged)?;
            let Some(mut plan) = self.plan(id, &mut work, &pending.flagged, &excluded)? else {
                return self.leave(id, created);
            };
            let scan = self.scan()?;
            let cur = Self::cur_of(&scan, id);
            if matches!(cur, Cur::Blocked) {
                return self.leave(id, created);
            }
            let leftover = versions::restore_path(self.root(), id);
            if leftover.exists()
                && !self.clear_leftover(
                    id,
                    &leftover,
                    &view,
                    &applicable,
                    &pending.extras,
                    &plan,
                )?
            {
                return self.leave(id, created);
            }
            if !self.settle_topic(id, &mut plan, &cur, &scan)? {
                return self.leave(id, created);
            }
            let blob = plan.content.as_deref().map(digest);
            let mut r = Round {
                plan,
                applicable,
                in_log,
                cur,
                scope: waiting.scope,
                scopes: waiting.scopes,
            };
            let already = match (&r.cur, &blob) {
                (Cur::File(f), Some(b)) => f.name == r.plan.file && digest(&f.bytes) == *b,
                (Cur::Absent, None) => true,
                _ => false,
            };
            if already {
                return self.commit(id, &r, &view, &pending, done);
            }
            if let (Cur::File(f), Some(b)) = (&r.cur, &blob)
                && f.name != r.plan.file
                && digest(&f.bytes) == *b
                && read_map::<Written>(self.root(), WRITTEN)?
                    .get(id)
                    .is_some_and(|w| w.file == r.plan.file && w.blob == *b)
            {
                // A kill between the exchange and the rename: finish this device's own write.
                let notes = self.root().join("notes");
                let target = notes.join(&r.plan.file);
                if target.exists() {
                    self.events.push(format!(
                        "notes/{}: not written: {} is taken by another note",
                        r.plan.file, r.plan.file
                    ));
                    return self.leave(id, created);
                }
                swap::rename_new(&notes.join(&f.name), &target)?;
                r.cur = Cur::File(versions::Found {
                    name: r.plan.file.clone(),
                    id: id.to_string(),
                    bytes: f.bytes.clone(),
                });
                return self.commit(id, &r, &view, &pending, done);
            }
            let heads = versions::heads(&view);
            let written = read_map::<Written>(self.root(), WRITTEN)?.remove(id);
            let known = match &r.cur {
                Cur::File(f) => {
                    let sum = digest(&f.bytes);
                    let known = heads
                        .iter()
                        .flatten()
                        .copied()
                        .chain(r.applicable.iter())
                        .any(|v| !v.is_deleted() && v.file == f.name && v.blob == sum)
                        || written.is_some_and(|w| w.file == f.name && w.blob == sum);
                    if known {
                        self.seen = Some((f.name.clone(), sum));
                    }
                    known
                }
                Cur::Absent => {
                    pending.absent
                        || !heads
                            .iter()
                            .filter(|g| !g[0].is_left())
                            .any(|g| !g[0].is_deleted())
                }
                Cur::Blocked => false,
            };
            if !known {
                let (file, bytes) = match &r.cur {
                    Cur::File(f) => (f.name.clone(), Some(f.bytes.clone())),
                    _ => (
                        self.parent_group(&view, None)
                            .first()
                            .map(|v| v.file.clone())
                            .unwrap_or_default(),
                        None,
                    ),
                };
                let out = self.local(id, &file, bytes.as_deref(), &pending.extras)?;
                pending.extend(out);
                self.hold(id, &pending)?;
                continue;
            }
            if round == MAX_ROUNDS {
                self.events.push(format!(
                    "notes/{}: not written: the file kept changing",
                    r.plan.file
                ));
                return self.leave(id, created);
            }
            let mut bases = read_bases(self.root())?;
            if !bases.contains_key(id)
                && let Some(base) = self.held_id(&view, &r.cur)
            {
                bases.insert(
                    id.to_string(),
                    Base {
                        base,
                        written: r.plan.final_id.clone(),
                    },
                );
                write_bases(self.root(), &bases)?;
                created = true;
            }
            match self.write(id, &r.plan, &r.cur, &r.scope)? {
                Wrote::Done => {
                    self.seen = blob.map(|b| (r.plan.file.clone(), b));
                    return self.commit(id, &r, &view, &pending, done);
                }
                Wrote::Raced | Wrote::Retry => {
                    self.leave(id, created)?;
                    created = false;
                }
                Wrote::Landed(file, older) => {
                    let out = self.local(id, &file, Some(&older), &pending.extras)?;
                    pending.extend(out);
                    self.hold(id, &pending)?;
                    let hidden = versions::restore_path(self.root(), id);
                    fs::remove_file(&hidden).map_err(|e| io_message("remove", &hidden, &e))?;
                    self.leave(id, created)?;
                    created = false;
                }
                Wrote::Skipped => return self.leave(id, created),
            }
        }
        let file = match self.current(id)? {
            Cur::File(f) => f.name,
            _ => id.to_string(),
        };
        self.events
            .push(format!("notes/{file}: not written: the file kept changing"));
        self.leave(id, created)
    }

    /// The name for a note that lost a topic collision: `<kind>-<topic>-<the last characters of its id, lowercased>.md`,
    /// with as many characters as it takes for no other note to hold that topic or file. None when even the whole id
    /// does not make one.
    fn suffixed(&self, file: &str, id: &str, scan: &versions::Scan) -> Option<String> {
        let stem = file.strip_suffix(".md")?;
        let lower = id.to_ascii_lowercase();
        let notes = self.root().join("notes");
        let topics: Vec<String> = scan
            .notes
            .values()
            .filter(|f| f.id != id)
            .map(|f| f.name.as_str())
            .chain(scan.skipped.iter().map(|s| s.name.as_str()))
            .filter_map(|name| note::parse_name(name).ok().map(|n| n.topic))
            .collect();
        (4..=lower.len()).find_map(|k| {
            let name = format!("{stem}-{}.md", lower.get(lower.len() - k..)?);
            let topic = note::parse_name(&name).ok()?.topic;
            (!topics.contains(&topic) && !notes.join(&name).exists()).then_some(name)
        })
    }

    /// Settles a topic collision the write of `plan` would cause: another note's file already holds the topic of the
    /// name the note is about to take. The note whose id sorts later is renamed, with a `renamed` version flagged
    /// `topic-taken`: the incoming one is written under the suffixed name, a local one is renamed here first, so every
    /// device records the same version. False, with a line, when nothing can be written yet.
    fn settle_topic(
        &mut self,
        id: &str,
        plan: &mut Plan,
        cur: &Cur,
        scan: &versions::Scan,
    ) -> Result<bool, String> {
        let Some(content) = plan.content.clone() else {
            return Ok(true);
        };
        if plan.left || matches!(cur, Cur::File(f) if f.name == plan.file) {
            return Ok(true);
        }
        let Ok(parsed) = note::parse_name(&plan.file) else {
            return Ok(true);
        };
        let holders: Vec<&versions::Found> = scan
            .notes
            .values()
            .filter(|f| f.id != id)
            .filter(|f| note::parse_name(&f.name).is_ok_and(|n| n.topic == parsed.topic))
            .collect();
        let blocked = |engine: &mut Self, file: &str| {
            engine.events.push(format!(
                "notes/{file}: not written: its topic is taken by another note"
            ));
            Ok(false)
        };
        if holders.iter().any(|h| h.id.as_str() < id) {
            let Some(name) = self.suffixed(&plan.file, id, scan) else {
                return blocked(self, &plan.file.clone());
            };
            let mut renamed = self.make(
                id,
                std::slice::from_ref(&plan.final_id),
                &name,
                Some(&content),
                versions::RENAMED,
            )?;
            renamed.flags = vec![TOPIC_TAKEN.to_string()];
            plan.final_id = renamed.version.clone();
            plan.file = name;
            plan.merges.push(renamed);
            return Ok(true);
        }
        for holder in holders {
            if !self.rename_holder(holder, scan)? {
                return blocked(self, &plan.file.clone());
            }
        }
        Ok(true)
    }

    /// Renames the younger note that holds a topic here to its suffixed name and records it, after the save its file
    /// may hold that no version has yet. False when the note cannot be renamed now.
    fn rename_holder(
        &mut self,
        holder: &versions::Found,
        scan: &versions::Scan,
    ) -> Result<bool, String> {
        let Some(name) = self.suffixed(&holder.name, &holder.id, scan) else {
            return Ok(false);
        };
        let sum = digest(&holder.bytes);
        let seen = self.seen.take();
        let held = |engine: &Self| -> Result<Option<Vec<Version>>, String> {
            let view = effective(versions::load(engine.root(), &holder.id)?.versions);
            let group = engine.parent_group(&view, None);
            let matches = group
                .first()
                .is_some_and(|v| !v.is_deleted() && v.file == holder.name && v.blob == sum);
            Ok(matches.then_some(group))
        };
        let mut group = held(self)?;
        if group.is_none() {
            let pending = self.local(&holder.id, &holder.name, Some(&holder.bytes), &[])?;
            if pending.extras.is_empty() {
                self.seen = None;
                group = held(self)?;
            }
        }
        self.seen = seen;
        let Some(group) = group else {
            return Ok(false);
        };
        let parents: Vec<String> = group.iter().map(|v| v.version.clone()).collect();
        let mut renamed = self.make(
            &holder.id,
            &parents,
            &name,
            Some(&holder.bytes),
            versions::RENAMED,
        )?;
        renamed.flags = vec![TOPIC_TAKEN.to_string()];
        (self.hook)(Step::Rename)?;
        let notes = self.root().join("notes");
        swap::rename_new(&notes.join(&holder.name), &notes.join(&name))?;
        self.append(&holder.id, &renamed)?;
        keep_base(self.lock, &holder.id, &renamed.version)?;
        Ok(true)
    }

    /// Removes the hidden file an interrupted write left when its bytes are a version of the note, staged, planned
    /// or a local save held back. False when it holds anything else: another writer owns it.
    fn clear_leftover(
        &mut self,
        id: &str,
        path: &Path,
        view: &[Version],
        applicable: &[Version],
        extras: &[Version],
        plan: &Plan,
    ) -> Result<bool, String> {
        let sum = digest(&fs::read(path).map_err(|e| io_message("read", path, &e))?);
        let written = read_map::<Written>(self.root(), WRITTEN)?;
        let ours = written.get(id).is_some_and(|w| w.blob == sum)
            || view
                .iter()
                .chain(applicable)
                .chain(extras)
                .chain(&plan.merges)
                .any(|v| v.blob == sum);
        if ours {
            fs::remove_file(path).map_err(|e| io_message("remove", path, &e))?;
        }
        Ok(ours)
    }

    fn refuse(&mut self, error: String, scope: &str, file: &str) -> Wrote {
        let event = if error == swap::UNSUPPORTED {
            format!("sync {scope}: cannot write notes on this filesystem: {error}")
        } else {
            format!("notes/{file}: cannot write: {error}")
        };
        self.events.push(event);
        Wrote::Skipped
    }

    fn write(&mut self, id: &str, plan: &Plan, cur: &Cur, scope: &str) -> Result<Wrote, String> {
        let notes = self.root().join("notes");
        let temp = versions::restore_path(self.root(), id);
        let target = notes.join(&plan.file);
        let taken = |engine: &mut Engine| {
            engine.events.push(format!(
                "notes/{}: not written: {} is taken by another note",
                plan.file, plan.file
            ));
            Ok(Wrote::Skipped)
        };
        match (cur, &plan.content) {
            (Cur::Absent, Some(bytes)) => {
                if target.exists() {
                    return taken(self);
                }
                (self.hook)(Step::Write)?;
                versions::write_temp(&temp, bytes, None)?;
                (self.hook)(Step::Swap)?;
                if let Err(e) = swap::rename_new(&temp, &target) {
                    let _ = fs::remove_file(&temp);
                    return Ok(if target.exists() {
                        Wrote::Retry
                    } else {
                        self.refuse(e, scope, &plan.file)
                    });
                }
                Ok(Wrote::Done)
            }
            (Cur::File(found), Some(bytes)) => {
                let old = notes.join(&found.name);
                if found.name != plan.file && target.exists() {
                    return taken(self);
                }
                (self.hook)(Step::Write)?;
                versions::write_temp(&temp, bytes, Some(&found.name))?;
                self.update_written(id, |w| {
                    w.file = plan.file.clone();
                    w.blob = digest(bytes);
                })?;
                (self.hook)(Step::Swap)?;
                if let Err(e) = (self.exchange)(&old, &temp) {
                    let _ = fs::remove_file(&temp);
                    self.clear_marker(id)?;
                    return Ok(self.refuse(e, scope, &found.name));
                }
                (self.hook)(Step::Inspect)?;
                let out = fs::read(&temp).map_err(|e| io_message("read", &temp, &e))?;
                if out != found.bytes {
                    // A save landed during the swap: put the file back as the agent left it.
                    (self.exchange)(&old, &temp)
                        .map_err(|e| format!("cannot put back notes/{}: {e}", found.name))?;
                    self.clear_marker(id)?;
                    let back = fs::read(&temp).map_err(|e| io_message("read", &temp, &e))?;
                    if back == *bytes {
                        let _ = fs::remove_file(&temp);
                        return Ok(Wrote::Raced);
                    }
                    // A second save landed in the file this call wrote, before the put-back: it is the newer one.
                    // Put it back in the file; the hidden file keeps the older one until it is recorded.
                    (self.exchange)(&old, &temp)
                        .map_err(|e| format!("cannot put back notes/{}: {e}", found.name))?;
                    return Ok(Wrote::Landed(found.name.clone(), out));
                }
                if found.name != plan.file {
                    (self.hook)(Step::Rename)?;
                    swap::rename_new(&old, &target).map_err(|e| {
                        format!(
                            "cannot rename notes/{} to notes/{}: {e}",
                            found.name, plan.file
                        )
                    })?;
                }
                (self.hook)(Step::Remove)?;
                fs::remove_file(&temp).map_err(|e| io_message("remove", &temp, &e))?;
                Ok(Wrote::Done)
            }
            (Cur::File(found), None) => {
                let old = notes.join(&found.name);
                (self.hook)(Step::Swap)?;
                if let Err(e) = swap::rename_new(&old, &temp) {
                    return Ok(self.refuse(e, scope, &found.name));
                }
                (self.hook)(Step::Inspect)?;
                let out = fs::read(&temp).map_err(|e| io_message("read", &temp, &e))?;
                if out == found.bytes {
                    (self.hook)(Step::Remove)?;
                    fs::remove_file(&temp).map_err(|e| io_message("remove", &temp, &e))?;
                    return Ok(Wrote::Done);
                }
                swap::rename_new(&temp, &old)?;
                Ok(Wrote::Raced)
            }
            (Cur::Absent, None) => Ok(Wrote::Done),
            (Cur::Blocked, _) => Ok(Wrote::Skipped),
        }
    }

    /// Appends the staged versions, the held back saves and the merges to the log, and keeps the stale-base entry.
    fn commit(
        &mut self,
        id: &str,
        r: &Round,
        view: &[Version],
        pending: &Pending,
        done: &mut HashSet<String>,
    ) -> Result<(), String> {
        (self.hook)(Step::Record)?;
        for v in &r.applicable {
            self.append(id, v)?;
            done.insert(v.version.clone());
        }
        for v in pending
            .extras
            .iter()
            .filter(|v| !r.in_log.contains(&v.version))
        {
            self.append(id, v)?;
        }
        for merged in &r.plan.merges {
            if let Some(bytes) = r.plan.bytes.get(&merged.version) {
                versions::write_blob(self.lock, bytes)?;
            }
            self.append(id, merged)?;
        }
        self.clear_written(id)?;
        if r.applicable.is_empty() && r.plan.merges.is_empty() && pending.extras.is_empty() {
            return Ok(());
        }
        let mut bases = read_bases(self.root())?;
        if pending.clears {
            bases.remove(id);
        }
        match bases.get_mut(id) {
            Some(entry) => entry.written = r.plan.final_id.clone(),
            // A file that came back over a local delete is text the agent can see: no entry, so its next delete sticks.
            None if !pending.absent => {
                if let Some(base) = self.held_id(view, &r.cur) {
                    bases.insert(
                        id.to_string(),
                        Base {
                            base,
                            written: r.plan.final_id.clone(),
                        },
                    );
                }
            }
            None => {}
        }
        write_bases(self.root(), &bases)?;
        let plan = &r.plan;
        if plan.conflicts > 0 {
            self.events.push(format!(
                "notes/{}: conflict in {} passage{}; run bilbo check",
                plan.file,
                plan.conflicts,
                if plan.conflicts == 1 { "" } else { "s" }
            ));
        }
        if plan.left
            && let Cur::File(f) = &r.cur
        {
            let scope = r.scopes.get(&plan.final_id).unwrap_or(&r.scope);
            self.events.push(format!(
                "sync {scope}: notes/{} left the scope; its history stays",
                f.name
            ));
        }
        Ok(())
    }

    /// Rewrites the summary entries of the notes this call recorded a version for.
    fn flush(&mut self) -> Result<(), String> {
        let touched = std::mem::take(&mut self.touched);
        refresh(self.lock, self.p, &touched)
    }

    /// Appends each declaration whose conflict version some log holds. One whose version has not arrived within
    /// `stale_days` is dropped, with a line; the rest wait.
    fn declare(&mut self, entries: &[Staged], done: &mut HashSet<String>) -> Result<(), String> {
        let wanted: Vec<&Staged> = entries.iter().filter(|e| e.declaration.is_some()).collect();
        if wanted.is_empty() {
            return Ok(());
        }
        for note in versions::note_ids(self.root())? {
            let log = versions::load(self.root(), &note)?;
            for entry in &wanted {
                let Some(declaration) = &entry.declaration else {
                    continue;
                };
                let known = log
                    .versions
                    .iter()
                    .any(|v| v.version == declaration.declare);
                if known && !log.declarations.contains(declaration) {
                    versions::append_declaration(self.lock, &note, declaration)?;
                    self.touched.insert(note.clone());
                }
                if known {
                    done.insert(declaration_key(declaration));
                }
            }
        }
        let limit = i64::from(self.p.stale_days) * SECONDS_PER_DAY;
        for entry in wanted {
            let Some(declaration) = &entry.declaration else {
                continue;
            };
            let key = declaration_key(declaration);
            let seen: jiff::Timestamp = entry.seen.parse().unwrap_or(self.p.now);
            if !done.contains(&key) && self.p.now.as_second() - seen.as_second() >= limit {
                done.insert(key);
                self.events.push(format!(
                    "sync {}: dropped a declaration of conflict {}: that version never arrived",
                    entry.scope,
                    declaration
                        .declare
                        .get(..12)
                        .unwrap_or(&declaration.declare)
                ));
            }
        }
        Ok(())
    }
}

/// The scopes this device moved a note out of in the last 30 days: for each version it recorded itself, the scope of a
/// parent that this device syncs and the version's own text no longer names. A `left` record carries no scope name,
/// so the parents' text is the only place to read it.
fn left_scopes(root: &Path, p: &Params, log: &[Version]) -> Vec<Left> {
    let window = jiff::SignedDuration::from_hours(24 * LEFT_DAYS);
    let by_id: HashMap<&str, &Version> = log.iter().map(|v| (v.version.as_str(), v)).collect();
    let scope_at = |v: &Version| {
        versions::content(root, v)
            .ok()
            .and_then(|bytes| scope_of(&bytes))
    };
    let mut lefts: Vec<Left> = Vec::new();
    for v in log.iter().filter(|v| v.device.is_none() && !v.is_deleted()) {
        let recent =
            v.at.parse::<jiff::Timestamp>()
                .is_ok_and(|at| p.now.duration_since(at) < window);
        if !recent {
            continue;
        }
        let own = scope_at(v);
        for parent in v.parents.iter().filter_map(|id| by_id.get(id.as_str())) {
            let Some(was) = scope_at(parent).filter(|name| (p.syncs)(name)) else {
                continue;
            };
            let left = Left {
                scope: was,
                at: v.at.clone(),
            };
            if own.as_deref() != Some(left.scope.as_str()) && !lefts.contains(&left) {
                lefts.push(left);
            }
        }
    }
    lefts
}

/// Rewrites the open-conflict summary's entry of each note in `notes` from its log, as the Open-conflict summary
/// says: the entry of `note::conflicts::summarize`, with the scopes it left, and none when it says nothing. The
/// caller holds the history lock.
fn refresh(lock: &Lock, p: &Params, notes: &BTreeSet<String>) -> Result<(), String> {
    if notes.is_empty() {
        return Ok(());
    }
    let root = lock.root();
    let mut summary = conflicts::read(root).unwrap_or_default();
    let before = summary.clone();
    for id in notes {
        let mut log = versions::load(root, id)?;
        log.versions = effective(log.versions);
        // The file the log's live head names; a note with only `left` or deleted heads keeps its last name.
        let file = versions::heads(&log.versions)
            .into_iter()
            .find(|g| !g[0].is_left() && !g[0].is_deleted())
            .map(|g| g[0].file.clone())
            .or_else(|| log.latest().map(|v| v.file.clone()))
            .unwrap_or_default();
        let mut entry = conflicts::summarize(root, &log, &file, p.now);
        entry.left = left_scopes(root, p, &log.versions);
        if entry.is_empty() {
            summary.notes.remove(id);
        } else {
            summary.notes.insert(id.clone(), entry);
        }
    }
    if summary != before {
        conflicts::write(lock, &summary)?;
    }
    Ok(())
}

/// Rewrites the summary's entries for `notes`, for a caller that recorded a version of them itself (a restore, a
/// scope change). The caller holds the history lock.
pub fn refresh_open(lock: &Lock, p: &Params, notes: &[String]) -> Result<(), String> {
    refresh(lock, p, &notes.iter().cloned().collect())
}

/// Rewrites the summary from every note's log: a start-up repair for a summary a crash left behind, or that does not
/// parse.
pub fn rebuild_open(lock: &Lock, p: &Params) -> Result<(), String> {
    let root = lock.root();
    let mut notes: BTreeSet<String> = versions::note_ids(root)?.into_iter().collect();
    notes.extend(conflicts::read(root).unwrap_or_default().notes.into_keys());
    refresh(lock, p, &notes)
}

fn declaration_key(declaration: &Declaration) -> String {
    format!(
        "declare:{}:{}:{}:{}",
        declaration.declare,
        declaration.reason,
        declaration.at,
        declaration.device.as_deref().unwrap_or("")
    )
}

/// Applies what `stage` put in the inbox, once for every note it touches: each note's heads (its log with the staged
/// versions) are merged, its file is written through the hidden file `notes/.bilbo-restore-<id>`, and only then do
/// the staged versions and the merges reach its log. Versions that wait for a parent stay in the inbox. Nothing is
/// written while `notes/` cannot be listed. `hook` is called before each `Step`, and `exchange` stands in for the
/// atomic exchange. Returns the lines to print, without `bilbo: `.
///
/// A watcher cycle runs `apply` first, then `versions::sweep_restore_leftovers` with `staged`, then the scan: `apply`
/// recognises the hidden file of its own interrupted write, merges included, and removes it, so the sweep only
/// records what is foreign.
pub fn apply(
    lock: &Lock,
    p: &Params,
    hook: &mut dyn FnMut(Step) -> Result<(), String>,
    exchange: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let root = lock.root();
    if versions::list(&root.join("notes")).is_err() {
        return Ok(Vec::new());
    }
    let mut engine = Engine {
        lock,
        p,
        at: versions::now_at(),
        hook,
        exchange,
        events: Vec::new(),
        seen: None,
        touched: BTreeSet::new(),
    };
    let inbox = read_inbox(root)?;
    let mut notes: BTreeSet<String> = inbox
        .iter()
        .filter_map(|e| e.record.as_ref())
        .map(|r| r.note.clone())
        .collect();
    notes.extend(read_map::<Written>(root, WRITTEN)?.into_keys());
    let mut done = HashSet::new();
    let applied = (|| {
        for id in notes {
            engine.seen = None;
            let held = engine.held(&id)?;
            engine.settle(&id, held, &mut done)?;
        }
        engine.declare(&inbox, &mut done)
    })();
    let flushed = engine.flush();
    applied?;
    flushed?;
    let rest: Vec<Staged> = read_inbox(root)?
        .into_iter()
        .filter(|e| match (&e.record, &e.declaration) {
            (Some(r), _) => !done.contains(&r.version.version),
            (None, Some(d)) => !done.contains(&declaration_key(d)),
            _ => false,
        })
        .collect();
    write_inbox(root, &rest)?;
    let mut events = engine.events;
    events.dedup();
    Ok(events)
}

/// Records a local change to a note's file: `bytes` is its text, none when the file is gone. A save follows the one
/// head group the file last held, never a version that waits to be written, and carries the text it dropped from an
/// open conflict. While a stale-base entry exists the save is merged against it, as the Stale base requirement says.
/// Every local save goes through here. Returns the lines to print.
pub fn record_local(
    lock: &Lock,
    p: &Params,
    note: &str,
    file: &str,
    bytes: Option<&[u8]>,
    at: &str,
) -> Result<Vec<String>, String> {
    record_local_with(
        lock,
        p,
        &mut |_| Ok(()),
        &mut swap::exchange,
        note,
        file,
        bytes,
        at,
    )
}

/// `record_local` with a hook and an exchange, as `apply` takes them.
#[expect(
    clippy::too_many_arguments,
    reason = "record_local's inputs and apply's two hooks"
)]
pub fn record_local_with(
    lock: &Lock,
    p: &Params,
    hook: &mut dyn FnMut(Step) -> Result<(), String>,
    exchange: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
    note: &str,
    file: &str,
    bytes: Option<&[u8]>,
    at: &str,
) -> Result<Vec<String>, String> {
    let mut engine = Engine {
        lock,
        p,
        at: at.to_string(),
        hook,
        exchange,
        events: Vec::new(),
        seen: None,
        touched: BTreeSet::new(),
    };
    let saved = (|| -> Result<(), String> {
        let held = engine.held(note)?;
        if !held.extras.is_empty() {
            // A killed write held saves back: finish it first, which records whatever the file holds too.
            return engine.settle(note, held, &mut HashSet::new());
        }
        let pending = engine.local(note, file, bytes, &[])?;
        if !pending.extras.is_empty() {
            engine.settle(note, pending, &mut HashSet::new())?;
        }
        Ok(())
    })();
    let flushed = engine.flush();
    saved?;
    flushed?;
    Ok(engine.events)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use super::*;

    const AT: &str = "2026-10-04T12:00:00-03:00";
    const ID: &str = "01JAAAAAAAAAAAAAAAAAAAAAAA";
    const FILE: &str = "plan-release.md";

    struct T {
        root: PathBuf,
        lock: Lock,
    }

    impl Drop for T {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn now() -> jiff::Timestamp {
        "2026-10-04T15:00:00Z".parse().unwrap()
    }

    fn store(name: &str) -> T {
        let root =
            std::env::temp_dir().join(format!("bilbo-integrate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("notes")).unwrap();
        let lock = versions::lock(&root).unwrap();
        T { root, lock }
    }

    /// A note whose passages hold `setup` and `rollout`.
    fn text(setup: &str, rollout: &str) -> String {
        format!(
            "---\nid: {ID}\ncreated: 2026-10-04T12:00-03:00\nscope: personal\n---\n\n# Plan\n\n## Setup\n{setup}\n\n## Rollout\n{rollout}\n"
        )
    }

    fn base() -> String {
        text("base setup", "base rollout")
    }

    fn write(t: &T, file: &str, text: &str) {
        fs::write(t.root.join("notes").join(file), text).unwrap();
    }

    fn read(t: &T, file: &str) -> String {
        fs::read_to_string(t.root.join("notes").join(file)).unwrap()
    }

    fn exists(t: &T, file: &str) -> bool {
        t.root.join("notes").join(file).exists()
    }

    /// Records `text` as a version after `parents` and, when `on_disk`, writes it as the file.
    fn local(t: &T, parents: &[&Version], text: &str, on_disk: bool) -> Version {
        let parents: Vec<String> = parents.iter().map(|v| v.version.clone()).collect();
        if on_disk {
            write(t, FILE, text);
        }
        versions::record(
            &t.lock,
            ID,
            &parents,
            FILE,
            Some(text.as_bytes()),
            "edited",
            AT,
        )
        .unwrap()
    }

    fn incoming(parents: &[&Version], file: &str, text: Option<&str>) -> (Record, Option<Blob>) {
        incoming_for(ID, parents, file, text)
    }

    fn incoming_for(
        id: &str,
        parents: &[&Version],
        file: &str,
        text: Option<&str>,
    ) -> (Record, Option<Blob>) {
        let parents: Vec<String> = parents.iter().map(|v| v.version.clone()).collect();
        let blob = text.map_or_else(
            || versions::DELETED.to_string(),
            |t| hash::sha256_hex(t.as_bytes()),
        );
        let version = Version {
            version: versions::version_id(id, &parents, file, &blob),
            parents,
            file: file.to_string(),
            blob,
            event: if text.is_some() { "edited" } else { "deleted" }.to_string(),
            at: AT.to_string(),
            device: Some("bagend".into()),
            ..Version::default()
        };
        let record = Record {
            note: id.to_string(),
            version,
        };
        (record, text.map(|t| Blob::new(t.as_bytes())))
    }

    fn stage_all(t: &T, items: &[(Record, Option<Blob>)]) -> Vec<String> {
        let records: Vec<Record> = items.iter().map(|(r, _)| r.clone()).collect();
        let blobs: Vec<Blob> = items.iter().filter_map(|(_, b)| b.clone()).collect();
        stage(&t.lock, "personal", &records, &blobs, &[], now()).unwrap()
    }

    fn params(syncs: &dyn Fn(&str) -> bool) -> Params<'_> {
        Params {
            syncs,
            now: now(),
            stale_days: 180,
        }
    }

    fn syncs_personal(scope: &str) -> bool {
        scope == "personal"
    }

    fn run(t: &T) -> Vec<String> {
        let p = params(&syncs_personal);
        apply(&t.lock, &p, &mut |_| Ok(()), &mut swap::exchange).unwrap()
    }

    fn log(t: &T) -> Vec<Version> {
        versions::load(&t.root, ID).unwrap().versions
    }

    fn inbox_len(t: &T) -> usize {
        read_inbox(&t.root).unwrap().len()
    }

    fn hidden(t: &T) -> bool {
        versions::restore_path(&t.root, ID).exists()
    }

    /// A store whose note holds the base text, recorded as `added`.
    fn started(name: &str) -> (T, Version) {
        let t = store(name);
        write(&t, FILE, &base());
        let h =
            versions::record(&t.lock, ID, &[], FILE, Some(base().as_bytes()), "added", AT).unwrap();
        (t, h)
    }

    #[test]
    fn a_version_that_follows_the_head_is_written_and_recorded() {
        let (t, h) = started("ff");
        let new = text("their setup", "base rollout");
        let incoming = incoming(&[&h], FILE, Some(&new));
        assert!(stage_all(&t, std::slice::from_ref(&incoming)).is_empty());
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), new);
        let versions = log(&t);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].version, incoming.0.version.version);
        assert_eq!(versions[1].device.as_deref(), Some("bagend"));
        assert_eq!(inbox_len(&t), 0);
        assert!(!hidden(&t));
        let base = stale_base(&t.root, ID).unwrap().unwrap();
        assert_eq!(
            (base.base, base.written),
            (h.version, versions[1].version.clone())
        );
    }

    #[test]
    fn two_passages_on_two_devices_merge_clean() {
        let (t, h) = started("clean");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let theirs = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&theirs));
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), text("their setup", "my rollout"));
        let versions = log(&t);
        let merged = versions.last().unwrap();
        assert_eq!(merged.event, versions::MERGED);
        let mut parents = merged.parents.clone();
        parents.sort();
        let mut want = vec![mine.version.clone(), theirs.0.version.version.clone()];
        want.sort();
        assert_eq!(parents, want);
        assert!(merged.conflict.is_empty() && merged.flags.is_empty());
        assert_eq!(versions.len(), 4);
    }

    #[test]
    fn a_conflict_is_written_with_its_block_and_named() {
        let (t, h) = started("conflict");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("base setup", "their rollout")),
            )],
        );
        let events = run(&t);
        assert_eq!(
            events,
            ["notes/plan-release.md: conflict in 1 passage; run bilbo check"]
        );
        let body = read(&t, FILE);
        assert!(
            body.contains("<<<<<<< bilbo ")
                && body.contains("my rollout")
                && body.contains("their rollout")
        );
        let merged = log(&t).pop().unwrap();
        assert_eq!(merged.conflict.len(), 1);
        assert_eq!(merged.conflict[0].passage, "Rollout");
    }

    #[test]
    fn three_heads_merge_two_at_a_time_and_every_device_agrees() {
        let mut ids = Vec::new();
        for name in ["three-a", "three-b"] {
            let (t, h) = started(name);
            let items = [
                incoming(&[&h], FILE, Some(&text("setup one", "base rollout"))),
                incoming(&[&h], FILE, Some(&text("base setup", "rollout two"))),
                incoming(&[&h], FILE, Some(&base().replace("# Plan", "# Plan three"))),
            ];
            let order: Vec<_> = if name == "three-a" {
                items.to_vec()
            } else {
                items.iter().rev().cloned().collect()
            };
            stage_all(&t, &order);
            assert!(run(&t).is_empty());
            let body = read(&t, FILE);
            assert!(
                body.contains("setup one")
                    && body.contains("rollout two")
                    && body.contains("# Plan three")
            );
            let versions = log(&t);
            let merges: Vec<&Version> = versions
                .iter()
                .filter(|v| v.event == versions::MERGED)
                .collect();
            assert_eq!(merges.len(), 2);
            assert_eq!(merges[1].parents.len(), 2);
            assert!(merges[1].parents.contains(&merges[0].version));
            let mut all: Vec<String> = versions.iter().map(|v| v.version.clone()).collect();
            all.sort();
            ids.push((all, body));
        }
        assert_eq!(ids[0], ids[1]);
    }

    #[test]
    fn the_same_content_on_two_devices_is_not_merged() {
        let (t, h) = started("same");
        let new = text("shared setup", "base rollout");
        local(&t, &[&h], &new, true);
        let middle = incoming(&[&h], FILE, Some(&text("middle", "base rollout")));
        let same = incoming(&[&middle.0.version], FILE, Some(&new));
        stage_all(&t, &[middle, same]);
        run(&t);
        let versions = log(&t);
        assert_eq!(versions.len(), 4);
        assert!(versions.iter().all(|v| v.event != versions::MERGED));
        assert_eq!(read(&t, FILE), new);
    }

    #[test]
    fn a_version_whose_parent_has_not_arrived_waits_then_applies_when_stale() {
        let (t, h) = started("waiting");
        let missing = incoming(&[&h], FILE, Some(&text("middle", "base rollout")));
        let child = incoming(
            &[&missing.0.version],
            FILE,
            Some(&text("later", "base rollout")),
        );
        stage_all(&t, std::slice::from_ref(&child));
        run(&t);
        assert_eq!(read(&t, FILE), base());
        assert_eq!(inbox_len(&t), 1);
        let p = Params {
            syncs: &syncs_personal,
            now: now() + jiff::SignedDuration::from_hours(24 * 181),
            stale_days: 180,
        };
        apply(&t.lock, &p, &mut |_| Ok(()), &mut swap::exchange).unwrap();
        let body = read(&t, FILE);
        assert!(body.contains("later") && body.contains("base setup"));
        assert_eq!(inbox_len(&t), 0);
        assert!(log(&t).iter().any(|v| v.version == child.0.version.version));
    }

    #[test]
    fn a_version_arriving_in_any_order_applies_once_its_parent_is_there() {
        let (t, h) = started("order");
        let first = incoming(&[&h], FILE, Some(&text("one", "base rollout")));
        let second = incoming(
            &[&first.0.version],
            FILE,
            Some(&text("two", "base rollout")),
        );
        stage_all(&t, &[second]);
        run(&t);
        assert_eq!(read(&t, FILE), base());
        stage_all(&t, &[first]);
        run(&t);
        assert_eq!(read(&t, FILE), text("two", "base rollout"));
        assert_eq!(log(&t).len(), 3);
    }

    #[test]
    fn staging_skips_other_scopes_and_what_it_already_holds() {
        let (t, h) = started("stage");
        let wrong = text("x", "y").replace("scope: personal", "scope: work");
        let bad = incoming(&[&h], FILE, Some(&wrong));
        let good = incoming(&[&h], FILE, Some(&text("good", "base rollout")));
        let events = stage_all(&t, &[bad, good.clone()]);
        assert_eq!(events.len(), 1);
        assert!(events[0].starts_with("sync personal: skipped version "));
        assert!(events[0].ends_with("does not say scope personal"));
        assert_eq!(inbox_len(&t), 1);
        stage_all(&t, std::slice::from_ref(&good));
        assert_eq!(inbox_len(&t), 1);
        run(&t);
        stage_all(&t, &[good]);
        assert_eq!(inbox_len(&t), 0);
    }

    #[test]
    fn three_hundred_versions_write_the_file_once() {
        let (t, h) = started("year");
        let mut items = Vec::new();
        let mut parent = h.clone();
        for n in 0..300 {
            let item = incoming(
                &[&parent],
                FILE,
                Some(&text(&format!("setup {n}"), "base rollout")),
            );
            parent = item.0.version.clone();
            items.push(item);
        }
        stage_all(&t, &items);
        let swaps = Cell::new(0);
        let p = params(&syncs_personal);
        apply(&t.lock, &p, &mut |_| Ok(()), &mut |a, b| {
            swaps.set(swaps.get() + 1);
            swap::exchange(a, b)
        })
        .unwrap();
        assert_eq!(swaps.get(), 1);
        assert_eq!(read(&t, FILE), text("setup 299", "base rollout"));
        assert_eq!(log(&t).len(), 301);
    }

    #[test]
    fn a_save_not_yet_recorded_at_poll_time_survives() {
        let (t, h) = started("unrecorded");
        write(&t, FILE, &text("base setup", "agent rollout"));
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), text("their setup", "agent rollout"));
        let versions = log(&t);
        assert_eq!(versions.len(), 4);
        let save = &versions[1];
        assert_eq!(save.event, "edited");
        assert_eq!(save.parents, std::slice::from_ref(&h.version));
        assert_eq!(versions[3].event, versions::MERGED);
    }

    #[test]
    fn a_write_between_the_read_and_the_exchange_is_kept() {
        let (t, h) = started("race");
        let theirs = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&theirs));
        let fired = Cell::new(false);
        let path = t.root.join("notes").join(FILE);
        let p = params(&syncs_personal);
        apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap && !fired.replace(true) {
                    fs::write(&path, text("base setup", "agent rollout")).unwrap();
                }
                Ok(())
            },
            &mut swap::exchange,
        )
        .unwrap();
        assert_eq!(read(&t, FILE), text("their setup", "agent rollout"));
        let versions = log(&t);
        let save = versions
            .iter()
            .find(|v| v.blob == hash::sha256_hex(text("base setup", "agent rollout").as_bytes()))
            .unwrap();
        assert_eq!(save.parents, std::slice::from_ref(&h.version));
        assert_eq!(save.event, "edited");
        assert!(
            versions
                .iter()
                .any(|v| v.version == theirs.0.version.version)
        );
        assert_eq!(versions.last().unwrap().event, versions::MERGED);
        assert!(!hidden(&t));
        assert_eq!(inbox_len(&t), 0);
    }

    #[test]
    fn a_filesystem_that_cannot_swap_writes_nothing_and_keeps_the_version() {
        let (t, h) = started("noswap");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        let p = params(&syncs_personal);
        let events = apply(&t.lock, &p, &mut |_| Ok(()), &mut |_, _| {
            Err(swap::UNSUPPORTED.to_string())
        })
        .unwrap();
        assert_eq!(
            events,
            [
                "sync personal: cannot write notes on this filesystem: it cannot swap files atomically"
            ]
        );
        assert_eq!(read(&t, FILE), base());
        assert!(!hidden(&t));
        assert_eq!(log(&t).len(), 1);
        assert_eq!(inbox_len(&t), 1);
        run(&t);
        assert_eq!(read(&t, FILE), text("their setup", "base rollout"));
    }

    #[test]
    fn a_kill_before_the_rename_of_a_new_note_is_completed_without_a_deletion() {
        let t = store("kill-new");
        let theirs = incoming(&[], FILE, Some(&base()));
        stage_all(&t, std::slice::from_ref(&theirs));
        let p = params(&syncs_personal);
        let killed = apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap {
                    Err("killed".into())
                } else {
                    Ok(())
                }
            },
            &mut swap::exchange,
        );
        assert_eq!(killed, Err("killed".to_string()));
        assert!(hidden(&t) && !exists(&t, FILE));
        let staged = staged(&t.root).unwrap();
        assert_eq!(staged.len(), 1);
        let swept = versions::sweep_restore_leftovers(&t.lock, AT, &staged).unwrap();
        assert!(swept.is_empty() && hidden(&t));
        run(&t);
        assert_eq!(read(&t, FILE), base());
        assert!(!hidden(&t));
        let versions = log(&t);
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, theirs.0.version.version);
    }

    #[test]
    fn a_kill_before_the_exchange_of_an_edit_leaves_no_extra_version() {
        let (t, h) = started("kill-edit");
        let new = text("their setup", "base rollout");
        let theirs = incoming(&[&h], FILE, Some(&new));
        stage_all(&t, std::slice::from_ref(&theirs));
        let p = params(&syncs_personal);
        let killed = apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap {
                    Err("killed".into())
                } else {
                    Ok(())
                }
            },
            &mut swap::exchange,
        );
        assert!(killed.is_err() && hidden(&t));
        assert_eq!(read(&t, FILE), base());
        versions::sweep_restore_leftovers(&t.lock, AT, &staged(&t.root).unwrap()).unwrap();
        run(&t);
        assert_eq!(read(&t, FILE), new);
        let ids: Vec<String> = log(&t).into_iter().map(|v| v.version).collect();
        assert_eq!(ids, [h.version, theirs.0.version.version]);
    }

    #[test]
    fn a_kill_after_the_exchange_is_recorded_without_a_save() {
        let (t, h) = started("kill-after");
        let new = text("their setup", "base rollout");
        let theirs = incoming(&[&h], FILE, Some(&new));
        stage_all(&t, std::slice::from_ref(&theirs));
        let p = params(&syncs_personal);
        let killed = apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Inspect {
                    Err("killed".into())
                } else {
                    Ok(())
                }
            },
            &mut swap::exchange,
        );
        assert!(killed.is_err());
        assert_eq!(read(&t, FILE), new);
        let swept =
            versions::sweep_restore_leftovers(&t.lock, AT, &staged(&t.root).unwrap()).unwrap();
        assert!(swept.is_empty());
        run(&t);
        let versions = log(&t);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].version, theirs.0.version.version);
        assert_eq!(read(&t, FILE), new);
    }

    #[test]
    fn a_save_while_a_version_waits_follows_the_log_head() {
        let (t, h) = started("waits");
        let theirs = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&theirs));
        let p = params(&syncs_personal);
        apply(&t.lock, &p, &mut |_| Ok(()), &mut |_, _| Err("no".into())).unwrap();
        assert_eq!(read(&t, FILE), base());
        write(&t, FILE, &text("base setup", "agent rollout"));
        record_local(
            &t.lock,
            &p,
            ID,
            FILE,
            Some(text("base setup", "agent rollout").as_bytes()),
            AT,
        )
        .unwrap();
        let save = log(&t).pop().unwrap();
        assert_eq!(save.parents, std::slice::from_ref(&h.version));
        run(&t);
        assert_eq!(read(&t, FILE), text("their setup", "agent rollout"));
        let merged = log(&t).pop().unwrap();
        assert!(
            merged.parents.contains(&save.version)
                && merged.parents.contains(&theirs.0.version.version)
        );
    }

    #[test]
    fn an_unwritable_notes_folder_waits() {
        let (t, h) = started("gone");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        fs::remove_dir_all(t.root.join("notes")).unwrap();
        assert!(run(&t).is_empty());
        assert!(!t.root.join("notes").exists());
        assert_eq!(inbox_len(&t), 1);
        fs::create_dir(t.root.join("notes")).unwrap();
        write(&t, FILE, &base());
        run(&t);
        assert_eq!(read(&t, FILE), text("their setup", "base rollout"));
    }

    #[test]
    fn a_stale_save_is_merged_against_the_version_before_the_write() {
        let (t, h) = started("stale");
        let theirs = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&theirs));
        run(&t);
        let p = params(&syncs_personal);
        let mine = text("base setup", "agent rollout");
        write(&t, FILE, &mine);
        let events = record_local(&t.lock, &p, ID, FILE, Some(mine.as_bytes()), AT).unwrap();
        assert!(events.is_empty());
        assert_eq!(read(&t, FILE), text("their setup", "agent rollout"));
        let versions = log(&t);
        let save = &versions[2];
        assert_eq!(save.parents, std::slice::from_ref(&h.version));
        let merged = versions.last().unwrap();
        assert_eq!(merged.event, versions::MERGED);
        assert!(merged.flags.contains(&"stale-base".to_string()));
        assert_eq!(merged.parents.len(), 2);
    }

    #[test]
    fn a_save_made_from_the_new_text_follows_the_written_version() {
        let (t, h) = started("fresh");
        let theirs = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&theirs));
        run(&t);
        let p = params(&syncs_personal);
        let mine = text("their setup", "agent rollout");
        write(&t, FILE, &mine);
        record_local(&t.lock, &p, ID, FILE, Some(mine.as_bytes()), AT).unwrap();
        let versions = log(&t);
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[2].parents, [theirs.0.version.version]);
        assert_eq!(versions[2].event, "edited");
        assert_eq!(stale_base(&t.root, ID).unwrap(), None);
    }

    #[test]
    fn a_verb_keeps_the_base_and_changes_the_written_version() {
        let (t, h) = started("keep");
        keep_base(&t.lock, ID, "ignored").unwrap();
        assert_eq!(stale_base(&t.root, ID).unwrap(), None);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        run(&t);
        let restored = local(&t, &[&h], &base(), true);
        keep_base(&t.lock, ID, &restored.version).unwrap();
        let entry = stale_base(&t.root, ID).unwrap().unwrap();
        assert_eq!((entry.base, entry.written), (h.version, restored.version));
    }

    #[test]
    fn a_deletion_removes_the_file_and_edit_beats_it() {
        let (t, h) = started("delete");
        let gone = incoming(&[&h], FILE, None);
        stage_all(&t, std::slice::from_ref(&gone));
        run(&t);
        assert!(!exists(&t, FILE) && !hidden(&t));
        let versions = log(&t);
        assert_eq!(versions.len(), 2);
        assert!(versions[1].is_deleted() && !versions[1].is_left());

        let (t, h) = started("edit-beats-delete");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let gone = incoming(&[&h], FILE, None);
        stage_all(&t, &[gone]);
        run(&t);
        assert_eq!(read(&t, FILE), text("base setup", "my rollout"));
        let merged = log(&t).pop().unwrap();
        assert_eq!(merged.flags, ["edit-beat-delete"]);
        assert!(merged.parents.contains(&mine.version) && merged.parents.len() == 2);
    }

    #[test]
    fn a_local_delete_after_a_sync_edit_loses_once() {
        let (t, h) = started("local-delete");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        run(&t);
        fs::remove_file(t.root.join("notes").join(FILE)).unwrap();
        let p = params(&syncs_personal);
        record_local(&t.lock, &p, ID, FILE, None, AT).unwrap();
        assert_eq!(read(&t, FILE), text("their setup", "base rollout"));
        let merged = log(&t).pop().unwrap();
        assert_eq!(merged.flags, ["edit-beat-delete"]);
        fs::remove_file(t.root.join("notes").join(FILE)).unwrap();
        record_local(&t.lock, &p, ID, FILE, None, AT).unwrap();
        assert!(!exists(&t, FILE));
        assert!(log(&t).pop().unwrap().is_deleted());
    }

    #[test]
    fn a_left_follows_the_head_and_never_merges_with_a_local_edit() {
        let (t, h) = started("left");
        let mut left = incoming(&[&h], "", None);
        left.0.version.event = versions::LEFT.to_string();
        left.0.version.version = "ab".repeat(32);
        stage_all(&t, std::slice::from_ref(&left));
        let events = run(&t);
        assert_eq!(
            events,
            ["sync personal: notes/plan-release.md left the scope; its history stays"]
        );
        assert!(!exists(&t, FILE));
        assert!(log(&t).pop().unwrap().is_left());

        let (t, h) = started("left-concurrent");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(&t, &[left]);
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), text("base setup", "my rollout"));
        let versions = log(&t);
        assert!(versions.iter().any(Version::is_left));
        assert!(versions.iter().all(|v| v.event != versions::MERGED));
    }

    #[test]
    fn a_declaration_reaches_the_log_when_its_conflict_is_known() {
        let (t, h) = started("declare");
        let declaration = Declaration {
            declare: h.version.clone(),
            reason: "intended".into(),
            at: AT.into(),
            device: Some("bagend".into()),
        };
        let stray = Declaration {
            declare: "cd".repeat(32),
            ..declaration.clone()
        };
        stage(
            &t.lock,
            "personal",
            &[],
            &[],
            &[declaration.clone(), stray],
            now(),
        )
        .unwrap();
        run(&t);
        assert_eq!(
            versions::load(&t.root, ID).unwrap().declarations,
            std::slice::from_ref(&declaration)
        );
        assert_eq!(inbox_len(&t), 1);
        stage(
            &t.lock,
            "personal",
            &[],
            &[],
            std::slice::from_ref(&declaration),
            now(),
        )
        .unwrap();
        run(&t);
        assert_eq!(
            versions::load(&t.root, ID).unwrap().declarations,
            [declaration]
        );
    }

    #[test]
    fn a_prune_keeps_the_text_of_a_staged_version() {
        let (t, h) = started("prune");
        let new = text("their setup", "base rollout");
        let theirs = incoming(&[&h], FILE, Some(&new));
        stage_all(&t, std::slice::from_ref(&theirs));
        let later = now() + jiff::SignedDuration::from_hours(24 * 400);
        let blobs = staged_blobs(&t.root).unwrap();
        versions::prune(&t.lock, 1, later, &versions::Guard::new(), &blobs).unwrap();
        assert_eq!(
            versions::content(&t.root, &theirs.0.version).unwrap(),
            new.as_bytes()
        );
    }

    #[test]
    fn temporaries_under_sync_and_scopes_are_swept() {
        let t = store("tmp");
        let sync = store::sync_dir(&t.root);
        let scope = store::scopes_dir(&t.root).join("scope-one");
        fs::create_dir_all(scope.join("out")).unwrap();
        fs::create_dir_all(&sync).unwrap();
        for path in [
            sync.join(".tmp-a"),
            scope.join(".tmp-b"),
            scope.join("out/.tmp-c"),
        ] {
            fs::write(path, "x").unwrap();
        }
        fs::write(scope.join("state.json"), "{}").unwrap();
        assert_eq!(sweep_temporaries(&t.root).unwrap(), 3);
        assert!(scope.join("state.json").exists());
    }

    #[test]
    fn resolving_a_conflict_records_the_text_it_dropped() {
        let (t, h) = started("dropped");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("base setup", "their rollout")),
            )],
        );
        run(&t);
        let p = params(&syncs_personal);
        let resolved = text("base setup", "my rollout");
        write(&t, FILE, &resolved);
        record_local(&t.lock, &p, ID, FILE, Some(resolved.as_bytes()), AT).unwrap();
        let save = log(&t).pop().unwrap();
        assert_eq!(save.dropped.len(), 1);
        assert_eq!(save.dropped[0].lines, ["their rollout"]);
    }

    fn kill_at(step: Step) -> impl FnMut(Step) -> Result<(), String> {
        move |s| {
            if s == step {
                Err("killed".into())
            } else {
                Ok(())
            }
        }
    }

    fn rows(t: &T) -> Vec<(String, Vec<String>)> {
        log(t)
            .into_iter()
            .map(|v| {
                (
                    v.event,
                    v.parents.iter().map(|p| p[..12].to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_kill_between_the_exchange_and_the_rename_finishes_the_rename() {
        let (t, h) = started("rename-kill");
        let new = text("their setup", "base rollout");
        let theirs = incoming(&[&h], "decision-release.md", Some(&new));
        stage_all(&t, std::slice::from_ref(&theirs));
        let p = params(&syncs_personal);
        let killed = apply(&t.lock, &p, &mut kill_at(Step::Rename), &mut swap::exchange);
        assert!(killed.is_err());
        assert_eq!(read(&t, FILE), new);
        run(&t);
        versions::sweep_restore_leftovers(&t.lock, AT, &staged(&t.root).unwrap()).unwrap();
        assert!(exists(&t, "decision-release.md") && !exists(&t, FILE));
        let ids: Vec<String> = log(&t).into_iter().map(|v| v.version).collect();
        assert_eq!(ids, [h.version, theirs.0.version.version]);
    }

    #[test]
    fn five_races_leave_the_agents_file_and_the_next_cycle_merges_it() {
        let (t, h) = started("five");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        let n = Cell::new(0);
        let path = t.root.join("notes").join(FILE);
        let p = params(&syncs_personal);
        let events = apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap {
                    n.set(n.get() + 1);
                    let rollout = format!("agent rollout {}", n.get());
                    fs::write(&path, text("base setup", &rollout)).unwrap();
                }
                Ok(())
            },
            &mut swap::exchange,
        )
        .unwrap();
        assert_eq!(
            events,
            ["notes/plan-release.md: not written: the file kept changing"]
        );
        let last = format!("agent rollout {}", n.get());
        assert!(read(&t, FILE).contains(&last));
        assert!(!hidden(&t));
        assert!(run(&t).is_empty());
        let body = read(&t, FILE);
        assert!(
            body.contains(&last) && body.contains("their setup"),
            "{body}"
        );
    }

    #[test]
    fn a_kill_before_the_exchange_of_a_merge_is_completed_before_the_sweep() {
        let (t, h) = started("merge-leftover");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        let p = params(&syncs_personal);
        let killed = apply(&t.lock, &p, &mut kill_at(Step::Swap), &mut swap::exchange);
        assert!(killed.is_err() && hidden(&t));
        run(&t);
        let swept =
            versions::sweep_restore_leftovers(&t.lock, AT, &staged(&t.root).unwrap()).unwrap();
        assert!(swept.is_empty() && !hidden(&t));
        assert_eq!(read(&t, FILE), text("their setup", "my rollout"));
        assert_eq!(log(&t).len(), 4);
    }

    #[test]
    fn a_foreign_leftover_keeps_the_note_unwritten() {
        let (t, h) = started("foreign");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        fs::write(versions::restore_path(&t.root, ID), "someone else's bytes").unwrap();
        run(&t);
        assert_eq!(read(&t, FILE), base());
        assert!(hidden(&t));
        assert_eq!(inbox_len(&t), 1);
    }

    #[test]
    fn a_stale_save_taken_out_of_the_swap_is_merged_against_the_entry_base() {
        let (t, h) = started("stale-race");
        let w0 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&w0));
        run(&t);
        let t2 = incoming(
            &[&w0.0.version],
            FILE,
            Some(&text("their setup", "their rollout")),
        );
        stage_all(&t, std::slice::from_ref(&t2));
        let fired = Cell::new(false);
        let path = t.root.join("notes").join(FILE);
        let stale = base().replace("# Plan", "# Plan by agent");
        let p = params(&syncs_personal);
        apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap && !fired.replace(true) {
                    fs::write(&path, &stale).unwrap();
                }
                Ok(())
            },
            &mut swap::exchange,
        )
        .unwrap();
        let body = read(&t, FILE);
        assert!(
            body.contains("# Plan by agent")
                && body.contains("their rollout")
                && body.contains("their setup")
        );
        let versions = log(&t);
        let save = versions
            .iter()
            .find(|v| v.blob == hash::sha256_hex(stale.as_bytes()))
            .unwrap();
        assert_eq!(save.parents, std::slice::from_ref(&h.version));
        assert!(
            versions
                .last()
                .unwrap()
                .flags
                .contains(&"stale-base".to_string())
        );
    }

    #[test]
    fn a_second_save_during_the_stale_merge_follows_the_first_and_loses_nothing() {
        let (t, h) = started("stale-second");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        run(&t);
        let p = params(&syncs_personal);
        let s1 = text("base setup", "agent rollout");
        write(&t, FILE, &s1);
        let fired = Cell::new(false);
        let path = t.root.join("notes").join(FILE);
        record_local_with(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Prepare && !fired.replace(true) {
                    fs::write(&path, text("base setup", "agent rollout 2")).unwrap();
                }
                Ok(())
            },
            &mut swap::exchange,
            ID,
            FILE,
            Some(s1.as_bytes()),
            AT,
        )
        .unwrap();
        let body = read(&t, FILE);
        assert!(
            body.contains("agent rollout 2") && body.contains("their setup"),
            "{body}"
        );
        let versions = log(&t);
        let live = versions::heads(&versions);
        assert_eq!(live.len(), 1);
        for v in versions.iter().filter(|v| v.event == "edited") {
            assert!(v.parents.len() <= 1, "{v:?}");
        }
    }

    #[test]
    fn a_failed_stale_write_leaves_one_head_and_the_entry() {
        let (t, h) = started("stale-failed");
        let w0 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&w0));
        run(&t);
        let p = params(&syncs_personal);
        let s1 = text("base setup", "agent rollout");
        write(&t, FILE, &s1);
        record_local_with(
            &t.lock,
            &p,
            &mut |_| Ok(()),
            &mut |_, _| Err("no".into()),
            ID,
            FILE,
            Some(s1.as_bytes()),
            AT,
        )
        .unwrap();
        assert_eq!(log(&t).len(), 2);
        assert!(stale_base(&t.root, ID).unwrap().is_some());
        let s2 = text("base setup", "agent rollout 2");
        write(&t, FILE, &s2);
        record_local(&t.lock, &p, ID, FILE, Some(s2.as_bytes()), AT).unwrap();
        let body = read(&t, FILE);
        assert!(
            body.contains("agent rollout 2") && body.contains("their setup"),
            "{body}"
        );
        assert_eq!(versions::heads(&log(&t)).len(), 1);
    }

    #[test]
    fn a_save_after_a_concurrent_left_follows_the_local_head() {
        let (t, h) = started("left-save");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let mut left = incoming(&[&h], "", None);
        left.0.version.event = versions::LEFT.to_string();
        left.0.version.version = "ab".repeat(32);
        stage_all(&t, &[left]);
        run(&t);
        let p = params(&syncs_personal);
        let s = text("base setup", "my rollout 2");
        write(&t, FILE, &s);
        record_local(&t.lock, &p, ID, FILE, Some(s.as_bytes()), AT).unwrap();
        assert_eq!(log(&t).pop().unwrap().parents, [mine.version]);
    }

    #[test]
    fn two_live_heads_give_a_save_one_parent_group() {
        let (t, h) = started("one-group");
        let a = local(&t, &[&h], &text("a setup", "base rollout"), false);
        let b = local(&t, &[&h], &text("base setup", "b rollout"), true);
        let p = params(&syncs_personal);
        let s = text("base setup", "b rollout 2");
        write(&t, FILE, &s);
        let mut engine = Engine {
            lock: &t.lock,
            p: &p,
            at: AT.into(),
            hook: &mut |_| Ok(()),
            exchange: &mut swap::exchange,
            events: Vec::new(),
            seen: Some((FILE.into(), b.blob.clone())),
            touched: BTreeSet::new(),
        };
        engine.local(ID, FILE, Some(s.as_bytes()), &[]).unwrap();
        let save = log(&t).pop().unwrap();
        assert_eq!(save.parents, [b.version]);
        let _ = a;
    }

    #[test]
    fn a_save_equal_to_the_written_bytes_keeps_the_entry() {
        let (t, h) = started("touch");
        let new = text("their setup", "base rollout");
        stage_all(&t, &[incoming(&[&h], FILE, Some(&new))]);
        run(&t);
        let p = params(&syncs_personal);
        record_local(&t.lock, &p, ID, FILE, Some(new.as_bytes()), AT).unwrap();
        assert!(stale_base(&t.root, ID).unwrap().is_some());
        assert_eq!(log(&t).len(), 2);
    }

    #[test]
    fn a_save_during_the_swap_of_a_deletion_wins() {
        let (t, h) = started("del-race");
        stage_all(&t, &[incoming(&[&h], FILE, None)]);
        let fired = Cell::new(false);
        let path = t.root.join("notes").join(FILE);
        let p = params(&syncs_personal);
        apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap && !fired.replace(true) {
                    fs::write(&path, text("base setup", "agent rollout")).unwrap();
                }
                Ok(())
            },
            &mut swap::exchange,
        )
        .unwrap();
        assert!(read(&t, FILE).contains("agent rollout"));
        assert!(!hidden(&t));
        assert_eq!(inbox_len(&t), 0);
    }

    #[test]
    fn a_blob_that_arrives_after_its_record_is_checked_for_its_scope() {
        let (t, h) = started("late-blob");
        let wrong = text("x", "y").replace("scope: personal", "scope: work");
        let (record, blob) = incoming(&[&h], FILE, Some(&wrong));
        stage(
            &t.lock,
            "personal",
            std::slice::from_ref(&record),
            &[],
            &[],
            now(),
        )
        .unwrap();
        assert_eq!(inbox_len(&t), 1);
        stage(&t.lock, "personal", &[record], &[blob.unwrap()], &[], now()).unwrap();
        let events = run(&t);
        assert_eq!(read(&t, FILE), base());
        assert!(
            events[0].ends_with("does not say scope personal"),
            "{events:?}"
        );
        assert_eq!(inbox_len(&t), 0);
    }

    #[test]
    fn a_kill_after_a_raced_exchange_keeps_the_agents_bytes_and_the_write() {
        let (t, h) = started("raced-kill");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        let path = t.root.join("notes").join(FILE);
        let p = params(&syncs_personal);
        let killed = apply(
            &t.lock,
            &p,
            &mut |step| match step {
                Step::Swap => {
                    fs::write(&path, text("base setup", "agent rollout")).unwrap();
                    Ok(())
                }
                Step::Inspect => Err("killed".into()),
                _ => Ok(()),
            },
            &mut swap::exchange,
        );
        assert!(killed.is_err() && hidden(&t));
        run(&t);
        let swept =
            versions::sweep_restore_leftovers(&t.lock, AT, &staged(&t.root).unwrap()).unwrap();
        assert_eq!(swept.len(), 1, "{swept:?}");
        run(&t);
        let body = read(&t, FILE);
        assert!(
            body.contains("agent rollout") && body.contains("their setup"),
            "{body}"
        );
    }

    #[test]
    fn a_deleted_head_beaten_by_an_edit_names_the_blocks_it_carries() {
        let (t, h) = started("ebd-conflict");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("base setup", "their rollout")),
            )],
        );
        run(&t);
        stage_all(&t, &[incoming(&[&h], FILE, None)]);
        run(&t);
        let last = log(&t).pop().unwrap();
        assert!(read(&t, FILE).contains("<<<<<<< bilbo "));
        assert_eq!(last.conflict.len(), 1);
    }

    #[test]
    fn a_parent_listed_outside_does_not_hold_a_version_back() {
        let (t, h) = started("outside");
        let mut item = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        item.0.version.parents.push("cd".repeat(32));
        item.0.version.outside.push("cd".repeat(32));
        item.0.version.version =
            versions::version_id(ID, &item.0.version.parents, FILE, &item.0.version.blob);
        stage_all(&t, &[item]);
        run(&t);
        assert_eq!(read(&t, FILE), text("their setup", "base rollout"));
    }

    #[test]
    fn a_new_note_never_replaces_a_file_that_holds_the_name() {
        let t = store("taken");
        let other = "not a note, no frontmatter\n".to_string();
        write(&t, FILE, &other);
        stage_all(&t, &[incoming(&[], FILE, Some(&base()))]);
        let events = run(&t);
        assert_eq!(
            events,
            ["notes/plan-release.md: not written: plan-release.md is taken by another note"]
        );
        assert_eq!(read(&t, FILE), other);
    }

    #[test]
    fn the_entry_base_is_the_version_the_file_held() {
        let (t, h) = started("held");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        run(&t);
        let entry = stale_base(&t.root, ID).unwrap().unwrap();
        assert_eq!(entry.base, mine.version);
        assert_eq!(entry.written, log(&t).pop().unwrap().version);
    }

    #[test]
    fn a_resolution_without_an_entry_records_the_dropped_text() {
        let (t, h) = started("plain-dropped");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("base setup", "their rollout")),
            )],
        );
        run(&t);
        fs::remove_file(store::sync_dir(&t.root).join(STALE_BASE)).unwrap();
        let p = params(&syncs_personal);
        let resolved = text("base setup", "my rollout");
        write(&t, FILE, &resolved);
        record_local(&t.lock, &p, ID, FILE, Some(resolved.as_bytes()), AT).unwrap();
        let save = log(&t).pop().unwrap();
        assert_eq!(save.dropped[0].lines, ["their rollout"]);
    }

    #[test]
    fn a_declaration_whose_version_never_arrives_is_dropped_when_stale() {
        let t = store("declare-stale");
        let declaration = Declaration {
            declare: "ef".repeat(32),
            reason: "intended".into(),
            at: AT.into(),
            device: None,
        };
        stage(&t.lock, "personal", &[], &[], &[declaration], now()).unwrap();
        run(&t);
        assert_eq!(inbox_len(&t), 1);
        let p = Params {
            syncs: &syncs_personal,
            now: now() + jiff::SignedDuration::from_hours(24 * 181),
            stale_days: 180,
        };
        let events = apply(&t.lock, &p, &mut |_| Ok(()), &mut swap::exchange).unwrap();
        assert_eq!(events.len(), 1);
        assert!(
            events[0].starts_with("sync personal: dropped a declaration of conflict efefefefefef")
        );
        assert_eq!(inbox_len(&t), 0);
    }
    const YOUNG: &str = "01JBBBBBBBBBBBBBBBBBBBQ2X7";

    fn note_text(id: &str, scope: &str, title: &str) -> String {
        format!(
            "---\nid: {id}\ncreated: 2026-10-04T12:00-03:00\nscope: {scope}\n---\n\n# {title}\n\n## Body\nfor {title}\n"
        )
    }

    /// Records a note's first version and writes its file.
    fn start_note(t: &T, id: &str, file: &str, text: &str) -> Version {
        write(t, file, text);
        versions::record(&t.lock, id, &[], file, Some(text.as_bytes()), "added", AT).unwrap()
    }

    fn log_of(t: &T, id: &str) -> Vec<Version> {
        versions::load(&t.root, id).unwrap().versions
    }

    fn version_ids(t: &T, id: &str) -> Vec<String> {
        log_of(t, id).into_iter().map(|v| v.version).collect()
    }

    fn scoped(text: &str, scope: &str) -> String {
        text.replace("scope: personal", &format!("scope: {scope}"))
    }

    fn stage_in(t: &T, scope: &str, items: &[(Record, Option<Blob>)]) {
        let records: Vec<Record> = items.iter().map(|(r, _)| r.clone()).collect();
        let blobs: Vec<Blob> = items.iter().filter_map(|(_, b)| b.clone()).collect();
        stage(&t.lock, scope, &records, &blobs, &[], now()).unwrap();
    }

    fn run_syncing(t: &T, scopes: &[&str]) -> Vec<String> {
        let syncs = |scope: &str| scopes.contains(&scope);
        let p = params(&syncs);
        apply(&t.lock, &p, &mut |_| Ok(()), &mut swap::exchange).unwrap()
    }

    /// A scope's `left` record of the version `moving`, as the wire carries it.
    fn left_record(
        id: &str,
        moving: &str,
        parents: &[&str],
        outside: &[&str],
    ) -> (Record, Option<Blob>) {
        let version = Version {
            version: moving.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            file: String::new(),
            blob: versions::DELETED.to_string(),
            event: versions::LEFT.to_string(),
            at: AT.to_string(),
            device: Some("bagend".into()),
            outside: outside.iter().map(|p| p.to_string()).collect(),
            ..Version::default()
        };
        (
            Record {
                note: id.to_string(),
                version,
            },
            None,
        )
    }

    #[test]
    fn a_younger_incoming_note_takes_the_suffixed_name() {
        let t = store("topic-incoming");
        assert_eq!(YOUNG.len(), 26);
        let old = note_text(ID, "personal", "Release");
        start_note(&t, ID, FILE, &old);
        let young = note_text(YOUNG, "personal", "Release again");
        let item = incoming_for(YOUNG, &[], FILE, Some(&young));
        stage_all(&t, std::slice::from_ref(&item));
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), old);
        assert_eq!(read(&t, "plan-release-q2x7.md"), young);
        assert!(!hidden(&t));
        let log = log_of(&t, YOUNG);
        assert_eq!(log.len(), 2);
        let renamed = &log[1];
        assert_eq!(renamed.event, versions::RENAMED);
        assert_eq!(renamed.flags, [TOPIC_TAKEN]);
        assert_eq!(renamed.file, "plan-release-q2x7.md");
        assert_eq!(
            renamed.parents,
            std::slice::from_ref(&item.0.version.version)
        );
        assert_eq!(
            renamed.version,
            versions::version_id(YOUNG, &renamed.parents, &renamed.file, &renamed.blob)
        );
        let again = run(&t);
        assert!(again.is_empty() && log_of(&t, YOUNG).len() == 2);
    }

    #[test]
    fn a_younger_local_note_is_renamed_here_after_its_unrecorded_save() {
        let t = store("topic-local");
        let young = note_text(YOUNG, "personal", "Release again");
        start_note(&t, YOUNG, FILE, &young);
        let edited = format!("{young}more\n");
        write(&t, FILE, &edited);
        let old = note_text(ID, "personal", "Release");
        stage_all(&t, &[incoming_for(ID, &[], FILE, Some(&old))]);
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), old);
        assert_eq!(read(&t, "plan-release-q2x7.md"), edited);
        let events: Vec<String> = log_of(&t, YOUNG).into_iter().map(|v| v.event).collect();
        assert_eq!(events, ["added", "edited", "renamed"]);
        assert_eq!(log_of(&t, YOUNG)[2].flags, [TOPIC_TAKEN]);
    }

    #[test]
    fn a_kill_before_the_rename_of_a_local_note_changes_nothing_and_the_next_cycle_finishes() {
        let t = store("topic-kill");
        let young = note_text(YOUNG, "personal", "Release again");
        start_note(&t, YOUNG, FILE, &young);
        let old = note_text(ID, "personal", "Release");
        stage_all(&t, &[incoming_for(ID, &[], FILE, Some(&old))]);
        let syncs = syncs_personal;
        let p = params(&syncs);
        let killed = apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Rename {
                    Err("killed".into())
                } else {
                    Ok(())
                }
            },
            &mut swap::exchange,
        );
        assert_eq!(killed.unwrap_err(), "killed");
        assert_eq!(read(&t, FILE), young);
        assert_eq!(log_of(&t, YOUNG).len(), 1);
        assert!(run(&t).is_empty());
        assert_eq!(read(&t, FILE), old);
        assert_eq!(read(&t, "plan-release-q2x7.md"), young);
        assert_eq!(log_of(&t, YOUNG).len(), 2);
    }

    #[test]
    fn both_devices_of_a_topic_collision_end_alike() {
        let old = note_text(ID, "personal", "Release");
        let young = note_text(YOUNG, "personal", "Release again");
        let a = store("topic-a");
        start_note(&a, ID, FILE, &old);
        let b = store("topic-b");
        start_note(&b, YOUNG, FILE, &young);
        stage_all(&a, &[incoming_for(YOUNG, &[], FILE, Some(&young))]);
        stage_all(&b, &[incoming_for(ID, &[], FILE, Some(&old))]);
        run(&a);
        run(&b);
        for t in [&a, &b] {
            assert_eq!(read(t, FILE), old);
            assert_eq!(read(t, "plan-release-q2x7.md"), young);
        }
        assert_eq!(version_ids(&a, YOUNG), version_ids(&b, YOUNG));
        assert_eq!(version_ids(&a, ID), version_ids(&b, ID));
    }

    #[test]
    fn a_rename_onto_a_taken_topic_converges() {
        let old = note_text(ID, "personal", "Release");
        let young = note_text(YOUNG, "personal", "Release again");
        // A renamed its note to `plan-release.md` while B created `decision-release.md`, whose id is older.
        let a = store("rename-a");
        let first = start_note(&a, YOUNG, "plan-x.md", &young);
        fs::rename(
            a.root.join("notes/plan-x.md"),
            a.root.join("notes/plan-release.md"),
        )
        .unwrap();
        versions::record(
            &a.lock,
            YOUNG,
            std::slice::from_ref(&first.version),
            "plan-release.md",
            Some(young.as_bytes()),
            "renamed",
            AT,
        )
        .unwrap();
        stage_all(
            &a,
            &[incoming_for(ID, &[], "decision-release.md", Some(&old))],
        );
        let b = store("rename-b");
        start_note(&b, ID, "decision-release.md", &old);
        let first_b = start_note(&b, YOUNG, "plan-x.md", &young);
        stage_all(
            &b,
            &[incoming_for(
                YOUNG,
                &[&first_b],
                "plan-release.md",
                Some(&young),
            )],
        );
        run(&a);
        run(&b);
        for t in [&a, &b] {
            assert_eq!(read(t, "decision-release.md"), old);
            assert_eq!(read(t, "plan-release-q2x7.md"), young);
            assert!(!exists(t, "plan-release.md") && !exists(t, "plan-x.md"));
        }
        assert_eq!(version_ids(&a, YOUNG), version_ids(&b, YOUNG));
    }

    #[test]
    fn a_taken_suffixed_name_adds_characters() {
        let t = store("topic-longer");
        start_note(&t, ID, FILE, &note_text(ID, "personal", "Release"));
        let third = "01JCCCCCCCCCCCCCCCCCCCCCCC";
        start_note(
            &t,
            third,
            "plan-release-q2x7.md",
            &note_text(third, "personal", "Third"),
        );
        let young = note_text(YOUNG, "personal", "Release again");
        stage_all(&t, &[incoming_for(YOUNG, &[], FILE, Some(&young))]);
        run(&t);
        assert_eq!(read(&t, "plan-release-bq2x7.md"), young);
    }

    #[test]
    fn a_note_whose_name_is_free_gets_no_rename() {
        let t = store("topic-free");
        start_note(&t, ID, FILE, &note_text(ID, "personal", "Release"));
        let other = note_text(YOUNG, "personal", "Other");
        stage_all(
            &t,
            &[incoming_for(YOUNG, &[], "plan-other.md", Some(&other))],
        );
        run(&t);
        assert_eq!(read(&t, "plan-other.md"), other);
        assert_eq!(log_of(&t, YOUNG).len(), 1);
    }

    #[test]
    fn edit_beats_delete_gives_every_device_the_same_merge() {
        let (a, h) = started("ebd-a");
        fs::remove_file(a.root.join("notes").join(FILE)).unwrap();
        let p = params(&syncs_personal);
        record_local(&a.lock, &p, ID, FILE, None, AT).unwrap();
        let edited = text("base setup", "their rollout");
        let edit = incoming(&[&h], FILE, Some(&edited));
        stage_all(&a, std::slice::from_ref(&edit));
        let (b, h) = started("ebd-b");
        local(&b, &[&h], &edited, true);
        let gone = incoming(&[&h], FILE, None);
        stage_all(&b, &[gone]);
        run(&a);
        run(&b);
        for t in [&a, &b] {
            assert_eq!(read(t, FILE), edited);
            let merged = log(t).pop().unwrap();
            assert_eq!(merged.event, versions::MERGED);
            assert_eq!(merged.flags, ["edit-beat-delete"]);
        }
        let mut left = version_ids(&a, ID);
        let mut right = version_ids(&b, ID);
        left.sort();
        right.sort();
        assert_eq!(left, right);
    }

    #[test]
    fn a_left_line_names_the_scope_its_record_came_through() {
        let (t, h) = started("left-scope");
        let left = left_record(ID, &"ab".repeat(32), &[&h.version], &[]);
        stage_in(&t, "shared", &[left]);
        let events = run(&t);
        assert_eq!(
            events,
            ["sync shared: notes/plan-release.md left the scope; its history stays"]
        );
    }

    #[test]
    fn a_left_line_names_its_own_scope_not_the_last_one_staged() {
        let (t, h) = started("left-own-scope");
        let left = left_record(ID, &"ab".repeat(32), &[&h.version], &[]);
        stage_in(&t, "shared", &[left]);
        let (mut waiting, blob) = incoming(&[&h], FILE, Some(&text("later", "base rollout")));
        waiting.version.parents = vec!["ef".repeat(32)];
        waiting.version.version =
            versions::version_id(ID, &waiting.version.parents, FILE, &waiting.version.blob);
        stage_all(&t, &[(waiting, blob)]);
        let events = run(&t);
        assert_eq!(
            events,
            ["sync shared: notes/plan-release.md left the scope; its history stays"]
        );
        assert_eq!(inbox_len(&t), 1);
    }

    #[test]
    fn two_unrelated_lefts_are_recorded_and_never_merged() {
        let (t, h) = started("two-lefts");
        let first = left_record(ID, &"ab".repeat(32), &[&h.version], &[]);
        let second = left_record(ID, &"cd".repeat(32), &[&h.version], &[]);
        stage_in(&t, "shared", &[first]);
        stage_in(&t, "team", &[second]);
        let events = run_syncing(&t, &["shared", "team"]);
        assert_eq!(events.len(), 1);
        assert!(!exists(&t, FILE));
        let versions = log(&t);
        assert_eq!(versions.len(), 3);
        assert!(versions.iter().all(|v| v.event != versions::MERGED));
        assert!(run_syncing(&t, &["shared", "team"]).is_empty());
        assert_eq!(log(&t).len(), 3);
    }

    #[test]
    fn a_left_never_starts_an_edit_beats_delete_round() {
        let (t, h) = started("left-not-delete");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let left = left_record(ID, &"ab".repeat(32), &[&h.version], &[]);
        stage_in(&t, "personal", &[left]);
        assert!(run(&t).is_empty());
        let versions = log(&t);
        assert!(versions.iter().all(|v| v.flags.is_empty()));
        assert!(versions.iter().all(|v| v.event != versions::MERGED));
        assert_eq!(read(&t, FILE), text("base setup", "my rollout"));
        assert_eq!(versions.last().unwrap().parents, [h.version]);
        let _ = mine;
    }

    #[test]
    fn a_merge_that_comes_back_as_a_left_removes_the_edited_note() {
        let (t, h) = started("left-after-edit");
        let edit = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let moved = "ab".repeat(32);
        let merged = "cd".repeat(32);
        stage_in(
            &t,
            "personal",
            &[left_record(ID, &moved, &[&h.version], &[])],
        );
        assert!(run(&t).is_empty());
        assert!(exists(&t, FILE));
        stage_in(
            &t,
            "personal",
            &[left_record(ID, &merged, &[&moved, &edit.version], &[])],
        );
        let events = run(&t);
        assert_eq!(
            events,
            ["sync personal: notes/plan-release.md left the scope; its history stays"]
        );
        assert!(!exists(&t, FILE));
        let versions = log(&t);
        assert!(versions.iter().all(|v| v.event != versions::MERGED));
        assert_eq!(versions.len(), 4);
    }

    #[test]
    fn a_note_that_comes_back_is_written_without_a_flag() {
        for unknown_parent in [false, true] {
            let (t, h) = started(if unknown_parent { "back-b" } else { "back-a" });
            let moved = "ab".repeat(32);
            stage_in(
                &t,
                "personal",
                &[left_record(ID, &moved, &[&h.version], &[])],
            );
            run(&t);
            assert!(!exists(&t, FILE));
            let back = text("coming", "back");
            let (mut record, blob) = incoming(&[&h], FILE, Some(&back));
            let parent = if unknown_parent {
                "ef".repeat(32)
            } else {
                moved.clone()
            };
            record.version.parents = vec![parent.clone()];
            record.version.outside = vec![parent];
            record.version.version =
                versions::version_id(ID, &record.version.parents, FILE, &record.version.blob);
            stage_all(&t, &[(record, blob)]);
            assert!(run(&t).is_empty());
            assert_eq!(read(&t, FILE), back);
            let versions = log(&t);
            assert!(versions.iter().all(|v| v.flags.is_empty()));
            assert!(versions.iter().all(|v| v.event != versions::MERGED));
        }
    }

    #[test]
    fn a_left_gives_way_to_the_full_version_of_the_same_id() {
        let moved_text = scoped(&base(), "shared");
        // Both arrive in one cycle, the left first.
        let (t, h) = started("both-one-cycle");
        let moved = incoming(&[&h], FILE, Some(&moved_text));
        let left = left_record(ID, &moved.0.version.version, &[&h.version], &[]);
        stage_in(&t, "personal", std::slice::from_ref(&left));
        stage_in(&t, "shared", std::slice::from_ref(&moved));
        assert!(run_syncing(&t, &["personal", "shared"]).is_empty());
        assert_eq!(read(&t, FILE), moved_text);
        assert_eq!(log(&t).len(), 2);
        assert_eq!(inbox_len(&t), 0);

        // The left is applied in an earlier cycle: the note is removed, then written back.
        let (t, h) = started("both-two-cycles");
        let moved = incoming(&[&h], FILE, Some(&moved_text));
        stage_in(&t, "personal", &[left]);
        let events = run_syncing(&t, &["personal", "shared"]);
        assert_eq!(events.len(), 1);
        assert!(!exists(&t, FILE));
        stage_in(&t, "shared", std::slice::from_ref(&moved));
        assert!(run_syncing(&t, &["personal", "shared"]).is_empty());
        assert_eq!(read(&t, FILE), moved_text);
        let size = log(&t).len();
        assert!(run_syncing(&t, &["personal", "shared"]).is_empty());
        assert_eq!(log(&t).len(), size);
        assert!(log(&t).iter().all(|v| v.event != versions::MERGED));
    }

    #[test]
    fn the_b2_sequence_on_the_device_that_moved_to_shared_ends_without_a_version() {
        // A moved the note to `shared` (M); B moved it to `work` (C1), merged to C2, and sent left(C2) to `shared`
        // and left(C1) to `personal`.
        let c1 = "c1".repeat(32);
        let c2 = "c2".repeat(32);
        for first_c2 in [false, true] {
            let (t, h) = started(if first_c2 { "b2-a-rev" } else { "b2-a" });
            let moved = local(&t, &[&h], &scoped(&base(), "shared"), true);
            let from_personal = left_record(ID, &c1, &[&h.version], &[]);
            let from_shared = left_record(ID, &c2, &[&c1, &moved.version], &[&c1]);
            let order: [(&str, &(Record, Option<Blob>)); 2] = if first_c2 {
                [("shared", &from_shared), ("personal", &from_personal)]
            } else {
                [("personal", &from_personal), ("shared", &from_shared)]
            };
            for (scope, item) in order {
                stage_in(&t, scope, std::slice::from_ref(item));
                run_syncing(&t, &["personal", "shared"]);
            }
            assert!(!exists(&t, FILE));
            let versions = log(&t);
            assert!(versions.iter().all(|v| v.event != versions::MERGED));
            assert_eq!(versions.len(), 4);
            run_syncing(&t, &["personal", "shared"]);
            assert_eq!(log(&t).len(), 4);
            assert_eq!(inbox_len(&t), 0);
        }
    }

    #[test]
    fn the_b2_sequence_on_the_device_that_moved_to_work_keeps_the_note() {
        let (t, h) = started("b2-b");
        let c1 = local(&t, &[&h], &scoped(&base(), "work"), true);
        let moved_text = scoped(&base(), "shared");
        let moved = incoming(&[&h], FILE, Some(&moved_text));
        stage_in(&t, "shared", std::slice::from_ref(&moved));
        stage_in(
            &t,
            "personal",
            &[left_record(
                ID,
                &moved.0.version.version,
                &[&h.version],
                &[],
            )],
        );
        assert!(run_syncing(&t, &["personal", "shared"]).is_empty());
        let body = read(&t, FILE);
        assert!(body.contains("scope: work"));
        let versions = log(&t);
        let merged = versions.last().unwrap();
        assert_eq!(merged.event, versions::MERGED);
        assert_eq!(merged.flags, ["scope-clash"]);
        assert!(merged.parents.contains(&c1.version));
        assert_eq!(versions.len(), 4);
        assert_eq!(inbox_len(&t), 0);
        run_syncing(&t, &["personal", "shared"]);
        assert_eq!(log(&t).len(), 4);
        assert_eq!(read(&t, FILE), body);
    }

    #[test]
    fn two_synced_scopes_leave_the_merged_note_without_a_scope() {
        let (t, h) = started("clash-two");
        let to_shared = incoming(&[&h], FILE, Some(&scoped(&base(), "shared")));
        let to_team = incoming(&[&h], FILE, Some(&scoped(&base(), "team")));
        stage_in(&t, "shared", &[to_shared]);
        stage_in(&t, "team", &[to_team]);
        assert!(run_syncing(&t, &["shared", "team"]).is_empty());
        assert!(!read(&t, FILE).contains("scope:"));
        let merged = log(&t).pop().unwrap();
        assert_eq!(merged.flags, ["scope-clash"]);
    }

    #[test]
    fn a_version_without_its_text_drops_out_after_the_stale_days() {
        let (t, h) = started("unheld");
        let lost = incoming(&[&h], FILE, Some(&text("lost", "base rollout")));
        let kept_text = text("kept", "base rollout");
        let kept = incoming(&[&h], FILE, Some(&kept_text));
        stage(
            &t.lock,
            "personal",
            &[lost.0.clone(), kept.0.clone()],
            &[kept.1.clone().unwrap()],
            &[],
            now(),
        )
        .unwrap();
        run(&t);
        assert_eq!(read(&t, FILE), base());
        assert_eq!(inbox_len(&t), 2);
        let later = Params {
            syncs: &syncs_personal,
            now: now() + jiff::SignedDuration::from_hours(24 * 181),
            stale_days: 180,
        };
        apply(&t.lock, &later, &mut |_| Ok(()), &mut swap::exchange).unwrap();
        assert_eq!(read(&t, FILE), kept_text);
        assert_eq!(inbox_len(&t), 0);
        let versions = log(&t);
        assert!(versions.iter().any(|v| v.version == lost.0.version.version));
        assert!(versions.iter().all(|v| v.event != versions::MERGED));
        let size = versions.len();
        apply(&t.lock, &later, &mut |_| Ok(()), &mut swap::exchange).unwrap();
        assert_eq!(log(&t).len(), size);
        assert_eq!(read(&t, FILE), kept_text);
    }

    #[test]
    fn a_stale_version_without_its_text_leaves_the_fold_of_its_children() {
        let (t, h) = started("unheld-child");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let lost = incoming(&[&h], FILE, Some(&text("lost", "base rollout")));
        let child_text = text("base setup", "base rollout").replace("# Plan", "# Plan child");
        let child = incoming(&[&lost.0.version], FILE, Some(&child_text));
        stage(
            &t.lock,
            "personal",
            &[lost.0.clone(), child.0.clone()],
            &[child.1.clone().unwrap()],
            &[],
            now(),
        )
        .unwrap();
        let later = Params {
            syncs: &syncs_personal,
            now: now() + jiff::SignedDuration::from_hours(24 * 181),
            stale_days: 180,
        };
        apply(&t.lock, &later, &mut |_| Ok(()), &mut swap::exchange).unwrap();
        let body = read(&t, FILE);
        assert!(body.contains("# Plan child") && body.contains("my rollout"));
        let merged = log(&t).pop().unwrap();
        assert!(
            merged.parents.contains(&mine.version)
                && merged.parents.contains(&child.0.version.version)
        );
    }

    #[test]
    fn a_merge_enters_the_summary_and_a_resolution_leaves_its_drops() {
        let (t, h) = started("summary");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("base setup", "their rollout")),
            )],
        );
        run(&t);
        let entry = conflicts::read(&t.root).unwrap().notes.remove(ID).unwrap();
        assert_eq!(entry.file, FILE);
        assert_eq!(entry.conflict.len(), 1);
        let conflict = entry.conflict[0].version.clone();

        // An edit that keeps the block keeps the conflict open.
        let p = params(&syncs_personal);
        let kept = format!("{}\nnote\n", read(&t, FILE));
        write(&t, FILE, &kept);
        record_local(&t.lock, &p, ID, FILE, Some(kept.as_bytes()), AT).unwrap();
        assert_eq!(
            conflicts::read(&t.root).unwrap().notes[ID].conflict.len(),
            1
        );

        // A resolution that drops a line moves the entry to the dropped text.
        let resolved = text("base setup", "my rollout");
        write(&t, FILE, &resolved);
        record_local(&t.lock, &p, ID, FILE, Some(resolved.as_bytes()), AT).unwrap();
        let entry = conflicts::read(&t.root).unwrap().notes.remove(ID).unwrap();
        assert!(entry.conflict.is_empty());
        assert_eq!(entry.dropped.len(), 1);
        assert_eq!(entry.dropped[0].lines, ["their rollout"]);
        let save = log(&t).pop().unwrap();
        assert_eq!(save.dropped.len(), 1);
        assert_eq!(save.dropped[0].lines, ["their rollout"]);

        // A declaration covers it, and the note leaves the summary.
        let declaration = Declaration {
            declare: conflict,
            reason: "intended".into(),
            at: AT.into(),
            device: Some("bagend".into()),
        };
        stage(&t.lock, "personal", &[], &[], &[declaration], now()).unwrap();
        run(&t);
        assert!(conflicts::read(&t.root).unwrap().notes.is_empty());
    }

    #[test]
    fn a_merge_without_a_conflict_is_a_notice_and_a_clean_note_has_no_entry() {
        let (t, h) = started("notice");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        run(&t);
        let entry = conflicts::read(&t.root).unwrap().notes.remove(ID).unwrap();
        assert!(entry.merged && entry.conflict.is_empty());

        let (t, h) = started("edit-beat-delete-notice");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(&t, &[incoming(&[&h], FILE, None)]);
        run(&t);
        let entry = conflicts::read(&t.root).unwrap().notes.remove(ID).unwrap();
        assert_eq!(entry.notices[0].flag, "edit-beat-delete");

        let (t, h) = started("clean");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        run(&t);
        assert!(conflicts::read(&t.root).unwrap().notes.is_empty());
    }

    #[test]
    fn a_move_out_of_a_syncing_scope_is_kept_for_30_days() {
        let (t, _h) = started("moved-out");
        let p = params(&syncs_personal);
        let moved = scoped(&base(), "work");
        write(&t, FILE, &moved);
        record_local(&t.lock, &p, ID, FILE, Some(moved.as_bytes()), AT).unwrap();
        let entry = conflicts::read(&t.root).unwrap().notes.remove(ID).unwrap();
        assert_eq!(
            entry.left,
            [Left {
                scope: "personal".into(),
                at: AT.into()
            }]
        );
        // A scope this device does not sync is no move out of a syncing one.
        let (t, _h) = started("moved-out-local");
        let syncs = |scope: &str| scope == "shared";
        let p = params(&syncs);
        write(&t, FILE, &moved);
        record_local(&t.lock, &p, ID, FILE, Some(moved.as_bytes()), AT).unwrap();
        assert!(conflicts::read(&t.root).unwrap().notes.is_empty());
        // After 30 days a refresh drops it.
        let (t, _h) = started("moved-out-expiry");
        let p = params(&syncs_personal);
        write(&t, FILE, &moved);
        record_local(&t.lock, &p, ID, FILE, Some(moved.as_bytes()), AT).unwrap();
        let later = Params {
            syncs: &syncs_personal,
            now: now() + jiff::SignedDuration::from_hours(24 * 31),
            stale_days: 180,
        };
        refresh_open(&t.lock, &later, &[ID.to_string()]).unwrap();
        assert!(conflicts::read(&t.root).unwrap().notes.is_empty());
    }

    #[test]
    fn a_summary_that_does_not_parse_is_rebuilt_from_the_logs() {
        let (t, h) = started("rebuild");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("base setup", "their rollout")),
            )],
        );
        run(&t);
        let want = conflicts::read(&t.root).unwrap();
        fs::write(conflicts::path(&t.root), "not json").unwrap();
        let p = params(&syncs_personal);
        rebuild_open(&t.lock, &p).unwrap();
        assert_eq!(conflicts::read(&t.root).unwrap(), want);
    }

    fn sweep(t: &T) -> Vec<String> {
        versions::sweep_restore_leftovers(&t.lock, AT, &staged(&t.root).unwrap()).unwrap()
    }

    /// RR1: a stale save is held back; the write is killed at Inspect. The next cycle (apply, sweep, scan) must not
    /// turn the agent's save into a child of the written version.
    #[test]
    fn crash_at_inspect_during_a_stale_merge() {
        let (t, h) = started("rr-stale-inspect");
        let w0 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&w0));
        run(&t);
        let p = params(&syncs_personal);
        let s1 = text("base setup", "agent rollout");
        write(&t, FILE, &s1);
        let killed = record_local_with(
            &t.lock,
            &p,
            &mut kill_at(Step::Inspect),
            &mut swap::exchange,
            ID,
            FILE,
            Some(s1.as_bytes()),
            AT,
        );
        assert!(killed.is_err());
        assert!(hidden(&t));
        let merged_bytes = read(&t, FILE);
        run(&t);
        sweep(&t);
        let _events =
            record_local(&t.lock, &p, ID, FILE, Some(merged_bytes.as_bytes()), AT).unwrap();
        let t2 = incoming(
            &[&w0.0.version],
            FILE,
            Some(&text("their setup", "base rollout").replace("# Plan", "# Plan v2")),
        );
        stage_all(&t, &[t2]);
        run(&t);
        let body = read(&t, FILE);
        assert!(body.contains("their setup"), "W0's edit survives");
        assert!(body.contains("agent rollout"), "the agent's edit survives");
        assert!(body.contains("# Plan v2"));
    }

    /// RR2: the same, killed at Record (hidden file already removed).
    #[test]
    fn crash_at_record_during_a_stale_merge() {
        let (t, h) = started("rr-stale-record");
        let w0 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&w0));
        run(&t);
        let p = params(&syncs_personal);
        let s1 = text("base setup", "agent rollout");
        write(&t, FILE, &s1);
        let killed = record_local_with(
            &t.lock,
            &p,
            &mut kill_at(Step::Record),
            &mut swap::exchange,
            ID,
            FILE,
            Some(s1.as_bytes()),
            AT,
        );
        assert!(killed.is_err());
        assert!(!hidden(&t));
        let merged_bytes = read(&t, FILE);
        run(&t);
        sweep(&t);
        let _events =
            record_local(&t.lock, &p, ID, FILE, Some(merged_bytes.as_bytes()), AT).unwrap();
        let versions = log(&t);
        assert!(
            versions
                .iter()
                .any(|v| v.blob == hash::sha256_hex(s1.as_bytes())),
            "the agent's save is in history"
        );
        assert!(
            versions
                .iter()
                .any(|v| v.flags.iter().any(|f| f == "stale-base")),
            "the merge is flagged"
        );
    }

    /// RR3: a second save lands between the exchange and the put-back of a raced swap.
    #[test]
    fn second_save_between_the_exchange_and_the_put_back() {
        let (t, h) = started("rr-double");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        let path = t.root.join("notes").join(FILE);
        let p = params(&syncs_personal);
        let n = Cell::new(0);
        let s2 = text("base setup", "agent rollout 2");
        apply(
            &t.lock,
            &p,
            &mut |step| {
                match step {
                    Step::Swap if n.get() == 0 => {
                        n.set(1);
                        fs::write(&path, text("base setup", "agent rollout 1")).unwrap();
                    }
                    Step::Inspect if n.get() == 1 => {
                        n.set(2);
                        fs::write(&path, &s2).unwrap();
                    }
                    _ => {}
                }
                Ok(())
            },
            &mut swap::exchange,
        )
        .unwrap();
        let body = read(&t, FILE);
        assert!(!hidden(&t));
        assert!(
            log(&t)
                .iter()
                .any(|v| v.blob == hash::sha256_hex(s2.as_bytes())),
            "the second save is in history"
        );
        assert!(
            body.contains("agent rollout 2"),
            "the agent's last save is in the file"
        );
    }

    /// RR4: the written version is a group of two (same text, different parents); a fresh save must follow both.
    #[test]
    fn a_save_after_a_grouped_written_version_follows_every_member() {
        let (t, h) = started("rr-group");
        let z = text("shared setup", "base rollout");
        let middle = incoming(&[&h], FILE, Some(&text("middle", "base rollout")));
        let x = incoming(&[&h], FILE, Some(&z));
        let y = incoming(&[&middle.0.version], FILE, Some(&z));
        stage_all(&t, &[middle, x, y]);
        run(&t);
        assert_eq!(read(&t, FILE), z);
        let before = log(&t);
        let heads = versions::heads(&before);
        assert_eq!((heads.len(), heads[0].len()), (1, 2));
        let p = params(&syncs_personal);
        let fresh = text("shared setup", "agent rollout");
        write(&t, FILE, &fresh);
        record_local(&t.lock, &p, ID, FILE, Some(fresh.as_bytes()), AT).unwrap();
        let after = log(&t);
        let heads = versions::heads(&after);
        assert_eq!(
            heads.len(),
            1,
            "one head after a save that follows the group"
        );
        assert_eq!(log(&t).last().unwrap().parents.len(), 2);
    }

    /// RR5: killed at Swap with a merge in the hidden file; a new version changes the plan before the next cycle.
    #[test]
    fn a_leftover_merge_is_foreign_once_the_plan_changes() {
        let (t, h) = started("rr-plan-changes");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        let t1 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&t1));
        let p = params(&syncs_personal);
        let killed = apply(&t.lock, &p, &mut kill_at(Step::Swap), &mut swap::exchange);
        assert!(killed.is_err() && hidden(&t));
        let t2 = incoming(
            &[&t1.0.version],
            FILE,
            Some(&text("their setup", "base rollout").replace("# Plan", "# Plan v2")),
        );
        stage_all(&t, &[t2]);
        run(&t);
        let swept = sweep(&t);
        run(&t);
        assert!(
            swept.is_empty(),
            "nothing recorded from the leftover: {swept:?}"
        );
        assert_eq!(log(&t).len(), 5, "H, mine, T1, T2, one merge");
    }

    /// RR6: killed at Rename; the target name is taken at the next cycle, then freed.
    #[test]
    fn a_taken_name_during_the_rename_finish_keeps_the_written_record() {
        let (t, h) = started("rr-rename-taken");
        let new = text("their setup", "base rollout");
        let theirs = incoming(&[&h], "decision-release.md", Some(&new));
        stage_all(&t, std::slice::from_ref(&theirs));
        let p = params(&syncs_personal);
        let killed = apply(&t.lock, &p, &mut kill_at(Step::Rename), &mut swap::exchange);
        assert!(killed.is_err());
        write(&t, "decision-release.md", "not a note\n");
        run(&t);
        sweep(&t);
        fs::remove_file(t.root.join("notes/decision-release.md")).unwrap();
        run(&t);
        assert!(exists(&t, "decision-release.md") && !exists(&t, FILE));
        let ids: Vec<String> = log(&t).into_iter().map(|v| v.version).collect();
        assert_eq!(
            ids,
            [h.version, theirs.0.version.version],
            "no extra version"
        );
    }

    /// RR7: a local delete while a version waits in the inbox: the edit wins once, then the delete goes through.
    #[test]
    fn a_local_delete_against_a_waiting_version_loses_once() {
        let mut failures = Vec::new();
        for n in 0..6 {
            let (t, h) = started(&format!("rr-del-wait-{n}"));
            let setup = format!("their setup {n}");
            let w0 = incoming(&[&h], FILE, Some(&text(&setup, "base rollout")));
            stage_all(&t, std::slice::from_ref(&w0));
            run(&t);
            let t1 = incoming(
                &[&w0.0.version],
                FILE,
                Some(&text(&setup, &format!("their rollout {n}"))),
            );
            stage_all(&t, std::slice::from_ref(&t1));
            let p = params(&syncs_personal);
            apply(&t.lock, &p, &mut |_| Ok(()), &mut |_, _| Err("no".into())).unwrap();
            assert_eq!(inbox_len(&t), 1);
            fs::remove_file(t.root.join("notes").join(FILE)).unwrap();
            record_local(&t.lock, &p, ID, FILE, None, AT).unwrap();
            assert!(exists(&t, FILE), "edit beats delete once");
            stale_base(&t.root, ID).unwrap();
            fs::remove_file(t.root.join("notes").join(FILE)).unwrap();
            record_local(&t.lock, &p, ID, FILE, None, AT).unwrap();
            if exists(&t, FILE) {
                failures.push(n);
            }
        }
        assert!(
            failures.is_empty(),
            "the file came back a second time in variants {failures:?}"
        );
    }

    /// RR8: a merge write killed at Inspect, then the plan changes: the file's bytes are this device's write.
    #[test]
    fn a_crashed_merge_write_is_known_once_the_plan_changes() {
        let (t, h) = started("rr-written-known");
        local(&t, &[&h], &text("base setup", "my rollout"), true);
        let t1 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&t1));
        let p = params(&syncs_personal);
        let killed = apply(
            &t.lock,
            &p,
            &mut kill_at(Step::Inspect),
            &mut swap::exchange,
        );
        assert!(killed.is_err());
        let m = read(&t, FILE);
        assert!(m.contains("their setup") && m.contains("my rollout"));
        let t2 = incoming(
            &[&t1.0.version],
            FILE,
            Some(&text("their setup", "base rollout").replace("# Plan", "# Plan v2")),
        );
        stage_all(&t, &[t2]);
        run(&t);
        sweep(&t);
        let body = read(&t, FILE);
        assert!(
            body.contains("their setup")
                && body.contains("my rollout")
                && body.contains("# Plan v2"),
            "{body}"
        );
        let versions = log(&t);
        assert!(
            !versions
                .iter()
                .any(|v| v.event == "edited" && v.blob == hash::sha256_hex(m.as_bytes())),
            "the crashed merge's bytes are not recorded as a local edit"
        );
    }

    /// RR9 (M-a's gap): a save with no stale-base entry, after a concurrent `left` whose id sorts first.
    #[test]
    fn a_save_without_an_entry_after_a_concurrent_left_follows_the_local_head() {
        let (t, h) = started("rr-left-noentry");
        let mine = local(&t, &[&h], &text("base setup", "my rollout"), true);
        let mut left = incoming(&[&h], "", None);
        left.0.version.event = versions::LEFT.to_string();
        left.0.version.version = "00".repeat(32);
        stage_all(&t, &[left]);
        run(&t);
        let _ = fs::remove_file(store::sync_dir(&t.root).join(STALE_BASE));
        let p = params(&syncs_personal);
        let s = text("base setup", "my rollout 2");
        write(&t, FILE, &s);
        record_local(&t.lock, &p, ID, FILE, Some(s.as_bytes()), AT).unwrap();
        let last = log(&t).pop().unwrap();
        assert_eq!(last.parents, [mine.version]);
    }

    #[test]
    fn apply_alone_finishes_a_killed_stale_merge() {
        let (t, h) = started("apply-finishes");
        let w0 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
        stage_all(&t, std::slice::from_ref(&w0));
        run(&t);
        let p = params(&syncs_personal);
        let saved = text("base setup", "agent rollout");
        write(&t, FILE, &saved);
        let killed = record_local_with(
            &t.lock,
            &p,
            &mut kill_at(Step::Inspect),
            &mut swap::exchange,
            ID,
            FILE,
            Some(saved.as_bytes()),
            AT,
        );
        assert!(killed.is_err());
        assert!(run(&t).is_empty());
        assert!(!hidden(&t));
        let versions = log(&t);
        let save = versions
            .iter()
            .find(|v| v.blob == hash::sha256_hex(saved.as_bytes()))
            .unwrap();
        assert_eq!(save.parents, std::slice::from_ref(&h.version));
        assert_eq!(versions.last().unwrap().flags, ["stale-base"]);
        assert!(!store::sync_dir(&t.root).join(WRITTEN).exists());
        assert_eq!(read(&t, FILE), text("their setup", "agent rollout"));
    }

    #[test]
    fn the_final_merge_of_a_fold_that_holds_a_stale_save_is_flagged() {
        for n in 0..6 {
            let (t, h) = started(&format!("flag-carry-{n}"));
            let w0 = incoming(&[&h], FILE, Some(&text("their setup", "base rollout")));
            stage_all(&t, std::slice::from_ref(&w0));
            run(&t);
            let one = incoming(
                &[&w0.0.version],
                FILE,
                Some(
                    &text("their setup", "base rollout").replace("# Plan", &format!("# Plan {n}")),
                ),
            );
            let two = incoming(
                &[&w0.0.version],
                FILE,
                Some(&text("their setup", &format!("rollout {n}"))),
            );
            // The agent saves over text it never read while both wait: the write is refused, so they stay staged.
            let p = params(&syncs_personal);
            stage_all(&t, &[one, two]);
            let saved = text("base setup", "agent rollout");
            write(&t, FILE, &saved);
            record_local_with(
                &t.lock,
                &p,
                &mut |_| Ok(()),
                &mut swap::exchange,
                ID,
                FILE,
                Some(saved.as_bytes()),
                AT,
            )
            .unwrap();
            apply(&t.lock, &p, &mut |_| Ok(()), &mut swap::exchange).unwrap();
            let versions = log(&t);
            let heads = versions::heads(&versions);
            assert_eq!(heads.len(), 1);
            assert!(
                heads[0][0].flags.iter().any(|f| f == "stale-base"),
                "variant {n}"
            );
        }
    }

    #[test]
    fn the_entry_base_is_the_version_the_file_held_even_after_a_kill_past_the_swap() {
        for n in 0..8 {
            let (t, h) = started(&format!("base-kill-{n}"));
            let mine = local(
                &t,
                &[&h],
                &text("base setup", &format!("my rollout {n}")),
                true,
            );
            local(
                &t,
                &[&h],
                &text("base setup", "base rollout").replace("# Plan", &format!("# Plan {n}")),
                false,
            );
            let theirs = incoming(
                &[&h],
                FILE,
                Some(&text(&format!("their setup {n}"), "base rollout")),
            );
            stage_all(&t, std::slice::from_ref(&theirs));
            let p = params(&syncs_personal);
            let killed = apply(
                &t.lock,
                &p,
                &mut kill_at(Step::Inspect),
                &mut swap::exchange,
            );
            assert!(killed.is_err());
            run(&t);
            let entry = stale_base(&t.root, ID).unwrap().unwrap();
            assert_eq!(entry.base, mine.version, "variant {n}");
        }
    }

    #[test]
    fn a_swap_that_races_every_time_ends_with_one_line_and_every_save_recorded() {
        let (t, h) = started("races-every-time");
        stage_all(
            &t,
            &[incoming(
                &[&h],
                FILE,
                Some(&text("their setup", "base rollout")),
            )],
        );
        let path = t.root.join("notes").join(FILE);
        let n = Cell::new(0);
        let p = params(&syncs_personal);
        let events = apply(
            &t.lock,
            &p,
            &mut |step| {
                if step == Step::Swap {
                    n.set(n.get() + 1);
                    fs::write(
                        &path,
                        text("base setup", &format!("agent rollout {}", n.get())),
                    )
                    .unwrap();
                }
                Ok(())
            },
            &mut swap::exchange,
        )
        .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.ends_with("not written: the file kept changing"))
                .count(),
            1,
            "{events:?}"
        );
        assert!(!hidden(&t));
        let body = read(&t, FILE);
        for k in 1..=n.get() {
            let saved = text("base setup", &format!("agent rollout {k}"));
            assert!(
                body == saved
                    || log(&t)
                        .iter()
                        .any(|v| v.blob == hash::sha256_hex(saved.as_bytes())),
                "save {k} is neither in the file nor in history"
            );
        }
    }

    #[test]
    fn a_device_that_receives_a_merged_conflict_prints_the_conflict_line() {
        let (t, h) = started("received-conflict");
        let body = text(
            "base setup",
            "<<<<<<< bilbo x\nmine\n======= bilbo y\ntheirs\n>>>>>>> bilbo\n",
        );
        let (mut record, blob) = incoming(&[&h], FILE, Some(&body));
        record.version.event = versions::MERGED.to_string();
        record.version.conflict = vec![Conflict {
            passage: "Rollout".into(),
            sides: vec!["x".into(), "y".into()],
        }];
        stage_all(&t, &[(record, blob)]);
        let events = run(&t);
        assert_eq!(
            events,
            ["notes/plan-release.md: conflict in 1 passage; run bilbo check"]
        );
        assert_eq!(read(&t, FILE), body);
        assert_eq!(rows(&t).len(), 2);
        assert!(run(&t).is_empty());
    }

    #[test]
    fn a_file_equal_to_a_later_head_group_records_nothing() {
        for n in 0..6 {
            let (t, h) = started(&format!("held-group-{n}"));
            let one = local(&t, &[&h], &text(&format!("one {n}"), "base rollout"), false);
            let two = local(&t, &[&h], &text("base setup", &format!("two {n}")), false);
            let view = log(&t);
            let blob = |v: &Version| v.blob.clone();
            for head in [&one, &two] {
                let held = held_group(&view, FILE, &blob(head)).unwrap();
                assert_eq!(held, std::slice::from_ref(&head.version));
            }
            assert!(held_group(&view, FILE, &hash::sha256_hex(b"other")).is_none());
            for head in [&one, &two] {
                let bytes = versions::content(&t.root, head).unwrap();
                write(&t, FILE, &String::from_utf8(bytes.clone()).unwrap());
                let p = params(&syncs_personal);
                record_local(&t.lock, &p, ID, FILE, Some(&bytes), AT).unwrap();
                assert_eq!(log(&t).len(), 3, "variant {n}");
            }
        }
    }
}
