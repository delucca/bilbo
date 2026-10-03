use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{note, rank};

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

/// `$XDG_CONFIG_HOME` when absolute, else `$HOME/.config`; `None` without either.
pub fn config_home(env: &Env) -> Option<PathBuf> {
    absolute(&env.xdg_config_home).or_else(|| absolute(&env.home).map(|h| h.join(".config")))
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
        let lines = note::lines(&text);
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
}
