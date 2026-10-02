mod common;

use std::path::Path;
use std::process::{Command, Stdio};

use common::{Run, TempDir, bilbo, store, write};

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
