mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{Run, Scoped, TempDir, bilbo, bilbo_scoped, scoped, store, write};

fn new(dir: &TempDir, root: &Path, args: &[&str]) -> Run {
    let mut full = vec!["new"];
    full.extend(args);
    bilbo(dir.path(), &[("BILBO_HOME", root.to_str().unwrap())], &full)
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn first_note_in_fresh_store() {
    let dir = TempDir::new("new-first");
    let root = dir.path().join("store");
    let run = new(&dir, &root, &["decision", "note-store"]);
    assert_eq!(run.code, 0);
    let expected = root.join("notes/decision-note-store.md");
    assert_eq!(run.stdout, format!("{}\n", expected.display()));
    assert!(expected.is_file());
}

#[cfg(unix)]
#[test]
fn unwritable_store_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("new-unwritable");
    let root = store(&dir);
    let notes = root.join("notes");
    std::fs::set_permissions(&notes, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = notes.join(".probe");
    if std::fs::write(&probe, "").is_ok() {
        let _ = std::fs::remove_file(&probe);
        eprintln!("skipped: the folder is writable despite 0o555 (running as root?)");
        return;
    }
    let run = new(&dir, &root, &["plan", "release"]);
    std::fs::set_permissions(&notes, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("cannot write") && run.stderr.contains("ermission"),
        "{}",
        run.stderr
    );
    assert!(names(&notes).is_empty());
}

#[test]
fn default_title_and_content() {
    let dir = TempDir::new("new-content");
    let root = dir.path().join("store");
    let minute = || {
        jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-3)))
            .strftime("%Y-%m-%dT%H:%M%:z")
            .to_string()
    };
    let before = minute();
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", root.to_str().unwrap()), ("TZ", "<-03>3")],
        &["new", "decision", "note-store"],
    );
    let after = minute();
    assert_eq!(run.code, 0);
    let text = std::fs::read_to_string(root.join("notes/decision-note-store.md")).unwrap();
    assert!(text.ends_with('\n'));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 6, "{text:?}");
    assert_eq!(lines[0], "---");
    let id = lines[1].strip_prefix("id: ").unwrap();
    assert!(
        id.len() == 26
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    );
    let created = lines[2].strip_prefix("created: ").unwrap();
    assert!(
        created == before || created == after,
        "{created} vs {before}/{after}"
    );
    assert_eq!(&lines[3..], ["---", "", "# Note store"]);
}

#[test]
fn explicit_title() {
    let dir = TempDir::new("new-title");
    let root = dir.path().join("store");
    let run = new(
        &dir,
        &root,
        &["decision", "note-store", "--title", "bilbo's note store"],
    );
    assert_eq!(run.code, 0);
    let text = std::fs::read_to_string(root.join("notes/decision-note-store.md")).unwrap();
    assert!(text.ends_with("\n\n# bilbo's note store\n"), "{text:?}");
}

#[test]
fn new_note_passes_check() {
    let dir = TempDir::new("new-check");
    let root = dir.path().join("store");
    assert_eq!(new(&dir, &root, &["plan", "release"]).code, 0);
    assert_eq!(
        new(
            &dir,
            &root,
            &["decision", "rollback", "--title", "Why we roll back"]
        )
        .code,
        0
    );
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", root.to_str().unwrap())],
        &["check"],
    );
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
}

#[test]
fn unusable_title_is_refused() {
    for (i, title) in ["", "a\nb", "   "].into_iter().enumerate() {
        let dir = TempDir::new(&format!("new-bad-title-{i}"));
        let root = dir.path().join("store");
        let run = new(&dir, &root, &["plan", "x", "--title", title]);
        assert_eq!(run.code, 2, "{title:?}");
        assert!(run.stderr.contains("--title must"), "{}", run.stderr);
        assert!(!root.exists());
    }
}

#[test]
fn taken_topic_under_another_kind_is_refused() {
    let dir = TempDir::new("new-taken");
    let root = dir.path().join("store");
    assert_eq!(new(&dir, &root, &["plan", "release"]).code, 0);
    let run = new(&dir, &root, &["decision", "release"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    let existing = root.join("notes/plan-release.md");
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: topic 'release' already has a note: {}\n",
            existing.display()
        )
    );
    assert_eq!(names(&root.join("notes")), ["plan-release.md"]);
}

#[test]
fn existing_file_is_left_unchanged() {
    let dir = TempDir::new("new-existing");
    let root = store(&dir);
    write(&root, "plan-release.md", "custom bytes\n\u{0}\n");
    let run = new(&dir, &root, &["plan", "release"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("already has a note"));
    assert_eq!(
        std::fs::read(root.join("notes/plan-release.md")).unwrap(),
        b"custom bytes\n\0\n"
    );
    assert_eq!(names(&root.join("notes")), ["plan-release.md"]);
}

#[test]
fn unknown_kind_lists_the_kinds() {
    let dir = TempDir::new("new-kind");
    let root = dir.path().join("store");
    let run = new(&dir, &root, &["idea", "foo"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unknown kind 'idea'"));
    for kind in [
        "plan",
        "spec",
        "design",
        "decision",
        "gotcha",
        "research",
        "review",
        "report",
        "reference",
    ] {
        assert!(run.stderr.contains(kind), "{kind}");
    }
    assert!(!root.exists());
}

#[test]
fn invalid_topic_is_refused() {
    let dir = TempDir::new("new-topic");
    let root = dir.path().join("store");
    for topic in ["Release_Steps", "foo--bar", "-foo", "foo-", ""] {
        let run = new(&dir, &root, &["plan", "--", topic]);
        assert_eq!(run.code, 2, "{topic:?}");
        assert!(
            run.stderr.contains(&format!("invalid topic '{topic}'")),
            "{}",
            run.stderr
        );
        assert!(!root.exists());
    }
}

#[test]
fn two_racing_runs_make_one_note() {
    for round in 0..20 {
        let dir = TempDir::new(&format!("new-race-{round}"));
        let root = dir.path().join("store");
        let spawn = |title: &str| {
            Command::new(env!("CARGO_BIN_EXE_bilbo"))
                .env_clear()
                .env("BILBO_HOME", &root)
                .args(["new", "plan", "release", "--title", title])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        };
        let (a, b) = (spawn("A"), spawn("B"));
        let outputs = [
            ("A", a.wait_with_output().unwrap()),
            ("B", b.wait_with_output().unwrap()),
        ];

        let mut codes: Vec<i32> = outputs
            .iter()
            .map(|(_, o)| o.status.code().unwrap())
            .collect();
        codes.sort();
        assert_eq!(codes, [0, 1], "round {round}");
        let (winner, _) = outputs.iter().find(|(_, o)| o.status.success()).unwrap();
        let loser = outputs.iter().find(|(_, o)| !o.status.success()).unwrap();
        assert!(String::from_utf8_lossy(&loser.1.stderr).contains("already has a note"));

        let notes = root.join("notes");
        assert_eq!(names(&notes), ["plan-release.md"], "round {round}");
        let text = std::fs::read_to_string(notes.join("plan-release.md")).unwrap();
        assert!(
            text.ends_with(&format!("# {winner}\n")),
            "round {round}: {text:?}"
        );
    }
}

#[test]
fn title_option_forms() {
    let dir = TempDir::new("new-title-forms");
    let root = dir.path().join("store");
    let cases: [(&[&str], &str, &str); 3] = [
        (&["--title", "T1", "plan", "a"], "plan-a.md", "# T1"),
        (&["plan", "b", "--title=T2"], "plan-b.md", "# T2"),
        (&["--", "plan", "c"], "plan-c.md", "# C"),
    ];
    for (args, file, title) in cases {
        let run = new(&dir, &root, args);
        assert_eq!(run.code, 0, "{args:?}: {}", run.stderr);
        let text = std::fs::read_to_string(root.join("notes").join(file)).unwrap();
        assert!(text.ends_with(&format!("{title}\n")), "{args:?}: {text:?}");
    }
    let run = new(&dir, &root, &["plan", "d", "--title=--help-me"]);
    assert_eq!(run.code, 0);
    let text = std::fs::read_to_string(root.join("notes/plan-d.md")).unwrap();
    assert!(text.ends_with("# --help-me\n"));
}

#[test]
fn bad_title_options_are_refused() {
    let dir = TempDir::new("new-bad-options");
    let root = dir.path().join("store");
    let cases: [(&[&str], &str); 4] = [
        (&["plan", "x", "--title"], "--title needs a value"),
        (
            &["plan", "x", "--title", "a", "--title=b"],
            "--title given more than once",
        ),
        (&["plan", "x", "y"], "unexpected argument 'y'"),
        (&["plan", "x", "--bogus"], "unknown option '--bogus'"),
    ];
    for (args, message) in cases {
        let run = new(&dir, &root, args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr.starts_with(&format!("bilbo: {message}\n")),
            "{}",
            run.stderr
        );
        assert!(!root.exists());
    }
}

#[test]
fn cross_kind_race_ends_with_one_note_or_a_reported_pair() {
    let rounds = 20;
    let mut doubles = 0;
    for round in 0..rounds {
        let dir = TempDir::new(&format!("new-cross-{round}"));
        let root = dir.path().join("store");
        let spawn = |kind: &str| {
            Command::new(env!("CARGO_BIN_EXE_bilbo"))
                .env_clear()
                .env("BILBO_HOME", &root)
                .args(["new", kind, "release"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        };
        let (mut plan, mut decision) = (spawn("plan"), spawn("decision"));
        let codes = [plan.wait().unwrap().code(), decision.wait().unwrap().code()];

        let notes = names(&root.join("notes"));
        match notes.as_slice() {
            [one] => {
                let winner = if one == "plan-release.md" { 0 } else { 1 };
                assert!(
                    one == "plan-release.md" || one == "decision-release.md",
                    "{notes:?}"
                );
                assert_eq!(codes[winner], Some(0), "round {round}: {codes:?}");
                assert_eq!(codes[1 - winner], Some(1), "round {round}: {codes:?}");
            }
            [first, second] => {
                doubles += 1;
                assert_eq!(
                    [first.as_str(), second.as_str()],
                    ["decision-release.md", "plan-release.md"]
                );
                assert_eq!(codes, [Some(0), Some(0)], "round {round}");
                let run = bilbo(
                    dir.path(),
                    &[("BILBO_HOME", root.to_str().unwrap())],
                    &["check"],
                );
                assert_eq!(run.code, 1, "round {round}");
                let lines: Vec<&str> = run.stdout.lines().collect();
                assert_eq!(lines.len(), 2, "round {round}: {lines:?}");
                assert!(
                    lines[0].starts_with("notes/decision-release.md: topic:")
                        && lines[0].contains("notes/plan-release.md")
                );
                assert!(
                    lines[1].starts_with("notes/plan-release.md: topic:")
                        && lines[1].contains("notes/decision-release.md")
                );
            }
            other => panic!("round {round}: unexpected notes {other:?}"),
        }
    }
    eprintln!("cross-kind race: {doubles} of {rounds} rounds ended with two notes");
}

#[test]
fn dangling_symlink_holds_its_topic() {
    let dir = TempDir::new("new-dangling");
    let root = store(&dir);
    let link = root.join("notes/decision-release.md");
    std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
    let run = new(&dir, &root, &["plan", "release"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: topic 'release' already has a note: {}\n",
            link.display()
        )
    );
    assert_eq!(names(&root.join("notes")), ["decision-release.md"]);
}

#[test]
fn title_value_may_look_like_help() {
    let dir = TempDir::new("new-title-help");
    let root = dir.path().join("store");
    let run = new(&dir, &root, &["plan", "x", "--title", "-h"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(root.join("notes/plan-x.md")).unwrap();
    assert!(text.ends_with("# -h\n"), "{text:?}");
}

#[test]
fn scope_value_may_look_like_help() {
    let dir = TempDir::new("new-scope-help");
    let root = dir.path().join("store");
    let run = new(&dir, &root, &["plan", "x", "--scope", "-h"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.starts_with("bilbo: scope '-h' is not declared"),
        "{}",
        run.stderr
    );
    assert!(!root.exists());
}

fn folder(s: &Scoped, rel: &str) -> PathBuf {
    let path = s.home.join(rel);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn note_text(s: &Scoped, topic: &str, kind: &str) -> String {
    std::fs::read_to_string(s.root.join(format!("notes/{kind}-{topic}.md"))).unwrap()
}

fn scope_line(text: &str) -> Option<&str> {
    text.lines().find(|l| l.starts_with("scope:"))
}

#[test]
fn a_scope_line_follows_created() {
    let dir = TempDir::new("new-scope-line");
    let s = scoped(&dir, &["scope.work.paths = ~/acme"]);
    let cwd = folder(&s, "other");
    let run = bilbo_scoped(
        &s,
        &cwd,
        &["new", "decision", "note-store", "--scope", "work"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    let text = note_text(&s, "note-store", "decision");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 7, "{text:?}");
    assert!(lines[2].starts_with("created: "));
    assert_eq!(lines[3], "scope: work");
    assert_eq!(&lines[4..], ["---", "", "# Note store"]);
}

#[test]
fn the_scope_after_equals_is_the_same() {
    let dir = TempDir::new("new-scope-equals");
    let s = scoped(&dir, &["scope.work.paths = ~/acme"]);
    let cwd = folder(&s, "other");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release", "--scope=work"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        scope_line(&note_text(&s, "release", "plan")),
        Some("scope: work")
    );
}

#[test]
fn the_flag_wins() {
    let dir = TempDir::new("new-scope-flag");
    let s = scoped(
        &dir,
        &["scope.work.paths = ~/acme", "scope.personal.paths = ~/me"],
    );
    let cwd = folder(&s, "acme/api");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release", "--scope", "personal"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        scope_line(&note_text(&s, "release", "plan")),
        Some("scope: personal")
    );
}

#[test]
fn the_working_directory_picks_the_scope() {
    let dir = TempDir::new("new-scope-cwd");
    let s = scoped(
        &dir,
        &["scope.work.paths = ~/acme", "scope.personal.paths = ~/me"],
    );
    let cwd = folder(&s, "acme/api");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert_eq!(
        scope_line(&note_text(&s, "release", "plan")),
        Some("scope: work")
    );
}

#[test]
fn the_longest_path_wins() {
    let dir = TempDir::new("new-scope-longest");
    let s = scoped(
        &dir,
        &[
            "scope.personal.paths = ~/Developer",
            "scope.work.paths = ~/Developer/acme",
        ],
    );
    let cwd = folder(&s, "Developer/acme");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        scope_line(&note_text(&s, "release", "plan")),
        Some("scope: work")
    );
}

#[test]
fn a_sibling_folder_does_not_match() {
    let dir = TempDir::new("new-scope-sibling");
    let s = scoped(&dir, &["scope.work.paths = ~/Developer/acme"]);
    folder(&s, "Developer/acme");
    let cwd = folder(&s, "Developer/acme-tools");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(scope_line(&note_text(&s, "release", "plan")), None);
}

#[test]
fn the_default_applies_when_no_path_matches() {
    let dir = TempDir::new("new-scope-default");
    let s = scoped(
        &dir,
        &[
            "scope.personal.sync = off",
            "scope.work.paths = ~/acme",
            "scope.default = personal",
        ],
    );
    let cwd = folder(&s, "elsewhere");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert_eq!(
        scope_line(&note_text(&s, "release", "plan")),
        Some("scope: personal")
    );
}

#[test]
fn nothing_matching_warns_and_exits_zero() {
    let dir = TempDir::new("new-scope-nothing");
    let s = scoped(
        &dir,
        &["scope.work.paths = ~/acme", "scope.personal.paths = ~/me"],
    );
    let cwd = folder(&s, "elsewhere");
    let run = bilbo_scoped(&s, &cwd, &["new", "plan", "release"]);
    let path = s.root.join("notes/plan-release.md");
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, format!("{}\n", path.display()));
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no scope for {}; scopes: personal, work; set one with bilbo scope set <name> {}\n",
            path.display(),
            path.display()
        )
    );
    assert_eq!(scope_line(&note_text(&s, "release", "plan")), None);
}

#[test]
fn no_scope_declared_is_silent() {
    let dir = TempDir::new("new-scope-none");
    let s = scoped(&dir, &["digest.log = off"]);
    let run = bilbo_scoped(&s, &s.home, &["new", "plan", "release"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert_eq!(scope_line(&note_text(&s, "release", "plan")), None);
}

#[test]
fn a_new_note_with_no_config_file_has_no_scope() {
    let dir = TempDir::new("new-no-config");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = dir.path().join("store");
    let run = bilbo(
        &home,
        &[
            ("HOME", home.to_str().unwrap()),
            ("BILBO_HOME", root.to_str().unwrap()),
        ],
        &["new", "plan", "release"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    let text = std::fs::read_to_string(root.join("notes/plan-release.md")).unwrap();
    assert_eq!(scope_line(&text), None);
}

#[test]
fn an_unknown_scope_lists_the_declared_ones() {
    let dir = TempDir::new("new-scope-unknown");
    let s = scoped(
        &dir,
        &["scope.personal.sync = off", "scope.work.sync = off"],
    );
    let run = bilbo_scoped(&s, &s.home, &["new", "plan", "release", "--scope", "acme"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("'acme'") && run.stderr.contains("personal, work"),
        "{}",
        run.stderr
    );
    assert!(!s.root.exists());
}

#[test]
fn no_scope_declared_names_the_config_path() {
    let dir = TempDir::new("new-scope-undeclared");
    let s = scoped(&dir, &["digest.log = off"]);
    let run = bilbo_scoped(&s, &s.home, &["new", "plan", "release", "--scope", "work"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("'work'"), "{}", run.stderr);
    assert!(
        run.stderr.contains("no scope is declared"),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains(s.config.to_str().unwrap()),
        "{}",
        run.stderr
    );
    assert!(!s.root.exists());
}

#[test]
fn no_config_path_names_the_literal_one() {
    let dir = TempDir::new("new-scope-no-path");
    let root = dir.path().join("store");
    let run = new(&dir, &root, &["plan", "release", "--scope", "work"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("$HOME/.config/bilbo/config"),
        "{}",
        run.stderr
    );
    assert!(!root.exists());
}

#[test]
fn scope_given_twice_or_without_a_value_is_refused() {
    let dir = TempDir::new("new-scope-twice");
    let s = scoped(&dir, &["scope.work.sync = off"]);
    for args in [
        &[
            "new", "plan", "release", "--scope", "work", "--scope", "work",
        ][..],
        &["new", "plan", "release", "--scope", "work", "--scope=work"],
        &["new", "plan", "release", "--scope"],
        &["new", "plan", "release", "--scope="],
        &["new", "plan", "release", "--scope", ""],
    ] {
        let run = bilbo_scoped(&s, &s.home, args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stderr.contains("--scope"), "{}", run.stderr);
        assert!(!s.root.exists());
    }
}

#[test]
fn a_broken_config_stops_new() {
    let dir = TempDir::new("new-scope-broken");
    let s = scoped(&dir, &["scope.work.embedder = remote"]);
    let run = bilbo_scoped(&s, &s.home, &["new", "plan", "release"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("scope.work.embedder"), "{}", run.stderr);
    assert!(!s.root.exists());
}

#[test]
fn scoped_notes_pass_check_and_unassigned_ones_fail_by_design() {
    let dir = TempDir::new("new-scope-check");
    let s = scoped(&dir, &["scope.work.paths = ~/acme"]);
    let inside = folder(&s, "acme");
    let outside = folder(&s, "elsewhere");
    assert_eq!(
        bilbo_scoped(&s, &inside, &["new", "plan", "release"]).code,
        0
    );
    assert_eq!(
        bilbo_scoped(&s, &outside, &["new", "plan", "other", "--scope", "work"]).code,
        0
    );
    let run = bilbo_scoped(&s, &outside, &["check"]);
    assert_eq!((run.code, run.stdout.as_str()), (0, ""), "{}", run.stdout);

    assert_eq!(
        bilbo_scoped(&s, &outside, &["new", "plan", "loose"]).code,
        0
    );
    let run = bilbo_scoped(&s, &outside, &["check"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/plan-loose.md: scope: missing; scopes: work\n"
    );
}
