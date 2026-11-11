use bilbo_search::work::Output;

use crate::console::message::{Detail, Message, group, sanitize};
use crate::console::theme::Mark;
use crate::console::view::View;

fn unread(output: &Output) -> u64 {
    output.unread.values().sum()
}

pub fn plain(output: &Output, milliseconds: u128) -> String {
    format!(
        "entries {}, chunks {}, unread {}, failed {}\nindex bytes {}, milliseconds {milliseconds}\n",
        output.entries,
        output.chunks,
        unread(output),
        output.failed.len(),
        output.index_bytes,
    )
}

/// One line for each kind no reader reads and for each entry the store could not give back.
pub fn left_out(output: &Output) -> Vec<Message> {
    let kinds = output.unread.iter().map(|(kind, entries)| Message {
        mark: Mark::Info,
        text: sanitize(&format!(
            "left out {entries} {} of kind {kind}: no reader reads it",
            if *entries == 1 { "entry" } else { "entries" }
        )),
        details: Vec::new(),
    });
    let failed = output.failed.iter().map(|(id, why)| Message {
        mark: Mark::Warning,
        text: sanitize(&format!("left out {id}: {why}")),
        details: Vec::new(),
    });
    kinds.chain(failed).collect()
}

pub fn human(output: &Output, milliseconds: u128, view: &View) -> String {
    let noun = if output.entries == 1 {
        "entry"
    } else {
        "entries"
    };
    let seconds = milliseconds / 1000;
    let tenths = milliseconds % 1000 / 100;
    let message = Message {
        mark: if output.failed.is_empty() {
            Mark::Done
        } else {
            Mark::Warning
        },
        text: format!("Indexed {} {noun}", group(output.entries)),
        details: vec![
            Detail::Text(format!(
                "{} chunks, {} unread, {} failed",
                group(output.chunks),
                group(unread(output)),
                group(output.failed.len() as u64),
            )),
            Detail::Text(format!(
                "{} index bytes in {seconds}.{tenths} s",
                group(output.index_bytes),
            )),
        ],
    };
    let mut text = message.human_lines(view).join("\n");
    text.push('\n');
    text
}
