use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::entries::StoreError;
use crate::id::Id;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaptureLabel(String);

impl CaptureLabel {
    /// `file:` and the path; `None` unless the path is absolute and UTF-8. No I/O: callers pass a canonical path.
    #[must_use]
    pub fn file(path: &Path) -> Option<CaptureLabel> {
        if !path.is_absolute() {
            return None;
        }
        Some(CaptureLabel(format!("file:{}", path.to_str()?)))
    }

    #[must_use]
    pub fn new(text: String) -> Option<CaptureLabel> {
        if text.is_empty() {
            None
        } else {
            Some(CaptureLabel(text))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.0.strip_prefix("file:").map(Path::new)
    }
}

impl fmt::Display for CaptureLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Label {
    Capture(CaptureLabel),
    Id(Id),
    Text(String),
}

/// The meta frame has one field for each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredLabels {
    pub capture: Option<CaptureLabel>,
    pub id: Option<Id>,
}

impl StoredLabels {
    #[must_use]
    pub fn list(&self) -> Vec<Label> {
        self.capture
            .iter()
            .cloned()
            .map(Label::Capture)
            .chain(self.id.map(Label::Id))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worded {
    pub text: String,
    pub words: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    pub entry: Id,
    pub labels: Vec<Worded>,
}

pub trait Labels {
    /// The entries with a text label that holds `words` next to each other and in order, in id order.
    /// No word: no entry.
    fn with_words(&self, words: &[String]) -> Result<Vec<Id>, StoreError>;
    /// The entries that belong to one of `entries` through id labels, however deep, without those given,
    /// in id order.
    fn belonging_to(&self, entries: &[Id]) -> Result<Vec<Id>, StoreError>;
}

pub trait Relabel {
    /// Each entry gets, in place of the ones it had, the text labels of the last item that names it; none
    /// when that item has none. An entry not given keeps its rows. All or nothing.
    fn relabel(&self, filed: &[Filed]) -> Result<(), StoreError>;
}

pub(crate) fn descendants(links: &[(Id, Id)], of: &[Id]) -> Vec<Id> {
    let mut below: HashMap<Id, Vec<Id>> = HashMap::new();
    for &(entry, above) in links {
        below.entry(above).or_default().push(entry);
    }
    let mut seen: HashSet<Id> = of.iter().copied().collect();
    let mut found = Vec::new();
    let mut next: Vec<Id> = seen.iter().copied().collect();
    while let Some(above) = next.pop() {
        for &entry in below.get(&above).map_or(&[][..], Vec::as_slice) {
            if seen.insert(entry) {
                found.push(entry);
                next.push(entry);
            }
        }
    }
    found.sort_unstable();
    found
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    use super::*;

    fn id(n: u8) -> Id {
        Id::from_bytes([n; 16])
    }

    #[test]
    fn a_capture_label_is_a_files_absolute_utf8_path_or_any_text_but_the_empty_one() {
        let label = CaptureLabel::file(Path::new("/notes/a.md")).expect("absolute");
        assert_eq!(label.as_str(), "file:/notes/a.md");
        assert_eq!(label.to_string(), "file:/notes/a.md");
        assert_eq!(label.path(), Some(Path::new("/notes/a.md")));
        assert_eq!(CaptureLabel::file(Path::new("notes/a.md")), None);
        let bytes = PathBuf::from(OsString::from_vec(b"/notes/\xFF.md".to_vec()));
        assert_eq!(CaptureLabel::file(&bytes), None);
        assert_eq!(CaptureLabel::new(String::new()), None);
        assert_eq!(
            CaptureLabel::new("stdin".into()).and_then(|o| o.path().map(Path::to_path_buf)),
            None
        );
    }

    #[test]
    fn the_stored_labels_list_the_capture_label_first_then_the_id_label() {
        let capture = CaptureLabel::file(Path::new("/n/a.md")).expect("label");
        let both = StoredLabels {
            capture: Some(capture.clone()),
            id: Some(id(1)),
        };
        assert_eq!(
            both.list(),
            vec![Label::Capture(capture.clone()), Label::Id(id(1))]
        );
        let only_capture = StoredLabels {
            capture: Some(capture.clone()),
            id: None,
        };
        assert_eq!(only_capture.list(), vec![Label::Capture(capture)]);
        let only_id = StoredLabels {
            capture: None,
            id: Some(id(1)),
        };
        assert_eq!(only_id.list(), vec![Label::Id(id(1))]);
        assert_eq!(StoredLabels::default().list(), Vec::<Label>::new());
    }

    #[test]
    fn what_belongs_to_some_entries_is_found_however_deep_and_a_cycle_ends() {
        let (a, b, c, d) = (id(1), id(2), id(3), id(4));
        let chain = [(b, a), (c, b), (d, c)];
        assert_eq!(descendants(&chain, &[a]), vec![b, c, d]);
        assert_eq!(descendants(&chain, &[b]), vec![c, d]);
        assert_eq!(descendants(&chain, &[a, c]), vec![b, d]);
        assert_eq!(descendants(&chain, &[]), Vec::<Id>::new());
        assert_eq!(descendants(&chain, &[d]), Vec::<Id>::new());
        let cycle = [(a, b), (b, a), (c, a)];
        assert_eq!(descendants(&cycle, &[a]), vec![b, c]);
        assert_eq!(descendants(&cycle, &[c]), Vec::<Id>::new());
        assert_eq!(descendants(&[(a, a)], &[a]), Vec::<Id>::new());
    }

    #[test]
    fn a_chain_of_a_hundred_thousand_is_walked_once() {
        let at = |n: u32| {
            let mut raw = [0u8; 16];
            raw[12..].copy_from_slice(&n.to_be_bytes());
            Id::from_bytes(raw)
        };
        let links: Vec<(Id, Id)> = (1..100_000).map(|n| (at(n), at(n - 1))).collect();
        let found = descendants(&links, &[at(0)]);
        assert_eq!(found.len(), 99_999);
        assert_eq!(found.first(), Some(&at(1)));
        assert_eq!(found.last(), Some(&at(99_999)));
    }
}
