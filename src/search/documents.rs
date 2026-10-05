//! The documents search reads: notes and library files, each cut into passages.

use std::path::{Path, PathBuf};

use crate::library::corpus;
use crate::note;
use crate::search::rank;
use crate::shared::markdown;
use crate::shared::store::{EntryKind, entries};

/// A note recall and index read, with its passages.
pub struct Stored {
    pub path: PathBuf,
    pub kind: String,
    pub created: Option<String>,
    /// The note's `scope` value when the key is valid; an invalid or absent key leaves the note unassigned.
    pub scope: Option<String>,
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
        let lines = markdown::lines(&text);
        let read = note::read(&text);
        let stem = entry.name.strip_suffix(".md").unwrap_or(&entry.name);
        stored.push(Stored {
            path: entry.path,
            kind: name.kind,
            created: read.created,
            scope: match read.scope {
                note::ScopeKey::Valid(name) => Some(name),
                _ => None,
            },
            document: rank::Document {
                passages: rank::passages(&lines[read.body_start - 1..], read.body_start, stem),
            },
        });
    }
    Ok(stored)
}

/// Whether a library file is a source or a corpus guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    Source,
    Guide,
}

/// A source or a guide library recall reads, with its passages and the sections they sit in.
pub struct Shelved {
    pub path: PathBuf,
    pub shelf: Shelf,
    /// `<corpus>/<name>` for a source, `<corpus>` for a guide: what the `library` verb takes.
    pub reference: String,
    pub document: rank::Document,
    /// Physical line of the first non-blank line of the body: the title's, when it has one.
    title_line: usize,
    last_line: usize,
    /// Start and end lines of each heading below the title, in line order.
    sections: Vec<(usize, usize)>,
}

impl Shelved {
    /// The lines of the section a passage starting on `line` sits in: the last heading below the title at or before
    /// `line` down to the line before the next heading of its level or above, else from the title to the line before
    /// the first heading below it.
    pub fn section(&self, line: usize) -> (usize, usize) {
        let at = self.sections.partition_point(|(start, _)| *start <= line);
        match at {
            0 => (
                self.title_line,
                self.sections
                    .first()
                    .map_or(self.last_line, |(start, _)| start - 1),
            ),
            n => self.sections[n - 1],
        }
    }
}

/// The sources and guides of the corpus folders `corpora` names, or of every valid corpus folder when it is empty,
/// in path order. Hidden entries, invalid names and files that cannot be read are skipped in silence.
pub fn read_library(root: &Path, corpora: &[String]) -> std::io::Result<Vec<Shelved>> {
    let mut shelved = Vec::new();
    for (name, dir) in corpus::corpus_dirs(root)? {
        if !corpora.is_empty() && !corpora.contains(&name) {
            continue;
        }
        let Ok(files) = entries(&dir) else {
            continue;
        };
        for entry in files {
            if !entry.utf8 || entry.kind != EntryKind::File {
                continue;
            }
            let Some(stem) = entry.name.strip_suffix(".md") else {
                continue;
            };
            let (shelf, reference) = if stem == "guide" {
                (Shelf::Guide, name.clone())
            } else if corpus::is_source_name(stem) {
                (Shelf::Source, format!("{name}/{stem}"))
            } else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&entry.path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let lines = markdown::lines(&text);
            let body_start = note::read(&text).body_start;
            let title_line = (body_start..=lines.len())
                .find(|n| !lines[n - 1].trim().is_empty())
                .unwrap_or(body_start);
            shelved.push(Shelved {
                path: entry.path,
                shelf,
                reference,
                document: rank::Document {
                    passages: rank::passages(
                        lines.get(body_start - 1..).unwrap_or(&[]),
                        body_start,
                        stem,
                    ),
                },
                title_line,
                last_line: lines.len(),
                sections: markdown::outline(&lines, title_line)
                    .into_iter()
                    .map(|s| (s.start, s.end))
                    .collect(),
            });
        }
    }
    Ok(shelved)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn shelf(tag: &str) -> Scratch {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("bilbo-store-{tag}-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&scratch.0);
        std::fs::create_dir_all(scratch.0.join("library/go")).unwrap();
        scratch
    }

    const SOURCE_FRONT: &str = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\nfetched: 2026-10-03\norigin: \"url: https://golang.org/doc/golang\"\ndigest: sha256:0000000000000000000000000000000000000000000000000000000000000000\n---\n";

    fn put(root: &Path, file: &str, text: &str) {
        let path = root.join("library").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn source_text(body: &str) -> String {
        format!("{SOURCE_FRONT}{body}")
    }

    fn only(root: &Path, text: &str) -> Shelved {
        put(root, "go/s.md", text);
        read_library(root, &[]).unwrap().remove(0)
    }

    #[test]
    fn a_note_carries_only_a_valid_scope() {
        let scratch = shelf("scope");
        let notes = scratch.0.join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        let note = |scope: &str| {
            format!(
                "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02T14:23-03:00\n{scope}---\n\n# T\n\ntext\n"
            )
        };
        std::fs::write(notes.join("plan-a.md"), note("scope: work\n")).unwrap();
        std::fs::write(notes.join("plan-b.md"), note("")).unwrap();
        std::fs::write(notes.join("plan-c.md"), note("scope: Work\n")).unwrap();
        std::fs::write(notes.join("plan-d.md"), note("scope: a\nscope: b\n")).unwrap();
        let scopes: Vec<Option<String>> = read_notes(&notes)
            .unwrap()
            .into_iter()
            .map(|n| n.scope)
            .collect();
        assert_eq!(scopes, [Some("work".to_string()), None, None, None]);
    }

    #[test]
    fn a_corpus_reads_its_guide_and_sources_in_name_order() {
        let scratch = shelf("order");
        let root = &scratch.0;
        put(
            root,
            "go/guide.md",
            "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-03T10:00-03:00\n---\n\n# go\n\n## b\n\ntext\n",
        );
        put(root, "go/b.md", &source_text("# B\n\nbee\n"));
        put(root, "go/a.md", &source_text("# A\n\nay\n"));
        let library = read_library(root, &[]).unwrap();
        let seen: Vec<_> = library
            .iter()
            .map(|s| (s.reference.as_str(), s.shelf))
            .collect();
        assert_eq!(
            seen,
            [
                ("go/a", Shelf::Source),
                ("go/b", Shelf::Source),
                ("go", Shelf::Guide)
            ]
        );
        assert_eq!(library[0].path, root.join("library/go/a.md"));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_entries_are_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = shelf("unreadable");
        let root = &scratch.0;
        put(root, "go/ok.md", &source_text("# Ok\n"));
        put(root, "go/locked.md", &source_text("# Locked\n"));
        put(root, "rust/a.md", &source_text("# A\n"));
        let lock = |path: PathBuf, mode: u32| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        lock(root.join("library/go/locked.md"), 0o000);
        lock(root.join("library/rust"), 0o000);
        if std::fs::read(root.join("library/go/locked.md")).is_ok()
            || std::fs::read_dir(root.join("library/rust")).is_ok()
        {
            lock(root.join("library/go/locked.md"), 0o644);
            lock(root.join("library/rust"), 0o755);
            eprintln!("skipped: the entries are readable despite mode 000 (running as root?)");
            return;
        }
        let read = read_library(root, &[]);
        lock(root.join("library/go/locked.md"), 0o644);
        lock(root.join("library/rust"), 0o755);
        let library = read.unwrap();
        let seen: Vec<_> = library.iter().map(|s| s.reference.as_str()).collect();
        assert_eq!(seen, ["go/ok"]);
    }

    #[test]
    fn corpora_narrow_the_read() {
        let scratch = shelf("narrow");
        let root = &scratch.0;
        put(root, "go/a.md", &source_text("# A\n"));
        put(root, "rust/a.md", &source_text("# A\n"));
        let library = read_library(root, &["rust".to_string()]).unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].reference, "rust/a");
    }

    #[test]
    fn invalid_entries_are_skipped() {
        let scratch = shelf("invalid");
        let root = &scratch.0;
        put(root, "go/Effective_Go.md", &source_text("# E\n"));
        put(root, "go/.draft.md", &source_text("# D\n"));
        put(root, "go/sub/deep.md", &source_text("# D\n"));
        put(root, "go/notes.txt", "x");
        put(root, "Go-Old/errors.md", &source_text("# E\n"));
        put(root, "plan/errors.md", &source_text("# E\n"));
        put(root, "go/ok.md", &source_text("# Ok\n"));
        let library = read_library(root, &[]).unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].reference, "go/ok");
    }

    #[test]
    fn frontmatter_words_are_in_no_passage() {
        let scratch = shelf("front");
        let s = only(&scratch.0, &source_text("# Title\n\nbody\n"));
        let all: String = s
            .document
            .passages
            .iter()
            .map(|p| format!("{} {}", p.path.join(" "), p.text))
            .collect();
        assert!(!all.contains("golang") && !all.contains("digest"), "{all}");
    }

    #[test]
    fn a_bad_digest_is_still_read() {
        let scratch = shelf("digest");
        let s = only(&scratch.0, &source_text("# Title\n\nedited body\n"));
        assert_eq!(s.document.passages.len(), 1);
    }

    #[test]
    fn a_source_without_a_title_uses_its_file_stem() {
        let scratch = shelf("stem");
        let s = only(&scratch.0, &source_text("just text\n"));
        assert_eq!(s.document.passages[0].path, ["s"]);
    }

    fn body() -> String {
        // The front takes lines 1 to 6.
        source_text(
            "# Title\n\
             intro\n\
             ## Concurrency\n\
             own text\n\
             ### Goroutines\n\
             ```\n\
             ## not a heading\n\
             ```\n\
             more\n\
             ### Channels\n\
             chan\n\
             ## Errors\n\
             err\n",
        )
    }

    #[test]
    fn the_section_of_a_level_3_heading() {
        let scratch = shelf("l3");
        let s = only(&scratch.0, &body());
        // Title 7, intro 8, Concurrency 9, Goroutines 11, Channels 16, Errors 18..19.
        assert_eq!(s.section(11), (11, 15));
        assert_eq!(s.section(13), (11, 15));
    }

    #[test]
    fn the_section_of_a_level_2_heading_holds_its_subsections() {
        let scratch = shelf("l2");
        let s = only(&scratch.0, &body());
        assert_eq!(s.section(9), (9, 17));
        assert_eq!(s.section(18), (18, 19));
    }

    #[test]
    fn the_title_passage_runs_to_the_first_heading() {
        let scratch = shelf("title");
        let s = only(&scratch.0, &body());
        assert_eq!(s.section(7), (7, 8));
    }

    #[test]
    fn a_later_part_of_a_split_passage_keeps_its_section() {
        let scratch = shelf("parts");
        let para = "word ".repeat(300);
        let text = source_text(&format!(
            "# Title\n\n## Big\n\n{para}\n\n{para}\n\n{para}\n\n## Next\n\nx\n"
        ));
        let s = only(&scratch.0, &text);
        let parts: Vec<_> = s
            .document
            .passages
            .iter()
            .filter(|p| p.path.last().is_some_and(|l| l == "Big"))
            .collect();
        assert!(parts.len() > 1);
        let later = parts[1].line;
        assert!(later > parts[0].line);
        assert_eq!(s.section(later), s.section(parts[0].line));
    }

    #[test]
    fn a_source_with_no_heading_below_its_title() {
        let scratch = shelf("flat");
        let s = only(&scratch.0, &source_text("# Title\n\ntext\nmore\n"));
        assert_eq!(s.section(7), (7, 10));
        assert_eq!(s.section(9), (7, 10));
    }

    #[test]
    fn a_guide_entry_ends_before_the_next_entry() {
        let scratch = shelf("guide");
        let root = &scratch.0;
        put(
            root,
            "go/guide.md",
            "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-03T10:00-03:00\n---\n\n# go\n\nabout\n\n## one\n\ntext\n\n## two\n\ntext\n",
        );
        let s = read_library(root, &[]).unwrap().remove(0);
        // Title 6, about 8, one 10, two 14..16.
        assert_eq!(s.section(6), (6, 9));
        assert_eq!(s.section(10), (10, 13));
        assert_eq!(s.section(14), (14, 16));
    }
}
