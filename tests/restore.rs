//! `bilbo restore` through the built binary. Histories come from `common::seed`, since a real watcher cannot be made
//! to record a rename or a deletion at a moment the scenario needs; the scenarios about a running watcher use one.
//! The write that lands during a restore, and each crash point, are unit tests of `note::restore`, which can stop
//! the sequence between its steps.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    IDS, Seed, TempDir, Watcher, bilbo, days_ago, note_text, poll_eq, seed, snapshot, store, write,
};

fn home(root: &Path) -> [(&str, &str); 1] {
    [("BILBO_HOME", root.to_str().unwrap())]
}

fn restore(root: &Path, args: &[&str]) -> common::Run {
    let mut all = vec!["restore"];
    all.extend(args);
    bilbo(&std::env::temp_dir(), &home(root), &all)
}

fn history(root: &Path, args: &[&str]) -> common::Run {
    let mut all = vec!["history"];
    all.extend(args);
    bilbo(&std::env::temp_dir(), &home(root), &all)
}

/// `event file` of each version, newest first.
fn events(root: &Path, note: &str) -> Vec<String> {
    let run = history(root, &[note]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    run.stdout
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split(' ').collect();
            format!("{} {}", parts[2], parts[3])
        })
        .collect()
}

fn shorts(root: &Path, note: &str) -> Vec<String> {
    history(root, &[note])
        .stdout
        .lines()
        .map(|line| line.split(' ').next().unwrap().to_string())
        .collect()
}

/// The notes and the logs, which a refused restore leaves as they were; taking the history lock may touch the rest.
fn state(root: &Path) -> String {
    let logs = root.join(".bilbo/history/notes");
    let logs = if logs.exists() {
        format!("{:?}", snapshot(&logs))
    } else {
        String::new()
    };
    format!("{:?}{logs}", snapshot(&root.join("notes")))
}

/// Starts a watcher on a store that also holds `plan-other.md`, and waits until it is recorded.
fn watch_with_other(root: &Path) -> Watcher {
    write(root, "plan-other.md", &note_text(IDS[1], "Other"));
    let watcher = Watcher::on(root);
    wait_other(root, &["added plan-other.md"]);
    watcher
}

fn wait_other(root: &Path, want: &[&str]) {
    let want: Vec<String> = want.iter().map(|e| e.to_string()).collect();
    poll_eq(
        "events of other",
        || {
            let run = history(root, &["other"]);
            run.stdout
                .lines()
                .map(|line| {
                    let parts: Vec<&str> = line.split(' ').collect();
                    format!("{} {}", parts[2], parts[3])
                })
                .collect::<Vec<String>>()
        },
        want,
    );
}

/// A later change to `plan-other.md`, waited for: a scan that records it has read everything written before it, so
/// a check made after it cannot pass because the watcher had not looked yet.
fn barrier(root: &Path) {
    write(
        root,
        "plan-other.md",
        &format!("{}\nBarrier.\n", note_text(IDS[1], "Other")),
    );
    wait_other(root, &["edited plan-other.md", "added plan-other.md"]);
}

fn text(body: &str) -> String {
    format!("{}\n{body}\n", note_text(IDS[0], "Release"))
}

fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join("notes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn file(root: &Path, name: &str) -> String {
    fs::read_to_string(root.join("notes").join(name)).unwrap()
}

/// `decision-release.md` added with "one" and edited to "two", which the file holds; returns the store root.
fn edited(dir: &TempDir) -> PathBuf {
    let root = store(dir);
    write(&root, "decision-release.md", &text("two"));
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text("one")),
                "added",
                &days_ago(2),
            ),
            Seed::new(
                "decision-release.md",
                Some(&text("two")),
                "edited",
                &days_ago(1),
            ),
        ],
    );
    root
}

/// The same, renamed to `plan-release.md` after the first version.
fn renamed(dir: &TempDir) -> PathBuf {
    let root = store(dir);
    write(&root, "plan-release.md", &text("one"));
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text("one")),
                "added",
                &days_ago(2),
            ),
            Seed::new(
                "plan-release.md",
                Some(&text("one")),
                "renamed",
                &days_ago(1),
            ),
        ],
    );
    root
}

#[test]
fn undoes_an_edit() {
    let dir = TempDir::new("restore-undo");
    let root = edited(&dir);
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("restored decision-release.md to {first}\n")
    );
    assert!(run.stderr.is_empty());
    assert_eq!(file(&root, "decision-release.md"), text("one"));
    assert_eq!(names(&root), ["decision-release.md"]);
    assert_eq!(
        events(&root, "release"),
        [
            "restored decision-release.md",
            "edited decision-release.md",
            "added decision-release.md"
        ]
    );
}

#[test]
fn prints_twelve_characters_whatever_prefix_was_typed() {
    let dir = TempDir::new("restore-prefix");
    let root = edited(&dir);
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first[..6]]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("restored decision-release.md to {first}\n")
    );
}

#[test]
fn names_the_note_by_id() {
    let dir = TempDir::new("restore-id");
    let root = edited(&dir);
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &[IDS[0], &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(file(&root, "decision-release.md"), text("one"));
}

#[test]
fn brings_back_a_deleted_note() {
    let dir = TempDir::new("restore-deleted");
    let root = store(&dir);
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text("one")),
                "added",
                &days_ago(3),
            ),
            Seed::new(
                "decision-release.md",
                Some(&text("two")),
                "edited",
                &days_ago(2),
            ),
            Seed::new("decision-release.md", None, "deleted", &days_ago(1)),
        ],
    );
    let edit = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &edit]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(file(&root, "decision-release.md"), text("two"));
    assert_eq!(names(&root), ["decision-release.md"]);
    assert_eq!(events(&root, "release")[0], "restored decision-release.md");
}

#[test]
fn a_missing_argument_is_a_usage_error() {
    let dir = TempDir::new("restore-usage");
    let root = edited(&dir);
    let before = state(&root);
    for args in [
        &["release"][..],
        &[],
        &["release", "abcdef", "extra"],
        &["--now", "x"],
    ] {
        let run = restore(&root, args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr.contains("bilbo restore <note> <version>"),
            "{args:?}"
        );
    }
    assert_eq!(state(&root), before);
}

#[test]
fn a_bad_version_is_a_usage_error_and_an_unknown_one_a_refusal() {
    let dir = TempDir::new("restore-version");
    let root = edited(&dir);
    let before = state(&root);
    let run = restore(&root, &["release", "xyz"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.starts_with("bilbo: 'xyz' is not a version"));
    let run = restore(&root, &["release", "abcdef"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr, "bilbo: no version abcdef of release\n");
    assert_eq!(state(&root), before);
}

#[test]
fn an_unrecorded_edit_is_kept() {
    let dir = TempDir::new("restore-unrecorded");
    let root = edited(&dir);
    write(&root, "decision-release.md", &text("three"));
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        events(&root, "release"),
        [
            "restored decision-release.md",
            "edited decision-release.md",
            "edited decision-release.md",
            "added decision-release.md"
        ]
    );
    let kept = &shorts(&root, "release")[1];
    assert_eq!(history(&root, &["release", kept]).stdout, text("three"));
}

#[test]
fn a_file_the_watcher_has_not_seen_is_the_current_one() {
    let dir = TempDir::new("restore-unseen");
    let root = store(&dir);
    write(&root, "plan-release.md", &text("three"));
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text("one")),
                "added",
                &days_ago(2),
            ),
            Seed::new("decision-release.md", None, "deleted", &days_ago(1)),
        ],
    );
    let first = shorts(&root, IDS[0])[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(names(&root), ["decision-release.md"]);
    assert_eq!(file(&root, "decision-release.md"), text("one"));
    assert_eq!(
        events(&root, IDS[0]),
        [
            "restored decision-release.md",
            "edited plan-release.md",
            "deleted decision-release.md",
            "added decision-release.md"
        ]
    );
}

fn leftover(root: &Path, bytes: &str) -> PathBuf {
    let path = root
        .join("notes")
        .join(format!(".bilbo-restore-{}", IDS[0]));
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_leftover_is_recorded_and_removed() {
    let dir = TempDir::new("restore-leftover-new");
    let root = edited(&dir);
    let path = leftover(&root, &text("lost"));
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: recorded notes/.bilbo-restore-{} from an interrupted restore\n",
            IDS[0]
        )
    );
    assert!(!path.exists());
    let list = shorts(&root, "release");
    let found = list
        .iter()
        .position(|v| history(&root, &["release", v]).stdout == text("lost"))
        .expect("the leftover's bytes are a version");
    assert!(found > 0, "restored is newer than the leftover");
    assert!(events(&root, "release")[found].starts_with("edited "));
}

#[test]
fn a_leftover_already_in_history_goes_quietly() {
    let dir = TempDir::new("restore-leftover-known");
    let root = edited(&dir);
    let path = leftover(&root, &text("one"));
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert!(!path.exists());
    assert_eq!(events(&root, "release").len(), 3);
}

#[test]
fn a_kill_between_the_swap_and_the_rename_needs_no_human() {
    let dir = TempDir::new("restore-killed");
    let root = store(&dir);
    // What the exchange leaves: the restored bytes under the old name, what came out under the hidden name.
    write(&root, "plan-release.md", &text("one"));
    leftover(&root, &text("two"));
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text("one")),
                "added",
                &days_ago(2),
            ),
            Seed::new(
                "plan-release.md",
                Some(&text("two")),
                "renamed",
                &days_ago(1),
            ),
        ],
    );
    let watcher = Watcher::on(&root);
    poll_eq(
        "the hidden file",
        || names(&root),
        vec!["plan-release.md".to_string()],
    );
    poll_eq(
        "the file recorded",
        || events(&root, "release")[0].clone(),
        "edited plan-release.md".to_string(),
    );
    assert_eq!(watcher.count("shares id"), 0, "{:?}", watcher.lines());
    assert_eq!(file(&root, "plan-release.md"), text("one"));
}

#[test]
fn goes_back_to_the_old_name() {
    let dir = TempDir::new("restore-old-name");
    let root = renamed(&dir);
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("restored decision-release.md to {first}\n")
    );
    assert_eq!(names(&root), ["decision-release.md"]);
    assert_eq!(file(&root, "decision-release.md"), text("one"));
}

#[test]
fn an_old_name_that_is_taken_changes_nothing() {
    let dir = TempDir::new("restore-taken");
    let root = renamed(&dir);
    write(&root, "decision-release.md", &note_text(IDS[1], "Other"));
    let before = state(&root);
    let first = shorts(&root, IDS[0])[1].clone();
    let run = restore(&root, &[IDS[0], &first]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        "bilbo: decision-release.md is taken by another note\n"
    );
    assert_eq!(state(&root), before);
}

#[test]
fn a_topic_taken_under_another_kind_changes_nothing() {
    let dir = TempDir::new("restore-taken-topic");
    let root = renamed(&dir);
    write(&root, "report-release.md", &note_text(IDS[1], "Other"));
    let before = state(&root);
    let first = shorts(&root, IDS[0])[1].clone();
    let run = restore(&root, &[IDS[0], &first]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        "bilbo: decision-release.md is taken by another note\n"
    );
    assert_eq!(state(&root), before);
}

#[test]
fn a_deletion_cannot_be_restored() {
    let dir = TempDir::new("restore-deletion");
    let root = store(&dir);
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text("one")),
                "added",
                &days_ago(2),
            ),
            Seed::new("decision-release.md", None, "deleted", &days_ago(1)),
        ],
    );
    let before = state(&root);
    let gone = shorts(&root, IDS[0])[0].clone();
    let run = restore(&root, &[IDS[0], &gone]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: version {gone} of {} is a deletion; delete the file instead\n",
            IDS[0]
        )
    );
    assert_eq!(state(&root), before);
}

#[test]
fn a_file_that_already_matches_is_left_alone() {
    let dir = TempDir::new("restore-matches");
    let root = edited(&dir);
    let before = state(&root);
    let second = shorts(&root, "release")[0].clone();
    let run = restore(&root, &["release", &second]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("decision-release.md already matches {second}\n")
    );
    assert!(run.stderr.is_empty());
    assert_eq!(state(&root), before);
}

#[test]
fn a_note_held_only_by_a_skipped_file_is_refused() {
    let dir = TempDir::new("restore-skipped");
    let root = edited(&dir);
    let big = format!("{}{}\n", text("big"), "x".repeat(1024 * 1024));
    write(&root, "decision-release.md", &big);
    let before = state(&root);
    let first = shorts(&root, IDS[0])[1].clone();
    let run = restore(&root, &[IDS[0], &first]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains("held by notes/decision-release.md"),
        "{}",
        run.stderr
    );
    assert_eq!(state(&root), before);
}

#[test]
fn works_without_a_watcher() {
    let dir = TempDir::new("restore-no-watcher");
    let root = edited(&dir);
    let first = shorts(&root, "release")[1].clone();
    assert_eq!(restore(&root, &["release", &first]).code, 0);
    assert_eq!(events(&root, "release")[0], "restored decision-release.md");
}

#[test]
fn a_running_watcher_does_not_record_it_again() {
    let dir = TempDir::new("restore-watcher");
    let root = edited(&dir);
    let watcher = watch_with_other(&root);
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    poll_eq(
        "the restored version",
        || events(&root, "release")[0].clone(),
        "restored decision-release.md".to_string(),
    );
    barrier(&root);
    assert_eq!(
        events(&root, "release"),
        [
            "restored decision-release.md",
            "edited decision-release.md",
            "added decision-release.md"
        ]
    );
    assert_eq!(watcher.count("shares id"), 0);
}

#[test]
fn a_restore_races_an_unrecorded_edit_with_a_watcher_running() {
    let dir = TempDir::new("restore-watcher-edit");
    let root = edited(&dir);
    let _watcher = watch_with_other(&root);
    write(&root, "decision-release.md", &text("three"));
    let first = shorts(&root, "release")[1].clone();
    let run = restore(&root, &["release", &first]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    barrier(&root);
    let list = events(&root, "release");
    assert_eq!(list[0], "restored decision-release.md");
    assert_eq!(list.iter().filter(|e| e.starts_with("restored")).count(), 1);
    let versions = shorts(&root, "release");
    assert!(
        versions
            .iter()
            .any(|v| history(&root, &["release", v]).stdout == text("three")),
        "the edit is in history"
    );
    assert_eq!(file(&root, "decision-release.md"), text("one"));
}

#[test]
fn a_missing_store_creates_nothing() {
    let dir = TempDir::new("restore-no-store");
    let home_dir = dir.path().join("home");
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", home_dir.to_str().unwrap())],
        &["restore", "release", "a1b2c3"],
    );
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", home_dir.display())
    );
    assert!(!home_dir.exists());
}
