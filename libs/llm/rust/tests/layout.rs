use std::sync::OnceLock;

use bilbo_layout::{Tables, Tree, bans, complete, words};

const PLACEMENT: &[(&str, &[&str])] = &[
    (
        "serde_json",
        &[
            "src/table.rs",
            "src/anthropic/api.rs",
            "src/anthropic/tool.rs",
            "src/openai/api.rs",
            "src/openai/tool.rs",
        ],
    ),
    ("rustix", &["src/child.rs"]),
    ("ureq", &["src/http.rs"]),
    ("bilbo_settings", &["src/", "tests/"]),
    (
        "api",
        &[
            "src/api.rs",
            "src/http.rs",
            "src/anthropic/mod.rs",
            "src/anthropic/api.rs",
            "src/openai/mod.rs",
            "src/openai/api.rs",
            "tests/integration/",
        ],
    ),
    (
        "tool",
        &[
            "src/tool.rs",
            "src/anthropic/",
            "src/openai/",
            "tests/integration/",
        ],
    ),
    ("anthropic", &["src/anthropic/", "tests/integration/"]),
    ("openai", &["src/openai/", "tests/integration/"]),
    ("claude", &[]),
    ("codex", &[]),
];
const CORE: &[&str] = &["child", "http", "keys", "secret", "table", "model"];
const ADAPTERS: &[&str] = &["api", "tool", "anthropic", "openai"];
const ROOTS: &[&str] = &["crate", "bilbo_llm"];
const SUITES: &[&str] = &["tests/integration/main.rs", "tests/layout.rs"];

fn real_tables() -> Tables<'static> {
    Tables {
        placement: PLACEMENT,
        core: CORE,
        adapters: ADAPTERS,
        roots: ROOTS,
        suites: SUITES,
    }
}

fn real() -> &'static Tree {
    static REAL: OnceLock<Tree> = OnceLock::new();
    REAL.get_or_init(|| Tree::load(env!("CARGO_MANIFEST_DIR")))
}

fn silent(found: &[String]) {
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_word_stands_only_in_the_files_of_its_row() {
    silent(&words(real(), &real_tables()));
}

#[test]
fn every_dependency_has_a_row_and_every_top_level_module_is_classed() {
    silent(&complete(real(), &real_tables()));
}

#[test]
fn lib_rs_holds_mod_lines_only_and_nothing_hides_a_name() {
    silent(&bans(real(), &real_tables()));
}
