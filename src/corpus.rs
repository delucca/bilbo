use std::path::{Path, PathBuf};

use crate::note::{self, Problem, is_topic};
use crate::rank;
use crate::source::{self, Source};
use crate::store::{self, Entry, EntryKind};

pub const RESERVED: [&str; 5] = ["show", "stage", "land", "plan", "read"];
pub const STUB_SOURCE: &str = "TODO: describe this source.";
pub const STUB_CORPUS: &str = "TODO: describe this corpus.";
const STALE_PREFIX: &str = "stale: re-ingested ";
const GUIDE_KEYS: [&str; 2] = ["id", "created"];
const NAME_RULE: &str = "use segments of a-z and 0-9 joined by single hyphens";

pub fn is_corpus_name(s: &str) -> bool {
    is_topic(s) && !RESERVED.contains(&s)
}

pub fn is_source_name(s: &str) -> bool {
    is_topic(s) && s != "guide"
}

/// A physical line of a guide.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct GuideEntry {
    pub name: String,
    /// Physical line of the `## <name>` heading.
    pub line: usize,
    /// The lines under the heading, up to the next entry.
    pub prose: Vec<Line>,
}

pub struct Guide {
    pub id: Option<String>,
    pub created: Option<String>,
    pub title: Option<String>,
    /// The lines between the title and the first entry.
    pub lead: Vec<Line>,
    pub entries: Vec<GuideEntry>,
    /// Frontmatter and title problems, repeated entries, and stub and stale lines.
    pub problems: Vec<Problem>,
}

/// Strict read of a guide's text.
pub fn read_guide(text: &str) -> Guide {
    let mut front = source::split_front(text, &GUIDE_KEYS);
    front.missing(&GUIDE_KEYS);
    let mut problems = std::mem::take(&mut front.problems);
    let mut guide = Guide {
        id: None,
        created: None,
        title: None,
        lead: Vec::new(),
        entries: Vec::new(),
        problems: Vec::new(),
    };
    if let Some(pair) = front.get("id") {
        if note::is_ulid(&pair.value) {
            guide.id = Some(pair.value.clone());
        } else {
            problems.push(Problem::at(pair.line, note::bad_id(&pair.value)));
        }
    }
    if let Some(pair) = front.get("created") {
        if note::is_created(&pair.value) {
            guide.created = Some(pair.value.clone());
        } else {
            problems.push(Problem::at(pair.line, note::bad_created(&pair.value)));
        }
    }
    if !front.body_known {
        guide.problems = problems;
        return guide;
    }

    let lines = note::lines(text);
    let first = front.body_start;
    let body = lines.get(first - 1..).unwrap_or(&[]);
    problems.extend(note::title_problem(body, first));

    let mut title_line = None;
    let mut entry_starts: Vec<(usize, String)> = Vec::new();
    for (i, line) in fenced_free(body) {
        match rank::heading(line) {
            Some((1, text)) if title_line.is_none() && entry_starts.is_empty() => {
                title_line = Some(i);
                guide.title = Some(text);
            }
            Some((2, text)) => entry_starts.push((i, text)),
            _ => {}
        }
    }

    let numbered = |range: std::ops::Range<usize>| -> Vec<Line> {
        range
            .map(|i| Line {
                number: first + i,
                text: body[i].to_string(),
            })
            .collect()
    };
    let lead_from = title_line.map_or(0, |t| t + 1);
    let lead_to = entry_starts.first().map_or(body.len(), |(i, _)| *i);
    guide.lead = numbered(lead_from.min(lead_to)..lead_to);
    for (k, (i, name)) in entry_starts.iter().enumerate() {
        let end = entry_starts.get(k + 1).map_or(body.len(), |(j, _)| *j);
        guide.entries.push(GuideEntry {
            name: name.clone(),
            line: first + i,
            prose: numbered(i + 1..end),
        });
    }

    for (k, entry) in guide.entries.iter().enumerate() {
        if guide.entries[..k].iter().any(|e| e.name == entry.name) {
            problems.push(Problem::at(
                entry.line,
                format!("entry '{}': given more than once", entry.name),
            ));
        }
        for line in &entry.prose {
            if is_stub(&line.text) {
                problems.push(Problem::at(
                    line.number,
                    format!(
                        "entry '{}': TODO stub; write the entry and remove the TODO line",
                        entry.name
                    ),
                ));
            } else if is_stale(&line.text) {
                problems.push(Problem::at(
                    line.number,
                    format!(
                        "entry '{}': stale; re-read the source, revise the entry and remove the stale line",
                        entry.name
                    ),
                ));
            }
        }
    }
    for line in &guide.lead {
        if is_stub(&line.text) {
            problems.push(Problem::at(
                line.number,
                "lead: TODO stub; describe the corpus and remove the TODO line",
            ));
        } else if is_stale(&line.text) {
            problems.push(Problem::at(line.number, "lead: stale line; remove it"));
        }
    }
    problems.sort_by_key(|p| p.line.unwrap_or(usize::MAX));
    guide.problems = problems;
    guide
}

fn is_stub(line: &str) -> bool {
    line == STUB_SOURCE || line == STUB_CORPUS
}

fn is_stale(line: &str) -> bool {
    line.starts_with(STALE_PREFIX)
}

/// The lines outside fenced code blocks, with their index, fence lines excluded.
fn fenced_free<'a>(lines: &[&'a str]) -> Vec<(usize, &'a str)> {
    let mut fence: Option<(char, usize)> = None;
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let run = note::fence_run(line);
        match (fence, run) {
            (Some((ch, len)), Some((c, l, rest)))
                if c == ch && l >= len && rest.trim().is_empty() =>
            {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some((c, l, _))) => fence = Some((c, l)),
            (None, None) => out.push((i, *line)),
        }
    }
    out
}

/// A new guide for `corpus`: frontmatter, the corpus name as the title, and the corpus stub.
pub fn new_guide(id: &str, created: &str, corpus: &str) -> String {
    format!("---\nid: {id}\ncreated: {created}\n---\n\n# {corpus}\n\n{STUB_CORPUS}\n")
}

/// `text` with a `## <name>` entry and its stub appended.
pub fn add_entry(text: &str, name: &str) -> String {
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&format!("## {name}\n\n{STUB_SOURCE}\n"));
    out
}

/// `text` with the stale line directly under the heading of entry `name`, replacing any stale line the entry
/// holds; unchanged when the guide has no such entry.
pub fn mark_stale(text: &str, name: &str, date: &str) -> String {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let heads: Vec<(usize, String)> = fenced_free(&lines)
        .into_iter()
        .filter_map(|(i, line)| match rank::heading(line) {
            Some((2, text)) => Some((i, text)),
            _ => None,
        })
        .collect();
    let Some(k) = heads.iter().position(|(_, text)| text == name) else {
        return text.to_string();
    };
    let start = heads[k].0;
    let end = heads.get(k + 1).map_or(lines.len(), |(j, _)| *j);
    let stale = format!("{STALE_PREFIX}{date}; re-read the source and revise this entry.");
    let mut out: Vec<&str> = lines.drain(..=start).collect();
    out.push(&stale);
    out.extend(lines.drain(..end - start - 1).filter(|l| !is_stale(l)));
    out.extend(lines);
    out.join("\n")
}

/// A source file of a corpus, read.
pub struct SourceFile {
    pub name: String,
    pub path: PathBuf,
    pub text: String,
    pub source: Source,
}

impl SourceFile {
    pub fn body(&self) -> &str {
        self.source.body(&self.text)
    }
}

/// The valid corpus folders of `<root>/library/`, sorted by name; none when it is missing.
pub fn corpus_dirs(root: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let library = store::library_dir(root);
    if !library.is_dir() {
        return Ok(Vec::new());
    }
    Ok(store::entries(&library)?
        .into_iter()
        .filter(|e| e.utf8 && e.kind == EntryKind::Folder && is_corpus_name(&e.name))
        .map(|e| (e.name, e.path))
        .collect())
}

/// The sources of a corpus folder in name order: regular, UTF-8 files named `<name>.md` with a source name. A
/// file that cannot be read as text is skipped.
pub fn read_sources(dir: &Path) -> std::io::Result<Vec<SourceFile>> {
    let mut out = Vec::new();
    for entry in store::entries(dir)? {
        let Some(name) = source_stem(&entry) else {
            continue;
        };
        if entry.kind != EntryKind::File {
            continue;
        }
        out.extend(read_source(&entry.path, name));
    }
    Ok(out)
}

/// The source file at `path`, `None` when it cannot be read as UTF-8 text.
pub fn read_source(path: &Path, name: String) -> Option<SourceFile> {
    let text = std::fs::read(path)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())?;
    Some(SourceFile {
        name,
        source: source::read(&text),
        path: path.to_path_buf(),
        text,
    })
}

fn source_stem(entry: &Entry) -> Option<String> {
    let stem = entry.name.strip_suffix(".md")?;
    (entry.utf8 && is_source_name(stem)).then(|| stem.to_string())
}

/// One row of the corpus listing.
pub struct Listing {
    pub name: String,
    pub sources: usize,
    pub bytes: usize,
    pub tokens: usize,
    /// The guide's title.
    pub title: Option<String>,
}

/// The corpora of the library with their totals, sorted by name.
pub fn listing(root: &Path) -> std::io::Result<Vec<Listing>> {
    let mut out = Vec::new();
    for (name, dir) in corpus_dirs(root)? {
        let sources = read_sources(&dir)?;
        let title = std::fs::read(dir.join("guide.md"))
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|text| read_guide(&text).title);
        out.push(Listing {
            name,
            sources: sources.len(),
            bytes: sources.iter().map(|s| s.body().len()).sum(),
            tokens: sources.iter().map(|s| source::tokens(s.body().len())).sum(),
            title,
        });
    }
    Ok(out)
}

/// What `<root>/library/` holds that `check` reports.
#[derive(Default)]
pub struct Found {
    /// `(path relative to the root, message)`.
    pub problems: Vec<(String, String)>,
    /// `(id, path)` of every file with a valid id.
    pub ids: Vec<(String, String)>,
}

impl Found {
    fn add(&mut self, path: &str, message: impl Into<String>) {
        self.problems.push((path.to_string(), message.into()));
    }
}

/// Every library problem and every file's id; nothing when `<root>/library/` is missing.
pub fn problems(root: &Path) -> Found {
    let mut found = Found::default();
    let library = store::library_dir(root);
    if !library.is_dir() {
        return found;
    }
    let entries = match store::entries(&library) {
        Ok(entries) => entries,
        Err(e) => {
            found.add("library", format!("read: {e}"));
            return found;
        }
    };
    for entry in &entries {
        let path = format!("library/{}", entry.name);
        if !entry.utf8 {
            found.add(&path, "name: not valid UTF-8");
            continue;
        }
        match &entry.kind {
            EntryKind::Folder if !is_topic(&entry.name) => found.add(
                &path,
                format!("corpus: invalid name '{}': {NAME_RULE}", entry.name),
            ),
            EntryKind::Folder if RESERVED.contains(&entry.name.as_str()) => found.add(
                &path,
                format!(
                    "corpus: '{}' is reserved for a library subcommand",
                    entry.name
                ),
            ),
            EntryKind::Folder => corpus(&mut found, entry, &path),
            EntryKind::Unreadable(e) => found.add(&path, format!("read: {e}")),
            EntryKind::File | EntryKind::Other => {
                found.add(&path, "entry: library/ holds only corpus folders")
            }
        }
    }
    found
}

fn corpus(found: &mut Found, corpus: &Entry, dir: &str) {
    let entries = match store::entries(&corpus.path) {
        Ok(entries) => entries,
        Err(e) => return found.add(dir, format!("read: {e}")),
    };
    let mut guide: Option<Option<Guide>> = None;
    let mut sources: Vec<String> = Vec::new();
    for entry in &entries {
        let path = format!("{dir}/{}", entry.name);
        if !entry.utf8 {
            found.add(&path, "name: not valid UTF-8");
            continue;
        }
        match &entry.kind {
            EntryKind::Folder => {
                found.add(&path, "folder: a corpus holds only guide.md and sources");
                continue;
            }
            EntryKind::Other => {
                found.add(&path, "entry: not a regular file");
                continue;
            }
            EntryKind::Unreadable(e) => found.add(&path, format!("read: {e}")),
            EntryKind::File => {}
        }
        if entry.name == "guide.md" {
            guide = Some(match &entry.kind {
                EntryKind::File => read_text(found, &entry.path, &path).map(|t| {
                    let guide = read_guide(&t);
                    report(found, &path, &guide.problems, guide.id.as_deref());
                    guide
                }),
                _ => None,
            });
        } else if let Some(name) = source_stem(entry) {
            sources.push(name);
            if entry.kind == EntryKind::File
                && let Some(text) = read_text(found, &entry.path, &path)
            {
                let read = source::read(&text);
                report(found, &path, &read.problems, read.id.as_deref());
            }
        } else {
            found.add(
                &path,
                "name: must be <name>.md, with segments of a-z and 0-9 joined by single hyphens",
            );
        }
    }

    let guide_path = format!("{dir}/guide.md");
    match guide {
        None => found.add(&guide_path, "guide: missing; every corpus needs one"),
        Some(None) => {}
        Some(Some(guide)) => {
            for name in &sources {
                if !guide.entries.iter().any(|e| &e.name == name) {
                    found.add(
                        &format!("{dir}/{name}.md"),
                        format!("guide: no '## {name}' entry in guide.md"),
                    );
                }
            }
            for entry in &guide.entries {
                if !sources.contains(&entry.name) {
                    let problem = Problem::at(
                        entry.line,
                        format!("entry '{0}': no source {0}.md in this corpus", entry.name),
                    );
                    found.add(&guide_path, problem.to_string());
                }
            }
        }
    }
}

fn read_text(found: &mut Found, real: &Path, path: &str) -> Option<String> {
    match std::fs::read(real) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                found.add(path, "encoding: not valid UTF-8");
                None
            }
        },
        Err(e) => {
            found.add(path, format!("read: {e}"));
            None
        }
    }
}

fn report(found: &mut Found, path: &str, problems: &[Problem], id: Option<&str>) {
    for problem in problems {
        found.add(path, problem.to_string());
    }
    if let Some(id) = id {
        found.ids.push((id.to_string(), path.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const GID: &str = "01M3EZ8NBEVNHZRTQ6T60171J2";
    const SID: &str = "01M3EZ8NVEC2KJQNGK5DTK349R";
    const CREATED: &str = "2026-09-26T13:16-03:00";

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("bilbo-corpus-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn put(&self, path: &str, text: &str) {
            let real = self.0.join(path);
            std::fs::create_dir_all(real.parent().unwrap()).unwrap();
            std::fs::write(real, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn guide(entries: &[&str]) -> String {
        let mut text =
            format!("---\nid: {GID}\ncreated: {CREATED}\n---\n\n# Go\n\nWhat it grounds.\n");
        for entry in entries {
            text.push_str(&format!("\n## {entry}\n\nProse.\n"));
        }
        text
    }

    fn source(id: &str, title: &str) -> String {
        let body = format!("# {title}\n\ntext\n");
        let front = source::Frontmatter {
            id: id.into(),
            fetched: "2026-08-23".into(),
            origin: "url: https://go.dev".into(),
            digest: source::digest(&body),
            kept: None,
            capture: None,
        };
        source::render(&front, &body)
    }

    fn messages(text: &str) -> Vec<String> {
        read_guide(text)
            .problems
            .iter()
            .map(|p| p.to_string())
            .collect()
    }

    fn found(scratch: &Scratch) -> Vec<String> {
        let mut out = problems(&scratch.0).problems;
        out.sort();
        out.into_iter().map(|(p, m)| format!("{p}: {m}")).collect()
    }

    #[test]
    fn names() {
        for good in ["go", "software-architecture", "a1-b2"] {
            assert!(is_corpus_name(good) && is_source_name(good), "{good}");
        }
        for bad in ["Go", "go--style", "-go", "go_x", "", "go.md"] {
            assert!(!is_corpus_name(bad) && !is_source_name(bad), "{bad}");
        }
        for word in ["show", "stage", "land", "plan", "read"] {
            assert!(!is_corpus_name(word), "{word}");
            assert!(is_source_name(word), "{word}");
        }
        assert!(is_corpus_name("guide") && !is_source_name("guide"));
        assert!(is_source_name("action-domain-responder"));
    }

    #[test]
    fn a_valid_guide() {
        let text = guide(&["effective-go", "uber-go-style-guide"]);
        let read = read_guide(&text);
        assert!(read.problems.is_empty());
        assert_eq!(read.id.as_deref(), Some(GID));
        assert_eq!(read.created.as_deref(), Some(CREATED));
        assert_eq!(read.title.as_deref(), Some("Go"));
        assert_eq!(
            read.lead
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            ["", "What it grounds.", ""]
        );
        assert_eq!(read.lead[1].number, 8);
        let names: Vec<_> = read
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.line))
            .collect();
        assert_eq!(names, [("effective-go", 10), ("uber-go-style-guide", 14)]);
        assert_eq!(read.entries[0].prose[1].text, "Prose.");
        assert_eq!(read.entries[0].prose[1].number, 12);
    }

    #[test]
    fn a_guide_with_a_sources_list() {
        let text = guide(&[]).replace(
            &format!("created: {CREATED}\n"),
            &format!("created: {CREATED}\nsources:\n  - \"url: https://go.dev\"\n"),
        );
        let found = messages(&text);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("unknown key 'sources'"), "{found:?}");
    }

    #[test]
    fn frontmatter_and_title_follow_the_note_rules() {
        let found = messages("---\nid: nope\n---\n\n# A\n\n# B\n");
        assert!(
            found[0].starts_with("id: 'nope' is not a canonical ULID"),
            "{found:?}"
        );
        assert!(found[1].starts_with("title: found 2"), "{found:?}");
        assert_eq!(found[2], "created: missing");
        assert_eq!(
            messages("# Go\n"),
            ["frontmatter: missing; line 1 must be '---' (line 1)"]
        );
        let no_title = guide(&[]).replace("# Go\n", "");
        assert!(messages(&no_title)[0].starts_with("title: missing"));
        let bad = guide(&[]).replace(CREATED, "2026-09-26");
        assert!(messages(&bad)[0].starts_with("created: '2026-09-26'"));
    }

    #[test]
    fn every_level_two_heading_outside_fences_is_an_entry() {
        let text = guide(&["a"]).replace("Prose.", "```md\n## x\n```\n\n### Inner");
        let read = read_guide(&text);
        assert_eq!(read.entries.len(), 1);
        assert_eq!(
            read.entries[0]
                .prose
                .iter()
                .filter(|l| l.text == "## x")
                .count(),
            1
        );
        assert!(read.problems.is_empty());
        let text = guide(&["a", "Reading order"]);
        let names: Vec<_> = read_guide(&text)
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["a", "Reading order"]);
    }

    #[test]
    fn a_repeated_entry_is_reported_on_the_second_heading() {
        let found = messages(&guide(&["effective-go", "effective-go"]));
        assert_eq!(
            found,
            ["entry 'effective-go': given more than once (line 14)"]
        );
    }

    #[test]
    fn stub_and_stale_lines() {
        let stub = guide(&["effective-go"]).replace("Prose.", STUB_SOURCE);
        assert_eq!(
            messages(&stub),
            ["entry 'effective-go': TODO stub; write the entry and remove the TODO line (line 12)"]
        );
        let stale = guide(&["effective-go"]).replace(
            "Prose.",
            "stale: re-ingested 2026-10-03; re-read the source and revise this entry.",
        );
        assert_eq!(
            messages(&stale),
            [
                "entry 'effective-go': stale; re-read the source, revise the entry and remove the stale line (line 12)"
            ]
        );
        let lead = guide(&[]).replace("What it grounds.", STUB_CORPUS);
        assert_eq!(
            messages(&lead),
            ["lead: TODO stub; describe the corpus and remove the TODO line (line 8)"]
        );
        let lead = guide(&[]).replace("What it grounds.", "stale: re-ingested 2026-10-03; x");
        assert_eq!(messages(&lead), ["lead: stale line; remove it (line 8)"]);
        assert!(
            messages(&guide(&["effective-go"]).replace("Prose.", "Two sentences. Revised."))
                .is_empty()
        );
    }

    #[test]
    fn a_new_guide_is_a_valid_stub() {
        let text = new_guide(GID, CREATED, "software-architecture");
        let read = read_guide(&text);
        assert_eq!(read.title.as_deref(), Some("software-architecture"));
        assert_eq!(messages(&text).len(), 1);
        assert!(messages(&text)[0].starts_with("lead: TODO stub"));
        assert!(text.ends_with(&format!("{STUB_CORPUS}\n")));
    }

    #[test]
    fn add_entry_appends_a_stub() {
        let text = add_entry(&guide(&["a"]), "b");
        let read = read_guide(&text);
        assert_eq!(read.entries.len(), 2);
        assert_eq!(read.entries[1].name, "b");
        assert!(text.ends_with(&format!("\n## b\n\n{STUB_SOURCE}\n")));
        assert!(text.contains("Prose.\n\n## b"));
    }

    #[test]
    fn add_entry_on_a_guide_without_a_final_newline() {
        let base = guide(&["a"]);
        let bare = base.trim_end_matches('\n');
        assert!(!bare.ends_with('\n'));
        assert_eq!(add_entry(bare, "b"), add_entry(&base, "b"));
    }

    #[test]
    fn mark_stale_puts_the_line_under_the_heading() {
        let stale = "stale: re-ingested 2026-10-03; re-read the source and revise this entry.";
        let text = mark_stale(&guide(&["a", "b"]), "a", "2026-10-03");
        let read = read_guide(&text);
        assert_eq!(read.entries[0].prose[0].text, stale);
        assert_eq!(read.entries[0].prose[2].text, "Prose.");
        assert_eq!(read.entries[1].prose.len(), 2);
        assert_eq!(text.matches(stale).count(), 1);
    }

    #[test]
    fn a_stale_line_written_twice_is_replaced() {
        let once = mark_stale(&guide(&["a", "b"]), "a", "2026-10-03");
        let twice = mark_stale(&once, "a", "2026-10-09");
        assert_eq!(twice.matches("stale: re-ingested ").count(), 1);
        assert!(twice.contains("stale: re-ingested 2026-10-09;"));
        assert!(!twice.contains("2026-10-03"));
        assert_eq!(read_guide(&twice).entries[0].prose[2].text, "Prose.");
    }

    #[test]
    fn mark_stale_keeps_everything_else_byte_for_byte() {
        let base = guide(&["a", "b"]);
        let marked = mark_stale(&base, "b", "2026-10-03");
        assert_eq!(
            marked.replace(
                "stale: re-ingested 2026-10-03; re-read the source and revise this entry.\n",
                ""
            ),
            base
        );
        let bare = base.trim_end_matches('\n').to_string();
        let marked = mark_stale(&bare, "b", "2026-10-03");
        assert!(!marked.ends_with('\n') && marked.contains("## b\nstale:"));
        assert_eq!(mark_stale(&base, "missing", "2026-10-03"), base);
    }

    #[test]
    fn mark_stale_ignores_a_fenced_heading() {
        let text = guide(&["a"]).replace("Prose.", "```\n## b\n```");
        assert_eq!(mark_stale(&text, "b", "2026-10-03"), text);
    }

    fn valid_library() -> Scratch {
        let scratch = Scratch::new("valid");
        scratch.put(
            "library/go/guide.md",
            &guide(&["effective-go", "inspecting-errors"]),
        );
        scratch.put("library/go/effective-go.md", &source(SID, "Effective Go"));
        scratch.put(
            "library/go/inspecting-errors.md",
            &source("01M3EZ8NVEC2KJQNGK5DTK3500", "Inspecting errors"),
        );
        scratch
    }

    #[test]
    fn a_clean_library() {
        let scratch = valid_library();
        assert_eq!(found(&scratch), Vec::<String>::new());
        let ids = problems(&scratch.0).ids;
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&(GID.to_string(), "library/go/guide.md".to_string())));
    }

    #[test]
    fn no_library_is_no_problem() {
        let scratch = Scratch::new("none");
        assert!(problems(&scratch.0).problems.is_empty());
    }

    #[test]
    fn hidden_entries_are_ignored() {
        let scratch = valid_library();
        scratch.put("library/.lock", "");
        scratch.put("library/go/.DS_Store", "x");
        scratch.put("library/.hidden/anything.txt", "x");
        assert!(found(&scratch).is_empty());
    }

    #[test]
    fn layout_problems() {
        let scratch = valid_library();
        scratch.put("library/effective-go.md", "loose");
        scratch.put("library/go/effective-go/page.md", "x");
        scratch.put("library/go/Effective_Go.md", "x");
        scratch.put("library/go/effective-go.txt", "x");
        scratch.put("library/Zed/guide.md", "x");
        scratch.put("library/go--style/guide.md", "x");
        scratch.put("library/plan/guide.md", "x");
        assert_eq!(
            found(&scratch),
            [
                "library/Zed: corpus: invalid name 'Zed': use segments of a-z and 0-9 joined by single hyphens",
                "library/effective-go.md: entry: library/ holds only corpus folders",
                "library/go--style: corpus: invalid name 'go--style': use segments of a-z and 0-9 joined by single hyphens",
                "library/go/Effective_Go.md: name: must be <name>.md, with segments of a-z and 0-9 joined by single hyphens",
                "library/go/effective-go: folder: a corpus holds only guide.md and sources",
                "library/go/effective-go.txt: name: must be <name>.md, with segments of a-z and 0-9 joined by single hyphens",
                "library/plan: corpus: 'plan' is reserved for a library subcommand",
            ]
        );
    }

    #[test]
    fn a_missing_guide() {
        let scratch = Scratch::new("no-guide");
        scratch.put("library/go/effective-go.md", &source(SID, "Effective Go"));
        assert_eq!(
            found(&scratch),
            ["library/go/guide.md: guide: missing; every corpus needs one"]
        );
    }

    #[test]
    fn a_source_with_no_entry_and_an_entry_with_no_source() {
        let scratch = valid_library();
        scratch.put(
            "library/go/guide.md",
            &guide(&["effective-go", "Reading order"]),
        );
        assert_eq!(
            found(&scratch),
            [
                "library/go/guide.md: entry 'Reading order': no source Reading order.md in this corpus (line 14)",
                "library/go/inspecting-errors.md: guide: no '## inspecting-errors' entry in guide.md",
            ]
        );
    }

    #[test]
    fn source_and_guide_problems_carry_their_paths() {
        let scratch = valid_library();
        let text = source(SID, "Effective Go").replace("text", "tent");
        scratch.put("library/go/effective-go.md", &text);
        scratch.put(
            "library/go/guide.md",
            &guide(&["effective-go", "inspecting-errors"]).replace("Prose.", STUB_SOURCE),
        );
        let lines = found(&scratch);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].starts_with("library/go/effective-go.md: digest: does not match the body"),
            "{lines:?}"
        );
        assert!(
            lines[1].starts_with("library/go/guide.md: entry 'effective-go': TODO stub"),
            "{lines:?}"
        );
        assert!(
            lines[2].starts_with("library/go/guide.md: entry 'inspecting-errors': TODO stub"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_file_that_is_not_utf8_is_reported() {
        let scratch = valid_library();
        std::fs::write(scratch.0.join("library/go/effective-go.md"), [0xff, 0xfe]).unwrap();
        assert_eq!(
            found(&scratch),
            ["library/go/effective-go.md: encoding: not valid UTF-8"]
        );
    }

    #[test]
    fn the_listing_sums_sources() {
        let scratch = valid_library();
        scratch.put(
            "library/rust/guide.md",
            &guide(&[]).replace("# Go", "# Rust"),
        );
        scratch.put("library/Bad/x.md", "x");
        let rows = listing(&scratch.0).unwrap();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["go", "rust"]);
        assert_eq!(rows[0].sources, 2);
        let body = "# Effective Go\n\ntext\n".len() + "# Inspecting errors\n\ntext\n".len();
        assert_eq!(rows[0].bytes, body);
        assert_eq!(rows[0].tokens, source::tokens(22) + source::tokens(26));
        assert_eq!(rows[0].title.as_deref(), Some("Go"));
        assert_eq!((rows[1].sources, rows[1].bytes), (0, 0));
    }

    #[test]
    fn the_listing_of_a_missing_library_is_empty() {
        assert!(listing(&Scratch::new("empty").0).unwrap().is_empty());
    }
}
