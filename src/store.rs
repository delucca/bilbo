use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::library::corpus;
use crate::markdown::fence_run;
use crate::{markdown, note, rank};

/// The environment variables root, config and cache resolution read; tests build it by hand.
pub struct Env {
    pub bilbo_home: Option<OsString>,
    pub xdg_data_home: Option<OsString>,
    pub home: Option<OsString>,
    pub bilbo_config: Option<OsString>,
    pub xdg_config_home: Option<OsString>,
    pub xdg_cache_home: Option<OsString>,
    pub xdg_state_home: Option<OsString>,
}

impl Env {
    pub fn from_process() -> Env {
        Env::from_vars(|name| std::env::var_os(name))
    }

    /// `var` looks a variable up by name; `from_process` passes the real environment.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Env {
        Env {
            bilbo_home: var("BILBO_HOME"),
            xdg_data_home: var("XDG_DATA_HOME"),
            home: var("HOME"),
            bilbo_config: var("BILBO_CONFIG"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            xdg_cache_home: var("XDG_CACHE_HOME"),
            xdg_state_home: var("XDG_STATE_HOME"),
        }
    }
}

/// The value as a path when it is set, not empty and absolute.
pub fn absolute(value: &Option<OsString>) -> Option<PathBuf> {
    value
        .as_ref()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The store root, or the stderr message (without "bilbo: ") for `Failure::Config`.
pub fn root(env: &Env) -> Result<PathBuf, String> {
    if let Some(home) = env.bilbo_home.as_ref().filter(|v| !v.is_empty()) {
        let path = PathBuf::from(home);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Err(format!(
                "BILBO_HOME must be an absolute path, got '{}'",
                path.display()
            ))
        };
    }
    if let Some(xdg) = absolute(&env.xdg_data_home) {
        return Ok(xdg.join("bilbo"));
    }
    if let Some(home) = absolute(&env.home) {
        return Ok(home.join(".local/share/bilbo"));
    }
    Err("cannot find the store root: set BILBO_HOME, or HOME, to an absolute path".into())
}

/// `$XDG_STATE_HOME` when absolute, else `$HOME/.local/state`; `None` without either.
pub fn state_dir(env: &Env) -> Option<PathBuf> {
    absolute(&env.xdg_state_home).or_else(|| absolute(&env.home).map(|h| h.join(".local/state")))
}

/// `<root>/library`, where sources live.
pub fn library_dir(root: &Path) -> PathBuf {
    root.join("library")
}

/// `<root>/.bilbo/captures`, the local evidence of what each source was cut from.
pub fn captures_dir(root: &Path) -> PathBuf {
    root.join(".bilbo/captures")
}

/// `<state>/bilbo/staging`, where staged text waits for `library land`; `None` without a state folder.
pub fn staging_dir(env: &Env) -> Option<PathBuf> {
    state_dir(env).map(|s| s.join("bilbo/staging"))
}

/// `<state>/bilbo/plans`, where `library plan` writes plans and `library read` logs; `None` without a state folder.
pub fn plans_dir(env: &Env) -> Option<PathBuf> {
    state_dir(env).map(|s| s.join("bilbo/plans"))
}

/// `$XDG_CONFIG_HOME` when absolute, else `$HOME/.config`; `None` without either.
pub fn config_home(env: &Env) -> Option<PathBuf> {
    absolute(&env.xdg_config_home).or_else(|| absolute(&env.home).map(|h| h.join(".config")))
}

/// `$XDG_CACHE_HOME/bilbo` when that is absolute, else `$HOME/.cache/bilbo`.
pub fn cache_dir(env: &Env) -> Option<PathBuf> {
    if let Some(xdg) = absolute(&env.xdg_cache_home) {
        return Some(xdg.join("bilbo"));
    }
    absolute(&env.home).map(|home| home.join(".cache/bilbo"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    Other,
    /// The stat failed; holds the io error text.
    Unreadable(String),
}

#[derive(Debug)]
pub struct Entry {
    /// The display name; lossy when `utf8` is false.
    pub name: String,
    /// False when the real name is not valid UTF-8; do I/O through `path`.
    pub utf8: bool,
    pub path: PathBuf,
    pub kind: EntryKind,
}

/// Entries of `dir` sorted by name, skipping names that start with '.'. Follows symlinks for the kind.
pub fn entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let file_name = item.file_name();
        let name = file_name.to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = item.path();
        let kind = match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => EntryKind::File,
            Ok(meta) if meta.is_dir() => EntryKind::Folder,
            Ok(_) => EntryKind::Other,
            Err(e) => EntryKind::Unreadable(e.to_string()),
        };
        let utf8 = file_name.to_str().is_some();
        entries.push(Entry {
            name,
            utf8,
            path,
            kind,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub const TOPIC_RULE: &str = "use segments of a-z and 0-9 joined by single hyphens";

pub fn is_topic(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

impl Problem {
    pub fn at(line: usize, message: impl Into<String>) -> Problem {
        Problem {
            line: Some(line),
            message: message.into(),
        }
    }

    pub fn whole(message: impl Into<String>) -> Problem {
        Problem {
            line: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(n) => write!(f, "{} (line {n})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// `body` starts at physical line `first_line`.
pub fn title_problem(body: &[&str], first_line: usize) -> Option<Problem> {
    let mut fence: Option<(char, usize)> = None;
    let mut titles = Vec::new();
    for (i, line) in body.iter().enumerate() {
        let run = fence_run(line);
        match (fence, run) {
            (Some((ch, len)), Some((c, l, rest)))
                if c == ch && l >= len && rest.trim().is_empty() =>
            {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some((c, l, _))) => fence = Some((c, l)),
            (None, None) if line.starts_with("# ") => titles.push(first_line + i),
            (None, None) => {}
        }
    }
    match titles.as_slice() {
        [] => Some(Problem::whole(
            "title: missing; add one '# <title>' line after the frontmatter",
        )),
        [_] => None,
        [_, second, ..] => Some(Problem::at(
            *second,
            format!(
                "title: found {} '# ' headings outside code fences, expected one",
                titles.len()
            ),
        )),
    }
}

/// A note recall and index read, with its passages.
pub struct Stored {
    pub path: PathBuf,
    pub kind: String,
    pub created: Option<String>,
    pub document: rank::Document,
}

/// The notes recall searches, in name order: UTF-8-named regular files with a valid note name, read lossily; others are skipped in silence.
pub fn read_notes(notes: &Path) -> std::io::Result<Vec<Stored>> {
    let mut stored = Vec::new();
    for entry in entries(notes)? {
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
        let lines = markdown::lines(&text);
        let read = note::read(&text);
        let stem = entry.name.strip_suffix(".md").unwrap_or(&entry.name);
        stored.push(Stored {
            path: entry.path,
            kind: name.kind,
            created: read.created,
            document: rank::Document {
                passages: rank::passages(&lines[read.body_start - 1..], read.body_start, stem),
            },
        });
    }
    Ok(stored)
}

/// Whether a library file is a source or a corpus guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    Source,
    Guide,
}

/// A source or a guide library recall reads, with its passages and the sections they sit in.
pub struct Shelved {
    pub path: PathBuf,
    pub shelf: Shelf,
    /// `<corpus>/<name>` for a source, `<corpus>` for a guide: what the `library` verb takes.
    pub reference: String,
    pub document: rank::Document,
    /// Physical line of the first non-blank line of the body: the title's, when it has one.
    title_line: usize,
    last_line: usize,
    /// Start and end lines of each heading below the title, in line order.
    sections: Vec<(usize, usize)>,
}

impl Shelved {
    /// The lines of the section a passage starting on `line` sits in: the last heading below the title at or before
    /// `line` down to the line before the next heading of its level or above, else from the title to the line before
    /// the first heading below it.
    pub fn section(&self, line: usize) -> (usize, usize) {
        let at = self.sections.partition_point(|(start, _)| *start <= line);
        match at {
            0 => (
                self.title_line,
                self.sections
                    .first()
                    .map_or(self.last_line, |(start, _)| start - 1),
            ),
            n => self.sections[n - 1],
        }
    }
}

/// The sources and guides of the corpus folders `corpora` names, or of every valid corpus folder when it is empty,
/// in path order. Hidden entries, invalid names and files that cannot be read are skipped in silence.
pub fn read_library(root: &Path, corpora: &[String]) -> std::io::Result<Vec<Shelved>> {
    let mut shelved = Vec::new();
    for (name, dir) in corpus::corpus_dirs(root)? {
        if !corpora.is_empty() && !corpora.contains(&name) {
            continue;
        }
        let Ok(files) = entries(&dir) else {
            continue;
        };
        for entry in files {
            if !entry.utf8 || entry.kind != EntryKind::File {
                continue;
            }
            let Some(stem) = entry.name.strip_suffix(".md") else {
                continue;
            };
            let (shelf, reference) = if stem == "guide" {
                (Shelf::Guide, name.clone())
            } else if corpus::is_source_name(stem) {
                (Shelf::Source, format!("{name}/{stem}"))
            } else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&entry.path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let lines = markdown::lines(&text);
            let body_start = note::read(&text).body_start;
            let title_line = (body_start..=lines.len())
                .find(|n| !lines[n - 1].trim().is_empty())
                .unwrap_or(body_start);
            shelved.push(Shelved {
                path: entry.path,
                shelf,
                reference,
                document: rank::Document {
                    passages: rank::passages(
                        lines.get(body_start - 1..).unwrap_or(&[]),
                        body_start,
                        stem,
                    ),
                },
                title_line,
                last_line: lines.len(),
                sections: markdown::outline(&lines, title_line)
                    .into_iter()
                    .map(|s| (s.start, s.end))
                    .collect(),
            });
        }
    }
    Ok(shelved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(bilbo: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> Env {
        Env {
            bilbo_home: bilbo.map(OsString::from),
            xdg_data_home: xdg.map(OsString::from),
            home: home.map(OsString::from),
            bilbo_config: None,
            xdg_config_home: None,
            xdg_cache_home: None,
            xdg_state_home: None,
        }
    }

    #[test]
    fn from_vars_reads_every_variable() {
        let table = [
            ("BILBO_HOME", "/a"),
            ("XDG_DATA_HOME", "/b"),
            ("HOME", "/c"),
            ("BILBO_CONFIG", "/d"),
            ("XDG_CONFIG_HOME", "/e"),
            ("XDG_CACHE_HOME", "/f"),
            ("XDG_STATE_HOME", "/g"),
        ];
        let lookup = |name: &str| {
            table
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        };
        let e = Env::from_vars(lookup);
        assert_eq!(e.bilbo_home, Some(OsString::from("/a")));
        assert_eq!(e.xdg_data_home, Some(OsString::from("/b")));
        assert_eq!(e.home, Some(OsString::from("/c")));
        assert_eq!(e.bilbo_config, Some(OsString::from("/d")));
        assert_eq!(e.xdg_config_home, Some(OsString::from("/e")));
        assert_eq!(e.xdg_cache_home, Some(OsString::from("/f")));
        assert_eq!(e.xdg_state_home, Some(OsString::from("/g")));
        let none = Env::from_vars(|_| None);
        assert!(none.bilbo_config.is_none() && none.xdg_cache_home.is_none());
    }

    #[test]
    fn bilbo_home_wins() {
        let e = env(Some("/srv/bilbo"), Some("/x"), Some("/home/a"));
        assert_eq!(root(&e), Ok(PathBuf::from("/srv/bilbo")));
    }

    #[test]
    fn xdg_data_home_is_used_next() {
        let e = env(None, Some("/x"), Some("/home/a"));
        assert_eq!(root(&e), Ok(PathBuf::from("/x/bilbo")));
    }

    #[test]
    fn default_root() {
        let e = env(None, None, Some("/home/a"));
        assert_eq!(root(&e), Ok(PathBuf::from("/home/a/.local/share/bilbo")));
    }

    #[test]
    fn macos_uses_the_same_default() {
        let path = root(&env(None, None, Some("/Users/a"))).unwrap();
        assert_eq!(path, PathBuf::from("/Users/a/.local/share/bilbo"));
        assert!(path.components().all(|c| c.as_os_str() != "Library"));
    }

    #[test]
    fn relative_xdg_data_home_is_ignored() {
        let e = env(None, Some("data"), Some("/home/a"));
        assert_eq!(root(&e), Ok(PathBuf::from("/home/a/.local/share/bilbo")));
    }

    #[test]
    fn relative_bilbo_home_is_refused() {
        let err = root(&env(Some("store"), Some("/x"), Some("/home/a"))).unwrap_err();
        assert!(err.contains("BILBO_HOME must be an absolute path"), "{err}");
        assert!(err.contains("'store'"), "{err}");
    }

    #[test]
    fn empty_bilbo_home_counts_as_unset() {
        let e = env(Some(""), Some("/x"), None);
        assert_eq!(root(&e), Ok(PathBuf::from("/x/bilbo")));
    }

    #[test]
    fn no_home_is_an_error() {
        assert!(root(&env(None, None, None)).is_err());
        assert!(root(&env(None, Some(""), Some("home"))).is_err());
    }

    #[test]
    fn state_dir_prefers_absolute_xdg_state_home() {
        let mut e = env(None, None, Some("/home/a"));
        e.xdg_state_home = Some(OsString::from("/s"));
        assert_eq!(state_dir(&e), Some(PathBuf::from("/s")));
        e.xdg_state_home = Some(OsString::from("rel"));
        assert_eq!(state_dir(&e), Some(PathBuf::from("/home/a/.local/state")));
    }

    #[test]
    fn state_dir_falls_back_to_home() {
        let e = env(None, None, Some("/home/a"));
        assert_eq!(state_dir(&e), Some(PathBuf::from("/home/a/.local/state")));
        assert_eq!(state_dir(&env(None, None, None)), None);
    }

    #[test]
    fn plans_live_under_the_state_folder() {
        let mut e = env(None, None, Some("/home/a"));
        assert_eq!(
            plans_dir(&e),
            Some(PathBuf::from("/home/a/.local/state/bilbo/plans"))
        );
        e.xdg_state_home = Some(OsString::from("/s"));
        assert_eq!(plans_dir(&e), Some(PathBuf::from("/s/bilbo/plans")));
        assert_eq!(plans_dir(&env(None, None, None)), None);
    }

    #[test]
    fn config_home_rules() {
        let mut e = env(None, None, Some("/home/a"));
        assert_eq!(config_home(&e), Some(PathBuf::from("/home/a/.config")));
        e.xdg_config_home = Some(OsString::from("/c"));
        assert_eq!(config_home(&e), Some(PathBuf::from("/c")));
        e.xdg_config_home = Some(OsString::from("rel"));
        assert_eq!(config_home(&e), Some(PathBuf::from("/home/a/.config")));
        assert_eq!(config_home(&env(None, None, None)), None);
    }

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn entries_skip_hidden_and_sort() {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("bilbo-store-entries-{}", std::process::id())),
        );
        let dir = &scratch.0;
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("b.md"), "").unwrap();
        std::fs::write(dir.join("a.md"), "").unwrap();
        std::fs::write(dir.join(".DS_Store"), "").unwrap();
        let listed = entries(dir).unwrap();
        let names: Vec<_> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.md", "b.md", "sub"]);
        assert_eq!(listed[2].kind, EntryKind::Folder);
        assert_eq!(listed[0].kind, EntryKind::File);
    }

    #[test]
    #[cfg_attr(target_os = "macos", ignore = "APFS refuses non-UTF-8 names")]
    fn non_utf8_names_keep_their_real_path() {
        use std::os::unix::ffi::OsStrExt;
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-store-utf8-{}", std::process::id())));
        let dir = &scratch.0;
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let odd = dir.join(std::ffi::OsStr::from_bytes(b"plan-\xff.md"));
        std::fs::write(&odd, "x").unwrap();
        let listed = entries(dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].utf8);
        assert_eq!(listed[0].path, odd);
        assert_eq!(listed[0].kind, EntryKind::File);
    }

    #[test]
    fn broken_symlink_is_unreadable() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-store-link-{}", std::process::id())));
        let dir = &scratch.0;
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), dir.join("plan-a.md")).unwrap();
        let listed = entries(dir).unwrap();
        assert!(matches!(listed[0].kind, EntryKind::Unreadable(_)));
    }

    fn shelf(tag: &str) -> Scratch {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-store-{tag}-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&scratch.0);
        std::fs::create_dir_all(scratch.0.join("library/go")).unwrap();
        scratch
    }

    const SOURCE_FRONT: &str = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\nfetched: 2026-10-03\norigin: \"url: https://golang.org/doc/golang\"\ndigest: sha256:0000000000000000000000000000000000000000000000000000000000000000\n---\n";

    fn put(root: &Path, file: &str, text: &str) {
        let path = root.join("library").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn source_text(body: &str) -> String {
        format!("{SOURCE_FRONT}{body}")
    }

    fn only(root: &Path, text: &str) -> Shelved {
        put(root, "go/s.md", text);
        read_library(root, &[]).unwrap().remove(0)
    }

    #[test]
    fn a_corpus_reads_its_guide_and_sources_in_name_order() {
        let scratch = shelf("order");
        let root = &scratch.0;
        put(
            root,
            "go/guide.md",
            "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-03T10:00-03:00\n---\n\n# go\n\n## b\n\ntext\n",
        );
        put(root, "go/b.md", &source_text("# B\n\nbee\n"));
        put(root, "go/a.md", &source_text("# A\n\nay\n"));
        let library = read_library(root, &[]).unwrap();
        let seen: Vec<_> = library
            .iter()
            .map(|s| (s.reference.as_str(), s.shelf))
            .collect();
        assert_eq!(
            seen,
            [
                ("go/a", Shelf::Source),
                ("go/b", Shelf::Source),
                ("go", Shelf::Guide)
            ]
        );
        assert_eq!(library[0].path, root.join("library/go/a.md"));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_entries_are_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = shelf("unreadable");
        let root = &scratch.0;
        put(root, "go/ok.md", &source_text("# Ok\n"));
        put(root, "go/locked.md", &source_text("# Locked\n"));
        put(root, "rust/a.md", &source_text("# A\n"));
        let lock = |path: PathBuf, mode: u32| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        lock(root.join("library/go/locked.md"), 0o000);
        lock(root.join("library/rust"), 0o000);
        if std::fs::read(root.join("library/go/locked.md")).is_ok()
            || std::fs::read_dir(root.join("library/rust")).is_ok()
        {
            lock(root.join("library/go/locked.md"), 0o644);
            lock(root.join("library/rust"), 0o755);
            eprintln!("skipped: the entries are readable despite mode 000 (running as root?)");
            return;
        }
        let read = read_library(root, &[]);
        lock(root.join("library/go/locked.md"), 0o644);
        lock(root.join("library/rust"), 0o755);
        let library = read.unwrap();
        let seen: Vec<_> = library.iter().map(|s| s.reference.as_str()).collect();
        assert_eq!(seen, ["go/ok"]);
    }

    #[test]
    fn corpora_narrow_the_read() {
        let scratch = shelf("narrow");
        let root = &scratch.0;
        put(root, "go/a.md", &source_text("# A\n"));
        put(root, "rust/a.md", &source_text("# A\n"));
        let library = read_library(root, &["rust".to_string()]).unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].reference, "rust/a");
    }

    #[test]
    fn invalid_entries_are_skipped() {
        let scratch = shelf("invalid");
        let root = &scratch.0;
        put(root, "go/Effective_Go.md", &source_text("# E\n"));
        put(root, "go/.draft.md", &source_text("# D\n"));
        put(root, "go/sub/deep.md", &source_text("# D\n"));
        put(root, "go/notes.txt", "x");
        put(root, "Go-Old/errors.md", &source_text("# E\n"));
        put(root, "plan/errors.md", &source_text("# E\n"));
        put(root, "go/ok.md", &source_text("# Ok\n"));
        let library = read_library(root, &[]).unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].reference, "go/ok");
    }

    #[test]
    fn frontmatter_words_are_in_no_passage() {
        let scratch = shelf("front");
        let s = only(&scratch.0, &source_text("# Title\n\nbody\n"));
        let all: String = s
            .document
            .passages
            .iter()
            .map(|p| format!("{} {}", p.path.join(" "), p.text))
            .collect();
        assert!(!all.contains("golang") && !all.contains("digest"), "{all}");
    }

    #[test]
    fn a_bad_digest_is_still_read() {
        let scratch = shelf("digest");
        let s = only(&scratch.0, &source_text("# Title\n\nedited body\n"));
        assert_eq!(s.document.passages.len(), 1);
    }

    #[test]
    fn a_source_without_a_title_uses_its_file_stem() {
        let scratch = shelf("stem");
        let s = only(&scratch.0, &source_text("just text\n"));
        assert_eq!(s.document.passages[0].path, ["s"]);
    }

    fn body() -> String {
        // The front takes lines 1 to 6.
        source_text(
            "# Title\n\
             intro\n\
             ## Concurrency\n\
             own text\n\
             ### Goroutines\n\
             ```\n\
             ## not a heading\n\
             ```\n\
             more\n\
             ### Channels\n\
             chan\n\
             ## Errors\n\
             err\n",
        )
    }

    #[test]
    fn the_section_of_a_level_3_heading() {
        let scratch = shelf("l3");
        let s = only(&scratch.0, &body());
        // Title 7, intro 8, Concurrency 9, Goroutines 11, Channels 16, Errors 18..19.
        assert_eq!(s.section(11), (11, 15));
        assert_eq!(s.section(13), (11, 15));
    }

    #[test]
    fn the_section_of_a_level_2_heading_holds_its_subsections() {
        let scratch = shelf("l2");
        let s = only(&scratch.0, &body());
        assert_eq!(s.section(9), (9, 17));
        assert_eq!(s.section(18), (18, 19));
    }

    #[test]
    fn the_title_passage_runs_to_the_first_heading() {
        let scratch = shelf("title");
        let s = only(&scratch.0, &body());
        assert_eq!(s.section(7), (7, 8));
    }

    #[test]
    fn a_later_part_of_a_split_passage_keeps_its_section() {
        let scratch = shelf("parts");
        let para = "word ".repeat(300);
        let text = source_text(&format!(
            "# Title\n\n## Big\n\n{para}\n\n{para}\n\n{para}\n\n## Next\n\nx\n"
        ));
        let s = only(&scratch.0, &text);
        let parts: Vec<_> = s
            .document
            .passages
            .iter()
            .filter(|p| p.path.last().is_some_and(|l| l == "Big"))
            .collect();
        assert!(parts.len() > 1);
        let later = parts[1].line;
        assert!(later > parts[0].line);
        assert_eq!(s.section(later), s.section(parts[0].line));
    }

    #[test]
    fn a_source_with_no_heading_below_its_title() {
        let scratch = shelf("flat");
        let s = only(&scratch.0, &source_text("# Title\n\ntext\nmore\n"));
        assert_eq!(s.section(7), (7, 10));
        assert_eq!(s.section(9), (7, 10));
    }

    #[test]
    fn a_guide_entry_ends_before_the_next_entry() {
        let scratch = shelf("guide");
        let root = &scratch.0;
        put(
            root,
            "go/guide.md",
            "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-03T10:00-03:00\n---\n\n# go\n\nabout\n\n## one\n\ntext\n\n## two\n\ntext\n",
        );
        let s = read_library(root, &[]).unwrap().remove(0);
        // Title 6, about 8, one 10, two 14..16.
        assert_eq!(s.section(6), (6, 9));
        assert_eq!(s.section(10), (10, 13));
        assert_eq!(s.section(14), (14, 16));
    }

    fn cache_env(xdg: Option<&str>, home: Option<&str>) -> Env {
        Env::from_vars(|name| match name {
            "XDG_CACHE_HOME" => xdg.map(Into::into),
            "HOME" => home.map(Into::into),
            _ => None,
        })
    }

    #[test]
    fn cache_dir_prefers_absolute_xdg_cache_home() {
        assert_eq!(
            cache_dir(&cache_env(Some("/x"), Some("/h"))),
            Some(PathBuf::from("/x/bilbo"))
        );
        assert_eq!(
            cache_dir(&cache_env(Some("x"), Some("/h"))),
            Some(PathBuf::from("/h/.cache/bilbo"))
        );
        assert_eq!(cache_dir(&cache_env(Some("x"), None)), None);
        assert_eq!(cache_dir(&cache_env(None, Some("h"))), None);
    }
}
