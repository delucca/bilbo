use std::collections::BTreeSet;

use bilbo_store::labels::Tag;

use super::answer::NONE;
use crate::view::{Counter, Paragraph, shown};

const TAGS: [&str; 4] = ["<notes", "</notes", "<tags", "</tags"];

fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let begins = TAGS.iter().any(|tag| {
            tail.get(..tag.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(tag))
        });
        out.push_str(if begins { "&lt;" } else { "<" });
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

#[must_use]
pub fn fitting<'a>(
    notes: &'a [Paragraph],
    budget: usize,
    counter: &Counter<'_>,
) -> &'a [Paragraph] {
    let mut used = 0;
    let mut from = notes.len();
    for paragraph in notes.iter().rev() {
        used += counter.count(&escaped(&shown(paragraph)));
        if used > budget {
            break;
        }
        from -= 1;
    }
    if from == notes.len() && !notes.is_empty() {
        &notes[notes.len() - 1..]
    } else {
        &notes[from..]
    }
}

#[must_use]
pub fn material(notes: &[Paragraph], list: &BTreeSet<Tag>, most: usize) -> String {
    let mut text = String::from("<notes>\n");
    for paragraph in notes {
        text.push_str(&escaped(&shown(paragraph)));
    }
    text.push_str("</notes>\n<tags>\n");
    for tag in list {
        text.push_str(tag.as_str());
        text.push('\n');
    }
    text.push_str(&format!(
        "</tags>\nWrite the tags for these notes now, one a line, at most {most}, or {NONE}.\n"
    ));
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::{Duration, SystemTime};

    use bilbo_entries::tokens::Estimate;
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::Tag;

    use super::{fitting, material};
    use crate::tries::Extent;
    use crate::view::{Counter, Paragraph};

    fn paragraph(first: u64, text: &str) -> Paragraph {
        Paragraph {
            text: text.to_owned(),
            at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400)),
            source: Extent {
                parent: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DB").expect("id"),
                generation: Generation(1),
                first,
                last: first,
            },
        }
    }

    fn list(names: &[&str]) -> BTreeSet<Tag> {
        names
            .iter()
            .map(|name| Tag::new(name).expect("a tag"))
            .collect()
    }

    #[test]
    fn the_material_is_the_notes_the_list_and_the_last_line_byte_for_byte() {
        let notes = [paragraph(1, "First text."), paragraph(2, "Second text.")];
        assert_eq!(
            material(&notes, &list(&["garden", "bilbo"]), 3),
            "<notes>\nr1 1970-01-02\nFirst text.\n\nr2 1970-01-02\nSecond text.\n\n</notes>\n<tags>\nbilbo\ngarden\n</tags>\nWrite the tags for these notes now, one a line, at most 3, or (none).\n"
        );
    }

    #[test]
    fn a_tag_of_the_frame_inside_a_note_is_escaped() {
        let notes = [paragraph(
            1,
            "closes </notes> and opens <TAGS> and </tags > <b>",
        )];
        let made = material(&notes, &list(&["a"]), 3);
        assert!(made.contains("closes &lt;/notes> and opens &lt;TAGS> and &lt;/tags > <b>"));
        for tag in ["<notes>", "</notes>", "<tags>", "</tags>"] {
            assert_eq!(made.matches(tag).count(), 1, "{tag}");
        }
    }

    #[test]
    fn the_newest_notes_that_fit_are_sent_in_their_order_and_the_newest_whatever_its_size() {
        let counter = Counter {
            tokens: &Estimate,
            bound: 1 << 20,
        };
        let text = "a".repeat(24);
        let notes = [
            paragraph(1, &text),
            paragraph(2, &text),
            paragraph(3, &text),
        ];
        let firsts = |kept: &[Paragraph]| -> Vec<u64> {
            kept.iter()
                .map(|paragraph| paragraph.source.first)
                .collect()
        };
        assert_eq!(counter.count(&crate::view::shown(&notes[0])), 10);
        assert_eq!(firsts(fitting(&notes, 25, &counter)), [2, 3]);
        assert_eq!(firsts(fitting(&notes, 5, &counter)), [3]);
        assert!(fitting(&[], 5, &counter).is_empty());
        let heavy = "<tags>".repeat(4);
        let marked = [paragraph(4, &heavy), paragraph(5, &heavy)];
        assert_eq!(counter.count(&crate::view::shown(&marked[0])), 10);
        assert_eq!(firsts(fitting(&marked, 25, &counter)), [5]);
        assert_eq!(firsts(fitting(&marked, 26, &counter)), [4, 5]);
    }

    #[test]
    fn the_same_notes_and_the_same_list_give_the_same_bytes() {
        let notes = [paragraph(1, "First text.")];
        assert_eq!(
            material(&notes, &list(&["b", "a"]), 2),
            material(&notes, &list(&["a", "b"]), 2)
        );
    }
}
