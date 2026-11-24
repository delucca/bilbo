use std::time::{SystemTime, UNIX_EPOCH};

use bilbo_store::id::Id;
use bilbo_store::labels::CaptureLabel;

use super::record::{self, Record};

#[must_use]
pub fn record(text: &str, at: SystemTime) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(record::write(&Record {
        text: text.to_owned(),
        at: at
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|since| since.as_secs()),
        from: Vec::new(),
    }))
}

#[must_use]
pub fn label(id: Id) -> CaptureLabel {
    CaptureLabel::new(format!("note:{id}")).expect("`note:` and an id are never empty")
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use bilbo_store::id::Id;

    use super::{label, record};
    use crate::note::Note;
    use crate::note::record as parsed;
    use crate::reader::Reader;

    #[test]
    fn a_written_record_holds_its_text_and_its_time_and_no_source() {
        let at = UNIX_EPOCH + Duration::from_secs(1_794_039_249);
        let bytes = record(
            "\n\n  The beet seeds are in the drawer.\nThe radishes are not.  \n\n",
            at,
        )
        .expect("a record");
        let found = parsed::read(&bytes).expect("the reader takes it");
        assert_eq!(
            found.text,
            "The beet seeds are in the drawer.\nThe radishes are not."
        );
        assert_eq!(found.at, Some(1_794_039_249));
        assert!(found.from.is_empty());
        let passages = Note.passages(&[&bytes]);
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].text.lines().next(), Some("2026-11-07"));
        assert_eq!(record("", at), None);
        assert_eq!(record(" \n\t \n", at), None);
    }

    #[test]
    fn a_written_notes_label_is_its_own_id() {
        let one = Id::from_bytes([1; 16]);
        let two = Id::from_bytes([2; 16]);
        assert_eq!(label(one).as_str(), format!("note:{one}"));
        assert_eq!(label(one).as_str().len(), "note:".len() + 26);
        assert_ne!(label(one), label(two));
    }
}
