use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use bilbo_programs::{Fail, Programs, find_in, plugin_folder};

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Folder(PathBuf);

impl Folder {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> Folder {
    loop {
        let dir = std::env::temp_dir().join(format!(
            "bilbo-programs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return Folder(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("{}: {error}", dir.display()),
        }
    }
}

fn written(dir: &Folder) -> Programs {
    Programs::write(&dir.path().join("bin"), &dir.path().join("state")).expect("written")
}

fn call(programs: &Programs, name: &str, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(programs.path(name))
        .args(args)
        .env_clear()
        .output()
        .expect("the stand-in runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn serve(programs: &Programs, input: &str) -> String {
    let mut child = Command::new(programs.path("codex"))
        .arg("app-server")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the stand-in runs");
    child
        .stdin
        .take()
        .expect("a pipe")
        .write_all(input.as_bytes())
        .expect("written");
    let output = child.wait_with_output().expect("ended");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_stand_in_keeps_its_state_logs_each_call_and_refuses_arguments_it_does_not_know() {
    let dir = scratch();
    let programs = written(&dir);
    let (code, out, _) = call(&programs, "claude", &["plugin", "list", "--json"]);
    assert_eq!(code, 0);
    assert!(!out.contains("bilbo@bilbo"), "{out}");
    let (code, out, _) = call(
        &programs,
        "claude",
        &["plugin", "marketplace", "add", "/p", "--json"],
    );
    assert_eq!((code, out.contains("\"outcome\":\"ok\"")), (0, true));
    let (code, ..) = call(
        &programs,
        "claude",
        &["plugin", "install", "bilbo@bilbo", "--json"],
    );
    assert_eq!(code, 0);
    let (_, out, _) = call(&programs, "claude", &["plugin", "list", "--json"]);
    assert!(
        out.contains("\"readFromFolder\":\"/p/plugins/bilbo\""),
        "{out}"
    );
    let (code, _, err) = call(&programs, "claude", &["frob"]);
    assert_eq!(code, 64);
    assert!(err.contains("unknown arguments"), "{err}");
    assert_eq!(
        programs.log("claude"),
        [
            "plugin list --json",
            "plugin marketplace add /p --json",
            "plugin install bilbo@bilbo --json",
            "plugin list --json",
            "frob"
        ]
    );
}

#[test]
fn a_command_a_test_marks_fails_before_or_after_its_work() {
    let dir = scratch();
    let programs = written(&dir);
    programs.set("agy", "copy", "/c").expect("state");
    programs
        .fail("agy", "uninstall", Fail::Exit("boom"), false)
        .expect("marked");
    let (code, _, err) = call(&programs, "agy", &["plugin", "uninstall", "bilbo"]);
    assert_eq!((code, err.trim()), (1, "boom"));
    assert_eq!(programs.get("agy", "copy").as_deref(), Some("/c"));
    programs
        .fail("agy", "uninstall", Fail::Exit("boom"), true)
        .expect("marked");
    let (code, out, err) = call(&programs, "agy", &["plugin", "uninstall", "bilbo"]);
    assert_eq!((code, err.trim(), out.as_str()), (1, "boom", ""));
    assert_eq!(programs.get("agy", "copy").as_deref(), Some(""));
    programs
        .fail("agy", "uninstall", Fail::Signal, false)
        .expect("marked");
    let (code, ..) = call(&programs, "agy", &["plugin", "uninstall", "bilbo"]);
    assert_eq!(code, -1);

    programs
        .set("claude", "market", "/p")
        .expect("a marketplace");
    programs
        .fail("claude", "install", Fail::Exit("boom"), true)
        .expect("marked");
    let args = ["plugin", "install", "bilbo@bilbo", "--json"];
    let (code, out, _) = call(&programs, "claude", &args);
    assert_eq!(code, 1);
    assert!(
        out.contains("\"outcome\":\"failed\"") && out.contains("\"message\":\"boom\""),
        "{out}"
    );
    assert!(!out.contains("\"ok\""), "{out}");
    assert_eq!(out.lines().count(), 1, "{out}");
    let (_, list, _) = call(&programs, "claude", &["plugin", "list", "--json"]);
    assert!(list.contains("bilbo@bilbo"), "{list}");
    programs
        .fail("claude", "install", Fail::Signal, true)
        .expect("marked");
    let (code, out, _) = call(&programs, "claude", &args);
    assert_eq!((code, out.as_str()), (-1, ""));
    programs.unfail("claude", "install").expect("taken away");
    let (code, ..) = call(&programs, "claude", &args);
    assert_eq!(code, 0);
    programs
        .fail("claude", "plist", Fail::Exit("boom"), true)
        .expect("marked");
    let (code, out, err) = call(&programs, "claude", &["plugin", "list", "--json"]);
    assert_eq!((code, err.trim(), out.as_str()), (1, "boom", ""));
}

#[test]
fn the_app_server_lists_the_two_hooks_and_keeps_what_it_is_told_to_trust() {
    let dir = scratch();
    let programs = written(&dir);
    programs.set("codex", "installed", "on").expect("state");
    programs
        .seed_trust("other:hook", "sha256:x")
        .expect("seeded");
    let input = "{\"method\":\"initialized\"}\n{\"id\":1,\"method\":\"hooks/list\",\"params\":{}}\n\
                 {\"id\":2,\"method\":\"config/batchWrite\",\"params\":{\"edits\":[{\"keyPath\":\"hooks.state\",\"mergeStrategy\":\"upsert\",\"value\":{\"k1\":{\"trusted_hash\":\"h1\"},\"k2\":{\"trusted_hash\":\"h2\"}}}],\"reloadUserConfig\":false}}\n";
    let text = serve(&programs, input);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.contains("userPromptSubmit") && text.contains("\"matcher\":\"compact\""));
    assert!(text.contains("\"trustStatus\":\"untrusted\""));
    assert_eq!(
        programs.trust(),
        [
            ("other:hook".to_owned(), "sha256:x".to_owned()),
            ("k1".to_owned(), "h1".to_owned()),
            ("k2".to_owned(), "h2".to_owned())
        ]
    );
    assert_eq!(
        programs.log("codex"),
        [
            "app-server",
            "rpc initialized",
            "rpc hooks/list",
            "rpc config/batchWrite"
        ]
    );
}

#[test]
fn the_plugin_folder_holds_the_files_setup_reads() {
    let dir = scratch();
    plugin_folder(dir.path()).expect("written");
    for file in [
        bilbo_programs::MARKETPLACE_CLAUDE,
        bilbo_programs::MARKETPLACE_CODEX,
        bilbo_programs::MANIFEST_AGY,
        bilbo_programs::HOOKS_FILE,
    ] {
        assert!(dir.path().join(file).is_file(), "{file}");
    }
}

#[test]
fn the_claude_stand_in_takes_the_marketplaces_plugins_out_with_it() {
    for (bring, stays) in [("other@elsewhere", true), ("other@bilbo", false)] {
        let dir = scratch();
        let programs = written(&dir);
        programs
            .items(
                "claude",
                "items",
                &[
                    r#"{"id":"x@bilbo","version":"1","scope":"user","enabled":true}"#,
                    r#"{"id":"y@elsewhere","version":"1","scope":"user","enabled":true}"#,
                    r#"{"id":"y@elsewhere","version":"x@bilbo"}"#,
                    r#"{"id": "x@bilbo"}"#,
                ],
            )
            .expect("items");
        programs
            .set("claude", "bring", bring)
            .expect("an install that brings another");
        call(
            &programs,
            "claude",
            &["plugin", "marketplace", "add", "/p", "--json"],
        );
        call(
            &programs,
            "claude",
            &["plugin", "install", "bilbo@bilbo", "--json"],
        );
        let (_, before, _) = call(&programs, "claude", &["plugin", "list", "--json"]);
        for id in ["bilbo@bilbo", bring, "x@bilbo", "y@elsewhere"] {
            assert!(before.contains(id), "{id}: {before}");
        }
        call(
            &programs,
            "claude",
            &["plugin", "marketplace", "remove", "bilbo", "--json"],
        );
        let (_, after, _) = call(&programs, "claude", &["plugin", "list", "--json"]);
        assert!(after.contains("cc-plugin-privacy@builtin"), "{after}");
        assert!(after.contains("\"id\":\"y@elsewhere\""), "{after}");
        assert_eq!(after.contains(bring), stays, "{bring}: {after}");
        assert!(
            after.contains(r#"{"id":"y@elsewhere","version":"x@bilbo"}"#),
            "{after}"
        );
        for gone in [
            "bilbo@bilbo",
            r#""id":"x@bilbo""#,
            r#""id": "x@bilbo""#,
            "\"id\":\"\"",
        ] {
            assert!(!after.contains(gone), "{gone}: {after}");
        }
    }
}

#[test]
fn the_codex_stand_in_keeps_one_trust_entry_a_key() {
    let dir = scratch();
    let programs = written(&dir);
    programs
        .seed_trust("other:hook", "sha256:x")
        .expect("seeded");
    let write = |value: &str| {
        let input = format!(
            "{{\"id\":1,\"method\":\"config/batchWrite\",\"params\":{{\"edits\":[{{\"keyPath\":\"hooks.state\",\"mergeStrategy\":\"upsert\",\"value\":{value}}}],\"reloadUserConfig\":false}}}}\n"
        );
        serve(&programs, &input)
    };
    write(r#"{"k1":{"trusted_hash":"h1"}}"#);
    write(r#"{"k1":{"trusted_hash":"h2"},"q\"b\\s":{"trusted_hash":"h3"}}"#);
    assert_eq!(
        programs.trust(),
        [
            ("other:hook".to_owned(), "sha256:x".to_owned()),
            ("k1".to_owned(), "h2".to_owned()),
            ("q\"b\\s".to_owned(), "h3".to_owned())
        ]
    );
    write(r#"{"k1": {"trusted_hash": "h5"}}"#);
    let table = programs.trust();
    assert_eq!(
        table,
        [
            ("other:hook".to_owned(), "sha256:x".to_owned()),
            ("q\"b\\s".to_owned(), "h3".to_owned()),
            ("k1".to_owned(), "h5".to_owned())
        ]
    );
    let answer = write(r#"{"k1":{"trusted_hash":"h9"},"A\u0041":{"trusted_hash":"h4"}}"#);
    assert!(answer.contains("\"error\""), "{answer}");
    assert_eq!(programs.trust(), table);
}

fn executable(path: &Path, mode: u32) {
    fs::write(path, "#!/bin/sh\n").expect("a file");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("a mode");
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("a script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("a mode");
}

#[test]
fn the_stand_ins_find_their_helpers_and_stop_at_one_that_failed() {
    let dir = scratch();
    let (plain, runnable, near) = (
        dir.path().join("plain"),
        dir.path().join("runnable"),
        dir.path().join("near"),
    );
    for folder in [&plain, &runnable, &near] {
        fs::create_dir(folder).expect("a folder");
    }
    executable(&plain.join("ln"), 0o644);
    executable(&runnable.join("ln"), 0o755);
    executable(&near.join("ln"), 0o755);
    let working = std::env::current_dir().expect("a working folder");
    let mut relative = PathBuf::new();
    for _ in working.components().skip(1) {
        relative.push("..");
    }
    relative.push(near.strip_prefix("/").expect("an absolute folder"));
    assert!(relative.is_relative() && relative.join("ln").is_file());
    let path = std::env::join_paths([relative, plain.clone(), runnable.clone()]).expect("a PATH");
    assert_eq!(find_in("ln", &path), Some(runnable.join("ln")));
    assert_eq!(find_in("ln", &OsString::from("relative")), None);

    let helpers = dir.path().join("helpers");
    fs::create_dir(&helpers).expect("a folder");
    let ran = dir.path().join("ln-ran");
    script(
        &helpers.join("mkdir"),
        "printf 'mkdir: refused\\n' >&2\nexit 1",
    );
    script(
        &helpers.join("ln"),
        &format!("printf 'ran\\n' >> '{}'\nexit 0", ran.display()),
    );
    script(&helpers.join("rm"), "exit 0");
    let programs = Programs::write_with(
        &dir.path().join("bin"),
        &dir.path().join("state"),
        helpers.as_os_str(),
    )
    .expect("written");
    let units = dir.path().join("units");
    fs::create_dir(&units).expect("a folder");
    fs::write(units.join("bilbo-refresh.timer"), "[Timer]\n").expect("a unit");
    programs.units(&units).expect("the units' folder");
    let (code, _, err) = call(
        &programs,
        "systemctl",
        &["--user", "enable", "bilbo-refresh.timer"],
    );
    assert!(!ran.exists(), "the link helper never ran");
    assert_eq!(code, 1);
    assert!(err.contains("mkdir: refused"), "{err}");
    assert!(
        programs
            .get("systemctl", "held.bilbo-refresh.timer")
            .is_none(),
        "nothing was recorded"
    );
    assert!(!ran.exists(), "the link helper never ran");
}

#[test]
fn a_fault_can_name_its_unit_and_be_for_one_call() {
    let dir = scratch();
    let programs = written(&dir);
    let units = dir.path().join("units");
    fs::create_dir(&units).expect("a folder");
    for unit in ["bilbo-refresh.timer", "bilbo-derive.timer"] {
        fs::write(units.join(unit), "[Timer]\n").expect("a unit");
    }
    programs.units(&units).expect("the units' folder");
    let restart = |unit: &str| call(&programs, "systemctl", &["--user", "restart", unit]).0;
    programs
        .fail_for(
            "systemctl",
            "restart",
            Some("bilbo-derive.timer"),
            false,
            Fail::Exit("no"),
            false,
        )
        .expect("marked");
    assert_eq!(restart("bilbo-refresh.timer"), 0);
    assert_eq!(restart("bilbo-derive.timer"), 1);
    assert_eq!(restart("bilbo-derive.timer"), 1);
    programs.unfail("systemctl", "restart").expect("taken away");
    fs::remove_file(
        dir.path()
            .join("state/systemctl.fail.restart.for.bilbo-derive.timer"),
    )
    .expect("taken away");
    programs
        .fail_for(
            "systemctl",
            "restart",
            None,
            true,
            Fail::Exit("once"),
            false,
        )
        .expect("marked");
    assert_eq!(restart("bilbo-refresh.timer"), 1);
    assert_eq!(restart("bilbo-refresh.timer"), 0);

    let plist = |label: &str| {
        let path = dir.path().join(format!("{label}.plist"));
        fs::write(
            &path,
            format!("<dict>\n\t<key>Label</key>\n\t<string>{label}</string>\n</dict>\n"),
        )
        .expect("a plist");
        path
    };
    let (one, two) = (plist("one"), plist("two"));
    programs
        .fail_for(
            "launchctl",
            "bootstrap",
            Some("one"),
            false,
            Fail::Exit("no"),
            false,
        )
        .expect("marked");
    let bootstrap = |domain: &str, path: &Path| {
        call(
            &programs,
            "launchctl",
            &["bootstrap", domain, path.to_str().expect("UTF-8")],
        )
        .0
    };
    assert_eq!(bootstrap("gui/501", &two), 0);
    assert_eq!(bootstrap("gui/501", &one), 1);
    let print = |target: &str| call(&programs, "launchctl", &["print", target]).0;
    assert_eq!(print("gui/501/two"), 0);
    assert_eq!(print("gui/502/two"), 113);
    assert_eq!(programs.loaded_labels(), ["gui/501/two"]);
    let (_, _, err) = call(&programs, "launchctl", &["print", "gui/502/two"]);
    assert!(err.contains("gui: 502") && !err.contains("501"), "{err}");

    programs
        .fail_for(
            "launchctl",
            "bootstrap",
            None,
            false,
            Fail::Exit("once"),
            false,
        )
        .expect("marked");
    let three = plist("three");
    for _ in 0..2 {
        let (code, _, err) = call(
            &programs,
            "launchctl",
            &["bootstrap", "gui/501", three.to_str().expect("UTF-8")],
        );
        assert_eq!(code, 1);
        assert_eq!(err.trim(), "once");
    }
}

#[test]
fn the_codex_stand_in_takes_trust_out_only_for_the_edits_r_r4_sends() {
    let dir = scratch();
    let programs = written(&dir);
    for key in ["keep\"me", "other\\key"] {
        programs.seed_trust(key, "sha256:x").expect("seeded");
    }
    let bytes = || fs::read(dir.path().join("state/codex.trust")).expect("the table");
    let before = programs.trust();
    let before_bytes = bytes();
    let write = |edits: &str| {
        let input = format!(
            "{{\"id\":1,\"method\":\"config/batchWrite\",\"params\":{{\"edits\":[{edits}],\"reloadUserConfig\":false}}}}\n"
        );
        serve(&programs, &input)
    };
    let good =
        |path: &str| format!(r#"{{"keyPath":"{path}","mergeStrategy":"replace","value":null}}"#);
    let first = r#"hooks.state.\"keep\\\"me\""#;
    let bad = [
        (
            "a value of {}",
            r#"{"keyPath":"hooks.state.\"other\\\\key\"","mergeStrategy":"replace","value":{}}"#
                .to_owned(),
        ),
        (
            "an upsert on a quoted key's path",
            r#"{"keyPath":"hooks.state.\"other\\\\key\"","mergeStrategy":"upsert","value":null}"#
                .to_owned(),
        ),
        (
            "a bare quote in the quoted part",
            good(r#"hooks.state.\"keep\"me\""#),
        ),
        (
            "a good edit and then a bad one",
            format!(
                "{},{}",
                good(first),
                r#"{"keyPath":"hooks.state.\"other\\\\key\"","mergeStrategy":"upsert","value":null}"#
            ),
        ),
    ];
    for (name, edits) in bad {
        let answer = write(&edits);
        assert!(
            answer.contains("this stand-in takes null replacements of hooks.state keys only"),
            "{name}: {answer}"
        );
        assert_eq!(programs.trust(), before, "{name}: the table is as it was");
        assert_eq!(bytes(), before_bytes, "{name}: its bytes are as they were");
    }
    let answer = write(&good(first));
    assert!(answer.contains("\"result\""), "{answer}");
    assert_eq!(
        programs.trust(),
        [("other\\key".to_owned(), "sha256:x".to_owned())],
        "exactly the key it names goes"
    );
}

#[test]
fn the_systemctl_stand_in_holds_both_units_until_a_reload_drops_them() {
    let dir = scratch();
    let programs = written(&dir);
    let units = dir.path().join("units");
    fs::create_dir(&units).expect("a folder");
    programs.units(&units).expect("the units' folder");
    let (timer, service) = ("bilbo-refresh.timer", "bilbo-refresh.service");
    for unit in [timer, service] {
        fs::write(units.join(unit), "[Unit]\n").expect("a unit");
    }
    let state = |unit: &str| {
        call(
            &programs,
            "systemctl",
            &["--user", "show", "--property=LoadState", "--value", unit],
        )
        .1
    };
    let reload = || call(&programs, "systemctl", &["--user", "daemon-reload"]).0;
    assert_eq!(reload(), 0);
    for unit in [timer, service] {
        fs::remove_file(units.join(unit)).expect("a unit gone");
        assert_eq!(
            state(unit),
            "loaded\n",
            "{unit} is held after its file goes"
        );
    }
    assert_eq!(reload(), 0);
    for unit in [timer, service] {
        assert_eq!(state(unit), "not-found\n", "{unit}");
    }

    for unit in [timer, service] {
        fs::write(units.join(unit), "[Unit]\n").expect("a unit");
    }
    assert_eq!(reload(), 0);
    programs
        .flag("systemctl", &format!("keep.{service}"))
        .expect("kept");
    for unit in [timer, service] {
        fs::remove_file(units.join(unit)).expect("a unit gone");
    }
    assert_eq!(reload(), 0);
    assert_eq!(state(timer), "not-found\n");
    assert_eq!(state(service), "loaded\n", "kept past the reload");
}
