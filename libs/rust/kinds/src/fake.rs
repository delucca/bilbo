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

    fn propose(&self, _bytes: &[u8]) -> Proposal {
        Proposal::default()
    }
}
