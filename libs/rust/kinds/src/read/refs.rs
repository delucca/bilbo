use std::fmt;

use bilbo_store::entries::{Generation, RecordRange};
use bilbo_store::id::{Id, Short};

pub const SCHEME: &str = "bilbo:";
pub(crate) const SHORTEST: usize = 8;
pub(super) const UUID: usize = 36;
const DASHES: [usize; 4] = [8, 13, 18, 23];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Entry(Id),
    Short(Short),
    Session(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Locator {
    pub generation: Option<Generation>,
    pub records: Option<RecordRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub target: Target,
    pub locator: Locator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed(String);

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Malformed {}

const FORMS: &str = "#r<a>, #r<a>-<b>, #g<n> or #g<n>:r<a>-<b>";

impl Ref {
    pub fn parse(text: &str) -> Result<Ref, Malformed> {
        let (target, locator) = match text.split_once('#') {
            Some((target, locator)) => (target, Some(locator)),
            None => (text, None),
        };
        Ok(Ref {
            target: parse_target(target)?,
            locator: locator.map(parse_locator).transpose()?.unwrap_or_default(),
        })
    }
}

fn parse_target(text: &str) -> Result<Target, Malformed> {
    let bare = text.strip_prefix(SCHEME).unwrap_or(text);
    if let Some(id) = Id::parse(bare) {
        return Ok(Target::Entry(id));
    }
    if let Some(short) = Short::parse(bare) {
        return Ok(Target::Short(short));
    }
    if bare.len() == text.len() && is_session(text) {
        return Ok(Target::Session(text.to_ascii_lowercase()));
    }
    Err(Malformed(format!(
        "'{text}' is not an entry id, its last {} characters, a session id or {SHORTEST} or more characters of one",
        Short::LENGTH
    )))
}

pub(super) fn is_session(text: &str) -> bool {
    (SHORTEST..=UUID).contains(&text.len())
        && text.bytes().enumerate().all(|(at, byte)| {
            if DASHES.contains(&at) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn parse_locator(text: &str) -> Result<Locator, Malformed> {
    let malformed = || Malformed(format!("'#{text}' is not a locator: use {FORMS}"));
    let (generation, records) = match text.strip_prefix('g') {
        Some(rest) => match rest.split_once(':') {
            Some((generation, records)) => (Some(generation), Some(records)),
            None => (Some(rest), None),
        },
        None => (None, Some(text)),
    };
    let generation = generation
        .map(|digits| {
            let number = number(digits).and_then(|n| u32::try_from(n).ok());
            match number.ok_or_else(malformed)? {
                0 => Err(Malformed("generations are numbered from 1".to_owned())),
                n => Ok(Generation(n)),
            }
        })
        .transpose()?;
    let records = records
        .map(|records| {
            let range = records.strip_prefix('r').ok_or_else(malformed)?;
            let (first, last) = match range.split_once('-') {
                Some((first, last)) => (first, Some(last)),
                None => (range, None),
            };
            let first = number(first).ok_or_else(malformed)?;
            let last = last.map_or(Some(first), number).ok_or_else(malformed)?;
            if first == 0 {
                return Err(Malformed("records are numbered from 1".to_owned()));
            }
            if last < first {
                return Err(Malformed(format!("#{text} ends before it starts")));
            }
            Ok(RecordRange {
                first,
                count: last - first + 1,
            })
        })
        .transpose()?;
    Ok(Locator {
        generation,
        records,
    })
}

/// Decimal digits only, no sign and no leading zero, below `u64::MAX` so a range can count it.
fn number(digits: &str) -> Option<u64> {
    let plain = !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    digits
        .parse()
        .ok()
        .filter(|&number| plain && number < u64::MAX)
}

pub(crate) fn last(range: RecordRange) -> u64 {
    range.first.saturating_add(range.count.saturating_sub(1))
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.generation.is_none() && self.records.is_none() {
            return Ok(());
        }
        f.write_str("#")?;
        if let Some(generation) = self.generation {
            write!(f, "g{}", generation.0)?;
            if self.records.is_some() {
                f.write_str(":")?;
            }
        }
        match self.records {
            Some(range) if range.count <= 1 => write!(f, "r{}", range.first),
            Some(range) => write!(f, "r{}-{}", range.first, last(range)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::{Id, Short};

    use super::{Locator, Ref, Target, last};

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
    const UUID: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";

    fn range(first: u64, count: u64) -> Option<RecordRange> {
        Some(RecordRange { first, count })
    }

    fn locator(text: &str) -> Locator {
        Ref::parse(&format!("{ID}{text}"))
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .locator
    }

    #[test]
    fn an_entry_id_is_a_ref_with_or_without_the_citation_prefix() {
        let id = Id::parse(ID).expect("id");
        for text in [ID.to_owned(), format!("bilbo:{ID}")] {
            let parsed = Ref::parse(&text).expect("a ref");
            assert_eq!(parsed.target, Target::Entry(id));
            assert_eq!(parsed.locator, Locator::default());
        }
    }

    #[test]
    fn a_short_id_is_a_ref_with_or_without_the_citation_prefix_and_a_locator() {
        let short = Short::parse("2T6W9YB0DE").expect("short");
        for (text, locator) in [
            ("2T6W9YB0DE", Locator::default()),
            ("bilbo:2T6W9YB0DE", Locator::default()),
            (
                "2T6W9YB0DE#r3-7",
                Locator {
                    generation: None,
                    records: range(3, 5),
                },
            ),
            (
                "2T6W9YB0DE#g1:r3",
                Locator {
                    generation: Some(Generation(1)),
                    records: range(3, 1),
                },
            ),
        ] {
            let parsed = Ref::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(parsed.target, Target::Short(short), "{text}");
            assert_eq!(parsed.locator, locator, "{text}");
        }
    }

    #[test]
    fn ten_hex_characters_are_a_short_id_and_eight_are_a_session_id() {
        let target = |text: &str| Ref::parse(text).map(|parsed| parsed.target);
        assert_eq!(
            target("ABCDEF0123"),
            Ok(Target::Short(Short::parse("ABCDEF0123").expect("short")))
        );
        for text in ["ABCDEF01", "abcdef01"] {
            assert_eq!(target(text), Ok(Target::Session("abcdef01".to_owned())));
        }
        assert!(target("abcdef0123").is_err());
    }

    #[test]
    fn each_locator_form_names_a_generation_a_range_or_both() {
        let cases = [
            ("", None, None),
            ("#r5", None, range(5, 1)),
            ("#r5-9", None, range(5, 5)),
            ("#r5-5", None, range(5, 1)),
            ("#g1", Some(Generation(1)), None),
            ("#g12:r5", Some(Generation(12)), range(5, 1)),
            ("#g1:r5-9", Some(Generation(1)), range(5, 5)),
            ("#r1-18446744073709551614", None, range(1, u64::MAX - 1)),
        ];
        for (text, generation, records) in cases {
            assert_eq!(
                locator(text),
                Locator {
                    generation,
                    records
                },
                "{text}"
            );
        }
    }

    #[test]
    fn a_locator_prints_as_it_is_written() {
        for text in ["", "#r5", "#r5-9", "#g1", "#g12:r5", "#g1:r5-9"] {
            assert_eq!(locator(text).to_string(), text);
        }
        assert_eq!(locator("#r5-5").to_string(), "#r5");
        assert_eq!(last(RecordRange { first: 5, count: 5 }), 9);
        assert_eq!(last(RecordRange { first: 5, count: 0 }), 5);
    }

    #[test]
    fn a_session_id_is_a_uuid_or_eight_or_more_of_its_characters_in_any_case() {
        for text in [
            UUID.to_owned(),
            UUID.to_uppercase(),
            UUID[..8].to_owned(),
            UUID[..9].to_owned(),
            UUID[..20].to_owned(),
        ] {
            let parsed = Ref::parse(&format!("{text}#r3")).expect("a ref");
            assert_eq!(
                parsed.target,
                Target::Session(text.to_lowercase()),
                "{text}"
            );
            assert_eq!(parsed.locator.records, range(3, 1));
        }
    }

    #[test]
    fn a_malformed_ref_says_what_is_wrong() {
        let not_a_target = |text: &str| {
            format!(
                "'{text}' is not an entry id, its last 10 characters, a session id or 8 or more characters of one"
            )
        };
        let not_a_locator = |text: &str| {
            format!("'{text}' is not a locator: use #r<a>, #r<a>-<b>, #g<n> or #g<n>:r<a>-<b>")
        };
        let cases = [
            (String::new(), not_a_target("")),
            ("#r5".to_owned(), not_a_target("")),
            ("nope".to_owned(), not_a_target("nope")),
            (ID.to_lowercase(), not_a_target(&ID.to_lowercase())),
            (ID[..25].to_owned(), not_a_target(&ID[..25])),
            (UUID[..7].to_owned(), not_a_target(&UUID[..7])),
            (format!("{UUID}0"), not_a_target(&format!("{UUID}0"))),
            (
                UUID.replace('-', "_"),
                not_a_target(&UUID.replace('-', "_")),
            ),
            ("5c3a9e1f0b7d".to_owned(), not_a_target("5c3a9e1f0b7d")),
            (
                format!("bilbo:{UUID}"),
                not_a_target(&format!("bilbo:{UUID}")),
            ),
            (
                format!("claude {UUID}"),
                not_a_target(&format!("claude {UUID}")),
            ),
            (format!("{ID}#"), not_a_locator("#")),
            (format!("{ID}#5"), not_a_locator("#5")),
            (format!("{ID}#r"), not_a_locator("#r")),
            (format!("{ID}#r5-"), not_a_locator("#r5-")),
            (format!("{ID}#r-5"), not_a_locator("#r-5")),
            (format!("{ID}#r05"), not_a_locator("#r05")),
            (format!("{ID}#r+5"), not_a_locator("#r+5")),
            (format!("{ID}#r5-9-12"), not_a_locator("#r5-9-12")),
            (format!("{ID}#r5 "), not_a_locator("#r5 ")),
            (format!("{ID}#R5"), not_a_locator("#R5")),
            (format!("{ID}#g"), not_a_locator("#g")),
            (format!("{ID}#g1:"), not_a_locator("#g1:")),
            (format!("{ID}#g1:5"), not_a_locator("#g1:5")),
            (format!("{ID}#g1r5"), not_a_locator("#g1r5")),
            (format!("{ID}#r5:g1"), not_a_locator("#r5:g1")),
            (format!("{ID}#g4294967296"), not_a_locator("#g4294967296")),
            (
                format!("{ID}#r18446744073709551615"),
                not_a_locator("#r18446744073709551615"),
            ),
            (format!("{ID}#r5#r6"), not_a_locator("#r5#r6")),
            (
                format!("{ID}#r9-5"),
                "#r9-5 ends before it starts".to_owned(),
            ),
            (format!("{ID}#r0"), "records are numbered from 1".to_owned()),
            (
                format!("{ID}#r0-4"),
                "records are numbered from 1".to_owned(),
            ),
            (
                format!("{ID}#g0"),
                "generations are numbered from 1".to_owned(),
            ),
            (
                format!("{ID}#g0:r5"),
                "generations are numbered from 1".to_owned(),
            ),
        ];
        for (text, reason) in cases {
            assert_eq!(
                Ref::parse(&text).map_err(|error| error.to_string()),
                Err(reason),
                "{text:?}"
            );
        }
    }
}
