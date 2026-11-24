use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

use bilbo_entries::kind::Kind;
use bilbo_entries::remember::Tagging;
use bilbo_store::id::Id;

use super::note::{self, Draft, Fate, Note, Written};
use crate::console::env::Env;
use crate::console::indexing::Indexing;
use crate::console::message::{Message, sanitize};
use crate::console::outcome::{Exit, Failure, Outcome};
use crate::console::private::{self, How};
use crate::console::settings::Settings;

pub struct Paths {
    pub folder: PathBuf,
    pub file: PathBuf,
}

pub enum Ended {
    Exited(i32),
    Signal,
}

pub trait Steps {
    fn make(&mut self) -> io::Result<()>;
    fn start(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> io::Result<Ended>;
    fn read(&mut self) -> io::Result<Vec<u8>>;
    fn keep(&mut self, text: &str, draft: &mut Draft<'_>) -> Result<Outcome, Failure>;
}

pub fn editor(var: &dyn Fn(&str) -> Option<OsString>) -> Option<OsString> {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| var(name).filter(|value| !value.is_empty()))
}

pub fn command(editor: &OsStr, file: &Path) -> (&'static str, Vec<OsString>) {
    let mut text = editor.to_os_string();
    text.push(" \"$@\"");
    (
        "/bin/sh",
        vec![
            OsString::from("-c"),
            text,
            OsString::from("sh"),
            file.as_os_str().to_owned(),
        ],
    )
}

pub fn remove(folder: &Path) -> Option<Message> {
    fs::remove_dir_all(folder).err().map(|error| {
        Message::warning(&format!(
            "cannot remove the note's folder {}: {error}",
            folder.display()
        ))
    })
}

fn refused(line: &str) -> Failure {
    Failure::Refused(sanitize(line))
}

fn stopped(failure: Failure, warning: Option<Message>) -> Result<Outcome, Failure> {
    match warning {
        None => Err(failure),
        Some(warning) => Ok(Outcome::new("")
            .with_diagnostics(vec![failure.message(), warning])
            .with_exit(Exit::Refused)),
    }
}

pub fn session(
    paths: &Paths,
    steps: &mut dyn Steps,
    remove: &mut dyn FnMut(&Path) -> Option<Message>,
) -> Result<Outcome, Failure> {
    let path = paths.file.display();
    if let Err(error) = steps.make() {
        return stopped(
            refused(&format!("cannot make the note's file {path}: {error}")),
            remove(&paths.folder),
        );
    }
    if let Err(error) = steps.start() {
        return stopped(
            refused(&format!("cannot start the editor: {error}")),
            remove(&paths.folder),
        );
    }
    let failed = match steps.wait() {
        Err(error) => {
            return Err(refused(&format!(
                "cannot wait for the editor: {error}; nothing was kept; its file is {path}"
            )));
        }
        Ok(Ended::Exited(0)) => None,
        Ok(Ended::Exited(code)) => Some(format!("the editor exited with {code}; nothing was kept")),
        Ok(Ended::Signal) => {
            Some("the editor was stopped by a signal; nothing was kept".to_owned())
        }
    };
    let saved = steps.read();
    if let Some(line) = failed {
        return match saved {
            Err(_) => Err(refused(&format!(
                "{line}; its file {path} could not be read"
            ))),
            Ok(bytes) if empty(&bytes) => stopped(refused(&line), remove(&paths.folder)),
            Ok(_) => Err(refused(&format!("{line}; what was saved stays in {path}"))),
        };
    }
    let bytes = match saved {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(refused(&format!(
                "cannot read the note's file {path}: {error}"
            )));
        }
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Err(refused(&format!(
            "refused the note: it is not UTF-8; what was saved stays in {path}"
        )));
    };
    let mut settle = |fate| match fate {
        Fate::Goes => remove(&paths.folder),
        Fate::Stays => None,
    };
    let mut draft = Draft {
        path: &paths.file,
        settle: &mut settle,
    };
    steps.keep(&text, &mut draft)
}

fn empty(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.trim().is_empty())
}

struct Editing<'a, 'b> {
    editor: &'a OsStr,
    paths: &'a Paths,
    folder: fs::File,
    child: Option<Child>,
    id: Id,
    tags: &'a Tagging,
    written: &'a Written,
    env: &'a mut Env,
    settings: &'a Settings,
    kinds: &'a [Box<dyn Kind>],
    indexing: &'a Indexing<'b>,
}

fn ended(status: ExitStatus) -> Ended {
    match status.code() {
        Some(code) => Ended::Exited(code),
        None => Ended::Signal,
    }
}

impl Steps for Editing<'_, '_> {
    fn make(&mut self) -> io::Result<()> {
        private::open(&self.folder, "note.md", &self.paths.file, How::New).map(drop)
    }

    fn start(&mut self) -> io::Result<()> {
        let (program, arguments) = command(self.editor, &self.paths.file);
        self.child = Some(Command::new(program).args(arguments).spawn()?);
        Ok(())
    }

    fn wait(&mut self) -> io::Result<Ended> {
        let Some(child) = self.child.as_mut() else {
            return Err(io::Error::other("no editor was started"));
        };
        child.wait().map(ended)
    }

    fn read(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        private::open(&self.folder, "note.md", &self.paths.file, How::Edited)?
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn keep(&mut self, text: &str, draft: &mut Draft<'_>) -> Result<Outcome, Failure> {
        note::keep(
            Note {
                text,
                id: self.id,
                tags: self.tags,
                draft: Some(draft),
            },
            self.written,
            self.env,
            self.settings,
            self.kinds,
            self.indexing,
        )
    }
}

pub fn run(
    editor: &OsStr,
    tags: &Tagging,
    written: &Written,
    env: &mut Env,
    settings: &Settings,
    kinds: &[Box<dyn Kind>],
    indexing: &Indexing<'_>,
) -> Result<Outcome, Failure> {
    let id = Id::mint().map_err(|error| refused(&error.to_string()))?;
    let name = format!(".bilbo-note-{id}");
    let temp = (env.var)("TMPDIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let base = fs::canonicalize(&temp).unwrap_or(temp);
    let folder = base.join(&name);
    let paths = Paths {
        file: folder.join("note.md"),
        folder,
    };
    let unmade = |error: &io::Error| {
        refused(&format!(
            "cannot make the note's folder {}: {error}",
            paths.folder.display()
        ))
    };
    let parent = fs::File::open(&base).map_err(|error| unmade(&error))?;
    let held = private::folder(&parent, &name, &paths.folder).map_err(|error| unmade(&error))?;
    let mut editing = Editing {
        editor,
        paths: &paths,
        folder: held,
        child: None,
        id,
        tags,
        written,
        env,
        settings,
        kinds,
        indexing,
    };
    session(&paths, &mut editing, &mut |folder| remove(folder))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use bilbo_entries::remember::WrittenAnswer;
    use bilbo_store::entries::StoreError;
    use bilbo_store::id::Id;

    use super::{Ended, Paths, Steps, command, editor, remove, session};
    use crate::console::closing::Closing;
    use crate::console::message::Message;
    use crate::console::outcome::{Failure, Outcome};
    use crate::console::view::View;
    use crate::remember::note::{self, Draft};

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
    const FILE: &str = "/tmp/.bilbo-note-01JC5Q3V7XKM4P8R2T6W9YB0DE/note.md";
    const FOLDER: &str = "/tmp/.bilbo-note-01JC5Q3V7XKM4P8R2T6W9YB0DE";

    fn values(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn visual_is_asked_before_editor_and_an_empty_value_is_unset() {
        let both = values(&[("VISUAL", "code --wait"), ("EDITOR", "vi")]);
        assert_eq!(editor(&both), Some(OsString::from("code --wait")));
        let empty_visual = values(&[("VISUAL", ""), ("EDITOR", "vi")]);
        assert_eq!(editor(&empty_visual), Some(OsString::from("vi")));
        let only_editor = values(&[("EDITOR", "vi")]);
        assert_eq!(editor(&only_editor), Some(OsString::from("vi")));
        assert_eq!(editor(&values(&[])), None);
        assert_eq!(editor(&values(&[("VISUAL", ""), ("EDITOR", "")])), None);
    }

    #[test]
    fn the_scratch_file_is_an_argument_of_the_editor_and_never_part_of_its_command() {
        let path = Path::new("/tmp/a b/it's $HOME/note.md");
        let (program, arguments) = command(OsStr::new("'/my dir/ed' --wait"), path);
        assert_eq!(program, "/bin/sh");
        assert_eq!(arguments[0], OsString::from("-c"));
        assert_eq!(arguments[1], OsString::from("'/my dir/ed' --wait \"$@\""));
        assert!(!arguments[1].to_string_lossy().contains("note.md"));
        assert!(!arguments[1].to_string_lossy().contains("it's"));
        assert_eq!(arguments.last(), Some(&path.as_os_str().to_owned()));
        assert_eq!(arguments.len(), 4);
    }

    #[test]
    fn a_scratch_file_that_cannot_be_removed_is_a_warning() {
        let base = std::env::temp_dir().join(format!("edit-remove-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let folder = base.join(".bilbo-note-x");
        fs::create_dir_all(&folder).expect("a folder");
        fs::write(folder.join("note.md"), "text").expect("a file");
        fs::set_permissions(&base, fs::Permissions::from_mode(0o500)).expect("read-only");
        let warning = remove(&folder).expect("a warning");
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).expect("restored");
        let error = io::Error::from_raw_os_error(13);
        assert_eq!(
            warning.text,
            format!(
                "cannot remove the note's folder {}: {error}",
                folder.display()
            )
        );
        assert_eq!(
            warning.lines(&View::PLAIN),
            [format!(
                "bilbo: cannot remove the note's folder {}: {error}",
                folder.display()
            )]
        );
        let _ = fs::remove_dir_all(&base);
    }

    struct Script {
        make: Option<io::Error>,
        start: Option<io::Error>,
        ended: Option<Ended>,
        lost: Option<io::Error>,
        read: Option<io::Result<Vec<u8>>>,
        answer: Option<WrittenAnswer>,
        reads: Cell<usize>,
    }

    impl Script {
        fn new() -> Script {
            Script {
                make: None,
                start: None,
                ended: Some(Ended::Exited(0)),
                lost: None,
                read: None,
                answer: None,
                reads: Cell::new(0),
            }
        }
    }

    impl Steps for Script {
        fn make(&mut self) -> io::Result<()> {
            self.make.take().map_or(Ok(()), Err)
        }
        fn start(&mut self) -> io::Result<()> {
            self.start.take().map_or(Ok(()), Err)
        }
        fn wait(&mut self) -> io::Result<Ended> {
            match self.lost.take() {
                Some(error) => Err(error),
                None => Ok(self.ended.take().expect("an end")),
            }
        }
        fn read(&mut self) -> io::Result<Vec<u8>> {
            self.reads.set(self.reads.get() + 1);
            self.read.take().expect("a read")
        }
        fn keep(&mut self, text: &str, draft: &mut Draft<'_>) -> Result<Outcome, Failure> {
            match self.answer.take() {
                None => {
                    assert!(text.trim().is_empty(), "{text:?}");
                    Ok(note::nothing_to_keep(Some(draft)))
                }
                Some(answer) => note::answered(answer, &[], Closing::default(), Some(draft)),
            }
        }
    }

    struct Held {
        lines: Vec<String>,
        code: u8,
        removed: Vec<PathBuf>,
        reads: usize,
    }

    fn conclude(mut script: Script, warning: Option<&str>) -> Held {
        let paths = Paths {
            folder: PathBuf::from(FOLDER),
            file: PathBuf::from(FILE),
        };
        let removed = RefCell::new(Vec::new());
        let result = session(&paths, &mut script, &mut |folder| {
            removed.borrow_mut().push(folder.to_path_buf());
            warning.map(Message::warning)
        });
        let (lines, code) = match &result {
            Ok(outcome) => (outcome.stderr_lines(&View::PLAIN), outcome.exit_code()),
            Err(failure) => (failure.stderr_lines(&View::PLAIN), failure.exit_code()),
        };
        Held {
            lines,
            code,
            removed: removed.into_inner(),
            reads: script.reads.get(),
        }
    }

    fn id() -> Id {
        Id::parse(ID).expect("an id")
    }

    fn stays() -> Vec<PathBuf> {
        Vec::new()
    }

    fn goes() -> Vec<PathBuf> {
        vec![PathBuf::from(FOLDER)]
    }

    fn stay() -> String {
        format!("; what was saved stays in {FILE}")
    }

    #[test]
    fn what_becomes_of_the_draft_follows_what_was_answered() {
        let said = |held: &Held, lines: &[String], code: u8, removed: Vec<PathBuf>, row: &str| {
            assert_eq!(held.lines, lines, "row {row}");
            assert_eq!(held.code, code, "row {row}");
            assert_eq!(held.removed, removed, "row {row}");
        };

        let mut script = Script::new();
        script.make = Some(io::Error::other("no room"));
        let held = conclude(script, None);
        said(
            &held,
            &[format!(
                "bilbo: cannot make the note's file {FILE}: no room"
            )],
            1,
            goes(),
            "3",
        );

        let mut script = Script::new();
        script.start = Some(io::Error::other("no sh"));
        let held = conclude(script, None);
        said(
            &held,
            &["bilbo: cannot start the editor: no sh".to_owned()],
            1,
            goes(),
            "4",
        );

        let mut script = Script::new();
        script.lost = Some(io::Error::other("lost"));
        script.read = Some(Ok(b"a sentence".to_vec()));
        let held = conclude(script, None);
        said(
            &held,
            &[format!(
                "bilbo: cannot wait for the editor: lost; nothing was kept; its file is {FILE}"
            )],
            1,
            stays(),
            "13",
        );
        assert_eq!(held.reads, 0, "row 13 reads nothing");

        let exited = "bilbo: the editor exited with 3; nothing was kept";
        for (ended, line) in [
            (Ended::Exited(3), exited),
            (
                Ended::Signal,
                "bilbo: the editor was stopped by a signal; nothing was kept",
            ),
        ] {
            let mut script = Script::new();
            script.ended = Some(ended);
            script.read = Some(Ok(b"a sentence".to_vec()));
            let held = conclude(script, None);
            said(&held, &[format!("{line}{}", stay())], 1, stays(), "5a");
        }
        let mut script = Script::new();
        script.ended = Some(Ended::Exited(3));
        script.read = Some(Ok(vec![0xff, 0xfe]));
        let held = conclude(script, None);
        said(
            &held,
            &[format!("{exited}{}", stay())],
            1,
            stays(),
            "5a, bytes that are not UTF-8: the editor's line, not row 8's",
        );
        for bytes in [
            &b""[..],
            b"  
	
",
        ] {
            let mut script = Script::new();
            script.ended = Some(Ended::Exited(3));
            script.read = Some(Ok(bytes.to_vec()));
            let held = conclude(script, None);
            said(&held, &[exited.to_owned()], 1, goes(), "5b");
        }
        let mut script = Script::new();
        script.ended = Some(Ended::Exited(3));
        script.read = Some(Err(io::Error::from(io::ErrorKind::NotFound)));
        let held = conclude(script, None);
        said(
            &held,
            &[format!("{exited}; its file {FILE} could not be read")],
            1,
            stays(),
            "5c, a file that is gone: the editor's line, not row 6's",
        );

        let mut script = Script::new();
        script.read = Some(Err(io::Error::other("denied")));
        let held = conclude(script, None);
        said(
            &held,
            &[format!("bilbo: cannot read the note's file {FILE}: denied")],
            1,
            stays(),
            "6",
        );

        let mut script = Script::new();
        script.read = Some(Ok(b" 
 "
        .to_vec()));
        let held = conclude(script, None);
        said(
            &held,
            &["bilbo: nothing to keep: the note is empty".to_owned()],
            1,
            goes(),
            "7",
        );

        let mut script = Script::new();
        script.read = Some(Ok(vec![0xff, 0xfe]));
        let held = conclude(script, None);
        said(
            &held,
            &[format!(
                "bilbo: refused the note: it is not UTF-8{}",
                stay()
            )],
            1,
            stays(),
            "8",
        );

        let mut script = Script::new();
        script.read = Some(Ok(b"a sentence".to_vec()));
        script.answer = Some(WrittenAnswer::BeforeCapture(StoreError::Missing(
            "no store".to_owned(),
        )));
        let held = conclude(script, None);
        said(
            &held,
            &[format!("bilbo: no store{}", stay())],
            1,
            stays(),
            "9",
        );
        assert!(!held.lines[0].contains("may have been kept"));

        let mut script = Script::new();
        script.read = Some(Ok(b"a sentence".to_vec()));
        script.answer = Some(WrittenAnswer::Refused("its id x is taken".to_owned()));
        let held = conclude(script, None);
        said(
            &held,
            &[format!(
                "bilbo: refused the note: its id x is taken{}",
                stay()
            )],
            1,
            stays(),
            "10",
        );

        let mut script = Script::new();
        script.read = Some(Ok(b"a sentence".to_vec()));
        script.answer = Some(WrittenAnswer::AtCapture {
            id: id(),
            error: StoreError::Missing("no store".to_owned()),
        });
        let held = conclude(script, None);
        said(
            &held,
            &[format!(
                "bilbo: no store; the note may have been kept as {ID}{}",
                stay()
            )],
            1,
            stays(),
            "11",
        );

        let mut script = Script::new();
        script.read = Some(Ok(b"a sentence".to_vec()));
        script.answer = Some(WrittenAnswer::Kept {
            id: id(),
            records: 1,
            raw_bytes: 10,
            stored_bytes: 40,
            tagged: false,
        });
        let held = conclude(script, None);
        said(
            &held,
            &[format!("bilbo: kept the note as {ID}")],
            0,
            goes(),
            "12",
        );

        let mut script = Script::new();
        script.read = Some(Ok(b"a sentence".to_vec()));
        script.answer = Some(WrittenAnswer::Kept {
            id: id(),
            records: 1,
            raw_bytes: 10,
            stored_bytes: 40,
            tagged: false,
        });
        let held = conclude(script, Some("cannot remove the folder"));
        said(
            &held,
            &[
                format!("bilbo: kept the note as {ID}"),
                "bilbo: cannot remove the folder".to_owned(),
            ],
            0,
            goes(),
            "12, a removal that fails changes no exit code",
        );

        let mut script = Script::new();
        script.ended = Some(Ended::Exited(3));
        script.read = Some(Ok(Vec::new()));
        let held = conclude(script, Some("cannot remove the folder"));
        said(
            &held,
            &[
                exited.to_owned(),
                "bilbo: cannot remove the folder".to_owned(),
            ],
            1,
            goes(),
            "5b, a removal that fails follows the run's line",
        );
    }
}
