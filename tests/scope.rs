//! `bilbo scope` through the built binary. The writes that race `scope set`, and a filesystem that cannot swap, are
//! unit tests of `note::scope`, which can run code between its two exchanges.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    IDS, Seed, TempDir, Watcher, bilbo, config, days_ago, note_text, poll_eq, seed, snapshot,
    store, write,
};

fn cwd() -> PathBuf {
    std::env::temp_dir()
}

fn scope(root: &Path, config: &Path, args: &[&str]) -> common::Run {
    let mut all = vec!["scope"];
    all.extend(args);
    bilbo(
        &cwd(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", config.to_str().unwrap()),
            ("HOME", "/home/tester"),
        ],
        &all,
    )
}

fn scoped(id: &str, key: &str) -> String {
    match key.strip_prefix("scope: ") {
        Some(value) if !value.contains('\n') => common::in_scope(&note_text(id, "T"), value),
        _ => note_text(id, "T").replacen("---\n\n", &format!("{key}\n---\n\n"), 1),
    }
}

fn path(root: &Path, name: &str) -> String {
    root.join("notes").join(name).display().to_string()
}

fn text(root: &Path, name: &str) -> String {
    fs::read_to_string(root.join("notes").join(name)).unwrap()
}

#[test]
fn lists_two_scopes_and_the_unassigned_notes() {
    let dir = TempDir::new("scope-list");
    let root = store(&dir);
    let conf = config(
        &dir,
        &[
            "scope.personal.sync = off",
            "scope.work.embedder = local",
            "scope.work.paths = ~/Developer/acme",
            "scope.default = personal",
        ],
    );
    for (i, key) in [
        "scope: personal",
        "scope: personal",
        "scope: personal",
        "scope: work",
        "",
        "scope: acme",
    ]
    .iter()
    .enumerate()
    {
        let id = format!("01M3YJ7R6HK6NQ30DCDB1P4D0{i}");
        let body = if key.is_empty() {
            note_text(&id, "T")
        } else {
            scoped(&id, key)
        };
        write(&root, &format!("plan-n{i}.md"), &body);
    }
    let run = scope(&root, &conf, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "personal\t3 notes\tsync off\tembedder any\tpaths -\tdefault\n\
         work\t1 note\tsync off\tembedder local\tpaths ~/Developer/acme\n\
         (unassigned)\t2 notes\tembedder local\n"
    );
    assert!(run.stderr.is_empty());
}

/// The store and config of `lists_two_scopes_and_the_unassigned_notes`, shortened.
fn two_scopes(dir: &TempDir) -> (PathBuf, PathBuf) {
    let root = store(dir);
    let conf = config(
        dir,
        &[
            "scope.personal.sync = off",
            "scope.work.embedder = local",
            "scope.work.paths = ~/Developer/acme",
            "scope.default = personal",
        ],
    );
    write(&root, "plan-n0.md", &scoped(IDS[0], "scope: personal"));
    write(&root, "plan-n1.md", &scoped(IDS[1], "scope: work"));
    write(&root, "plan-n2.md", &note_text(IDS[2], "T"));
    (root, conf)
}

#[test]
fn the_listing_is_a_table_on_a_terminal() {
    let dir = TempDir::new("scope-tty");
    let (root, conf) = two_scopes(&dir);
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", conf.to_str().unwrap()),
        ("HOME", "/home/tester"),
        ("TERM", "xterm-256color"),
        ("NO_COLOR", "1"),
        ("LANG", "C.UTF-8"),
    ];
    let run = common::bilbo_tty(&cwd(), &env, &["scope"], 100);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert!(lines[0].starts_with("SCOPE"), "{}", run.stdout);
    assert_eq!(lines.iter().filter(|l| l.contains("(default)")).count(), 1);
    assert!(lines[1].starts_with("personal (default)"), "{}", run.stdout);
    assert!(lines.last().unwrap().starts_with("unassigned"));
    assert!(!run.stdout.contains('\t') && !run.stdout.contains('\x1b'));
}

#[test]
fn forced_colour_never_reaches_a_pipe() {
    let dir = TempDir::new("scope-forced");
    let (root, conf) = two_scopes(&dir);
    let run = bilbo(
        &cwd(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", conf.to_str().unwrap()),
            ("HOME", "/home/tester"),
            ("CLICOLOR_FORCE", "1"),
            ("FORCE_COLOR", "1"),
        ],
        &["scope"],
    );
    assert_eq!(
        run.stdout,
        "personal\t1 note\tsync off\tembedder any\tpaths -\tdefault\n\
         work\t1 note\tsync off\tembedder local\tpaths ~/Developer/acme\n\
         (unassigned)\t1 note\tembedder local\n"
    );
}

#[test]
fn sync_takes_a_url() {
    let dir = TempDir::new("scope-sync-url");
    let root = store(&dir);
    let conf = config(&dir, &["scope.personal.sync = https://relay.example.net"]);
    let run = scope(&root, &conf, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .contains("personal\t0 notes\tsync https://relay.example.net\tembedder any"),
        "{}",
        run.stdout
    );
    assert!(run.stderr.is_empty());
}

#[test]
fn a_sync_password_is_not_echoed() {
    let dir = TempDir::new("scope-sync-secret");
    let root = store(&dir);
    let conf = config(
        &dir,
        &["scope.personal.sync = https://u:sekrit@relay.example.net"],
    );
    let run = scope(&root, &conf, &[]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("scope.personal.sync"), "{}", run.stderr);
    assert!(!run.stderr.contains("sekrit"), "{}", run.stderr);
}

#[test]
fn says_so_when_no_scope_is_declared() {
    let dir = TempDir::new("scope-none");
    let root = store(&dir);
    let conf = config(&dir, &["digest.enable = on"]);
    for i in 0..4 {
        let id = format!("01M3YJ7R6HK6NQ30DCDB1P4D0{i}");
        write(&root, &format!("plan-n{i}.md"), &note_text(&id, "T"));
    }
    let run = scope(&root, &conf, &[]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "(unassigned)\t4 notes\tembedder any\n");
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no scopes declared; add scope.<name>.* keys to {}\n",
            conf.display()
        )
    );
}

#[test]
fn a_missing_store_counts_as_empty_and_is_not_created() {
    let dir = TempDir::new("scope-no-store");
    let root = dir.path().join("store");
    let conf = config(&dir, &["scope.work.embedder = local"]);
    let run = scope(&root, &conf, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "work\t0 notes\tsync off\tembedder local\tpaths -\n(unassigned)\t0 notes\tembedder local\n"
    );
    assert!(!root.exists());
}

#[test]
fn an_extra_argument_is_a_usage_error() {
    let dir = TempDir::new("scope-extra");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    for (arg, message) in [
        ("work", "bilbo: unexpected argument 'work'\n"),
        ("--json", "bilbo: unknown option '--json'\n"),
    ] {
        let run = scope(&root, &conf, &[arg]);
        assert_eq!(run.code, 2);
        assert!(run.stdout.is_empty());
        assert!(run.stderr.starts_with(message), "{}", run.stderr);
    }
}

#[test]
fn a_broken_config_stops_the_listing() {
    let dir = TempDir::new("scope-broken");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = remote"]);
    let run = scope(&root, &conf, &[]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("scope.work.embedder"), "{}", run.stderr);
}

#[test]
fn listing_changes_nothing() {
    let dir = TempDir::new("scope-readonly");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &scoped(IDS[0], "scope: work"));
    let before = snapshot(&root);
    assert_eq!(scope(&root, &conf, &[]).code, 0);
    assert_eq!(snapshot(&root), before);
}

#[test]
fn fills_a_missing_key() {
    let dir = TempDir::new("scope-fill");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "gotcha-acme-deploy.md", &note_text(IDS[0], "T"));
    let run = scope(
        &root,
        &conf,
        &["set", "work", &path(&root, "gotcha-acme-deploy.md")],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "notes/gotcha-acme-deploy.md: set work\n");
    assert_eq!(
        text(&root, "gotcha-acme-deploy.md"),
        scoped(IDS[0], "scope: work")
    );
    assert!(run.stderr.is_empty());
}

#[test]
fn a_note_already_in_the_scope_keeps_its_bytes_and_time() {
    let dir = TempDir::new("scope-kept");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &scoped(IDS[0], "scope: work"));
    let before = snapshot(&root.join("notes"));
    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "notes/plan-a.md: kept work\n");
    assert_eq!(snapshot(&root.join("notes")), before);
}

#[test]
fn another_scope_is_kept_without_force_and_replaced_with_it() {
    let dir = TempDir::new("scope-force");
    let root = store(&dir);
    let conf = config(
        &dir,
        &["scope.work.embedder = local", "scope.personal.sync = off"],
    );
    write(&root, "plan-a.md", &scoped(IDS[0], "scope: personal"));
    let file = path(&root, "plan-a.md");
    let before = snapshot(&root.join("notes"));
    let run = scope(&root, &conf, &["set", "work", &file]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "notes/plan-a.md: kept personal; --force replaces it\n"
    );
    assert_eq!(snapshot(&root.join("notes")), before);

    let run = scope(&root, &conf, &["set", "--force", "work", &file]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "notes/plan-a.md: replaced personal with work\n");
    let lines: Vec<String> = text(&root, "plan-a.md").lines().map(String::from).collect();
    assert_eq!(lines[3], "scope: work");
    assert_eq!(
        text(&root, "plan-a.md"),
        scoped(IDS[0], "scope: work"),
        "every other line is unchanged"
    );
}

#[test]
fn force_mends_an_invalid_value() {
    let dir = TempDir::new("scope-mend");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &scoped(IDS[0], "scope: Work"));
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", conf.to_str().unwrap()),
    ];
    let before = bilbo(&cwd(), &env, &["check"]);
    assert!(
        before.stdout.contains("scope: invalid name 'Work'"),
        "{}",
        before.stdout
    );
    let run = scope(
        &root,
        &conf,
        &["set", "--force", "work", &path(&root, "plan-a.md")],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(text(&root, "plan-a.md"), scoped(IDS[0], "scope: work"));
    let check = bilbo(
        &cwd(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", conf.to_str().unwrap()),
        ],
        &["check"],
    );
    assert!(!check.stdout.contains("scope:"), "{}", check.stdout);
}

#[test]
fn bulk_triage_takes_two_passes() {
    let dir = TempDir::new("scope-bulk");
    let root = store(&dir);
    let conf = config(
        &dir,
        &["scope.work.embedder = local", "scope.personal.sync = off"],
    );
    write(&root, "gotcha-acme-one.md", &note_text(IDS[0], "T"));
    write(&root, "gotcha-acme-two.md", &note_text(IDS[1], "T"));
    write(&root, "plan-home.md", &note_text(IDS[2], "T"));
    let paths = |prefix: &str| -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join("notes"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(prefix))
            .collect();
        names.sort();
        names.iter().map(|n| path(&root, n)).collect()
    };
    let mut args = vec!["set", "work"];
    let acme = paths("gotcha-acme-");
    args.extend(acme.iter().map(String::as_str));
    assert_eq!(scope(&root, &conf, &args).code, 0);
    let mut args = vec!["set", "personal"];
    let all = paths("");
    args.extend(all.iter().map(String::as_str));
    assert_eq!(scope(&root, &conf, &args).code, 0);
    assert!(text(&root, "gotcha-acme-one.md").contains("scope: work\n"));
    assert!(text(&root, "gotcha-acme-two.md").contains("scope: work\n"));
    assert!(text(&root, "plan-home.md").contains("scope: personal\n"));
}

#[test]
fn one_bad_file_does_not_stop_the_others() {
    let dir = TempDir::new("scope-one-bad");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &note_text(IDS[0], "T"));
    write(&root, "plan-b.md", &note_text(IDS[1], "T"));
    let stray = dir.path().join("x.md");
    fs::write(&stray, "x").unwrap();
    let stray = stray.display().to_string();
    let run = scope(
        &root,
        &conf,
        &[
            "set",
            "work",
            &path(&root, "plan-a.md"),
            &stray,
            &path(&root, "plan-b.md"),
        ],
    );
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "notes/plan-a.md: set work\nnotes/plan-b.md: set work\n"
    );
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: {stray}: not a note in {}\n",
            root.join("notes").display()
        )
    );
    assert!(text(&root, "plan-a.md").contains("scope: work\n"));
    assert!(text(&root, "plan-b.md").contains("scope: work\n"));
}

#[test]
fn an_undeclared_scope_changes_nothing() {
    let dir = TempDir::new("scope-undeclared");
    let root = store(&dir);
    let conf = config(
        &dir,
        &["scope.work.embedder = local", "scope.personal.sync = off"],
    );
    write(&root, "plan-a.md", &note_text(IDS[0], "T"));
    let before = snapshot(&root);
    let run = scope(&root, &conf, &["set", "acme", &path(&root, "plan-a.md")]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("scope 'acme' is not declared in")
            && run.stderr.contains("declared scopes: personal, work"),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(&root), before);
}

#[test]
fn missing_arguments_are_a_usage_error() {
    let dir = TempDir::new("scope-missing");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    let before = snapshot(&root);
    for args in [&["set"][..], &["set", "work"], &["set", "--force", "work"]] {
        let run = scope(&root, &conf, args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr
                .starts_with("bilbo: scope set needs a scope name and at least one file\n"),
            "{}",
            run.stderr
        );
    }
    let run = scope(&root, &conf, &["set", "--frobnicate", "work", "x"]);
    assert_eq!(run.code, 2);
    assert_eq!(snapshot(&root), before);
}

#[test]
fn refuses_what_is_not_a_note_in_the_store() {
    let dir = TempDir::new("scope-refuse-path");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &note_text(IDS[0], "T"));
    fs::create_dir(root.join("notes/sub")).unwrap();
    write(&root, "sub/plan-b.md", &note_text(IDS[1], "T"));
    fs::write(root.join("notes/readme.md"), "x").unwrap();
    std::os::unix::fs::symlink(
        root.join("notes/plan-a.md"),
        root.join("notes/plan-link.md"),
    )
    .unwrap();
    let before = snapshot(&root);
    let notes = root.join("notes").display().to_string();
    for (arg, reason) in [
        (
            path(&root, "sub/plan-b.md"),
            format!("not a note in {notes}"),
        ),
        (path(&root, "sub"), "not a regular file".to_string()),
        (
            path(&root, "plan-link.md"),
            "not a regular file".to_string(),
        ),
        (
            path(&root, "readme.md"),
            "name: must be <kind>-<topic>.md".to_string(),
        ),
        (path(&root, "missing.md"), "cannot read".to_string()),
    ] {
        let run = scope(&root, &conf, &["set", "work", &arg]);
        assert_eq!(run.code, 1, "{arg}: {}", run.stderr);
        assert!(run.stdout.is_empty());
        assert!(
            run.stderr.starts_with(&format!("bilbo: {arg}: ")) && run.stderr.contains(&reason),
            "{arg}: {}",
            run.stderr
        );
    }
    assert_eq!(snapshot(&root), before);
}

#[test]
fn refuses_broken_frontmatter_and_two_scope_lines() {
    let dir = TempDir::new("scope-refuse-file");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", "# Title\n");
    let two = scoped(IDS[1], "scope: work\nscope: personal");
    write(&root, "plan-b.md", &two);
    write(
        &root,
        "plan-c.md",
        "---\ncreated: 2026-10-02T14:23-03:00\n---\n\n# T\n",
    );
    let before = snapshot(&root.join("notes"));
    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .starts_with("bilbo: notes/plan-a.md: frontmatter"),
        "{}",
        run.stderr
    );
    let run = scope(
        &root,
        &conf,
        &["set", "--force", "work", &path(&root, "plan-b.md")],
    );
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.starts_with("bilbo: notes/plan-b.md: scope"),
        "{}",
        run.stderr
    );
    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-c.md")]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.starts_with("bilbo: notes/plan-c.md: id"),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(&root.join("notes")), before);
}

#[test]
fn a_parked_file_of_an_unwatched_note_blocks_the_next_set() {
    let dir = TempDir::new("scope-parked");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &note_text(IDS[0], "T"));
    let hidden = format!(".bilbo-restore-{}", IDS[0]);
    write(&root, &hidden, "parked");
    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("left in place"), "{}", run.stderr);
    assert!(
        run.stderr
            .contains(&format!("notes/{hidden} is left from an earlier run"))
            && run.stderr.contains("bilbo watch records it"),
        "{}",
        run.stderr
    );
    assert_eq!(text(&root, "plan-a.md"), note_text(IDS[0], "T"));
    assert_eq!(text(&root, &hidden), "parked");
}

#[test]
fn the_change_is_in_history() {
    let dir = TempDir::new("scope-history");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &note_text(IDS[0], "T"));
    let _watcher = Watcher::on(&root);
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", conf.to_str().unwrap()),
    ];
    let events = || -> Vec<String> {
        bilbo(&cwd(), &env, &["history", "a"])
            .stdout
            .lines()
            .map(|line| line.split(' ').nth(2).unwrap_or_default().to_string())
            .collect()
    };
    poll_eq("the note recorded", events, vec!["added".to_string()]);
    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    poll_eq(
        "the edit recorded",
        events,
        vec!["edited".to_string(), "added".to_string()],
    );
    let newest = bilbo(&cwd(), &env, &["history", "a"]);
    let short = newest.stdout.split(' ').next().unwrap().to_string();
    let shown = bilbo(&cwd(), &env, &["history", "a", &short]);
    assert!(shown.stdout.contains("scope: work\n"), "{}", shown.stdout);
}

#[test]
fn a_failed_lock_keeps_the_earlier_refusals_and_exits_1() {
    let dir = TempDir::new("scope-lock");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    write(&root, "plan-a.md", &note_text(IDS[0], "T"));
    fs::write(root.join(".bilbo"), "not a folder").unwrap();
    let stray = dir.path().join("x.md");
    fs::write(&stray, "x").unwrap();
    let stray = stray.display().to_string();
    let run = scope(
        &root,
        &conf,
        &["set", "work", &stray, &path(&root, "plan-a.md")],
    );
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with(&format!("bilbo: {stray}: not a note in")),
        "{}",
        run.stderr
    );
    assert!(run.stderr.lines().count() >= 2, "{}", run.stderr);
    assert_eq!(text(&root, "plan-a.md"), note_text(IDS[0], "T"));
}

#[test]
fn a_missing_config_file_stops_the_verb() {
    let dir = TempDir::new("scope-missing-config");
    let root = store(&dir);
    let missing = dir.path().join("no-such-config");
    let run = scope(&root, &missing, &[]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with(&format!("bilbo: cannot read {}: ", missing.display())),
        "{}",
        run.stderr
    );
}

fn sync_dir(root: &Path) -> PathBuf {
    let dir = root.join(".bilbo/sync");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn history(root: &Path, conf: &Path, args: &[&str]) -> common::Run {
    let mut all = vec!["history"];
    all.extend(args);
    bilbo(
        &cwd(),
        &[
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", conf.to_str().unwrap()),
        ],
        &all,
    )
}

#[test]
fn the_version_it_wrote_is_recorded_without_a_watcher() {
    let dir = TempDir::new("scope-recorded");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    let before = note_text(IDS[0], "T");
    write(&root, "plan-a.md", &before);
    seed(
        &root,
        IDS[0],
        &[Seed::new("plan-a.md", Some(&before), "added", &days_ago(1))],
    );
    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let listed = history(&root, &conf, &["a"]).stdout;
    let events: Vec<&str> = listed
        .lines()
        .map(|l| l.split(' ').nth(2).unwrap())
        .collect();
    assert_eq!(events, ["edited", "added"]);
    let short = listed.split(' ').next().unwrap();
    let shown = history(&root, &conf, &["a", short]).stdout;
    assert_eq!(shown, text(&root, "plan-a.md"));
}

#[test]
fn a_stale_base_entry_keeps_its_base_and_takes_the_version_it_wrote() {
    let dir = TempDir::new("scope-entry");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    let one = note_text(IDS[0], "T");
    let two = format!("{one}\nsync wrote this\n");
    write(&root, "plan-a.md", &two);
    let ids = seed(
        &root,
        IDS[0],
        &[
            Seed::new("plan-a.md", Some(&one), "added", &days_ago(2)),
            Seed::new("plan-a.md", Some(&two), "edited", &days_ago(1)),
        ],
    );
    let entry = serde_json::json!({ IDS[0]: { "base": ids[0], "written": ids[1] } });
    fs::write(sync_dir(&root).join("stale-base.json"), entry.to_string()).unwrap();

    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    let listed = history(&root, &conf, &["a"]).stdout;
    let newest = listed.split(' ').next().unwrap();
    assert_eq!(listed.lines().count(), 3, "{listed}");
    let bytes = fs::read(root.join(".bilbo/sync/stale-base.json")).unwrap();
    let kept = &serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()[IDS[0]];
    assert_eq!(kept["base"], ids[0]);
    assert!(kept["written"].as_str().unwrap().starts_with(newest));
}

#[test]
fn a_leftover_holding_a_staged_version_stays_for_the_watcher() {
    let dir = TempDir::new("scope-staged");
    let root = store(&dir);
    let conf = config(&dir, &["scope.work.embedder = local"]);
    let before = note_text(IDS[0], "T");
    write(&root, "plan-a.md", &before);
    seed(
        &root,
        IDS[0],
        &[Seed::new("plan-a.md", Some(&before), "added", &days_ago(1))],
    );
    let inbound = format!("{before}\ninbound\n");
    let hidden = format!(".bilbo-restore-{}", IDS[0]);
    write(&root, &hidden, &inbound);
    let line = serde_json::json!({
        "seen": "2026-10-04T12:00:00-03:00",
        "scope": "work",
        "record": {
            "note": IDS[0],
            "version": "f".repeat(64),
            "parents": [],
            "file": "plan-a.md",
            "blob": common::sha256_hex(inbound.as_bytes()),
            "event": "edited",
            "at": "2026-10-04T12:00:00-03:00",
        },
    });
    fs::write(sync_dir(&root).join("inbox.jsonl"), format!("{line}\n")).unwrap();

    let run = scope(&root, &conf, &["set", "work", &path(&root, "plan-a.md")]);

    assert_eq!(run.code, 1);
    assert!(!run.stderr.contains("recorded"), "{}", run.stderr);
    assert_eq!(text(&root, &hidden), inbound);
    assert_eq!(text(&root, "plan-a.md"), before);
    assert_eq!(history(&root, &conf, &["a"]).stdout.lines().count(), 1);
}
