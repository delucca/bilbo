use std::sync::OnceLock;

use bilbo_layout::{Tables, Tree, bans, complete, words};

const PLACEMENT: &[(&str, &[&str])] = &[
    ("xxhash_rust", &["src/instruction.rs"]),
    ("serde_json", &["src/record.rs", "tests/acceptance/"]),
    ("markdown", &[]),
    ("session", &[]),
    ("note", &[]),
    ("bilbo_entries", &["src/", "tests/"]),
    ("bilbo_llm", &["src/run.rs", "tests/acceptance/"]),
    ("bilbo_settings", &["src/", "tests/"]),
    ("bilbo_store", &["src/", "tests/"]),
    ("anthropic", &[]),
    ("openai", &[]),
    ("claude", &[]),
    ("codex", &[]),
];
const CORE: &[&str] = &[
    "answer",
    "check",
    "instruction",
    "keys",
    "log",
    "plan",
    "record",
    "run",
    "scope",
    "spool",
    "tries",
    "view",
    "window",
];
const ADAPTERS: &[&str] = &[];
const ROOTS: &[&str] = &["crate", "bilbo_derive"];
const SUITES: &[&str] = &[
    "tests/acceptance/main.rs",
    "tests/contract/main.rs",
    "tests/layout.rs",
];

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
