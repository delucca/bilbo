mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Fake, Run, TempDir, bilbo, fakes, snapshot, unused_port};

struct Machine {
    dir: TempDir,
    home: PathBuf,
    bin: PathBuf,
    /// Where the fakes keep their state.
    state: PathBuf,
}

fn machine(name: &str) -> Machine {
    let dir = TempDir::new(name);
    let home = dir.path().join("home");
    let bin = dir.path().join("bin");
    let state = dir.path().join("state");
    for folder in [&home, &bin, &state] {
        std::fs::create_dir_all(folder).unwrap();
    }
    Machine {
        dir,
        home,
        bin,
        state,
    }
}

/// `watch_setup` with `--no-watch`, so a test of another step needs no service manager and sees
/// the watch line as skipped; `--remove` and `--interactive` take no such flag.
fn setup(m: &Machine, env: &[(&str, &str)], args: &[&str]) -> Run {
    let mut full = args.to_vec();
    if !args
        .iter()
        .any(|a| matches!(*a, "--remove" | "--interactive" | "--no-watch"))
    {
        full.push("--no-watch");
    }
    watch_setup(m, env, &full)
}

/// `bilbo setup <args>` with `HOME` and `PATH` of the machine, then `env`, a later name replacing an earlier one.
fn watch_setup(m: &Machine, env: &[(&str, &str)], args: &[&str]) -> Run {
    let mut vars = vec![
        ("HOME", m.home.to_str().unwrap()),
        ("PATH", m.bin.to_str().unwrap()),
    ];
    for (name, value) in env {
        vars.retain(|(n, _)| n != name);
        vars.push((name, value));
    }
    let mut full = vec!["setup"];
    full.extend(args);
    bilbo(&m.home, &vars, &full)
}

/// The report line of `step`.
fn step<'a>(run: &'a Run, step: &str) -> Option<&'a str> {
    run.stdout
        .lines()
        .find(|line| line.starts_with(&format!("{step} ")))
}

fn root(m: &Machine) -> PathBuf {
    m.home.join(".local/share/bilbo")
}

fn config_path(m: &Machine) -> PathBuf {
    m.home.join(".config/bilbo/config")
}

fn lines(run: &Run) -> Vec<&str> {
    run.stdout.lines().collect()
}

fn write_config(m: &Machine, lines: &[&str]) {
    let path = config_path(m);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
}

/// Runs `setup` with `args`, expects a usage error that starts with `message`, and that nothing on the machine changed.
fn usage_error(m: &Machine, args: &[&str], message: &str) -> Run {
    let before = snapshot(m.dir.path());
    let run = watch_setup(m, &[], args);
    assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
    assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
    assert!(
        run.stderr.starts_with(&format!("bilbo: {message}\n")),
        "{args:?}: {}",
        run.stderr
    );
    assert!(run.stderr.contains("bilbo: usage: bilbo new"));
    assert_eq!(snapshot(m.dir.path()), before, "{args:?} wrote something");
    run
}

fn embedder_args<'a>(fake: &'a Fake, model: &'a str) -> Vec<&'a str> {
    vec![
        "--yes",
        "--embedder-url",
        &fake.url,
        "--embedder-model",
        model,
        "--no-plugin",
        "--no-timer",
    ]
}

// ---------------------------------------------------------------------------
// Modes and flags

#[test]
fn modes_a_pipe_runs_without_the_wizard() {
    let m = machine("setup-modes-pipe");
    fakes::install(&m.bin, &m.state, &[TIMER_TOOL]);
    let run = watch_setup(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert_eq!(
        lines(&run),
        [
            format!("store created: {}/notes", root(&m).display()),
            format!("config written: {}", config_path(&m).display()),
            "key skipped: no embedder".to_string(),
            "model skipped: not local".to_string(),
            "server skipped: not local".to_string(),
            "embedder skipped: none configured".to_string(),
            "claude skipped: not found".to_string(),
            "codex skipped: not found".to_string(),
            "hook skipped: no codex plugin".to_string(),
            "timer skipped: no embedder".to_string(),
            format!("watch installed: watching {}/notes", root(&m).display()),
            "sync skipped: no scope syncs".to_string(),
        ]
    );
}

#[test]
fn modes_interactive_needs_a_terminal() {
    let m = machine("setup-modes-terminal");
    usage_error(
        &m,
        &["--interactive"],
        "the wizard needs a terminal on stdin and stderr; --interactive cannot run here",
    );
}

#[test]
fn modes_yes_and_interactive_conflict() {
    let m = machine("setup-modes-conflict");
    usage_error(
        &m,
        &["--yes", "--interactive"],
        "--yes and --interactive cannot be used together",
    );
}

#[test]
fn modes_interactive_with_an_answer_flag_conflicts() {
    let m = machine("setup-modes-answer");
    for flag in [
        ["--no-plugin"].as_slice(),
        &["--no-timer"],
        &["--no-watch"],
        &["--index-every", "5"],
        &["--embedder-url", "http://x:1", "--embedder-model", "m"],
    ] {
        let mut args = vec!["--interactive"];
        args.extend(flag);
        usage_error(
            &m,
            &args,
            &format!(
                "--interactive cannot be used with {}; it answers a wizard question",
                flag[0]
            ),
        );
    }
}

#[test]
fn flags_unknown_option_never_echoes_its_value() {
    let m = machine("setup-flags-unknown");
    for args in [["--token", "sk-123"].as_slice(), &["--token=sk-123"]] {
        let run = usage_error(&m, args, "unknown option '--token'");
        assert!(!run.stderr.contains("sk-123"));
    }
}

#[test]
fn flags_positional_is_not_echoed() {
    let m = machine("setup-flags-positional");
    let run = usage_error(
        &m,
        &["--yes", "sk-secret"],
        "setup takes only options; argument 2 is not one",
    );
    assert!(!run.stderr.contains("sk-secret"));
}

#[test]
fn flags_missing_value() {
    let m = machine("setup-flags-missing");
    usage_error(&m, &["--claude"], "--claude needs a value");
    usage_error(&m, &["--embedder-url="], "--embedder-url needs a value");
    usage_error(&m, &["--index-every", ""], "--index-every needs a value");
}

#[test]
fn flags_model_without_url() {
    let m = machine("setup-flags-model");
    usage_error(
        &m,
        &["--yes", "--embedder-model", "m"],
        "--embedder-model needs --embedder-url",
    );
    usage_error(
        &m,
        &["--yes", "--embedder-token-env", "KEY"],
        "--embedder-token-env needs --embedder-url",
    );
    usage_error(
        &m,
        &["--yes", "--embedder-query-prefix", ""],
        "--embedder-query-prefix needs --embedder-url",
    );
}

#[test]
fn flags_url_without_model() {
    let m = machine("setup-flags-url");
    usage_error(
        &m,
        &["--yes", "--embedder-url", "http://x:1"],
        "--embedder-url needs --embedder-model",
    );
}

#[test]
fn flags_interval_out_of_range() {
    let m = machine("setup-flags-interval");
    for value in ["0", "1441", "abc"] {
        usage_error(
            &m,
            &["--yes", "--index-every", value],
            &format!("--index-every takes 1 to 1440 minutes, got '{value}'"),
        );
    }
}

#[test]
fn flags_tool_path_not_executable() {
    let m = machine("setup-flags-tool");
    usage_error(
        &m,
        &["--yes", "--claude", "/nope/claude"],
        "--claude /nope/claude is not an executable file",
    );
    usage_error(
        &m,
        &["--yes", "--codex", "relative/codex"],
        "--codex relative/codex is not an executable file",
    );
    let plain = m.bin.join("plain");
    std::fs::write(&plain, "x").unwrap();
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    usage_error(
        &m,
        &["--yes", "--codex", plain.to_str().unwrap()],
        &format!("--codex {} is not an executable file", plain.display()),
    );
}

#[test]
fn flags_repeated() {
    let m = machine("setup-flags-repeated");
    usage_error(&m, &["--yes", "--yes"], "--yes given more than once");
    usage_error(
        &m,
        &["--embedder-url", "http://a:1", "--embedder-url=http://b:1"],
        "--embedder-url given more than once",
    );
}

#[test]
fn flags_value_flag_does_not_swallow_an_option() {
    let m = machine("setup-flags-swallow");
    usage_error(
        &m,
        &["--embedder-query-prefix", "--no-plugin"],
        "--embedder-query-prefix needs a value, got the option --no-plugin",
    );
    usage_error(
        &m,
        &["--claude", "--index-every=30"],
        "--claude needs a value, got the option --index-every",
    );
}

#[test]
fn flags_url_credentials_not_echoed() {
    let m = machine("setup-flags-credentials");
    let run = usage_error(
        &m,
        &[
            "--yes",
            "--embedder-url",
            "http://user:secretpw@host:1",
            "--embedder-model",
            "m",
        ],
        "--embedder-url must not hold a user name or password",
    );
    assert!(!run.stderr.contains("secretpw"));
}

#[test]
fn flags_url_shape_and_variable_and_file() {
    let m = machine("setup-flags-values");
    usage_error(
        &m,
        &[
            "--yes",
            "--embedder-url",
            "ftp://x",
            "--embedder-model",
            "m",
        ],
        "--embedder-url must be an http:// or https:// URL with a host, got 'ftp://x'",
    );
    usage_error(
        &m,
        &[
            "--yes",
            "--embedder-url",
            "http://x:1",
            "--embedder-model",
            "m",
            "--embedder-token-env",
            "1BAD",
        ],
        "--embedder-token-env must be a variable name (letters, digits and _, not starting with a digit)",
    );
    usage_error(
        &m,
        &[
            "--yes",
            "--embedder-url",
            "http://x:1",
            "--embedder-model",
            "m",
            "--embedder-token-file",
            "relative/key",
        ],
        "--embedder-token-file must be an absolute path or start with ~/",
    );
}

#[test]
fn flags_token_env_and_file_conflict() {
    let m = machine("setup-flags-token");
    usage_error(
        &m,
        &[
            "--yes",
            "--embedder-url",
            "http://x:1",
            "--embedder-model",
            "m",
            "--embedder-token-env",
            "KEY",
            "--embedder-token-file",
            "/k",
        ],
        "--embedder-token-env and --embedder-token-file cannot be used together",
    );
}

#[test]
fn flags_no_timer_and_index_every_conflict() {
    let m = machine("setup-flags-timer");
    usage_error(
        &m,
        &["--yes", "--no-timer", "--index-every", "5"],
        "--no-timer and --index-every cannot be used together",
    );
}

#[test]
fn flags_no_plugin_and_claude_conflict() {
    let m = machine("setup-flags-plugin");
    usage_error(
        &m,
        &["--yes", "--no-plugin", "--claude", "/bin/sh"],
        "--no-plugin and --claude cannot be used together",
    );
    usage_error(
        &m,
        &["--yes", "--no-plugin", "--plugin-source", "a/b#c"],
        "--no-plugin and --plugin-source cannot be used together",
    );
}

#[test]
fn flags_bad_plugin_source() {
    let m = machine("setup-flags-source");
    usage_error(
        &m,
        &["--yes", "--plugin-source", "nope"],
        "--plugin-source nope is not a folder with .claude-plugin/marketplace.json, nor <owner>/<repo>#<ref>",
    );
}

#[test]
fn flags_remove_takes_only_four_flags() {
    let m = machine("setup-flags-remove");
    usage_error(
        &m,
        &["--remove", "--embedder-url", "http://x:1"],
        "--remove cannot be used with --embedder-url",
    );
}

#[test]
fn flags_local_with_an_embedder_flag_conflicts() {
    let m = machine("setup-flags-local-conflict");
    for (flag, value) in [
        ("--embedder-url", "http://a:1"),
        ("--embedder-model", "m"),
        ("--embedder-token-env", "KEY"),
        ("--embedder-token-file", "/k"),
        ("--embedder-query-prefix", "p"),
    ] {
        usage_error(
            &m,
            &["--yes", "--embedder-local", flag, value],
            &format!("--embedder-local and {flag} cannot be used together"),
        );
    }
}

#[test]
fn flags_port_without_local() {
    let m = machine("setup-flags-port");
    usage_error(
        &m,
        &["--yes", "--embedder-port", "9000"],
        "--embedder-port needs --embedder-local",
    );
}

#[test]
fn flags_llama_server_without_local() {
    let m = machine("setup-flags-llama");
    usage_error(
        &m,
        &["--yes", "--llama-server", "/x/llama-server"],
        "--llama-server needs --embedder-local",
    );
}

#[test]
fn flags_port_out_of_range() {
    let m = machine("setup-flags-port-range");
    for value in ["80", "70000", "x", "-1"] {
        usage_error(
            &m,
            &[
                "--yes",
                "--embedder-local",
                &format!("--embedder-port={value}"),
            ],
            &format!("--embedder-port takes 1024 to 65535, got '{value}'"),
        );
    }
}

#[test]
fn flags_llama_server_not_executable() {
    let m = machine("setup-flags-llama-path");
    usage_error(
        &m,
        &[
            "--yes",
            "--embedder-local",
            "--llama-server",
            "/nope/llama-server",
        ],
        "--llama-server /nope/llama-server is not an executable file",
    );
    let plain = m.bin.join("plain");
    std::fs::write(&plain, "x").unwrap();
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    usage_error(
        &m,
        &[
            "--yes",
            "--embedder-local",
            "--llama-server",
            plain.to_str().unwrap(),
        ],
        &format!(
            "--llama-server {} is not an executable file",
            plain.display()
        ),
    );
}

#[test]
fn flags_local_answers_the_wizard() {
    let m = machine("setup-flags-local-wizard");
    usage_error(
        &m,
        &["--interactive", "--embedder-local"],
        "--interactive cannot be used with --embedder-local; it answers a wizard question",
    );
}

#[test]
fn flags_remove_rejects_local() {
    let m = machine("setup-flags-local-remove");
    usage_error(
        &m,
        &["--remove", "--embedder-local"],
        "--remove cannot be used with --embedder-local",
    );
}

// ---------------------------------------------------------------------------
// Store and config

#[test]
fn report_fresh_run() {
    let m = machine("setup-report");
    fakes::install(&m.bin, &m.state, &[TIMER_TOOL]);
    let run = watch_setup(&m, &[], &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert_eq!(
        lines(&run),
        [
            format!("store created: {}/notes", root(&m).display()),
            format!("config written: {}", config_path(&m).display()),
            "key skipped: no embedder".to_string(),
            "model skipped: not local".to_string(),
            "server skipped: not local".to_string(),
            "embedder skipped: none configured".to_string(),
            "claude skipped: --no-plugin".to_string(),
            "codex skipped: --no-plugin".to_string(),
            "hook skipped: no codex plugin".to_string(),
            "timer skipped: no embedder".to_string(),
            format!("watch installed: watching {}/notes", root(&m).display()),
            "sync skipped: no scope syncs".to_string(),
        ]
    );
    assert!(root(&m).join("notes").is_dir());
}

#[test]
fn store_existing_is_kept() {
    let m = machine("setup-store-kept");
    let notes = root(&m).join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let note = notes.join("plan-x.md");
    std::fs::write(&note, common::note_text(common::IDS[0], "X")).unwrap();
    let before = std::fs::read(&note).unwrap();
    let run = setup(&m, &[], &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(lines(&run)[0], format!("store kept: {}", notes.display()));
    assert_eq!(std::fs::read(&note).unwrap(), before);
}

#[test]
fn store_root_cannot_be_created() {
    let m = machine("setup-store-file");
    let file = m.dir.path().join("a-file");
    std::fs::write(&file, "x").unwrap();
    let home = file.to_str().unwrap();
    let run = setup(&m, &[("BILBO_HOME", home)], &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let report = lines(&run);
    assert!(
        report[0].starts_with(&format!("store failed: cannot create {home}/notes: ")),
        "{report:?}"
    );
    assert_eq!(report.len(), 12, "a failed step stops nothing: {report:?}");
    assert!(report[1].starts_with("config written: "));
}

#[cfg(target_os = "linux")]
#[test]
fn store_root_in_proc_fails() {
    let m = machine("setup-store-proc");
    let run = setup(
        &m,
        &[("BILBO_HOME", "/proc/bilbo")],
        &["--yes", "--no-plugin"],
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        lines(&run)[0].starts_with("store failed: cannot create /proc/bilbo/notes: "),
        "{:?}",
        lines(&run)
    );
}

#[test]
fn config_flags_become_settings() {
    let m = machine("setup-config-flags");
    let fake = Fake::start(8);
    let run = setup(&m, &[], &embedder_args(&fake, "nomic-embed-text"));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(config_path(&m)).unwrap();
    assert!(text.starts_with("# bilbo config, written by bilbo setup "));
    assert!(text.contains(&format!("embedder.url = {}\n", fake.url)));
    assert!(text.contains("embedder.model = nomic-embed-text\n"));
    assert!(!text.contains("query_prefix"));
    std::fs::write(
        root(&m).join("notes/plan-alpha.md"),
        common::note_text(common::IDS[0], "Alpha plan"),
    )
    .unwrap();
    let recall = bilbo(
        &m.home,
        &[
            ("HOME", m.home.to_str().unwrap()),
            ("PATH", m.bin.to_str().unwrap()),
        ],
        &["recall", "alpha"],
    );
    assert_eq!(recall.code, 0, "{}", recall.stderr);
    assert!(!recall.stderr.contains("config"), "{}", recall.stderr);
}

#[test]
fn config_no_embedder_writes_comments_only() {
    let m = machine("setup-config-empty");
    let run = setup(&m, &[], &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(config_path(&m)).unwrap();
    assert!(
        text.lines()
            .next()
            .unwrap()
            .starts_with("# bilbo config, written by bilbo setup ")
    );
    assert!(
        text.lines().all(|l| l.is_empty() || l.starts_with('#')),
        "{text}"
    );
    std::fs::write(
        root(&m).join("notes/plan-alpha.md"),
        common::note_text(common::IDS[0], "Alpha plan"),
    )
    .unwrap();
    let recall = bilbo(
        &m.home,
        &[
            ("HOME", m.home.to_str().unwrap()),
            ("PATH", m.bin.to_str().unwrap()),
        ],
        &["recall", "alpha"],
    );
    assert_eq!(recall.code, 0, "{}", recall.stderr);
    assert!(recall.stderr.is_empty(), "{}", recall.stderr);
}

#[test]
fn config_existing_with_other_flags_is_refused() {
    let m = machine("setup-config-other");
    let fake = Fake::start(8);
    write_config(
        &m,
        &[
            &format!("embedder.url = {}", fake.url),
            "embedder.model = a",
        ],
    );
    let before = snapshot(m.dir.path());
    let run = setup(&m, &[], &embedder_args(&fake, "b"));
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains(&format!(
        "bilbo: {} already sets other embedder settings; edit it, or run bilbo setup in a terminal to change it\n",
        config_path(&m).display()
    )));
    assert!(fake.requests().is_empty());
    assert_eq!(snapshot(m.dir.path()), before);
}

#[test]
fn config_empty_gets_the_embedder_from_flags() {
    let m = machine("setup-config-empty");
    let fake = Fake::start(8);
    let plain = ["--yes", "--no-plugin", "--no-timer"];
    assert_eq!(setup(&m, &[], &plain).code, 0);
    let old = std::fs::read_to_string(config_path(&m)).unwrap();
    let run = setup(&m, &[], &embedder_args(&fake, "m"));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(
        report[1],
        format!("config updated: {}", config_path(&m).display())
    );
    assert_eq!(report[5], "embedder ok: 8 dimensions");
    assert_eq!(fake.requests().len(), 1);
    let backup = config_path(&m).with_file_name("config.bak");
    assert_eq!(std::fs::read_to_string(backup).unwrap(), old);
    let now = std::fs::read_to_string(config_path(&m)).unwrap();
    assert!(
        now.contains(&format!("embedder.url = {}", fake.url)),
        "{now}"
    );
    assert!(now.contains("embedder.model = m"), "{now}");
}

#[test]
fn config_with_only_a_floor_is_refused_with_flags() {
    let m = machine("setup-config-floor");
    let fake = Fake::start(8);
    write_config(&m, &["embedder.min_similarity = 0.6"]);
    let before = snapshot(m.dir.path());
    let run = setup(&m, &[], &embedder_args(&fake, "m"));
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains(&format!(
        "bilbo: {} already sets settings but no embedder; edit it, or run bilbo setup in a terminal to change it\n",
        config_path(&m).display()
    )));
    assert!(fake.requests().is_empty());
    assert_eq!(snapshot(m.dir.path()), before);
}

#[test]
fn config_same_flags_again_are_kept() {
    let m = machine("setup-config-same");
    let fake = Fake::start(8);
    let args = embedder_args(&fake, "m");
    assert_eq!(setup(&m, &[], &args).code, 0);
    let run = setup(&m, &[], &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(
        report[1],
        format!("config kept: {}", config_path(&m).display())
    );
    assert_eq!(report[5], "embedder skipped: config kept");
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn config_managed_link_is_kept() {
    let m = machine("setup-config-link");
    let target = m.dir.path().join("managed-config");
    std::fs::write(
        &target,
        "embedder.url = http://127.0.0.1:1\nembedder.model = m\n",
    )
    .unwrap();
    std::fs::create_dir_all(config_path(&m).parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, config_path(&m)).unwrap();
    let run = setup(&m, &[], &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(
        report[1],
        format!("config kept: managed elsewhere ({})", target.display())
    );
    assert_eq!(report[2], "key skipped: local embedder");
    assert_eq!(report[5], "embedder skipped: config kept");
    assert_eq!(
        std::fs::read_link(config_path(&m)).unwrap(),
        target,
        "the link stays"
    );
}

#[test]
fn config_managed_with_flags_is_a_usage_error() {
    let m = machine("setup-config-link-flags");
    let target = m.dir.path().join("managed-config");
    std::fs::write(&target, "# managed\n").unwrap();
    std::fs::create_dir_all(config_path(&m).parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, config_path(&m)).unwrap();
    let before = snapshot(m.dir.path());
    let run = setup(
        &m,
        &[],
        &[
            "--yes",
            "--embedder-url",
            "http://x:1",
            "--embedder-model",
            "m",
        ],
    );
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains(&format!(
            "bilbo: {} is managed elsewhere; change the embedder there, not with --embedder-url\n",
            config_path(&m).display()
        )),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(m.dir.path()), before);
}

/// Gives a folder its write bit back so the temp folder can be deleted.
struct Restore(PathBuf);

impl Drop for Restore {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

#[test]
fn config_read_only_folder_is_managed() {
    let m = machine("setup-config-readonly");
    write_config(&m, &["# nothing here"]);
    let folder = config_path(&m).parent().unwrap().to_path_buf();
    let _restore = Restore(folder.clone());
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    let run = setup(&m, &[], &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run)[1],
        format!(
            "config kept: managed elsewhere ({})",
            config_path(&m).display()
        )
    );
    assert_eq!(
        std::fs::read_to_string(config_path(&m)).unwrap(),
        "# nothing here\n"
    );
}

#[test]
fn config_missing_bilbo_config_file_is_created() {
    let m = machine("setup-config-explicit");
    let fake = Fake::start(8);
    let file = m.dir.path().join("elsewhere/config");
    let run = setup(
        &m,
        &[("BILBO_CONFIG", file.to_str().unwrap())],
        &embedder_args(&fake, "m"),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run)[1],
        format!("config written: {}", file.display())
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&format!("embedder.url = {}\n", fake.url)));
    assert!(text.contains("embedder.model = m\n"));
    assert!(!config_path(&m).exists());
}

#[test]
fn config_relative_bilbo_config_is_refused() {
    let m = machine("setup-config-relative");
    let before = snapshot(m.dir.path());
    let run = setup(
        &m,
        &[("BILBO_CONFIG", "rel/config")],
        &["--yes", "--no-plugin"],
    );
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        "bilbo: BILBO_CONFIG must be an absolute path, got 'rel/config'\n"
    );
    assert_eq!(snapshot(m.dir.path()), before);
}

#[test]
fn config_broken_existing_config_exits_2() {
    let m = machine("setup-config-broken");
    write_config(&m, &["bogus = 1"]);
    let before = snapshot(m.dir.path());
    let run = setup(&m, &[], &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("unknown key 'bogus'"), "{}", run.stderr);
    assert_eq!(snapshot(m.dir.path()), before);
}

#[test]
fn config_qwen_prefix_from_flags() {
    let m = machine("setup-config-qwen");
    let fake = Fake::start(8);
    let run = setup(&m, &[], &embedder_args(&fake, "qwen3-embedding-0.6b"));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(config_path(&m)).unwrap();
    assert!(
        text.contains(
            "embedder.query_prefix = \"Instruct: Given a question, retrieve notes that answer it\\nQuery: \"\n"
        ),
        "{text}"
    );
    let upper = machine("setup-config-qwen-upper");
    let run = setup(&upper, &[], &embedder_args(&fake, "Qwen3-Embedding-0.6B"));
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        std::fs::read_to_string(config_path(&upper))
            .unwrap()
            .contains("embedder.query_prefix = ")
    );
}

#[test]
fn config_empty_prefix_flag_writes_none() {
    let fake = Fake::start(8);
    for (name, extra) in [
        ("setup-config-noprefix-eq", vec!["--embedder-query-prefix="]),
        (
            "setup-config-noprefix-arg",
            vec!["--embedder-query-prefix", ""],
        ),
    ] {
        let m = machine(name);
        let mut args = embedder_args(&fake, "qwen3-embedding-0.6b");
        args.extend(extra);
        let run = setup(&m, &[], &args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        let text = std::fs::read_to_string(config_path(&m)).unwrap();
        assert!(!text.contains("query_prefix"), "{text}");
    }
}

#[test]
fn rerun_changes_nothing() {
    let m = machine("setup-rerun");
    let fake = Fake::start(8);
    let args = embedder_args(&fake, "m");
    let first = setup(&m, &[], &args);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let before = snapshot(m.dir.path());
    let second = setup(&m, &[], &args);
    assert_eq!(second.code, 0, "{}", second.stderr);
    for line in lines(&second) {
        for word in ["created", "written", "installed", "updated"] {
            assert!(
                line.split_whitespace()
                    .nth(1)
                    .is_none_or(|s| s.trim_end_matches(':') != word),
                "{line}"
            );
        }
    }
    assert_eq!(snapshot(m.dir.path()), before);
    assert_eq!(fake.requests().len(), 1);
}

// ---------------------------------------------------------------------------
// Embedder check and key

#[test]
fn embedder_ok_reports_dimensions() {
    let m = machine("setup-embedder-ok");
    let fake = Fake::start(1024);
    let run = setup(&m, &[], &embedder_args(&fake, "qwen3-embedding-0.6b"));
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(report[2], "key skipped: local embedder");
    assert_eq!(report[5], "embedder ok: 1024 dimensions");
    assert_eq!(fake.inputs(), ["bilbo setup check"]);
    assert_eq!(fake.requests()[0].model, "qwen3-embedding-0.6b");
}

#[test]
fn embedder_404_refuses_and_writes_nothing() {
    let m = machine("setup-embedder-404");
    let fake = Fake::start(8);
    fake.status(404);
    let before = snapshot(m.dir.path());
    let run = setup(&m, &[], &embedder_args(&fake, "nope"));
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains(&fake.url), "{}", run.stderr);
    assert!(run.stderr.contains("404"), "{}", run.stderr);
    assert_eq!(snapshot(m.dir.path()), before);
    assert!(!root(&m).exists() && !config_path(&m).exists());
}

#[test]
fn embedder_existing_config_is_not_checked() {
    let m = machine("setup-embedder-existing");
    let fake = Fake::start(8);
    write_config(
        &m,
        &[
            &format!("embedder.url = {}", fake.url),
            "embedder.model = m",
        ],
    );
    let run = setup(&m, &[], &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(lines(&run)[5], "embedder skipped: config kept");
    assert!(fake.requests().is_empty());
}

fn bearer(fake: &Fake) -> String {
    fake.requests()[0]
        .headers
        .iter()
        .find(|(name, _)| name == "authorization")
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

#[test]
fn embedder_token_never_printed() {
    let fake = Fake::start(8);

    let by_variable = machine("setup-token-var");
    let mut args = embedder_args(&fake, "m");
    args.extend(["--embedder-token-env", "MY_KEY"]);
    let run = setup(&by_variable, &[("MY_KEY", "sk-secret-123")], &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(bearer(&fake), "Bearer sk-secret-123");
    for text in [
        &run.stdout,
        &run.stderr,
        &std::fs::read_to_string(config_path(&by_variable)).unwrap(),
    ] {
        assert!(!text.contains("sk-secret-123"), "{text}");
    }

    let by_file = machine("setup-token-file");
    let key = by_file.dir.path().join("key");
    std::fs::write(&key, "sk-file-456\n").unwrap();
    let fake = Fake::start(8);
    let mut args = embedder_args(&fake, "m");
    args.extend(["--embedder-token-file", key.to_str().unwrap()]);
    let run = setup(&by_file, &[], &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(bearer(&fake), "Bearer sk-file-456");
    for text in [
        &run.stdout,
        &run.stderr,
        &std::fs::read_to_string(config_path(&by_file)).unwrap(),
    ] {
        assert!(!text.contains("sk-file-456"), "{text}");
    }
}

#[test]
fn key_from_variable_is_ok() {
    let m = machine("setup-key-var");
    let fake = Fake::start(8);
    let mut args = embedder_args(&fake, "m");
    args.extend(["--embedder-token-env", "MY_KEY"]);
    let run = setup(&m, &[("MY_KEY", "sk-1")], &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(lines(&run)[2], "key ok: variable MY_KEY");
    assert!(
        std::fs::read_to_string(config_path(&m))
            .unwrap()
            .contains("embedder.token_env = MY_KEY\n")
    );
}

#[test]
fn key_from_file_is_ok() {
    let m = machine("setup-key-file");
    let fake = Fake::start(8);
    let key: &Path = &m.dir.path().join("key");
    std::fs::write(key, "sk-1\n").unwrap();
    let mut args = embedder_args(&fake, "m");
    args.extend(["--embedder-token-file", key.to_str().unwrap()]);
    let run = setup(&m, &[], &args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(lines(&run)[2], format!("key ok: file {}", key.display()));
    assert!(
        std::fs::read_to_string(config_path(&m))
            .unwrap()
            .contains(&format!("embedder.token_file = {}\n", key.display()))
    );
}

#[test]
fn key_unset_variable_refuses() {
    let m = machine("setup-key-unset");
    let fake = Fake::start(8);
    let before = snapshot(m.dir.path());
    let mut args = embedder_args(&fake, "m");
    args.extend(["--embedder-token-env", "MY_KEY"]);
    let run = setup(&m, &[], &args);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("MY_KEY"), "{}", run.stderr);
    assert!(fake.requests().is_empty());
    assert_eq!(snapshot(m.dir.path()), before);
}

#[test]
fn key_local_embedder_is_skipped() {
    let m = machine("setup-key-local");
    let fake = Fake::start(8);
    let run = setup(&m, &[], &embedder_args(&fake, "m"));
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(lines(&run)[2], "key skipped: local embedder");
}

// ---------------------------------------------------------------------------
// Plugin step

const CLAUDE_STATE_OLD: &str = "github delucca/bilbo v0.0.9";
const CODEX_STATE_OLD: &str = "git https://github.com/delucca/bilbo.git v0.0.9";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn agents_machine(name: &str, tools: &[&str]) -> Machine {
    let m = machine(name);
    fakes::install(&m.bin, &m.state, tools);
    m
}

fn plugin_setup(m: &Machine, extra: &[&str]) -> Run {
    let mut args = vec!["--yes", "--no-timer"];
    args.extend(extra);
    setup(m, &[], &args)
}

/// The calls of a fake that are not the two read-only lists or an app-server request.
fn changing(m: &Machine, tool: &str) -> Vec<String> {
    fakes::log(&m.state, tool)
        .into_iter()
        .filter(|call| !call.ends_with("list --json") && !call.starts_with("app-server"))
        .collect()
}

#[test]
fn plugin_fresh_install_in_claude() {
    let m = agents_machine("plugin-claude", &["claude"]);
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        fakes::log(&m.state, "claude"),
        [
            "plugin marketplace list --json",
            "plugin list --json",
            format!("plugin marketplace add delucca/bilbo#v{VERSION} --json").as_str(),
            "plugin install bilbo@bilbo --json",
        ]
    );
    let lines = lines(&run);
    assert!(lines.contains(&format!("claude installed: delucca/bilbo#v{VERSION}").as_str()));
    assert!(lines.contains(&"codex skipped: not found"));
    assert_eq!(fakes::get(&m.state, "claude", "plugin"), "true");
}

#[test]
fn plugin_fresh_install_in_codex() {
    let m = agents_machine("plugin-codex", &["codex"]);
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        fakes::log(&m.state, "codex"),
        [
            "plugin marketplace list --json",
            "plugin list --json",
            format!("plugin marketplace add delucca/bilbo --ref v{VERSION} --json").as_str(),
            "plugin add bilbo@bilbo --json",
            "app-server",
            "app-server initialize",
            "app-server hooks/list",
            "app-server config/batchWrite",
        ]
    );
    let lines = lines(&run);
    assert!(lines.contains(&format!("codex installed: delucca/bilbo#v{VERSION}").as_str()));
    assert!(lines.contains(&"hook installed: trusted in Codex"));
    assert!(lines.contains(&"claude skipped: not found"));
}

#[test]
fn plugin_older_source_is_updated() {
    let m = agents_machine("plugin-older", &["claude", "codex"]);
    fakes::set(&m.state, "claude", "marketplace", CLAUDE_STATE_OLD);
    fakes::set(&m.state, "claude", "plugin", "true");
    fakes::set(&m.state, "codex", "marketplace", CODEX_STATE_OLD);
    fakes::set(&m.state, "codex", "plugin", "0.0.9 true");
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        changing(&m, "claude"),
        [
            "plugin marketplace remove bilbo --json",
            format!("plugin marketplace add delucca/bilbo#v{VERSION} --json").as_str(),
            "plugin install bilbo@bilbo --json",
        ]
    );
    assert_eq!(
        changing(&m, "codex"),
        [
            "plugin marketplace remove bilbo --json",
            format!("plugin marketplace add delucca/bilbo --ref v{VERSION} --json").as_str(),
            "plugin add bilbo@bilbo --json",
        ]
    );
    let lines = lines(&run);
    assert!(lines.contains(&format!("claude updated: delucca/bilbo#v{VERSION}").as_str()));
    assert!(lines.contains(&format!("codex updated: delucca/bilbo#v{VERSION}").as_str()));
}

#[test]
fn plugin_codex_current_is_kept() {
    let m = agents_machine("plugin-codex-current", &["codex"]);
    fakes::set(
        &m.state,
        "codex",
        "marketplace",
        format!("git https://github.com/delucca/bilbo.git v{VERSION}").as_str(),
    );
    fakes::set(
        &m.state,
        "codex",
        "plugin",
        format!("{VERSION} true").as_str(),
    );
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"codex kept"));
    assert!(changing(&m, "codex").is_empty());
}

#[test]
fn plugin_disabled_is_reinstalled() {
    let m = agents_machine("plugin-disabled", &["claude"]);
    fakes::set(
        &m.state,
        "claude",
        "marketplace",
        format!("github delucca/bilbo v{VERSION}").as_str(),
    );
    fakes::set(&m.state, "claude", "plugin", "false");
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        changing(&m, "claude"),
        ["plugin install bilbo@bilbo --json"]
    );
    assert!(lines(&run).contains(&format!("claude updated: delucca/bilbo#v{VERSION}").as_str()));
    assert_eq!(fakes::get(&m.state, "claude", "plugin"), "true");
}

#[test]
fn plugin_tool_absent_is_skipped() {
    let m = agents_machine("plugin-absent", &["claude"]);
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"codex skipped: not found"));
    assert!(fakes::log(&m.state, "codex").is_empty());
}

#[test]
fn plugin_no_plugin_runs_nothing() {
    let m = agents_machine("plugin-none", &["claude", "codex"]);
    let run = plugin_setup(&m, &["--no-plugin"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines = lines(&run);
    assert!(lines.contains(&"claude skipped: --no-plugin"));
    assert!(lines.contains(&"codex skipped: --no-plugin"));
    assert!(fakes::log(&m.state, "claude").is_empty());
    assert!(fakes::log(&m.state, "codex").is_empty());
}

#[test]
fn plugin_failing_install_does_not_stop_later_steps() {
    let m = agents_machine("plugin-fail", &["claude", "codex", TIMER_TOOL]);
    fakes::set(
        &m.state,
        "claude",
        "fail-install",
        r#"{"command":"install","outcome":"failed","message":"boom","failureCode":"x"}"#,
    );
    let run = watch_setup(&m, &[], &["--yes"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let lines = lines(&run);
    assert!(lines.contains(&format!("claude failed: delucca/bilbo#v{VERSION}: boom").as_str()));
    assert!(lines.contains(&format!("codex installed: delucca/bilbo#v{VERSION}").as_str()));
    assert!(lines.contains(&"timer skipped: no embedder"));
    assert_eq!(lines.last(), Some(&"sync skipped: no scope syncs"));
    assert!(watch_exists(&m));
}

#[test]
fn plugin_missing_tag_names_the_source_and_the_hint() {
    let m = agents_machine("plugin-tag", &["claude", "codex"]);
    fakes::set(
        &m.state,
        "claude",
        "fail-marketplace-add",
        r#"{"command":"marketplace-add","outcome":"failed","message":"Failed to clone marketplace repository: nope","failureCode":"error_not_found"}"#,
    );
    fakes::set(
        &m.state,
        "codex",
        "fail-marketplace-add",
        format!("Error: git checkout v{VERSION} failed with status exit status: 1").as_str(),
    );
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let hint = "; for a build without a release tag, pass --plugin-source <folder>";
    let lines = lines(&run);
    assert!(lines.contains(&format!("claude failed: delucca/bilbo#v{VERSION}: Failed to clone marketplace repository: nope{hint}").as_str()));
    assert!(lines.contains(
        &format!(
            "codex failed: delucca/bilbo#v{VERSION}: git checkout v{VERSION} failed with status exit status: 1{hint}"
        )
        .as_str()
    ));
}

/// A folder holding both marketplaces, as the package's `share/bilbo`.
fn package_folder(folder: &Path) {
    for manifest in [".claude-plugin", ".agents/plugins"] {
        std::fs::create_dir_all(folder.join(manifest)).unwrap();
        std::fs::write(folder.join(manifest).join("marketplace.json"), "{}\n").unwrap();
    }
}

/// Runs `exe setup` with the same clean environment as `setup`.
fn setup_with(exe: &Path, m: &Machine, args: &[&str]) -> Run {
    let output = std::process::Command::new(exe)
        .env_clear()
        .env("HOME", &m.home)
        .env("PATH", &m.bin)
        .current_dir(&m.home)
        .arg("setup")
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn packaged_binary(m: &Machine) -> (PathBuf, PathBuf) {
    let prefix = std::fs::canonicalize(m.dir.path()).unwrap().join("prefix");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    let exe = prefix.join("bin/bilbo");
    std::fs::copy(env!("CARGO_BIN_EXE_bilbo"), &exe).unwrap();
    let share = prefix.join("share/bilbo");
    package_folder(&share);
    (exe, share)
}

#[test]
fn plugin_package_folder_is_the_source() {
    let m = agents_machine("plugin-package", &["claude", "codex"]);
    let (exe, share) = packaged_binary(&m);
    fakes::set(&m.state, "codex", "local-version", VERSION);
    let run = setup_with(&exe, &m, &["--yes", "--no-timer", "--no-watch"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let share = share.display();
    let lines = lines(&run);
    assert!(lines.contains(&format!("claude installed: {share}").as_str()));
    assert!(lines.contains(&format!("codex installed: {share}").as_str()));
    assert!(
        fakes::log(&m.state, "claude").contains(&format!("plugin marketplace add {share} --json"))
    );
    assert!(
        fakes::log(&m.state, "codex").contains(&format!("plugin marketplace add {share} --json"))
    );
}

#[test]
fn plugin_symlinked_binary_finds_the_package() {
    let m = agents_machine("plugin-symlink", &["claude"]);
    let (exe, share) = packaged_binary(&m);
    let link = m.dir.path().join("link");
    std::fs::create_dir_all(&link).unwrap();
    std::os::unix::fs::symlink(&exe, link.join("bilbo")).unwrap();
    let run = setup_with(
        &link.join("bilbo"),
        &m,
        &["--yes", "--no-timer", "--no-watch"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&format!("claude installed: {}", share.display()).as_str()));
}

#[test]
fn plugin_source_flag_folder_and_repo() {
    let m = agents_machine("plugin-source", &["claude", "codex"]);
    let folder = std::fs::canonicalize(m.dir.path()).unwrap().join("market");
    package_folder(&folder);
    fakes::set(&m.state, "codex", "local-version", VERSION);
    let shown = folder.display().to_string();
    let run = plugin_setup(&m, &["--plugin-source", &shown]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert!(report.contains(&format!("claude installed: {shown}").as_str()));
    assert!(report.contains(&format!("codex installed: {shown}").as_str()));

    let m = agents_machine("plugin-source-repo", &["claude", "codex"]);
    let run = plugin_setup(&m, &["--plugin-source=someone/fork#v9"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"claude installed: someone/fork#v9"));
    assert!(
        fakes::log(&m.state, "claude")
            .contains(&"plugin marketplace add someone/fork#v9 --json".to_string())
    );
    assert!(
        fakes::log(&m.state, "codex")
            .contains(&"plugin marketplace add someone/fork --ref v9 --json".to_string())
    );
}

#[test]
fn plugin_claude_flag_path_is_used() {
    let m = agents_machine("plugin-claude-flag", &[]);
    let elsewhere = m.dir.path().join("elsewhere");
    fakes::install(&elsewhere, &m.state, &["claude"]);
    let program = elsewhere.join("claude");
    let run = plugin_setup(&m, &["--claude", program.to_str().unwrap()]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines = lines(&run);
    assert!(lines.contains(&format!("claude installed: delucca/bilbo#v{VERSION}").as_str()));
    assert!(lines.contains(&"codex skipped: not found"));
}

#[test]
fn plugin_rerun_runs_no_changing_command() {
    let m = agents_machine("plugin-rerun", &["claude", "codex"]);
    let first = plugin_setup(&m, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let claude = changing(&m, "claude").len();
    let codex = changing(&m, "codex").len();
    assert_eq!((claude, codex), (2, 2));
    let writes = |m: &Machine| {
        fakes::log(&m.state, "codex")
            .iter()
            .filter(|call| *call == "app-server config/batchWrite")
            .count()
    };
    assert_eq!(writes(&m), 1);
    let second = plugin_setup(&m, &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    let lines = lines(&second);
    assert!(lines.contains(&"claude kept"));
    assert!(lines.contains(&"codex kept"));
    assert_eq!(changing(&m, "claude").len(), claude);
    assert_eq!(changing(&m, "codex").len(), codex);
    assert_eq!(writes(&m), 1);
}

// ---------------------------------------------------------------------------
// Codex hook trust

fn batch_writes(m: &Machine) -> usize {
    fakes::log(&m.state, "codex")
        .iter()
        .filter(|call| *call == "app-server config/batchWrite")
        .count()
}

#[test]
fn hook_fresh_install_trusts() {
    let m = agents_machine("hook-fresh", &["codex"]);
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook installed: trusted in Codex"));
    let log = fakes::log(&m.state, "codex");
    assert!(
        log.contains(&"app-server hooks/list".to_string()),
        "{log:?}"
    );
    assert_eq!(batch_writes(&m), 1);
    assert_eq!(fakes::get(&m.state, "codex", "trust"), "sha256:fake");
    let order: Vec<_> = lines(&run)
        .into_iter()
        .filter(|l| l.starts_with("codex ") || l.starts_with("hook ") || l.starts_with("timer "))
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(order, ["codex", "hook", "timer"]);
}

#[test]
fn hook_rerun_keeps() {
    let m = agents_machine("hook-rerun", &["codex"]);
    assert_eq!(plugin_setup(&m, &[]).code, 0);
    let before = fakes::log(&m.state, "codex").len();
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook kept: trusted in Codex"));
    assert_eq!(batch_writes(&m), 1);
    let second = &fakes::log(&m.state, "codex")[before..];
    assert!(
        second.contains(&"app-server hooks/list".to_string()),
        "{second:?}"
    );
    assert!(!second.contains(&"app-server config/batchWrite".to_string()));
}

#[test]
fn hook_changed_is_trusted_again() {
    let m = agents_machine("hook-changed", &["codex"]);
    assert_eq!(plugin_setup(&m, &[]).code, 0);
    fakes::set(&m.state, "codex", "hook-hash", "sha256:changed");
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook updated: trusted in Codex"));
    assert_eq!(batch_writes(&m), 2);
    assert_eq!(fakes::get(&m.state, "codex", "trust"), "sha256:changed");
}

#[test]
fn hook_no_plugin_skips() {
    let m = agents_machine("hook-no-plugin", &["codex"]);
    let run = plugin_setup(&m, &["--no-plugin"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook skipped: no codex plugin"));
    assert!(
        fakes::log(&m.state, "codex")
            .iter()
            .all(|call| !call.starts_with("app-server"))
    );
}

#[test]
fn hook_codex_lists_no_bilbo_hook() {
    let m = agents_machine("hook-none", &["codex"]);
    fakes::set(&m.state, "codex", "no-hook", "1");
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook skipped: codex lists no bilbo hook"));
    assert_eq!(batch_writes(&m), 0);
}

#[test]
fn hook_unwritable_config_fails() {
    let m = agents_machine("hook-readonly", &["codex"]);
    fakes::set(&m.state, "codex", "readonly", "1");
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let report = lines(&run);
    let hook = report
        .iter()
        .find(|l| l.starts_with("hook failed: "))
        .unwrap();
    assert!(hook.contains("failed to persist config.toml"), "{hook}");
    assert!(!hook.contains('\n') && hook.chars().count() < 260);
    assert_eq!(
        report.iter().find(|l| l.starts_with("timer ")),
        Some(&"timer skipped: --no-timer")
    );
    assert_eq!(fakes::get(&m.state, "codex", "trust"), "");
}

#[test]
fn hook_app_server_missing_fails() {
    let m = agents_machine("hook-no-server", &["codex"]);
    fakes::set(&m.state, "codex", "fail-app-server", "no app-server here");
    let run = plugin_setup(&m, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let report = lines(&run);
    assert!(
        report
            .iter()
            .any(|l| l.starts_with("hook failed: ") && l.ends_with("no app-server here")),
        "{report:?}"
    );
    assert_eq!(
        report.iter().find(|l| l.starts_with("timer ")),
        Some(&"timer skipped: --no-timer")
    );
}

#[test]
fn hook_remove_deletes_the_trust() {
    let m = agents_machine("hook-remove", &["claude", "codex"]);
    assert_eq!(plugin_setup(&m, &[]).code, 0);
    assert_eq!(fakes::get(&m.state, "codex", "trust"), "sha256:fake");
    let run = setup(&m, &[], &["--remove", "--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook removed"));
    assert_eq!(fakes::get(&m.state, "codex", "trust"), "");
    let writes = std::fs::read_to_string(m.state.join("codex.batch-writes")).unwrap();
    let last = writes.lines().last().unwrap();
    assert!(
        last.contains(
            r#""keyPath":"hooks.state.\"bilbo@bilbo:hooks/hooks.json:user_prompt_submit:0:0\"""#
        ) && last.contains(r#""value":null"#),
        "{last}"
    );
}

#[test]
fn hook_remove_without_trust() {
    let m = agents_machine("hook-remove-none", &["codex"]);
    let run = setup(&m, &[], &["--remove", "--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(lines(&run).contains(&"hook skipped: not trusted"));
    assert_eq!(batch_writes(&m), 0);
}

// ---------------------------------------------------------------------------
// Timer step

const TIMER_TOOL: &str = if cfg!(target_os = "macos") {
    "launchctl"
} else {
    "systemctl"
};

fn timer_machine(name: &str) -> Machine {
    agents_machine(name, &[TIMER_TOOL])
}

/// The files the job lives in on this platform, whether or not they exist.
fn timer_files(m: &Machine) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![
            m.home
                .join("Library/LaunchAgents/io.github.delucca.bilbo.index.plist"),
        ]
    } else {
        let units = m.home.join(".config/systemd/user");
        vec![
            units.join("bilbo-index.service"),
            units.join("bilbo-index.timer"),
        ]
    }
}

fn timer_text(m: &Machine) -> String {
    timer_files(m)
        .iter()
        .map(|file| std::fs::read_to_string(file).unwrap())
        .collect()
}

fn timer_exists(m: &Machine) -> bool {
    timer_files(m).iter().all(|file| file.exists())
}

fn timer_gone(m: &Machine) -> bool {
    timer_files(m).iter().all(|file| !file.exists())
}

/// The calls the manager fake saw.
fn manager_log(m: &Machine) -> Vec<String> {
    fakes::log(&m.state, TIMER_TOOL)
}

#[cfg(target_os = "macos")]
fn uid() -> String {
    let output = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// What installing the job asks of the manager, after planning.
#[cfg(target_os = "macos")]
fn install_calls(m: &Machine) -> Vec<String> {
    let uid = uid();
    vec![
        format!("bootout --wait gui/{uid}/io.github.delucca.bilbo.index"),
        format!("bootstrap gui/{uid} {}", timer_files(m)[0].display()),
    ]
}

#[cfg(target_os = "linux")]
fn install_calls(_m: &Machine) -> Vec<String> {
    [
        "--user is-system-running",
        "--user daemon-reload",
        "--user enable bilbo-index.timer",
        "--user restart bilbo-index.timer",
    ]
    .map(String::from)
    .to_vec()
}

/// The watcher's files on this platform, whether or not they exist.
fn watch_files(m: &Machine) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![
            m.home
                .join("Library/LaunchAgents/io.github.delucca.bilbo.watch.plist"),
        ]
    } else {
        vec![m.home.join(".config/systemd/user/bilbo-watch.service")]
    }
}

fn watch_text(m: &Machine) -> String {
    watch_files(m)
        .iter()
        .map(|file| std::fs::read_to_string(file).unwrap())
        .collect()
}

fn watch_exists(m: &Machine) -> bool {
    watch_files(m).iter().all(|file| file.exists())
}

fn watch_gone(m: &Machine) -> bool {
    watch_files(m).iter().all(|file| !file.exists())
}

/// The text that sets the job's interval.
fn interval_text(minutes: u32) -> String {
    if cfg!(target_os = "macos") {
        format!("<integer>{}</integer>", minutes * 60)
    } else {
        format!("OnUnitActiveSec={minutes}min")
    }
}

/// The text that sets one variable of the job.
fn variable_text(name: &str, value: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("<key>{name}</key>\n\t\t<string>{value}</string>")
    } else {
        format!("Environment=\"{name}={value}\"")
    }
}

/// A config with a local embedder that is never asked, so the timer tests need no server.
fn timer_config(m: &Machine) {
    write_config(
        m,
        &["embedder.url = http://127.0.0.1:9", "embedder.model = m"],
    );
}

fn timer_setup(m: &Machine, extra: &[&str]) -> Run {
    let mut args = vec!["--yes", "--no-plugin"];
    args.extend(extra);
    setup(m, &[], &args)
}

fn binary() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_bilbo")).unwrap()
}

#[test]
fn timer_installed_every_15() {
    let m = timer_machine("timer-15");
    let fake = Fake::start(8);
    let run = setup(
        &m,
        &[],
        &[
            "--yes",
            "--no-plugin",
            "--embedder-url",
            &fake.url,
            "--embedder-model",
            "m",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "timer"), Some("timer installed: every 15 min"));
    assert!(timer_exists(&m));
    let text = timer_text(&m);
    assert!(text.contains(&interval_text(15)), "{text}");
    assert!(text.contains(&binary().display().to_string()), "{text}");
    assert!(m.home.join(".local/state/bilbo").is_dir());
    assert_eq!(manager_log(&m), install_calls(&m));
}

#[test]
fn timer_every_30() {
    let m = timer_machine("timer-30");
    timer_config(&m);
    let run = timer_setup(&m, &["--index-every", "30"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "timer"), Some("timer installed: every 30 min"));
    assert!(timer_text(&m).contains(&interval_text(30)));
}

#[test]
fn timer_carries_locations() {
    let m = timer_machine("timer-locations");
    timer_config(&m);
    let data = m.dir.path().join("data");
    let cache = m.dir.path().join("cache");
    let (data, cache) = (data.to_str().unwrap(), cache.to_str().unwrap());
    let run = setup(
        &m,
        &[("BILBO_HOME", data), ("XDG_CACHE_HOME", cache)],
        &["--yes", "--no-plugin"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "timer"), Some("timer installed: every 15 min"));
    let text = timer_text(&m);
    assert!(text.contains(&variable_text("BILBO_HOME", data)), "{text}");
    assert!(
        text.contains(&variable_text("XDG_CACHE_HOME", cache)),
        "{text}"
    );
    assert!(!text.contains("XDG_STATE_HOME"), "{text}");
    assert!(!text.contains("PATH"), "{text}");
    assert!(m.home.join(".local/state/bilbo").is_dir());
}

#[test]
fn timer_key_in_a_variable_fails() {
    let m = timer_machine("timer-key-variable");
    write_config(
        &m,
        &[
            "embedder.url = https://embed.example.com",
            "embedder.model = m",
            "embedder.token_env = MY_KEY",
        ],
    );
    let run = setup(&m, &[("MY_KEY", "s3cr3t-value")], &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "timer"),
        Some(
            "timer failed: the index timer cannot read the key variable MY_KEY; keep the key in a file (--embedder-token-file) or pass --no-timer"
        )
    );
    assert!(timer_gone(&m));
    assert!(!run.stdout.contains("s3cr3t-value"));
    assert!(!run.stderr.contains("s3cr3t-value"));
}

#[test]
fn timer_no_embedder_is_skipped() {
    let m = timer_machine("timer-no-embedder");
    let run = timer_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "timer"), Some("timer skipped: no embedder"));
    assert!(timer_gone(&m));
    assert!(manager_log(&m).is_empty());
}

#[test]
fn timer_turned_off_is_removed() {
    let m = timer_machine("timer-off");
    timer_config(&m);
    let skipped = timer_setup(&m, &["--no-timer"]);
    assert_eq!(skipped.code, 0, "{}", skipped.stderr);
    assert_eq!(step(&skipped, "timer"), Some("timer skipped: --no-timer"));
    assert!(manager_log(&m).is_empty());

    let installed = timer_setup(&m, &[]);
    assert_eq!(installed.code, 0, "{}", installed.stderr);
    assert!(timer_exists(&m));

    let removed = timer_setup(&m, &["--no-timer"]);
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    assert_eq!(step(&removed, "timer"), Some("timer removed: --no-timer"));
    assert!(timer_gone(&m));
    if cfg!(target_os = "macos") {
        assert_eq!(fakes::get(&m.state, "launchctl", "loaded"), "");
    } else {
        assert!(
            manager_log(&m)
                .iter()
                .any(|call| call == "--user disable --now bilbo-index.timer")
        );
        assert_eq!(fakes::get(&m.state, "systemctl", "enabled"), "");
    }

    let again = timer_setup(&m, &[]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert!(timer_exists(&m));
    write_config(&m, &["# keywords only"]);
    let none = timer_setup(&m, &[]);
    assert_eq!(none.code, 0, "{}", none.stderr);
    assert_eq!(step(&none, "timer"), Some("timer removed: no embedder"));
    assert!(timer_gone(&m));
}

#[test]
fn timer_moved_binary_is_updated() {
    let m = timer_machine("timer-moved");
    timer_config(&m);
    let first = timer_setup(&m, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert!(timer_text(&m).contains(&binary().display().to_string()));

    let (exe, _share) = packaged_binary(&m);
    let run = setup_with(&exe, &m, &["--yes", "--no-plugin"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "timer"), Some("timer updated: every 15 min"));
    let text = timer_text(&m);
    assert!(text.contains(&exe.display().to_string()), "{text}");
    assert!(!text.contains(&binary().display().to_string()), "{text}");
}

#[test]
fn timer_rerun_is_kept() {
    let m = timer_machine("timer-rerun");
    timer_config(&m);
    let first = timer_setup(&m, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let calls = manager_log(&m).len();
    let stamps = |m: &Machine| -> Vec<std::time::SystemTime> {
        timer_files(m)
            .iter()
            .map(|file| std::fs::metadata(file).unwrap().modified().unwrap())
            .collect()
    };
    let before = stamps(&m);
    let second = timer_setup(&m, &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(step(&second, "timer"), Some("timer kept"));
    assert_eq!(stamps(&m), before);
    let log = manager_log(&m);
    let changing = &log[calls..];
    assert!(
        changing
            .iter()
            .all(|call| call.ends_with("is-system-running")),
        "{changing:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn timer_launchctl_missing_fails() {
    let m = machine("timer-no-launchctl");
    timer_config(&m);
    let run = timer_setup(&m, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "timer"),
        Some("timer failed: launchctl not found on PATH")
    );
    assert!(timer_gone(&m));
}

#[cfg(target_os = "macos")]
#[test]
fn timer_bootstrap_failure_fails_the_step() {
    let m = timer_machine("timer-bootstrap");
    timer_config(&m);
    fakes::set(
        &m.state,
        "launchctl",
        "fail-bootstrap",
        "Bootstrap failed: 5: Input/output error",
    );
    let run = timer_setup(&m, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "timer"),
        Some(
            format!(
                "timer failed: launchctl bootstrap gui/{} {} failed: Bootstrap failed: 5: Input/output error",
                uid(),
                timer_files(&m)[0].display()
            )
            .as_str()
        )
    );
}

#[cfg(target_os = "linux")]
#[test]
fn timer_no_user_session_is_skipped() {
    let m = timer_machine("timer-no-session");
    timer_config(&m);
    fakes::set(&m.state, "systemctl", "state", "offline");
    let run = timer_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "timer"),
        Some("timer skipped: no systemd user session")
    );
    assert!(timer_gone(&m));
    assert_eq!(manager_log(&m), ["--user is-system-running"]);
}

#[cfg(target_os = "linux")]
#[test]
fn timer_systemctl_missing_is_skipped() {
    let m = machine("timer-no-systemctl");
    timer_config(&m);
    let run = timer_setup(&m, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "timer"),
        Some("timer skipped: no systemd user session")
    );
    assert!(timer_gone(&m));
}

// ---------------------------------------------------------------------------
// watch step

/// `setup --yes --no-plugin` with the watcher wanted, then `extra`.
fn watch_run(m: &Machine, env: &[(&str, &str)], extra: &[&str]) -> Run {
    let mut args = vec!["--yes", "--no-plugin"];
    args.extend(extra);
    watch_setup(m, env, &args)
}

fn watching(m: &Machine) -> String {
    format!("watch installed: watching {}/notes", root(m).display())
}

#[test]
fn watch_is_installed_without_an_embedder() {
    let m = timer_machine("watch-fresh");
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "timer"), Some("timer skipped: no embedder"));
    assert_eq!(step(&run, "watch"), Some(watching(&m).as_str()));
    assert!(timer_gone(&m));
    assert!(watch_exists(&m));
    let text = watch_text(&m);
    let program = binary().display().to_string();
    assert!(text.contains(&program), "{text}");
    assert!(text.contains("watch"), "{text}");
    assert!(
        text.contains(
            &m.home
                .join(".local/state/bilbo/watch.log")
                .display()
                .to_string()
        ),
        "{text}"
    );
    assert!(m.state.join("watch.started").exists());
    assert!(m.home.join(".local/state/bilbo").is_dir());
}

#[test]
fn watch_rerun_is_kept_and_a_moved_binary_updates_it() {
    let m = timer_machine("watch-rerun");
    assert_eq!(watch_run(&m, &[], &[]).code, 0);
    let calls = manager_log(&m).len();
    let second = watch_run(&m, &[], &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(step(&second, "watch"), Some("watch kept"));
    assert!(
        manager_log(&m)[calls..]
            .iter()
            .all(|call| call.ends_with("is-system-running")),
        "{:?}",
        manager_log(&m)
    );

    let (exe, _share) = packaged_binary(&m);
    let run = setup_with(&exe, &m, &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "watch"),
        Some(format!("watch updated: watching {}/notes", root(&m).display()).as_str())
    );
    let text = watch_text(&m);
    assert!(text.contains(&exe.display().to_string()), "{text}");
}

#[test]
fn watch_no_watch_is_skipped_and_removes_an_installed_one() {
    let m = timer_machine("watch-off");
    let skipped = watch_run(&m, &[], &["--no-watch"]);
    assert_eq!(skipped.code, 0, "{}", skipped.stderr);
    assert_eq!(step(&skipped, "watch"), Some("watch skipped: --no-watch"));
    assert!(watch_gone(&m));
    assert!(manager_log(&m).is_empty());

    assert_eq!(watch_run(&m, &[], &[]).code, 0);
    assert!(watch_exists(&m));
    let removed = watch_run(&m, &[], &["--no-watch"]);
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    assert_eq!(step(&removed, "watch"), Some("watch removed: --no-watch"));
    assert!(watch_gone(&m));
    if cfg!(target_os = "macos") {
        assert_eq!(fakes::get(&m.state, "launchctl", "loaded-watch"), "");
    } else {
        assert!(
            manager_log(&m)
                .iter()
                .any(|call| call == "--user disable --now bilbo-watch.service")
        );
        assert_eq!(fakes::get(&m.state, "systemctl", "watch-enabled"), "");
    }
}

#[test]
fn watch_loads_and_unloads_without_touching_the_timer() {
    let m = timer_machine("watch-loaded");
    timer_config(&m);
    assert_eq!(watch_run(&m, &[], &[]).code, 0);
    assert!(timer_exists(&m) && watch_exists(&m));
    if cfg!(target_os = "macos") {
        assert_ne!(fakes::get(&m.state, "launchctl", "loaded"), "");
        assert_ne!(fakes::get(&m.state, "launchctl", "loaded-watch"), "");
    } else {
        assert_eq!(fakes::get(&m.state, "systemctl", "enabled"), "enabled");
        assert_eq!(
            fakes::get(&m.state, "systemctl", "watch-enabled"),
            "enabled"
        );
    }
    let removed = watch_run(&m, &[], &["--no-watch"]);
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    assert!(timer_exists(&m));
    assert!(watch_gone(&m));
}

#[test]
fn watch_a_key_variable_does_not_stop_it() {
    let m = timer_machine("watch-key-variable");
    write_config(
        &m,
        &[
            "embedder.url = https://embed.example.com",
            "embedder.model = m",
            "embedder.token_env = MY_KEY",
        ],
    );
    let run = watch_run(&m, &[("MY_KEY", "s3cr3t-value")], &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        step(&run, "timer").is_some_and(|l| l.starts_with("timer failed:") && l.contains("MY_KEY")),
        "{}",
        run.stdout
    );
    assert_eq!(step(&run, "watch"), Some(watching(&m).as_str()));
    assert!(timer_gone(&m));
    let text = watch_text(&m);
    assert!(
        !text.contains("MY_KEY") && !text.contains("s3cr3t-value"),
        "{text}"
    );
}

#[test]
fn watch_carries_setups_locations() {
    let m = timer_machine("watch-locations");
    let data = m.dir.path().join("data");
    let cache = m.dir.path().join("cache");
    let (data, cache) = (data.to_str().unwrap(), cache.to_str().unwrap());
    let run = watch_run(&m, &[("BILBO_HOME", data), ("XDG_CACHE_HOME", cache)], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "watch"),
        Some(format!("watch installed: watching {data}/notes").as_str())
    );
    let text = watch_text(&m);
    assert!(text.contains(&variable_text("BILBO_HOME", data)), "{text}");
    assert!(
        text.contains(&variable_text("XDG_CACHE_HOME", cache)),
        "{text}"
    );
    assert!(!text.contains("XDG_STATE_HOME"), "{text}");
    assert!(!text.contains("PATH"), "{text}");
}

#[cfg(target_os = "macos")]
#[test]
fn watch_installing_without_the_service_manager_fails() {
    let m = machine("watch-no-launchctl");
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "watch"),
        Some("watch failed: launchctl not found on PATH")
    );
    assert!(watch_gone(&m));
}

#[cfg(target_os = "linux")]
#[test]
fn watch_without_a_user_session_is_skipped() {
    let m = timer_machine("watch-no-session");
    fakes::set(&m.state, "systemctl", "state", "offline");
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "watch"),
        Some("watch skipped: no systemd user session")
    );
    assert!(watch_gone(&m));
}

#[test]
fn watch_bootstrap_failure_fails_the_step() {
    let m = timer_machine("watch-load-fails");
    if cfg!(target_os = "macos") {
        fakes::set(
            &m.state,
            "launchctl",
            "fail-bootstrap-watch",
            "Bootstrap failed: 5: Input/output error",
        );
    } else {
        fakes::set(
            &m.state,
            "systemctl",
            "fail-enable",
            "Failed to enable unit: boom",
        );
    }
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        step(&run, "watch").is_some_and(|l| l.starts_with("watch failed: ")),
        "{}",
        run.stdout
    );
    assert!(watch_gone(&m), "a failed load leaves no files");
}

#[test]
fn watch_removal_without_the_manager_fails_and_keeps_the_files() {
    let m = timer_machine("watch-remove-no-manager");
    assert_eq!(watch_run(&m, &[], &[]).code, 0);
    let manager = if cfg!(target_os = "macos") {
        "launchctl"
    } else {
        "systemctl"
    };
    let empty = m.dir.path().join("empty");
    let env = [("PATH", empty.to_str().unwrap())];
    let remove = setup(&m, &env, &["--remove", "--yes"]);
    assert_eq!(remove.code, 1, "{}", remove.stderr);
    assert_eq!(
        step(&remove, "watch"),
        Some(format!("watch failed: {manager} not found on PATH").as_str())
    );
    assert!(watch_exists(&m));
    let off = watch_run(&m, &env, &["--no-watch"]);
    assert_eq!(off.code, 1, "{}", off.stderr);
    assert_eq!(
        step(&off, "watch"),
        Some(format!("watch failed: {manager} not found on PATH").as_str())
    );
    assert!(watch_exists(&m));
}

#[test]
fn watch_flag_takes_no_value_and_no_remove() {
    let m = machine("watch-flag-usage");
    usage_error(
        &m,
        &["--yes", "--no-watch=true"],
        "--no-watch takes no value",
    );
    usage_error(
        &m,
        &["--remove", "--no-watch"],
        "--remove cannot be used with --no-watch",
    );
    usage_error(
        &m,
        &["--interactive", "--no-watch"],
        "--interactive cannot be used with --no-watch; it answers a wizard question",
    );
}

// ---------------------------------------------------------------------------
// Remove

fn token_path(m: &Machine) -> PathBuf {
    m.home.join(".config/bilbo/token")
}

/// Every step installed: a config with a key file, both plugins and the timer.
fn installed_machine(name: &str) -> Machine {
    let m = agents_machine(name, &["claude", "codex", TIMER_TOOL]);
    std::fs::create_dir_all(token_path(&m).parent().unwrap()).unwrap();
    std::fs::write(token_path(&m), "secret-key\n").unwrap();
    write_config(
        &m,
        &[
            "embedder.url = http://127.0.0.1:9",
            "embedder.model = m",
            &format!("embedder.token_file = {}", token_path(&m).display()),
        ],
    );
    let run = watch_setup(&m, &[], &["--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(timer_exists(&m));
    assert!(watch_exists(&m));
    m
}

#[test]
fn remove_an_install() {
    let m = installed_machine("remove-install");
    let history = root(&m).join(".bilbo/history/notes");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(history.join("x.jsonl"), "{}\n").unwrap();
    let run = setup(&m, &[], &["--remove", "--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run),
        [
            format!("store skipped: kept {}/notes", root(&m).display()),
            format!("config skipped: kept {}", config_path(&m).display()),
            format!("key skipped: kept {}", token_path(&m).display()),
            "model skipped: no model".to_string(),
            "server skipped: not installed".to_string(),
            "claude removed".to_string(),
            "codex removed".to_string(),
            "hook removed".to_string(),
            "timer removed".to_string(),
            "watch removed".to_string(),
        ]
    );
    assert!(
        history.join("x.jsonl").is_file(),
        "history survives --remove"
    );
    assert!(timer_gone(&m));
    assert!(watch_gone(&m));
    for tool in ["claude", "codex"] {
        assert_eq!(fakes::get(&m.state, tool, "marketplace"), "");
        assert_eq!(fakes::get(&m.state, tool, "plugin"), "");
    }
    if cfg!(target_os = "macos") {
        assert_eq!(fakes::get(&m.state, "launchctl", "loaded"), "");
    } else {
        assert_eq!(fakes::get(&m.state, "systemctl", "enabled"), "");
    }
}

#[test]
fn remove_nothing_installed() {
    let m = agents_machine("remove-nothing", &["claude", "codex", TIMER_TOOL]);
    let run = setup(&m, &[], &["--remove", "--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run),
        [
            "store skipped: no store",
            "config skipped: no config",
            "key skipped: no key file",
            "model skipped: no model",
            "server skipped: not installed",
            "claude skipped: not installed",
            "codex skipped: not installed",
            "hook skipped: not trusted",
            "timer skipped: not installed",
            "watch skipped: not installed",
        ]
    );
    assert!(
        fakes::log(&m.state, "claude")
            .iter()
            .all(|call| call.ends_with("list --json"))
    );

    let bare = machine("remove-no-tools");
    let run = setup(&bare, &[], &["--remove", "--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run)[5..8],
        [
            "claude skipped: not found",
            "codex skipped: not found",
            "hook skipped: not found"
        ]
    );
}

#[test]
fn remove_with_setup_flags_is_a_usage_error() {
    let m = machine("remove-flags");
    usage_error(
        &m,
        &["--remove", "--no-timer"],
        "--remove cannot be used with --no-timer",
    );
    usage_error(
        &m,
        &["--remove", "--yes", "--plugin-source", "x/y#v1"],
        "--remove cannot be used with --plugin-source",
    );
}

#[test]
fn remove_accepts_tool_paths() {
    let m = machine("remove-paths");
    let tools = m.dir.path().join("tools");
    fakes::install(&tools, &m.state, &["claude"]);
    fakes::set(
        &m.state,
        "claude",
        "marketplace",
        format!("github delucca/bilbo v{VERSION}").as_str(),
    );
    fakes::set(&m.state, "claude", "plugin", "true");
    let claude = tools.join("claude");
    let run = setup(
        &m,
        &[],
        &["--remove", "--yes", "--claude", claude.to_str().unwrap()],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines = lines(&run);
    assert!(lines.contains(&"claude removed"), "{lines:?}");
    assert!(lines.contains(&"codex skipped: not found"), "{lines:?}");
    assert_eq!(fakes::get(&m.state, "claude", "marketplace"), "");
}

#[test]
fn remove_keeps_store_config_and_key() {
    let m = installed_machine("remove-keeps");
    let note = root(&m).join("notes/n.md");
    std::fs::write(&note, "kept\n").unwrap();
    let files = [note, config_path(&m), token_path(&m)];
    let before: Vec<_> = files.iter().map(|f| std::fs::read(f).unwrap()).collect();
    let run = setup(&m, &[], &["--remove", "--yes"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let after: Vec<_> = files.iter().map(|f| std::fs::read(f).unwrap()).collect();
    assert_eq!(after, before);
    for line in &lines(&run)[..3] {
        assert!(line.contains("skipped: kept /"), "{line}");
    }
    assert!(!run.stdout.contains("secret-key"));
}

#[test]
fn timer_removal_without_the_manager_fails_and_keeps_the_files() {
    let m = timer_machine("timer-remove-no-manager");
    timer_config(&m);
    assert_eq!(timer_setup(&m, &[]).code, 0);
    assert!(timer_exists(&m));
    let run = setup(
        &m,
        &[("PATH", m.dir.path().join("empty").to_str().unwrap())],
        &["--yes", "--no-plugin", "--no-timer"],
    );
    let manager = if cfg!(target_os = "macos") {
        "launchctl"
    } else {
        "systemctl"
    };
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "timer"),
        Some(format!("timer failed: {manager} not found on PATH").as_str())
    );
    assert!(timer_exists(&m), "a rerun with the tool can finish");

    let remove = setup(
        &m,
        &[("PATH", m.dir.path().join("empty").to_str().unwrap())],
        &["--remove", "--yes"],
    );
    assert_eq!(remove.code, 1, "{}", remove.stderr);
    assert_eq!(
        step(&remove, "timer"),
        Some(format!("timer failed: {manager} not found on PATH").as_str())
    );
    assert!(timer_exists(&m));

    let done = timer_setup(&m, &["--no-timer"]);
    assert_eq!(done.code, 0, "{}", done.stderr);
    assert!(timer_gone(&m));
}

#[test]
fn remove_with_a_relative_config_path_is_refused() {
    let m = machine("remove-relative-config");
    let before = snapshot(m.dir.path());
    let run = setup(
        &m,
        &[("BILBO_CONFIG", "rel/config")],
        &["--remove", "--yes"],
    );
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("BILBO_CONFIG must be an absolute path"),
        "{}",
        run.stderr
    );
    assert_eq!(snapshot(m.dir.path()), before);
}

#[test]
fn config_dangling_link_is_managed_elsewhere_at_both_paths() {
    let m = machine("config-dangling");
    let gone = m.dir.path().join("gone");
    let default = config_path(&m);
    std::fs::create_dir_all(default.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&gone, &default).unwrap();
    let explicit = m.dir.path().join("explicit-config");
    std::os::unix::fs::symlink(&gone, &explicit).unwrap();
    let args = ["--yes", "--no-plugin", "--no-timer"];
    let expected = format!("config kept: managed elsewhere ({})", gone.display());
    for env in [vec![], vec![("BILBO_CONFIG", explicit.to_str().unwrap())]] {
        let run = setup(&m, &env, &args);
        assert_eq!(run.code, 0, "{env:?}: {}", run.stderr);
        assert_eq!(lines(&run)[1], expected, "{env:?}");
    }
}

// ---------------------------------------------------------------------------
// The local embedder's plan

/// A machine with the manager fake and, when `llama`, a `llama-server` fake on PATH.
fn local_plan_machine(name: &str, llama: bool) -> Machine {
    let m = machine(name);
    let mut tools = vec![TIMER_TOOL];
    if llama {
        tools.push("llama-server");
    }
    fakes::install(&m.bin, &m.state, &tools);
    m
}

/// `setup --yes --embedder-local` on `port`, expecting a refusal that leaves HOME as it was.
fn local_plan_refused(m: &Machine, port: u16, code: i32, message: &str) -> Run {
    let before = snapshot(&m.home);
    let port = port.to_string();
    let run = setup(
        m,
        &[],
        &[
            "--yes",
            "--no-plugin",
            "--no-timer",
            "--embedder-local",
            "--embedder-port",
            &port,
        ],
    );
    assert_eq!(run.code, code, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains(message), "{}", run.stderr);
    assert_eq!(snapshot(&m.home), before, "the refusal wrote something");
    run
}

#[test]
fn local_plan_llama_server_not_found() {
    let m = local_plan_machine("setup-local-plan-no-llama", false);
    let run = local_plan_refused(&m, unused_port(), 1, "llama-server not found on PATH");
    assert!(run.stderr.contains("brew install llama.cpp"));
    assert!(run.stderr.contains("--llama-server"));
}

#[test]
fn local_plan_port_in_use() {
    let m = local_plan_machine("setup-local-plan-port", true);
    let fake = Fake::start(1024);
    let port = fake.port();
    let run = local_plan_refused(
        &m,
        port,
        1,
        &format!("127.0.0.1:{port} is already in use; pick another port with --embedder-port"),
    );
    assert!(!run.stderr.contains("llama-server not found"));
}

#[test]
fn local_plan_managed_config_elsewhere_is_refused() {
    let m = local_plan_machine("setup-local-plan-managed", true);
    let target = m.dir.path().join("managed-config");
    std::fs::write(
        &target,
        "embedder.url = http://embedder.example:8081\nembedder.model = qwen3-embedding-0.6b\n",
    )
    .unwrap();
    std::fs::create_dir_all(config_path(&m).parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, config_path(&m)).unwrap();
    local_plan_refused(
        &m,
        unused_port(),
        2,
        &format!(
            "{} is managed elsewhere; change the embedder there, not with --embedder-local",
            config_path(&m).display()
        ),
    );
}

#[test]
fn local_plan_present_config_with_another_embedder_is_refused() {
    let m = local_plan_machine("setup-local-plan-present", true);
    write_config(
        &m,
        &[
            "embedder.url = http://embedder.example:8081",
            "embedder.model = qwen3-embedding-0.6b",
        ],
    );
    local_plan_refused(
        &m,
        unused_port(),
        1,
        &format!(
            "{} already sets other embedder settings",
            config_path(&m).display()
        ),
    );
}

#[cfg(target_os = "macos")]
#[test]
fn local_plan_launchctl_missing() {
    let m = machine("setup-local-plan-no-launchctl");
    fakes::install(&m.bin, &m.state, &["llama-server"]);
    local_plan_refused(
        &m,
        unused_port(),
        1,
        "the local embedder needs launchctl, which is not on PATH",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn local_plan_no_user_session() {
    let m = local_plan_machine("setup-local-plan-no-session", true);
    fakes::set(&m.state, "systemctl", "state", "offline");
    local_plan_refused(
        &m,
        unused_port(),
        1,
        "the local embedder needs a systemd user session",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn local_plan_systemctl_missing() {
    let m = machine("setup-local-plan-no-systemctl");
    fakes::install(&m.bin, &m.state, &["llama-server"]);
    local_plan_refused(
        &m,
        unused_port(),
        1,
        "the local embedder needs a systemd user session",
    );
}

// ---------------------------------------------------------------------------
// Prepare: download, service, readiness, check

const LOCAL_MODEL: &str = "qwen3-embedding-0.6b";

/// Where the pinned model goes under the machine's cache folder.
fn model_file(m: &Machine) -> PathBuf {
    m.home
        .join(".cache/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf")
}

/// A sparse file of the pinned size, so planning finds the model and nothing downloads.
fn place_model(m: &Machine) {
    let path = model_file(m);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(path)
        .unwrap()
        .set_len(639_150_592)
        .unwrap();
}

/// The fake `llama-server` in `folder`; returns its path.
fn llama(m: &Machine, folder: &Path) -> PathBuf {
    fakes::install(folder, &m.state, &["llama-server"]);
    folder.join("llama-server")
}

/// The embedder's service file on this platform.
fn service_file(m: &Machine) -> PathBuf {
    if cfg!(target_os = "macos") {
        m.home
            .join("Library/LaunchAgents/io.github.delucca.bilbo.embedder.plist")
    } else {
        m.home.join(".config/systemd/user/bilbo-embedder.service")
    }
}

/// The manager fake, a fake `llama-server` on PATH, the model in place and a fake embedder that
/// comes up when the manager starts the service.
fn local_machine(name: &str) -> (Machine, Fake) {
    let m = machine(name);
    fakes::install(&m.bin, &m.state, &[TIMER_TOOL]);
    llama(&m, &m.bin);
    place_model(&m);
    let fake = Fake::start_when(1024, &m.state.join("embedder.started"));
    (m, fake)
}

/// `setup --yes --no-plugin --no-timer --embedder-local` on the fake's port, then `extra`.
fn local_setup(m: &Machine, fake: &Fake, extra: &[&str]) -> Run {
    let port = fake.port().to_string();
    let mut args = vec![
        "--yes",
        "--no-plugin",
        "--no-timer",
        "--embedder-local",
        "--embedder-port",
        &port,
    ];
    args.extend(extra);
    setup(m, &[], &args)
}

#[test]
fn a_triggered_fake_refuses_until_the_trigger_and_drops_cleanly() {
    let dir = TempDir::new("fake-trigger");
    let trigger = dir.path().join("started");
    let fake = Fake::start_when(4, &trigger);
    let addr = format!("127.0.0.1:{}", fake.port()).parse().unwrap();
    let probe = || std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(1));
    assert!(probe().is_err());
    std::fs::write(&trigger, "").unwrap();
    let mut up = false;
    for _ in 0..500 {
        if probe().is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(up);
    drop(fake);
    // A fake dropped between the trigger and the bind still joins.
    for _ in 0..20 {
        let trigger = dir.path().join("again");
        let fake = Fake::start_when(4, &trigger);
        std::fs::write(&trigger, "").unwrap();
        drop(fake);
        std::fs::remove_file(&trigger).unwrap();
    }
}

/// The manager calls that change something: everything but the session probe.
fn changing_calls(m: &Machine) -> Vec<String> {
    manager_log(m)
        .into_iter()
        .filter(|call| call != "--user is-system-running")
        .collect()
}

fn embed_requests(fake: &Fake) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path == "/v1/embeddings")
        .count()
}

#[test]
fn local_fresh_run_reports_every_step() {
    let (m, fake) = local_machine("setup-local-fresh");
    let port = fake.port().to_string();
    let run = watch_setup(
        &m,
        &[],
        &[
            "--yes",
            "--no-plugin",
            "--embedder-local",
            "--embedder-port",
            &port,
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "nothing downloads: {}", run.stderr);
    assert_eq!(
        lines(&run),
        [
            format!("store created: {}/notes", root(&m).display()),
            format!("config written: {}", config_path(&m).display()),
            "key skipped: local embedder".to_string(),
            format!("model kept: {}", model_file(&m).display()),
            format!("server installed: 127.0.0.1:{port}"),
            "embedder ok: 1024 dimensions".to_string(),
            "claude skipped: --no-plugin".to_string(),
            "codex skipped: --no-plugin".to_string(),
            "hook skipped: no codex plugin".to_string(),
            "timer installed: every 15 min".to_string(),
            format!("watch installed: watching {}/notes", root(&m).display()),
            "sync skipped: no scope syncs".to_string(),
        ]
    );
    let text = std::fs::read_to_string(config_path(&m)).unwrap();
    assert!(
        text.contains(&format!("embedder.url = http://127.0.0.1:{port}\n")),
        "{text}"
    );
    assert!(
        text.contains(&format!("embedder.model = {LOCAL_MODEL}\n")),
        "{text}"
    );
    assert!(
        text.contains(
            "embedder.query_prefix = \"Instruct: Given a question, retrieve notes that answer it\\nQuery: \"\n"
        ),
        "{text}"
    );
    assert!(service_file(&m).exists());
}

#[test]
fn local_service_file_runs_llama_server_on_loopback() {
    let (m, fake) = local_machine("setup-local-service-file");
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(service_file(&m)).unwrap();
    let port = fake.port().to_string();
    let log = m.home.join(".local/state/bilbo/embedder.log");
    for needle in [
        m.bin.join("llama-server").display().to_string(),
        model_file(&m).display().to_string(),
        "127.0.0.1".to_string(),
        port.clone(),
        log.display().to_string(),
    ] {
        assert!(text.contains(&needle), "{needle} is not in {text}");
    }
    #[cfg(target_os = "macos")]
    {
        assert!(text.contains("<key>KeepAlive</key>\n\t<true/>"), "{text}");
        assert!(text.contains("<key>RunAtLoad</key>\n\t<true/>"), "{text}");
        let uid = uid();
        assert_eq!(
            manager_log(&m),
            [
                format!("bootout --wait gui/{uid}/io.github.delucca.bilbo.embedder"),
                format!("bootstrap gui/{uid} {}", service_file(&m).display()),
            ]
        );
    }
    #[cfg(target_os = "linux")]
    {
        assert!(text.contains("Restart=on-failure"), "{text}");
        assert!(text.contains("WantedBy=default.target"), "{text}");
        assert_eq!(
            changing_calls(&m),
            [
                "--user daemon-reload",
                "--user enable bilbo-embedder.service",
                "--user restart bilbo-embedder.service",
            ]
        );
    }
}

#[test]
fn local_chosen_port() {
    let (m, fake) = local_machine("setup-local-port");
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(config_path(&m)).unwrap();
    assert!(
        text.contains(&format!(
            "embedder.url = http://127.0.0.1:{}\n",
            fake.port()
        )),
        "{text}"
    );
    assert_ne!(fake.port(), 8737);
}

#[test]
fn local_waits_while_the_model_loads() {
    let (m, fake) = local_machine("setup-local-loading");
    fake.loading(3);
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(fake.health_checks() >= 4, "{}", fake.health_checks());
    assert_eq!(embed_requests(&fake), 1);
    assert!(lines(&run).contains(&"embedder ok: 1024 dimensions"));
}

#[test]
fn local_failed_check_writes_nothing_else() {
    let (m, fake) = local_machine("setup-local-check-fails");
    fake.status(500);
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stderr.contains(&fake.url), "{}", run.stderr);
    assert!(run.stderr.contains("500"), "{}", run.stderr);
    assert!(run.stderr.contains("setup changed nothing else"));
    assert_eq!(
        lines(&run),
        [
            format!("model kept: {}", model_file(&m).display()),
            format!(
                "server failed: {}; the service was removed; see {}",
                run.stderr
                    .lines()
                    .next()
                    .unwrap()
                    .trim_start_matches("bilbo: ")
                    .trim_end_matches("; setup changed nothing else"),
                m.home.join(".local/state/bilbo/embedder.log").display()
            ),
        ]
    );
    assert!(model_file(&m).exists());
    assert!(!service_file(&m).exists());
    assert!(!root(&m).exists());
    assert!(!config_path(&m).exists());
    let calls = manager_log(&m);
    if cfg!(target_os = "macos") {
        let last = calls.last().unwrap();
        assert!(
            last.starts_with("bootout ") && last.ends_with("io.github.delucca.bilbo.embedder"),
            "{calls:?}"
        );
    } else {
        assert!(
            calls.contains(&"--user disable --now bilbo-embedder.service".to_string()),
            "{calls:?}"
        );
    }
}

#[test]
fn local_failed_load_writes_nothing_else() {
    let (m, fake) = local_machine("setup-local-load-fails");
    if cfg!(target_os = "macos") {
        fakes::set(
            &m.state,
            "launchctl",
            "fail-bootstrap-embedder",
            "Bootstrap failed: 5: Input/output error",
        );
    } else {
        fakes::set(
            &m.state,
            "systemctl",
            "fail-restart-bilbo-embedder.service",
            "Failed to restart bilbo-embedder.service",
        );
    }
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(report.len(), 2, "{report:?}");
    assert_eq!(
        report[0],
        format!("model kept: {}", model_file(&m).display())
    );
    assert!(report[1].starts_with("server failed: "), "{}", report[1]);
    assert!(
        report[1].ends_with(&format!(
            "; see {}",
            m.home.join(".local/state/bilbo/embedder.log").display()
        )),
        "{}",
        report[1]
    );
    assert!(run.stderr.contains("setup changed nothing else"));
    assert!(!service_file(&m).exists());
    assert!(!root(&m).exists());
    assert!(!config_path(&m).exists());
    assert_eq!(embed_requests(&fake), 0);
}

#[test]
fn local_rerun_keeps_everything() {
    let (m, fake) = local_machine("setup-local-rerun");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let mtime = std::fs::metadata(service_file(&m))
        .unwrap()
        .modified()
        .unwrap();
    let calls = changing_calls(&m);
    let embeds = embed_requests(&fake);
    let second = local_setup(&m, &fake, &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    let report = lines(&second);
    assert_eq!(
        report[3],
        format!("model kept: {}", model_file(&m).display())
    );
    assert_eq!(report[4], format!("server kept: 127.0.0.1:{}", fake.port()));
    assert_eq!(
        std::fs::metadata(service_file(&m))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
    assert_eq!(changing_calls(&m), calls, "no state-changing command");
    assert_eq!(embed_requests(&fake), embeds, "no embed request");
}

#[test]
fn local_moved_llama_server_updates_the_service() {
    let (m, fake) = local_machine("setup-local-moved");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let before = changing_calls(&m).len();
    let other = llama(&m, &m.dir.path().join("elsewhere"));
    let second = local_setup(&m, &fake, &["--llama-server", other.to_str().unwrap()]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(
        lines(&second)[4],
        format!("server updated: 127.0.0.1:{}", fake.port())
    );
    let text = std::fs::read_to_string(service_file(&m)).unwrap();
    assert!(text.contains(&other.display().to_string()), "{text}");
    assert!(
        !text.contains(&m.bin.join("llama-server").display().to_string()),
        "{text}"
    );
    let after = changing_calls(&m);
    assert!(after.len() > before, "{after:?}");
    if cfg!(target_os = "macos") {
        assert!(after[before].starts_with("bootout "), "{after:?}");
        assert!(after[before + 1].starts_with("bootstrap "), "{after:?}");
    } else {
        assert!(
            after[before..].contains(&"--user restart bilbo-embedder.service".to_string()),
            "{after:?}"
        );
    }
}

#[test]
fn local_found_on_path_keeps_the_path() {
    let (m, fake) = local_machine("setup-local-symlink");
    let real = llama(&m, &m.dir.path().join("real"));
    std::fs::remove_file(m.bin.join("llama-server")).unwrap();
    std::os::unix::fs::symlink(&real, m.bin.join("llama-server")).unwrap();
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(service_file(&m)).unwrap();
    assert!(
        text.contains(&m.bin.join("llama-server").display().to_string()),
        "{text}"
    );
    assert!(!text.contains(&real.display().to_string()), "{text}");
}

#[test]
fn local_managed_config_installs_model_and_server() {
    let (m, fake) = local_machine("setup-local-managed");
    let target = m.dir.path().join("managed-config");
    std::fs::write(
        &target,
        format!(
            "embedder.url = http://127.0.0.1:{}\nembedder.model = {LOCAL_MODEL}\n",
            fake.port()
        ),
    )
    .unwrap();
    std::fs::create_dir_all(config_path(&m).parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, config_path(&m)).unwrap();
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(
        report[1],
        format!("config kept: managed elsewhere ({})", target.display())
    );
    assert_eq!(
        report[3],
        format!("model kept: {}", model_file(&m).display())
    );
    assert_eq!(
        report[4],
        format!("server installed: 127.0.0.1:{}", fake.port())
    );
    assert_eq!(report[5], "embedder ok: 1024 dimensions");
    assert!(service_file(&m).exists());
    assert_eq!(std::fs::read_link(config_path(&m)).unwrap(), target);
}

#[test]
fn local_bilbos_own_server_is_not_a_conflict() {
    let (m, fake) = local_machine("setup-local-own-server");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let second = local_setup(&m, &fake, &[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(
        !second.stderr.contains("already in use"),
        "{}",
        second.stderr
    );
    assert_eq!(
        lines(&second)[4],
        format!("server kept: 127.0.0.1:{}", fake.port())
    );
}

#[test]
fn remove_the_local_embedder() {
    let (m, fake) = local_machine("remove-local");
    let run = local_setup(&m, &fake, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(service_file(&m).exists());
    let before = changing_calls(&m).len();
    let removed = setup(&m, &[], &["--remove", "--yes"]);
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    let report = lines(&removed);
    assert_eq!(
        report[3],
        format!("model skipped: kept {}", model_file(&m).display())
    );
    assert_eq!(report[4], "server removed");
    assert!(!service_file(&m).exists());
    assert!(model_file(&m).exists());
    assert!(
        changing_calls(&m)[before..]
            .iter()
            .any(|call| call.contains("embedder")),
        "{:?}",
        manager_log(&m)
    );
    if cfg!(target_os = "macos") {
        assert_eq!(fakes::get(&m.state, "launchctl", "loaded-embedder"), "");
    } else {
        assert_eq!(fakes::get(&m.state, "systemctl", "embedder-enabled"), "");
    }
}

/// Points the config at a file that sets `embedder.url = http://embedder.example:8081` and the local model.
fn move_config_elsewhere(m: &Machine) {
    let target = m.dir.path().join("managed-config");
    std::fs::write(
        &target,
        "embedder.url = http://embedder.example:8081\nembedder.model = qwen3-embedding-0.6b\n",
    )
    .unwrap();
    std::fs::remove_file(config_path(m)).unwrap();
    std::os::unix::fs::symlink(&target, config_path(m)).unwrap();
}

#[test]
fn remove_unused_service_when_the_config_moves() {
    let (m, fake) = local_machine("remove-unused-moves");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert!(service_file(&m).exists());
    move_config_elsewhere(&m);
    let before = changing_calls(&m).len();
    let run = setup(&m, &[], &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(report[3], "model skipped: not local");
    assert_eq!(report[4], "server removed: not local");
    assert!(!service_file(&m).exists());
    assert!(model_file(&m).exists());
    assert!(
        changing_calls(&m)[before..]
            .iter()
            .any(|call| call.contains("embedder")),
        "{:?}",
        manager_log(&m)
    );
    let again = setup(&m, &[], &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert_eq!(lines(&again)[4], "server skipped: not local");
}

#[test]
fn remove_unused_service_not_for_a_local_config() {
    let (m, fake) = local_machine("remove-unused-local-config");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let mtime = std::fs::metadata(service_file(&m))
        .unwrap()
        .modified()
        .unwrap();
    let calls = changing_calls(&m);
    let run = setup(&m, &[], &["--yes", "--no-plugin", "--no-timer"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let report = lines(&run);
    assert_eq!(report[3], "model skipped: not asked");
    assert_eq!(report[4], "server skipped: not asked");
    assert_eq!(
        std::fs::metadata(service_file(&m))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
    assert_eq!(changing_calls(&m), calls, "no state-changing command");
}

#[test]
fn remove_unused_service_without_the_manager_fails() {
    let (m, fake) = local_machine("remove-unused-no-manager");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    move_config_elsewhere(&m);
    let run = setup(
        &m,
        &[("PATH", m.dir.path().join("empty").to_str().unwrap())],
        &["--yes", "--no-plugin", "--no-timer"],
    );
    let manager = if cfg!(target_os = "macos") {
        "launchctl"
    } else {
        "systemctl"
    };
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        lines(&run)[4],
        format!("server failed: {manager} not found on PATH")
    );
    assert!(service_file(&m).exists());
}

#[test]
fn remove_without_the_manager_keeps_the_service() {
    let (m, fake) = local_machine("remove-no-manager");
    let first = local_setup(&m, &fake, &[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let run = setup(
        &m,
        &[("PATH", m.dir.path().join("empty").to_str().unwrap())],
        &["--remove", "--yes"],
    );
    let manager = if cfg!(target_os = "macos") {
        "launchctl"
    } else {
        "systemctl"
    };
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        lines(&run)[4],
        format!("server failed: {manager} not found on PATH")
    );
    assert!(service_file(&m).exists());
}

// ---------------------------------------------------------------------------
// Sync step

const DEVICE_FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");

/// Puts the fixture keys of `who` where setup looks for them, with the modes git does not keep.
fn enrol(m: &Machine, who: &str) {
    let keys = m.home.join(".local/state/bilbo/keys");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o700)).unwrap();
    for file in ["owner.key", "device.key"] {
        let target = keys.join(file);
        std::fs::copy(Path::new(DEVICE_FIXTURES).join(who).join(file), &target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn scope_note(m: &Machine, name: &str, scope: Option<&str>) {
    let notes = root(m).join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let scope = scope.map_or(String::new(), |s| format!("scope: {s}\n"));
    std::fs::write(
        notes.join(format!("plan-{name}.md")),
        format!(
            "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02T14:23-03:00\n{scope}---\n\n# T\n\ntext\n"
        ),
    )
    .unwrap();
}

fn syncing_machine(name: &str, url: &str) -> Machine {
    let m = timer_machine(name);
    write_config(&m, &[&format!("scope.personal.sync = {url}")]);
    m
}

#[test]
fn sync_nothing_syncs() {
    let m = timer_machine("sync-none");
    write_config(&m, &["scope.personal.sync = off"]);
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(step(&run, "sync"), Some("sync skipped: no scope syncs"));
}

#[test]
fn sync_a_scope_reports_its_note_count() {
    let m = timer_machine("sync-ok");
    let folder = m.dir.path().join("bilbo");
    std::fs::create_dir_all(&folder).unwrap();
    let url = format!("file://{}", folder.display());
    write_config(&m, &[&format!("scope.personal.sync = {url}")]);
    enrol(&m, "rhosgobel");
    for i in 0..12 {
        scope_note(&m, &format!("n{i}"), Some("personal"));
    }
    scope_note(&m, "other", Some("work"));
    scope_note(&m, "bare", None);
    let before = snapshot(&folder);
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "sync"),
        Some(format!("sync ok: personal through {url} (12 notes)").as_str())
    );
    assert_eq!(snapshot(&folder), before, "the step wrote to the folder");
}

#[test]
fn sync_joins_the_scopes_with_a_comma() {
    let m = timer_machine("sync-two");
    let (a, b) = (m.dir.path().join("a"), m.dir.path().join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    write_config(
        &m,
        &[
            &format!("scope.personal.sync = file://{}", a.display()),
            &format!("scope.work.sync = file://{}", b.display()),
        ],
    );
    enrol(&m, "rhosgobel");
    scope_note(&m, "w", Some("work"));
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "sync"),
        Some(
            format!(
                "sync ok: personal through file://{} (0 notes), work through file://{} (1 notes)",
                a.display(),
                b.display()
            )
            .as_str()
        )
    );
}

#[test]
fn sync_without_a_device_key_is_skipped_and_exits_0() {
    let m = syncing_machine("sync-no-key", "file:///srv/bilbo");
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        step(&run, "sync"),
        Some("sync skipped: no device key; run bilbo device init in a terminal")
    );
}

#[test]
fn sync_without_the_watcher_fails() {
    let m = timer_machine("sync-no-watch");
    let folder = m.dir.path().join("bilbo");
    std::fs::create_dir_all(&folder).unwrap();
    write_config(
        &m,
        &[&format!(
            "scope.personal.sync = file://{}",
            folder.display()
        )],
    );
    enrol(&m, "rhosgobel");
    let run = watch_setup(&m, &[], &["--yes", "--no-plugin", "--no-watch"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "sync"),
        Some("sync failed: sync needs the watcher; drop --no-watch")
    );
}

#[test]
fn sync_into_a_missing_folder_fails_and_creates_nothing() {
    let m = syncing_machine("sync-missing", "file:///Volumes/usb/bilbo");
    enrol(&m, "rhosgobel");
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let line = step(&run, "sync").unwrap();
    assert!(
        line.starts_with("sync failed: file:///Volumes/usb/bilbo is not reachable: "),
        "{line}"
    );
    assert!(!Path::new("/Volumes/usb/bilbo").exists());
}

#[test]
fn sync_into_a_folder_that_takes_no_writes_fails() {
    let m = timer_machine("sync-readonly");
    let folder = m.dir.path().join("locked");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
    let url = format!("file://{}", folder.display());
    write_config(&m, &[&format!("scope.personal.sync = {url}")]);
    enrol(&m, "rhosgobel");
    let run = watch_run(&m, &[], &[]);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    // SAFETY: geteuid takes no arguments and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        step(&run, "sync"),
        Some(format!("sync failed: {url} is not reachable: the folder is not writable").as_str())
    );
}

#[test]
fn sync_with_a_damaged_key_fails_with_its_message() {
    let m = syncing_machine("sync-damaged", "file:///srv/bilbo");
    enrol(&m, "rhosgobel");
    let keys = m.home.join(".local/state/bilbo/keys");
    std::fs::set_permissions(
        keys.join("device.key"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let run = watch_run(&m, &[], &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        step(&run, "sync").unwrap().starts_with("sync failed: "),
        "{}",
        run.stdout
    );
}

#[test]
fn sync_a_failed_step_does_not_stop_the_rest() {
    let m = syncing_machine("sync-last", "https://relay.example");
    enrol(&m, "rhosgobel");
    let run = watch_run(&m, &[], &[]);
    let names: Vec<&str> = lines(&run)
        .iter()
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "store", "config", "key", "model", "server", "embedder", "claude", "codex", "hook",
            "timer", "watch", "sync"
        ]
    );
}
