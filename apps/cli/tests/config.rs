use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;

mod scratch;

struct Run {
    code: u8,
    stdout: String,
    stderr: Vec<String>,
}

fn bilbo(vars: HashMap<String, OsString>, args: &[&str]) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var: std::rc::Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        notice: Box::new(move |message| {
            sink.lock().expect("notices").extend(message.plain_lines());
        }),
    };
    let argv: Vec<OsString> = std::iter::once("bilbo")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .map_err(Failure::from);
    let (code, stdout, lines) = match bilbo_cli::run(&argv, &mut env, settings) {
        Ok(outcome) => (
            outcome.exit_code(),
            outcome.stdout(&View::PLAIN),
            outcome.stderr_lines(&View::PLAIN),
        ),
        Err(failure) => (
            failure.exit_code(),
            String::new(),
            failure.stderr_lines(&View::PLAIN),
        ),
    };
    let mut stderr = said.lock().expect("notices").clone();
    stderr.extend(lines);
    Run {
        code,
        stdout,
        stderr,
    }
}

struct Sandbox {
    dir: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch::scratch_dir(test);
        for folder in ["home", "config"] {
            fs::create_dir_all(dir.join(folder)).expect("create scratch");
        }
        Sandbox { dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("config/config.toml")
    }

    fn put(&self, text: &str) {
        fs::write(self.file(), text).expect("a config file");
    }

    fn text(&self) -> String {
        fs::read_to_string(self.file()).expect("the config file")
    }

    fn vars(&self) -> HashMap<String, OsString> {
        scratch::vars(&self.dir)
    }

    fn run(&self, args: &[&str]) -> Run {
        bilbo(self.vars(), args)
    }
}

#[test]
fn set_writes_one_key_and_leaves_every_other_byte() {
    let sandbox = Sandbox::new("config-set-one");
    sandbox.put("# mine\n[store]\nseal_at =   \"16 MiB\"   # big\n\n[derive]\nenabled = false\n");
    let run = sandbox.run(&["config", "set", "derive.quiet", "30m"]);
    let file = sandbox.file().display().to_string();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, [format!("bilbo: set derive.quiet in {file}")]);
    assert_eq!(run.stdout, "");
    assert_eq!(
        sandbox.text(),
        "# mine\n[store]\nseal_at =   \"16 MiB\"   # big\n\n[derive]\nenabled = false\nquiet = \"30m\"\n"
    );
    let read = sandbox.run(&["config", "derive.quiet"]);
    assert_eq!((read.code, read.stdout.as_str()), (0, "30m\n"));
}

#[test]
fn set_makes_the_file_and_its_folder_when_the_variable_names_one_that_is_missing() {
    let sandbox = Sandbox::new("config-set-make");
    let mut vars = sandbox.vars();
    let file = sandbox.dir.join("new/folder/config.toml");
    vars.insert("BILBO_CONFIG".to_owned(), file.clone().into_os_string());
    let run = bilbo(vars.clone(), &["config", "set", "derive.enabled", "true"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        fs::read_to_string(&file).expect("made"),
        "[derive]\nenabled = true\n"
    );
    let read = bilbo(vars, &["config", "derive.enabled"]);
    assert_eq!(read.stdout, "true\n");
}

#[test]
fn set_runs_when_the_settings_do_not_load_and_mends_them_a_key_at_a_time() {
    let sandbox = Sandbox::new("config-set-mend");
    let file = sandbox.file().display().to_string();
    sandbox.put("[store]\nseal_at = \"0\"\n\n[recall]\nbudget = 1\n");
    let run = sandbox.run(&["config", "set", "store.seal_at", "8 MiB"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [
            format!("bilbo: set store.seal_at in {file}"),
            format!(
                "bilbo: {file} still holds a value bilbo refuses: {file}:5: recall.budget must be a whole number from 256 to 1000000, got 1"
            ),
        ]
    );
    let run = sandbox.run(&["config", "set", "recall.budget", "4000"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, [format!("bilbo: set recall.budget in {file}")]);
    assert_eq!(sandbox.run(&["config", "recall.budget"]).stdout, "4000\n");

    let broken = "[store\nseal_at = 1\n";
    sandbox.put(broken);
    let run = sandbox.run(&["config", "set", "derive.enabled", "true"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "derive.enabled = true\n");
    assert!(
        run.stderr[0].starts_with(&format!("bilbo: cannot set derive.enabled: {file}:1: ")),
        "{:?}",
        run.stderr
    );
    assert_eq!(sandbox.text(), broken);
}

#[test]
fn a_value_the_key_refuses_is_said_in_the_keys_words_and_nothing_is_written() {
    let sandbox = Sandbox::new("config-set-refused");
    let before = "[derive]\nquiet = \"1h\"\n";
    sandbox.put(before);
    let cases: [(&str, &str, &str); 7] = [
        (
            "recall.budget",
            "255",
            "recall.budget must be a whole number from 256 to 1000000, got 255",
        ),
        (
            "recall.budget",
            "1000001",
            "recall.budget must be a whole number from 256 to 1000000, got 1000001",
        ),
        (
            "derive.enabled",
            "yes",
            "derive.enabled must be true or false, got \"yes\"",
        ),
        (
            "llm.anthropic.api.endpoint",
            "http://localhost:1",
            "llm.anthropic.api.endpoint must name the address in place of localhost: 127.0.0.1",
        ),
        (
            "llm.anthropic.api.endpoint",
            "https://u:hunter2@example.com",
            "llm.anthropic.api.endpoint must hold no user name or password, got one",
        ),
        (
            "llm.openai.via",
            "cli",
            "llm.openai.via must be api or tool, got \"cli\"",
        ),
        (
            "llm.anthropic.via",
            "cli",
            "llm.anthropic.via must be api or tool, got \"cli\"",
        ),
    ];
    for (key, value, line) in cases {
        let run = sandbox.run(&["config", "set", key, value]);
        assert_eq!(run.code, 2, "{key} {value}");
        assert_eq!(run.stderr, [format!("bilbo: {line}")], "{key} {value}");
        assert!(!run.stdout.contains("hunter2"));
        assert_eq!(sandbox.text(), before);
    }
}

#[test]
fn config_set_writes_a_key_of_a_ways_table() {
    let sandbox = Sandbox::new("config-set-way");
    sandbox.put("[llm.anthropic]\nmodel = \"m\"\n");
    let run = sandbox.run(&[
        "config",
        "set",
        "llm.anthropic.api.endpoint",
        "https://e.test/v1",
    ]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.text(),
        "[llm.anthropic]\nmodel = \"m\"\n\n[llm.anthropic.api]\nendpoint = \"https://e.test/v1\"\n"
    );
    let sandbox = Sandbox::new("config-set-via");
    sandbox.put("[llm.anthropic.api]\nendpoint = \"https://example.test/v1\"\n");
    let run = sandbox.run(&["config", "set", "llm.anthropic.via", "tool"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.text(),
        "[llm.anthropic.api]\nendpoint = \"https://example.test/v1\"\n\n[llm.anthropic]\nvia = \"tool\"\n"
    );
    let read = sandbox.run(&["config", "llm.anthropic.via"]);
    assert_eq!((read.code, read.stdout.as_str()), (0, "tool\n"));
}

#[test]
fn config_set_takes_the_tool_for_openai() {
    let sandbox = Sandbox::new("config-set-openai-tool");
    sandbox.put("[llm.openai.api]\nendpoint = \"https://example.test/v1\"\n");
    let run = sandbox.run(&["config", "set", "llm.openai.via", "tool"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.text(),
        "[llm.openai.api]\nendpoint = \"https://example.test/v1\"\n\n[llm.openai]\nvia = \"tool\"\n"
    );
    let read = sandbox.run(&["config", "llm.openai.via"]);
    assert_eq!((read.code, read.stdout.as_str()), (0, "tool\n"));
}

#[test]
fn an_unknown_key_is_the_usage_error_config_gives() {
    let sandbox = Sandbox::new("config-set-unknown");
    let run = sandbox.run(&["config", "set", "store.sealat", "1"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr[0].starts_with("bilbo: unknown key 'store.sealat'; the keys are paths.root"),
        "{:?}",
        run.stderr
    );
    assert!(!sandbox.file().exists() || sandbox.text().is_empty());
}

type Make = fn(&Sandbox);

fn kept(folder: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("a folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn every_reason_set_does_not_write_has_its_clause_its_line_and_leaves_the_file() {
    let sandbox = Sandbox::new("config-set-walk");
    let file = sandbox.file().display().to_string();
    let folder = sandbox.dir.join("config");
    let by_hand = "; set it by hand:";
    let rows: Vec<(&str, Make, &str, &str, &str, bool)> = vec![
        (
            "a link",
            |s| {
                let target = s.dir.join("managed.toml");
                fs::write(&target, "[derive]\n").expect("a target");
                symlink(&target, s.file()).expect("a link");
            },
            "derive.enabled",
            "true",
            "it is a symbolic link",
            false,
        ),
        (
            "read-only",
            |s| {
                s.put("[derive]\n");
                fs::set_permissions(s.file(), fs::Permissions::from_mode(0o444)).expect("a mode");
            },
            "derive.enabled",
            "true",
            "it is read-only",
            false,
        ),
        (
            "dotted",
            |s| s.put("derive.enabled = false\n"),
            "derive.enabled",
            "true",
            "line 1 sets it with a dotted key",
            true,
        ),
        (
            "dotted over lines",
            |s| s.put("derive.in = [\n  \"sessions\",\n]\n"),
            "derive.in",
            "[\"sessions\"]",
            "line 1 sets it with a dotted key",
            true,
        ),
        (
            "table by a dotted key",
            |s| s.put("derive.enabled = true\n\n[llm.anthropic]\nmodel = \"m\"\n"),
            "derive.quiet",
            "45m",
            "line 1 sets its table with a dotted key",
            true,
        ),
        (
            "in braces",
            |s| s.put("derive = { enabled = false }\n"),
            "derive.enabled",
            "true",
            "line 1 sets it inside braces",
            true,
        ),
        (
            "table in braces",
            |s| s.put("llm = { anthropic = {} }\n"),
            "llm.anthropic.model",
            "m",
            "line 1 sets its table inside braces",
            true,
        ),
        (
            "a table of its own",
            |s| s.put("[derive.instructions]\nsession = \"/x\"\n"),
            "derive.instructions",
            "{ session = \"/y\" }",
            "line 1 sets it as a table of its own",
            true,
        ),
        (
            "a value that is no table above the key",
            |s| s.put("derive = 1\n"),
            "derive.enabled",
            "true",
            "bilbo could not place it",
            true,
        ),
        (
            "an escaped header",
            |s| s.put("[\"der\\u0069ve\"]\nquiet = \"1h\"\n"),
            "derive.enabled",
            "true",
            "bilbo could not place it",
            true,
        ),
    ];
    for (name, make, key, value, why, locked) in rows {
        let _ = fs::remove_file(sandbox.file());
        make(&sandbox);
        let before = fs::read(sandbox.file()).ok();
        let listing = kept(&folder);
        let run = sandbox.run(&["config", "set", key, value]);
        assert_eq!(run.code, 2, "{name}: {:?}", run.stderr);
        assert_eq!(
            run.stderr,
            [format!("bilbo: cannot set {key} in {file}: {why}{by_hand}")],
            "{name}"
        );
        let shown = if value.starts_with(['[', '{']) {
            value.to_owned()
        } else {
            format!("\"{value}\"")
        };
        let shown = if value == "true" {
            value.to_owned()
        } else {
            shown
        };
        assert_eq!(run.stdout, format!("{key} = {shown}\n"), "{name}");
        assert_eq!(fs::read(sandbox.file()).ok(), before, "{name}");
        let mut expected = listing.clone();
        let lock = "config.toml.lock".to_owned();
        if locked && !expected.contains(&lock) {
            expected.push(lock);
            expected.sort();
        }
        assert_eq!(kept(&folder), expected, "{name}");
    }

    let line = "derive.enabled = true\n";
    sandbox.put("[derive]\nenabled = false\n");
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).expect("a mode");
    let writable = sandbox.run(&["config", "set", "derive.enabled", "true"]);
    let mut vars = sandbox.vars();
    let missing = folder.join("other.toml");
    vars.insert("BILBO_CONFIG".to_owned(), missing.clone().into_os_string());
    let absent = bilbo(vars, &["config", "set", "derive.enabled", "true"]);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).expect("a mode");
    for (run, path) in [
        (&writable, file.clone()),
        (&absent, missing.display().to_string()),
    ] {
        assert_eq!(run.code, 2, "{:?}", run.stderr);
        assert_eq!(run.stdout, line);
        assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
        assert!(
            run.stderr[0].starts_with(&format!("bilbo: cannot set derive.enabled in {path}: "))
                && run.stderr[0].ends_with(by_hand)
                && !run.stderr[0].contains("add "),
            "{:?}",
            run.stderr
        );
    }
    assert_eq!(sandbox.text(), "[derive]\nenabled = false\n");
    assert!(!missing.exists());
    assert_eq!(kept(&folder), ["config.toml", "config.toml.lock"]);

    let broken = "[store\nseal_at = 1\n";
    sandbox.put(broken);
    let run = sandbox.run(&["config", "set", "derive.enabled", "true"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, line);
    assert!(
        run.stderr[0].starts_with(&format!("bilbo: cannot set derive.enabled: {file}:1: "))
            && run.stderr[0].ends_with(by_hand),
        "{:?}",
        run.stderr
    );
    assert_eq!(sandbox.text(), broken);
}

#[test]
fn a_table_is_set_whole() {
    let sandbox = Sandbox::new("config-set-table");
    let run = sandbox.run(&[
        "config",
        "set",
        "derive.instructions",
        "{ session = \"/x/y.txt\" }",
    ]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.text(),
        "[derive]\ninstructions = { session = \"/x/y.txt\" }\n"
    );
    let read = sandbox.run(&["config", "derive.instructions"]);
    assert_eq!(read.code, 0, "{:?}", read.stderr);
}

#[test]
fn a_text_that_is_no_toml_is_taken_as_text() {
    let sandbox = Sandbox::new("config-set-text");
    for (key, value) in [("derive.quiet", "30m"), ("llm.provider", "anthropic")] {
        let run = sandbox.run(&["config", "set", key, value]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
    }
    assert_eq!(
        sandbox.text(),
        "[derive]\nquiet = \"30m\"\n\n[llm]\nprovider = \"anthropic\"\n"
    );
}

#[test]
fn the_commands_that_turn_derive_on_run_in_order() {
    let sandbox = Sandbox::new("config-set-on");
    for args in [
        ["llm.provider", "anthropic"],
        ["llm.anthropic.model", "claude-haiku-5-5"],
        ["derive.in", "[\"sessions\"]"],
        ["derive.quiet", "0s"],
        ["derive.enabled", "true"],
    ] {
        let run = sandbox.run(&["config", "set", args[0], args[1]]);
        assert_eq!(run.code, 0, "{args:?} {:?}", run.stderr);
        assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    }
    assert_eq!(sandbox.run(&["config", "derive.enabled"]).stdout, "true\n");
}

#[test]
fn set_takes_its_turn_on_the_lock_and_reads_the_file_again() {
    let sandbox = Sandbox::new("config-set-lock");
    sandbox.put("[derive]\nquiet = \"1h\"\n");
    let mut lock = sandbox.file().into_os_string();
    lock.push(".lock");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .expect("a lock file");
    held.lock().expect("the lock");
    let vars = sandbox.vars();
    let setting =
        std::thread::spawn(move || bilbo(vars, &["config", "set", "derive.enabled", "true"]));
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(sandbox.text(), "[derive]\nquiet = \"1h\"\n");
    assert!(!setting.is_finished());
    sandbox.put("[derive]\nquiet = \"2h\"\n");
    drop(held);
    let run = setting.join().expect("the setter");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(sandbox.text(), "[derive]\nquiet = \"2h\"\nenabled = true\n");
}

#[test]
fn a_text_the_key_takes_only_as_a_text_is_written_as_one() {
    let sandbox = Sandbox::new("config-set-date");
    let run = sandbox.run(&["config", "set", "llm.anthropic.api.version", "2026-10-21"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.text(),
        "[llm.anthropic.api]\nversion = \"2026-10-21\"\n"
    );
    let read = sandbox.run(&["config", "llm.anthropic.api.version"]);
    assert_eq!((read.code, read.stdout.as_str()), (0, "2026-10-21\n"));
}

#[test]
fn the_warning_reads_the_file_alone_and_a_variable_that_wins_is_said() {
    let sandbox = Sandbox::new("config-set-warning");
    let file = sandbox.file().display().to_string();
    let mut vars = sandbox.vars();
    vars.insert("BILBO_DERIVE_ENABLED".to_owned(), "maybe".into());
    let run = bilbo(vars, &["config", "set", "derive.quiet", "30m"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, [format!("bilbo: set derive.quiet in {file}")]);
    let mut vars = sandbox.vars();
    vars.insert("BILBO_DERIVE_ENABLED".to_owned(), "false".into());
    let run = bilbo(vars, &["config", "set", "derive.enabled", "true"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [
            format!("bilbo: set derive.enabled in {file}"),
            format!("bilbo: BILBO_DERIVE_ENABLED is set and wins over derive.enabled in {file}"),
        ]
    );
}

#[test]
fn set_on_a_file_with_a_bom_leaves_a_file_every_verb_reads() {
    let sandbox = Sandbox::new("config-set-bom");
    sandbox.put("\u{feff}[store]\ncompression = 3\n");
    let run = sandbox.run(&["config", "set", "follow", "[]"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let read = sandbox.run(&["config", "follow"]);
    assert_eq!(read.code, 0, "{:?}", read.stderr);
}
