use crate::console::message::line;
use crate::console::view::View;

/// No escape sequence of a record passes.
pub fn body(view: &View, text: &str) -> String {
    if !view.terminal {
        return text.to_owned();
    }
    text.split('\n')
        .map(|text| {
            text.split('\t')
                .map(|piece| line(view, piece))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
