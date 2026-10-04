//! `bilbo library show`.

use std::fs;
use std::path::{Path, PathBuf};

use super::reference::{find, parse_reference, section_for, split_anchor};
use super::{Args, Output, or_dash, root, sections, usage};
use crate::Failure;
use crate::library::source;
use crate::shared::markdown::{self, Section};
use crate::shared::store::{self, EntryKind};

/// The capture folder whose `landed` file records this id with this digest.
fn capture_folder(root: &Path, id: &str, digest: &str) -> Option<PathBuf> {
    store::entries(&store::captures_dir(root))
        .ok()?
        .into_iter()
        .filter(|e| e.kind == EntryKind::Folder)
        .find(|e| {
            fs::read_to_string(e.path.join("landed")).is_ok_and(|landed| {
                landed.lines().any(|line| {
                    let mut parts = line.split('\t');
                    parts.next() == Some(id) && parts.next() == Some(digest)
                })
            })
        })
        .map(|e| e.path)
}

pub fn run(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let [reference] = args.operands(["<reference>"])?;
    let depth = args
        .one("--depth")
        .map(|value| {
            value
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| value.parse::<usize>().ok().filter(|n| *n >= 1))
                .flatten()
                .ok_or_else(|| {
                    usage(format!(
                        "--depth '{value}' is not a whole number of 1 or more"
                    ))
                })
        })
        .transpose()?;
    let (reference, anchor) = split_anchor(reference)?;
    let parsed = parse_reference(reference)?;
    let root = root(env)?;
    let file = find(&root, &parsed)?;

    let s = &file.source;
    let lines = markdown::lines(&file.text);
    let sections = sections(&file);
    let bytes = file.body().len();
    let mut shown: Vec<&Section> = sections.iter().collect();
    if let Some(anchor) = anchor {
        let i = section_for(&sections, reference, anchor)?;
        let level = sections[i].level;
        let inside = sections[i + 1..]
            .iter()
            .take_while(|s| s.level > level)
            .count();
        shown = sections[i..=i + inside].iter().collect();
    }
    if let Some(depth) = depth {
        shown.retain(|section| section.path.len() <= depth);
    }

    let title = lines
        .get(s.body_start - 1)
        .and_then(|line| line.strip_prefix("# "))
        .unwrap_or("-");
    let mut out = vec![
        format!("path: {}", file.path.display()),
        format!("id: {}", or_dash(&s.id)),
        format!("title: {title}"),
        format!("origin: {}", or_dash(&s.origin)),
        format!("fetched: {}", or_dash(&s.fetched)),
    ];
    for (key, value) in [("kept", &s.kept), ("capture", &s.capture)] {
        if s.keys.iter().any(|k| k == key) {
            out.push(format!("{key}: {}", or_dash(value)));
        }
    }
    out.push(format!(
        "lines: {}-{}",
        s.body_start,
        lines.len().max(s.body_start)
    ));
    out.push(format!("tokens: {}", markdown::tokens(bytes)));
    out.push(format!("headings: {}", sections.len()));
    let catalog = source::is_catalog(bytes, &sections);
    out.push(format!("catalog: {}", if catalog { "yes" } else { "no" }));
    if let (Some(id), Some(digest)) = (&s.id, &s.digest)
        && let Some(folder) = capture_folder(&root, id, digest)
    {
        out.push(format!("capture folder: {}", folder.display()));
    }
    out.push(String::new());
    out.extend(shown.iter().map(|section| {
        format!(
            "{}-{}\t{} tokens\t{}",
            section.start,
            section.end,
            section.tokens,
            section.path_text()
        )
    }));
    Ok(Output::lines(out))
}
