use std::io;

use crate::recall::log::escaped;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claimed {
    Seen,
    Marked,
}

pub trait Sessions {
    fn claim(&mut self, thread: &str) -> io::Result<Claimed>;
}

pub(crate) fn field(thread: &str) -> String {
    if thread == "-" {
        "\\-".to_owned()
    } else {
        escaped(thread)
    }
}

pub fn mark(seconds: u64, thread: &str) -> String {
    format!("s1\t{seconds}\t{}\n", field(thread))
}

pub fn whole(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1)
}

pub fn holds(bytes: &[u8], thread: &str) -> bool {
    let wanted = field(thread);
    bytes[..whole(bytes)]
        .split(|byte| *byte == b'\n')
        .any(|line| {
            let mut parts = line.split(|byte| *byte == b'\t');
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(b"s1"), Some(time), Some(thread), None) => {
                    !time.is_empty()
                        && time.iter().all(u8::is_ascii_digit)
                        && !thread.is_empty()
                        && thread == wanted.as_bytes()
                }
                _ => false,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{field, holds, mark, whole};

    #[test]
    fn a_mark_is_found_by_its_whole_session_and_no_other() {
        let mut bytes = Vec::new();
        for (at, thread) in [(1, "abc-def"), (2, "Abc-Def"), (3, "tab\there\nline")] {
            bytes.extend_from_slice(mark(at, thread).as_bytes());
        }
        for thread in ["abc-def", "Abc-Def", "tab\there\nline"] {
            assert!(holds(&bytes, thread), "{thread:?}");
        }
        for thread in [
            "abc",
            "def",
            "bc-def",
            "ABC-DEF",
            "abc-defg",
            "tab",
            "line",
            "tab\there",
        ] {
            assert!(!holds(&bytes, thread), "{thread:?}");
        }
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 3);
        assert_eq!(mark(3, "tab\there\nline"), "s1\t3\ttab\\there\\nline\n");

        assert_eq!(field("-"), "\\-");
        assert_eq!(mark(7, "-"), "s1\t7\t\\-\n");
        let dash = mark(7, "-").into_bytes();
        assert!(holds(&dash, "-"));
        assert!(!holds(&dash, "\\-"));
        assert!(!holds(&mark(7, "\\-").into_bytes(), "-"));
    }

    #[test]
    fn only_a_whole_mark_marks_a_session() {
        let around = |middle: &[u8]| {
            let mut bytes = mark(1, "before").into_bytes();
            bytes.extend_from_slice(middle);
            bytes.extend_from_slice(mark(2, "after").as_bytes());
            bytes
        };
        let none: [(&[u8], &str); 9] = [
            (b"s1\tlost\n", "lost"),
            (b"s1\t5\tlost\tmore\n", "lost"),
            (b"s1\tsoon\tlost\n", "lost"),
            (b"s2\t5\tlost\n", "lost"),
            (b"s1\t5\tl\xffst\n", "lost"),
            (b"s1\t5\tl\xffst\n", "l\u{fffd}st"),
            (b"s1\t1\t\n", ""),
            (b"s1\t+5\tlost\n", "lost"),
            (b"s1\t\tlost\n", "lost"),
        ];
        for (line, thread) in none {
            let bytes = around(line);
            assert!(holds(&bytes, "before") && holds(&bytes, "after"));
            assert!(!holds(&bytes, thread), "{line:?} {thread:?}");
        }
        let long = around(b"s1\t99999999999999999999999999999999\tlong\n");
        assert!(holds(&long, "long"));
        let mut tail = mark(1, "before").into_bytes();
        tail.extend_from_slice(b"s1\t5\tlost");
        assert!(!holds(&tail, "lost"));
        assert!(holds(&tail, "before"));
        tail.extend_from_slice(b"\n");
        assert!(holds(&tail, "lost"));
        assert!(!holds(b"s1\t5\t", "lost"));
        assert!(!holds(b"", "lost"));
    }

    #[test]
    fn the_whole_lines_end_where_the_tail_begins() {
        assert_eq!(whole(b"one\ntwo\n"), 8);
        assert_eq!(whole(b"one\ntwo"), 4);
        assert_eq!(whole(b"one"), 0);
        assert_eq!(whole(b""), 0);
        assert_eq!(whole(b"\n"), 1);
    }
}
