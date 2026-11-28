use std::sync::OnceLock;

use bilbo_layout::{Tables, Tree, bans, complete, words};

const PLACEMENT: &[(&str, &[&str])] = &[
    (
        "serde_json",
        &[
            "src/agy.rs",
            "src/claude.rs",
            "src/codex.rs",
            "src/talk.rs",
            "tests/integration/",
        ],
    ),
    (
        "fs",
        &[
            "src/agy.rs",
            "src/claude.rs",
            "src/codex.rs",
            "src/disk.rs",
            "src/launchd.rs",
            "src/systemd.rs",
            "src/program.rs",
            "tests/",
        ],
    ),
    ("rustix", &["src/program.rs", "tests/integration/"]),
    ("Command", &["src/program.rs", "tests/integration/"]),
    (
        "bilbo_settings",
        &[
            "src/agy.rs",
            "src/claude.rs",
            "src/codex.rs",
            "src/disk.rs",
            "src/keys.rs",
            "tests/",
        ],
    ),
    ("claude", &["src/claude.rs", "tests/"]),
    ("codex", &["src/codex.rs", "tests/"]),
    ("agy", &["src/agy.rs", "tests/"]),
    ("launchd", &["src/launchd.rs", "tests/"]),
    ("systemd", &["src/systemd.rs", "tests/"]),
    ("disk", &["src/disk.rs", "src/launchd.rs", "src/systemd.rs"]),
];
const CORE: &[&str] = &[
    "keys", "plugin", "program", "report", "steps", "talk", "timer",
];
const ADAPTERS: &[&str] = &["agy", "claude", "codex", "disk", "launchd", "systemd"];
const ROOTS: &[&str] = &["crate", "bilbo_setup"];
const SUITES: &[&str] = &[
    "tests/acceptance/main.rs",
    "tests/integration/main.rs",
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
