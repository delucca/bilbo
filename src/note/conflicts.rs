//! Open conflicts, dropped text, declarations and stray markers, judged from the open-conflict summary, the note's log
//! and its file now.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::note::merge::{self, Block, BlockSide};
use crate::note::versions::{self, Lock, Log, Version};

/// Flags older than this stay out of a summary entry.
const NOTICE_DAYS: i64 = 7;

/// `<root>/.bilbo/sync/open.json`.
pub fn path(root: &Path) -> PathBuf {
    sync_dir(root).join("open.json")
}

fn sync_dir(root: &Path) -> PathBuf {
    root.join(".bilbo/sync")
}

/// The open-conflict summary, by note id. The watcher keeps it; `check`, `sync` and the digest read it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    #[serde(default)]
    pub notes: BTreeMap<String, Entry>,
}

/// What one note has pending. A note with nothing to say has no entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub file: String,
    /// Conflicts that a recorded version holds blocks for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict: Vec<Open>,
    /// Dropped lines no declaration covers, from recorded resolutions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<Lost>,
    /// The latest version is a `merged` version without conflict.
    #[serde(default, skip_serializing_if = "is_false")]
    pub merged: bool,
    /// The flags recorded in the last 7 days.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<Notice>,
    /// The scopes this device moved the note out of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<Left>,
}

/// A passage in conflict: the version that holds its blocks, its heading path and the versions of its sides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Open {
    pub version: String,
    pub passage: String,
    pub sides: Vec<String>,
}

/// Lines a resolution dropped from one passage. `conflict` is the version `bilbo sync declare` names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lost {
    pub conflict: String,
    pub passage: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub at: String,
    pub flag: String,
}

/// A move out of the syncing scope `scope`, recorded here at `at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Left {
    pub scope: String,
    pub at: String,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl Entry {
    /// Whether an open conflict or undeclared dropped text waits.
    pub fn waits(&self) -> bool {
        !self.conflict.is_empty() || !self.dropped.is_empty()
    }

    /// Whether it says nothing, so the summary holds no entry for the note.
    pub fn is_empty(&self) -> bool {
        !self.waits() && !self.merged && self.notices.is_empty() && self.left.is_empty()
    }
}

/// Reads the summary; a missing file is an empty one. An error is the reason alone, for the caller to put after a path.
pub fn read(root: &Path) -> Result<Summary, String> {
    match fs::read(path(root)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Summary::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Replaces the summary through a temporary file and a rename, so a reader sees the old one or the new one.
pub fn write(lock: &Lock, summary: &Summary) -> Result<(), String> {
    let dir = sync_dir(lock.root());
    let io = |what: &str, path: &Path, e: std::io::Error| {
        format!("cannot {what} {}: {e}", path.display())
    };
    fs::create_dir_all(&dir).map_err(|e| io("create", &dir, e))?;
    let mut bytes = serde_json::to_vec(summary).map_err(|e| format!("cannot encode: {e}"))?;
    bytes.push(b'\n');
    let (tmp, target) = (versions::temp_path(&dir)?, dir.join("open.json"));
    fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, &target))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io("write", &target, e)
        })
}

type ById<'a> = HashMap<&'a str, &'a Version>;

fn by_id(log: &Log) -> ById<'_> {
    log.versions
        .iter()
        .map(|v| (v.version.as_str(), v))
        .collect()
}

fn declared(log: &Log) -> HashSet<&str> {
    log.declarations
        .iter()
        .map(|d| d.declare.as_str())
        .collect()
}

/// The ancestors of `start` that carry a conflict, for `passage` when given, nearest first (a level by id). The walk
/// goes through versions that carry none, and for a passage stops at a version that dropped lines of it: that was the
/// resolution of an earlier conflict.
fn carriers<'a>(ids: &ById<'a>, start: &'a Version, passage: Option<&str>) -> Vec<&'a Version> {
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut frontier = vec![start];
    while !frontier.is_empty() {
        let mut next: Vec<&Version> = Vec::new();
        let mut found: Vec<&Version> = Vec::new();
        for v in &frontier {
            for p in &v.parents {
                let Some(&parent) = ids.get(p.as_str()) else {
                    continue;
                };
                if !seen.insert(parent.version.as_str()) {
                    continue;
                }
                let holds = |c: &versions::Conflict| passage.is_none_or(|p| c.passage == p);
                if parent.conflict.iter().any(holds) {
                    found.push(parent);
                    next.push(parent);
                } else if passage.is_none_or(|p| parent.dropped.iter().all(|d| d.passage != p)) {
                    next.push(parent);
                }
            }
        }
        found.sort_by(|a, b| a.version.cmp(&b.version));
        out.extend(found);
        next.sort_by(|a, b| a.version.cmp(&b.version));
        frontier = next;
    }
    out
}

/// Whether a declaration covers drops of `passage` whose nearest carrier is `conflict`: it names that version or any
/// version of its chain of carriers for the passage.
fn covered(ids: &ById, declared: &HashSet<&str>, conflict: &str, passage: &str) -> bool {
    declared.contains(conflict)
        || ids.get(conflict).is_some_and(|v| {
            carriers(ids, v, Some(passage))
                .iter()
                .any(|c| declared.contains(c.version.as_str()))
        })
}

/// `lines` of `passage` that `text` does not hold.
fn lacking(passage: &str, lines: &[String], text: &str) -> Vec<String> {
    let held = [Block {
        passage: passage.to_string(),
        line: 0,
        end: 0,
        sides: vec![BlockSide {
            version: String::new(),
            time: String::new(),
            lines: lines.to_vec(),
        }],
    }];
    merge::dropped(&held, text)
        .into_iter()
        .flat_map(|d| d.lines)
        .collect()
}

/// A note's entry from its log, as the watcher rewrites it after recording a version of a note. Every head counts:
/// - a head that carries `conflict` holds those conflicts open; any other head keeps each passage of its nearest
///   carriers whose block its content still holds, so an edit that leaves a block leaves the conflict open;
/// - a drop recorded in a head's ancestry counts while that head's content lacks the line, and unless a declaration
///   names a version of the chain of carriers it belongs to; a drop with no carrier in the log, because a prune took
///   it, cannot be judged and is left out;
/// - `merged` is true when the last log line is an auto-merge: the watcher calls this right after appending the
///   version it recorded, so that line is the one it just wrote;
/// - `notices` are the flags of the 7 days before `now`.
///
/// `left` stays empty, because the log does not name the scope.
pub fn summarize(root: &Path, log: &Log, file: &str, now: jiff::Timestamp) -> Entry {
    let ids = by_id(log);
    let declared = declared(log);
    let mut entry = Entry {
        file: file.to_string(),
        ..Entry::default()
    };
    let mut heads: Vec<&Version> = versions::heads(&log.versions)
        .into_iter()
        .flatten()
        .collect();
    heads.sort_by(|a, b| a.version.cmp(&b.version));
    let mut drops: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
    for head in heads {
        let text = versions::content(root, head)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let mut opens: Vec<Open> = head
            .conflict
            .iter()
            .map(|c| Open {
                version: head.version.clone(),
                passage: c.passage.clone(),
                sides: c.sides.clone(),
            })
            .collect();
        if let (true, Some(text)) = (opens.is_empty(), &text) {
            let mut blocks = merge::blocks(text);
            for carrier in carriers(&ids, head, None) {
                for c in &carrier.conflict {
                    let held = blocks.iter().position(|b| b.passage == c.passage);
                    if let (Some(i), false) = (held, opens.iter().any(|o| o.passage == c.passage)) {
                        blocks.remove(i);
                        opens.push(Open {
                            version: carrier.version.clone(),
                            passage: c.passage.clone(),
                            sides: c.sides.clone(),
                        });
                    }
                }
            }
        }
        for open in opens {
            let dup = |o: &Open| o.version == open.version && o.passage == open.passage;
            if !entry.conflict.iter().any(dup) {
                entry.conflict.push(open);
            }
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut pending = vec![head];
        while let Some(v) = pending.pop() {
            if !seen.insert(v.version.as_str()) {
                continue;
            }
            pending.extend(v.parents.iter().filter_map(|p| ids.get(p.as_str())));
            for drop in &v.dropped {
                let Some(nearest) = carriers(&ids, v, Some(&drop.passage)).first().copied() else {
                    continue;
                };
                if covered(&ids, &declared, &nearest.version, &drop.passage) {
                    continue;
                }
                let lines = match &text {
                    Some(text) => lacking(&drop.passage, &drop.lines, text),
                    None => drop.lines.clone(),
                };
                let held = drops
                    .entry((nearest.version.as_str(), drop.passage.as_str()))
                    .or_default();
                for line in lines {
                    if !held.contains(&line) {
                        held.push(line);
                    }
                }
            }
        }
    }
    entry.dropped = drops
        .into_iter()
        .filter(|(_, lines)| !lines.is_empty())
        .map(|((conflict, passage), lines)| Lost {
            conflict: conflict.to_string(),
            passage: passage.to_string(),
            lines,
        })
        .collect();
    entry.merged = log
        .latest()
        .is_some_and(|v| v.event == versions::MERGED && v.conflict.is_empty());
    let window = jiff::SignedDuration::from_hours(24 * NOTICE_DAYS);
    for v in &log.versions {
        let recent =
            v.at.parse::<jiff::Timestamp>()
                .is_ok_and(|at| now.duration_since(at) < window);
        if recent {
            entry.notices.extend(v.flags.iter().map(|flag| Notice {
                at: v.at.clone(),
                flag: flag.clone(),
            }));
        }
    }
    entry
}

/// A conflict block that is open in the file now.
#[derive(Debug, PartialEq, Eq)]
pub struct Standing {
    pub version: String,
    pub passage: String,
    /// The sides of the block in the file.
    pub sides: usize,
    /// The line of its first marker.
    pub line: usize,
}

/// What a note holds now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Judged {
    pub conflicts: Vec<Standing>,
    /// Undeclared dropped lines, per conflict version and passage. `bilbo sync declare` names their `conflict`.
    pub dropped: Vec<Lost>,
    /// The lines, counted from 1, of full-form markers outside fences that belong to no open conflict.
    pub stray: Vec<usize>,
}

/// Judges the file `text` against the note's summary `entry` and its `log`. An open conflict needs the record and a
/// block in the file, found by its passage or, when the heading changed, by a side label; markers alone are stray. A
/// recorded conflict with no block left is judged as if the resolution were recorded: the lines of its sides that
/// `text` lacks are dropped, from the conflict version's content. Recorded drops count while `text` still lacks the
/// lines, and none counts once a declaration covers it.
pub fn judge(root: &Path, entry: Option<&Entry>, log: &Log, text: &str) -> Judged {
    let found = merge::blocks(text);
    let mut used = vec![false; found.len()];
    let mut judged = Judged::default();
    let mut lost = Vec::new();
    if let Some(entry) = entry {
        let mut resolved: BTreeMap<&str, Vec<&Open>> = BTreeMap::new();
        for open in &entry.conflict {
            let free = |i: &usize| !used[*i];
            let hit = (0..found.len())
                .filter(free)
                .find(|&i| found[i].passage == open.passage)
                .or_else(|| {
                    (0..found.len())
                        .filter(free)
                        .find(|&i| shares_label(&found[i], open))
                });
            match hit {
                Some(i) => {
                    used[i] = true;
                    judged.conflicts.push(Standing {
                        version: open.version.clone(),
                        passage: found[i].passage.clone(),
                        sides: found[i].sides.len(),
                        line: found[i].line,
                    });
                }
                None => resolved.entry(&open.version).or_default().push(open),
            }
        }
        for (version, opens) in resolved {
            lost.extend(unrecorded(root, log, version, &opens, text));
        }
        for recorded in &entry.dropped {
            let lines = lacking(&recorded.passage, &recorded.lines, text);
            if !lines.is_empty() {
                lost.push(Lost {
                    lines,
                    ..recorded.clone()
                });
            }
        }
    }
    let ids = by_id(log);
    let declared = declared(log);
    lost.retain(|l| !covered(&ids, &declared, &l.conflict, &l.passage));
    judged.dropped = lost;
    let spans: Vec<(usize, usize)> = found
        .iter()
        .zip(&used)
        .filter(|(_, used)| **used)
        .map(|(b, _)| (b.line, b.end))
        .collect();
    judged.stray = merge::marker_lines(text)
        .into_iter()
        .filter(|n| {
            !spans
                .iter()
                .any(|(first, last)| (first..=last).contains(&n))
        })
        .collect();
    judged
}

/// Whether one of the block's side labels is the start of one of the conflict's side versions.
fn shares_label(block: &Block, open: &Open) -> bool {
    block
        .sides
        .iter()
        .any(|s| !s.version.is_empty() && open.sides.iter().any(|id| id.starts_with(&s.version)))
}

/// The drops of conflicts of `version` whose blocks the file no longer holds. Content that a prune took leaves
/// nothing to compare.
fn unrecorded(root: &Path, log: &Log, version: &str, opens: &[&Open], text: &str) -> Vec<Lost> {
    let Some(v) = log.versions.iter().find(|v| v.version == version) else {
        return Vec::new();
    };
    let Ok(bytes) = versions::content(root, v) else {
        return Vec::new();
    };
    let mut had = merge::blocks(&String::from_utf8_lossy(&bytes));
    let mut lost = Vec::new();
    for open in opens {
        if let Some(i) = had.iter().position(|b| b.passage == open.passage) {
            let block = had.remove(i);
            let lines: Vec<String> = block.sides.iter().flat_map(|s| s.lines.clone()).collect();
            let lines = lacking(&block.passage, &lines, text);
            if !lines.is_empty() {
                lost.push(Lost {
                    conflict: version.to_string(),
                    passage: block.passage,
                    lines,
                });
            }
        }
    }
    lost
}

/// The scopes this device moved the note out of within the last 30 days before `now` that the note is not in again,
/// sorted and without repeats. `current` is the scope the note holds now.
pub fn left_scopes<'a>(
    entry: &'a Entry,
    current: Option<&str>,
    now: jiff::Timestamp,
) -> Vec<&'a str> {
    let window = jiff::SignedDuration::from_hours(24 * 30);
    let names: BTreeSet<&str> = entry
        .left
        .iter()
        .filter(|l| Some(l.scope.as_str()) != current)
        .filter(|l| {
            l.at.parse::<jiff::Timestamp>()
                .is_ok_and(|at| now.duration_since(at) < window)
        })
        .map(|l| l.scope.as_str())
        .collect();
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::versions::{Conflict, Declaration, Dropped};

    const OPEN: &str = "<<<<<<< bilbo ";
    const SEP: &str = "======= bilbo ";
    const CLOSE: &str = ">>>>>>> bilbo";
    const NOTE: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
    const NOW: &str = "2026-10-05T12:00:00-03:00";

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("bilbo-conflicts-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("notes")).unwrap();
        Scratch(dir)
    }

    fn now() -> jiff::Timestamp {
        NOW.parse().unwrap()
    }

    fn id(n: u8) -> String {
        format!("{n:012x}{}", "0".repeat(52))
    }

    /// A version with the given id, parents and bytes, appended to the note's log.
    fn put(lock: &Lock, v: &str, parents: &[&str], bytes: &str, event: &str) -> Version {
        let blob = versions::write_blob(lock, bytes.as_bytes()).unwrap();
        let version = Version {
            version: v.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            file: "plan-x.md".into(),
            blob,
            event: event.into(),
            at: NOW.into(),
            ..Version::default()
        };
        versions::append(lock, NOTE, &version).unwrap();
        version
    }

    fn block(sides: &[&str]) -> String {
        let mut out = String::new();
        for (n, side) in sides.iter().enumerate() {
            let mark = if n == 0 { OPEN } else { SEP };
            out.push_str(&format!(
                "{mark}{} 2026-10-03T14:23-03:00\n{side}\n",
                &id(n as u8 + 1)[..12]
            ));
        }
        out.push_str(&format!("{CLOSE}\n"));
        out
    }

    fn entry(passage: &str, version: &str) -> Entry {
        Entry {
            file: "plan-x.md".into(),
            conflict: vec![Open {
                version: version.into(),
                passage: passage.into(),
                sides: vec![id(1), id(2)],
            }],
            ..Entry::default()
        }
    }

    fn conflicted() -> String {
        format!(
            "# X\n\n## Flakes\n\n{}\n## Other\n\nkept\n",
            block(&["use A\nand B", "use C"])
        )
    }

    #[test]
    fn the_summary_round_trips_and_a_missing_one_is_empty() {
        let s = scratch("roundtrip");
        assert_eq!(read(&s.0).unwrap(), Summary::default());
        let lock = versions::lock(&s.0).unwrap();
        let mut summary = Summary::default();
        summary.notes.insert(NOTE.into(), entry("Flakes", &id(3)));
        write(&lock, &summary).unwrap();
        assert_eq!(read(&s.0).unwrap(), summary);
        let names: Vec<_> = fs::read_dir(s.0.join(".bilbo/sync"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["open.json"]);
        write(&lock, &Summary::default()).unwrap();
        assert_eq!(read(&s.0).unwrap(), Summary::default());
    }

    #[test]
    fn an_unreadable_summary_is_an_error() {
        let s = scratch("garbled");
        fs::create_dir_all(s.0.join(".bilbo/sync")).unwrap();
        fs::write(path(&s.0), "{").unwrap();
        assert!(!read(&s.0).unwrap_err().is_empty());
    }

    #[test]
    fn a_recorded_block_in_the_file_is_an_open_conflict() {
        let s = scratch("open");
        let judged = judge(
            &s.0,
            Some(&entry("Flakes", &id(3))),
            &Log::default(),
            &conflicted(),
        );
        assert_eq!(judged.conflicts.len(), 1);
        assert_eq!(judged.conflicts[0].passage, "Flakes");
        assert_eq!(judged.conflicts[0].sides, 2);
        assert_eq!(judged.conflicts[0].line, 5);
        assert!(judged.dropped.is_empty() && judged.stray.is_empty());
    }

    #[test]
    fn markers_without_a_record_are_stray_and_never_a_conflict() {
        let s = scratch("pasted");
        let text = conflicted();
        for entry in [None, Some(&Entry::default())] {
            let judged = judge(&s.0, entry, &Log::default(), &text);
            assert!(judged.conflicts.is_empty());
            assert_eq!(judged.stray, [5, 8, 10]);
        }
    }

    #[test]
    fn a_block_is_matched_by_its_passage_not_its_side_count() {
        let s = scratch("passage");
        let text = format!(
            "## A\n\n{}\n## B\n\n{}",
            block(&["x", "y", "z"]),
            block(&["p", "q"])
        );
        let mut e = entry("B", &id(3));
        e.conflict.push(Open {
            version: id(3),
            passage: "A".into(),
            sides: vec![id(1), id(2)],
        });
        let judged = judge(&s.0, Some(&e), &Log::default(), &text);
        let sides: Vec<(String, usize)> = judged
            .conflicts
            .iter()
            .map(|c| (c.passage.clone(), c.sides))
            .collect();
        assert_eq!(sides, [("B".to_string(), 2), ("A".to_string(), 3)]);
        assert!(judged.stray.is_empty());
    }

    #[test]
    fn a_block_of_another_passage_is_stray() {
        let s = scratch("elsewhere");
        let mut other = entry("Gone", &id(3));
        other.conflict[0].sides = vec![id(7), id(8)];
        let judged = judge(&s.0, Some(&other), &Log::default(), &conflicted());
        assert!(judged.conflicts.is_empty());
        assert_eq!(judged.stray, [5, 8, 10]);
    }

    #[test]
    fn markers_in_fences_and_longer_lines_are_not_stray() {
        let s = scratch("quoted");
        let text = "# X\n\n```\n>>>>>>> bilbo\n```\n\n>>>>>>> bilbo was here\n\n<<<<<<< bilbo nothex 2026\n\n>>>>>>> bilbo\n";
        let judged = judge(&s.0, None, &Log::default(), text);
        assert_eq!(judged.stray, [11]);
    }

    #[test]
    fn nothing_above_the_frontmatter_close_counts() {
        let s = scratch("front");
        let text = "---\nid: x\nnote: >>>>>>> bilbo\n>>>>>>> bilbo\n---\n\n# X\n";
        assert!(judge(&s.0, None, &Log::default(), text).stray.is_empty());
    }

    #[test]
    fn the_leftover_of_a_nested_paste_is_stray() {
        let s = scratch("nested");
        let text = format!(
            "## Flakes\n\n<<<<<<< bilbo {} t\n{}======= bilbo {} t\nz\n>>>>>>> bilbo\n",
            &id(1)[..12],
            block(&["i", "j"]),
            &id(2)[..12]
        );
        let e = entry("Flakes", &id(3));
        let judged = judge(&s.0, Some(&e), &Log::default(), &text);
        assert_eq!(judged.conflicts.len(), 1);
        assert_eq!(judged.stray, [9, 11]);
    }

    /// A log whose conflict version holds `conflicted()`.
    fn conflict_log(s: &Scratch) -> Log {
        let lock = versions::lock(&s.0).unwrap();
        put(&lock, &id(3), &[], &conflicted(), versions::MERGED);
        versions::load(&s.0, NOTE).unwrap()
    }

    #[test]
    fn a_resolution_not_yet_recorded_is_judged_by_its_drops() {
        let s = scratch("unrecorded");
        let log = conflict_log(&s);
        let resolved = "# X\n\n## Flakes\n\nuse A\n\n## Other\n\nkept\n";
        let judged = judge(&s.0, Some(&entry("Flakes", &id(3))), &log, resolved);
        assert!(judged.conflicts.is_empty() && judged.stray.is_empty());
        assert_eq!(
            judged.dropped,
            [Lost {
                conflict: id(3),
                passage: "Flakes".into(),
                lines: vec!["and B".into(), "use C".into()],
            }]
        );
    }

    #[test]
    fn keeping_every_line_drops_nothing() {
        let s = scratch("kept-all");
        let log = conflict_log(&s);
        let resolved = "# X\n\n## Flakes\n\nuse C\nuse A\n  and   B\n\n## Other\n\nkept\n";
        let judged = judge(&s.0, Some(&entry("Flakes", &id(3))), &log, resolved);
        assert_eq!(judged, Judged::default());
    }

    #[test]
    fn a_declaration_covers_the_unrecorded_and_the_recorded_drops() {
        let s = scratch("declared");
        let log = conflict_log(&s);
        {
            let lock = versions::lock(&s.0).unwrap();
            let declaration = Declaration {
                declare: id(3),
                reason: "superseded".into(),
                at: NOW.into(),
                device: None,
            };
            versions::append_declaration(&lock, NOTE, &declaration).unwrap();
        }
        let log = Log {
            declarations: versions::load(&s.0, NOTE).unwrap().declarations,
            ..log
        };
        let resolved = "# X\n\n## Flakes\n\nuse A\n";
        let judged = judge(&s.0, Some(&entry("Flakes", &id(3))), &log, resolved);
        assert!(judged.dropped.is_empty());
        let e = Entry {
            dropped: vec![Lost {
                conflict: id(3),
                passage: "Flakes".into(),
                lines: vec!["use C".into()],
            }],
            ..Entry::default()
        };
        assert!(judge(&s.0, Some(&e), &log, resolved).dropped.is_empty());
    }

    #[test]
    fn recorded_drops_clear_when_the_lines_come_back() {
        let s = scratch("restored");
        let e = Entry {
            dropped: vec![Lost {
                conflict: id(3),
                passage: "Flakes".into(),
                lines: vec!["use C".into(), "and D".into()],
            }],
            ..Entry::default()
        };
        let log = Log::default();
        let judged = judge(&s.0, Some(&e), &log, "## Flakes\n\nuse A\n");
        assert_eq!(judged.dropped[0].lines, ["use C", "and D"]);
        let judged = judge(&s.0, Some(&e), &log, "## Flakes\n\nuse A\nand D\n");
        assert_eq!(judged.dropped[0].lines, ["use C"]);
        assert!(
            judge(&s.0, Some(&e), &log, "use C\nand D\n")
                .dropped
                .is_empty()
        );
    }

    #[test]
    fn pruned_content_leaves_nothing_to_compare() {
        let s = scratch("pruned");
        let log = Log::default();
        let judged = judge(
            &s.0,
            Some(&entry("Flakes", &id(3))),
            &log,
            "## Flakes\n\nx\n",
        );
        assert_eq!(judged, Judged::default());
    }

    fn at(day: u8) -> String {
        format!("2026-10-{day:02}T12:00:00-03:00")
    }

    const RESOLVED: &str = "# X\n\n## Flakes\n\nuse A\n\n## Other\n\nkept\n";

    /// Appends a version of `plan-x.md` with `bytes`, recorded on `day`; `carries` gives it the Flakes conflict and
    /// `dropped` lists lines it dropped from Flakes.
    fn rec(
        lock: &Lock,
        v: u8,
        parents: &[u8],
        bytes: &str,
        day: u8,
        carries: bool,
        dropped: &[&str],
    ) {
        let parents: Vec<String> = parents.iter().map(|p| id(*p)).collect();
        let version = Version {
            version: id(v),
            parents,
            file: "plan-x.md".into(),
            blob: versions::write_blob(lock, bytes.as_bytes()).unwrap(),
            event: if carries { "merged" } else { "edited" }.into(),
            at: at(day),
            conflict: if carries {
                vec![Conflict {
                    passage: "Flakes".into(),
                    sides: vec![id(1), id(2)],
                }]
            } else {
                Vec::new()
            },
            dropped: if dropped.is_empty() {
                Vec::new()
            } else {
                vec![Dropped {
                    passage: "Flakes".into(),
                    lines: dropped.iter().map(|l| l.to_string()).collect(),
                }]
            },
            ..Version::default()
        };
        versions::append(lock, NOTE, &version).unwrap();
    }

    fn declare(lock: &Lock, version: &str) {
        let declaration = Declaration {
            declare: version.to_string(),
            reason: "r".into(),
            at: at(4),
            device: None,
        };
        versions::append_declaration(lock, NOTE, &declaration).unwrap();
    }

    fn summed(s: &Scratch) -> Entry {
        let log = versions::load(&s.0, NOTE).unwrap();
        summarize(&s.0, &log, "plan-x.md", now())
    }

    #[test]
    fn a_conflict_head_holds_its_conflict_open() {
        let s = scratch("sum-head");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 1, &[], "# X\n", 1, false, &[]);
        rec(&lock, 3, &[1], &conflicted(), 2, true, &[]);
        let got = summed(&s);
        assert_eq!(got.conflict, entry("Flakes", &id(3)).conflict);
        assert!(got.waits() && !got.is_empty() && !got.merged);
    }

    #[test]
    fn an_edit_that_leaves_the_block_leaves_the_conflict_open() {
        let s = scratch("sum-edit");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 3, &[], &conflicted(), 1, true, &[]);
        rec(
            &lock,
            4,
            &[3],
            &conflicted().replace("kept", "kept more"),
            2,
            false,
            &[],
        );
        let got = summed(&s);
        assert_eq!(got.conflict, entry("Flakes", &id(3)).conflict);
        let judged = judge(
            &s.0,
            Some(&got),
            &versions::load(&s.0, NOTE).unwrap(),
            &conflicted().replace("kept", "kept more"),
        );
        assert_eq!(judged.conflicts.len(), 1);
        assert!(judged.stray.is_empty());
    }

    #[test]
    fn a_resolution_leaves_no_open_conflict_and_keeps_its_drops() {
        let s = scratch("sum-resolved");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 3, &[], &conflicted(), 1, true, &[]);
        rec(&lock, 4, &[3], RESOLVED, 2, false, &["and B", "use C"]);
        rec(
            &lock,
            5,
            &[4],
            &RESOLVED.replace("kept", "kept more"),
            3,
            false,
            &[],
        );
        let got = summed(&s);
        assert!(got.conflict.is_empty());
        assert_eq!(
            got.dropped,
            [Lost {
                conflict: id(3),
                passage: "Flakes".into(),
                lines: vec!["and B".into(), "use C".into()]
            }]
        );
    }

    #[test]
    fn restored_lines_leave_the_entry() {
        let s = scratch("sum-restored");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 3, &[], &conflicted(), 1, true, &[]);
        rec(&lock, 4, &[3], RESOLVED, 2, false, &["and B", "use C"]);
        rec(
            &lock,
            5,
            &[4],
            &RESOLVED.replace("use A", "use A\nand B"),
            3,
            false,
            &[],
        );
        assert_eq!(summed(&s).dropped[0].lines, ["use C"]);
        rec(
            &lock,
            6,
            &[5],
            &RESOLVED.replace("use A", "use A\nand B\nuse C"),
            4,
            false,
            &[],
        );
        assert!(summed(&s).is_empty());
    }

    #[test]
    fn a_declaration_of_any_carrier_in_the_chain_covers_the_drop() {
        for declared in [3, 4, 5] {
            let s = scratch(&format!("sum-chain-{declared}"));
            let lock = versions::lock(&s.0).unwrap();
            rec(&lock, 3, &[], &conflicted(), 1, true, &[]);
            rec(&lock, 4, &[3], &conflicted(), 2, true, &[]);
            rec(&lock, 5, &[4], &conflicted(), 3, true, &[]);
            rec(&lock, 6, &[5], RESOLVED, 4, false, &["use C"]);
            let log = versions::load(&s.0, NOTE).unwrap();
            let before = summarize(&s.0, &log, "plan-x.md", now());
            assert_eq!(before.dropped.len(), 1, "{declared}");
            assert_eq!(before.dropped[0].conflict, id(5));
            declare(&lock, &id(declared));
            let log = versions::load(&s.0, NOTE).unwrap();
            assert!(
                summarize(&s.0, &log, "plan-x.md", now()).is_empty(),
                "{declared}"
            );
            let judged = judge(&s.0, Some(&before), &log, RESOLVED);
            assert!(judged.dropped.is_empty(), "{declared}");
        }
    }

    #[test]
    fn a_declaration_of_an_earlier_episode_does_not_cover_a_later_one() {
        let s = scratch("sum-episodes");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 3, &[], &conflicted(), 1, true, &[]);
        rec(&lock, 4, &[3], RESOLVED, 2, false, &["use C"]);
        rec(&lock, 5, &[4], &conflicted(), 3, true, &[]);
        rec(&lock, 6, &[5], RESOLVED, 4, false, &["use C"]);
        declare(&lock, &id(3));
        let got = summed(&s);
        assert_eq!(got.dropped.len(), 1);
        assert_eq!(got.dropped[0].conflict, id(5));
    }

    #[test]
    fn a_drop_without_a_carrier_in_the_log_cannot_be_judged() {
        let s = scratch("sum-pruned");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 4, &[], RESOLVED, 2, false, &["use C"]);
        assert!(summed(&s).is_empty());
    }

    #[test]
    fn a_renamed_heading_above_a_kept_block_keeps_the_conflict_open() {
        let s = scratch("renamed");
        let text = conflicted().replace("## Flakes", "## Flake inputs");
        let judged = judge(&s.0, Some(&entry("Flakes", &id(3))), &Log::default(), &text);
        assert_eq!(judged.conflicts.len(), 1);
        assert_eq!(judged.conflicts[0].passage, "Flake inputs");
        assert!(judged.stray.is_empty());
    }

    #[test]
    fn flags_of_the_last_week_are_notices() {
        let s = scratch("sum-notices");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 3, &[], "# X\n", 1, false, &[]);
        let mut log = versions::load(&s.0, NOTE).unwrap();
        log.versions[0].flags = vec!["stale-base".into()];
        log.versions[0].at = at(4);
        let got = summarize(&s.0, &log, "plan-x.md", now());
        assert_eq!(
            got.notices,
            [Notice {
                at: at(4),
                flag: "stale-base".into()
            }]
        );
        log.versions[0].at = "2026-09-20T12:00:00-03:00".into();
        assert!(summarize(&s.0, &log, "plan-x.md", now()).notices.is_empty());
    }

    #[test]
    fn an_auto_merge_is_flagged_in_the_entry() {
        let s = scratch("sum-merged");
        let lock = versions::lock(&s.0).unwrap();
        rec(&lock, 3, &[], "# X\n", 1, false, &[]);
        let mut log = versions::load(&s.0, NOTE).unwrap();
        log.versions[0].event = versions::MERGED.into();
        let got = summarize(&s.0, &log, "plan-x.md", now());
        assert!(got.merged && !got.waits());
    }

    #[test]
    fn left_scopes_cover_30_days_and_stop_when_the_note_is_back() {
        let e = Entry {
            left: vec![
                Left {
                    scope: "personal".into(),
                    at: at(4),
                },
                Left {
                    scope: "personal".into(),
                    at: at(3),
                },
                Left {
                    scope: "work".into(),
                    at: "2026-08-01T12:00:00-03:00".into(),
                },
                Left {
                    scope: "shared".into(),
                    at: "garbled".into(),
                },
            ],
            ..Entry::default()
        };
        assert_eq!(left_scopes(&e, None, now()), ["personal"]);
        assert!(left_scopes(&e, Some("personal"), now()).is_empty());
        assert_eq!(left_scopes(&e, Some("work"), now()), ["personal"]);
    }
}
