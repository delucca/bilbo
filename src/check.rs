use std::path::Path;

use crate::store::{self, Entry, EntryKind};
use crate::{Failure, corpus, note};

/// Problems as `(path, message)`; a key shared by files is kept as `(key, path)`.
#[derive(Default)]
struct Scan {
    found: Vec<(String, String)>,
    topics: Vec<(String, String)>,
    ids: Vec<(String, String)>,
}

/// Problem lines "<path relative to root>: <message>", sorted; empty means a clean store.
pub fn run(args: &[String], env: &store::Env) -> Result<Vec<String>, Failure> {
    if let Some(arg) = args.first() {
        return Err(Failure::Usage(if arg.starts_with('-') && arg != "-" {
            format!("unknown option '{arg}'")
        } else {
            format!("unexpected argument '{arg}'")
        }));
    }
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() && !store::library_dir(&root).is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let mut scan = Scan::default();
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

    scan.found.sort();
    Ok(scan
        .found
        .into_iter()
        .map(|(path, message)| single_line(&format!("{path}: {message}")))
        .collect())
}

impl Scan {
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
        if let Some(id) = read.id {
            self.ids.push((id, path.to_string()));
        }
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
