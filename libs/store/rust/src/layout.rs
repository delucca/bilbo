use std::path::PathBuf;

/// The store's three folders, as the settings resolved them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}
