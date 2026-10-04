use unicode_normalization::UnicodeNormalization;

/// A body normalized line by line, with the physical line of every byte of `text`.
#[derive(Debug, PartialEq, Eq)]
pub struct Body {
    pub text: String,
    /// One physical line per byte of `text`.
    pub lines: Vec<usize>,
}

impl Body {
    /// The first and last physical line of the bytes `start..end` of `text`.
    /// Out-of-range bytes clamp to the last line; an empty body gives `(0, 0)`.
    pub fn span(&self, start: usize, end: usize) -> (usize, usize) {
        let last = self.lines.len().saturating_sub(1);
        let first = start.min(last);
        let end = end.max(first + 1).min(last + 1).max(1);
        (
            self.lines.get(first).copied().unwrap_or(0),
            self.lines.get(end - 1).copied().unwrap_or(0),
        )
    }
}

/// NFKC, the six entities decoded, quotes made straight, links and images reduced to their text, autolinks to their
/// target, backslash escapes, `*`, `_`, backticks and `~~` dropped, whitespace collapsed and trimmed. Case is kept.
pub fn normalize(text: &str) -> String {
    let folded: Vec<char> = decode_entities(&text.nfkc().collect::<String>())
        .chars()
        .map(straight)
        .collect();
    let chars = strip_links(&folded, '(', ')');
    let chars = strip_links(&chars, '[', ']');
    let chars = strip_autolinks(&chars);
    let chars = unescape(&chars);
    let chars = drop_marks(&chars);
    let text: String = chars.into_iter().collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The normalized lines of a body joined by single spaces, `lines[0]` being physical line `first_line`. A line that
/// normalizes to nothing adds nothing. A construct that spans lines keeps its markup, as each line stands alone.
pub fn body_form(lines: &[&str], first_line: usize) -> Body {
    let mut text = String::new();
    let mut map = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let part = normalize(line);
        if part.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
            map.push(map.last().copied().unwrap_or(first_line));
        }
        text.push_str(&part);
        map.resize(text.len(), first_line + i);
    }
    Body { text, lines: map }
}

fn straight(c: char) -> char {
    match c {
        '‘' | '’' | '‚' | '‛' | '′' => '\'',
        '“' | '”' | '„' | '‟' | '″' => '"',
        _ => c,
    }
}

/// `[text](target)` and `[text][ref]`, each optionally behind `!`, become `text`.
fn strip_links(chars: &[char], open: char, close: char) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match link_at(chars, i, open, close) {
            Some((inner, end)) => {
                out.extend_from_slice(inner);
                i = end;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn link_at(chars: &[char], i: usize, open: char, close: char) -> Option<(&[char], usize)> {
    let bracket = i + usize::from(chars[i] == '!');
    if chars.get(bracket) != Some(&'[') {
        return None;
    }
    let start = bracket + 1;
    let end = start + chars[start..].iter().position(|c| *c == ']')?;
    if chars.get(end + 1) != Some(&open) {
        return None;
    }
    let target = end + 2;
    let last = target + chars[target..].iter().position(|c| *c == close)?;
    Some((&chars[start..end], last + 1))
}

/// `<https://x>`, `<http://x>` and `<mailto:x>` become the target.
fn strip_autolinks(chars: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<'
            && let Some(end) = autolink_end(&chars[i + 1..])
        {
            out.extend_from_slice(&chars[i + 1..i + 1 + end]);
            i += end + 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The length of the target when `rest` starts with one and then a `>`.
fn autolink_end(rest: &[char]) -> Option<usize> {
    let scheme = ["https:", "http:", "mailto:"].into_iter().find(|scheme| {
        rest.iter()
            .zip(scheme.chars())
            .filter(|(a, b)| *a == b)
            .count()
            == scheme.len()
            && rest.len() >= scheme.len()
    })?;
    let body = rest[scheme.len()..]
        .iter()
        .take_while(|c| **c != '>' && !c.is_whitespace())
        .count();
    (body > 0 && rest.get(scheme.len() + body) == Some(&'>')).then_some(scheme.len() + body)
}

fn unescape(chars: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1).is_some_and(char::is_ascii_punctuation) {
            i += 1;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn drop_marks(chars: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' | '_' | '`' => i += 1,
            '~' if chars.get(i + 1) == Some(&'~') => i += 2,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn decode_entities(text: &str) -> String {
    const ENTITIES: [(&str, char); 6] = [
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
        ("&#39;", '\''),
        ("&#124;", '|'),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match ENTITIES.iter().find(|(name, _)| rest.starts_with(name)) {
            Some((name, c)) => {
                out.push(*c);
                rest = &rest[name.len()..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_and_spacing() {
        assert_eq!(normalize("the **zero  value**"), "the zero value");
        assert_eq!(normalize("  the zero value\n"), "the zero value");
    }

    #[test]
    fn an_entity_in_a_table_cell() {
        assert_eq!(normalize("a &#124; b"), "a | b");
        assert_eq!(normalize("a | b"), "a | b");
        assert_eq!(
            normalize("&amp; &lt; &gt; &quot; &#39; &#124;"),
            "& < > \" ' |"
        );
    }

    #[test]
    fn an_entity_decodes_once() {
        assert_eq!(normalize("&amp;lt;"), "&lt;");
        assert_eq!(normalize("fish & chips &copy;"), "fish & chips &copy;");
    }

    #[test]
    fn an_entity_decodes_before_markup_is_stripped() {
        assert_eq!(normalize("&lt;https://x.dev&gt;"), "https://x.dev");
    }

    #[test]
    fn case_is_not_folded() {
        assert_ne!(normalize("Option"), normalize("option"));
    }

    #[test]
    fn a_decomposed_accent_equals_a_composed_one() {
        assert_eq!(normalize("cafe\u{301}"), normalize("caf\u{e9}"));
        assert_eq!(normalize("caf\u{e9}"), "caf\u{e9}");
    }

    #[test]
    fn compatibility_forms_fold() {
        assert_eq!(normalize("a … b"), "a ... b");
        assert_eq!(normalize("\u{fb01}le \u{ff21}\u{ff42}"), "file Ab");
        assert_eq!(normalize("a\u{a0}b"), "a b");
    }

    #[test]
    fn curly_quotes_are_straight() {
        assert_eq!(normalize("“it’s” ‘x′ „y‟"), "\"it's\" 'x' \"y\"");
    }

    #[test]
    fn links_images_and_autolinks_reduce() {
        assert_eq!(
            normalize("see [the book](https://x.dev/a) now"),
            "see the book now"
        );
        assert_eq!(normalize("![alt text](a.png)"), "alt text");
        assert_eq!(normalize("[the book][ref]"), "the book");
        assert_eq!(
            normalize("<https://x.dev/a> and <mailto:a@b.c>"),
            "https://x.dev/a and mailto:a@b.c"
        );
        assert_eq!(normalize("a <b> c <https:> d"), "a <b> c <https:> d");
        assert_eq!(normalize("x [not a link] y"), "x [not a link] y");
        assert_eq!(normalize("!bang [a](b)"), "!bang a");
    }

    #[test]
    fn escapes_and_marks_drop() {
        assert_eq!(normalize(r"a\*b \_c\_ \\ \q"), r"ab c \ \q");
        assert_eq!(normalize("`Option` ~~old~~ a~b ~~~c"), "Option old a~b ~c");
        assert_eq!(normalize("snake_case_name"), "snakecasename");
    }

    #[test]
    fn body_form_maps_each_byte_to_its_line() {
        let body = body_form(&["", "## The `Option` type", "", "plain text"], 10);
        assert_eq!(body.text, "## The Option type plain text");
        assert_eq!(body.lines.len(), body.text.len());
        let at = body.text.find("Option").unwrap();
        assert_eq!(body.span(at, at + 6), (11, 11));
        let at = body.text.find("plain").unwrap();
        assert_eq!(body.span(at, body.text.len()), (13, 13));
        assert_eq!(body.span(0, body.text.len()), (11, 13));
    }

    #[test]
    fn a_link_split_over_two_lines_keeps_its_brackets_and_maps_both_lines() {
        let body = body_form(&["see [the", "book](https://x.dev) now"], 5);
        assert_eq!(body.text, "see [the book](https://x.dev) now");
        let at = body.text.find("the").unwrap();
        let end = body.text.find("book").unwrap() + 4;
        assert_eq!(body.span(at, end), (5, 6));
        let quote = normalize("the book");
        assert!(body.text.contains(&quote));
    }

    #[test]
    fn a_multibyte_line_maps_every_byte() {
        let body = body_form(&["caf\u{e9}", "b"], 1);
        assert_eq!(body.text, "caf\u{e9} b");
        assert_eq!(body.lines, [1, 1, 1, 1, 1, 1, 2]);
    }

    #[test]
    fn span_is_total() {
        let body = body_form(&["ab", "cd"], 3);
        assert_eq!(body.span(0, 99), (3, 4));
        assert_eq!(body.span(99, 120), (4, 4));
        assert_eq!(body.span(2, 2), (3, 3));
        let empty = body_form(&[""], 1);
        assert_eq!(empty.span(0, 0), (0, 0));
        assert_eq!(empty.span(5, 9), (0, 0));
    }

    #[test]
    fn an_empty_body_has_no_text() {
        let body = body_form(&["", "  ", "***"], 1);
        assert_eq!(body.text, "");
        assert!(body.lines.is_empty());
    }
}
