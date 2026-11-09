use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use bilbo_kinds::follows::Follow;
use bilbo_settings::folder;
use bilbo_settings::key::{Origin, quote};

use super::messages::{info, warning};
use crate::console::message::{Detail, Mark, Message, sanitize};
use crate::console::settings::{self, Settings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// `defaults`: the file declared no follow, so the defaults were written before the new one.
    /// `unsynced`: why its folder could not be synced after the rename.
    Added {
        config: PathBuf,
        defaults: bool,
        unsynced: Option<String>,
    },
    Already(PathBuf),
}

impl Saved {
    pub fn message(&self) -> Message {
        match self {
            Self::Added {
                config,
                defaults,
                unsynced,
            } => {
                let mut text = format!("saved the follow in {}", config.display());
                if *defaults {
                    text.push_str(", after the default follows it would replace");
                }
                match unsynced {
                    Some(error) => {
                        warning(&format!("{text}; its folder could not be synced: {error}"))
                    }
                    None => info(&text),
                }
            }
            Self::Already(path) => info(&format!("{} is followed already", path.display())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    Tables(String),
    ArrayItem(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub config: PathBuf,
    pub reason: String,
    pub remedy: Remedy,
}

impl Refusal {
    pub fn message(&self) -> Message {
        let (ask, lines) = match &self.remedy {
            Remedy::Tables(lines) => ("add these lines to it:", lines),
            Remedy::ArrayItem(item) => ("add this table to its follow array:", item),
        };
        Message {
            mark: Mark::Error,
            text: sanitize(&format!(
                "cannot save the follow in {}: {}; {ask}",
                self.config.display(),
                self.reason
            )),
            details: vec![Detail::Text(lines.trim_end().to_owned())],
        }
    }
}

/// Where `save_with` is, for a test that races two saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saving {
    /// About to take the lock beside the config file.
    Locking,
    /// Holds the lock and has read the file; writes next.
    Read,
}

/// Appends `follow` as a `[[follow]]` table, every earlier byte kept. A file that declared no
/// follow gets the defaults first, since a declared follow replaces them.
pub fn save(
    settings: &Settings,
    follow: &Follow,
    var: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Saved, Refusal> {
    save_with(settings, follow, var, &mut |_| {})
}

/// Reads the file again under `<config>.lock`, so two saves at once keep both follows.
pub fn save_with(
    settings: &Settings,
    follow: &Follow,
    var: &dyn Fn(&str) -> Option<OsString>,
    hook: &mut dyn FnMut(Saving),
) -> Result<Saved, Refusal> {
    let config = &settings.config.value;
    let refusal = |reason: String, remedy: Remedy| Refusal {
        config: config.clone(),
        reason,
        remedy,
    };
    let first_lines = if declares_inline(config, &settings.follow.origin) {
        Remedy::ArrayItem(settings::inline_follow(follow))
    } else {
        Remedy::Tables(lines(
            &settings.follow.value,
            settings.follow.origin == Origin::Default,
            follow,
        ))
    };
    check(config).map_err(|reason| refusal(reason, first_lines.clone()))?;
    let lock =
        lock_file(config).map_err(|error| refusal(error.to_string(), first_lines.clone()))?;
    hook(Saving::Locking);
    lock.lock()
        .map_err(|error| refusal(error.to_string(), first_lines.clone()))?;
    let current = settings::load(var, &settings::read_regular)
        .map_err(|error| refusal(error.to_string(), first_lines.clone()))?;
    if current.follow.value.iter().any(|declared| {
        fs::canonicalize(&declared.path).unwrap_or_else(|_| declared.path.clone()) == follow.path
    }) {
        return Ok(Saved::Already(follow.path.clone()));
    }
    let defaults = current.follow.origin == Origin::Default;
    let lines = lines(&current.follow.value, defaults, follow);
    let refuse = |reason: String| refusal(reason, Remedy::Tables(lines.clone()));
    let before = read(config).map_err(refuse)?;
    hook(Saving::Read);
    if let Origin::File { line, .. } = current.follow.origin
        && inline(&before, line)
    {
        return Err(Refusal {
            config: config.clone(),
            reason: "it declares follow as an inline array".to_owned(),
            remedy: Remedy::ArrayItem(settings::inline_follow(follow)),
        });
    }
    let text = appended(&before, &lines);
    let loaded = settings::load(var, &|path| {
        if path == config {
            Ok(text.clone())
        } else {
            settings::read_regular(path)
        }
    })
    .map_err(|error| refuse(error.to_string()))?;
    if !loaded.follow.value.contains(follow) {
        return Err(refuse("the file would not declare it".to_owned()));
    }
    let unsynced = replace(config, &text).map_err(|error| refuse(error.to_string()))?;
    Ok(Saved::Added {
        config: config.clone(),
        defaults,
        unsynced,
    })
}

fn lines(declared: &[Follow], defaults: bool, follow: &Follow) -> String {
    let mut tables: Vec<&Follow> = if defaults {
        declared.iter().collect()
    } else {
        Vec::new()
    };
    tables.push(follow);
    let tables: Vec<String> = tables.into_iter().map(table).collect();
    tables.join("\n")
}

/// `<config>.lock` beside the file, created private with its folder, never removed.
fn lock_file(config: &Path) -> io::Result<fs::File> {
    let folder = config
        .parent()
        .ok_or_else(|| io::Error::other("it has no folder"))?;
    fs::create_dir_all(folder)?;
    let mut name = config
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?
        .to_owned();
    name.push(".lock");
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(folder.join(name))
}

pub fn table(follow: &Follow) -> String {
    let mut text = format!("[[follow]]\npath = {}\n", folder::show(&follow.path));
    if let Some(kind) = &follow.kind {
        text.push_str(&format!("kind = {}\n", quote(kind)));
    }
    text
}

pub fn appended(before: &[u8], tables: &str) -> Vec<u8> {
    let mut text = before.to_vec();
    if !text.is_empty() {
        if !text.ends_with(b"\n") {
            text.push(b'\n');
        }
        text.push(b'\n');
    }
    text.extend_from_slice(tables.as_bytes());
    text
}

/// A link (a Nix-managed config), a read-only file or another account's file is refused: replacing
/// it would cut it from what manages it.
fn check(config: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(config) {
        Ok(meta) if meta.file_type().is_symlink() => Err("it is a symbolic link".to_owned()),
        Ok(meta) if !meta.is_file() => Err("it is not a regular file".to_owned()),
        Ok(_) => fs::OpenOptions::new()
            .write(true)
            .open(config)
            .map(|_| true)
            .map_err(|_| "it is read-only".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// Judged before any refusal: a read-only or linked file is read as it is.
fn declares_inline(config: &Path, origin: &Origin) -> bool {
    let Origin::File { line, .. } = origin else {
        return false;
    };
    settings::read_regular(config)
        .ok()
        .is_some_and(|bytes| inline(&bytes, *line))
}

fn read(config: &Path) -> Result<Vec<u8>, String> {
    if check(config)? {
        fs::read(config).map_err(|error| error.to_string())
    } else {
        Ok(Vec::new())
    }
}

/// Only the `[[follow]]` form starts the key's value on a `[[` header.
fn inline(before: &[u8], line: usize) -> bool {
    before
        .split(|&byte| byte == b'\n')
        .nth(line.saturating_sub(1))
        .is_some_and(|text| !text.trim_ascii_start().starts_with(b"[["))
}

/// Temp file then rename, keeping the old permissions; a failed folder sync is returned, not
/// refused.
fn replace(config: &Path, text: &[u8]) -> io::Result<Option<String>> {
    let folder = config
        .parent()
        .ok_or_else(|| io::Error::other("it has no folder"))?;
    let name = config
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?;
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = folder.join(temporary);
    let renamed = (|| {
        let mut file = fs::File::create_new(&temporary)?;
        if let Ok(meta) = fs::metadata(config) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(text)?;
        file.sync_all()?;
        fs::rename(&temporary, config)
    })();
    if renamed.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    finish(renamed, || fs::File::open(folder)?.sync_all())
}

fn finish(
    renamed: io::Result<()>,
    sync: impl FnOnce() -> io::Result<()>,
) -> io::Result<Option<String>> {
    renamed?;
    Ok(sync().err().map(|error| error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_kinds::follows::Follow;

    use std::io;

    use super::{Refusal, Remedy, Saved, appended, finish, table};
    use crate::console::message::Mark;
    use crate::console::view::View;

    fn follow(path: &str, kind: Option<&str>) -> Follow {
        Follow {
            path: PathBuf::from(path),
            kind: kind.map(str::to_owned),
        }
    }

    #[test]
    fn a_follow_is_a_table_with_its_path_and_its_kind_when_it_has_one() {
        assert_eq!(
            table(&follow("/h/notes", Some("markdown"))),
            "[[follow]]\npath = \"/h/notes\"\nkind = \"markdown\"\n"
        );
        assert_eq!(
            table(&follow("/h/a \"b\"", None)),
            "[[follow]]\npath = \"/h/a \\\"b\\\"\"\n"
        );
    }

    #[test]
    fn the_tables_go_after_the_file_as_it_was_with_a_blank_line_between() {
        let tables = "[[follow]]\npath = \"/x\"\n";
        assert_eq!(appended(b"", tables), tables.as_bytes());
        assert_eq!(
            appended(b"[store]\ncompression = 3\n", tables),
            b"[store]\ncompression = 3\n\n[[follow]]\npath = \"/x\"\n"
        );
        assert_eq!(
            appended(b"# no newline at the end", tables),
            b"# no newline at the end\n\n[[follow]]\npath = \"/x\"\n"
        );
        let odd = b"[store]  \r\n#  spaced   comment\r\n\n\n";
        assert!(appended(odd, tables).starts_with(odd));
    }

    #[test]
    fn a_refusal_gives_the_lines_to_paste() {
        let refusal = Refusal {
            config: PathBuf::from("/h/.config/bilbo/config.toml"),
            reason: "it is a symbolic link".to_owned(),
            remedy: Remedy::Tables(table(&follow("/h/notes", Some("markdown")))),
        };
        let message = refusal.message();
        assert_eq!(message.mark, Mark::Error);
        assert_eq!(
            message.lines(&View::PLAIN),
            [
                "bilbo: cannot save the follow in /h/.config/bilbo/config.toml: it is a symbolic link; add these lines to it:",
                "bilbo: [[follow]]",
                "bilbo: path = \"/h/notes\"",
                "bilbo: kind = \"markdown\"",
            ]
        );
    }

    #[test]
    fn what_was_saved_reads_as_one_line() {
        let config = PathBuf::from("/c/config.toml");
        let said = |saved: Saved| {
            let message = saved.message();
            (message.mark, message.text)
        };
        assert_eq!(
            said(Saved::Added {
                config: config.clone(),
                defaults: false,
                unsynced: None
            }),
            (Mark::Info, "saved the follow in /c/config.toml".to_owned())
        );
        assert_eq!(
            said(Saved::Added {
                config,
                defaults: true,
                unsynced: None
            }),
            (
                Mark::Info,
                "saved the follow in /c/config.toml, after the default follows it would replace"
                    .to_owned()
            )
        );
        assert_eq!(
            said(Saved::Already(PathBuf::from("/h/notes"))),
            (Mark::Info, "/h/notes is followed already".to_owned())
        );
    }

    #[test]
    fn a_failed_folder_sync_after_the_rename_is_saved_with_a_warning() {
        assert_eq!(
            finish(Ok(()), || Err(io::Error::other("no sync"))).expect("renamed"),
            Some("no sync".to_owned())
        );
        assert!(
            finish(Err(io::Error::other("no rename")), || panic!(
                "never synced"
            ))
            .is_err()
        );
        let message = Saved::Added {
            config: "/c/config.toml".into(),
            defaults: false,
            unsynced: Some("no sync".into()),
        }
        .message();
        assert_eq!(message.mark, Mark::Warning);
        assert_eq!(
            message.text,
            "saved the follow in /c/config.toml; its folder could not be synced: no sync"
        );
    }
}
