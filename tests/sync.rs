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
    let dir = TempDir::new(name);
    let folder = folder(&dir, &[1, 2]);
    let mut a = Site::new("rivendell", &folder);
    let mut b = Site::new("bagend", &folder);
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
    assert_eq!(segments(&folder, RIVENDELL).len(), before + 1);
    assert_eq!(b.read(FILE), Some(edited));
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
    assert_eq!(ids(&a, TOPIC), ids(&b, TOPIC));
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
