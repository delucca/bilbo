//! One scope's replica: its sync state, its pull and push over the transport, acknowledgements, the outbox and the
//! known heads that pruning keeps.
//!
//! ```text
//! <root>/.bilbo/scopes/<scope id>/state.json   cursors, acks, the outbox's bookkeeping, the stopped line
//! <root>/.bilbo/scopes/<scope id>/seen.jsonl   one line per version, blob and declaration pushed or applied
//! <root>/.bilbo/scopes/<scope id>/out/<seq>.seg  this device's segments until every live device acknowledged them
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::identity::keys::Identity;
use crate::identity::manifest::{self, Opened, Recipient};
use crate::note::versions::{self, Declaration, Guard, Hold, Version};
use crate::note::{self, ScopeKey, marks};
use crate::shared::{config, frontmatter, hash, store};
use crate::sync::segment::{self, Blob, Plaintext, Record, Refusal};
use crate::sync::transport::{self, Put, Transport};

const HOUR: i64 = 60 * 60;
const DAY: i64 = 24 * HOUR;
/// How long a device folder with nothing new waits before a poll lists it, besides probing past the cursor.
const LIST_EVERY: i64 = 10 * 60;
/// The most segment bytes one pull reads, so a device that joins late applies a long history over several polls.
const PULL_BYTES: usize = 64 * 1024 * 1024;
const TMP_PREFIX: &str = ".tmp-";
/// Quiet polls a store that resumed from the transport waits before its first push.
const SETTLE_POLLS: u8 = 2;

/// The own segment `seq` as the outbox holds it, with what the staleness rule needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sent {
    /// Unix seconds by this device's clock when the segment was created.
    pub at: i64,
    /// Whether it holds versions; an acknowledgement only segment never makes a device stale.
    pub versions: bool,
}

/// Where reading a device stopped and why, until a pull reads past it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    pub seq: u64,
    /// The reason alone: `missing`, a failed read, or why the segment was refused.
    pub why: String,
    /// Whether the segment's writer can replace it: the transport does not keep what it stores.
    pub replaceable: bool,
}

/// A transport problem the watcher saw, for `bilbo sync` to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    /// `unreachable` or `full`.
    pub kind: String,
    /// Unix seconds when it began.
    pub since: i64,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    name: String,
    device: String,
    /// The highest seq of this device's own folder that this store wrote or read.
    own: u64,
    /// The highest seq applied from each device.
    cursors: BTreeMap<String, u64>,
    /// What each device acknowledged: acker, then target device, then the highest seq of the target it applied.
    acks: BTreeMap<String, BTreeMap<String, u64>>,
    /// Own segments not yet acknowledged by every listed device.
    sent: BTreeMap<u64, Sent>,
    /// Whether a segment holding versions was applied since this device's last segment.
    owed: bool,
    /// Unix seconds of the last acknowledgement only segment.
    last_ack: Option<i64>,
    /// The devices the latest confirmed version listed at the last pull.
    listed: BTreeSet<String>,
    /// The highest seq read from a device the latest confirmed version dropped, fixed when it was dropped.
    cutoffs: BTreeMap<String, u64>,
    /// Notes whose mark line was printed.
    marked: BTreeSet<String>,
    /// Unix seconds when each device was first seen listed.
    since: BTreeMap<String, i64>,
    /// Unix seconds of the last committed pull and of the last segment created.
    pulled_at: Option<i64>,
    pushed_at: Option<i64>,
    /// Where reading each stuck device stopped.
    stops: BTreeMap<String, Stop>,
    /// The transport problem watch last reported.
    error: Option<Problem>,
    /// Why the scope does not sync, as watch last decided it.
    stopped: Option<String>,
    /// Why this device stopped pushing: another store writes as it.
    halted: Option<String>,
}

fn scope_dir(root: &Path, id: &str) -> PathBuf {
    store::scopes_dir(root).join(id)
}

fn state_path(root: &Path, id: &str) -> PathBuf {
    scope_dir(root, id).join("state.json")
}

fn seen_path(root: &Path, id: &str) -> PathBuf {
    scope_dir(root, id).join("seen.jsonl")
}

fn out_dir(root: &Path, id: &str) -> PathBuf {
    scope_dir(root, id).join("out")
}

fn out_path(root: &Path, id: &str, seq: u64) -> PathBuf {
    out_dir(root, id).join(format!("{seq:020}.seg"))
}

fn io_message(verb: &str, path: &Path, e: &std::io::Error) -> String {
    format!("cannot {verb} {}: {e}", path.display())
}

impl State {
    fn read(root: &Path, id: &str) -> Result<Option<State>, String> {
        let path = state_path(root, id);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("{} is not valid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_message("read", &path, &e)),
        }
    }

    fn save(&self, root: &Path, id: &str) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| format!("cannot encode state: {e}"))?;
        write_atomic(&state_path(root, id), &bytes)
    }

    /// The highest seq of this device's own segments that `device` acknowledged.
    fn acked(&self, device: &str) -> u64 {
        self.acks
            .get(device)
            .and_then(|a| a.get(&self.device))
            .copied()
            .unwrap_or(0)
    }

    /// The listed devices that left a segment of this device holding versions unacknowledged for more than
    /// `stale_days` by this device's own clock.
    fn stale(&self, others: &BTreeSet<String>, now: i64, stale_days: u32) -> BTreeSet<String> {
        let horizon = now - i64::from(stale_days) * DAY;
        others
            .iter()
            .filter(|d| {
                let acked = self.acked(d);
                let since = self.since.get(*d).copied().unwrap_or(i64::MIN);
                self.sent
                    .iter()
                    .any(|(seq, s)| s.versions && *seq > acked && s.at.max(since) < horizon)
            })
            .cloned()
            .collect()
    }
}

/// A file written whole through a hidden name in its folder and a rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().expect("a path has a folder");
    fs::create_dir_all(dir).map_err(|e| io_message("create", dir, &e))?;
    let name = frontmatter::mint_ulid().map_err(|e| format!("cannot mint a name: {e}"))?;
    let temporary = dir.join(format!("{TMP_PREFIX}{name}"));
    let written = File::create(&temporary).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    written
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|e| {
            let _ = fs::remove_file(&temporary);
            io_message("write", path, &e)
        })
}

/// One line of `seen.jsonl`: where a version, blob or declaration was pushed or applied.
#[derive(Debug, Serialize, Deserialize)]
struct Seen {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    declaration: Option<String>,
    /// Set on a version that only a `left` record carried: it is not in this scope's log.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    left: bool,
    device: String,
    seq: u64,
}

/// `seen.jsonl` as sets and places.
#[derive(Default)]
struct SeenIndex {
    /// Every place a version was seen, as device and seq.
    versions: HashMap<String, Vec<(String, u64)>>,
    blobs: HashSet<String>,
    declarations: HashSet<String>,
    /// The versions a `left` record carried.
    lefts: HashSet<String>,
}

fn read_seen(root: &Path, id: &str) -> Result<SeenIndex, String> {
    let path = seen_path(root, id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SeenIndex::default()),
        Err(e) => return Err(io_message("read", &path, &e)),
    };
    let mut index = SeenIndex::default();
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(seen) = serde_json::from_slice::<Seen>(line) else {
            continue;
        };
        if let Some(version) = seen.version {
            if seen.left {
                index.lefts.insert(version.clone());
            }
            index
                .versions
                .entry(version)
                .or_default()
                .push((seen.device, seen.seq));
        } else if let Some(blob) = seen.blob {
            index.blobs.insert(blob);
        } else if let Some(declaration) = seen.declaration {
            index.declarations.insert(declaration);
        }
    }
    Ok(index)
}

fn append_seen(root: &Path, id: &str, lines: &[Seen]) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    let path = seen_path(root, id);
    let dir = path.parent().expect("a path has a folder");
    fs::create_dir_all(dir).map_err(|e| io_message("create", dir, &e))?;
    let mut text = Vec::new();
    for line in lines {
        serde_json::to_writer(&mut text, line).map_err(|e| format!("cannot encode seen: {e}"))?;
        text.push(b'\n');
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(&text).and_then(|()| f.sync_all()))
        .map_err(|e| io_message("append to", &path, &e))
}

/// A declaration as the wire carries it: with its device, which a local one leaves empty.
fn on_the_wire(declaration: &Declaration, device: &str) -> Declaration {
    let mut wire = declaration.clone();
    wire.device.get_or_insert_with(|| device.to_string());
    wire
}

fn declaration_key(declaration: &Declaration, device: &str) -> String {
    let wire = serde_json::to_vec(&on_the_wire(declaration, device)).unwrap_or_default();
    hash::sha256_hex(&wire)
}

/// The seen lines for what a segment of `device` at `seq` holds.
fn seen_lines(plaintext: &Plaintext, device: &str, seq: u64, me: &str) -> Vec<Seen> {
    let line = |version, blob, declaration| Seen {
        version,
        blob,
        declaration,
        left: false,
        device: device.to_string(),
        seq,
    };
    let mut lines = Vec::new();
    for record in &plaintext.records {
        let mut seen = line(Some(record.version.version.clone()), None, None);
        seen.left = record.version.is_left();
        lines.push(seen);
    }
    for blob in &plaintext.blobs {
        lines.push(line(None, Some(blob.hash.clone()), None));
    }
    for declaration in &plaintext.declarations {
        let key = declaration_key(declaration, me);
        lines.push(line(None, None, Some(key)));
    }
    lines
}

/// The latest confirmed version, which decides who may write.
fn latest_confirmed(scope: &manifest::Scope) -> Option<&manifest::Version> {
    scope
        .versions
        .iter()
        .rfind(|v| !scope.pending.contains(&v.manifest.n))
}

/// A device's name as the newest version that lists it gives it, else its id.
fn who(scope: &manifest::Scope, id: &str) -> String {
    scope
        .versions
        .iter()
        .rev()
        .flat_map(|v| &v.manifest.devices)
        .find(|d| d.id == id)
        .map_or_else(|| id.to_string(), |d| d.name.clone())
}

/// `scope` as of the last confirmed version that lists `device`, the view that admits a dropped device's segments.
fn as_of_last_listing(scope: &manifest::Scope, device: &str) -> Option<manifest::Scope> {
    let last = scope
        .versions
        .iter()
        .rposition(|v| !scope.pending.contains(&v.manifest.n) && v.manifest.lists(device))?;
    Some(manifest::Scope {
        id: scope.id.clone(),
        versions: scope.versions[..=last]
            .iter()
            .map(|v| manifest::Version {
                manifest: v.manifest.clone(),
                bytes: v.bytes.clone(),
            })
            .collect(),
        invalid: None,
        pending: scope.pending.clone(),
    })
}

/// What a pull verified, for the integrate step to stage. Nothing of it counts as applied until `commit`.
pub struct Pulled {
    /// Records in seq order per device, each with the `device` that wrote its segment (none for this device's own).
    pub records: Vec<Record>,
    /// The blobs those records name that the segments carried, once each.
    pub blobs: Vec<Blob>,
    /// The declarations, each with its device.
    pub declarations: Vec<Declaration>,
    /// Lines for stderr, without the `bilbo: ` prefix.
    pub events: Vec<String>,
    /// Whether the read stopped at its size limit with segments left; poll again at once.
    pub more: bool,
    next: State,
    seen: Vec<Seen>,
    blob_hashes: HashSet<String>,
    bytes: usize,
    progress: bool,
}

/// What a push did.
#[derive(Debug, Default)]
pub struct Pushed {
    /// Lines for stderr, without the `bilbo: ` prefix.
    pub events: Vec<String>,
    /// The seqs of the segments created.
    pub segments: Vec<u64>,
    /// The transport's message when it refused a write as full: the push is retried at the next poll.
    pub full: Option<String>,
}

/// What `bilbo sync` shows of a scope's replica.
#[derive(Debug, PartialEq)]
pub struct Status {
    /// Why the scope does not sync, from watch's last decision or the other-content stop.
    pub stopped: Option<String>,
    /// The highest seq of this device's own folder.
    pub own: u64,
    /// The highest seq applied from each device.
    pub cursors: BTreeMap<String, u64>,
    /// The highest own seq each other device acknowledged.
    pub acked: BTreeMap<String, u64>,
    /// Own segments not yet acknowledged by every listed device.
    pub sent: BTreeMap<u64, Sent>,
    /// Unix seconds of the last committed pull and of the last segment created.
    pub pulled_at: Option<i64>,
    pub pushed_at: Option<i64>,
    /// Where reading each stuck device stopped.
    pub stops: BTreeMap<String, Stop>,
    /// The transport problem watch last reported.
    pub error: Option<Problem>,
    /// Unix seconds when each device was first seen listed.
    pub since: BTreeMap<String, i64>,
}

/// The state of scope `id` as `bilbo sync` reads it, `None` when the store holds none.
pub fn status(root: &Path, id: &str) -> Result<Option<Status>, String> {
    let Some(state) = State::read(root, id)? else {
        return Ok(None);
    };
    let acked = state
        .acks
        .keys()
        .filter(|d| **d != state.device)
        .map(|d| (d.clone(), state.acked(d)))
        .collect();
    Ok(Some(Status {
        stopped: state.stopped.clone().or_else(|| state.halted.clone()),
        own: state.own,
        cursors: state.cursors,
        acked,
        sent: state.sent,
        pulled_at: state.pulled_at,
        pushed_at: state.pushed_at,
        stops: state.stops,
        since: state.since,
        error: state.error,
    }))
}

/// How a poll last looked at one device folder, kept in memory.
#[derive(Default)]
struct Scan {
    listed: Option<i64>,
    gap: bool,
}

/// One scope's replica, which the watcher keeps across cycles. Every call takes the clock, so a test never sleeps.
pub struct Replica {
    root: PathBuf,
    id: String,
    me: String,
    state: State,
    /// Whether the store has no state and resumes from the transport; ends with the first push.
    resuming: bool,
    /// Whether a pull of this store has been committed.
    pulled_once: bool,
    /// Consecutive committed pulls that applied nothing.
    quiet: u8,
    swept: bool,
    scans: BTreeMap<String, Scan>,
    /// The scope each version's own bytes say, which never changes; `None` when it has none or is unreadable.
    scopes: HashMap<String, Option<String>>,
    /// The last line printed per channel, so a condition that holds is printed once.
    said: BTreeMap<String, String>,
}

/// Moves the state, the seen lines and the outbox of a store whose device id changed into a hidden folder: they
/// belong to the old device, and a segment of theirs under the new device's folder would have two writers.
fn set_aside(root: &Path, id: &str) -> Result<(), String> {
    let dir = scope_dir(root, id);
    let name = frontmatter::mint_ulid().map_err(|e| format!("cannot mint a name: {e}"))?;
    let aside = dir.join(format!(".old-{name}"));
    fs::create_dir_all(&aside).map_err(|e| io_message("create", &aside, &e))?;
    for entry in ["state.json", "seen.jsonl", "out"] {
        match fs::rename(dir.join(entry), aside.join(entry)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_message("move", &dir.join(entry), &e)),
        }
    }
    Ok(())
}

fn say(said: &mut BTreeMap<String, String>, channel: &str, line: String, events: &mut Vec<String>) {
    if said.get(channel) != Some(&line) {
        said.insert(channel.to_string(), line.clone());
        events.push(line);
    }
}

impl Replica {
    /// The replica of scope `id`, named `name` in the config, for the device `device`.
    pub fn open(root: &Path, id: &str, name: &str, device: &str) -> Result<Replica, String> {
        let (mut state, resuming) = match State::read(root, id)? {
            Some(state) if state.device == device => (state, false),
            Some(_) => {
                set_aside(root, id)?;
                (State::default(), true)
            }
            None => (State::default(), true),
        };
        state.name = name.to_string();
        state.device = device.to_string();
        Ok(Replica {
            root: root.to_path_buf(),
            id: id.to_string(),
            me: device.to_string(),
            state,
            resuming,
            pulled_once: false,
            quiet: 0,
            swept: false,
            scans: BTreeMap::new(),
            scopes: HashMap::new(),
            said: BTreeMap::new(),
        })
    }

    /// Why the scope does not sync: what `set_stopped` was given, else the other-content stop.
    pub fn stopped(&self) -> Option<&str> {
        self.state
            .stopped
            .as_deref()
            .or(self.state.halted.as_deref())
    }

    /// Records why the scope does not sync, or that it does, for `bilbo sync`. A scope that is stopped pushes nothing.
    pub fn set_stopped(&mut self, line: Option<String>) -> Result<(), String> {
        if self.state.stopped == line {
            return Ok(());
        }
        self.state.stopped = line;
        self.state.save(&self.root, &self.id)
    }

    /// Records the transport problem watch is reporting, or that there is none, for `bilbo sync`.
    pub fn set_error(&mut self, error: Option<Problem>) -> Result<(), String> {
        if self.state.error == error {
            return Ok(());
        }
        self.state.error = error;
        self.state.save(&self.root, &self.id)
    }

    fn say(&mut self, channel: &str, line: String, events: &mut Vec<String>) {
        say(&mut self.said, channel, line, events);
    }

    /// Reads what other devices wrote since the last commit, in seq order per device, and verifies it. It changes
    /// nothing on disk: the caller stages the result and then calls `commit`, so a crash between the two reads the same
    /// segments again.
    pub fn pull(
        &mut self,
        t: &dyn Transport,
        identity: &Identity,
        now: Timestamp,
    ) -> Result<Pulled, String> {
        if !self.swept {
            let _ = t.sweep(SystemTime::from(now));
            self.swept = true;
        }
        let scope = manifest::read_scope(&self.root, &self.id)?;
        let mut pulled = Pulled {
            records: Vec::new(),
            blobs: Vec::new(),
            declarations: Vec::new(),
            events: Vec::new(),
            more: false,
            next: self.state.clone(),
            seen: Vec::new(),
            blob_hashes: HashSet::new(),
            bytes: 0,
            progress: false,
        };
        if self.state.stopped.is_some() {
            return Ok(pulled);
        }
        let who = Recipient::device(&identity.device);
        let opened = manifest::open(&scope, &who).map_err(|e| e.to_string())?;
        let (Some(opened), Some(latest)) = (opened, latest_confirmed(&scope)) else {
            return Ok(pulled);
        };
        let listed: BTreeSet<String> = latest
            .manifest
            .devices
            .iter()
            .map(|d| d.id.clone())
            .collect();
        for dropped in self.state.listed.difference(&listed) {
            let at = self.state.cursors.get(dropped).copied().unwrap_or(0);
            pulled.next.cutoffs.entry(dropped.clone()).or_insert(at);
        }
        pulled.next.cutoffs.retain(|d, _| !listed.contains(d));
        pulled.next.listed = listed.clone();
        for device in &listed {
            pulled
                .next
                .since
                .entry(device.clone())
                .or_insert(now.as_second());
        }
        let mut devices = t.devices(&self.id)?;
        devices.sort_by_key(|d| !listed.contains(d));
        let cx = Cx {
            t,
            scope: &scope,
            opened: &opened,
            listed: &listed,
            now: now.as_second(),
        };
        for device in &devices {
            if pulled.more {
                break;
            }
            self.read_device(&cx, &mut pulled, device)?;
        }
        pulled.progress = pulled.next.cursors != self.state.cursors;
        pulled.next.pulled_at = Some(now.as_second());
        Ok(pulled)
    }

    /// Persists what `pulled` applied: the seen lines first, then the cursors, acks and the debt of an
    /// acknowledgement. Call it once the caller holds the records.
    pub fn commit(&mut self, pulled: &Pulled) -> Result<(), String> {
        append_seen(&self.root, &self.id, &pulled.seen)?;
        self.state = pulled.next.clone();
        self.state.save(&self.root, &self.id)?;
        self.quiet = if pulled.progress {
            0
        } else {
            self.quiet.saturating_add(1)
        };
        self.pulled_once = true;
        Ok(())
    }

    fn read_device(&mut self, cx: &Cx, pulled: &mut Pulled, device: &str) -> Result<(), String> {
        let is_me = device == self.me;
        if is_me && !self.resuming {
            return self.check_own(cx, pulled);
        }
        let listed = cx.listed.contains(device);
        let view = if listed {
            None
        } else {
            match as_of_last_listing(cx.scope, device) {
                Some(view) => Some(view),
                None => return Ok(()),
            }
        };
        let manifests = view.as_ref().unwrap_or(cx.scope);
        let channel = format!("pull:{device}");
        let name = who(cx.scope, device);
        let cursor = pulled.next.cursors.get(device).copied().unwrap_or(0);
        let limit = if listed {
            u64::MAX
        } else {
            pulled
                .next
                .cutoffs
                .get(device)
                .copied()
                .unwrap_or_else(|| ack_limit(&pulled.next.acks, device, cx.listed, &self.me))
        };
        pulled.next.cursors.entry(device.to_string()).or_insert(0);
        if cursor >= limit {
            pulled.next.stops.remove(device);
            return Ok(());
        }
        let seqs = self.discover(cx, device, cursor)?;
        let mut seq = cursor;
        while seq < limit {
            seq += 1;
            if pulled.bytes >= PULL_BYTES {
                pulled.more = true;
                break;
            }
            if !seqs.contains(&seq) {
                let later = seqs.iter().any(|s| *s > seq);
                self.scans.entry(device.to_string()).or_default().gap = later;
                if !later {
                    break;
                }
                let (line, why) = (format!("segment {seq} of {name} is missing"), "missing");
                self.stop(cx, pulled, device, seq, line, why.to_string());
                return Ok(());
            }
            let path = transport::segment_path(&self.id, device, seq);
            let (line, why) = match cx.t.get(&path) {
                Ok(Some(bytes)) => {
                    pulled.bytes += bytes.len();
                    let open = segment::open(&bytes, &self.id, device, seq, manifests, cx.opened);
                    match self.apply(cx, pulled, device, seq, &name, is_me, open) {
                        Ok(()) => continue,
                        Err(why) => (format!("segment {seq} of {name}: {why}"), why),
                    }
                }
                Ok(None) => (
                    format!("segment {seq} of {name} is missing"),
                    "missing".into(),
                ),
                Err(e) => (
                    format!("cannot read segment {seq} of {name}: {e}"),
                    format!("cannot read it: {e}"),
                ),
            };
            self.stop(cx, pulled, device, seq, line, why);
            return Ok(());
        }
        pulled.next.stops.remove(device);
        self.said.remove(&channel);
        Ok(())
    }

    /// Says once that reading `device` stopped at `seq`, and keeps where and why for `bilbo sync`.
    fn stop(
        &mut self,
        cx: &Cx,
        pulled: &mut Pulled,
        device: &str,
        seq: u64,
        line: String,
        why: String,
    ) {
        let line = format!("sync {}: {line}", self.state.name);
        self.say(&format!("pull:{device}"), line, &mut pulled.events);
        pulled.next.stops.insert(
            device.to_string(),
            Stop {
                seq,
                why,
                replaceable: !cx.t.keeps(),
            },
        );
    }

    /// The seqs of `device`'s folder past `cursor` that this poll can name: a probe past the cursor, and a listing on a
    /// first read, while a gap is known, or when nothing new has shown for a while.
    fn discover(&mut self, cx: &Cx, device: &str, cursor: u64) -> Result<BTreeSet<u64>, String> {
        let first = !self.state.cursors.contains_key(device);
        let mut seqs: BTreeSet<u64> = cx.t.probe(&self.id, device, cursor)?.into_iter().collect();
        let scan = self.scans.entry(device.to_string()).or_default();
        let due = scan.listed.is_none_or(|at| cx.now - at >= LIST_EVERY);
        if first || scan.gap || (seqs.is_empty() && due) {
            seqs.extend(cx.t.list_after(&self.id, device, cursor)?);
            scan.listed = Some(cx.now);
        }
        Ok(seqs)
    }

    /// Applies one opened segment to `pulled`, or says why reading `device` stops.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        cx: &Cx,
        pulled: &mut Pulled,
        device: &str,
        seq: u64,
        name: &str,
        is_me: bool,
        open: Result<segment::Read, Refusal>,
    ) -> Result<(), String> {
        let read = match open {
            Ok(read) => read,
            Err(refusal) if is_me && matches!(refusal, Refusal::Failed(_)) => {
                let line = format!(
                    "sync {}: segment {seq} of this device is damaged and no copy is left",
                    self.state.name
                );
                self.say(&format!("damaged:{seq}"), line, &mut pulled.events);
                pulled.next.cursors.insert(device.to_string(), seq);
                pulled.next.own = pulled.next.own.max(seq);
                return Ok(());
            }
            Err(refusal) => return Err(refusal.to_string()),
        };
        let mut plaintext = read.plaintext;
        for skipped in &read.skipped {
            let line = format!(
                "sync {}: a record of segment {seq} of {name} was skipped: {}",
                self.state.name, skipped.why
            );
            pulled.events.push(line);
        }
        let own_name = &cx.opened.name;
        plaintext.records.retain(|record| {
            let why = scope_mismatch(record, &plaintext.blobs, own_name);
            if let Some(why) = &why {
                let line = format!(
                    "sync {}: a record of segment {seq} of {name} was skipped: {why}",
                    self.state.name
                );
                pulled.events.push(line);
            }
            why.is_none()
        });
        let holds_versions = !plaintext.records.is_empty();
        pulled
            .seen
            .extend(seen_lines(&plaintext, device, seq, &self.me));
        let acks = pulled.next.acks.entry(device.to_string()).or_default();
        for (target, applied) in &plaintext.acks {
            let at = acks.entry(target.clone()).or_insert(0);
            *at = (*at).max(*applied);
        }
        for mut record in plaintext.records {
            record.version.device = (!is_me).then(|| device.to_string());
            pulled.records.push(record);
        }
        for blob in plaintext.blobs {
            if pulled.blob_hashes.insert(blob.hash.clone()) {
                pulled.blobs.push(blob);
            }
        }
        for mut declaration in plaintext.declarations {
            declaration.device.get_or_insert_with(|| device.to_string());
            pulled.declarations.push(declaration);
        }
        pulled.next.cursors.insert(device.to_string(), seq);
        if is_me {
            pulled.next.own = pulled.next.own.max(seq);
        } else if holds_versions {
            pulled.next.owed = true;
        }
        Ok(())
    }

    /// A segment in this device's folder past what this store wrote is another store's, unless the outbox holds
    /// exactly it, a segment whose creation was not booked before a crash.
    fn check_own(&mut self, cx: &Cx, pulled: &mut Pulled) -> Result<(), String> {
        let found = cx.t.probe(&self.id, &self.me, self.state.own)?;
        for seq in found {
            let path = transport::segment_path(&self.id, &self.me, seq);
            let kept = fs::read(out_path(&self.root, &self.id, seq)).ok();
            let theirs = cx.t.get(&path)?;
            if theirs.is_some() && theirs == kept {
                continue;
            }
            let line = format!(
                "segment {seq} of this device holds other content; another store writes as this device"
            );
            pulled.next.halted = Some(format!("sync {}: {line}", self.state.name));
            let line = format!("sync {}: {line}", self.state.name);
            self.say("other-content", line, &mut pulled.events);
            break;
        }
        Ok(())
    }
}

/// The highest seq of the dropped `device` that a device the latest confirmed version lists, or this one,
/// acknowledged: the most a store with no record of the drop reads.
fn ack_limit(
    acks: &BTreeMap<String, BTreeMap<String, u64>>,
    device: &str,
    listed: &BTreeSet<String>,
    me: &str,
) -> u64 {
    acks.iter()
        .filter(|(acker, _)| *acker != device && (listed.contains(*acker) || *acker == me))
        .filter_map(|(_, targets)| targets.get(device).copied())
        .max()
        .unwrap_or(0)
}

/// What one pull reads from.
struct Cx<'a> {
    t: &'a dyn Transport,
    scope: &'a manifest::Scope,
    opened: &'a Opened,
    listed: &'a BTreeSet<String>,
    now: i64,
}

/// Why a record does not belong in this scope's log, when its blob is in the segment and says another scope.
fn scope_mismatch(record: &Record, blobs: &[Blob], name: &str) -> Option<String> {
    let v = &record.version;
    if v.is_deleted() {
        return None;
    }
    let blob = blobs.iter().find(|b| b.hash == v.blob)?;
    let text = blob.bytes().ok().and_then(|b| String::from_utf8(b).ok());
    let says = text.map(|t| note::read(&t).scope);
    (says != Some(ScopeKey::Valid(name.to_string())))
        .then(|| format!("its text does not say scope: {name}"))
}

impl Replica {
    /// Pushes what this scope's log holds that no segment of it carries yet, or an acknowledgement. It reads the
    /// logs and the local manifests only.
    pub fn push(
        &mut self,
        t: &dyn Transport,
        identity: &Identity,
        settings: &config::Settings,
        now: Timestamp,
    ) -> Result<Pushed, String> {
        let mut out = Pushed::default();
        if self.stopped().is_some() {
            return Ok(out);
        }
        let scope = manifest::read_scope(&self.root, &self.id)?;
        let who = Recipient::device(&identity.device);
        let opened = manifest::open(&scope, &who).map_err(|e| e.to_string())?;
        let (Some(opened), Some(latest)) = (opened, latest_confirmed(&scope)) else {
            return Ok(out);
        };
        if !latest.manifest.lists(&self.me) {
            return Ok(out);
        }
        if self.resuming && !self.settled() {
            return Ok(out);
        }
        let Some(epoch) = manifest::usable_epoch(&scope, &opened) else {
            return Ok(out);
        };
        let newer = scope
            .versions
            .iter()
            .any(|v| scope.pending.contains(&v.manifest.n) && v.manifest.epoch > epoch);
        if newer {
            return Ok(out);
        }
        let key: [u8; 32] = **opened.keys.get(&epoch).ok_or("no key for the epoch")?;
        let sealing = Sealing {
            t,
            identity,
            scope: &scope,
            opened: &opened,
            epoch,
            key,
            now,
        };
        if !self.finish_outbox(&sealing, &mut out)? {
            return Ok(out);
        }
        self.settle(&scope, settings, now)?;
        if !self.repair_outbox(&sealing)? {
            return Ok(out);
        }
        let mut gathered = self.gather(settings)?;
        gathered.plaintext.acks = self.acks_to_send();
        let ack_only =
            gathered.plaintext.records.is_empty() && gathered.plaintext.declarations.is_empty();
        if ack_only && !self.ack_due(now.as_second()) {
            return Ok(out);
        }
        gathered.plaintext.at = now.to_string();
        for part in segment::split(gathered.plaintext) {
            let seq = self.state.own + 1;
            let bytes =
                match segment::seal(&self.id, &identity.device.sign, seq, epoch, &key, &part) {
                    Ok(bytes) => bytes,
                    Err(why) => {
                        if !self.said.contains_key("push") {
                            let line = format!("sync {}: cannot push: {why}", self.state.name);
                            self.said.insert("push".into(), line.clone());
                            out.events.push(line);
                        }
                        continue;
                    }
                };
            self.said.remove("push");
            write_atomic(&out_path(&self.root, &self.id, seq), &bytes)?;
            if !self.create(&sealing, seq, &bytes, &part, &mut out)? {
                break;
            }
        }
        if !out.segments.is_empty() && !gathered.marked.is_empty() {
            out.events.append(&mut gathered.events);
            self.state.marked.extend(gathered.marked);
            self.state.save(&self.root, &self.id)?;
        }
        Ok(out)
    }

    /// Whether the first push of a store that resumed from the transport may go: it read the transport at least once
    /// and the last polls found nothing new.
    fn settled(&mut self) -> bool {
        let settled = self.pulled_once
            && (self.state.cursors.values().all(|s| *s == 0) || self.quiet >= SETTLE_POLLS);
        if settled {
            self.resuming = false;
        }
        settled
    }

    /// The highest seq applied from each other device.
    fn acks_to_send(&self) -> BTreeMap<String, u64> {
        self.state
            .cursors
            .iter()
            .filter(|(d, s)| **d != self.me && **s > 0)
            .map(|(d, s)| (d.clone(), *s))
            .collect()
    }

    /// An acknowledgement only segment: owed, and none in the last hour.
    fn ack_due(&self, now: i64) -> bool {
        self.state.owed && self.state.last_ack.is_none_or(|at| now - at >= HOUR)
    }

    /// Drops what every live device acknowledged: its outbox file and the bookkeeping of what it was.
    fn settle(
        &mut self,
        scope: &manifest::Scope,
        settings: &config::Settings,
        now: Timestamp,
    ) -> Result<(), String> {
        let Some(latest) = latest_confirmed(scope) else {
            return Ok(());
        };
        let others: BTreeSet<String> = latest
            .manifest
            .devices
            .iter()
            .map(|d| d.id.clone())
            .filter(|d| *d != self.me)
            .collect();
        let stale = self
            .state
            .stale(&others, now.as_second(), settings.sync.stale_days);
        let live: Vec<&String> = others.difference(&stale).collect();
        let mut changed = false;
        let own = self.state.own;
        for seq in self.outbox()?.into_iter().filter(|q| *q <= own) {
            let acked = !live.is_empty() && live.iter().all(|d| self.state.acked(d) >= seq);
            if acked {
                let path = out_path(&self.root, &self.id, seq);
                fs::remove_file(&path).map_err(|e| io_message("remove", &path, &e))?;
            }
        }
        let before = self.state.sent.len();
        let state = &self.state;
        let still: BTreeMap<u64, Sent> = state
            .sent
            .iter()
            .filter(|(seq, _)| !others.iter().all(|d| state.acked(d) >= **seq))
            .map(|(seq, s)| (*seq, s.clone()))
            .collect();
        if still.len() != before {
            self.state.sent = still;
            changed = true;
        }
        if changed {
            self.state.save(&self.root, &self.id)?;
        }
        Ok(())
    }

    /// The seqs the outbox holds, ascending.
    fn outbox(&self) -> Result<Vec<u64>, String> {
        let dir = out_dir(&self.root, &self.id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_message("read", &dir, &e)),
        };
        let mut seqs: Vec<u64> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let stem = name.strip_suffix(".seg")?;
                (stem.len() == 20).then(|| stem.parse().ok()).flatten()
            })
            .collect();
        seqs.sort_unstable();
        Ok(seqs)
    }

    /// Creates the segments the outbox holds past `own`, which a crash left between writing them and booking them.
    /// `false` when one cannot go, with the reason in `out`.
    fn finish_outbox(&mut self, s: &Sealing, out: &mut Pushed) -> Result<bool, String> {
        let own = self.state.own;
        for seq in self.outbox()?.into_iter().filter(|q| *q > own) {
            let path = out_path(&self.root, &self.id, seq);
            let bytes = fs::read(&path).map_err(|e| io_message("read", &path, &e))?;
            let read = segment::open(&bytes, &self.id, &self.me, seq, s.scope, s.opened)
                .map_err(|why| format!("the outbox segment {seq} does not open: {why}"))?;
            if !self.create(s, seq, &bytes, &read.plaintext, out)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// On a transport that does not keep what it stores, creates again an outbox segment the folder lost and replaces
    /// one that no longer verifies. `false` when another store wrote one of them.
    fn repair_outbox(&mut self, s: &Sealing) -> Result<bool, String> {
        if s.t.keeps() {
            return Ok(true);
        }
        let own = self.state.own;
        for seq in self.outbox()?.into_iter().filter(|q| *q <= own) {
            let kept = fs::read(out_path(&self.root, &self.id, seq))
                .map_err(|e| format!("cannot read the outbox: {e}"))?;
            let path = transport::segment_path(&self.id, &self.me, seq);
            match s.t.get(&path) {
                Ok(Some(held)) if held == kept => {}
                Ok(None) => match s.t.create(&path, &kept) {
                    Put::Created | Put::Exists => {}
                    Put::Full(_) => return Ok(true),
                    Put::Unreachable(reason) => return Err(reason),
                },
                Ok(Some(held)) => {
                    let mine = segment::open(&held, &self.id, &self.me, seq, s.scope, s.opened);
                    if mine.is_ok() {
                        self.halt(seq)?;
                        return Ok(false);
                    }
                    s.t.replace(&path, &kept)?;
                }
                Err(_) => s.t.replace(&path, &kept)?,
            }
        }
        Ok(true)
    }

    fn halt(&mut self, seq: u64) -> Result<(), String> {
        let line = format!(
            "sync {}: segment {seq} of this device holds other content; another store writes as this device",
            self.state.name
        );
        self.state.halted = Some(line);
        self.state.save(&self.root, &self.id)
    }

    /// Creates segment `seq` on the transport and books it. `false` when it cannot go: the transport is full, or
    /// another store wrote that seq.
    fn create(
        &mut self,
        s: &Sealing,
        seq: u64,
        bytes: &[u8],
        plaintext: &Plaintext,
        out: &mut Pushed,
    ) -> Result<bool, String> {
        let path = transport::segment_path(&self.id, &self.me, seq);
        match s.t.create(&path, bytes) {
            Put::Created => {}
            Put::Exists => match s.t.get(&path)? {
                Some(held) if held == bytes => {}
                Some(held) => {
                    let mine = segment::open(&held, &self.id, &self.me, seq, s.scope, s.opened);
                    if mine.is_ok() || s.t.keeps() {
                        self.halt(seq)?;
                        let line = self.state.halted.clone().unwrap_or_default();
                        self.say("other-content", line, &mut out.events);
                        return Ok(false);
                    }
                    s.t.replace(&path, bytes)?;
                }
                None => return Ok(false),
            },
            Put::Full(message) => {
                out.full = Some(message);
                return Ok(false);
            }
            Put::Unreachable(reason) => return Err(reason),
        }
        self.book(seq, plaintext, s.now.as_second())?;
        out.segments.push(seq);
        self.resuming = false;
        Ok(true)
    }

    /// Records that segment `seq` of this device is on the transport.
    fn book(&mut self, seq: u64, plaintext: &Plaintext, now: i64) -> Result<(), String> {
        let lines = seen_lines(plaintext, &self.me, seq, &self.me);
        append_seen(&self.root, &self.id, &lines)?;
        self.state.own = seq;
        self.state.cursors.insert(self.me.clone(), seq);
        self.state.sent.insert(
            seq,
            Sent {
                at: now,
                versions: !plaintext.records.is_empty(),
            },
        );
        self.state.owed = false;
        self.state.pushed_at = Some(now);
        if plaintext.records.is_empty() && plaintext.declarations.is_empty() {
            self.state.last_ack = Some(now);
        }
        self.state.save(&self.root, &self.id)
    }
}

/// What a push seals and writes with.
struct Sealing<'a> {
    t: &'a dyn Transport,
    identity: &'a Identity,
    scope: &'a manifest::Scope,
    opened: &'a Opened,
    epoch: u64,
    key: [u8; 32],
    now: Timestamp,
}

/// What a push found to send.
struct Gathered {
    plaintext: Plaintext,
    events: Vec<String>,
    /// Notes whose mark line this push printed.
    marked: Vec<String>,
}

impl Replica {
    /// The scope's log that no segment carries yet: the versions whose own bytes say this scope, `left` records for
    /// versions that moved a note out of it, their blobs and the declarations of their conflicts.
    fn gather(&mut self, settings: &config::Settings) -> Result<Gathered, String> {
        let name = self.state.name.clone();
        let seen = read_seen(&self.root, &self.id)?;
        let mut plaintext = Plaintext::new("");
        let mut events = Vec::new();
        let mut marked = Vec::new();
        let mut blob_hashes: HashSet<String> = HashSet::new();
        for note_id in versions::note_ids(&self.root)? {
            let log = versions::load(&self.root, &note_id)?;
            let by_id: HashMap<&str, &Version> = log
                .versions
                .iter()
                .map(|v| (v.version.as_str(), v))
                .collect();
            let known = |id: &str| seen.versions.contains_key(id);
            let in_scope = |id: &str| known(id) && !seen.lefts.contains(id);
            let mut fresh: Vec<&Version> = Vec::new();
            let mut left: Vec<&Version> = Vec::new();
            let mut listed: HashSet<&str> = HashSet::new();
            for v in &log.versions {
                if known(&v.version) || v.is_left() || !listed.insert(v.version.as_str()) {
                    continue;
                }
                let Some(own) = scope_of(&self.root, &mut self.scopes, &by_id, v) else {
                    continue;
                };
                if own.as_deref() == Some(name.as_str()) {
                    fresh.push(v);
                    continue;
                }
                let mut unsure = false;
                let mut follows = false;
                for p in &v.parents {
                    follows |= in_scope(p);
                    if let Some(parent) = by_id.get(p.as_str()) {
                        match scope_of(&self.root, &mut self.scopes, &by_id, parent) {
                            Some(scope) => follows |= scope.as_deref() == Some(name.as_str()),
                            None => unsure = true,
                        }
                    }
                }
                if follows {
                    left.push(v);
                } else if unsure {
                    continue;
                }
            }
            let sending: HashSet<&str> = fresh.iter().map(|v| v.version.as_str()).collect();
            for v in fresh {
                let mut wire = v.clone();
                wire.device = None;
                wire.outside = v
                    .parents
                    .iter()
                    .filter(|p| !in_scope(p) && !sending.contains(p.as_str()))
                    .cloned()
                    .collect();
                if !v.is_deleted() {
                    let bytes = match versions::content(&self.root, v) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    };
                    if !seen.blobs.contains(&v.blob) && blob_hashes.insert(v.blob.clone()) {
                        plaintext.blobs.push(Blob::new(&bytes));
                    }
                    if !self.state.marked.contains(&note_id) && !marked.contains(&note_id) {
                        let text = String::from_utf8_lossy(&bytes);
                        if let Some(other) = mark_of_other(settings, &name, &v.file, &text) {
                            events.push(format!(
                                "notes/{}: pushed to '{name}' while holding a mark of '{other}'",
                                v.file
                            ));
                            marked.push(note_id.clone());
                        }
                    }
                }
                plaintext.records.push(Record {
                    note: note_id.clone(),
                    version: wire,
                });
            }
            for v in left {
                plaintext.records.push(Record {
                    note: note_id.clone(),
                    version: Version {
                        version: v.version.clone(),
                        parents: v.parents.clone(),
                        file: String::new(),
                        blob: versions::DELETED.to_string(),
                        event: versions::LEFT.to_string(),
                        at: v.at.clone(),
                        outside: v
                            .parents
                            .iter()
                            .filter(|p| !in_scope(p) && !sending.contains(p.as_str()))
                            .cloned()
                            .collect(),
                        ..Version::default()
                    },
                });
            }
            for d in &log.declarations {
                let key = declaration_key(d, &self.me);
                let in_log = in_scope(&d.declare)
                    || sending.contains(d.declare.as_str())
                    || by_id.get(d.declare.as_str()).is_some_and(|c| {
                        scope_of(&self.root, &mut self.scopes, &by_id, c)
                            .flatten()
                            .as_deref()
                            == Some(name.as_str())
                    });
                if in_log && !seen.declarations.contains(&key) {
                    plaintext.declarations.push(on_the_wire(d, &self.me));
                }
            }
        }
        Ok(Gathered {
            plaintext,
            events,
            marked,
        })
    }
}

/// The scope a version's own bytes say: `Some(None)` when they say none or another reading of it, `None` when they
/// cannot be read now, which is not cached. A deletion has no bytes and takes its parents' scope.
fn scope_of(
    root: &Path,
    cache: &mut HashMap<String, Option<String>>,
    by_id: &HashMap<&str, &Version>,
    v: &Version,
) -> Option<Option<String>> {
    if let Some(cached) = cache.get(&v.version) {
        return Some(cached.clone());
    }
    let found = if v.is_left() {
        None
    } else if v.is_deleted() {
        let parents: Vec<Option<Option<String>>> = v
            .parents
            .iter()
            .filter_map(|p| by_id.get(p.as_str()))
            .map(|p| scope_of(root, cache, by_id, p))
            .collect();
        match parents.iter().flatten().flatten().next() {
            Some(scope) => Some(scope.clone()),
            None if parents.iter().any(Option::is_none) => return None,
            None => None,
        }
    } else {
        let bytes = versions::content(root, v).ok()?;
        match String::from_utf8(bytes).map(|text| note::read(&text).scope) {
            Ok(ScopeKey::Valid(scope)) => Some(scope),
            _ => None,
        }
    };
    cache.insert(v.version.clone(), found.clone());
    Some(found)
}

/// The first other declared scope whose marks `text` holds.
fn mark_of_other(settings: &config::Settings, own: &str, file: &str, text: &str) -> Option<String> {
    let topic = note::parse_name(file).ok()?.topic;
    settings
        .scopes
        .iter()
        .filter(|s| s.name != own)
        .find(|s| marks::first_mark(s, &topic, text).is_some())
        .map(|s| s.name.clone())
}

/// The versions of each note that no live device is known to lack, as `versions::prune` needs them: per note, the
/// known heads of every device of a syncing scope that is not stale, or the whole log when one holds none.
pub fn known_heads(
    root: &Path,
    settings: &config::Settings,
    now: Timestamp,
) -> Result<Guard, String> {
    let mut guard = Guard::new();
    for id in manifest::scope_ids(root)? {
        let Some(state) = State::read(root, &id)? else {
            continue;
        };
        let syncs = settings
            .scopes
            .iter()
            .any(|s| s.name == state.name && s.sync != "off");
        if !syncs {
            continue;
        }
        let scope = manifest::read_scope(root, &id)?;
        let Some(latest) = latest_confirmed(&scope) else {
            continue;
        };
        let others: BTreeSet<String> = latest
            .manifest
            .devices
            .iter()
            .map(|d| d.id.clone())
            .filter(|d| *d != state.device)
            .collect();
        let stale = state.stale(&others, now.as_second(), settings.sync.stale_days);
        let live: Vec<&String> = others.difference(&stale).collect();
        if live.is_empty() {
            continue;
        }
        let seen = read_seen(root, &id)?;
        for note_id in versions::note_ids(root)? {
            let log = versions::load(root, &note_id)?;
            if !log
                .versions
                .iter()
                .any(|v| seen.versions.contains_key(&v.version))
            {
                continue;
            }
            let mut hold = Hold::After(BTreeSet::new());
            for device in &live {
                let holds = |v: &&Version| {
                    seen.versions.get(&v.version).is_some_and(|places| {
                        places.iter().any(|(from, seq)| {
                            from == *device
                                || state
                                    .acks
                                    .get(*device)
                                    .and_then(|a| a.get(from))
                                    .is_some_and(|applied| applied >= seq)
                        })
                    })
                };
                let known: Vec<&Version> = log.versions.iter().filter(holds).collect();
                let followed: HashSet<&str> = known
                    .iter()
                    .flat_map(|v| v.parents.iter().map(String::as_str))
                    .collect();
                let heads: BTreeSet<String> = known
                    .iter()
                    .filter(|v| !followed.contains(v.version.as_str()))
                    .map(|v| v.version.clone())
                    .collect();
                hold = match (hold, heads.is_empty()) {
                    (_, true) | (Hold::All, _) => Hold::All,
                    (Hold::After(mut all), false) => {
                        all.extend(heads);
                        Hold::After(all)
                    }
                };
            }
            guard
                .entry(note_id)
                .and_modify(|held| {
                    *held = match (&*held, &hold) {
                        (Hold::After(a), Hold::After(b)) => {
                            Hold::After(a.union(b).cloned().collect())
                        }
                        _ => Hold::All,
                    }
                })
                .or_insert(hold);
        }
    }
    Ok(guard)
}

impl Replica {
    /// Drops the `seen.jsonl` lines of versions no log holds any more, of blobs and declarations that only they
    /// named. The watcher runs it after `apply` in a cycle, so a version still waiting in the inbox keeps its line.
    /// Returns how many lines it dropped.
    pub fn trim_seen(&self) -> Result<usize, String> {
        let path = seen_path(&self.root, &self.id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(io_message("read", &path, &e)),
        };
        let mut held_versions: HashSet<String> = HashSet::new();
        let mut held_blobs: HashSet<String> = HashSet::new();
        let mut held_declarations: HashSet<String> = HashSet::new();
        for note_id in versions::note_ids(&self.root)? {
            let log = versions::load(&self.root, &note_id)?;
            for v in &log.versions {
                held_versions.insert(v.version.clone());
                held_blobs.insert(v.blob.clone());
            }
            for d in &log.declarations {
                held_declarations.insert(declaration_key(d, &self.me));
            }
        }
        let mut kept = Vec::new();
        let mut dropped = 0;
        for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let keep = match serde_json::from_slice::<Seen>(line) {
                Ok(Seen {
                    version: Some(v), ..
                }) => held_versions.contains(&v),
                Ok(Seen { blob: Some(b), .. }) => held_blobs.contains(&b),
                Ok(Seen {
                    declaration: Some(d),
                    ..
                }) => held_declarations.contains(&d),
                _ => false,
            };
            if keep {
                kept.extend_from_slice(line);
                kept.push(b'\n');
            } else {
                dropped += 1;
            }
        }
        if dropped > 0 {
            write_atomic(&path, &kept)?;
        }
        Ok(dropped)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::identity::keys::{Device, Owner};
    use crate::identity::manifest::Member;
    use crate::shared::store::Env;
    use crate::sync::transport::Folder;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn identity(name: &str, seed: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[0; 16]).file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    const AT: &str = "2026-10-05T10:00:00-03:00";
    const T0: i64 = 1_790_000_000;

    fn ts(seconds: i64) -> Timestamp {
        Timestamp::from_second(T0 + seconds).unwrap()
    }

    fn ulid(i: usize) -> String {
        format!("{i:026}")
    }

    /// Two devices of one scope `personal`, each with a store of its own and one folder between them.
    struct World {
        dir: Scratch,
        a: Identity,
        b: Identity,
        c: Identity,
        scope: String,
        settings: config::Settings,
    }

    impl World {
        fn new(name: &str) -> World {
            World::build(name, false, false)
        }

        /// Version 1 lists `c` too, a device that never runs here.
        fn three(name: &str) -> World {
            World::build(name, true, false)
        }

        /// Version 1 lists `a` alone and version 2 adds `b` at the same epoch.
        fn staged(name: &str) -> World {
            World::build(name, false, true)
        }

        fn build(name: &str, third: bool, staged: bool) -> World {
            let dir =
                std::env::temp_dir().join(format!("bilbo-replica-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("folder")).unwrap();
            let (a, b, c) = (identity("a", 1), identity("b", 3), identity("c", 5));
            let lock = manifest::lock(&dir.join("a")).unwrap();
            let url = "file:///x";
            let mut others = if staged {
                Vec::new()
            } else {
                vec![Member::of(&b.device)]
            };
            if third {
                others.push(Member::of(&c.device));
            }
            let made = manifest::create(&lock, &a, "personal", url, &others).unwrap();
            let scope = made.scope;
            if staged {
                let first = manifest::read_scope(&dir.join("a"), &scope).unwrap();
                let opened = manifest::open(&first, &Recipient::device(&a.device))
                    .unwrap()
                    .unwrap();
                for v in &first.versions {
                    manifest::confirm(&lock, &scope, v.manifest.n, &v.bytes).unwrap();
                }
                let key: [u8; 32] = *opened.keys[&1];
                manifest::add_device(&lock, &first, &key, &Member::of(&b.device), &a).unwrap();
            }
            drop(lock);
            let world = World {
                settings: settings(&dir, ""),
                dir: Scratch(dir),
                a,
                b,
                c,
                scope,
            };
            world.confirm_everywhere();
            world
        }

        fn root(&self, who: char) -> PathBuf {
            self.dir.0.join(who.to_string())
        }

        fn confirm_everywhere(&self) {
            let from = manifest::read_scope(&self.root('a'), &self.scope).unwrap();
            for v in &from.versions {
                for who in ['a', 'b'] {
                    let lock = manifest::lock(&self.root(who)).unwrap();
                    let _ = manifest::adopt(&lock, &self.scope, v.manifest.n, &v.bytes);
                    manifest::confirm(&lock, &self.scope, v.manifest.n, &v.bytes).unwrap();
                }
            }
        }

        /// A second store of device `a` with the scope's confirmed versions: the same key, another store.
        fn second_store(&self) -> PathBuf {
            let other = self.dir.0.join("a2");
            let lock = manifest::lock(&other).unwrap();
            let from = manifest::read_scope(&self.root('a'), &self.scope).unwrap();
            for v in &from.versions {
                manifest::adopt(&lock, &self.scope, v.manifest.n, &v.bytes).unwrap();
                manifest::confirm(&lock, &self.scope, v.manifest.n, &v.bytes).unwrap();
            }
            other
        }

        fn folder(&self, who: &Identity) -> Folder {
            Folder::new(self.dir.0.join("folder"), &who.device.id())
        }

        fn replica(&self, who: char) -> Replica {
            let id = if who == 'a' { &self.a } else { &self.b };
            Replica::open(&self.root(who), &self.scope, "personal", &id.device.id()).unwrap()
        }

        fn segment_file(&self, who: &Identity, seq: u64) -> PathBuf {
            self.dir.0.join("folder").join(transport::segment_path(
                &self.scope,
                &who.device.id(),
                seq,
            ))
        }

        fn pull(&self, replica: &mut Replica, who: &Identity, at: i64) -> Pulled {
            let t = self.folder(who);
            let pulled = replica.pull(&t, who, ts(at)).unwrap();
            replica.commit(&pulled).unwrap();
            pulled
        }

        fn push(&self, replica: &mut Replica, who: &Identity, at: i64) -> Pushed {
            let t = self.folder(who);
            replica.push(&t, who, &self.settings, ts(at)).unwrap()
        }
    }

    fn settings(dir: &Path, extra: &str) -> config::Settings {
        let path = dir.join("config");
        let text = format!(
            "scope.personal.sync = file:///x\nscope.work.sync = off\nscope.work.marks = acme\n{extra}"
        );
        fs::write(&path, text).unwrap();
        let env = Env::from_vars(|name| (name == "BILBO_CONFIG").then(|| OsString::from(&path)));
        config::load(&env).unwrap()
    }

    /// Records a note's file as a new version in `root`'s history.
    fn save(root: &Path, note: &str, file: &str, scope: Option<&str>, body: &str) -> Version {
        let lock = versions::lock(root).unwrap();
        let parents = versions::last_parents(root, note).unwrap();
        let scope_line = scope.map(|s| format!("scope: {s}\n")).unwrap_or_default();
        let text = format!("---\nid: {note}\ncreated: {AT}\n{scope_line}---\n\n# {body}\n");
        let event = if parents.is_empty() {
            versions::ADDED
        } else {
            versions::EDITED
        };
        versions::record(
            &lock,
            note,
            &parents,
            file,
            Some(text.as_bytes()),
            event,
            AT,
        )
        .unwrap()
    }

    /// What the integrate step does with a pull: the blobs and the versions into the history.
    fn stage(root: &Path, pulled: &Pulled) {
        let lock = versions::lock(root).unwrap();
        for blob in &pulled.blobs {
            versions::write_blob(&lock, &blob.bytes().unwrap()).unwrap();
        }
        for record in &pulled.records {
            versions::append(&lock, &record.note, &record.version).unwrap();
        }
    }

    fn versions_of(pulled: &Pulled) -> Vec<&str> {
        pulled
            .records
            .iter()
            .map(|r| r.version.version.as_str())
            .collect()
    }

    #[test]
    fn an_edit_reaches_the_other_replica_with_its_blob_and_device() {
        let w = World::new("reach");
        let v = save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        let pushed = w.push(&mut a, &w.a, 1);
        assert_eq!(pushed.segments, vec![1]);
        let pulled = w.pull(&mut b, &w.b, 2);
        assert_eq!(versions_of(&pulled), vec![v.version.as_str()]);
        assert_eq!(pulled.records[0].version.device, Some(w.a.device.id()));
        assert_eq!(pulled.blobs.len(), 1);
        assert!(pulled.events.is_empty());
        let again = w.pull(&mut b, &w.b, 3);
        assert!(again.records.is_empty());
    }

    #[test]
    fn nothing_leaves_a_note_with_no_scope_or_another_one() {
        let w = World::new("noscope");
        save(&w.root('a'), &ulid(1), "plan-x.md", None, "one");
        save(&w.root('a'), &ulid(2), "plan-y.md", Some("work"), "two");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        let pushed = w.push(&mut a, &w.a, 1);
        assert!(pushed.segments.is_empty());
        assert!(!w.segment_file(&w.a, 1).exists());
    }

    #[test]
    fn assigning_an_old_note_pushes_its_current_text_and_names_the_parent_outside() {
        let w = World::new("assign");
        let old = save(&w.root('a'), &ulid(1), "plan-x.md", None, "old");
        let now = save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "new");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let pulled = w.pull(&mut b, &w.b, 2);
        assert_eq!(versions_of(&pulled), vec![now.version.as_str()]);
        assert_eq!(pulled.records[0].version.outside, vec![old.version]);
        assert_eq!(pulled.blobs.len(), 1);
    }

    #[test]
    fn a_version_in_the_scope_goes_with_its_ancestors_that_also_say_it() {
        let w = World::new("closure");
        let first = save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let second = save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "two");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let pulled = w.pull(&mut b, &w.b, 2);
        assert_eq!(
            versions_of(&pulled),
            vec![first.version.as_str(), second.version.as_str()]
        );
        assert!(pulled.records.iter().all(|r| r.version.outside.is_empty()));
    }

    #[test]
    fn a_move_out_of_the_scope_sends_a_left_record_with_no_text() {
        let w = World::new("left");
        let first = save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let moved = save(&w.root('a'), &ulid(1), "plan-x.md", Some("work"), "one");
        let pushed = w.push(&mut a, &w.a, 2);
        assert_eq!(pushed.segments, vec![2]);
        let pulled = w.pull(&mut b, &w.b, 3);
        let left: Vec<_> = pulled
            .records
            .iter()
            .filter(|r| r.version.is_left())
            .collect();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].version.version, moved.version);
        assert_eq!(left[0].version.parents, vec![first.version]);
        assert!(pulled.blobs.iter().all(|b| b.bytes().unwrap() != b"work"));
        let raw = fs::read_to_string(w.segment_file(&w.a, 2)).unwrap();
        assert!(!raw.contains("work"));
        let again = w.push(&mut a, &w.a, 4);
        assert!(again.segments.is_empty());
    }

    #[test]
    fn a_later_version_after_the_move_also_sends_one_left() {
        let w = World::new("left-later");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("work"), "one");
        w.push(&mut a, &w.a, 2);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("work"), "two");
        let pushed = w.push(&mut a, &w.a, 3);
        assert!(pushed.segments.is_empty());
    }

    #[test]
    fn the_first_push_of_a_marked_note_prints_one_line() {
        let w = World::new("marks");
        save(
            &w.root('a'),
            &ulid(1),
            "plan-x.md",
            Some("personal"),
            "acme rollout",
        );
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        let pushed = w.push(&mut a, &w.a, 1);
        assert_eq!(
            pushed.events,
            vec!["notes/plan-x.md: pushed to 'personal' while holding a mark of 'work'"]
        );
        save(
            &w.root('a'),
            &ulid(1),
            "plan-x.md",
            Some("personal"),
            "acme again",
        );
        let second = w.push(&mut a, &w.a, 2);
        assert!(second.events.is_empty());
        assert_eq!(second.segments, vec![2]);
        save(
            &w.root('a'),
            &ulid(2),
            "plan-y.md",
            Some("personal"),
            "plain",
        );
        assert!(w.push(&mut a, &w.a, 3).events.is_empty());
    }

    #[test]
    fn nothing_is_pushed_twice_and_a_blob_travels_once() {
        let w = World::new("once");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        save(&w.root('a'), &ulid(2), "plan-y.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        assert_eq!(w.push(&mut a, &w.a, 1).segments, vec![1]);
        assert!(w.push(&mut a, &w.a, 2).segments.is_empty());
        let pulled = w.pull(&mut b, &w.b, 3);
        assert_eq!(pulled.records.len(), 2);
        let mut c = w.replica('a');
        w.pull(&mut c, &w.a, 4);
        assert!(w.push(&mut c, &w.a, 5).segments.is_empty());
    }

    #[test]
    fn an_acknowledgement_goes_out_once_an_hour_and_only_when_owed() {
        let w = World::new("acks");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        w.pull(&mut b, &w.b, 2);
        w.pull(&mut b, &w.b, 3);
        w.pull(&mut b, &w.b, 3);
        assert_eq!(w.push(&mut b, &w.b, 4).segments, vec![1]);
        assert!(w.push(&mut b, &w.b, 5).segments.is_empty());
        let heard = w.pull(&mut a, &w.a, 6);
        assert!(heard.records.is_empty());
        assert!(w.push(&mut a, &w.a, 7).segments.is_empty());
        assert!(w.push(&mut a, &w.a, 7 + 24 * 3600).segments.is_empty());
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "two");
        w.push(&mut a, &w.a, 100);
        w.pull(&mut b, &w.b, 101);
        assert!(w.push(&mut b, &w.b, 102).segments.is_empty());
        assert_eq!(w.push(&mut b, &w.b, 4 + 3601).segments, vec![2]);
        assert!(w.push(&mut b, &w.b, 4 + 3602).segments.is_empty());
    }

    #[test]
    fn idle_devices_write_one_segment_each_and_go_quiet() {
        let w = World::new("quiet");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        let mut written = w.push(&mut a, &w.a, 1).segments.len();
        for second in 2..40 {
            w.pull(&mut b, &w.b, second);
            written += w.push(&mut b, &w.b, second).segments.len();
            w.pull(&mut a, &w.a, second);
            written += w.push(&mut a, &w.a, second).segments.len();
        }
        assert_eq!(written, 2);
    }

    #[test]
    fn a_deleted_segment_comes_back_until_everyone_live_acknowledged_it() {
        let w = World::new("survive");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let file = w.segment_file(&w.a, 1);
        let bytes = fs::read(&file).unwrap();
        fs::remove_file(&file).unwrap();
        w.push(&mut a, &w.a, 2);
        assert_eq!(fs::read(&file).unwrap(), bytes);
        assert_eq!(versions_of(&w.pull(&mut b, &w.b, 3)).len(), 1);
        w.pull(&mut b, &w.b, 4);
        w.pull(&mut b, &w.b, 4);
        w.push(&mut b, &w.b, 5);
        w.pull(&mut a, &w.a, 6);
        fs::remove_file(&file).unwrap();
        w.push(&mut a, &w.a, 7);
        assert!(!file.exists());
        assert!(!out_path(&w.root('a'), &w.scope, 1).exists());
    }

    #[test]
    fn a_truncated_segment_is_replaced_from_the_outbox_before_the_other_applies_it() {
        let w = World::new("damaged");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let file = w.segment_file(&w.a, 1);
        let bytes = fs::read(&file).unwrap();
        fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
        let held = w.pull(&mut b, &w.b, 2);
        assert!(held.records.is_empty());
        assert_eq!(held.events.len(), 1);
        w.push(&mut a, &w.a, 3);
        assert_eq!(fs::read(&file).unwrap(), bytes);
        assert_eq!(w.pull(&mut b, &w.b, 4).records.len(), 1);
    }

    fn forget(w: &World, who: char) {
        let dir = scope_dir(&w.root(who), &w.scope);
        for name in ["state.json", "seen.jsonl", "out"] {
            let path = dir.join(name);
            let _ = fs::remove_file(&path).or_else(|_| fs::remove_dir_all(&path));
        }
        let _ = fs::remove_dir_all(store::history_dir(&w.root(who)));
    }

    #[test]
    fn a_store_that_lost_its_state_reads_its_own_segments_and_pushes_the_next_seq() {
        let w = World::new("resume");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        for i in 1..=3 {
            save(
                &w.root('a'),
                &ulid(1),
                "plan-x.md",
                Some("personal"),
                &format!("v{i}"),
            );
            assert_eq!(w.push(&mut a, &w.a, i).segments, vec![i as u64]);
        }
        forget(&w, 'a');
        let mut a = w.replica('a');
        let first = w.pull(&mut a, &w.a, 10);
        assert_eq!(first.records.len(), 3);
        assert!(first.records.iter().all(|r| r.version.device.is_none()));
        assert!(first.events.is_empty());
        stage(&w.root('a'), &first);
        assert!(w.push(&mut a, &w.a, 11).segments.is_empty());
        w.pull(&mut a, &w.a, 12);
        assert!(w.push(&mut a, &w.a, 12).segments.is_empty());
        w.pull(&mut a, &w.a, 13);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "v4");
        let pushed = w.push(&mut a, &w.a, 13);
        assert_eq!(pushed.segments, vec![4]);
        assert!(pushed.events.is_empty());
        assert!(a.stopped().is_none());
    }

    #[test]
    fn resuming_with_a_damaged_own_segment_and_no_copy_says_so_and_goes_on() {
        let w = World::new("nocopy");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        for i in 1..=2 {
            save(
                &w.root('a'),
                &ulid(1),
                "plan-x.md",
                Some("personal"),
                &format!("v{i}"),
            );
            w.push(&mut a, &w.a, i);
        }
        let file = w.segment_file(&w.a, 1);
        let bytes = fs::read(&file).unwrap();
        fs::write(&file, &bytes[..10]).unwrap();
        forget(&w, 'a');
        let mut a = w.replica('a');
        let pulled = w.pull(&mut a, &w.a, 10);
        assert_eq!(pulled.records.len(), 1);
        assert_eq!(
            pulled.events,
            vec!["sync personal: segment 1 of this device is damaged and no copy is left"]
        );
        assert_eq!(a.state.own, 2);
    }

    #[test]
    fn a_push_onto_a_seq_another_store_wrote_halts_without_overwriting() {
        let w = World::new("twowriters");
        let mut first = w.replica('a');
        w.pull(&mut first, &w.a, 0);
        let mut second = w.replica('a');
        w.pull(&mut second, &w.a, 0);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        assert_eq!(w.push(&mut first, &w.a, 1).segments, vec![1]);
        let before = fs::read(w.segment_file(&w.a, 1)).unwrap();
        let other = w.second_store();
        let mut stranger = Replica::open(&other, &w.scope, "personal", &w.a.device.id()).unwrap();
        stranger.state.own = 0;
        stranger.resuming = false;
        save(&other, &ulid(9), "plan-z.md", Some("personal"), "other");
        let t = w.folder(&w.a);
        let pushed = stranger.push(&t, &w.a, &w.settings, ts(2)).unwrap();
        assert!(pushed.segments.is_empty());
        assert_eq!(
            pushed.events,
            vec![
                "sync personal: segment 1 of this device holds other content; another store writes as this device"
            ]
        );
        assert!(stranger.stopped().is_some());
        assert_eq!(fs::read(w.segment_file(&w.a, 1)).unwrap(), before);
        let again = stranger.push(&t, &w.a, &w.settings, ts(3)).unwrap();
        assert!(again.events.is_empty() && again.segments.is_empty());
        let status = status(&other, &w.scope).unwrap().unwrap();
        assert!(status.stopped.unwrap().contains("holds other content"));
    }

    #[test]
    fn a_pull_notices_a_segment_another_store_wrote_as_this_device() {
        let w = World::new("otherpull");
        let mut mine = w.replica('a');
        w.pull(&mut mine, &w.a, 0);
        w.push(&mut mine, &w.a, 0);
        let root = w.second_store();
        let mut other = Replica::open(&root, &w.scope, "personal", &w.a.device.id()).unwrap();
        let t = w.folder(&w.a);
        let first = other.pull(&t, &w.a, ts(0)).unwrap();
        other.commit(&first).unwrap();
        save(&root, &ulid(1), "plan-x.md", Some("personal"), "one");
        other.push(&t, &w.a, &w.settings, ts(1)).unwrap();
        let seen = w.pull(&mut mine, &w.a, 2);
        assert_eq!(seen.events.len(), 1);
        assert!(seen.events[0].contains("holds other content"));
        assert!(mine.stopped().is_some());
    }

    #[test]
    fn segments_apply_in_order_and_reading_stops_at_a_gap_until_it_closes() {
        let w = World::new("gap");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        for i in 1..=4 {
            save(
                &w.root('a'),
                &ulid(1),
                "plan-x.md",
                Some("personal"),
                &format!("v{i}"),
            );
            w.push(&mut a, &w.a, i);
        }
        let three = w.segment_file(&w.a, 3);
        let bytes = fs::read(&three).unwrap();
        fs::remove_file(&three).unwrap();
        let held = w.pull(&mut b, &w.b, 10);
        assert_eq!(held.records.len(), 2);
        assert_eq!(
            held.events,
            vec!["sync personal: segment 3 of a is missing"]
        );
        let still = w.pull(&mut b, &w.b, 11);
        assert!(still.records.is_empty() && still.events.is_empty());
        fs::write(&three, bytes).unwrap();
        assert_eq!(w.pull(&mut b, &w.b, 12).records.len(), 2);
    }

    #[test]
    fn a_newer_format_stops_that_device_with_a_line_naming_it() {
        let w = World::new("format");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        let path = w.segment_file(&w.a, 1);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, br#"{"format":2}"#).unwrap();
        let held = w.pull(&mut b, &w.b, 1);
        assert_eq!(held.events.len(), 1);
        assert!(held.events[0].contains("segment 1 of a"));
        assert!(held.events[0].contains("upgrade bilbo"));
    }

    #[test]
    fn a_tampered_segment_applies_nothing_and_names_the_device_and_seq() {
        let w = World::new("tamper");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let file = w.segment_file(&w.a, 1);
        let mut text = fs::read_to_string(&file).unwrap();
        let at = text.find("\"ciphertext\":\"").unwrap() + 14;
        let flipped = if &text[at..at + 1] == "A" { "B" } else { "A" };
        text.replace_range(at..at + 1, flipped);
        fs::write(&file, text).unwrap();
        let held = w.pull(&mut b, &w.b, 2);
        assert!(held.records.is_empty());
        assert!(held.events[0].contains("segment 1 of a"));
        assert_eq!(w.pull(&mut b, &w.b, 3).events.len(), 0);
    }

    #[test]
    fn a_revoked_device_is_read_no_further_than_what_was_applied_when_it_was_dropped() {
        let w = World::new("revoked");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        save(&w.root('b'), &ulid(2), "plan-b.md", Some("personal"), "b1");
        w.pull(&mut b, &w.b, 0);
        w.push(&mut b, &w.b, 1);
        assert_eq!(w.pull(&mut a, &w.a, 2).records.len(), 1);
        let from = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        let opened = manifest::open(&from, &Recipient::device(&w.a.device))
            .unwrap()
            .unwrap();
        let lock = manifest::lock(&w.root('a')).unwrap();
        manifest::revoke(&lock, &from, &opened, &w.a, &w.b.device.id()).unwrap();
        drop(lock);
        let after = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        let lock = manifest::lock(&w.root('a')).unwrap();
        for v in &after.versions {
            manifest::confirm(&lock, &w.scope, v.manifest.n, &v.bytes).unwrap();
        }
        drop(lock);
        save(&w.root('b'), &ulid(2), "plan-b.md", Some("personal"), "b2");
        assert_eq!(w.push(&mut b, &w.b, 3).segments, vec![2]);
        let late = w.pull(&mut a, &w.a, 4);
        assert!(late.records.is_empty());
        assert!(late.events.is_empty());
        assert_eq!(a.state.cutoffs.get(&w.b.device.id()), Some(&1));
        assert!(w.pull(&mut a, &w.a, 5).events.is_empty());
    }

    #[test]
    fn a_store_that_lost_its_state_reads_a_dropped_device_as_far_as_the_others_acknowledged() {
        let w = World::new("dropped-resume");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.pull(&mut b, &w.b, 0);
        for i in 1..=2 {
            save(
                &w.root('b'),
                &ulid(2),
                "plan-b.md",
                Some("personal"),
                &format!("b{i}"),
            );
            w.push(&mut b, &w.b, i);
        }
        assert_eq!(w.pull(&mut a, &w.a, 3).records.len(), 2);
        w.pull(&mut a, &w.a, 3);
        w.pull(&mut a, &w.a, 3);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "a1");
        assert_eq!(w.push(&mut a, &w.a, 4).segments, vec![1]);
        save(&w.root('b'), &ulid(2), "plan-b.md", Some("personal"), "b3");
        assert_eq!(w.push(&mut b, &w.b, 5).segments, vec![3]);
        let from = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        let opened = manifest::open(&from, &Recipient::device(&w.a.device))
            .unwrap()
            .unwrap();
        let lock = manifest::lock(&w.root('a')).unwrap();
        manifest::revoke(&lock, &from, &opened, &w.a, &w.b.device.id()).unwrap();
        let after = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        for v in &after.versions {
            manifest::confirm(&lock, &w.scope, v.manifest.n, &v.bytes).unwrap();
        }
        drop(lock);
        forget(&w, 'a');
        let mut again = w.replica('a');
        let pulled = w.pull(&mut again, &w.a, 10);
        assert_eq!(pulled.records.len(), 3);
        assert!(pulled.events.is_empty(), "{:?}", pulled.events);
        assert_eq!(again.state.cursors[&w.b.device.id()], 2);
    }

    #[test]
    fn a_device_added_without_a_rotation_is_read_at_once() {
        let w = World::staged("added");
        let scope = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        assert_eq!(scope.versions.len(), 2);
        assert!(!scope.versions[0].manifest.lists(&w.b.device.id()));
        assert_eq!(
            scope.versions[0].manifest.epoch,
            scope.versions[1].manifest.epoch
        );
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut b, &w.b, 0);
        save(&w.root('b'), &ulid(2), "plan-b.md", Some("personal"), "b1");
        w.push(&mut b, &w.b, 1);
        assert_eq!(w.pull(&mut a, &w.a, 2).records.len(), 1);
    }

    #[test]
    fn a_pending_version_that_introduces_an_epoch_holds_every_push() {
        let w = World::new("pending");
        let from = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        let opened = manifest::open(&from, &Recipient::device(&w.a.device))
            .unwrap()
            .unwrap();
        let lock = manifest::lock(&w.root('a')).unwrap();
        manifest::revoke(&lock, &from, &opened, &w.a, &w.b.device.id()).unwrap();
        drop(lock);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        assert!(w.push(&mut a, &w.a, 1).segments.is_empty());
        let after = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        let lock = manifest::lock(&w.root('a')).unwrap();
        for v in &after.versions {
            manifest::confirm(&lock, &w.scope, v.manifest.n, &v.bytes).unwrap();
        }
        drop(lock);
        assert_eq!(w.push(&mut a, &w.a, 2).segments, vec![1]);
        let raw = fs::read_to_string(w.segment_file(&w.a, 1)).unwrap();
        assert!(raw.contains("\"epoch\":2"));
    }

    #[test]
    fn a_large_first_push_splits_into_segments_of_at_most_eight_mebibytes() {
        let w = World::new("large");
        let big = "x".repeat(1000 * 1000);
        for i in 0..22 {
            save(
                &w.root('a'),
                &ulid(i + 1),
                "plan-x.md",
                Some("personal"),
                &format!("{i}{big}"),
            );
        }
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        let pushed = w.push(&mut a, &w.a, 1);
        assert!(pushed.segments.len() >= 3);
        for seq in &pushed.segments {
            let size = fs::metadata(w.segment_file(&w.a, *seq)).unwrap().len();
            assert!(size <= segment::MAX_BYTES as u64);
        }
        assert_eq!(w.pull(&mut b, &w.b, 2).records.len(), 22);
    }

    #[test]
    fn a_small_push_is_one_segment() {
        let w = World::new("small");
        save(
            &w.root('a'),
            &ulid(1),
            "plan-x.md",
            Some("personal"),
            &"y".repeat(4096),
        );
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        assert_eq!(w.push(&mut a, &w.a, 1).segments, vec![1]);
    }

    /// A transport that refuses or loses the next creates it is told to.
    struct Flaky {
        inner: Folder,
        full: AtomicU32,
        lost: AtomicU32,
    }

    impl Flaky {
        fn new(inner: Folder, full: u32, lost: u32) -> Flaky {
            Flaky {
                inner,
                full: AtomicU32::new(full),
                lost: AtomicU32::new(lost),
            }
        }
    }

    fn take(counter: &AtomicU32) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }

    impl Transport for Flaky {
        fn reachable(&self) -> Result<(), String> {
            self.inner.reachable()
        }
        fn keeps(&self) -> bool {
            self.inner.keeps()
        }
        fn scopes(&self) -> Result<Vec<String>, String> {
            self.inner.scopes()
        }
        fn devices(&self, scope: &str) -> Result<Vec<String>, String> {
            self.inner.devices(scope)
        }
        fn list_after(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
            self.inner.list_after(scope, device, cursor)
        }
        fn probe(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
            self.inner.probe(scope, device, cursor)
        }
        fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
            self.inner.get(path)
        }
        fn create(&self, path: &str, bytes: &[u8]) -> Put {
            if take(&self.full) {
                return Put::Full("No space left on device".into());
            }
            if take(&self.lost) {
                return Put::Unreachable("the volume went away".into());
            }
            self.inner.create(path, bytes)
        }
        fn highest_manifest(&self, scope: &str) -> Result<Option<u64>, String> {
            self.inner.highest_manifest(scope)
        }
        fn replace(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
            self.inner.replace(path, bytes)
        }
        fn sweep(&self, now: SystemTime) -> Result<(), String> {
            self.inner.sweep(now)
        }
        fn remove_mailbox(&self, nameplate: &str) -> Result<(), String> {
            self.inner.remove_mailbox(nameplate)
        }
    }

    #[test]
    fn a_full_transport_keeps_the_segment_in_the_outbox_and_retries() {
        let w = World::new("full");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        let t = Flaky::new(w.folder(&w.a), 1, 0);
        let pushed = a.push(&t, &w.a, &w.settings, ts(1)).unwrap();
        assert_eq!(pushed.full.as_deref(), Some("No space left on device"));
        assert!(pushed.segments.is_empty());
        assert!(out_path(&w.root('a'), &w.scope, 1).exists());
        let retry = a.push(&t, &w.a, &w.settings, ts(2)).unwrap();
        assert_eq!(retry.segments, vec![1]);
        assert!(retry.full.is_none());
    }

    #[test]
    fn a_crash_between_the_outbox_and_the_create_creates_the_same_bytes_again() {
        let w = World::new("crash");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        let t = Flaky::new(w.folder(&w.a), 0, 1);
        assert_eq!(
            a.push(&t, &w.a, &w.settings, ts(1)).unwrap_err(),
            "the volume went away"
        );
        let kept = fs::read(out_path(&w.root('a'), &w.scope, 1)).unwrap();
        let mut restarted = w.replica('a');
        w.pull(&mut restarted, &w.a, 2);
        let pushed = w.push(&mut restarted, &w.a, 3);
        assert_eq!(pushed.segments, vec![1]);
        assert_eq!(fs::read(w.segment_file(&w.a, 1)).unwrap(), kept);
        assert!(w.push(&mut restarted, &w.a, 4).segments.is_empty());
    }

    #[test]
    fn a_segment_created_before_a_crash_booked_it_counts_as_done() {
        let w = World::new("booked");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let state = state_path(&w.root('a'), &w.scope);
        let seen = seen_path(&w.root('a'), &w.scope);
        fs::remove_file(&state).unwrap();
        fs::remove_file(&seen).unwrap();
        let mut again = w.replica('a');
        again.resuming = false;
        w.pull(&mut again, &w.a, 2);
        let pushed = w.push(&mut again, &w.a, 3);
        assert_eq!(pushed.segments, vec![1]);
        assert!(again.stopped().is_none());
        assert_eq!(again.state.own, 1);
        assert!(w.push(&mut again, &w.a, 4).segments.is_empty());
        assert!(!w.segment_file(&w.a, 2).exists());
    }

    #[test]
    fn an_oversize_record_is_reported_once_and_never_retried_in_a_loop() {
        let w = World::new("oversize");
        let note = ulid(1);
        let first = save(&w.root('a'), &note, "plan-x.md", Some("personal"), "one");
        let lock = versions::lock(&w.root('a')).unwrap();
        let mut heavy = first.clone();
        heavy.version = "e".repeat(64);
        heavy.parents = vec![first.version];
        heavy.dropped = vec![versions::Dropped {
            passage: "p".into(),
            lines: vec!["z".repeat(9 * 1024 * 1024)],
        }];
        versions::append(&lock, &note, &heavy).unwrap();
        drop(lock);
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        let pushed = w.push(&mut a, &w.a, 1);
        assert_eq!(pushed.events.len(), 1, "{:?}", pushed.events);
        assert!(
            pushed.events[0].contains("cannot push"),
            "{:?}",
            pushed.events
        );
        let again = w.push(&mut a, &w.a, 2);
        assert!(again.events.is_empty(), "{:?}", again.events);
    }

    #[test]
    fn a_stopped_scope_pushes_nothing_and_shows_its_line() {
        let w = World::new("stopped");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        a.set_stopped(Some("sync personal: the manifest pins x".into()))
            .unwrap();
        assert!(w.push(&mut a, &w.a, 1).segments.is_empty());
        let shown = status(&w.root('a'), &w.scope).unwrap().unwrap();
        assert_eq!(
            shown.stopped.as_deref(),
            Some("sync personal: the manifest pins x")
        );
        a.set_stopped(None).unwrap();
        assert_eq!(w.push(&mut a, &w.a, 2).segments, vec![1]);
    }

    fn guard(w: &World, at: i64) -> Guard {
        known_heads(&w.root('a'), &w.settings, ts(at)).unwrap()
    }

    #[test]
    fn a_device_that_acknowledged_nothing_holds_back_every_version_until_it_does_or_goes_stale() {
        let w = World::new("heads");
        let note = ulid(1);
        save(&w.root('a'), &note, "plan-x.md", Some("personal"), "one");
        let two = save(&w.root('a'), &note, "plan-x.md", Some("personal"), "two");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        assert_eq!(guard(&w, 2).get(&note), Some(&Hold::All));
        w.pull(&mut b, &w.b, 3);
        w.pull(&mut b, &w.b, 4);
        w.pull(&mut b, &w.b, 4);
        w.push(&mut b, &w.b, 5);
        w.pull(&mut a, &w.a, 6);
        let held = guard(&w, 7);
        assert_eq!(
            held.get(&note),
            Some(&Hold::After(BTreeSet::from([two.version.clone()])))
        );
        let c = 181 * 24 * 3600;
        save(&w.root('a'), &note, "plan-x.md", Some("personal"), "three");
        w.push(&mut a, &w.a, 10);
        assert!(guard(&w, 10).contains_key(&note));
        assert!(!guard(&w, 10 + c).contains_key(&note));
    }

    #[test]
    fn staleness_counts_only_this_devices_own_segments_by_its_own_clock() {
        let w = World::new("stale");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 0);
        let others = BTreeSet::from([w.b.device.id()]);
        let days = |d: i64| T0 + d * 24 * 3600;
        assert!(a.state.stale(&others, days(179), 180).is_empty());
        assert!(
            a.state
                .stale(&others, days(181), 180)
                .contains(&w.b.device.id())
        );
        a.state
            .acks
            .entry(w.b.device.id())
            .or_default()
            .insert(w.a.device.id(), 1);
        assert!(a.state.stale(&others, days(400), 180).is_empty());
    }

    #[test]
    fn the_seen_file_drops_what_no_log_holds() {
        let w = World::new("trim");
        let note = ulid(1);
        save(&w.root('a'), &note, "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        assert_eq!(a.trim_seen().unwrap(), 0);
        fs::remove_file(versions::log_path(&w.root('a'), &note)).unwrap();
        assert_eq!(a.trim_seen().unwrap(), 2);
        assert_eq!(fs::read(seen_path(&w.root('a'), &w.scope)).unwrap(), b"");
    }

    #[test]
    fn the_outbox_waits_for_every_live_device_and_a_stale_one_does_not_hold_it() {
        let w = World::three("outbox");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 0);
        let kept = out_path(&w.root('a'), &w.scope, 1);
        let (b, c) = (w.b.device.id(), w.c.device.id());
        a.state
            .acks
            .entry(b)
            .or_default()
            .insert(w.a.device.id(), 1);
        w.push(&mut a, &w.a, 24 * 3600);
        assert!(kept.exists(), "c is live and has not acknowledged");
        w.push(&mut a, &w.a, 200 * 24 * 3600);
        assert!(!kept.exists(), "c is stale and b acknowledged");
        assert!(
            a.state
                .stale(&BTreeSet::from([c]), T0 + 200 * 24 * 3600, 180)
                .len()
                == 1
        );
    }

    #[test]
    fn records_of_another_scope_in_a_segment_are_skipped_with_a_line() {
        let w = World::new("mismatch");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        let own = w.push(&mut a, &w.a, 1);
        assert_eq!(own.segments, vec![1]);
        let mut forged = Plaintext::new("now");
        let text = format!(
            "---\nid: {}\ncreated: {AT}\nscope: work\n---\n\n# x\n",
            ulid(5)
        );
        let forged_blob = Blob::new(text.as_bytes());
        let v = {
            let parents: Vec<String> = Vec::new();
            let id = versions::version_id(&ulid(5), &parents, "plan-w.md", &forged_blob.hash);
            Version {
                version: id,
                file: "plan-w.md".into(),
                blob: forged_blob.hash.clone(),
                event: versions::ADDED.into(),
                at: AT.into(),
                ..Version::default()
            }
        };
        forged.records.push(Record {
            note: ulid(5),
            version: v,
        });
        forged.blobs.push(forged_blob);
        let scope = manifest::read_scope(&w.root('a'), &w.scope).unwrap();
        let opened = manifest::open(&scope, &Recipient::device(&w.a.device))
            .unwrap()
            .unwrap();
        let bytes =
            segment::seal(&w.scope, &w.a.device.sign, 2, 1, &opened.keys[&1], &forged).unwrap();
        fs::write(w.segment_file(&w.a, 2), bytes).unwrap();
        let pulled = w.pull(&mut b, &w.b, 2);
        assert_eq!(pulled.records.len(), 1);
        assert!(pulled.events.iter().any(|e| e.contains("scope: personal")));
        assert_eq!(b.state.cursors[&w.a.device.id()], 2);
    }

    #[test]
    fn a_pull_changes_nothing_on_disk_until_it_is_committed() {
        let w = World::new("commit");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let t = w.folder(&w.b);
        let first = b.pull(&t, &w.b, ts(2)).unwrap();
        assert_eq!(first.records.len(), 1);
        let second = b.pull(&t, &w.b, ts(3)).unwrap();
        assert_eq!(second.records.len(), 1);
        assert!(!seen_path(&w.root('b'), &w.scope).exists());
        b.commit(&second).unwrap();
        assert!(b.pull(&t, &w.b, ts(4)).unwrap().records.is_empty());
    }

    fn blob_file(root: &Path, blob: &str) -> PathBuf {
        let dir = store::history_dir(root).join("blobs").join(&blob[..2]);
        dir.join(&blob[2..])
    }

    #[test]
    fn a_store_opened_under_another_device_id_resumes_and_leaves_the_old_files_aside() {
        let w = World::new("newdevice");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        for i in 1..=2 {
            save(
                &w.root('a'),
                &ulid(1),
                "plan-x.md",
                Some("personal"),
                &format!("v{i}"),
            );
            assert_eq!(w.push(&mut a, &w.a, i).segments, vec![i as u64]);
        }
        let mut again =
            Replica::open(&w.root('a'), &w.scope, "personal", &w.b.device.id()).unwrap();
        assert!(again.resuming);
        let dir = scope_dir(&w.root('a'), &w.scope);
        assert!(!dir.join("out").exists() && !dir.join("seen.jsonl").exists());
        let aside: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".old-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert!(aside[0].path().join("out").exists());
        let t = w.folder(&w.b);
        for at in 3..=5 {
            let pulled = again.pull(&t, &w.b, ts(at)).unwrap();
            again.commit(&pulled).unwrap();
        }
        save(&w.root('a'), &ulid(2), "plan-y.md", Some("personal"), "new");
        let pushed = again.push(&t, &w.b, &w.settings, ts(6)).unwrap();
        assert_eq!(pushed.segments, vec![1]);
        assert!(w.segment_file(&w.b, 1).exists() && !w.segment_file(&w.b, 2).exists());
        let mut reader = w.replica('a');
        let seen = w.pull(&mut reader, &w.a, 7);
        assert!(seen.events.is_empty(), "{:?}", seen.events);
        assert_eq!(seen.records.len(), 3);
    }

    #[test]
    fn an_unbooked_segment_that_the_peer_acknowledged_stays_in_the_outbox_and_nothing_halts() {
        let w = World::new("unbooked-ack");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        assert_eq!(w.push(&mut a, &w.a, 1).segments, vec![1]);
        a.state.own = 0;
        a.state.sent.clear();
        a.state.cursors.remove(&w.a.device.id());
        a.state.save(&w.root('a'), &w.scope).unwrap();
        for at in [2, 3, 3] {
            w.pull(&mut b, &w.b, at);
        }
        assert_eq!(w.push(&mut b, &w.b, 4).segments, vec![1]);
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 5);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "two");
        let pushed = w.push(&mut a, &w.a, 6);
        assert!(
            pushed.events.is_empty() && a.stopped().is_none(),
            "{:?}",
            pushed.events
        );
        assert_eq!(pushed.segments, vec![1, 2]);
        assert!(out_path(&w.root('a'), &w.scope, 2).exists());
    }

    #[test]
    fn a_blob_that_cannot_be_read_sends_no_left_and_is_pushed_once_it_can() {
        let w = World::new("blobgone");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let two = save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "two");
        let file = blob_file(&w.root('a'), &two.blob);
        let bytes = fs::read(&file).unwrap();
        fs::remove_file(&file).unwrap();
        assert!(w.push(&mut a, &w.a, 2).segments.is_empty());
        assert!(
            w.pull(&mut b, &w.b, 3)
                .records
                .iter()
                .all(|r| !r.version.is_left())
        );
        fs::write(&file, bytes).unwrap();
        assert_eq!(w.push(&mut a, &w.a, 4).segments, vec![2]);
        let got = w.pull(&mut b, &w.b, 5);
        assert_eq!(versions_of(&got), vec![two.version.as_str()]);
    }

    #[test]
    fn the_ack_limit_of_a_dropped_device_counts_only_listed_ackers_and_this_device() {
        let mut acks: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
        let put = |acks: &mut BTreeMap<String, BTreeMap<String, u64>>, from: &str, to: &str, n| {
            acks.entry(from.to_string())
                .or_default()
                .insert(to.to_string(), n);
        };
        put(&mut acks, "e", "c", 13);
        put(&mut acks, "d", "c", 50);
        put(&mut acks, "c", "c", 99);
        put(&mut acks, "me", "c", 4);
        let listed = BTreeSet::from(["e".to_string()]);
        assert_eq!(ack_limit(&acks, "c", &listed, "me"), 13);
        assert_eq!(ack_limit(&acks, "c", &BTreeSet::new(), "me"), 4);
        assert_eq!(ack_limit(&acks, "zz", &listed, "me"), 0);
    }

    #[test]
    fn status_after_a_gap_names_the_device_the_seq_and_why_then_clears() {
        let w = World::new("status-gap");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        for i in 1..=3 {
            save(
                &w.root('a'),
                &ulid(1),
                "plan-x.md",
                Some("personal"),
                &format!("v{i}"),
            );
            w.push(&mut a, &w.a, i);
        }
        let two = w.segment_file(&w.a, 2);
        let bytes = fs::read(&two).unwrap();
        fs::remove_file(&two).unwrap();
        w.pull(&mut b, &w.b, 10);
        let shown = status(&w.root('b'), &w.scope).unwrap().unwrap();
        let stop = &shown.stops[&w.a.device.id()];
        assert_eq!(
            (stop.seq, stop.why.as_str(), stop.replaceable),
            (2, "missing", true)
        );
        assert_eq!(shown.pulled_at, Some(T0 + 10));
        fs::write(&two, bytes).unwrap();
        w.pull(&mut b, &w.b, 11);
        assert!(
            status(&w.root('b'), &w.scope)
                .unwrap()
                .unwrap()
                .stops
                .is_empty()
        );
        let aa = status(&w.root('a'), &w.scope).unwrap().unwrap();
        assert_eq!(aa.pushed_at, Some(T0 + 3));
        assert_eq!(aa.sent.len(), 3);
        assert!(aa.acked.is_empty());
    }

    #[test]
    fn status_shows_a_refused_segment_and_the_reported_problem() {
        let w = World::new("status-refused");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let file = w.segment_file(&w.a, 1);
        let bytes = fs::read(&file).unwrap();
        fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
        w.pull(&mut b, &w.b, 2);
        let stop = status(&w.root('b'), &w.scope).unwrap().unwrap().stops;
        assert_eq!(stop[&w.a.device.id()].seq, 1);
        assert!(stop[&w.a.device.id()].why.contains("JSON"), "{stop:?}");
        let problem = Problem {
            kind: "full".into(),
            since: T0 + 5,
            message: "No space left on device".into(),
        };
        b.set_error(Some(problem.clone())).unwrap();
        assert_eq!(
            status(&w.root('b'), &w.scope).unwrap().unwrap().error,
            Some(problem)
        );
        b.set_error(None).unwrap();
        assert!(
            status(&w.root('b'), &w.scope)
                .unwrap()
                .unwrap()
                .error
                .is_none()
        );
    }

    #[test]
    fn a_stopped_replica_pulls_nothing() {
        let w = World::new("stoppedpull");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        b.set_stopped(Some(
            "sync personal: this device was removed from the scope".into(),
        ))
        .unwrap();
        assert!(w.pull(&mut b, &w.b, 2).records.is_empty());
        b.set_stopped(None).unwrap();
        assert_eq!(w.pull(&mut b, &w.b, 3).records.len(), 1);
    }

    #[test]
    fn a_left_record_names_the_parents_that_are_not_in_the_scope_as_outside() {
        let w = World::new("leftoutside");
        let note = ulid(1);
        let first = save(&w.root('a'), &note, "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        let elsewhere = save(&w.root('a'), &note, "plan-x.md", Some("work"), "one");
        let lock = versions::lock(&w.root('a')).unwrap();
        let text = format!("---\nid: {note}\ncreated: {AT}\nscope: work\n---\n\n# merged\n");
        let parents = vec![first.version.clone(), elsewhere.version.clone()];
        let merged = versions::record(
            &lock,
            &note,
            &parents,
            "plan-x.md",
            Some(text.as_bytes()),
            versions::MERGED,
            AT,
        )
        .unwrap();
        drop(lock);
        w.push(&mut a, &w.a, 2);
        let got = w.pull(&mut b, &w.b, 3);
        let left = got
            .records
            .iter()
            .find(|r| r.version.version == merged.version)
            .unwrap();
        assert!(left.version.is_left());
        assert_eq!(left.version.outside, vec![elsewhere.version.clone()]);
        let third = got
            .records
            .iter()
            .find(|r| r.version.version == elsewhere.version)
            .unwrap();
        assert!(third.version.is_left());
    }

    #[test]
    fn a_device_listed_only_recently_is_not_stale_for_a_segment_older_than_it() {
        let w = World::new("since");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 0);
        let others = BTreeSet::from([w.b.device.id()]);
        let days = |d: i64| T0 + d * 24 * 3600;
        a.state.since.insert(w.b.device.id(), days(199));
        assert!(a.state.stale(&others, days(200), 180).is_empty());
        assert_eq!(a.state.stale(&others, days(199 + 181), 180).len(), 1);
    }

    #[test]
    fn an_oversize_part_does_not_hold_back_the_notes_after_it() {
        let w = World::new("oversize-rest");
        let note = ulid(1);
        let first = save(&w.root('a'), &note, "plan-x.md", Some("personal"), "one");
        let lock = versions::lock(&w.root('a')).unwrap();
        let mut heavy = first.clone();
        heavy.version = "e".repeat(64);
        heavy.parents = vec![first.version];
        heavy.dropped = vec![versions::Dropped {
            passage: "p".into(),
            lines: vec!["z".repeat(9 * 1024 * 1024)],
        }];
        versions::append(&lock, &note, &heavy).unwrap();
        drop(lock);
        save(&w.root('a'), &ulid(2), "plan-y.md", Some("personal"), "two");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        let pushed = w.push(&mut a, &w.a, 1);
        assert_eq!(pushed.events.len(), 1);
        let got = w.pull(&mut b, &w.b, 2);
        assert!(got.records.iter().any(|r| r.note == ulid(2)));
        assert!(
            got.records
                .iter()
                .all(|r| r.version.version != heavy.version)
        );
    }

    #[test]
    fn status_after_a_resume_does_not_list_this_device_as_an_acker() {
        let w = World::new("status-resume");
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 0);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        w.push(&mut a, &w.a, 1);
        forget(&w, 'a');
        let mut a = w.replica('a');
        w.pull(&mut a, &w.a, 2);
        let shown = status(&w.root('a'), &w.scope).unwrap().unwrap();
        assert!(shown.acked.is_empty(), "{:?}", shown.acked);
    }

    #[test]
    fn an_acknowledgement_created_after_a_crash_counts_for_the_hour() {
        let w = World::new("finish-ack");
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        let mut a = w.replica('a');
        let mut b = w.replica('b');
        w.pull(&mut a, &w.a, 0);
        w.push(&mut a, &w.a, 1);
        for at in [2, 3, 3] {
            w.pull(&mut b, &w.b, at);
        }
        let lost = Flaky::new(w.folder(&w.b), 0, 1);
        assert!(b.push(&lost, &w.b, &w.settings, ts(4)).is_err());
        let mut b = w.replica('b');
        for at in [5, 6, 6] {
            w.pull(&mut b, &w.b, at);
        }
        assert_eq!(w.push(&mut b, &w.b, 7).segments, vec![1]);
        assert_eq!(b.state.last_ack, Some(T0 + 7));
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "two");
        w.push(&mut a, &w.a, 8);
        w.pull(&mut b, &w.b, 9);
        assert!(w.push(&mut b, &w.b, 10).segments.is_empty());
    }

    #[test]
    fn the_natural_flow_of_two_stores_with_one_key_halts_the_established_store() {
        let w = World::new("natural");
        let mut first = w.replica('a');
        w.pull(&mut first, &w.a, 0);
        save(&w.root('a'), &ulid(1), "plan-x.md", Some("personal"), "one");
        assert_eq!(w.push(&mut first, &w.a, 1).segments, vec![1]);
        let other = w.second_store();
        let mut second = Replica::open(&other, &w.scope, "personal", &w.a.device.id()).unwrap();
        let t = w.folder(&w.a);
        for at in 2..=4 {
            let p = second.pull(&t, &w.a, ts(at)).unwrap();
            second.commit(&p).unwrap();
        }
        save(&other, &ulid(9), "plan-z.md", Some("personal"), "other");
        let pushed = second.push(&t, &w.a, &w.settings, ts(5)).unwrap();
        assert_eq!(pushed.segments, vec![2]);
        assert!(pushed.events.is_empty() && second.stopped().is_none());
        let seen = w.pull(&mut first, &w.a, 6);
        assert_eq!(seen.events.len(), 1);
        assert!(first.stopped().is_some());
    }
}
