use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod config;
mod read;
mod reindex;
mod remember;
mod scratch;
mod sessions;
mod tty;

pub(crate) struct Run {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[expect(
    clippy::disallowed_methods,
    reason = "a test finds the binary through its runfiles variable"
)]
pub(crate) fn binary() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_bilbo") {
        return PathBuf::from(path);
    }
    let runfiles = runfiles::Runfiles::create().expect("runfiles");
    runfiles
        .rlocation(std::env::var("BILBO").expect("BILBO names the binary"))
        .expect("the binary is in the runfiles")
}

pub(crate) fn bilbo<S: AsRef<OsStr>>(args: &[S], env: &[(&str, &str)]) -> Run {
    let output = Command::new(binary())
        .env_clear()
        .envs(env.iter().copied())
        .args(args)
        .output()
        .expect("bilbo runs");
    Run {
        code: output.status.code().expect("an exit code"),
        stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
    }
}

const OVERVIEW_START: &str = "bilbo keeps durable memory for coding agents: ";
const VERBS: &str = "; the verbs are remember, read, reindex, config";
const SEE: &str = "bilbo: see 'bilbo --help'\n";

#[test]
fn every_help_form_prints_the_overview_on_stdout_and_exits_0() {
    let reference = bilbo(&["--help"], &[]);
    assert_eq!(reference.code, 0);
    assert!(
        reference.stdout.starts_with(OVERVIEW_START),
        "{}",
        reference.stdout
    );
    assert!(
        reference
            .stdout
            .ends_with("\nDocs: https://github.com/delucca/bilbo/wiki\n")
    );
    assert_eq!(reference.stderr, "");
    for form in [&["-h"][..], &["help"], &["help", "help"]] {
        let run = bilbo(form, &[]);
        assert_eq!(run.code, 0, "{form:?}");
        assert_eq!(run.stdout, reference.stdout, "{form:?}");
        assert_eq!(run.stderr, "", "{form:?}");
    }
}

#[test]
fn version_prints_bilbo_and_the_build_version() {
    let run = bilbo(&["--version"], &[]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, format!("bilbo {}\n", env!("CARGO_PKG_VERSION")));
    assert_eq!(run.stderr, "");
}

#[test]
fn no_arguments_is_a_usage_error() {
    let run = bilbo::<&str>(&[], &[]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, format!("bilbo: missing verb{VERBS}\n{SEE}"));
}

#[test]
fn an_unknown_option_is_a_usage_error() {
    let run = bilbo(&["--frobnicate"], &[]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!("bilbo: unknown option '--frobnicate'\n{SEE}")
    );
}

#[test]
fn an_unknown_verb_is_a_usage_error_with_or_without_help() {
    for form in [&["frob"][..], &["help", "frob"], &["frob", "--help"]] {
        let run = bilbo(form, &[]);
        assert_eq!(run.code, 2, "{form:?}");
        assert_eq!(run.stdout, "", "{form:?}");
        assert_eq!(
            run.stderr,
            format!("bilbo: unknown verb 'frob'{VERBS}\n{SEE}"),
            "{form:?}"
        );
    }
}

#[test]
fn a_pipe_gets_no_escape_byte_whatever_the_environment_says() {
    let env = [
        ("TERM", "xterm-256color"),
        ("CLICOLOR_FORCE", "1"),
        ("FORCE_COLOR", "1"),
        ("LANG", "en_US.UTF-8"),
    ];
    let help = bilbo(&["--help"], &env);
    assert!(!help.stdout.contains('\x1b'), "{}", help.stdout);
    assert_eq!(help.stdout, bilbo(&["--help"], &[]).stdout);
    let error = bilbo(&["frob"], &env);
    assert!(!error.stderr.contains('\x1b'), "{}", error.stderr);
    assert!(
        error.stderr.lines().all(|line| line.starts_with("bilbo: ")),
        "{}",
        error.stderr
    );
}

#[test]
fn a_non_utf8_option_is_a_usage_error() {
    use std::os::unix::ffi::OsStrExt;
    let run = bilbo(&[OsStr::from_bytes(b"--\xff")], &[]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert!(
        run.stderr.starts_with("bilbo: unknown option '--"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.ends_with(&format!("'\n{SEE}")), "{}", run.stderr);
    assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
}

#[test]
fn help_to_a_reader_that_went_away_is_not_a_failure() {
    let mut child = Command::new(binary())
        .env_clear()
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bilbo starts");
    drop(child.stdout.take());
    let status = child.wait().expect("bilbo exits");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("reads stderr");
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn a_failed_stdout_write_exits_1_with_the_reason_on_stderr() {
    let readable_only = std::fs::File::open("/dev/null").expect("/dev/null opens");
    let output = Command::new(binary())
        .env_clear()
        .arg("--help")
        .stdout(Stdio::from(readable_only))
        .stderr(Stdio::piped())
        .output()
        .expect("bilbo runs");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.starts_with("bilbo: cannot write to stdout: "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
}
