//! `bilbo watch` through the built binary: each test runs it as a child with its own `BILBO_HOME` and polls
//! `bilbo history` for the result, never a fixed sleep. A check that something was not recorded waits for a later
//! change to another note, since a scan that records that one has also read the first.

mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use common::{
    IDS, Seed, TempDir, Watcher, bilbo, config, days_ago, note_text, poll_eq, snapshot, store,
    write,
};

fn home(root: &Path) -> [(&str, &str); 1] {
    [("BILBO_HOME", root.to_str().unwrap())]
}

/// The events of a note's versions, newest first; empty while it has none.
fn events(root: &Path, topic: &str) -> Vec<String> {
    let run = bilbo(root, &home(root), &["history", topic]);
    if run.code != 0 {
        return Vec::new();
    }
    run.stdout
        .lines()
        .map(|line| line.split(' ').nth(2).unwrap().to_string())
        .collect()
}

fn wait_events(root: &Path, topic: &str, want: &[&str]) {
    let want: Vec<String> = want.iter().map(|e| e.to_string()).collect();
    poll_eq(&format!("events of {topic}"), || events(root, topic), want);
}

/// The text of a note's newest version.
fn newest(root: &Path, topic: &str) -> String {
    let list = bilbo(root, &home(root), &["history", topic]);
    let version = list.stdout.split(' ').next().unwrap().to_string();
    bilbo(root, &home(root), &["history", topic, &version]).stdout
}

/// A store with `decision-release.md`, `plan-other.md` and `plan-old.md`.
fn three(dir: &TempDir) -> std::path::PathBuf {
    let root = store(dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    write(&root, "plan-other.md", &note_text(IDS[1], "Other"));
    write(&root, "plan-old.md", &note_text(IDS[2], "Old"));
    root
}

fn wait_added(root: &Path, topics: &[&str]) {
    for topic in topics {
        wait_events(root, topic, &["added"]);
    }
}

#[test]
fn watch_starts() {
    let dir = TempDir::new("watch-starts");
    let root = store(&dir);
    let mut watcher = Watcher::on(&root);
    assert_eq!(
        watcher.lines(),
        [format!("bilbo: watching {}/notes", root.display())]
    );
    assert!(watcher.running());
}

#[test]
fn no_store() {
    let dir = TempDir::new("watch-no-store");
    let root = dir.path().join("missing");
    let run = bilbo(dir.path(), &home(&root), &["watch"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", root.display())
    );
    assert!(!root.exists());
}

#[test]
fn a_store_without_notes_waits() {
    let dir = TempDir::new("watch-no-notes");
    let root = dir.path().join("store");
    fs::create_dir_all(root.join("library")).unwrap();
    let mut watcher = Watcher::start(&home(&root));
    watcher.wait_for("bilbo: cannot read ");
    let notes = root.join("notes");
    let line = watcher.lines().remove(0);
    assert!(
        line.starts_with(&format!("bilbo: cannot read {}: ", notes.display()))
            && line.ends_with("; waiting"),
        "{line}"
    );
    assert!(watcher.running());
    fs::create_dir_all(&notes).unwrap();
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    watcher.wait_for(&format!("bilbo: watching {}", notes.display()));
    wait_events(&root, "release", &["added"]);
}

#[test]
fn an_argument_is_a_usage_error() {
    let dir = TempDir::new("watch-argument");
    let root = store(&dir);
    let run = bilbo(dir.path(), &home(&root), &["watch", "--now"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with("bilbo: unknown option '--now'\n"));
}

#[test]
fn a_second_watcher_waits() {
    let dir = TempDir::new("watch-second");
    let root = three(&dir);
    let mut first = Watcher::on(&root);
    wait_added(&root, &["release", "other", "old"]);
    let mut second = Watcher::start(&home(&root));
    second.wait_for(&format!(
        "bilbo: bilbo watch is already running for {}; waiting",
        root.display()
    ));
    assert!(first.running() && second.running());
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nA paragraph.\n");
    write(&root, "decision-release.md", &text);
    wait_events(&root, "release", &["edited", "added"]);
    assert_eq!(second.count("watching"), 0);
}

#[test]
fn the_second_takes_over() {
    let dir = TempDir::new("watch-takeover");
    let root = three(&dir);
    let mut first = Watcher::on(&root);
    wait_added(&root, &["release", "other", "old"]);
    let second = Watcher::start(&home(&root));
    second.wait_for("already running");
    first.stop();
    second.wait_for(&format!("bilbo: watching {}/notes", root.display()));
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nA paragraph.\n");
    write(&root, "decision-release.md", &text);
    wait_events(&root, "release", &["edited", "added"]);
}

#[test]
fn history_does_not_block_a_starting_watcher() {
    let dir = TempDir::new("watch-history-probe");
    let root = store(&dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    {
        let _seed = Watcher::on(&root);
        wait_events(&root, "release", &["added"]);
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let looping = {
        let (stop, root) = (stop.clone(), root.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                bilbo(&root, &home(&root), &["history", "release"]);
            }
        })
    };
    let watcher = Watcher::on(&root);
    let waited = watcher.count("already running");
    // Killed before the join: on macOS the watcher can inherit the pipe of a `bilbo history` run and keep it open.
    drop(watcher);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    looping.join().unwrap();
    assert_eq!(waited, 0);
}

#[test]
fn two_stores_two_watchers() {
    let (a, b) = (TempDir::new("watch-a"), TempDir::new("watch-b"));
    let (root_a, root_b) = (store(&a), store(&b));
    let mut first = Watcher::on(&root_a);
    let mut second = Watcher::on(&root_b);
    assert!(first.running() && second.running());
    assert_eq!(second.count("already running"), 0);
}

#[test]
fn a_new_note_is_added() {
    let dir = TempDir::new("watch-new");
    let root = store(&dir);
    let _watcher = Watcher::on(&root);
    let run = bilbo(dir.path(), &home(&root), &["new", "decision", "release"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    wait_events(&root, "release", &["added"]);
}

#[test]
fn an_edit_is_recorded() {
    let dir = TempDir::new("watch-edit");
    let root = three(&dir);
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release"]);
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nAnother paragraph.\n");
    write(&root, "decision-release.md", &text);
    wait_events(&root, "release", &["edited", "added"]);
    assert_eq!(newest(&root, "release"), text);
}

#[test]
fn a_rename_keeps_the_note() {
    let (_dir, root) = three_store("watch-rename");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release"]);
    fs::rename(
        root.join("notes/decision-release.md"),
        root.join("notes/plan-release.md"),
    )
    .unwrap();
    wait_events(&root, "release", &["renamed", "added"]);
    let run = bilbo(&root, &home(&root), &["history", "release"]);
    assert!(
        run.stdout
            .lines()
            .next()
            .unwrap()
            .ends_with("renamed plan-release.md")
    );
}

fn three_store(name: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new(name);
    let root = three(&dir);
    (dir, root)
}

#[test]
fn a_deletion_is_recorded() {
    let (_dir, root) = three_store("watch-delete");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other", "old"]);
    fs::remove_file(root.join("notes/plan-old.md")).unwrap();
    wait_events(&root, "old", &["deleted", "added"]);
    assert_eq!(events(&root, "release"), ["added"]);
    assert_eq!(events(&root, "other"), ["added"]);
}

#[test]
fn a_burst_of_saves_is_one_version() {
    let (_dir, root) = three_store("watch-burst");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release"]);
    let mut last = String::new();
    for save in 1..=5 {
        last = format!("{}\nSave {save}.\n", note_text(IDS[0], "Release"));
        write(&root, "decision-release.md", &last);
        std::thread::sleep(Duration::from_secs(1));
    }
    wait_events(&root, "release", &["edited", "added"]);
    assert_eq!(newest(&root, "release"), last);
}

#[test]
fn busy_neighbors_do_not_hold_a_note_back() {
    let (_dir, root) = three_store("watch-busy");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other"]);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let busy = {
        let (stop, root) = (stop.clone(), root.clone());
        std::thread::spawn(move || {
            let mut n = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                n += 1;
                write(
                    &root,
                    "plan-other.md",
                    &format!("{}\nSave {n}.\n", note_text(IDS[1], "Other")),
                );
                std::thread::sleep(Duration::from_secs(1));
            }
        })
    };
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nA paragraph.\n");
    write(&root, "decision-release.md", &text);
    let started = std::time::Instant::now();
    wait_events(&root, "release", &["edited", "added"]);
    let waited = started.elapsed();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    busy.join().unwrap();
    assert!(waited < Duration::from_secs(14), "{waited:?}");
}

#[test]
fn touching_a_file_records_nothing() {
    let (_dir, root) = three_store("watch-touch");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other"]);
    age_the_cache(&root);
    let touched = std::process::Command::new("touch")
        .arg(root.join("notes/decision-release.md"))
        .status()
        .unwrap();
    assert!(touched.success());
    let mut other = note_text(IDS[1], "Other");
    other.push_str("\nMore.\n");
    write(&root, "plan-other.md", &other);
    wait_events(&root, "other", &["edited", "edited", "added"]);
    assert_eq!(events(&root, "release"), ["added"]);
}

#[test]
fn a_rewrite_that_keeps_the_modification_time_is_recorded() {
    let (dir, root) = three_store("watch-cp-p");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other"]);
    age_the_cache(&root);
    let target = root.join("notes/decision-release.md");
    let source = dir.path().join("source.md");
    let text = note_text(IDS[0], "Releasf");
    assert_eq!(text.len(), note_text(IDS[0], "Release").len());
    fs::write(&source, &text).unwrap();
    let same = |args: &[&Path]| {
        std::process::Command::new(args[0].to_str().unwrap())
            .args(&args[1..])
            .status()
            .unwrap()
            .success()
    };
    assert!(same(&[
        Path::new("touch"),
        Path::new("-r"),
        &target,
        &source
    ]));
    assert!(same(&[Path::new("cp"), Path::new("-p"), &source, &target]));
    wait_events(&root, "release", &["edited", "added"]);
    assert_eq!(newest(&root, "release"), text);
}

#[test]
fn the_folder_is_moved_away_and_back() {
    let (_dir, root) = three_store("watch-moved");
    let watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other", "old"]);
    fs::rename(root.join("notes"), root.join("notes.bak")).unwrap();
    watcher.wait_for("bilbo: cannot read ");
    fs::rename(root.join("notes.bak"), root.join("notes")).unwrap();
    watcher.wait_count("bilbo: watching ", 2);
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nBack.\n");
    write(&root, "decision-release.md", &text);
    wait_events(&root, "release", &["edited", "added"]);
    assert_eq!(events(&root, "other"), ["added"]);
    assert_eq!(events(&root, "old"), ["added"]);
}

#[test]
fn an_emptied_folder_records_no_deletions() {
    let (_dir, root) = three_store("watch-emptied");
    let watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other", "old"]);
    for name in ["decision-release.md", "plan-other.md", "plan-old.md"] {
        fs::remove_file(root.join("notes").join(name)).unwrap();
    }
    watcher.wait_for(&format!(
        "bilbo: {}/notes holds no notes; not recording deletions",
        root.display()
    ));
    for id in IDS {
        assert_eq!(events(&root, id), ["added"]);
    }
}

#[test]
fn one_note_among_others_is_deleted() {
    let (_dir, root) = three_store("watch-one-deleted");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other", "old"]);
    fs::remove_file(root.join("notes/plan-other.md")).unwrap();
    wait_events(&root, "other", &["deleted", "added"]);
    assert_eq!(events(&root, "release"), ["added"]);
    assert_eq!(events(&root, "old"), ["added"]);
}

#[test]
fn changes_while_stopped_are_recorded_at_start() {
    let (_dir, root) = three_store("watch-while-stopped");
    {
        let _first = Watcher::on(&root);
        wait_added(&root, &["release", "other", "old"]);
    }
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nOne.\n");
    write(&root, "decision-release.md", &text);
    text.push_str("\nTwo.\n");
    write(&root, "decision-release.md", &text);
    fs::remove_file(root.join("notes/plan-old.md")).unwrap();
    let _second = Watcher::on(&root);
    wait_events(&root, "release", &["edited", "added"]);
    wait_events(&root, "old", &["deleted", "added"]);
    assert_eq!(events(&root, "other"), ["added"]);
    assert_eq!(newest(&root, "release"), text);
}

#[test]
fn a_first_start_adds_every_note() {
    let dir = TempDir::new("watch-forty");
    let root = store(&dir);
    let topics: Vec<String> = (0..40).map(|n| format!("n{n:02}")).collect();
    for (n, topic) in topics.iter().enumerate() {
        let id = format!("01M3YJ7R6HK6NQ30DCDB1P4D{n:02}");
        write(
            &root,
            &format!("decision-{topic}.md"),
            &note_text(&id, topic),
        );
    }
    let _watcher = Watcher::on(&root);
    for topic in &topics {
        wait_events(&root, topic, &["added"]);
    }
}

/// Starts a watcher on a store holding `decision-release.md` and `plan-other.md`, once both are recorded.
fn two_recorded(name: &str) -> (TempDir, std::path::PathBuf, Watcher) {
    let dir = TempDir::new(name);
    let root = store(&dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    write(&root, "plan-other.md", &note_text(IDS[1], "Other"));
    let watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other"]);
    (dir, root, watcher)
}

/// Lets the files written so far age past the watcher's racy window and then has a scan cache them, so a later
/// rewrite is judged by the stat key and not by the rule that never trusts a fresh file. The sleep is that aging, not
/// a wait for a recording.
fn age_the_cache(root: &Path) {
    std::thread::sleep(Duration::from_secs(3));
    barrier(root);
}

/// A later change to `plan-other.md`, waited for: a scan that records it has read everything written before it.
fn barrier(root: &Path) {
    let mut text = note_text(IDS[1], "Other");
    text.push_str("\nBarrier.\n");
    write(root, "plan-other.md", &text);
    wait_events(root, "other", &["edited", "added"]);
}

#[test]
fn a_file_without_an_id_is_named_once() {
    let (_dir, root, watcher) = two_recorded("watch-no-id");
    write(&root, "plan-x.md", "# No frontmatter\n");
    let line = "bilbo: notes/plan-x.md: not recorded: no valid id in the frontmatter";
    watcher.wait_for(line);
    barrier(&root);
    assert_eq!(watcher.count(line), 1);
    assert!(events(&root, "x").is_empty());
}

#[test]
fn a_shared_id_is_named_and_not_deleted() {
    let (_dir, root, watcher) = two_recorded("watch-shared");
    let text = note_text(IDS[0], "Release");
    write(&root, "plan-release-copy.md", &text);
    watcher.wait_for(&format!(
        "bilbo: notes/decision-release.md: not recorded: shares id {} with plan-release-copy.md",
        IDS[0]
    ));
    watcher.wait_for(&format!(
        "bilbo: notes/plan-release-copy.md: not recorded: shares id {} with decision-release.md",
        IDS[0]
    ));
    barrier(&root);
    assert_eq!(events(&root, "release"), ["added"]);
    // Recording resumes once one copy is gone and the note changes.
    fs::remove_file(root.join("notes/plan-release-copy.md")).unwrap();
    write(&root, "decision-release.md", &format!("{text}\nAfter.\n"));
    wait_events(&root, "release", &["edited", "added"]);
}

#[test]
fn a_name_that_is_not_a_note_name_is_named() {
    let (_dir, root, watcher) = two_recorded("watch-bad-name");
    write(&root, "Release.md", &note_text(IDS[2], "Other release"));
    let line = "bilbo: notes/Release.md: not recorded: name is not <kind>-<topic>.md";
    watcher.wait_for(line);
    barrier(&root);
    assert_eq!(watcher.count(line), 1);
    assert!(events(&root, "release").len() == 1);
}

#[test]
fn a_hidden_file_is_ignored_in_silence() {
    let (_dir, root, watcher) = two_recorded("watch-hidden");
    write(
        &root,
        ".decision-release.md.swp",
        &note_text(IDS[2], "Swap"),
    );
    barrier(&root);
    assert_eq!(watcher.lines().len(), 1, "{:?}", watcher.lines());
}

#[test]
fn a_huge_file_is_named() {
    let (_dir, root, watcher) = two_recorded("watch-huge");
    let mut text = note_text(IDS[2], "Huge");
    text.push_str(&"x".repeat(5 * 1024 * 1024));
    write(&root, "plan-huge.md", &text);
    let line = "bilbo: notes/plan-huge.md: not recorded: larger than 1 MiB";
    watcher.wait_for(line);
    barrier(&root);
    assert!(events(&root, "huge").is_empty());
}

#[test]
fn notes_are_left_as_the_agent_wrote_them() {
    let dir = TempDir::new("watch-leaves");
    let root = store(&dir);
    let _watcher = Watcher::on(&root);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    write(&root, "plan-other.md", &note_text(IDS[1], "Other"));
    let written = snapshot(&root.join("notes"));
    wait_added(&root, &["release", "other"]);
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nMore.\n");
    write(&root, "decision-release.md", &text);
    let written_again = snapshot(&root.join("notes"));
    wait_events(&root, "release", &["edited", "added"]);
    assert_ne!(written, written_again);
    assert_eq!(snapshot(&root.join("notes")), written_again);
}

#[test]
fn a_restore_leftover_is_swept() {
    let (_dir, root, watcher) = two_recorded("watch-leftover");
    let leftover = root.join(format!("notes/.bilbo-restore-{}", IDS[0]));
    fs::write(&leftover, "bytes no version holds\n").unwrap();
    poll_eq("the leftover", || leftover.exists(), false);
    // The sweep records the leftover's bytes, then the scan records the file, which an interrupted restore left holding
    // the note's own text.
    wait_events(&root, "release", &["edited", "edited", "added"]);
    let list = bilbo(&root, &home(&root), &["history", "release"]);
    let swept = list
        .stdout
        .lines()
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let run = bilbo(&root, &home(&root), &["history", "release", swept]);
    assert_eq!(run.stdout, "bytes no version holds\n");
    watcher.wait_for(&format!(
        "bilbo: recorded notes/.bilbo-restore-{} from an interrupted restore",
        IDS[0]
    ));
}

#[test]
fn a_leftover_without_history_is_named_once() {
    let (_dir, root, watcher) = two_recorded("watch-leftover-orphan");
    let leftover = root.join(format!("notes/.bilbo-restore-{}", IDS[2]));
    fs::write(&leftover, "bytes\n").unwrap();
    let line = format!(
        "bilbo: notes/.bilbo-restore-{} left in place: no history for its note",
        IDS[2]
    );
    watcher.wait_for(&line);
    barrier(&root);
    assert_eq!(watcher.count(&line), 1);
    assert!(leftover.exists());
}

/// A store with `decision-release.md` holding `text`, whose history is `ages` days old, one version each, the last
/// one matching the file.
fn aged(dir: &TempDir, ages: &[i64]) -> std::path::PathBuf {
    let root = store(dir);
    let texts: Vec<String> = (0..ages.len())
        .map(|n| format!("{}\nVersion {n}.\n", note_text(IDS[0], "Release")))
        .collect();
    write(&root, "decision-release.md", texts.last().unwrap());
    let seeds: Vec<Seed> = ages
        .iter()
        .zip(&texts)
        .map(|(age, text)| Seed::new("decision-release.md", Some(text), "edited", &days_ago(*age)))
        .collect();
    common::seed(&root, IDS[0], &seeds);
    root
}

#[test]
fn old_versions_are_pruned_at_start() {
    let dir = TempDir::new("watch-prune");
    let root = aged(&dir, &[200, 100, 10]);
    let watcher = Watcher::on(&root);
    assert_eq!(watcher.count("bilbo: pruned 1 versions older than "), 1);
    assert_eq!(events(&root, "release").len(), 2);
}

#[test]
fn nothing_to_prune_prints_nothing() {
    let dir = TempDir::new("watch-no-prune");
    let root = aged(&dir, &[10, 5]);
    let watcher = Watcher::on(&root);
    assert_eq!(watcher.count("pruned"), 0);
    assert_eq!(events(&root, "release").len(), 2);
}

#[test]
fn keep_days_sets_the_window() {
    let dir = TempDir::new("watch-keep-days");
    let root = aged(&dir, &[100, 40, 10]);
    let file = config(&dir, &["history.keep_days = 30"]);
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", file.to_str().unwrap()),
    ];
    let watcher = Watcher::start(&env);
    watcher.wait_for("bilbo: pruned 1 versions older than ");
    watcher.wait_for("bilbo: watching ");
    assert_eq!(events(&root, "release").len(), 2);
}

#[test]
fn the_default_window_is_90_days() {
    let dir = TempDir::new("watch-default-days");
    let root = aged(&dir, &[100, 40, 10]);
    let watcher = Watcher::on(&root);
    assert_eq!(watcher.count("pruned"), 0);
    assert_eq!(events(&root, "release").len(), 3);
}

#[test]
fn an_explicit_config_that_does_not_exist_is_an_error() {
    let dir = TempDir::new("watch-no-config");
    let root = store(&dir);
    let missing = dir.path().join("nope");
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", missing.to_str().unwrap()),
        ],
        &["watch"],
    );
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains(missing.to_str().unwrap()),
        "{}",
        run.stderr
    );
}

#[test]
fn a_bad_keep_days_is_an_error() {
    for value in ["0", "3651", "2w"] {
        let dir = TempDir::new("watch-bad-days");
        let root = store(&dir);
        let file = config(&dir, &[&format!("history.keep_days = {value}")]);
        let run = bilbo(
            dir.path(),
            &[
                ("BILBO_HOME", root.to_str().unwrap()),
                ("BILBO_CONFIG", file.to_str().unwrap()),
            ],
            &["watch"],
        );
        assert_eq!(run.code, 2, "{value}");
        assert!(run.stderr.contains("history.keep_days"), "{}", run.stderr);
    }
}

#[test]
fn a_lost_lock_file_stops_the_watcher() {
    let (_dir, root, mut watcher) = two_recorded("watch-lock-gone");
    fs::remove_dir_all(root.join(".bilbo")).unwrap();
    poll_eq("the watcher to stop", || watcher.running(), false);
    assert!(watcher.count("watch.lock is gone") >= 1);
}

#[test]
fn a_symlinked_note_is_not_deleted() {
    let (dir, root) = three_store("watch-symlink");
    let _watcher = Watcher::on(&root);
    wait_added(&root, &["release", "other"]);
    let elsewhere = dir.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let target = elsewhere.join("decision-release.md");
    fs::rename(root.join("notes/decision-release.md"), &target).unwrap();
    std::os::unix::fs::symlink(&target, root.join("notes/decision-release.md")).unwrap();
    barrier(&root);
    assert_eq!(events(&root, "release"), ["added"]);
}

/// The folder goes away inside the quiet window of a pending event, as the 10-second check runs: the watch is dropped
/// with a scan still due, and a loop that cannot clear it spins.
#[test]
fn a_watcher_idles_while_the_folder_is_away() {
    let (_dir, root) = three_store("watch-idle");
    let watcher = Watcher::on(&root);
    let started = std::time::Instant::now();
    wait_added(&root, &["release", "other", "old"]);
    std::thread::sleep(Duration::from_millis(9300).saturating_sub(started.elapsed()));
    let mut text = note_text(IDS[0], "Release");
    text.push_str("\nA change, so an event is pending.\n");
    write(&root, "decision-release.md", &text);
    fs::rename(root.join("notes"), root.join("notes.bak")).unwrap();
    watcher.wait_for("bilbo: cannot read ");
    std::thread::sleep(Duration::from_secs(3));
    let Some(before) = common::cpu_seconds(watcher.pid()) else {
        eprintln!("no way to read a process's CPU time here; skipped");
        return;
    };
    std::thread::sleep(Duration::from_secs(8));
    let used = common::cpu_seconds(watcher.pid()).unwrap() - before;
    assert!(used < 1.0, "{used} s of CPU in 8 s with notes/ away");
}
