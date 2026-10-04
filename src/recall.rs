use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::rank::{self, Document, Hit};
use crate::store::Shelf;
use crate::{Failure, config, corpus, embed, note, store, vectors};

const DEFAULT_LIMIT: usize = 10;
const QUERY_BYTES: usize = 2000;
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

struct Request {
    /// The query arguments joined by single spaces.
    query: String,
    /// `rank::words(&query)`, never empty.
    words: Vec<String>,
    /// Empty means every kind.
    kinds: Vec<String>,
    limit: usize,
    /// Search the library instead of the notes: `--library`, or any `--corpus`.
    library: bool,
    /// The corpora `--corpus` named; empty means every corpus.
    corpora: Vec<String>,
}

/// What `run` keeps of a note beside its `Document`, at the same index.
struct Found {
    path: PathBuf,
    kind: String,
    created: Option<String>,
}

pub struct Output {
    /// stderr lines (without "bilbo: "), printed before stdout.
    pub warnings: Vec<String>,
    /// Three lines per hit, an empty line between hits, best hit first.
    pub lines: Vec<String>,
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let request = parse(args)?;
    let settings = config::load(env).map_err(Failure::Config)?;
    let root = store::root(env).map_err(Failure::Config)?;
    if request.library {
        return library(&request, &root);
    }
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let stored = store::read_notes(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;

    let (documents, found): (Vec<Document>, Vec<Found>) = stored
        .into_iter()
        .map(|n| {
            (
                n.document,
                Found {
                    path: n.path,
                    kind: n.kind,
                    created: n.created,
                },
            )
        })
        .unzip();

    let allowed: Vec<bool> = found
        .iter()
        .map(|f| request.kinds.is_empty() || request.kinds.contains(&f.kind))
        .collect();

    let keyword: Vec<Hit> = rank::keyword(&request.words, &documents)
        .into_iter()
        .filter(|hit| allowed[hit.document])
        .collect();
    let (meaning, mut warnings) = match &settings.embedder {
        Some(embedder) => meaning(embedder, env, &root, &request.query, &documents, &allowed),
        None => (Vec::new(), Vec::new()),
    };
    let hits: Vec<Hit> = rank::fuse(&keyword, &meaning)
        .into_iter()
        .take(request.limit)
        .collect();
    if hits.is_empty() {
        warnings.push("no notes match".into());
        return Err(Failure::Refused(warnings.join("\n")));
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
        out.push(rank::snippet(passage));
    }
    Ok(Output {
        warnings,
        lines: out,
    })
}

/// Keyword search over the sources and guides of the library, one block per file.
fn library(request: &Request, root: &Path) -> Result<Output, Failure> {
    let library = store::library_dir(root);
    let present = corpus::corpus_dirs(root)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", library.display())))?;
    if present.is_empty() {
        return Err(Failure::Refused(format!(
            "no library at {}",
            root.display()
        )));
    }
    if let Some(missing) = request
        .corpora
        .iter()
        .find(|c| !present.iter().any(|(name, _)| name == *c))
    {
        return Err(Failure::Refused(format!(
            "no corpus '{missing}' in {}",
            library.display()
        )));
    }
    let mut shelved = store::read_library(root, &request.corpora)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", library.display())))?;
    let documents: Vec<Document> = shelved
        .iter_mut()
        .map(|s| {
            std::mem::replace(
                &mut s.document,
                Document {
                    passages: Vec::new(),
                },
            )
        })
        .collect();
    let hits: Vec<Hit> = rank::fuse(&rank::keyword(&request.words, &documents), &[])
        .into_iter()
        .take(request.limit)
        .collect();
    if hits.is_empty() {
        return Err(Failure::Refused("no sources match".into()));
    }

    let mut out = Vec::new();
    for hit in hits {
        let file = &shelved[hit.document];
        let passage = &documents[hit.document].passages[hit.passage];
        let (start, end) = file.section(passage.line);
        if !out.is_empty() {
            out.push(String::new());
        }
        let shelf = match file.shelf {
            Shelf::Source => "source",
            Shelf::Guide => "guide",
        };
        out.push(format!(
            "{}:{}\t{shelf}\t{}\t{start}-{end}",
            file.path.display(),
            passage.line,
            file.reference
        ));
        out.push(match &passage.path[1..] {
            [] => "-".to_string(),
            below => below.join(" > "),
        });
        out.push(rank::snippet(passage));
    }
    Ok(Output {
        warnings: Vec::new(),
        lines: out,
    })
}

/// The meaning order (at most `rank::CANDIDATES` hits, only documents whose `allowed` is true) and the warnings.
fn meaning(
    embedder: &config::Embedder,
    env: &store::Env,
    root: &Path,
    query: &str,
    documents: &[Document],
    allowed: &[bool],
) -> (Vec<Hit>, Vec<String>) {
    let cache = store::cache_dir(env)
        .map(|dir| vectors::load(&vectors::path(&dir, root)))
        .unwrap_or_default();
    let (found, missing) = vectors::lookup(&cache, &embedder.model, documents);
    let indexed_any = found.iter().flatten().any(Option::is_some);

    let mut warnings = Vec::new();
    let mut hits = Vec::new();
    if indexed_any {
        let mut text = format!("{}{query}", embedder.query_prefix);
        text.truncate(text.floor_char_boundary(QUERY_BYTES));
        match embed::query(embedder, &text, QUERY_TIMEOUT, cache.dims) {
            Ok(q) => {
                hits = rank::meaning(&q, &found, embedder.min_similarity)
                    .into_iter()
                    .map(|(_, hit)| hit)
                    .filter(|hit| allowed[hit.document])
                    .take(rank::CANDIDATES)
                    .collect();
            }
            Err(e) => warnings.push(format!("embedder unavailable ({e}); keyword results only")),
        }
    }
    if missing > 0 {
        warnings.push(format!("{missing} passages not indexed; run bilbo index"));
    }
    (hits, warnings)
}

fn parse(args: &[String]) -> Result<Request, Failure> {
    let mut query: Vec<&str> = Vec::new();
    let mut kinds = Vec::new();
    let mut limit = None;
    let mut library = false;
    let mut corpora: Vec<String> = Vec::new();
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
        } else if arg == "--library" {
            library = true;
        } else if arg.starts_with("--library=") {
            return Err(Failure::Usage("--library takes no value".into()));
        } else if arg == "--corpus" || arg.starts_with("--corpus=") {
            let value = option_value(arg, "--corpus", &mut rest)?;
            if !corpus::is_corpus_name(&value) {
                return Err(Failure::Usage(format!(
                    "invalid corpus '{value}': use segments of a-z and 0-9 joined by single hyphens, not {}",
                    corpus::RESERVED.join(", ")
                )));
            }
            if !corpora.contains(&value) {
                corpora.push(value);
            }
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
    let library = library || !corpora.is_empty();
    if library && !kinds.is_empty() {
        return Err(Failure::Usage(
            "--kind filters notes and cannot be used with --library or --corpus".into(),
        ));
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
        query,
        words,
        kinds,
        limit: limit.unwrap_or(DEFAULT_LIMIT),
        library,
        corpora,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn usage(list: &[&str]) -> String {
        match parse(&args(list)) {
            Err(Failure::Usage(message)) => message,
            Err(_) => panic!("not a usage error"),
            Ok(_) => panic!("{list:?} parsed"),
        }
    }

    fn ok(list: &[&str]) -> Request {
        match parse(&args(list)) {
            Ok(request) => request,
            Err(_) => panic!("{list:?} did not parse"),
        }
    }

    #[test]
    fn plain_recall_searches_the_notes() {
        let request = ok(&["rollback"]);
        assert!(!request.library && request.corpora.is_empty());
    }

    #[test]
    fn library_is_a_flag_anywhere() {
        for list in [
            ["--library", "wrapping", "errors"],
            ["wrapping", "--library", "errors"],
            ["wrapping", "errors", "--library"],
        ] {
            let request = ok(&list);
            assert!(request.library && request.corpora.is_empty());
            assert_eq!(request.query, "wrapping errors");
        }
    }

    #[test]
    fn library_takes_no_value() {
        assert!(usage(&["wrapping", "--library=go"]).contains("--library"));
    }

    #[test]
    fn corpus_implies_library_and_repeats() {
        let request = ok(&["errors", "--corpus", "go", "--corpus=rust"]);
        assert!(request.library);
        assert_eq!(request.corpora, ["go", "rust"]);
        let again = ok(&["errors", "--corpus", "go", "--corpus", "go"]);
        assert_eq!(again.corpora, ["go"]);
    }

    #[test]
    fn corpus_needs_a_value() {
        assert!(usage(&["errors", "--corpus"]).contains("--corpus needs a value"));
        assert!(usage(&["errors", "--corpus="]).contains("--corpus needs a value"));
    }

    #[test]
    fn a_bad_or_reserved_corpus_is_named() {
        assert!(usage(&["errors", "--corpus", "Go"]).contains("'Go'"));
        assert!(usage(&["errors", "--corpus", "plan"]).contains("'plan'"));
        assert!(usage(&["errors", "--corpus", "go--x"]).contains("'go--x'"));
    }

    #[test]
    fn kind_with_the_library_is_refused() {
        for list in [
            &["goroutine", "--library", "--kind", "reference"][..],
            &["goroutine", "--kind", "reference", "--corpus", "go"][..],
        ] {
            let message = usage(list);
            assert!(
                message.contains("--kind") && message.contains("--library"),
                "{message}"
            );
        }
    }

    #[test]
    fn an_unknown_kind_is_still_a_kind_error() {
        assert!(
            usage(&["goroutine", "--library", "--kind", "idea"]).contains("unknown kind 'idea'")
        );
    }

    #[test]
    fn two_dashes_end_the_options_before_library() {
        let request = ok(&["--", "--library", "flag"]);
        assert!(!request.library);
        assert_eq!(request.query, "--library flag");
        let request = ok(&["--library", "--", "--corpus", "go"]);
        assert!(request.library && request.corpora.is_empty());
        assert_eq!(request.query, "--corpus go");
    }

    #[test]
    fn limit_works_with_the_library() {
        let request = ok(&["errors", "--library", "--limit=3"]);
        assert_eq!(request.limit, 3);
    }
}
