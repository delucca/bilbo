use bilbo_entries::tag::{Carrier, Output};
use bilbo_store::labels::{By, Tagged};

use crate::console::message::line;
use crate::console::view::View;

pub fn tags(items: &[Tagged], view: &View) -> String {
    let mut text = String::new();
    let mut at = 0;
    while at < items.len() {
        let tag = &items[at].tag;
        let kinds: Vec<&Tagged> = items[at..]
            .iter()
            .take_while(|item| item.tag == *tag)
            .collect();
        let entries: u64 = kinds.iter().map(|item| item.entries).sum();
        let by_kind: Vec<String> = kinds
            .iter()
            .map(|item| format!("{} {}", line(view, &item.kind), item.entries))
            .collect();
        text.push_str(&format!(
            "{tag}\t{entries} entries\t{}\n",
            by_kind.join(", ")
        ));
        at += kinds.len();
    }
    text
}

pub fn entries(carriers: &[Carrier], view: &View) -> String {
    carriers
        .iter()
        .map(|carrier| {
            let capture = carrier
                .capture
                .as_ref()
                .map_or_else(|| "-".to_owned(), |label| line(view, label.as_str()));
            let by = match carrier.by {
                By::Person => "person",
                By::Model => "model",
            };
            format!(
                "{}\t{}\t{capture}\t{by}\n",
                carrier.id,
                line(view, &carrier.kind)
            )
        })
        .collect()
}

pub fn changed(output: &Output) -> String {
    format!(
        "changed {}, unchanged {}\n",
        output.changed, output.unchanged
    )
}

#[cfg(test)]
mod tests {
    use bilbo_entries::tag::Carrier;
    use bilbo_store::id::Id;
    use bilbo_store::labels::{By, CaptureLabel, Tag, Tagged};

    use super::{entries, tags};
    use crate::console::view::View;

    fn tagged(name: &str, kind: &str, entries: u64) -> Tagged {
        Tagged {
            tag: Tag::new(name).expect("a tag"),
            kind: kind.to_owned(),
            entries,
        }
    }

    fn carrier(capture: Option<&str>, by: By) -> Carrier {
        Carrier {
            id: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DB").expect("an id"),
            kind: "session".to_owned(),
            capture: capture.and_then(|text| CaptureLabel::new(text.to_owned())),
            by,
        }
    }

    #[test]
    fn the_tags_are_one_row_a_tag_with_its_count_and_its_kinds() {
        let items = [
            tagged("garden", "markdown", 4),
            tagged("garden", "session", 209),
        ];
        assert_eq!(
            tags(&items, &View::PLAIN),
            "tag:garden\t213 entries\tmarkdown 4, session 209\n"
        );
    }

    #[test]
    fn an_entry_of_a_tag_is_four_fields_by_tabs() {
        assert_eq!(
            entries(
                &[
                    carrier(Some("file:/n/a.jsonl"), By::Model),
                    carrier(None, By::Person)
                ],
                &View::PLAIN
            ),
            "01JC5Q3V7XKM4P8R2T6W9YB0DB\tsession\tfile:/n/a.jsonl\tmodel\n\
             01JC5Q3V7XKM4P8R2T6W9YB0DB\tsession\t-\tperson\n"
        );
    }

    #[test]
    fn a_capture_label_with_a_tab_or_a_line_feed_stays_one_field_of_one_row() {
        let text = entries(
            &[carrier(Some("file:/n/a\tb\nc"), By::Person)],
            &View::PLAIN,
        );
        assert_eq!(text.lines().count(), 1);
        let fields: Vec<&str> = text.trim_end_matches('\n').split('\t').collect();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[2], "file:/n/a\\tb\\nc");
    }

    #[test]
    fn a_kind_is_escaped_in_both_listings_as_read_escapes_it() {
        let kind = "mark\tdown\nx";
        let listed = tags(&[tagged("garden", kind, 4)], &View::PLAIN);
        assert_eq!(listed, "tag:garden\t4 entries\tmark\\tdown\\nx 4\n");
        let mut one = carrier(Some("file:/n/a"), By::Person);
        one.kind = kind.to_owned();
        let row = entries(&[one], &View::PLAIN);
        assert_eq!(row.lines().count(), 1);
        assert_eq!(row.trim_end_matches('\n').split('\t').count(), 4);
        assert_eq!(
            row.trim_end_matches('\n').split('\t').nth(1),
            Some("mark\\tdown\\nx")
        );

        let bidi = "mark\u{202e}down";
        let shown = tags(&[tagged("garden", bidi, 4)], &View::HUMAN);
        assert!(!shown.contains('\u{202e}'), "{shown:?}");
        assert!(shown.contains("mark\\u{202e}down"), "{shown:?}");
        let mut two = carrier(Some("file:/n/a"), By::Person);
        two.kind = bidi.to_owned();
        let shown = entries(&[two], &View::HUMAN);
        assert!(!shown.contains('\u{202e}'), "{shown:?}");
        assert_eq!(shown.split('\t').nth(1), Some("mark\\u{202e}down"));
    }
}
