use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::{self, Visit};

const COMPONENTS: [&str; 3] = ["config", "help", "remember"];
const MODULE: &str = "console";
const THEME: [&str; 2] = ["console", "theme"];
const ROOTS: [&str; 2] = ["crate", "bilbo_cli"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Tree,
    ModuleNamesComponent,
    ComponentNamedOutside,
    Cycle,
    Escape,
}

#[derive(Default)]
struct Report {
    violations: Vec<(Rule, String)>,
    edges: BTreeMap<String, BTreeSet<String>>,
    theme_escape: bool,
}

impl Report {
    fn of(&self, rule: Rule) -> Vec<&str> {
        self.violations
            .iter()
            .filter(|(r, _)| *r == rule)
            .map(|(_, text)| text.as_str())
            .collect()
    }

    fn add(&mut self, rule: Rule, text: String) {
        self.violations.push((rule, text));
    }
}

struct Decl {
    name: String,
    test: bool,
}

#[derive(Default)]
struct Found {
    paths: Vec<Vec<String>>,
    decls: Vec<Decl>,
    problems: Vec<String>,
    escape: bool,
    mods_only: bool,
}

struct Scan(Found);

fn id(ident: &impl ToString) -> String {
    let text = ident.to_string();
    text.strip_prefix("r#").unwrap_or(&text).to_owned()
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::ForeignMod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::TraitAlias(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

fn lit_has_escape(lit: &syn::Lit) -> bool {
    match lit {
        syn::Lit::Str(s) => s.value().contains('\u{1b}'),
        syn::Lit::ByteStr(b) => b.value().contains(&0x1b),
        syn::Lit::CStr(c) => c.value().as_bytes().contains(&0x1b),
        syn::Lit::Char(c) => c.value() == '\u{1b}',
        syn::Lit::Byte(b) => b.value() == 0x1b,
        _ => false,
    }
}

fn colon2(tokens: &[TokenTree], at: usize) -> bool {
    matches!(
        (tokens.get(at), tokens.get(at + 1)),
        (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
            if a.as_char() == ':' && b.as_char() == ':'
    )
}

fn path_in_tokens(
    tokens: &[TokenTree],
    mut at: usize,
    mut segs: Vec<String>,
    found: &mut Found,
) -> usize {
    if let Some(TokenTree::Ident(ident)) = tokens.get(at) {
        segs.push(id(ident));
        at += 1;
    }
    while colon2(tokens, at) {
        match tokens.get(at + 2) {
            Some(TokenTree::Ident(ident)) => {
                segs.push(id(ident));
                at += 3;
            }
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                for chunk in inner.split(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ','))
                {
                    if chunk.is_empty() {
                        continue;
                    }
                    path_in_tokens(chunk, 0, segs.clone(), found);
                }
                return at + 3;
            }
            _ => break,
        }
    }
    found.paths.push(segs);
    at
}

fn has_ident(stream: TokenStream, name: &str) -> bool {
    stream.into_iter().any(|tree| match tree {
        TokenTree::Ident(ident) => ident == name,
        TokenTree::Group(group) => has_ident(group.stream(), name),
        _ => false,
    })
}

fn scan_tokens(stream: TokenStream, found: &mut Found) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut at = 0;
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Ident(ident)
                if ROOTS.contains(&id(ident).as_str()) || ident == "self" || ident == "super" =>
            {
                at = path_in_tokens(&tokens, at, Vec::new(), found);
            }
            TokenTree::Group(group) => {
                scan_tokens(group.stream(), found);
                at += 1;
            }
            TokenTree::Literal(literal) => {
                if lit_has_escape(&syn::Lit::new(literal.clone())) {
                    found.escape = true;
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
}

fn flatten(tree: &syn::UseTree, prefix: &mut Vec<String>, found: &mut Found) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(id(&p.ident));
            flatten(&p.tree, prefix, found);
            prefix.pop();
        }
        syn::UseTree::Name(n) => record(prefix, &id(&n.ident), found),
        syn::UseTree::Rename(r) => {
            let name = id(&r.ident);
            let root = if prefix.is_empty() {
                Some(name.as_str())
            } else if name == "self" && prefix.len() == 1 {
                Some(prefix[0].as_str())
            } else {
                None
            };
            if root.is_some_and(|root| ROOTS.contains(&root)) {
                found.problems.push(format!(
                    "`use {} as {}` hides every path behind the alias",
                    prefix.first().unwrap_or(&name),
                    r.rename
                ));
            }
            record(prefix, &name, found);
        }
        syn::UseTree::Glob(_) => found.paths.push(prefix.clone()),
        syn::UseTree::Group(g) => {
            for item in &g.items {
                flatten(item, prefix, found);
            }
        }
    }
}

fn record(prefix: &[String], name: &str, found: &mut Found) {
    let mut path = prefix.to_vec();
    if name != "self" {
        path.push(name.to_owned());
    }
    found.paths.push(path);
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if is_cfg_test(item_attrs(item)) {
            if let syn::Item::Mod(m) = item
                && m.content.is_none()
            {
                self.0.decls.push(Decl {
                    name: id(&m.ident),
                    test: true,
                });
            }
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs: &[syn::Attribute] = match item {
            syn::ImplItem::Const(i) => &i.attrs,
            syn::ImplItem::Fn(i) => &i.attrs,
            syn::ImplItem::Type(i) => &i.attrs,
            _ => &[],
        };
        if !is_cfg_test(attrs) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attrs: &[syn::Attribute] = match item {
            syn::TraitItem::Const(i) => &i.attrs,
            syn::TraitItem::Fn(i) => &i.attrs,
            syn::TraitItem::Type(i) => &i.attrs,
            syn::TraitItem::Macro(i) => &i.attrs,
            _ => &[],
        };
        if !is_cfg_test(attrs) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.content.is_some() {
            self.0.problems.push(format!("inline `mod {}`", item.ident));
        } else {
            self.0.decls.push(Decl {
                name: id(&item.ident),
                test: false,
            });
        }
        visit::visit_item_mod(self, item);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.0
            .problems
            .push(format!("`extern crate {}`", id(&item.ident)));
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item.mac.path.is_ident("macro_rules") {
            self.0
                .problems
                .push("a `macro_rules!` definition".to_owned());
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        let in_cfg_attr = attr.path().is_ident("cfg_attr")
            && matches!(&attr.meta, syn::Meta::List(list) if has_ident(list.tokens.clone(), "path"));
        if attr.path().is_ident("path") || in_cfg_attr {
            self.0.problems.push("a `#[path]` attribute".to_owned());
        }
        if let syn::Meta::List(list) = &attr.meta {
            scan_tokens(list.tokens.clone(), &mut self.0);
        }
        visit::visit_attribute(self, attr);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        flatten(&item.tree, &mut Vec::new(), &mut self.0);
        visit::visit_item_use(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.0
            .paths
            .push(path.segments.iter().map(|s| id(&s.ident)).collect());
        visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last()
            && ["include", "include_str", "include_bytes"].contains(&id(&last.ident).as_str())
        {
            self.0.problems.push(format!(
                "`{}!` pulls in a file the scan cannot follow",
                last.ident
            ));
        }
        scan_tokens(mac.tokens.clone(), &mut self.0);
        visit::visit_macro(self, mac);
    }

    fn visit_lit(&mut self, lit: &'ast syn::Lit) {
        if lit_has_escape(lit) {
            self.0.escape = true;
        }
    }
}

fn scan(file: &syn::File) -> Found {
    let mut scan = Scan(Found::default());
    for attr in &file.attrs {
        scan.visit_attribute(attr);
    }
    for item in &file.items {
        scan.visit_item(item);
    }
    scan.0.mods_only = file.items.iter().all(|item| {
        is_cfg_test(item_attrs(item)) || matches!(item, syn::Item::Mod(m) if m.content.is_none())
    });
    scan.0
}

struct Node {
    path: Vec<String>,
    file: String,
    test: bool,
    found: Found,
}

fn child_files(parent: &str, name: &str) -> [String; 2] {
    let dir = parent.rsplit_once('/').map_or("", |(dir, _)| dir);
    let leaf = parent.rsplit('/').next().unwrap_or(parent);
    let base = if leaf == "lib.rs" || leaf == "mod.rs" {
        dir.to_owned()
    } else {
        let stem = leaf.trim_end_matches(".rs");
        if dir.is_empty() {
            stem.to_owned()
        } else {
            format!("{dir}/{stem}")
        }
    };
    let join = |tail: String| {
        if base.is_empty() {
            tail
        } else {
            format!("{base}/{tail}")
        }
    };
    [join(format!("{name}.rs")), join(format!("{name}/mod.rs"))]
}

fn resolve(
    raw: &[String],
    module: &[String],
    modules: &BTreeSet<Vec<String>>,
) -> Option<Vec<String>> {
    let first = raw.first()?;
    match first.as_str() {
        "crate" | "bilbo_cli" => Some(raw[1..].to_vec()),
        "self" => Some([module, &raw[1..]].concat()),
        "super" => {
            let ups = raw.iter().take_while(|s| *s == "super").count();
            let rest = raw[ups..].iter().skip_while(|s| *s == "self").cloned();
            let keep = module.len().checked_sub(ups)?;
            Some(module[..keep].iter().cloned().chain(rest).collect())
        }
        name => {
            let mut child = module.to_vec();
            child.push(name.to_owned());
            modules.contains(&child).then(|| [module, raw].concat())
        }
    }
}

fn theme() -> Vec<String> {
    THEME.map(String::from).to_vec()
}

fn cycle(edges: &BTreeMap<String, BTreeSet<String>>) -> Option<Vec<String>> {
    fn visit(
        node: &str,
        edges: &BTreeMap<String, BTreeSet<String>>,
        path: &mut Vec<String>,
        done: &mut BTreeSet<String>,
    ) -> Option<Vec<String>> {
        if let Some(at) = path.iter().position(|p| p == node) {
            let mut found = path[at..].to_vec();
            found.push(node.to_owned());
            return Some(found);
        }
        if done.contains(node) {
            return None;
        }
        path.push(node.to_owned());
        for next in edges.get(node).into_iter().flatten() {
            if let Some(found) = visit(next, edges, path, done) {
                return Some(found);
            }
        }
        path.pop();
        done.insert(node.to_owned());
        None
    }
    let mut done = BTreeSet::new();
    edges
        .keys()
        .find_map(|node| visit(node, edges, &mut Vec::new(), &mut done))
}

fn analyze(sources: &BTreeMap<String, String>, components: &[&str]) -> Report {
    let mut report = Report::default();
    let parse = |rel: &str, report: &mut Report| -> Option<syn::File> {
        let text = sources.get(rel)?;
        match syn::parse_file(text) {
            Ok(file) => Some(file),
            Err(error) => {
                report.add(Rule::Tree, format!("src/{rel}: does not parse: {error}"));
                None
            }
        }
    };

    let mut nodes: Vec<Node> = Vec::new();
    let mut reached: BTreeSet<String> = BTreeSet::new();
    let mut queue = vec![(Vec::<String>::new(), "lib.rs".to_owned(), false)];
    while let Some((path, file, test)) = queue.pop() {
        if !reached.insert(file.clone()) {
            continue;
        }
        let Some(parsed) = parse(&file, &mut report) else {
            report.add(Rule::Tree, format!("src/{file}: missing"));
            continue;
        };
        let found = scan(&parsed);
        for decl in &found.decls {
            let candidates = child_files(&file, &decl.name);
            let Some(target) = candidates.iter().find(|c| sources.contains_key(*c)) else {
                report.add(
                    Rule::Tree,
                    format!("src/{file}: `mod {}` has no file", decl.name),
                );
                continue;
            };
            let mut child = path.clone();
            child.push(decl.name.clone());
            queue.push((child, target.clone(), test || decl.test));
        }
        nodes.push(Node {
            path,
            file,
            test,
            found,
        });
    }

    let main = parse("main.rs", &mut report).map(|file| scan(&file));
    if main.is_none() {
        report.add(Rule::Tree, "src/main.rs: missing".to_owned());
    }
    reached.insert("main.rs".to_owned());
    for rel in sources.keys().filter(|rel| !reached.contains(*rel)) {
        report.add(
            Rule::Tree,
            format!("src/{rel}: not reached from lib.rs through `mod` items"),
        );
    }

    let production: Vec<&Node> = nodes.iter().filter(|n| !n.test).collect();
    let modules: BTreeSet<Vec<String>> = production.iter().map(|n| n.path.clone()).collect();
    for node in &production {
        if node.path.len() == 1 {
            let name = node.path[0].as_str();
            if name != MODULE && !components.contains(&name) {
                report.add(
                    Rule::Tree,
                    format!(
                        "src/{}: `{name}` is neither a component nor {MODULE}",
                        node.file
                    ),
                );
            }
            if node.file != format!("{name}/mod.rs") {
                report.add(
                    Rule::Tree,
                    format!("src/{}: a top-level folder needs {name}/mod.rs", node.file),
                );
            }
        }
        if node.path.as_slice() == [MODULE] && !node.found.mods_only {
            report.add(
                Rule::Tree,
                format!("src/{}: {MODULE}/mod.rs holds mod lines only", node.file),
            );
        }
    }
    if !modules.contains(&theme()) {
        report.add(
            Rule::Tree,
            format!("src/{}/{}.rs is missing", THEME[0], THEME[1]),
        );
    }
    if let Some(found) = &main
        && !found.decls.is_empty()
    {
        report.add(Rule::Tree, "src/main.rs: declares modules".to_owned());
    }

    let mut scanned: Vec<(String, String, &[String], &Found)> = production
        .iter()
        .map(|n| {
            let top = n.path.first().map_or("lib.rs", String::as_str).to_owned();
            (top, n.file.clone(), n.path.as_slice(), &n.found)
        })
        .collect();
    if let Some(found) = &main {
        scanned.push(("main.rs".to_owned(), "main.rs".to_owned(), &[], found));
    }

    for (top, file, module, found) in &scanned {
        for problem in &found.problems {
            report.add(Rule::Tree, format!("src/{file}: {problem}"));
        }
        if found.escape && *module != theme().as_slice() {
            report.add(
                Rule::Escape,
                format!("src/{file}: an escape character outside src/console/theme.rs"),
            );
        }
        if *module == theme().as_slice() {
            report.theme_escape = found.escape;
        }
        for raw in &found.paths {
            let Some(path) = resolve(raw, module, &modules) else {
                continue;
            };
            let to = match path.first() {
                Some(first) if modules.contains(&vec![first.clone()]) => first.clone(),
                _ => "lib.rs".to_owned(),
            };
            if to == *top {
                continue;
            }
            report
                .edges
                .entry(top.clone())
                .or_default()
                .insert(to.clone());
            let shown = raw.join("::");
            if top == MODULE && components.contains(&to.as_str()) {
                report.add(
                    Rule::ModuleNamesComponent,
                    format!("src/{file}: uses `{shown}`"),
                );
            }
            if top != "lib.rs" && components.contains(&to.as_str()) {
                report.add(
                    Rule::ComponentNamedOutside,
                    format!("src/{file}: uses `{shown}`"),
                );
            }
        }
    }
    if let Some(found) = cycle(&report.edges) {
        report.add(Rule::Cycle, found.join(" -> "));
    }
    report
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel names the scanned tree through LAYOUT_ROOT"
)]
fn src() -> PathBuf {
    match std::env::var("LAYOUT_ROOT") {
        Ok(root) => {
            let runfiles = runfiles::Runfiles::create().expect("runfiles");
            let lib = runfiles
                .rlocation(root)
                .expect("src/lib.rs is in the runfiles");
            lib.parent().expect("src/lib.rs has a parent").to_path_buf()
        }
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
    }
}

fn walk(src: &Path, dir: &Path, found: &mut BTreeMap<String, String>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            walk(src, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let rel = path
                .strip_prefix(src)
                .expect("a file under src/")
                .to_string_lossy()
                .replace('\\', "/");
            found.insert(rel, text);
        }
    }
}

fn real() -> &'static Report {
    static REAL: OnceLock<Report> = OnceLock::new();
    REAL.get_or_init(analyze_src)
}

fn analyze_src() -> Report {
    let src = src();
    let mut sources = BTreeMap::new();
    walk(&src, &src, &mut sources);
    assert!(
        sources.contains_key("lib.rs") && sources.contains_key("main.rs"),
        "the scan did not find src/: {}",
        src.display()
    );
    analyze(&sources, &COMPONENTS)
}

fn assert_clean(report: &Report, rule: Rule) {
    let found = report.of(rule);
    assert!(found.is_empty(), "{rule:?}: {found:#?}");
}

#[test]
fn the_module_tree_holds_main_lib_the_listed_components_and_the_console() {
    assert_clean(real(), Rule::Tree);
}

#[test]
fn the_module_names_no_component() {
    assert_clean(real(), Rule::ModuleNamesComponent);
}

#[test]
fn a_component_is_named_only_by_lib_rs_and_itself() {
    assert_clean(real(), Rule::ComponentNamedOutside);
}

#[test]
fn top_level_entries_form_no_cycle() {
    let report = real();
    let edges = &report.edges;
    assert!(
        edges.get("help").is_some_and(|to| to.contains(MODULE))
            && edges
                .get("lib.rs")
                .is_some_and(|to| to.contains("help") && to.contains(MODULE))
            && edges.get("main.rs").is_some_and(|to| to.contains(MODULE)),
        "the scan no longer sees the known edges: {edges:?}"
    );
    assert_clean(report, Rule::Cycle);
}

#[test]
fn escape_codes_live_only_in_the_theme() {
    let report = real();
    assert_clean(report, Rule::Escape);
    assert!(
        report.theme_escape,
        "the scan no longer sees the escape codes in the theme"
    );
}

fn base() -> BTreeMap<String, String> {
    [
        ("lib.rs", "pub mod console;\npub mod help;\npub fn run() { help::run(); console::view::show(); }\n"),
        ("main.rs", "use bilbo_cli::console::view;\nfn main() { view::show(); bilbo_cli::run(); }\n"),
        ("console/mod.rs", "pub mod theme;\npub mod view;\n"),
        ("console/theme.rs", "pub const BOLD: &str = \"\\x1b[1m\";\n"),
        ("console/view.rs", "pub fn show() {}\n"),
        ("help/mod.rs", "use crate::console::view;\npub fn run() { view::show(); }\n"),
    ]
    .into_iter()
    .map(|(rel, text)| (rel.to_owned(), text.to_owned()))
    .collect()
}

fn with(edits: &[(&str, &str)]) -> Report {
    let mut sources = base();
    for (rel, text) in edits {
        sources.insert((*rel).to_owned(), (*text).to_owned());
    }
    analyze(&sources, &["help", "ingest"])
}

fn fires(report: &Report, rule: Rule, needle: &str) {
    assert!(
        report.of(rule).iter().any(|text| text.contains(needle)),
        "{rule:?} should mention {needle:?}: {:#?}",
        report.violations
    );
}

fn assert_silent(report: &Report) {
    assert!(report.violations.is_empty(), "{:#?}", report.violations);
}

#[test]
fn a_clean_tree_has_no_violation() {
    let report = with(&[]);
    assert_silent(&report);
    assert!(report.theme_escape);
    assert!(report.edges["lib.rs"].contains("help"));
}

#[test]
fn paths_resolve_through_super_self_use_trees_globs_and_relative_names() {
    for (name, source) in [
        ("super::super", "pub fn f() { super::super::help::run(); }"),
        (
            "a use group",
            "pub use crate::{console::view::*, help::run};",
        ),
        ("a glob", "pub use crate::help::*;"),
        ("a rename", "use crate::help::run as go;"),
        ("self in a group", "use crate::help::{self, run};"),
        (
            "a macro",
            "pub fn f() { format!(\"https://x {}\", crate::help::run()); }",
        ),
        ("a macro group", "pub fn f() { wrap!(crate::{help::run}); }"),
        (
            "an attribute argument",
            "#[arg(value_parser = crate::help::run)]\npub fn f() {}",
        ),
        (
            "a nested block",
            "pub fn f() { { use crate::help::run; run(); } }",
        ),
        (
            "an impl",
            "pub struct S;\nimpl S { pub fn f() { crate::help::run(); } }",
        ),
        ("a raw identifier", "pub fn f() { crate::r#help::run(); }"),
    ] {
        let report = with(&[(
            "console/view.rs",
            &format!("pub fn show() {{}}\n{source}\n"),
        )]);
        assert!(
            !report.of(Rule::ModuleNamesComponent).is_empty(),
            "{name}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn a_component_naming_another_component_or_a_non_lib_file_naming_one_is_found() {
    let lib = "pub mod console;\npub mod help;\npub mod ingest;\npub fn run() { help::run(); console::view::show(); }\n";
    let report = with(&[
        ("lib.rs", lib),
        ("ingest/mod.rs", "pub use super::help::run;\n"),
    ]);
    fires(&report, Rule::ComponentNamedOutside, "super::help::run");
    for source in [
        "fn main() { bilbo_cli::help::run(); }\n",
        "fn main() { bilbo_cli::r#help::run(); }\n",
        "fn main() { r#bilbo_cli::help::run(); }\n",
        "use r#bilbo_cli::help::run;\nfn main() { run(); }\n",
        "fn main() { wrap!(bilbo_cli::r#help::run()); }\n",
    ] {
        let report = with(&[("main.rs", source)]);
        fires(&report, Rule::ComponentNamedOutside, "help");
    }
}

#[test]
fn a_cycle_through_lib_rs_is_found() {
    let report = with(&[("console/view.rs", "pub fn show() { crate::run(); }\n")]);
    assert_eq!(report.of(Rule::Cycle).len(), 1, "{:#?}", report.violations);
}

#[test]
fn code_behind_cfg_test_is_skipped_and_all_other_code_is_scanned() {
    let skipped = "#[cfg(test)]\nmod tests {\n    use crate::help::run;\n}\n\
                   #[cfg(test)]\nfn only_in_tests() { crate::help::run(); }\n\
                   pub struct S;\nimpl S {\n    #[cfg(test)]\n    fn t() { crate::help::run(); }\n}\n\
                   pub trait Example {\n    #[cfg(test)]\n    fn example() { let _ = crate::help::run; }\n}\n\
                   pub fn show() {}\n";
    assert_silent(&with(&[("console/view.rs", skipped)]));
    for (source, rule) in [
        (
            "#[cfg(test)]\nmod tests_support {}\npub fn show() { crate::help::run(); }\n",
            Rule::ModuleNamesComponent,
        ),
        (
            "#[cfg(test)]\nmod tests {}\npub use crate::help::run;\n",
            Rule::ModuleNamesComponent,
        ),
        (
            "#[cfg(test)]\nmod tests {}\npub const BAD: &str = \"\\x1b[1m\";\n",
            Rule::Escape,
        ),
        (
            "#[cfg(not(test))]\npub fn show() { crate::help::run(); }\n",
            Rule::ModuleNamesComponent,
        ),
        (
            "pub trait Example {\n    fn example() { let _ = crate::help::run; }\n}\n",
            Rule::ModuleNamesComponent,
        ),
    ] {
        let report = with(&[("console/view.rs", source)]);
        assert!(
            !report.of(rule).is_empty(),
            "{source}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn strings_and_comments_are_data_not_paths() {
    let source = "// crate::help::run\npub const S: &str = \"crate::help::run\";\n\
                  pub const U: &str = \"https://example.test\"; pub fn show() {}\n";
    assert_silent(&with(&[("console/view.rs", source)]));
}

#[test]
fn escape_codes_are_found_in_every_literal_kind_and_in_macros() {
    for literal in [
        "\"\\x1B[1m\"",
        "b\"\\x1b\"",
        "c\"\\x1b\"",
        "'\\u{1b}'",
        "b'\\x1b'",
    ] {
        let source = format!("pub fn show() {{ let _ = {literal}; }}\n");
        fires(
            &with(&[("console/view.rs", &source)]),
            Rule::Escape,
            "console/view.rs",
        );
    }
    let in_macro = "pub fn show() { wrap!(\"\\u{001b}[1m\"); }\n";
    fires(
        &with(&[("console/view.rs", in_macro)]),
        Rule::Escape,
        "console/view.rs",
    );
}

fn appended(edits: &[(&str, &str)], removed: &[&str]) -> Report {
    let mut sources = base();
    for (rel, text) in edits {
        sources.entry((*rel).to_owned()).or_default().push_str(text);
    }
    for rel in removed {
        sources.remove(*rel);
    }
    analyze(&sources, &["help", "ingest"])
}

type Case = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

#[test]
fn modules_are_files_reached_from_lib_rs_without_path_attributes_includes_aliases_or_macros() {
    let cases: [Case; 18] = [
        ("a stray file", &[("stray.rs", "")], "stray.rs: not reached"),
        (
            "a path target",
            &[
                ("console/mod.rs", "#[path = \"../help/x.rs\"]\nmod x;"),
                ("help/x.rs", ""),
            ],
            "#[path]",
        ),
        (
            "a path to the probe",
            &[
                ("lib.rs", "#[path = \"probe.rs\"]\nmod live;"),
                ("probe.rs", ""),
            ],
            "#[path]",
        ),
        (
            "a path inside cfg_attr",
            &[
                (
                    "console/mod.rs",
                    "#[cfg_attr(not(test), path = \"../p.rs\")]\nmod x;",
                ),
                ("console/x.rs", ""),
            ],
            "#[path]",
        ),
        (
            "an include",
            &[("console/view.rs", "include!(\"../probe.rs\");")],
            "include!",
        ),
        (
            "an include of text",
            &[("console/view.rs", "const T: &str = include_str!(\"x\");")],
            "include_str!",
        ),
        (
            "an include of bytes",
            &[("console/view.rs", "const T: &[u8] = include_bytes!(\"x\");")],
            "include_bytes!",
        ),
        (
            "an inline module",
            &[("lib.rs", "pub mod command {}")],
            "inline `mod command`",
        ),
        (
            "a verb module in the console facade",
            &[("console/mod.rs", "pub mod verb {}")],
            "inline `mod verb`",
        ),
        (
            "an item in the console facade",
            &[("console/mod.rs", "pub fn x() {}")],
            "mod lines only",
        ),
        (
            "an unlisted top module",
            &[("lib.rs", "pub mod probe;"), ("probe.rs", "")],
            "neither a component nor console",
        ),
        (
            "a raw module name",
            &[("lib.rs", "pub mod r#probe;"), ("probe.rs", "")],
            "`probe` is neither",
        ),
        (
            "a crate alias",
            &[("console/view.rs", "use crate as root;")],
            "use crate as root",
        ),
        (
            "a grouped crate alias",
            &[("console/view.rs", "use crate::{self as root};")],
            "as root",
        ),
        (
            "a binary alias",
            &[("main.rs", "use bilbo_cli as cli;")],
            "use bilbo_cli as cli",
        ),
        (
            "a grouped binary alias",
            &[("main.rs", "use bilbo_cli::{self as app};")],
            "as app",
        ),
        (
            "an extern crate alias",
            &[("main.rs", "extern crate bilbo_cli as cli;")],
            "extern crate bilbo_cli",
        ),
        (
            "a macro definition",
            &[("lib.rs", "macro_rules! m { () => {} }")],
            "macro_rules!",
        ),
    ];
    for (name, edits, needle) in cases {
        let report = appended(edits, &[]);
        let found = report.of(Rule::Tree);
        assert!(
            found.iter().any(|t| t.contains(needle)),
            "{name}: {found:#?}"
        );
    }
    fires(
        &appended(&[("main.rs", "mod x;"), ("x.rs", "")], &[]),
        Rule::Tree,
        "declares modules",
    );
    let report = appended(&[], &["console/theme.rs"]);
    fires(&report, Rule::Tree, "theme.rs is missing");
}

#[test]
fn a_file_reached_only_through_cfg_test_is_test_only() {
    let lib = "pub mod console;\npub mod help;\n#[cfg(test)]\nmod probe;\npub fn run() { help::run(); console::view::show(); }\n";
    let probe = "use crate::help::run;\npub fn x() { run(); let _ = \"\\x1b\"; }\n";
    assert_silent(&with(&[("lib.rs", lib), ("probe.rs", probe)]));
    let loose = lib.replace("#[cfg(test)]\n", "");
    let report = with(&[("lib.rs", &loose), ("probe.rs", probe)]);
    fires(&report, Rule::Tree, "neither a component nor console");
}
