use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::{self, Visit};

const COMPONENTS: [&str; 7] = [
    "config", "derive", "help", "read", "recall", "reindex", "remember",
];
const MODULE: &str = "console";
const THEME: [&str; 2] = ["console", "theme"];
const ROOTS: [&str; 2] = ["crate", "bilbo_cli"];
const SHARED: [&str; 6] = [
    "bilbo_derive",
    "bilbo_entries",
    "bilbo_llm",
    "bilbo_search",
    "bilbo_settings",
    "bilbo_store",
];
const OWN: &str = "bilbo_";
const ADAPTERS: [(&str, &[&str]); 3] = [
    ("bilbo_llm", &["anthropic", "openai"]),
    ("bilbo_entries", &["markdown", "note", "session"]),
    ("bilbo_search", &["qwen", "tantivy", "whichlang"]),
];
const INDEX_TEST: &str = "reindex.rs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Tree,
    ModuleNamesComponent,
    ComponentNamedOutside,
    Cycle,
    Escape,
    KindNamedOutside,
    AdapterNamedOutside,
}

#[derive(Default)]
struct Report {
    violations: Vec<(Rule, String)>,
    edges: BTreeMap<String, BTreeSet<String>>,
    theme_escape: bool,
    kinds: BTreeSet<String>,
    adapters: BTreeSet<String>,
    tested: BTreeSet<String>,
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
    reexports: Vec<Vec<String>>,
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
        TokenTree::Ident(ident) => id(&ident) == name,
        TokenTree::Group(group) => has_ident(group.stream(), name),
        _ => false,
    })
}

fn has_punct(stream: TokenStream, ch: char) -> bool {
    stream.into_iter().any(|tree| match tree {
        TokenTree::Punct(punct) => punct.as_char() == ch,
        TokenTree::Group(group) => has_punct(group.stream(), ch),
        _ => false,
    })
}

fn scan_tokens(stream: TokenStream, found: &mut Found) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut at = 0;
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Ident(ident)
                if ROOTS.contains(&id(ident).as_str())
                    || id(ident).starts_with(OWN)
                    || ident == "self"
                    || ident == "super" =>
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

fn is_adapter(krate: &str, module: &str) -> bool {
    ADAPTERS
        .iter()
        .any(|(name, modules)| *name == krate && modules.contains(&module))
}

fn bound(krate: &str, module: &str, rest: &[TokenTree]) -> Option<String> {
    let stops = || {
        Some(format!(
            "`{krate}::{module}` stops at the module, which gives it a name of the file's own: go on to an item"
        ))
    };
    let selfed = || {
        Some(format!(
            "`{krate}::{module}::{{self}}` gives the module a name of the file's own: name its items"
        ))
    };
    let globbed = || {
        Some(format!(
            "`{krate}::{module}::*` brings in all the module holds without naming it: name its items"
        ))
    };
    if matches!(rest.first(), Some(TokenTree::Ident(next)) if next == "as") {
        return Some(format!(
            "`{krate}::{module} as …` gives the module a name of the file's own: write the path to its item where it is used"
        ));
    }
    if !colon2(rest, 0) {
        return stops();
    }
    match rest.get(2) {
        Some(TokenTree::Ident(item)) if item == "self" => selfed(),
        Some(TokenTree::Ident(_)) => None,
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
            if has_ident(group.stream(), "self") {
                selfed()
            } else if has_punct(group.stream(), '*') {
                globbed()
            } else {
                None
            }
        }
        Some(TokenTree::Punct(star)) if star.as_char() == '*' => globbed(),
        _ => stops(),
    }
}

fn adapter_element(
    krate: &str,
    modules: &[&str],
    element: &[TokenTree],
    named: &mut BTreeSet<String>,
    hidden: &mut Vec<String>,
) {
    match element {
        [TokenTree::Ident(ident), ..] if modules.contains(&id(ident).as_str()) => {
            named.insert(format!("{krate}::{}", id(ident)));
            hidden.extend(bound(krate, &id(ident), &element[1..]));
        }
        [TokenTree::Ident(ident), TokenTree::Ident(alias), ..]
            if ident == "self" && alias == "as" =>
        {
            hidden.push(format!(
                "`{krate}::{{self as …}}` gives the crate a second name"
            ));
        }
        [TokenTree::Punct(star), ..] if star.as_char() == '*' => {
            hidden.push(format!(
                "`{krate}::*` brings in every module without naming one"
            ));
        }
        [TokenTree::Group(group), ..] => {
            let inner: Vec<TokenTree> = group.stream().into_iter().collect();
            for chunk in inner.split(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ',')) {
                adapter_element(krate, modules, chunk, named, hidden);
            }
        }
        _ => {}
    }
}

fn adapters_in(stream: TokenStream, named: &mut BTreeSet<String>, hidden: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (at, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => adapters_in(group.stream(), named, hidden),
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                let mut next = at + 1;
                if matches!(tokens.get(next), Some(TokenTree::Punct(bang)) if bang.as_char() == '!')
                {
                    next += 1;
                }
                if let Some(TokenTree::Group(group)) = tokens.get(next)
                    && group.delimiter() == Delimiter::Bracket
                {
                    let first = match group.stream().into_iter().next() {
                        Some(TokenTree::Ident(first)) => id(&first),
                        _ => String::new(),
                    };
                    if first == "path" || (first == "cfg_attr" && has_ident(group.stream(), "path"))
                    {
                        hidden.push(
                            "a `#[path]` attribute puts a module where the scan does not look"
                                .to_owned(),
                        );
                    }
                }
            }
            TokenTree::Ident(ident) => {
                let name = id(ident);
                if name == "include"
                    && matches!(tokens.get(at + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!')
                {
                    hidden.push("`include!` pulls in a file the scan cannot follow".to_owned());
                }
                if name == "include"
                    && matches!(tokens.get(at + 1), Some(TokenTree::Ident(next)) if next == "as")
                {
                    hidden.push(
                        "`include as …` gives `include!` a second name, and the scan would not see what it pulls in: write the code out"
                            .to_owned(),
                    );
                }
                if ident == "macro_rules"
                    && matches!(tokens.get(at + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!')
                {
                    hidden.push(
                        "`macro_rules!` can write a pair the scan never reads: write the code out"
                            .to_owned(),
                    );
                }
                let Some((krate, modules)) = ADAPTERS.iter().find(|(krate, _)| *krate == name)
                else {
                    continue;
                };
                if matches!(tokens.get(at + 1), Some(TokenTree::Ident(next)) if next == "as") {
                    hidden.push(format!("`{krate} as …` gives the crate a second name"));
                } else if colon2(&tokens, at + 1) {
                    match tokens.get(at + 3) {
                        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                            let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                            for chunk in inner
                                .split(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ','))
                            {
                                adapter_element(krate, modules, chunk, named, hidden);
                            }
                        }
                        Some(_) => {
                            adapter_element(krate, modules, &tokens[at + 3..], named, hidden);
                        }
                        None => {}
                    }
                }
            }
            _ => {}
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
        let before = self.0.paths.len();
        flatten(&item.tree, &mut Vec::new(), &mut self.0);
        if !matches!(item.vis, syn::Visibility::Inherited) {
            let named = self.0.paths[before..].to_vec();
            self.0.reexports.extend(named);
        }
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
        "self" | "super" => {
            let mut kept = module.to_vec();
            let mut rest = raw;
            while let [head, tail @ ..] = rest {
                match head.as_str() {
                    "self" => {}
                    "super" => {
                        kept.pop()?;
                    }
                    _ => break,
                }
                rest = tail;
            }
            kept.extend(rest.iter().cloned());
            Some(kept)
        }
        name => {
            let mut child = module.to_vec();
            child.push(name.to_owned());
            modules.contains(&child).then(|| [module, raw].concat())
        }
    }
}

fn is_kind_crate(name: &str) -> bool {
    name.starts_with(OWN) && !SHARED.contains(&name) && !ROOTS.contains(&name)
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

fn analyze(
    sources: &BTreeMap<String, String>,
    components: &[&str],
    tests: &BTreeMap<String, String>,
) -> Report {
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
        // A re-export would give the crate a second name, which the check below cannot follow.
        for raw in &found.reexports {
            if raw.first().is_some_and(|first| is_kind_crate(first)) {
                report.add(
                    Rule::KindNamedOutside,
                    format!("src/{file}: re-exports `{}`", raw.join("::")),
                );
            }
        }
        for raw in &found.reexports {
            if raw.windows(2).any(|pair| is_adapter(&pair[0], &pair[1])) {
                report.add(
                    Rule::AdapterNamedOutside,
                    format!("src/{file}: re-exports `{}`", raw.join("::")),
                );
            }
        }
        for raw in &found.paths {
            if let Some(first) = raw.first().filter(|first| is_kind_crate(first)) {
                if file == "lib.rs" {
                    report.kinds.insert(first.clone());
                } else {
                    report.add(
                        Rule::KindNamedOutside,
                        format!("src/{file}: uses `{}`", raw.join("::")),
                    );
                }
            }
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
    let mut adapters = BTreeSet::new();
    let mut tested = BTreeSet::new();
    for (dir, files, home, holds, advice) in [
        (
            "src",
            sources,
            "lib.rs",
            &mut adapters,
            "src/lib.rs alone names an adapter module; take what it builds as an argument"
                .to_owned(),
        ),
        (
            "tests",
            tests,
            INDEX_TEST,
            &mut tested,
            format!(
                "among the tests tests/{INDEX_TEST} alone names an adapter module; go through the binary or a fake"
            ),
        ),
    ] {
        for (rel, text) in files {
            let stream = match text.parse::<TokenStream>() {
                Ok(stream) => stream,
                Err(error) => {
                    report.add(Rule::Tree, format!("{dir}/{rel}: does not lex: {error}"));
                    continue;
                }
            };
            let mut named = BTreeSet::new();
            let mut hidden = Vec::new();
            adapters_in(stream, &mut named, &mut hidden);
            for pair in named {
                if rel == home {
                    holds.insert(pair);
                } else {
                    report.add(
                        Rule::AdapterNamedOutside,
                        format!("{dir}/{rel}: names `{pair}`: {advice}"),
                    );
                }
            }
            for text in hidden {
                report.add(Rule::AdapterNamedOutside, format!("{dir}/{rel}: {text}"));
            }
        }
    }
    report.adapters = adapters;
    report.tested = tested;
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
                .expect("a file under the walked folder")
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
    let tests = src.parent().expect("src/ has a parent").join("tests");
    let mut test_files = BTreeMap::new();
    walk(&tests, &tests, &mut test_files);
    assert!(
        test_files.contains_key("layout.rs") && test_files.contains_key(INDEX_TEST),
        "the scan did not find tests/: {}",
        tests.display()
    );
    analyze(&sources, &COMPONENTS, &test_files)
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
fn a_kind_or_adapter_crate_is_named_only_by_lib_rs() {
    assert_clean(real(), Rule::KindNamedOutside);
}

#[test]
fn an_adapter_module_is_named_only_by_lib_rs_and_the_index_test() {
    let report = real();
    assert_clean(report, Rule::AdapterNamedOutside);
    let missing: Vec<String> = ADAPTERS
        .iter()
        .flat_map(|(krate, modules)| {
            modules
                .iter()
                .map(move |module| format!("{krate}::{module}"))
        })
        .filter(|pair| !report.adapters.contains(pair))
        .collect();
    assert!(
        missing.is_empty(),
        "src/lib.rs no longer names {missing:?}: drop it from ADAPTERS, or the scan stopped seeing it"
    );
    assert!(
        report.tested.contains("bilbo_search::tantivy"),
        "the scan no longer sees tests/reindex.rs name the index"
    );
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
    analyze(&sources, &["help", "ingest"], &BTreeMap::new())
}

fn with_tests(files: &[(&str, &str)]) -> Report {
    let tests: BTreeMap<String, String> = files
        .iter()
        .map(|(rel, text)| ((*rel).to_owned(), (*text).to_owned()))
        .collect();
    analyze(&base(), &["help", "ingest"], &tests)
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
            "self::super::super",
            "pub fn f() { self::super::super::help::run(); }",
        ),
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
fn a_kind_crate_named_outside_lib_rs_is_found() {
    let registry = "pub mod console;\npub mod help;\npub fn kinds() { bilbo_pdf::Pdf; let _: bilbo_epub::Epub; }\npub fn run() { help::run(); console::view::show(); }\n";
    let clean = with(&[("lib.rs", registry)]);
    assert_silent(&clean);
    assert_eq!(
        clean.kinds.iter().map(String::as_str).collect::<Vec<_>>(),
        ["bilbo_epub", "bilbo_pdf"]
    );
    for (rel, text, named) in [
        (
            "help/mod.rs",
            "use crate::console::view;\nuse bilbo_pdf::Pdf;\npub fn run() { view::show(); }\n",
            "bilbo_pdf::Pdf",
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\npub fn run() { view::show(); bilbo_epub::Epub; }\n",
            "bilbo_epub::Epub",
        ),
        (
            "console/view.rs",
            "pub fn show() { let _ = bilbo_djvu::Djvu; }\n",
            "bilbo_djvu::Djvu",
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\npub fn run() { view::show(); vec![Box::new(bilbo_pdf::Pdf)]; }\n",
            "bilbo_pdf::Pdf",
        ),
        (
            "main.rs",
            "use bilbo_cli::console::view;\nfn main() { view::show(); bilbo_cli::run(); bilbo_pdf::Pdf; }\n",
            "bilbo_pdf::Pdf",
        ),
    ] {
        let report = with(&[(rel, text)]);
        fires(&report, Rule::KindNamedOutside, named);
        fires(&report, Rule::KindNamedOutside, rel);
    }
    for reexport in [
        "pub use bilbo_pdf as pdf;",
        "pub use bilbo_pdf::Pdf;",
        "pub(crate) use bilbo_epub::{self, Epub};",
    ] {
        let report = with(&[("lib.rs", &format!("{registry}{reexport}\n"))]);
        fires(&report, Rule::KindNamedOutside, "re-exports `bilbo_");
        fires(&report, Rule::KindNamedOutside, "lib.rs");
    }
    assert_silent(&with(&[(
        "lib.rs",
        &format!("{registry}use bilbo_pdf::Pdf;\npub use bilbo_entries::kind::Kind;\n"),
    )]));
    let shared = "use crate::console::view;\nuse bilbo_entries::kind::Kind;\npub fn run() { view::show(); bilbo_store::id::Id::mint(); bilbo_settings::file::File::default(); }\n";
    assert_silent(&with(&[("help/mod.rs", shared)]));
    let tested = "use crate::console::view;\npub fn run() { view::show(); }\n#[cfg(test)]\nmod tests { fn kind() { bilbo_pdf::Pdf; } }\n";
    assert_silent(&with(&[("help/mod.rs", tested)]));
}

#[test]
fn an_adapter_module_named_outside_lib_rs_is_found() {
    let registry = "pub mod console;\npub mod help;\npub fn kinds() { bilbo_entries::markdown::Markdown; let _: bilbo_entries::session::Session; }\npub fn run() { help::run(); console::view::show(); }\n";
    let clean = with(&[("lib.rs", registry)]);
    assert_silent(&clean);
    assert_eq!(
        clean
            .adapters
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["bilbo_entries::markdown", "bilbo_entries::session"]
    );
    for (rel, text, named) in [
        (
            "help/mod.rs",
            "use crate::console::view;\nuse bilbo_entries::markdown::Markdown;\npub fn run() { view::show(); }\n",
            &["bilbo_entries::markdown"][..],
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\npub fn run() { view::show(); bilbo_entries::session::Session; }\n",
            &["bilbo_entries::session"],
        ),
        (
            "console/view.rs",
            "pub fn show() { let _ = bilbo_search::tantivy::Tantivy::at; }\n",
            &["bilbo_search::tantivy"],
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\npub fn run() { view::show(); vec![Box::new(bilbo_search::qwen::Qwen)]; }\n",
            &["bilbo_search::qwen"],
        ),
        (
            "main.rs",
            "use bilbo_cli::console::view;\nfn main() { view::show(); bilbo_cli::run(); bilbo_search::whichlang::Whichlang; }\n",
            &["bilbo_search::whichlang"],
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\nuse bilbo_search::{index::Index, tantivy::{Committed, Tantivy}};\npub fn run() { view::show(); }\n",
            &["bilbo_search::tantivy"],
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\nuse bilbo_entries::{{session}};\npub fn run() { view::show(); }\n",
            &["bilbo_entries::session"],
        ),
        (
            "console/view.rs",
            "pub fn show() {}\npub fn f() { let _ = self::super::bilbo_search::tantivy::Tantivy; }\n",
            &["bilbo_search::tantivy"],
        ),
        (
            "console/view.rs",
            "pub fn show() {}\nmod inner { pub fn f() { bilbo_search::tantivy::folder; } }\n",
            &["bilbo_search::tantivy"],
        ),
        (
            "console/view.rs",
            "pub fn show() {}\npub fn f() { bilbo_search::r#tantivy::folder; r#bilbo_entries::session::RENDER; }\n",
            &["bilbo_search::tantivy", "bilbo_entries::session"],
        ),
        (
            "console/view.rs",
            "pub fn show() {}\npub fn f() { ::bilbo_search::tantivy::folder; }\n",
            &["bilbo_search::tantivy"],
        ),
        (
            "help/mod.rs",
            "use crate::console::view;\npub fn run() { view::show(); }\n#[cfg(test)]\nmod tests { fn index() { bilbo_search::tantivy::Tantivy; } }\n",
            &["bilbo_search::tantivy"],
        ),
    ] {
        let report = with(&[(rel, text)]);
        for pair in named {
            fires(&report, Rule::AdapterNamedOutside, &format!("`{pair}`"));
        }
        fires(&report, Rule::AdapterNamedOutside, rel);
    }
    for reexport in [
        "pub use bilbo_entries::markdown::Markdown;",
        "pub(crate) use bilbo_entries::session::{self, Session};",
        "pub use bilbo_search::tantivy as index;",
    ] {
        let report = with(&[("lib.rs", &format!("{registry}{reexport}\n"))]);
        fires(&report, Rule::AdapterNamedOutside, "re-exports `bilbo_");
        fires(&report, Rule::AdapterNamedOutside, "lib.rs");
    }
    assert_silent(&with(&[(
        "lib.rs",
        &format!(
            "{registry}use bilbo_entries::markdown::Markdown;\npub use bilbo_entries::kind::Kind;\n"
        ),
    )]));
    let shared = "use crate::console::view;\nuse bilbo_entries::kind::Kind;\npub fn run() { view::show(); bilbo_search::index::Index; bilbo_store::id::Id::mint(); bilbo_settings::file::File::default(); }\n";
    assert_silent(&with(&[("help/mod.rs", shared)]));
    let words = "use crate::console::view;\npub fn run() { view::show(); let session = 1; let markdown = session; fn tantivy() {} }\n";
    assert_silent(&with(&[("help/mod.rs", words)]));
    let variant = "use crate::console::view;\npub fn run() { view::show(); bilbo_entries::read::refs::Target::Session; }\n";
    assert_silent(&with(&[("help/mod.rs", variant)]));
    let data = "use crate::console::view;\n// bilbo_search::tantivy\nconst S: &str = \"bilbo_entries::session\";\npub fn run() { view::show(); }\n";
    assert_silent(&with(&[("help/mod.rs", data)]));
}

#[test]
fn a_test_other_than_the_index_test_naming_an_adapter_module_is_found() {
    let index = with_tests(&[(
        "reindex.rs",
        "use bilbo_search::tantivy::{Committed, Tantivy}; static QWEN: bilbo_search::qwen::Qwen;",
    )]);
    assert_silent(&index);
    assert_eq!(
        index.tested.iter().map(String::as_str).collect::<Vec<_>>(),
        ["bilbo_search::qwen", "bilbo_search::tantivy"]
    );
    for (rel, text) in [
        ("recall.rs", "use bilbo_search::tantivy::Committed;"),
        (
            "sandbox/mod.rs",
            "fn kind() { bilbo_entries::session::Session; }",
        ),
    ] {
        fires(
            &with_tests(&[(rel, text)]),
            Rule::AdapterNamedOutside,
            &format!("tests/{rel}"),
        );
    }
    assert_silent(&with_tests(&[(
        "sessions.rs",
        "fn session() { let session = 1; }",
    )]));
    fires(
        &with_tests(&[("recall.rs", "fn broken() {")]),
        Rule::Tree,
        "tests/recall.rs: does not lex",
    );
}

#[test]
fn an_alias_or_a_glob_of_a_crate_with_adapter_modules_is_found() {
    let everywhere = |source: &str| {
        [
            appended(&[("help/mod.rs", source)], &[]),
            appended(&[("lib.rs", source)], &[]),
            with_tests(&[("recall.rs", source)]),
        ]
    };
    for (source, needle) in [
        ("use bilbo_search as search;", "second name"),
        ("use bilbo_search::{self as search};", "second name"),
        ("use bilbo_entries::*;", "without naming one"),
        ("use bilbo_entries::{read, *};", "without naming one"),
        ("fn f() { wrap!(use bilbo_search as s;); }", "second name"),
    ] {
        for report in everywhere(source) {
            fires(&report, Rule::AdapterNamedOutside, needle);
        }
    }
    for source in [
        "use bilbo_search::recall::keys as recall;",
        "use bilbo_search::index::*;",
        "use bilbo_search::{self, index::Index};",
    ] {
        for report in everywhere(source) {
            assert_silent(&report);
        }
    }
}

#[test]
fn an_adapter_module_given_a_name_is_found_in_every_file() {
    let everywhere = |source: &str| {
        [
            appended(&[("help/mod.rs", source)], &[]),
            appended(&[("lib.rs", source)], &[]),
            with_tests(&[("reindex.rs", source)]),
            with_tests(&[("recall.rs", source)]),
        ]
    };
    for (source, needle) in [
        (
            "use bilbo_search::tantivy as engine;",
            "`bilbo_search::tantivy as",
        ),
        (
            "use bilbo_entries::{session as kind};",
            "`bilbo_entries::session as",
        ),
        (
            "pub use bilbo_search::whichlang as detector;",
            "`bilbo_search::whichlang as",
        ),
        (
            "use bilbo_search::tantivy;",
            "`bilbo_search::tantivy` stops at the module",
        ),
        (
            "use bilbo_search::{index::Index, tantivy};",
            "`bilbo_search::tantivy` stops at the module",
        ),
        (
            "fn f() { let _ = bilbo_search::tantivy; }",
            "`bilbo_search::tantivy` stops at the module",
        ),
        (
            "fn f() { wrap!(bilbo_search::tantivy); }",
            "`bilbo_search::tantivy` stops at the module",
        ),
        (
            "use bilbo_search::tantivy::{self, Tantivy};",
            "`bilbo_search::tantivy::{self}`",
        ),
        (
            "use bilbo_search::tantivy::{self as engine};",
            "`bilbo_search::tantivy::{self}`",
        ),
        (
            "use bilbo_search::{qwen::{Qwen, {self}}};",
            "`bilbo_search::qwen::{self}`",
        ),
        (
            "use bilbo_search::tantivy::self;",
            "`bilbo_search::tantivy::{self}`",
        ),
        (
            "use bilbo_search::tantivy::*;",
            "`bilbo_search::tantivy::*` brings in all the module holds",
        ),
        (
            "use bilbo_search::tantivy::{Tantivy, *};",
            "`bilbo_search::tantivy::*` brings in all the module holds",
        ),
        (
            "use bilbo_entries::{markdown::*};",
            "`bilbo_entries::markdown::*`",
        ),
        (
            "macro_rules! bind { ($k:ident, $m:ident) => { use $k::$m as engine; } } bind!(bilbo_search, tantivy);",
            "`macro_rules!` can write a pair",
        ),
        (
            "#[path = \"../bindings.inc\"] mod bridge;",
            "a `#[path]` attribute puts a module",
        ),
        (
            "#[cfg_attr(test, path = \"../bindings.inc\")] mod bridge;",
            "a `#[path]` attribute puts a module",
        ),
        (
            "#[cfg_attr(all(), r#path = \"engine.rs\")] mod other;",
            "a `#[path]` attribute puts a module",
        ),
        (
            "#[cfg_attr(all(), cfg_attr(all(), r#path = \"engine.rs\"))] mod other;",
            "a `#[path]` attribute puts a module",
        ),
        (
            "include!(\"../bindings.inc\");",
            "`include!` pulls in a file",
        ),
        (
            "fn f() { std::include!(\"../bindings.inc\"); }",
            "`include!` pulls in a file",
        ),
        (
            "r#include!(\"../bindings.inc\");",
            "`include!` pulls in a file",
        ),
        (
            "#[cfg(test)] mod tests { include!(\"../bindings.inc\"); }",
            "`include!` pulls in a file",
        ),
        (
            "use std::include as load; load!(\"../bindings.inc\");",
            "`include as",
        ),
        ("use std::{include as load};", "`include as"),
    ] {
        for report in everywhere(source) {
            fires(&report, Rule::AdapterNamedOutside, needle);
        }
    }

    let report = appended(
        &[
            (
                "lib.rs",
                "use bilbo_search::tantivy as engine;\npub use self::engine as exposed;\n",
            ),
            ("main.rs", "fn open() { bilbo_cli::exposed::folder; }\n"),
        ],
        &[],
    );
    fires(
        &report,
        Rule::AdapterNamedOutside,
        "src/lib.rs: `bilbo_search::tantivy as",
    );
    assert!(
        report
            .of(Rule::AdapterNamedOutside)
            .iter()
            .all(|text| !text.contains("main.rs")),
        "{:#?}",
        report.violations
    );

    let report = appended(
        &[
            (
                "lib.rs",
                "#[cfg(test)]\nuse bilbo_search::tantivy as engine;\n#[cfg(test)]\nmod probe;\n",
            ),
            (
                "probe.rs",
                "pub fn verb() { let _ = crate::engine::folder; }\n",
            ),
        ],
        &[],
    );
    fires(
        &report,
        Rule::AdapterNamedOutside,
        "src/lib.rs: `bilbo_search::tantivy as",
    );
    assert!(
        report.of(Rule::Cycle).is_empty(),
        "{:#?}",
        report.violations
    );

    let report = with_tests(&[
        (
            "reindex.rs",
            "use bilbo_search::tantivy as engine;\nmod index_probe;\n",
        ),
        (
            "index_probe/mod.rs",
            "fn probe() { let _ = super::engine::folder; }\n",
        ),
    ]);
    fires(
        &report,
        Rule::AdapterNamedOutside,
        "tests/reindex.rs: `bilbo_search::tantivy as",
    );
    assert!(
        report
            .of(Rule::AdapterNamedOutside)
            .iter()
            .all(|text| !text.contains("index_probe")),
        "{:#?}",
        report.violations
    );

    let report = with_tests(&[
        (
            "reindex.rs",
            "#[path = \"../bindings.inc\"]\nmod bridge;\nmod index_probe;\n",
        ),
        (
            "index_probe/mod.rs",
            "fn probe() { let _ = super::bridge::engine::folder; }\n",
        ),
    ]);
    fires(
        &report,
        Rule::AdapterNamedOutside,
        "tests/reindex.rs: a `#[path]` attribute",
    );
    assert!(
        report
            .of(Rule::AdapterNamedOutside)
            .iter()
            .all(|text| !text.contains("index_probe")),
        "{:#?}",
        report.violations
    );

    let report = appended(
        &[
            (
                "lib.rs",
                "#[cfg(test)]\ninclude!(\"../bindings.inc\");\n#[cfg(test)]\nmod probe;\n",
            ),
            (
                "probe.rs",
                "pub fn verb() { let _ = crate::engine::folder; }\n",
            ),
        ],
        &[],
    );
    fires(
        &report,
        Rule::AdapterNamedOutside,
        "src/lib.rs: `include!` pulls in a file",
    );
    assert!(
        report
            .of(Rule::Tree)
            .iter()
            .all(|text| !text.contains("include!")),
        "{:#?}",
        report.violations
    );

    let report = appended(
        &[
            (
                "lib.rs",
                "use std::include as load;\nload!(\"../bindings.inc\");\n",
            ),
            ("main.rs", "fn open() { bilbo_cli::engine::folder; }\n"),
        ],
        &[],
    );
    fires(
        &report,
        Rule::AdapterNamedOutside,
        "src/lib.rs: `include as",
    );
    assert!(
        report
            .of(Rule::Tree)
            .iter()
            .all(|text| !text.contains("pulls in a file")),
        "{:#?}",
        report.violations
    );

    for source in [
        "fn open(path: &std::path::Path) -> std::path::PathBuf { let path = path.to_path_buf(); path }",
        "#[derive(Debug)] struct Probe { path: std::path::PathBuf }",
        "fn include(include: bool) -> bool { include }",
    ] {
        for report in everywhere(source) {
            assert_silent(&report);
        }
    }
    let embedded = "const T: &str = include_str!(\"x\"); const B: &[u8] = include_bytes!(\"x\");";
    assert_silent(&with_tests(&[("reindex.rs", embedded)]));
    assert_silent(&with_tests(&[("recall.rs", embedded)]));
    for report in [
        appended(&[("help/mod.rs", embedded)], &[]),
        appended(&[("lib.rs", embedded)], &[]),
    ] {
        assert!(
            report.of(Rule::AdapterNamedOutside).is_empty(),
            "{:#?}",
            report.violations
        );
    }

    for source in [
        "use bilbo_search::tantivy::{Committed, Row, Tantivy};",
        "use bilbo_search::{index::Index, tantivy::Tantivy};",
        "fn at(cache: &std::path::Path) { bilbo_search::tantivy::Tantivy::at(bilbo_search::tantivy::folder(cache), bilbo_search::tantivy::Options { memory: 1, merge_deleted_share: 1 }); }",
        "const ID: &str = bilbo_search::qwen::ID; fn load() { let _ = bilbo_search::qwen::Qwen::load(); }",
        "fn kinds() { bilbo_entries::markdown::Markdown::default(); let _: bilbo_entries::session::Session; }",
        "use bilbo_search::tantivy::Tantivy as Index;",
    ] {
        assert_silent(&appended(&[("lib.rs", source)], &[]));
        assert_silent(&with_tests(&[("reindex.rs", source)]));
    }
}

#[test]
fn a_cycle_through_lib_rs_is_found() {
    let report = with(&[("console/view.rs", "pub fn show() { crate::run(); }\n")]);
    assert_eq!(report.of(Rule::Cycle).len(), 1, "{:#?}", report.violations);
    let report = appended(
        &[
            ("lib.rs", "use bilbo_search::tantivy::folder;\n"),
            ("help/mod.rs", "pub fn open() { self::super::folder; }\n"),
        ],
        &[],
    );
    assert_eq!(report.of(Rule::Cycle).len(), 1, "{:#?}", report.violations);
    fires(&report, Rule::Cycle, "help -> lib.rs");
    assert!(
        report.of(Rule::AdapterNamedOutside).is_empty(),
        "{:#?}",
        report.violations
    );
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
    analyze(&sources, &["help", "ingest"], &BTreeMap::new())
}

type Case = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

#[test]
fn modules_are_files_reached_from_lib_rs_without_path_attributes_includes_aliases_or_macros() {
    let cases: [Case; 19] = [
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
        (
            "a file that does not lex",
            &[("console/view.rs", "pub fn broken() {")],
            "does not lex",
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
