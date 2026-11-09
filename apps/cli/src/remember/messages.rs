use bilbo_kinds::remember::{self, Output};
use bilbo_store::segments::Notice;

use crate::console::message::{Message, sanitize};
use crate::console::theme::Mark;

pub fn refusals(outputs: &[Output]) -> Vec<Message> {
    outputs
        .iter()
        .flat_map(|output| &output.items)
        .filter_map(|item| match &item.outcome {
            remember::Outcome::Refused(reason) => Some(warning(&format!(
                "refused {}: {reason}",
                item.path.display()
            ))),
            _ => None,
        })
        .collect()
}

pub fn warning(text: &str) -> Message {
    message(Mark::Warning, text)
}

pub fn info(text: &str) -> Message {
    message(Mark::Info, text)
}

fn message(mark: Mark, text: &str) -> Message {
    Message {
        mark,
        text: sanitize(text),
        details: Vec::new(),
    }
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn notice_message(notice: &Notice) -> Message {
    match notice {
        Notice::Waiting => message(
            Mark::Info,
            "waiting for another bilbo to finish writing the store",
        ),
        Notice::CutTornTail {
            path,
            offset,
            bytes,
        } => message(
            Mark::Warning,
            &format!(
                "cut {bytes} bytes of a torn frame at byte {offset} of {}",
                path.display()
            ),
        ),
        Notice::RebuiltCatalog { entries, segments } => message(
            Mark::Info,
            &format!(
                "rebuilt the catalog from the store: {} in {}",
                plural(*entries, "entry", "entries"),
                plural(*segments, "segment", "segments")
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bilbo_store::segments::Notice;

    use super::notice_message;
    use crate::console::theme::Mark;

    #[test]
    fn every_store_notice_is_a_message() {
        let cases = [
            (
                Notice::Waiting,
                Mark::Info,
                "waiting for another bilbo to finish writing the store",
            ),
            (
                Notice::CutTornTail {
                    path: PathBuf::from("/s/open.seg"),
                    offset: 12,
                    bytes: 20,
                },
                Mark::Warning,
                "cut 20 bytes of a torn frame at byte 12 of /s/open.seg",
            ),
            (
                Notice::RebuiltCatalog {
                    entries: 3,
                    segments: 1,
                },
                Mark::Info,
                "rebuilt the catalog from the store: 3 entries in 1 segment",
            ),
            (
                Notice::RebuiltCatalog {
                    entries: 1,
                    segments: 2,
                },
                Mark::Info,
                "rebuilt the catalog from the store: 1 entry in 2 segments",
            ),
        ];
        for (notice, mark, text) in cases {
            let message = notice_message(&notice);
            assert_eq!(message.mark, mark);
            assert_eq!(message.text, text);
        }
    }
}
