//! The agent plugin's files, checked by string: no tool is needed in CI.

use std::fs;
use std::path::PathBuf;

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

#[test]
fn plugin_files_exist() {
    for file in [
        CLAUDE_MANIFEST,
        CODEX_MANIFEST,
        CLAUDE_MARKETPLACE,
        CODEX_MARKETPLACE,
        SKILL,
    ] {
        assert!(path(file).is_file(), "{file} is not a file");
    }
    for absent in ["hooks", "agents", ".mcp.json"] {
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
