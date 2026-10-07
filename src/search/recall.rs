use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::host::{prompt, terminal};
use crate::library::corpus;
use crate::search::documents::{self, Shelf};
use crate::search::rank::{self, Document, Hit};
use crate::search::{embed, vectors};
use crate::shared::{config, markdown, store, text};
use crate::{Failure, note};

const DEFAULT_LIMIT: usize = 10;
const QUERY_BYTES: usize = 2000;
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const NOTES_HINT: &str = "bilbo matches whole words without stemming: try another form of a word, or --library for sources";
const LIBRARY_HINT: &str =
    "the library matches keywords only: try the words and language the source would use";

struct Request {
    /// The query arguments joined by single spaces.
    query: String,
    /// `text::words(&query)`, never empty.
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
    scope: Option<String>,
}

pub struct Output {
    /// stderr lines (without "bilbo: "), printed before stdout.
    pub warnings: Vec<String>,
    /// The plain view: three lines per hit, an empty line between hits, best hit first.
    pub lines: Vec<String>,
    /// The hits the human view shows, best first.
    pub hits: Vec<Shown>,
    /// Hits before `--limit`.
    pub total: usize,
    /// The query's folded words, to bold.
    pub words: Vec<String>,
    /// Scopes the config declares; the view shows a note's scope when there are two or more.
    pub scopes: usize,
}

/// One hit as the human view shows it.
pub struct Shown {
    /// The note's or source's title; `<corpus> guide` for a guide.
    pub title: String,
    /// The headings below the title, down to the passage's.
    pub headings: Vec<String>,
    /// The passage text as stored.
    pub text: String,
    pub meta: Meta,
}

pub enum Meta {
    Note {
        kind: String,
        scope: Option<String>,
        created: Option<String>,
        path: PathBuf,
        line: usize,
        /// No query word is in the passage: only the embedder found it.
        meaning: bool,
    },
    Library {
        guide: bool,
        reference: String,
        path: PathBuf,
        start: usize,
        end: usize,
    },
}

impl Output {
    /// The `note-recall` and `library-recall` human view; `lines` is the plain one.
    pub fn view(&self, term: &terminal::Term, now: jiff::Timestamp) -> Vec<String> {
        let rank_width = self.hits.len().to_string().len().max(2);
        let indent = " ".repeat(rank_width + 2);
        let avail = term.width.saturating_sub(rank_width + 2).max(1);
        let mut out = Vec::new();
        for (i, hit) in self.hits.iter().enumerate() {
            if i > 0 {
                out.push(String::new());
            }
            out.push(self.title_line(term, i, rank_width, hit));
            for line in self.snippet(term, &hit.text, avail) {
                out.push(format!("{indent}{line}"));
            }
            for line in meta_lines(term, now, &hit.meta, self.scopes, avail) {
                out.push(format!("{indent}{line}"));
            }
        }
        let (one, many) = match self.hits.first().map(|hit| &hit.meta) {
            Some(Meta::Library { .. }) => ("file", "files"),
            _ => ("note", "notes"),
        };
        let all = terminal::count(self.total as u64, one, many);
        let count = match self.hits.len() == self.total {
            true => format!("{all}, best first"),
            false => format!(
                "{} of {all}, best first {} --limit {} shows all",
                self.hits.len(),
                terminal::glyph(term, terminal::Mark::Dot),
                self.total
            ),
        };
        out.push(String::new());
        out.push(terminal::paint(term, terminal::Tone::Dim, &count));
        out
    }

    fn title_line(
        &self,
        term: &terminal::Term,
        i: usize,
        rank_width: usize,
        hit: &Shown,
    ) -> String {
        let rank = terminal::pad(&(i + 1).to_string(), rank_width, terminal::Align::Right);
        let mut line = format!(
            "{}  {}",
            terminal::paint(term, terminal::Tone::Dim, &rank),
            terminal::paint(
                term,
                terminal::Tone::Bold,
                &visible(&markdown::plain(&hit.title))
            )
        );
        for heading in &hit.headings {
            line.push_str(&format!(
                " {} {}",
                terminal::mark(term, terminal::Mark::Path),
                self.bold_words(term, &visible(&markdown::plain(heading)))
            ));
        }
        terminal::cut(term, &line, term.width)
    }

    /// `text` with each run of letters or digits that is a whole query word in bold.
    fn bold_words(&self, term: &terminal::Term, text: &str) -> String {
        let mut out = String::new();
        let mut at = 0;
        for (start, end) in runs(text) {
            out.push_str(&text[at..start]);
            let run = &text[start..end];
            match self.matches(run) {
                true => out.push_str(&terminal::paint(term, terminal::Tone::Bold, run)),
                false => out.push_str(run),
            }
            at = end;
        }
        out.push_str(&text[at..]);
        out
    }

    fn matches(&self, run: &str) -> bool {
        matches!(text::words(run).as_slice(), [word] if self.words.contains(word))
    }

    /// The passage cut down to the lines the view shows, windowed on its first matching word.
    fn snippet(&self, term: &terminal::Term, text: &str, avail: usize) -> Vec<String> {
        if let Some(code) = code_lines(text) {
            return code_snippet(term, &code, avail);
        }
        let flat = visible(&markdown::plain(text));
        if flat.is_empty() {
            return Vec::new();
        }
        let max_lines = if term.width < 60 { 2 } else { 3 };
        let unpainted = terminal::Term {
            paint: false,
            ..term.clone()
        };
        let cut = terminal::glyph(term, terminal::Mark::Cut);
        let lead = format!("{cut} ");
        let first = runs(&flat)
            .find(|(s, e)| self.matches(&flat[*s..*e]))
            .map(|(s, _)| s);
        let mut body = flat.clone();
        let mut led = false;
        if let Some(first) = first
            && !shows(
                &terminal::wrap(&unpainted, &flat, avail, terminal::Long::Cut),
                &flat,
                first,
                avail,
                max_lines,
            )
        {
            let from = flat[..first]
                .match_indices(['.', '!', '?', '•'])
                .map(|(at, mark)| at + mark.len())
                .filter(|end| flat[*end..].starts_with(' '))
                .map(|end| end + 1)
                .next_back()
                .unwrap_or(0);
            body = format!("{lead}{}", &flat[from..]);
            let target = lead.len() + first - from;
            if !shows(
                &terminal::wrap(&unpainted, &body, avail, terminal::Long::Cut),
                &body,
                target,
                avail,
                max_lines,
            ) {
                body = format!("{lead}{}", &flat[first..]);
            }
            led = true;
        }
        let mut lines = terminal::wrap(&unpainted, &body, avail, terminal::Long::Cut);
        let more = lines.len() > max_lines;
        if more {
            lines.truncate(max_lines);
            let last = lines.last_mut().expect("a line");
            let tail = format!(" {cut}");
            while terminal::width_of(last) + terminal::width_of(&tail) > avail {
                match last.rfind(' ') {
                    Some(at) => last.truncate(at),
                    None => break,
                }
            }
            last.push_str(&tail);
        }
        let last = lines.len() - 1;
        lines
            .into_iter()
            .enumerate()
            .map(|(i, mut line)| {
                let mut head = String::new();
                let mut tail = String::new();
                if led && i == 0 {
                    line = line[cut.len()..].to_string();
                    head = terminal::mark(term, terminal::Mark::Cut);
                }
                if more && i == last {
                    line.truncate(line.len() - cut.len());
                    tail = terminal::mark(term, terminal::Mark::Cut);
                }
                format!("{head}{}{tail}", self.bold_words(term, &line))
            })
            .collect()
    }
}

/// `text` with each control character but a tab written as an escape (`\u{1b}`), so a note's or a
/// source's text never reaches the terminal as a control sequence.
fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\t' {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// The lines inside a fenced block that opens the passage, blank lines left out, and whether
/// anything follows the lines returned (more block lines or text after the block); `None` when the
/// passage does not open with a fence.
fn code_lines(text: &str) -> Option<(Vec<String>, bool)> {
    let lines = markdown::lines(text);
    let (ch, len, _) = markdown::fence_run(lines.first()?)?;
    let close = lines[1..].iter().position(|line| {
        markdown::fence_run(line)
            .is_some_and(|(c, l, rest)| c == ch && l >= len && rest.trim().is_empty())
    });
    let (inside, after) = match close {
        Some(at) => (&lines[1..=at], &lines[at + 2..]),
        None => (&lines[1..], &lines[..0]),
    };
    let shown = inside
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            for c in line.chars() {
                match c {
                    '\t' => out.push_str("    "),
                    c if c.is_control() => out.extend(c.escape_default()),
                    c => out.push(c),
                }
            }
            out
        })
        .collect();
    Some((shown, after.iter().any(|line| !line.trim().is_empty())))
}

/// The first lines of a code passage as written, dim; the last one ends with a cut mark when more
/// follows.
fn code_snippet(
    term: &terminal::Term,
    (lines, follows): &(Vec<String>, bool),
    avail: usize,
) -> Vec<String> {
    let max_lines = if term.width < 60 { 2 } else { 3 };
    let more = lines.len() > max_lines || *follows;
    let unpainted = terminal::Term {
        paint: false,
        ..term.clone()
    };
    let tail = format!(" {}", terminal::glyph(term, terminal::Mark::Cut));
    let shown = lines.len().min(max_lines);
    lines
        .iter()
        .take(max_lines)
        .enumerate()
        .map(|(i, line)| {
            let room = avail.saturating_sub(terminal::width_of(&tail)).max(1);
            let text = match more && i + 1 == shown && terminal::width_of(line) <= room {
                true => format!("{line}{tail}"),
                false => terminal::cut(&unpainted, line, avail),
            };
            terminal::paint(term, terminal::Tone::Dim, &text)
        })
        .collect()
}

/// The byte ranges of the runs of letters and digits in `text`.
fn runs(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while chars.next_if(|(_, c)| !c.is_alphanumeric()).is_some() {}
        let (start, _) = *chars.peek()?;
        let mut end = start;
        while let Some((at, c)) = chars.next_if(|(_, c)| c.is_alphanumeric()) {
            end = at + c.len_utf8();
        }
        Some((start, end))
    })
}

/// Whether the word at byte `target` of `text` is still in view in the `lines` it wraps to: on a line
/// before the last, or on the last within its first `avail - 2` columns unless more lines follow it.
/// Wrapping never merges or splits a word, so the word is found by its index, not by bytes.
fn shows(lines: &[String], text: &str, target: usize, avail: usize, max_lines: usize) -> bool {
    let word = text[..target].matches(' ').count();
    let mut seen = 0;
    for (i, line) in lines.iter().enumerate() {
        let words = line.split(' ').count();
        if word < seen + words {
            let before: Vec<&str> = line.split(' ').take(word - seen).collect();
            let column = match before.is_empty() {
                true => 0,
                false => terminal::width_of(&before.join(" ")) + 1,
            };
            return i < max_lines - 1
                || (i == max_lines - 1 && (lines.len() == max_lines || column < avail - 2));
        }
        seen += words;
    }
    false
}

/// The dim meta lines of a hit, broken at `avail` columns.
fn meta_lines(
    term: &terminal::Term,
    now: jiff::Timestamp,
    meta: &Meta,
    scopes: usize,
    avail: usize,
) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut colored: Vec<String> = Vec::new();
    let dot = format!(" {} ", terminal::glyph(term, terminal::Mark::Dot));
    let tail = match meta {
        Meta::Note {
            kind,
            scope,
            created,
            path,
            line,
            meaning,
        } => {
            parts.push(kind.clone());
            colored.push(kind.clone());
            if let (Some(scope), true) = (scope, scopes >= 2) {
                parts.push(scope.clone());
                colored.push(scope.clone());
            }
            if let Some(created) = created {
                parts.push(terminal::ago(now, created));
            }
            if *meaning {
                parts.push("by meaning".into());
            }
            format!("{}:{line}", visible(&terminal::tilde(term, path)))
        }
        Meta::Library {
            guide,
            reference,
            start,
            end,
            ..
        } => {
            let shelf = if *guide { "guide" } else { "source" };
            parts.push(shelf.into());
            colored.push(shelf.into());
            format!("{reference}{dot}lines {start}-{end}")
        }
    };
    let mut next = 0;
    let (path, linked) = match meta {
        Meta::Note { path, .. } => (path, None),
        Meta::Library {
            path, reference, ..
        } => (path, Some(reference.as_str())),
    };
    let piece = |piece: &str| match linked {
        Some(reference) if piece.contains(reference) => {
            piece.replacen(reference, &terminal::link(term, path, reference), 1)
        }
        Some(_) => piece.to_string(),
        None => terminal::link(term, path, piece),
    };
    terminal::tail_lines_with(&parts.join(&dot), &tail, &dot, avail, piece)
        .into_iter()
        .map(|line| {
            let tokens: Vec<String> = line
                .split(' ')
                .map(|token| match colored.get(next) {
                    Some(word) if word == token => {
                        next += 1;
                        terminal::paint(term, terminal::Tone::Cyan, token)
                    }
                    _ => token.to_string(),
                })
                .collect();
            terminal::paint(term, terminal::Tone::Dim, &tokens.join(" "))
        })
        .collect()
}

pub fn run(
    args: &[String],
    env: &store::Env,
    spinner: &prompt::Spinner,
) -> Result<Output, Failure> {
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
    let stored = documents::read_notes(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;

    let withheld = vectors::withheld(&stored, &settings);
    let (documents, found): (Vec<Document>, Vec<Found>) = stored
        .into_iter()
        .map(|n| {
            (
                n.document,
                Found {
                    path: n.path,
                    kind: n.kind,
                    created: n.created,
                    scope: n.scope,
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
    let (meaning, warnings) = match &settings.embedder {
        Some(embedder) => meaning(
            (embedder, spinner),
            env,
            &root,
            &request.query,
            &documents,
            &allowed,
            &withheld,
        ),
        None => (Vec::new(), Vec::new()),
    };
    let fused = rank::fuse(&keyword, &meaning);
    let total = fused.len();
    let hits: Vec<Hit> = fused.into_iter().take(request.limit).collect();
    if hits.is_empty() {
        return Err(Failure::Unmatched {
            warnings,
            message: "no notes match".into(),
            query: request.query,
            hint: NOTES_HINT.into(),
        });
    }

    let mut out = Vec::new();
    let mut shown = Vec::new();
    for hit in hits {
        let Found {
            path,
            kind,
            created,
            scope,
        } = &found[hit.document];
        let passage = &documents[hit.document].passages[hit.passage];
        shown.push(Shown {
            title: passage.path[0].clone(),
            headings: passage.path[1..].to_vec(),
            text: passage.text.clone(),
            meta: Meta::Note {
                kind: kind.clone(),
                scope: scope.clone(),
                created: created.clone(),
                path: path.clone(),
                line: passage.line,
                meaning: rank::shared(passage, &request.words) == 0,
            },
        });
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
        hits: shown,
        total,
        words: request.words,
        scopes: settings.scopes.len(),
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
    let mut shelved = documents::read_library(root, &request.corpora)
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
    let fused = rank::fuse(&rank::keyword(&request.words, &documents), &[]);
    let total = fused.len();
    let hits: Vec<Hit> = fused.into_iter().take(request.limit).collect();
    if hits.is_empty() {
        return Err(Failure::Unmatched {
            warnings: Vec::new(),
            message: "no sources match".into(),
            query: request.query.clone(),
            hint: LIBRARY_HINT.into(),
        });
    }

    let mut out = Vec::new();
    let mut shown = Vec::new();
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
        let guide = matches!(file.shelf, Shelf::Guide);
        shown.push(Shown {
            title: match guide {
                true => format!("{} guide", file.reference),
                false => passage.path[0].clone(),
            },
            headings: passage.path[1..].to_vec(),
            text: passage.text.clone(),
            meta: Meta::Library {
                guide,
                reference: file.reference.clone(),
                path: file.path.clone(),
                start,
                end,
            },
        });
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
        hits: shown,
        total,
        words: request.words.clone(),
        scopes: 0,
    })
}

/// The meaning order (at most `rank::CANDIDATES` hits, only documents whose `allowed` is true) and the warnings.
fn meaning(
    (embedder, spinner): (&config::Embedder, &prompt::Spinner),
    env: &store::Env,
    root: &Path,
    query: &str,
    documents: &[Document],
    allowed: &[bool],
    withheld: &HashSet<u64>,
) -> (Vec<Hit>, Vec<String>) {
    let cache = store::cache_dir(env)
        .map(|dir| vectors::load(&vectors::path(&dir, root)))
        .unwrap_or_default();
    let (found, missing) = vectors::lookup(&cache, &embedder.model, documents, withheld);
    let indexed_any = found.iter().flatten().any(Option::is_some);

    let mut warnings = Vec::new();
    let mut hits = Vec::new();
    if indexed_any {
        let mut text = format!("{}{query}", embedder.query_prefix);
        text.truncate(text.floor_char_boundary(QUERY_BYTES));
        let answer = spinner.wait("Waiting for the embedder", || {
            embed::query(embedder, &text, QUERY_TIMEOUT, cache.dims)
        });
        match answer {
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
        let unit = if missing == 1 { "passage" } else { "passages" };
        warnings.push(format!("{missing} {unit} not indexed; run bilbo index"));
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
    let words = text::words(&query);
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

/// `bilbo recall --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo recall: search the notes, or the library's sources, best match first.

Usage:
  bilbo recall <query>... [--kind <kind>]... [--limit <n>]
  bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]

Options:
  --kind <kind>      Only notes of this kind; repeat for several kinds
  --limit <n>        Print at most n hits (default 10)
  --library          Search the library's sources and guides, by keyword only
  --corpus <corpus>  Only this corpus; repeat for several; implies --library
  --                 End the options: every later argument is a query word
Options go before or after the query words, and --kind=<kind> works too.
Kinds: plan, spec, design, decision, gotcha, research, review, report,
reference.

Matching: whole words of 2 or more letters or digits, ignoring case and
accents, with no stemming ('notes' does not find 'note'). With an embedder,
notes also match by meaning; when it is down, or a note changed since the
last 'bilbo index', stderr says so.

Output: one block per hit, best first, a blank line between blocks:
  <path>:<line> TAB <kind> TAB <created>
  <heading path>
  <first 300 characters of the passage, or ->
With --library the first line is
  <path>:<line> TAB source|guide TAB <reference> TAB <start>-<end>
where <reference> is what 'bilbo library show' or 'bilbo library' takes, and
the heading path starts below the title, or is - for the title's own text.

Exit: 0 hits; 1 nothing matched, or no store, library or corpus; 2 usage or
config error.

Examples:
  bilbo recall sqlite busy timeout
  bilbo recall --kind decision --kind gotcha -- release tags
  bilbo recall --library --corpus go -- 'goroutine leaks'

Docs: https://github.com/delucca/bilbo/wiki/Commands#recall
"#;

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

    fn now() -> jiff::Timestamp {
        "2026-10-07T01:00:00-03:00".parse().unwrap()
    }

    fn note(title: &str, headings: &[&str], text: &str, meta: Meta) -> Shown {
        Shown {
            title: title.into(),
            headings: headings.iter().map(|h| h.to_string()).collect(),
            text: text.into(),
            meta,
        }
    }

    fn note_meta(kind: &str, created: &str, file: &str, line: usize) -> Meta {
        Meta::Note {
            kind: kind.into(),
            scope: None,
            created: Some(created.into()),
            path: format!("/home/a/.local/share/bilbo/notes/{file}").into(),
            line,
            meaning: false,
        }
    }

    fn output(hits: Vec<Shown>, total: usize, query: &str, scopes: usize) -> Output {
        Output {
            warnings: Vec::new(),
            lines: Vec::new(),
            hits,
            total,
            words: text::words(query),
            scopes,
        }
    }

    fn busy() -> Shown {
        note(
            "Sqlite busy timeout",
            &["Fix"],
            "Set `busy_timeout = 5000` right after opening the connection, before any query. Without it a second writer gets SQLITE_BUSY at once instead of waiting.",
            note_meta(
                "gotcha",
                "2026-10-07T00:43-03:00",
                "gotcha-sqlite-busy-timeout.md",
                9,
            ),
        )
    }

    fn release() -> Shown {
        note(
            "Release checklist",
            &["Steps", "Rollback"],
            "The release goes out in three stages, each gated by a manual check. First the build is tagged and the artifacts are signed on the release machine, which keeps the signing key offline. Then the staging deploy runs the full smoke suite against a copy of production data, and the on-call engineer reads the dashboards for twenty minutes. Only after both pass does the production deploy start. If the error rate climbs, stop the deploy and raise the busy flag; the timeout for the health probe is ninety seconds.",
            note_meta(
                "decision",
                "2026-10-02T00:00-03:00",
                "decision-release-checklist.md",
                31,
            ),
        )
    }

    fn expect(notation: &str) -> Vec<String> {
        notation.lines().map(terminal::styled).collect()
    }

    fn unpainted(notation: &str) -> Vec<String> {
        notation.lines().map(terminal::plain).collect()
    }

    /// The painted view stripped of its codes is the unpainted view: paint never changes layout.
    fn same_layout(output: &Output, width: usize, unicode: bool) {
        let painted = output.view(&terminal::fixed(width, true, unicode), now());
        let stripped: Vec<String> = painted
            .iter()
            .map(|line| terminal::stripped(line))
            .collect();
        assert_eq!(
            stripped,
            output.view(&terminal::fixed(width, false, unicode), now())
        );
    }

    const N1: &str = "{d} 1{/d}  {b}Sqlite busy timeout{/b} {d}›{/d} Fix
    Set {b}busy{/b}_{b}timeout{/b} = 5000 right after opening the connection, before any query. Without it a
    second writer gets SQLITE_{b}BUSY{/b} at once instead of waiting.
    {d}{c}gotcha{/c} · 17 min ago · ~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:9{/d}

{d} 2{/d}  {b}Release checklist{/b} {d}›{/d} Steps {d}›{/d} Rollback
    {d}…{/d} If the error rate climbs, stop the deploy and raise the {b}busy{/b} flag; the {b}timeout{/b} for the health
    probe is ninety seconds.
    {d}{c}decision{/c} · 5 days ago · ~/.local/share/bilbo/notes/decision-release-checklist.md:31{/d}

{d}2 notes, best first{/d}";

    #[test]
    fn two_notes_at_full_width() {
        let out = output(vec![busy(), release()], 2, "busy timeout", 1);
        assert_eq!(
            out.view(&terminal::fixed(100, true, true), now()),
            expect(N1)
        );
        assert_eq!(
            out.view(&terminal::fixed(100, false, true), now()),
            unpainted(N1)
        );
        same_layout(&out, 100, true);
    }

    #[test]
    fn two_notes_at_50_columns() {
        let out = output(vec![busy(), release()], 2, "busy timeout", 1);
        let want = " 1  Sqlite busy timeout › Fix
    Set busy_timeout = 5000 right after opening
    the connection, before any query. Without it …
    gotcha · 17 min ago
    ~/.local/share/bilbo/notes/
    gotcha-sqlite-busy-timeout.md:9

 2  Release checklist › Steps › Rollback
    … If the error rate climbs, stop the deploy
    and raise the busy flag; the timeout for the …
    decision · 5 days ago
    ~/.local/share/bilbo/notes/
    decision-release-checklist.md:31

2 notes, best first";
        let term = terminal::fixed(50, false, true);
        let view = out.view(&term, now());
        assert_eq!(view, unpainted(want));
        assert!(view.iter().all(|l| terminal::width_of(l) <= 50));
        same_layout(&out, 50, true);
    }

    fn passage(text: &str) -> Shown {
        note(
            "Wide tokens",
            &[],
            text,
            note_meta("gotcha", "2026-10-07T00:43-03:00", "gotcha-wide.md", 3),
        )
    }

    #[test]
    fn a_wide_token_before_an_accented_match_does_not_panic() {
        let text = format!(
            "https://example.com/{} café is the word we want to find in this passage of text",
            "a".repeat(32)
        );
        let out = output(vec![passage(&text)], 1, "café", 1);
        for width in [50, 60, 100] {
            let view = out.view(&terminal::fixed(width, false, true), now());
            assert!(view.iter().any(|l| l.contains("café")), "{view:?}");
        }
    }

    #[test]
    fn a_match_on_the_edge_of_the_last_visible_line() {
        let at = |pad: usize| {
            let text = format!(
                "{} {} ox and then some more words",
                "a".repeat(45),
                "b".repeat(pad)
            );
            let out = output(vec![passage(&text)], 1, "ox", 1);
            out.view(&terminal::fixed(50, false, true), now())
        };
        assert!(at(42)[1].starts_with("    aaaa"));
        assert!(at(43)[1].starts_with("    … "));
    }

    #[test]
    fn one_note_in_ascii() {
        let out = output(vec![busy()], 1, "busy timeout", 1);
        let want = " 1  Sqlite busy timeout > Fix
    Set busy_timeout = 5000 right after opening the connection, before any query. Without it a
    second writer gets SQLITE_BUSY at once instead of waiting.
    gotcha - 17 min ago - ~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:9

1 note, best first";
        assert_eq!(
            out.view(&terminal::fixed(100, false, false), now()),
            unpainted(want)
        );
    }

    #[test]
    fn a_limit_a_scope_and_a_meaning_hit() {
        let hit = note(
            "Write-ahead logging notes",
            &["Readers and writers"],
            "Readers never block the writer and the writer never blocks readers; checkpoints run when the log passes 1000 pages.",
            Meta::Note {
                kind: "research".into(),
                scope: Some("personal".into()),
                created: Some("2026-08-02T10:00-03:00".into()),
                path: "/home/a/.local/share/bilbo/notes/research-wal-notes.md".into(),
                line: 12,
                meaning: true,
            },
        );
        let out = output(vec![hit], 5, "sqlite lock contention", 2);
        let want = "{d} 1{/d}  {b}Write-ahead logging notes{/b} {d}›{/d} Readers and writers
    Readers never block the writer and the writer never blocks readers; checkpoints run when the log
    passes 1000 pages.
    {d}{c}research{/c} · {c}personal{/c} · 2026-08-02 · by meaning{/d}
    {d}~/.local/share/bilbo/notes/research-wal-notes.md:12{/d}

{d}1 of 5 notes, best first · --limit 5 shows all{/d}";
        assert_eq!(
            out.view(&terminal::fixed(100, true, true), now()),
            expect(want)
        );
        same_layout(&out, 100, true);
    }

    #[test]
    fn library_hits() {
        let lib = |reference: &str, start, end, guide| Meta::Library {
            guide,
            reference: reference.into(),
            path: format!("/home/a/.local/share/bilbo/library/{reference}.md").into(),
            start,
            end,
        };
        let hits = vec![
            note(
                "Clippy Lints",
                &["unnecessary\\_clippy\\_cfg", "What it does"],
                "Checks for `#[cfg_attr(clippy, allow(clippy::lint))]` and suggests to replace it with `#[allow(clippy::lint)]`.",
                lib("rust/clippy-lints", 30972, 30976, false),
            ),
            note(
                "rust guide",
                &["effective-rust-clippy"],
                "Item 29: argues Clippy earns its own Item, surveying the correctness, idiom, concision, and performance lint categories, how to configure and suppress lints, and recommends reading the full lint list as a way to learn idiomatic Rust.",
                lib("rust", 97, 100, true),
            ),
            note(
                "Clippy",
                &[],
                "[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/clippy.svg)](https://github.com/rust-lang/rust-clippy#license) A collection of lints to catch common mistakes and improve your [Rust](https://github.com/rust-lang/rust) code. [There are over 800 lints included in this crate!](https://rust-lang.github.io/rust-clippy/master/index.html)",
                lib("rust/introduction-clippy-documentation", 10, 51, false),
            ),
        ];
        let out = output(hits, 41, "clippy lint", 0);
        let want = "{d} 1{/d}  {b}Clippy Lints{/b} {d}›{/d} unnecessary_{b}clippy{/b}_cfg {d}›{/d} What it does
    Checks for #[cfg_attr({b}clippy{/b}, allow({b}clippy{/b}::{b}lint{/b}))] and suggests to replace it with
    #[allow({b}clippy{/b}::{b}lint{/b})].
    {d}{c}source{/c} · rust/clippy-lints · lines 30972-30976{/d}

{d} 2{/d}  {b}rust guide{/b} {d}›{/d} effective-rust-{b}clippy{/b}
    Item 29: argues {b}Clippy{/b} earns its own Item, surveying the correctness, idiom, concision, and
    performance {b}lint{/b} categories, how to configure and suppress lints, and recommends reading the
    full {b}lint{/b} list as a way to learn idiomatic Rust.
    {d}{c}guide{/c} · rust · lines 97-100{/d}

{d} 3{/d}  {b}Clippy{/b}
    A collection of lints to catch common mistakes and improve your Rust code. There are over 800
    lints included in this crate!
    {d}{c}source{/c} · rust/introduction-clippy-documentation · lines 10-51{/d}

{d}3 of 41 files, best first · --limit 41 shows all{/d}";
        assert_eq!(
            out.view(&terminal::fixed(100, true, true), now()),
            expect(want)
        );
        same_layout(&out, 100, true);
    }

    fn pragmas() -> Shown {
        note(
            "Sqlite pragmas",
            &["Setup"],
            "```sql\nPRAGMA busy_timeout = 5000;\nPRAGMA journal_mode = WAL;\nPRAGMA synchronous = NORMAL;\nPRAGMA foreign_keys = ON;\n```",
            note_meta(
                "gotcha",
                "2026-10-07T00:43-03:00",
                "gotcha-sqlite-pragmas.md",
                9,
            ),
        )
    }

    #[test]
    fn a_code_block_shows_as_written() {
        let out = output(vec![pragmas()], 1, "busy timeout", 1);
        let want = "{d} 1{/d}  {b}Sqlite pragmas{/b} {d}›{/d} Setup
    {d}PRAGMA busy_timeout = 5000;{/d}
    {d}PRAGMA journal_mode = WAL;{/d}
    {d}PRAGMA synchronous = NORMAL; …{/d}
    {d}{c}gotcha{/c} · 17 min ago · ~/.local/share/bilbo/notes/gotcha-sqlite-pragmas.md:9{/d}

{d}1 note, best first{/d}";
        assert_eq!(
            out.view(&terminal::fixed(100, true, true), now()),
            expect(want)
        );
        same_layout(&out, 100, true);
    }

    #[test]
    fn a_code_block_at_50_columns_and_in_ascii() {
        let out = output(vec![pragmas()], 1, "busy timeout", 1);
        let want = " 1  Sqlite pragmas › Setup
    PRAGMA busy_timeout = 5000;
    PRAGMA journal_mode = WAL; …
    gotcha · 17 min ago
    ~/.local/share/bilbo/notes/
    gotcha-sqlite-pragmas.md:9

1 note, best first";
        assert_eq!(
            out.view(&terminal::fixed(50, false, true), now()),
            unpainted(want)
        );
        same_layout(&out, 50, true);
        let view = out.view(&terminal::fixed(100, false, false), now());
        assert_eq!(view[3], "    PRAGMA synchronous = NORMAL; ...");
        same_layout(&out, 100, false);
    }

    /// The passage lines of a hit whose passage is `text`, unpainted at 100 columns.
    fn passage_lines(text: &str) -> Vec<String> {
        let view = output(vec![passage(text)], 1, "zzz", 1)
            .view(&terminal::fixed(100, false, true), now());
        view[1..view.len() - 3].to_vec()
    }

    #[test]
    fn code_lines_keep_their_indent_and_end_cleanly() {
        assert_eq!(
            passage_lines("```\n\tlet x = 1;\n    let y = 2;\n```"),
            ["        let x = 1;", "        let y = 2;"]
        );
    }

    #[test]
    fn text_after_a_block_ends_with_a_cut_mark() {
        assert_eq!(
            passage_lines("```\nmake test\n```\nThen read the log."),
            ["    make test …"]
        );
    }

    #[test]
    fn blank_lines_in_a_block_are_left_out() {
        assert_eq!(
            passage_lines("```\nfirst\n\nsecond\n```"),
            ["    first", "    second"]
        );
    }

    #[test]
    fn an_escape_in_a_block_is_written_out() {
        let lines = passage_lines("```\necho \u{1b}[31mred\n```");
        assert_eq!(lines, ["    echo \\u{1b}[31mred"]);
        assert!(lines.iter().all(|l| !l.contains('\u{1b}')));
    }

    #[test]
    fn an_empty_block_shows_no_passage() {
        let view = output(vec![passage("```\n```")], 1, "zzz", 1)
            .view(&terminal::fixed(100, false, true), now());
        assert!(view[1].starts_with("    gotcha"), "{view:?}");
    }

    #[test]
    fn an_unclosed_block_shows_its_lines() {
        assert_eq!(passage_lines("```\na\nb"), ["    a", "    b"]);
    }

    #[test]
    fn prose_that_holds_a_block_stays_flattened() {
        assert_eq!(
            passage_lines("Run this first:\n```\nmake\n```"),
            ["    Run this first: make"]
        );
    }

    #[test]
    fn a_long_code_line_is_cut() {
        let long = "a".repeat(120);
        let lines = passage_lines(&format!("```\n{long}\n```"));
        assert_eq!(lines, [format!("    {}…", "a".repeat(95))]);
    }

    #[test]
    fn a_cut_code_line_with_more_after_it_has_one_mark() {
        let long = "a".repeat(120);
        let lines = passage_lines(&format!(
            "```
{long}
```
Then read the log."
        ));
        assert_eq!(lines, [format!("    {}…", "a".repeat(95))]);
    }

    #[test]
    fn control_characters_never_reach_the_human_view() {
        let hit = note(
            "Bell\u{7}\u{1b}[2Jtitle",
            &["Head\u{1b}[31ming"],
            "Prose with \u{1b}[31mred\u{7} and \u{9b}1m.",
            note_meta("gotcha", "2026-10-07T00:43-03:00", "gotcha-x.md", 9),
        );
        let out = output(vec![hit], 1, "prose", 1);
        for paint in [false, true] {
            let view = out.view(&terminal::fixed(100, paint, true), now());
            let all = view.join("\n");
            let bare = terminal::stripped(&all);
            assert!(!bare.contains(['\u{7}', '\u{9b}']), "{bare:?}");
            assert!(!bare.contains('\u{1b}'), "{bare:?}");
            if !paint {
                assert!(!all.contains('\u{1b}'));
            }
        }
        let view = out.view(&terminal::fixed(100, false, true), now());
        assert_eq!(
            view[0],
            " 1  Bell\\u{7}\\u{1b}[2Jtitle › Head\\u{1b}[31ming"
        );
        assert_eq!(
            view[1],
            "    Prose with \\u{1b}[31mred\\u{7} and \\u{9b}1m."
        );
        let lines = output(vec![passage("```\nbell\u{7}\n```")], 1, "zzz", 1)
            .view(&terminal::fixed(100, false, true), now());
        assert_eq!(lines[1], "    bell\\u{7}");
    }

    fn linked() -> terminal::Term {
        terminal::Term {
            links: Some("shire-box".into()),
            ..terminal::fixed(100, true, true)
        }
    }

    #[test]
    fn a_note_path_is_linked() {
        let out = output(vec![busy()], 1, "busy timeout", 1);
        let view = out.view(&linked(), now());
        let want = "    {d}{c}gotcha{/c} · 17 min ago · {l:file://shire-box/home/a/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md}~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:9{/l}{/d}";
        assert_eq!(view[3], terminal::styled(want));
        let unlinked = out.view(&terminal::fixed(100, true, true), now());
        let stripped: Vec<String> = view.iter().map(|l| terminal::stripped(l)).collect();
        let want: Vec<String> = unlinked.iter().map(|l| terminal::stripped(l)).collect();
        assert_eq!(stripped, want);
        assert!(!unlinked.join("").contains("\x1b]8"));
    }

    #[test]
    fn a_broken_path_carries_the_link_on_each_piece() {
        let out = output(vec![pragmas()], 1, "busy timeout", 1);
        let term = terminal::Term {
            links: Some("shire-box".into()),
            ..terminal::fixed(50, true, true)
        };
        let view = out.view(&term, now());
        let uri = "file://shire-box/home/a/.local/share/bilbo/notes/gotcha-sqlite-pragmas.md";
        assert_eq!(
            view[view.len() - 4],
            terminal::styled(&format!(
                "    {{d}}{{l:{uri}}}~/.local/share/bilbo/notes/{{/l}}{{/d}}"
            ))
        );
        assert_eq!(
            view[view.len() - 3],
            terminal::styled(&format!(
                "    {{d}}{{l:{uri}}}gotcha-sqlite-pragmas.md:9{{/l}}{{/d}}"
            ))
        );
    }

    #[test]
    fn a_library_reference_is_linked() {
        let hit = note(
            "Effective Go",
            &["Concurrency"],
            "Goroutines run concurrently.",
            Meta::Library {
                guide: false,
                reference: "go/effective-go".into(),
                path: "/home/a/.local/share/bilbo/library/go/effective-go.md".into(),
                start: 340,
                end: 380,
            },
        );
        let view = output(vec![hit], 1, "goroutine", 0).view(&linked(), now());
        let want = "    {d}{c}source{/c} · {l:file://shire-box/home/a/.local/share/bilbo/library/go/effective-go.md}go/effective-go{/l} · lines 340-380{/d}";
        assert_eq!(view[2], terminal::styled(want));
    }

    #[test]
    fn an_empty_passage_shows_no_snippet() {
        let mut hit = busy();
        hit.text = String::new();
        let view = output(vec![hit], 1, "busy", 1).view(&terminal::fixed(100, false, true), now());
        assert_eq!(view[0], " 1  Sqlite busy timeout › Fix");
        assert!(view[1].starts_with("    gotcha"));
    }

    #[test]
    fn ranks_are_right_aligned() {
        for (count, first, second) in [(10, " 1  ", "10  "), (100, "  1  ", "100  ")] {
            let hits = (0..count).map(|_| busy()).collect();
            let view =
                output(hits, count, "busy", 1).view(&terminal::fixed(100, false, true), now());
            assert!(view[0].starts_with(first), "{:?}", view[0]);
            let last = view.iter().rev().find(|l| l.contains("Sqlite")).unwrap();
            assert!(last.starts_with(second), "{last:?}");
            let indent = first.len();
            assert!(
                view[1].starts_with(&" ".repeat(indent)) && !view[1][indent..].starts_with(' ')
            );
        }
    }
}
