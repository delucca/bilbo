use std::fmt;

use xxhash_rust::xxh3;

use crate::entries::{Head, Mark};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Unchanged,
    New {
        with_meta: bool,
    },
    NewGeneration,
    /// The records `from..` follow the prefix that ends at `prior_end`.
    Appended {
        from: usize,
        prior_end: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub action: Action,
    /// The mark of the whole input.
    pub mark: Mark,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BadEnds {
    NotRising,
    NotCovering,
}

impl fmt::Display for BadEnds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRising => f.write_str("record ends must rise"),
            Self::NotCovering => f.write_str("the last record must end at the last byte"),
        }
    }
}

pub fn validate(bytes: &[u8], ends: &[usize]) -> Result<(), BadEnds> {
    let mut previous = 0;
    for &end in ends {
        if end <= previous {
            return Err(BadEnds::NotRising);
        }
        previous = end;
    }
    if previous == bytes.len() {
        Ok(())
    } else {
        Err(BadEnds::NotCovering)
    }
}

pub fn decide(head: Option<&Head>, bytes: &[u8], ends: &[usize]) -> Decision {
    let whole = Mark {
        end: bytes.len() as u64,
        hash: xxh3::xxh3_128(bytes),
    };
    let unchanged = |action| Decision {
        action,
        mark: whole,
    };
    let Some(head) = head else {
        return unchanged(Action::New { with_meta: true });
    };
    if head.generation.0 == 0 {
        return unchanged(Action::New { with_meta: false });
    }
    let Some(end) = usize::try_from(head.mark.end)
        .ok()
        .filter(|&end| end <= bytes.len())
    else {
        return unchanged(Action::NewGeneration);
    };
    let boundary = if end == 0 {
        Some(0)
    } else {
        ends.iter().position(|&e| e == end).map(|i| i + 1)
    };
    let prefix_holds = xxh3::xxh3_128(&bytes[..end]) == head.mark.hash;
    match boundary {
        Some(k) if prefix_holds && k as u64 == head.records => {
            if end == bytes.len() {
                unchanged(Action::Unchanged)
            } else {
                unchanged(Action::Appended {
                    from: k,
                    prior_end: end,
                })
            }
        }
        _ => unchanged(Action::NewGeneration),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entries::Generation;
    use crate::id::Id;

    fn head(generation: u32, records: u64, prefix: &[u8]) -> Head {
        Head {
            id: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id"),
            generation: Generation(generation),
            records,
            mark: Mark {
                end: prefix.len() as u64,
                hash: xxh3::xxh3_128(prefix),
            },
        }
    }

    fn ends_of(bytes: &[u8]) -> Vec<usize> {
        let mut ends: Vec<usize> = bytes
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
            .map(|(i, _)| i + 1)
            .collect();
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            ends.push(bytes.len());
        }
        ends
    }

    fn action(head: Option<&Head>, bytes: &[u8]) -> Action {
        decide(head, bytes, &ends_of(bytes)).action
    }

    #[test]
    fn capture_decides_by_the_whole_earlier_prefix() {
        let h = head(1, 2, b"a\nb\n");
        let cases: Vec<(&str, Option<Head>, &[u8], Action)> = vec![
            ("no head", None, b"a\n", Action::New { with_meta: true }),
            (
                "generation 0",
                Some(head(0, 0, b"")),
                b"a\n",
                Action::New { with_meta: false },
            ),
            ("same bytes", Some(h), b"a\nb\n", Action::Unchanged),
            (
                "grown at a record end",
                Some(h),
                b"a\nb\nc\nd\n",
                Action::Appended {
                    from: 2,
                    prior_end: 4,
                },
            ),
            (
                "changed inside the prefix",
                Some(h),
                b"a\nX\nc\n",
                Action::NewGeneration,
            ),
            ("shorter", Some(h), b"a\n", Action::NewGeneration),
            (
                "a last line without a newline, grown",
                Some(head(1, 2, b"a\nb")),
                b"a\nbc\n",
                Action::NewGeneration,
            ),
            (
                "same prefix bytes, another record count",
                Some(head(1, 3, b"a\nb\n")),
                b"a\nb\nc\n",
                Action::NewGeneration,
            ),
        ];
        for (name, head, bytes, expected) in cases {
            assert_eq!(action(head.as_ref(), bytes), expected, "{name}");
        }
        let grown = decide(Some(&h), b"a\nb\nc\n", &[2, 4, 6]);
        assert_eq!(
            grown.mark,
            Mark {
                end: 6,
                hash: xxh3::xxh3_128(b"a\nb\nc\n")
            }
        );
    }

    #[test]
    fn offset_0_is_a_record_boundary() {
        let empty = head(1, 0, b"");
        let full = head(1, 1, b"x\n");
        let cases: Vec<(&str, Head, &[u8], Action)> = vec![
            ("empty and empty", empty, b"", Action::Unchanged),
            (
                "empty generation, then x",
                empty,
                b"x\n",
                Action::Appended {
                    from: 0,
                    prior_end: 0,
                },
            ),
            ("records, then empty", full, b"", Action::NewGeneration),
            (
                "emptied generation, empty again",
                head(2, 0, b""),
                b"",
                Action::Unchanged,
            ),
        ];
        for (name, head, bytes, expected) in cases {
            assert_eq!(action(Some(&head), bytes), expected, "{name}");
        }
    }

    #[test]
    fn record_ends_must_rise_and_end_at_the_last_byte() {
        assert_eq!(validate(b"", &[]), Ok(()));
        assert_eq!(validate(b"a\nb", &[2, 3]), Ok(()));
        assert_eq!(validate(b"a\nb", &[3, 2]), Err(BadEnds::NotRising));
        assert_eq!(validate(b"a\nb", &[2, 2, 3]), Err(BadEnds::NotRising));
        assert_eq!(validate(b"a\nb", &[0, 3]), Err(BadEnds::NotRising));
        assert_eq!(validate(b"a\nb", &[2]), Err(BadEnds::NotCovering));
        assert_eq!(validate(b"a\nb", &[]), Err(BadEnds::NotCovering));
        assert_eq!(validate(b"", &[1]), Err(BadEnds::NotCovering));
        assert_eq!(validate(b"ab", &[2, 4]), Err(BadEnds::NotCovering));
    }

    #[test]
    fn bad_record_ends_say_what_is_wrong() {
        assert_eq!(BadEnds::NotRising.to_string(), "record ends must rise");
        assert_eq!(
            BadEnds::NotCovering.to_string(),
            "the last record must end at the last byte"
        );
    }
}
