use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use bilbo_programs::{Fail, Programs, plugin_folder};

use super::scratch::{Scratch, closed};
use super::{Run, binary};

const NAMES: [&str; 5] = ["claude", "codex", "agy", "launchctl", "systemctl"];
const MARKETPLACE_ID: &str = "bilbo@bilbo";

struct World {
    scratch: Scratch,
    programs: Programs,
    tools: PathBuf,
    standins: PathBuf,
    bin: PathBuf,
    fixture: PathBuf,
}

type Row = (String, String, String);

fn rows(run: &Run) -> Vec<Row> {
    run.stdout
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 3, "{line}");
            (
                fields[0].to_owned(),
                fields[1].to_owned(),
                fields[2].to_owned(),
            )
        })
        .collect()
}

fn statuses(run: &Run) -> Vec<(String, String)> {
    rows(run)
        .into_iter()
        .map(|(step, status, _)| (step, status))
        .collect()
}

const STEPS: [&str; 7] = [
    "notes",
    "claude",
    "codex",
    "codex-hooks",
    "agy",
    "refresh-timer",
    "derive-timer",
];

fn snapshot(dir: &Path, found: &mut Vec<(String, Vec<u8>)>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("a folder")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() && !path.is_symlink() {
            found.push((path.display().to_string(), Vec::new()));
            snapshot(&path, found);
        } else {
            let bytes = fs::read(&path).unwrap_or_default();
            found.push((path.display().to_string(), bytes));
        }
    }
}

fn run_raw(program: &Path, args: &[&str], env: Vec<(OsString, OsString)>) -> Run {
    let output = Command::new(program)
        .env_clear()
        .envs(env)
        .args(args)
        .output()
        .expect("bilbo runs");
    Run {
        code: output.status.code().expect("an exit code"),
        stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
    }
}

impl World {
    fn new(test: &str) -> World {
        let scratch = Scratch::new(test);
        let tools = scratch.dir.join("tools");
        let standins = scratch.dir.join("standins");
        let programs = Programs::write(&tools, &standins).expect("stand-ins");
        programs
            .set("codex", "version", env!("CARGO_PKG_VERSION"))
            .expect("a version");
        let fixture = scratch.dir.join("fixture");
        plugin_folder(&fixture).expect("a plugin folder");
        let world = World {
            bin: fs::canonicalize(binary()).expect("the binary"),
            scratch,
            programs,
            tools,
            standins,
            fixture,
        };
        world
            .programs
            .units(&world.units())
            .expect("the units' folder");
        world
    }

    fn home(&self) -> PathBuf {
        self.scratch.dir.join("home")
    }

    fn units(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.home().join("Library/LaunchAgents")
        } else {
            self.home().join(".config/systemd/user")
        }
    }

    fn logs(&self) -> PathBuf {
        self.scratch.dir.join("state/bilbo/timer")
    }

    fn unit_files(&self, verb: &str) -> Vec<PathBuf> {
        if cfg!(target_os = "macos") {
            vec![
                self.units()
                    .join(format!("io.github.delucca.bilbo.{verb}.plist")),
            ]
        } else {
            vec![
                self.units().join(format!("bilbo-{verb}.service")),
                self.units().join(format!("bilbo-{verb}.timer")),
            ]
        }
    }

    fn unit_text(&self, verb: &str) -> String {
        self.unit_files(verb)
            .iter()
            .map(|path| fs::read_to_string(path).expect("a unit file"))
            .collect()
    }

    fn package(&self, label: &str) -> PathBuf {
        let folder = self.scratch.dir.join(label);
        fs::create_dir_all(folder.join("bin")).expect("a bin folder");
        let program = folder.join("bin/bilbo");
        let real = fs::canonicalize(binary()).expect("the binary");
        fs::hard_link(&real, &program)
            .or_else(|_| fs::copy(&real, &program).map(|_| ()))
            .expect("a copy of the binary");
        plugin_folder(&folder.join("share/bilbo")).expect("a plugin folder");
        program
    }

    fn share(&self, label: &str) -> PathBuf {
        self.scratch.dir.join(label).join("share/bilbo")
    }

    fn run_with(&self, program: &Path, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut env: Vec<(OsString, OsString)> = self
            .scratch
            .env()
            .into_iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect();
        env.push((OsString::from("PATH"), self.tools.clone().into_os_string()));
        for (name, value) in extra {
            env.retain(|(held, _)| held.as_os_str() != *name);
            env.push((OsString::from(name), OsString::from(value)));
        }
        run_raw(program, args, env)
    }

    fn setup(&self, program: &Path) -> Run {
        self.setup_with(program, &[])
    }

    fn setup_with(&self, program: &Path, extra: &[(&str, &str)]) -> Run {
        if program == self.bin {
            let fixture = self.fixture.display().to_string();
            self.run_with(program, &["setup", "--plugin", &fixture], extra)
        } else {
            self.run_with(program, &["setup"], extra)
        }
    }

    fn expected(&self, share: &Path, status: [&str; 7], extra: [&str; 7]) -> Vec<Row> {
        let share = share.display().to_string();
        let notes = self.home().join(".local/share/bilbo/notes");
        let config = self.scratch.dir.join("config/config.toml");
        let made = format!(
            "made {} and followed it in {}, after the default follows it would replace",
            notes.display(),
            config.display()
        );
        let details = [
            if status[0] == "installed" {
                made
            } else {
                format!("{} is followed as markdown", notes.display())
            },
            format!("{MARKETPLACE_ID} from {share}{}", extra[1]),
            format!("{MARKETPLACE_ID} from {share}{}", extra[2]),
            format!("Codex trusts 2 hooks of bilbo@bilbo{}", extra[3]),
            format!("bilbo from {share}{}", extra[4]),
            format!("{}{}", self.timer_line("refresh", "10m"), extra[5]),
            format!("{}{}{CLAUSE}", self.timer_line("derive", "30m"), extra[6]),
        ];
        STEPS
            .iter()
            .zip(status)
            .zip(details)
            .map(|((step, status), detail)| ((*step).to_owned(), status.to_owned(), detail))
            .collect()
    }

    fn log_len(&self) -> Vec<usize> {
        NAMES
            .iter()
            .map(|name| self.programs.log(name).len())
            .collect()
    }

    fn since(&self, before: &[usize]) -> Vec<String> {
        NAMES
            .iter()
            .zip(before)
            .flat_map(|(name, held)| {
                self.programs.log(name)[*held..]
                    .iter()
                    .map(|line| format!("{name} {line}"))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn loaded(&self, verb: &str) -> bool {
        if cfg!(target_os = "macos") {
            let held = format!(
                "gui/{}/io.github.delucca.bilbo.{verb}",
                rustix::process::getuid().as_raw()
            );
            self.programs.loaded_labels().contains(&held)
        } else {
            let unit = format!("bilbo-{verb}.timer");
            self.programs
                .get("systemctl", &format!("active.{unit}"))
                .as_deref()
                == Some("active")
                && self
                    .units()
                    .join("timers.target.wants")
                    .join(&unit)
                    .is_symlink()
        }
    }

    fn fail_load(&self, verb: &str) -> String {
        if cfg!(target_os = "macos") {
            let label = format!("io.github.delucca.bilbo.{verb}");
            self.programs
                .fail_for(
                    "launchctl",
                    "bootstrap",
                    Some(&label),
                    false,
                    Fail::Exit("no"),
                    false,
                )
                .expect("marked");
            format!("launchctl.fail.bootstrap.for.{label}")
        } else {
            let unit = format!("bilbo-{verb}.timer");
            self.programs
                .fail_for(
                    "systemctl",
                    "restart",
                    Some(&unit),
                    false,
                    Fail::Exit("no"),
                    false,
                )
                .expect("marked");
            format!("systemctl.fail.restart.for.{unit}")
        }
    }

    fn fail_unload(&self, verb: &str) -> String {
        if cfg!(target_os = "macos") {
            let label = format!("io.github.delucca.bilbo.{verb}");
            self.programs
                .fail_for(
                    "launchctl",
                    "bootout",
                    Some(&label),
                    false,
                    Fail::Exit("no"),
                    false,
                )
                .expect("marked");
            format!("launchctl.fail.bootout.for.{label}")
        } else {
            let unit = format!("bilbo-{verb}.timer");
            self.programs
                .fail_for(
                    "systemctl",
                    "disable",
                    Some(&unit),
                    false,
                    Fail::Exit("no"),
                    false,
                )
                .expect("marked");
            format!("systemctl.fail.disable.for.{unit}")
        }
    }

    fn lift(&self, state_file: &str) {
        fs::remove_file(self.standins.join(state_file)).expect("the fault is lifted");
    }

    fn timer_line(&self, verb: &str, every: &str) -> String {
        format!(
            "every {every}, log {}",
            self.logs().join(format!("{verb}.log")).display()
        )
    }
}

fn found_status(run: &Run, step: &str) -> String {
    rows(run)
        .into_iter()
        .find(|(name, _, _)| name == step)
        .expect("a row")
        .1
}

fn scratch_folder(world: &World, name: &str) -> PathBuf {
    let folder = world.scratch.dir.join(name);
    fs::create_dir_all(&folder).expect("a folder");
    folder
}

const CLAUSE: &str = "; derive.enabled is false in the config and the unit";

#[test]
fn a_root_of_bilbo_0_20_stops_setup_before_anything_is_written() {
    let world = World::new("setup-root-0-20");
    let program = world.bin.clone();
    fs::create_dir_all(world.scratch.dir.join("store/notes")).expect("a 0.20 root");
    let mut before = Vec::new();
    snapshot(&world.scratch.dir, &mut before);
    let run = world.setup(&program);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: {}/store/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder\n",
            world.scratch.dir.display()
        )
    );
    for name in NAMES {
        assert!(world.programs.log(name).is_empty(), "{name}");
    }
    for verb in ["refresh", "derive"] {
        for file in world.unit_files(verb) {
            assert!(!file.exists(), "{}", file.display());
        }
    }
    assert!(!world.logs().exists());
    let mut after = Vec::new();
    snapshot(&world.scratch.dir, &mut after);
    assert_eq!(
        before, after,
        "every file is as it was, the config's bytes among them"
    );
}

#[test]
fn the_derive_clause_is_read_as_a_unit_will_read_it() {
    let derive_row = |run: &Run| rows(run).pop().expect("the last row").2;
    let world = World::new("setup-clause-file");
    let program = world.bin.clone();
    world.scratch.config(&closed(
        "derive.enabled = true\nderive.in = [\"x\"]\nllm.provider = \"anthropic\"\n",
    ));
    let run = world.setup_with(&program, &[("BILBO_DERIVE_ENABLED", "false")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(derive_row(&run), world.timer_line("derive", "30m"));

    let world = World::new("setup-clause-variable");
    let program = world.bin.clone();
    let run = world.setup_with(&program, &[("BILBO_DERIVE_ENABLED", "true")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        derive_row(&run),
        format!("{}{CLAUSE}", world.timer_line("derive", "30m"))
    );

    let world = World::new("setup-clause-order");
    let program = world.bin.clone();
    world
        .scratch
        .config(&closed("derive.enabled = false\nllm.provider = \"\"\n"));
    let run = world.setup(&program);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        derive_row(&run),
        format!("{}{CLAUSE}", world.timer_line("derive", "30m"))
    );
    for (name, config, said) in [
        (
            "setup-clause-provider",
            "derive.enabled = true\n",
            "; llm.provider is empty in the config and the unit",
        ),
        (
            "setup-clause-in",
            "derive.enabled = true\nllm.provider = \"anthropic\"\n",
            "; derive.in is empty in the config and the unit",
        ),
    ] {
        let world = World::new(name);
        let program = world.bin.clone();
        world.scratch.config(&closed(config));
        let run = world.setup(&program);
        assert_eq!(run.code, 0, "{name}: {}", run.stderr);
        assert_eq!(
            rows(&run).pop().expect("the last row"),
            (
                "derive-timer".to_owned(),
                "installed".to_owned(),
                format!("{}{said}", world.timer_line("derive", "30m"))
            ),
            "{name}"
        );
    }
}

fn expected_reads(world: &World) -> Vec<(&'static str, Vec<String>)> {
    let uid = rustix::process::getuid().as_raw();
    let labels = ["refresh", "derive"].map(|verb| format!("io.github.delucca.bilbo.{verb}"));
    let units = ["refresh", "derive"].map(|verb| format!("bilbo-{verb}.timer"));
    let _ = world;
    let lists = || {
        vec![
            "plugin marketplace list --json".to_owned(),
            "plugin list --json".to_owned(),
        ]
    };
    let mut launchctl: Vec<String> = labels
        .iter()
        .map(|label| format!("print gui/{uid}/{label}"))
        .collect();
    launchctl.sort();
    let mut systemctl = vec!["--user is-system-running".to_owned()];
    for unit in &units {
        systemctl.push(format!("--user is-active {unit}"));
        systemctl.push(format!("--user is-enabled {unit}"));
    }
    vec![
        ("claude", lists()),
        ("codex", {
            let mut codex = lists();
            codex.extend(
                [
                    "app-server",
                    "rpc initialize",
                    "rpc initialized",
                    "rpc hooks/list",
                ]
                .map(str::to_owned),
            );
            codex
        }),
        ("agy", vec!["plugin list --json".to_owned()]),
        ("launchctl", launchctl),
        ("systemctl", systemctl),
    ]
}

#[test]
fn a_first_run_installs_every_step_and_a_second_keeps_every_one() {
    let world = World::new("setup-first-second");
    let program = world.bin.clone();
    let other = world.scratch.dir.join("fixture-two");
    plugin_folder(&other).expect("a second plugin folder");
    world.scratch.config(&closed(&format!(
        "setup.plugin = \"{}\"\n",
        other.display()
    )));
    let first = world.setup(&program);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert_eq!(first.stderr, "");
    assert_eq!(
        rows(&first),
        world.expected(&world.fixture, ["installed"; 7], [""; 7])
    );
    let mtimes = |world: &World| -> Vec<std::time::SystemTime> {
        ["refresh", "derive"]
            .iter()
            .flat_map(|verb| world.unit_files(verb))
            .map(|file| {
                fs::metadata(file)
                    .expect("a unit")
                    .modified()
                    .expect("a time")
            })
            .collect()
    };
    let (held, moved) = (world.log_len(), mtimes(&world));
    let second = world.setup(&program);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(
        rows(&second),
        world.expected(&world.fixture, ["kept"; 7], [""; 7])
    );
    let wanted = expected_reads(&world);
    for (name, held) in NAMES.iter().zip(&held) {
        let log = world.programs.log(name);
        let allowed = &wanted
            .iter()
            .find(|(known, _)| known == name)
            .expect("a tool")
            .1;
        for line in &log[*held..] {
            assert!(allowed.contains(line), "{name}: {line}");
        }
        assert!(
            log.iter()
                .all(|line| !line.contains(&other.display().to_string())),
            "{name}: the flag wins over setup.plugin"
        );
    }
    assert_eq!(mtimes(&world), moved, "no unit file was written");
}

fn list_of(world: &World, tool: &str, args: &[&str]) -> String {
    let run = world.run_with(&world.tools.join(tool), args, &[]);
    assert_eq!(run.code, 0, "{tool}: {}", run.stderr);
    run.stdout
}

#[test]
fn a_run_from_another_package_updates_the_plugins_and_the_timers() {
    let world = World::new("setup-another-package");
    let first = world.package("one");
    let second = world.package("two");
    let skill = world
        .share("two")
        .join("plugins/bilbo/skills/recall/SKILL.md");
    let text = fs::read_to_string(&skill).expect("a skill");
    fs::write(&skill, format!("{text}One more line.\n")).expect("a changed skill");
    assert_eq!(world.setup(&first).code, 0);
    let run = world.setup(&second);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let (one, two) = (world.share("one"), world.share("two"));
    assert_eq!(
        statuses(&run),
        [
            ("notes".to_owned(), "kept".to_owned()),
            ("claude".to_owned(), "updated".to_owned()),
            ("codex".to_owned(), "updated".to_owned()),
            ("codex-hooks".to_owned(), "kept".to_owned()),
            ("agy".to_owned(), "updated".to_owned()),
            ("refresh-timer".to_owned(), "updated".to_owned()),
            ("derive-timer".to_owned(), "updated".to_owned()),
        ]
    );
    let details: Vec<String> = rows(&run)
        .into_iter()
        .map(|(_, _, detail)| detail)
        .collect();
    assert_eq!(
        details[1],
        format!(
            "bilbo@bilbo from {}; its marketplace came from {}",
            two.display(),
            one.display()
        )
    );
    assert!(
        details[4].ends_with("; its copy differed"),
        "{}",
        details[4]
    );
    assert!(details[5].contains("; its unit differed"), "{}", details[5]);
    for verb in ["refresh", "derive"] {
        let text = world.unit_text(verb);
        assert!(text.contains(&second.display().to_string()), "{verb}");
        assert!(!text.contains(&first.display().to_string()), "{verb}");
    }
    let plugin = two.join("plugins/bilbo").display().to_string();
    let decoded = |tool: &str, args: &[&str]| -> serde_json::Value {
        serde_json::from_str(&list_of(&world, tool, args)).expect("a list that is JSON")
    };
    let claude = decoded("claude", &["plugin", "list", "--json"]);
    let ours: Vec<&serde_json::Value> = claude
        .as_array()
        .expect("a list")
        .iter()
        .filter(|item| item["id"].as_str().is_some_and(|id| id.ends_with("@bilbo")))
        .collect();
    assert_eq!(ours.len(), 1, "{claude}");
    assert_eq!(ours[0]["id"], "bilbo@bilbo");
    assert_eq!(ours[0]["readFromFolder"], plugin.as_str(), "{claude}");
    let market = decoded("claude", &["plugin", "marketplace", "list", "--json"]);
    assert_eq!(market.as_array().expect("a list").len(), 1, "{market}");
    assert_eq!(market[0]["name"], "bilbo");
    assert_eq!(market[0]["path"], two.display().to_string().as_str());
    let codex = decoded("codex", &["plugin", "list", "--json"]);
    let ours: Vec<&serde_json::Value> = codex["installed"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|item| {
            item["pluginId"]
                .as_str()
                .is_some_and(|id| id.ends_with("@bilbo"))
        })
        .collect();
    assert_eq!(ours.len(), 1, "{codex}");
    assert_eq!(ours[0]["pluginId"], "bilbo@bilbo");
    assert_eq!(ours[0]["source"]["path"], plugin.as_str(), "{codex}");
    let codex_market = decoded("codex", &["plugin", "marketplace", "list", "--json"]);
    assert_eq!(
        codex_market["marketplaces"]
            .as_array()
            .expect("a list")
            .len(),
        1
    );
    assert_eq!(
        codex_market["marketplaces"][0]["root"],
        two.display().to_string().as_str()
    );
    let agy = decoded("agy", &["plugin", "list", "--json"]);
    let ours: Vec<&serde_json::Value> = agy
        .as_array()
        .expect("a list")
        .iter()
        .filter(|item| item["name"] == "bilbo")
        .collect();
    assert_eq!(ours.len(), 1, "{agy}");
    assert_eq!(ours[0]["path"], plugin.as_str(), "{agy}");
    assert_eq!(agy.as_array().expect("a list").len(), 1, "{agy}");
}

#[test]
fn a_tool_that_is_absent_is_skipped() {
    let world = World::new("setup-absent-tool");
    let program = world.bin.clone();
    assert_eq!(world.setup(&program).code, 0);
    fs::remove_file(world.tools.join("codex")).expect("codex is taken off the machine");
    let run = world.setup(&program);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut wanted = world.expected(&world.fixture, ["kept"; 7], [""; 7]);
    wanted[2] = (
        "codex".to_owned(),
        "skipped".to_owned(),
        "codex is not on PATH".to_owned(),
    );
    wanted[3] = (
        "codex-hooks".to_owned(),
        "skipped".to_owned(),
        "the codex step was skipped".to_owned(),
    );
    assert_eq!(rows(&run), wanted);
}

#[test]
fn a_tool_kept_out_is_skipped_from_the_variable_and_from_the_file() {
    for (name, from_variable) in [("setup-off-variable", true), ("setup-off-file", false)] {
        let world = World::new(name);
        let program = world.bin.clone();
        let run = if from_variable {
            world.setup_with(&program, &[("BILBO_SETUP_CODEX_ENABLED", "false")])
        } else {
            world
                .scratch
                .config(&closed("setup.codex.enabled = false\n"));
            world.setup(&program)
        };
        assert_eq!(run.code, 0, "{name}: {}", run.stderr);
        let mut wanted = world.expected(&world.fixture, ["installed"; 7], [""; 7]);
        wanted[2] = (
            "codex".to_owned(),
            "skipped".to_owned(),
            "setup.codex.enabled is false".to_owned(),
        );
        wanted[3] = (
            "codex-hooks".to_owned(),
            "skipped".to_owned(),
            "the codex step was skipped".to_owned(),
        );
        assert_eq!(rows(&run), wanted, "{name}");
        assert!(world.programs.log("codex").is_empty(), "{name}");
    }
}

#[test]
fn a_failing_command_fails_its_step_and_the_rest_go_on() {
    let world = World::new("setup-failing-command");
    let program = world.bin.clone();
    world
        .programs
        .fail("claude", "install", Fail::Exit("boom"), false)
        .expect("marked");
    let run = world.setup(&program);
    assert_eq!(run.code, 1);
    let mut wanted = world.expected(&world.fixture, ["installed"; 7], [""; 7]);
    wanted[1] = (
        "claude".to_owned(),
        "failed".to_owned(),
        "claude plugin install bilbo@bilbo --json: boom".to_owned(),
    );
    assert_eq!(rows(&run), wanted);
    assert_eq!(
        run.stderr,
        "bilbo: 1 step failed: claude; bilbo setup tries it again\n"
    );
    world.programs.unfail("claude", "install").expect("lifted");
    let again = world.setup(&program);
    assert_eq!(again.code, 0, "{}", again.stderr);
    let mut wanted = world.expected(&world.fixture, ["kept"; 7], [""; 7]);
    wanted[1].1 = "installed".to_owned();
    assert_eq!(rows(&again), wanted);
}

#[test]
fn output_setup_cannot_read_fails_the_step_and_changes_nothing() {
    let world = World::new("setup-unreadable-output");
    let program = world.bin.clone();
    world
        .programs
        .print("claude", "market-list", "{}")
        .expect("a list that is no array");
    let run = world.setup(&program);
    assert_eq!(run.code, 1);
    let found = rows(&run);
    assert_eq!(found[1].1, "failed");
    assert_eq!(
        found[1].2,
        "cannot read claude plugin marketplace list --json: its output is not the JSON bilbo reads"
    );
    assert_eq!(
        world.programs.log("claude"),
        ["plugin marketplace list --json"]
    );
}

#[test]
fn another_plugin_of_the_marketplace_stops_the_step() {
    let world = World::new("setup-another-plugin");
    let program = world.bin.clone();
    world
        .programs
        .set("claude", "market", "/elsewhere")
        .expect("a marketplace");
    world
        .programs
        .set("claude", "plugin", "on")
        .expect("a plugin");
    world
        .programs
        .set("claude", "from", "/elsewhere/plugins/bilbo")
        .expect("a folder");
    world
        .programs
        .set("claude", "also", "other@bilbo")
        .expect("another plugin");
    let run = world.setup(&program);
    assert_eq!(run.code, 1);
    let found = rows(&run);
    assert_eq!(found[1].1, "failed");
    assert_eq!(
        found[1].2,
        "claude has other@bilbo (user scope) installed from the marketplace bilbo; bilbo setup changes no marketplace another installation uses"
    );
    assert_eq!(
        world.programs.log("claude"),
        ["plugin marketplace list --json", "plugin list --json"]
    );
    let lists = world.run_with(
        &world.tools.join("claude"),
        &["plugin", "list", "--json"],
        &[],
    );
    assert!(lists.stdout.contains("other@bilbo"), "{}", lists.stdout);
    assert!(lists.stdout.contains("bilbo@bilbo"), "{}", lists.stdout);
}

#[test]
fn an_app_server_that_does_not_answer_fails_the_hooks_step_alone() {
    let world = World::new("setup-silent-app-server");
    let program = world.bin.clone();
    world.programs.flag("codex", "silent").expect("silent");
    let run = world.setup_with(&program, &[("BILBO_SETUP_WAIT", "1s")]);
    assert_eq!(run.code, 1);
    let found = rows(&run);
    assert_eq!(found[3].0, "codex-hooks");
    assert_eq!(found[3].1, "failed");
    assert!(found[3].2.contains("did not answer"), "{}", found[3].2);
    assert!(found[3].2.contains("within 1s"), "{}", found[3].2);
    for at in [0, 1, 2, 4, 5, 6] {
        assert_eq!(found[at].1, "installed", "{:?}", found[at]);
    }
    assert_eq!(
        run.stderr,
        "bilbo: 1 step failed: codex-hooks; bilbo setup tries it again\n"
    );
}

#[test]
fn trust_is_written_only_for_bilbos_hooks() {
    let world = World::new("setup-trust-only-bilbos");
    let program = world.bin.clone();
    world
        .programs
        .items(
            "codex",
            "items.hooks",
            &[r#"{"key":"other@market:hooks/hooks.json:x:0:0","pluginId":"other@market"}"#],
        )
        .expect("a hook of another plugin");
    world
        .programs
        .seed_trust("other@market:hooks/hooks.json:x:0:0", "sha256:x")
        .expect("a trust entry of another plugin");
    let run = world.setup(&program);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let trust = world.programs.trust();
    assert_eq!(
        trust[0],
        (
            "other@market:hooks/hooks.json:x:0:0".to_owned(),
            "sha256:x".to_owned()
        )
    );
    assert_eq!(trust.len(), 3);
    for (key, _) in &trust[1..] {
        assert!(key.starts_with("bilbo@bilbo:"), "{key}");
    }
}

#[test]
fn a_plugin_turned_off_and_a_unit_changed_are_put_back() {
    let world = World::new("setup-put-back");
    let program = world.bin.clone();
    assert_eq!(world.setup(&program).code, 0);
    world
        .programs
        .set("claude", "plugin", "off")
        .expect("turned off");
    fs::write(&world.unit_files("refresh")[0], "changed\n").expect("a changed unit");
    let run = world.setup(&program);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let found = rows(&run);
    assert_eq!(found[1].1, "updated");
    assert!(
        found[1].2.ends_with("; it was turned off"),
        "{}",
        found[1].2
    );
    assert_eq!(found[5].1, "updated");
    assert_eq!(
        found[5].2,
        format!("{}; its unit differed", world.timer_line("refresh", "10m"))
    );
    assert_eq!(found[6].1, "kept");
}

#[test]
fn the_units_hold_the_program_the_log_the_folder_variables_and_the_path() {
    let world = World::new("setup-unit-texts");
    let program = world.bin.clone();
    let dir = world.scratch.dir.display().to_string();
    let home = world.home().display().to_string();
    let xdg_config = world.scratch.dir.join("xdg-config");
    let units = if cfg!(target_os = "macos") {
        world.units()
    } else {
        xdg_config.join("systemd/user")
    };
    world.programs.units(&units).expect("the units' folder");
    let tools = world.tools.display().to_string();
    let xdg = xdg_config.display().to_string();
    let data = format!("{dir}/data");
    let extra = [
        ("XDG_CONFIG_HOME", xdg.as_str()),
        ("XDG_DATA_HOME", data.as_str()),
        ("BILBO_SETUP_TIMER_PATH", "/opt/extra:/usr/bin"),
    ];
    let run = world.setup_with(&program, &extra);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let variables = [
        ("HOME", home.clone()),
        ("BILBO_CONFIG", format!("{dir}/config/config.toml")),
        ("XDG_CONFIG_HOME", xdg.clone()),
        ("BILBO_HOME", format!("{dir}/store")),
        ("XDG_DATA_HOME", data.clone()),
        ("XDG_STATE_HOME", format!("{dir}/state")),
        ("XDG_CACHE_HOME", format!("{dir}/cache")),
        ("PATH", format!("{tools}:/opt/extra:/usr/bin")),
    ];
    let bin = program.display().to_string();
    for (verb, every, description) in [
        (
            "refresh",
            10,
            "Keep what every bilbo follow gained or changed",
        ),
        ("derive", 30, "Write notes about the entries bilbo keeps"),
    ] {
        let log = world
            .logs()
            .join(format!("{verb}.log"))
            .display()
            .to_string();
        if cfg!(target_os = "macos") {
            let mut text = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n",
            );
            for (name, value) in &variables {
                text.push_str(&format!(
                    "\t\t<key>{name}</key>\n\t\t<string>{value}</string>\n"
                ));
            }
            text.push_str(&format!(
                "\t</dict>\n\t<key>Label</key>\n\t<string>io.github.delucca.bilbo.{verb}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{bin}</string>\n\t\t<string>{verb}</string>\n\t</array>\n\t<key>RunAtLoad</key>\n\t<false/>\n\t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n\t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\t<key>StartInterval</key>\n\t<integer>{}</integer>\n</dict>\n</plist>\n",
                every * 60
            ));
            let found = fs::read_to_string(&world.unit_files(verb)[0]).expect("the plist");
            assert_eq!(found, text, "{verb}");
        } else {
            let mut service =
                format!("[Unit]\nDescription={description}\n\n[Service]\nType=oneshot\n");
            for (name, value) in &variables {
                service.push_str(&format!("Environment=\"{name}={value}\"\n"));
            }
            service.push_str(&format!(
                "ExecStart=\"{bin}\" {verb}\nStandardOutput=append:{log}\nStandardError=append:{log}\n"
            ));
            let timer = format!(
                "[Unit]\nDescription=Run bilbo {verb} every {every} min\n\n[Timer]\nOnActiveSec={every}min\nOnUnitActiveSec={every}min\n\n[Install]\nWantedBy=timers.target\n"
            );
            assert_eq!(
                fs::read_to_string(units.join(format!("bilbo-{verb}.service"))).expect("a service"),
                service,
                "{verb}"
            );
            assert_eq!(
                fs::read_to_string(units.join(format!("bilbo-{verb}.timer"))).expect("a timer"),
                timer,
                "{verb}"
            );
        }
    }
    let in_front = scratch_folder(&world, "in-front");
    let front = format!("{}:{tools}", in_front.display());
    let mut again = extra.to_vec();
    again.push(("PATH", front.as_str()));
    let kept = world.setup_with(&program, &again);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    let mut wanted = world.expected(&world.fixture, ["kept"; 7], [""; 7]);
    wanted[0].2 = format!("{data}/bilbo/notes is followed as markdown");
    assert_eq!(rows(&kept), wanted);
}

#[test]
fn a_scheduler_that_refuses_a_unit_fails_that_timer_alone() {
    let world = World::new("setup-refused-load");
    let program = world.bin.clone();
    let fault = world.fail_load("refresh");
    let first = world.setup(&program);
    assert_eq!(first.code, 1);
    let found = rows(&first);
    assert_eq!(found[5].1, "failed", "{:?}", found[5]);
    assert_eq!(found[6].1, "installed", "{:?}", found[6]);
    assert!(!world.loaded("refresh"));
    assert!(world.loaded("derive"));
    world.lift(&fault);
    let second = world.setup(&program);
    assert_eq!(second.code, 0, "{}", second.stderr);
    let found = rows(&second);
    assert_eq!(found[5].1, "updated");
    assert_eq!(
        found[5].2,
        format!("{}; it was not loaded", world.timer_line("refresh", "10m"))
    );
    assert_eq!(found[6].1, "kept");
}

#[test]
fn a_run_from_another_package_whose_unload_fails_switches_at_the_next_run() {
    let world = World::new("setup-unload-fails");
    let first = world.package("one");
    let second = world.package("two");
    assert_eq!(world.setup(&first).code, 0);
    let bytes = |world: &World| -> Vec<Vec<u8>> {
        world
            .unit_files("refresh")
            .iter()
            .map(|file| fs::read(file).expect("a unit file"))
            .collect()
    };
    let before = bytes(&world);
    let fault = world.fail_unload("refresh");
    let run = world.setup(&second);
    assert_eq!(run.code, 1);
    assert_eq!(found_status(&run, "refresh-timer"), "failed");
    assert_eq!(bytes(&world), before, "the files are the first's still");
    let one = first.display().to_string();
    let two = second.display().to_string();
    let text = world.unit_text("refresh");
    assert!(text.contains(&one) && !text.contains(&two), "{text}");
    world.lift(&fault);
    let held = world.log_len();
    let third = world.setup(&second);
    assert_eq!(third.code, 0, "{}", third.stderr);
    assert_eq!(found_status(&third, "refresh-timer"), "updated");
    let text = world.unit_text("refresh");
    assert!(text.contains(&two) && !text.contains(&one), "{text}");
    let changed: Vec<String> = world
        .since(&held)
        .into_iter()
        .filter(|line| {
            [
                "bootout",
                "bootstrap",
                "--user stop",
                "--user disable",
                "--user enable",
                "--user restart",
            ]
            .iter()
            .any(|word| line.contains(word))
        })
        .collect();
    let unloads = changed
        .iter()
        .filter(|line| line.contains("bootout") || line.contains("--user disable"))
        .count();
    let loads = changed
        .iter()
        .filter(|line| line.contains("bootstrap") || line.contains("--user restart"))
        .count();
    assert!(unloads >= 1 && loads >= 1, "{changed:?}");
    for line in &changed {
        assert!(
            line.contains("io.github.delucca.bilbo.refresh")
                || line.contains("bilbo-refresh.timer"),
            "{line}"
        );
    }
}

fn audit_names(world: &World, shares: &[String], replaced: bool) {
    let uid = rustix::process::getuid().as_raw();
    let labels = ["refresh", "derive"].map(|verb| format!("io.github.delucca.bilbo.{verb}"));
    let lists = ["plugin marketplace list --json", "plugin list --json"];
    for name in ["claude", "codex", "agy"] {
        let mut allowed: Vec<String> = lists.iter().map(|line| (*line).to_owned()).collect();
        match name {
            "claude" => {
                allowed.extend(
                    [
                        "plugin marketplace remove bilbo --json",
                        "plugin install bilbo@bilbo --json",
                        "plugin uninstall bilbo@bilbo --json",
                    ]
                    .map(str::to_owned),
                );
            }
            "codex" => {
                allowed.extend(
                    [
                        "plugin marketplace remove bilbo --json",
                        "plugin add bilbo@bilbo --json",
                        "plugin remove bilbo@bilbo --json",
                        "app-server",
                        "rpc initialize",
                        "rpc initialized",
                        "rpc hooks/list",
                        "rpc config/batchWrite",
                    ]
                    .map(str::to_owned),
                );
            }
            _ => {
                allowed = vec![
                    "plugin list --json".to_owned(),
                    "plugin uninstall bilbo".to_owned(),
                ];
            }
        }
        for share in shares {
            if name == "agy" {
                allowed.push(format!("plugin install {share}/plugins/bilbo"));
            } else {
                allowed.push(format!("plugin marketplace add {share} --json"));
            }
        }
        let log = world.programs.log(name);
        for line in &log {
            assert!(allowed.contains(line), "{name}: {line}");
        }
        if name == "agy" && replaced {
            assert!(
                log.contains(&"plugin uninstall bilbo".to_owned()),
                "AGY's replacement ran"
            );
        }
    }
    let mut launchctl = Vec::new();
    for label in &labels {
        launchctl.push(format!("print gui/{uid}/{label}"));
        launchctl.push(format!("bootout --wait gui/{uid}/{label}"));
        launchctl.push(format!(
            "bootstrap gui/{uid} {}/{label}.plist",
            world.units().display()
        ));
    }
    for line in world.programs.log("launchctl") {
        assert!(launchctl.contains(&line), "{line}");
    }
    let mut systemctl = vec![
        "--user is-system-running".to_owned(),
        "--user daemon-reload".to_owned(),
    ];
    for unit in ["bilbo-refresh.timer", "bilbo-derive.timer"] {
        for verb in [
            "is-active",
            "is-enabled",
            "stop",
            "disable",
            "enable",
            "restart",
        ] {
            systemctl.push(format!("--user {verb} {unit}"));
        }
    }
    for line in world.programs.log("systemctl") {
        assert!(systemctl.contains(&line), "{line}");
    }
}

#[test]
fn setup_names_nothing_that_is_not_bilbos() {
    let plain = World::new("setup-names-plain");
    let program = plain.bin.clone();
    assert_eq!(plain.setup(&program).code, 0);
    assert_eq!(plain.setup(&program).code, 0);
    audit_names(&plain, &[plain.fixture.display().to_string()], false);

    let pair = World::new("setup-names-pair");
    let first = pair.package("one");
    let second = pair.package("two");
    let skill = pair
        .share("two")
        .join("plugins/bilbo/skills/recall/SKILL.md");
    let text = fs::read_to_string(&skill).expect("a skill");
    fs::write(&skill, format!("{text}One more line.\n")).expect("a changed skill");
    assert_eq!(pair.setup(&first).code, 0);
    assert_eq!(pair.setup(&second).code, 0);
    audit_names(
        &pair,
        &[
            pair.share("one").display().to_string(),
            pair.share("two").display().to_string(),
        ],
        true,
    );
}

#[test]
fn a_tool_in_a_folder_whose_name_is_no_utf8_is_found_and_its_timer_says_why_not() {
    let world = World::new("setup-no-utf8-path");
    let raw = world
        .scratch
        .dir
        .join(std::ffi::OsStr::from_bytes(b"tools-\xff"));
    if fs::create_dir(&raw).is_err() {
        return;
    }
    let lossy = world.scratch.dir.join("tools-\u{fffd}");
    fs::create_dir(&lossy).expect("a folder named as the text reads");
    fs::write(lossy.join("claude"), "#!/bin/sh\nexit 1\n").expect("a decoy");
    fs::set_permissions(
        lossy.join("claude"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("a mode");
    let standins =
        Programs::write(&raw, &world.scratch.dir.join("standins-two")).expect("stand-ins");
    standins
        .set("codex", "version", env!("CARGO_PKG_VERSION"))
        .expect("a version");
    standins.units(&world.units()).expect("the units' folder");
    let mut env: Vec<(OsString, OsString)> = world
        .scratch
        .env()
        .into_iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();
    env.push((
        OsString::from("PATH"),
        OsString::from_vec(raw.as_os_str().as_bytes().to_vec()),
    ));
    let fixture = world.fixture.display().to_string();
    let run = run_raw(&world.bin, &["setup", "--plugin", &fixture], env);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let found = rows(&run);
    assert_eq!(found[1].0, "claude");
    assert_eq!(found[1].1, "installed", "{:?}", found[1]);
    let why = "a folder of PATH is not UTF-8, which a unit cannot carry".to_owned();
    assert_eq!(
        found[5..],
        [
            ("refresh-timer".to_owned(), "failed".to_owned(), why.clone()),
            ("derive-timer".to_owned(), "failed".to_owned(), why),
        ]
    );
}

const REMOVAL: [&str; 6] = [
    "claude",
    "codex-hooks",
    "codex",
    "agy",
    "refresh-timer",
    "derive-timer",
];

impl World {
    fn remove(&self, extra: &[(&str, &str)]) -> Run {
        self.run_with(&self.bin, &["setup", "--remove"], extra)
    }

    fn timer_name(verb: &str) -> String {
        if cfg!(target_os = "macos") {
            format!("io.github.delucca.bilbo.{verb}")
        } else {
            format!("bilbo-{verb}.timer")
        }
    }

    fn removed() -> Vec<Row> {
        let row =
            |step: &str, status: &str, detail: String| (step.to_owned(), status.to_owned(), detail);
        vec![
            row(
                "claude",
                "removed",
                format!("{MARKETPLACE_ID} and the marketplace bilbo"),
            ),
            row(
                "codex-hooks",
                "removed",
                "2 trust entries of bilbo@bilbo".to_owned(),
            ),
            row(
                "codex",
                "removed",
                format!("{MARKETPLACE_ID} and the marketplace bilbo"),
            ),
            row("agy", "removed", "bilbo".to_owned()),
            row("refresh-timer", "removed", World::timer_name("refresh")),
            row("derive-timer", "removed", World::timer_name("derive")),
        ]
    }

    fn absent() -> Vec<Row> {
        let row = |step: &str, detail: String| (step.to_owned(), "absent".to_owned(), detail);
        vec![
            row("claude", "no plugin bilbo@bilbo".to_owned()),
            row(
                "codex-hooks",
                "Codex keeps no trust of bilbo@bilbo".to_owned(),
            ),
            row("codex", "no plugin bilbo@bilbo".to_owned()),
            row("agy", "no plugin bilbo".to_owned()),
            row(
                "refresh-timer",
                format!("no timer {}", World::timer_name("refresh")),
            ),
            row(
                "derive-timer",
                format!("no timer {}", World::timer_name("derive")),
            ),
        ]
    }

    fn nothing_listed(&self) {
        for tool in ["claude", "codex"] {
            for args in [
                &["plugin", "list", "--json"][..],
                &["plugin", "marketplace", "list", "--json"],
            ] {
                let listed = list_of(self, tool, args);
                assert!(!listed.contains("bilbo"), "{tool} {args:?}: {listed}");
            }
        }
        let listed = list_of(self, "agy", &["plugin", "list", "--json"]);
        assert!(!listed.contains("bilbo"), "agy: {listed}");
    }

    fn nothing_loaded(&self) {
        for verb in ["refresh", "derive"] {
            assert!(!self.loaded(verb), "{verb} is loaded");
            for file in self.unit_files(verb) {
                assert!(!file.exists(), "{}", file.display());
            }
        }
        if cfg!(target_os = "macos") {
            assert!(self.programs.loaded_labels().is_empty());
        }
    }
}

#[test]
fn a_removal_leaves_no_plugin_and_no_timer_and_keeps_the_rest() {
    let world = World::new("setup-remove-all");
    world
        .programs
        .seed_trust("other@market:hooks/hooks.json:x:0:0", "sha256:x")
        .expect("another plugin's trust");
    let first = world.setup(&world.bin.clone());
    assert_eq!(first.code, 0, "{}", first.stderr);
    fs::create_dir_all(world.logs()).expect("the logs' folder");
    for verb in ["refresh", "derive"] {
        fs::write(
            world.logs().join(format!("{verb}.log")),
            format!("a line of {verb}\n"),
        )
        .expect("a log line");
    }
    let notes = world.home().join(".local/share/bilbo/notes");
    fs::write(notes.join("a.md"), "a note\n").expect("a note");
    let (mut before_notes, mut before_logs) = (Vec::new(), Vec::new());
    snapshot(&notes, &mut before_notes);
    snapshot(&world.logs(), &mut before_logs);
    let config = fs::read(world.scratch.dir.join("config/config.toml")).expect("config");
    let store = world.scratch.dir.join("store");
    let mut before_store = Vec::new();
    if store.exists() {
        snapshot(&store, &mut before_store);
    }

    let run = world.remove(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(rows(&run), World::removed());
    assert_eq!(
        statuses(&run)
            .iter()
            .map(|(step, _)| step.as_str())
            .collect::<Vec<_>>(),
        REMOVAL
    );
    world.nothing_listed();
    world.nothing_loaded();
    assert_eq!(
        world.programs.trust(),
        [(
            "other@market:hooks/hooks.json:x:0:0".to_owned(),
            "sha256:x".to_owned()
        )]
    );
    let (mut after_notes, mut after_logs, mut after_store) = (Vec::new(), Vec::new(), Vec::new());
    snapshot(&notes, &mut after_notes);
    snapshot(&world.logs(), &mut after_logs);
    if store.exists() {
        snapshot(&store, &mut after_store);
    }
    assert_eq!(before_notes, after_notes, "the notes folder");
    assert_eq!(before_logs, after_logs, "the two logs");
    assert_eq!(before_store, after_store, "the store");
    assert_eq!(
        config,
        fs::read(world.scratch.dir.join("config/config.toml")).expect("config"),
        "the config file, its follow among it"
    );
}

#[test]
fn a_second_removal_changes_nothing() {
    let world = World::new("setup-remove-twice");
    assert_eq!(world.setup(&world.bin.clone()).code, 0);
    assert_eq!(world.remove(&[]).code, 0);
    let held = world.log_len();
    let run = world.remove(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run), World::absent());
    let changing = [
        "install",
        "uninstall",
        "add ",
        "remove",
        "bootout",
        "bootstrap",
        "stop",
        "disable",
        "enable",
        "restart",
        "daemon-reload",
        "batchWrite",
    ];
    let log = world.since(&held);
    assert!(!log.is_empty());
    for line in &log {
        assert!(
            changing.iter().all(|word| !line.contains(word)),
            "a second removal ran {line}"
        );
    }

    let world = World::new("setup-remove-twice-codex-off");
    let off = [("BILBO_SETUP_CODEX_ENABLED", "false")];
    assert_eq!(world.setup_with(&world.bin.clone(), &off).code, 0);
    for (pass, status) in [("first", "removed"), ("second", "absent")] {
        let run = world.remove(&off);
        assert_eq!(run.code, 0, "{pass}: {}", run.stderr);
        let found = rows(&run);
        let (hooks, codex) = (&found[1], &found[2]);
        assert_eq!(
            (hooks.1.as_str(), hooks.2.as_str()),
            ("skipped", "the codex step was skipped"),
            "{pass}"
        );
        assert_eq!(
            (codex.1.as_str(), codex.2.as_str()),
            ("skipped", "setup.codex.enabled is false"),
            "{pass}"
        );
        for index in [0, 3, 4, 5] {
            assert_eq!(found[index].1, status, "{pass} {}", found[index].0);
        }
    }
    assert!(world.programs.log("codex").is_empty());
}

#[test]
fn a_removal_takes_out_a_plugin_from_another_folder_and_one_turned_off() {
    let world = World::new("setup-remove-elsewhere");
    let first = world.package("one");
    let run = world.run_with(&first, &["setup"], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    fs::remove_dir_all(world.share("one")).expect("the package's folder is gone");
    world
        .programs
        .set("claude", "plugin", "off")
        .expect("turned off");
    let run = world.run_with(&world.bin, &["setup", "--remove"], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run), World::removed());
    world.nothing_listed();
}

#[test]
fn a_setup_after_a_removal_installs_again_and_finds_the_notes_followed() {
    let world = World::new("setup-remove-then-setup");
    let program = world.bin.clone();
    assert_eq!(world.setup(&program).code, 0);
    assert_eq!(world.remove(&[]).code, 0);
    let run = world.setup(&program);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        rows(&run),
        world.expected(
            &world.fixture,
            [
                "kept",
                "installed",
                "installed",
                "installed",
                "installed",
                "installed",
                "installed"
            ],
            [""; 7]
        )
    );
}

#[test]
fn a_tool_named_by_its_flag_or_its_key_is_installed_and_removed_with_no_tool_on_path() {
    let world = World::new("setup-named-tools");
    let scheduler = world.scratch.dir.join("scheduler-only");
    fs::create_dir_all(&scheduler).expect("a folder");
    for name in ["launchctl", "systemctl"] {
        std::os::unix::fs::symlink(world.tools.join(name), scheduler.join(name))
            .expect("a link to the scheduler's stand-in");
    }
    let path = scheduler.display().to_string();
    let tools = world.tools.display().to_string();
    let fixture = world.fixture.display().to_string();
    let (claude, codex, agy) = (
        format!("{tools}/claude"),
        format!("{tools}/codex"),
        format!("{tools}/agy"),
    );
    let flags = [
        format!("--claude={claude}"),
        format!("--codex={codex}"),
        format!("--agy={agy}"),
    ];
    let with = |args: &[&str]| world.run_with(&world.bin, args, &[("PATH", path.as_str())]);

    let mut setup = vec!["setup"];
    setup.extend(flags.iter().map(String::as_str));
    setup.extend(["--plugin", fixture.as_str()]);
    let run = with(&setup);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        rows(&run),
        world.expected(&world.fixture, ["installed"; 7], [""; 7])
    );
    for name in ["claude", "codex", "agy"] {
        assert!(!world.programs.log(name).is_empty(), "{name} was asked");
    }
    for verb in ["refresh", "derive"] {
        assert_eq!(
            unit_path(&world.unit_text(verb)).split(':').next(),
            Some(tools.as_str()),
            "the {verb} unit's PATH opens with the tools' folder"
        );
    }
    let mut remove = vec!["setup", "--remove"];
    remove.extend(flags.iter().map(String::as_str));
    let run = with(&remove);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run), World::removed());

    let world = World::new("setup-named-tools-by-key");
    let scheduler = world.scratch.dir.join("scheduler-only");
    fs::create_dir_all(&scheduler).expect("a folder");
    for name in ["launchctl", "systemctl"] {
        std::os::unix::fs::symlink(world.tools.join(name), scheduler.join(name))
            .expect("a link to the scheduler's stand-in");
    }
    let path = scheduler.display().to_string();
    let tools = world.tools.display().to_string();
    let fixture = world.fixture.display().to_string();
    world.scratch.config(&closed(&format!(
        "setup.claude.program = \"{tools}/claude\"\nsetup.codex.program = \"{tools}/codex\"\nsetup.agy.program = \"{tools}/agy\"\n"
    )));
    let with = |args: &[&str]| world.run_with(&world.bin, args, &[("PATH", path.as_str())]);
    let run = with(&["setup", "--plugin", fixture.as_str()]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        rows(&run),
        world.expected(&world.fixture, ["installed"; 7], [""; 7])
    );
    let run = with(&["setup", "--remove"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(rows(&run), World::removed());
}

fn unit_path(text: &str) -> String {
    if cfg!(target_os = "macos") {
        let after = text
            .split("<key>PATH</key>")
            .nth(1)
            .expect("a PATH in the plist");
        let value = after.split("<string>").nth(1).expect("a string");
        value.split("</string>").next().expect("its end").to_owned()
    } else {
        let line = text
            .lines()
            .find_map(|line| line.strip_prefix("Environment=\"PATH="))
            .expect("a PATH in the service");
        line.strip_suffix('"').expect("a closing quote").to_owned()
    }
}
