//! `bilbo watch`: records a version of each note when it changes. `notify` only says when to look; every decision
//! comes from a scan of `<root>/notes/` against history, so a missed event costs time, never content.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::event::AccessKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::Failure;
use crate::note::versions::{self, Found, Lock, Scan, Skip};
use crate::shared::{config, hash, store};

/// How long `notes/` stays quiet before a scan, and the longest a scan waits while events keep coming.
const QUIET: Duration = Duration::from_secs(2);
const CAP: Duration = Duration::from_secs(10);
/// How often the folder and the lock file are checked, whatever the events say.
const CHECK: Duration = Duration::from_secs(10);
const BACKSTOP: Duration = Duration::from_secs(600);
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 3600);
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
        keep_days: settings.history.keep_days,
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

struct Watch<'a> {
    root: PathBuf,
    notes: PathBuf,
    keep_days: u32,
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

    /// The `.tmp-*` leftovers under `history/` go, and the first prune runs.
    fn housekeeping(&mut self) {
        let result = versions::lock(&self.root).and_then(|lock| {
            versions::sweep_temporaries(&lock)?;
            self.prune_under(&lock)
        });
        if let Err(e) = result {
            self.say(&e);
        }
    }

    fn prune_under(&mut self, lock: &Lock) -> Result<(), String> {
        let pruned = versions::prune(lock, self.keep_days, jiff::Timestamp::now())?;
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
        let mut scan = self.connect(true);
        loop {
            if scan {
                scan = false;
                pending = None;
                retry = None;
                next_backstop = Instant::now() + BACKSTOP;
                self.check_lock()?;
                if let Err(e) = self.scan() {
                    self.report(&e);
                    retry = Some(Instant::now() + CHECK);
                }
            }
            let settled = pending.map(|(first, last)| (last + QUIET).min(first + CAP));
            let deadline = [
                settled,
                retry,
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

    /// One scan: an unlocked pass that does the slow reading, then a locked one that sweeps, reads what changed since
    /// and records the differences.
    fn scan(&mut self) -> Result<(), String> {
        if let Err(reason) = self.refresh() {
            return self.lost(&reason);
        }
        let lock = versions::lock(&self.root)?;
        let at = versions::now_at();
        match versions::sweep_restore_leftovers(&lock, &at) {
            Ok(messages) => self.announce_sweep(messages),
            Err(e) => {
                return match fs::read_dir(&self.notes) {
                    Err(reason) => self.lost(&reason.to_string()),
                    Ok(_) => Err(e),
                };
            }
        }
        let items = match self.refresh() {
            Ok(items) => items,
            Err(reason) => return self.lost(&reason),
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
        self.record(&lock, &scan, &at)?;
        self.last_error = None;
        Ok(())
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

    fn record(&mut self, lock: &Lock, scan: &Scan, at: &str) -> Result<(), String> {
        let mut heads = HashMap::new();
        for id in versions::note_ids(&self.root)? {
            if let Some(head) = versions::load(&self.root, &id)?.versions.pop() {
                heads.insert(id, head);
            }
        }
        for (id, found) in &scan.notes {
            let digest = &self.digests[&found.name];
            let current = Some((found.name.as_str(), digest.as_str()));
            let Some(event) = versions::event_for(heads.get(id), current) else {
                continue;
            };
            let path = self.notes.join(&found.name);
            match fs::read(&path) {
                Ok(bytes) => {
                    versions::record(lock, id, &found.name, Some(&bytes), event, at)?;
                }
                // Gone since the scan: the next one sees it.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
            }
        }
        if scan.has_unreadable() {
            return Ok(());
        }
        let present = versions::present_ids(scan);
        let live: Vec<&String> = heads
            .iter()
            .filter(|(_, head)| !head.is_deleted())
            .map(|(id, _)| id)
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
            return Ok(());
        }
        self.empty = false;
        for id in live.into_iter().filter(|id| !present.contains(id.as_str())) {
            versions::record(lock, id, &heads[id].file, None, versions::DELETED, at)?;
        }
        Ok(())
    }
}
