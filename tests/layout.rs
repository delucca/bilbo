//! The shape of `src/` that AGENTS.md's Architecture rules state: one folder per domain with its verbs inside,
//! the Shared Kernel in `shared/`, verbs reached only from `main`, and each listed crate in its own files.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The domains, one folder each under `src/`.
const DOMAINS: [&str; 9] = [
    "citation", "host", "identity", "library", "note", "relay", "search", "setup", "sync",
];

/// The verbs, by path below `src/`: a file, or a folder (ending in `/`) whose files are all the verb's.
const VERBS: [&str; 16] = [
    "check.rs",
    "citation/cite.rs",
    "identity/device.rs",
    "identity/pair/",
    "library/cli/",
    "note/history.rs",
    "note/new.rs",
    "note/restore.rs",
    "note/scope.rs",
    "note/watch.rs",
    "relay/",
    "search/digest.rs",
    "search/index.rs",
    "search/recall.rs",
    "setup/",
    "sync/cli.rs",
];

/// Each crate and the only files under `src/` that may name it. `tests/common` also uses `sha2`, to write library
/// files with a correct digest.
const PLACEMENT: [(&str, &[&str]); 18] = [
    ("cliclack", &["host/prompt.rs"]),
    ("console", &["host/terminal.rs"]),
    (
        "libc",
        &[
            "host/prompt.rs",
            "host/swap.rs",
            "host/terminal.rs",
            "identity/keys.rs",
        ],
    ),
    ("notify", &["note/watch.rs"]),
    ("ring", &["host/model.rs"]),
    ("sha2", &["identity/keys.rs", "shared/hash.rs"]),
    ("unicode_normalization", &["shared/text.rs"]),
    ("htmd", &["library/html.rs"]),
    ("markup5ever_rcdom", &["library/html.rs"]),
    ("ed25519_dalek", &["identity/keys.rs"]),
    ("hpke", &["identity/keys.rs"]),
    ("chacha20poly1305", &["identity/keys.rs"]),
    ("hkdf", &["identity/keys.rs"]),
    ("getrandom", &["identity/keys.rs"]),
    ("base64", &["identity/pake.rs", "sync/segment.rs"]),
    ("spake2", &["identity/pake.rs"]),
    ("rand_core", &["identity/pake.rs"]),
    ("httparse", &["relay/http.rs"]),
];

const PRINTS: [&str; 5] = ["print!(", "println!(", "eprint!(", "eprintln!(", "dbg!("];

struct File {
    /// The path below `src/`, `/`-separated.
    rel: String,
    /// The code before `#[cfg(test)] mod tests`, each line cut at its first `//`.
    code: String,
}

impl File {
    /// The entry directly under `src/`: `library` for `library/cli/land.rs`, `check` for `check.rs`.
    fn top(&self) -> &str {
        let first = self.rel.split('/').next().unwrap();
        first.strip_suffix(".rs").unwrap_or(first)
    }

    /// The verb this file belongs to, if any.
    fn verb(&self) -> Option<&'static str> {
        VERBS.into_iter().find(|v| match v.strip_suffix('/') {
            Some(folder) => self.rel.starts_with(&format!("{folder}/")),
            None => self.rel == *v,
        })
    }

    fn paths(&self) -> BTreeSet<String> {
        crate_paths(&self.code)
    }
}

/// The module path of a verb: `library::cli` for `library/cli/`, `check` for `check.rs`.
fn module_of(verb: &str) -> String {
    verb.trim_end_matches('/')
        .trim_end_matches(".rs")
        .replace('/', "::")
}

/// Whether `path` is module `module` or something inside it.
fn within(path: &str, module: &str) -> bool {
    path == module || path.starts_with(&format!("{module}::"))
}

fn files() -> Vec<File> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    walk(&src, &src, &mut found);
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    found
}

fn walk(src: &Path, dir: &Path, found: &mut Vec<File>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(src, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let rel = path
                .strip_prefix(src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            found.push(File {
                rel,
                code: production(&text),
            });
        }
    }
}

/// `text` up to the `#[cfg(test)]` line that opens `mod tests`, each line cut at its first `//`.
fn production(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim_start().starts_with("mod tests"))
        .unwrap_or(lines.len());
    lines[..end]
        .iter()
        .map(|line| line.split("//").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `code` holds `word` with no identifier character just before it.
fn names(code: &str, word: &str) -> bool {
    code.match_indices(word)
        .any(|(at, _)| !code[..at].chars().next_back().is_some_and(ident_char))
}

/// Every `crate::` path in `code`, `use` groups expanded: `crate::a::{b, c::{self, d}}` gives `a::b`, `a::c` and
/// `a::c::d`.
fn crate_paths(code: &str) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for (at, _) in code.match_indices("crate::") {
        if !code[..at].chars().next_back().is_some_and(ident_char) {
            use_tree(code, at + "crate::".len(), "", &mut paths);
        }
    }
    paths
}

fn skip_space(text: &str, at: usize) -> usize {
    at + text[at..].len() - text[at..].trim_start().len()
}

fn ident(text: &str, at: usize) -> usize {
    text[at..]
        .find(|c: char| !ident_char(c))
        .map_or(text.len(), |n| at + n)
}

/// Adds the paths of the use tree that starts at `at`, below `prefix`; returns where the tree ends.
fn use_tree(text: &str, at: usize, prefix: &str, paths: &mut BTreeSet<String>) -> usize {
    let mut at = skip_space(text, at);
    if text[at..].starts_with('{') {
        at += 1;
        loop {
            at = skip_space(text, at);
            if text[at..].starts_with('}') {
                return at + 1;
            }
            let next = use_tree(text, at, prefix, paths);
            if next == at {
                return at;
            }
            at = skip_space(text, next);
            if text[at..].starts_with(',') {
                at += 1;
            }
        }
    }
    let end = ident(text, at);
    let name = &text[at..end];
    if name.is_empty() {
        return at;
    }
    let path = match (name, prefix) {
        ("self", _) => prefix.to_string(),
        (_, "") => name.to_string(),
        _ => format!("{prefix}::{name}"),
    };
    if text[end..].starts_with("::") {
        return use_tree(text, end + 2, &path, paths);
    }
    paths.insert(path);
    let after = skip_space(text, end);
    match text[after..].strip_prefix("as") {
        Some(rest) if rest.starts_with(char::is_whitespace) => {
            ident(text, skip_space(text, after + 2))
        }
        _ => end,
    }
}

/// A cycle in `edges`, as the parts it passes through, if there is one.
fn cycle(edges: &BTreeMap<String, BTreeSet<String>>) -> Option<Vec<String>> {
    fn visit(
        node: &str,
        edges: &BTreeMap<String, BTreeSet<String>>,
        path: &mut Vec<String>,
        done: &mut BTreeSet<String>,
    ) -> Option<Vec<String>> {
        if let Some(at) = path.iter().position(|p| p == node) {
            let mut found = path[at..].to_vec();
            found.push(node.to_string());
            return Some(found);
        }
        if done.contains(node) {
            return None;
        }
        path.push(node.to_string());
        for next in edges.get(node).into_iter().flatten() {
            if let Some(found) = visit(next, edges, path, done) {
                return Some(found);
            }
        }
        path.pop();
        done.insert(node.to_string());
        None
    }
    let mut done = BTreeSet::new();
    edges
        .keys()
        .find_map(|node| visit(node, edges, &mut Vec::new(), &mut done))
}

#[test]
fn reference_rules() {
    let code = "use crate::{Failure, note};\nuse crate::library::{corpus, reading::{self as r, Plan}};\n\
                let x = crate::host::command::find(y);\nuse crate::shared::{\n    store::{self, Env},\n    text,\n};";
    let expected: BTreeSet<String> = [
        "Failure",
        "host::command::find",
        "library::corpus",
        "library::reading",
        "library::reading::Plan",
        "note",
        "shared::store",
        "shared::store::Env",
        "shared::text",
    ]
    .map(String::from)
    .into();
    assert_eq!(crate_paths(code), expected);
    assert!(crate_paths("let s = notcrate::x; my_crate::y").is_empty());
    assert_eq!(
        production(
            "fn a() {} // crate::verbs\n#[cfg(test)]\nfn t() {}\n#[cfg(test)]\nmod tests {\n    crate::x\n}"
        ),
        "fn a() {} \n#[cfg(test)]\nfn t() {}"
    );
    assert!(names("use cliclack::Input;", "cliclack::"));
    assert!(!names("use my_cliclack::Input;", "cliclack::"));
    assert_eq!(module_of("library/cli/"), "library::cli");
    assert_eq!(module_of("search/recall.rs"), "search::recall");
    assert!(within("search::recall::run", "search::recall"));
    assert!(!within("note::newer", "note::new"));
    let mut edges = BTreeMap::new();
    edges.insert("a".to_string(), BTreeSet::from(["b".to_string()]));
    edges.insert("b".to_string(), BTreeSet::from(["c".to_string()]));
    assert_eq!(cycle(&edges), None);
    edges.insert("c".to_string(), BTreeSet::from(["a".to_string()]));
    assert_eq!(
        cycle(&edges),
        Some(["a", "b", "c", "a"].map(String::from).to_vec())
    );
}

#[test]
fn every_file_has_a_place() {
    let files = files();
    for file in &files {
        let placed = file.rel == "main.rs"
            || file.verb().is_some()
            || file.rel.starts_with("shared/")
            || DOMAINS
                .iter()
                .any(|d| file.rel.starts_with(&format!("{d}/")));
        assert!(
            placed,
            "src/{}: neither main.rs, a verb, shared/ nor a domain folder; add the domain to DOMAINS or the verb to \
             VERBS in tests/layout.rs",
            file.rel
        );
    }
    for folder in DOMAINS.iter().chain(&["shared"]) {
        assert!(
            files.iter().any(|f| f.rel == format!("{folder}/mod.rs")),
            "src/{folder}/mod.rs is missing"
        );
    }
    for verb in VERBS {
        assert!(
            files.iter().any(|f| f.verb() == Some(verb)),
            "src/{verb} is listed in VERBS but missing"
        );
    }
}

#[test]
fn modules_with_children_are_mod_rs() {
    let files = files();
    for file in &files {
        let Some(stem) = file.rel.strip_suffix(".rs") else {
            continue;
        };
        if stem == "mod" || stem.ends_with("/mod") {
            continue;
        }
        assert!(
            !files.iter().any(|f| f.rel.starts_with(&format!("{stem}/"))),
            "src/{}: a module with children is `{stem}/mod.rs`",
            file.rel
        );
    }
}

#[test]
fn only_main_prints() {
    for file in files().iter().filter(|f| f.rel != "main.rs") {
        for print in PRINTS {
            assert!(
                !file.code.contains(print),
                "src/{}: `{print}` outside src/main.rs; pass lines back to main",
                file.rel
            );
        }
    }
}

#[test]
fn items_are_pub_or_private() {
    for file in files() {
        for banned in ["pub(crate)", "pub(super)"] {
            assert!(
                !file.code.contains(banned),
                "src/{}: `{banned}`; items are `pub` or private",
                file.rel
            );
        }
        assert!(
            !names(&file.code, "pub use "),
            "src/{}: `pub use`; no re-exports",
            file.rel
        );
        let test_only = TEST_ONLY.contains(&file.rel.as_str());
        assert!(
            test_only || !file.code.lines().any(|l| l.trim_end().ends_with("::*;")),
            "src/{}: a glob import outside test code",
            file.rel
        );
    }
}

#[test]
fn a_domain_root_never_calls_its_own_verb() {
    let files = files();
    for verb in VERBS {
        let Some((domain, leaf)) = verb
            .trim_end_matches('/')
            .trim_end_matches(".rs")
            .split_once('/')
        else {
            continue;
        };
        let root = format!("{domain}/mod.rs");
        let file = files
            .iter()
            .find(|f| f.rel == root)
            .unwrap_or_else(|| panic!("src/{root} is missing"));
        let code: String = file
            .code
            .lines()
            .filter(|l| {
                !l.trim_start().starts_with("pub mod ") && !l.trim_start().starts_with("mod ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !names(&code, &format!("{leaf}::")),
            "src/{root}: calls `{leaf}::`; only main uses a verb's module"
        );
    }
}

/// Files compiled only into the tests, as a `#[cfg(test)] mod` line declares them.
const TEST_ONLY: [&str; 3] = [
    "identity/pair/exchange.rs",
    "setup/driven.rs",
    "setup/fakes.rs",
];

#[test]
fn every_test_only_file_is_declared_under_cfg_test() {
    for rel in TEST_ONLY {
        let (folder, file) = rel.rsplit_once('/').expect("a test-only file has a parent");
        let module = file.trim_end_matches(".rs");
        let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(folder)
            .join("mod.rs");
        let text = std::fs::read_to_string(&parent).expect("the parent's mod.rs");
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        let declared = lines.iter().enumerate().any(|(i, line)| {
            let named = line.strip_prefix("pub ").unwrap_or(line);
            named == format!("mod {module};") && i > 0 && lines[i - 1] == "#[cfg(test)]"
        });
        assert!(
            declared,
            "src/{rel}: no `#[cfg(test)]` above `mod {module};` in {}",
            parent.display()
        );
    }
}

#[test]
fn verbs_are_reached_only_from_main() {
    let files = files();
    assert!(
        files
            .iter()
            .any(|f| f.rel == "note/new.rs" && f.paths().contains("Failure")),
        "the scan no longer sees `Failure` in src/note/new.rs"
    );
    for file in files.iter().filter(|f| f.rel != "main.rs") {
        let paths = file.paths();
        // Test-only files may start a verb, as the unit tests of `setup` start a relay.
        for verb in VERBS
            .into_iter()
            .filter(|v| file.verb() != Some(*v) && !TEST_ONLY.contains(&file.rel.as_str()))
        {
            let module = module_of(verb);
            if let Some(path) = paths.iter().find(|p| within(p, &module)) {
                panic!(
                    "src/{}: uses `{path}`; only main uses a verb's module",
                    file.rel
                );
            }
        }
        if file.verb().is_none() {
            assert!(
                !paths.contains("Failure"),
                "src/{}: only main and the verbs name `Failure`",
                file.rel
            );
        }
        let in_verb_folder =
            file.verb().is_some_and(|v| v.ends_with('/')) && !file.rel.ends_with("mod.rs");
        let reaches_up = if in_verb_folder {
            "super::super"
        } else {
            "super::"
        };
        assert!(
            !file.code.contains(reaches_up),
            "src/{}: `{reaches_up}`; name modules outside a verb's own folder by absolute `crate::` paths",
            file.rel
        );
    }
}

#[test]
fn shared_uses_no_domain_and_serves_two() {
    let files = files();
    let module = |path: &str| path.split("::").nth(1).unwrap_or("").to_string();
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut uses: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in &files {
        let shared = file.paths().into_iter().filter(|p| within(p, "shared"));
        if let Some(own) = file.rel.strip_prefix("shared/") {
            let own = own.trim_end_matches(".rs").to_string();
            for path in file.paths() {
                assert!(
                    within(&path, "shared"),
                    "src/{}: shared/ uses `{path}`; it never imports a domain",
                    file.rel
                );
            }
            uses.entry(own.clone())
                .or_default()
                .extend(shared.map(|p| module(&p)).filter(|m| *m != own));
        } else if DOMAINS.contains(&file.top()) {
            for m in shared {
                users
                    .entry(module(&m))
                    .or_default()
                    .insert(file.top().to_string());
            }
        }
    }
    loop {
        let mut grew = false;
        for (other, used) in &uses {
            let through = users.get(other).cloned().unwrap_or_default();
            for own in used {
                let domains = users.entry(own.clone()).or_default();
                let before = domains.len();
                domains.extend(through.iter().cloned());
                grew |= domains.len() > before;
            }
        }
        if !grew {
            break;
        }
    }
    for file in files
        .iter()
        .filter(|f| f.rel.starts_with("shared/") && !f.rel.ends_with("mod.rs"))
    {
        let own = file.rel["shared/".len()..].trim_end_matches(".rs");
        let domains = users.get(own).cloned().unwrap_or_default();
        assert!(
            domains.len() >= 2,
            "src/{}: used by {domains:?}; shared/ admits a module two domains use, directly or through shared/",
            file.rel
        );
    }
    assert!(
        users.get("store").is_some_and(|d| d.len() >= 3),
        "the scan no longer sees the domains that use shared::store: {users:?}"
    );
}

#[test]
fn domains_form_no_cycle() {
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let files = files();
    for file in files
        .iter()
        .filter(|f| DOMAINS.contains(&f.top()) && f.verb().is_none())
    {
        let from = file.top().to_string();
        for path in file.paths() {
            let to = path.split("::").next().unwrap().to_string();
            assert!(
                to == "shared" || DOMAINS.contains(&to.as_str()),
                "src/{}: domain code uses `{path}`",
                file.rel
            );
            if to != from && to != "shared" {
                edges.entry(from.clone()).or_default().insert(to);
            }
        }
    }
    assert!(
        edges
            .get("search")
            .is_some_and(|to| to.contains("library") && to.contains("note")),
        "the scan no longer sees search's known edges to library and note: {edges:?}"
    );
    if let Some(found) = cycle(&edges) {
        panic!(
            "domains depend on each other in a cycle: {}",
            found.join(" -> ")
        );
    }
}

#[test]
fn each_listed_crate_stays_in_its_files() {
    let files = files();
    for (krate, allowed) in PLACEMENT {
        let word = format!("{krate}::");
        let users: Vec<&str> = files
            .iter()
            .filter(|f| names(&f.code, &word))
            .map(|f| f.rel.as_str())
            .collect();
        for user in &users {
            assert!(
                allowed.contains(user),
                "src/{user}: `{krate}` belongs only in {allowed:?}"
            );
        }
        assert!(
            !users.is_empty(),
            "`{krate}` appears in none of {allowed:?}; update PLACEMENT in tests/layout.rs"
        );
    }
}
