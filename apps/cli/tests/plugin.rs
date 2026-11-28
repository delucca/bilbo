use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Value, json};

const NINE: [&str; 9] = [
    ".agents/plugins/marketplace.json",
    ".claude-plugin/marketplace.json",
    "plugins/bilbo/.antigravity-plugin/plugin.json",
    "plugins/bilbo/.claude-plugin/plugin.json",
    "plugins/bilbo/.codex-plugin/plugin.json",
    "plugins/bilbo/hooks.json",
    "plugins/bilbo/hooks/hooks.json",
    "plugins/bilbo/skills/note/SKILL.md",
    "plugins/bilbo/skills/recall/SKILL.md",
];

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel names the plugin's anchor file through PLUGIN_ANCHOR"
)]
fn root() -> PathBuf {
    match std::env::var("PLUGIN_ANCHOR") {
        Ok(anchor) => {
            let runfiles = runfiles::Runfiles::create().expect("runfiles");
            let anchor = runfiles
                .rlocation(anchor)
                .expect("the anchor is in the runfiles");
            anchor
                .parent()
                .and_then(Path::parent)
                .expect("the repo's root")
                .to_path_buf()
        }
        Err(_) => Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
    }
}

fn read(name: &str) -> String {
    let path = root().join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn json(name: &str) -> Value {
    serde_json::from_str(&read(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn walk(base: &Path, dir: &Path, found: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            walk(base, &path, found);
        } else {
            let name = path.strip_prefix(base).expect("under the root");
            found.insert(name.to_string_lossy().into_owned());
        }
    }
}

const SKILLS: [(&str, &str); 2] = [
    ("note", "plugins/bilbo/skills/note/SKILL.md"),
    ("recall", "plugins/bilbo/skills/recall/SKILL.md"),
];

fn frontmatter(text: &str) -> Vec<(String, String)> {
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("---"));
    lines
        .take_while(|line| *line != "---")
        .map(|line| {
            let (key, value) = line.split_once(": ").expect("key: value");
            (key.to_owned(), value.to_owned())
        })
        .collect()
}

#[test]
fn the_plugin_holds_its_files_and_no_other() {
    let base = root();
    let mut found = BTreeSet::new();
    for folder in ["plugins", ".claude-plugin", ".agents/plugins"] {
        walk(&base, &base.join(folder), &mut found);
    }
    assert_eq!(found, NINE.iter().map(|name| (*name).to_owned()).collect());
    let mut skills = BTreeSet::new();
    walk(&base, &base.join("plugins/bilbo/skills"), &mut skills);
    let folders: BTreeSet<&str> = skills
        .iter()
        .map(|name| name.split('/').nth(3).expect("a skill's folder"))
        .collect();
    assert_eq!(folders, BTreeSet::from(["note", "recall"]));
}

#[test]
fn both_marketplaces_name_bilbo_and_point_at_the_plugin() {
    let claude = json(".claude-plugin/marketplace.json");
    assert_eq!(claude["name"], "bilbo");
    assert_eq!(claude["plugins"].as_array().map(Vec::len), Some(1));
    assert_eq!(claude["plugins"][0]["name"], "bilbo");
    assert_eq!(claude["plugins"][0]["source"], "./plugins/bilbo");
    let codex = json(".agents/plugins/marketplace.json");
    assert_eq!(codex["name"], "bilbo");
    assert_eq!(codex["plugins"].as_array().map(Vec::len), Some(1));
    assert_eq!(codex["plugins"][0]["name"], "bilbo");
    assert_eq!(codex["plugins"][0]["source"]["path"], "./plugins/bilbo");
}

#[test]
fn claudes_manifests_set_no_version_and_codexs_sets_bilbos() {
    for name in [
        ".claude-plugin/marketplace.json",
        "plugins/bilbo/.claude-plugin/plugin.json",
        "plugins/bilbo/.antigravity-plugin/plugin.json",
    ] {
        let value = json(name);
        assert!(value.get("version").is_none(), "{name}");
        assert!(value["plugins"][0].get("version").is_none(), "{name}");
    }
    assert_eq!(
        json("plugins/bilbo/.codex-plugin/plugin.json")["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn the_descriptions_agree() {
    let marketplace = json(".claude-plugin/marketplace.json");
    let mut found = vec![marketplace["plugins"][0]["description"].clone()];
    for name in [
        "plugins/bilbo/.claude-plugin/plugin.json",
        "plugins/bilbo/.codex-plugin/plugin.json",
        "plugins/bilbo/.antigravity-plugin/plugin.json",
    ] {
        found.push(json(name)["description"].clone());
    }
    assert!(found[0].as_str().is_some_and(|text| !text.is_empty()));
    assert!(found.iter().all(|text| *text == found[0]), "{found:?}");
}

#[test]
fn a_skills_frontmatter_holds_the_four_keys_in_order_and_its_folders_name() {
    for (folder, name) in SKILLS {
        let keys: Vec<(String, String)> = frontmatter(&read(name));
        assert_eq!(
            keys.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
            ["name", "description", "license", "allowed-tools"],
            "{name}"
        );
        assert_eq!(keys[0].1, folder, "{name}");
    }
}

#[test]
fn a_description_is_one_line_of_at_most_1024_characters() {
    for (_, name) in SKILLS {
        let text = read(name);
        let description = frontmatter(&text)
            .into_iter()
            .find(|(key, _)| key == "description")
            .map(|(_, value)| value)
            .expect("a description");
        assert!(!description.is_empty() && !description.contains('\n'));
        assert!(description.chars().count() <= 1024, "{name}");
    }
}

fn entries(text: &str) -> Vec<String> {
    frontmatter(text)
        .into_iter()
        .find(|(key, _)| key == "allowed-tools")
        .map(|(_, value)| value)
        .expect("allowed-tools")
        .split(", ")
        .filter_map(|entry| entry.strip_prefix("Bash(")?.strip_suffix(')'))
        .map(str::to_owned)
        .collect()
}

fn covers(entries: &[String], command: &str) -> bool {
    entries.iter().any(|entry| match entry.strip_suffix(" *") {
        Some(prefix) => command.starts_with(&format!("{prefix} ")),
        None => command == entry,
    })
}

fn commands(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut closing = None;
    for line in text.lines() {
        let trimmed = line.trim();
        match &closing {
            None if trimmed == "```bash" => closing = Some(line.len() - line.trim_start().len()),
            Some(indent) if trimmed == "```" && line.len() - line.trim_start().len() == *indent => {
                closing = None;
            }
            Some(_) if !trimmed.is_empty() => found.push(trimmed.to_owned()),
            _ => {}
        }
    }
    found
}

#[test]
fn every_command_a_skill_runs_is_allowed_or_left_to_the_person() {
    for (_, name) in SKILLS {
        let text = read(name);
        let allowed = entries(&text);
        let found = commands(&text);
        assert!(!found.is_empty(), "{name}");
        for left in ["rm x", "mv x y", "bilbo forget x"] {
            assert!(!covers(&allowed, left), "{name}: {left}");
        }
        for command in found {
            assert!(
                !command.contains(';') && !command.contains("&&") && !command.contains(" | "),
                "{name}: {command}"
            );
            let left = command.starts_with("rm ")
                || (command.starts_with("mv ") && !command.starts_with("mv -n "))
                || command.starts_with("bilbo forget ");
            if left {
                assert!(!covers(&allowed, &command), "{name}: {command}");
            } else {
                assert!(covers(&allowed, &command), "{name}: {command}");
            }
        }
    }
}

fn hook_files() -> (Value, Value) {
    (
        json("plugins/bilbo/hooks/hooks.json"),
        json("plugins/bilbo/hooks.json"),
    )
}

const COMPACT: &str = "command -v bilbo >/dev/null 2>&1 || exit 0; echo 'Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.'";

#[test]
fn the_hooks_file_holds_the_two_events_and_their_commands() {
    assert_eq!(
        hook_files().0,
        json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [{
                        "type": "command",
                        "command": "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0",
                        "timeout": 10
                    }]
                }],
                "SessionStart": [{
                    "matcher": "compact",
                    "hooks": [{
                        "type": "command",
                        "command": COMPACT,
                        "timeout": 10
                    }]
                }]
            }
        })
    );
}

#[test]
fn agys_hooks_file_runs_the_digest_before_each_model_call() {
    assert_eq!(
        hook_files().1,
        json!({
            "bilbo-digest": {
                "PreInvocation": [{
                    "type": "command",
                    "command": "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest --agy; exit 0",
                    "timeout": 10
                }]
            }
        })
    );
}

fn prompt_hooks() -> [(String, &'static str); 2] {
    let (shared, agy) = hook_files();
    [
        (
            shared["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
                .as_str()
                .expect("a command")
                .to_owned(),
            "digest",
        ),
        (
            agy["bilbo-digest"]["PreInvocation"][0]["command"]
                .as_str()
                .expect("a command")
                .to_owned(),
            "digest --agy",
        ),
    ]
}

fn compact_hook() -> String {
    hook_files().0["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("a command")
        .to_owned()
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bilbo-plugin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_hook(command: &str, path: &Path) -> Ran {
    let done = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .env_clear()
        .env("PATH", path)
        .output()
        .expect("sh runs");
    Ran {
        code: done.status.code(),
        stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
    }
}

fn stand_in(dir: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("bilbo");
    fs::write(&path, format!("#!/bin/sh\n{body}")).expect("a stand-in");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable");
}

#[test]
fn a_prompt_hook_is_silent_and_exits_0_without_bilbo() {
    let empty = scratch();
    for (command, _) in prompt_hooks() {
        let ran = run_hook(&command, &empty);
        assert_eq!(
            (ran.code, ran.stdout.as_str(), ran.stderr.as_str()),
            (Some(0), "", ""),
            "{command}"
        );
    }
}

#[test]
fn a_prompt_hook_exits_0_when_bilbo_exits_2() {
    for (command, asked) in prompt_hooks() {
        let dir = scratch();
        let log = dir.join("calls");
        stand_in(
            &dir,
            &format!(
                "echo \"$*\" >> '{}'\necho blocked >&2\nexit 2\n",
                log.display()
            ),
        );
        let ran = run_hook(&command, &dir);
        assert_eq!((ran.code, ran.stdout.as_str()), (Some(0), ""), "{command}");
        assert_eq!(
            fs::read_to_string(&log).expect("a call"),
            format!("{asked}\n")
        );
    }
}

#[test]
fn the_compact_hook_is_silent_without_bilbo_and_never_runs_it() {
    let command = compact_hook();
    let empty = scratch();
    let ran = run_hook(&command, &empty);
    assert_eq!(
        (ran.code, ran.stdout.as_str(), ran.stderr.as_str()),
        (Some(0), "", "")
    );
    let dir = scratch();
    let mark = dir.join("mark");
    stand_in(&dir, &format!("echo ran > '{}'\n", mark.display()));
    let ran = run_hook(&command, &dir);
    assert_eq!(ran.code, Some(0));
    assert!(
        ran.stdout.starts_with("Context was compacted."),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("bilbo note skill"));
    assert_eq!(ran.stdout.lines().count(), 1);
    assert!(!mark.exists());
}

#[test]
fn no_manifest_names_the_hooks() {
    for name in [
        "plugins/bilbo/.claude-plugin/plugin.json",
        "plugins/bilbo/.codex-plugin/plugin.json",
        "plugins/bilbo/.antigravity-plugin/plugin.json",
    ] {
        assert!(json(name).get("hooks").is_none(), "{name}");
    }
}

#[test]
fn the_skills_name_the_commands_and_lines_bilbo_has() {
    let recall = read("plugins/bilbo/skills/recall/SKILL.md");
    for needle in [
        "bilbo recall -- '",
        "bilbo recall --in='<value>' -- '",
        "--page 2",
        "bilbo read '<ref>'",
        "`nothing matched`",
        "`nothing on page <n>: the list ended on page <m>`",
        "`no store at <root>`",
        "`another bilbo is writing the store`",
        "`--in '<value>' names no entry`",
        "`the index is behind the store by <n> entries`",
        "`the index cannot be compared with the store's catalog`",
        "`--in may name fewer entries than it should`",
        "more: ",
    ] {
        assert!(recall.contains(needle), "recall: {needle}");
    }
    let note = read("plugins/bilbo/skills/note/SKILL.md");
    for needle in [
        "bilbo config follow",
        "bilbo remember '",
        "bilbo recall",
        "bilbo read '",
        "bilbo forget '",
        "test -e '",
        "test -L '",
        "realpath '",
        "mv -n '",
        "bilbo setup",
    ] {
        assert!(note.contains(needle), "note: {needle}");
    }
}
