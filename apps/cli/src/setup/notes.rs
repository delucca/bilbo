use std::cell::RefCell;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use bilbo_entries::follows::Follow;
use bilbo_settings::key::{Origin, Resolved};
use bilbo_setup::report::{Line, Status};
use bilbo_setup::steps::Notes;

use crate::console::message::Message;
use crate::console::settings::SETUP_NOTES;

pub enum Answer {
    Saved {
        config: PathBuf,
        defaults: bool,
        unsynced: Option<String>,
    },
    Already,
    Refused {
        reason: String,
        message: Message,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Skip,
    Failed(String),
    Keep,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub plan: Plan,
    pub folder: PathBuf,
    pub followed: bool,
}

fn canonical_of(path: &Path) -> PathBuf {
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut here = path.to_path_buf();
    loop {
        if let Ok(found) = fs::canonicalize(&here) {
            return rest
                .iter()
                .rev()
                .fold(found, |whole, part| whole.join(part));
        }
        match (here.file_name().map(ToOwned::to_owned), here.parent()) {
            (Some(name), Some(parent))
                if !matches!(here.components().next_back(), Some(Component::ParentDir)) =>
            {
                rest.push(name);
                here = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn looked_at(path: &Path) -> Result<Option<bool>, std::io::Error> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.is_dir())),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub fn decide(
    notes: &Resolved<Option<PathBuf>>,
    follows: &[Follow],
    kind: &str,
    config: &Path,
) -> Decision {
    let Some(key_folder) = &notes.value else {
        return Decision {
            plan: Plan::Skip,
            folder: PathBuf::new(),
            followed: false,
        };
    };
    let failed = |folder: &Path, followed: bool, detail: String| Decision {
        plan: Plan::Failed(detail),
        folder: folder.to_path_buf(),
        followed,
    };
    let mut folder = key_folder.clone();
    let mut followed = false;
    if matches!(notes.origin, Origin::Default) {
        for follow in follows
            .iter()
            .filter(|follow| follow.kind.as_deref() == Some(kind))
        {
            match looked_at(&follow.path) {
                Ok(Some(true)) => {
                    folder = follow.path.clone();
                    followed = true;
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    return failed(
                        &follow.path,
                        true,
                        format!("cannot read {}: {error}", follow.path.display()),
                    );
                }
            }
        }
    }
    let canonical = canonical_of(&folder);
    let canonical_config = canonical_of(config);
    if canonical == canonical_config {
        return failed(
            &folder,
            followed,
            format!(
                "{} is the config file {}",
                folder.display(),
                config.display()
            ),
        );
    }
    if canonical.starts_with(&canonical_config) {
        return failed(
            &folder,
            followed,
            format!(
                "{} is under the config file {}",
                folder.display(),
                config.display()
            ),
        );
    }
    match fs::metadata(&folder) {
        Ok(meta) if !meta.is_dir() => {
            return failed(
                &folder,
                followed,
                format!("{} is not a folder", folder.display()),
            );
        }
        Err(error) if error.kind() == ErrorKind::NotADirectory => {
            return failed(
                &folder,
                followed,
                format!("{} is not a folder", folder.display()),
            );
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return failed(
                &folder,
                followed,
                format!("cannot read {}: {error}", folder.display()),
            );
        }
        _ => {}
    }
    let matching: Vec<&Follow> = follows
        .iter()
        .filter(|follow| canonical_of(&follow.path) == canonical)
        .collect();
    let other = matching
        .iter()
        .find_map(|follow| follow.kind.as_deref().filter(|named| *named != kind));
    let plan = if let Some(named) = other {
        Plan::Failed(format!(
            "{} is followed as {named}, and a notes folder is followed as {kind}",
            folder.display()
        ))
    } else if matching.iter().any(|follow| follow.kind.is_none()) {
        Plan::Failed(format!(
            "{} is followed with no kind, and a notes folder is followed as {kind}; give that follow kind = \"{kind}\"",
            folder.display()
        ))
    } else if matching.is_empty() {
        Plan::Save
    } else {
        Plan::Keep
    };
    Decision {
        plan,
        folder,
        followed,
    }
}

pub fn worded(
    answer: &Answer,
    made: bool,
    folder: &Path,
    config: &Path,
) -> (Status, String, Option<Message>) {
    match answer {
        Answer::Saved {
            config,
            defaults,
            unsynced,
        } => {
            let after = if *defaults {
                ", after the default follows it would replace"
            } else {
                ""
            };
            let detail = if made {
                format!(
                    "made {} and followed it in {}{after}",
                    folder.display(),
                    config.display()
                )
            } else {
                format!(
                    "followed {} in {}{after}",
                    folder.display(),
                    config.display()
                )
            };
            let warning = unsynced.as_ref().map(|error| {
                Message::warning(&format!(
                    "saved the follow in {}{after}; its folder could not be synced: {error}",
                    config.display()
                ))
            });
            (Status::Installed, detail, warning)
        }
        Answer::Already => (
            Status::Failed,
            format!(
                "{} came to follow {} while setup ran; bilbo setup looks again",
                config.display(),
                folder.display()
            ),
            None,
        ),
        Answer::Refused { reason, message } => (
            Status::Failed,
            format!("cannot save the follow in {}: {reason}", config.display()),
            Some(message.clone()),
        ),
    }
}

pub struct Keeper<'a> {
    pub decision: Decision,
    pub kind: &'a str,
    pub config: PathBuf,
    pub save: &'a dyn Fn(&Path) -> Answer,
    pub said: Rc<RefCell<Vec<Message>>>,
}

impl Keeper<'_> {
    fn line(status: Status, detail: String) -> Line {
        Line {
            step: "notes",
            status,
            detail,
        }
    }

    fn failed(detail: String) -> Line {
        Keeper::line(Status::Failed, detail)
    }
}

impl Notes for Keeper<'_> {
    fn see_to(&self) -> Line {
        let folder = &self.decision.folder;
        match &self.decision.plan {
            Plan::Skip => {
                return Keeper::line(
                    Status::Skipped,
                    format!("{} is empty", SETUP_NOTES.about.name),
                );
            }
            Plan::Failed(detail) => return Keeper::failed(detail.clone()),
            Plan::Keep | Plan::Save => {}
        }
        let made = match fs::metadata(folder) {
            Ok(meta) if meta.is_dir() => false,
            Ok(_) => return Keeper::failed(format!("{} is not a folder", folder.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if let Err(error) = fs::create_dir_all(folder) {
                    return Keeper::failed(format!("cannot make {}: {error}", folder.display()));
                }
                true
            }
            Err(error) => {
                return Keeper::failed(format!("cannot make {}: {error}", folder.display()));
            }
        };
        let canonical = match fs::canonicalize(folder) {
            Ok(canonical) => canonical,
            Err(error) => {
                return Keeper::failed(format!("cannot read {}: {error}", folder.display()));
            }
        };
        match fs::metadata(&canonical) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Keeper::failed(format!("{} is not a folder", folder.display())),
            Err(error) => {
                return Keeper::failed(format!("cannot read {}: {error}", folder.display()));
            }
        }
        if self.decision.plan == Plan::Keep {
            return if made {
                Keeper::line(Status::Installed, format!("made {}", folder.display()))
            } else {
                Keeper::line(
                    Status::Kept,
                    format!("{} is followed as {}", folder.display(), self.kind),
                )
            };
        }
        let answer = (self.save)(&canonical);
        let (status, detail, message) = worded(&answer, made, &canonical, &self.config);
        self.said.borrow_mut().extend(message);
        Keeper::line(status, detail)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bilbo_setup::report::Status;

    use super::{Answer, worded};
    use crate::console::message::Message;

    fn saved(defaults: bool, unsynced: Option<&str>) -> Answer {
        Answer::Saved {
            config: "/c/config.toml".into(),
            defaults,
            unsynced: unsynced.map(str::to_owned),
        }
    }

    #[test]
    fn the_notes_step_says_each_answer_of_the_saver() {
        let folder = Path::new("/n/notes");
        let config = Path::new("/c/config.toml");
        let after = ", after the default follows it would replace";
        for (made, defaults) in [(true, false), (true, true), (false, false), (false, true)] {
            let (status, detail, warning) = worded(&saved(defaults, None), made, folder, config);
            let tail = if defaults { after } else { "" };
            let head = if made {
                "made /n/notes and followed it in /c/config.toml"
            } else {
                "followed /n/notes in /c/config.toml"
            };
            assert_eq!(
                (status, detail.as_str()),
                (Status::Installed, format!("{head}{tail}").as_str())
            );
            assert!(
                warning.is_none(),
                "the saver's line for a good save is not said"
            );
        }
        for defaults in [false, true] {
            let (status, detail, warning) =
                worded(&saved(defaults, Some("disk full")), true, folder, config);
            assert_eq!(status, Status::Installed);
            assert!(
                detail.starts_with("made /n/notes and followed it"),
                "{detail}"
            );
            let tail = if defaults { after } else { "" };
            assert_eq!(
                warning.map(|message| message.text),
                Some(format!(
                    "saved the follow in /c/config.toml{tail}; its folder could not be synced: disk full"
                ))
            );
        }
        let (status, detail, warning) = worded(&Answer::Already, false, folder, config);
        assert_eq!(
            (status, detail.as_str(), warning),
            (
                Status::Failed,
                "/c/config.toml came to follow /n/notes while setup ran; bilbo setup looks again",
                None
            )
        );
        for (reason, said) in [
            (
                "it is read-only",
                "cannot save the follow in /c/config.toml: it is read-only; add these lines to it:",
            ),
            (
                "it declares follow as an inline array",
                "cannot save the follow in /c/config.toml: it declares follow as an inline array; add this table to its follow array:",
            ),
        ] {
            let message = Message::error(said);
            let (status, detail, warning) = worded(
                &Answer::Refused {
                    reason: reason.to_owned(),
                    message: message.clone(),
                },
                false,
                folder,
                config,
            );
            assert_eq!(status, Status::Failed);
            assert_eq!(
                detail,
                format!("cannot save the follow in /c/config.toml: {reason}")
            );
            assert_eq!(warning, Some(message));
        }
    }
}
