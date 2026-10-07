//! `bilbo check`: every problem in the store's notes and library, one line each; it changes nothing.

use std::collections::BTreeMap;
use std::path::Path;

use crate::host::terminal;
use crate::library::corpus;
use crate::note::{conflicts, marks, versions};
use crate::shared::config::{self, Settings};
use crate::shared::store::{self, Entry, EntryKind};
use crate::{Failure, note};

/// Problems as `(path, message)`; a key shared by files is kept as `(key, path)`.
struct Scan<'a> {
    settings: &'a Settings,
    root: &'a Path,
    summary: conflicts::Summary,
    now: jiff::Timestamp,
    found: Vec<(String, String)>,
    warnings: Vec<(String, String)>,
    topics: Vec<(String, String)>,
    ids: Vec<(String, String)>,
}

/// What `check` prints, and whether any of it is a problem.
pub struct Output {
    pub lines: Vec<String>,
    pub failed: bool,
    /// Each problem and warning as `(path, message, warning)`, in the order of `lines`; a warning's
    /// message keeps its ` (warning)` suffix.
    pub problems: Vec<(String, String, bool)>,
    /// Entries checked in `<root>/notes/`, and corpus folders in `<root>/library/`.
    pub notes: usize,
    pub corpora: usize,
}

/// Problem lines "<path relative to root>: <message>", sorted; no lines means a clean store.
pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    if let Some(arg) = args.first() {
        return Err(Failure::Usage(if arg.starts_with('-') && arg != "-" {
            format!("unknown option '{arg}'")
        } else {
            format!("unexpected argument '{arg}'")
        }));
    }
    let settings = config::load(env).map_err(Failure::Config)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() && !store::library_dir(&root).is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let mut scan = Scan {
        settings: &settings,
        root: &root,
        summary: conflicts::Summary::default(),
        now: jiff::Timestamp::now(),
        found: Vec::new(),
        warnings: Vec::new(),
        topics: Vec::new(),
        ids: Vec::new(),
    };
    match conflicts::read(&root) {
        Ok(summary) => scan.summary = summary,
        Err(e) => scan.add(".bilbo/sync/open.json", format!("read: {e}")),
    }
    let mut checked = 0;
    if notes.is_dir() {
        let entries = store::entries(&notes)
            .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;
        entries.iter().for_each(|entry| scan.entry(entry));
        checked = entries.len();
    }
    let library = corpus::problems(&root);
    scan.found.extend(library.problems);
    scan.ids.extend(library.ids);
    scan.found.extend(shared(&scan.topics, |key, others| {
        format!("topic: '{key}' is also the topic of {others}")
    }));
    scan.found.extend(shared(&scan.ids, |key, others| {
        format!("id: {key} is also the id of {others}")
    }));

    let failed = !scan.found.is_empty();
    let mut all: Vec<(String, String, bool)> = scan
        .found
        .into_iter()
        .map(|(path, message)| (path, message, false))
        .collect();
    all.extend(
        scan.warnings
            .into_iter()
            .map(|(path, message)| (path, message, true)),
    );
    all.sort();
    let problems: Vec<(String, String, bool)> = all
        .into_iter()
        .map(|(path, message, warning)| (single_line(&path), single_line(&message), warning))
        .collect();
    let lines = problems
        .iter()
        .map(|(path, message, _)| format!("{path}: {message}"))
        .collect();
    let corpora = corpus::corpus_dirs(&root).map_or(0, |c| c.len());
    Ok(Output {
        lines,
        failed,
        problems,
        notes: checked,
        corpora,
    })
}

impl Output {
    /// The `store-check` human view; `lines` is the plain one.
    pub fn view(&self, term: &terminal::Term) -> Vec<String> {
        use terminal::{Mark, Tone};
        if self.problems.is_empty() {
            return vec![format!(
                "{}  No problems in {} and {}",
                terminal::mark(term, Mark::Done),
                terminal::paint(
                    term,
                    Tone::Bold,
                    &terminal::count(self.notes as u64, "note", "notes")
                ),
                terminal::paint(
                    term,
                    Tone::Bold,
                    &terminal::count(self.corpora as u64, "corpus", "corpora")
                ),
            )];
        }
        let mut out = Vec::new();
        let mut files = 0;
        let mut rest = self.problems.as_slice();
        while let Some((path, _, _)) = rest.first() {
            let end = rest.iter().take_while(|(p, _, _)| p == path).count();
            let (file, after) = rest.split_at(end);
            rest = after;
            if files > 0 {
                out.push(String::new());
            }
            files += 1;
            out.push(terminal::paint(term, Tone::Bold, path));
            out.extend(file_lines(term, file));
        }
        let warnings = self.problems.iter().filter(|(_, _, w)| *w).count() as u64;
        let problems = self.problems.len() as u64 - warnings;
        let files = terminal::count(files, "file", "files");
        let bold = |n: u64, one: &str, many: &str| {
            terminal::paint(term, Tone::Bold, &terminal::count(n, one, many))
        };
        out.push(String::new());
        out.push(match (problems, warnings) {
            (0, w) => format!(
                "{}  {} in {files}",
                terminal::mark(term, Mark::Warning),
                bold(w, "warning", "warnings")
            ),
            (p, 0) => format!(
                "{}  {} in {files}",
                terminal::mark(term, Mark::Error),
                bold(p, "problem", "problems")
            ),
            (p, w) => format!(
                "{}  {}, {} in {files}",
                terminal::mark(term, Mark::Error),
                bold(p, "problem", "problems"),
                bold(w, "warning", "warnings")
            ),
        });
        out
    }
}

/// The lines of one file's problems: the mark, the head dim in a column, the rest wrapped.
fn file_lines(term: &terminal::Term, file: &[(String, String, bool)]) -> Vec<String> {
    use terminal::{Align, Mark, Tone};
    let messages: Vec<(String, bool)> = file
        .iter()
        .map(|(_, message, warning)| {
            let message = message.strip_suffix(" (warning)").unwrap_or(message);
            (terminal::tilde_text(term, message), *warning)
        })
        .collect();
    let split: Vec<(&str, &str, bool)> = messages
        .iter()
        .map(|(message, warning)| {
            let (head, rest) = message.split_once(": ").unwrap_or(("", message));
            (head, rest, *warning)
        })
        .collect();
    let head_w = split
        .iter()
        .map(|(head, _, _)| terminal::width_of(head))
        .max()
        .unwrap_or(0);
    let indent = match head_w {
        0 => 5,
        w => w + 7,
    };
    let width = term.width.saturating_sub(indent).max(1);
    let mut out = Vec::new();
    for (head, rest, warning) in split {
        let lead = terminal::mark(term, if warning { Mark::Warning } else { Mark::Error });
        let column = match head_w {
            0 => String::new(),
            w => format!(
                "{}  ",
                terminal::pad(&terminal::paint(term, Tone::Dim, head), w, Align::Left)
            ),
        };
        for (i, line) in terminal::wrap(term, rest, width, terminal::Long::Split)
            .into_iter()
            .enumerate()
        {
            out.push(match i {
                0 => format!("  {lead}  {column}{line}"),
                _ => format!("{}{line}", " ".repeat(indent)),
            });
        }
    }
    out
}

impl Scan<'_> {
    fn add(&mut self, path: &str, message: impl Into<String>) {
        self.found.push((path.to_string(), message.into()));
    }

    fn entry(&mut self, entry: &Entry) {
        let path = format!("notes/{}", entry.name);
        if !entry.utf8 {
            return self.add(&path, "name: not valid UTF-8");
        }
        match &entry.kind {
            EntryKind::Folder => self.add(&path, "folder: notes/ holds only note files"),
            EntryKind::Other => self.add(&path, "entry: not a regular file"),
            EntryKind::Unreadable(e) => {
                self.add(&path, format!("read: {e}"));
                self.name(&entry.name, &path);
            }
            EntryKind::File => self.file(&entry.name, &entry.path, &path),
        }
    }

    fn name(&mut self, name: &str, path: &str) {
        match note::parse_name(name) {
            Ok(parsed) => self.topics.push((parsed.topic, path.to_string())),
            Err(message) => self.add(path, message),
        }
    }

    fn file(&mut self, name: &str, real: &Path, path: &str) {
        self.name(name, path);
        if !name.ends_with(".md") {
            return;
        }
        let bytes = match std::fs::read(real) {
            Ok(bytes) => bytes,
            Err(e) => return self.add(path, format!("read: {e}")),
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return self.add(path, "encoding: not valid UTF-8");
        };
        let read = note::read(&text);
        for problem in &read.problems {
            self.add(path, problem.to_string());
        }
        if let Some(id) = &read.id {
            self.ids.push((id.clone(), path.to_string()));
        }
        self.scope(&read.scope, name, &text, path);
        self.sync(&read, name, &text, path);
    }

    /// The Sync conflicts and A note that left its scope rules for one note.
    fn sync(&mut self, read: &note::Note, name: &str, text: &str, path: &str) {
        let entry = read
            .id
            .as_ref()
            .and_then(|id| self.summary.notes.get(id))
            .cloned();
        let marked = ["<<<<<<< bilbo", "======= bilbo", ">>>>>>> bilbo"]
            .iter()
            .any(|marker| text.contains(marker));
        if entry.is_none() && !marked {
            return;
        }
        let log = match (&entry, &read.id) {
            (Some(_), Some(id)) => versions::load(self.root, id).unwrap_or_else(|e| {
                self.add(path, format!("history: read: {e}"));
                versions::Log::default()
            }),
            _ => versions::Log::default(),
        };
        let judged = conflicts::judge(self.root, entry.as_ref(), &log, text);
        for open in &judged.conflicts {
            self.add(
                path,
                format!(
                    "conflict: '{}' holds {} sides; keep what is right, remove the markers",
                    open.passage, open.sides
                ),
            );
        }
        let topic = note::parse_name(name)
            .map(|n| n.topic)
            .unwrap_or_else(|_| name.trim_end_matches(".md").to_string());
        let mut passages: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for lost in &judged.dropped {
            let lines = passages.entry(&lost.passage).or_default();
            for line in &lost.lines {
                if !lines.contains(&line.as_str()) {
                    lines.push(line);
                }
            }
        }
        for (passage, lines) in passages {
            let first: String = lines[0].chars().take(80).collect();
            self.add(
                path,
                format!(
                    "conflict: dropped {} lines of '{passage}', first \"{first}\"; restore them or run bilbo sync declare {topic} \"<why>\"",
                    lines.len()
                ),
            );
        }
        for line in &judged.stray {
            self.add(path, format!("line {line}: stray conflict marker"));
        }
        let current = match &read.scope {
            note::ScopeKey::Valid(own) => Some(own.as_str()),
            _ => None,
        };
        for scope in entry
            .iter()
            .flat_map(|e| conflicts::left_scopes(e, current, self.now))
        {
            self.warnings.push((
                path.to_string(),
                format!(
                    "scope: left '{scope}'; other devices of '{scope}' no longer hold this note"
                ),
            ));
        }
    }

    /// The Scope problems and Scope marks rules for one note.
    fn scope(&mut self, key: &note::ScopeKey, name: &str, text: &str, path: &str) {
        let settings = self.settings;
        let topic = note::parse_name(name).map(|n| n.topic).unwrap_or_default();
        let names = settings.scope_names();
        let own = match key {
            note::ScopeKey::Invalid => return,
            note::ScopeKey::Absent if names.is_empty() => return,
            note::ScopeKey::Absent => {
                return self.unassigned(path, "scope: missing".into(), &topic, text);
            }
            note::ScopeKey::Valid(own) => own,
        };
        if settings.scope(own).is_none() {
            let message = format!(
                "scope: '{own}' is not declared in {}",
                settings.shown_path()
            );
            return self.unassigned(path, message, &topic, text);
        }
        for other in settings.scopes.iter().filter(|s| s.name != *own) {
            if let Some((place, mark)) = marks::first_mark(other, &topic, text) {
                self.warnings.push((
                    path.to_string(),
                    format!(
                        "scope: '{own}' but {place} holds '{mark}', a mark of '{}' (warning)",
                        other.name
                    ),
                ));
            }
        }
    }

    /// A problem for a note with no declared scope, ending with the scopes whose marks it holds.
    fn unassigned(&mut self, path: &str, message: String, topic: &str, text: &str) {
        let names = self.settings.scope_names();
        let listed = if names.is_empty() {
            String::new()
        } else {
            format!("; scopes: {}", names.join(", "))
        };
        let holds: Vec<&str> = self
            .settings
            .scopes
            .iter()
            .filter(|s| marks::first_mark(s, topic, text).is_some())
            .map(|s| s.name.as_str())
            .collect();
        let suffix = if holds.is_empty() {
            String::new()
        } else {
            format!("; holds marks of {}", holds.join(", "))
        };
        self.add(path, format!("{message}{listed}{suffix}"));
    }
}

/// One problem per file that shares its key with another file, naming every other file.
fn shared(
    items: &[(String, String)],
    message: impl Fn(&str, &str) -> String,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, path) in items {
        let mut others: Vec<&str> = items
            .iter()
            .filter(|(k, p)| k == key && p != path)
            .map(|(_, p)| p.as_str())
            .collect();
        if !others.is_empty() {
            others.sort();
            out.push((path.clone(), message(key, &others.join(", "))));
        }
    }
    out
}

/// File names can hold newlines; a problem must stay one line.
fn single_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// `bilbo check --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo check: print every problem in the notes and the library; it changes
nothing.

Usage:
  bilbo check

check reads every note and source against the store's rules, and each note's
scope against the config, and reports every problem it finds in one run.

Output: one line per problem or warning, sorted by path, then by message:
  <path relative to the root>: <message>
A warning does not change the exit code.

Exit: 0 no problem; 1 a problem, or no store; 2 usage or config error.

Examples:
  bilbo check

Docs: https://github.com/delucca/bilbo/wiki/Commands#check
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::terminal::{fixed, plain, styled};

    fn output(rows: &[(&str, &str, bool)], notes: usize, corpora: usize) -> Output {
        let problems: Vec<(String, String, bool)> = rows
            .iter()
            .map(|(p, m, w)| (p.to_string(), m.to_string(), *w))
            .collect();
        Output {
            lines: problems
                .iter()
                .map(|(p, m, _)| format!("{p}: {m}"))
                .collect(),
            failed: rows.iter().any(|(_, _, w)| !w),
            problems,
            notes,
            corpora,
        }
    }

    const DUP_A: &str = "id: 01M3YJ7R6HK6NQ30DCDB1P4DYB is also the id of notes/gotcha-dup-id.md";
    const DUP_B: &str =
        "id: 01M3YJ7R6HK6NQ30DCDB1P4DYB is also the id of notes/decision-bilbo-note-store.md";
    const SCOPE: &str =
        "scope: 'nosuch' is not declared in /home/a/.config/bilbo/config; scopes: personal, work";
    const MARK: &str = "scope: 'personal' but the title holds 'acme', a mark of 'work' (warning)";
    const ULID: &str = "id: 'nope' is not a canonical ULID: 26 characters of 0-9 and A-Z without I, L, O, U, the first 0-7 (line 2)";

    fn mixed() -> Output {
        output(
            &[
                ("notes/decision-bilbo-note-store.md", DUP_A, false),
                ("notes/gotcha-dup-id.md", DUP_B, false),
                (
                    "notes/gotcha-dup-id.md",
                    "line 13: stray conflict marker",
                    false,
                ),
                ("notes/gotcha-dup-id.md", SCOPE, false),
                ("notes/plan-a.md", MARK, true),
                ("notes/plan-broken.md", "created: missing", false),
                ("notes/plan-broken.md", ULID, false),
            ],
            9,
            2,
        )
    }

    #[test]
    fn the_view_groups_by_file() {
        let want = "{b}notes/decision-bilbo-note-store.md{/b}
  {r}■{/r}  {d}id{/d}  01M3YJ7R6HK6NQ30DCDB1P4DYB is also the id of notes/gotcha-dup-id.md

{b}notes/gotcha-dup-id.md{/b}
  {r}■{/r}  {d}id{/d}       01M3YJ7R6HK6NQ30DCDB1P4DYB is also the id of notes/decision-bilbo-note-store.md
  {r}■{/r}  {d}line 13{/d}  stray conflict marker
  {r}■{/r}  {d}scope{/d}    'nosuch' is not declared in ~/.config/bilbo/config; scopes: personal, work

{b}notes/plan-a.md{/b}
  {y}▲{/y}  {d}scope{/d}  'personal' but the title holds 'acme', a mark of 'work'

{b}notes/plan-broken.md{/b}
  {r}■{/r}  {d}created{/d}  missing
  {r}■{/r}  {d}id{/d}       'nope' is not a canonical ULID: 26 characters of 0-9 and A-Z without I, L, O, U, the
              first 0-7 (line 2)

{r}■{/r}  {b}6 problems{/b}, {b}1 warning{/b} in 4 files";
        let painted = mixed().view(&fixed(100, true, true));
        assert_eq!(painted, styled(want).lines().collect::<Vec<_>>());
        assert_eq!(
            mixed().view(&fixed(100, false, true)),
            plain(want).lines().collect::<Vec<_>>()
        );
    }

    #[test]
    fn only_warnings_say_so() {
        let one = output(&[("notes/plan-a.md", MARK, true)], 3, 0);
        let view = one.view(&fixed(100, true, true));
        assert_eq!(
            view.last().unwrap(),
            &styled("{y}▲{/y}  {b}1 warning{/b} in 1 file")
        );
    }

    #[test]
    fn a_clean_store_says_so() {
        let clean = |notes, corpora| output(&[], notes, corpora);
        assert_eq!(
            clean(12, 2).view(&fixed(100, true, true)),
            [styled(
                "{g}◆{/g}  No problems in {b}12 notes{/b} and {b}2 corpora{/b}"
            )]
        );
        assert_eq!(
            clean(1, 1).view(&fixed(100, false, true)),
            ["◆  No problems in 1 note and 1 corpus"]
        );
        assert_eq!(
            clean(12, 2).view(&fixed(100, false, false)),
            ["*  No problems in 12 notes and 2 corpora"]
        );
    }

    #[test]
    fn a_message_without_a_head() {
        let one = output(&[("notes/x.md", "stray text", false)], 1, 0);
        assert_eq!(
            one.view(&fixed(100, false, true))[..2],
            ["notes/x.md", "  ■  stray text"]
        );
    }
}
