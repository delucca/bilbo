mod common;

use std::path::Path;

use common::{IDS, Run, TempDir, bilbo, note_text, snapshot, store, write};

fn check(dir: &TempDir, root: &Path) -> Run {
    bilbo(
        dir.path(),
        &[("BILBO_HOME", root.to_str().unwrap())],
        &["check"],
    )
}

fn stdout_lines(run: &Run) -> Vec<&str> {
    run.stdout.lines().collect()
}

#[test]
fn clean_store_prints_nothing() {
    let dir = TempDir::new("check-clean");
    let root = store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[0], "A"));
    write(&root, "decision-b.md", &note_text(IDS[1], "B"));
    let run = check(&dir, &root);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty() && run.stderr.is_empty());
}

#[test]
fn bad_created_is_reported() {
    let dir = TempDir::new("check-created");
    let root = store(&dir);
    for bad in ["2026-02-30T10:00-03:00", "2026-10-02"] {
        let text = note_text(IDS[0], "A").replace("2026-10-02T14:23-03:00", bad);
        write(&root, "plan-a.md", &text);
        let run = check(&dir, &root);
        assert_eq!(run.code, 1);
        let lines = stdout_lines(&run);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].starts_with(&format!("notes/plan-a.md: created: '{bad}' is not")),
            "{lines:?}"
        );
        assert!(lines[0].ends_with("(line 3)"));
    }
}

#[test]
fn empty_store_prints_nothing() {
    let dir = TempDir::new("check-empty");
    let root = store(&dir);
    let run = check(&dir, &root);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
}

#[test]
fn two_problems_in_one_file() {
    let dir = TempDir::new("check-two");
    let root = store(&dir);
    let text =
        "---\nid: 01m3yj7r6hk6nq30dcdb1p4dyb\ncreated: 2026-10-02T14:23-03:00\n---\n\n# A\n\n# B\n";
    write(&root, "plan-release.md", text);
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("notes/plan-release.md: id: '01m3"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("notes/plan-release.md: title: found 2"),
        "{lines:?}"
    );
}

#[test]
fn shared_id_names_the_other_file() {
    let dir = TempDir::new("check-shared-id");
    let root = store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[0], "A"));
    write(&root, "plan-b.md", &note_text(IDS[0], "B"));
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        format!(
            "notes/plan-a.md: id: {id} is also the id of notes/plan-b.md\nnotes/plan-b.md: id: {id} is also the id of notes/plan-a.md\n",
            id = IDS[0]
        )
    );
}

#[test]
fn shared_topic_names_the_other_file() {
    let dir = TempDir::new("check-shared-topic");
    let root = store(&dir);
    write(&root, "plan-release.md", &note_text(IDS[0], "A"));
    write(&root, "decision-release.md", &note_text(IDS[1], "B"));
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/decision-release.md: topic: 'release' is also the topic of notes/plan-release.md\n\
         notes/plan-release.md: topic: 'release' is also the topic of notes/decision-release.md\n"
    );
}

#[test]
fn different_topics_coexist() {
    let dir = TempDir::new("check-different");
    let root = store(&dir);
    write(&root, "plan-release.md", &note_text(IDS[0], "A"));
    write(&root, "plan-rollback.md", &note_text(IDS[1], "B"));
    let run = check(&dir, &root);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
}

#[test]
fn hidden_entry_is_ignored() {
    let dir = TempDir::new("check-hidden");
    let root = store(&dir);
    std::fs::write(root.join("notes/.DS_Store"), [0u8, 159, 146, 150]).unwrap();
    let run = check(&dir, &root);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
}

#[test]
fn bad_names_are_reported() {
    let dir = TempDir::new("check-names");
    let root = store(&dir);
    write(&root, "Idea-Foo.md", &note_text(IDS[0], "A"));
    write(&root, "idea-bar.md", &note_text(IDS[1], "B"));
    write(&root, "plan-foo--bar.md", &note_text(IDS[2], "C"));
    write(&root, "notes.txt", "not a note");
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    let kinds = "plan, spec, design, decision, gotcha, research, review, report, reference";
    assert_eq!(
        run.stdout,
        format!(
            "notes/Idea-Foo.md: name: unknown kind 'Idea'; kinds: {kinds}\n\
             notes/idea-bar.md: name: unknown kind 'idea'; kinds: {kinds}\n\
             notes/notes.txt: name: must be <kind>-<topic>.md\n\
             notes/plan-foo--bar.md: name: invalid topic 'foo--bar': use segments of a-z and 0-9 joined by single hyphens\n"
        )
    );
}

#[test]
fn subfolder_is_reported() {
    let dir = TempDir::new("check-subfolder");
    let root = store(&dir);
    std::fs::create_dir(root.join("notes/archive")).unwrap();
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/archive: folder: notes/ holds only note files\n"
    );
}

#[test]
fn check_leaves_store_as_found() {
    let dir = TempDir::new("check-readonly");
    let root = store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[0], "A"));
    write(&root, "plan-b.md", &note_text(IDS[0], "B"));
    write(&root, "decision-a.md", "# no frontmatter\n");
    write(&root, ".new-X.tmp", "hidden");
    std::fs::create_dir(root.join("notes/archive")).unwrap();
    let before = snapshot(&root);
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert!(!run.stdout.is_empty());
    assert_eq!(snapshot(&root), before);
}

#[test]
fn missing_store_is_a_problem() {
    let dir = TempDir::new("check-missing");
    let root = dir.path().join("nowhere");
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", root.display())
    );
    assert!(!root.exists());
}

#[test]
fn lines_are_sorted_by_path_then_message() {
    let dir = TempDir::new("check-order");
    let root = store(&dir);
    let broken = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02\n---\n\nno heading\n";
    write(&root, "plan-a.md", broken);
    write(&root, "plan-a.md.bak", "backup");
    write(&root, "design-z.md", "# no frontmatter\n");
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    let paths: Vec<&str> = lines
        .iter()
        .map(|l| l.split(": ").next().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "notes/design-z.md",
            "notes/plan-a.md",
            "notes/plan-a.md",
            "notes/plan-a.md.bak"
        ],
        "{lines:?}"
    );
    assert!(lines[1].starts_with("notes/plan-a.md: created:"));
    assert!(lines[2].starts_with("notes/plan-a.md: title:"));
}

#[test]
fn control_characters_in_names_keep_one_line_per_problem() {
    let dir = TempDir::new("check-control");
    let root = store(&dir);
    write(&root, "plan-a\nb.md", &note_text(IDS[0], "A"));
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 1, "{:?}", run.stdout);
    assert!(
        lines[0].starts_with("notes/plan-a\\nb.md: name: invalid topic 'a\\nb'"),
        "{lines:?}"
    );
}

#[test]
#[cfg_attr(target_os = "macos", ignore = "APFS refuses non-UTF-8 names")]
fn non_utf8_name_is_reported_and_not_read() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new("check-non-utf8");
    let root = store(&dir);
    let odd = root
        .join("notes")
        .join(std::ffi::OsStr::from_bytes(b"plan-\xff.md"));
    std::fs::write(&odd, "not a note").unwrap();
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/plan-\u{fffd}.md: name: not valid UTF-8\n"
    );
}

#[test]
fn dangling_symlink_still_holds_its_topic() {
    let dir = TempDir::new("check-dangling");
    let root = store(&dir);
    write(&root, "plan-release.md", &note_text(IDS[0], "A"));
    std::os::unix::fs::symlink(
        dir.path().join("missing"),
        root.join("notes/decision-release.md"),
    )
    .unwrap();
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].starts_with("notes/decision-release.md: read:"),
        "{lines:?}"
    );
    assert_eq!(
        lines[1],
        "notes/decision-release.md: topic: 'release' is also the topic of notes/plan-release.md"
    );
    assert_eq!(
        lines[2],
        "notes/plan-release.md: topic: 'release' is also the topic of notes/decision-release.md"
    );
}
