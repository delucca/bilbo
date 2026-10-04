//! The agent plugin's files, checked by string and by running the hook command: no tool is needed in CI.

mod common;

use common::TempDir;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> String {
    fs::read_to_string(path(relative)).unwrap_or_else(|e| panic!("cannot read {relative}: {e}"))
}

/// The string value of the first JSON key `key`, whatever the formatting.
fn json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let rest = text[text.find(&needle)? + needle.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

const CLAUDE_MANIFEST: &str = "plugins/bilbo/.claude-plugin/plugin.json";
const CODEX_MANIFEST: &str = "plugins/bilbo/.codex-plugin/plugin.json";
const CLAUDE_MARKETPLACE: &str = ".claude-plugin/marketplace.json";
const CODEX_MARKETPLACE: &str = ".agents/plugins/marketplace.json";
const RECALL_SKILL: &str = "plugins/bilbo/skills/recall/SKILL.md";
const NOTE_SKILL: &str = "plugins/bilbo/skills/note/SKILL.md";
const REFERENCE_SKILL: &str = "plugins/bilbo/skills/reference/SKILL.md";
const READER_BRIEF: &str = "plugins/bilbo/skills/reference/references/reader.md";
const HOOKS: &str = "plugins/bilbo/hooks/hooks.json";
const COMPACT_COMMAND: &str = "command -v bilbo >/dev/null 2>&1 || exit 0; echo 'Context was compacted. If this session settled something later sessions should know, such as a decision, a gotcha or a plan, save it with the bilbo note skill once the current task allows.'";
const DIGEST_COMMAND: &str = "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0";

#[test]
fn plugin_files_exist() {
    for file in [
        CLAUDE_MANIFEST,
        CODEX_MANIFEST,
        CLAUDE_MARKETPLACE,
        CODEX_MARKETPLACE,
        RECALL_SKILL,
        NOTE_SKILL,
        REFERENCE_SKILL,
        READER_BRIEF,
        HOOKS,
    ] {
        assert!(path(file).is_file(), "{file} is not a file");
    }
    for absent in ["agents", ".mcp.json"] {
        let full = format!("plugins/bilbo/{absent}");
        assert!(!path(&full).exists(), "{full} must not exist");
    }
    let mut skills: Vec<String> = fs::read_dir(path("plugins/bilbo/skills"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    skills.sort();
    assert_eq!(skills, ["note", "recall", "reference"]);
}

#[test]
fn marketplaces_point_at_the_plugin() {
    for file in [CLAUDE_MARKETPLACE, CODEX_MARKETPLACE] {
        let text = read(file);
        assert_eq!(
            json_string(&text, "name").as_deref(),
            Some("bilbo"),
            "{file} name"
        );
        assert!(
            text.contains("\"./plugins/bilbo\""),
            "{file} has no source ./plugins/bilbo"
        );
    }
}

#[test]
fn claude_manifests_set_no_version() {
    for file in [CLAUDE_MARKETPLACE, CLAUDE_MANIFEST] {
        assert!(!read(file).contains("\"version\""), "{file} sets a version");
    }
}

#[test]
fn codex_version_matches_cargo() {
    let got = json_string(&read(CODEX_MANIFEST), "version");
    assert_eq!(
        got.as_deref(),
        Some(env!("CARGO_PKG_VERSION")),
        "{CODEX_MANIFEST} version differs from Cargo.toml"
    );
}

/// The `(key, value)` pairs of a skill's frontmatter, in file order.
fn frontmatter(file: &str) -> Vec<(String, String)> {
    let text = read(file);
    let rest = text
        .strip_prefix("---\n")
        .expect("the skill opens with frontmatter");
    rest.lines()
        .take_while(|line| *line != "---")
        .map(|line| {
            let (key, value) = line.split_once(": ").expect("a key: value line");
            (key.to_string(), value.to_string())
        })
        .collect()
}

fn folder(file: &str) -> std::ffi::OsString {
    path(file).parent().unwrap().file_name().unwrap().to_owned()
}

#[test]
fn recall_skill_frontmatter() {
    let front = frontmatter(RECALL_SKILL);
    let keys: Vec<&str> = front.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["name", "description", "license", "allowed-tools"]);
    assert_eq!(front[0].1, "recall");
    assert_eq!(folder(RECALL_SKILL), "recall");
}

#[test]
fn note_skill_frontmatter() {
    let front = frontmatter(NOTE_SKILL);
    let keys: Vec<&str> = front.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["name", "description", "license", "allowed-tools"]);
    assert_eq!(front[0].1, "note");
    assert_eq!(front[2].1, "Apache-2.0");
    assert_eq!(folder(NOTE_SKILL), "note");
    assert_eq!(
        front[3].1,
        "Bash(command -v bilbo), Bash(bilbo recall *), Bash(bilbo new *), Bash(bilbo check), Bash(mv -n *), Read, Edit"
    );
}

#[test]
fn note_skill_description_says_when() {
    let front = frontmatter(NOTE_SKILL);
    let description = &front[1].1;
    for needle in [
        "later session",
        "decision",
        "gotcha",
        "plan",
        "note this",
        "note that",
        "NOT for",
    ] {
        assert!(
            description.contains(needle),
            "the description lacks {needle:?}"
        );
    }
}

#[test]
fn recall_skill_drives_bilbo() {
    let text = read(RECALL_SKILL);
    for needle in [
        "command -v bilbo",
        "bilbo recall [--kind K]... [--limit N] -- '",
        "bilbo: no notes match",
        "Judge exit 1 by the last stderr line",
        "keyword results only",
        "at most two more queries",
        "say which query produced the hits",
        "recall: bilbo is not on PATH; install the bilbo CLI first",
    ] {
        assert!(text.contains(needle), "the skill lacks {needle:?}");
    }
    assert!(
        !text.contains("nbrecall"),
        "the skill names the legacy tool"
    );
}

/// The trimmed non-empty lines inside the ```` ```bash ```` fences of a skill, a heredoc's body left out.
fn bash_lines(text: &str) -> Vec<String> {
    let mut in_bash = false;
    let mut in_heredoc = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if in_heredoc {
            in_heredoc = trimmed != "EOF";
        } else if trimmed.starts_with("```") {
            in_bash = trimmed == "```bash";
        } else if in_bash && !trimmed.is_empty() {
            in_heredoc = trimmed.ends_with("<<'EOF'");
            lines.push(trimmed.to_string());
        }
    }
    lines
}

#[test]
fn note_skill_drives_bilbo() {
    let text = read(NOTE_SKILL);
    let body = text.splitn(3, "---\n").nth(2).unwrap();
    let commands = bash_lines(&text);
    for command in [
        "command -v bilbo",
        "bilbo recall --limit 5 -- '<words that name the subject>'",
        "bilbo new <kind> <topic> [--title '<title>']",
        "bilbo check",
        "mv -n '<root>/notes/<old kind>-<topic>.md' '<root>/notes/<new kind>-<topic>.md'",
    ] {
        assert!(
            commands.iter().any(|c| c == command),
            "no bash line {command:?}"
        );
    }
    for needle in [
        "note: bilbo is not on PATH; install the bilbo CLI first",
        "bilbo: no notes match",
        "bilbo: no store at <root>",
        "already has a note",
        "sources:\n     - \"code: src/new.rs\"\n     - \"url: https://example.org/page\"",
    ] {
        assert!(text.contains(needle), "the skill lacks {needle:?}");
    }
    for banned in [
        "nbrecall",
        "mint-ulid",
        "date +%F",
        "Notebooks",
        "notebook.org",
        "argument-hint",
        "supersedes:",
    ] {
        assert!(!text.contains(banned), "the skill holds {banned:?}");
    }
    assert!(
        !body.lines().any(|l| l.starts_with("kind:")),
        "the skill holds a kind: line"
    );
}

const LEGACY_STRINGS: [&str; 9] = [
    "nbrecall",
    "check_citations",
    "plan_reads",
    "source-reader",
    "uv run",
    "Notebooks",
    "index.md",
    "argument-hint",
    "note: <",
];

#[test]
fn reference_skill_frontmatter() {
    let front = frontmatter(REFERENCE_SKILL);
    let keys: Vec<&str> = front.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["name", "description", "license", "allowed-tools"]);
    assert_eq!(front[0].1, "reference");
    assert_eq!(front[2].1, "Apache-2.0");
    assert_eq!(folder(REFERENCE_SKILL), "reference");
    assert_eq!(
        front[3].1,
        "Bash(command -v bilbo), Bash(bilbo library *), Bash(bilbo cite *), Read, Agent(general-purpose), SendMessage"
    );
}

#[test]
fn reference_skill_description_says_when() {
    let front = frontmatter(REFERENCE_SKILL);
    let description = &front[1].1;
    for needle in ["sources", "NOT for", "recall"] {
        assert!(
            description.contains(needle),
            "the description lacks {needle:?}"
        );
    }
}

#[test]
fn reference_skill_drives_bilbo() {
    let text = read(REFERENCE_SKILL);
    let commands = bash_lines(&text);
    for command in [
        "command -v bilbo",
        "bilbo library",
        "bilbo library <corpus>",
        "bilbo library show '<corpus>/<name>#<anchor>'",
        "bilbo library plan '<ref>'...",
        "bilbo library read <plan> <slice>",
        "bilbo library read <plan> <slice> --part 1/2",
        "bilbo cite --plan <plan> <<'EOF'",
    ] {
        assert!(
            commands.iter().any(|c| c == command),
            "no bash line {command:?}"
        );
    }
    for needle in [
        "reference: bilbo is not on PATH; install the bilbo CLI first",
        "bilbo: no store at <root>",
        "picks from <corpus> (<k> of <n> sources):",
        "-- end slice",
        "lookup only, not searched",
        "general-purpose",
        "references/reader.md",
        "100,000",
        "`ok`",
        "`quote_elsewhere`",
        "`ambiguous`",
        "`too_short`",
        "`quote_missing`",
        "`anchor_missing`",
        "`unread`",
        "`id_missing`",
    ] {
        assert!(text.contains(needle), "the skill lacks {needle:?}");
    }
    for banned in LEGACY_STRINGS {
        assert!(!text.contains(banned), "the skill holds {banned:?}");
    }
}

#[test]
fn reader_brief_holds_its_sections() {
    let text = read(READER_BRIEF);
    for needle in [
        "## Reading rules",
        "## Citation rules",
        "## Support judgment",
        "## Output contract",
        "## Boundaries",
        "bilbo library read",
        "--part",
        "bilbo cite --plan",
        "read:",
        "not read:",
    ] {
        assert!(text.contains(needle), "the brief lacks {needle:?}");
    }
    for banned in LEGACY_STRINGS {
        assert!(!text.contains(banned), "the brief holds {banned:?}");
    }
}

/// Every command line in a skill's `bash` fences is one command that `allowed-tools` lets through.
#[test]
fn skills_allow_every_command_they_run() {
    for file in [RECALL_SKILL, NOTE_SKILL, REFERENCE_SKILL] {
        let front = frontmatter(file);
        let allowed: Vec<String> = front[3]
            .1
            .split(", ")
            .filter_map(|e| e.strip_prefix("Bash(")?.strip_suffix(')'))
            .map(str::to_string)
            .collect();
        let text = read(file);
        for trimmed in bash_lines(&text) {
            let trimmed = trimmed.as_str();
            for chain in ["&&", ";", "|"] {
                assert!(
                    !trimmed.contains(chain),
                    "{file}: {trimmed:?} chains with {chain:?}"
                );
            }
            let ok = allowed
                .iter()
                .any(|pattern| match pattern.strip_suffix('*') {
                    Some(prefix) => trimmed.starts_with(prefix) || trimmed == prefix.trim_end(),
                    None => trimmed == pattern,
                });
            assert!(ok, "{file}: {trimmed:?} is outside allowed-tools");
        }
    }
}

fn json(file: &str) -> serde_json::Value {
    serde_json::from_str(&read(file)).unwrap_or_else(|e| panic!("{file} is not JSON: {e}"))
}

#[test]
fn plugin_descriptions_agree() {
    let claude = json(CLAUDE_MANIFEST)["description"].clone();
    let codex = json(CODEX_MANIFEST)["description"].clone();
    let market = json(CLAUDE_MARKETPLACE)["plugins"][0]["description"].clone();
    assert_eq!(claude, codex);
    assert_eq!(claude, market);
    let text = claude.as_str().expect("a string description");
    for needle in ["Write", "recall", "sources"] {
        assert!(text.contains(needle), "{text}");
    }
}

#[test]
fn codex_interface_names_write() {
    let manifest = json(CODEX_MANIFEST);
    let interface = &manifest["interface"];
    assert_eq!(
        interface["capabilities"],
        serde_json::json!(["Read", "Write"])
    );
    let prompts = interface["defaultPrompt"].as_array().unwrap();
    assert_eq!(prompts.len(), 3);
    for prompt in prompts {
        assert!(prompt.as_str().unwrap().chars().count() <= 128);
    }
}

#[test]
fn digest_hook_runs_bilbo_digest_and_never_blocks() {
    let hooks: serde_json::Value = serde_json::from_str(&read(HOOKS)).expect("hooks.json is JSON");
    let events = hooks["hooks"].as_object().expect("a hooks object");
    assert_eq!(
        events.keys().collect::<Vec<_>>(),
        ["SessionStart", "UserPromptSubmit"]
    );
    let groups = events["UserPromptSubmit"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert!(groups[0].get("matcher").is_none(), "no matcher");
    let handlers = groups[0]["hooks"].as_array().unwrap();
    assert_eq!(handlers.len(), 1);
    assert_eq!(
        handlers[0],
        serde_json::json!({"type": "command", "command": DIGEST_COMMAND, "timeout": 5})
    );
}

#[test]
fn manifests_do_not_name_the_hooks() {
    for file in [CLAUDE_MANIFEST, CODEX_MANIFEST] {
        assert!(!read(file).contains("\"hooks\""), "{file} names hooks");
    }
}

/// The hook's command from hooks.json, run under `/bin/sh -c` with `bin` as the whole PATH.
fn run_hook(event: &str, bin: &std::path::Path) -> std::process::Output {
    let hooks: serde_json::Value = serde_json::from_str(&read(HOOKS)).unwrap();
    let command = hooks["hooks"][event][0]["hooks"][0]["command"]
        .as_str()
        .expect("a command");
    Command::new("/bin/sh")
        .args(["-c", command])
        .env_clear()
        .env("PATH", bin)
        .output()
        .expect("sh runs")
}

#[test]
fn compaction_hook_runs_on_compact_only() {
    let hooks = json(HOOKS);
    let groups = hooks["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["matcher"], "compact");
    let handlers = groups[0]["hooks"].as_array().unwrap();
    assert_eq!(handlers.len(), 1);
    assert_eq!(
        handlers[0],
        serde_json::json!({"type": "command", "command": COMPACT_COMMAND, "timeout": 5})
    );
}

#[test]
fn digest_hook_is_silent_without_bilbo() {
    let dir = TempDir::new("hook-absent");
    let out = run_hook("UserPromptSubmit", dir.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}

#[test]
fn digest_hook_exits_zero_when_bilbo_fails() {
    let dir = TempDir::new("hook-old");
    let fake = dir.path().join("bilbo");
    // Written by a child process: a write fd held here can make the exec fail with ETXTBSY.
    let made = Command::new("/bin/sh")
        .args([
            "-c",
            r#"printf '%s\n' '#!/bin/sh' 'echo old >&2' 'exit 2' > "$1" && chmod 755 "$1""#,
            "sh",
        ])
        .arg(&fake)
        .status()
        .unwrap();
    assert!(made.success());
    let out = run_hook("UserPromptSubmit", dir.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, b"old\n");
}

#[test]
fn compaction_hook_is_silent_without_bilbo() {
    let dir = TempDir::new("compact-absent");
    let out = run_hook("SessionStart", dir.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}

#[test]
fn compaction_hook_nudges_without_running_bilbo() {
    let dir = TempDir::new("compact-nudge");
    let fake = dir.path().join("bilbo");
    let marker = dir.path().join("ran");
    // Written by a child process: a write fd held here can make the exec fail with ETXTBSY.
    let made = Command::new("/bin/sh")
        .args([
            "-c",
            r#"printf '%s\n' '#!/bin/sh' "touch '$2'" 'exit 2' > "$1" && chmod 755 "$1""#,
            "sh",
        ])
        .arg(&fake)
        .arg(&marker)
        .status()
        .unwrap();
    assert!(made.success());
    let out = run_hook("SessionStart", dir.path());
    assert_eq!(out.status.code(), Some(0));
    let echoed = COMPACT_COMMAND
        .split("echo '")
        .nth(1)
        .unwrap()
        .strip_suffix('\'')
        .unwrap();
    assert_eq!(out.stdout, format!("{echoed}\n").into_bytes());
    assert!(out.stderr.is_empty());
    assert!(!marker.exists(), "the hook ran bilbo");
}
