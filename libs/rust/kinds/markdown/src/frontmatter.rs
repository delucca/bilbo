use bilbo_store::id::Id;

const FENCE: &str = "---";

/// The line that closes a frontmatter block, counted from 0: `lines` starts with `---` and holds
/// a later `---`. The lines come without their line ending, the first without a byte order mark.
pub fn close<'l>(mut lines: impl Iterator<Item = &'l str>) -> Option<usize> {
    if lines.next()? != FENCE {
        return None;
    }
    lines.position(|line| line == FENCE).map(|at| at + 1)
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

pub fn id(text: &str) -> Option<Id> {
    let close = close(lines(text))?;
    let mut found: Option<Option<Id>> = None;
    for line in lines(text).take(close).skip(1) {
        if found.is_some() || line.starts_with([' ', '-']) {
            continue;
        }
        if let Some(("id", value)) = line.split_once(':') {
            found = Some(value.strip_prefix(' ').and_then(Id::parse));
        }
    }
    found?
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
    fn a_block_closes_at_the_first_later_fence() {
        let close = |text: &str| super::close(text.lines());
        assert_eq!(close("---\nid: x\n---\nbody\n---\n"), Some(2));
        assert_eq!(close("---\n---\n"), Some(1));
        assert_eq!(close("---\nid: x\n"), None);
        assert_eq!(close("# Title\n---\n"), None);
        assert_eq!(close("--- \nid: x\n---\n"), None);
        assert_eq!(close(""), None);
    }

    #[test]
    fn a_bom_and_crlf_lines_do_not_hide_the_id() {
        assert_eq!(id(include_str!("../fixtures/bom.md")), valid());
        assert_eq!(id(include_str!("../fixtures/crlf.md")), valid());
    }
}
