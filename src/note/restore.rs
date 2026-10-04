//! `bilbo restore`: puts a past version of a note back as its newest version. The whole sequence runs under
//! `history/lock` and replaces the file with an atomic exchange, so no edit is lost and no two visible files ever
//! hold the note's id.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::Failure;
use crate::host::swap;
use crate::note::parse_name;
use crate::note::versions::{
    self, ContentError, Found, Lock, NameError, Scan, Version, VersionError,
};
use crate::shared::hash;
use crate::shared::store;

/// What `apply` is about to do when it calls its hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Write the version's bytes to the hidden file.
    Write,
    /// Exchange the hidden file with the note's file, or rename it into place when the note has none.
    Swap,
    /// Rename the note's file, now holding the restored bytes, to the version's name.
    Rename,
    /// Read what the exchange took out.
    Inspect,
    /// Delete the hidden file.
    Remove,
    /// Record the `restored` version.
    Record,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// An exit-2 message.
    Usage(String),
    /// An exit-1 message; the sweep's lines come first.
    Refused(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Restored { file: String, version: String },
    Matches { file: String, version: String },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Done {
    /// stderr lines (without "bilbo: "): what the sweep recorded.
    pub messages: Vec<String>,
    pub outcome: Outcome,
}

pub struct Output {
    pub warnings: Vec<String>,
    pub lines: Vec<String>,
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let (note, version) = parse(args)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let done = apply(&root, &note, &version, &mut |_| Ok(())).map_err(|e| match e {
        Error::Usage(message) => Failure::Usage(message),
        Error::Refused(message) => Failure::Refused(message),
    })?;
    let line = match done.outcome {
        Outcome::Restored { file, version } => format!("restored {file} to {version}"),
        Outcome::Matches { file, version } => format!("{file} already matches {version}"),
    };
    Ok(Output {
        warnings: done.messages,
        lines: vec![line],
    })
}

fn parse(args: &[String]) -> Result<(String, String), Failure> {
    if let Some(option) = args.iter().find(|a| a.starts_with('-')) {
        return Err(Failure::Usage(format!("unknown option '{option}'")));
    }
    match args {
        [note, version] => Ok((note.clone(), version.clone())),
        [] | [_] => Err(Failure::Usage("restore needs a note and a version".into())),
        [_, _, extra, ..] => Err(Failure::Usage(format!("unexpected argument '{extra}'"))),
    }
}

/// Restores `version` (a prefix of its id) of `note` (an id or a topic). `hook` is called before each `Step`; an
/// error from it stops the restore where it is, without cleanup, as a kill would.
pub fn apply(
    root: &Path,
    note: &str,
    version: &str,
    hook: &mut dyn FnMut(Step) -> Result<(), String>,
) -> Result<Done, Error> {
    if !root.join("notes").is_dir() {
        return Err(Error::Refused(format!("no store at {}", root.display())));
    }
    if versions::find_version(&[], version) == Err(VersionError::Invalid) {
        return Err(Error::Usage(not_a_version(version)));
    }
    let lock = versions::lock(root).map_err(Error::Refused)?;
    let at = versions::now_at();
    let mut messages = versions::sweep_restore_leftovers(&lock, &at).map_err(Error::Refused)?;
    match restore(&lock, note, version, &at, hook) {
        Ok(outcome) => Ok(Done { messages, outcome }),
        Err(Error::Refused(message)) => {
            messages.push(message);
            Err(Error::Refused(messages.join("\n")))
        }
        Err(Error::Usage(message)) => {
            messages.push(message);
            Err(Error::Usage(messages.join("\n")))
        }
    }
}

fn not_a_version(prefix: &str) -> String {
    format!("'{prefix}' is not a version: use 6 to 64 hexadecimal characters of its id")
}

fn refused<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error::Refused(message.into()))
}

fn swap_error(message: String) -> Error {
    if message == swap::UNSUPPORTED {
        Error::Refused(format!("cannot restore on this filesystem: {message}"))
    } else {
        Error::Refused(message)
    }
}

fn restore(
    lock: &Lock,
    note: &str,
    prefix: &str,
    at: &str,
    hook: &mut dyn FnMut(Step) -> Result<(), String>,
) -> Result<Outcome, Error> {
    let root = lock.root();
    let notes = root.join("notes");
    let scan = versions::scan(&notes)
        .or_else(|e| refused(format!("cannot read {}: {e}", notes.display())))?;
    let named = versions::resolve(root, note, &scan).map_err(|e| match e {
        NameError::Usage(message) => Error::Usage(message),
        NameError::NoHistory => Error::Refused(format!("no history for {note}")),
        NameError::Failed(message) => Error::Refused(message),
    })?;
    let log = versions::load(root, &named.id).map_err(Error::Refused)?;
    let picked = pick(&log.versions, prefix, note)?;
    let bytes = versions::content(root, picked).map_err(|e| match e {
        ContentError::Deleted => Error::Refused(format!(
            "version {prefix} of {note} is a deletion; delete the file instead"
        )),
        ContentError::Pruned => Error::Refused(format!("version {prefix} of {note} was pruned")),
        ContentError::Io(message) => Error::Refused(message),
    })?;
    let current = scan.notes.get(&named.id);
    if current.is_none() && !named.skipped.is_empty() {
        let files: Vec<String> = named.skipped.iter().map(|n| format!("notes/{n}")).collect();
        return refused(format!(
            "{note} is held by {}, which is not recorded; cannot restore it",
            files.join(", ")
        ));
    }
    if current.is_none() && scan.has_unreadable() {
        return refused(format!(
            "a file of notes/ cannot be read, and it may hold {note}; cannot restore it"
        ));
    }
    let short = picked.short().to_string();
    if let Some(found) = current
        && found.name == picked.file
        && hash::sha256_hex(&found.bytes) == picked.blob
    {
        return Ok(Outcome::Matches {
            file: picked.file.clone(),
            version: short,
        });
    }
    if current.map(|f| f.name.as_str()) != Some(picked.file.as_str())
        && is_taken(&notes, &scan, &named.id, &picked.file)
    {
        return refused(format!("{} is taken by another note", picked.file));
    }
    versions::record_difference(
        lock,
        &named.id,
        current.map(|f| (f.name.as_str(), f.bytes.as_slice())),
        at,
    )
    .map_err(Error::Refused)?;

    let temp = versions::restore_path(root, &named.id);
    let target = notes.join(&picked.file);
    hook(Step::Write).map_err(Error::Refused)?;
    write_temp(&temp, &bytes, current).map_err(Error::Refused)?;
    hook(Step::Swap).map_err(Error::Refused)?;
    let Some(found) = current else {
        swap::rename_new(&temp, &target).map_err(|e| {
            let _ = fs::remove_file(&temp);
            if target.exists() {
                Error::Refused(format!("{} is taken by another note", picked.file))
            } else {
                swap_error(e)
            }
        })?;
        let lead = format!("restored {} to {short}", picked.file);
        finish(lock, &named.id, &picked.file, &bytes, &temp, at, hook)
            .map_err(|s| s.error(&lead))?;
        return Ok(restored(&picked.file, &short));
    };
    let old = notes.join(&found.name);
    swap::exchange(&old, &temp).map_err(|e| {
        let _ = fs::remove_file(&temp);
        swap_error(e)
    })?;
    let mut file = picked.file.as_str();
    let mut late = None;
    if found.name != picked.file {
        hook(Step::Rename).map_err(Error::Refused)?;
        if let Err(e) = swap::rename_new(&old, &target) {
            file = &found.name;
            late = Some(if target.exists() {
                format!("; {} is taken by another note", picked.file)
            } else {
                format!(", but {e}")
            });
        }
    }
    let lead = format!("restored {file} to {short}");
    hook(Step::Inspect).map_err(Error::Refused)?;
    let out = fs::read(&temp)
        .map_err(|e| Error::Refused(format!("{lead}, but cannot read {}: {e}", temp.display())))?;
    if out != found.bytes {
        versions::record_difference(lock, &named.id, Some((&found.name, &out)), at)
            .map_err(|e| Error::Refused(format!("{lead}, but {e}")))?;
    }
    finish(lock, &named.id, file, &bytes, &temp, at, hook).map_err(|s| s.error(&lead))?;
    match late {
        Some(tail) => refused(format!("{lead}{tail}")),
        None => Ok(restored(file, &short)),
    }
}

fn restored(file: &str, version: &str) -> Outcome {
    Outcome::Restored {
        file: file.to_string(),
        version: version.to_string(),
    }
}

/// Why `finish` stopped: the hook (a simulated kill) or a real failure after the file was replaced.
enum Stop {
    Hook(String),
    Failed(String),
}

impl Stop {
    fn error(self, lead: &str) -> Error {
        match self {
            Stop::Hook(message) => Error::Refused(message),
            Stop::Failed(message) => Error::Refused(format!("{lead}, but {message}")),
        }
    }
}

/// Deletes the hidden file and records `restored`.
fn finish(
    lock: &Lock,
    id: &str,
    file: &str,
    bytes: &[u8],
    temp: &Path,
    at: &str,
    hook: &mut dyn FnMut(Step) -> Result<(), String>,
) -> Result<(), Stop> {
    hook(Step::Remove).map_err(Stop::Hook)?;
    match fs::remove_file(temp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(Stop::Failed(format!(
                "cannot remove {}: {e}",
                temp.display()
            )));
        }
        _ => {}
    }
    hook(Step::Record).map_err(Stop::Hook)?;
    versions::record(lock, id, file, Some(bytes), versions::RESTORED, at).map_err(Stop::Failed)?;
    Ok(())
}

fn pick<'a>(versions: &'a [Version], prefix: &str, note: &str) -> Result<&'a Version, Error> {
    versions::find_version(versions, prefix).map_err(|e| match e {
        VersionError::Invalid => Error::Usage(not_a_version(prefix)),
        VersionError::Ambiguous(found) => {
            let lines: Vec<String> = found
                .iter()
                .map(|v| format!("{} {} {} {}", v.short(), v.minute(), v.event, v.file))
                .collect();
            Error::Usage(format!(
                "version {prefix} of {note} matches more than one version:\n{}",
                lines.join("\n")
            ))
        }
        VersionError::Missing => Error::Refused(format!("no version {prefix} of {note}")),
    })
}

/// Whether `name`, the version's file, would collide with another note: a file by that name, or a file of another
/// note, or one that holds no valid id, with the same topic under any kind.
fn is_taken(notes: &Path, scan: &Scan, id: &str, name: &str) -> bool {
    if notes.join(name).exists() {
        return true;
    }
    let Ok(topic) = parse_name(name).map(|n| n.topic) else {
        return false;
    };
    let found = scan.notes.values().map(|f| (&f.name, Some(&f.id)));
    let skipped = scan.skipped.iter().map(|s| (&s.name, s.id.as_ref()));
    found.chain(skipped).any(|(other, other_id)| {
        other_id.map(String::as_str) != Some(id)
            && parse_name(other).is_ok_and(|n| n.topic == topic)
    })
}

/// Writes the version's bytes to the hidden file, with the permissions of the file it will replace.
fn write_temp(temp: &Path, bytes: &[u8], current: Option<&Found>) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", temp.display());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(fail)?;
    // The file is ours from here on, so a failure removes it.
    let filled = (|| {
        file.write_all(bytes)?;
        if let Some(found) = current
            && let Ok(meta) = fs::metadata(temp.with_file_name(&found.name))
        {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()
    })();
    filled.map_err(|e| {
        let _ = fs::remove_file(temp);
        fail(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::versions::{ADDED, EDITED, RENAMED, load, record_difference};
    use std::path::PathBuf;

    const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4D01";
    const OTHER: &str = "01M3YJ7R6HK6NQ30DCDB1P4D02";

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-restore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("notes")).unwrap();
        Scratch(dir)
    }

    fn text(id: &str, body: &str) -> Vec<u8> {
        format!("---\nid: {id}\ncreated: 2026-10-02T14:23-03:00\n---\n\n# T\n\n{body}\n")
            .into_bytes()
    }

    fn put(root: &Path, name: &str, id: &str, body: &str) {
        fs::write(root.join("notes").join(name), text(id, body)).unwrap();
    }

    /// What `bilbo watch` would record for the file as it is now.
    fn watch(root: &Path, name: &str, id: &str) {
        let bytes = fs::read(root.join("notes").join(name)).unwrap();
        let lock = versions::lock(root).unwrap();
        record_difference(&lock, id, Some((name, &bytes)), &versions::now_at()).unwrap();
    }

    /// A note recorded as `added` with "one", then `edited` with "two"; returns the first version's prefix.
    fn fixture(scratch: &Scratch) -> (PathBuf, String) {
        let root = scratch.0.clone();
        put(&root, "decision-release.md", ID, "one");
        watch(&root, "decision-release.md", ID);
        put(&root, "decision-release.md", ID, "two");
        watch(&root, "decision-release.md", ID);
        let first = load(&root, ID).unwrap().versions[0].short().to_string();
        (root, first)
    }

    /// The same, then renamed to `plan-release.md` and recorded as `renamed`.
    fn renamed_fixture(scratch: &Scratch) -> (PathBuf, String) {
        let (root, first) = fixture(scratch);
        let notes = root.join("notes");
        fs::rename(
            notes.join("decision-release.md"),
            notes.join("plan-release.md"),
        )
        .unwrap();
        watch(&root, "plan-release.md", ID);
        (root, first)
    }

    fn events(root: &Path) -> Vec<String> {
        load(root, ID)
            .unwrap()
            .versions
            .iter()
            .map(|v| format!("{} {}", v.event, v.file))
            .collect()
    }

    fn files(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join("notes"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn read(root: &Path, name: &str) -> Vec<u8> {
        fs::read(root.join("notes").join(name)).unwrap()
    }

    fn go(root: &Path, version: &str) -> Result<Done, Error> {
        apply(root, "release", version, &mut |_| Ok(()))
    }

    fn crash_at(at: Step) -> impl FnMut(Step) -> Result<(), String> {
        move |step| {
            if step == at {
                Err(format!("killed before {step:?}"))
            } else {
                Ok(())
            }
        }
    }

    fn restore_file(id: &str) -> String {
        format!(".bilbo-restore-{id}")
    }

    #[test]
    fn restores_a_version_as_the_newest() {
        let s = scratch("plain");
        let (root, first) = fixture(&s);
        let done = go(&root, &first).unwrap();
        assert_eq!(
            done.outcome,
            Outcome::Restored {
                file: "decision-release.md".into(),
                version: first.clone()
            }
        );
        assert!(done.messages.is_empty());
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(
            events(&root),
            [
                "added decision-release.md",
                "edited decision-release.md",
                "restored decision-release.md"
            ]
        );
        let log = load(&root, ID).unwrap();
        assert_eq!(log.versions[2].parents, [log.versions[1].version.clone()]);
    }

    #[test]
    fn a_longer_prefix_prints_the_listed_form() {
        let s = scratch("prefix");
        let (root, _) = fixture(&s);
        let full = load(&root, ID).unwrap().versions[0].version.clone();
        let done = go(&root, &full).unwrap();
        let Outcome::Restored { version, .. } = done.outcome else {
            panic!("not restored");
        };
        assert_eq!(version, full[..12]);
    }

    #[test]
    fn an_unrecorded_edit_is_recorded_first() {
        let s = scratch("unrecorded");
        let (root, first) = fixture(&s);
        put(&root, "decision-release.md", ID, "three");
        go(&root, &first).unwrap();
        assert_eq!(
            events(&root),
            [
                "added decision-release.md",
                "edited decision-release.md",
                "edited decision-release.md",
                "restored decision-release.md"
            ]
        );
        let log = load(&root, ID).unwrap();
        assert_eq!(
            versions::content(&root, &log.versions[2]).unwrap(),
            text(ID, "three")
        );
    }

    #[test]
    fn a_file_that_already_matches_is_left_alone() {
        let s = scratch("matches");
        let (root, _) = fixture(&s);
        let second = load(&root, ID).unwrap().versions[1].short().to_string();
        let before = events(&root);
        let done = go(&root, &second).unwrap();
        assert_eq!(
            done.outcome,
            Outcome::Matches {
                file: "decision-release.md".into(),
                version: second
            }
        );
        assert_eq!(events(&root), before);
    }

    #[test]
    fn a_write_during_the_restore_is_recorded() {
        let s = scratch("race");
        let (root, first) = fixture(&s);
        let target = root.join("notes/decision-release.md");
        let mut hook = |step: Step| {
            if step == Step::Swap {
                fs::write(&target, text(ID, "agent")).unwrap();
            }
            Ok(())
        };
        apply(&root, "release", &first, &mut hook).unwrap();
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
        assert_eq!(files(&root), ["decision-release.md"]);
        let log = load(&root, ID).unwrap();
        let events: Vec<&str> = log.versions.iter().map(|v| v.event.as_str()).collect();
        assert_eq!(events, [ADDED, EDITED, EDITED, "restored"]);
        assert_eq!(
            versions::content(&root, &log.versions[2]).unwrap(),
            text(ID, "agent")
        );
    }

    #[test]
    fn a_kill_before_the_hidden_file_changes_nothing() {
        let s = scratch("kill-write");
        let (root, first) = fixture(&s);
        let err = apply(&root, "release", &first, &mut crash_at(Step::Write)).unwrap_err();
        assert_eq!(err, Error::Refused("killed before Write".into()));
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(read(&root, "decision-release.md"), text(ID, "two"));
        go(&root, &first).unwrap();
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
    }

    #[test]
    fn a_kill_before_the_swap_leaves_the_note_untouched() {
        let s = scratch("kill-swap");
        let (root, first) = fixture(&s);
        apply(&root, "release", &first, &mut crash_at(Step::Swap)).unwrap_err();
        assert_eq!(
            files(&root),
            [restore_file(ID), "decision-release.md".to_string()]
        );
        assert_eq!(read(&root, "decision-release.md"), text(ID, "two"));
        assert_eq!(read(&root, &restore_file(ID)), text(ID, "one"));
        let done = go(&root, &first).unwrap();
        assert!(
            done.messages.is_empty(),
            "the hidden file is already a version"
        );
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
    }

    #[test]
    fn a_kill_between_the_swap_and_the_rename_leaves_one_visible_file() {
        let s = scratch("kill-rename");
        let (root, first) = renamed_fixture(&s);
        apply(&root, "release", &first, &mut crash_at(Step::Rename)).unwrap_err();
        assert_eq!(
            files(&root),
            [restore_file(ID), "plan-release.md".to_string()]
        );
        assert_eq!(read(&root, "plan-release.md"), text(ID, "one"));
        assert_eq!(read(&root, &restore_file(ID)), text(ID, "two"));
        let scan = versions::scan(&root.join("notes")).unwrap();
        assert!(scan.skipped.is_empty());
        assert_eq!(scan.notes[ID].name, "plan-release.md");
        // The watcher's sweep deletes the hidden file silently, then records the file as an edit.
        let lock = versions::lock(&root).unwrap();
        let messages = versions::sweep_restore_leftovers(&lock, &versions::now_at()).unwrap();
        assert!(messages.is_empty());
        drop(lock);
        assert_eq!(files(&root), ["plan-release.md"]);
        watch(&root, "plan-release.md", ID);
        assert_eq!(events(&root).last().unwrap(), "edited plan-release.md");
        // A fresh restore finishes the job.
        go(&root, &first).unwrap();
        assert_eq!(files(&root), ["decision-release.md"]);
    }

    #[test]
    fn a_kill_after_the_swap_keeps_what_came_out() {
        for step in [Step::Inspect, Step::Remove, Step::Record] {
            let s = scratch(&format!("kill-{step:?}"));
            let (root, first) = fixture(&s);
            let target = root.join("notes/decision-release.md");
            let mut hook = |at: Step| {
                if at == Step::Swap {
                    fs::write(&target, text(ID, "agent")).unwrap();
                }
                crash_at(step)(at)
            };
            apply(&root, "release", &first, &mut hook).unwrap_err();
            assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
            let hidden = files(&root).contains(&restore_file(ID));
            assert_eq!(hidden, step != Step::Record, "{step:?}");
            if hidden {
                assert_eq!(read(&root, &restore_file(ID)), text(ID, "agent"));
            }
            // The next restore (or watcher) finds the agent's bytes in a version.
            let lock = versions::lock(&root).unwrap();
            let messages = versions::sweep_restore_leftovers(&lock, &versions::now_at()).unwrap();
            drop(lock);
            // Only a kill before the inspection leaves the agent's bytes unrecorded.
            assert_eq!(
                messages.len(),
                usize::from(step == Step::Inspect),
                "{step:?}"
            );
            assert_eq!(files(&root), ["decision-release.md"], "{step:?}");
            let agent = load(&root, ID)
                .unwrap()
                .versions
                .iter()
                .any(|v| versions::content(&root, v).is_ok_and(|b| b == text(ID, "agent")));
            assert!(agent, "{step:?}: the agent's write is in history");
        }
    }

    #[test]
    fn a_kill_after_the_remove_is_picked_up_by_the_next_scan() {
        let s = scratch("kill-record");
        let (root, first) = fixture(&s);
        apply(&root, "release", &first, &mut crash_at(Step::Record)).unwrap_err();
        assert_eq!(files(&root), ["decision-release.md"]);
        let done = go(&root, &first).unwrap();
        assert!(matches!(done.outcome, Outcome::Matches { .. }));
        watch(&root, "decision-release.md", ID);
        let log = load(&root, ID).unwrap();
        let last = log.versions.last().unwrap();
        assert_eq!(
            format!("{} {}", last.event, last.file),
            "edited decision-release.md"
        );
        assert_eq!(versions::content(&root, last).unwrap(), text(ID, "one"));
    }

    #[test]
    fn a_usage_error_after_the_sweep_still_names_the_leftover() {
        let s = scratch("usage-sweep");
        let (root, first) = fixture(&s);
        fs::write(root.join("notes").join(restore_file(ID)), text(ID, "lost")).unwrap();
        let err = apply(&root, "Not A Topic", &first, &mut |_| Ok(())).unwrap_err();
        let Error::Usage(message) = err else {
            panic!("not a usage error");
        };
        assert!(
            message.starts_with(&format!(
                "recorded notes/{} from an interrupted restore\n",
                restore_file(ID)
            )),
            "{message}"
        );
        assert_eq!(files(&root), ["decision-release.md"]);
    }

    #[test]
    fn a_topic_under_another_kind_without_an_id_counts_as_taken() {
        let s = scratch("taken-no-id");
        let (root, first) = renamed_fixture(&s);
        fs::write(root.join("notes/report-release.md"), "no frontmatter\n").unwrap();
        let err = apply(&root, ID, &first, &mut |_| Ok(())).unwrap_err();
        assert_eq!(
            err,
            Error::Refused("decision-release.md is taken by another note".into())
        );
        assert_eq!(read(&root, "plan-release.md"), text(ID, "two"));
    }

    #[test]
    fn restores_across_a_rename() {
        let s = scratch("rename");
        let (root, first) = renamed_fixture(&s);
        let done = go(&root, &first).unwrap();
        assert_eq!(
            done.outcome,
            Outcome::Restored {
                file: "decision-release.md".into(),
                version: first
            }
        );
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
        assert_eq!(
            events(&root).last().unwrap(),
            "restored decision-release.md"
        );
        assert!(events(&root).contains(&format!("{RENAMED} plan-release.md")));
    }

    #[test]
    fn a_file_the_watcher_has_not_seen_is_the_current_one() {
        let s = scratch("unseen");
        let (root, first) = fixture(&s);
        let notes = root.join("notes");
        {
            let lock = versions::lock(&root).unwrap();
            record_difference(&lock, ID, None, &versions::now_at()).unwrap();
        }
        fs::remove_file(notes.join("decision-release.md")).unwrap();
        put(&root, "plan-release.md", ID, "three");
        go(&root, &first).unwrap();
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
        let log = load(&root, ID).unwrap();
        let kinds: Vec<&str> = log.versions.iter().map(|v| v.event.as_str()).collect();
        assert_eq!(kinds, [ADDED, EDITED, "deleted", EDITED, "restored"]);
        assert_eq!(log.versions[3].file, "plan-release.md");
    }

    #[test]
    fn a_deleted_note_comes_back() {
        let s = scratch("deleted");
        let (root, first) = fixture(&s);
        {
            let lock = versions::lock(&root).unwrap();
            record_difference(&lock, ID, None, &versions::now_at()).unwrap();
        }
        fs::remove_file(root.join("notes/decision-release.md")).unwrap();
        go(&root, &first).unwrap();
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(read(&root, "decision-release.md"), text(ID, "one"));
    }

    #[test]
    fn a_deleted_file_not_yet_recorded_is_recorded() {
        let s = scratch("unrecorded-delete");
        let (root, first) = fixture(&s);
        fs::remove_file(root.join("notes/decision-release.md")).unwrap();
        apply(&root, ID, &first, &mut |_| Ok(())).unwrap();
        let log = load(&root, ID).unwrap();
        let kinds: Vec<&str> = log.versions.iter().map(|v| v.event.as_str()).collect();
        assert_eq!(kinds, [ADDED, EDITED, "deleted", "restored"]);
    }

    #[test]
    fn a_taken_topic_changes_nothing() {
        let s = scratch("taken-topic");
        let (root, first) = renamed_fixture(&s);
        put(&root, "report-release.md", OTHER, "other");
        let before = events(&root);
        let err = apply(&root, ID, &first, &mut |_| Ok(())).unwrap_err();
        assert_eq!(
            err,
            Error::Refused("decision-release.md is taken by another note".into())
        );
        assert_eq!(files(&root), ["plan-release.md", "report-release.md"]);
        assert_eq!(read(&root, "plan-release.md"), text(ID, "two"));
        assert_eq!(events(&root), before);
    }

    #[test]
    fn a_taken_name_changes_nothing() {
        let s = scratch("taken-name");
        let (root, first) = renamed_fixture(&s);
        put(&root, "decision-release.md", OTHER, "other");
        let err = apply(&root, ID, &first, &mut |_| Ok(())).unwrap_err();
        assert_eq!(
            err,
            Error::Refused("decision-release.md is taken by another note".into())
        );
        assert_eq!(files(&root), ["decision-release.md", "plan-release.md"]);
        assert_eq!(read(&root, "plan-release.md"), text(ID, "two"));
    }

    #[test]
    fn a_name_taken_during_the_restore_keeps_the_restored_text_under_the_old_name() {
        let s = scratch("taken-late");
        let (root, first) = renamed_fixture(&s);
        let other = root.join("notes/decision-release.md");
        let mut hook = |step: Step| {
            if step == Step::Rename {
                fs::write(&other, text(OTHER, "other")).unwrap();
            }
            Ok(())
        };
        let err = apply(&root, "release", &first, &mut hook).unwrap_err();
        assert_eq!(
            err,
            Error::Refused(format!(
                "restored plan-release.md to {first}; decision-release.md is taken by another note"
            ))
        );
        assert_eq!(files(&root), ["decision-release.md", "plan-release.md"]);
        assert_eq!(read(&root, "plan-release.md"), text(ID, "one"));
        assert_eq!(read(&root, "decision-release.md"), text(OTHER, "other"));
        assert_eq!(events(&root).last().unwrap(), "restored plan-release.md");
    }

    #[test]
    fn a_deletion_is_refused() {
        let s = scratch("deletion");
        let (root, _) = fixture(&s);
        {
            let lock = versions::lock(&root).unwrap();
            record_difference(&lock, ID, None, &versions::now_at()).unwrap();
        }
        let gone = load(&root, ID).unwrap().versions[2].short().to_string();
        let err = go(&root, &gone).unwrap_err();
        assert_eq!(
            err,
            Error::Refused(format!(
                "version {gone} of release is a deletion; delete the file instead"
            ))
        );
    }

    #[test]
    fn a_note_held_only_by_a_skipped_file_is_refused() {
        let s = scratch("skipped");
        let (root, first) = fixture(&s);
        let big = "x".repeat(versions::MAX_BYTES);
        put(&root, "decision-release.md", ID, &big);
        let before = files(&root);
        let err = go(&root, &first).unwrap_err();
        let Error::Refused(message) = err else {
            panic!("not refused");
        };
        assert!(
            message.contains("held by notes/decision-release.md"),
            "{message}"
        );
        assert_eq!(files(&root), before);
        let name = apply(&root, ID, &first, &mut |_| Ok(())).unwrap_err();
        assert!(matches!(name, Error::Refused(_)));
    }

    #[test]
    fn a_leftover_with_new_bytes_is_recorded_first() {
        let s = scratch("leftover-new");
        let (root, first) = fixture(&s);
        fs::write(root.join("notes").join(restore_file(ID)), text(ID, "lost")).unwrap();
        let done = go(&root, &first).unwrap();
        assert_eq!(
            done.messages,
            [format!(
                "recorded notes/{} from an interrupted restore",
                restore_file(ID)
            )]
        );
        assert_eq!(files(&root), ["decision-release.md"]);
        let log = load(&root, ID).unwrap();
        let lost = log
            .versions
            .iter()
            .position(|v| versions::content(&root, v).is_ok_and(|b| b == text(ID, "lost")))
            .unwrap();
        assert_eq!(log.versions[lost].event, EDITED);
        assert!(lost < log.versions.len() - 1);
        assert_eq!(log.versions.last().unwrap().event, "restored");
    }

    #[test]
    fn a_leftover_already_in_history_is_deleted_quietly() {
        let s = scratch("leftover-known");
        let (root, first) = fixture(&s);
        fs::write(root.join("notes").join(restore_file(ID)), text(ID, "one")).unwrap();
        let done = go(&root, &first).unwrap();
        assert!(done.messages.is_empty());
        assert_eq!(files(&root), ["decision-release.md"]);
        assert_eq!(events(&root).len(), 3);
    }

    #[test]
    fn a_missing_store_creates_nothing() {
        let dir =
            std::env::temp_dir().join(format!("bilbo-restore-nostore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let err = apply(&dir, "release", "abcdef", &mut |_| Ok(())).unwrap_err();
        assert_eq!(
            err,
            Error::Refused(format!("no store at {}", dir.display()))
        );
        assert!(!dir.exists());
    }

    #[test]
    fn bad_names_and_versions_are_usage_errors() {
        let s = scratch("usage");
        let (root, _) = fixture(&s);
        assert!(matches!(go(&root, "xyz"), Err(Error::Usage(_))));
        assert!(matches!(
            apply(&root, "Not A Topic", "abcdef", &mut |_| Ok(())),
            Err(Error::Usage(_))
        ));
        assert_eq!(
            go(&root, "abcdef"),
            Err(Error::Refused("no version abcdef of release".into()))
        );
        assert_eq!(files(&root), ["decision-release.md"]);
    }

    #[test]
    fn the_hook_sees_the_steps_in_order() {
        let s = scratch("order");
        let (root, first) = renamed_fixture(&s);
        let mut seen = Vec::new();
        apply(&root, "release", &first, &mut |step| {
            seen.push(step);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            seen,
            [
                Step::Write,
                Step::Swap,
                Step::Rename,
                Step::Inspect,
                Step::Remove,
                Step::Record
            ]
        );
    }
}
