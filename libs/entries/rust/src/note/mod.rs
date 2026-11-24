pub mod record;
mod render;
pub mod written;

use crate::reader::{Passage, Reader};

pub const NAME: &str = "note";

pub const RENDER: &str = "1";

#[derive(Debug, Clone, Copy)]
pub struct Note;

impl Reader for Note {
    fn kind(&self) -> &'static str {
        NAME
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        (1..)
            .zip(records)
            .filter_map(|(number, bytes)| {
                record::read(bytes).map(|found| render::passage(&found, number))
            })
            .collect()
    }

    fn appends_only(&self) -> bool {
        true
    }

    fn stands_alone(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::RecordRange;

    use super::{NAME, Note};
    use crate::reader::Reader;

    const DERIVED: &str = include_str!("../../fixtures/note/derived.jsonl");

    fn records(text: &str) -> Vec<&[u8]> {
        text.split_inclusive('\n').map(str::as_bytes).collect()
    }

    #[test]
    fn each_note_record_is_a_passage_of_one_record_that_stands_alone() {
        let note = Note;
        let found = note.passages(&records(DERIVED));
        let ranges: Vec<RecordRange> = found.iter().map(|passage| passage.range).collect();
        let one = |first| RecordRange { first, count: 1 };
        assert_eq!(ranges, [one(1), one(2), one(3), one(5), one(6)]);
        assert_eq!(note.kind(), NAME);
        assert_eq!(NAME, "note");
        assert!(note.stands_alone() && note.appends_only());
    }

    #[test]
    fn appending_records_changes_no_earlier_passage() {
        let note = Note;
        let all = records(DERIVED);
        let whole = note.passages(&all);
        for end in 0..=all.len() {
            let prefix = note.passages(&all[..end]);
            assert_eq!(prefix, whole[..prefix.len()], "{end}");
        }
    }

    #[test]
    fn nothing_a_record_holds_makes_the_reader_fail() {
        let note = Note;
        let big = format!("{{\"text\":\"{}\"}}\n", "a".repeat(1 << 20));
        let deep = format!("{}{}\n", "[".repeat(10_000), "]".repeat(10_000));
        let held = [
            big.as_bytes(),
            deep.as_bytes(),
            b"{\"text\":\"\0\"}\n",
            b"\xff\xfe\n",
        ];
        let found = note.passages(&held);
        assert_eq!(found.len(), 1);
    }
}
