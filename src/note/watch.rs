//! `bilbo watch`: records a version of each note when it changes, and runs the sync cycle of each syncing scope.
//! `notify` only says when to look; every decision comes from a scan of `<root>/notes/` against history, so a missed
//! event costs time, never content.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::event::AccessKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::Failure;
use crate::host::swap;
use crate::identity::keys::{self, Identity};
use crate::identity::manifest::{self, Recipient};
use crate::note::versions::{self, Found, Lock, Scan, Skip};
use crate::shared::{config, hash, store};
use crate::sync::replica::{self, Problem, Replica};
use crate::sync::transport::{self, Transport};
use crate::sync::{integrate, manifests};

/// How long `notes/` stays quiet before a scan, and the longest a scan waits while events keep coming.
const QUIET: Duration = Duration::from_secs(2);
const CAP: Duration = Duration::from_secs(10);
/// How often the folder and the lock file are checked, whatever the events say.
const CHECK: Duration = Duration::from_secs(10);
const BACKSTOP: Duration = Duration::from_secs(600);
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 3600);
/// The longest a failing sync cycle waits for the next one.
const POLL_CAP: Duration = Duration::from_secs(600);
const LOCK_TRIES: u32 = 10;
const LOCK_WAIT: Duration = Duration::from_millis(100);
/// A file written this recently may be written again within the clock's resolution, so its stat proves nothing.
const RACY_SECS: i64 = 2;

/// Runs until stopped, so it returns only when it cannot start or loses its lock file.
pub fn run(args: &[String], env: &store::Env, say: &mut dyn FnMut(&str)) -> Result<(), Failure> {
    if let Some(arg) = args.first() {
        return Err(Failure::Usage(if arg.starts_with('-') {
            format!("unknown option '{arg}'")
        } else {
            format!("unexpected argument '{arg}'")
        }));
    }
    let settings = config::load(env).map_err(Failure::Config)?;
    let root = store::root(env).map_err(Failure::Config)?;
    if !root.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let lock = take_lock(&root, say)?;
    let (tx, rx) = mpsc::channel();
    let mut watch = Watch {
        notes: root.join("notes"),
        root,
        env,
        settings,
        config_error: None,
        syncing: BTreeSet::new(),
        scopes: HashMap::new(),
        held: HashMap::new(),
        inbound: Vec::new(),
        cycle_lines: HashSet::new(),
        wait: Duration::ZERO,
        more: false,
        trim: false,
        lock,
        say,
        tx,
        rx,
        watcher: None,
        cache: HashMap::new(),
        stats: HashMap::new(),
        digests: HashMap::new(),
        degraded: false,
        printed: HashMap::new(),
        outage: false,
        empty: false,
        leftovers: HashSet::new(),
        last_error: None,
    };
    watch.housekeeping();
    watch.serve()
}

fn same_file(file: &File, path: &Path) -> bool {
    match (file.metadata(), fs::metadata(path)) {
        (Ok(held), Ok(now)) => (held.dev(), held.ino()) == (now.dev(), now.ino()),
        _ => false,
    }
}

/// Takes `watch.lock`: waits a second for it, and when another watcher still holds it, says so and blocks until it
/// frees. The file taken must still be the one at the path, or `.bilbo/` was deleted under it and the new one is taken.
fn take_lock(root: &Path, say: &mut dyn FnMut(&str)) -> Result<File, Failure> {
    let path = versions::watch_lock_path(root);
    let refused = |what: &str, e: std::io::Error| {
        Failure::Refused(format!("cannot {what} {}: {e}", path.display()))
    };
    let mut announced = false;
    loop {
        let dir = path.parent().expect("the lock file has a folder");
        fs::create_dir_all(dir).map_err(|e| refused("create", e))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| refused("open", e))?;
        if !try_briefly(&file).map_err(|e| refused("lock", e))? {
            if !announced {
                announced = true;
                say(&format!(
                    "bilbo watch is already running for {}; waiting",
                    root.display()
                ));
            }
            file.lock().map_err(|e| refused("lock", e))?;
        }
        if same_file(&file, &path) {
            return Ok(file);
        }
    }
}

/// Tries the lock up to `LOCK_TRIES` times, `LOCK_WAIT` apart: a `bilbo history` probe holds it for an instant.
fn try_briefly(file: &File) -> std::io::Result<bool> {
    for attempt in 0..LOCK_TRIES {
        match file.try_lock() {
            Ok(()) => return Ok(true),
            Err(TryLockError::WouldBlock) => {
                if attempt + 1 < LOCK_TRIES {
                    std::thread::sleep(LOCK_WAIT);
                }
            }
            Err(TryLockError::Error(e)) => return Err(e),
        }
    }
    Ok(false)
}

/// What a file's stat says changed: any write moves the ctime, which no tool can set.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stat {
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
    ino: u64,
}

impl Stat {
    fn of(meta: &fs::Metadata) -> Stat {
        Stat {
            len: meta.len(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
            ino: meta.ino(),
        }
    }

    fn racy(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.ctime.0 >= now - RACY_SECS || self.mtime.0 >= now - RACY_SECS
    }
}

/// A note file as the cache keeps it: its content as a digest, so the cache holds no bytes.
#[derive(Clone)]
struct Entry {
    name: String,
    id: String,
    digest: String,
}

type Classified = Result<Entry, Skip>;

/// What the watcher keeps of one syncing scope between cycles.
struct Active {
    /// The scope id the replica is of, and the device it was opened for.
    id: String,
    device: String,
    replica: Replica,
    /// The transport problem being reported, kept for the time it began.
    problem: Option<Problem>,
}

/// A scope whose manifests let it sync this cycle.
struct Ready {
    name: String,
    url: String,
    t: Box<dyn Transport>,
    /// Every step of the cycle so far worked.
    ok: bool,
    /// The transport's message when it refused a write as full.
    full: Option<String>,
}

/// What one sync cycle did: how its scan went, when it ran one, and how long until the next cycle.
struct Tick {
    scanned: Option<Result<usize, String>>,
    wait: Duration,
}

struct Watch<'a> {
    root: PathBuf,
    notes: PathBuf,
    env: &'a store::Env,
    /// The settings of the last config that parsed, read again at the start of every sync cycle.
    settings: config::Settings,
    config_error: Option<String>,
    /// The scopes that sync on this device: declared with a URL, with a device key held.
    syncing: BTreeSet<String>,
    /// What the watcher keeps of each syncing scope between cycles, by name.
    scopes: HashMap<String, Active>,
    /// Lines that hold while a condition does, by scope and channel, printed once until it ends.
    held: HashMap<String, String>,
    /// The lines this cycle's staging and applying returned, printed when the last cycle did not return them.
    inbound: Vec<String>,
    cycle_lines: HashSet<String>,
    /// The wait before the next sync cycle: the poll interval, doubled while a cycle fails.
    wait: Duration,
    /// A pull stopped at its size limit with segments left: the next cycle comes at once.
    more: bool,
    /// A prune ran: the next cycle trims `seen.jsonl` once it has applied what it pulled.
    trim: bool,
    lock: File,
    say: &'a mut dyn FnMut(&str),
    tx: Sender<notify::Result<notify::Event>>,
    rx: Receiver<notify::Result<notify::Event>>,
    /// The watch on `notes/` and the folder it was made on; `None` while the folder cannot be read.
    watcher: Option<(RecommendedWatcher, (u64, u64))>,
    /// Each file's last reading, kept while its stat is unchanged.
    cache: HashMap<String, (Stat, Classified)>,
    stats: HashMap<String, Stat>,
    /// The digest of each accepted file's bytes, by name, as of the last reading.
    digests: HashMap<String, String>,
    /// The folder lists but the watch could not be made: scans follow the 10-second check.
    degraded: bool,
    /// The skip lines printed, by file name, with the reason and stat they were printed for.
    printed: HashMap<String, (String, Option<Stat>)>,
    outage: bool,
    empty: bool,
    leftovers: HashSet<String>,
    last_error: Option<String>,
}

impl Watch<'_> {
    fn say(&mut self, message: &str) {
        (self.say)(message);
    }

    /// The `.tmp-*` leftovers under `history/`, `sync/` and `scopes/` go, and the first prune runs.
    fn housekeeping(&mut self) {
        // The first cycle has not read the keys yet, so the config alone says which scopes sync.
        self.syncing = (self.settings.scopes.iter())
            .filter(|s| s.sync != "off")
            .map(|s| s.name.clone())
            .collect();
        let result = versions::lock(&self.root).and_then(|lock| {
            versions::sweep_temporaries(&lock)?;
            integrate::sweep_temporaries(&self.root)?;
            self.with_params(|params| integrate::rebuild_open(&lock, params))?;
            self.prune_under(&lock)
        });
        if let Err(e) = result {
            self.say(&e);
        }
    }

    fn prune_under(&mut self, lock: &Lock) -> Result<(), String> {
        let now = jiff::Timestamp::now();
        let guard = replica::known_heads(&self.root, &self.settings, now)?;
        let staged = integrate::staged_blobs(&self.root)?;
        let pruned = versions::prune(lock, self.settings.history.keep_days, now, &guard, &staged)?;
        self.trim = true;
        for warning in &pruned.warnings {
            self.say(warning);
        }
        if pruned.versions > 0 {
            self.say(&format!(
                "pruned {} versions older than {}",
                pruned.versions, pruned.cutoff
            ));
        }
        Ok(())
    }

    fn prune(&mut self) {
        if let Err(e) = versions::lock(&self.root).and_then(|lock| self.prune_under(&lock)) {
            self.say(&e);
        }
    }

    fn serve(&mut self) -> Result<(), Failure> {
        let start = Instant::now();
        let (mut next_check, mut next_backstop, mut next_prune) =
            (start + CHECK, start + BACKSTOP, start + PRUNE_EVERY);
        let mut pending: Option<(Instant, Instant)> = None;
        let mut retry: Option<Instant> = None;
        let mut next_poll = start;
        let mut scan = self.connect(true);
        loop {
            if Instant::now() >= next_poll {
                self.check_lock()?;
                let tick = self.tick();
                next_poll = Instant::now() + tick.wait;
                if let Some(result) = tick.scanned {
                    scan = false;
                    pending = None;
                    retry = None;
                    next_backstop = Instant::now() + BACKSTOP;
                    if let Err(e) = result {
                        self.report(&e);
                        retry = Some(Instant::now() + CHECK);
                    }
                }
            }
            if scan {
                scan = false;
                pending = None;
                retry = None;
                next_backstop = Instant::now() + BACKSTOP;
                self.check_lock()?;
                match self.scan(false) {
                    // What a save recorded is pushed at once, unless the last cycle failed and is waiting out its backoff.
                    Ok(recorded) => {
                        if recorded > 0 && !self.syncing.is_empty() && self.wait <= self.poll() {
                            next_poll = Instant::now();
                        }
                    }
                    Err(e) => {
                        self.report(&e);
                        retry = Some(Instant::now() + CHECK);
                    }
                }
            }
            let settled = pending.map(|(first, last)| (last + QUIET).min(first + CAP));
            let deadline = [
                settled,
                retry,
                Some(next_poll),
                Some(next_check),
                Some(next_backstop),
                Some(next_prune),
            ]
            .into_iter()
            .flatten()
            .min()
            .expect("the checks always have a deadline");
            match self
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Ok(event)) => {
                    if event.need_rescan() {
                        scan = true;
                    } else if !matches!(
                        event.kind,
                        EventKind::Access(AccessKind::Open(_) | AccessKind::Read)
                    ) {
                        let now = Instant::now();
                        pending = Some((pending.map_or(now, |(first, _)| first), now));
                    }
                }
                Ok(Err(_)) => scan = self.connect(false),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
            let now = Instant::now();
            if now >= next_check {
                next_check = now + CHECK;
                scan |= self.check()?;
            }
            if now >= next_prune {
                next_prune = now + PRUNE_EVERY;
                self.prune();
            }
            let settled = pending.map(|(first, last)| (last + QUIET).min(first + CAP));
            scan |= settled.is_some_and(|due| now >= due)
                || retry.is_some_and(|due| now >= due)
                || now >= next_backstop;
            if self.watcher.is_none() && !self.degraded {
                // Only a scan clears these, and none runs while `notes/` is away.
                scan = false;
                pending = None;
                retry = None;
            }
        }
    }

    /// The lock file must still be the one at its path.
    fn check_lock(&self) -> Result<(), Failure> {
        let lock_path = versions::watch_lock_path(&self.root);
        if same_file(&self.lock, &lock_path) {
            return Ok(());
        }
        Err(Failure::Refused(format!(
            "{} is gone; stopping",
            lock_path.display()
        )))
    }

    /// The lock file must still be the one at its path, and `notes/` the folder the watch was made on.
    fn check(&mut self) -> Result<bool, Failure> {
        self.check_lock()?;
        match fs::metadata(&self.notes) {
            Ok(meta) if meta.is_dir() => {
                let watched = self.watcher.as_ref().map(|(_, folder)| *folder);
                if watched == Some((meta.dev(), meta.ino())) {
                    return Ok(false);
                }
                Ok(self.connect(self.watcher.is_none()))
            }
            Ok(_) => {
                self.lose("not a folder");
                Ok(false)
            }
            Err(e) => {
                self.lose(&e.to_string());
                Ok(false)
            }
        }
    }

    /// Stops watching and says why once: nothing is recorded until `notes/` lists again.
    fn lose(&mut self, reason: &str) {
        self.watcher = None;
        self.degraded = false;
        if !self.outage {
            self.outage = true;
            let message = format!("cannot read {}: {reason}; waiting", self.notes.display());
            self.say(&message);
        }
    }

    /// Makes the watch on `notes/`; true when a scan should follow: the watch stands, or `notes/` lists without it. `announce` prints the watching line.
    fn connect(&mut self, announce: bool) -> bool {
        self.watcher = None;
        self.degraded = false;
        let meta = match fs::metadata(&self.notes) {
            Ok(meta) if meta.is_dir() => meta,
            Ok(_) => {
                self.lose("not a folder");
                return false;
            }
            Err(e) => {
                self.lose(&e.to_string());
                return false;
            }
        };
        if let Err(e) = fs::read_dir(&self.notes) {
            self.lose(&e.to_string());
            return false;
        }
        let made = notify::recommended_watcher(self.tx.clone()).and_then(|mut watcher| {
            watcher.watch(&self.notes, RecursiveMode::NonRecursive)?;
            Ok(watcher)
        });
        match made {
            Ok(watcher) => self.watcher = Some((watcher, (meta.dev(), meta.ino()))),
            Err(e) => {
                self.lose(&format!("cannot watch it: {e}"));
                self.degraded = true;
                return true;
            }
        }
        if announce || self.outage {
            let message = format!("watching {}", self.notes.display());
            self.say(&message);
        }
        self.outage = false;
        true
    }

    /// Prints an error once until it changes.
    fn report(&mut self, message: &str) {
        if self.last_error.as_deref() != Some(message) {
            self.last_error = Some(message.to_string());
            self.say(message);
        }
    }

    /// Lists `notes/` and reads the files whose stat changed since the last reading.
    fn refresh(&mut self) -> Result<Vec<Classified>, String> {
        let listed = versions::list(&self.notes)?;
        let mut cache = HashMap::new();
        let mut stats = HashMap::new();
        let mut digests = HashMap::new();
        let mut items = Vec::new();
        for (name, path) in listed {
            let stat = match fs::metadata(&path) {
                Ok(meta) => Some(Stat::of(&meta)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => None,
            };
            let kept = stat.and_then(|s| self.cache.remove(&name).filter(|(old, _)| *old == s));
            let item = match kept {
                Some((_, item)) => item,
                None => versions::classify(&name, &path).map(|found| Entry {
                    digest: hash::sha256_hex(&found.bytes),
                    name: found.name,
                    id: found.id,
                }),
            };
            if let Ok(entry) = &item {
                digests.insert(name.clone(), entry.digest.clone());
            }
            if let Some(stat) = stat {
                stats.insert(name.clone(), stat);
                let unreadable = item.as_ref().err().is_some_and(|skip| skip.unreadable);
                if !unreadable && !stat.racy() {
                    cache.insert(name, (stat, item.clone()));
                }
            }
            items.push(item);
        }
        self.cache = cache;
        self.stats = stats;
        self.digests = digests;
        Ok(items)
    }

    /// One scan: an unlocked pass that does the slow reading, then a locked one that applies what other devices
    /// wrote (when `apply`), sweeps, reads what changed since and records the differences. Returns how many notes it
    /// recorded a change of.
    fn scan(&mut self, apply: bool) -> Result<usize, String> {
        if let Err(reason) = self.refresh() {
            return self.lost(&reason).map(|()| 0);
        }
        let lock = versions::lock(&self.root)?;
        let at = versions::now_at();
        if apply {
            let lines = self.apply(&lock)?;
            self.inbound.extend(lines);
        }
        let staged = integrate::staged(&self.root)?;
        match versions::sweep_restore_leftovers(&lock, &at, &staged) {
            Ok(messages) => self.announce_sweep(messages),
            Err(e) => {
                return match fs::read_dir(&self.notes) {
                    Err(reason) => self.lost(&reason.to_string()).map(|()| 0),
                    Ok(_) => Err(e),
                };
            }
        }
        let items = match self.refresh() {
            Ok(items) => items,
            Err(reason) => return self.lost(&reason).map(|()| 0),
        };
        // The scan names the notes; their bytes stay on disk until one needs a new blob.
        let scan = versions::group(
            items
                .into_iter()
                .map(|item| {
                    item.map(|entry| Found {
                        name: entry.name,
                        id: entry.id,
                        bytes: Vec::new(),
                    })
                })
                .collect(),
        );
        self.announce_skips(&scan);
        let recorded = self.record(&lock, &scan, &at)?;
        self.last_error = None;
        Ok(recorded)
    }

    /// What `integrate` needs besides the store, for the cycle it runs in.
    fn with_params<T>(&self, run: impl FnOnce(&integrate::Params) -> T) -> T {
        let syncing = self.syncing.clone();
        let syncs = |name: &str| syncing.contains(name);
        let params = integrate::Params {
            syncs: &syncs,
            now: jiff::Timestamp::now(),
            stale_days: self.settings.sync.stale_days,
        };
        run(&params)
    }

    /// Writes the versions other devices sent into the notes. Nothing is written while `notes/` cannot be listed.
    fn apply(&mut self, lock: &Lock) -> Result<Vec<String>, String> {
        self.with_params(|params| {
            integrate::apply(lock, params, &mut |_| Ok(()), &mut swap::exchange)
        })
    }

    /// The folder cannot be listed: nothing is recorded, and the scan is not a failure.
    fn lost(&mut self, reason: &str) -> Result<(), String> {
        self.lose(reason);
        Ok(())
    }

    /// Recorded leftovers are said once, as they go; one left in place is said once until it changes.
    fn announce_sweep(&mut self, messages: Vec<String>) {
        let mut left = HashSet::new();
        for message in messages {
            if message.contains("left in place") {
                if !self.leftovers.contains(&message) {
                    self.say(&message);
                }
                left.insert(message);
            } else {
                self.say(&message);
            }
        }
        self.leftovers = left;
    }

    fn announce_skips(&mut self, scan: &Scan) {
        let names: HashSet<&str> = scan.skipped.iter().map(|s| s.name.as_str()).collect();
        self.printed.retain(|name, _| names.contains(name.as_str()));
        for skip in &scan.skipped {
            let seen = (skip.reason.clone(), self.stats.get(&skip.name).copied());
            if self.printed.get(&skip.name) != Some(&seen) {
                self.say(&format!(
                    "notes/{}: not recorded: {}",
                    skip.name, skip.reason
                ));
                self.printed.insert(skip.name.clone(), seen);
            }
        }
    }

    /// Records each note whose file differs from its latest version, through `integrate::record_local`, the one path
    /// of a local save. Returns how many it recorded.
    fn record(&mut self, lock: &Lock, scan: &Scan, at: &str) -> Result<usize, String> {
        let mut views = HashMap::new();
        for id in versions::note_ids(&self.root)? {
            let view = integrate::effective(versions::load(&self.root, &id)?.versions);
            if !view.is_empty() {
                views.insert(id, view);
            }
        }
        let mut recorded = 0;
        for (id, found) in &scan.notes {
            let digest = &self.digests[&found.name];
            // The file is a head group's text, whichever group that is: nothing changed.
            if views
                .get(id)
                .is_some_and(|view| integrate::held_group(view, &found.name, digest).is_some())
            {
                continue;
            }
            let path = self.notes.join(&found.name);
            match fs::read(&path) {
                Ok(bytes) => {
                    let lines = self.with_params(|params| {
                        integrate::record_local(lock, params, id, &found.name, Some(&bytes), at)
                    })?;
                    recorded += 1;
                    for line in lines {
                        self.say(&line);
                    }
                }
                // Gone since the scan: the next one sees it.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
            }
        }
        if scan.has_unreadable() {
            return Ok(recorded);
        }
        let present = versions::present_ids(scan);
        // A note is live while a head group is neither a `left` nor a deletion; the file name is that group's.
        let live: Vec<(&String, String)> = views
            .iter()
            .filter_map(|(id, view)| {
                let head = versions::heads(view)
                    .into_iter()
                    .find(|g| !g[0].is_left() && !g[0].is_deleted())?;
                Some((id, head[0].file.clone()))
            })
            .collect();
        if present.is_empty() && !live.is_empty() {
            if !self.empty {
                self.empty = true;
                let message = format!(
                    "{} holds no notes; not recording deletions",
                    self.notes.display()
                );
                self.say(&message);
            }
            return Ok(recorded);
        }
        self.empty = false;
        for (id, file) in live
            .into_iter()
            .filter(|(id, _)| !present.contains(id.as_str()))
        {
            let lines = self
                .with_params(|params| integrate::record_local(lock, params, id, &file, None, at))?;
            recorded += 1;
            for line in lines {
                self.say(&line);
            }
        }
        Ok(recorded)
    }
}

/// The sync cycle: manifests, pull, apply, push, per syncing scope.
impl Watch<'_> {
    fn poll(&self) -> Duration {
        Duration::from_secs(u64::from(self.settings.sync.poll_seconds))
    }

    /// Reads the config again; one that no longer parses leaves the last good settings, and is said once.
    fn reload(&mut self) {
        match config::load(self.env) {
            Ok(settings) => {
                self.settings = settings;
                self.config_error = None;
            }
            Err(e) => {
                if self.config_error.as_deref() != Some(&e) {
                    self.say(&e);
                    self.config_error = Some(e);
                }
            }
        }
    }

    fn identity(&self) -> Result<Option<Identity>, String> {
        match store::keys_dir(self.env) {
            Some(dir) => keys::read_identity(&dir),
            None => Ok(None),
        }
    }

    /// Says `line` when it differs from the last one held for this scope and channel, and returns whether it did. `None`
    /// ends the condition, so a later `Some` is said again.
    fn hold(&mut self, name: &str, channel: &str, line: Option<String>) -> bool {
        let key = format!("{name}/{channel}");
        if self.held.get(&key) == line.as_ref() {
            return false;
        }
        match line {
            Some(line) => {
                self.say(&line);
                self.held.insert(key, line);
            }
            None => {
                self.held.remove(&key);
            }
        }
        true
    }

    /// Forgets the scopes the config no longer syncs, so one that comes back says its lines again.
    fn release(&mut self, wanted: &[(String, String)]) {
        let named = |name: &str| wanted.iter().any(|(n, _)| n == name);
        self.scopes.retain(|name, _| named(name));
        self.held
            .retain(|key, _| key.split_once('/').is_some_and(|(name, _)| named(name)));
    }

    /// One sync cycle, when the config names a syncing scope: for each, the manifest step, a pull and its staging,
    /// then one locked pass that applies what was staged, sweeps and scans, and a push. A cycle that fails doubles
    /// the wait before the next, up to ten minutes.
    fn tick(&mut self) -> Tick {
        self.more = false;
        self.reload();
        let poll = self.poll();
        let wanted: Vec<(String, String)> = self
            .settings
            .scopes
            .iter()
            .filter(|s| s.sync != "off")
            .map(|s| (s.name.clone(), s.sync.clone()))
            .collect();
        self.release(&wanted);
        let identity = self.identity();
        if wanted.is_empty() {
            self.syncing.clear();
            self.wait = poll;
            return Tick {
                scanned: None,
                wait: poll,
            };
        }
        let now = jiff::Timestamp::now();
        let mut failed = false;
        let mut ready = Vec::new();
        for (name, url) in &wanted {
            self.hold(name, "start", Some(format!("syncing {name} through {url}")));
            match &identity {
                Ok(Some(identity)) => {
                    self.hold(name, "keys", None);
                    if let Some(scope) = self.prepare(name, url, identity, now, &mut failed) {
                        ready.push(scope);
                    }
                }
                Ok(None) => {
                    let line = format!(
                        "sync {name}: no device key; run bilbo device init or bilbo device recover"
                    );
                    self.hold(name, "keys", Some(line));
                }
                Err(e) => {
                    self.hold(name, "keys", Some(format!("sync {name}: {e}")));
                }
            }
        }
        // A scope stopped this cycle does not sync: only the ready ones count for the scope-clash rule.
        self.syncing = ready.iter().map(|scope| scope.name.clone()).collect();
        let (Ok(Some(identity)), false) = (&identity, ready.is_empty()) else {
            return Tick {
                scanned: None,
                wait: self.backoff(failed),
            };
        };
        for scope in &mut ready {
            self.pull(scope, identity, now, &mut failed);
        }
        let scanned = self.scan(true);
        self.announce_inbound();
        self.trim_seen();
        for scope in &mut ready {
            if scope.ok {
                self.push(scope, identity, now, &mut failed);
            }
            self.settle(scope);
        }
        Tick {
            scanned: Some(scanned),
            wait: self.backoff(failed),
        }
    }

    fn backoff(&mut self, failed: bool) -> Duration {
        let poll = self.poll();
        self.wait = if failed {
            (self.wait.max(poll) * 2).min(POLL_CAP.max(poll))
        } else {
            poll
        };
        if self.more { Duration::ZERO } else { self.wait }
    }

    /// The scope's step: its manifests, and whether it may sync. Returns the scope when it may.
    fn prepare(
        &mut self,
        name: &str,
        url: &str,
        identity: &Identity,
        now: jiff::Timestamp,
        failed: &mut bool,
    ) -> Option<Ready> {
        let device = identity.device.id();
        let t = match transport::open(url, &device) {
            Ok(t) => t,
            Err(e) => {
                self.hold(name, "transport", Some(format!("sync {name}: {e}")));
                return None;
            }
        };
        self.hold(name, "transport", None);
        let root = self.root.clone();
        let input = manifests::Input {
            root: &root,
            name,
            url,
            identity,
            now,
        };
        let out = match manifests::step(&*t, &input) {
            Ok(out) => out,
            Err(e) => {
                self.unreachable(name, url, &e);
                self.keep_unreachable(name, identity, &e);
                *failed = true;
                return None;
            }
        };
        for line in &out.events {
            self.say(line);
        }
        if let Some(why) = &out.error {
            self.unreachable(name, url, why);
            self.keep_unreachable(name, identity, why);
            *failed = true;
            return None;
        }
        let Some(id) = out.scope else {
            let stop = out.stop.map(|stop| stop.line);
            if self.hold(name, "stop", stop.clone()) {
                self.mark_stopped(name, identity, stop.as_deref());
            }
            return None;
        };
        self.hold(name, "stop", None);
        let ours = self
            .scopes
            .get(name)
            .is_some_and(|a| a.id == id && a.device == device);
        if !ours {
            match Replica::open(&root, &id, name, &device) {
                Ok(replica) => {
                    self.scopes.insert(
                        name.to_string(),
                        Active {
                            id,
                            device,
                            replica,
                            problem: None,
                        },
                    );
                }
                Err(e) => {
                    self.hold(name, "replica", Some(format!("sync {name}: {e}")));
                    return None;
                }
            }
        }
        self.hold(name, "replica", None);
        let active = self.scopes.get_mut(name).expect("the scope is open");
        if let Err(e) = active.replica.set_stopped(None) {
            self.say(&e);
        }
        Some(Ready {
            name: name.to_string(),
            url: url.to_string(),
            t,
            ok: true,
            full: out.full,
        })
    }

    /// The ids of the scopes of this name the store holds a manifest of that this device is a member of.
    fn scope_ids(&mut self, name: &str, identity: &Identity) -> Option<Vec<String>> {
        let owner = identity.owner.sign.public();
        let who = Recipient::device(&identity.device);
        match manifest::survey(&self.root, Some(&owner), Some(&who)) {
            Ok(known) => Some(
                known
                    .into_iter()
                    .filter(|k| k.mine && k.last_name.as_deref() == Some(name))
                    .map(|k| k.scope.id)
                    .collect(),
            ),
            Err(e) => {
                self.say(&e);
                None
            }
        }
    }

    /// Writes the line that stops the scope into the state of each scope of this name the store holds, for `bilbo sync`.
    fn mark_stopped(&mut self, name: &str, identity: &Identity, line: Option<&str>) {
        let Some(ids) = self.scope_ids(name, identity) else {
            return;
        };
        let device = identity.device.id();
        for id in ids {
            let line = line.map(str::to_string);
            let result = match self.scopes.get_mut(name).filter(|a| a.id == id) {
                Some(active) => active.replica.set_stopped(line),
                None => Replica::open(&self.root, &id, name, &device)
                    .and_then(|mut replica| replica.set_stopped(line)),
            };
            if let Err(e) = result {
                self.say(&e);
            }
        }
    }

    /// Reads what other devices wrote, stages it for `apply` and then books it as read.
    fn pull(
        &mut self,
        scope: &mut Ready,
        identity: &Identity,
        now: jiff::Timestamp,
        failed: &mut bool,
    ) {
        let Some(active) = self.scopes.get_mut(&scope.name) else {
            return;
        };
        let pulled = match active.replica.pull(&*scope.t, identity, now) {
            Ok(pulled) => pulled,
            Err(e) => {
                self.unreachable(&scope.name, &scope.url, &e);
                scope.ok = false;
                *failed = true;
                return;
            }
        };
        for line in &pulled.events {
            self.say(line);
        }
        let staged = versions::lock(&self.root).and_then(|lock| {
            integrate::stage(
                &lock,
                &scope.name,
                &pulled.records,
                &pulled.blobs,
                &pulled.declarations,
                now,
            )
        });
        match staged {
            Ok(lines) => self.inbound.extend(lines),
            Err(e) => {
                self.say(&e);
                scope.ok = false;
                return;
            }
        }
        let active = self.scopes.get_mut(&scope.name).expect("the scope is open");
        if let Err(e) = active.replica.commit(&pulled) {
            self.say(&e);
            scope.ok = false;
        }
        self.more |= pulled.more;
    }

    /// Pushes what the scope's log holds that the transport lacks, or an acknowledgement.
    fn push(
        &mut self,
        scope: &mut Ready,
        identity: &Identity,
        now: jiff::Timestamp,
        failed: &mut bool,
    ) {
        let Some(active) = self.scopes.get_mut(&scope.name) else {
            return;
        };
        match active
            .replica
            .push(&*scope.t, identity, &self.settings, now)
        {
            Ok(pushed) => {
                for line in &pushed.events {
                    self.say(line);
                }
                scope.full = pushed.full.or(scope.full.take());
            }
            Err(e) => {
                self.unreachable(&scope.name, &scope.url, &e);
                scope.ok = false;
                *failed = true;
            }
        }
    }

    /// What a cycle that reached the transport leaves: no unreachable line, and a full one while it holds.
    fn settle(&mut self, scope: &Ready) {
        if !scope.ok {
            return;
        }
        let name = &scope.name;
        self.hold(name, "reach", None);
        let line = scope.full.as_ref().map(|m| format!("sync {name}: {m}"));
        self.hold(name, "full", line);
        match &scope.full {
            Some(message) => self.problem(name, "full", message),
            None => self.clear_problem(name),
        }
    }

    /// The transport cannot be read or written: said once until it can.
    fn unreachable(&mut self, name: &str, url: &str, why: &str) {
        let line = format!("sync {name}: {url} is not reachable: {why}");
        self.hold(name, "reach", Some(line));
        self.problem(name, "unreachable", why);
    }

    /// A folder unreachable before any replica of the scope opened leaves its error in the state of each scope of
    /// this name the store holds, where `bilbo sync` reads it; the first cycle that reaches the folder clears it.
    fn keep_unreachable(&mut self, name: &str, identity: &Identity, why: &str) {
        if self.scopes.contains_key(name) {
            return;
        }
        let Some(ids) = self.scope_ids(name, identity) else {
            return;
        };
        let device = identity.device.id();
        for id in ids {
            let result = Replica::open(&self.root, &id, name, &device).and_then(|mut replica| {
                let since = match replica.error() {
                    Some(p) if p.kind == "unreachable" => p.since,
                    _ => jiff::Timestamp::now().as_second(),
                };
                replica.set_error(Some(Problem {
                    kind: "unreachable".to_string(),
                    since,
                    message: why.to_string(),
                }))
            });
            if let Err(e) = result {
                self.say(&e);
            }
        }
    }

    fn problem(&mut self, name: &str, kind: &str, message: &str) {
        let Some(active) = self.scopes.get_mut(name) else {
            return;
        };
        let since = match &active.problem {
            Some(p) if p.kind == kind => p.since,
            _ => jiff::Timestamp::now().as_second(),
        };
        let problem = Problem {
            kind: kind.to_string(),
            since,
            message: message.to_string(),
        };
        let result = active.replica.set_error(Some(problem.clone()));
        active.problem = Some(problem);
        if let Err(e) = result {
            self.say(&e);
        }
    }

    fn clear_problem(&mut self, name: &str) {
        let Some(active) = self.scopes.get_mut(name) else {
            return;
        };
        active.problem = None;
        if let Err(e) = active.replica.set_error(None) {
            self.say(&e);
        }
    }

    /// The lines staging and applying returned: each said once until a cycle goes by without it.
    fn announce_inbound(&mut self) {
        let mut lines = HashSet::new();
        for line in std::mem::take(&mut self.inbound) {
            if lines.insert(line.clone()) && !self.cycle_lines.contains(&line) {
                self.say(&line);
            }
        }
        self.cycle_lines = lines;
    }

    /// After a prune and an apply, `seen.jsonl` drops the versions no log holds.
    fn trim_seen(&mut self) {
        if !std::mem::take(&mut self.trim) {
            return;
        }
        let mut errors = Vec::new();
        for active in self.scopes.values() {
            if let Err(e) = active.replica.trim_seen() {
                errors.push(e);
            }
        }
        for e in errors {
            self.say(&e);
        }
    }
}
