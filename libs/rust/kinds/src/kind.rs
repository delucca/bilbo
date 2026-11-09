use std::path::{Path, PathBuf};

use bilbo_store::id::Id;

use crate::files::Files;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Claims {
    pub files: Vec<PathBuf>,
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
    /// The files under `target` (canonical) this kind takes, in a stable order.
    fn claims(&self, files: &dyn Files, target: &Path) -> Claims;
    /// Bytes after the last end are not a record yet (a line still being written): a later run
    /// keeps them.
    fn split(&self, bytes: &[u8]) -> Result<Vec<usize>, Refusal>;
    /// `kept` gives the entry kept for another file, if any.
    fn propose(&self, path: &Path, bytes: &[u8], kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal;
}

#[cfg(test)]
mod tests {
    use super::Refusal;

    #[test]
    fn a_refusal_is_an_error_that_displays_its_reason() {
        let refusal = Refusal("not UTF-8 (byte 3)".to_owned());
        assert_eq!(refusal.to_string(), "not UTF-8 (byte 3)");
        let error: &dyn std::error::Error = &refusal;
        assert_eq!(error.to_string(), "not UTF-8 (byte 3)");
    }
}
