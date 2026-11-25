use std::path::Path;

pub const ID_A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
pub const ID_B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DF";

pub fn note_with(id: &str, heading: &str) -> String {
    format!("---\nid: {id}\n---\n# {heading}\n")
}

pub fn moved_line(new: &Path, id: &str, old: &Path) -> String {
    format!(
        "bilbo: moved {}: its id {id} was held by file:{}",
        text(new),
        text(old)
    )
}

pub fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
}

pub fn counts(new: u64, appended: u64, generation: u64, unchanged: u64, refused: u64) -> String {
    format!(
        "new {new}, appended {appended}, new generation {generation}, unchanged {unchanged}, refused {refused}"
    )
}
