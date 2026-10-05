//! Applying the versions other devices wrote: the inbox, heads, merges, inbound writes, the stale-base rule, deletes,
//! topic collisions and scope moves.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::host::swap;
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

/// What this device put into a note's file and has not recorded yet: the file name and the blob. It is written before
/// each swap and cleared when the write is recorded, so a kill in between never turns the bytes into a save.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Written {
    file: String,
    blob: String,
}

/// The stale-base entry of a note, if a sync write left one.
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

/// The note ids and blob hashes of the versions waiting in the inbox, for `versions::sweep_restore_leftovers`.
pub fn staged(root: &Path) -> Result<BTreeSet<(String, String)>, String> {
    Ok(read_inbox(root)?
        .into_iter()
        .filter_map(|entry| entry.record)
        .filter(|r| !r.version.is_deleted())
        .map(|r| (r.note, r.version.blob))
        .collect())
}

/// The blobs of the versions waiting in the inbox, for `versions::prune`: no log names them yet.
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
    let mut taken: HashSet<String> = read_inbox(root)?
        .into_iter()
        .filter_map(|entry| entry.record)
        .map(|r| r.version.version)
        .collect();
    let mut logs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut entries = Vec::new();
    for record in records {
        let v = &record.version;
        let in_log = match logs.get(&record.note) {
            Some(ids) => ids,
            None => {
                let ids = versions::load(root, &record.note)?
                    .versions
                    .into_iter()
                    .map(|v| v.version)
                    .collect();
                logs.entry(record.note.clone()).or_insert(ids)
            }
        };
        if taken.contains(&v.version) || in_log.contains(&v.version) {
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
        taken.insert(v.version.clone());
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
    if previous.conflict.is_empty() {
        return Ok(Vec::new());
    }
    let before = match versions::content(root, previous) {
        Ok(before) => before,
        Err(ContentError::Pruned | ContentError::Deleted) => return Ok(Vec::new()),
        Err(ContentError::Io(e)) => return Err(e),
    };
    let after = String::from_utf8_lossy(bytes);
    let kept = merge::blocks(&after);
    let resolved: Vec<merge::Block> = merge::blocks(&String::from_utf8_lossy(&before))
        .into_iter()
        .filter(|block| previous.conflict.iter().any(|c| c.passage == block.passage))
        .filter(|block| {
            !kept.iter().any(|k| {
                k.passage == block.passage
                    && k.sides
                        .iter()
                        .map(|s| &s.version)
                        .eq(block.sides.iter().map(|s| &s.version))
            })
        })
        .collect();
    Ok(merge::dropped(&resolved, &after))
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

/// What one round of a note's settle planned, and what it found.
struct Round {
    plan: Plan,
    applicable: Vec<Version>,
    in_log: HashSet<String>,
    cur: Cur,
    scope: String,
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
}

fn digest(bytes: &[u8]) -> String {
    hash::sha256_hex(bytes)
}

impl Engine<'_> {
    fn root(&self) -> &Path {
        self.lock.root()
    }

    fn current(&self, id: &str) -> Result<Cur, String> {
        let notes = self.root().join("notes");
        let scan =
            versions::scan(&notes).map_err(|e| format!("cannot read {}: {e}", notes.display()))?;
        if let Some(found) = scan.notes.get(id) {
            return Ok(Cur::File(found.clone()));
        }
        if scan.skipped.iter().any(|s| s.id.as_deref() == Some(id)) {
            return Ok(Cur::Blocked);
        }
        Ok(Cur::Absent)
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

    fn clear_written(&self, id: &str) -> Result<(), String> {
        let mut written: BTreeMap<String, Written> = read_map(self.root(), WRITTEN)?;
        if written.remove(id).is_some() {
            write_map(self.root(), WRITTEN, &written)?;
        }
        Ok(())
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
            version.dropped = dropped_after(self.root(), rep, bytes)?;
        }
        versions::append(self.lock, id, &version)?;
        self.seen = sum.map(|sum| (file.to_string(), sum));
        Ok(Some(version))
    }

    /// A save of `bytes` that follows `parent`, not yet appended.
    fn build_save(
        &self,
        id: &str,
        parent: &Version,
        file: &str,
        bytes: &[u8],
        sum: &str,
    ) -> Result<Version, String> {
        let event =
            versions::event_for(Some(parent), Some((file, sum))).unwrap_or(versions::EDITED);
        let mut version = self.make(
            id,
            std::slice::from_ref(&parent.version),
            file,
            Some(bytes),
            event,
        )?;
        version.dropped = dropped_after(self.root(), parent, bytes)?;
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
        let mut view = versions::load(self.root(), id)?.versions;
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
            let save = self.build_save(id, &written, file, bytes, &sum)?;
            versions::append(self.lock, id, &save)?;
            self.drop_entry(id)?;
            self.seen = Some((file.to_string(), sum));
            return Ok(Pending::default());
        }
        let save = self.build_save(id, &parent, file, bytes, &sum)?;
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
        let deleted = self.make(
            id,
            std::slice::from_ref(&base.version),
            &base.file,
            None,
            versions::DELETED,
        )?;
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
        flagged: &HashSet<String>,
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
        work.push(merged.clone());
        plan.merges.push(merged.clone());
        Ok(Head {
            members: vec![merged],
            bytes,
        })
    }

    /// Plans a note's heads: merged two at a time in id order, or removed when every head is a `left`. `None` when a
    /// head's text is not held.
    fn plan(
        &self,
        id: &str,
        work: &mut Vec<Version>,
        flagged: &HashSet<String>,
    ) -> Result<Option<Plan>, String> {
        let groups: Vec<Vec<Version>> = versions::heads(work)
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
            acc = self.join(id, acc, next, work, &mut plan, flagged)?;
        }
        plan.file = acc.rep().file.clone();
        plan.final_id = acc.rep().version.clone();
        plan.conflicts = plan
            .merges
            .iter()
            .find(|m| m.version == plan.final_id)
            .map_or(0, |m| m.conflict.len());
        plan.content = acc.bytes;
        Ok(Some(plan))
    }

    /// The staged versions of `id` that wait for the log, with the scope each arrived through. A version whose
    /// text does not say that scope is skipped with a line and counts as done; one whose text is not held waits.
    fn waiting(
        &mut self,
        id: &str,
        in_log: &HashSet<String>,
        done: &mut HashSet<String>,
    ) -> Result<(Vec<(Version, jiff::Timestamp)>, String), String> {
        let mut waiting = Vec::new();
        let mut scope = String::new();
        for entry in read_inbox(self.root())? {
            let Some(record) = entry.record.filter(|r| r.note == id) else {
                continue;
            };
            let v = record.version;
            if done.contains(&v.version) {
                continue;
            }
            if in_log.contains(&v.version) {
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
            scope = entry.scope;
            waiting.push((v, seen));
        }
        Ok((waiting, scope))
    }

    /// Ends a note's settle without a commit: what this call set up before the swap goes.
    fn leave(&self, id: &str, created: bool) -> Result<(), String> {
        self.clear_written(id)?;
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
        for round in 0..=MAX_ROUNDS {
            (self.hook)(Step::Prepare)?;
            let log = versions::load(self.root(), id)?;
            let in_log: HashSet<String> = log.versions.iter().map(|v| v.version.clone()).collect();
            pending.extras.retain(|v| !in_log.contains(&v.version));
            let (waiting, scope) = self.waiting(id, &in_log, done)?;
            let applicable = self.applicable(&log.versions, &waiting);
            let mut view = log.versions;
            view.extend(pending.extras.iter().cloned());
            let mut work = view.clone();
            work.extend(applicable.iter().cloned());
            let Some(plan) = self.plan(id, &mut work, &pending.flagged)? else {
                return self.leave(id, created);
            };
            let cur = self.current(id)?;
            if matches!(cur, Cur::Blocked) {
                return self.leave(id, created);
            }
            let leftover = versions::restore_path(self.root(), id);
            if leftover.exists()
                && !self.clear_leftover(&leftover, &view, &applicable, &pending.extras, &plan)?
            {
                return self.leave(id, created);
            }
            let blob = plan.content.as_deref().map(digest);
            let mut r = Round {
                plan,
                applicable,
                in_log,
                cur,
                scope,
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
                && read_map::<Written>(self.root(), WRITTEN)?.get(id)
                    == Some(&Written {
                        file: r.plan.file.clone(),
                        blob: b.clone(),
                    })
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

    /// Removes the hidden file an interrupted write left when its bytes are a version of the note, staged, planned
    /// or a local save held back. False when it holds anything else: another writer owns it.
    fn clear_leftover(
        &mut self,
        path: &Path,
        view: &[Version],
        applicable: &[Version],
        extras: &[Version],
        plan: &Plan,
    ) -> Result<bool, String> {
        let sum = digest(&fs::read(path).map_err(|e| io_message("read", path, &e))?);
        let ours = view
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
                let mut written: BTreeMap<String, Written> = read_map(self.root(), WRITTEN)?;
                written.insert(
                    id.to_string(),
                    Written {
                        file: plan.file.clone(),
                        blob: digest(bytes),
                    },
                );
                write_map(self.root(), WRITTEN, &written)?;
                (self.hook)(Step::Swap)?;
                if let Err(e) = (self.exchange)(&old, &temp) {
                    let _ = fs::remove_file(&temp);
                    return Ok(self.refuse(e, scope, &found.name));
                }
                (self.hook)(Step::Inspect)?;
                let out = fs::read(&temp).map_err(|e| io_message("read", &temp, &e))?;
                if out != found.bytes {
                    // A save landed during the swap: put the file back as the agent left it.
                    (self.exchange)(&old, &temp)
                        .map_err(|e| format!("cannot put back notes/{}: {e}", found.name))?;
                    let _ = fs::remove_file(&temp);
                    return Ok(Wrote::Raced);
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
            versions::append(self.lock, id, v)?;
            done.insert(v.version.clone());
        }
        for v in pending
            .extras
            .iter()
            .filter(|v| !r.in_log.contains(&v.version))
        {
            versions::append(self.lock, id, v)?;
        }
        for merged in &r.plan.merges {
            if let Some(bytes) = r.plan.bytes.get(&merged.version) {
                versions::write_blob(self.lock, bytes)?;
            }
            versions::append(self.lock, id, merged)?;
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
            None => {
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
            self.events.push(format!(
                "sync {}: notes/{} left the scope; its history stays",
                r.scope, f.name
            ));
        }
        Ok(())
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
    };
    let inbox = read_inbox(root)?;
    let notes: BTreeSet<String> = inbox
        .iter()
        .filter_map(|e| e.record.as_ref())
        .map(|r| r.note.clone())
        .collect();
    let mut done = HashSet::new();
    for id in notes {
        engine.seen = None;
        engine.settle(&id, Pending::default(), &mut done)?;
    }
    engine.declare(&inbox, &mut done)?;
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
    };
    let pending = engine.local(note, file, bytes, &[])?;
    if !pending.extras.is_empty() {
        engine.settle(note, pending, &mut HashSet::new())?;
    }
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
        let parents: Vec<String> = parents.iter().map(|v| v.version.clone()).collect();
        let blob = text.map_or_else(
            || versions::DELETED.to_string(),
            |t| hash::sha256_hex(t.as_bytes()),
        );
        let version = Version {
            version: versions::version_id(ID, &parents, file, &blob),
            parents,
            file: file.to_string(),
            blob,
            event: if text.is_some() { "edited" } else { "deleted" }.to_string(),
            at: AT.to_string(),
            device: Some("bagend".into()),
            ..Version::default()
        };
        let record = Record {
            note: ID.to_string(),
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
        let other = text("x", "y").replace(ID, "01JBBBBBBBBBBBBBBBBBBBBBBB");
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
}
