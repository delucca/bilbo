//! Every action a workflow uses is pinned by a full commit SHA, the way `ci.yml` pins its own.

use std::fs;
use std::path::PathBuf;

/// The value of a `uses:` line, without quotes or a trailing comment.
fn uses(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix('-').unwrap_or(line).trim_start();
    let value = match line.strip_prefix('{') {
        Some(flow) => {
            let (_, rest) = flow.split_once("uses:")?;
            rest.split([',', '}']).next().unwrap()
        }
        None => line.strip_prefix("uses:")?,
    };
    let value = value.split(" #").next().unwrap().trim();
    Some(value.trim_matches(|c| c == '"' || c == '\''))
}

/// A local action (`./path`) runs from the same commit; anything else needs `@<40 lowercase hex>`.
fn pinned(value: &str) -> bool {
    if value.starts_with("./") {
        return true;
    }
    value.rsplit_once('@').is_some_and(|(_, sha)| {
        sha.len() == 40 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

#[test]
fn pin_rules() {
    for value in [
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "github/codeql-action/init@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "./.github/actions/setup",
    ] {
        assert!(pinned(value), "{value} should count as pinned");
    }
    for value in [
        "actions/checkout@v6",
        "actions/checkout@main",
        "actions/checkout@3D3C42E5AAC5BA805825DA76410C181273BA90B1",
        "actions/checkout@3d3c42e5",
        "actions/checkout",
        "docker://alpine:3.20",
    ] {
        assert!(!pinned(value), "{value} should not count as pinned");
    }
    assert_eq!(
        uses(r#"      - uses: "actions/checkout@v6" # v6"#),
        Some("actions/checkout@v6")
    );
    assert_eq!(
        uses("        uses: actions/upload-artifact@v7"),
        Some("actions/upload-artifact@v7")
    );
    assert_eq!(
        uses("      -   uses: actions/cache@v4"),
        Some("actions/cache@v4")
    );
    assert_eq!(
        uses("      - { uses: actions/cache@v4 }"),
        Some("actions/cache@v4")
    );
    assert_eq!(
        uses("      - {name: x, uses: actions/cache@v4}"),
        Some("actions/cache@v4")
    );
    assert_eq!(uses("      # uses: actions/checkout@v6"), None);
    assert_eq!(uses("      - name: uses"), None);
}

#[test]
fn every_action_is_pinned_by_sha() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "{} holds no workflow", dir.display());
    let mut unpinned = Vec::new();
    for file in &files {
        let name = file.file_name().unwrap().to_string_lossy();
        let text = fs::read_to_string(file).unwrap();
        for (n, line) in text.lines().enumerate() {
            if uses(line).is_some_and(|value| !pinned(value)) {
                unpinned.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        unpinned.is_empty(),
        "actions not pinned by commit SHA:\n{}",
        unpinned.join("\n")
    );
}
