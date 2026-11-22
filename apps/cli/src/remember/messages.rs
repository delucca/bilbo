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

pub fn tagged(output: &Output, fresh: &[Tag]) -> Vec<Message> {
    if output.tagged == 0 {
        return Vec::new();
    }
    std::iter::once(Message::info(&format!("tagged {} entries", output.tagged)))
        .chain(new_tags(fresh))
        .collect()
}
