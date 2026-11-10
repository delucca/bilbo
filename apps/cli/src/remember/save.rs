use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use bilbo_kinds::follows::Follow;
use bilbo_settings::edit;
use bilbo_settings::folder;
use bilbo_settings::key::{Origin, quote};

use super::messages::{info, warning};
use crate::console::message::{Detail, Message, sanitize};
use crate::console::settings::{self, Settings};
use crate::console::theme::Mark;

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
    edit::check(config).map_err(|reason| refusal(reason, first_lines.clone()))?;
    let lock =
        edit::lock_file(config).map_err(|error| refusal(error.to_string(), first_lines.clone()))?;
    hook(Saving::Locking);
    lock.lock()
        .map_err(|error| refusal(error.to_string(), first_lines.clone()))?;
    let kind: Vec<_> = settings.kind.iter().map(|(key, _)| *key).collect();
    let current = settings::load(var, &edit::read_regular, &kind)
        .map_err(|error| refusal(error.to_string(), first_lines.clone()))?;
    if current.follow.value.iter().any(|declared| {
        fs::canonicalize(&declared.path).unwrap_or_else(|_| declared.path.clone()) == follow.path
    }) {
        return Ok(Saved::Already(follow.path.clone()));
    }
    let defaults = current.follow.origin == Origin::Default;
    let lines = lines(&current.follow.value, defaults, follow);
    let refuse = |reason: String| refusal(reason, Remedy::Tables(lines.clone()));
    let before = edit::read(config).map_err(refuse)?;
    hook(Saving::Read);
    if let Origin::File { line, .. } = current.follow.origin
        && edit::inline(&before, line)
    {
        return Err(Refusal {
            config: config.clone(),
            reason: "it declares follow as an inline array".to_owned(),
            remedy: Remedy::ArrayItem(settings::inline_follow(follow)),
        });
    }
    let text = edit::appended(&before, &lines);
    let loaded = settings::load(
        var,
        &|path| {
            if path == config {
                Ok(text.clone())
            } else {
                edit::read_regular(path)
            }
        },
        &kind,
    )
    .map_err(|error| refuse(error.to_string()))?;
    if !loaded.follow.value.contains(follow) {
        return Err(refuse("the file would not declare it".to_owned()));
    }
    let unsynced = edit::replace(config, &text).map_err(|error| refuse(error.to_string()))?;
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

pub fn table(follow: &Follow) -> String {
    let mut text = format!("[[follow]]\npath = {}\n", folder::show(&follow.path));
    if let Some(kind) = &follow.kind {
        text.push_str(&format!("kind = {}\n", quote(kind)));
    }
    text
}

/// Judged before any refusal: a read-only or linked file is read as it is.
fn declares_inline(config: &Path, origin: &Origin) -> bool {
    let Origin::File { line, .. } = origin else {
        return false;
    };
    edit::read_regular(config)
        .ok()
        .is_some_and(|bytes| edit::inline(&bytes, *line))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_kinds::follows::Follow;

    use super::{Refusal, Remedy, Saved, table};
    use crate::console::theme::Mark;
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
