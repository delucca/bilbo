use bilbo_entries::remember::{self, Output};
use bilbo_store::labels::Tag;

use crate::console::message::{Message, new_tags};

pub fn refusals(outputs: &[Output]) -> Vec<Message> {
    outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| match &item.outcome {
            remember::Outcome::Refused(reason) => Some(Message::warning(&format!(
                "refused {}: {reason}",
                item.path.display()
            ))),
            _ => None,
        })
        .collect()
}

pub fn moves(outputs: &[Output]) -> Vec<Message> {
    outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| {
            let moved = item.moved.as_ref()?;
            Some(Message::info(&format!(
                "moved {}: its id {} was held by {}",
                item.path.display(),
                moved.id,
                moved.from
            )))
        })
        .collect()
}

pub fn withdrawals(outputs: &[Output]) -> Vec<Message> {
    outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| {
            let (remember::Outcome::NewGeneration(id), Some(generation)) =
                (&item.outcome, item.withdrew)
            else {
                return None;
            };
            Some(Message::info(&format!(
                "withdrew {}: its file is gone; bilbo read {id}#g{} prints the outline of what it held",
                item.path.display(),
                generation.0
            )))
        })
        .collect()
}

pub fn tagged(output: &Output, fresh: &[Tag]) -> Vec<Message> {
    if output.tagged == 0 {
        return Vec::new();
    }
    std::iter::once(Message::info(&format!("tagged {} entries", output.tagged)))
        .chain(new_tags(fresh))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_entries::remember::{Item, Moved, Outcome, Output};
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::CaptureLabel;

    use super::{moves, refusals, withdrawals};
    use crate::console::theme::Mark;

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

    fn output(items: Vec<Item>) -> Output {
        Output {
            target: PathBuf::from("/home/me/notes"),
            kind: Some("markdown".to_owned()),
            claimed: items.len() as u64,
            items,
            records: 0,
            raw_bytes: 0,
            stored_bytes: 0,
            tagged: 0,
        }
    }

    fn item(path: &str, outcome: Outcome, moved: bool) -> Item {
        Item {
            path: PathBuf::from(path),
            outcome,
            moved: moved.then(|| Moved {
                id: Id::parse(ID).expect("an id"),
                from: CaptureLabel::new("file:/home/me/notes/decision-shed.md".into())
                    .expect("a label"),
                bytes: 90,
            }),
            carried: None,
            withdrew: None,
        }
    }

    fn lines(messages: &[crate::console::message::Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(crate::console::message::Message::plain_lines)
            .collect()
    }

    #[test]
    fn a_moved_file_says_the_label_its_id_was_held_by() {
        let id = Id::parse(ID).expect("an id");
        let output = output(vec![
            item("/home/me/notes/a.md", Outcome::Unchanged(id), false),
            item(
                "/home/me/notes/report-shed.md",
                Outcome::Unchanged(id),
                true,
            ),
            item("/home/me/notes/b.md", Outcome::Unchanged(id), true),
        ]);
        let messages = moves(&[output]);
        assert!(messages.iter().all(|message| message.mark == Mark::Info));
        assert_eq!(
            lines(&messages),
            [
                format!(
                    "bilbo: moved /home/me/notes/report-shed.md: its id {ID} was held by file:/home/me/notes/decision-shed.md"
                ),
                format!(
                    "bilbo: moved /home/me/notes/b.md: its id {ID} was held by file:/home/me/notes/decision-shed.md"
                ),
            ]
        );
    }

    #[test]
    fn a_file_that_moved_and_was_then_refused_says_both() {
        let output = output(vec![item(
            "/home/me/notes/report-shed.md",
            Outcome::Refused("its entry would carry more than tag.cap tags (2)".to_owned()),
            true,
        )]);
        assert_eq!(
            lines(&refusals(std::slice::from_ref(&output))),
            [
                "bilbo: refused /home/me/notes/report-shed.md: its entry would carry more than tag.cap tags (2)"
            ]
        );
        assert_eq!(
            lines(&moves(std::slice::from_ref(&output))),
            [format!(
                "bilbo: moved /home/me/notes/report-shed.md: its id {ID} was held by file:/home/me/notes/decision-shed.md"
            )]
        );
    }

    #[test]
    fn a_withdrawn_entry_says_how_to_read_what_it_held() {
        let id = Id::parse(ID).expect("an id");
        let mut gone = item(
            "/home/me/notes/decision-shed.md",
            Outcome::NewGeneration(id),
            false,
        );
        gone.withdrew = Some(Generation(2));
        let output = output(vec![
            item("/home/me/notes/a.md", Outcome::Unchanged(id), false),
            item("/home/me/notes/b.md", Outcome::NewGeneration(id), false),
            gone,
        ]);
        let messages = withdrawals(&[output]);
        assert!(messages.iter().all(|message| message.mark == Mark::Info));
        assert_eq!(
            lines(&messages),
            [format!(
                "bilbo: withdrew /home/me/notes/decision-shed.md: its file is gone; bilbo read {ID}#g2 prints the outline of what it held"
            )]
        );
    }
}
