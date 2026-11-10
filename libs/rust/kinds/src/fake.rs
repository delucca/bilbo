use std::path::{Path, PathBuf};

use crate::files::{Files, Node};
use crate::kind::{Claims, Kind, Proposal, Refusal};

/// Claims the files whose name ends with a suffix; each file is one record.
pub struct Suffix {
    pub name: &'static str,
    pub suffix: &'static str,
    pub refused: Vec<(PathBuf, String)>,
}

impl Suffix {
    pub fn new(name: &'static str, suffix: &'static str) -> Suffix {
        Suffix {
            name,
            suffix,
            refused: Vec::new(),
        }
    }

    fn walk(&self, files: &dyn Files, path: &Path, claims: &mut Claims) {
        match files.node(path) {
            Ok(Node::File) if path.to_string_lossy().ends_with(self.suffix) => {
                claims.files.push(path.to_path_buf());
            }
            Ok(Node::Folder) => {
                for name in files.list(path).unwrap_or_default() {
                    self.walk(files, &path.join(name), claims);
                }
            }
            _ => {}
        }
    }
}

impl Kind for Suffix {
    fn name(&self) -> &'static str {
        self.name
    }

    fn summary(&self) -> &'static str {
        "a fake kind"
    }

    fn claims(&self, files: &dyn Files, target: &Path) -> Claims {
        let mut claims = Claims::default();
        self.walk(files, target, &mut claims);
        claims.refused.clone_from(&self.refused);
        claims
    }

    fn split(&self, bytes: &[u8]) -> Result<Vec<usize>, Refusal> {
        Ok(if bytes.is_empty() {
            Vec::new()
        } else {
            vec![bytes.len()]
        })
    }

    fn propose(
        &self,
        _path: &Path,
        _bytes: &[u8],
        _kept: &dyn Fn(&Path) -> Option<bilbo_store::id::Id>,
    ) -> Proposal {
        Proposal::default()
    }
}

pub struct Chars;

impl crate::tokens::Tokens for Chars {
    fn count(&self, text: &str) -> usize {
        text.chars().count()
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        text.char_indices()
            .nth(budget)
            .map_or(text.len(), |(at, _)| at)
    }
}

pub fn single(record: u64, text: &str) -> crate::reader::Passage {
    section(record, &[text])
}

pub fn section(first: u64, lines: &[&str]) -> crate::reader::Passage {
    let text = lines.concat();
    crate::reader::Passage {
        range: bilbo_store::entries::RecordRange {
            first,
            count: lines.len() as u64,
        },
        pieces: vec![crate::reader::Piece {
            start: 0,
            label: text.lines().next().unwrap_or_default().to_owned(),
        }],
        ends: lines
            .iter()
            .scan(0, |end, line| {
                *end += line.len();
                Some(*end)
            })
            .collect(),
        text,
        at: None,
        outline: None,
    }
}
