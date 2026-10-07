//! Citations: parsing `bilbo:` citations and checking them against notes and sources, and the
//! `cite` verb.

pub mod cite;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::shared::frontmatter;
use crate::shared::markdown::{self, Resolved, Section};
use crate::shared::store::{self, EntryKind};
use crate::shared::text::{self, Body};

const PREFIX: &str = "bilbo:";
const ID_LEN: usize = 26;
const MIN_WORDS: usize = 6;
const HINT_CHARS: usize = 300;
const LISTED_PATHS: usize = 5;

/// A `bilbo:<id>[#<anchor>] "<quote>"` in a draft. `quote` is as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    pub line: usize,
    pub id: String,
    pub anchor: Option<String>,
    pub quote: String,
}

/// Text that looks like a citation and is not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Every citation of `text` in order, and a notice for each `bilbo:<id>` with no quote, in line order.
pub fn parse(text: &str) -> (Vec<Citation>, Vec<Notice>) {
    let mut citations = Vec::new();
    let mut notices = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find(PREFIX) {
        let start = at + found;
        let id_at = start + PREFIX.len();
        match scan(text, id_at) {
            Scan::Cited { anchor, quote, end } => {
                let line = line_of(text, start);
                citations.push(Citation {
                    line,
                    id: text[id_at..id_at + ID_LEN].to_string(),
                    anchor,
                    quote,
                });
                at = end;
            }
            Scan::NoQuote => {
                notices.push(Notice {
                    line: line_of(text, start),
                    message: format!(
                        "the citation {PREFIX}{} has no quote and was not checked",
                        &text[id_at..id_at + ID_LEN]
                    ),
                });
                at = id_at + ID_LEN;
            }
            Scan::Skip => at = id_at,
        }
    }
    (citations, notices)
}

enum Scan {
    Cited {
        anchor: Option<String>,
        quote: String,
        end: usize,
    },
    /// An id with no quote after it.
    NoQuote,
    /// Not an id at all.
    Skip,
}

fn scan(text: &str, id_at: usize) -> Scan {
    let run = text[id_at..]
        .bytes()
        .take_while(u8::is_ascii_alphanumeric)
        .count();
    if run != ID_LEN {
        return Scan::Skip;
    }
    let after = id_at + ID_LEN;
    let (anchor, open) = match text[after..].chars().next() {
        Some('#') => {
            let from = after + 1;
            let line_end = text[from..].find('\n').map_or(text.len(), |n| from + n);
            let Some(open) = text[from..line_end].find(['"', '“']).map(|n| from + n) else {
                return Scan::NoQuote;
            };
            let head = text[from..open].trim_end_matches([' ', '\t']);
            if head.len() == open - from {
                return Scan::NoQuote;
            }
            (Some(head.to_string()), open)
        }
        Some(' ' | '\t') => {
            let rest = text[after..].trim_start_matches([' ', '\t']);
            (None, text.len() - rest.len())
        }
        _ => return Scan::NoQuote,
    };
    let quote = if text[open..].starts_with('"') {
        straight_quote(text, open + 1)
    } else if text[open..].starts_with('“') {
        curly_quote(text, open + '“'.len_utf8())
    } else {
        None
    };
    match quote {
        Some((quote, end)) => Scan::Cited {
            anchor,
            quote: quote.to_string(),
            end,
        },
        None => Scan::NoQuote,
    }
}

/// The text up to the first `"` that is not followed by a letter or digit, with inner quotes in pairs; the quote
/// spans no blank line and holds at least one character.
fn straight_quote(text: &str, body: usize) -> Option<(&str, usize)> {
    let mut i = body;
    while let Some(c) = text[i..].chars().next() {
        if c == '"' {
            let next = text[i + 1..].chars().next();
            if i > body && !next.is_some_and(char::is_alphanumeric) {
                return Some((&text[body..i], i + 1));
            }
            let mut j = i + 1;
            loop {
                let inner = text[j..].chars().next()?;
                if inner == '"' {
                    break;
                }
                if blank_line_at(&text[j..]) {
                    return None;
                }
                j += inner.len_utf8();
            }
            i = j + 1;
            continue;
        }
        if blank_line_at(&text[i..]) {
            return None;
        }
        i += c.len_utf8();
    }
    None
}

/// The text up to the first `”`, which is not followed by a letter or digit; no blank line, at least one character.
fn curly_quote(text: &str, body: usize) -> Option<(&str, usize)> {
    for (k, c) in text[body..].char_indices() {
        let at = body + k;
        if c == '”' {
            let after = at + c.len_utf8();
            let next = text[after..].chars().next();
            return (k > 0 && !next.is_some_and(char::is_alphanumeric))
                .then(|| (&text[body..at], after));
        }
        if blank_line_at(&text[at..]) {
            return None;
        }
    }
    None
}

/// Whether `s` starts with a newline, spaces, tabs or carriage returns, and another newline.
fn blank_line_at(s: &str) -> bool {
    s.strip_prefix('\n')
        .is_some_and(|rest| rest.trim_start_matches([' ', '\t', '\r']).starts_with('\n'))
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].bytes().filter(|b| *b == b'\n').count() + 1
}

/// What a file's id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Note,
    Guide,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: Kind,
    pub path: PathBuf,
    /// The note's file stem, or `<corpus>/<name>` (`<corpus>/guide` for a guide).
    pub name: String,
}

/// The id of every note, guide and source of a store.
pub struct Ids {
    found: HashMap<String, Vec<Target>>,
}

impl Ids {
    /// Reads the frontmatter of every `.md` file in `<root>/notes/` and in each folder of `<root>/library/`, whatever
    /// its name. A file that cannot be read, is not UTF-8 or has no valid id adds nothing; hidden entries are absent.
    pub fn scan(root: &Path) -> Ids {
        let mut ids = Ids {
            found: HashMap::new(),
        };
        if let Ok(entries) = store::entries(&root.join("notes")) {
            for entry in entries {
                if let Some(stem) = md_stem(&entry) {
                    ids.add(Kind::Note, entry.path, stem);
                }
            }
        }
        let folders = store::entries(&store::library_dir(root)).unwrap_or_default();
        for folder in folders.into_iter().filter(|e| e.kind == EntryKind::Folder) {
            let Ok(entries) = store::entries(&folder.path) else {
                continue;
            };
            for entry in entries {
                let Some(stem) = md_stem(&entry) else {
                    continue;
                };
                let kind = if stem == "guide" {
                    Kind::Guide
                } else {
                    Kind::Source
                };
                let name = format!("{}/{stem}", folder.name);
                ids.add(kind, entry.path, name);
            }
        }
        ids
    }

    fn add(&mut self, kind: Kind, path: PathBuf, name: String) {
        let Some(id) = front_text(&path).and_then(|text| front_id(&text)) else {
            return;
        };
        self.found
            .entry(id)
            .or_default()
            .push(Target { kind, path, name });
    }

    /// The file with this id; the message says why there is none: no file has it, or several do.
    pub fn resolve(&self, id: &str) -> Result<&Target, String> {
        match self.found.get(id).map(Vec::as_slice) {
            None | Some([]) => Err("no note or source has this id".to_string()),
            Some([one]) => Ok(one),
            Some(many) => {
                let mut paths: Vec<_> = many.iter().map(|t| t.path.display().to_string()).collect();
                paths.sort();
                Err(format!(
                    "{} files share this id: {}",
                    many.len(),
                    paths.join(", ")
                ))
            }
        }
    }
}

fn md_stem(entry: &store::Entry) -> Option<String> {
    if entry.kind != EntryKind::File {
        return None;
    }
    entry.name.strip_suffix(".md").map(str::to_string)
}

/// The frontmatter of the file at `path`, read line by line up to its closing `---`: a source is never read whole to
/// learn its id. `None` when the file cannot be read, is not UTF-8 or has no opening `---`.
fn front_text(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    const MAX_BYTES: usize = 64 * 1024;
    let mut reader = BufReader::new(std::fs::File::open(path).ok()?);
    let mut text = Vec::new();
    let mut first = true;
    while text.len() < MAX_BYTES {
        let start = text.len();
        if reader.read_until(b'\n', &mut text).ok()? == 0 {
            break;
        }
        let line = &text[start..];
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let line = if first {
            line.strip_prefix("\u{feff}".as_bytes()).unwrap_or(line)
        } else {
            line
        };
        match (first, line == b"---") {
            (true, false) => return None,
            (false, true) => break,
            _ => {}
        }
        first = false;
    }
    String::from_utf8(text).ok()
}

/// The first `id:` value of the frontmatter when it is a canonical ULID.
fn front_id(text: &str) -> Option<String> {
    let lines = markdown::lines(text);
    if lines.first() != Some(&"---") {
        return None;
    }
    let close = lines[1..].iter().position(|l| *l == "---")?;
    let (_, value) = lines[1..=close]
        .iter()
        .filter_map(|l| frontmatter::split_key(l))
        .find(|(key, _)| *key == "id")?;
    let value = value.strip_prefix(' ')?;
    frontmatter::is_ulid(value).then(|| value.to_string())
}

/// The body of a file's text, after its frontmatter, and the physical line it starts on. A file with no closed
/// frontmatter is all body, from line 1.
pub fn body_of(text: &str) -> (&str, usize) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut offset = 0;
    for (i, piece) in text.split_inclusive('\n').enumerate() {
        let line = piece.strip_suffix('\n').unwrap_or(piece);
        let line = line.strip_suffix('\r').unwrap_or(line);
        match (i, line) {
            (0, "---") => {}
            (0, _) => break,
            (_, "---") => return (&text[offset + piece.len()..], i + 2),
            _ => {}
        }
        offset += piece.len();
    }
    (text, 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    QuoteElsewhere,
    Ambiguous,
    QuoteMissing,
    AnchorMissing,
    IdMissing,
    TooShort,
    Unread,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::QuoteElsewhere => "quote_elsewhere",
            Verdict::Ambiguous => "ambiguous",
            Verdict::QuoteMissing => "quote_missing",
            Verdict::AnchorMissing => "anchor_missing",
            Verdict::IdMissing => "id_missing",
            Verdict::TooShort => "too_short",
            Verdict::Unread => "unread",
        }
    }

    /// The verdicts that make `cite` exit 1; the others are warnings or pass.
    pub fn fails(self) -> bool {
        matches!(
            self,
            Verdict::QuoteMissing | Verdict::AnchorMissing | Verdict::IdMissing | Verdict::Unread
        )
    }
}

/// A verdict, its one-line detail, and the physical lines (first, last) of every match of the quote in the whole
/// body, whatever section the verdict was decided on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub verdict: Verdict,
    pub detail: String,
    pub spans: Vec<(usize, usize)>,
}

impl Outcome {
    pub fn id_missing(detail: String) -> Outcome {
        Outcome::new(Verdict::IdMissing, detail, Vec::new())
    }

    fn new(verdict: Verdict, detail: String, spans: Vec<(usize, usize)>) -> Outcome {
        Outcome {
            verdict,
            detail: one_line(&detail),
            spans,
        }
    }
}

/// A body prepared for any number of checks: normalized once, with its sections in physical lines.
pub struct Document {
    body: Body,
    /// The words of `body`, in the form a hint shows.
    readable: Vec<String>,
    sections: Vec<Section>,
}

impl Document {
    /// `body` is the text after the frontmatter, whose first line is physical line `first_line`.
    pub fn new(body: &str, first_line: usize) -> Document {
        let lines = markdown::lines(body);
        let blank = lines.iter().take_while(|l| l.trim().is_empty()).count();
        let mut sections = markdown::outline(&lines, blank + 1);
        for s in &mut sections {
            s.start += first_line - 1;
            s.end += first_line - 1;
        }
        let readable = lines
            .iter()
            .filter(|line| !text::normalize(line).is_empty())
            .flat_map(|line| {
                text::readable(line)
                    .split(' ')
                    .filter(|w| !w.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        Document {
            body: text::body_form(&lines, first_line),
            readable,
            sections,
        }
    }

    /// The verdict among `ok`, `quote_elsewhere`, `ambiguous`, `quote_missing`, `anchor_missing` and `too_short`.
    pub fn check(&self, citation: &Citation) -> Outcome {
        let frags = fragments(&citation.quote);
        let all = self.matches(&frags, 0, self.body.text.len());
        let resolved = citation
            .anchor
            .as_deref()
            .map(|anchor| markdown::resolve(&self.sections, anchor));
        let mut hint_over = None;
        let outcome = match resolved {
            Some(Resolved::Missing) => {
                let anchor = citation.anchor.as_deref().unwrap_or_default();
                let mut detail = format!("no section matches '{anchor}'");
                if !all.is_empty() {
                    detail.push_str(&format!("; the quote is {}", self.where_is(&frags, &all)));
                }
                return Outcome::new(Verdict::AnchorMissing, detail, all);
            }
            Some(Resolved::One(k)) => {
                let spans = self.in_section(k, &frags);
                hint_over = Some(k);
                if spans.is_empty() {
                    None
                } else {
                    Some(Outcome::new(Verdict::Ok, lines_detail(&spans), spans))
                }
            }
            Some(Resolved::Ambiguous(ks)) => {
                let holding: Vec<usize> = ks
                    .iter()
                    .copied()
                    .filter(|k| !self.in_section(*k, &frags).is_empty())
                    .collect();
                if holding.is_empty() {
                    None
                } else {
                    let spans = holding
                        .iter()
                        .flat_map(|k| self.in_section(*k, &frags))
                        .collect();
                    let detail = format!(
                        "{} sections match; the quote is under {}",
                        ks.len(),
                        self.paths(&holding)
                    );
                    Some(Outcome::new(Verdict::Ambiguous, detail, spans))
                }
            }
            None => {
                let end = self.sections.first().map_or(usize::MAX, |s| s.start - 1);
                let (a, b) = self.byte_range(0, end);
                let spans = self.matches(&frags, a, b);
                if spans.is_empty() {
                    None
                } else {
                    Some(Outcome::new(Verdict::Ok, lines_detail(&spans), spans))
                }
            }
        };
        match outcome {
            Some(found)
                if !matches!(found.verdict, Verdict::Ok | Verdict::Ambiguous) || !short(&frags) =>
            {
                Outcome {
                    spans: all,
                    ..found
                }
            }
            Some(_) => {
                let words = frags.join(" ").split_whitespace().count();
                let unit = if words == 1 { "word" } else { "words" };
                let detail = format!("{words} {unit}; quote at least {MIN_WORDS}");
                Outcome::new(Verdict::TooShort, detail, all)
            }
            None if !all.is_empty() => {
                let detail = format!("the quote is {}", self.where_is(&frags, &all));
                Outcome::new(Verdict::QuoteElsewhere, detail, all)
            }
            None => {
                let detail = if frags.is_empty() {
                    "the quote holds no text".to_string()
                } else {
                    let (from, to) = match hint_over {
                        Some(k) => self.section_bytes(k),
                        None => (0, self.body.text.len()),
                    };
                    match nearest(&self.body.text[from..to], &frags.join(" ")) {
                        Some(window) => format!("nearest passage: {}", self.hint(from, window)),
                        None => "the quote is not in the text".to_string(),
                    }
                };
                Outcome::new(Verdict::QuoteMissing, detail, Vec::new())
            }
        }
    }

    /// The words of a `nearest` window as the source writes them, cut to `HINT_CHARS` characters. `from` is the
    /// byte of the body where the searched text starts.
    fn hint(&self, from: usize, (start, len): (usize, usize)) -> String {
        let words: Vec<&str> = self
            .body
            .text
            .split(' ')
            .filter(|w| !w.is_empty())
            .collect();
        let offset = self.body.text[..from]
            .split(' ')
            .filter(|w| !w.is_empty())
            .count()
            + start;
        let shown: Vec<&str> = if self.readable.len() == words.len() {
            self.readable[offset..offset + len]
                .iter()
                .map(String::as_str)
                .collect()
        } else {
            words[offset..offset + len].to_vec()
        };
        shown.join(" ").chars().take(HINT_CHARS).collect()
    }

    /// Where a quote found outside its anchor sits: under the deepest sections that hold it, before the first
    /// heading, or across sections.
    fn where_is(&self, frags: &[String], all: &[(usize, usize)]) -> String {
        let holding: Vec<usize> = (0..self.sections.len())
            .filter(|k| !self.in_section(*k, frags).is_empty())
            .collect();
        let deepest: Vec<usize> = holding
            .iter()
            .copied()
            .filter(|k| {
                let s = &self.sections[*k];
                !holding.iter().any(|o| {
                    let o = &self.sections[*o];
                    o.start > s.start && o.end <= s.end
                })
            })
            .collect();
        if !deepest.is_empty() {
            return format!("under {}", self.paths(&deepest));
        }
        let first = self.sections.first().map_or(usize::MAX, |s| s.start);
        if all.iter().any(|(_, last)| *last < first) {
            "in the title or the text before the first heading".to_string()
        } else {
            "across sections".to_string()
        }
    }

    fn paths(&self, sections: &[usize]) -> String {
        let mut names: Vec<String> = sections
            .iter()
            .take(LISTED_PATHS)
            .map(|k| self.sections[*k].path_text())
            .collect();
        if sections.len() > LISTED_PATHS {
            names.push(format!("and {} more", sections.len() - LISTED_PATHS));
        }
        names.join("; ")
    }

    /// The bytes of the normalized body whose physical line is in `after + 1..=upto`.
    fn byte_range(&self, after: usize, upto: usize) -> (usize, usize) {
        let lines = &self.body.lines;
        (
            lines.partition_point(|l| *l <= after),
            lines.partition_point(|l| *l <= upto),
        )
    }

    fn section_bytes(&self, k: usize) -> (usize, usize) {
        let s = &self.sections[k];
        self.byte_range(s.start, s.end)
    }

    fn in_section(&self, k: usize, frags: &[String]) -> Vec<(usize, usize)> {
        let (a, b) = self.section_bytes(k);
        self.matches(frags, a, b)
    }

    /// The physical line spans of every match of `frags`, in order, within the bytes `from..to` of the body.
    fn matches(&self, frags: &[String], from: usize, to: usize) -> Vec<(usize, usize)> {
        let hay = &self.body.text[from..to];
        let Some((first, rest)) = frags.split_first() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(found) = hay[at..].find(first.as_str()) {
            let start = at + found;
            let mut end = start + first.len();
            for frag in rest {
                match hay[end..].find(frag.as_str()) {
                    Some(j) => end += j + frag.len(),
                    None => return out,
                }
            }
            out.push(self.body.span(from + start, from + end));
            at = start + hay[start..].chars().next().map_or(1, char::len_utf8);
        }
        out
    }
}

/// `Document::new(body, first_line).check(citation)`.
#[cfg(test)]
fn check(citation: &Citation, body: &str, first_line: usize) -> Outcome {
    Document::new(body, first_line).check(citation)
}

/// A quote's Text normalization with its line-number prefixes dropped, split on each run of three or more dots into
/// the fragments that must appear in order.
pub fn fragments(quote: &str) -> Vec<String> {
    let unprefixed: Vec<&str> = quote.split('\n').map(drop_prefix).collect();
    let folded = text::normalize(&unprefixed.join("\n"));
    let mut out = Vec::new();
    let mut part = String::new();
    let mut chars = folded.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '.' {
            let mut run = 1;
            while chars.peek() == Some(&'.') {
                chars.next();
                run += 1;
            }
            if run >= 3 {
                push_fragment(&mut out, &mut part);
            } else {
                part.extend(std::iter::repeat_n('.', run));
            }
        } else {
            part.push(c);
        }
    }
    push_fragment(&mut out, &mut part);
    out
}

fn push_fragment(out: &mut Vec<String>, part: &mut String) {
    let trimmed = part.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    part.clear();
}

/// `line` without a leading `<spaces><digits>` and a tab or `→`.
fn drop_prefix(line: &str) -> &str {
    let rest = line.trim_start_matches([' ', '\t']);
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return line;
    }
    let rest = &rest[digits..];
    rest.strip_prefix(['\t', '→']).unwrap_or(line)
}

fn short(frags: &[String]) -> bool {
    frags.join(" ").split_whitespace().count() < MIN_WORDS
}

fn lines_detail(spans: &[(usize, usize)]) -> String {
    let (a, b) = spans[0];
    format!("lines {a}-{b}")
}

/// A row's detail is one line: control characters become spaces.
fn one_line(detail: &str) -> String {
    detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The window of as many words as the quote has, with the most distinct quote words, first best, as its first word
/// and its length in the words of `hay`; `None` when no window shares a word.
fn nearest(hay: &str, quote: &str) -> Option<(usize, usize)> {
    let words: Vec<&str> = hay.split(' ').filter(|w| !w.is_empty()).collect();
    let wanted: Vec<&str> = quote.split(' ').filter(|w| !w.is_empty()).collect();
    let n = wanted.len().min(words.len());
    if n == 0 {
        return None;
    }
    let wanted: HashSet<&str> = wanted.into_iter().collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut shared = 0;
    let mut best = (0, 0);
    for (i, word) in words.iter().enumerate() {
        if wanted.contains(word) {
            let c = counts.entry(word).or_insert(0);
            shared += usize::from(*c == 0);
            *c += 1;
        }
        if i >= n {
            let old = words[i - n];
            if let Some(c) = counts.get_mut(old) {
                *c -= 1;
                shared -= usize::from(*c == 0);
            }
        }
        if i + 1 >= n && shared > best.0 {
            best = (shared, i + 1 - n);
        }
    }
    (best.0 > 0).then_some((best.1, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SID: &str = "01M3EZ8NVEC2KJQNGK5DTK349R";
    const GID: &str = "01M3EZ8NBEVNHZRTQ6T60171J2";
    const NID: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";

    const BODY: &str = "\
# Error Handling in Rust

Opening words that sit under the title before any section begins here.

## Unwrap

Calling **unwrap** on an `Option` panics when the value is `None`, which is
fine for prototypes but a poor choice in library code.

See [the book](https://doc.rust-lang.org/book/) for “unwrap_or” and friends.

```sh
# not a heading, only a shell comment inside a fence
```

### Deeper

A nested subsection explains that expect adds a message to the panic output.

## The ? operator

The question mark operator converts the error with From and returns early from the function.

## The `Option` type

Pattern matching reads a value out of the Option safely and without a panic.

## Examples

The first examples section shows a parser that reads integers from a file.

## Examples

The second examples section shows a config loader that merges environment variables.

## Tables

| name | note |
| a &#124; b | cafe\u{301} |
";

    fn cite(anchor: Option<&str>, quote: &str) -> Citation {
        Citation {
            line: 1,
            id: SID.to_string(),
            anchor: anchor.map(str::to_string),
            quote: quote.to_string(),
        }
    }

    fn on(body: &str, first_line: usize, anchor: Option<&str>, quote: &str) -> Outcome {
        check(&cite(anchor, quote), body, first_line)
    }

    fn verdict(anchor: Option<&str>, quote: &str) -> Verdict {
        on(BODY, 6, anchor, quote).verdict
    }

    fn one(text: &str) -> Citation {
        let (citations, notices) = parse(text);
        assert_eq!(citations.len(), 1, "{notices:?}");
        citations.into_iter().next().unwrap()
    }

    fn all(anchor: &str) -> Verdict {
        verdict(
            Some(anchor),
            "shows a config loader that merges environment variables",
        )
    }

    // Citation form

    #[test]
    fn a_citation_with_an_anchor() {
        let c = one(&format!(
            "see bilbo:{SID}#Concurrency > Goroutines \"A goroutine has a simple model: it is a function executing concurrently\" now"
        ));
        assert_eq!(c.id, SID);
        assert_eq!(c.anchor.as_deref(), Some("Concurrency > Goroutines"));
        assert_eq!(
            c.quote,
            "A goroutine has a simple model: it is a function executing concurrently"
        );
        assert_eq!(c.line, 1);
    }

    #[test]
    fn a_citation_without_an_anchor() {
        let c = one(&format!("bilbo:{SID} \"the zero value is useful\""));
        assert_eq!(c.anchor, None);
        let c = one(&format!("bilbo:{SID}\t\t“the zero value is useful”."));
        assert_eq!(c.quote, "the zero value is useful");
    }

    #[test]
    fn two_citations_on_one_line_stay_apart() {
        let text = format!(
            "Claim (bilbo:{SID}#Unwrap \"fine for prototypes but a poor choice\"; bilbo:{SID}#Deeper \"expect adds a message to the panic output\")."
        );
        let (citations, notices) = parse(&text);
        assert!(notices.is_empty());
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[1].anchor.as_deref(), Some("Deeper"));
        assert_eq!(
            citations[1].quote,
            "expect adds a message to the panic output"
        );
    }

    #[test]
    fn several_citations_keep_their_line_numbers() {
        let text = format!(
            "Intro line.\n- `bilbo:{SID}#Unwrap \"fine for prototypes but a poor choice in library code\"`\n- bilbo:{SID}#Deeper \"converts the error\"\n\n- bilbo:{SID} \"anything at all goes here now\"\n"
        );
        let (citations, _) = parse(&text);
        let lines: Vec<usize> = citations.iter().map(|c| c.line).collect();
        assert_eq!(lines, [2, 3, 5]);
    }

    #[test]
    fn not_a_citation() {
        for text in [
            "bilbo: no store at /tmp/x",
            "bilbo:short \"words here\"",
            &format!("bilbo:{SID}X \"words here\""),
            "plain text",
        ] {
            let (citations, notices) = parse(text);
            assert!(citations.is_empty(), "{text}");
            assert!(notices.is_empty(), "{text}");
        }
    }

    #[test]
    fn a_lowercase_id_is_still_a_citation() {
        let c = one(&format!(
            "bilbo:{} \"words here are long enough\"",
            SID.to_lowercase()
        ));
        assert_eq!(c.id, SID.to_lowercase());
        let store = Scratch::new();
        store.put("notes/decision-a.md", &front(SID));
        let ids = Ids::scan(&store.0);
        assert!(ids.resolve(SID).is_ok());
        assert!(ids.resolve(&c.id).is_err());
    }

    #[test]
    fn straight_quotes_pair_and_a_close_is_not_followed_by_a_word() {
        let c = one(&format!(
            "bilbo:{SID} \"say \"hi\" and \"bye\" to it\" next"
        ));
        assert_eq!(c.quote, "say \"hi\" and \"bye\" to it");
        let c = one(&format!("bilbo:{SID} \"a \"b\"c d\" e"));
        assert_eq!(c.quote, "a \"b\"c d");
        let (citations, notices) = parse(&format!("bilbo:{SID} \"words\"more"));
        assert!(citations.is_empty());
        assert_eq!(notices.len(), 1);
    }

    #[test]
    fn a_quote_spans_no_blank_line() {
        let text = format!("bilbo:{SID} \"one two\n\nthree four\"");
        let (citations, notices) = parse(&text);
        assert!(citations.is_empty());
        assert_eq!(notices.len(), 1);
        let text = format!("bilbo:{SID} \"one two\n  three four\"");
        assert_eq!(one(&text).quote, "one two\n  three four");
        let text = format!("bilbo:{SID} “one two\n\nthree four”");
        assert!(parse(&text).0.is_empty());
    }

    #[test]
    fn a_curly_quote_runs_to_the_curly_close() {
        let c = one(&format!("bilbo:{SID}#A “say \"hi\" to it”"));
        assert_eq!(c.quote, "say \"hi\" to it");
        assert!(parse(&format!("bilbo:{SID} “words”more")).0.is_empty());
    }

    #[test]
    fn a_citation_without_a_quote_is_a_notice() {
        for text in [
            format!("one\n\nthree\nfour\nbilbo:{SID}#Concurrency\nnext"),
            format!("four\nbilbo:{SID}#Concurrency with no quote\n"),
            format!("see bilbo:{SID}."),
        ] {
            let (citations, notices) = parse(&text);
            assert!(citations.is_empty());
            assert_eq!(notices.len(), 1, "{text}");
            assert!(notices[0].message.contains("no quote"));
        }
        let (_, notices) = parse(&format!("a\nb\nc\nd\nbilbo:{SID}#Concurrency\n"));
        assert_eq!(notices[0].to_string().split(':').next(), Some("line 5"));
    }

    #[test]
    fn other_text_with_a_path_and_a_quote_is_quiet() {
        for text in [
            "a\nsee: ./library/go.md#Errors \"some quoted words here\"\n",
            "source: /x/library/go.md#Errors \"some quoted words here\"",
            "~/x.md \"some quoted words here\"",
        ] {
            assert_eq!(parse(text), (Vec::new(), Vec::new()), "{text}");
        }
    }

    // Resolving the id

    struct Scratch(PathBuf);

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    impl Scratch {
        fn new() -> Scratch {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("bilbo-citation-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn put(&self, path: &str, text: &str) {
            let real = self.0.join(path);
            std::fs::create_dir_all(real.parent().unwrap()).unwrap();
            std::fs::write(real, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn front(id: &str) -> String {
        format!("---\nid: {id}\ncreated: 2026-09-26T13:16-03:00\n---\n\n# T\n\ntext\n")
    }

    #[test]
    fn ids_resolve_notes_sources_and_guides() {
        let store = Scratch::new();
        store.put("notes/decision-release-tags.md", &front(NID));
        store.put("library/go/effective-go.md", &front(SID));
        store.put("library/go/guide.md", &front(GID));
        let ids = Ids::scan(&store.0);
        let note = ids.resolve(NID).unwrap();
        assert_eq!(
            (note.kind, note.name.as_str()),
            (Kind::Note, "decision-release-tags")
        );
        let source = ids.resolve(SID).unwrap();
        assert_eq!(
            (source.kind, source.name.as_str()),
            (Kind::Source, "go/effective-go")
        );
        assert_eq!(source.path, store.0.join("library/go/effective-go.md"));
        let guide = ids.resolve(GID).unwrap();
        assert_eq!((guide.kind, guide.name.as_str()), (Kind::Guide, "go/guide"));
    }

    #[test]
    fn an_unknown_or_shared_id_does_not_resolve() {
        let store = Scratch::new();
        store.put("notes/decision-a.md", &front(NID));
        store.put("notes/decision-b.md", &front(NID));
        store.put("library/go/effective-go.md", &front(SID));
        let ids = Ids::scan(&store.0);
        assert!(
            ids.resolve("01M3EZ8NVEC2KJQNGK5DTK349S")
                .unwrap_err()
                .contains("no note or source")
        );
        let shared = ids.resolve(NID).unwrap_err();
        assert!(shared.starts_with("2 files share this id"), "{shared}");
        assert!(ids.resolve(SID).is_ok());
    }

    #[test]
    fn a_malformed_or_hidden_file_adds_nothing_and_fails_nothing() {
        let store = Scratch::new();
        store.put("notes/decision-bad.md", "no frontmatter\n");
        store.put("notes/decision-open.md", &format!("---\nid: {NID}\n"));
        store.put("notes/decision-badid.md", "---\nid: nope\n---\n");
        store.put(
            "notes/decision-twice.md",
            &format!("---\nid: nope\nid: {NID}\n---\n"),
        );
        store.put("notes/.decision-hidden.md", &front(NID));
        store.put("notes/readme.txt", &front(NID));
        store.put("library/.lock/x.md", &front(NID));
        store.put("library/go/.hidden.md", &front(NID));
        store.put("library/go/ok.md", &front(SID));
        std::fs::write(store.0.join("notes/decision-bin.md"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::write(store.0.join("library/stray.md"), front(GID)).unwrap();
        let ids = Ids::scan(&store.0);
        assert!(ids.resolve(NID).is_err());
        assert!(ids.resolve(GID).is_err());
        assert!(ids.resolve(SID).is_ok());
        assert!(Ids::scan(&store.0.join("missing")).resolve(SID).is_err());
    }

    #[test]
    fn a_file_with_an_odd_name_still_counts_for_its_id() {
        let store = Scratch::new();
        store.put("library/go/Bad_Name.md", &front(SID));
        store.put("library/Odd_Corpus/x.md", &front(GID));
        let ids = Ids::scan(&store.0);
        assert_eq!(ids.resolve(SID).unwrap().name, "go/Bad_Name");
        assert!(ids.resolve(GID).is_ok());
        store.put("library/go/ok.md", &front(SID));
        let shared = Ids::scan(&store.0).resolve(SID).unwrap_err();
        assert!(shared.starts_with("2 files share this id"), "{shared}");
    }

    #[test]
    fn a_crlf_note_still_has_its_id() {
        let text = front(NID).replace('\n', "\r\n");
        assert_eq!(front_id(&text).as_deref(), Some(NID));
    }

    #[test]
    fn the_body_starts_after_the_frontmatter() {
        let text = front(SID);
        let (body, first) = body_of(&text);
        assert_eq!((body, first), ("\n# T\n\ntext\n", 5));
        assert_eq!(body_of("# T\ntext\n"), ("# T\ntext\n", 1));
        assert_eq!(body_of("---\nid: x\n# T\n"), ("---\nid: x\n# T\n", 1));
        assert_eq!(body_of("---\r\nid: x\r\n---\r\n# T\r\n"), ("# T\r\n", 4));
        assert_eq!(body_of("---\n---\n"), ("", 3));
    }

    // Normalizing quotes

    #[test]
    fn markup_quotes_and_spacing_are_normalized() {
        let quote = "Calling unwrap on an Option panics when the value is None, which is\n  fine for prototypes";
        assert_eq!(verdict(Some("Unwrap"), quote), Verdict::Ok);
        assert_eq!(
            verdict(Some("Unwrap"), "See the book for \"unwrap_or\" and friends"),
            Verdict::Ok
        );
        assert_eq!(
            verdict(
                Some("Unwrap"),
                "Calling **unwrap** on an `Option` panics when"
            ),
            Verdict::Ok
        );
        let c = one(&format!(
            "bilbo:{SID}#Unwrap “Calling unwrap on an Option panics when the value”"
        ));
        assert_eq!(check(&c, BODY, 6).verdict, Verdict::Ok);
    }

    #[test]
    fn a_pipe_in_a_table_cell_matches_a_retyped_pipe() {
        assert_eq!(
            verdict(Some("Tables"), "note | | a | b | cafe"),
            Verdict::QuoteMissing
        );
        assert_eq!(
            verdict(Some("Tables"), "note | | a | b | caf\u{e9} |"),
            Verdict::Ok
        );
    }

    #[test]
    fn a_decomposed_accent_matches_a_composed_one() {
        assert_eq!(
            verdict(Some("Tables"), "note | | a | b | caf\u{e9}"),
            Verdict::Ok
        );
        assert_eq!(
            verdict(Some("Tables"), "note | | a | b | cafe\u{301}"),
            Verdict::Ok
        );
    }

    #[test]
    fn ellipsis_splits_into_ordered_fragments() {
        let quote = "Calling unwrap on an Option panics ... a poor choice in library code";
        assert_eq!(verdict(Some("Unwrap"), quote), Verdict::Ok);
        assert_eq!(
            verdict(
                Some("Unwrap"),
                "Calling unwrap on an Option panics … fine for prototypes"
            ),
            Verdict::Ok
        );
        let reversed = "a poor choice in library code ... Calling unwrap on an Option panics";
        assert_eq!(verdict(Some("Unwrap"), reversed), Verdict::QuoteMissing);
        assert_eq!(fragments("a ... b .... c .. d"), ["a", "b", "c .. d"]);
        assert_eq!(fragments("a…b"), ["a", "b"]);
    }

    #[test]
    fn a_read_prefixed_quote_resolves() {
        let quote = "    15\tCalling **unwrap** on an `Option` panics when the value is `None`, which is\n    16\tfine for prototypes but a poor choice";
        assert_eq!(verdict(Some("Unwrap"), quote), Verdict::Ok);
        assert_eq!(
            verdict(Some("Unwrap"), "15→Calling unwrap on an Option panics when"),
            Verdict::Ok
        );
        assert_eq!(fragments("820\tA goroutine has"), ["A goroutine has"]);
        assert_eq!(fragments("2026 is the year"), ["2026 is the year"]);
    }

    // Resolving the anchor and verdicts

    #[test]
    fn the_quote_under_its_heading_is_ok_with_its_lines() {
        let outcome = on(
            BODY,
            6,
            Some("The ? operator"),
            "converts the error with From and returns early",
        );
        assert_eq!(outcome.verdict, Verdict::Ok);
        assert_eq!(outcome.spans, [(27, 27)]);
        assert_eq!(outcome.detail, "lines 27-27");
        let outcome = on(
            BODY,
            6,
            Some("Unwrap"),
            "unwrap on an Option panics when the value is None, which is fine for prototypes",
        );
        assert_eq!(outcome.spans, [(12, 13)]);
    }

    #[test]
    fn a_section_holds_its_subsections_and_stops_at_the_next_heading_of_its_level() {
        let nested = "explains that expect adds a message to the panic output";
        assert_eq!(verdict(Some("Unwrap"), nested), Verdict::Ok);
        assert_eq!(verdict(Some("Deeper"), nested), Verdict::Ok);
        assert_eq!(verdict(Some("Unwrap > Deeper"), nested), Verdict::Ok);
        let outcome = on(
            BODY,
            6,
            Some("Deeper"),
            "converts the error with From and returns early",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
        assert_eq!(outcome.detail, "the quote is under The ? operator");
        let outcome = on(
            BODY,
            6,
            Some("The ? operator"),
            "Calling unwrap on an Option panics when",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
        assert!(
            outcome.detail.ends_with("under Unwrap"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn a_wrong_anchor_names_the_deepest_section() {
        let outcome = on(
            BODY,
            6,
            Some("Unwrap"),
            "converts the error with From and returns early",
        );
        assert_eq!(outcome.detail, "the quote is under The ? operator");
        let outcome = on(
            BODY,
            6,
            Some("The ? operator"),
            "explains that expect adds a message",
        );
        assert_eq!(outcome.detail, "the quote is under Unwrap > Deeper");
    }

    #[test]
    fn a_backticked_heading_matches_a_plain_anchor() {
        let quote = "reads a value out of the Option safely";
        assert_eq!(verdict(Some("The Option type"), quote), Verdict::Ok);
        assert_eq!(verdict(Some("The `Option` type"), quote), Verdict::Ok);
        let outcome = on(BODY, 6, Some("The Options type"), quote);
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
        assert!(outcome.detail.contains("The Options type"));
        assert!(
            outcome.detail.ends_with("under The `Option` type"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn the_title_is_not_an_anchor() {
        let outcome = on(
            BODY,
            6,
            Some("Error Handling in Rust"),
            "converts the error with From and returns early",
        );
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
        assert!(
            outcome.detail.ends_with("under The ? operator"),
            "{}",
            outcome.detail
        );
        let outcome = on(
            BODY,
            6,
            Some("Error Handling in Rust"),
            "Opening words that sit under the title",
        );
        assert!(
            outcome
                .detail
                .ends_with("in the title or the text before the first heading"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn an_anchor_with_the_quote_nowhere_is_just_missing() {
        let outcome = on(
            BODY,
            6,
            Some("Panics"),
            "nothing like this sentence appears anywhere",
        );
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
        assert_eq!(outcome.detail, "no section matches 'Panics'");
        assert_eq!(
            verdict(Some(""), "Calling unwrap on an Option panics when"),
            Verdict::AnchorMissing
        );
    }

    #[test]
    fn case_counts_in_an_anchor() {
        assert_eq!(
            verdict(Some("unwrap"), "Calling unwrap on an Option panics when"),
            Verdict::AnchorMissing
        );
    }

    #[test]
    fn no_anchor_in_a_file_with_sections() {
        let outcome = on(
            BODY,
            6,
            None,
            "converts the error with From and returns early",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
        assert_eq!(outcome.detail, "the quote is under The ? operator");
        assert_eq!(
            verdict(
                None,
                "Opening words that sit under the title before any section"
            ),
            Verdict::Ok
        );
        assert_eq!(
            verdict(
                None,
                "Error Handling in Rust Opening words that sit under the title"
            ),
            Verdict::Ok
        );
    }

    #[test]
    fn a_quote_across_two_sections_is_elsewhere_across_sections() {
        let quote = "before any section begins here. ## Unwrap Calling unwrap on an Option panics";
        let outcome = on(BODY, 6, None, quote);
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
        assert!(
            outcome.detail.ends_with("across sections"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn a_headingless_source_is_cited_without_an_anchor() {
        let body = "# Title\n\nSome plain text that has no headings below the title at all.\n";
        let outcome = on(
            body,
            8,
            None,
            "plain text that has no headings below the title",
        );
        assert_eq!(outcome.verdict, Verdict::Ok);
        assert_eq!(outcome.spans, [(10, 10)]);
        assert_eq!(
            on(body, 8, Some("Title"), "plain text that has no headings").verdict,
            Verdict::AnchorMissing
        );
    }

    #[test]
    fn every_same_named_section_is_searched_and_the_verdict_is_ambiguous() {
        let outcome = on(
            BODY,
            6,
            Some("Examples"),
            "shows a config loader that merges environment variables",
        );
        assert_eq!(outcome.verdict, Verdict::Ambiguous);
        assert_eq!(
            outcome.detail,
            "2 sections match; the quote is under Examples"
        );
        assert_eq!(outcome.spans.len(), 1);
        assert_eq!(all("Examples"), Verdict::Ambiguous);
        assert_eq!(
            verdict(
                Some("Examples"),
                "shows a parser that reads integers from a file"
            ),
            Verdict::Ambiguous
        );
        assert_eq!(
            verdict(
                Some("Examples"),
                "an Examples section that appears nowhere in this file"
            ),
            Verdict::QuoteMissing
        );
    }

    #[test]
    fn an_ambiguous_anchor_with_the_quote_elsewhere_is_elsewhere() {
        let outcome = on(
            BODY,
            6,
            Some("Examples"),
            "converts the error with From and returns early",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
    }

    #[test]
    fn a_retyped_quote_is_missing_with_a_hint() {
        let outcome = on(
            BODY,
            6,
            Some("Unwrap"),
            "Calling unwrap on an Option crashes when the value is None",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
        assert!(
            outcome
                .detail
                .starts_with("nearest passage: Calling unwrap on an Option panics"),
            "{}",
            outcome.detail
        );
        assert!(outcome.spans.is_empty());
        let outcome = on(
            BODY,
            6,
            None,
            "Calling unwrap on an Option crashes when the value is None",
        );
        assert!(
            outcome.detail.contains("Option panics"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn a_hint_comes_from_the_cited_section_and_stays_short() {
        let outcome = on(BODY, 6, Some("Examples > Nothing"), "x");
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
        let long = format!("# T\n\n## S\n\n{}\n", "word ".repeat(200));
        let quote = format!("word {}", "other ".repeat(120));
        let outcome = on(&long, 1, Some("S"), &quote);
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
        assert!(outcome.detail.len() <= "nearest passage: ".len() + HINT_CHARS);
        let outcome = on(BODY, 6, Some("Unwrap"), "zzz yyy xxx www vvv uuu");
        assert_eq!(outcome.detail, "the quote is not in the text");
        let body =
            "# T\n\n## A\n\nalpha beta gamma\n\n## B\n\nsecret outside words appear only here\n";
        let outcome = on(
            body,
            1,
            Some("A"),
            "secret outside words appear only here x",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
        assert!(!outcome.detail.contains("secret"), "{}", outcome.detail);
    }

    #[test]
    fn a_hint_keeps_the_sources_words() {
        let body = "# T\n\n## Busy\n\nWhen a lock is held, the call returns SQLITE_BUSY at once instead of waiting.\n";
        let outcome = on(body, 1, None, "the call returns a busy error at once");
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
        assert!(outcome.detail.contains("SQLITE_BUSY"), "{}", outcome.detail);
        assert!(!outcome.detail.contains("SQLITEBUSY"), "{}", outcome.detail);
        let outcome = on(
            body,
            1,
            Some("Busy"),
            "the call returns a busy error at once",
        );
        assert!(outcome.detail.contains("SQLITE_BUSY"), "{}", outcome.detail);
    }

    #[test]
    fn a_hint_drops_emphasis_marks() {
        let body = "# T\n\n## Busy\n\nthe _call_ returns **fast** and `code`\n";
        let outcome = on(body, 1, None, "the call returns fast and code twice");
        assert!(
            outcome.detail.contains("the call returns fast and code"),
            "{}",
            outcome.detail
        );
    }

    #[test]
    fn nearest_picks_the_first_best_window() {
        let hay = "a b c d e f a b x d e f";
        assert_eq!(nearest(hay, "a b c d"), Some((0, 4)));
        assert_eq!(nearest(hay, "a b x d"), Some((6, 4)));
        assert_eq!(nearest("one two", "three four five"), None);
        assert_eq!(nearest("", "a"), None);
        assert_eq!(nearest("a b", "a b c d e"), Some((0, 2)));
    }

    #[test]
    fn too_short_is_a_warning() {
        assert_eq!(
            verdict(Some("Unwrap"), "Calling unwrap on"),
            Verdict::TooShort
        );
        assert_eq!(
            verdict(Some("Unwrap"), "Calling unwrap on ... library code"),
            Verdict::TooShort
        );
        assert_eq!(
            verdict(Some("Unwrap"), "Calling unwrap on an Nothing"),
            Verdict::QuoteMissing
        );
        assert_eq!(
            verdict(Some("Unwrap"), "Calling unwrap on an Option"),
            Verdict::TooShort
        );
        assert_eq!(
            verdict(Some("Unwrap"), "Calling unwrap on an Option panics"),
            Verdict::Ok
        );
        assert!(!Verdict::TooShort.fails());
        let outcome = on(BODY, 6, Some("Unwrap"), "Calling unwrap on");
        assert_eq!(outcome.detail, "3 words; quote at least 6");
        assert_eq!(outcome.spans, [(12, 12)]);
    }

    #[test]
    fn a_short_quote_that_is_elsewhere_stays_elsewhere() {
        assert_eq!(
            verdict(Some("Deeper"), "Calling unwrap on"),
            Verdict::QuoteElsewhere
        );
    }

    #[test]
    fn a_fenced_comment_is_no_heading() {
        let anchor = "not a heading, only a shell comment inside a fence";
        let outcome = on(BODY, 6, Some(anchor), "only a shell comment inside a fence");
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
    }

    #[test]
    fn frontmatter_is_never_searched() {
        let text = front(SID);
        let (body, first) = body_of(&text);
        let outcome = on(
            body,
            first,
            None,
            "id: 01M3EZ8NVEC2KJQNGK5DTK349R created: 2026-09-26T13:16-03:00",
        );
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
    }

    #[test]
    fn an_empty_quote_is_missing() {
        let outcome = on(BODY, 6, Some("Unwrap"), "...");
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
        assert_eq!(outcome.detail, "the quote holds no text");
        let outcome = on(BODY, 6, Some("Nope"), "...");
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
        assert!(Outcome::id_missing("a\tb".to_string()).verdict.fails());
    }

    #[test]
    fn every_match_reports_its_lines() {
        let body =
            "# T\n\n## S\n\nthe same words appear here.\n\nfiller\n\nthe same words appear here.\n";
        let outcome = on(body, 3, Some("S"), "the same words appear here");
        assert_eq!(outcome.spans, [(7, 7), (11, 11)]);
        let outcome = on(body, 3, None, "the same words appear here");
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
        assert_eq!(outcome.spans, [(7, 7), (11, 11)]);
    }

    #[test]
    fn a_match_spanning_lines_reports_both() {
        let body = "# T\n\n## S\n\nfirst half of\nthe second half here\n";
        let outcome = on(body, 1, Some("S"), "first half of the second half");
        assert_eq!(outcome.spans, [(5, 6)]);
    }

    #[test]
    fn a_document_serves_many_citations() {
        let doc = Document::new(BODY, 6);
        let a = doc.check(&cite(
            Some("Unwrap"),
            "Calling unwrap on an Option panics when",
        ));
        let b = doc.check(&cite(
            Some("Deeper"),
            "Calling unwrap on an Option panics when",
        ));
        assert_eq!(
            (a.verdict, b.verdict),
            (Verdict::Ok, Verdict::QuoteElsewhere)
        );
    }

    #[test]
    fn the_failing_verdicts() {
        let failing: Vec<&str> = [
            Verdict::Ok,
            Verdict::QuoteElsewhere,
            Verdict::Ambiguous,
            Verdict::QuoteMissing,
            Verdict::AnchorMissing,
            Verdict::IdMissing,
            Verdict::TooShort,
            Verdict::Unread,
        ]
        .into_iter()
        .filter(|v| v.fails())
        .map(Verdict::name)
        .collect();
        assert_eq!(
            failing,
            ["quote_missing", "anchor_missing", "id_missing", "unread"]
        );
    }

    #[test]
    fn the_details_are_one_line() {
        let outcome = on(BODY, 6, Some("No\tsuch\nthing"), "x");
        assert!(!outcome.detail.contains(['\t', '\n']));
    }

    const NOTE: &str = "---\nid: 01M3EZ8NVEC2KJQNGK5DTK349R\ncreated: 2026-09-26T13:16-03:00\n---\n\n# Release tags\n\nLead paragraph that says tags are cheap and clear.\n\n## Why\n\nTags name the commit that carries the version bump.\n";

    fn note_on(anchor: Option<&str>, quote: &str) -> Outcome {
        let (body, first) = body_of(NOTE);
        on(body, first, anchor, quote)
    }

    #[test]
    fn a_note_body_is_checked_like_a_source_body() {
        let outcome = note_on(Some("Why"), "name the commit that carries the version bump");
        assert_eq!(outcome.verdict, Verdict::Ok);
        assert_eq!(outcome.spans, [(12, 12)]);
        let outcome = note_on(None, "name the commit that carries the version bump");
        assert_eq!(outcome.verdict, Verdict::QuoteElsewhere);
        assert_eq!(outcome.detail, "the quote is under Why");
        let outcome = note_on(Some("Why"), "tags are cheap and clear indeed");
        assert_eq!(outcome.verdict, Verdict::QuoteMissing);
    }

    #[test]
    fn a_note_title_is_not_an_anchor_and_its_lead_is_under_no_section() {
        let outcome = note_on(
            Some("Release tags"),
            "Lead paragraph that says tags are cheap",
        );
        assert_eq!(outcome.verdict, Verdict::AnchorMissing);
        assert!(
            outcome.detail.contains("title or the text before"),
            "{}",
            outcome.detail
        );
        let outcome = note_on(None, "Lead paragraph that says tags are cheap and clear");
        assert_eq!(outcome.verdict, Verdict::Ok);
        assert_eq!(outcome.spans, [(8, 8)]);
    }

    #[test]
    fn a_headingless_note_is_cited_without_an_anchor() {
        let text = "---\nid: 01M3EZ8NVEC2KJQNGK5DTK349R\ncreated: 2026-09-26T13:16-03:00\n---\n\n# Plain note\n\nSome plain text with no headings below the title.\n";
        let (body, first) = body_of(text);
        let outcome = on(
            body,
            first,
            None,
            "plain text with no headings below the title",
        );
        assert_eq!(outcome.verdict, Verdict::Ok);
    }

    #[test]
    fn an_ambiguous_anchor_with_a_short_quote_is_too_short() {
        let body = "# T\n\n## A\n\n### What\n\nshort quote here ok\n\n## B\n\n### What\n\nother\n";
        let outcome = on(body, 1, Some("What"), "short quote here ok");
        assert_eq!(outcome.verdict, Verdict::TooShort);
    }

    #[test]
    fn spans_hold_every_match_in_the_body_not_only_the_anchored_section() {
        let body = "# T\n\n## A\n\nthe same words appear here now ok.\n\n## B\n\nthe same words appear here now ok.\n";
        let outcome = on(body, 1, Some("A"), "the same words appear here now ok");
        assert_eq!(outcome.verdict, Verdict::Ok);
        assert_eq!(outcome.spans, [(5, 5), (9, 9)]);
    }
}
