use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::shared::frontmatter;
use crate::shared::store::{self, EntryKind};
use crate::{Failure, note};

struct Request {
    kind: String,
    topic: String,
    title: String,
}

/// The created note's absolute path, and a stderr line when there is one to print.
pub struct Output {
    pub path: PathBuf,
    pub warning: Option<String>,
}

/// Parses and validates args, creates the note.
pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let request = parse(args)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    refuse_taken_topic(&notes, &request.topic)?;
    fs::create_dir_all(&notes)
        .map_err(|e| Failure::Refused(format!("cannot create {}: {e}", notes.display())))?;

    let id = frontmatter::mint_ulid()
        .map_err(|e| Failure::Refused(format!("cannot read /dev/urandom: {e}")))?;
    let text = note::render(&id, &frontmatter::now_created(), &request.title);
    if let Some(problem) = note::read(&text).problems.first() {
        return Err(Failure::Refused(format!(
            "internal error: the rendered note has problems: {problem}"
        )));
    }

    let file_name = format!("{}-{}.md", request.kind, request.topic);
    write_new(&notes, &file_name, &request.topic, &id, &text).map_err(|e| match e {
        WriteError::Taken(existing) => taken(&request.topic, &existing),
        WriteError::List { notes, source } => {
            Failure::Refused(format!("cannot read {}: {source}", notes.display()))
        }
        WriteError::Io { path, source } => {
            Failure::Refused(format!("cannot write {}: {source}", path.display()))
        }
    })?;
    Ok(Output {
        path: notes.join(file_name),
        warning: None,
    })
}

fn taken(topic: &str, existing: &Path) -> Failure {
    Failure::Refused(format!(
        "topic '{topic}' already has a note: {}",
        existing.display()
    ))
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

fn parse(args: &[String]) -> Result<Request, Failure> {
    let mut positional: Vec<&str> = Vec::new();
    let mut title: Option<String> = None;
    let mut options_ended = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if options_ended || !arg.starts_with('-') || arg == "-" {
            positional.push(arg);
        } else if arg == "--" {
            options_ended = true;
        } else {
            let value = if arg == "--title" {
                iter.next()
                    .ok_or_else(|| usage("--title needs a value"))?
                    .clone()
            } else if let Some(value) = arg.strip_prefix("--title=") {
                value.to_string()
            } else {
                return Err(usage(format!("unknown option '{arg}'")));
            };
            if title.replace(value).is_some() {
                return Err(usage("--title given more than once"));
            }
        }
    }

    let (kind, topic) = match positional.as_slice() {
        [] => return Err(usage("missing <kind> and <topic>")),
        [_] => return Err(usage("missing <topic>")),
        [kind, topic] => (*kind, *topic),
        [_, _, extra, ..] => return Err(usage(format!("unexpected argument '{extra}'"))),
    };
    if !note::KINDS.contains(&kind) {
        return Err(usage(format!(
            "unknown kind '{kind}'; kinds: {}",
            note::kinds_list()
        )));
    }
    if !store::is_topic(topic) {
        return Err(usage(format!(
            "invalid topic '{topic}': use segments of a-z and 0-9 joined by single hyphens"
        )));
    }
    let title = match title {
        Some(t) if t.trim().is_empty() => return Err(usage("--title must not be empty")),
        Some(t) if t.contains(['\n', '\r']) => return Err(usage("--title must be one line")),
        Some(t) => t,
        None => note::default_title(topic),
    };
    Ok(Request {
        kind: kind.into(),
        topic: topic.into(),
        title,
    })
}

fn refuse_taken_topic(notes: &Path, topic: &str) -> Result<(), Failure> {
    if !notes.is_dir() {
        return Ok(());
    }
    match find_taken(notes, topic) {
        Ok(None) => Ok(()),
        Ok(Some(existing)) => Err(taken(topic, &existing)),
        Err(e) => Err(Failure::Refused(format!(
            "cannot read {}: {e}",
            notes.display()
        ))),
    }
}

/// The note in `notes` that already holds `topic`, under any kind.
fn find_taken(notes: &Path, topic: &str) -> io::Result<Option<PathBuf>> {
    let found = store::entries(notes)?.into_iter().find(|entry| {
        entry.kind != EntryKind::Folder
            && entry.utf8
            && note::parse_name(&entry.name).is_ok_and(|n| n.topic == topic)
    });
    Ok(found.map(|entry| entry.path))
}

enum WriteError {
    /// Another note holds the topic; names it.
    Taken(PathBuf),
    /// Listing the notes folder failed.
    List {
        notes: PathBuf,
        source: io::Error,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

fn io_error(path: &Path, source: io::Error) -> WriteError {
    WriteError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Writes `.new-<id>.tmp` in `dir`, fsyncs, checks the topic once more, hard-links the temp file to
/// `dir/file_name` and removes it on every path after its creation.
fn write_new(
    dir: &Path,
    file_name: &str,
    topic: &str,
    id: &str,
    text: &str,
) -> Result<(), WriteError> {
    let tmp = dir.join(format!(".new-{id}.tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| io_error(&tmp, e))?;
    let linked = publish(&mut file, &tmp, dir, file_name, topic, text);
    drop(file);
    // A failed unlink leaves a hidden temp file; the note itself already exists.
    let _ = fs::remove_file(&tmp);
    linked
}

fn publish(
    file: &mut fs::File,
    tmp: &Path,
    dir: &Path,
    file_name: &str,
    topic: &str,
    text: &str,
) -> Result<(), WriteError> {
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| io_error(tmp, e))?;
    // The fsync is slow; a rival note may have landed meanwhile. The link below only catches the same file name.
    if let Some(existing) = find_taken(dir, topic).map_err(|source| WriteError::List {
        notes: dir.to_path_buf(),
        source,
    })? {
        return Err(WriteError::Taken(existing));
    }
    let target = dir.join(file_name);
    fs::hard_link(tmp, &target).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => WriteError::Taken(target),
        _ => io_error(&target, e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn write_new_refuses_an_existing_target() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-write-new-{}", std::process::id())));
        let dir = &scratch.0;
        let _ = fs::remove_dir_all(dir);
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("plan-a.md"), "keep me").unwrap();

        let err = write_new(dir, "plan-a.md", "other", "ID", "new text").unwrap_err();

        assert!(matches!(err, WriteError::Taken(p) if p == dir.join("plan-a.md")));
        assert_eq!(fs::read(dir.join("plan-a.md")).unwrap(), b"keep me");
        let left: Vec<_> = fs::read_dir(dir).unwrap().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn write_new_refuses_a_rival_topic_and_cleans_up() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-write-rival-{}", std::process::id())));
        let dir = &scratch.0;
        let _ = fs::remove_dir_all(dir);
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("decision-a.md"), "keep me").unwrap();

        let err = write_new(dir, "plan-a.md", "a", "ID", "new text").unwrap_err();

        assert!(matches!(err, WriteError::Taken(p) if p == dir.join("decision-a.md")));
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
        assert!(!dir.join("plan-a.md").exists());
    }

    #[test]
    fn an_existing_temp_file_is_a_write_error_not_a_taken_topic() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-write-tmp-{}", std::process::id())));
        let dir = &scratch.0;
        let _ = fs::remove_dir_all(dir);
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(".new-ID.tmp"), "other run").unwrap();

        let err = write_new(dir, "plan-a.md", "a", "ID", "text").unwrap_err();

        assert!(matches!(err, WriteError::Io { .. }));
        assert_eq!(fs::read(dir.join(".new-ID.tmp")).unwrap(), b"other run");
    }
}
