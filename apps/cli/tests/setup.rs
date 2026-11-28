use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bilbo_cli::console::env::{Env, Who};
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;
use bilbo_store::layout::Layout;
use bilbo_store::segments::Store;

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
        who: Who::Nobody,
        program: None,
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
        let dir = loop {
            let dir = scratch::scratch_dir(test);
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("{}: {error}", dir.display()),
            }
        };
        let sandbox = Sandbox { dir };
        for folder in ["home", "config"] {
            fs::create_dir_all(sandbox.dir.join(folder)).expect("create scratch");
        }
        sandbox
    }

    fn file(&self) -> PathBuf {
        self.dir.join("config/config.toml")
    }

    fn put(&self, text: &str) {
        fs::write(self.file(), text).expect("a config file");
    }

    fn vars(&self) -> HashMap<String, OsString> {
        scratch::vars(&self.dir)
    }

    fn notes(&self) -> PathBuf {
        self.dir.join("home/.local/share/bilbo/notes")
    }

    fn text(&self) -> String {
        fs::read_to_string(self.file()).expect("the config file")
    }

    fn bare(&self) -> (HashMap<String, OsString>, PathBuf) {
        let mut vars = self.vars();
        vars.remove("BILBO_CONFIG");
        let home = self.dir.join("xdg");
        fs::create_dir_all(&home).expect("a config home");
        vars.insert("XDG_CONFIG_HOME".to_owned(), home.clone().into_os_string());
        (vars, home.join("bilbo/config.toml"))
    }

    fn writes_though_read_only(&self) -> bool {
        let probe = self.dir.join("probe-write");
        fs::write(&probe, "").expect("a file");
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o444)).expect("a mode");
        let writes = fs::OpenOptions::new().append(true).open(&probe).is_ok();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o644)).expect("a mode");
        let _ = fs::remove_file(&probe);
        writes
    }

    fn reads_though_closed(&self) -> bool {
        let probe = self.dir.join("probe-read");
        fs::create_dir_all(probe.join("inner")).expect("a folder");
        let _closed = Closed::new(&probe);
        fs::metadata(probe.join("inner")).is_ok()
    }
}

struct Closed(PathBuf);

impl Closed {
    fn new(folder: &std::path::Path) -> Closed {
        fs::set_permissions(folder, fs::Permissions::from_mode(0o000)).expect("a mode");
        Closed(folder.to_path_buf())
    }
}

impl Drop for Closed {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

fn has_a_scheduler() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

#[test]
fn with_nothing_on_path_the_tools_are_skipped_and_nothing_is_started() {
    let sandbox = Sandbox::new("setup-nothing-on-path");
    sandbox.put("");
    let run = bilbo(sandbox.vars(), &["setup"]);
    let (scheduler, status, code) = if cfg!(target_os = "macos") {
        ("launchctl", "failed", 1)
    } else {
        ("systemctl", "skipped", 0)
    };
    let timers = if has_a_scheduler() {
        format!(
            "refresh-timer\t{status}\t{scheduler} is not on PATH\nderive-timer\t{status}\t{scheduler} is not on PATH\n"
        )
    } else {
        "refresh-timer\tskipped\tthis platform has neither launchd nor systemd\nderive-timer\tskipped\tthis platform has neither launchd nor systemd\n".to_owned()
    };
    let notes = sandbox.notes();
    assert_eq!(
        run.stdout,
        format!(
            "notes\tinstalled\tmade {} and followed it in {}, after the default follows it would replace\nclaude\tskipped\tclaude is not on PATH\ncodex\tskipped\tcodex is not on PATH\ncodex-hooks\tskipped\tthe codex step was skipped\nagy\tskipped\tagy is not on PATH\n{timers}",
            notes.display(),
            sandbox.file().display()
        ),
        "{:?}",
        run.stderr
    );
    assert_eq!(run.code, code, "{:?}", run.stderr);
    for folder in ["store", "state", "cache"] {
        assert!(!sandbox.dir.join(folder).exists(), "{folder} was made");
    }
}

#[test]
fn a_logs_folder_or_a_units_folder_inside_the_root_is_a_config_error() {
    if !has_a_scheduler() {
        return;
    }
    let sandbox = Sandbox::new("setup-inside-root");
    let root = sandbox.dir.join("store");
    let link = sandbox.dir.join("link");
    fs::create_dir_all(&root).expect("a root");
    symlink(&root, &link).expect("a link to the root");
    for logs in [root.join("library"), link.join("library")] {
        sandbox.put(&format!("setup.timer.logs = \"{}\"\n", logs.display()));
        let run = bilbo(sandbox.vars(), &["setup"]);
        assert_eq!(run.code, 2, "{logs:?}: {:?}", run.stderr);
        assert_eq!(run.stdout, "");
        assert_eq!(
            run.stderr,
            [format!(
                "bilbo: setup.timer.logs names {}, inside the store root {}, which holds only segments; set setup.timer.logs to a folder outside the root",
                logs.display(),
                root.display()
            )],
            "{logs:?}"
        );
        assert!(!root.join("library").exists(), "{logs:?}: nothing was made");
    }

    sandbox.put("");
    let home = sandbox.dir.join("home");
    let (above, units) = if cfg!(target_os = "macos") {
        (home.join("Library"), home.join("Library/LaunchAgents"))
    } else {
        (home.join(".config"), home.join(".config/systemd/user"))
    };
    let mut vars = sandbox.vars();
    vars.insert("BILBO_HOME".to_owned(), above.clone().into_os_string());
    let run = bilbo(vars, &["setup"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: the units' folder {} is inside the store root {}, which holds only segments; set paths.root to a folder outside it",
            units.display(),
            above.display()
        )]
    );
    assert!(!units.exists(), "nothing was made");
}

const FOLLOWS: &str = "follow = []\n";

fn tree(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).expect("a folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() && !path.is_symlink() {
                pending.push(path.clone());
            }
            found.push(path);
        }
    }
    found.sort();
    found
}

fn only_line(run: &Run) -> &str {
    assert_eq!(run.stdout, "", "no row");
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    &run.stderr[0]
}

#[test]
fn a_units_folder_or_a_home_the_settings_refuse_ends_setup() {
    if !has_a_scheduler() {
        return;
    }
    let sandbox = Sandbox::new("setup-refused-home");
    sandbox.put(&format!("{FOLLOWS}setup.notes = \"\"\n"));
    let root = sandbox.dir.join("store");
    let data = sandbox.dir.join("data");
    let mut base = sandbox.vars();
    base.insert("XDG_DATA_HOME".to_owned(), data.into_os_string());
    if cfg!(target_os = "macos") {
        base.insert(
            "BILBO_SETUP_CODEX_ENABLED".to_owned(),
            OsString::from("false"),
        );
    }
    let refused = |vars: HashMap<String, OsString>, said: &str, name: &str| {
        let before = tree(&sandbox.dir);
        let run = bilbo(vars, &["setup"]);
        assert_eq!(run.code, 2, "{name}: {:?}", run.stderr);
        assert_eq!(only_line(&run), format!("bilbo: {said}"), "{name}");
        assert_eq!(tree(&sandbox.dir), before, "{name}: nothing was made");
    };
    if cfg!(target_os = "macos") {
        let mut relative = base.clone();
        relative.insert("HOME".to_owned(), OsString::from("h"));
        refused(
            relative,
            "HOME is not an absolute path: h",
            "a relative HOME",
        );
        let dotted = format!("{}/home/../home", sandbox.dir.display());
        let mut with_dots = base.clone();
        with_dots.insert("HOME".to_owned(), OsString::from(&dotted));
        refused(
            with_dots,
            &format!("HOME has a . or .. component: {dotted}"),
            "a HOME with a dot-dot",
        );
        let mut unset = base.clone();
        unset.remove("HOME");
        refused(unset, "HOME is not set", "no HOME");
    } else {
        let sub = root.join("sub");
        fs::create_dir_all(&sub).expect("a folder in the root");
        let link = sandbox.dir.join("link");
        symlink(&sub, &link).expect("a link into the root");
        let through = format!("{}/link/..", sandbox.dir.display());
        let mut dotted = base.clone();
        dotted.insert("XDG_CONFIG_HOME".to_owned(), OsString::from(&through));
        refused(
            dotted,
            &format!("XDG_CONFIG_HOME has a . or .. component: {through}"),
            "an XDG_CONFIG_HOME with a dot-dot",
        );
        let mut unset = base.clone();
        unset.insert(
            "XDG_CONFIG_HOME".to_owned(),
            sandbox.dir.join("config-home").into_os_string(),
        );
        unset.remove("HOME");
        refused(
            unset.clone(),
            "HOME is not set",
            "no HOME for Codex's hooks",
        );
        unset.insert(
            "BILBO_SETUP_CODEX_ENABLED".to_owned(),
            OsString::from("false"),
        );
        let run = bilbo(unset, &["setup"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        assert!(
            run.stdout
                .contains("codex\tskipped\tsetup.codex.enabled is false")
        );
    }
}

#[test]
fn setups_checks_come_in_their_order_before_a_home_is_needed() {
    let sandbox = Sandbox::new("setup-check-order");
    sandbox.put(FOLLOWS);
    let root = sandbox.dir.join("store");
    let mut vars = sandbox.vars();
    vars.remove("HOME");
    vars.insert(
        "XDG_DATA_HOME".to_owned(),
        sandbox.dir.join("data").into_os_string(),
    );
    let run = bilbo(vars.clone(), &["setup", "--plugin", "relative"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        only_line(&run),
        "bilbo: --plugin is not an absolute path: relative"
    );

    fs::create_dir_all(root.join("notes")).expect("a 0.20 root");
    let run = bilbo(vars.clone(), &["setup"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        only_line(&run),
        format!(
            "bilbo: {}/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder",
            root.display()
        )
    );
    fs::remove_dir(root.join("notes")).expect("the folder is taken away");

    if has_a_scheduler() {
        let logs = root.join("library");
        sandbox.put(&format!(
            "{FOLLOWS}setup.timer.logs = \"{}\"\n",
            logs.display()
        ));
        let run = bilbo(vars.clone(), &["setup"]);
        assert_eq!(run.code, 2, "{:?}", run.stderr);
        assert_eq!(
            only_line(&run),
            format!(
                "bilbo: setup.timer.logs names {}, inside the store root {}, which holds only segments; set setup.timer.logs to a folder outside the root",
                logs.display(),
                root.display()
            )
        );
        sandbox.put(FOLLOWS);
    }

    let run = bilbo(vars, &["setup"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(only_line(&run), "bilbo: HOME is not set");
}

#[test]
fn a_folder_check_that_cannot_look_ends_setup_with_exit_2() {
    if !has_a_scheduler() {
        return;
    }
    let sandbox = Sandbox::new("setup-cannot-look");
    let (a, b) = (sandbox.dir.join("a"), sandbox.dir.join("b"));
    symlink(&b, &a).expect("a link");
    symlink(&a, &b).expect("a link back");
    let logs = a.join("x");
    sandbox.put(&format!(
        "{FOLLOWS}setup.timer.logs = \"{}\"\n",
        logs.display()
    ));
    let layout = Layout {
        root: sandbox.dir.join("store"),
        state: sandbox.dir.join("state/bilbo"),
        cache: sandbox.dir.join("cache/bilbo"),
    };
    let error =
        Store::is_inside_root(&layout, &logs).expect_err("a loop of links cannot be looked at");
    let Failure::Refused(text) = Failure::from(error) else {
        panic!("the store gives a refusal");
    };
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(only_line(&run), format!("bilbo: {text}"));
}

fn first_row(run: &Run) -> (String, String, String) {
    let line = run
        .stdout
        .lines()
        .next()
        .unwrap_or_else(|| panic!("a row: {:?} {}", run.stderr, run.code));
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(fields.len(), 3, "{line}");
    assert_eq!(fields[0], "notes");
    (
        fields[0].to_owned(),
        fields[1].to_owned(),
        fields[2].to_owned(),
    )
}

fn row(status: &str, detail: String) -> (String, String, String) {
    ("notes".to_owned(), status.to_owned(), detail)
}

fn follows_of(sandbox: &Sandbox) -> String {
    bilbo(sandbox.vars(), &["config", "follow"]).stdout
}

#[test]
fn a_fresh_home_gets_a_notes_folder_followed_after_the_defaults() {
    let sandbox = Sandbox::new("setup-notes-fresh");
    let (vars, config) = sandbox.bare();
    assert!(!config.exists());
    let notes = sandbox.notes();
    let run = bilbo(vars.clone(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row(
            "installed",
            format!(
                "made {} and followed it in {}, after the default follows it would replace",
                notes.display(),
                config.display()
            )
        ),
        "{:?}",
        run.stderr
    );
    assert!(notes.is_dir());
    assert!(config.is_file(), "the first config file was made");
    let home = sandbox.dir.join("home");
    assert_eq!(
        bilbo(vars, &["config", "follow"]).stdout,
        format!(
            "[{{ path = \"{h}/.claude/projects\", kind = \"session\" }}, {{ path = \"{h}/.codex/sessions\", kind = \"session\" }}, {{ path = \"{n}\", kind = \"markdown\" }}]\n",
            h = home.display(),
            n = notes.display()
        )
    );
}

#[test]
fn a_second_run_keeps_the_notes_folder_and_writes_no_config() {
    let sandbox = Sandbox::new("setup-notes-second");
    sandbox.put("");
    bilbo(sandbox.vars(), &["setup"]);
    let held = fs::read(sandbox.file()).expect("the config");
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row(
            "kept",
            format!("{} is followed as markdown", sandbox.notes().display())
        )
    );
    assert_eq!(fs::read(sandbox.file()).expect("the config"), held);
}

fn follow_table(path: &std::path::Path) -> String {
    format!(
        "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
        path.display()
    )
}

#[test]
fn a_config_that_follows_a_markdown_folder_keeps_it() {
    let sandbox = Sandbox::new("setup-notes-own-folder");
    let mine = sandbox.dir.join("mine");
    fs::create_dir_all(&mine).expect("a folder");
    sandbox.put(&follow_table(&mine));
    let list = format!(
        "[{{ path = \"{}\", kind = \"markdown\" }}]\n",
        mine.display()
    );
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row(
            "kept",
            format!("{} is followed as markdown", mine.display())
        )
    );
    assert!(!sandbox.notes().exists(), "no second notes folder");
    assert_eq!(follows_of(&sandbox), list);

    let one = sandbox.dir.join("one.md");
    fs::write(&one, "# one\n").expect("a file");
    let gone = sandbox.dir.join("gone");
    for (name, follow) in [("a file", one), ("a folder that is gone", gone)] {
        sandbox.put(&follow_table(&follow));
        let run = bilbo(sandbox.vars(), &["setup"]);
        assert_eq!(
            first_row(&run),
            row(
                "installed",
                format!(
                    "made {} and followed it in {}",
                    sandbox.notes().display(),
                    sandbox.file().display()
                )
            ),
            "{name}: {:?}",
            run.stderr
        );
        assert_eq!(
            follows_of(&sandbox),
            format!(
                "[{{ path = \"{}\", kind = \"markdown\" }}, {{ path = \"{}\", kind = \"markdown\" }}]\n",
                follow.display(),
                sandbox.notes().display()
            ),
            "{name}"
        );
        assert!(sandbox.notes().is_dir(), "{name}: the folder is on disk");
        fs::remove_dir_all(sandbox.dir.join("home/.local"))
            .expect("the notes folder is taken away");
    }

    let real = sandbox.dir.join("data/notes");
    fs::create_dir_all(&real).expect("a folder");
    let link = sandbox.dir.join("notes-link");
    symlink(&real, &link).expect("a link");
    sandbox.put(&follow_table(&link));
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row(
            "kept",
            format!("{} is followed as markdown", link.display())
        ),
        "{:?}",
        run.stderr
    );
    assert!(!sandbox.notes().exists());
    assert_eq!(
        follows_of(&sandbox),
        format!(
            "[{{ path = \"{}\", kind = \"markdown\" }}]\n",
            link.display()
        ),
        "the follow is there as configured, and its target was not saved too"
    );

    if !sandbox.reads_though_closed() {
        let locked = sandbox.dir.join("locked");
        fs::create_dir_all(locked.join("notes")).expect("a folder");
        let follow = locked.join("notes");
        let text = follow_table(&follow);
        sandbox.put(&text);
        let _closed = Closed::new(&locked);
        let error = fs::metadata(&follow).expect_err("the folder cannot be looked at");
        let run = bilbo(sandbox.vars(), &["setup"]);
        assert_eq!(
            first_row(&run),
            row(
                "failed",
                format!("cannot read {}: {error}", follow.display())
            ),
            "{:?}",
            run.stderr
        );
        assert!(!sandbox.notes().exists(), "no second notes folder");
        assert_eq!(sandbox.text(), text);
    }
}

#[test]
fn a_notes_folder_named_is_made_and_followed_beside_the_one_there() {
    let sandbox = Sandbox::new("setup-notes-named");
    let mine = sandbox.dir.join("mine");
    fs::create_dir_all(&mine).expect("a folder");
    sandbox.put(&format!(
        "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
        mine.display()
    ));
    let other = sandbox.dir.join("other");
    let run = bilbo(
        sandbox.vars(),
        &["setup", "--notes", other.to_str().expect("UTF-8")],
    );
    assert_eq!(
        first_row(&run),
        row(
            "installed",
            format!(
                "made {} and followed it in {}",
                other.display(),
                sandbox.file().display()
            )
        ),
        "{:?}",
        run.stderr
    );
    assert!(other.is_dir());
    assert_eq!(
        follows_of(&sandbox),
        format!(
            "[{{ path = \"{}\", kind = \"markdown\" }}, {{ path = \"{}\", kind = \"markdown\" }}]\n",
            mine.display(),
            other.display()
        )
    );
}

#[test]
fn a_notes_folder_inside_the_root_is_a_config_error() {
    let sandbox = Sandbox::new("setup-notes-inside-root");
    sandbox.put(FOLLOWS);
    let root = sandbox.dir.join("store");
    let link = sandbox.dir.join("link");
    fs::create_dir_all(&root).expect("a root");
    symlink(&root, &link).expect("a link to the root");
    for notes in [root.join("notes"), link.join("notes")] {
        let run = bilbo(
            sandbox.vars(),
            &["setup", "--notes", notes.to_str().expect("UTF-8")],
        );
        assert_eq!(run.code, 2, "{:?}", run.stderr);
        assert_eq!(
            only_line(&run),
            format!(
                "bilbo: setup.notes names {}, inside the store root {}, which holds only segments; set setup.notes to a folder outside the root",
                notes.display(),
                root.display()
            )
        );
        assert!(
            !root.join("notes").exists(),
            "nothing was made under the root"
        );
    }
    let inside = root.join("mine");
    fs::create_dir_all(&inside).expect("a folder in the root");
    sandbox.put(&format!(
        "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
        inside.display()
    ));
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        only_line(&run),
        format!(
            "bilbo: the followed notes folder {} is inside the store root {}, which holds only segments; follow a notes folder outside the root",
            inside.display(),
            root.display()
        )
    );
}

#[test]
fn a_config_folder_inside_the_root_is_a_config_error() {
    let sandbox = Sandbox::new("setup-config-inside-root");
    let root = sandbox.dir.join("store");
    let mut vars = sandbox.vars();
    vars.remove("BILBO_CONFIG");
    vars.insert(
        "XDG_CONFIG_HOME".to_owned(),
        root.join("notes").into_os_string(),
    );
    vars.insert(
        "XDG_DATA_HOME".to_owned(),
        sandbox.dir.join("data").into_os_string(),
    );
    let run = bilbo(vars, &["setup"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        only_line(&run),
        format!(
            "bilbo: the config file's folder {}/notes/bilbo is inside the store root {}, which holds only segments; set paths.root to a folder outside it",
            root.display(),
            root.display()
        )
    );
    assert!(!root.join("notes").exists(), "nothing was made in the root");
}

fn failed_line(run: &Run) -> String {
    let names: Vec<&str> = run
        .stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let step = fields.next()?;
            (fields.next() == Some("failed")).then_some(step)
        })
        .collect();
    let (noun, them) = if names.len() == 1 {
        ("step", "it")
    } else {
        ("steps", "them")
    };
    format!(
        "bilbo: {} {noun} failed: {}; bilbo setup tries {them} again",
        names.len(),
        names.join(", ")
    )
}

#[test]
fn a_config_setup_cannot_write_fails_the_step_and_says_the_lines() {
    let sandbox = Sandbox::new("setup-config-read-only");
    if !sandbox.writes_though_read_only() {
        sandbox.put("");
        fs::set_permissions(sandbox.file(), fs::Permissions::from_mode(0o444)).expect("a mode");
        let notes = sandbox.notes();
        let run = bilbo(sandbox.vars(), &["setup"]);
        let config = sandbox.file().display().to_string();
        assert_eq!(
            first_row(&run),
            row(
                "failed",
                format!("cannot save the follow in {config}: it is read-only")
            ),
            "{:?}",
            run.stderr
        );
        let home = sandbox.dir.join("home");
        let table = |path: String, kind: &str| {
            [
                "bilbo: [[follow]]".to_owned(),
                format!("bilbo: path = \"{path}\""),
                format!("bilbo: kind = \"{kind}\""),
                "bilbo: ".to_owned(),
            ]
        };
        let mut wanted = vec![format!(
            "bilbo: cannot save the follow in {config}: it is read-only; add these lines to it:"
        )];
        wanted.extend(table(
            format!("{}/.claude/projects", home.display()),
            "session",
        ));
        wanted.extend(table(
            format!("{}/.codex/sessions", home.display()),
            "session",
        ));
        wanted.extend(table(notes.display().to_string(), "markdown"));
        wanted.pop();
        wanted.push(failed_line(&run));
        assert_eq!(run.stderr, wanted);
        assert_eq!(run.code, 1);
    }

    let inline = Sandbox::new("setup-config-inline");
    let other = inline.dir.join("other");
    fs::create_dir_all(&other).expect("a folder");
    inline.put(&format!(
        "follow = [{{ path = \"{}\", kind = \"session\" }}]\n",
        other.display()
    ));
    let run = bilbo(inline.vars(), &["setup"]);
    let config = inline.file().display().to_string();
    assert_eq!(
        first_row(&run),
        row(
            "failed",
            format!("cannot save the follow in {config}: it declares follow as an inline array")
        ),
        "{:?}",
        run.stderr
    );
    assert_eq!(
        run.stderr,
        [
            format!(
                "bilbo: cannot save the follow in {config}: it declares follow as an inline array; add this table to its follow array:"
            ),
            format!(
                "bilbo: {{ path = \"{}\", kind = \"markdown\" }}",
                inline.notes().display()
            ),
            failed_line(&run),
        ]
    );
    assert_eq!(run.code, 1);
}

#[test]
fn a_config_setup_cannot_write_that_follows_the_folder_is_kept() {
    let sandbox = Sandbox::new("setup-config-kept");
    let notes = sandbox.notes();
    fs::create_dir_all(&notes).expect("the notes folder");
    let text = format!(
        "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
        notes.display()
    );
    sandbox.put(&text);
    if !sandbox.writes_though_read_only() {
        fs::set_permissions(sandbox.file(), fs::Permissions::from_mode(0o444)).expect("a mode");
        let run = bilbo(sandbox.vars(), &["setup"]);
        assert_eq!(
            first_row(&run),
            row(
                "kept",
                format!("{} is followed as markdown", notes.display())
            )
        );
        assert_eq!(sandbox.text(), text);
    }

    let linked = Sandbox::new("setup-config-link");
    let notes = linked.notes();
    fs::create_dir_all(&notes).expect("the notes folder");
    let target = linked.dir.join("real.toml");
    fs::write(
        &target,
        text.replace(
            &sandbox.notes().display().to_string(),
            &notes.display().to_string(),
        ),
    )
    .expect("a target");
    symlink(&target, linked.file()).expect("a link");
    let before = (
        fs::read(&target).expect("the target"),
        fs::metadata(&target)
            .expect("meta")
            .modified()
            .expect("a time"),
    );
    let run = bilbo(linked.vars(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row(
            "kept",
            format!("{} is followed as markdown", notes.display())
        )
    );
    assert!(linked.file().is_symlink(), "the link is still a link");
    assert_eq!(
        (
            fs::read(&target).expect("the target"),
            fs::metadata(&target)
                .expect("meta")
                .modified()
                .expect("a time")
        ),
        before
    );
}

#[test]
fn a_notes_folder_followed_as_another_kind_fails_the_step() {
    let sandbox = Sandbox::new("setup-notes-other-kind");
    let notes = sandbox.notes();
    let text = format!(
        "[[follow]]\npath = \"{}\"\nkind = \"session\"\n",
        notes.display()
    );
    sandbox.put(&text);
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row(
            "failed",
            format!(
                "{} is followed as session, and a notes folder is followed as markdown",
                notes.display()
            )
        )
    );
    assert_eq!(sandbox.text(), text);
    assert_eq!(run.code, 1);
}

#[test]
fn a_notes_folder_followed_with_no_kind_or_twice_is_decided_by_its_follows() {
    let sandbox = Sandbox::new("setup-notes-follows");
    let notes = sandbox.notes();
    fs::create_dir_all(&notes).expect("the notes folder");
    let table = |kind: Option<&str>| match kind {
        Some(kind) => format!(
            "[[follow]]\npath = \"{}\"\nkind = \"{kind}\"\n",
            notes.display()
        ),
        None => format!("[[follow]]\npath = \"{}\"\n", notes.display()),
    };
    let cases: Vec<(&str, String, &str, String)> = vec![
        (
            "no kind",
            table(None),
            "failed",
            format!(
                "{} is followed with no kind, and a notes folder is followed as markdown; give that follow kind = \"markdown\"",
                notes.display()
            ),
        ),
        (
            "markdown and session",
            format!("{}{}", table(Some("markdown")), table(Some("session"))),
            "failed",
            format!(
                "{} is followed as session, and a notes folder is followed as markdown",
                notes.display()
            ),
        ),
        (
            "markdown twice",
            format!("{}{}", table(Some("markdown")), table(Some("markdown"))),
            "kept",
            format!("{} is followed as markdown", notes.display()),
        ),
    ];
    for (name, text, status, detail) in cases {
        sandbox.put(&text);
        for _ in 0..2 {
            let run = bilbo(sandbox.vars(), &["setup"]);
            assert_eq!(first_row(&run), row(status, detail.clone()), "{name}");
            assert_eq!(sandbox.text(), text, "{name}");
        }
    }
}

#[test]
fn a_path_that_is_no_folder_fails_the_step_and_saves_no_follow() {
    let sandbox = Sandbox::new("setup-notes-no-folder");
    sandbox.put("");
    let file = sandbox.dir.join("not-a-folder");
    fs::write(&file, "x").expect("a file");
    let run = bilbo(
        sandbox.vars(),
        &["setup", "--notes", file.to_str().expect("UTF-8")],
    );
    assert_eq!(
        first_row(&run),
        row("failed", format!("{} is not a folder", file.display()))
    );
    assert_eq!(sandbox.text(), "");
    assert_eq!(run.code, 1, "{:?}", run.stderr);

    let laid = Sandbox::new("setup-notes-no-folder-config-in-root");
    let xdg = laid.dir.join("xdg");
    let root = xdg.join("bilbo");
    let mut vars = laid.vars();
    vars.remove("BILBO_CONFIG");
    vars.insert("XDG_CONFIG_HOME".to_owned(), xdg.clone().into_os_string());
    vars.insert("BILBO_HOME".to_owned(), root.clone().into_os_string());
    vars.insert(
        "XDG_DATA_HOME".to_owned(),
        laid.dir.join("data").into_os_string(),
    );
    let outside = laid.dir.join("elsewhere");
    fs::write(&outside, "x").expect("a file");
    let run = bilbo(
        vars,
        &["setup", "--notes", outside.to_str().expect("UTF-8")],
    );
    assert_eq!(
        first_row(&run),
        row("failed", format!("{} is not a folder", outside.display()))
    );
    assert_eq!(run.stdout.lines().count(), 7, "{:?}", run.stderr);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(!root.exists(), "nothing was made in the root");

    let beneath = Sandbox::new("setup-notes-beneath-a-file");
    beneath.put("");
    let plain = beneath.dir.join("plain");
    fs::write(&plain, "x").expect("a file");
    let child = plain.join("child");
    let run = bilbo(
        beneath.vars(),
        &["setup", "--notes", child.to_str().expect("UTF-8")],
    );
    assert_eq!(
        first_row(&run),
        row("failed", format!("{} is not a folder", child.display())),
        "{:?}",
        run.stderr
    );
    assert_eq!(run.stdout.lines().count(), 7, "{:?}", run.stderr);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(plain.is_file(), "the file is as it was");
    assert_eq!(beneath.text(), "");
    assert!(!beneath.notes().exists(), "nothing was made");

    let kinded = Sandbox::new("setup-notes-no-folder-session");
    let file = kinded.dir.join("a-file");
    fs::write(&file, "x").expect("a file");
    kinded.put(&format!(
        "[[follow]]\npath = \"{}\"\nkind = \"session\"\n",
        file.display()
    ));
    let run = bilbo(
        kinded.vars(),
        &["setup", "--notes", file.to_str().expect("UTF-8")],
    );
    assert_eq!(
        first_row(&run),
        row("failed", format!("{} is not a folder", file.display()))
    );
    assert_eq!(run.code, 1, "{:?}", run.stderr);
}

#[test]
fn a_notes_folder_at_or_under_the_config_file_is_refused_before_anything_is_made() {
    let sandbox = Sandbox::new("setup-notes-at-config");
    let (vars, config) = sandbox.bare();
    for (name, folder, said) in [
        (
            "at",
            config.clone(),
            format!("{0} is the config file {0}", config.display()),
        ),
        (
            "under",
            config.join("sub"),
            format!(
                "{} is under the config file {}",
                config.join("sub").display(),
                config.display()
            ),
        ),
    ] {
        let run = bilbo(
            vars.clone(),
            &["setup", "--notes", folder.to_str().expect("UTF-8")],
        );
        assert_eq!(
            first_row(&run),
            row("failed", said),
            "{name}: {:?}",
            run.stderr
        );
        assert_eq!(
            run.stdout.lines().count(),
            7,
            "{name}: the other rows follow"
        );
        assert_eq!(run.code, 1, "{name}: {:?}", run.stderr);
        assert!(!config.exists(), "{name}: the config's path was made");
        assert!(
            !config.parent().expect("a folder").exists(),
            "{name}: the config's folder was made"
        );
    }
}

#[test]
fn an_empty_setup_notes_skips_the_step() {
    let sandbox = Sandbox::new("setup-notes-empty");
    sandbox.put("setup.notes = \"\"\n");
    let run = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(
        first_row(&run),
        row("skipped", "setup.notes is empty".to_owned())
    );
    assert!(!sandbox.notes().exists());
    assert_eq!(sandbox.text(), "setup.notes = \"\"\n");
}

#[test]
fn a_removal_does_not_ask_about_the_root() {
    let sandbox = Sandbox::new("setup-remove-root");
    sandbox.put(FOLLOWS);
    let root = sandbox.dir.join("store");
    fs::create_dir_all(root.join("notes")).expect("a 0.20 root");
    let setup = bilbo(sandbox.vars(), &["setup"]);
    assert_eq!(setup.code, 2, "{:?}", setup.stderr);
    let run = bilbo(sandbox.vars(), &["setup", "--remove"]);
    assert_ne!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(run.stdout.lines().count(), 6, "{}", run.stdout);
    assert!(
        run.stderr.iter().all(|line| !line.contains("0.20")),
        "{:?}",
        run.stderr
    );
    assert!(
        root.join("notes").is_dir(),
        "the removal wrote nothing there"
    );
}

#[test]
fn a_removal_with_notes_or_plugin_is_a_usage_error_whatever_the_config() {
    let sandbox = Sandbox::new("setup-remove-usage");
    sandbox.put("this is no toml [[[\n");
    for (flag, reason) in [
        ("--notes", "a removal leaves the notes folder"),
        ("--plugin", "a removal reads no plugin folder"),
    ] {
        let run = bilbo(sandbox.vars(), &["setup", "--remove", flag, "x"]);
        assert_eq!(run.code, 2, "{flag}: {:?}", run.stderr);
        assert_eq!(
            run.stderr[0],
            format!("bilbo: {flag} does not stand with --remove: {reason}"),
            "{flag}: {:?}",
            run.stderr
        );
        assert_eq!(
            run.stderr.last().map(String::as_str),
            Some("bilbo: see 'bilbo setup --help'"),
            "{flag}: {:?}",
            run.stderr
        );
        assert!(
            run.stderr.iter().all(|line| !line.contains("config.toml")),
            "{flag}: no line of the config error: {:?}",
            run.stderr
        );
    }
    let run = bilbo(sandbox.vars(), &["setup", "--remove"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert!(
        run.stderr.iter().any(|line| line.contains("config.toml")),
        "{:?}",
        run.stderr
    );
}
