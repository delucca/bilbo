use std::collections::BTreeSet;

use bilbo_store::labels::Tag;

pub const NONE: &str = "(none)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Both,
    Empty,
    Marks,
    Strays,
}

impl Why {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Why::Both => "both",
            Why::Empty => "empty",
            Why::Marks => "marks",
            Why::Strays => "strays",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Picks { picks: Vec<Tag>, dropped: u64 },
    NoneFits { dropped: u64 },
    NoTag(Why),
}

fn stripped(piece: &str) -> &str {
    let piece = piece.trim();
    let piece = ['-', '*', '•']
        .iter()
        .find_map(|bullet| piece.strip_prefix(*bullet))
        .map_or(piece, str::trim_start);
    let piece = ['`', '"', '\'']
        .iter()
        .find_map(|mark| {
            piece
                .strip_prefix(*mark)
                .and_then(|inner| inner.strip_suffix(*mark))
        })
        .unwrap_or(piece);
    piece.strip_prefix(Tag::SCHEME).unwrap_or(piece)
}

fn is_fence(piece: &str) -> bool {
    piece
        .trim()
        .strip_prefix("```")
        .is_some_and(|rest| !rest.contains(char::is_whitespace))
}

#[must_use]
pub fn read(text: &str, list: &BTreeSet<Tag>, most: usize) -> Read {
    let mut none = false;
    let mut picks: Vec<Tag> = Vec::new();
    let mut strays = 0u64;
    for line in text.lines() {
        if line.trim() == NONE {
            none = true;
            continue;
        }
        for piece in line.split(',') {
            if is_fence(piece) {
                continue;
            }
            let name = stripped(piece);
            if name.is_empty() {
                continue;
            }
            match list.iter().find(|tag| tag.as_str() == name) {
                Some(tag) if picks.contains(tag) => {}
                Some(tag) => picks.push(tag.clone()),
                None => strays += 1,
            }
        }
    }
    match (picks.is_empty(), none) {
        (false, true) => Read::NoTag(Why::Both),
        (false, false) => {
            let extra = u64::try_from(picks.len().saturating_sub(most)).unwrap_or(u64::MAX);
            picks.truncate(most);
            Read::Picks {
                picks,
                dropped: extra + strays,
            }
        }
        (true, true) => Read::NoneFits { dropped: strays },
        (true, false) if text.trim().is_empty() => Read::NoTag(Why::Empty),
        (true, false) if strays > 0 => Read::NoTag(Why::Strays),
        (true, false) => Read::NoTag(Why::Marks),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bilbo_store::labels::Tag;

    use super::{Read, Why, read};

    fn tag(name: &str) -> Tag {
        Tag::new(name).expect("a tag")
    }

    fn list(names: &[&str]) -> BTreeSet<Tag> {
        names.iter().map(|name| tag(name)).collect()
    }

    fn picks(names: &[&str], dropped: u64) -> Read {
        Read::Picks {
            picks: names.iter().map(|name| tag(name)).collect(),
            dropped,
        }
    }

    #[test]
    fn picks_are_read_one_a_piece_after_the_marks_are_stripped() {
        let list = list(&["bilbo", "garden"]);
        for (text, expected) in [
            ("- bilbo", picks(&["bilbo"], 0)),
            ("`garden`", picks(&["garden"], 0)),
            ("\"bilbo\"", picks(&["bilbo"], 0)),
            ("tag:garden", picks(&["garden"], 0)),
            ("bilbo, garden", picks(&["bilbo", "garden"], 0)),
            ("* garden", picks(&["garden"], 0)),
            ("garden\nbilbo\ngarden", picks(&["garden", "bilbo"], 0)),
        ] {
            assert_eq!(read(text, &list, 3), expected, "{text}");
        }
    }

    #[test]
    fn the_none_line_is_looked_for_before_any_mark_is_stripped() {
        let list_of_one = list(&["bilbo"]);
        for text in ["(none)", "  (none)  ", "\n(none)\n"] {
            assert_eq!(
                read(text, &list_of_one, 3),
                Read::NoneFits { dropped: 0 },
                "{text:?}"
            );
        }
        for text in ["`(none)`", "- (none)", "(None)", "none"] {
            assert_eq!(
                read(text, &list_of_one, 3),
                Read::NoTag(Why::Strays),
                "{text}"
            );
        }
        assert_eq!(read("none", &list(&["none"]), 3), picks(&["none"], 0));
    }

    #[test]
    fn every_answer_has_its_outcome() {
        let list = list(&["bilbo", "garden"]);
        for (text, expected) in [
            ("bilbo\nstray", picks(&["bilbo"], 1)),
            ("(none)\nstray", Read::NoneFits { dropped: 1 }),
            ("bilbo\n(none)", Read::NoTag(Why::Both)),
            ("", Read::NoTag(Why::Empty)),
            (" \n\t\n", Read::NoTag(Why::Empty)),
            ("```\n```\n- \n", Read::NoTag(Why::Marks)),
            ("stray\nother, thing", Read::NoTag(Why::Strays)),
            ("```two words\nbilbo", picks(&["bilbo"], 1)),
            ("```two words", Read::NoTag(Why::Strays)),
            ("```text", Read::NoTag(Why::Marks)),
        ] {
            assert_eq!(read(text, &list, 3), expected, "{text:?}");
        }
    }

    #[test]
    fn a_name_that_is_not_on_the_list_is_never_a_pick() {
        let list = list(&["bilbo"]);
        for text in [
            "Bilbo",
            "bilb",
            "bilbos",
            "bilb\u{43e}",
            "tag:tag:bilbo",
            "bilbo garden",
        ] {
            assert_eq!(read(text, &list, 3), Read::NoTag(Why::Strays), "{text}");
        }
    }

    #[test]
    fn picks_past_the_most_are_dropped_and_counted() {
        let list = list(&["a", "b"]);
        assert_eq!(read("a\nb\nstray", &list, 1), picks(&["a"], 2));
    }
}
