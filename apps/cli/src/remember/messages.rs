use bilbo_kinds::remember::{self, Output};

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
