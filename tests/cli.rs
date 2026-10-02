mod common;

use common::{TempDir, bilbo};

const USAGE: &str = "\
usage: bilbo new <kind> <topic> [--title <text>]
       bilbo check
       bilbo recall <query>... [--kind <kind>]... [--limit <n>]
       bilbo --help
new creates <root>/notes/<kind>-<topic>.md and prints its path.
check prints every problem in the store and changes nothing.
recall prints the notes that best match the query, best first, 10 unless --limit says otherwise.
kinds: plan, spec, design, decision, gotcha, research, review, report, reference
root: $BILBO_HOME, else $XDG_DATA_HOME/bilbo, else $HOME/.local/share/bilbo
";

fn prefixed_usage() -> String {
    USAGE.lines().map(|l| format!("bilbo: {l}\n")).collect()
}

#[test]
fn no_arguments_is_usage_error() {
    let dir = TempDir::new("cli-none");
    let run = bilbo(dir.path(), &[], &[]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: missing verb\n{}", prefixed_usage())
    );
}

#[test]
fn unknown_verb_is_usage_error() {
    let dir = TempDir::new("cli-verb");
    let home = dir.path().join("home");
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", home.to_str().unwrap())],
        &["frobnicate"],
    );
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unknown verb 'frobnicate'\n"));
    assert!(
        run.stderr.contains("bilbo new")
            && run.stderr.contains("bilbo check")
            && run.stderr.contains("bilbo recall")
    );
    assert!(!home.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn help_goes_to_stdout() {
    let dir = TempDir::new("cli-help");
    for flag in ["--help", "-h"] {
        let run = bilbo(dir.path(), &[], &[flag]);
        assert_eq!(run.code, 0);
        assert_eq!(run.stdout, USAGE);
        assert!(run.stderr.is_empty());
    }
}

#[test]
fn help_after_a_verb_prints_help() {
    let dir = TempDir::new("cli-help-verb");
    for args in [["check", "--help"], ["new", "-h"], ["recall", "--help"]] {
        let run = bilbo(dir.path(), &[], &args);
        assert_eq!(run.code, 0);
        assert_eq!(run.stdout, USAGE);
        assert!(run.stderr.is_empty());
    }
}

#[test]
fn unknown_top_level_option_is_usage_error() {
    let dir = TempDir::new("cli-option");
    let run = bilbo(dir.path(), &[], &["--verbose"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with("bilbo: unknown option '--verbose'\n")
    );
}

fn home(dir: &TempDir) -> String {
    dir.path().join("store").to_str().unwrap().to_string()
}

#[test]
fn check_verb_runs() {
    let dir = TempDir::new("cli-check");
    let home = home(&dir);
    std::fs::create_dir_all(dir.path().join("store/notes")).unwrap();
    let run = bilbo(dir.path(), &[("BILBO_HOME", &home)], &["check"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.is_empty());
}

#[test]
fn no_recall_hits_exits_1() {
    let dir = TempDir::new("cli-recall-empty");
    let home = home(&dir);
    std::fs::create_dir_all(dir.path().join("store/notes")).unwrap();
    let run = bilbo(dir.path(), &[("BILBO_HOME", &home)], &["recall", "wumpus"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr, "bilbo: no notes match\n");
}

#[test]
fn unknown_option_is_not_help() {
    let dir = TempDir::new("cli-unknown-option");
    let run = bilbo(dir.path(), &[], &["check", "--verbose"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unknown option '--verbose'\n"));
}

#[test]
fn success_exits_0() {
    let dir = TempDir::new("cli-success");
    let home = home(&dir);
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", &home)],
        &["new", "plan", "release-steps"],
    );
    assert_eq!(run.code, 0);
}

#[test]
fn refusal_exits_1() {
    let dir = TempDir::new("cli-refusal");
    let home = home(&dir);
    let env = [("BILBO_HOME", home.as_str())];
    assert_eq!(
        bilbo(dir.path(), &env, &["new", "plan", "release-steps"]).code,
        0
    );
    assert_eq!(
        bilbo(dir.path(), &env, &["new", "plan", "release-steps"]).code,
        1
    );
}

#[test]
fn missing_topic_exits_2() {
    let dir = TempDir::new("cli-missing-topic");
    let run = bilbo(dir.path(), &[], &["new", "plan"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.starts_with("bilbo: missing <topic>\n"));
}

#[test]
fn failure_leaves_stdout_empty() {
    let dir = TempDir::new("cli-failure-stdout");
    let home = home(&dir);
    let env = [("BILBO_HOME", home.as_str())];
    bilbo(dir.path(), &env, &["new", "plan", "release"]);
    let run = bilbo(dir.path(), &env, &["new", "decision", "release"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with("bilbo: "));
}

#[test]
fn relative_bilbo_home_is_refused() {
    let dir = TempDir::new("cli-relative-home");
    for args in [
        &["new", "plan", "x"][..],
        &["check"][..],
        &["recall", "rollback"][..],
    ] {
        let run = bilbo(dir.path(), &[("BILBO_HOME", "store")], args);
        assert_eq!(run.code, 2);
        assert!(run.stdout.is_empty());
        assert_eq!(
            run.stderr,
            "bilbo: BILBO_HOME must be an absolute path, got 'store'\n"
        );
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn every_stderr_line_of_a_multi_line_argument_is_prefixed() {
    let dir = TempDir::new("cli-newline-arg");
    let run = bilbo(dir.path(), &[], &["foo\nbar"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr
            .starts_with("bilbo: unknown verb 'foo\nbilbo: bar'\n")
    );
}

#[test]
fn non_utf8_argument_is_usage_error() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new("cli-non-utf8");
    let arg = std::ffi::OsStr::from_bytes(b"\xff\xfe");
    let run = common::bilbo_os(dir.path(), &[], &[arg]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with("bilbo: arguments must be valid UTF-8\n")
    );
}
