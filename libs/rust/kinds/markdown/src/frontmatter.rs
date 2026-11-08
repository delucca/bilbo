use bilbo_store::id::Id;

const FENCE: &str = "---";

pub fn id(text: &str) -> Option<Id> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    if lines.next()? != FENCE {
        return None;
    }
    let mut found: Option<Option<Id>> = None;
    for line in lines {
        if line == FENCE {
            return found?;
        }
        if found.is_some() || line.starts_with([' ', '-']) {
            continue;
        }
        if let Some(("id", value)) = line.split_once(':') {
            found = Some(value.strip_prefix(' ').and_then(Id::parse));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

    fn valid() -> Option<Id> {
        Id::parse(VALID)
    }

    #[test]
    fn a_frontmatter_id_that_is_a_canonical_ulid_is_proposed() {
        let note = include_str!("../fixtures/note.md");
        assert_eq!(id(note), valid());
        assert!(valid().is_some());
    }

    #[test]
    fn no_id_is_proposed_without_a_usable_one() {
        let cases = [
            ("no frontmatter", "# Title\n".to_string()),
            ("unclosed", format!("---\nid: {VALID}\n")),
            (
                "lowercase",
                "---\nid: 01jc5q3v7xkm4p8r2t6w9yb0de\n---\n".into(),
            ),
            (
                "25 chars",
                "---\nid: 01JC5Q3V7XKM4P8R2T6W9YB0D\n---\n".into(),
            ),
            ("no space", format!("---\nid:{VALID}\n---\n")),
            ("two spaces", format!("---\nid:  {VALID}\n---\n")),
            ("trailing space", format!("---\nid: {VALID} \n---\n")),
            (
                "only an item under sources",
                format!("---\nsources:\n  - id: {VALID}\n---\n"),
            ),
            (
                "a second id after an invalid first",
                format!("---\nid: nope\nid: {VALID}\n---\n"),
            ),
            (
                "after the block",
                format!("---\ntitle: x\n---\nid: {VALID}\n"),
            ),
            ("empty text", String::new()),
        ];
        for (name, text) in cases {
            assert_eq!(id(&text), None, "{name}");
        }
    }

    #[test]
    fn a_note_that_is_only_frontmatter_keeps_its_id() {
        assert_eq!(id(&format!("---\nid: {VALID}\n---")), valid());
        assert_eq!(id(&format!("---\r\nid: {VALID}\r\n---")), valid());
    }

    #[test]
    fn a_bom_and_crlf_lines_do_not_hide_the_id() {
        assert_eq!(id(include_str!("../fixtures/bom.md")), valid());
        assert_eq!(id(include_str!("../fixtures/crlf.md")), valid());
    }
}
