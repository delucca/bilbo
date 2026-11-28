use bilbo_entries::remember::{self, Output};
use bilbo_store::labels::Tag;

use crate::console::message::{Message, new_tags, renders_as_written, shell_word};

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

pub fn passed(outputs: &[Output]) -> Vec<Message> {
    outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| {
            let Some(remember::Met::Own { id, .. }) = item.forgotten.as_ref() else {
                return None;
            };
            let path = item.path.to_string_lossy();
            let said = format!("passed over {path}: its entry {id} was forgotten");
            Some(Message::info(&if renders_as_written(&path) {
                format!(
                    "{said}; bilbo remember {} keeps it again",
                    shell_word(&path)
                )
            } else {
                said
            }))
        })
        .collect()
}

pub fn again(outputs: &[Output]) -> Vec<Message> {
    outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| {
            let remember::Met::Again { id, .. } = item.forgotten.as_ref()? else {
                return None;
            };
            let path = item.path.display();
            Some(Message::info(&match item.outcome {
                remember::Outcome::Refused(_) => format!(
                    "took {path} back as entry {id}, which was forgotten; its file was refused"
                ),
                _ => format!("kept {path} again: its entry {id} was forgotten"),
            }))
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

    use bilbo_entries::remember::{Item, Met, Moved, Outcome, Output};
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;
    use bilbo_store::labels::CaptureLabel;

    use super::{again, moves, passed, refusals, withdrawals};
    use crate::console::theme::Mark;
    use crate::console::view::View;

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
            forgotten: None,
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

    #[test]
    fn a_file_passed_over_says_whose_it_was_and_how_to_keep_it_again() {
        let id = Id::parse(ID).expect("an id");
        let mut over = item("/home/me/notes/shed.md", Outcome::Unchanged(id), false);
        over.forgotten = Some(Met::Own { id, bytes: 0 });
        let mut odd = item("/home/me/notes/x$(id)'s.md", Outcome::Unchanged(id), false);
        odd.forgotten = Some(Met::Own { id, bytes: 0 });
        let mut broken = item("/home/me/notes/x\ny.md", Outcome::Unchanged(id), false);
        broken.forgotten = Some(Met::Own { id, bytes: 0 });
        let mut reordering = item(
            "/home/me/notes/x\u{202e}y.md",
            Outcome::Unchanged(id),
            false,
        );
        reordering.forgotten = Some(Met::Own { id, bytes: 0 });
        let output = output(vec![
            item("/home/me/notes/a.md", Outcome::Unchanged(id), false),
            over,
            odd,
            broken,
            reordering,
        ]);
        let messages = passed(&[output]);
        assert!(messages.iter().all(|message| message.mark == Mark::Info));
        assert_eq!(
            lines(&messages),
            [
                format!(
                    "bilbo: passed over /home/me/notes/shed.md: its entry {ID} was forgotten; bilbo remember /home/me/notes/shed.md keeps it again"
                ),
                format!(
                    "bilbo: passed over /home/me/notes/x$(id)'s.md: its entry {ID} was forgotten; bilbo remember '/home/me/notes/x$(id)'\\''s.md' keeps it again"
                ),
                format!("bilbo: passed over /home/me/notes/x\\ny.md: its entry {ID} was forgotten"),
                format!(
                    "bilbo: passed over /home/me/notes/x\u{202e}y.md: its entry {ID} was forgotten"
                ),
            ]
        );
        for view in [&View::PLAIN, &View::HUMAN] {
            let said: Vec<String> = messages
                .iter()
                .flat_map(|message| message.lines(view))
                .collect();
            assert_eq!(said.len(), 4, "{said:?}");
            for (at, line) in said.iter().enumerate() {
                assert_eq!(line.contains("keeps it again"), at < 2, "{line}");
            }
        }
    }

    #[test]
    fn a_file_kept_again_says_which_entry_came_back() {
        let id = Id::parse(ID).expect("an id");
        let mut kept = item("/home/me/notes/shed.md", Outcome::Appended(id), false);
        kept.forgotten = Some(Met::Again { id, bytes: 64 });
        let mut refused = item(
            "/home/me/notes/garden.md",
            Outcome::Refused("it is too large to keep: a frame holds at most 4 GiB".to_owned()),
            false,
        );
        refused.forgotten = Some(Met::Again { id, bytes: 64 });
        let mut over = item("/home/me/notes/b.md", Outcome::Unchanged(id), false);
        over.forgotten = Some(Met::Own { id, bytes: 0 });
        let output = output(vec![
            item("/home/me/notes/a.md", Outcome::Unchanged(id), false),
            kept,
            refused,
            over,
        ]);
        let messages = again(&[output]);
        assert!(messages.iter().all(|message| message.mark == Mark::Info));
        assert_eq!(
            lines(&messages),
            [
                format!("bilbo: kept /home/me/notes/shed.md again: its entry {ID} was forgotten"),
                format!(
                    "bilbo: took /home/me/notes/garden.md back as entry {ID}, which was forgotten; its file was refused"
                ),
            ]
        );
    }
}
