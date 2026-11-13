use std::path::{Path, PathBuf};

use bilbo_store::id::Id;

use crate::files::Files;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    origin: PathBuf,
    files: Vec<PathBuf>,
}

impl Claim {
    #[must_use]
    pub fn file(path: PathBuf) -> Claim {
        Claim {
            files: vec![path.clone()],
            origin: path,
        }
    }

    #[must_use]
    pub fn or(mut self, file: PathBuf) -> Claim {
        self.files.push(file);
        self
    }

    #[must_use]
    pub fn origin(&self) -> &Path {
        &self.origin
    }

    /// Never empty: the first that is a file is read.
    #[must_use]
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Claims {
    pub entries: Vec<Claim>,
    pub refused: Vec<(PathBuf, String)>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Proposal {
    pub id: Option<Id>,
    /// The entry this one belongs to, such as a subagent's session.
    pub parent: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

pub trait Kind {
    fn name(&self) -> &'static str;
    /// One line for the help page.
    fn summary(&self) -> &'static str;
    /// The entries under `target` this kind takes, in a stable order, one per origin. `target` is
    /// canonical, or names a file that is gone in a canonical folder.
    fn claims(&self, files: &dyn Files, target: &Path) -> Claims;
    /// What `file` holds once read: the entry's bytes.
    fn unpack(&self, _file: &Path, bytes: Vec<u8>) -> Result<Vec<u8>, Refusal> {
        Ok(bytes)
    }
    /// Bytes after the last end are not a record yet (a line still being written): a later run
    /// keeps them.
    fn split(&self, bytes: &[u8]) -> Result<Vec<usize>, Refusal>;
    /// `origin` is the claim's; `kept` gives the entry kept for another origin, if any.
    fn propose(&self, origin: &Path, bytes: &[u8], kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal;
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Claim, Kind, Refusal};
    use crate::fake::Suffix;

    #[test]
    fn a_plain_claim_is_kept_under_the_file_it_reads() {
        let claim = Claim::file(PathBuf::from("/t/a.md"));
        assert_eq!(claim.origin(), Path::new("/t/a.md"));
        assert_eq!(claim.files(), [PathBuf::from("/t/a.md")]);
    }

    #[test]
    fn a_claim_tries_its_own_file_first_and_then_each_one_added() {
        let claim = Claim::file(PathBuf::from("/t/a"))
            .or(PathBuf::from("/t/a.z"))
            .or(PathBuf::from("/t/a.y"));
        assert_eq!(claim.origin(), Path::new("/t/a"));
        assert_eq!(
            claim.files(),
            [
                PathBuf::from("/t/a"),
                PathBuf::from("/t/a.z"),
                PathBuf::from("/t/a.y")
            ]
        );
    }

    #[test]
    fn a_kind_that_packs_nothing_hands_the_bytes_on_as_read() {
        let bytes = b"as read".to_vec();
        let kind = Suffix::new("one", ".one");
        assert_eq!(kind.unpack(Path::new("/t/a"), bytes.clone()), Ok(bytes));
    }

    #[test]
    fn a_refusal_is_an_error_that_displays_its_reason() {
        let refusal = Refusal("not UTF-8 (byte 3)".to_owned());
        assert_eq!(refusal.to_string(), "not UTF-8 (byte 3)");
        let error: &dyn std::error::Error = &refusal;
        assert_eq!(error.to_string(), "not UTF-8 (byte 3)");
    }
}
