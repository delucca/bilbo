pub mod answer;
pub mod asked;
pub mod block;
pub mod instruction;
pub mod keys;
pub mod plan;
pub mod record;
pub mod view;

use bilbo_store::entries::Head;
use bilbo_store::id::Id;
use bilbo_store::labels::CaptureLabel;

pub const KIND: &str = "summary";

pub struct Rules<'a> {
    pub section: &'a keys::Section,
    pub instruction: &'a str,
    pub id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub asking: Option<(u64, u8)>,
    pub sent: usize,
    pub made: Option<view::Sent>,
    pub retire: bool,
    pub head: Head,
    pub notes: Option<u64>,
    pub record: Option<record::Summary>,
}

pub(crate) fn content_of(text: &str) -> Option<u128> {
    (text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u128::from_str_radix(text, 16).ok())
        .flatten()
}

#[must_use]
pub fn label(parent: Id) -> CaptureLabel {
    CaptureLabel::new(format!("{KIND}:{parent}"))
        .expect("a capture label of a summary is not empty")
}

#[cfg(test)]
mod tests {
    use bilbo_store::id::Id;

    use super::{KIND, label};

    #[test]
    fn the_label_is_summary_and_the_entrys_id() {
        let first = Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("id");
        let second = Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAG").expect("id");
        assert_eq!(label(first).as_str(), "summary:01M6ZGE4KTBYYH07AS2KVMSCAF");
        assert_eq!(label(first).as_str().len(), 34);
        assert_ne!(label(first), label(second));
        assert_ne!(label(first), crate::plan::label(first));
        assert_eq!(KIND, "summary");
    }
}
