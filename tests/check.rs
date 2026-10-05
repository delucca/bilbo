mod common;

use std::path::{Path, PathBuf};

use common::{IDS, Run, TempDir, bilbo, bilbo_scoped, note_text, scoped, snapshot, store, write};

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

/// A note with `extra` lines after `created` and `body` after the title.
fn scoped_note(id: &str, extra: &str, body: &str) -> String {
    format!("---\nid: {id}\ncreated: 2026-10-02T14:23-03:00\n{extra}---\n\n# T\n\n{body}")
}

fn check_scoped(dir: &TempDir, lines: &[&str], notes: &[(&str, String)]) -> Run {
    let s = scoped(dir, lines);
    for (name, text) in notes {
        std::fs::create_dir_all(s.root.join("notes")).unwrap();
        write(&s.root, name, text);
    }
    bilbo_scoped(&s, &s.home, &["check"])
}

#[test]
fn no_scopes_and_no_scope_lines_print_nothing_about_scope() {
    let dir = TempDir::new("check-scope-none");
    let run = check_scoped(
        &dir,
        &["digest.log = off"],
        &[("plan-a.md", scoped_note(IDS[0], "", "text\n"))],
    );
    assert_eq!((run.code, run.stdout.as_str()), (0, ""), "{}", run.stderr);
}

#[test]
fn a_missing_scope_is_a_problem() {
    let dir = TempDir::new("check-scope-missing");
    let run = check_scoped(
        &dir,
        &["scope.personal.sync = off", "scope.work.sync = off"],
        &[("plan-release.md", scoped_note(IDS[0], "", "text\n"))],
    );
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/plan-release.md: scope: missing; scopes: personal, work\n"
    );
}

#[test]
fn an_undeclared_scope_is_a_problem() {
    let dir = TempDir::new("check-scope-undeclared");
    let s = scoped(&dir, &["scope.work.sync = off"]);
    std::fs::create_dir_all(s.root.join("notes")).unwrap();
    write(
        &s.root,
        "plan-release.md",
        &scoped_note(IDS[0], "scope: acme\n", "x\n"),
    );
    let run = bilbo_scoped(&s, &s.home, &["check"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        format!(
            "notes/plan-release.md: scope: 'acme' is not declared in {}; scopes: work\n",
            s.config.display()
        )
    );
}

#[test]
fn a_scope_with_no_scopes_declared_is_a_problem() {
    let dir = TempDir::new("check-scope-none-declared");
    let s = scoped(&dir, &["digest.log = off"]);
    std::fs::create_dir_all(s.root.join("notes")).unwrap();
    write(
        &s.root,
        "plan-release.md",
        &scoped_note(IDS[0], "scope: work\n", "x\n"),
    );
    let run = bilbo_scoped(&s, &s.home, &["check"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        format!(
            "notes/plan-release.md: scope: 'work' is not declared in {}\n",
            s.config.display()
        )
    );
}

#[test]
fn an_invalid_scope_value_is_reported_once() {
    let dir = TempDir::new("check-scope-invalid");
    let run = check_scoped(
        &dir,
        &["scope.work.sync = off"],
        &[
            ("plan-a.md", scoped_note(IDS[0], "scope: Work\n", "x\n")),
            (
                "plan-b.md",
                scoped_note(IDS[1], "scope: work\nscope: work\n", "x\n"),
            ),
        ],
    );
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in lines {
        assert!(line.contains("scope"), "{line}");
        assert!(
            !line.contains("is not declared") && !line.contains("missing"),
            "{line}"
        );
    }
}

#[test]
fn a_broken_config_stops_check() {
    let dir = TempDir::new("check-scope-broken");
    let run = check_scoped(
        &dir,
        &["scope.work.embedder = remote"],
        &[("plan-a.md", scoped_note(IDS[0], "", "x\n"))],
    );
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("scope.work.embedder"), "{}", run.stderr);
    assert!(run.stdout.is_empty());
}

#[test]
fn check_ignores_what_the_config_does_not_say_about_scopes() {
    let dir = TempDir::new("check-config-ignored");
    let root = store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[0], "A"));
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("HOME", home.to_str().unwrap()),
        ],
        &["check"],
    );
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "", "")
    );

    let url = common::dead_url();
    let config = common::config(
        &dir,
        &[&format!("embedder.url = {url}"), "embedder.model = m"],
    );
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", config.to_str().unwrap()),
        ],
        &["check"],
    );
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "", "")
    );
}

#[test]
fn check_reads_the_config() {
    let dir = TempDir::new("check-config-read");
    let root = store(&dir);
    write(&root, "plan-a.md", &note_text(IDS[0], "A"));
    let missing = dir.path().join("nope");
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", missing.to_str().unwrap()),
        ],
        &["check"],
    );
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains(missing.to_str().unwrap()),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty());
}

#[test]
fn a_mark_of_another_scope_warns_and_leaves_the_exit_at_zero() {
    let dir = TempDir::new("check-mark-line");
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme", "scope.personal.sync = off"],
        &[(
            "gotcha-deploy.md",
            scoped_note(
                IDS[0],
                "scope: personal\n",
                "one\ntwo\nAcme's deploy\nacme again\n",
            ),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(
        run.stdout,
        "notes/gotcha-deploy.md: scope: 'personal' but line 11 holds 'acme', a mark of 'work' (warning)\n"
    );
}

#[test]
fn a_mark_in_the_file_name_warns() {
    let dir = TempDir::new("check-mark-name");
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme", "scope.personal.sync = off"],
        &[(
            "gotcha-acme-deploy.md",
            scoped_note(IDS[0], "scope: personal\n", "nothing\n"),
        )],
    );
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        "notes/gotcha-acme-deploy.md: scope: 'personal' but the file name holds 'acme', a mark of 'work' (warning)\n"
    );
}

#[test]
fn a_mark_of_the_notes_own_scope_is_silent() {
    let dir = TempDir::new("check-mark-own");
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme"],
        &[("plan-a.md", scoped_note(IDS[0], "scope: work\n", "acme\n"))],
    );
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
}

#[test]
fn an_unassigned_note_lists_the_marks_it_holds() {
    let dir = TempDir::new("check-mark-unassigned");
    let s = scoped(
        &dir,
        &["scope.personal.sync = off", "scope.work.marks = acme"],
    );
    std::fs::create_dir_all(s.root.join("notes")).unwrap();
    write(&s.root, "plan-x.md", &scoped_note(IDS[0], "", "see acme\n"));
    write(
        &s.root,
        "plan-y.md",
        &scoped_note(IDS[1], "scope: gone\n", "see acme\n"),
    );
    let run = bilbo_scoped(&s, &s.home, &["check"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        format!(
            "notes/plan-x.md: scope: missing; scopes: personal, work; holds marks of work\n\
             notes/plan-y.md: scope: 'gone' is not declared in {}; scopes: personal, work; holds marks of work\n",
            s.config.display()
        )
    );
}

#[test]
fn warnings_and_problems_sort_together_by_path() {
    let dir = TempDir::new("check-mark-sort");
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme", "scope.personal.sync = off"],
        &[
            (
                "plan-b.md",
                scoped_note(IDS[0], "scope: personal\n", "acme\n"),
            ),
            ("plan-a.md", scoped_note(IDS[1], "", "x\n")),
            ("plan-c.md", scoped_note(IDS[2], "scope: work\n", "x\n")),
        ],
    );
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("notes/plan-a.md: scope: missing"));
    assert!(lines[1].starts_with("notes/plan-b.md: ") && lines[1].ends_with("(warning)"));
}

#[test]
fn marks_match_in_sources_but_not_inside_words() {
    let dir = TempDir::new("check-mark-matching");
    let sources = "sources:\n  - \"url: https://wiki.acme.example/deploy\"\n";
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme", "scope.personal.sync = off"],
        &[
            (
                "plan-a.md",
                scoped_note(IDS[0], &format!("scope: personal\n{sources}"), "x\n"),
            ),
            (
                "plan-b.md",
                scoped_note(IDS[1], "scope: personal\n", "acmeish\n"),
            ),
        ],
    );
    assert_eq!(run.code, 0);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("notes/plan-a.md: scope: 'personal' but line 6 holds 'acme'"),
        "{}",
        lines[0]
    );
}

#[test]
fn a_path_mark_matches_in_either_form_and_not_a_sibling() {
    let dir = TempDir::new("check-mark-path");
    let s = scoped(
        &dir,
        &[
            "scope.work.marks = ~/Developer/acme",
            "scope.personal.sync = off",
        ],
    );
    std::fs::create_dir_all(s.root.join("notes")).unwrap();
    let absolute = format!("see {}/Developer/acme/api/main.go\n", s.home.display());
    write(
        &s.root,
        "plan-a.md",
        &scoped_note(IDS[0], "scope: personal\n", &absolute),
    );
    write(
        &s.root,
        "plan-b.md",
        &scoped_note(IDS[1], "scope: personal\n", "see ~/Developer/acme/x\n"),
    );
    write(
        &s.root,
        "plan-c.md",
        &scoped_note(IDS[2], "scope: personal\n", "see ~/Developer/acme-tools\n"),
    );
    let run = bilbo_scoped(&s, &s.home, &["check"]);
    assert_eq!(run.code, 0);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("notes/plan-a.md: ") && lines[1].starts_with("notes/plan-b.md: "));
}

#[test]
fn a_warning_prints_the_mark_as_written() {
    let dir = TempDir::new("check-mark-written");
    let run = check_scoped(
        &dir,
        &[
            "scope.work.marks = ~/Developer/acme, Ação",
            "scope.personal.sync = off",
        ],
        &[(
            "plan-a.md",
            scoped_note(IDS[0], "scope: personal\n", "see ~/Developer/acme/x\n"),
        )],
    );
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        "notes/plan-a.md: scope: 'personal' but line 9 holds '~/Developer/acme', a mark of 'work' (warning)\n"
    );
}

#[test]
fn a_note_holding_marks_of_two_scopes_gets_a_warning_for_each() {
    let dir = TempDir::new("check-mark-two");
    let run = check_scoped(
        &dir,
        &[
            "scope.personal.marks = Ação",
            "scope.work.marks = acme",
            "scope.beta.sync = off",
        ],
        &[(
            "gotcha-acme-deploy.md",
            scoped_note(IDS[0], "scope: beta\n", "AÇÃO\n"),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(
        run.stdout,
        "notes/gotcha-acme-deploy.md: scope: 'beta' but line 9 holds 'Ação', a mark of 'personal' (warning)\n\
         notes/gotcha-acme-deploy.md: scope: 'beta' but the file name holds 'acme', a mark of 'work' (warning)\n"
    );
}

#[test]
fn holds_marks_of_lists_the_scopes_sorted() {
    let dir = TempDir::new("check-mark-sorted");
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme", "scope.personal.marks = Ação"],
        &[("plan-x.md", scoped_note(IDS[0], "", "acao and acme\n"))],
    );
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/plan-x.md: scope: missing; scopes: personal, work; holds marks of personal, work\n"
    );
}

#[test]
fn the_earliest_place_of_a_scopes_marks_is_reported() {
    let dir = TempDir::new("check-mark-earliest");
    let run = check_scoped(
        &dir,
        &[
            "scope.work.marks = acme, ~/Developer/zeta",
            "scope.beta.sync = off",
        ],
        &[(
            "plan-a.md",
            scoped_note(IDS[0], "scope: beta\n", "x ~/Developer/zeta/y\nacme\n"),
        )],
    );
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        "notes/plan-a.md: scope: 'beta' but line 9 holds '~/Developer/zeta', a mark of 'work' (warning)\n"
    );
}

#[test]
fn a_problem_and_a_warning_on_one_file_sort_by_message() {
    let dir = TempDir::new("check-mark-same-file");
    let text = scoped_note(IDS[0], "scope: beta\n", "acme\n").replace("T14:23-03:00", "");
    let run = check_scoped(
        &dir,
        &["scope.work.marks = acme", "scope.beta.sync = off"],
        &[("plan-a.md", text)],
    );
    assert_eq!(run.code, 1);
    let lines = stdout_lines(&run);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("notes/plan-a.md: created:"),
        "{lines:?}"
    );
    assert!(lines[1].ends_with("(warning)"), "{lines:?}");
}

#[test]
fn arguments_are_checked_before_the_config() {
    let dir = TempDir::new("check-args-before-config");
    let root = store(&dir);
    let missing = dir.path().join("nope");
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", missing.to_str().unwrap()),
        ],
        &["check", "extra"],
    );
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unexpected argument"), "{}", run.stderr);
    assert!(
        !run.stderr.contains(missing.to_str().unwrap()),
        "{}",
        run.stderr
    );
}
