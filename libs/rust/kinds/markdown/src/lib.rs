mod frontmatter;
mod read;

use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_kinds::kind::{Claims, Kind, Proposal, Refusal};
use bilbo_kinds::reader::{Passage, Reader};
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Source};
use bilbo_store::id::Id;

const NAME: &str = "markdown";

pub const HEADING_WIDTH: Key<usize> = Key {
    about: About {
        name: "markdown.heading_width",
        variable: Some("BILBO_MARKDOWN_HEADING_WIDTH"),
        default: Default::Toml("100"),
        allowed: "a whole number from 20 to 1000",
        doc: "The most characters a heading may have inside a section label of a Markdown file, an ellipsis included: a longer heading is cut and ends in one.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|width| usize::try_from(width).ok())
            .filter(|width| (20..=1000).contains(width))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

#[derive(Debug, Clone, Copy)]
pub struct Markdown {
    heading_width: usize,
}

impl Markdown {
    pub fn new(heading_width: usize) -> Self {
        Self { heading_width }
    }
}

impl std::default::Default for Markdown {
    fn default() -> Self {
        let file = File::default();
        let width = HEADING_WIDTH
            .resolve(&Source {
                var: &|_| None,
                file: &file,
            })
            .expect("the default is a literal its key's unit test parses");
        Self::new(width.value)
    }
}

impl Reader for Markdown {
    fn kind(&self) -> &'static str {
        NAME
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        read::passages(records, self.heading_width)
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

    fn heading_width(env: Option<&str>, text: &str) -> Result<usize, String> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let var = |name: &str| {
            (name == "BILBO_MARKDOWN_HEADING_WIDTH")
                .then(|| env.map(Into::into))
                .flatten()
        };
        HEADING_WIDTH
            .resolve(&Source {
                var: &var,
                file: &file,
            })
            .map(|resolved| resolved.value)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn the_heading_width_is_20_to_1000_characters_and_defaults_to_100() {
        assert_eq!(heading_width(None, ""), Ok(100));
        assert_eq!(heading_width(None, "markdown.heading_width = 40"), Ok(40));
        assert_eq!(
            heading_width(Some("1000"), "markdown.heading_width = 40"),
            Ok(1000)
        );
        assert_eq!(heading_width(Some("20"), ""), Ok(20));
        for text in ["19", "1001", "0", "-5", "wide", "20.5"] {
            assert_eq!(
                heading_width(Some(text), ""),
                Err(format!(
                    "BILBO_MARKDOWN_HEADING_WIDTH must be a whole number from 20 to 1000, got \"{text}\""
                )),
                "{text}"
            );
        }
        assert_eq!(
            heading_width(None, "markdown.heading_width = 1001"),
            Err("/c/config.toml:1: markdown.heading_width must be a whole number from 20 to 1000, got 1001".to_owned())
        );
    }

    #[test]
    fn a_reader_clips_the_headings_of_a_label_at_its_width() {
        let note = format!("# {}\n## a\n", "h".repeat(30));
        let records: Vec<&[u8]> = note.split_inclusive('\n').map(str::as_bytes).collect();
        for (width, clipped) in [(20, true), (30, false)] {
            let label = Markdown::new(width).passages(&records)[1].pieces[0]
                .label
                .clone();
            assert_eq!(label.contains('…'), clipped, "{width}");
        }
        assert_eq!(
            Markdown::default().passages(&records)[1].pieces[0].label,
            format!("{} > a", "h".repeat(30))
        );
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
        let claims = Markdown::default().claims(&files, Path::new("/n"));
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
            let claims = Markdown::default().claims(&files, &Path::new("/n").join(name));
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
        let claims = Markdown::default().claims(&files, Path::new("/n"));
        assert_eq!(paths(&claims), ["/n/a.md", "/n/z.md"]);
        assert_eq!(claims.refused.len(), 1);
        assert_eq!(claims.refused[0].0, Path::new("/n/locked"));
        assert!(claims.refused[0].1.starts_with("cannot read: "));
    }

    #[test]
    fn a_missing_target_is_refused() {
        let claims = Markdown::default().claims(&Memory::new(), Path::new("/nope"));
        assert!(claims.files.is_empty());
        assert_eq!(claims.refused.len(), 1);
    }

    #[test]
    fn each_line_is_a_record_and_the_records_join_back_to_the_file() {
        for (name, bytes) in FIXTURES {
            let ends = Markdown::default().split(bytes).unwrap_or_default();
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
        assert_eq!(Markdown::default().split(b"a\nb"), Ok(vec![2, 3]));
        assert_eq!(Markdown::default().split(b"a\nb\n"), Ok(vec![2, 4]));
    }

    #[test]
    fn an_empty_file_has_no_records() {
        assert_eq!(Markdown::default().split(b""), Ok(vec![]));
    }

    #[test]
    fn a_crlf_file_keeps_its_carriage_returns() {
        assert_eq!(Markdown::default().split(b"a\r\nb\r\n"), Ok(vec![3, 6]));
    }

    #[test]
    fn a_file_that_is_not_utf8_is_refused_with_the_byte_it_breaks_at() {
        assert_eq!(
            Markdown::default().split(b"ab\n\xff\xfe"),
            Err(Refusal("not UTF-8 (byte 3)".to_string()))
        );
        assert_eq!(propose(b"---\n\xff\n"), Proposal::default());
    }

    #[test]
    fn the_kind_is_named_markdown_with_its_summary() {
        assert_eq!(Markdown::default().name(), "markdown");
        assert_eq!(
            Markdown::default().summary(),
            ".md files, one record per line"
        );
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
        Markdown::default().propose(Path::new("/n/a.md"), bytes, &|_| {
            Some(Id::from_bytes([7; 16]))
        })
    }
}
