use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use bilbo_kinds::reader::{Passage, Piece};
use bilbo_store::entries::RecordRange;

use crate::json::{self, Value};

/// What Python's `str.isspace` adds to Rust's `char::is_whitespace`: the four separators.
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

pub fn head_of(piece: &str) -> &str {
    let first = piece.split('\n').next().unwrap_or_default();
    first.find(':').map_or(first, |colon| &first[..=colon])
}

pub fn input(value: &Value) -> String {
    match value {
        Value::Dict(fields) => fields
            .iter()
            .map(|(key, value)| match value {
                Value::Str(text) => format!("{key}: {text}"),
                other => format!("{key}: {}", other.dumps()),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.dumps(),
    }
}

pub fn labelled(head: String, body: &str) -> String {
    if body.is_empty() {
        head
    } else {
        format!("{head}\n{body}")
    }
}

/// A JSON scalar as a Python dict key: `1`, `1.0` and `true` are one key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    None,
    Text(String),
    Whole(String),
    Float(u64),
    /// `json.loads` gives every `NaN` the same object, which a dict finds by identity.
    Nan,
}

/// `None` for a list or an object, which Python cannot hash.
fn key(value: Option<&Value>) -> Option<Key> {
    Some(match value {
        None | Some(Value::Null) => Key::None,
        Some(Value::Str(text)) => Key::Text(text.clone()),
        Some(Value::Bool(value)) => Key::Whole(u8::from(*value).to_string()),
        Some(Value::Int(digits)) => Key::Whole(json::int(digits).to_owned()),
        Some(Value::Float(value)) if value.is_nan() => Key::Nan,
        Some(Value::Float(value)) if value.is_finite() && value.fract() == 0.0 => {
            Key::Whole(json::int(&format!("{value:.0}")).to_owned())
        }
        Some(Value::Float(value)) => Key::Float(value.to_bits()),
        Some(Value::List(_) | Value::Dict(_)) => return None,
    })
}

#[derive(Debug, Default)]
pub struct Tools(HashMap<Key, String>);

impl Tools {
    /// `false` when the id cannot be a key: the eval's render stops there.
    pub fn name(&mut self, id: Option<&Value>, name: String) -> bool {
        key(id).map(|key| self.0.insert(key, name)).is_some()
    }

    pub fn named(&self, id: Option<&Value>) -> Option<&str> {
        key(id).map(|key| self.0.get(&key).map_or("unknown", String::as_str))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outline {
    Listed,
    Skipped,
}

impl Outline {
    pub fn of(listed: bool) -> Outline {
        if listed {
            Outline::Listed
        } else {
            Outline::Skipped
        }
    }
}

pub fn passage(
    record: u64,
    pieces: &[String],
    at: Option<SystemTime>,
    outline: Outline,
) -> Option<Passage> {
    if pieces.is_empty() {
        return None;
    }
    let mut start = 0;
    let heads = pieces
        .iter()
        .map(|piece| {
            let head = Piece {
                start,
                label: head_of(piece).to_owned(),
            };
            start += piece.len() + 1;
            head
        })
        .collect();
    let text = pieces.join("\n");
    Some(Passage {
        range: RecordRange {
            first: record,
            count: 1,
        },
        ends: vec![text.len()],
        outline: (outline == Outline::Listed)
            .then(|| text.lines().next().unwrap_or_default().to_owned()),
        text,
        pieces: heads,
        at,
    })
}

fn digits(text: &str, from: usize, count: usize) -> Option<u64> {
    let part = text.get(from..from + count)?;
    part.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| part.parse().ok())
        .flatten()
}

fn days(year: u64, month: u64, day: u64) -> Option<u64> {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let length = *lengths.get(usize::try_from(month.checked_sub(1)?).ok()?)?;
    if year < 1970 || !(1..=length).contains(&day) {
        return None;
    }
    let before = year - 1;
    let leaps = before / 4 - before / 100 + before / 400 - (1969 / 4 - 1969 / 100 + 1969 / 400);
    let in_year: u64 = lengths.iter().take(usize::try_from(month - 1).ok()?).sum();
    Some((year - 1970) * 365 + leaps + in_year + day - 1)
}

/// A harness timestamp: `2026-11-07T16:39:12.345Z`, the fraction optional, the zone `Z`, an
/// offset or none (then UTC). `None` for anything else and for a time before 1970.
pub fn time(value: Option<&Value>) -> Option<SystemTime> {
    let text = value?.as_str()?;
    let bytes = text.as_bytes();
    let marks = bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && matches!(bytes.get(10), Some(b'T' | b' '))
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':');
    if !marks {
        return None;
    }
    let day = days(
        digits(text, 0, 4)?,
        digits(text, 5, 2)?,
        digits(text, 8, 2)?,
    )?;
    let (hour, minute, second) = (
        digits(text, 11, 2)?,
        digits(text, 14, 2)?,
        digits(text, 17, 2)?,
    );
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = text.get(19..)?;
    let mut nanos = 0u32;
    if let Some(fraction) = rest.strip_prefix('.') {
        let count = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if count == 0 {
            return None;
        }
        let padded = format!("{:0<9}", &fraction[..count.min(9)]);
        nanos = padded.parse().ok()?;
        rest = &fraction[count..];
    }
    let local = day * 86_400 + hour * 3600 + minute * 60 + second;
    let seconds = match rest {
        "" | "Z" => local,
        zone => {
            let east = match zone.as_bytes().first()? {
                b'+' => true,
                b'-' => false,
                _ => return None,
            };
            if zone.len() != 6 || zone.as_bytes()[3] != b':' {
                return None;
            }
            let (hours, minutes) = (digits(zone, 1, 2)?, digits(zone, 4, 2)?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            let offset = hours * 3600 + minutes * 60;
            if east {
                local.checked_sub(offset)?
            } else {
                local.checked_add(offset)?
            }
        }
    };
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(seconds, nanos))
}

/// `YYYY-MM-DD HH:MM` in UTC, the second dropped.
pub fn minute(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted % 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        of_day / 3600,
        of_day % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bilbo_kinds::reader::Piece;

    use super::{Outline, Tools, head_of, passage, strip, time};
    use crate::json::{Value, parse};

    #[test]
    fn strip_takes_what_pythons_isspace_takes_and_nothing_else() {
        // Every character `str.isspace` is true for, from CPython 3.13's tables.
        let spaces = [
            0x9, 0xa, 0xb, 0xc, 0xd, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x85, 0xa0, 0x1680, 0x2000,
            0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028,
            0x2029, 0x202f, 0x205f, 0x3000,
        ];
        for code in 0..=0x3_0000u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let text = format!("{c}x{c}");
            let stripped = spaces.contains(&code);
            assert_eq!(strip(&text) == "x", stripped, "U+{code:04X}");
        }
        assert_eq!(strip("\u{1c} a b \u{1f}\n"), "a b");
        assert_eq!(strip("\u{200b}a\u{feff}"), "\u{200b}a\u{feff}");
        assert_eq!(strip(" \n\t"), "");
    }

    #[test]
    fn a_label_line_ends_at_the_first_colon_of_the_first_line() {
        for (piece, head) in [
            ("user: hello: world", "user:"),
            ("tool_use Edit:\nfile: a", "tool_use Edit:"),
            ("tool_use mcp__a:b:\nx", "tool_use mcp__a:"),
            ("tool_use two\nlines:", "tool_use two"),
            ("no colon here", "no colon here"),
            ("", ""),
        ] {
            assert_eq!(head_of(piece), head, "{piece:?}");
        }
    }

    fn value(text: &str) -> Value {
        parse(text.as_bytes()).expect("JSON")
    }

    #[test]
    fn a_result_finds_the_name_its_call_gave_the_same_id() {
        let mut tools = Tools::default();
        assert!(tools.name(Some(&value("\"toolu_1\"")), "Bash".to_owned()));
        assert!(tools.name(None, "NoId".to_owned()));
        assert_eq!(tools.named(Some(&value("\"toolu_1\""))), Some("Bash"));
        assert_eq!(tools.named(Some(&value("\"toolu_2\""))), Some("unknown"));
        assert_eq!(tools.named(None), Some("NoId"));
        assert_eq!(tools.named(Some(&Value::Null)), Some("NoId"));
        assert!(tools.name(Some(&value("\"toolu_1\"")), "Edit".to_owned()));
        assert_eq!(tools.named(Some(&value("\"toolu_1\""))), Some("Edit"));
    }

    #[test]
    fn ids_are_keys_as_python_compares_them() {
        let mut tools = Tools::default();
        assert!(tools.name(Some(&value("1")), "one".to_owned()));
        for same in ["1", "1.0", "true", "1e0"] {
            assert_eq!(tools.named(Some(&value(same))), Some("one"), "{same}");
        }
        assert_eq!(tools.named(Some(&value("\"1\""))), Some("unknown"));
        assert!(tools.name(Some(&value("-0.0")), "zero".to_owned()));
        for same in ["0", "-0", "false", "0.0"] {
            assert_eq!(tools.named(Some(&value(same))), Some("zero"), "{same}");
        }
        assert!(tools.name(Some(&value("1e20")), "big".to_owned()));
        assert_eq!(
            tools.named(Some(&value("100000000000000000000"))),
            Some("big")
        );
        assert!(tools.name(Some(&value("1.5")), "half".to_owned()));
        assert_eq!(tools.named(Some(&value("1.5"))), Some("half"));
        assert!(tools.name(Some(&value("NaN")), "nan".to_owned()));
        assert_eq!(tools.named(Some(&value("NaN"))), Some("nan"));
        assert!(tools.name(Some(&value("1e400")), "infinite".to_owned()));
        assert_eq!(tools.named(Some(&value("Infinity"))), Some("infinite"));
        assert_eq!(tools.named(Some(&value("-Infinity"))), Some("unknown"));
        for unhashable in ["[]", "{}", "[1]"] {
            assert!(!tools.name(Some(&value(unhashable)), "x".to_owned()));
            assert_eq!(tools.named(Some(&value(unhashable))), None, "{unhashable}");
        }
    }

    #[test]
    fn a_passage_joins_its_pieces_and_says_where_each_starts_in_bytes() {
        let pieces = [
            "assistant: olá".to_owned(),
            "tool_use Bash:\ncommand: ls".to_owned(),
        ];
        let found = passage(7, &pieces, None, Outline::Skipped).expect("a passage");
        assert_eq!(found.text, "assistant: olá\ntool_use Bash:\ncommand: ls");
        assert_eq!((found.range.first, found.range.count), (7, 1));
        assert_eq!(found.ends, [found.text.len()]);
        assert_eq!(found.outline, None);
        let listed = passage(7, &pieces[1..], None, Outline::Listed).expect("a passage");
        assert_eq!(listed.outline.as_deref(), Some("tool_use Bash:"));
        assert_eq!(
            found.pieces,
            [
                Piece {
                    start: 0,
                    label: "assistant:".to_owned()
                },
                Piece {
                    start: 16,
                    label: "tool_use Bash:".to_owned()
                },
            ]
        );
        assert_eq!(passage(7, &[], None, Outline::Listed), None);
    }

    fn at(text: &str) -> Option<u64> {
        time(Some(&Value::Str(text.to_owned()))).map(|time| {
            let since = time
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("after 1970");
            u64::try_from(since.as_millis()).expect("fits")
        })
    }

    #[test]
    fn a_harness_timestamp_is_read_with_its_fraction_and_zone() {
        for (text, millis) in [
            ("1970-01-01T00:00:00Z", 0),
            ("2026-11-07T16:39:12.345Z", 1_794_069_552_345),
            ("2026-11-07T16:39:12Z", 1_794_069_552_000),
            ("2026-11-07T16:39:12", 1_794_069_552_000),
            ("2026-11-07 16:39:12.5Z", 1_794_069_552_500),
            ("2026-11-07T16:39:12.123456789Z", 1_794_069_552_123),
            ("2026-11-07T16:39:12.1234567891234Z", 1_794_069_552_123),
            ("2026-11-07T13:39:12-03:00", 1_794_069_552_000),
            ("2026-11-07T18:09:12+01:30", 1_794_069_552_000),
            ("2024-02-29T00:00:00Z", 1_709_164_800_000),
            ("2000-03-01T00:00:00Z", 951_868_800_000),
            ("2100-03-01T00:00:00Z", 4_107_542_400_000),
        ] {
            assert_eq!(at(text), Some(millis), "{text}");
        }
    }

    #[test]
    fn a_zone_offset_past_a_day_or_an_hour_is_no_time() {
        for text in [
            "2026-11-07T16:39:12+99:99",
            "2026-11-07T16:39:12+24:00",
            "2026-11-07T16:39:12-00:60",
        ] {
            assert_eq!(at(text), None, "{text}");
        }
        assert_eq!(at("2026-11-07T16:39:12+23:59"), Some(1_793_983_212_000));
    }

    #[test]
    fn anything_else_is_no_time() {
        for text in [
            "",
            "2026-11-07",
            "2026-11-07T16:39",
            "2026-13-07T16:39:12Z",
            "2026-02-30T16:39:12Z",
            "2025-02-29T00:00:00Z",
            "2100-02-29T00:00:00Z",
            "2026-11-07T24:00:00Z",
            "2026-11-07T16:60:00Z",
            "2026-11-07T16:39:12.Z",
            "2026-11-07T16:39:12X",
            "2026-11-07T16:39:12+0100",
            "2026-11-07T16:39:12+1:00",
            "1969-12-31T23:59:59Z",
            "1970-01-01T00:00:00+01:00",
            "２０２６-11-07T16:39:12Z",
            "+026-11-07T16:39:12Z",
            "2026-11-07T16:39:12Zé",
        ] {
            assert_eq!(at(text), None, "{text:?}");
        }
        assert_eq!(time(None), None);
        assert_eq!(time(Some(&Value::Int("5".to_owned()))), None);
        assert_eq!(
            time(Some(&Value::Str("1970-01-01T00:00:01Z".to_owned()))),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
        );
    }
}
