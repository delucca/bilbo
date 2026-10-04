mod common;

use std::path::{Path, PathBuf};

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

const DIGEST: &str = "9b312fd3a9d5203944d60e7b319c7703e53785ac2b7721b2975cba4dd758526d";
const GUIDE_ID: &str = "01M3EZ8NBEVNHZRTQ6T60171J2";

fn source_text(id: &str) -> String {
    format!(
        "---\nid: {id}\nfetched: 2026-08-23\norigin: \"url: https://go.dev/doc/effective_go\"\ndigest: sha256:{DIGEST}\n---\n# Effective Go\n\ntext\n"
    )
}

fn guide_text(entries: &[(&str, &str)]) -> String {
    let mut text = format!(
        "---\nid: {GUIDE_ID}\ncreated: 2026-09-26T13:16-03:00\n---\n\n# Go\n\nWhat it grounds.\n"
    );
    for (name, prose) in entries {
        text.push_str(&format!("\n## {name}\n\n{prose}\n"));
    }
    text
}

fn put(root: &Path, path: &str, text: &str) {
    let real = root.join(path);
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(real, text).unwrap();
}

/// A store with `notes/` and a `go` corpus holding `effective-go`.
fn library_store(dir: &TempDir) -> PathBuf {
    let root = store(dir);
    put(
        &root,
        "library/go/guide.md",
        &guide_text(&[("effective-go", "Prose.")]),
    );
    put(&root, "library/go/effective-go.md", &source_text(IDS[1]));
    root
}

#[test]
fn clean_store_with_a_library_prints_nothing() {
    let dir = TempDir::new("check-lib-clean");
    let root = library_store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[0], "A"));
    put(&root, "library/.lock", "");
    put(&root, "library/go/.DS_Store", "x");
    let run = check(&dir, &root);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.is_empty() && run.stderr.is_empty());
}

#[test]
fn an_edited_source_names_the_digest() {
    let dir = TempDir::new("check-lib-digest");
    let root = library_store(&dir);
    let edited = source_text(IDS[1]).replace("text", "tent");
    put(&root, "library/go/effective-go.md", &edited);
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("library/go/effective-go.md: digest: does not match the body"),
        "{lines:?}"
    );
    put(
        &root,
        "library/go/effective-go.md",
        &format!("{}\n", source_text(IDS[1])),
    );
    assert_eq!(check(&dir, &root).code, 1);
}

#[test]
fn a_stub_entry_names_the_entry() {
    let dir = TempDir::new("check-lib-stub");
    let root = library_store(&dir);
    put(
        &root,
        "library/go/guide.md",
        &guide_text(&[("effective-go", "TODO: describe this source.")]),
    );
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "library/go/guide.md: entry 'effective-go': TODO stub; write the entry and remove the TODO line (line 12)\n"
    );
}

#[test]
fn a_stale_entry_is_reported() {
    let dir = TempDir::new("check-lib-stale");
    let root = library_store(&dir);
    let stale = "stale: re-ingested 2026-10-03; re-read the source and revise this entry.";
    put(
        &root,
        "library/go/guide.md",
        &guide_text(&[("effective-go", stale)]),
    );
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert!(
        run.stdout.contains("entry 'effective-go': stale;"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_source_with_no_entry_and_an_entry_with_no_source() {
    let dir = TempDir::new("check-lib-entries");
    let root = library_store(&dir);
    put(
        &root,
        "library/go/inspecting-errors.md",
        &source_text(IDS[2]),
    );
    put(
        &root,
        "library/go/guide.md",
        &guide_text(&[("effective-go", "Prose."), ("Reading order", "Prose.")]),
    );
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "library/go/guide.md: entry 'Reading order': no source Reading order.md in this corpus (line 14)\n\
         library/go/inspecting-errors.md: guide: no '## inspecting-errors' entry in guide.md\n"
    );
}

#[test]
fn a_reserved_corpus_and_a_loose_file() {
    let dir = TempDir::new("check-lib-layout");
    let root = library_store(&dir);
    put(&root, "library/plan/guide.md", "x");
    put(&root, "library/effective-go.md", "x");
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "library/effective-go.md: entry: library/ holds only corpus folders\n\
         library/plan: corpus: 'plan' is reserved for a library subcommand\n"
    );
}

#[test]
fn a_note_and_a_source_share_an_id() {
    let dir = TempDir::new("check-lib-shared-id");
    let root = library_store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[1], "A"));
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        format!(
            "library/go/effective-go.md: id: {id} is also the id of notes/plan-a.md\n\
             notes/plan-a.md: id: {id} is also the id of library/go/effective-go.md\n",
            id = IDS[1]
        )
    );
}

#[test]
fn a_guide_and_a_source_share_an_id() {
    let dir = TempDir::new("check-lib-guide-id");
    let root = library_store(&dir);
    put(
        &root,
        "library/go/guide.md",
        &guide_text(&[("effective-go", "Prose.")]).replace(GUIDE_ID, IDS[1]),
    );
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout.lines().count(), 2, "{}", run.stdout);
    assert!(run.stdout.contains("library/go/guide.md: id: "));
}

#[test]
fn a_library_without_notes_is_a_store() {
    let dir = TempDir::new("check-lib-only");
    let root = dir.path().join("store");
    put(
        &root,
        "library/go/guide.md",
        &guide_text(&[("effective-go", "Prose.")]),
    );
    put(&root, "library/go/effective-go.md", &source_text(IDS[1]));
    assert!(!root.join("notes").exists());
    let run = check(&dir, &root);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.is_empty() && run.stderr.is_empty());
}

#[test]
fn neither_folder_is_no_store() {
    let dir = TempDir::new("check-neither");
    let root = dir.path().join("store");
    std::fs::create_dir_all(&root).unwrap();
    let run = check(&dir, &root);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", root.display())
    );
}

#[test]
fn captures_are_not_checked() {
    let dir = TempDir::new("check-captures");
    let root = library_store(&dir);
    put(&root, ".bilbo/captures/not-the-hash/capture.md", "text");
    std::fs::create_dir_all(root.join(".bilbo/captures/0000/")).unwrap();
    put(&root, ".bilbo/captures/stray.txt", "x");
    let run = check(&dir, &root);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.is_empty());
}

#[test]
fn library_check_leaves_the_store_as_found() {
    let dir = TempDir::new("check-lib-readonly");
    let root = library_store(&dir);
    put(
        &root,
        "library/go/effective-go.md",
        &source_text(IDS[1]).replace("text", "tent"),
    );
    put(&root, "library/plan/guide.md", "x");
    let before = snapshot(&root);
    assert_eq!(check(&dir, &root).code, 1);
    assert_eq!(snapshot(&root), before);
}
