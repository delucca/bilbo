use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use super::Run;
use super::scratch::{CLAUDE, MAIN, SUBAGENT, Scratch};

const OTHER: &str = "9b1d2c3e-4f5a-4b6c-8d7e-0f1a2b3c4d5e";
const ROLLOUT: &str = "rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl";
const META: &[u8] =
    include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.meta.json");
const CODEX: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
);
const CODEX_PACKED: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl.zst"
);
const SECOND: &str = "rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl";
const SECOND_PACKED: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl.zst"
);
const ROLLOUT_ID: &str = "019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7";

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
            "bilbo: skipped {}: no such file or folder\n",
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

fn payload(scratch: &Scratch) -> Vec<String> {
    let bytes = fs::read(scratch.dir.join("cache/bilbo/index/meta.json")).expect("meta.json");
    let text = String::from_utf8(bytes).expect("UTF-8");
    let key = text.find("\"payload\"").expect("a payload");
    let at = key + text[key..].find(":").expect("a colon");
    let at = at + text[at..].find('"').expect("the payload's quote") + 1;
    let end = at + text[at..].find('"').expect("the end of the payload");
    text[at..end]
        .split("\\n")
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
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
fn the_index_follows_a_bare_remember_over_a_session_that_grew() {
    let scratch = harness("index_follows");
    let bare = scratch.run(&["remember"]);
    assert_eq!(bare.code, 0, "{}", bare.stderr);
    let lines = payload(&scratch);
    assert_eq!(lines.last().map(String::as_str), Some("failed -"));

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(main_session(&scratch))
        .expect("open");
    file.write_all(b"{\"type\":\"user\",\"sessionId\":\"s\"}\n")
        .expect("grow");
    let bare = scratch.run(&["remember"]);
    assert_eq!(bare.code, 0, "{}", bare.stderr);
    assert_eq!(bare.stderr, "");
    assert_eq!(first(&bare), counts(0, 1, 0, 3, 0));
    let grown = payload(&scratch);
    assert!(seq(&grown) > seq(&lines), "{grown:?}");
    assert_eq!(grown[..6], lines[..6], "the manifest stays");
    assert_eq!(grown.last().map(String::as_str), Some("failed -"));

    let reindexed = scratch.run(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{}", reindexed.stderr);
    let rebuilt = payload(&scratch);
    assert_eq!(rebuilt[..6], lines[..6]);
    assert_eq!(rebuilt.last().map(String::as_str), Some("failed -"));
}

#[test]
fn a_bare_run_that_finds_the_store_held_exits_0_at_once_asks_for_one_more_run_and_the_next_run_takes_it()
 {
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
    writer.unlock().expect("release");
    lock.unlock().expect("release");
    let run = scratch.run(&["remember"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stderr
            .lines()
            .any(|line| line.starts_with("bilbo: ran the follows again for another bilbo:")),
        "{}",
        run.stderr
    );
    assert_eq!(
        fs::metadata(state.join("store.again")).expect("flag").len(),
        0
    );
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

fn put(scratch: &Scratch, name: &str, bytes: &[u8]) {
    let path = scratch.dir.join("home/.codex/sessions").join(name);
    fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    fs::write(path, bytes).expect("a rollout");
}

#[test]
fn a_sessions_folder_with_plain_and_packed_rollouts_keeps_each_once() {
    let scratch = harness("packed_each_once");
    let sessions = scratch.path("home/.codex/sessions");
    fs::remove_dir_all(&sessions).expect("only the test's rollouts");
    put(&scratch, &format!("2026/11/06/{ROLLOUT}"), CODEX);
    put(&scratch, &format!("2026/11/07/{SECOND}.zst"), SECOND_PACKED);
    let run = scratch.run(&["remember", &sessions]);
    assert_eq!(
        (run.code, first(&run)),
        (0, counts(2, 0, 0, 0, 0).as_str()),
        "{}",
        run.stderr
    );
    let run = scratch.run(&["remember", &sessions]);
    assert_eq!((run.code, first(&run)), (0, counts(0, 0, 0, 2, 0).as_str()));
}

#[test]
fn a_packed_rollout_that_replaces_a_plain_one_stays_the_same_entry() {
    let scratch = harness("packed_replaces_plain");
    let sessions = scratch.path("home/.codex/sessions");
    let plain = scratch
        .dir
        .join("home/.codex/sessions/2026/11/06")
        .join(ROLLOUT);
    let run = scratch.run(&["remember", &sessions]);
    assert_eq!(first(&run), counts(1, 0, 0, 0, 0));
    fs::remove_file(&plain).expect("Codex removes the plain file");
    put(&scratch, &format!("2026/11/06/{ROLLOUT}.zst"), CODEX_PACKED);
    let run = scratch.run(&["remember", &sessions]);
    assert_eq!((run.code, first(&run)), (0, counts(0, 0, 0, 1, 0).as_str()));
    assert!(
        run.stdout
            .lines()
            .nth(1)
            .is_some_and(|line| line.starts_with("records 0,")),
        "{}",
        run.stdout
    );
    let read = scratch.run(&["read", ROLLOUT_ID]);
    assert_eq!(read.code, 0, "{}", read.stderr);
    let label = read
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("label: "))
        .expect("a label");
    assert!(label.ends_with(".jsonl (capture)"), "{label}");
    assert!(
        read.stdout.lines().any(|line| line == "generation: 1"),
        "{}",
        read.stdout
    );
}
