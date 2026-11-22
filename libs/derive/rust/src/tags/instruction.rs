const TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/tags.dat"));

#[must_use]
pub fn text() -> &'static str {
    TEXT
}

#[must_use]
pub fn id() -> String {
    crate::instruction::id(TEXT)
}

#[cfg(test)]
mod tests {
    use super::{id, text};

    #[test]
    fn the_shipped_tags_instruction_is_the_one_that_was_reviewed() {
        assert_eq!(id(), "a5d31572b3105b1b");
        assert!(text().ends_with("line (none) and nothing else.\n"));
    }

    #[test]
    fn the_tags_instruction_is_no_instruction_of_a_kind() {
        let kinds: Vec<&str> = crate::instruction::shipped()
            .iter()
            .map(|(kind, _)| *kind)
            .collect();
        assert_eq!(kinds, ["session"]);
    }
}
