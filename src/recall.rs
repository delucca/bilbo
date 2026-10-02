use std::path::PathBuf;

use crate::rank::{self, Document};
use crate::store::{self, EntryKind};
use crate::{Failure, note};

const DEFAULT_LIMIT: usize = 10;
const SNIPPET_CHARS: usize = 300;

struct Request {
    /// `rank::words(&query)`, never empty.
    words: Vec<String>,
    /// Empty means every kind.
    kinds: Vec<String>,
    limit: usize,
}

/// What `run` keeps of a note beside its `Document`, at the same index.
struct Found {
    path: PathBuf,
    kind: String,
    created: Option<String>,
}

/// Output lines: three per hit, an empty line between hits, best hit first.
pub fn run(args: &[String], env: &store::Env) -> Result<Vec<String>, Failure> {
    let request = parse(args)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let entries = store::entries(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;

    let mut found: Vec<Found> = Vec::new();
    let mut documents = Vec::new();
    for entry in &entries {
        if !entry.utf8 || entry.kind != EntryKind::File {
            continue;
        }
        let Ok(name) = note::parse_name(&entry.name) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&entry.path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let lines = note::lines(&text);
        let read = note::read(&text);
        let stem = entry.name.strip_suffix(".md").unwrap_or(&entry.name);
        documents.push(Document {
            passages: rank::passages(&lines[read.body_start - 1..], read.body_start, stem),
        });
        found.push(Found {
            path: entry.path.clone(),
            kind: name.kind,
            created: read.created,
        });
    }

    let hits: Vec<_> = rank::rank(&request.words, &documents)
        .into_iter()
        .filter(|hit| request.kinds.is_empty() || request.kinds.contains(&found[hit.document].kind))
        .take(request.limit)
        .collect();
    if hits.is_empty() {
        return Err(Failure::Refused("no notes match".into()));
    }

    let mut out = Vec::new();
    for hit in hits {
        let Found {
            path,
            kind,
            created,
        } = &found[hit.document];
        let passage = &documents[hit.document].passages[hit.passage];
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(format!(
            "{}:{}\t{kind}\t{}",
            path.display(),
            passage.line,
            created.as_deref().unwrap_or("-")
        ));
        out.push(passage.path.join(" > "));
        let collapsed = passage
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let snippet: String = collapsed.chars().take(SNIPPET_CHARS).collect();
        out.push(if snippet.is_empty() {
            "-".into()
        } else {
            snippet
        });
    }
    Ok(out)
}

fn parse(args: &[String]) -> Result<Request, Failure> {
    let mut query: Vec<&str> = Vec::new();
    let mut kinds = Vec::new();
    let mut limit = None;
    let mut options_ended = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if options_ended {
            query.push(arg);
        } else if arg == "--" {
            options_ended = true;
        } else if arg == "--kind" || arg.starts_with("--kind=") {
            let value = option_value(arg, "--kind", &mut rest)?;
            if !note::KINDS.contains(&value.as_str()) {
                return Err(Failure::Usage(format!(
                    "unknown kind '{value}'; kinds: {}",
                    note::kinds_list()
                )));
            }
            kinds.push(value);
        } else if arg == "--limit" || arg.starts_with("--limit=") {
            if limit.is_some() {
                return Err(Failure::Usage("--limit given more than once".into()));
            }
            let value = option_value(arg, "--limit", &mut rest)?;
            limit = Some(parse_limit(&value)?);
        } else if arg.starts_with('-') && arg.chars().nth(1).is_some_and(|c| !c.is_whitespace()) {
            return Err(Failure::Usage(format!("unknown option '{arg}'")));
        } else {
            query.push(arg);
        }
    }
    if query.is_empty() {
        return Err(Failure::Usage("missing <query>".into()));
    }
    let query = query.join(" ");
    let words = rank::words(&query);
    if words.is_empty() {
        return Err(Failure::Usage(format!(
            "query '{query}' has no words of 2 or more letters or digits"
        )));
    }
    Ok(Request {
        words,
        kinds,
        limit: limit.unwrap_or(DEFAULT_LIMIT),
    })
}

/// The value of `name` given as `name=<value>` or as the next argument; it must not be empty.
fn option_value<'a>(
    arg: &str,
    name: &str,
    rest: &mut impl Iterator<Item = &'a String>,
) -> Result<String, Failure> {
    let value = match arg.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
        Some(value) => Some(value.to_string()),
        None => rest.next().cloned(),
    };
    value
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Failure::Usage(format!("{name} needs a value")))
}

fn parse_limit(value: &str) -> Result<usize, Failure> {
    let digits = value.bytes().all(|b| b.is_ascii_digit());
    match value.parse::<usize>() {
        Ok(0) => {}
        Ok(n) if digits => return Ok(n),
        Err(_) if digits => return Ok(usize::MAX),
        _ => {}
    }
    Err(Failure::Usage(format!(
        "--limit must be a whole number of 1 or more, got '{value}'"
    )))
}
