mod frontmatter;
mod read;

use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_kinds::kind::{Claims, Kind, Proposal, Refusal};
use bilbo_kinds::reader::{Passage, Reader};
use bilbo_store::id::Id;

const NAME: &str = "markdown";

#[derive(Debug, Clone, Copy, Default)]
pub struct Markdown;

impl Reader for Markdown {
    fn kind(&self) -> &'static str {
        NAME
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        read::passages(records)
    }
}

impl Kind for Markdown {
    fn name(&self) -> &'static str {
        NAME
    }

    fn summary(&self) -> &'static str {
        ".md files, one record per line"
    }

    fn claims(&self, files: &dyn Files, target: &Path) -> Claims {
        let mut claims = Claims::default();
        match files.node(target) {
            Ok(Node::File) => {
                if is_markdown(target) {
                    claims.files.push(target.to_path_buf());
                }
            }
            Ok(Node::Folder) => walk(files, target, &mut claims),
            Ok(Node::Other) => {}
            Err(error) => refuse(&mut claims, target, &error),
        }
        claims
    }

    fn split(&self, bytes: &[u8]) -> Result<Vec<usize>, Refusal> {
        if let Err(error) = std::str::from_utf8(bytes) {
            return Err(Refusal(format!("not UTF-8 (byte {})", error.valid_up_to())));
        }
        let mut ends: Vec<usize> = bytes
            .iter()
            .enumerate()
            .filter(|&(_, &byte)| byte == b'\n')
            .map(|(at, _)| at + 1)
            .collect();
        if bytes.last().is_some_and(|&last| last != b'\n') {
            ends.push(bytes.len());
        }
        Ok(ends)
    }

    fn propose(&self, _path: &Path, bytes: &[u8], _kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal {
        Proposal {
            id: std::str::from_utf8(bytes).ok().and_then(frontmatter::id),
            parent: None,
        }
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "md")
}

fn refuse(claims: &mut Claims, path: &Path, error: &std::io::Error) {
    claims
        .refused
        .push((path.to_path_buf(), format!("cannot read: {error}")));
}

fn walk(files: &dyn Files, folder: &Path, claims: &mut Claims) {
    let names = match files.list(folder) {
        Ok(names) => names,
        Err(error) => return refuse(claims, folder, &error),
    };
    for name in names {
        if name.as_encoded_bytes().first() == Some(&b'.') {
            continue;
        }
        let path: PathBuf = folder.join(&name);
        match files.node(&path) {
            Ok(Node::File) if is_markdown(&path) => claims.files.push(path),
            Ok(Node::Folder) => walk(files, &path, claims),
            Ok(_) => {}
            Err(error) => refuse(claims, &path, &error),
        }
    }
}

#[cfg(test)]
mod tests {
    use bilbo_kinds::files::Memory;

    use super::*;

    const FIXTURES: [(&str, &[u8]); 6] = [
        ("note.md", include_bytes!("../fixtures/note.md")),
        (
            "no-final-newline.md",
            include_bytes!("../fixtures/no-final-newline.md"),
        ),
        ("bad-id.md", include_bytes!("../fixtures/bad-id.md")),
        (
            "no-frontmatter.md",
            include_bytes!("../fixtures/no-frontmatter.md"),
        ),
        ("crlf.md", include_bytes!("../fixtures/crlf.md")),
        ("bom.md", include_bytes!("../fixtures/bom.md")),
    ];

    fn paths(claims: &Claims) -> Vec<&str> {
        claims
            .files
            .iter()
            .map(|path| path.to_str().unwrap_or_default())
            .collect()
    }

    #[test]
    fn a_folder_is_walked_for_md_files_skipping_hidden_entries_and_links() {
        let files = Memory::new()
            .file("/n/a.md", "a")
            .file("/n/sub/b.md", "b")
            .file("/n/.hidden.md", "h")
            .file("/n/.git/c.md", "c")
            .file("/n/x.txt", "x")
            .other("/n/link.md")
            .file("/n/deep/er/d.md", "d");
        let claims = Markdown.claims(&files, Path::new("/n"));
        assert_eq!(
            paths(&claims),
            ["/n/a.md", "/n/deep/er/d.md", "/n/sub/b.md"]
        );
        assert!(claims.refused.is_empty());
    }

    #[test]
    fn a_named_file_is_claimed_only_when_it_is_md() {
        let files = Memory::new()
            .file("/n/a.md", "a")
            .file("/n/.a.md", "a")
            .file("/n/a.txt", "a")
            .file("/n/a.MD", "a");
        for (name, claimed) in [
            ("a.md", true),
            (".a.md", true),
            ("a.txt", false),
            ("a.MD", false),
        ] {
            let claims = Markdown.claims(&files, &Path::new("/n").join(name));
            assert_eq!(claims.files.len(), usize::from(claimed), "{name}");
            assert!(claims.refused.is_empty(), "{name}");
        }
    }

    #[test]
    fn an_unreadable_folder_is_refused_and_the_walk_goes_on() {
        let files = Memory::new()
            .file("/n/a.md", "a")
            .unreadable("/n/locked", Node::Folder)
            .file("/n/z.md", "z");
        let claims = Markdown.claims(&files, Path::new("/n"));
        assert_eq!(paths(&claims), ["/n/a.md", "/n/z.md"]);
        assert_eq!(claims.refused.len(), 1);
        assert_eq!(claims.refused[0].0, Path::new("/n/locked"));
        assert!(claims.refused[0].1.starts_with("cannot read: "));
    }

    #[test]
    fn a_missing_target_is_refused() {
        let claims = Markdown.claims(&Memory::new(), Path::new("/nope"));
        assert!(claims.files.is_empty());
        assert_eq!(claims.refused.len(), 1);
    }

    #[test]
    fn each_line_is_a_record_and_the_records_join_back_to_the_file() {
        for (name, bytes) in FIXTURES {
            let ends = Markdown.split(bytes).unwrap_or_default();
            assert_eq!(ends.last().copied(), Some(bytes.len()), "{name}");
            let mut start = 0;
            let mut joined = Vec::new();
            for end in ends {
                assert!(end > start, "{name}");
                let record = &bytes[start..end];
                assert_eq!(
                    record.iter().filter(|&&byte| byte == b'\n').count(),
                    usize::from(record.ends_with(b"\n")),
                    "{name}"
                );
                joined.extend_from_slice(record);
                start = end;
            }
            assert_eq!(joined, bytes, "{name}");
        }
    }

    #[test]
    fn a_last_line_without_a_newline_is_one_record() {
        assert_eq!(Markdown.split(b"a\nb"), Ok(vec![2, 3]));
        assert_eq!(Markdown.split(b"a\nb\n"), Ok(vec![2, 4]));
    }

    #[test]
    fn an_empty_file_has_no_records() {
        assert_eq!(Markdown.split(b""), Ok(vec![]));
    }

    #[test]
    fn a_crlf_file_keeps_its_carriage_returns() {
        assert_eq!(Markdown.split(b"a\r\nb\r\n"), Ok(vec![3, 6]));
    }

    #[test]
    fn a_file_that_is_not_utf8_is_refused_with_the_byte_it_breaks_at() {
        assert_eq!(
            Markdown.split(b"ab\n\xff\xfe"),
            Err(Refusal("not UTF-8 (byte 3)".to_string()))
        );
        assert_eq!(propose(b"---\n\xff\n"), Proposal::default());
    }

    #[test]
    fn the_kind_is_named_markdown_with_its_summary() {
        assert_eq!(Markdown.name(), "markdown");
        assert_eq!(Markdown.summary(), ".md files, one record per line");
    }

    #[test]
    fn a_note_proposes_its_frontmatter_id_and_no_parent() {
        assert!(propose(FIXTURES[0].1).id.is_some());
        assert_eq!(
            propose(FIXTURES[0].1).parent,
            None,
            "even with another entry kept"
        );
        assert!(propose(FIXTURES[3].1).id.is_none());
    }

    fn propose(bytes: &[u8]) -> Proposal {
        Markdown.propose(Path::new("/n/a.md"), bytes, &|_| {
            Some(Id::from_bytes([7; 16]))
        })
    }
}
