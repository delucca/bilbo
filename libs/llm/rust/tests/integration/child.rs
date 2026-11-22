use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::time::{Duration, Instant};

use bilbo_llm::child::{self, Bounds, End, Environment};

use crate::{Scratch, gone, signalable};

const SH: &str = "/bin/sh";
const POLL: Duration = Duration::from_millis(5);

fn go(
    scratch: &Scratch,
    script: &Path,
    arg: &str,
    stdin: &[u8],
    limit: usize,
    timeout: Duration,
) -> child::Run {
    child::run(
        Path::new(SH),
        &[script.to_str().expect("a text path"), arg],
        Environment::Inherit,
        scratch.path(),
        stdin,
        Bounds {
            limit,
            timeout,
            poll: POLL,
        },
    )
    .expect("a run")
}

fn pids(path: &Path) -> Vec<i32> {
    fs::read_to_string(path)
        .expect("the pid file")
        .lines()
        .map(|line| line.parse().expect("a pid"))
        .collect()
}

#[test]
fn a_timeout_stops_the_tool_and_what_it_started() {
    let scratch = Scratch::new("child");
    let script = scratch.script(
        "tool.sh",
        "read line; echo \"got $line\"; sleep 30 &\necho $! > \"$1\"\necho $$ >> \"$1\"\nsleep 30",
    );
    let pidfile = scratch.path().join("pids");
    let start = Instant::now();
    let run = go(
        &scratch,
        &script,
        pidfile.to_str().unwrap(),
        b"hello\n",
        1024,
        Duration::from_millis(600),
    );
    assert_eq!(run.end, End::TimedOut);
    assert_eq!(run.stdout, b"got hello\n");
    assert!(start.elapsed() < Duration::from_secs(3));
    let ids = pids(&pidfile);
    assert_eq!(ids.len(), 2);
    for &id in &ids {
        assert!(gone(id), "process {id} still lives");
    }
    assert!(!signalable(&format!("-{}", ids[1])));
}

#[test]
fn a_tool_that_exits_leaves_nothing_it_started_and_holds_no_pipe() {
    let scratch = Scratch::new("child");
    let script = scratch.script("tool.sh", "sleep 30 &\necho $! > \"$1\"\necho done");
    let pidfile = scratch.path().join("pids");
    let start = Instant::now();
    let run = go(
        &scratch,
        &script,
        pidfile.to_str().unwrap(),
        b"",
        1024,
        Duration::from_secs(10),
    );
    assert_eq!(run.end, End::Exited(0));
    assert_eq!(run.stdout, b"done\n");
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(gone(pids(&pidfile)[0]));
}

#[test]
fn output_over_the_limit_is_its_own_end_and_stops_the_tool() {
    let scratch = Scratch::new("child");
    let script = scratch.script(
        "loop.sh",
        "sleep 30 &\necho $! > \"$1\"\nwhile :; do echo xxxxxxxxxxxxxxxx; done",
    );
    let pidfile = scratch.path().join("pids");
    let run = go(
        &scratch,
        &script,
        pidfile.to_str().unwrap(),
        b"",
        100,
        Duration::from_secs(10),
    );
    assert_eq!(run.end, End::OverLimit);
    assert!(gone(pids(&pidfile)[0]));

    for (width, end) in [(100, End::Exited(0)), (101, End::OverLimit)] {
        let script = scratch.script("width.sh", &format!("printf '%0{width}d' 0"));
        let run = go(&scratch, &script, "", b"", 100, Duration::from_secs(10));
        assert_eq!(run.end, end, "{width} bytes");
    }
}

#[test]
fn a_run_looks_for_the_end_once_a_poll() {
    let scratch = Scratch::new("child");
    let script = scratch.script("tool.sh", "sleep 0.05");
    let poll = Duration::from_millis(400);
    let start = Instant::now();
    let run = child::run(
        Path::new(SH),
        &[script.to_str().unwrap()],
        Environment::Inherit,
        scratch.path(),
        b"",
        Bounds {
            limit: 1024,
            timeout: Duration::from_secs(10),
            poll,
        },
    )
    .unwrap();
    assert_eq!(run.end, End::Exited(0));
    assert!(start.elapsed() >= poll, "{:?}", start.elapsed());
}

#[test]
fn a_program_that_cannot_be_started_is_its_own_end() {
    let scratch = Scratch::new("child");
    let run = child::run(
        &scratch.path().join("missing"),
        &[],
        Environment::Inherit,
        scratch.path(),
        b"",
        Bounds {
            limit: 1024,
            timeout: Duration::from_secs(1),
            poll: POLL,
        },
    )
    .unwrap();
    assert!(
        matches!(&run.end, End::NotStarted(text) if text.contains("No such file")),
        "{:?}",
        run.end
    );
    assert!(run.stdout.is_empty());
}

#[test]
fn the_child_gets_only_the_variables_it_is_given_and_runs_in_its_folder() {
    let scratch = Scratch::new("child");
    let own = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .find(|name| {
            !["PATH", "PWD", "OLDPWD", "SHLVL", "GIVEN"].contains(&name.as_str())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        })
        .unwrap();
    let script = scratch.script(
        "env.sh",
        &format!("echo \"$PWD\"\necho \"$GIVEN\"\necho \"${{{own}-unset}}\""),
    );
    let run = child::run(
        Path::new(SH),
        &[script.to_str().unwrap()],
        Environment::Only(&[("GIVEN", "value")]),
        scratch.path(),
        b"",
        Bounds {
            limit: 1024,
            timeout: Duration::from_secs(10),
            poll: POLL,
        },
    )
    .unwrap();
    assert_eq!(run.end, End::Exited(0));
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        format!("{}\nvalue\nunset\n", scratch.path().display())
    );
}

#[test]
fn the_folder_is_made_private_and_what_stands_in_it_is_removed() {
    let scratch = Scratch::new("child");
    let made = scratch.path().join("made");
    child::folder(&made).unwrap();
    assert_eq!(
        fs::metadata(&made).unwrap().permissions().mode() & 0o777,
        0o700
    );

    let outside = scratch.path().join("outside.txt");
    fs::write(&outside, "keep").unwrap();
    fs::write(made.join("file"), "x").unwrap();
    symlink(&outside, made.join("link")).unwrap();
    fs::create_dir(made.join("empty")).unwrap();
    child::folder(&made).unwrap();
    assert_eq!(fs::read_dir(&made).unwrap().count(), 0);
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
}

#[test]
fn a_folder_that_is_a_link_is_refused_before_anything_is_removed() {
    let scratch = Scratch::new("child");
    let target = scratch.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file"), "keep").unwrap();
    let link = scratch.path().join("link");
    symlink(&target, &link).unwrap();
    assert_eq!(
        child::folder(&link).unwrap_err(),
        format!(
            "{} is a link: bilbo will not run a program there",
            link.display()
        )
    );
    assert_eq!(fs::read_to_string(target.join("file")).unwrap(), "keep");
}

#[test]
fn a_path_that_is_no_folder_is_refused() {
    let scratch = Scratch::new("child");
    let file = scratch.path().join("file");
    fs::write(&file, "keep").unwrap();
    assert_eq!(
        child::folder(&file).unwrap_err(),
        format!("{} is not a folder", file.display())
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep");
}

#[test]
fn a_folder_in_it_that_is_not_empty_is_refused() {
    let scratch = Scratch::new("child");
    let work = scratch.path().join("work");
    fs::create_dir_all(work.join("inner")).unwrap();
    fs::write(work.join("inner").join("file"), "keep").unwrap();
    fs::write(work.join("loose"), "x").unwrap();
    assert_eq!(
        child::folder(&work).unwrap_err(),
        format!("{} holds a folder that is not empty: inner", work.display())
    );
    assert_eq!(
        fs::read_to_string(work.join("inner").join("file")).unwrap(),
        "keep"
    );
    assert!(work.join("loose").exists());
}
