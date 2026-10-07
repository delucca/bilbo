//! `bilbo history` through the built binary. Histories come from a running `bilbo watch` where the recording is what
//! the scenario is about, and from `common::seed` where it needs ids, ages or content a watcher cannot be made to
//! produce.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    IDS, Seed, TempDir, Watcher, bilbo, bilbo_bytes, days_ago, note_text, poll_eq, seed, snapshot,
    store, write,
};

fn home(root: &Path) -> [(&str, &str); 1] {
    [("BILBO_HOME", root.to_str().unwrap())]
}

fn history(root: &Path, args: &[&str]) -> common::Run {
    let mut all = vec!["history"];
    all.extend(args);
    bilbo(&std::env::temp_dir(), &home(root), &all)
}

fn events(root: &Path, topic: &str) -> Vec<String> {
    let run = history(root, &[topic]);
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

/// The 12-character ids of a note's versions, newest first.
fn shorts(root: &Path, topic: &str) -> Vec<String> {
    history(root, &[topic])
        .stdout
        .lines()
        .map(|line| line.split(' ').next().unwrap().to_string())
        .collect()
}

const WARNING: &str = "bilbo: bilbo watch is not running; recent edits may not be recorded\n";

fn text(n: usize) -> String {
    format!("{}\nVersion {n}.\n", note_text(IDS[0], "Release"))
}

/// A store whose `decision-release.md` was added and then had a paragraph appended, recorded by a real watcher.
fn recorded(dir: &TempDir) -> (PathBuf, String, String) {
    let root = store(dir);
    let first = note_text(IDS[0], "Release");
    write(&root, "decision-release.md", &first);
    let watcher = Watcher::on(&root);
    wait_events(&root, "release", &["added"]);
    let second = format!("{first}\nA paragraph.\n");
    write(&root, "decision-release.md", &second);
    wait_events(&root, "release", &["edited", "added"]);
    drop(watcher);
    (root, first, second)
}

#[test]
fn history_sits_beside_the_notes() {
    let dir = TempDir::new("history-beside");
    let root = store(&dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    let before = snapshot(&root.join("notes"));
    let _watcher = Watcher::on(&root);
    wait_events(&root, "release", &["added"]);
    let log = root.join(format!(".bilbo/history/notes/{}.jsonl", IDS[0]));
    assert!(
        fs::read_to_string(log)
            .unwrap()
            .contains("decision-release.md")
    );
    assert_eq!(snapshot(&root.join("notes")), before);
}

#[test]
fn losing_the_history_folder_starts_over() {
    let dir = TempDir::new("history-lost");
    let (root, _, second) = recorded(&dir);
    fs::remove_dir_all(root.join(".bilbo/history")).unwrap();
    let notes = snapshot(&root.join("notes"));
    let _watcher = Watcher::on(&root);
    wait_events(&root, "release", &["added"]);
    assert_eq!(snapshot(&root.join("notes")), notes);
    let version = &shorts(&root, "release")[0];
    assert_eq!(history(&root, &["release", version]).stdout, second);
}

#[test]
fn old_edits_are_dropped() {
    let dir = TempDir::new("history-retention-old");
    let root = store(&dir);
    let texts: Vec<String> = (0..4).map(text).collect();
    write(&root, "decision-release.md", &texts[3]);
    let ages = [200, 150, 100, 10];
    let seeds: Vec<Seed> = ages
        .iter()
        .zip(&texts)
        .map(|(age, t)| Seed::new("decision-release.md", Some(t), "edited", &days_ago(*age)))
        .collect();
    seed(&root, IDS[0], &seeds);
    let watcher = Watcher::on(&root);
    assert_eq!(watcher.count("bilbo: pruned 2 versions older than "), 1);
    assert_eq!(events(&root, "release").len(), 2);
    assert_eq!(
        history(&root, &["release", &shorts(&root, "release")[1]]).stdout,
        texts[2]
    );
    let blobs = fs::read_dir(root.join(".bilbo/history/blobs"))
        .unwrap()
        .map(|d| fs::read_dir(d.unwrap().path()).unwrap().count())
        .sum::<usize>();
    assert_eq!(blobs, 2);
}

#[test]
fn a_note_untouched_for_a_year_keeps_its_version() {
    let dir = TempDir::new("history-retention-year");
    let root = store(&dir);
    write(&root, "decision-release.md", &text(0));
    seed(
        &root,
        IDS[0],
        &[Seed::new(
            "decision-release.md",
            Some(&text(0)),
            "added",
            &days_ago(400),
        )],
    );
    let _watcher = Watcher::on(&root);
    assert_eq!(events(&root, "release"), ["added"]);
}

#[test]
fn a_deleted_note_keeps_its_last_text() {
    let dir = TempDir::new("history-retention-deleted");
    let root = store(&dir);
    let kept = text(1);
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text(0)),
                "added",
                &days_ago(400),
            ),
            Seed::new("decision-release.md", Some(&kept), "edited", &days_ago(300)),
            Seed::new("decision-release.md", None, "deleted", &days_ago(200)),
        ],
    );
    write(&root, "plan-other.md", &note_text(IDS[1], "Other"));
    let _watcher = Watcher::on(&root);
    wait_events(&root, "other", &["added"]);
    let list = history(&root, &[IDS[0]]);
    let lines: Vec<&str> = list.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains(" deleted ") && lines[1].contains(" edited "));
    let edit = lines[1].split(' ').next().unwrap();
    assert_eq!(history(&root, &[IDS[0], edit]).stdout, kept);
}

#[test]
fn an_unreadable_record_stops_content_removal() {
    let dir = TempDir::new("history-retention-garbled");
    let root = store(&dir);
    write(&root, "decision-release.md", &text(2));
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some(&text(0)),
                "edited",
                &days_ago(300),
            ),
            Seed::new(
                "decision-release.md",
                Some(&text(1)),
                "edited",
                &days_ago(250),
            ),
            Seed::new(
                "decision-release.md",
                Some(&text(2)),
                "edited",
                &days_ago(10),
            ),
        ],
    );
    let log = root.join(format!(".bilbo/history/notes/{}.jsonl", IDS[0]));
    let mut lines: Vec<String> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    lines[1] = "{garbled".into();
    fs::write(&log, lines.join("\n") + "\n").unwrap();
    write(&root, "plan-other.md", &note_text(IDS[1], "Other"));
    seed(
        &root,
        IDS[1],
        &[Seed::new(
            "plan-other.md",
            Some(&note_text(IDS[1], "Other")),
            "added",
            &days_ago(200),
        )],
    );
    let before = snapshot(&root.join(".bilbo/history/blobs"));
    let watcher = Watcher::on(&root);
    watcher.wait_for(&format!(
        "bilbo: history/notes/{}.jsonl line 2 is unreadable; no content removed",
        IDS[0]
    ));
    let after = snapshot(&root.join(".bilbo/history/blobs"));
    for path in before.keys().filter(|p| p.is_file()) {
        assert!(after.contains_key(path), "{} was removed", path.display());
    }
}

#[test]
fn a_note_is_named_by_topic() {
    let dir = TempDir::new("history-topic");
    let (root, _, _) = recorded(&dir);
    let run = history(&root, &["release"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.lines().count(), 2);
}

#[test]
fn a_deleted_note_is_named_by_topic() {
    let dir = TempDir::new("history-deleted-topic");
    let (root, _, _) = recorded(&dir);
    write(&root, "plan-other.md", &note_text(IDS[1], "Other"));
    {
        let _watcher = Watcher::on(&root);
        fs::remove_file(root.join("notes/decision-release.md")).unwrap();
        poll_eq(
            "the deletion",
            || history(&root, &["release"]).stdout.lines().count(),
            3,
        );
    }
    let run = history(&root, &["release"]);
    assert!(
        run.stdout
            .lines()
            .next()
            .unwrap()
            .contains(" deleted decision-release.md")
    );
}

#[test]
fn a_note_is_named_by_id() {
    let dir = TempDir::new("history-id");
    let (root, _, _) = recorded(&dir);
    fs::rename(
        root.join("notes/decision-release.md"),
        root.join("notes/plan-moved.md"),
    )
    .unwrap();
    let run = history(&root, &[IDS[0]]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.lines().count(), 2);
}

#[test]
fn an_unknown_note_has_no_history() {
    let dir = TempDir::new("history-unknown");
    let root = store(&dir);
    let run = history(&root, &["wumpus"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr, "bilbo: no history for wumpus\n");
}

#[test]
fn one_topic_two_notes_is_a_usage_error() {
    let dir = TempDir::new("history-two-notes");
    let root = store(&dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    write(&root, "plan-release.md", &note_text(IDS[1], "Release"));
    let run = history(&root, &["release"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    for needle in [
        &format!("decision-release.md ({})", IDS[0]),
        &format!("plan-release.md ({})", IDS[1]),
    ] {
        assert!(run.stderr.contains(needle.as_str()), "{}", run.stderr);
    }
}

#[test]
fn a_name_that_is_not_a_topic_is_a_usage_error() {
    let dir = TempDir::new("history-not-topic");
    let root = store(&dir);
    let run = history(&root, &["Release_Notes"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with("bilbo: 'Release_Notes' is neither"));
}

#[test]
fn a_short_history_is_listed_newest_first() {
    let dir = TempDir::new("history-list");
    let (root, _, _) = recorded(&dir);
    let run = history(&root, &["release"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stderr, WARNING);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].ends_with(" edited decision-release.md"),
        "{lines:?}"
    );
    assert!(
        lines[1].ends_with(" added decision-release.md"),
        "{lines:?}"
    );
    for line in &lines {
        let mut parts = line.split(' ');
        let id = parts.next().unwrap();
        assert!(id.len() == 12 && id.bytes().all(|b| b.is_ascii_hexdigit()));
        let time = parts.next().unwrap();
        assert_eq!(time.len(), 22, "{time}");
        assert_eq!(&time[10..11], "T");
    }
}

#[test]
fn the_listing_keeps_the_recorded_offset_in_another_time_zone() {
    let dir = TempDir::new("history-tz");
    let root = store(&dir);
    seed(
        &root,
        IDS[0],
        &[
            Seed::new(
                "decision-release.md",
                Some("one\n"),
                "added",
                "2026-10-03T14:23:05-03:00",
            ),
            Seed::new(
                "decision-release.md",
                Some("two\n"),
                "edited",
                "2026-10-03T23:59:30+09:00",
            ),
        ],
    );
    for zone in ["Asia/Tokyo", "America/Sao_Paulo", "UTC"] {
        let run = bilbo(
            dir.path(),
            &[("BILBO_HOME", root.to_str().unwrap()), ("TZ", zone)],
            &["history", IDS[0]],
        );
        let lines: Vec<&str> = run.stdout.lines().collect();
        assert!(
            lines[0].contains(" 2026-10-03T23:59+09:00 edited ")
                && lines[1].contains(" 2026-10-03T14:23-03:00 added "),
            "{zone}: {lines:?}"
        );
    }
}

#[test]
fn a_prefix_names_a_version() {
    let dir = TempDir::new("history-prefix");
    let (root, first, _) = recorded(&dir);
    let oldest = shorts(&root, "release").pop().unwrap();
    let run = history(&root, &["release", &oldest[..6]]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, first);
}

#[test]
fn a_prefix_that_is_too_short_is_a_usage_error() {
    let dir = TempDir::new("history-short-prefix");
    let (root, _, _) = recorded(&dir);
    let run = history(&root, &["release", "a1b"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
}

#[test]
fn a_version_that_does_not_exist_is_refused() {
    let dir = TempDir::new("history-no-version");
    let (root, _, _) = recorded(&dir);
    let run = history(&root, &["release", "ffffff"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr, "bilbo: no version ffffff of release\n");
}

#[test]
fn an_ambiguous_prefix_lists_the_matches() {
    let dir = TempDir::new("history-ambiguous");
    let root = store(&dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    let (a, b) = (
        "abcdef".to_string() + &"0".repeat(58),
        "abcdef".to_string() + &"1".repeat(58),
    );
    seed(
        &root,
        IDS[0],
        &[
            Seed::new("decision-release.md", Some("one\n"), "added", &days_ago(2)).id(&a),
            Seed::new("decision-release.md", Some("two\n"), "edited", &days_ago(1)).id(&b),
        ],
    );
    let run = history(&root, &["release", "abcdef"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains(&a[..12]) && run.stderr.contains(&b[..12]),
        "{}",
        run.stderr
    );
    assert_eq!(history(&root, &["release", &a[..7]]).code, 0);
    assert_eq!(history(&root, &[IDS[0], &b]).stdout, "two\n");
}

#[test]
fn a_version_prints_byte_for_byte() {
    let dir = TempDir::new("history-exact");
    let root = store(&dir);
    let bare = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02T14:23-03:00\n---\n\n# No newline at the end";
    write(&root, "decision-release.md", bare);
    let _watcher = Watcher::on(&root);
    wait_events(&root, "release", &["added"]);
    let version = &shorts(&root, "release")[0];
    let (code, stdout, stderr) = bilbo_bytes(&root, &home(&root), &["history", "release", version]);
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_eq!(stdout, bare.as_bytes());
}

#[test]
fn a_deleted_version_has_no_text() {
    let dir = TempDir::new("history-deletion");
    let root = store(&dir);
    let ids = seed(
        &root,
        IDS[0],
        &[
            Seed::new("decision-release.md", Some("one\n"), "added", &days_ago(2)),
            Seed::new("decision-release.md", None, "deleted", &days_ago(1)),
        ],
    );
    let run = history(&root, &["release", &ids[1][..12]]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.starts_with(&format!(
            "bilbo: version {} of release is a deletion\n",
            &ids[1][..12]
        )),
        "{}",
        run.stderr
    );
}

#[test]
fn content_pruned_during_the_read_is_refused() {
    let dir = TempDir::new("history-pruned");
    let root = store(&dir);
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    let ids = seed(
        &root,
        IDS[0],
        &[Seed::new(
            "decision-release.md",
            Some("one\n"),
            "added",
            &days_ago(2),
        )],
    );
    let blob = common::sha256_hex(b"one\n");
    fs::remove_file(root.join(format!(
        ".bilbo/history/blobs/{}/{}",
        &blob[..2],
        &blob[2..]
    )))
    .unwrap();
    let run = history(&root, &["release", &ids[0][..12]]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with(&format!(
        "bilbo: version {} of release was pruned\n",
        &ids[0][..12]
    )));
}

#[test]
fn a_diff_shows_what_an_edit_changed() {
    let dir = TempDir::new("history-diff");
    let (root, _, _) = recorded(&dir);
    let versions = shorts(&root, "release");
    let run = history(&root, &["release", "--diff", &versions[1], &versions[0]]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.starts_with(&format!(
        "--- decision-release.md@{}\n+++ decision-release.md@{}\n@@ ",
        versions[1], versions[0]
    )));
    assert!(run.stdout.contains("\n+A paragraph.\n"), "{}", run.stdout);
}

#[test]
fn a_diff_against_the_file_on_disk_shows_an_unrecorded_edit() {
    let dir = TempDir::new("history-diff-now");
    let (root, _, second) = recorded(&dir);
    write(
        &root,
        "decision-release.md",
        &format!("{second}Unrecorded.\n"),
    );
    let latest = &shorts(&root, "release")[0];
    let run = history(&root, &["release", "--diff", latest]);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.contains("+++ decision-release.md@now\n"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("\n+Unrecorded.\n"));
}

#[test]
fn a_diff_without_a_difference_prints_nothing() {
    let dir = TempDir::new("history-diff-same");
    let (root, _, _) = recorded(&dir);
    let latest = &shorts(&root, "release")[0];
    let run = history(&root, &["release", "--diff", latest]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
}

#[test]
fn a_diff_against_now_after_a_rename_names_the_new_file() {
    let dir = TempDir::new("history-diff-rename");
    let (root, _, _) = recorded(&dir);
    let before = shorts(&root, "release").pop().unwrap();
    {
        let _watcher = Watcher::on(&root);
        fs::rename(
            root.join("notes/decision-release.md"),
            root.join("notes/plan-release.md"),
        )
        .unwrap();
        wait_events(&root, "release", &["renamed", "edited", "added"]);
    }
    let run = history(&root, &["release", "--diff", &before]);
    assert_eq!(run.code, 0);
    assert!(
        run.stdout.starts_with(&format!(
            "--- decision-release.md@{before}\n+++ plan-release.md@now\n"
        )),
        "{}",
        run.stdout
    );
}

#[test]
fn a_deleted_note_cannot_be_diffed_against_now() {
    let dir = TempDir::new("history-diff-deleted");
    let root = store(&dir);
    let ids = seed(
        &root,
        IDS[0],
        &[
            Seed::new("decision-release.md", Some("one\n"), "added", &days_ago(2)),
            Seed::new("decision-release.md", None, "deleted", &days_ago(1)),
        ],
    );
    let run = history(&root, &["release", "--diff", &ids[0][..12]]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with("bilbo: release has no file; name two versions\n"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_deletion_is_diffed_as_empty_text() {
    let dir = TempDir::new("history-diff-empty");
    let root = store(&dir);
    let ids = seed(
        &root,
        IDS[0],
        &[
            Seed::new("decision-release.md", Some("one\n"), "added", &days_ago(2)),
            Seed::new("decision-release.md", None, "deleted", &days_ago(1)),
        ],
    );
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    let (a, b) = (&ids[0][..12], &ids[1][..12]);
    let run = history(&root, &["release", "--diff", a, b]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\n-one\n"), "{}", run.stdout);
}

#[test]
fn a_skipped_file_that_holds_the_note_is_named_in_a_diff_against_now() {
    let dir = TempDir::new("history-diff-skipped");
    let root = store(&dir);
    let ids = seed(
        &root,
        IDS[0],
        &[Seed::new(
            "decision-release.md",
            Some("one\n"),
            "added",
            &days_ago(2),
        )],
    );
    let mut big = note_text(IDS[0], "Release");
    big.push_str(&"x".repeat(1024 * 1024 + 1));
    write(&root, "decision-release.md", &big);
    let run = history(&root, &["release", "--diff", &ids[0][..12]]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains("notes/decision-release.md"),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("has no file"));
}

#[test]
fn arguments_are_checked() {
    let dir = TempDir::new("history-arguments");
    let root = store(&dir);
    for args in [
        vec![],
        vec!["--now"],
        vec!["release", "--now"],
        vec!["release", "abcdef", "012345"],
        vec!["release", "--diff"],
        vec!["release", "--diff", "abcdef", "012345", "fedcba"],
    ] {
        let run = history(&root, &args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty());
    }
}

#[test]
fn a_stale_history_is_warned_about_unless_a_watcher_runs() {
    let dir = TempDir::new("history-stale");
    let (root, _, _) = recorded(&dir);
    let run = history(&root, &["release"]);
    assert_eq!((run.code, run.stderr.as_str()), (0, WARNING));
    let _watcher = Watcher::on(&root);
    let run = history(&root, &["release"]);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
}

#[test]
fn history_changes_nothing() {
    let dir = TempDir::new("history-read-only");
    let (root, _, _) = recorded(&dir);
    let versions = shorts(&root, "release");
    let before = snapshot(&root);
    for args in [
        vec!["release"],
        vec![IDS[0]],
        vec!["release", versions[0].as_str()],
        vec!["release", "--diff", versions[1].as_str()],
        vec![
            "release",
            "--diff",
            versions[1].as_str(),
            versions[0].as_str(),
        ],
        vec!["wumpus"],
    ] {
        history(&root, &args);
    }
    assert_eq!(snapshot(&root), before);
}

#[test]
fn a_missing_store_is_refused() {
    let dir = TempDir::new("history-no-store");
    let root = dir.path().join("missing");
    let run = history(&root, &["release"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", root.display())
    );
    assert!(!root.exists());
}

#[test]
fn history_ignores_the_config() {
    let dir = TempDir::new("history-config");
    let (root, _, _) = recorded(&dir);
    let missing = dir.path().join("nope");
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", missing.to_str().unwrap()),
        ],
        &["history", "release"],
    );
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.lines().count(), 2);
}

const RHOSGOBEL: &str = "gr2q7gf5lh6pzfdnurnkvputhp";
const BYWATER: &str = "wyxim75c6m5p4ywv22ywilqweh";

fn hex(c: char) -> String {
    c.to_string().repeat(64)
}

/// Copies the fixture store's scope folder, which lists `rhosgobel` and `bywater`, under `<root>/.bilbo/scopes/`.
fn with_manifest(root: &Path) {
    let scope = "5wdimrj6nm6qlhvdto4qvgz5qy";
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/device/store/.bilbo/scopes")
        .join(scope)
        .join("manifest");
    let to = root.join(".bilbo/scopes").join(scope).join("manifest");
    fs::create_dir_all(&to).unwrap();
    for n in ["1.json", "2.json"] {
        fs::copy(from.join(n), to.join(n)).unwrap();
    }
}

/// A hand-written version line: version, parents, event, file, blob and extra fields.
type Row = (
    String,
    Vec<String>,
    &'static str,
    &'static str,
    String,
    serde_json::Value,
);

/// Appends hand-written version lines to a note's log.
fn log_lines(root: &Path, note: &str, lines: &[Row]) {
    write(root, "decision-release.md", &note_text(IDS[0], "Release"));
    let path = root.join(format!(".bilbo/history/notes/{note}.jsonl"));
    let mut text = fs::read_to_string(&path).unwrap_or_default();
    for (n, (version, parents, event, file, blob, extra)) in lines.iter().enumerate() {
        let mut line = serde_json::json!({
            "version": version,
            "parents": parents,
            "file": file,
            "blob": blob,
            "event": event,
            "at": format!("2026-10-04T12:0{n}:00-03:00"),
        });
        for (key, value) in extra.as_object().unwrap() {
            line[key] = value.clone();
        }
        text.push_str(&line.to_string());
        text.push('\n');
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn plain() -> serde_json::Value {
    serde_json::json!({})
}

#[test]
fn a_version_from_another_device_names_it() {
    let dir = TempDir::new("history-from");
    let root = store(&dir);
    with_manifest(&root);
    log_lines(
        &root,
        IDS[0],
        &[
            (
                hex('a'),
                vec![],
                "added",
                "decision-release.md",
                hex('1'),
                plain(),
            ),
            (
                hex('b'),
                vec![hex('a')],
                "edited",
                "decision-release.md",
                hex('2'),
                serde_json::json!({"device": BYWATER}),
            ),
            (
                hex('c'),
                vec![hex('b')],
                "edited",
                "decision-release.md",
                hex('3'),
                serde_json::json!({"device": "unlistedunlistedunlisted0"}),
            ),
        ],
    );
    let lines: Vec<String> = history(&root, &["release"])
        .stdout
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 3);
    assert!(
        lines[0].ends_with(" edited decision-release.md from unlistedunlistedunlisted0"),
        "{lines:?}"
    );
    assert!(
        lines[1].ends_with(" edited decision-release.md from bywater"),
        "{lines:?}"
    );
    assert!(
        lines[2].ends_with(" added decision-release.md"),
        "{lines:?}"
    );
    assert!(!lines[2].contains(" from "));
}

#[test]
fn a_device_shows_as_its_id_without_a_manifest() {
    let dir = TempDir::new("history-from-id");
    let root = store(&dir);
    log_lines(
        &root,
        IDS[0],
        &[(
            hex('a'),
            vec![],
            "edited",
            "decision-release.md",
            hex('1'),
            serde_json::json!({"device": RHOSGOBEL}),
        )],
    );
    let run = history(&root, &["release"]);
    assert!(
        run.stdout
            .trim_end()
            .ends_with(&format!("edited decision-release.md from {RHOSGOBEL}")),
        "{}",
        run.stdout
    );
}

#[test]
fn a_merge_with_a_conflict_is_flagged() {
    let dir = TempDir::new("history-merge");
    let root = store(&dir);
    with_manifest(&root);
    log_lines(
        &root,
        IDS[0],
        &[
            (
                hex('a'),
                vec![],
                "added",
                "decision-release.md",
                hex('1'),
                plain(),
            ),
            (
                hex('b'),
                vec![hex('a')],
                "edited",
                "decision-release.md",
                hex('2'),
                serde_json::json!({"device": BYWATER}),
            ),
            (
                hex('c'),
                vec![hex('a'), hex('b')],
                "merged",
                "decision-release.md",
                hex('3'),
                serde_json::json!({"conflict": [{"passage": "# Release", "sides": [hex('a')[..12], hex('b')[..12]]}]}),
            ),
        ],
    );
    let lines: Vec<String> = history(&root, &["release"])
        .stdout
        .lines()
        .map(str::to_string)
        .collect();
    assert!(
        lines[0].ends_with(" merged decision-release.md [conflict]"),
        "{lines:?}"
    );
    assert!(
        lines[1].ends_with(" edited decision-release.md from bywater"),
        "{lines:?}"
    );
}

#[test]
fn flags_and_dropped_text_are_listed_together() {
    let dir = TempDir::new("history-flags");
    let root = store(&dir);
    log_lines(
        &root,
        IDS[0],
        &[
            (
                hex('a'),
                vec![],
                "added",
                "decision-release.md",
                hex('1'),
                plain(),
            ),
            (
                hex('b'),
                vec![hex('a')],
                "merged",
                "decision-release.md",
                hex('2'),
                serde_json::json!({
                    "device": RHOSGOBEL,
                    "flags": ["stale-base", "key-kept"],
                    "dropped": [{"passage": "# Release", "lines": ["x"]}],
                }),
            ),
        ],
    );
    let first = history(&root, &["release"])
        .stdout
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(
        first.ends_with(&format!(
            " merged decision-release.md from {RHOSGOBEL} [stale-base, key-kept, dropped]"
        )),
        "{first}"
    );
}

#[test]
fn a_left_version_shows_the_followed_file_and_has_no_text() {
    let dir = TempDir::new("history-left");
    let root = store(&dir);
    let ids = seed(
        &root,
        IDS[0],
        &[Seed::new(
            "decision-release.md",
            Some("one\n"),
            "added",
            &days_ago(2),
        )],
    );
    log_lines(
        &root,
        IDS[0],
        &[(
            hex('9'),
            vec![ids[0].clone()],
            "left",
            "decision-release.md",
            "deleted".into(),
            plain(),
        )],
    );
    let list = history(&root, &["release"]);
    assert_eq!(list.code, 0);
    let first = list.stdout.lines().next().unwrap().to_string();
    assert!(first.ends_with(" left decision-release.md"), "{first}");
    let printed = history(&root, &["release", &hex('9')[..12]]);
    assert_eq!(printed.code, 1);
    assert!(printed.stdout.is_empty());
    let diff = history(
        &root,
        &["release", "--diff", &ids[0][..12], &hex('9')[..12]],
    );
    assert_eq!(diff.code, 0);
    assert!(diff.stdout.contains("-one"), "{}", diff.stdout);
}

#[test]
fn a_declaration_line_is_not_a_version() {
    let dir = TempDir::new("history-declare");
    let root = store(&dir);
    let ids = seed(
        &root,
        IDS[0],
        &[Seed::new(
            "decision-release.md",
            Some("one\n"),
            "added",
            &days_ago(2),
        )],
    );
    write(&root, "decision-release.md", &note_text(IDS[0], "Release"));
    let path = root.join(format!(".bilbo/history/notes/{}.jsonl", IDS[0]));
    let mut log = fs::read_to_string(&path).unwrap();
    log.push_str(&format!(
        "{{\"declare\":\"{}\",\"reason\":\"tidy-up\",\"at\":\"2026-10-04T12:00:00-03:00\"}}\n",
        ids[0]
    ));
    fs::write(&path, log).unwrap();
    let run = history(&root, &["release"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.lines().count(), 1);
}

/// stdout and stderr on terminals, without colour.
fn on_terminal(root: &Path, args: &[&str]) -> common::Run {
    let mut all = vec!["history"];
    all.extend(args);
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("NO_COLOR", "1"),
        ("LANG", "C.UTF-8"),
    ];
    common::bilbo_tty(&std::env::temp_dir(), &env, &all, 100)
}

#[test]
fn equal_versions_say_so_on_a_terminal_and_nothing_in_a_pipe() {
    let dir = TempDir::new("history-equal");
    let (root, _, _) = recorded(&dir);
    let version = shorts(&root, "release")[0].clone();
    let run = on_terminal(&root, &["release", "--diff", &version, &version]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("◇  no changes between {version} and {version}\n")
    );
    let piped = history(&root, &["release", "--diff", &version, &version]);
    assert_eq!(piped.stdout, "");
}

#[test]
fn a_version_prints_its_bytes_on_a_terminal() {
    let dir = TempDir::new("history-bytes-tty");
    let (root, first, _) = recorded(&dir);
    let version = shorts(&root, "release")[1].clone();
    let run = on_terminal(&root, &["release", &version]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, first);
}
