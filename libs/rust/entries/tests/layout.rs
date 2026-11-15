use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use proc_macro2::{Delimiter, Group, Ident, TokenStream, TokenTree};
use toml::de::{DeTable, DeValue};

const PLACEMENT: &[(&str, &[&str])] = &[
    ("markdown", &["src/markdown/"]),
    ("session", &["src/session/", "tests/contract/"]),
    ("zstd", &["src/session/"]),
    ("bilbo_settings", &["src/", "tests/"]),
    ("bilbo_store", &["src/", "tests/"]),
];
const CORE: &[&str] = &[
    "fake", "files", "follows", "kind", "read", "reader", "remember", "tokens",
];
const ADAPTERS: &[&str] = &["markdown", "session"];
const ROOTS: &[&str] = &["crate", "bilbo_entries"];
const SUITES: &[&str] = &[
    "tests/acceptance/main.rs",
    "tests/contract/main.rs",
    "tests/layout.rs",
];

const TOY_PLACEMENT: &[(&str, &[&str])] = &[
    ("engine", &["src/engine.rs", "tests/integration/"]),
    ("helper", &["src/core.rs"]),
    ("foreign", &[]),
    ("bilbo_store", &["src/", "tests/"]),
];
const TOY_SUITES: &[&str] = &["tests/integration/main.rs", "tests/layout.rs"];
const TOY_CORE: &[&str] = &["core", "fake"];
const TOY_ADAPTERS: &[&str] = &["engine"];
const TOY_ROOTS: &[&str] = &["crate", "bilbo_toy"];

const TOY_MANIFEST: &str = "[package]
name = \"bilbo-toy\"

[dependencies]
bilbo-store.workspace = true
engine.workspace = true
helper = \"1\"

[dev-dependencies]
serde_json.workspace = true
";

#[derive(Clone, Copy)]
struct Tables<'a> {
    placement: &'a [(&'a str, &'a [&'a str])],
    core: &'a [&'a str],
    adapters: &'a [&'a str],
    roots: &'a [&'a str],
    suites: &'a [&'a str],
}

struct Tree {
    files: BTreeMap<String, String>,
    manifest: String,
}

impl Tree {
    fn file(mut self, path: &str, text: &str) -> Self {
        let held = self.files.entry(path.to_owned()).or_default();
        held.push_str(text);
        held.push('\n');
        self
    }

    fn under(mut self, header: &str, line: &str) -> Self {
        let at = format!("{header}\n");
        assert!(self.manifest.contains(&at), "the toy has no {header}");
        self.manifest = self.manifest.replacen(&at, &format!("{at}{line}\n"), 1);
        self
    }

    fn without(mut self, path: &str) -> Self {
        assert!(self.files.remove(path).is_some(), "the toy has no {path}");
        self
    }

    fn tail(mut self, text: &str) -> Self {
        self.manifest.push_str(text);
        self
    }
}

fn real_tables() -> Tables<'static> {
    Tables {
        placement: PLACEMENT,
        core: CORE,
        adapters: ADAPTERS,
        roots: ROOTS,
        suites: SUITES,
    }
}

fn toy_tables() -> Tables<'static> {
    Tables {
        placement: TOY_PLACEMENT,
        core: TOY_CORE,
        adapters: TOY_ADAPTERS,
        roots: TOY_ROOTS,
        suites: TOY_SUITES,
    }
}

fn toy() -> Tree {
    let files = [
        (
            "src/lib.rs",
            "pub mod core;\npub mod engine;\n#[cfg(test)]\nmod fake;\n",
        ),
        (
            "src/core.rs",
            "pub fn run() { helper::help(); bilbo_store::keep(); }\n",
        ),
        (
            "src/engine.rs",
            "pub struct Engine;\npub fn open() { engine::open(); }\n",
        ),
        ("src/fake.rs", ""),
        (
            "tests/integration/main.rs",
            "use bilbo_toy::engine::Engine;\n",
        ),
        ("tests/layout.rs", ""),
    ];
    Tree {
        files: files
            .into_iter()
            .map(|(path, text)| (path.to_owned(), text.to_owned()))
            .collect(),
        manifest: TOY_MANIFEST.to_owned(),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel names the scanned tree through LAYOUT_ROOT"
)]
fn root() -> PathBuf {
    match std::env::var("LAYOUT_ROOT") {
        Ok(root) => {
            let runfiles = runfiles::Runfiles::create().expect("runfiles");
            let manifest = runfiles
                .rlocation(root)
                .expect("Cargo.toml is in the runfiles");
            manifest
                .parent()
                .expect("Cargo.toml has a parent")
                .to_path_buf()
        }
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    }
}

fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            walk(root, &path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let rel = path
                .strip_prefix(root)
                .expect("a file under the package")
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel, text);
        }
    }
}

fn real() -> &'static Tree {
    static REAL: OnceLock<Tree> = OnceLock::new();
    REAL.get_or_init(|| {
        let root = root();
        let mut files = BTreeMap::new();
        walk(&root, &root.join("src"), &mut files);
        walk(&root, &root.join("tests"), &mut files);
        let manifest = root.join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        assert!(
            files.contains_key("src/lib.rs") && files.contains_key("tests/layout.rs"),
            "the scan found no src/lib.rs or no tests/layout.rs under {}",
            root.display()
        );
        Tree { files, manifest }
    })
}

fn lex(file: &str, text: &str) -> Vec<TokenTree> {
    text.parse::<TokenStream>()
        .unwrap_or_else(|e| panic!("{file} does not lex: {e}"))
        .into_iter()
        .collect()
}

fn children(group: &Group) -> Vec<TokenTree> {
    group.stream().into_iter().collect()
}

fn name(ident: &Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

fn is(token: Option<&TokenTree>, mark: char) -> bool {
    matches!(token, Some(TokenTree::Punct(p)) if p.as_char() == mark)
}

fn word(token: Option<&TokenTree>) -> Option<String> {
    match token {
        Some(TokenTree::Ident(ident)) => Some(name(ident)),
        _ => None,
    }
}

fn is_path_sep(tokens: &[TokenTree]) -> bool {
    is(tokens.first(), ':') && is(tokens.get(1), ':')
}

fn shown(token: Option<&TokenTree>) -> String {
    token.map_or_else(|| "end of file".to_owned(), ToString::to_string)
}

fn covers(place: &str, file: &str) -> bool {
    if place.ends_with('/') {
        file.starts_with(place)
    } else {
        file == place
    }
}

fn idents(tokens: &[TokenTree], found: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(ident) => {
                found.insert(name(ident));
            }
            TokenTree::Group(group) => idents(&children(group), found),
            _ => {}
        }
    }
}

fn holds_ident(tokens: &[TokenTree], wanted: &str) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Ident(ident) => name(ident) == wanted,
        TokenTree::Group(group) => holds_ident(&children(group), wanted),
        _ => false,
    })
}

fn words(tree: &Tree, tables: &Tables) -> Vec<String> {
    let mut named = BTreeMap::new();
    for (file, text) in &tree.files {
        if file != "src/lib.rs" {
            let mut found = BTreeSet::new();
            idents(&lex(file, text), &mut found);
            named.insert(file.as_str(), found);
        }
    }
    let mut found = Vec::new();
    for (row, places) in tables.placement {
        let row = *row;
        let shown_places = if places.is_empty() {
            "no file of this crate".to_owned()
        } else {
            places.join(", ")
        };
        for (file, held) in &named {
            if held.contains(row) && !places.iter().any(|place| covers(place, file)) {
                found.push(format!(
                    "{file}: names `{row}`, which stands only in {shown_places}: go through the port, or rename it if it is not the tool"
                ));
            }
        }
        if places.is_empty() {
            continue;
        }
        for place in *places {
            if !tree.files.keys().any(|file| covers(place, file)) {
                found.push(format!(
                    "PLACEMENT `{row}`: {place} is not in the tree: fix the row"
                ));
            }
        }
        let seen = named
            .iter()
            .any(|(file, held)| held.contains(row) && places.iter().any(|p| covers(p, file)));
        if !seen {
            found.push(format!(
                "PLACEMENT `{row}`: none of its files names it: drop the row, or the scan no longer sees the word"
            ));
        }
    }
    found
}

fn table<'a, 'i>(parent: &'a DeTable<'i>, key: &str) -> Option<&'a DeTable<'i>> {
    parent
        .iter()
        .find(|(name, _)| name.get_ref().as_ref() == key)
        .and_then(|(_, value)| match value.get_ref() {
            DeValue::Table(inner) => Some(inner),
            _ => None,
        })
}

fn dependencies(manifest: &str) -> BTreeSet<String> {
    let parsed =
        DeTable::parse(manifest).unwrap_or_else(|e| panic!("Cargo.toml does not parse: {e}"));
    let root = parsed.get_ref();
    let mut tables = Vec::new();
    tables.extend(table(root, "dependencies"));
    if let Some(targets) = table(root, "target") {
        for (_, target) in targets {
            if let DeValue::Table(target) = target.get_ref() {
                tables.extend(table(target, "dependencies"));
            }
        }
    }
    tables
        .into_iter()
        .flat_map(|deps| deps.iter().map(|(key, _)| key.get_ref().replace('-', "_")))
        .collect()
}

fn module_item(tokens: &[TokenTree], mut at: usize) -> Result<(String, usize), String> {
    while is(tokens.get(at), '#') {
        at += 1;
        if is(tokens.get(at), '!') {
            at += 1;
        }
        match tokens.get(at) {
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket => at += 1,
            other => return Err(shown(other)),
        }
    }
    if word(tokens.get(at)).as_deref() == Some("pub") {
        at += 1;
        if let Some(TokenTree::Group(group)) = tokens.get(at)
            && group.delimiter() == Delimiter::Parenthesis
        {
            at += 1;
        }
    }
    if word(tokens.get(at)).as_deref() != Some("mod") {
        return Err(shown(tokens.get(at)));
    }
    at += 1;
    let Some(declared) = word(tokens.get(at)) else {
        return Err(shown(tokens.get(at)));
    };
    at += 1;
    if !is(tokens.get(at), ';') {
        return Err(shown(tokens.get(at)));
    }
    Ok((declared, at + 1))
}

fn lib_modules(tree: &Tree) -> (Vec<String>, Option<String>) {
    let text = tree
        .files
        .get("src/lib.rs")
        .expect("the tree holds src/lib.rs");
    let tokens = lex("src/lib.rs", text);
    let mut declared = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        match module_item(&tokens, at) {
            Ok((module, next)) => {
                declared.push(module);
                at = next;
            }
            Err(token) => return (declared, Some(token)),
        }
    }
    (declared, None)
}

fn complete(tree: &Tree, tables: &Tables) -> Vec<String> {
    let mut found = Vec::new();
    for dependency in dependencies(&tree.manifest) {
        if !tables.placement.iter().any(|(row, _)| *row == dependency) {
            found.push(format!(
                "Cargo.toml: `{dependency}` has no row in PLACEMENT: add one that lists the files that may name it"
            ));
        }
    }
    let (declared, _) = lib_modules(tree);
    for module in &declared {
        let module = module.as_str();
        if !tables.core.contains(&module) && !tables.adapters.contains(&module) {
            found.push(format!(
                "src/lib.rs: `{module}` is in neither CORE nor ADAPTERS: class it"
            ));
        }
    }
    for (list, listed) in [("CORE", tables.core), ("ADAPTERS", tables.adapters)] {
        for module in listed {
            if !declared.iter().any(|d| d == module) {
                found.push(format!(
                    "{list} `{module}`: src/lib.rs declares no such module: drop it"
                ));
            }
        }
    }
    for module in tables.core {
        if tables.adapters.contains(module) {
            found.push(format!("CORE and ADAPTERS both list `{module}`"));
        }
    }
    for module in tables.adapters {
        let own = [format!("src/{module}.rs"), format!("src/{module}/")];
        let row = tables.placement.iter().find(|(row, _)| row == module);
        if !row.is_some_and(|(_, places)| places.iter().any(|place| own.iter().any(|o| o == place)))
        {
            found.push(format!(
                "ADAPTERS `{module}`: PLACEMENT needs a row `{module}` that lists src/{module}.rs or src/{module}/"
            ));
        }
    }
    let entries: BTreeSet<&str> = tree
        .files
        .keys()
        .filter_map(|file| file.strip_prefix("src/"))
        .filter_map(|rest| rest.split('/').next())
        .map(|entry| entry.strip_suffix(".rs").unwrap_or(entry))
        .filter(|entry| *entry != "lib")
        .collect();
    for entry in entries {
        if !declared.iter().any(|d| d == entry) {
            found.push(format!(
                "src/{entry}: no `mod` line of src/lib.rs declares it"
            ));
        }
    }
    whole(tree, tables, &mut found);
    found
}

fn module_decls(
    tokens: &[TokenTree],
    nested: bool,
    top: &mut Vec<String>,
    inside: &mut Vec<String>,
) {
    for (at, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => module_decls(&children(group), true, top, inside),
            TokenTree::Ident(ident) if name(ident) == "mod" => {
                if let Some(declared) = word(tokens.get(at + 1))
                    && is(tokens.get(at + 2), ';')
                {
                    if nested {
                        inside.push(declared);
                    } else {
                        top.push(declared);
                    }
                }
            }
            _ => {}
        }
    }
}

fn whole(tree: &Tree, tables: &Tables, found: &mut Vec<String>) {
    let mut tops = BTreeMap::new();
    for (file, text) in &tree.files {
        let (mut top, mut inside) = (Vec::new(), Vec::new());
        module_decls(&lex(file, text), false, &mut top, &mut inside);
        for declared in inside {
            found.push(format!(
                "{file}: `mod {declared};` inside a block: declare it at the top of a file, where the scan can find its file"
            ));
        }
        tops.insert(file.as_str(), top);
    }
    let mut pending: Vec<String> = Vec::new();
    if tree.files.contains_key("src/lib.rs") {
        pending.push("src/lib.rs".to_owned());
    }
    for root in tables.suites {
        if tree.files.contains_key(*root) {
            pending.push((*root).to_owned());
        } else {
            found.push(format!(
                "SUITES: {root} is not in the tree: the scan lost it, or drop it from SUITES"
            ));
        }
    }
    let mut reached = BTreeSet::new();
    while let Some(file) = pending.pop() {
        if !reached.insert(file.clone()) {
            continue;
        }
        let folder = if file == "src/lib.rs"
            || tables.suites.contains(&file.as_str())
            || file.ends_with("/mod.rs")
        {
            file.rsplit_once('/').map_or("", |(dir, _)| dir)
        } else {
            file.strip_suffix(".rs").unwrap_or(&file)
        };
        for declared in &tops[file.as_str()] {
            let first = format!("{folder}/{declared}.rs");
            let second = format!("{folder}/{declared}/mod.rs");
            match [&first, &second].into_iter().find(|c| tree.files.contains_key(c.as_str())) {
                Some(path) => pending.push(path.clone()),
                None => found.push(format!(
                    "{file}: `mod {declared};` has no file in the tree: neither {first} nor {second}"
                )),
            }
        }
    }
    for file in tree.files.keys().filter(|file| !reached.contains(*file)) {
        found.push(format!(
            "{file}: no `mod` line reaches it from src/lib.rs or a suite: declare it, or list its root in SUITES"
        ));
    }
}

fn brace_use(file: &str, root: &str, group: &Group, found: &mut Vec<String>) {
    let tokens = children(group);
    for element in tokens.split(|token| is(Some(token), ',')) {
        match element.first() {
            Some(TokenTree::Ident(ident))
                if name(ident) == "self" && word(element.get(1)).as_deref() == Some("as") =>
            {
                found.push(format!(
                    "{file}: `{root} as …` gives the crate root a second name"
                ));
            }
            Some(TokenTree::Punct(p)) if p.as_char() == '*' => found.push(format!(
                "{file}: `{root}::*` brings in every module without naming one"
            )),
            Some(TokenTree::Group(inner)) if inner.delimiter() == Delimiter::Brace => {
                brace_use(file, root, inner, found);
            }
            _ => {}
        }
    }
}

fn root_use(file: &str, root: &str, rest: &[TokenTree], found: &mut Vec<String>) {
    if word(rest.first()).as_deref() == Some("as") {
        found.push(format!(
            "{file}: `{root} as …` gives the crate root a second name"
        ));
    } else if is_path_sep(rest) {
        match rest.get(2) {
            Some(TokenTree::Punct(p)) if p.as_char() == '*' => found.push(format!(
                "{file}: `{root}::*` brings in every module without naming one"
            )),
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                brace_use(file, root, group, found);
            }
            _ => {}
        }
    }
}

fn brace_alias(file: &str, used: &str, group: &Group, found: &mut Vec<String>) {
    let tokens = children(group);
    for element in tokens.split(|token| is(Some(token), ',')) {
        match element.first() {
            Some(TokenTree::Ident(ident))
                if name(ident) == "self" && word(element.get(1)).as_deref() == Some("as") =>
            {
                found.push(second_name(file, used));
            }
            Some(TokenTree::Group(inner)) if inner.delimiter() == Delimiter::Brace => {
                brace_alias(file, used, inner, found);
            }
            _ => {}
        }
    }
}

fn second_name(file: &str, used: &str) -> String {
    format!("{file}: `{used} as …` gives `{used}` a second name: write `{used}` where it is used")
}

fn word_use(file: &str, used: &str, rest: &[TokenTree], found: &mut Vec<String>) {
    if word(rest.first()).as_deref() == Some("as") {
        found.push(second_name(file, used));
    } else if is_path_sep(rest)
        && let Some(TokenTree::Group(group)) = rest.get(2)
        && group.delimiter() == Delimiter::Brace
    {
        brace_alias(file, used, group, found);
    }
}

fn attribute(file: &str, rest: &[TokenTree], found: &mut Vec<String>) {
    let rest = if is(rest.first(), '!') {
        &rest[1..]
    } else {
        rest
    };
    let Some(TokenTree::Group(group)) = rest.first() else {
        return;
    };
    if group.delimiter() != Delimiter::Bracket {
        return;
    }
    let inner = children(group);
    let path = match word(inner.first()).as_deref() {
        Some("path") => true,
        Some("cfg_attr") => holds_ident(&inner, "path"),
        _ => false,
    };
    if path {
        found.push(format!(
            "{file}: `#[path]` puts a module where the scan does not look"
        ));
    }
}

fn scan(file: &str, tokens: &[TokenTree], tables: &Tables, found: &mut Vec<String>) {
    for (at, token) in tokens.iter().enumerate() {
        let rest = &tokens[at + 1..];
        match token {
            TokenTree::Group(group) => scan(file, &children(group), tables, found),
            TokenTree::Punct(p) if p.as_char() == '#' => attribute(file, rest, found),
            TokenTree::Ident(ident) => {
                let ident = name(ident);
                if ident == "macro_rules" && is(rest.first(), '!') {
                    found.push(format!(
                        "{file}: `macro_rules!` can write a name the scan never reads: write the code out"
                    ));
                }
                if ident == "include" && is(rest.first(), '!') {
                    found.push(format!(
                        "{file}: `include!` pulls in a file the scan does not read"
                    ));
                }
                if tables.roots.contains(&ident.as_str()) {
                    root_use(file, &ident, rest, found);
                }
                if tables.placement.iter().any(|(row, _)| *row == ident) {
                    word_use(file, &ident, rest, found);
                }
            }
            _ => {}
        }
    }
}

fn bans(tree: &Tree, tables: &Tables) -> Vec<String> {
    let mut found = Vec::new();
    if let (_, Some(token)) = lib_modules(tree) {
        found.push(format!(
            "src/lib.rs: holds `mod` lines only: found `{token}`"
        ));
    }
    for (file, text) in &tree.files {
        scan(file, &lex(file, text), tables, &mut found);
    }
    found
}

fn fires(found: &[String], needle: &str) {
    assert!(
        found.iter().any(|text| text.contains(needle)),
        "should mention {needle:?}: {found:#?}"
    );
}

fn silent(found: &[String]) {
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_word_stands_only_in_the_files_of_its_row() {
    let found = words(real(), &real_tables());
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn every_dependency_has_a_row_and_every_top_level_module_is_classed() {
    let found = complete(real(), &real_tables());
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn lib_rs_holds_mod_lines_only_and_nothing_hides_a_name() {
    let found = bans(real(), &real_tables());
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_word_outside_its_row_is_found_in_every_spelling_and_text_is_no_word() {
    let tables = toy_tables();
    silent(&words(&toy(), &tables));
    let leak = "names `engine`";
    let cases = [
        (
            "src/core.rs",
            "pub fn leak() -> engine::Handle { todo!() }",
            leak,
        ),
        ("src/core.rs", "use crate::engine::Engine;", leak),
        (
            "src/core.rs",
            "fn f(_: self::super::engine::Engine) {}",
            leak,
        ),
        (
            "src/core.rs",
            "use crate::{core::run, engine::{self, Engine}};",
            leak,
        ),
        (
            "src/core.rs",
            "mod inner { fn f() { super::super::engine::open(); } }",
            leak,
        ),
        ("src/core.rs", "fn g() { wrap!(engine::open()); }", leak),
        ("src/core.rs", "fn h() { crate::r#engine::open(); }", leak),
        (
            "src/core.rs",
            "#[cfg(test)] mod tests { fn t() { crate::engine::open(); } }",
            leak,
        ),
        ("src/core.rs", "#[derive(engine::Probe)] struct P;", leak),
        (
            "tests/acceptance/main.rs",
            "use bilbo_toy::engine::Engine;",
            "tests/acceptance/main.rs: names `engine`",
        ),
        (
            "src/engine.rs",
            "fn x() { helper::help(); }",
            "src/engine.rs: names `helper`",
        ),
        (
            "src/core.rs",
            "use bilbo_other::foreign::Thing;",
            "names `foreign`, which stands only in no file of this crate",
        ),
    ];
    for (path, text, needle) in cases {
        fires(&words(&toy().file(path, text), &tables), needle);
    }
    let text = "// engine::open\nconst DOC: &str = \"engine helper foreign\";\nfn engines() {}\nstruct Engine;";
    silent(&words(&toy().file("src/core.rs", text), &tables));
}

#[test]
fn a_dependency_without_a_row_and_a_module_left_unclassed_are_found() {
    let tables = toy_tables();
    silent(&complete(&toy(), &tables));
    let rogue = "`rogue` has no row";
    let cases = [
        (toy().under("[dependencies]", "rogue = \"1\""), rogue),
        (
            toy().under(
                "[dependencies]",
                "rogue-tool = { package = \"other\", version = \"1\" }",
            ),
            "`rogue_tool` has no row",
        ),
        (
            toy().under("[dependencies]", "rogue.workspace = true"),
            rogue,
        ),
        (
            toy().tail("\n[dependencies.rogue]\nversion = \"1\"\n"),
            rogue,
        ),
        (
            toy().tail("\n[target.'cfg(unix)'.dependencies]\nrogue = \"1\"\n"),
            rogue,
        ),
        (
            toy()
                .file("src/lib.rs", "pub mod probe;")
                .file("src/probe.rs", ""),
            "`probe` is in neither",
        ),
        (
            toy()
                .file("src/lib.rs", "pub mod r#probe;")
                .file("src/probe.rs", ""),
            "`probe` is in neither",
        ),
        (toy().file("src/stray.rs", ""), "src/stray: no `mod` line"),
        (
            toy().file("src/stray/mod.rs", ""),
            "src/stray: no `mod` line",
        ),
    ];
    for (tree, needle) in &cases {
        fires(&complete(tree, &tables), needle);
    }
    let quiet = toy().under("[dev-dependencies]", "rogue = \"1\"");
    silent(&complete(&quiet, &tables));
    let cases = [
        (
            toy().under("[dependencies]", "bilbo-settings.workspace = true"),
            "`bilbo_settings` has no row",
        ),
        (
            toy().under(
                "[dependencies]",
                "bilbo_math = { package = \"strsim\", version = \"0.11.1\" }",
            ),
            "`bilbo_math` has no row",
        ),
        (
            toy().without("src/core.rs"),
            "src/lib.rs: `mod core;` has no file in the tree",
        ),
        (
            toy().file("src/lib.rs", "pub mod lost;"),
            "src/lib.rs: `mod lost;` has no file in the tree: neither src/lost.rs nor src/lost/mod.rs",
        ),
        (
            toy().file("tests/integration/main.rs", "mod fakes;"),
            "tests/integration/main.rs: `mod fakes;` has no file in the tree: neither tests/integration/fakes.rs nor tests/integration/fakes/mod.rs",
        ),
        (
            toy().without("tests/integration/main.rs"),
            "SUITES: tests/integration/main.rs is not in the tree",
        ),
        (
            toy().file("tests/acceptance/main.rs", ""),
            "tests/acceptance/main.rs: no `mod` line reaches it",
        ),
        (
            toy().file("tests/integration/stray.rs", ""),
            "tests/integration/stray.rs: no `mod` line reaches it",
        ),
        (
            toy().file("src/core.rs", "mod inner { mod part; }"),
            "src/core.rs: `mod part;` inside a block",
        ),
    ];
    for (tree, needle) in &cases {
        fires(&complete(tree, &tables), needle);
    }
    let deep = ["core", "fake", "deep"];
    let whole = toy()
        .file("tests/integration/main.rs", "mod fakes;")
        .file("tests/integration/fakes.rs", "")
        .file("src/lib.rs", "pub mod deep;")
        .file("src/deep/mod.rs", "mod inner;")
        .file("src/deep/inner.rs", "")
        .file("src/engine.rs", "mod part;")
        .file("src/engine/part.rs", "");
    silent(&complete(
        &whole,
        &Tables {
            core: &deep,
            ..tables
        },
    ));

    let ghost = ["engine", "ghost"];
    fires(
        &complete(
            &toy(),
            &Tables {
                adapters: &ghost,
                ..tables
            },
        ),
        "ADAPTERS `ghost`: src/lib.rs declares no such module",
    );
    let twice = ["core", "fake", "engine"];
    fires(
        &complete(
            &toy(),
            &Tables {
                core: &twice,
                ..tables
            },
        ),
        "CORE and ADAPTERS both list `engine`",
    );
    let rows = [
        ("helper", &["src/core.rs"] as &[&str]),
        ("foreign", &[]),
        ("bilbo_store", &["src/", "tests/"]),
    ];
    let found = complete(
        &toy(),
        &Tables {
            placement: &rows,
            ..tables
        },
    );
    fires(&found, "`engine` has no row");
    fires(&found, "ADAPTERS `engine`: PLACEMENT needs a row");
    let rows = [
        ("engine", &["tests/integration/"] as &[&str]),
        ("helper", &["src/core.rs"]),
        ("foreign", &[]),
        ("bilbo_store", &["src/", "tests/"]),
    ];
    fires(
        &complete(
            &toy(),
            &Tables {
                placement: &rows,
                ..tables
            },
        ),
        "ADAPTERS `engine`: PLACEMENT needs a row",
    );
}

#[test]
fn an_item_in_lib_rs_a_macro_a_path_attribute_an_include_an_alias_and_a_glob_are_found() {
    let tables = toy_tables();
    silent(&bans(&toy(), &tables));
    let cases = [
        (
            "src/lib.rs",
            "pub use engine::Engine;",
            "holds `mod` lines only",
        ),
        ("src/lib.rs", "pub fn f() {}", "holds `mod` lines only"),
        ("src/lib.rs", "pub mod inline {}", "holds `mod` lines only"),
        ("src/core.rs", "macro_rules! m { () => {} }", "macro_rules!"),
        (
            "src/core.rs",
            "#[path = \"engine.rs\"] mod other;",
            "#[path]",
        ),
        (
            "src/core.rs",
            "#[cfg_attr(test, path = \"engine.rs\")] mod other;",
            "#[path]",
        ),
        ("src/core.rs", "include!(\"engine.rs\");", "include!"),
        ("src/core.rs", "use crate as root;", "crate as"),
        ("src/core.rs", "use crate::{self as root};", "crate as"),
        (
            "tests/integration/main.rs",
            "use bilbo_toy as toy;",
            "bilbo_toy as",
        ),
        (
            "tests/integration/main.rs",
            "extern crate bilbo_toy as toy;",
            "bilbo_toy as",
        ),
        ("src/core.rs", "use crate::*;", "crate::*"),
        ("src/core.rs", "use crate::{core::run, *};", "crate::*"),
        (
            "tests/integration/main.rs",
            "use bilbo_toy::*;",
            "bilbo_toy::*",
        ),
        (
            "src/engine.rs",
            "pub use engine as motor;",
            "src/engine.rs: `engine as",
        ),
        (
            "src/engine.rs",
            "use engine::{self as motor};",
            "gives `engine` a second name",
        ),
        (
            "src/engine.rs",
            "use engine::{{self as motor}, Handle};",
            "gives `engine` a second name",
        ),
        (
            "src/engine.rs",
            "extern crate engine as motor;",
            "gives `engine` a second name",
        ),
        (
            "src/core.rs",
            "use crate::engine as motor;",
            "gives `engine` a second name",
        ),
        (
            "tests/integration/main.rs",
            "use bilbo_toy::{engine as motor};",
            "tests/integration/main.rs: `engine as",
        ),
        (
            "tests/integration/main.rs",
            "use bilbo_toy::engine as e;",
            "tests/integration/main.rs: `engine as",
        ),
        (
            "src/core.rs",
            "use helper as aid;",
            "gives `helper` a second name",
        ),
        (
            "src/core.rs",
            "use bilbo_store as stock;",
            "gives `bilbo_store` a second name",
        ),
    ];
    for (path, text, needle) in cases {
        fires(&bans(&toy().file(path, text), &tables), needle);
    }
    let passes = toy()
        .file(
            "src/core.rs",
            "const T: &str = include_str!(\"x\"); const B: &[u8] = include_bytes!(\"x\"); use std::error::Error as StdError; use crate::core::*; use helper::help as aid; mod tests { use super::*; }",
        )
        .file(
            "tests/integration/main.rs",
            "use bilbo_toy::engine::Engine as E;",
        );
    silent(&bans(&passes, &tables));
}

#[test]
fn a_row_that_lists_a_missing_place_or_sees_nothing_is_found() {
    let gone = [
        ("engine", &["src/engine.rs", "tests/gone/"] as &[&str]),
        ("helper", &["src/core.rs"]),
        ("foreign", &[]),
        ("bilbo_store", &["src/", "tests/"]),
    ];
    let tables = Tables {
        placement: &gone,
        ..toy_tables()
    };
    fires(&words(&toy(), &tables), "tests/gone/ is not in the tree");
    let blind = [
        (
            "engine",
            &["src/engine.rs", "tests/integration/"] as &[&str],
        ),
        ("helper", &["src/fake.rs"]),
        ("foreign", &[]),
        ("bilbo_store", &["src/", "tests/"]),
    ];
    let tables = Tables {
        placement: &blind,
        ..toy_tables()
    };
    fires(
        &words(&toy(), &tables),
        "PLACEMENT `helper`: none of its files names it",
    );
}

#[test]
#[should_panic(expected = "cannot read")]
fn a_missing_folder_stops_the_scan() {
    let root = root();
    walk(&root, &root.join("no-such-folder"), &mut BTreeMap::new());
}

#[test]
#[should_panic(expected = "src/core.rs does not lex")]
fn a_file_that_does_not_lex_stops_the_scan() {
    words(&toy().file("src/core.rs", "fn broken() {"), &toy_tables());
}
