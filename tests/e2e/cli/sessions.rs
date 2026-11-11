use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::scratch::Scratch;
use super::{Run, binary};

const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const OTHER: &str = "9b1d2c3e-4f5a-4b6c-8d7e-0f1a2b3c4d5e";
const ROLLOUT: &str = "rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl";
const CLAUDE: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
);
const SUBAGENT: &[u8] =
    include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.jsonl");
const META: &[u8] =
    include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.meta.json");
const CODEX: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
);
const CLAUDE_STOP: &str =
    include_str!("../../../apps/cli/tests/fixtures/hook/claude-code-stop.json");
const CODEX_STOP: &str = include_str!("../../../apps/cli/tests/fixtures/hook/codex-stop.json");

fn counts(new: u64, appended: u64, generation: u64, unchanged: u64, refused: u64) -> String {
    format!(
        "new {new}, appended {appended}, new generation {generation}, unchanged {unchanged}, refused {refused}"
    )
}

/// Every run of the binary gets this HOME, never the real one.
fn harness(test: &str) -> Scratch {
    let scratch = Scratch::new(test);
    let project = scratch
        .dir
        .join("home/.claude/projects/-home-me-src-garden");
    for (path, bytes) in [
        (project.join(format!("{MAIN}.jsonl")), CLAUDE),
        (
            project.join(format!("{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl")),
            SUBAGENT,
        ),
        (
            project.join(format!(
                "{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.meta.json"
            )),
            META,
        ),
        (project.join("memory/MEMORY.md"), b"- a memory\n"),
        (project.join(format!("memory/{OTHER}.jsonl")), CLAUDE),
        (
            project.join(format!("{MAIN}/tool-results/{OTHER}.jsonl")),
            CLAUDE,
        ),
        (
            project.join(format!("{MAIN}/workflows/{OTHER}.jsonl")),
            CLAUDE,
        ),
        (
            project.join(format!("{MAIN}/subagents/workflows/wf_1/agent-b2.jsonl")),
            SUBAGENT,
        ),
        (
            scratch
                .dir
                .join("home/.codex/sessions/2026/11/06")
                .join(ROLLOUT),
            CODEX,
        ),
    ] {
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(path, bytes).expect("a fixture");
    }
    scratch
}

fn main_session(scratch: &Scratch) -> PathBuf {
    scratch.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{MAIN}.jsonl"
    ))
}

fn with_stdin(scratch: &Scratch, args: &[&str], stdin: &[u8]) -> Run {
    let env = scratch.env();
    let mut child = Command::new(binary())
        .env_clear()
        .envs(env.iter().map(|(name, value)| (*name, value.as_str())))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bilbo starts");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("bilbo exits");
    Run {
        code: output.status.code().expect("an exit code"),
        stdout: String::from_utf8(output.stdout).expect("UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8"),
    }
}

fn first(run: &Run) -> &str {
    run.stdout.lines().next().unwrap_or_default()
}

#[test]
fn a_projects_folder_and_a_sessions_folder_are_kept_and_a_second_run_keeps_nothing() {
    let scratch = harness("sessions_twice");
    let projects = scratch.path("home/.claude/projects");
    let sessions = scratch.path("home/.codex/sessions");
    let run = scratch.run(&["remember", &projects, "--kind", "session"]);
    assert_eq!(
        (run.code, first(&run)),
        (0, counts(3, 0, 0, 0, 0).as_str()),
        "{}",
        run.stderr
    );
    let run = scratch.run(&["remember", &sessions]);
    assert_eq!(
        (run.code, first(&run)),
        (0, counts(1, 0, 0, 0, 0).as_str()),
        "{}",
        run.stderr
    );
    let run = scratch.run(&["remember", &projects, "--kind", "session"]);
    assert_eq!(first(&run), counts(0, 0, 0, 3, 0));
    let run = scratch.run(&["remember", &sessions]);
    assert_eq!(first(&run), counts(0, 0, 0, 1, 0));
    assert_eq!(
        scratch.run(&["remember", &projects]).code,
        2,
        "two kinds claim"
    );
}

#[test]
fn a_grown_session_appends_only_its_new_lines() {
    let scratch = harness("sessions_grown");
    let main = main_session(&scratch);
    let path = main.to_str().expect("UTF-8");
    assert_eq!(scratch.run(&["remember", path]).code, 0);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&main)
        .expect("open");
    file.write_all(b"{\"type\":\"user\",\"sessionId\":\"s\"}\n")
        .expect("grow");
    let run = scratch.run(&["remember", path]);
    assert_eq!(first(&run), counts(0, 1, 0, 0, 0));
}

#[test]
fn a_bare_run_with_no_follow_declared_runs_the_two_default_follows() {
    let scratch = harness("bare_defaults");
    let run = scratch.run(&["remember"]);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    assert_eq!(first(&run), counts(4, 0, 0, 0, 0));
    let run = scratch.run(&["remember"]);
    assert_eq!(first(&run), counts(0, 0, 0, 4, 0));
}

#[test]
fn a_bare_run_with_declared_follows_runs_those_instead() {
    let scratch = harness("bare_declared");
    scratch.write("a.md", "a\n");
    let config = format!(
        "[[follow]]\npath = \"{}\"\n\n[[follow]]\npath = \"~/gone\"\nkind = \"session\"\n",
        scratch.path("notes")
    );
    scratch.config(&config);
    let run = scratch.run(&["remember"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(first(&run), counts(1, 0, 0, 0, 0));
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: skipped {}: no such folder\n",
            scratch.path("home/gone")
        )
    );
}

#[test]
fn follow_saves_the_target_in_the_config_file_after_the_defaults() {
    let scratch = harness("follow_saves");
    scratch.write("a.md", "a\n");
    let config = scratch.path("config/config.toml");
    fs::write(&config, "# mine\n[store]\ncompression = 3").expect("a config");
    let notes = scratch.path("notes");
    let run = scratch.run(&["remember", &notes, "--follow"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(first(&run), counts(1, 0, 0, 0, 0));
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: saved the follow in {config}, after the default follows it would replace\n"
        )
    );
    let home = scratch.path("home");
    assert_eq!(
        fs::read_to_string(&config).expect("the config"),
        format!(
            "# mine\n[store]\ncompression = 3\n\n\
             [[follow]]\npath = \"{home}/.claude/projects\"\nkind = \"session\"\n\n\
             [[follow]]\npath = \"{home}/.codex/sessions\"\nkind = \"session\"\n\n\
             [[follow]]\npath = \"{notes}\"\nkind = \"markdown\"\n"
        )
    );
    let again = scratch.run(&["remember", &notes, "--follow"]);
    assert_eq!(
        again.stderr,
        format!("bilbo: {notes} is followed already\n")
    );
    scratch.write("b.md", "b\n");
    let bare = scratch.run(&["remember"]);
    assert_eq!(first(&bare), counts(5, 0, 0, 1, 0));
}

#[test]
fn follow_never_replaces_a_linked_config_file_and_prints_the_lines_to_add() {
    let scratch = harness("follow_linked");
    scratch.write("a.md", "a\n");
    let config = scratch.path("config/config.toml");
    let managed = scratch.dir.join("managed.toml");
    fs::write(
        &managed,
        "[[follow]]\npath = \"~/.claude/projects\"\nkind = \"session\"\n",
    )
    .expect("the managed file");
    fs::remove_file(&config).expect("remove");
    symlink(&managed, &config).expect("link");
    let notes = scratch.path("notes");
    let run = scratch.run(&["remember", &notes, "--follow"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        first(&run),
        counts(1, 0, 0, 0, 0),
        "the target is kept first"
    );
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: cannot save the follow in {config}: it is a symbolic link; add these lines to it:\n\
             bilbo: [[follow]]\n\
             bilbo: path = \"{notes}\"\n\
             bilbo: kind = \"markdown\"\n"
        )
    );
    assert!(
        fs::symlink_metadata(&config)
            .expect("link")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&managed).expect("managed"),
        "[[follow]]\npath = \"~/.claude/projects\"\nkind = \"session\"\n"
    );
}

#[test]
fn a_hook_with_each_recorded_payload_keeps_the_new_lines_and_seals_nothing() {
    let scratch = harness("hook_payloads");
    let home = scratch.path("home");
    for payload in [CLAUDE_STOP, CODEX_STOP] {
        let payload = payload.replace("/home/me", &home);
        let run = with_stdin(&scratch, &["remember", "--hook"], payload.as_bytes());
        assert_eq!(
            (run.code, run.stdout.as_str(), run.stderr.as_str()),
            (0, "", "")
        );
        scratch.backdate(61);
    }
    assert_eq!(scratch.segments(), 0, "the hook never seals, even when due");
    let main = main_session(&scratch);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&main)
        .expect("open");
    file.write_all(b"{\"type\":\"user\",\"sessionId\":\"s\"}\n")
        .expect("grow");
    let payload = CLAUDE_STOP.replace("/home/me", &home);
    assert_eq!(
        with_stdin(&scratch, &["remember", "--hook"], payload.as_bytes()).code,
        0
    );
    let run = scratch.run(&["remember", main.to_str().expect("UTF-8")]);
    assert_eq!(
        first(&run),
        counts(0, 0, 0, 1, 0),
        "the hook kept the new line"
    );
    assert_eq!(scratch.segments(), 1);
}

/// The lines of the payload the index's last commit recorded, read from its `meta.json`.
fn payload(scratch: &Scratch) -> (Vec<u8>, Vec<String>) {
    let bytes = fs::read(scratch.dir.join("cache/bilbo/index/meta.json")).expect("meta.json");
    let text = String::from_utf8(bytes.clone()).expect("UTF-8");
    let key = text.find("\"payload\"").expect("a payload");
    let at = key + text[key..].find(":").expect("a colon");
    let at = at + text[at..].find('"').expect("the payload's quote") + 1;
    let end = at + text[at..].find('"').expect("the end of the payload");
    let lines = text[at..end]
        .split("\\n")
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    (bytes, lines)
}

fn seq(lines: &[String]) -> u64 {
    lines
        .iter()
        .find_map(|line| line.strip_prefix("seq "))
        .expect("a seq line")
        .parse()
        .expect("a number")
}

#[test]
fn the_index_follows_a_remember_and_a_hook_turn_waits_for_the_next_one() {
    let scratch = harness("index_follows");
    let bare = scratch.run(&["remember"]);
    assert_eq!(bare.code, 0, "{}", bare.stderr);
    let (made, lines) = payload(&scratch);
    assert_eq!(lines.last().map(String::as_str), Some("failed -"));

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(main_session(&scratch))
        .expect("open");
    file.write_all(b"{\"type\":\"user\",\"sessionId\":\"s\"}\n")
        .expect("grow");
    let payload_in = CLAUDE_STOP.replace("/home/me", &scratch.path("home"));
    let hook = with_stdin(&scratch, &["remember", "--hook"], payload_in.as_bytes());
    assert_eq!(
        (hook.code, hook.stdout.as_str(), hook.stderr.as_str()),
        (0, "", "")
    );
    assert_eq!(
        payload(&scratch).0,
        made,
        "the hook turn left the index as it was"
    );

    let bare = scratch.run(&["remember"]);
    assert_eq!(bare.code, 0, "{}", bare.stderr);
    assert_eq!(bare.stderr, "");
    let (_, grown) = payload(&scratch);
    assert!(seq(&grown) > seq(&lines), "{grown:?}");
    assert_eq!(grown[..6], lines[..6], "the manifest stays");
    assert_eq!(grown.last().map(String::as_str), Some("failed -"));

    let reindexed = scratch.run(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{}", reindexed.stderr);
    let (_, rebuilt) = payload(&scratch);
    assert_eq!(rebuilt[..6], lines[..6]);
    assert_eq!(rebuilt.last().map(String::as_str), Some("failed -"));
}

#[test]
fn a_hook_exits_0_with_garbage_on_stdin() {
    let scratch = harness("hook_garbage");
    for input in [
        &b"garbage"[..],
        b"",
        b"{\"transcript_path\":\"/nowhere.jsonl\"}",
    ] {
        let run = with_stdin(&scratch, &["remember", "--hook"], input);
        assert_eq!((run.code, run.stdout.as_str()), (0, ""));
        assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    }
    assert!(!scratch.dir.join("store").exists());
}

#[test]
fn a_bare_run_or_a_hook_that_finds_the_store_held_exits_0_at_once_and_asks_for_one_more_run() {
    let scratch = harness("held");
    assert_eq!(scratch.run(&["remember"]).code, 0);
    let state = scratch.dir.join("state/bilbo");
    let lock = fs::File::open(state.join("store.lock")).expect("the lock file");
    lock.lock().expect("hold the store");
    let writer = fs::File::open(state.join("store.writer")).expect("the writer's lock file");
    writer.lock().expect("as a writer does");
    let run = scratch.run(&["remember"]);
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(
        run.stderr,
        "bilbo: another bilbo is writing the store; requested another pass over the follows\n"
    );
    assert_eq!(
        fs::metadata(state.join("store.again")).expect("flag").len(),
        1
    );
    fs::write(state.join("store.again"), b"").expect("clear the flag");
    let payload = CLAUDE_STOP.replace("/home/me", &scratch.path("home"));
    let run = with_stdin(&scratch, &["remember", "--hook"], payload.as_bytes());
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "", "")
    );
    assert_eq!(
        fs::metadata(state.join("store.again")).expect("flag").len(),
        1
    );
    writer.unlock().expect("release");
    lock.unlock().expect("release");
}

#[test]
fn follow_creates_a_config_file_that_is_missing() {
    let scratch = harness("follow_absent");
    scratch.write("a.md", "a\n");
    let notes = scratch.path("notes");
    let config = scratch.dir.join("home/.config/bilbo/config.toml");
    assert!(!config.exists());
    let run = scratch.run_without(&["remember", &notes, "--follow"], "BILBO_CONFIG");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let home = scratch.path("home");
    assert_eq!(
        fs::read_to_string(&config).expect("the new config"),
        format!(
            "[[follow]]\npath = \"{home}/.claude/projects\"\nkind = \"session\"\n\n\
             [[follow]]\npath = \"{home}/.codex/sessions\"\nkind = \"session\"\n\n\
             [[follow]]\npath = \"{notes}\"\nkind = \"markdown\"\n"
        )
    );
    assert!(
        scratch
            .dir
            .join("home/.config/bilbo/config.toml.lock")
            .exists()
    );
}

#[test]
fn follow_refuses_a_read_only_config_file_and_leaves_its_bytes() {
    let scratch = harness("follow_read_only");
    scratch.write("a.md", "a\n");
    let config = scratch.config("# mine\n");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o400)).expect("read-only");
    let notes = scratch.path("notes");
    let run = scratch.run(&["remember", &notes, "--follow"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        first(&run),
        counts(1, 0, 0, 0, 0),
        "the target is kept first"
    );
    assert!(
        run.stderr.starts_with(&format!(
            "bilbo: cannot save the follow in {config}: it is read-only; add these lines to it:\n"
        )),
        "{}",
        run.stderr
    );
    assert_eq!(fs::read_to_string(&config).expect("config"), "# mine\n");
    assert!(!scratch.dir.join("config/config.toml.lock").exists());
}

#[test]
fn follow_names_the_inline_table_when_a_read_only_or_linked_config_declares_follow_inline() {
    for (case, declared) in [
        ("empty", "follow = []\n"),
        (
            "populated",
            "# mine\nfollow = [{ path = \"/elsewhere\" }]\n\n[store]\ncompression = 3\n",
        ),
    ] {
        for linked in [false, true] {
            let scratch = harness(&format!("follow_inline_{case}_{linked}"));
            scratch.write("a.md", "a\n");
            let config = scratch.config(declared);
            let managed = scratch.dir.join("managed.toml");
            let (target, reason) = if linked {
                fs::write(&managed, declared).expect("the managed file");
                fs::remove_file(&config).expect("remove");
                symlink(&managed, &config).expect("link");
                (managed.clone(), "it is a symbolic link")
            } else {
                fs::set_permissions(&config, fs::Permissions::from_mode(0o400)).expect("read-only");
                (PathBuf::from(&config), "it is read-only")
            };
            let notes = scratch.path("notes");
            let run = scratch.run(&["remember", &notes, "--follow"]);
            assert_eq!(run.code, 2, "{case} linked={linked}: {}", run.stderr);
            assert_eq!(
                run.stderr,
                format!(
                    "bilbo: cannot save the follow in {config}: {reason}; add this table to its follow array:\n\
                     bilbo: {{ path = \"{notes}\", kind = \"markdown\" }}\n"
                ),
                "{case} linked={linked}"
            );
            assert_eq!(
                fs::read_to_string(&target).expect("the bytes"),
                declared,
                "{case} linked={linked}"
            );
            assert!(!scratch.dir.join("config/config.toml.lock").exists());
        }
    }
}

#[test]
fn follow_that_cannot_write_leaves_the_files_bytes_and_no_temporary_file() {
    let scratch = harness("follow_failed_write");
    scratch.write("a.md", "a\n");
    let config = scratch.config("# mine\n[store]\ncompression = 3");
    let folder = scratch.dir.join("config");
    fs::write(folder.join("config.toml.lock"), b"").expect("the lock a save leaves");
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).expect("no new file in it");
    let notes = scratch.path("notes");
    let run = scratch.run(&["remember", &notes, "--follow"]);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("restore");
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(
        run.stderr
            .contains(&format!("bilbo: cannot save the follow in {config}: ")),
        "{}",
        run.stderr
    );
    assert_eq!(
        fs::read_to_string(&config).expect("config"),
        "# mine\n[store]\ncompression = 3"
    );
    let mut names: Vec<String> = fs::read_dir(&folder)
        .expect("the folder")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["config.toml", "config.toml.lock"]);
}
