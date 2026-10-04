//! `bilbo library` and `bilbo library <corpus>`: the corpora, and one corpus's guide.

use std::fs;

use super::{Output, io_failure, or_dash, refused, root, sections};
use crate::Failure;
use crate::library::corpus::{self, SourceFile};
use crate::library::source;
use crate::shared::markdown;
use crate::shared::store;

fn plural(n: usize) -> &'static str {
    if n == 1 { "source" } else { "sources" }
}

pub fn run(env: &store::Env) -> Result<Output, Failure> {
    let root = root(env)?;
    let listing = corpus::listing(&root).map_err(io_failure("read", &store::library_dir(&root)))?;
    let lines = listing
        .iter()
        .map(|c| {
            format!(
                "{}\t{} {}\t{} KB\t{} tokens\t{}",
                c.name,
                c.sources,
                plural(c.sources),
                source::kb(c.bytes),
                c.tokens,
                c.title.as_deref().unwrap_or("-")
            )
        })
        .collect();
    Ok(Output::lines(lines))
}

fn facts(file: &SourceFile) -> String {
    let s = &file.source;
    let bytes = file.body().len();
    let sections = sections(file);
    let mut line = format!(
        "`{}.md` · {} · {} KB · {} tokens · fetched {} · {} headings",
        file.name,
        or_dash(&s.id),
        source::kb(bytes),
        markdown::tokens(bytes),
        or_dash(&s.fetched),
        sections.len()
    );
    if source::is_catalog(bytes, &sections) {
        line.push_str(" · catalog");
    }
    if s.keys.iter().any(|k| k == "capture") {
        line.push_str(&format!(" · capture {}", or_dash(&s.capture)));
    }
    line
}

pub fn show_corpus(name: &str, env: &store::Env) -> Result<Output, Failure> {
    let root = root(env)?;
    let library = store::library_dir(&root);
    let dir = library.join(name);
    if !dir.is_dir() {
        return Err(refused(format!(
            "no corpus '{name}' in {}",
            library.display()
        )));
    }
    let sources = corpus::read_sources(&dir).map_err(io_failure("read", &dir))?;
    let guide_path = dir.join("guide.md");
    let mut lines = vec![guide_path.display().to_string()];
    let mut named: Vec<String> = Vec::new();
    let text = fs::read(&guide_path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    if let Some(text) = &text {
        let guide = corpus::read_guide(text);
        for (i, line) in markdown::lines(text)
            .iter()
            .enumerate()
            .skip(guide.body_start - 1)
        {
            lines.push(line.to_string());
            for entry in guide.entries.iter().filter(|e| e.line == i + 1) {
                lines.push(match sources.iter().find(|s| s.name == entry.name) {
                    Some(file) => facts(file),
                    None => format!("`{}.md` · missing", entry.name),
                });
            }
        }
        named = guide.entries.into_iter().map(|e| e.name).collect();
    }
    for file in sources.iter().filter(|s| !named.contains(&s.name)) {
        lines.push(format!("## {}", file.name));
        lines.push(facts(file));
        lines.push("(no entry in guide.md)".into());
    }
    Ok(Output::lines(lines))
}
