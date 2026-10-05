//! Sync through the built binary: two stores, `rivendell` and `bagend`, built from the golden keys and manifests of
//! `tests/fixtures/device/`, each with its own `bilbo watch` over one temporary `file://` folder. Every wait is a
//! poll with a deadline. A check that something did not happen waits on a barrier: a later change that has to travel
//! through the same cycles. No test feeds a phrase, and the race, the filesystem that cannot swap and the hour-long
//! acknowledgement rule stay in the unit tests of `sync::replica` and `sync::integrate`.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Run, TempDir, Watcher, bilbo, config, poll_eq};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");
const RIVENDELL: &str = "gr2q7gf5lh6pzfdnurnkvputhp";
const BAGEND: &str = "wyxim75c6m5p4ywv22ywilqweh";
const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
const OTHER: &str = "01M3YE296FMNXYZS89787DMY0A";
const FILE: &str = "decision-release.md";
const TOPIC: &str = "release";

/// The fixture scope's id: the one folder of the fixture store.
fn scope() -> String {
    let mut ids: Vec<String> = fs::read_dir(Path::new(FIXTURES).join("store/.bilbo/scopes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    ids.remove(0)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// The fixture's manifest versions `versions`, as bytes by number.
fn manifests(versions: &[u64]) -> BTreeMap<u64, Vec<u8>> {
    let dir = Path::new(FIXTURES).join(format!("store/.bilbo/scopes/{}/manifest", scope()));
    versions
        .iter()
        .map(|n| (*n, fs::read(dir.join(format!("{n}.json"))).unwrap()))
        .collect()
}

/// The folder the devices share, holding the manifest versions `versions` of the fixture scope.
fn folder(dir: &TempDir, versions: &[u64]) -> PathBuf {
    let folder = dir.path().join("folder");
    fs::create_dir_all(&folder).unwrap();
    if !versions.is_empty() {
        put_manifests(&folder, versions);
    }
    folder
}

fn put_manifests(folder: &Path, versions: &[u64]) {
    let into = folder.join(format!("scopes/{}/manifest", scope()));
    fs::create_dir_all(&into).unwrap();
    for (n, bytes) in manifests(versions) {
        fs::write(into.join(format!("{n}.json")), bytes).unwrap();
    }
}

fn url(folder: &Path) -> String {
    format!("file://{}", folder.display())
}

/// One device: a store, its own state folder and config, and a watcher while it runs.
struct Site {
    dir: TempDir,
    name: &'static str,
    env: Vec<(String, String)>,
    watcher: Option<Watcher>,
}

impl Site {
    /// `who`'s keys and the fixture manifest versions `versions` in a new store, syncing as `lines` say.
    fn build(who: &'static str, keys: bool, versions: &[u64], lines: &[String]) -> Site {
        let dir = TempDir::new(who);
        let root = dir.path().join("store");
        fs::create_dir_all(root.join("notes")).unwrap();
        if !versions.is_empty() {
            let into = root.join(format!(".bilbo/scopes/{}/manifest", scope()));
            fs::create_dir_all(&into).unwrap();
            for (n, bytes) in manifests(versions) {
                fs::write(into.join(format!("{n}.json")), bytes).unwrap();
            }
        }
        let site = Site {
            env: vec![
                ("BILBO_HOME".into(), root.to_str().unwrap().into()),
                (
                    "BILBO_CONFIG".into(),
                    dir.path().join("config").to_str().unwrap().into(),
                ),
                (
                    "XDG_STATE_HOME".into(),
                    dir.path().join("state").to_str().unwrap().into(),
                ),
                ("HOME".into(), "/home/tester".into()),
            ],
            dir,
            name: who,
            watcher: None,
        };
        if keys {
            site.install_keys();
        }
        site.configure(lines);
        site
    }

    /// The enrolled device `who`, syncing `personal` through `folder` every second.
    fn new(who: &'static str, folder: &Path) -> Site {
        Site::build(who, true, &[1, 2], &Site::syncing(folder))
    }

    fn syncing(folder: &Path) -> Vec<String> {
        vec![
            format!("scope.personal.sync = {}", url(folder)),
            "sync.poll_seconds = 1".to_string(),
        ]
    }

    fn install_keys(&self) {
        let keys = self.dir.path().join("state/bilbo/keys");
        copy_tree(&Path::new(FIXTURES).join(self.name), &keys);
        chmod(&keys, 0o700);
        for file in ["owner.key", "device.key"] {
            chmod(&keys.join(file), 0o600);
        }
    }

    fn configure(&self, lines: &[String]) {
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        config(&self.dir, &lines);
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("store")
    }

    fn notes(&self) -> PathBuf {
        self.root().join("notes")
    }

    fn pairs(&self) -> Vec<(&str, &str)> {
        self.env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    fn bilbo(&self, args: &[&str]) -> Run {
        bilbo(self.dir.path(), &self.pairs(), args)
    }

    fn start(&mut self) {
        let watcher = Watcher::start(&self.pairs());
        watcher.wait_for(&format!("bilbo: watching {}", self.notes().display()));
        self.watcher = Some(watcher);
    }

    fn stop(&mut self) {
        self.watcher = None;
    }

    fn lines(&self) -> Vec<String> {
        self.watcher.as_ref().map_or_else(Vec::new, Watcher::lines)
    }

    fn count(&self, needle: &str) -> usize {
        self.lines().iter().filter(|l| l.contains(needle)).count()
    }

    fn wait_for(&self, needle: &str) {
        self.watcher.as_ref().unwrap().wait_for(needle);
    }

    fn write(&self, name: &str, text: &str) {
        fs::write(self.notes().join(name), text).unwrap();
    }

    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.notes().join(name)).ok()
    }

    /// The lines of `bilbo history <topic>`, newest first; empty while the note has no history.
    fn history(&self, topic: &str) -> Vec<String> {
        let run = self.bilbo(&["history", topic]);
        if run.code != 0 {
            return Vec::new();
        }
        run.stdout.lines().map(str::to_string).collect()
    }

    /// The events of a note's versions, newest first.
    fn events(&self, topic: &str) -> Vec<String> {
        self.history(topic)
            .iter()
            .map(|line| line.split(' ').nth(2).unwrap().to_string())
            .collect()
    }

    /// Waits until the events of a note's versions, newest first, are `want`.
    fn wait_events(&self, topic: &str, want: &[&str]) {
        let want: Vec<String> = want.iter().map(|e| e.to_string()).collect();
        poll_eq(
            &format!("events of {topic} on {}", self.name),
            || self.events(topic),
            want,
        );
    }

    /// Waits until the note's file holds `text`.
    fn wait_text(&self, name: &str, text: &str) {
        poll_eq(
            &format!("{} holds {name}", self.name),
            || self.read(name),
            Some(text.to_string()),
        );
    }

    /// Waits until the note's file holds `needle`, and returns its text.
    fn wait_holding(&self, name: &str, needle: &str) -> String {
        let text = std::cell::RefCell::new(String::new());
        poll_eq(
            &format!("{} holds {needle:?} in {name}", self.name),
            || {
                let now = self.read(name).unwrap_or_default();
                let found = now.contains(needle);
                *text.borrow_mut() = now;
                found
            },
            true,
        );
        text.into_inner()
    }

    fn device_dir(&self, folder: &Path, device: &str) -> PathBuf {
        folder.join(format!("scopes/{}/devices/{device}", scope()))
    }
}

/// The segment files of `device`'s folder, sorted.
fn segments(folder: &Path, device: &str) -> Vec<PathBuf> {
    let dir = folder.join(format!("scopes/{}/devices/{device}", scope()));
    let mut files: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(items) => items.map(|e| e.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    };
    files.retain(|p| {
        let name = p.file_name().unwrap().to_str().unwrap();
        name.len() == 24 && name.ends_with(".seg") && name[..20].bytes().all(|b| b.is_ascii_digit())
    });
    files.sort();
    files
}

/// The segments of `device`'s folder with their bytes.
fn segments_with_bytes(folder: &Path, device: &str) -> BTreeMap<PathBuf, Vec<u8>> {
    segments(folder, device)
        .into_iter()
        .map(|p| {
            let bytes = fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect()
}

/// Every file under `dir`, with its bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(items) = fs::read_dir(dir) else {
            return;
        };
        for entry in items {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(dir, &mut files);
    files
}

/// A synced note: the title, a `## Setup` and a `## Rollout`, each of one line.
fn note(id: &str, setup: &str, rollout: &str) -> String {
    format!(
        "---\nid: {id}\ncreated: 2026-10-02T14:23-03:00\nscope: personal\n---\n\n# Release\n\n## Setup\n\n{setup}\n\n## Rollout\n\n{rollout}\n"
    )
}

/// Starts both devices on a new folder, with `decision-release.md` written on `rivendell` and arrived on `bagend`.
fn synced(name: &str) -> (TempDir, PathBuf, Site, Site, String) {
    synced_with(name, &[])
}

/// Like `synced`, with `extra` config lines on both devices.
fn synced_with(name: &str, extra: &[&str]) -> (TempDir, PathBuf, Site, Site, String) {
    let dir = TempDir::new(name);
    let folder = folder(&dir, &[1, 2]);
    let mut lines = Site::syncing(&folder);
    lines.extend(extra.iter().map(|l| l.to_string()));
    let mut a = Site::build("rivendell", true, &[1, 2], &lines);
    let mut b = Site::build("bagend", true, &[1, 2], &lines);
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    b.start();
    b.wait_text(FILE, &text);
    // B's acknowledgement of what it applied is written once, so a later count of its segments starts from there.
    poll_eq("B's acknowledgement", || segments(&folder, BAGEND).len(), 1);
    (dir, folder, a, b, text)
}

#[test]
fn an_assigned_note_syncs_and_an_edit_follows() {
    let dir = TempDir::new("assigned");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    a.start();
    b.start();
    for site in [&a, &b] {
        site.wait_for(&format!("bilbo: syncing personal through {}", url(&folder)));
    }
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    b.wait_text(FILE, &text);
    let edited = format!("{text}\nA paragraph from rivendell.\n");
    a.write(FILE, &edited);
    b.wait_text(FILE, &edited);
    let history = b.history(TOPIC);
    assert!(
        history[0].ends_with(&format!("edited {FILE} from rivendell")),
        "{history:?}"
    );
    for site in [&a, &b] {
        assert_eq!(site.count("none of this device's scopes"), 0);
    }
}

/// Whether any file under `dir`, by path or by bytes, holds `needle`.
fn holds(dir: &Path, needle: &str) -> bool {
    tree(dir).iter().any(|(path, bytes)| {
        path.to_string_lossy().contains(needle)
            || bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
    })
}

#[test]
fn the_first_sync_uploads_nothing_and_a_local_scope_stays_local() {
    let dir = TempDir::new("local");
    let folder = folder(&dir, &[1, 2]);
    let mut lines = Site::syncing(&folder);
    lines.push("scope.work.sync = off".to_string());
    let mut a = Site::build("rivendell", true, &[1, 2], &lines);
    let mut b = Site::new("bagend", &folder);
    a.write(
        "plan-unassigned.md",
        &common::note_text(OTHER, "Unassigned"),
    );
    a.write(
        "plan-work.md",
        &common::in_scope(
            &common::note_text("01M3YJ7R6HK6NQ30DCDB1P4D00", "Work"),
            "work",
        ),
    );
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    b.start();
    b.wait_text(FILE, &text);
    let mut names: Vec<String> = fs::read_dir(b.notes())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, [FILE]);
    assert_eq!(a.events("unassigned"), ["added"]);
    assert_eq!(a.events("work"), ["added"]);
}

#[test]
fn nothing_readable_leaves_the_device() {
    let dir = TempDir::new("clear");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    let text = note(ID, "Install it.", "Ship on friday.").replace("# Release", "# Release plan");
    a.write("plan-release-plan.md", &text);
    a.start();
    b.start();
    b.wait_text("plan-release-plan.md", &text);
    for needle in [
        "release-plan",
        "Ship on friday",
        "ship on friday",
        "personal",
    ] {
        assert!(!holds(&folder, needle), "the folder holds {needle:?}");
    }
}

#[test]
fn the_layout_ignores_names_it_does_not_know_and_a_small_push_is_one_segment() {
    let dir = TempDir::new("layout");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    poll_eq(
        "the first segment",
        || segments(&folder, RIVENDELL).len(),
        1,
    );
    let first = &segments(&folder, RIVENDELL)[0];
    assert_eq!(
        first.file_name().unwrap().to_str().unwrap(),
        "00000000000000000001.seg"
    );
    let devices = a.device_dir(&folder, RIVENDELL);
    fs::write(
        devices.join("00000000000000000003 (conflicted copy).seg"),
        b"not a segment",
    )
    .unwrap();
    fs::write(devices.join("notes.txt"), b"nor this").unwrap();
    b.start();
    b.wait_text(FILE, &text);
    let edited = format!("{text}\nMore.\n");
    a.write(FILE, &edited);
    b.wait_text(FILE, &edited);
    assert_eq!(segments(&folder, RIVENDELL).len(), 2);
    assert_eq!(b.count("segment"), 0, "{:?}", b.lines());
}

#[test]
fn without_a_device_key_nothing_is_pushed_until_one_appears() {
    let dir = TempDir::new("nokey");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::build("rivendell", false, &[1, 2], &Site::syncing(&folder));
    let mut b = Site::new("bagend", &folder);
    let line = "bilbo: sync personal: no device key; run bilbo device init or bilbo device recover";
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    b.start();
    a.wait_for(line);
    a.wait_events(TOPIC, &["added"]);
    assert!(segments(&folder, RIVENDELL).is_empty());
    assert_eq!(b.read(FILE), None);
    // The user enrols the device while watch runs: its next cycle syncs, without a restart.
    a.install_keys();
    b.wait_text(FILE, &text);
    assert_eq!(a.count(line), 1);
    assert_eq!(a.count("bilbo: syncing personal through"), 1);
}

#[test]
fn turning_a_scope_off_stops_the_cycle_and_a_bad_config_keeps_the_last_good() {
    let (dir, folder, a, b, text) = synced("off");
    let before = segments(&folder, RIVENDELL).len();
    // A config that no longer parses keeps the settings that worked, and says so once.
    a.configure(&["sync.poll_seconds = 0".to_string()]);
    a.wait_for("sync.poll_seconds");
    let edited = format!("{text}\nWhile the config is broken.\n");
    a.write(FILE, &edited);
    b.wait_text(FILE, &edited);
    assert_eq!(a.count("sync.poll_seconds"), 1, "{:?}", a.lines());
    // Off: the next cycles pull and push nothing, and the notes stay.
    a.configure(&[
        "scope.personal.sync = off".to_string(),
        "sync.poll_seconds = 1".to_string(),
    ]);
    let local = format!("{edited}\nWhile the scope is off.\n");
    a.write(FILE, &local);
    poll_eq(
        "the local edit",
        || a.events(TOPIC)[0].clone(),
        "edited".to_string(),
    );
    poll_eq("two edits", || a.events(TOPIC).len(), 3);
    // A cycle that reads a config naming another folder says so, and has read the scope as off before.
    let elsewhere = dir.path().join("elsewhere");
    a.configure(&Site::syncing(&elsewhere));
    a.wait_for(&format!(
        "bilbo: syncing personal through {}",
        url(&elsewhere)
    ));
    // A cycle that had already reread the still-syncing config when the scope went off may push the first local edit
    // (its scan runs after its pull); no later cycle does, so at most one segment follows the broken-config edit.
    let sent = segments(&folder, RIVENDELL).len();
    assert!(
        (before + 1..=before + 2).contains(&sent),
        "{sent} segments after {before}"
    );
    let pushed = b.read(FILE).unwrap();
    assert!(
        pushed == edited || pushed == local,
        "bagend holds neither edit: {pushed}"
    );
    assert!(sent == before + 2 || pushed == edited);
    assert!(!elsewhere.exists(), "watch created a transport folder");
    assert!(a.read(FILE).unwrap().contains("While the scope is off."));
}

#[test]
fn a_relay_url_is_not_supported_yet() {
    let dir = TempDir::new("relay");
    let lines = vec![
        "scope.personal.sync = https://relay.example".to_string(),
        "sync.poll_seconds = 1".to_string(),
    ];
    let mut a = Site::build("rivendell", true, &[1, 2], &lines);
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    a.start();
    a.wait_for(
        "bilbo: sync personal: https transports are not supported yet; use a file:// folder",
    );
    a.wait_events(TOPIC, &["added"]);
    assert_eq!(a.count("not supported yet"), 1);
    drop(dir);
}

#[test]
fn an_agent_saves_over_text_it_never_read() {
    let (_dir, _folder, a, b, _text) = synced("stale");
    let theirs = note(ID, "Install it with care.", "Ship on Monday.");
    b.write(FILE, &theirs);
    a.wait_text(FILE, &theirs);
    // The agent on A saves an edit of `## Rollout` made from the read it took before B's edit arrived.
    a.write(FILE, &note(ID, "Install it.", "Ship on Friday."));
    let both = note(ID, "Install it with care.", "Ship on Friday.");
    a.wait_text(FILE, &both);
    b.wait_text(FILE, &both);
    let history = a.history(TOPIC);
    assert!(
        history[0].ends_with(&format!("merged {FILE} [stale-base]")),
        "{history:?}"
    );
}

#[test]
fn an_agent_that_read_the_new_text_is_recorded_as_following_it() {
    let (_dir, _folder, a, b, _text) = synced("fresh");
    let theirs = note(ID, "Install it with care.", "Ship on Monday.");
    b.write(FILE, &theirs);
    a.wait_text(FILE, &theirs);
    a.wait_events(TOPIC, &["edited", "added"]);
    let mine = note(ID, "Install it with care.", "Ship on Friday.");
    a.write(FILE, &mine);
    a.wait_events(TOPIC, &["edited", "edited", "added"]);
    b.wait_text(FILE, &mine);
    assert_eq!(a.read(FILE), Some(mine));
    let history = a.history(TOPIC);
    assert!(!history[0].contains(" from "), "{history:?}");
    assert!(history[1].ends_with("from bagend"), "{history:?}");
    assert!(history.iter().all(|l| !l.contains("stale-base")));
}

#[test]
fn the_first_push_of_a_marked_note_says_so_once() {
    let dir = TempDir::new("marks");
    let folder = folder(&dir, &[1, 2]);
    let mut lines = Site::syncing(&folder);
    lines.push("scope.work.sync = off".to_string());
    lines.push("scope.work.marks = acme".to_string());
    let mut a = Site::build("rivendell", true, &[1, 2], &lines);
    let mut b = Site::new("bagend", &folder);
    let marked = note(ID, "Install it for acme.", "Ship on Monday.");
    a.write(FILE, &marked);
    a.start();
    b.start();
    b.wait_text(FILE, &marked);
    let line =
        "bilbo: notes/decision-release.md: pushed to 'personal' while holding a mark of 'work'";
    assert_eq!(a.count(line), 1, "{:?}", a.lines());
    let again = note(ID, "Install it for acme, twice.", "Ship on Monday.");
    a.write(FILE, &again);
    b.wait_text(FILE, &again);
    // A note with no mark travels without one.
    let plain = note(OTHER, "Nothing here.", "Nothing at all.");
    a.write("plan-other.md", &plain);
    b.wait_text("plan-other.md", &plain);
    assert_eq!(a.count("while holding a mark"), 1, "{:?}", a.lines());
}

/// Stops both devices, edits on each while neither runs, and starts them again.
fn offline_edits(a: &mut Site, b: &mut Site, edit_a: &str, edit_b: &str) {
    a.stop();
    b.stop();
    a.write(FILE, edit_a);
    b.write(FILE, edit_b);
    a.start();
    b.start();
}

/// What `bilbo history` lists of a note on each device: the version ids, which devices that converged share.
fn ids(site: &Site, topic: &str) -> Vec<String> {
    let mut ids: Vec<String> = site
        .history(topic)
        .iter()
        .map(|l| l.split(' ').next().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// Both devices send each other a note and read it, so each has run cycles since the last thing worth checking.
fn barrier(a: &Site, b: &Site) {
    let to_b = note(OTHER, "From rivendell.", "To bagend.");
    a.write("plan-other.md", &to_b);
    b.wait_text("plan-other.md", &to_b);
    let to_a = note(OTHER, "From bagend.", "To rivendell.");
    b.write("plan-other.md", &to_a);
    a.wait_text("plan-other.md", &to_a);
}

#[test]
fn edits_to_different_passages_merge_clean_and_the_devices_go_quiet() {
    let (_dir, folder, mut a, mut b, _text) = synced("quiet");
    let edit_a = note(ID, "Install it with care.", "Ship on Monday.");
    let edit_b = note(ID, "Install it.", "Ship on Friday.");
    offline_edits(&mut a, &mut b, &edit_a, &edit_b);
    let merged = note(ID, "Install it with care.", "Ship on Friday.");
    a.wait_text(FILE, &merged);
    b.wait_text(FILE, &merged);
    // Either device may make the merge or receive it: wait until both hold it, and the same versions.
    poll_eq(
        "one history",
        || {
            let merged = |s: &Site| s.events(TOPIC).iter().any(|e| e == "merged");
            merged(&a) && merged(&b) && ids(&a, TOPIC) == ids(&b, TOPIC)
        },
        true,
    );
    let settled = ids(&a, TOPIC);
    let own = segments_with_bytes(&folder, RIVENDELL);
    barrier(&a, &b);
    assert_eq!(ids(&a, TOPIC), settled, "a merge storm on rivendell");
    assert_eq!(ids(&b, TOPIC), settled, "a merge storm on bagend");
    for site in [&a, &b] {
        assert_eq!(site.count("conflict in"), 0, "{:?}", site.lines());
        assert_eq!(
            site.events(TOPIC).iter().filter(|e| *e == "merged").count(),
            1
        );
    }
    // A segment is created once and never changed.
    let later = segments_with_bytes(&folder, RIVENDELL);
    for (path, bytes) in own {
        assert!(
            later.get(&path) == Some(&bytes),
            "{} changed",
            path.display()
        );
    }
}

#[test]
fn a_passage_edited_on_both_sides_is_a_conflict_on_pull() {
    let (_dir, folder, mut a, mut b, _text) = synced("conflict");
    a.stop();
    b.stop();
    // B edits `## Rollout` and pushes it while A is down. A then edits the same passage offline and comes up alone,
    // so it is A that merges: the device that makes the merge prints the line.
    let sent = segments(&folder, BAGEND).len();
    b.write(FILE, &note(ID, "Install it.", "Ship on Tuesday."));
    b.start();
    poll_eq(
        "B's edit pushed",
        || segments(&folder, BAGEND).len(),
        sent + 1,
    );
    b.stop();
    a.write(FILE, &note(ID, "Install it.", "Ship on Friday."));
    a.start();
    let line = "bilbo: notes/decision-release.md: conflict in 1 passage; run bilbo check";
    a.wait_for(line);
    let text = a.wait_holding(FILE, "<<<<<<< bilbo");
    assert!(text.contains("Ship on Friday.") && text.contains("Ship on Tuesday."));
    b.start();
    b.wait_text(FILE, &text);
    // B's text arrives before its log commits the merge, so the ids are polled.
    poll_eq(
        "the same history",
        || ids(&a, TOPIC) == ids(&b, TOPIC),
        true,
    );
    assert_eq!(a.count(line), 1);
}

#[test]
fn a_device_recovered_before_the_folder_was_read_syncs_nothing_until_it_is() {
    let dir = TempDir::new("unread");
    let folder = folder(&dir, &[]);
    let mut b = Site::build("bagend", true, &[], &Site::syncing(&folder));
    b.start();
    let line = "bilbo: sync personal: this device is not in the scope; run bilbo device recover on this device";
    b.wait_for(line);
    assert!(
        !folder.join("scopes").exists(),
        "watch published a manifest"
    );
    // The folder reaches the device: the watcher restores the one scope that lists it, and syncs.
    put_manifests(&folder, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    b.wait_for(&format!(
        "bilbo: sync personal: resumed scope {} from the folder (2 devices)",
        scope()
    ));
    b.wait_text(FILE, &text);
    assert_eq!(b.count(line), 1);
}

#[test]
fn the_history_folder_was_deleted() {
    let (_dir, folder, mut a, b, text) = synced("lost");
    poll_eq("a segment", || segments(&folder, RIVENDELL).len(), 1);
    a.stop();
    fs::remove_dir_all(a.root().join(".bilbo")).unwrap();
    a.start();
    a.wait_for(&format!(
        "bilbo: sync personal: resumed scope {} from the folder (2 devices)",
        scope()
    ));
    // Until the first cycle has put the note back into history, an edit has no base to merge against.
    a.wait_events(TOPIC, &["added"]);
    let edited = format!("{text}\nAfter the loss.\n");
    a.write(FILE, &edited);
    b.wait_text(FILE, &edited);
    let names: Vec<String> = segments(&folder, RIVENDELL)
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        ["00000000000000000001.seg", "00000000000000000002.seg"]
    );
    assert_eq!(a.count("other content"), 0, "{:?}", a.lines());
}

#[test]
fn a_folder_without_this_devices_scopes_is_said_once_and_gets_nothing() {
    let dir = TempDir::new("moved");
    let moved = dir.path().join("moved");
    fs::create_dir_all(&moved).unwrap();
    let mut a = Site::build("rivendell", true, &[1, 2], &Site::syncing(&moved));
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    a.start();
    let line = format!(
        "bilbo: sync personal: {} holds none of this device's scopes; if the folder moved, change scope.personal.sync on every device",
        moved.display()
    );
    a.wait_for(&line);
    a.wait_events(TOPIC, &["added"]);
    assert_eq!(a.count(&line), 1);
    assert!(
        tree(&moved).is_empty(),
        "something was written to the new folder"
    );
}

#[test]
fn an_unreachable_folder_is_said_once_and_the_versions_wait() {
    let dir = TempDir::new("unmounted");
    let folder = dir.path().join("volume/bilbo");
    let mut a = Site::build("rivendell", true, &[1, 2], &Site::syncing(&folder));
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    a.wait_for(&format!(
        "bilbo: sync personal: {} is not reachable:",
        url(&folder)
    ));
    a.wait_events(TOPIC, &["added"]);
    assert!(
        !dir.path().join("volume").exists(),
        "watch created the folder"
    );
    // The volume comes back: the waiting version is pushed.
    put_manifests(&folder, &[1, 2]);
    poll_eq("the push", || segments(&folder, RIVENDELL).len(), 1);
    assert_eq!(a.count("is not reachable"), 1, "{:?}", a.lines());
    let mut b = Site::new("bagend", &folder);
    b.start();
    b.wait_text(FILE, &text);
}

#[test]
fn a_device_added_elsewhere_is_shown_once_and_listed() {
    let dir = TempDir::new("added");
    let folder = folder(&dir, &[1]);
    // The fixture's version 1 lists `bagend` alone; version 2 adds `rivendell`.
    let mut b = Site::build("bagend", true, &[1], &Site::syncing(&folder));
    b.start();
    b.wait_for("bilbo: syncing personal through");
    put_manifests(&folder, &[1, 2]);
    let line = "bilbo: sync personal: device rivendell added by owner key (manifest 2)";
    b.wait_for(line);
    let changes = b
        .root()
        .join(format!(".bilbo/scopes/{}/changes.jsonl", scope()));
    poll_eq("changes.jsonl", || changes.exists(), true);
    assert!(fs::read_to_string(&changes).unwrap().contains("rivendell"));
    // Watch wrote no manifest of its own, and the new device reads what `bagend` pushes under the same epoch.
    let mut a = Site::new("rivendell", &folder);
    let text = note(ID, "Install it.", "Ship on Monday.");
    b.write(FILE, &text);
    a.start();
    a.wait_text(FILE, &text);
    assert_eq!(b.count(line), 1);
    assert_eq!(segments(&folder, BAGEND).len(), 1);
    assert_eq!(
        tree(&folder.join(format!("scopes/{}/manifest", scope()))).len(),
        2
    );
}

#[test]
fn a_damaged_or_deleted_own_segment_comes_back_before_anyone_reads_it() {
    let dir = TempDir::new("damaged");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let text = note(ID, "Install it.", "Ship on Monday.");
    a.write(FILE, &text);
    a.start();
    poll_eq("a segment", || segments(&folder, RIVENDELL).len(), 1);
    let path = segments(&folder, RIVENDELL).remove(0);
    let whole = fs::read(&path).unwrap();
    fs::write(&path, &whole[..whole.len() / 2]).unwrap();
    poll_eq(
        "the replaced file",
        || fs::read(&path).ok(),
        Some(whole.clone()),
    );
    fs::remove_file(&path).unwrap();
    poll_eq("the recreated file", || fs::read(&path).ok(), Some(whole));
    let mut b = Site::new("bagend", &folder);
    b.start();
    b.wait_text(FILE, &text);
}

#[test]
fn a_tampered_segment_is_refused_by_name() {
    let dir = TempDir::new("tampered");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    a.start();
    poll_eq("a segment", || segments(&folder, RIVENDELL).len(), 1);
    a.stop();
    let path = segments(&folder, RIVENDELL).remove(0);
    let mut bytes = fs::read(&path).unwrap();
    let at = bytes
        .windows(14)
        .position(|w| w == b"\"ciphertext\":\"")
        .unwrap()
        + 40;
    bytes[at] = if bytes[at] == b'A' { b'B' } else { b'A' };
    fs::write(&path, bytes).unwrap();
    b.start();
    b.wait_for("bilbo: sync personal: segment 1 of rivendell");
    b.wait_for("segment 1");
    assert_eq!(b.read(FILE), None);
}

#[test]
fn two_stores_with_one_device_key_stop_the_established_one() {
    let dir = TempDir::new("twins");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut twin = Site::new("rivendell", &folder);
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    a.start();
    poll_eq("a segment", || segments(&folder, RIVENDELL).len(), 1);
    let first = segments_with_bytes(&folder, RIVENDELL);
    twin.write(
        "plan-other.md",
        &note(OTHER, "From the twin.", "Never alone."),
    );
    twin.start();
    a.wait_for(
        "bilbo: sync personal: segment 2 of this device holds other content; another store writes as this device",
    );
    // No segment is overwritten: the first is as it was, and the twin's is the second.
    let later = segments_with_bytes(&folder, RIVENDELL);
    assert_eq!(later.len(), 2);
    for (path, bytes) in first {
        assert!(
            later.get(&path) == Some(&bytes),
            "{} changed",
            path.display()
        );
    }
}

/// The text of note `n` of the large push: about 900 KB, under the 1 MiB a note may hold.
fn big(n: usize) -> String {
    let mut text = format!(
        "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4D{n:02X}\ncreated: 2026-10-02T14:23-03:00\nscope: personal\n---\n\n# Big {n}\n\n"
    );
    let mut line = 0;
    while text.len() < 900_000 {
        text.push_str(&format!(
            "Line {line} of note {n}: the quick brown fox jumps over the lazy dog.\n"
        ));
        line += 1;
    }
    text
}

#[test]
fn a_large_first_push_splits_into_segments_of_at_most_eight_mebibytes() {
    let dir = TempDir::new("large");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    for n in 0..20 {
        a.write(&format!("plan-big{n:02}.md"), &big(n));
    }
    a.start();
    b.start();
    poll_eq(
        "the notes arrive",
        || fs::read_dir(b.notes()).unwrap().count(),
        20,
    );
    let sizes: Vec<u64> = segments(&folder, RIVENDELL)
        .iter()
        .map(|p| fs::metadata(p).unwrap().len())
        .collect();
    assert!(sizes.len() >= 3, "{sizes:?}");
    assert!(sizes.iter().all(|s| *s <= 8 * 1024 * 1024), "{sizes:?}");
    assert_eq!(b.read("plan-big07.md"), Some(big(7)));
}

#[test]
fn idle_devices_go_quiet() {
    let (_dir, folder, a, b, text) = synced("idle");
    let edited = format!("{text}\nOne edit.\n");
    a.write(FILE, &edited);
    b.wait_text(FILE, &edited);
    // B acknowledges once. A reads that acknowledgement, which is the cycle after which any chatter would show.
    poll_eq("an acknowledgement", || segments(&folder, BAGEND).len(), 1);
    let state = a
        .root()
        .join(format!(".bilbo/scopes/{}/state.json", scope()));
    poll_eq(
        "A has read the acknowledgement",
        || {
            let json: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&state).unwrap()).unwrap();
            json["cursors"][BAGEND].as_u64()
        },
        Some(1),
    );
    assert_eq!(segments(&folder, RIVENDELL).len(), 2);
    assert_eq!(segments(&folder, BAGEND).len(), 1);
}

#[test]
fn a_forged_manifest_is_ignored_and_named_once() {
    let dir = TempDir::new("forged");
    let folder = folder(&dir, &[1]);
    let mut b = Site::build("bagend", true, &[1], &Site::syncing(&folder));
    b.start();
    b.wait_for("bilbo: syncing personal through");
    let mut forged = manifests(&[2]).remove(&2).unwrap();
    let middle = forged.len() / 2;
    forged[middle] ^= 1;
    let path = folder.join(format!("scopes/{}/manifest/2.json", scope()));
    fs::write(&path, forged).unwrap();
    b.wait_for("bilbo: sync personal: manifest/2.json is invalid");
    // A note that has to travel is the barrier: the cycles that read it have read the forgery again.
    let text = note(ID, "Install it.", "Ship on Monday.");
    b.write(FILE, &text);
    poll_eq("a segment", || segments(&folder, BAGEND).len(), 1);
    assert_eq!(b.count("manifest/2.json"), 1, "{:?}", b.lines());
    let held = b.root().join(format!(".bilbo/scopes/{}/manifest", scope()));
    assert!(held.join("1.json").exists() && !held.join("2.json").exists());
}

#[test]
fn a_new_scope_syncs_at_once() {
    let dir = TempDir::new("fresh-scope");
    let folder = folder(&dir, &[]);
    let mut b = Site::build("bagend", true, &[1], &Site::syncing(&folder));
    let held = b.root().join(format!(".bilbo/scopes/{}/manifest", scope()));
    fs::write(held.join("1.pending"), b"").unwrap();
    b.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    b.start();
    poll_eq("a segment", || segments(&folder, BAGEND).len(), 1);
    let published = folder.join(format!("scopes/{}/manifest/1.json", scope()));
    assert_eq!(fs::read(published).unwrap(), manifests(&[1])[&1]);
    assert!(!held.join("1.pending").exists());
}

#[test]
fn a_version_waits_while_the_notes_folder_is_gone() {
    let (_dir, _folder, a, b, text) = synced("away");
    let away = b.root().join("notes.away");
    fs::rename(b.notes(), &away).unwrap();
    let edited = format!("{text}\nWritten while the folder is away.\n");
    a.write(FILE, &edited);
    // The version reaches the inbox and waits there: no file is written and no folder is made.
    let inbox = b.root().join(".bilbo/sync/inbox.jsonl");
    poll_eq(
        "the inbox",
        || fs::metadata(&inbox).map(|m| m.len() > 0).unwrap_or(false),
        true,
    );
    assert!(!b.notes().exists(), "watch created notes/");
    fs::rename(&away, b.notes()).unwrap();
    b.wait_text(FILE, &edited);
}

const YOUNG: &str = "01M3YJ7R6HK6NQ30DCDB1PQ2X7";

/// The history lines of a note on a device, newest first.
fn history_holds(site: &Site, topic: &str, needle: &str) -> bool {
    site.history(topic).iter().any(|l| l.contains(needle))
}

#[test]
fn a_deletion_travels() {
    let (_dir, _folder, a, b, _text) = synced("deletion");
    // Watch records no deletion while `notes/` holds no note at all, so another note stays.
    a.write("plan-keep.md", &common::note_text(OTHER, "Keep"));
    a.wait_events("keep", &["added"]);
    fs::remove_file(a.notes().join(FILE)).unwrap();
    poll_eq("the file is gone from bagend", || b.read(FILE), None);
    b.wait_events(ID, &["deleted", "added"]);
    assert!(
        b.history(ID)[0].contains("deleted decision-release.md from rivendell"),
        "{:?}",
        b.history(ID)
    );
    assert_eq!(a.read(FILE), None);
}

/// A deletes the note while B, offline, edits it; both start again and hold the edit.
fn edit_beats_delete(name: &str) -> (TempDir, PathBuf, Site, Site) {
    let (dir, folder, mut a, mut b, _text) = synced(name);
    a.stop();
    b.stop();
    fs::remove_file(a.notes().join(FILE)).unwrap();
    let edited = note(ID, "Install it.", "Ship on Friday.");
    b.write(FILE, &edited);
    a.start();
    b.start();
    a.wait_text(FILE, &edited);
    b.wait_text(FILE, &edited);
    poll_eq(
        "one history with the flagged merge",
        || {
            let flagged = |s: &Site| history_holds(s, TOPIC, "edit-beat-delete");
            flagged(&a) && flagged(&b) && ids(&a, TOPIC) == ids(&b, TOPIC)
        },
        true,
    );
    (dir, folder, a, b)
}

#[test]
fn an_edit_beats_a_delete_on_both_devices() {
    let (_dir, _folder, a, b) = edit_beats_delete("edit-beats-delete");
    for site in [&a, &b] {
        assert!(
            site.events(TOPIC).iter().any(|e| e == "merged"),
            "{:?}",
            site.history(TOPIC)
        );
        let flagged: Vec<String> = site
            .history(TOPIC)
            .into_iter()
            .filter(|l| l.contains("edit-beat-delete"))
            .collect();
        assert_eq!(flagged.len(), 1, "{flagged:?}");
        assert!(flagged[0].contains(" merged "), "{flagged:?}");
    }
    // Both devices settle: nothing more is recorded after a round trip.
    let settled = ids(&a, TOPIC);
    barrier(&a, &b);
    assert_eq!(ids(&a, TOPIC), settled);
    assert_eq!(ids(&b, TOPIC), settled);
}

/// One topic made on both devices while they are apart, `old` on `rivendell`, `young` on `bagend` or the reverse.
fn collision(name: &str, a_holds: &str, b_holds: &str) {
    let dir = TempDir::new(name);
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    let old = note(ID, "The older note.", "Ship on Monday.");
    let young = note(YOUNG, "The younger note.", "Ship on Friday.");
    let text = |id: &str| if id == ID { &old } else { &young };
    a.write(FILE, text(a_holds));
    b.write(FILE, text(b_holds));
    a.start();
    b.start();
    for site in [&a, &b] {
        site.wait_text(FILE, &old);
        site.wait_text("decision-release-q2x7.md", &young);
    }
    for site in [&a, &b] {
        let mut names: Vec<String> = fs::read_dir(site.notes())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["decision-release-q2x7.md", "decision-release.md"],
            "{}",
            site.name
        );
        let check = site.bilbo(&["check"]);
        assert!(
            !check.stdout.contains("topic"),
            "check on {}: {}{}",
            site.name,
            check.stdout,
            check.stderr
        );
    }
    // The renames are one version, recorded the same on both devices.
    poll_eq(
        "one history for the renamed note",
        || {
            ids(&a, "release-q2x7") == ids(&b, "release-q2x7")
                && !ids(&a, "release-q2x7").is_empty()
        },
        true,
    );
    let settled = (ids(&a, "release-q2x7"), ids(&a, TOPIC));
    barrier(&a, &b);
    assert_eq!((ids(&a, "release-q2x7"), ids(&a, TOPIC)), settled);
    assert_eq!((ids(&b, "release-q2x7"), ids(&b, TOPIC)), settled);
}

#[test]
fn a_topic_made_on_two_devices_keeps_the_older_note_when_it_is_on_rivendell() {
    collision("collision-a", ID, YOUNG);
}

#[test]
fn a_topic_made_on_two_devices_keeps_the_older_note_when_it_is_on_bagend() {
    collision("collision-b", YOUNG, ID);
}

#[test]
fn a_rename_onto_a_taken_topic_converges() {
    let (_dir, _folder, mut a, mut b, _text) = synced("rename-onto");
    let young = note(YOUNG, "The younger note.", "Ship on Friday.");
    a.write("plan-x.md", &young);
    b.wait_text("plan-x.md", &young);
    a.stop();
    b.stop();
    // A renames `plan-x.md` onto the topic `release` while B, apart, has the older note under it: it is the
    // younger note that gives way.
    fs::rename(
        a.notes().join("plan-x.md"),
        a.notes().join("plan-release.md"),
    )
    .unwrap();
    a.start();
    b.start();
    let old = note(ID, "Install it.", "Ship on Monday.");
    for site in [&a, &b] {
        site.wait_text(FILE, &old);
        site.wait_text("plan-release-q2x7.md", &young);
        poll_eq(
            &format!("{} lists only the renamed files", site.name),
            || {
                let mut names: Vec<String> = fs::read_dir(site.notes())
                    .unwrap()
                    .map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect();
                names.sort();
                names
            },
            vec![
                "decision-release.md".to_string(),
                "plan-release-q2x7.md".to_string(),
            ],
        );
    }
    poll_eq(
        "one history for the renamed note",
        || ids(&a, "release-q2x7") == ids(&b, "release-q2x7"),
        true,
    );
}

const LOCAL_WORK: &[&str] = &["scope.work.sync = off"];

fn in_work(text: &str) -> String {
    text.replacen("scope: personal", "scope: work", 1)
}

#[test]
fn a_note_that_left_a_scope_before_it_synced_never_reaches_the_transport() {
    let dir = TempDir::new("prior");
    let folder = folder(&dir, &[1, 2]);
    let off = vec![
        "scope.personal.sync = off".to_string(),
        "scope.work.sync = off".to_string(),
        "sync.poll_seconds = 1".to_string(),
    ];
    let mut a = Site::build("rivendell", true, &[1, 2], &off);
    let secret = note(ID, "Secret setup.", "Secret rollout.");
    a.write(FILE, &secret);
    a.start();
    a.wait_events(TOPIC, &["added"]);
    // The note moves to `work`, a scope that never syncs, while sync is off everywhere.
    a.write(FILE, &secret.replace("scope: personal", "scope: work"));
    a.wait_events(TOPIC, &["edited", "added"]);
    // `personal` starts to sync; `work` stays local.
    let mut on = Site::syncing(&folder);
    on.push("scope.work.sync = off".to_string());
    a.configure(&on);
    a.wait_for("bilbo: syncing personal through");
    let mut b = Site::new("bagend", &folder);
    b.start();
    // Barriers: another personal note crosses to B and back, so any version of the first one has had its cycles.
    let other = note(OTHER, "From rivendell.", "To bagend.");
    a.write("plan-other.md", &other);
    b.wait_text("plan-other.md", &other);
    let back = note(OTHER, "From bagend.", "To rivendell.");
    b.write("plan-other.md", &back);
    a.wait_text("plan-other.md", &back);
    let log =
        fs::read_to_string(a.root().join(format!(".bilbo/history/notes/{ID}.jsonl"))).unwrap();
    let ids: Vec<String> = log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v["version"].as_str().map(str::to_string))
        .collect();
    assert_eq!(ids.len(), 2, "the note has its two versions here: {log}");
    for id in &ids {
        assert!(
            !holds(&a.root().join(".bilbo/scopes"), id),
            "rivendell's seen.jsonl names {id}"
        );
        assert!(
            !holds(&b.root().join(".bilbo/scopes"), id),
            "bagend's scope state names {id}"
        );
    }
    assert!(
        !b.root()
            .join(format!(".bilbo/history/notes/{ID}.jsonl"))
            .exists(),
        "a note of a local scope reached bagend"
    );
    assert_eq!(b.read(FILE), None);
}

#[test]
fn a_note_that_moves_to_a_local_scope_leaves_the_other_device_and_can_come_back() {
    let (_dir, folder, a, b, text) = synced_with("moves", LOCAL_WORK);
    let sent = segments(&folder, RIVENDELL).len();
    a.write(FILE, &in_work(&text));
    poll_eq("the left file", || b.read(FILE), None);
    b.wait_events(ID, &["left", "added"]);
    b.wait_for("bilbo: sync personal: notes/decision-release.md left the scope; its history stays");
    assert_eq!(b.count("left the scope"), 1);
    // A keeps the note, in `work`, and what it pushed after the move is one more segment with no text in it.
    assert_eq!(a.read(FILE), Some(in_work(&text)));
    poll_eq(
        "the marker",
        || segments(&folder, RIVENDELL).len(),
        sent + 1,
    );
    assert!(!holds(&folder, "Ship on"));
    // It moves back: B writes the file with that version, and nothing is flagged.
    let back = format!("{text}\nBack in personal.\n");
    a.write(FILE, &back);
    b.wait_text(FILE, &back);
    assert!(!history_holds(&b, TOPIC, "edit-beat-delete"));
    assert!(!history_holds(&a, TOPIC, "edit-beat-delete"));
    assert_eq!(b.events(ID)[0], "edited");
}

#[test]
fn a_note_that_loses_its_scope_key_leaves_the_other_device_and_stays_here() {
    let (_dir, _folder, a, b, text) = synced("loses-key");
    let keyless = text.replacen("scope: personal\n", "", 1);
    a.write(FILE, &keyless);
    poll_eq("the left file", || b.read(FILE), None);
    b.wait_events(ID, &["left", "added"]);
    assert_eq!(a.read(FILE), Some(keyless));
}

#[test]
fn a_left_marker_against_a_local_edit_waits_for_the_merge_that_follows_the_edit() {
    let (_dir, _folder, mut a, mut b, text) = synced_with("left-vs-edit", LOCAL_WORK);
    a.stop();
    b.stop();
    a.write(FILE, &in_work(&text));
    let edited = note(ID, "Install it.", "Ship on Friday.");
    b.write(FILE, &edited);
    a.start();
    b.start();
    // B's edit reaches A through `personal` next to A's own marker. A's merge follows the edit and comes back as
    // a `left`, and only then does B remove the file: it said nothing about the first marker, which it held back.
    poll_eq("B removes the file", || b.read(FILE), None);
    b.wait_for("bilbo: sync personal: notes/decision-release.md left the scope; its history stays");
    b.wait_events(ID, &["left", "left", "edited", "added"]);
    a.wait_events(ID, &["merged", "edited", "edited", "added"]);
    assert_eq!(b.count("left the scope"), 1, "{:?}", b.lines());
    assert_eq!(a.read(FILE), Some(in_work(&edited)));
}

// `bilbo sync`: the status report and `declare` through the binary.

/// The state of `scope()`'s `state.json` with every field of a fresh one, `fields` over it.
fn write_state(site: &Site, fields: serde_json::Value) {
    let mut state = serde_json::json!({
        "name": "personal", "device": RIVENDELL, "own": 0, "cursors": {}, "acks": {}, "sent": {},
        "owed": false, "last_ack": null, "listed": [], "cutoffs": {}, "marked": [], "since": {},
        "pulled_at": null, "pushed_at": null, "stops": {}, "error": null, "stopped": null, "halted": null,
    });
    for (key, value) in fields.as_object().unwrap() {
        state[key] = value.clone();
    }
    let dir = site.root().join(format!(".bilbo/scopes/{}", scope()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("state.json"), state.to_string()).unwrap();
}

/// Holds the watcher's lock, as a running `bilbo watch` does, until dropped.
fn watching(site: &Site) -> fs::File {
    let path = site.root().join(".bilbo/watch.lock");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .unwrap();
    file.lock().unwrap();
    file
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

const DAY: i64 = 86_400;

/// A second as the report writes it, to the minute in UTC.
fn minute(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%Y-%m-%dT%H:%M%:z")
        .to_string()
}

fn sent(at: i64) -> serde_json::Value {
    serde_json::json!({"at": at, "versions": true})
}

impl Site {
    /// `bilbo sync <args>`, in UTC so the times of the report are known.
    fn sync(&self, args: &[&str]) -> Run {
        let mut env = self.pairs();
        env.push(("TZ", "UTC"));
        let mut all = vec!["sync"];
        all.extend_from_slice(args);
        bilbo(self.dir.path(), &env, &all)
    }

    /// Waits until a run of `bilbo sync` is one that `want` accepts, and returns that run.
    fn wait_status(&self, what: &str, want: impl Fn(&Run) -> bool) -> Run {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            let run = self.sync(&[]);
            if want(&run) {
                return run;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{what} on {}: last run {} stdout {:?} stderr {:?}",
                self.name,
                run.code,
                run.stdout,
                run.stderr
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

fn has_line(run: &Run, line: &str) -> bool {
    run.stdout.lines().any(|l| l == line)
}

fn has_prefix(run: &Run, prefix: &str) -> bool {
    run.stdout.lines().any(|l| l.starts_with(prefix))
}

fn stderr_has(run: &Run, line: &str) -> bool {
    run.stderr.lines().any(|l| l == line)
}

/// A site that no watcher runs on: keys, the fixture manifests and a syncing `personal`.
fn quiet(name: &str) -> (TempDir, PathBuf, Site) {
    let dir = TempDir::new(name);
    let folder = folder(&dir, &[1, 2]);
    let site = Site::new("rivendell", &folder);
    (dir, folder, site)
}

#[test]
fn status_of_two_devices_in_step() {
    let (_dir, folder, a, b, _text) = synced("status-step");
    a.write(
        "plan-unassigned.md",
        &common::note_text(OTHER, "Unassigned"),
    );
    a.write(
        "plan-work.md",
        &common::in_scope(
            &common::note_text("01M3YJ7R6HK6NQ30DCDB1P4D00", "Work"),
            "work",
        ),
    );
    a.wait_events("unassigned", &["added"]);
    a.wait_events("work", &["added"]);
    let run = a.wait_status("in step", |run| {
        has_line(run, "device personal bagend: up to date")
            && has_prefix(run, "scope ")
            && !run.stdout.contains("never")
    });
    let lines: Vec<&str> = run.stdout.lines().collect();
    let head = format!("scope personal {}: 1 notes, pushed 20", url(&folder));
    assert!(
        lines[0].starts_with(&head) && lines[0].contains(", pulled 20"),
        "{lines:?}"
    );
    assert_eq!(
        lines[1..],
        [
            "device personal rivendell: this device",
            "device personal bagend: up to date",
            "local: 2 notes sync nowhere"
        ],
        "{lines:?}"
    );
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    // The other device sees the same.
    let run = b.wait_status("in step", |run| {
        has_line(run, "device personal rivendell: up to date")
    });
    assert!(has_line(&run, "device personal bagend: this device"));
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
}

#[test]
fn an_open_conflict_is_listed_and_fails_on_both_devices() {
    let (_dir, folder, mut a, mut b, _text) = synced("status-conflict");
    a.stop();
    b.stop();
    let sent_by_b = segments(&folder, BAGEND).len();
    b.write(FILE, &note(ID, "Install it.", "Ship on Tuesday."));
    b.start();
    poll_eq(
        "B's edit",
        || segments(&folder, BAGEND).len(),
        sent_by_b + 1,
    );
    b.stop();
    a.write(FILE, &note(ID, "Install it.", "Ship on Friday."));
    a.start();
    let text = a.wait_holding(FILE, "<<<<<<< bilbo");
    b.start();
    b.wait_text(FILE, &text);
    for site in [&a, &b] {
        let run = site.wait_status("a conflict", |run| {
            has_line(run, "conflict notes/decision-release.md: 1 passage")
        });
        assert_eq!(run.code, 1, "{}", run.stdout);
        assert_eq!(run.stderr, "", "a watcher runs");
    }
}

/// An open conflict on both devices, where A has already removed the markers by keeping its own side.
fn resolved_by_dropping(name: &str) -> (TempDir, PathBuf, Site, Site, String) {
    let (dir, folder, mut a, mut b, _text) = synced(name);
    a.stop();
    b.stop();
    let sent_by_b = segments(&folder, BAGEND).len();
    b.write(FILE, &note(ID, "Install it.", "Ship on Tuesday."));
    b.start();
    poll_eq(
        "B's edit",
        || segments(&folder, BAGEND).len(),
        sent_by_b + 1,
    );
    b.stop();
    a.write(FILE, &note(ID, "Install it.", "Ship on Friday."));
    a.start();
    let conflicted = a.wait_holding(FILE, "<<<<<<< bilbo");
    b.start();
    b.wait_text(FILE, &conflicted);
    let resolved = note(ID, "Install it.", "Ship on Friday.");
    (dir, folder, a, b, resolved)
}

#[test]
fn dropped_text_is_counted_until_it_is_declared_and_the_declaration_syncs() {
    let (_dir, _folder, a, b, resolved) = resolved_by_dropping("status-dropped");
    a.write(FILE, &resolved);
    let dropped = "dropped notes/decision-release.md: 1 line not declared";
    let run = a.wait_status("the dropped line", |run| has_line(run, dropped));
    assert_eq!(run.code, 1);
    assert!(!run.stdout.contains("conflict notes"), "{}", run.stdout);
    b.wait_text(FILE, &resolved);
    b.wait_status("the dropped line", |run| has_line(run, dropped));
    assert!(b.bilbo(&["check"]).stdout.contains("dropped 1 lines"));
    let declared = a.sync(&["declare", "release", "Tuesday was superseded"]);
    assert_eq!(
        declared.stdout,
        "declared decision-release.md: 1 lines dropped on purpose\n"
    );
    assert_eq!((declared.code, declared.stderr.as_str()), (0, ""));
    assert!(!a.bilbo(&["check"]).stdout.contains("dropped"));
    let run = a.wait_status("no dropped line", |run| !has_prefix(run, "dropped "));
    assert_eq!(run.code, 0);
    // The declaration syncs with the note.
    poll_eq(
        "the declaration reaches bagend",
        || b.bilbo(&["check"]).stdout.contains("dropped"),
        false,
    );
    b.wait_status("no dropped line", |run| !has_prefix(run, "dropped "));
}

#[test]
fn declaring_before_the_watcher_records_the_resolution_applies_to_that_conflict() {
    let (_dir, _folder, mut a, _b, resolved) = resolved_by_dropping("status-declare-early");
    a.stop();
    a.write(FILE, &resolved);
    let declared = a.sync(&["declare", "release", "B's date was superseded"]);
    assert_eq!(
        declared.stdout,
        "declared decision-release.md: 1 lines dropped on purpose\n"
    );
    assert_eq!(declared.code, 0, "{}", declared.stderr);
    a.start();
    a.wait_events(TOPIC, &["edited", "merged", "edited", "edited", "added"]);
    assert!(!a.bilbo(&["check"]).stdout.contains("dropped"));
    let run = a.wait_status("no dropped line", |run| !has_prefix(run, "dropped "));
    assert_eq!(run.code, 0);
}

#[test]
fn declare_without_dropped_text_and_with_a_bad_reason() {
    let (_dir, _folder, a, _b, _text) = synced("status-nothing");
    let run = a.sync(&["declare", "release", "x"]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (1, "", "bilbo: release has no dropped text to declare\n")
    );
    let run = a.sync(&["declare", "release", "one\ntwo"]);
    assert_eq!((run.code, run.stdout.as_str()), (2, ""));
    assert!(run.stderr.starts_with("bilbo: "), "{}", run.stderr);
}

#[test]
fn an_edit_that_beat_a_delete_is_a_notice_on_both_devices() {
    let (_dir, _folder, a, b) = edit_beats_delete("status-notice");
    for site in [&a, &b] {
        let run = site.wait_status("the notice", |run| {
            run.stdout.lines().any(|l| {
                l.starts_with("notice 20")
                    && l.ends_with(" notes/decision-release.md: edit-beat-delete")
            })
        });
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    }
}

#[test]
fn a_device_added_elsewhere_is_listed_as_a_change() {
    let dir = TempDir::new("status-added");
    let folder = folder(&dir, &[1]);
    let mut b = Site::build("bagend", true, &[1], &Site::syncing(&folder));
    b.start();
    b.wait_for("bilbo: syncing personal through");
    put_manifests(&folder, &[1, 2]);
    b.wait_for("bilbo: sync personal: device rivendell added by owner key (manifest 2)");
    let run = b.wait_status("the change", |run| has_prefix(run, "change "));
    let change = run
        .stdout
        .lines()
        .find(|l| l.starts_with("change "))
        .unwrap();
    assert!(
        change.starts_with("change 20")
            && change.ends_with(" personal: device rivendell added by owner key (manifest 2)"),
        "{change}"
    );
    assert!(
        run.stdout.contains("device personal rivendell: "),
        "{}",
        run.stdout
    );
}

#[test]
fn changes_of_the_last_thirty_days_are_listed_and_older_ones_are_not() {
    let (_dir, _folder, a) = quiet("status-changes");
    let _lock = watching(&a);
    let moria = common::days_ago(3);
    let epoch = common::days_ago(1);
    let old = common::days_ago(31);
    let lines = [
        serde_json::json!({"at": moria, "n": 4, "kind": "device", "device": "moria", "signer": "owner key"}),
        serde_json::json!({"at": epoch, "n": 5, "kind": "epoch", "signer": "owner key"}),
        serde_json::json!({"at": old, "n": 3, "kind": "device", "device": "gone", "signer": "owner key"}),
    ];
    let text: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    fs::write(
        a.root()
            .join(format!(".bilbo/scopes/{}/changes.jsonl", scope())),
        text.join("\n") + "\n",
    )
    .unwrap();
    let run = a.sync(&[]);
    // The time is kept in the offset it was written with, to the minute.
    let created = |at: &str| format!("{}{}", &at[..16], &at[19..]);
    let changes: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("change "))
        .collect();
    assert_eq!(
        changes,
        [
            format!(
                "change {} personal: device moria added by owner key (manifest 4)",
                created(&moria)
            ),
            format!(
                "change {} personal: epoch changed (manifest 5)",
                created(&epoch)
            ),
        ],
        "{}",
        run.stdout
    );
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
}

#[test]
fn a_flag_from_yesterday_is_a_notice_and_an_old_one_is_not() {
    let (_dir, _folder, a) = quiet("status-notices");
    let _lock = watching(&a);
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    let yesterday = common::days_ago(1);
    let old = common::days_ago(8);
    let open = serde_json::json!({"notes": {ID: {"file": FILE, "notices": [
        {"at": yesterday, "flag": "edit-beat-delete"},
        {"at": old, "flag": "stale-base"},
    ]}}});
    let dir = a.root().join(".bilbo/sync");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("open.json"), open.to_string()).unwrap();
    let run = a.sync(&[]);
    let notices: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("notice "))
        .collect();
    assert_eq!(
        notices,
        [format!(
            "notice {}{} notes/decision-release.md: edit-beat-delete",
            &yesterday[..16],
            &yesterday[19..]
        )],
        "{}",
        run.stdout
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
}

#[test]
fn a_device_behind_a_week_a_device_gone_for_good_and_a_wrong_clock() {
    let (_dir, _folder, a) = quiet("status-devices");
    let _lock = watching(&a);
    a.configure(&[
        format!("scope.personal.sync = {}", "file:///srv/bilbo"),
        "sync.stale_days = 180".to_string(),
    ]);
    let t = now();
    // bagend took nothing of the last 4 segments, the oldest from 7 days ago.
    write_state(
        &a,
        serde_json::json!({
            "own": 4,
            "sent": {"1": sent(t - 7 * DAY), "2": sent(t - 6 * DAY), "3": sent(t - 5 * DAY), "4": sent(t - 4 * DAY)},
            "acks": {BAGEND: {RIVENDELL: 0}},
        }),
    );
    let run = a.sync(&[]);
    assert!(
        has_line(&run, "device personal bagend: behind by 4 segments"),
        "{}",
        run.stdout
    );
    assert!(has_line(&run, "device personal rivendell: this device"));
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    // A segment from 200 days ago that it never took.
    let long_ago = t - 200 * DAY;
    write_state(
        &a,
        serde_json::json!({
            "own": 1, "sent": {"1": sent(long_ago)}, "acks": {BAGEND: {RIVENDELL: 0}},
            "since": {BAGEND: long_ago - DAY},
        }),
    );
    let run = a.sync(&[]);
    let stale = format!("device personal bagend: stale since {}", minute(long_ago));
    assert!(has_line(&run, &stale), "{}", run.stdout);
    // A clock a year behind acknowledges every segment within minutes: only the acknowledgements count.
    write_state(
        &a,
        serde_json::json!({
            "own": 2, "sent": {"1": sent(t - 3600), "2": sent(t - 3000)},
            "acks": {BAGEND: {RIVENDELL: 2}},
        }),
    );
    let run = a.sync(&[]);
    assert!(
        has_line(&run, "device personal bagend: up to date"),
        "{}",
        run.stdout
    );
}

#[test]
fn versions_held_back_for_a_version_that_never_arrived_are_listed() {
    let (_dir, _folder, a) = quiet("status-waiting");
    let _lock = watching(&a);
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    let staged = |version: &str, parent: &str| {
        serde_json::json!({
            "seen": "2027-01-15T07:00:00Z", "scope": "personal",
            "record": {
                "note": ID, "version": version.repeat(64), "parents": [parent.repeat(64)],
                "file": FILE, "blob": "d".repeat(64), "event": "edited",
                "at": "2027-01-15T07:00:00+00:00", "device": BAGEND,
            },
        })
        .to_string()
    };
    // Version 1 follows 0, which never arrived; 2 follows 1.
    let lines = [staged("1", "0"), staged("2", "1")];
    let dir = a.root().join(".bilbo/sync");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("inbox.jsonl"), lines.join("\n") + "\n").unwrap();
    let run = a.sync(&[]);
    assert!(
        has_line(&run, "waiting personal bagend: 2 versions"),
        "{}",
        run.stdout
    );
}

#[test]
fn an_unreachable_folder_is_named_on_stderr_and_still_listed() {
    let (dir, folder, a, _b, _text) = synced("status-unmounted");
    let away = dir.path().join("folder.away");
    fs::rename(&folder, &away).unwrap();
    a.wait_for(&format!(
        "bilbo: sync personal: {} is not reachable:",
        url(&folder)
    ));
    let run = a.wait_status("the error", |run| {
        run.stderr.contains("not reachable since")
    });
    assert_eq!(run.code, 1);
    assert!(
        has_prefix(&run, &format!("scope personal {}: ", url(&folder))),
        "{}",
        run.stdout
    );
    let line = run
        .stderr
        .lines()
        .find(|l| l.contains("not reachable since"))
        .unwrap();
    assert!(
        line.starts_with(&format!(
            "bilbo: sync personal: {} not reachable since 20",
            url(&folder)
        )) && line.ends_with(": the folder does not exist"),
        "{line}"
    );
    assert!(!folder.exists(), "status or watch created the folder");
    // It comes back: the error leaves the report.
    fs::rename(&away, &folder).unwrap();
    let run = a.wait_status("no error", |run| !run.stderr.contains("not reachable"));
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
}

#[test]
fn a_folder_that_was_never_reachable_is_named_on_stderr() {
    let dir = TempDir::new("status-never");
    let folder = dir.path().join("volume/bilbo");
    let mut a = Site::build("rivendell", true, &[1, 2], &Site::syncing(&folder));
    a.write(FILE, &note(ID, "Install it.", "Ship on Monday."));
    a.start();
    a.wait_for(&format!(
        "bilbo: sync personal: {} is not reachable:",
        url(&folder)
    ));
    let run = a.wait_status("the error", |run| {
        run.stderr.contains("not reachable since")
    });
    assert_eq!(run.code, 1);
    let line = run
        .stderr
        .lines()
        .find(|l| l.contains("not reachable since"))
        .unwrap();
    assert!(
        line.starts_with(&format!(
            "bilbo: sync personal: {} not reachable since 20",
            url(&folder)
        )) && line.ends_with(": the folder does not exist"),
        "{line}"
    );
    assert!(
        !dir.path().join("volume").exists(),
        "watch created the folder"
    );
    // The volume comes back: the error leaves the report.
    put_manifests(&folder, &[1, 2]);
    let run = a.wait_status("no error", |run| !run.stderr.contains("not reachable"));
    assert_eq!(run.code, 0, "{}", run.stderr);
}

#[test]
fn a_reader_stuck_at_a_missing_segment_is_named_on_stderr() {
    let dir = TempDir::new("status-gap");
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
    // B writes four segments while A is not running, then loses its third.
    b.start();
    for n in 1..=4 {
        b.write(FILE, &note(ID, "Install it.", &format!("Ship on day {n}.")));
        poll_eq("B's segment", || segments(&folder, BAGEND).len(), n);
    }
    b.stop();
    fs::remove_file(&segments(&folder, BAGEND)[2]).unwrap();
    a.start();
    let line = "bilbo: sync personal: bagend stopped at segment 3: missing";
    let run = a.wait_status("the stop", |run| stderr_has(run, line));
    assert_eq!(run.code, 1);
    assert!(has_prefix(&run, "scope personal "), "{}", run.stdout);
}

#[test]
fn a_scope_the_manifest_pins_elsewhere_is_named_on_stderr() {
    let (_dir, _folder, a) = quiet("status-pin");
    let _lock = watching(&a);
    let pin = "sync personal: the manifest pins file://, the config says file:///srv/bilbo; run bilbo device init to move the scope, or set the config back";
    write_state(&a, serde_json::json!({"stopped": pin}));
    let run = a.sync(&[]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr, format!("bilbo: {pin}\n"));
    assert!(has_prefix(&run, "scope personal "), "{}", run.stdout);
}

#[test]
fn without_a_watcher_status_says_so_and_fails() {
    let (_dir, _folder, a) = quiet("status-no-watcher");
    let run = a.sync(&[]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        "bilbo: bilbo watch is not running; nothing syncs\n"
    );
    assert!(has_prefix(&run, "scope personal "), "{}", run.stdout);
}

#[test]
fn sync_that_was_never_turned_on_is_a_refusal() {
    let (_dir, _folder, a) = quiet("status-off");
    a.configure(&["scope.personal.sync = off".to_string()]);
    let run = a.sync(&[]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no scope syncs; set scope.<name>.sync in {}\n",
            a.dir.path().join("config").display()
        )
    );
}

#[test]
fn an_unknown_argument_is_a_usage_error_and_a_wrong_home_has_no_store() {
    let (dir, _folder, a) = quiet("status-args");
    let run = a.sync(&["now"]);
    assert_eq!((run.code, run.stdout.as_str()), (2, ""));
    assert!(
        run.stderr.starts_with("bilbo: ") && run.stderr.contains("now"),
        "{}",
        run.stderr
    );
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let mut env = a.pairs();
    env.retain(|(k, _)| *k != "BILBO_HOME");
    env.push(("BILBO_HOME", empty.to_str().unwrap()));
    let run = bilbo(dir.path(), &env, &["sync"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", empty.display())
    );
}

/// The metadata of every entry under `dir`: bytes and modification times.
fn stamps(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>, std::time::SystemTime)> {
    common::snapshot(dir)
        .into_iter()
        .map(|(path, (bytes, at))| (path, bytes, at))
        .collect()
}

#[test]
fn status_changes_nothing_in_the_root_or_the_folder() {
    let (_dir, folder, mut a, mut b, _text) = resolved_by_dropping("status-readonly");
    a.write(FILE, &note(ID, "Install it.", "Ship on Friday."));
    a.wait_status("the dropped line", |run| has_prefix(run, "dropped "));
    // Both devices stop, so nothing but `bilbo sync` can touch what is compared.
    a.stop();
    b.stop();
    let (root, shared) = (stamps(&a.root()), stamps(&folder));
    let run = a.sync(&[]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("dropped notes/"), "{}", run.stdout);
    assert_eq!(stamps(&a.root()), root, "the root changed");
    assert_eq!(stamps(&folder), shared, "the folder changed");
}
