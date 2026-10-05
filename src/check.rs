use std::collections::BTreeMap;
use std::path::Path;

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
    if notes.is_dir() {
        let entries = store::entries(&notes)
            .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;
        entries.iter().for_each(|entry| scan.entry(entry));
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
    let mut all = scan.found;
    all.extend(scan.warnings);
    all.sort();
    let lines = all
        .into_iter()
        .map(|(path, message)| single_line(&format!("{path}: {message}")))
        .collect();
    Ok(Output { lines, failed })
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
