use std::borrow::Cow;

use serde::de::IgnoredAny;
use xxhash_rust::xxh3;

/// Padding does not count.
pub const THRESHOLD: usize = 256;

/// Replaces each JSON string value of 256+ base64 or base64url characters, after an optional
/// `data:...;base64,` head, by a placeholder with its length and digest. Keys and hex-only strings
/// pass; a record that is not JSON comes back as it is.
pub fn filter(record: &[u8]) -> Cow<'_, [u8]> {
    let found = candidates(record);
    if found.is_empty() || serde_json::from_slice::<IgnoredAny>(record).is_err() {
        return Cow::Borrowed(record);
    }
    let mut out = Vec::with_capacity(record.len());
    let mut copied = 0;
    for (start, end, placeholder) in found {
        out.extend_from_slice(&record[copied..start]);
        out.push(b'"');
        out.extend_from_slice(placeholder.as_bytes());
        out.push(b'"');
        copied = end;
    }
    out.extend_from_slice(&record[copied..]);
    Cow::Owned(out)
}

/// Tokens are found as if `record` were JSON; `filter` uses them only once it is. A string followed
/// by `:` is a key.
fn candidates(record: &[u8]) -> Vec<(usize, usize, String)> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = record[at..].iter().position(|&byte| byte == b'"') {
        let start = at + offset;
        let mut cursor = start + 1;
        while cursor < record.len() && record[cursor] != b'"' {
            cursor += if record[cursor] == b'\\' { 2 } else { 1 };
        }
        let end = (cursor + 1).min(record.len());
        at = end;
        if end - start < THRESHOLD + 2 || is_key(&record[end..]) {
            continue;
        }
        if let Ok(text) = serde_json::from_slice::<String>(&record[start..end])
            && let Some(placeholder) = placeholder(&text)
        {
            found.push((start, end, placeholder));
        }
    }
    found
}

fn is_key(after: &[u8]) -> bool {
    after
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        == Some(&b':')
}

pub fn placeholder(text: &str) -> Option<String> {
    let (media, body) = match text.strip_prefix("data:") {
        Some(rest) => {
            let (head, body) = rest.split_once(";base64,")?;
            (media_of(head)?, body)
        }
        None => (None, text),
    };
    let unpadded = body
        .strip_suffix("==")
        .or_else(|| body.strip_suffix('='))
        .unwrap_or(body);
    let base64 = unpadded.len() >= THRESHOLD
        && unpadded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_'));
    if !base64 || unpadded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let digest = xxh3::xxh3_64(text.as_bytes());
    Some(match media {
        Some(media) => format!("[opaque {media}: {} chars, xxh3 {digest:016x}]", text.len()),
        None => format!("[opaque: {} chars, xxh3 {digest:016x}]", text.len()),
    })
}

pub fn is_placeholder(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("[opaque") else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(']') else {
        return false;
    };
    let rest = match rest.strip_prefix(": ") {
        Some(rest) => rest,
        None => {
            let Some((media, rest)) = rest.strip_prefix(' ').and_then(|r| r.split_once(": "))
            else {
                return false;
            };
            if !is_media_type(media) {
                return false;
            }
            rest
        }
    };
    let Some((digits, digest)) = rest.split_once(" chars, xxh3 ") else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && digest.len() == 16
        && digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// `Some(None)` for an empty media type, `None` when the head is not one followed by
/// `;<name>=<value>`.
fn media_of(head: &str) -> Option<Option<&str>> {
    let mut parts = head.split(';');
    let media = parts.next().unwrap_or_default();
    let parameters = parts.all(|parameter| {
        parameter
            .split_once('=')
            .is_some_and(|(name, value)| is_token(name) && is_token(value))
    });
    if !parameters {
        None
    } else if media.is_empty() {
        Some(None)
    } else {
        is_media_type(media).then_some(Some(media))
    }
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b'_'))
}

fn is_media_type(text: &str) -> bool {
    let Some((kind, subtype)) = text.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && kind.bytes().all(|byte| byte.is_ascii_alphabetic())
        && !subtype.is_empty()
        && subtype
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::{THRESHOLD, filter, is_placeholder, placeholder};

    fn base64(length: usize) -> String {
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .chars()
            .cycle()
            .take(length)
            .collect()
    }

    fn line(value: &str) -> String {
        format!("{{\"type\":\"x\",\"data\":\"{value}\",\"n\":1}}\n")
    }

    fn filtered(text: &str) -> String {
        String::from_utf8(filter(text.as_bytes()).into_owned()).expect("UTF-8")
    }

    fn kept(text: &str) -> bool {
        filter(text.as_bytes()) == text.as_bytes()
    }

    #[test]
    fn a_string_of_256_base64_characters_or_more_is_opaque_and_255_is_not() {
        assert_eq!(THRESHOLD, 256);
        assert!(kept(&line(&base64(255))));
        assert!(!kept(&line(&base64(256))));
        assert!(!kept(&line(&base64(257))));
    }

    #[test]
    fn padding_does_not_count_toward_the_threshold() {
        assert!(kept(&line(&format!("{}=", base64(THRESHOLD - 1)))));
        assert!(kept(&line(&format!("{}==", base64(THRESHOLD - 2)))));
        assert!(!kept(&line(&format!("{}==", base64(THRESHOLD)))));
        assert!(kept(&line(&format!("{}===", base64(THRESHOLD)))));
    }

    #[test]
    fn both_alphabets_count_and_any_other_character_keeps_the_string() {
        let url: String = base64(300).replace('+', "-").replace('/', "_");
        assert!(!kept(&line(&url)));
        let mixed = format!("{}-_+/", base64(300));
        assert!(!kept(&line(&mixed)));
        for other in [" ", ".", "!", "\\n", "\\u00e9"] {
            let text = format!("{}{other}{}", base64(200), base64(200));
            assert!(kept(&line(&text)), "{other:?}");
        }
    }

    #[test]
    fn a_hex_string_of_any_length_passes() {
        let hex: String = "0123456789abcdef".chars().cycle().take(300).collect();
        assert!(kept(&line(&hex)));
        assert!(kept(&line(&hex.to_uppercase())));
        let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
        assert!(kept(&line(sha)));
    }

    #[test]
    fn an_escaped_string_is_judged_by_what_it_decodes_to() {
        let escaped = base64(300).replace('/', "\\/");
        assert!(escaped.contains("\\/"));
        let text = line(&escaped);
        assert_eq!(
            filtered(&text),
            line(&placeholder(&base64(300)).expect("opaque"))
        );
        let split = format!("{}\\u0041{}", base64(150), base64(150));
        assert!(!kept(&line(&split)));
    }

    #[test]
    fn a_data_uri_is_opaque_and_keeps_its_media_type() {
        let uri = format!("data:image/png;base64,{}", base64(400));
        assert_eq!(
            placeholder(&uri),
            Some(format!(
                "[opaque image/png: {} chars, xxh3 {:016x}]",
                uri.len(),
                xxhash_rust::xxh3::xxh3_64(uri.as_bytes())
            ))
        );
        let bad_media = format!("data:image png;base64,{}", base64(400));
        assert_eq!(placeholder(&bad_media), None);
        let short = format!("data:image/png;base64,{}", base64(THRESHOLD - 1));
        assert_eq!(placeholder(&short), None);
    }

    #[test]
    fn a_data_uri_with_parameters_keeps_its_type_and_an_empty_type_is_none() {
        let body = base64(400);
        let digest = |uri: &str| xxhash_rust::xxh3::xxh3_64(uri.as_bytes());
        let svg = format!("data:image/svg+xml;charset=utf-8;base64,{body}");
        assert_eq!(
            placeholder(&svg),
            Some(format!(
                "[opaque image/svg+xml: {} chars, xxh3 {:016x}]",
                svg.len(),
                digest(&svg)
            ))
        );
        let empty = format!("data:;base64,{body}");
        assert_eq!(
            placeholder(&empty),
            Some(format!(
                "[opaque: {} chars, xxh3 {:016x}]",
                empty.len(),
                digest(&empty)
            ))
        );
        assert_eq!(
            placeholder(&format!("data:image/png;charset;base64,{body}")),
            None
        );
        assert_eq!(
            placeholder(&format!("data:image/png;charset=;base64,{body}")),
            None
        );
    }

    #[test]
    fn the_placeholder_names_the_length_and_the_xxh3_digest() {
        let text = base64(300);
        assert_eq!(
            placeholder(&text),
            Some(format!(
                "[opaque: 300 chars, xxh3 {:016x}]",
                xxhash_rust::xxh3::xxh3_64(text.as_bytes())
            ))
        );
        let again = placeholder(&base64(300));
        assert_eq!(placeholder(&text), again);
        assert_ne!(placeholder(&text), placeholder(&base64(301)));
        let shown = placeholder(&text).expect("opaque");
        assert!(shown.len() < 64, "{shown}");
        assert_eq!(placeholder(&shown), None);
    }

    #[test]
    fn only_the_opaque_strings_change_and_the_line_keeps_its_place() {
        let text = format!(
            "{{\"a\":\"{}\",\"b\":[\"short\",\"{}\"],\"c\":{{\"d\":\"{}\"}}}}\n",
            base64(300),
            base64(256),
            "kept"
        );
        let expected = format!(
            "{{\"a\":\"{}\",\"b\":[\"short\",\"{}\"],\"c\":{{\"d\":\"kept\"}}}}\n",
            placeholder(&base64(300)).expect("opaque"),
            placeholder(&base64(256)).expect("opaque"),
        );
        assert_eq!(filtered(&text), expected);
        assert!(filtered(&text).ends_with('\n'));
    }

    #[test]
    fn an_object_key_is_kept_byte_for_byte_beside_a_value_that_is_replaced() {
        let key = base64(300);
        for text in [
            format!("{{\"{key}\":\"{key}\"}}\n"),
            format!("{{\"{key}\" :\n\t\"{key}\",\"k\":[\"{key}\"]}}\n"),
        ] {
            let value = placeholder(&key).expect("opaque");
            let expected = text
                .replacen(&format!(":\"{key}\""), &format!(":\"{value}\""), 1)
                .replacen(&format!(":\n\t\"{key}\""), &format!(":\n\t\"{value}\""), 1)
                .replacen(&format!("[\"{key}\"]"), &format!("[\"{value}\"]"), 1);
            let out = filtered(&text);
            assert_eq!(out, expected, "{text}");
            assert!(out.starts_with(&format!("{{\"{key}\"")), "{out}");
        }
    }

    #[test]
    fn a_record_that_is_not_json_passes_unchanged() {
        for text in [
            format!("\"{}\" and more\n", base64(300)),
            format!("{{\"a\":\"{}\"\n", base64(300)),
            format!("text \"{}", base64(300)),
            String::from("plain markdown line\n"),
            String::new(),
        ] {
            assert!(kept(&text), "{text:?}");
        }
        assert!(!kept(&format!("\"{}\"\n", base64(300))));
    }

    #[test]
    fn what_placeholder_writes_is_recognised_and_nothing_near_it() {
        let uri = format!("data:image/png;base64,{}", base64(400));
        for text in [base64(300), uri] {
            let written = placeholder(&text).expect("opaque");
            assert!(is_placeholder(&written), "{written}");
        }
        for text in [
            "[opaque: 300 chars, xxh3 0123456789abcdef]",
            "[opaque image/png: 4096 chars, xxh3 0123456789abcdef]",
        ] {
            assert!(is_placeholder(text), "{text}");
        }
        for text in [
            "/home/me/shed",
            "[opaque]",
            "[opaque: 300 chars, xxh3 0123456789abcde]",
            "[opaque: 300 chars, xxh3 0123456789ABCDEF]",
            "[opaque: 300 chars, xxh3 0123456789abcdef] ",
            "see [opaque: 300 chars, xxh3 0123456789abcdef]",
            "[opaque: chars, xxh3 0123456789abcdef]",
            "[opaque not-media: 300 chars, xxh3 0123456789abcdef]",
            "[opaque: 300 chars, xxh3 0123456789abcdef0]",
        ] {
            assert!(!is_placeholder(text), "{text}");
        }
    }
}
