use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use crate::answer::Block;
use crate::tries::Extent;
use crate::view::Paragraph;
use crate::window::{Sent, Window};

const EDGE: &[char] = &[
    '`', '\'', '"', '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', ':', '!', '?', '*',
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    pub kept: Vec<Paragraph>,
    pub range: u64,
    pub names: u64,
}

fn clean(word: &str) -> &str {
    word.trim_matches(EDGE)
        .trim_end_matches('.')
        .trim_matches(EDGE)
}

fn looks_like_name(word: &str) -> bool {
    let mut chars = word.chars();
    let first = chars.next();
    let later_upper = chars.any(char::is_uppercase);
    let letters: Vec<char> = word.chars().collect();
    let inner_dot = letters.len() > 2 && letters[1..letters.len() - 1].contains(&'.');
    let dotted = inner_dot
        && word.split('.').filter(|part| !part.is_empty()).count() >= 2
        && word.split('.').any(|part| part.chars().count() > 1);
    word.chars().any(|c| c.is_ascii_digit())
        || word.contains(['/', '_'])
        || word.contains("::")
        || (first == Some('-') && word.chars().any(char::is_alphabetic))
        || (later_upper && word.chars().any(char::is_lowercase))
        || dotted
}

fn names(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(clean)
        .filter(|word| looks_like_name(word))
        .collect()
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn stands_whole(name: &str, material: &str) -> bool {
    material.match_indices(name).any(|(at, _)| {
        let before = material[..at].chars().next_back();
        let after = material[at + name.len()..].chars().next();
        !before.is_some_and(word_char) && !after.is_some_and(word_char)
    })
}

fn is_locator_line(line: &str) -> bool {
    let (place, day) = line.split_once(' ').unwrap_or((line, ""));
    let numbers = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let range = place
        .strip_prefix('r')
        .is_some_and(|range| range.split('-').count() <= 2 && range.split('-').all(numbers));
    range && (day.is_empty() || crate::answer::is_day(day))
}

fn searchable(material: &str) -> String {
    let mut out = String::with_capacity(material.len());
    for line in material.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if let Some(rest) = bare.strip_prefix("#r")
            && let Some(space) = rest.find(' ')
            && rest[..space]
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-')
            && !rest[..space].is_empty()
        {
            out.push_str(&rest[space + 1..]);
            out.push('\n');
        } else if is_locator_line(bare) {
            out.push('\n');
        } else {
            out.push_str(line);
        }
    }
    out
}

fn found(name: &str, material: &str) -> bool {
    if stands_whole(name, material) {
        return true;
    }
    let parts: Vec<&str> = name
        .split("->")
        .flat_map(|piece| piece.split(".."))
        .flat_map(|piece| piece.split(['/', '|', ',']))
        .map(clean)
        .filter(|part| looks_like_name(part))
        .collect();
    !parts.is_empty() && parts.iter().all(|part| stands_whole(part, material))
}

fn dropped_for_names(text: &str, material: &str, share: f64) -> bool {
    if share <= 0.0 {
        return false;
    }
    let names = names(text);
    if names.is_empty() {
        return false;
    }
    let material = searchable(material);
    let held = names.iter().filter(|name| found(name, &material)).count();
    (held as f64 / names.len() as f64) < share
}

#[must_use]
pub fn check(
    blocks: &[Block],
    window: &Window,
    passages: &[Sent],
    material: &str,
    share: f64,
    parent: Id,
    generation: Generation,
) -> Checked {
    let mut checked = Checked::default();
    let meets = |sent: &Sent, block: &Block| {
        sent.range.first <= block.last && sent.range.first + sent.range.count > block.first
    };
    for block in blocks {
        if block.first < 1
            || block.first > block.last
            || block.last > window.last
            || !window.passages.iter().any(|sent| meets(sent, block))
        {
            checked.range += 1;
        } else if dropped_for_names(&block.text, material, share) {
            checked.names += 1;
        } else {
            let at = passages
                .iter()
                .filter(|sent| meets(sent, block))
                .find_map(|sent| sent.at);
            checked.kept.push(Paragraph {
                text: block.text.clone(),
                at,
                source: Extent {
                    parent,
                    generation,
                    first: block.first,
                    last: block.last,
                },
            });
        }
    }
    checked
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{check, found, looks_like_name, names, searchable};
    use crate::answer::Block;
    use crate::tries::Extent;
    use crate::view::{Paragraph, material};
    use crate::window::{Sent, Window};

    fn parent() -> Id {
        Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DB").expect("id")
    }

    fn window(first: u64, last: u64) -> Window {
        Window {
            first,
            last,
            text: String::new(),
            sent: 0,
            passages: passages(&[(first, last - first + 1, None)]),
        }
    }

    fn passages(list: &[(u64, u64, Option<u64>)]) -> Vec<Sent> {
        list.iter()
            .map(|&(first, count, at)| Sent {
                range: RecordRange { first, count },
                at: at.map(at_second),
            })
            .collect()
    }

    fn at_second(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn block(first: u64, last: u64, text: &str) -> Block {
        Block {
            first,
            last,
            text: text.to_owned(),
        }
    }

    fn run(blocks: &[Block], material: &str, share: f64) -> (usize, u64, u64) {
        let checked = check(
            blocks,
            &window(5, 10),
            &[],
            material,
            share,
            parent(),
            Generation(1),
        );
        (checked.kept.len(), checked.range, checked.names)
    }

    #[test]
    fn a_range_inside_the_entry_up_to_the_windows_last_record_is_kept() {
        let blocks = [block(5, 10, "a"), block(7, 7, "b"), block(3, 8, "c")];
        assert_eq!(run(&blocks, "", 0.5), (3, 0, 0));
    }

    #[test]
    fn a_range_backwards_from_0_or_past_the_window_is_dropped_as_range() {
        let blocks = [
            block(0, 3, "a"),
            block(6, 5, "b"),
            block(8, 11, "c"),
            block(10, 10, "d"),
        ];
        assert_eq!(run(&blocks, "", 0.5), (1, 3, 0));
    }

    #[test]
    fn a_dotted_name_is_found_without_cutting_a_character() {
        assert_eq!(names("see é.rs now"), ["é.rs"]);
        assert!(names("a.é").is_empty());
        assert_eq!(names("é.é.rs"), ["é.é.rs"]);
    }

    #[test]
    fn a_word_is_a_name_by_five_marks_and_a_compound_of_prose_is_none() {
        for word in [
            "v2",
            "3.11",
            "r12",
            "src/a.rs",
            "chunk_tokens",
            "plan::held",
            "--dry-run",
            "-v",
            "HashMap",
            "eBPF",
            "main.rs",
            "example.com",
            "a.rs",
        ] {
            assert!(looks_like_name(word), "{word}");
        }
        for word in [
            "follow-up",
            "read-only",
            "built-in",
            "no-op",
            "bilbo-derive",
            "e.g",
            "i.e",
            "API",
            "README",
            "user",
            "",
            "-",
        ] {
            assert!(!looks_like_name(word), "{word}");
        }
        let plain = run(&[block(5, 5, "(word) end. Seal - the follow-up")], "", 1.0);
        assert_eq!(plain, (1, 0, 0));
    }

    #[test]
    fn a_name_is_found_whole_and_never_in_a_locator() {
        let earlier = [Paragraph {
            text: "The assistant counted.".to_owned(),
            at: Some(at_second(1_794_039_249)),
            source: Extent {
                parent: parent(),
                generation: Generation(1),
                first: 3,
                last: 19,
            },
        }];
        let shown = searchable(&material(&earlier, None, "#r142 the count is high\n"));
        for name in ["142", "19", "2026"] {
            assert!(!found(name, &shown), "{name}");
        }
        assert!(found("count", &shown));
        assert!(!found(
            "config/settings",
            &searchable("see config and settings")
        ));
        let path = searchable("see src/main.rs");
        assert!(found("src/main.rs", &path));
        assert!(found("main.rs", &path));
        assert!(!found("ain.rs", &path));
        assert!(found("src/main.rs", &searchable("main.rs")));
    }

    #[test]
    fn a_name_is_found_whole_or_by_its_parts() {
        assert!(found("src/a.rs", "see src/a.rs here"));
        assert!(!found("a/b_c", "a and b"));
        assert!(found("x1,y2", "x1 and y2"));
        assert!(!found("x1,y2", "x1 only"));
        assert!(found("a->b_c", "b_c"));
        assert!(found("one..two2", "two2"));
        assert!(found("p|q_r", "q_r"));
    }

    #[test]
    fn a_note_is_dropped_when_fewer_than_the_share_of_its_names_are_found() {
        let material = "src/a.rs";
        let two = [block(5, 5, "It used src/a.rs and src/b.rs.")];
        let three = [block(5, 5, "It used src/a.rs and src/b.rs and src/c.rs.")];
        assert_eq!(run(&two, material, 0.5), (1, 0, 0));
        assert_eq!(run(&three, material, 0.5), (0, 0, 1));
        assert_eq!(run(&two, material, 0.51), (0, 0, 1));
    }

    #[test]
    fn a_share_of_0_drops_none_and_of_1_drops_for_one_name_not_found() {
        let blocks = [block(5, 5, "It used src/a.rs and src/b.rs.")];
        assert_eq!(run(&blocks, "src/a.rs", 0.0), (1, 0, 0));
        assert_eq!(run(&blocks, "src/a.rs", 1.0), (0, 0, 1));
        assert_eq!(run(&blocks, "src/a.rs src/b.rs", 1.0), (1, 0, 0));
    }

    #[test]
    fn a_note_with_no_name_is_kept_at_every_share() {
        let blocks = [block(5, 5, "The user asked for a change.")];
        for share in [0.0, 0.5, 1.0] {
            assert_eq!(run(&blocks, "", share), (1, 0, 0), "{share}");
        }
    }

    #[test]
    fn names_are_looked_for_in_all_the_call_was_shown() {
        let earlier = [Paragraph {
            text: "The assistant edited src/only_earlier.rs.".to_owned(),
            at: None,
            source: Extent {
                parent: parent(),
                generation: Generation(1),
                first: 1,
                last: 2,
            },
        }];
        let shown = material(&earlier, None, "#r5 hello\n");
        let blocks = [block(5, 5, "It went back to src/only_earlier.rs.")];
        assert_eq!(run(&blocks, &shown, 1.0), (1, 0, 0));
        assert_eq!(run(&blocks, "#r5 hello\n", 1.0), (0, 0, 1));
    }

    #[test]
    fn a_block_that_meets_no_passage_the_window_sent_is_dropped() {
        let mut sent = window(5, 10);
        sent.passages = passages(&[(5, 1, None), (6, 2, None)]);
        let blocks = [
            block(1, 2, "a"),
            block(3, 4, "b"),
            block(9, 10, "c"),
            block(1, 6, "d"),
            block(5, 5, "e"),
            block(7, 9, "f"),
        ];
        let checked = check(&blocks, &sent, &[], "", 0.5, parent(), Generation(1));
        assert_eq!(checked.range, 3);
        let kept: Vec<&str> = checked.kept.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(kept, ["d", "e", "f"]);
    }

    #[test]
    fn a_kept_block_takes_the_time_of_the_first_dated_passage_in_its_range() {
        let entry = passages(&[
            (1, 1, Some(50)),
            (2, 3, None),
            (5, 1, Some(70)),
            (6, 5, None),
        ]);
        let ranges = [(1, 6, Some(50)), (5, 6, Some(70)), (6, 7, None)];
        let dropped = [block(1, 2, "x"), block(3, 4, "y")];
        let blocks: Vec<Block> = dropped
            .iter()
            .cloned()
            .chain(
                ranges
                    .iter()
                    .map(|&(first, last, _)| block(first, last, "a")),
            )
            .collect();
        let checked = check(
            &blocks,
            &window(5, 10),
            &entry,
            "",
            0.5,
            parent(),
            Generation(2),
        );
        assert_eq!(checked.range, 2);
        let expected: Vec<Paragraph> = ranges
            .iter()
            .map(|&(first, last, at)| Paragraph {
                text: "a".to_owned(),
                at: at.map(at_second),
                source: Extent {
                    parent: parent(),
                    generation: Generation(2),
                    first,
                    last,
                },
            })
            .collect();
        assert_eq!(checked.kept, expected);
    }
}
