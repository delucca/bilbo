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
const SKILL: &str = "plugins/bilbo/skills/recall/SKILL.md";
const HOOKS: &str = "plugins/bilbo/hooks/hooks.json";
const DIGEST_COMMAND: &str = "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo digest; exit 0";

#[test]
fn plugin_files_exist() {
    for file in [
        CLAUDE_MANIFEST,
        CODEX_MANIFEST,
        CLAUDE_MARKETPLACE,
        CODEX_MARKETPLACE,
        SKILL,
        HOOKS,
    ] {
        assert!(path(file).is_file(), "{file} is not a file");
    }
    for absent in ["agents", ".mcp.json"] {
        let full = format!("plugins/bilbo/{absent}");
        assert!(!path(&full).exists(), "{full} must not exist");
    }
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

#[test]
fn recall_skill_frontmatter() {
    let text = read(SKILL);
    let rest = text
        .strip_prefix("---\n")
        .expect("the skill opens with frontmatter");
    let front: Vec<&str> = rest.lines().take_while(|line| *line != "---").collect();
    let keys: Vec<&str> = front
        .iter()
        .map(|line| line.split(':').next().unwrap())
        .collect();
    assert_eq!(keys, ["name", "description", "license", "allowed-tools"]);
    assert_eq!(front[0], "name: recall");
    let folder = path(SKILL)
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_owned();
    assert_eq!(folder, "recall");
}

#[test]
fn recall_skill_drives_bilbo() {
    let text = read(SKILL);
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

#[test]
fn digest_hook_runs_bilbo_digest_and_never_blocks() {
    let hooks: serde_json::Value = serde_json::from_str(&read(HOOKS)).expect("hooks.json is JSON");
    let events = hooks["hooks"].as_object().expect("a hooks object");
    assert_eq!(events.keys().collect::<Vec<_>>(), ["UserPromptSubmit"]);
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
fn run_hook(bin: &std::path::Path) -> std::process::Output {
    let hooks: serde_json::Value = serde_json::from_str(&read(HOOKS)).unwrap();
    let command = hooks["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
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
fn digest_hook_is_silent_without_bilbo() {
    let dir = TempDir::new("hook-absent");
    let out = run_hook(dir.path());
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
    let out = run_hook(dir.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, b"old\n");
}
