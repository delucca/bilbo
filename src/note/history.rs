//! `bilbo history`: lists a note's recorded versions, prints one, or diffs two. It reads history and takes no lock.

use std::collections::HashMap;
use std::path::Path;

use crate::Failure;
use crate::identity::manifest;
use crate::note::diff;
use crate::note::versions::{self, ContentError, NameError, Named, Scan, Version, VersionError};
use crate::shared::store;

const STALE: &str = "bilbo watch is not running; recent edits may not be recorded";

enum Request {
    List,
    Print(String),
    Diff(String, Option<String>),
}

struct Parsed {
    note: String,
    request: Request,
}

pub struct Output {
    /// stderr lines (without "bilbo: ").
    pub warnings: Vec<String>,
    /// The listing, one line per version.
    pub lines: Vec<String>,
    /// A printed version or a diff, written to stdout as is.
    pub bytes: Vec<u8>,
}

pub fn run(args: &[String], env: &store::Env) -> Result<Output, Failure> {
    let parsed = parse(args)?;
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let scan = versions::scan(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;
    let named = versions::resolve(&root, &parsed.note, &scan).map_err(|e| match e {
        NameError::Usage(message) => Failure::Usage(message),
        NameError::NoHistory => Failure::Refused(format!("no history for {}", parsed.note)),
        NameError::Failed(message) => Failure::Refused(message),
    })?;
    let log = versions::load(&root, &named.id).map_err(Failure::Refused)?;
    let mut output = Output {
        warnings: Vec::new(),
        lines: Vec::new(),
        bytes: Vec::new(),
    };
    let note = &parsed.note;
    let names = Names::of(&root, &log.versions);
    match &parsed.request {
        Request::List => {
            output.lines = log.versions.iter().rev().map(|v| line(v, &names)).collect();
        }
        Request::Print(prefix) => {
            let version = pick(&log.versions, prefix, note, &names)?;
            output.bytes = text(&root, version, prefix, note)?;
        }
        Request::Diff(a, b) => {
            let first = pick(&log.versions, a, note, &names)?;
            let old = text_or_empty(&root, first, a, note)?;
            let (new_name, new) = match b {
                Some(b) => {
                    let second = pick(&log.versions, b, note, &names)?;
                    let bytes = text_or_empty(&root, second, b, note)?;
                    (format!("{}@{b}", second.file), bytes)
                }
                None => {
                    let (file, bytes) = current(&scan, &named, note)?;
                    (format!("{file}@now"), bytes)
                }
            };
            let diff = diff::unified(
                &format!("{}@{a}", first.file),
                &new_name,
                &String::from_utf8_lossy(&old),
                &String::from_utf8_lossy(&new),
            );
            output.bytes = diff.into_bytes();
        }
    }
    if !versions::watcher_running(&root) {
        output.warnings.push(STALE.into());
    }
    Ok(output)
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::Usage(message.into())
}

fn parse(args: &[String]) -> Result<Parsed, Failure> {
    let mut args = args.iter();
    let note = match args.next() {
        None => return Err(usage("history needs a note")),
        Some(arg) if arg.starts_with('-') => return Err(usage(format!("unknown option '{arg}'"))),
        Some(arg) => arg.clone(),
    };
    let rest: Vec<&String> = args.collect();
    let request = match rest.as_slice() {
        [] => Request::List,
        [flag, tail @ ..] if flag.as_str() == "--diff" => match tail {
            [] => return Err(usage("--diff needs a version")),
            [a] => Request::Diff((*a).clone(), None),
            [a, b] => Request::Diff((*a).clone(), Some((*b).clone())),
            [_, _, extra, ..] => {
                return Err(usage(format!("unexpected argument '{extra}'")));
            }
        },
        [option, ..] if option.starts_with('-') => {
            return Err(usage(format!("unknown option '{option}'")));
        }
        [version] => Request::Print((*version).clone()),
        [_, extra, ..] => return Err(usage(format!("unexpected argument '{extra}'"))),
    };
    Ok(Parsed { note, request })
}

/// The names the local manifests give device ids.
struct Names(HashMap<String, String>);

impl Names {
    /// Reads the manifests only when some version came from another device. A manifest that cannot be read names no
    /// one.
    fn of(root: &Path, versions: &[Version]) -> Names {
        let mut names = HashMap::new();
        if versions.iter().any(|v| v.device.is_some()) {
            for id in manifest::scope_ids(root).unwrap_or_default() {
                let Ok(scope) = manifest::read_scope(root, &id) else {
                    continue;
                };
                for version in scope.versions.iter().rev() {
                    for entry in &version.manifest.devices {
                        names
                            .entry(entry.id.clone())
                            .or_insert_with(|| entry.name.clone());
                    }
                }
            }
        }
        Names(names)
    }
}

/// `<version> <time> <event> <file>`, then ` from <device>` and ` [<flags>]` when the version has them. A device no
/// manifest names shows as its id.
fn line(v: &Version, names: &Names) -> String {
    let mut line = format!("{} {} {} {}", v.short(), v.minute(), v.event, v.file);
    if let Some(device) = &v.device {
        line.push_str(" from ");
        line.push_str(names.0.get(device).unwrap_or(device));
    }
    let mut flags: Vec<&str> = v.flags.iter().map(String::as_str).collect();
    if !v.conflict.is_empty() {
        flags.push("conflict");
    }
    if !v.dropped.is_empty() {
        flags.push("dropped");
    }
    if !flags.is_empty() {
        line.push_str(&format!(" [{}]", flags.join(", ")));
    }
    line
}

fn pick<'a>(
    versions: &'a [Version],
    prefix: &str,
    note: &str,
    names: &Names,
) -> Result<&'a Version, Failure> {
    versions::find_version(versions, prefix).map_err(|e| match e {
        VersionError::Invalid => usage(format!(
            "'{prefix}' is not a version: use 6 to 64 hexadecimal characters of its id"
        )),
        VersionError::Ambiguous(found) => {
            let lines: Vec<String> = found.iter().map(|v| line(v, names)).collect();
            usage(format!(
                "version {prefix} of {note} matches more than one version:\n{}",
                lines.join("\n")
            ))
        }
        VersionError::Missing => Failure::Refused(format!("no version {prefix} of {note}")),
    })
}

/// A version's bytes; a deletion and a pruned version are refusals.
fn text(
    root: &std::path::Path,
    version: &Version,
    prefix: &str,
    note: &str,
) -> Result<Vec<u8>, Failure> {
    versions::content(root, version).map_err(|e| match e {
        ContentError::Deleted => {
            Failure::Refused(format!("version {prefix} of {note} is a deletion"))
        }
        ContentError::Pruned => Failure::Refused(format!("version {prefix} of {note} was pruned")),
        ContentError::Io(message) => Failure::Refused(message),
    })
}

/// A version's bytes for a diff, where a deletion is empty text.
fn text_or_empty(
    root: &std::path::Path,
    version: &Version,
    prefix: &str,
    note: &str,
) -> Result<Vec<u8>, Failure> {
    if version.is_deleted() {
        return Ok(Vec::new());
    }
    text(root, version, prefix, note)
}

/// The note's file in `notes/` as it is now, with its name.
fn current(scan: &Scan, named: &Named, note: &str) -> Result<(String, Vec<u8>), Failure> {
    if let Some(found) = scan.notes.get(&named.id) {
        return Ok((found.name.clone(), found.bytes.clone()));
    }
    let message = if named.skipped.is_empty() {
        format!("{note} has no file; name two versions")
    } else {
        let files: Vec<String> = named.skipped.iter().map(|n| format!("notes/{n}")).collect();
        format!(
            "{note} is held by {}, which is not recorded; name two versions",
            files.join(", ")
        )
    };
    Err(Failure::Refused(message))
}
