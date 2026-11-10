//! JSON as Python's `json` module reads and writes it, which is what the eval's render printed:
//! an object keeps its keys in the order written, an integer keeps every digit, and a float
//! prints as Python's `repr`.

use std::collections::HashMap;
use std::fmt::Write;

/// Nesting past this is not read: the walk is recursive.
const DEPTH: usize = 128;
const WIDE: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(String),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    /// A key written twice keeps its first place and its last value, as a Python dict does.
    Dict(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }

    pub fn is_dict(&self) -> bool {
        matches!(self, Value::Dict(_))
    }

    /// Python's truth: `None`, `False`, zero, and an empty string, list or dict are false.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(value) => *value,
            Value::Int(digits) => digits.bytes().any(|byte| matches!(byte, b'1'..=b'9')),
            Value::Float(value) => *value != 0.0,
            Value::Str(text) => !text.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Dict(fields) => !fields.is_empty(),
        }
    }

    /// `json.dumps(value, ensure_ascii=False)`.
    pub fn dumps(&self) -> String {
        let mut out = String::new();
        self.write(&JSON, &mut out);
        out
    }

    /// `str(value)`, as an f-string prints a value that is not a string.
    pub fn python_str(&self) -> String {
        match self {
            Value::Str(text) => text.clone(),
            other => {
                let mut out = String::new();
                other.write(&REPR, &mut out);
                out
            }
        }
    }

    fn write(&self, words: &Words, out: &mut String) {
        match self {
            Value::Null => out.push_str(words.null),
            Value::Bool(true) => out.push_str(words.truth),
            Value::Bool(false) => out.push_str(words.falsehood),
            Value::Int(digits) => out.push_str(int(digits)),
            Value::Float(value) => out.push_str(&float(*value, words.nan, words.infinity)),
            Value::Str(text) => (words.string)(text, out),
            Value::List(items) => {
                out.push('[');
                for (at, item) in items.iter().enumerate() {
                    if at > 0 {
                        out.push_str(", ");
                    }
                    item.write(words, out);
                }
                out.push(']');
            }
            Value::Dict(fields) => {
                out.push('{');
                for (at, (key, value)) in fields.iter().enumerate() {
                    if at > 0 {
                        out.push_str(", ");
                    }
                    (words.string)(key, out);
                    out.push_str(": ");
                    value.write(words, out);
                }
                out.push('}');
            }
        }
    }
}

struct Words {
    null: &'static str,
    truth: &'static str,
    falsehood: &'static str,
    nan: &'static str,
    infinity: &'static str,
    string: fn(&str, &mut String),
}

const JSON: Words = Words {
    null: "null",
    truth: "true",
    falsehood: "false",
    nan: "NaN",
    infinity: "Infinity",
    string: quoted,
};

const REPR: Words = Words {
    null: "None",
    truth: "True",
    falsehood: "False",
    nan: "nan",
    infinity: "inf",
    string: repr_str,
};

/// Python reads `-0` as the integer 0.
pub fn int(digits: &str) -> &str {
    if digits == "-0" { "0" } else { digits }
}

fn zeros(count: u32) -> String {
    "0".repeat(usize::try_from(count).unwrap_or(usize::MAX))
}

/// Python's `repr` of a float: the shortest digits that read back, in exponent form below 1e-4
/// and from 1e16 on.
fn float(value: f64, nan: &str, infinity: &str) -> String {
    if value.is_nan() {
        return nan.to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value.is_infinite() {
        return format!("{sign}{infinity}");
    }
    if value == 0.0 {
        return format!("{sign}0.0");
    }
    let magnitude = value.abs();
    let shortest = format!("{magnitude:e}");
    // Two decimals of the shortest length can read back as the value when it lies half way
    // between them. Python prints the one rounded half to even, which is what a precision gives.
    let places = shortest
        .bytes()
        .take_while(|byte| *byte != b'e')
        .filter(u8::is_ascii_digit)
        .count();
    let rounded = format!("{magnitude:.*e}", places.saturating_sub(1));
    let shortest = if rounded.parse() == Ok(magnitude) {
        rounded
    } else {
        shortest
    };
    let (mantissa, exponent) = shortest.split_once('e').unwrap_or((&shortest, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let point = exponent + 1;
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let body = if !(-3..=16).contains(&point) {
        let (first, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let mark = if exponent < 0 { '-' } else { '+' };
        format!("{first}{fraction}e{mark}{:02}", exponent.abs())
    } else if point <= 0 {
        format!("0.{}{digits}", zeros(point.unsigned_abs()))
    } else if point >= count {
        format!("{digits}{}.0", zeros((point - count).unsigned_abs()))
    } else {
        let at = usize::try_from(point).unwrap_or(digits.len());
        let (whole, fraction) = digits.split_at(at.min(digits.len()));
        format!("{whole}.{fraction}")
    };
    format!("{sign}{body}")
}

fn quoted(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A character Python calls unprintable is escaped only when it is ASCII.
fn repr_str(text: &str, out: &mut String) {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c < ' ' || c == '\u{7f}' => {
                let _ = write!(out, "\\x{:02x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
}

/// As `json.loads` reads it: `NaN`, `Infinity` and `-Infinity` are numbers; a lone surrogate escape reads as U+FFFD.
/// `None` for anything it refuses, and for nesting past `DEPTH`.
pub fn parse(record: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(record).ok()?;
    let mut walk = Walk {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    walk.space();
    let value = walk.value(0)?;
    walk.space();
    (walk.at == walk.bytes.len()).then_some(value)
}

struct Walk<'t> {
    text: &'t str,
    bytes: &'t [u8],
    at: usize,
}

impl Walk<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn word(&mut self, word: &str) -> bool {
        let found = self.bytes[self.at..].starts_with(word.as_bytes());
        if found {
            self.at += word.len();
        }
        found
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        match self.peek()? {
            b'{' => self.dict(depth),
            b'[' => self.list(depth),
            b'"' => self.string().map(Value::Str),
            b'n' => self.word("null").then_some(Value::Null),
            b't' => self.word("true").then_some(Value::Bool(true)),
            b'f' => self.word("false").then_some(Value::Bool(false)),
            b'N' => self.word("NaN").then_some(Value::Float(f64::NAN)),
            b'I' => self.word("Infinity").then_some(Value::Float(f64::INFINITY)),
            b'-' if self.bytes[self.at..].starts_with(b"-Infinity") => {
                self.at += "-Infinity".len();
                Some(Value::Float(f64::NEG_INFINITY))
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn dict(&mut self, depth: usize) -> Option<Value> {
        if depth >= DEPTH {
            return None;
        }
        self.at += 1;
        let mut fields: Vec<(String, Value)> = Vec::new();
        let mut places: Option<HashMap<String, usize>> = None;
        self.space();
        if self.peek()? == b'}' {
            self.at += 1;
            return Some(Value::Dict(fields));
        }
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.space();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            self.space();
            let value = self.value(depth + 1)?;
            let found = match &places {
                Some(places) => places.get(&key).copied(),
                None => fields.iter().position(|(name, _)| *name == key),
            };
            match found {
                Some(at) => fields[at].1 = value,
                None => {
                    if let Some(places) = &mut places {
                        places.insert(key.clone(), fields.len());
                    }
                    fields.push((key, value));
                }
            }
            if places.is_none() && fields.len() >= WIDE {
                let known = fields.iter().enumerate();
                places = Some(known.map(|(at, (name, _))| (name.clone(), at)).collect());
            }
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Some(Value::Dict(fields));
                }
                _ => return None,
            }
        }
    }

    fn list(&mut self, depth: usize) -> Option<Value> {
        if depth >= DEPTH {
            return None;
        }
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.peek()? == b']' {
            self.at += 1;
            return Some(Value::List(items));
        }
        loop {
            self.space();
            items.push(self.value(depth + 1)?);
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(Value::List(items));
                }
                _ => return None,
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        let leading_zero = self.peek() == Some(b'0');
        match self.digits() {
            0 => return None,
            1 => {}
            _ if leading_zero => return None,
            _ => {}
        }
        let mut whole = true;
        if self.peek() == Some(b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return None;
            }
            whole = false;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.at;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                // Python stops the number before an exponent with no digit: the text then fails.
                self.at = mark;
                return None;
            }
            whole = false;
        }
        let token = &self.text[start..self.at];
        if whole {
            Some(Value::Int(token.to_owned()))
        } else {
            token.parse().ok().map(Value::Float)
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.text.get(self.at..self.at + 4)?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        self.at += 4;
        u32::from_str_radix(digits, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while !matches!(self.peek()?, b'"' | b'\\' | 0..=0x1f) {
                self.at += 1;
            }
            out.push_str(&self.text[start..self.at]);
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.escaped()?),
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    fn escaped(&mut self) -> Option<char> {
        let first = self.hex4()?;
        if (0xD800..0xDC00).contains(&first) && self.bytes[self.at..].starts_with(b"\\u") {
            let back = self.at;
            self.at += 2;
            let second = self.hex4()?;
            if (0xDC00..0xE000).contains(&second) {
                let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                return char::from_u32(code);
            }
            self.at = back;
        }
        Some(char::from_u32(first).unwrap_or(char::REPLACEMENT_CHARACTER))
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, parse};

    fn dumped(text: &str) -> Option<String> {
        parse(text.as_bytes()).map(|value| value.dumps())
    }

    #[test]
    fn an_object_prints_its_keys_in_the_order_written_with_pythons_separators() {
        assert_eq!(
            dumped(r#"{"b":1,"a":[1,2,{"z":null,"y":true}],"c":{},"d":[]}"#).as_deref(),
            Some(r#"{"b": 1, "a": [1, 2, {"z": null, "y": true}], "c": {}, "d": []}"#)
        );
        assert_eq!(
            dumped(" { \"k\" :\tfalse ,\r\n\"l\":\"v\" } ").as_deref(),
            Some(r#"{"k": false, "l": "v"}"#)
        );
    }

    #[test]
    fn a_key_written_twice_keeps_its_first_place_and_its_last_value() {
        assert_eq!(
            dumped(r#"{"a":1,"b":2,"a":3}"#).as_deref(),
            Some(r#"{"a": 3, "b": 2}"#)
        );
    }

    #[test]
    fn a_wide_object_keeps_its_order_and_its_last_values_and_reads_in_linear_time() {
        let wide: Vec<String> = (0..40).map(|n| format!("\"k{n}\":{n}")).collect();
        let text = format!(
            "{{{},\"k3\":\"x\",\"k39\":\"y\",\"new\":1}}",
            wide.join(",")
        );
        let Some(Value::Dict(fields)) = parse(text.as_bytes()) else {
            panic!("an object");
        };
        assert_eq!(fields.len(), 41);
        assert_eq!(fields[3], ("k3".to_owned(), Value::Str("x".to_owned())));
        assert_eq!(fields[39], ("k39".to_owned(), Value::Str("y".to_owned())));
        assert_eq!(fields[40].0, "new");

        let keys: Vec<String> = (0..200_000).map(|n| format!("\"{n}\":0")).collect();
        let text = format!("{{{}}}", keys.join(","));
        let started = std::time::Instant::now();
        let Some(Value::Dict(fields)) = parse(text.as_bytes()) else {
            panic!("an object");
        };
        assert_eq!(fields.len(), 200_000);
        // A scan of the fields for each key is twenty thousand million comparisons.
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    #[test]
    fn a_string_keeps_its_characters_and_escapes_only_what_python_escapes() {
        let cases = [
            (r#""é ✓ 😀 /""#, r#""é ✓ 😀 /""#),
            (r#""é✓😀\/""#, r#""é✓😀/""#),
            (r#""a\"b\\c""#, r#""a\"b\\c""#),
            (r#""\n\r\t\b\f""#, r#""\n\r\t\b\f""#),
            (
                r#""\u0000\u001f\u007f\u0085 ""#,
                "\"\\u0000\\u001f\u{7f}\u{85}\u{2028}\"",
            ),
            (r#""\ud83d""#, "\"\u{fffd}\""),
            (r#""\ude00x""#, "\"\u{fffd}x\""),
            (r#""\ud83dA""#, "\"\u{fffd}A\""),
        ];
        for (text, printed) in cases {
            assert_eq!(dumped(text).as_deref(), Some(printed), "{text}");
        }
    }

    #[test]
    fn an_integer_keeps_every_digit_and_minus_zero_is_zero() {
        for (text, printed) in [
            ("0", "0"),
            ("-0", "0"),
            ("-7", "-7"),
            ("18446744073709551616", "18446744073709551616"),
            (
                "-123456789012345678901234567890",
                "-123456789012345678901234567890",
            ),
        ] {
            assert_eq!(dumped(text).as_deref(), Some(printed), "{text}");
        }
    }

    #[test]
    fn a_float_prints_as_pythons_repr() {
        for (text, printed) in [
            ("1.0", "1.0"),
            ("-0.0", "-0.0"),
            ("0.0", "0.0"),
            ("1e0", "1.0"),
            ("1E5", "100000.0"),
            ("1.5", "1.5"),
            ("0.1", "0.1"),
            ("100.25", "100.25"),
            ("0.0001", "0.0001"),
            ("0.00001", "1e-05"),
            ("0.000012345", "1.2345e-05"),
            ("1e15", "1000000000000000.0"),
            ("9999999999999998.0", "9999999999999998.0"),
            ("1e16", "1e+16"),
            ("1.5e16", "1.5e+16"),
            ("123456789012345680000.0", "1.2345678901234568e+20"),
            ("1e22", "1e+22"),
            ("1e-7", "1e-07"),
            ("1.7976931348623157e308", "1.7976931348623157e+308"),
            ("5e-324", "5e-324"),
            ("2.5e-100", "2.5e-100"),
            ("0.30000000000000004", "0.30000000000000004"),
            ("1e400", "Infinity"),
            ("-1e400", "-Infinity"),
            ("NaN", "NaN"),
            ("Infinity", "Infinity"),
            ("-Infinity", "-Infinity"),
            ("3.0e2", "300.0"),
            ("-12.5E-1", "-1.25"),
            ("628661958223826.25", "628661958223826.2"),
            ("628661958223826.75", "628661958223826.8"),
            ("-1325584522169260.25", "-1325584522169260.2"),
            ("-233739651248482.625", "-233739651248482.62"),
            ("9007199254740993.0", "9007199254740992.0"),
            ("0.1e-322", "1e-323"),
            ("2.2250738585072014e-308", "2.2250738585072014e-308"),
            ("4.35", "4.35"),
            ("2.675", "2.675"),
        ] {
            assert_eq!(dumped(text).as_deref(), Some(printed), "{text}");
        }
    }

    #[test]
    fn what_json_loads_refuses_is_not_read() {
        for text in [
            "",
            " ",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "{a:1}",
            "{\"a\" 1}",
            "'a'",
            "01",
            "-",
            "1.",
            ".5",
            "1e",
            "1e+",
            "+1",
            "nul",
            "True",
            "nan",
            "\"a",
            "\"a\\x\"",
            "\"a\\u12\"",
            "\"a\\u12g4\"",
            "\"tab\there\"",
            "\"line\nbreak\"",
            "1 2",
            "{} x",
            "\u{feff}{}",
            "\u{a0}{}",
        ] {
            assert_eq!(parse(text.as_bytes()), None, "{text:?}");
        }
        assert_eq!(parse(b"\xff"), None);
        assert_eq!(parse(b"\"\xc3\""), None);
    }

    #[test]
    fn nesting_is_read_to_a_depth_and_refused_past_it() {
        let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(parse(nested(128).as_bytes()).is_some());
        assert_eq!(parse(nested(129).as_bytes()), None);
        assert_eq!(parse("[".repeat(100_000).as_bytes()), None);
        let objects = format!("{}1{}", "{\"a\":".repeat(129), "}".repeat(129));
        assert_eq!(parse(objects.as_bytes()), None);
    }

    #[test]
    fn truth_is_pythons() {
        for (text, truth) in [
            ("null", false),
            ("false", false),
            ("true", true),
            ("0", false),
            ("-0", false),
            ("0.0", false),
            ("-0.0", false),
            ("0e5", false),
            ("7", true),
            ("100", true),
            ("0.1", true),
            ("NaN", true),
            ("\"\"", false),
            ("\"false\"", true),
            ("\"0\"", true),
            ("[]", false),
            ("[0]", true),
            ("{}", false),
            ("{\"a\":null}", true),
        ] {
            assert_eq!(
                parse(text.as_bytes()).map(|v| v.truthy()),
                Some(truth),
                "{text}"
            );
        }
    }

    #[test]
    fn a_value_in_an_f_string_prints_as_pythons_str() {
        for (text, printed) in [
            ("\"Bash\"", "Bash"),
            ("null", "None"),
            ("true", "True"),
            ("false", "False"),
            ("-0", "0"),
            ("12", "12"),
            ("1.50", "1.5"),
            ("1e400", "inf"),
            ("NaN", "nan"),
            ("[\"a\", 1, null]", "['a', 1, None]"),
            (
                "{\"k\": [true], \"it's\": \"q\\\"\"}",
                "{'k': [True], \"it's\": 'q\"'}",
            ),
            ("[\"both ' and \\\"\"]", "['both \\' and \"']"),
            ("[\"a\\nb\\\\\\u0001\"]", "['a\\nb\\\\\\x01']"),
        ] {
            assert_eq!(
                parse(text.as_bytes()).map(|value| value.python_str()),
                Some(printed.to_owned()),
                "{text}"
            );
        }
    }

    #[test]
    fn a_field_is_found_by_its_key_in_an_object_only() {
        let value = parse(br#"{"a": "x", "b": {"c": 1}}"#).expect("an object");
        assert_eq!(value.get("a").and_then(Value::as_str), Some("x"));
        assert!(value.get("b").is_some_and(Value::is_dict));
        assert_eq!(value.get("z"), None);
        assert_eq!(Value::List(vec![]).get("a"), None);
        assert_eq!(Value::Int("1".to_owned()).as_str(), None);
    }
}
