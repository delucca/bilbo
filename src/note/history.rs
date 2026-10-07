//! `bilbo history`: lists a note's recorded versions, prints one, or diffs two. It reads history and takes no lock.

use std::collections::HashMap;
use std::path::Path;

use crate::Failure;
use crate::host::terminal;
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
    pub shown: Shown,
}

/// Which form `bytes` and `lines` hold.
pub enum Shown {
    List,
    Version,
    /// The 12-character versions a diff compared; `b` is `None` when the second side is the note's file.
    Diff {
        a: String,
        b: Option<String>,
    },
}

impl Output {
    /// The `note-history` human view; a printed version has none, since its bytes are the result.
    pub fn view(&self, term: &terminal::Term, now: jiff::Timestamp) -> Vec<String> {
        use terminal::{Mark, Tone};
        match &self.shown {
            Shown::Version => self.lines.clone(),
            Shown::List => self.list_view(term, now),
            Shown::Diff { a, b } if self.bytes.is_empty() => vec![format!(
                "{}  no changes between {} and {}",
                terminal::mark(term, Mark::Kept),
                terminal::paint(term, Tone::Cyan, a),
                terminal::paint(term, Tone::Cyan, b.as_deref().unwrap_or("now"))
            )],
            Shown::Diff { .. } => String::from_utf8_lossy(&self.bytes)
                .lines()
                .map(|line| {
                    let tone = if line.starts_with("--- ") || line.starts_with("+++ ") {
                        Some(Tone::Bold)
                    } else if line.starts_with("@@") {
                        Some(Tone::Cyan)
                    } else if line.starts_with('-') {
                        Some(Tone::Red)
                    } else if line.starts_with('+') {
                        Some(Tone::Green)
                    } else {
                        None
                    };
                    tone.map_or_else(
                        || line.to_string(),
                        |tone| terminal::paint(term, tone, line),
                    )
                })
                .collect(),
        }
    }

    /// The note's file name, then a table of its versions read back from `lines`.
    fn list_view(&self, term: &terminal::Term, now: jiff::Timestamp) -> Vec<String> {
        use terminal::Tone;
        let mut first = "";
        let mut rows = Vec::new();
        for text in &self.lines {
            let mut words = text.splitn(5, ' ');
            let (Some(version), Some(time), Some(event), Some(file)) =
                (words.next(), words.next(), words.next(), words.next())
            else {
                continue;
            };
            if first.is_empty() {
                first = file;
            }
            let tail = words.next().unwrap_or("");
            let (from, flags) = match tail.split_once(" [") {
                Some((from, flags)) => (from, Some(format!("[{flags}"))),
                None if tail.starts_with('[') => ("", Some(tail.to_string())),
                None => (tail, None),
            };
            let extras: Vec<&str> = [
                (!from.is_empty()).then_some(from),
                flags.as_deref(),
                (file != first).then_some(file),
            ]
            .into_iter()
            .flatten()
            .collect();
            let event_tone = match event {
                "added" => Some(Tone::Green),
                "restored" => Some(Tone::Cyan),
                _ => None,
            };
            rows.push(vec![
                terminal::paint(term, Tone::Cyan, version),
                event_tone.map_or_else(
                    || event.to_string(),
                    |tone| terminal::paint(term, tone, event),
                ),
                terminal::ago(now, time),
                terminal::paint(term, Tone::Dim, &extras.join(" ")),
            ]);
        }
        let mut out = vec![terminal::paint(term, Tone::Bold, first)];
        out.extend(
            terminal::table(&rows, &[false; 4])
                .into_iter()
                .map(|row| format!("  {row}")),
        );
        out
    }
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
        shown: Shown::List,
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
            output.shown = Shown::Version;
        }
        Request::Diff(a, b) => {
            let first = pick(&log.versions, a, note, &names)?;
            let old = text_or_empty(&root, first, a, note)?;
            let (new_name, new, second) = match b {
                Some(b) => {
                    let second = pick(&log.versions, b, note, &names)?;
                    let bytes = text_or_empty(&root, second, b, note)?;
                    (
                        format!("{}@{b}", second.file),
                        bytes,
                        Some(second.short().to_string()),
                    )
                }
                None => {
                    let (file, bytes) = current(&scan, &named, note)?;
                    (format!("{file}@now"), bytes, None)
                }
            };
            let diff = diff::unified(
                &format!("{}@{a}", first.file),
                &new_name,
                &String::from_utf8_lossy(&old),
                &String::from_utf8_lossy(&new),
            );
            output.bytes = diff.into_bytes();
            output.shown = Shown::Diff {
                a: first.short().to_string(),
                b: second,
            };
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

/// `bilbo history --help`; its Usage block is also the synopsis a usage error shows.
pub const HELP: &str = r#"bilbo history: list a note's past versions, print one, or diff two.

Usage:
  bilbo history <note>
  bilbo history <note> <version>
  bilbo history <note> --diff <a> [<b>]

<note> is a topic or a note id; a deleted note is found by its last topic.
<version>, <a> and <b> are 6 to 64 hex characters of a version id.
history changes nothing. When no 'bilbo watch' runs, stderr says recent edits
may not be recorded.

Output, by form:
  list    One line per version, newest first:
          <version> <time> <event> <file name>
          then ' from <device>' when another device recorded it, and
          ' [<flag>, ...]' for its flags: conflict, dropped.
          Events: added, edited, renamed, deleted, restored, merged, left
  print   The version's bytes, exactly as recorded
  --diff  A unified diff from <a> to <b>, or to the note's file now; empty
          when they hold the same text

Exit: 0 success; 1 no store, no history, no such version, a deletion or a
pruned version; 2 usage or config error, an ambiguous note or version
included.

Examples:
  bilbo history release-tags
  bilbo history release-tags 3f2a9c
  bilbo history release-tags --diff 3f2a9c

Docs: https://github.com/delucca/bilbo/wiki/Commands#history
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-07T01:00:00-03:00";

    fn output(lines: &[&str], bytes: &str, shown: Shown) -> Output {
        Output {
            warnings: Vec::new(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
            bytes: bytes.as_bytes().to_vec(),
            shown,
        }
    }

    fn now() -> jiff::Timestamp {
        NOW.parse().unwrap()
    }

    #[test]
    fn the_list_view() {
        let out = output(
            &[
                "5b71e17cc108 2026-10-07T00:44-03:00 edited gotcha-busy.md",
                "2c0a8c85acdc 2026-10-07T00:44-03:00 edited gotcha-busy.md",
                "91e0aa3b17f2 2026-10-05T00:30-03:00 edited gotcha-busy.md from bywater",
                "0c3f8d2e6a41 2026-10-04T00:30-03:00 restored gotcha-busy.md [conflict]",
                "d4f94f03789f 2026-10-02T00:30-03:00 added gotcha-sqlite-busy.md",
            ],
            "",
            Shown::List,
        );
        let plain = out.view(&terminal::fixed(100, false, true), now());
        assert_eq!(
            plain,
            [
                "gotcha-busy.md",
                "  5b71e17cc108  edited    16 min ago",
                "  2c0a8c85acdc  edited    16 min ago",
                "  91e0aa3b17f2  edited    2 days ago  from bywater",
                "  0c3f8d2e6a41  restored  3 days ago  [conflict]",
                "  d4f94f03789f  added     5 days ago  gotcha-sqlite-busy.md",
            ]
        );
        let painted = out.view(&terminal::fixed(100, true, true), now());
        assert_eq!(painted[0], terminal::styled("{b}gotcha-busy.md{/b}"));
        assert_eq!(
            painted[1],
            terminal::styled("  {c}5b71e17cc108{/c}  edited    16 min ago")
        );
        assert_eq!(
            painted[5],
            terminal::styled(
                "  {c}d4f94f03789f{/c}  {g}added{/g}     5 days ago  {d}gotcha-sqlite-busy.md{/d}"
            )
        );
    }

    #[test]
    fn equal_versions_say_so() {
        let term = terminal::fixed(100, true, true);
        let same = |b: Option<&str>| {
            output(
                &[],
                "",
                Shown::Diff {
                    a: "d4f94f03789f".into(),
                    b: b.map(String::from),
                },
            )
            .view(&term, now())
        };
        assert_eq!(
            same(Some("5b71e17cc108")),
            [terminal::styled(
                "{d}◇{/d}  no changes between {c}d4f94f03789f{/c} and {c}5b71e17cc108{/c}"
            )]
        );
        assert_eq!(
            same(None),
            [terminal::styled(
                "{d}◇{/d}  no changes between {c}d4f94f03789f{/c} and {c}now{/c}"
            )]
        );
    }

    #[test]
    fn a_diff_in_colour() {
        let out = output(
            &[],
            "--- x.md@d4f94f03789f\n+++ x.md@5b71e17cc108\n@@ -8,2 +8,2 @@\n ## Fix\n-Set 5000.\n+Set 10000.\n",
            Shown::Diff {
                a: "d4f94f03789f".into(),
                b: Some("5b71e17cc108".into()),
            },
        );
        let want = "{b}--- x.md@d4f94f03789f{/b}
{b}+++ x.md@5b71e17cc108{/b}
{c}@@ -8,2 +8,2 @@{/c}
 ## Fix
{r}-Set 5000.{/r}
{g}+Set 10000.{/g}";
        assert_eq!(
            out.view(&terminal::fixed(100, true, true), now()),
            terminal::styled(want).lines().collect::<Vec<_>>()
        );
        assert_eq!(
            out.view(&terminal::fixed(100, false, true), now()),
            String::from_utf8_lossy(&out.bytes)
                .lines()
                .collect::<Vec<_>>()
        );
    }
}
