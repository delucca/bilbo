#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub first: u64,
    pub last: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blocks {
    pub blocks: Vec<Block>,
    pub stray: usize,
}

fn number(text: &str) -> Option<(Option<u64>, &str)> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    Some((text[..digits].parse().ok(), &text[digits..]))
}

pub(crate) fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(at, byte)| {
            matches!(at, 4 | 7) == (*byte == b'-') && (*byte == b'-' || byte.is_ascii_digit())
        })
}

enum Line {
    Range(u64, u64),
    Starts,
    Text,
}

fn bounds(rest: &str) -> Option<(Option<u64>, Option<u64>, &str)> {
    let (first, after) = number(rest.strip_prefix('r')?)?;
    let next = after.trim_start();
    for dash in ['-', '\u{2013}', '\u{2014}'] {
        if let Some(second) = next.strip_prefix(dash) {
            let second = second.trim_start();
            if let Some((last, tail)) = number(second.strip_prefix('r').unwrap_or(second)) {
                return Some((first, last, tail));
            }
        }
    }
    Some((first, first, after))
}

fn classify(line: &str) -> Line {
    let mut rest = line
        .trim()
        .trim_start_matches(['-', '*', '\u{2022}'])
        .trim_start();
    if let Some(after) = rest.strip_prefix('#') {
        rest = after;
    }
    let Some((first, last, tail)) = bounds(rest) else {
        return Line::Text;
    };
    if tail.starts_with(|c: char| c.is_alphanumeric()) {
        return Line::Text;
    }
    let tail = tail.trim().trim_end_matches(['*', ':']).trim();
    if tail.is_empty() || is_day(tail) {
        match first.zip(last) {
            Some((first, last)) => Line::Range(first, last),
            None => Line::Starts,
        }
    } else {
        Line::Starts
    }
}

#[must_use]
pub fn blocks(answer: &str) -> Blocks {
    let mut found = Blocks::default();
    let mut open: Option<(u64, u64, Vec<&str>)> = None;
    for line in answer.lines() {
        if line.starts_with("```") {
            continue;
        }
        match classify(line) {
            Line::Range(first, last) => {
                close(open.take(), &mut found);
                open = Some((first, last, Vec::new()));
            }
            Line::Starts => {
                close(open.take(), &mut found);
                found.stray += 1;
            }
            Line::Text => {
                if let Some((_, _, lines)) = open.as_mut() {
                    lines.push(line);
                } else if !line.trim().is_empty() {
                    found.stray += 1;
                }
            }
        }
    }
    close(open, &mut found);
    found
}

fn close(open: Option<(u64, u64, Vec<&str>)>, found: &mut Blocks) {
    let Some((first, last, lines)) = open else {
        return;
    };
    let blank = |line: &&str| line.trim().is_empty();
    let from = lines.iter().position(|line| !blank(line));
    let to = lines.iter().rposition(|line| !blank(line));
    match from.zip(to) {
        Some((from, to)) => found.blocks.push(Block {
            first,
            last,
            text: lines[from..=to].join("\n"),
        }),
        None => found.stray += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, blocks};

    fn block(first: u64, last: u64, text: &str) -> Block {
        Block {
            first,
            last,
            text: text.to_owned(),
        }
    }

    #[test]
    fn an_answer_is_blocks_each_a_range_line_and_a_paragraph() {
        let found = blocks("r1-3\nThe user asked.\nThen more.\n\nr4\nThe assistant did it.\n");
        assert_eq!(
            found.blocks,
            [
                block(1, 3, "The user asked.\nThen more."),
                block(4, 4, "The assistant did it.")
            ]
        );
        assert_eq!(found.stray, 0);
    }

    #[test]
    fn a_range_is_read_with_or_without_the_second_r() {
        for (line, range) in [
            ("r3-7", (3, 7)),
            ("r3-r7", (3, 7)),
            ("r3 - 7", (3, 7)),
            ("#r3-7", (3, 7)),
            ("- r3-7", (3, 7)),
            ("* r3", (3, 3)),
            ("• r3", (3, 3)),
            ("r3", (3, 3)),
            ("  r12-14  ", (12, 14)),
        ] {
            let found = blocks(&format!("{line}\ntext"));
            assert_eq!(found.blocks, [block(range.0, range.1, "text")], "{line}");
            assert_eq!(found.stray, 0, "{line}");
        }
    }

    #[test]
    fn a_day_after_a_range_is_ignored() {
        assert_eq!(
            blocks("r3-7 2026-11-07\ntext").blocks,
            [block(3, 7, "text")]
        );
        assert_eq!(blocks("r3 2026-11-07\ntext").blocks, [block(3, 3, "text")]);
        let soon = blocks("r3-7 soon\ntext");
        assert!(soon.blocks.is_empty());
        assert_eq!(soon.stray, 2);
    }

    #[test]
    fn a_range_line_may_be_emphasised_use_a_long_dash_or_end_in_a_colon() {
        for (line, range) in [
            ("**r12-14**", (12, 14)),
            ("*r12*", (12, 12)),
            ("r12\u{2013}14", (12, 14)),
            ("r12\u{2014}14", (12, 14)),
            ("r12-14:", (12, 14)),
            ("**r12\u{2013}14:**", (12, 14)),
            ("**r3-19 2026-11-07**", (3, 19)),
        ] {
            let found = blocks(&format!("{line}\ntext"));
            assert_eq!(found.blocks, [block(range.0, range.1, "text")], "{line}");
            assert_eq!(found.stray, 0, "{line}");
        }
    }

    #[test]
    fn a_line_that_starts_as_a_range_and_goes_on_closes_the_block_above() {
        for line in [
            "r2: The user asked about beta.",
            "r3, r17",
            "r12-14 and more",
        ] {
            let found = blocks(&format!(
                "r1\nThe user asked about alpha.\n\n{line}\nMore about beta."
            ));
            assert_eq!(
                found.blocks,
                [block(1, 1, "The user asked about alpha.")],
                "{line}"
            );
            assert_eq!(found.stray, 2, "{line}");
        }
        let found = blocks("r1\nThe user asked about alpha.\n\nr2d2 was named.\nMore.");
        assert_eq!(
            found.blocks,
            [block(
                1,
                1,
                "The user asked about alpha.\n\nr2d2 was named.\nMore."
            )]
        );
        assert_eq!(found.stray, 0);
    }

    #[test]
    fn a_line_that_is_only_a_code_fence_is_skipped() {
        let found = blocks("```\nr1\ntext\n```");
        assert_eq!(found.blocks, [block(1, 1, "text")]);
        assert_eq!(found.stray, 0);
    }

    #[test]
    fn a_paragraph_keeps_its_inner_line_feeds_and_loses_the_blank_lines_at_its_ends() {
        let found = blocks("r1\n\n\nfirst\n\nsecond\n\n\n");
        assert_eq!(found.blocks, [block(1, 1, "first\n\nsecond")]);
    }

    #[test]
    fn text_before_the_first_range_and_a_block_with_no_text_are_stray() {
        let found = blocks("Here are the notes:\n\nsome more\nr1\ntext\nr2\n\nr3\nlast");
        assert_eq!(found.blocks, [block(1, 1, "text"), block(3, 3, "last")]);
        assert_eq!(found.stray, 3);
    }

    #[test]
    fn an_answer_of_white_space_alone_has_no_block_and_nothing_stray() {
        for answer in ["", "\n", "  \n\t\n\n"] {
            let found = blocks(answer);
            assert!(found.blocks.is_empty(), "{answer:?}");
            assert_eq!(found.stray, 0, "{answer:?}");
        }
    }

    #[test]
    fn a_number_that_fits_no_record_is_no_range() {
        let found = blocks("r99999999999999999999\ntext");
        assert!(found.blocks.is_empty());
        assert_eq!(found.stray, 2);
        let second = blocks("r1-99999999999999999999\ntext");
        assert!(second.blocks.is_empty());
    }

    #[test]
    fn a_range_line_whose_number_does_not_fit_closes_its_block_and_is_stray() {
        let found = blocks("r1-3\nfirst paragraph\nr99999999999999999999\nnot a note");
        assert_eq!(found.blocks, [block(1, 3, "first paragraph")]);
        assert_eq!(found.stray, 2);
        let found = blocks("r1-3\nfirst paragraph\nr2d2 is a droid");
        assert_eq!(
            found.blocks,
            [block(1, 3, "first paragraph\nr2d2 is a droid")]
        );
        assert_eq!(found.stray, 0);
    }

    #[test]
    fn a_carriage_return_before_a_line_feed_ends_a_line() {
        let found = blocks("r1-2\r\nfirst\r\nsecond\r\n");
        assert_eq!(found.blocks, [block(1, 2, "first\nsecond")]);
    }
}
