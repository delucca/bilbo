use crate::note::{self, Problem};
use crate::{hash, rank, text};

pub const CAPTURES: [&str; 2] = ["external", "legacy"];
pub const ORIGIN_TYPES: [&str; 2] = ["url", "doc"];
const KEYS: [&str; 6] = ["id", "fetched", "origin", "digest", "kept", "capture"];
const REQUIRED: [&str; 4] = ["id", "fetched", "origin", "digest"];
const KEPT_RULE: &str = "is not ascending, non-overlapping <a>-<b> ranges from 1";
const CATALOG_BYTES: usize = 55_000;
const CATALOG_SECTIONS: usize = 40;

/// One `<key>: <value>` frontmatter line.
pub struct Pair {
    pub key: String,
    pub line: usize,
    pub value: String,
}

/// A frontmatter split off a file's text; `read` and the guide reader both build on it.
pub struct Front {
    /// The first line of each allowed key, in file order.
    pub pairs: Vec<Pair>,
    /// Allowed keys that appeared, whatever their value.
    pub seen: Vec<String>,
    /// Byte offset in the text of the first byte after the closing `---` line's newline.
    pub body_offset: usize,
    /// Physical line the body starts on.
    pub body_start: usize,
    /// False when the frontmatter opens and never closes: the body is unknown.
    pub body_known: bool,
    pub problems: Vec<Problem>,
}

impl Front {
    pub fn get(&self, key: &str) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.key == key)
    }

    /// A `<key>: missing` problem for each of `required` that never appeared, in a file whose frontmatter closes.
    pub fn missing(&mut self, required: &[&str]) {
        if self.body_start == 1 {
            return;
        }
        for key in required {
            if !self.seen.iter().any(|s| s == key) {
                self.problems
                    .push(Problem::whole(format!("{key}: missing")));
            }
        }
    }
}

/// Splits the frontmatter of `text` into the lines of `keys`, with the `note-store` messages for delimiters,
/// unknown keys and repeats. Lines keep their `\r`, so a CRLF delimiter is no delimiter.
pub fn split_front(text: &str, keys: &[&str]) -> Front {
    let mut front = Front {
        pairs: Vec::new(),
        seen: Vec::new(),
        body_offset: 0,
        body_start: 1,
        body_known: true,
        problems: Vec::new(),
    };
    let mut from = 0;
    if text.starts_with('\u{feff}') {
        front
            .problems
            .push(Problem::at(1, "encoding: remove the byte order mark"));
        from = '\u{feff}'.len_utf8();
    }
    let mut spans: Vec<(usize, &str)> = Vec::new();
    let mut at = from;
    for piece in text[from..].split_inclusive('\n') {
        spans.push((at, piece.strip_suffix('\n').unwrap_or(piece)));
        at += piece.len();
    }
    if spans.first().map(|s| s.1) != Some("---") {
        front
            .problems
            .push(Problem::at(1, "frontmatter: missing; line 1 must be '---'"));
        return front;
    }
    let Some(close) = spans[1..].iter().position(|s| s.1 == "---") else {
        front
            .problems
            .push(Problem::at(1, "frontmatter: no closing '---' line"));
        front.body_known = false;
        return front;
    };
    let close = close + 1;
    front.body_offset = spans.get(close + 1).map_or(text.len(), |s| s.0);
    front.body_start = close + 2;

    let mut in_list = false;
    for (i, (_, line)) in spans[1..close].iter().enumerate() {
        let n = i + 2;
        if line.starts_with([' ', '-']) {
            if !in_list {
                front.problems.push(unexpected(n, keys));
            }
            continue;
        }
        in_list = false;
        let Some((key, rest)) = note::split_key(line) else {
            front.problems.push(unexpected(n, keys));
            continue;
        };
        if !keys.contains(&key) {
            front
                .problems
                .push(Problem::at(n, format!("frontmatter: unknown key '{key}'")));
            in_list = key == "sources";
            continue;
        }
        if front.seen.iter().any(|s| s == key) {
            front
                .problems
                .push(Problem::at(n, format!("{key}: given more than once")));
            continue;
        }
        front.seen.push(key.to_string());
        match rest.strip_prefix(' ') {
            Some(value) => front.pairs.push(Pair {
                key: key.to_string(),
                line: n,
                value: value.to_string(),
            }),
            None => front
                .problems
                .push(Problem::at(n, format!("{key}: expected '{key}: <value>'"))),
        }
    }
    front
}

fn unexpected(n: usize, keys: &[&str]) -> Problem {
    let expected: Vec<String> = keys.iter().map(|k| format!("'{k}: '")).collect();
    Problem::at(
        n,
        format!(
            "frontmatter: unexpected line; expected {}",
            expected.join(", ")
        ),
    )
}

/// A source's frontmatter with every value valid. `origin` holds `<type>: <value>` without its quotes and `kept`
/// the ranges as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: String,
    pub fetched: String,
    pub origin: String,
    pub digest: String,
    pub kept: Option<String>,
    pub capture: Option<String>,
}

pub struct Source {
    /// The first `id:` value when it is a canonical ULID (used for the shared-id rule).
    pub id: Option<String>,
    /// The value of each key that holds a valid one; `origin` without its quotes.
    pub fetched: Option<String>,
    pub origin: Option<String>,
    pub digest: Option<String>,
    pub kept: Option<String>,
    pub capture: Option<String>,
    /// Every allowed key the frontmatter holds, whatever its value.
    pub keys: Vec<String>,
    /// Byte offset in the text of the body's first byte.
    pub body_offset: usize,
    /// Physical line the body starts on: the title's line.
    pub body_start: usize,
    /// Every problem in line order, whole-file ones last.
    pub problems: Vec<Problem>,
}

impl Source {
    /// The body of `text`, the text `read` took this source from.
    pub fn body<'a>(&self, text: &'a str) -> &'a str {
        &text[self.body_offset..]
    }
}

/// Strict read of a whole source file's text.
pub fn read(text: &str) -> Source {
    let mut front = split_front(text, &KEYS);
    front.missing(&REQUIRED);
    let mut problems = std::mem::take(&mut front.problems);

    let valid = |key: &str, test: &dyn Fn(&str) -> Option<String>, bad: &mut Vec<Problem>| {
        let pair = front.get(key)?;
        match test(&pair.value) {
            Some(message) => {
                bad.push(Problem::at(pair.line, message));
                None
            }
            None => Some(pair.value.clone()),
        }
    };
    let id = valid(
        "id",
        &|v| (!note::is_ulid(v)).then(|| note::bad_id(v)),
        &mut problems,
    );
    let fetched = valid(
        "fetched",
        &|v| (!is_date(v)).then(|| format!("fetched: '{v}' is not YYYY-MM-DD, a real date")),
        &mut problems,
    );
    let origin = valid(
        "origin",
        &|v| {
            origin_value(v)
                .is_none()
                .then(|| "origin: write it as origin: \"<url or doc>: <value>\"".to_string())
        },
        &mut problems,
    );
    let digest = valid(
        "digest",
        &|v| {
            (!is_digest(v))
                .then(|| format!("digest: '{v}' is not sha256: and 64 lowercase hex digits"))
        },
        &mut problems,
    );
    let kept = valid(
        "kept",
        &|v| {
            parse_kept(v)
                .err()
                .map(|rule| format!("kept: '{v}' {rule}"))
        },
        &mut problems,
    );
    let capture = valid(
        "capture",
        &|v| (!CAPTURES.contains(&v)).then(|| format!("capture: '{v}' is not external or legacy")),
        &mut problems,
    );
    let digest_at = front.get("digest").map(|p| p.line);

    if front.body_known {
        let body = &text[front.body_offset..];
        if let (Some(digest), Some(line)) = (&digest, digest_at)
            && *digest != self::digest(body)
        {
            problems.push(Problem::at(
                line,
                "digest: does not match the body; only bilbo library land writes a source",
            ));
        }
        let lines = note::lines(text);
        let body_lines = lines.get(front.body_start - 1..).unwrap_or(&[]);
        problems.extend(title_problems(body_lines, front.body_start));
    }
    problems.sort_by_key(|p| p.line.unwrap_or(usize::MAX));

    let origin = origin.and_then(|o| origin_value(&o).map(str::to_string));
    Source {
        id,
        fetched,
        origin,
        digest,
        kept,
        capture,
        keys: front.seen,
        body_offset: front.body_offset,
        body_start: front.body_start,
        problems,
    }
}

/// Whether `item` is a valid origin without its quotes: `<type>: <value>`.
pub fn valid_origin(item: &str) -> bool {
    origin_value(&format!("\"{item}\"")) == Some(item)
}

/// The `<type>: <value>` inside the quotes of an origin value, when it is valid.
fn origin_value(v: &str) -> Option<&str> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains(['"', '\\']) {
        return None;
    }
    let (kind, value) = inner.split_once(": ")?;
    (ORIGIN_TYPES.contains(&kind) && !value.trim().is_empty()).then_some(inner)
}

pub fn is_date(v: &str) -> bool {
    const SHAPE: &[u8; 10] = b"dddd-dd-dd";
    let b = v.as_bytes();
    b.len() == 10
        && SHAPE.iter().zip(b).all(|(p, c)| match p {
            b'd' => c.is_ascii_digit(),
            _ => p == c,
        })
        && v.parse::<jiff::civil::Date>().is_ok()
}

fn is_digest(v: &str) -> bool {
    v.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// Indexes of the lines outside fenced code blocks, fence lines excluded.
pub fn outside_fences(lines: &[&str]) -> Vec<usize> {
    let mut fence: Option<(char, usize)> = None;
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let run = note::fence_run(line);
        match (fence, run) {
            (Some((ch, len)), Some((c, l, rest)))
                if c == ch && l >= len && rest.trim().is_empty() =>
            {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some((c, l, _))) => fence = Some((c, l)),
            (None, None) => out.push(i),
        }
    }
    out
}

/// `body` starts at physical line `first_line`.
fn title_problems(body: &[&str], first_line: usize) -> Vec<Problem> {
    let mut problems = Vec::new();
    if !body.first().is_some_and(|l| l.starts_with("# ")) {
        problems.push(Problem::at(
            first_line,
            "title: the body must open with a '# <title>' line",
        ));
    }
    // `note::title_problem` also reports a missing title; the rule above already did.
    problems.extend(note::title_problem(body, first_line).filter(|p| p.line.is_some()));
    problems
}

/// `sha256:<hex>` of a body, the bytes after the line that closes the frontmatter.
pub fn digest(body: &str) -> String {
    format!("sha256:{}", hash::sha256_hex(body.as_bytes()))
}

/// The frontmatter in the key order `id`, `fetched`, `origin`, `digest`, `kept`, `capture`, then `body`, which
/// opens with its `# <title>` line.
pub fn render(front: &Frontmatter, body: &str) -> String {
    let mut out = format!(
        "---\nid: {}\nfetched: {}\norigin: \"{}\"\ndigest: {}\n",
        front.id, front.fetched, front.origin, front.digest
    );
    if let Some(kept) = &front.kept {
        out.push_str(&format!("kept: {kept}\n"));
    }
    if let Some(capture) = &front.capture {
        out.push_str(&format!("capture: {capture}\n"));
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// A heading below the title and the lines it spans.
#[derive(Debug, PartialEq, Eq)]
pub struct Section {
    /// Physical line of the heading.
    pub start: usize,
    /// Physical line of the last line, inclusive.
    pub end: usize,
    pub level: usize,
    /// The enclosing headings below the title, then this one.
    pub path: Vec<String>,
    /// Bytes of the section's lines, each counted with its newline.
    pub bytes: usize,
    pub tokens: usize,
}

impl Section {
    pub fn path_text(&self) -> String {
        self.path.join(" > ")
    }
}

/// The sections of a file whose `lines` come from `note::lines`; its body, and so its title, starts on physical
/// line `body_start`.
pub fn outline(lines: &[&str], body_start: usize) -> Vec<Section> {
    let body = lines.get(body_start - 1..).unwrap_or(&[]);
    let mut headings: Vec<(usize, usize, String)> = outside_fences(body)
        .into_iter()
        .filter_map(|i| rank::heading(body[i]).map(|(level, text)| (i, level, text)))
        .collect();
    if headings
        .first()
        .is_some_and(|(i, level, _)| *i == 0 && *level == 1)
    {
        headings.remove(0);
    }

    let mut stack: Vec<(usize, &str)> = Vec::new();
    let mut sections = Vec::new();
    for (h, (i, level, text)) in headings.iter().enumerate() {
        while stack.last().is_some_and(|(top, _)| top >= level) {
            stack.pop();
        }
        stack.push((*level, text));
        let next = headings[h + 1..]
            .iter()
            .find(|(_, other, _)| other <= level)
            .map_or(body.len(), |(j, _, _)| *j);
        let bytes = body[*i..next].iter().map(|l| l.len() + 1).sum();
        sections.push(Section {
            start: body_start + i,
            end: body_start + next - 1,
            level: *level,
            path: stack.iter().map(|(_, t)| t.to_string()).collect(),
            bytes,
            tokens: tokens(bytes),
        });
    }
    sections
}

/// Bytes divided by 2.5, rounded up.
pub fn tokens(bytes: usize) -> usize {
    (bytes * 2).div_ceil(5)
}

/// Bytes divided by 1,000, rounded up.
pub fn kb(bytes: usize) -> usize {
    bytes.div_ceil(1000)
}

/// The cut level and the number of sections at or above it: the shallowest level from 2 to 6 with at least two.
pub fn cut(sections: &[Section]) -> Option<(usize, usize)> {
    (2..=6).find_map(|level| {
        let count = sections.iter().filter(|s| s.level <= level).count();
        (count >= 2).then_some((level, count))
    })
}

pub fn is_catalog(body_bytes: usize, sections: &[Section]) -> bool {
    body_bytes > CATALOG_BYTES && cut(sections).is_some_and(|(_, count)| count > CATALOG_SECTIONS)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
    One(usize),
    Ambiguous(Vec<usize>),
    Missing,
}

/// The sections whose heading path ends with the anchor's parts, each compared after the Text normalization, with case.
pub fn resolve(sections: &[Section], anchor: &str) -> Resolved {
    let anchor = text::normalize(anchor);
    if anchor.is_empty() {
        return Resolved::Missing;
    }
    let tail = format!(" > {anchor}");
    let found: Vec<usize> = sections
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            let path = s
                .path
                .iter()
                .map(|part| text::normalize(part))
                .collect::<Vec<_>>()
                .join(" > ");
            path == anchor || path.ends_with(&tail)
        })
        .map(|(i, _)| i)
        .collect();
    match found.as_slice() {
        [] => Resolved::Missing,
        [one] => Resolved::One(*one),
        _ => Resolved::Ambiguous(found),
    }
}

/// Inclusive 1-based line ranges from `3-120,130-130`; the error is the rule the value breaks.
pub fn parse_kept(value: &str) -> Result<Vec<(usize, usize)>, &'static str> {
    let number = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<usize>().ok())
            .flatten()
    };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for part in value.split(',') {
        let (a, b) = part
            .split_once('-')
            .and_then(|(a, b)| Some((number(a)?, number(b)?)))
            .ok_or(KEPT_RULE)?;
        if a < 1 || a > b || ranges.last().is_some_and(|(_, end)| a <= *end) {
            return Err(KEPT_RULE);
        }
        ranges.push((a, b));
    }
    Ok(ranges)
}

/// Ranges as `a-b` joined by commas, with ranges that touch merged.
pub fn format_kept(ranges: &[(usize, usize)]) -> String {
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for &(a, b) in ranges {
        match merged.last_mut() {
            Some(last) if a == last.1 + 1 => last.1 = b,
            _ => merged.push((a, b)),
        }
    }
    merged
        .iter()
        .map(|(a, b)| format!("{a}-{b}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01M3EZ8NVEC2KJQNGK5DTK349R";

    fn front(body: &str) -> Frontmatter {
        Frontmatter {
            id: ID.into(),
            fetched: "2026-08-23".into(),
            origin: "url: https://go.dev/doc/effective_go".into(),
            digest: digest(body),
            kept: None,
            capture: None,
        }
    }

    fn with_lines(extra: &[&str], body: &str) -> String {
        let mut out = format!(
            "---\nid: {ID}\nfetched: 2026-08-23\norigin: \"url: https://go.dev\"\ndigest: {}\n",
            digest(body)
        );
        for line in extra {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("---\n");
        out.push_str(body);
        out
    }

    fn messages(text: &str) -> Vec<String> {
        read(text).problems.iter().map(|p| p.to_string()).collect()
    }

    fn only(text: &str, needle: &str) {
        let found = messages(text);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains(needle), "{found:?}");
    }

    const BODY: &str = "# Effective Go\n\n## Concurrency\n\ntext\n";

    #[test]
    fn full_frontmatter_is_valid() {
        let text = with_lines(&["kept: 12-1904,1910-1950", "capture: external"], BODY);
        let read = read(&text);
        assert!(read.problems.is_empty());
        assert_eq!(read.kept.as_deref(), Some("12-1904,1910-1950"));
        assert_eq!(read.capture.as_deref(), Some("external"));
        assert_eq!(read.origin.as_deref(), Some("url: https://go.dev"));
        assert_eq!(read.digest, Some(digest(BODY)));
        assert_eq!(read.fetched.as_deref(), Some("2026-08-23"));
        assert_eq!(read.id.as_deref(), Some(ID));
        assert_eq!(read.body_start, 9);
        assert_eq!(read.body(&text), BODY);
    }

    #[test]
    fn any_key_order_is_valid() {
        let text = format!(
            "---\ndigest: {}\norigin: \"doc: A book\"\nfetched: 2026-08-23\nid: {ID}\n---\n{BODY}",
            digest(BODY)
        );
        assert!(messages(&text).is_empty());
    }

    #[test]
    fn note_keys_are_not_source_keys() {
        for key in [
            "created: 2026-09-26T10:12-03:00",
            "scope: go",
            "sources:\n  - \"url: https://go.dev\"",
        ] {
            let name = key.split(':').next().unwrap();
            let found = messages(&with_lines(&[key], BODY));
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(
                found[0].contains(&format!("unknown key '{name}'")),
                "{found:?}"
            );
        }
    }

    #[test]
    fn a_missing_key_is_named() {
        let text = format!(
            "---\nid: {ID}\nfetched: 2026-08-23\norigin: \"url: https://go.dev\"\n---\n{BODY}"
        );
        let read = read(&text);
        assert_eq!(read.problems.len(), 1);
        assert_eq!(read.problems[0].to_string(), "digest: missing");
    }

    #[test]
    fn repeated_keys_and_bad_ids() {
        only(
            &with_lines(&["capture: legacy", "capture: legacy"], BODY),
            "capture: given more than once",
        );
        let text = with_lines(&[], BODY).replace(ID, "01m3ez8nvec2kjqngk5dtk349r");
        let found = messages(&text);
        assert!(found[0].starts_with("id: '01m3"), "{found:?}");
    }

    #[test]
    fn delimiters() {
        assert_eq!(
            messages("# T\n"),
            ["frontmatter: missing; line 1 must be '---' (line 1)"]
        );
        assert_eq!(
            messages("---\nid: x\n"),
            ["frontmatter: no closing '---' line (line 1)"]
        );
        let crlf = with_lines(&[], BODY).replace('\n', "\r\n");
        assert!(messages(&crlf)[0].starts_with("frontmatter: missing"));
    }

    #[test]
    fn fetched_and_origin() {
        assert!(
            messages(&with_lines(&[], BODY).replace(
                "url: https://go.dev",
                "doc: The Go Programming Language, chapter 8"
            ))
            .is_empty()
        );
        for bad in [
            "2026-02-30",
            "2026-08-23T10:00-03:00",
            "yesterday",
            "2026-8-3",
        ] {
            let text = with_lines(&[], BODY).replace("2026-08-23", bad);
            only(&text, &format!("fetched: '{bad}' is not YYYY-MM-DD"));
        }
        for bad in [
            "https://go.dev",
            "\"code: src/main.rs\"",
            "\"url: \"",
            "\"url: a\\\\b\"",
            "\"url: a\"b\"",
        ] {
            let text = with_lines(&[], BODY).replace("\"url: https://go.dev\"", bad);
            only(&text, "origin: write it as");
        }
    }

    #[test]
    fn each_valid_value_is_kept_beside_the_invalid_ones() {
        let text =
            with_lines(&["kept: 3-5", "capture: legacy"], BODY).replace("2026-08-23", "yesterday");
        let read = read(&text);
        assert_eq!(read.fetched, None);
        assert_eq!(read.origin.as_deref(), Some("url: https://go.dev"));
        assert_eq!(read.digest, Some(digest(BODY)));
        assert_eq!(read.kept.as_deref(), Some("3-5"));
        assert_eq!(read.capture.as_deref(), Some("legacy"));
        assert!(read.keys.iter().any(|k| k == "fetched"));
    }

    #[test]
    fn origin_items() {
        assert!(valid_origin("url: https://go.dev"));
        assert!(valid_origin("doc: The Go Programming Language, chapter 8"));
        for bad in [
            "web: https://go.dev",
            "url: ",
            "https://go.dev",
            "url: a\"b",
            "url: a\\b",
        ] {
            assert!(!valid_origin(bad), "{bad}");
        }
    }

    #[test]
    fn kept_ranges() {
        for good in ["3-120,130-130,140-200", "1-1", "3-5,6-8"] {
            assert!(
                messages(&with_lines(&[&format!("kept: {good}")], BODY)).is_empty(),
                "{good}"
            );
        }
        for bad in [
            "130-200,3-120",
            "3-120,100-200",
            "0-10",
            "20-10",
            "",
            "3",
            "3-",
            "a-b",
            "3-5,",
            "+3-5",
            "3-5, 7-9",
        ] {
            only(
                &with_lines(&[&format!("kept: {bad}")], BODY),
                &format!("kept: '{bad}' is not ascending"),
            );
        }
    }

    #[test]
    fn capture_labels() {
        for good in ["external", "legacy"] {
            assert!(messages(&with_lines(&[&format!("capture: {good}")], BODY)).is_empty());
        }
        only(
            &with_lines(&["capture: webfetch"], BODY),
            "capture: 'webfetch' is not external or legacy",
        );
    }

    #[test]
    fn digest_shape_and_match() {
        let text = with_lines(&[], BODY);
        let digest_line = format!("digest: {}", digest(BODY));
        for bad in [
            "digest: abc",
            &format!("digest: {}", digest(BODY).to_uppercase()),
            &format!("digest: {}", &digest(BODY)[7..]),
        ] {
            only(&text.replace(&digest_line, bad), "digest: '");
        }
        only(
            &text.replace("text", "tent"),
            "digest: does not match the body",
        );
        only(&format!("{text}\n"), "digest: does not match the body");
        assert!(messages(&text).is_empty());
    }

    #[test]
    fn digest_is_sha256_of_the_body() {
        assert_eq!(
            digest(""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest("abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn titles() {
        let fenced = "# Effective Go\n\n## A\n\n```sh\n# comment\n```\n";
        assert!(messages(&with_lines(&[], fenced)).is_empty());
        for body in [
            "\n# Effective Go\n",
            "intro\n\n# Effective Go\n",
            "",
            "## Only a section\n",
        ] {
            let found = messages(&with_lines(&[], body));
            assert!(
                found
                    .iter()
                    .any(|m| m.starts_with("title: the body must open with")),
                "{body:?} {found:?}"
            );
        }
        let two = messages(&with_lines(&[], "# A\n\n# B\n"));
        assert_eq!(
            two,
            ["title: found 2 '# ' headings outside code fences, expected one (line 9)"]
        );
    }

    #[test]
    fn problems_come_in_line_order() {
        let text = with_lines(&["capture: x", "kept: 0-1"], "# A\n\n# B\n");
        let found = messages(&text);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(
            found[0].ends_with("(line 6)") && found[1].ends_with("(line 7)"),
            "{found:?}"
        );
    }

    #[test]
    fn render_writes_keys_in_contract_order() {
        let mut full = front(BODY);
        full.kept = Some("3-9".into());
        full.capture = Some("legacy".into());
        let text = render(&full, BODY);
        assert_eq!(
            text,
            format!(
                "---\nid: {ID}\nfetched: 2026-08-23\norigin: \"url: https://go.dev/doc/effective_go\"\ndigest: {}\nkept: 3-9\ncapture: legacy\n---\n{BODY}",
                digest(BODY)
            )
        );
        let back = read(&text);
        assert_eq!(back.id.as_deref(), Some(ID));
        assert_eq!(back.kept, full.kept);
        assert_eq!(back.capture, full.capture);
        assert_eq!(back.digest, Some(full.digest));
        let bare = render(&front(BODY), BODY);
        assert!(!bare.contains("kept") && !bare.contains("capture"));
        assert!(read(&bare).problems.is_empty());
    }

    fn outline_of(body: &str) -> Vec<Section> {
        let text = with_lines(&[], body);
        let read = read(&text);
        outline(&note::lines(&text), read.body_start)
    }

    fn summary(sections: &[Section]) -> Vec<(usize, usize, String)> {
        sections
            .iter()
            .map(|s| (s.start, s.end, s.path_text()))
            .collect()
    }

    #[test]
    fn nested_heading_path() {
        let sections = outline_of(
            "# Effective Go\n\n## Concurrency\n\nx\n\n### Goroutines\n\ny\n\n## Errors\n\nz\n",
        );
        assert_eq!(
            summary(&sections),
            [
                (9, 16, "Concurrency".to_string()),
                (13, 16, "Concurrency > Goroutines".to_string()),
                (17, 19, "Errors".to_string()),
            ]
        );
        assert_eq!(sections[0].level, 2);
        assert_eq!(sections[1].level, 3);
    }

    #[test]
    fn section_bytes_count_each_newline() {
        let sections = outline_of("# T\n\n## A\n\nxy\n");
        assert_eq!(sections[0].bytes, "## A\n\nxy\n".len());
        assert_eq!(sections[0].tokens, tokens(sections[0].bytes));
    }

    #[test]
    fn fenced_heading_opens_no_section() {
        let sections = outline_of("# T\n\n```\n## not a heading\n```\n\n## Real\n");
        assert_eq!(summary(&sections).len(), 1);
        assert_eq!(sections[0].path_text(), "Real");
    }

    #[test]
    fn headingless_source_has_no_sections() {
        assert!(outline_of("# T\n\nparagraph\n\nmore\n").is_empty());
    }

    #[test]
    fn seven_hashes_are_plain_text_and_six_are_a_section() {
        let sections = outline_of("# T\n\n####### seven\n\n###### six\n");
        assert_eq!(summary(&sections).len(), 1);
        assert_eq!(sections[0].path_text(), "six");
        assert_eq!(sections[0].level, 6);
    }

    #[test]
    fn a_skipped_level_nests_under_the_last_shallower_heading() {
        let sections = outline_of("# T\n\n## A\n\n#### Deep\n\n## B\n");
        assert_eq!(sections[1].path_text(), "A > Deep");
    }

    #[test]
    fn sizes_round_up() {
        assert_eq!((tokens(1000), kb(1000)), (400, 1));
        assert_eq!((tokens(1001), kb(1001)), (401, 2));
        assert_eq!((tokens(0), kb(0)), (0, 0));
        assert_eq!(tokens(96211), 38485);
        let corpus = 1001 + 1001;
        assert_eq!((tokens(1001) * 2, kb(corpus)), (802, 3));
        assert_eq!(tokens(1001) + tokens(1001), 802);
    }

    fn sections_at(levels: &[(usize, usize)]) -> Vec<Section> {
        levels
            .iter()
            .flat_map(|(level, n)| std::iter::repeat_n(*level, *n))
            .map(|level| Section {
                start: 1,
                end: 1,
                level,
                path: vec![],
                bytes: 0,
                tokens: 0,
            })
            .collect()
    }

    #[test]
    fn a_lint_catalog() {
        let sections = sections_at(&[(2, 849), (3, 2000)]);
        assert_eq!(cut(&sections), Some((2, 849)));
        assert!(is_catalog(891_000, &sections));
    }

    #[test]
    fn a_book_is_not_a_catalog() {
        let sections = sections_at(&[(2, 16), (3, 43)]);
        assert_eq!(cut(&sections), Some((2, 16)));
        assert!(!is_catalog(102_000, &sections));
    }

    #[test]
    fn one_chapter_over_many_entries() {
        let sections = sections_at(&[(2, 1), (3, 60)]);
        assert_eq!(cut(&sections), Some((3, 61)));
        assert!(is_catalog(80_000, &sections));
    }

    #[test]
    fn a_short_source_with_many_sections_is_not_a_catalog() {
        let sections = sections_at(&[(2, 300)]);
        assert!(!is_catalog(30_000, &sections));
        assert!(!is_catalog(55_000, &sections));
        assert!(is_catalog(55_001, &sections));
        assert!(!is_catalog(80_000, &sections_at(&[(2, 40)])));
        assert!(is_catalog(80_000, &sections_at(&[(2, 41)])));
    }

    #[test]
    fn no_cut_level_without_two_sections() {
        assert_eq!(cut(&[]), None);
        assert_eq!(cut(&sections_at(&[(2, 1)])), None);
        assert!(!is_catalog(80_000, &sections_at(&[(2, 1)])));
    }

    fn lint_sections() -> Vec<Section> {
        let body = "# Lints\n\n## needless_return\n\n### What it does\n\n## needless_range_loop\n\n### What it does\n";
        outline_of(body)
    }

    #[test]
    fn a_trailing_part_resolves() {
        let sections = lint_sections();
        assert_eq!(
            resolve(&sections, "needless_return > What it does"),
            Resolved::One(1)
        );
        assert_eq!(resolve(&sections, "needless_range_loop"), Resolved::One(2));
        assert_eq!(
            resolve(&sections, "needless_return  >   What   it does"),
            Resolved::One(1)
        );
    }

    #[test]
    fn a_bare_heading_that_repeats_is_ambiguous() {
        assert_eq!(
            resolve(&lint_sections(), "What it does"),
            Resolved::Ambiguous(vec![1, 3])
        );
    }

    #[test]
    fn case_counts() {
        let sections = lint_sections();
        assert_eq!(resolve(&sections, "what it does"), Resolved::Missing);
        assert_eq!(resolve(&sections, ""), Resolved::Missing);
        assert_eq!(resolve(&sections, "it does"), Resolved::Missing);
    }

    #[test]
    fn markup_in_a_heading_resolves_through_a_plain_anchor() {
        let sections =
            outline_of("# Book\n\n## The `Option` type\n\n### A **bold** [link](x)\n\n## Other\n");
        assert_eq!(resolve(&sections, "The Option type"), Resolved::One(0));
        assert_eq!(resolve(&sections, "The `Option` type"), Resolved::One(0));
        assert_eq!(
            resolve(&sections, "The Option type > A bold link"),
            Resolved::One(1)
        );
        assert_eq!(resolve(&sections, "The Options type"), Resolved::Missing);
        assert_eq!(resolve(&sections, "the option type"), Resolved::Missing);
        assert_eq!(resolve(&sections, "**"), Resolved::Missing);
    }

    #[test]
    fn kept_parses_and_merges_touching_ranges() {
        assert_eq!(parse_kept("6-400,420-900"), Ok(vec![(6, 400), (420, 900)]));
        assert_eq!(format_kept(&[(6, 400), (420, 900)]), "6-400,420-900");
        assert_eq!(format_kept(&[(6, 400), (401, 900)]), "6-900");
        assert_eq!(
            format_kept(&[(1, 2), (3, 3), (4, 9), (11, 12)]),
            "1-9,11-12"
        );
        assert!(parse_kept("99999999999999999999999-1").is_err());
    }
}
