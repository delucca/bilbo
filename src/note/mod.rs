//! Notes: kinds, names, the strict reader and the renderer, the `new`, `watch` and `history` verbs, and the version store they share.

pub mod diff;
pub mod history;
pub mod new;
pub mod versions;
pub mod watch;

use crate::shared::frontmatter::{bad_created, bad_id, is_created, is_ulid, split_key};
use crate::shared::markdown::split_lines;
use crate::shared::store::{Problem, TOPIC_RULE, is_topic, title_problem};

pub const KINDS: [&str; 9] = [
    "plan",
    "spec",
    "design",
    "decision",
    "gotcha",
    "research",
    "review",
    "report",
    "reference",
];
pub const SOURCE_TYPES: [&str; 4] = ["url", "code", "doc", "search"];

pub fn kinds_list() -> String {
    KINDS.join(", ")
}

#[derive(Debug)]
pub struct Name {
    pub kind: String,
    pub topic: String,
}

pub fn parse_name(file_name: &str) -> Result<Name, String> {
    let shape = || "name: must be <kind>-<topic>.md".to_string();
    let stem = file_name.strip_suffix(".md").ok_or_else(shape)?;
    let (kind, topic) = stem
        .split_once('-')
        .filter(|(kind, _)| !kind.is_empty())
        .ok_or_else(shape)?;
    if !KINDS.contains(&kind) {
        return Err(format!(
            "name: unknown kind '{kind}'; kinds: {}",
            kinds_list()
        ));
    }
    if !is_topic(topic) {
        return Err(format!("name: invalid topic '{topic}': {TOPIC_RULE}"));
    }
    Ok(Name {
        kind: kind.into(),
        topic: topic.into(),
    })
}

pub fn default_title(topic: &str) -> String {
    let spaced = topic.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub struct Note {
    pub id: Option<String>,
    /// The first `created:` value when it passes `is_created`.
    pub created: Option<String>,
    /// Physical line the body starts on: the line after the closing `---`, or 1 when there is no closed frontmatter.
    pub body_start: usize,
    pub problems: Vec<Problem>,
}

/// Strict read of a whole file's text: every problem, in line order, whole-file ones last. `id` is the first
/// `id:` value when it is a canonical ULID (used for the shared-id rule).
pub fn read(text: &str) -> Note {
    let mut problems = Vec::new();
    let text = match text.strip_prefix('\u{feff}') {
        Some(rest) => {
            problems.push(Problem::at(1, "encoding: remove the byte order mark"));
            rest
        }
        None => text,
    };
    if let Some(n) = text.split('\n').position(|l| l.ends_with('\r')) {
        problems.push(Problem::at(n + 1, "line endings: use LF, not CRLF"));
    }
    let lines = split_lines(text);

    let mut id = None;
    let mut created = None;
    let mut body_start = 1;
    if lines.first() != Some(&"---") {
        problems.push(Problem::at(1, "frontmatter: missing; line 1 must be '---'"));
        problems.extend(title_problem(&lines, 1));
    } else if let Some(close) = lines[1..].iter().position(|l| *l == "---") {
        let close = close + 1;
        (id, created) = read_keys(&lines[1..close], &mut problems);
        body_start = close + 2;
        problems.extend(title_problem(&lines[close + 1..], close + 2));
    } else {
        problems.push(Problem::at(1, "frontmatter: no closing '---' line"));
    }
    problems.sort_by_key(|p| p.line.unwrap_or(usize::MAX));
    Note {
        id,
        created,
        body_start,
        problems,
    }
}

/// The id `read` reports for `text`, without reading the rest: the first `id:` value of a closed frontmatter, when it
/// is a canonical ULID.
pub fn read_id(text: &str) -> Option<String> {
    let lines = split_lines(text.strip_prefix('\u{feff}').unwrap_or(text));
    if lines.first() != Some(&"---") {
        return None;
    }
    let close = lines[1..].iter().position(|l| *l == "---")? + 1;
    let (_, rest) = lines[1..close]
        .iter()
        .filter(|line| !line.starts_with([' ', '-']))
        .filter_map(|line| split_key(line))
        .find(|(key, _)| *key == "id")?;
    Some(rest.strip_prefix(' ')?.to_string()).filter(|value| is_ulid(value))
}

/// "---\nid: <id>\ncreated: <created>\n---\n\n# <title>\n"
pub fn render(id: &str, created: &str, title: &str) -> String {
    format!("---\nid: {id}\ncreated: {created}\n---\n\n# {title}\n")
}

#[derive(Default)]
struct Keys {
    seen_id: bool,
    seen_created: bool,
    seen_sources: bool,
    in_sources: bool,
    sources_line: Option<usize>,
    items: usize,
    id: Option<String>,
    created: Option<String>,
}

/// `lines` are the ones between the delimiters; the first is physical line 2.
fn read_keys(lines: &[&str], problems: &mut Vec<Problem>) -> (Option<String>, Option<String>) {
    let mut keys = Keys::default();
    for (i, line) in lines.iter().enumerate() {
        keys.line(i + 2, line, problems);
    }
    keys.close_sources(problems);
    if !keys.seen_id {
        problems.push(Problem::whole("id: missing"));
    }
    if !keys.seen_created {
        problems.push(Problem::whole("created: missing"));
    }
    (keys.id, keys.created)
}

impl Keys {
    fn line(&mut self, n: usize, line: &str, problems: &mut Vec<Problem>) {
        if line.starts_with([' ', '-']) {
            if self.in_sources {
                self.items += 1;
                check_item(n, line, problems);
            } else {
                problems.push(unexpected_line(n));
            }
            return;
        }
        self.close_sources(problems);
        match split_key(line) {
            Some((key, rest)) => self.key(n, key, rest, problems),
            None => problems.push(unexpected_line(n)),
        }
    }

    fn key(&mut self, n: usize, key: &str, rest: &str, problems: &mut Vec<Problem>) {
        match key {
            "id" => {
                let first = !std::mem::replace(&mut self.seen_id, true);
                repeated(first, n, key, problems);
                if let Some(value) = value(n, key, rest, problems) {
                    if !is_ulid(value) {
                        problems.push(Problem::at(n, bad_id(value)));
                    } else if first {
                        self.id = Some(value.to_string());
                    }
                }
            }
            "created" => {
                let first = !std::mem::replace(&mut self.seen_created, true);
                repeated(first, n, key, problems);
                if let Some(value) = value(n, key, rest, problems) {
                    if !is_created(value) {
                        problems.push(Problem::at(n, bad_created(value)));
                    } else if first {
                        self.created = Some(value.to_string());
                    }
                }
            }
            "sources" => {
                let first = !std::mem::replace(&mut self.seen_sources, true);
                repeated(first, n, key, problems);
                self.in_sources = true;
                self.items = 0;
                if rest.is_empty() {
                    self.sources_line = Some(n);
                } else {
                    problems.push(Problem::at(
                        n,
                        "sources: write 'sources:' alone, with one '  - \"<type>: <value>\"' line per source below it",
                    ));
                }
            }
            _ => problems.push(Problem::at(n, format!("frontmatter: unknown key '{key}'"))),
        }
    }

    fn close_sources(&mut self, problems: &mut Vec<Problem>) {
        self.in_sources = false;
        if let Some(n) = self.sources_line.take()
            && self.items == 0
        {
            problems.push(Problem::at(
                n,
                "sources: empty list; add a source or remove the key",
            ));
        }
    }
}

fn repeated(first: bool, n: usize, key: &str, problems: &mut Vec<Problem>) {
    if !first {
        problems.push(Problem::at(n, format!("{key}: given more than once")));
    }
}

/// The text after `<key>: `, or a problem when the space is missing.
fn value<'a>(n: usize, key: &str, rest: &'a str, problems: &mut Vec<Problem>) -> Option<&'a str> {
    let value = rest.strip_prefix(' ');
    if value.is_none() {
        problems.push(Problem::at(n, format!("{key}: expected '{key}: <value>'")));
    }
    value
}

fn unexpected_line(n: usize) -> Problem {
    Problem::at(
        n,
        "frontmatter: unexpected line; expected 'id: ', 'created: ' or 'sources:'",
    )
}

fn check_item(n: usize, line: &str, problems: &mut Vec<Problem>) {
    let inner = line
        .strip_prefix("  - \"")
        .and_then(|rest| rest.strip_suffix('"'))
        .filter(|inner| !inner.contains(['"', '\\']));
    let Some((kind, value)) = inner.and_then(|inner| inner.split_once(": ")) else {
        problems.push(Problem::at(
            n,
            "sources: write each item as '  - \"<type>: <value>\"'",
        ));
        return;
    };
    if !SOURCE_TYPES.contains(&kind) {
        problems.push(Problem::at(
            n,
            format!(
                "sources: unknown type '{kind}'; types: {}",
                SOURCE_TYPES.join(", ")
            ),
        ));
    } else if value.trim().is_empty() {
        problems.push(Problem::at(n, format!("sources: empty value for '{kind}'")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::frontmatter::{mint_ulid, now_created};
    use crate::shared::markdown::lines;

    const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
    const CREATED: &str = "2026-10-02T14:23-03:00";

    fn joined(lines: &[&str]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    fn messages(text: &str) -> Vec<String> {
        read(text).problems.iter().map(|p| p.to_string()).collect()
    }

    fn with_front(extra: &[&str]) -> String {
        let mut all = vec![
            "---",
            "id: 01M3YJ7R6HK6NQ30DCDB1P4DYB",
            "created: 2026-10-02T14:23-03:00",
        ];
        all.extend(extra);
        all.extend(["---", "", "# Title"]);
        joined(&all)
    }

    fn with_body(body: &[&str]) -> String {
        let mut all = vec![
            "---",
            "id: 01M3YJ7R6HK6NQ30DCDB1P4DYB",
            "created: 2026-10-02T14:23-03:00",
            "---",
            "",
        ];
        all.extend(body);
        joined(&all)
    }

    #[test]
    fn read_id_agrees_with_read() {
        const OTHER: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYC";
        let cases = [
            with_front(&[]),
            with_body(&["text"]),
            "---\ncreated: 2026-10-02T14:23-03:00\n---\n\n# T\n".to_string(),
            format!("---\nid: {ID}\nid: {OTHER}\n---\n"),
            format!("---\nid: bad\nid: {ID}\n---\n"),
            format!("---\nid: {ID}\ncreated: {CREATED}\n"),
            format!("---\nid:{ID}\n---\n"),
            format!("---\nid:  {ID}\n---\n"),
            format!("---\nsources:\n  - \"url: id: {OTHER}\"\nid: {ID}\n---\n"),
            format!("---\r\nid: {ID}\r\n---\r\n"),
            format!("\u{feff}---\nid: {ID}\n---\n"),
            format!("\n---\nid: {ID}\n---\n"),
            format!("# T\n\nid: {ID}\n"),
            String::new(),
        ];
        for text in cases {
            assert_eq!(read_id(&text), read(&text).id, "{text:?}");
        }
        assert_eq!(read_id(&with_front(&[])).as_deref(), Some(ID));
    }

    #[test]
    fn well_named_note() {
        let name = parse_name("decision-note-store.md").unwrap();
        assert_eq!(name.kind, "decision");
        assert_eq!(name.topic, "note-store");
    }

    #[test]
    fn bad_names_are_invalid() {
        for name in [
            "Idea-Foo.md",
            "idea-foo.md",
            "plan-foo--bar.md",
            "plan.md",
            "plan-.md",
            "notes.txt",
        ] {
            assert!(parse_name(name).is_err(), "{name}");
        }
        assert!(
            parse_name("idea-foo.md")
                .unwrap_err()
                .contains("unknown kind 'idea'")
        );
        assert!(
            parse_name("plan-foo--bar.md")
                .unwrap_err()
                .contains("invalid topic 'foo--bar'")
        );
    }

    #[test]
    fn minimal_frontmatter_is_valid() {
        assert!(messages(&with_front(&[])).is_empty());
        let flipped = joined(&[
            "---",
            "created: 2026-10-02T14:23-03:00",
            "id: 01M3YJ7R6HK6NQ30DCDB1P4DYB",
            "---",
            "# T",
        ]);
        assert!(messages(&flipped).is_empty());
        assert_eq!(read(&with_front(&[])).id.as_deref(), Some(ID));
    }

    #[test]
    fn removed_keys_are_named() {
        for key in [
            "kind: decision",
            "supersedes: 01M3YJ7R6HK6NQ30DCDB1P4DYB",
            "project: bilbo",
        ] {
            let found = messages(&with_front(&[key]));
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(
                found[0].contains(key.split(':').next().unwrap()),
                "{found:?}"
            );
        }
    }

    #[test]
    fn missing_frontmatter_is_invalid() {
        let found = messages("# Title\n");
        assert_eq!(
            found,
            ["frontmatter: missing; line 1 must be '---' (line 1)"]
        );
    }

    #[test]
    fn frontmatter_grammar_edges() {
        let bom = format!("\u{feff}{}", with_front(&[]));
        assert_eq!(
            messages(&bom),
            ["encoding: remove the byte order mark (line 1)"]
        );

        let crlf = with_front(&[]).replace('\n', "\r\n");
        assert_eq!(messages(&crlf), ["line endings: use LF, not CRLF (line 1)"]);

        let blank = with_front(&[""]);
        let found = messages(&blank);
        assert_eq!(found.len(), 1);
        assert!(
            found[0].starts_with("frontmatter: unexpected line") && found[0].ends_with("(line 4)"),
            "{found:?}"
        );

        let spaced = with_front(&[]).replacen("---", "--- ", 1);
        assert!(messages(&spaced)[0].starts_with("frontmatter: missing"));

        let open = joined(&[
            "---",
            "id: 01M3YJ7R6HK6NQ30DCDB1P4DYB",
            "created: 2026-10-02T14:23-03:00",
            "# Title",
        ]);
        assert_eq!(
            messages(&open),
            ["frontmatter: no closing '---' line (line 1)"]
        );

        let dup = messages(&with_front(&["id: 01M3YJ7R6HK6NQ30DCDB1P4D00"]));
        assert_eq!(dup, ["id: given more than once (line 4)"]);

        let glued = messages(&with_front(&[]).replace("id: ", "id:"));
        assert_eq!(glued, ["id: expected 'id: <value>' (line 2)"]);
    }

    #[test]
    fn non_canonical_ids_are_invalid() {
        for id in [
            "01m3yj7r6hk6nq30dcdb1p4dyb",
            "01M3YJ7R6HK6NQ30DCDB1P4DY",
            "81M3YJ7R6HK6NQ30DCDB1P4DYB",
            "01M3YJ7R6HK6NQ30DCDB1P4DYI",
            "01M3YJ7R6HK6NQ30DCDB1P4DYL",
            "01M3YJ7R6HK6NQ30DCDB1P4DYO",
            "01M3YJ7R6HK6NQ30DCDB1P4DYU",
        ] {
            assert!(!is_ulid(id), "{id}");
            assert!(
                !messages(&with_front(&[]).replace(ID, id)).is_empty(),
                "{id}"
            );
        }
    }

    #[test]
    fn other_created_forms_are_invalid() {
        for s in [
            "2026-10-02",
            "2026-10-02T14:23:05-03:00",
            "2026-10-02T17:23Z",
            "2026-10-02T14:23-03:00 ",
            "2026-10-02T14:23-03:00\n",
            " 2026-10-02T14:23-03:00",
            "2026-10-02T14:23+0300",
            "2026-10-02T14:23-00:00",
            "2026-10-02T14:23-03:00:00",
            "2026-10-02T4:23-03:00",
            "2026-1-2T14:23-03:00",
            "+2026-10-02T14:23-03:00",
            "-0001-01-01T00:00+00:00",
            "2026-10-02T24:00-03:00",
            "2026-10-02T14:60-03:00",
            "2026-13-02T14:23-03:00",
            "",
        ] {
            assert!(!is_created(s), "{s:?}");
        }
        let found = messages(&with_front(&[]).replace(CREATED, "2026-10-02"));
        assert!(
            found[0].starts_with("created: '2026-10-02' is not"),
            "{found:?}"
        );
    }

    #[test]
    fn valid_sources_list() {
        let found = messages(&with_front(&[
            "sources:",
            "  - \"url: https://github.com/delucca/bilbo\"",
            "  - \"code: src/main.rs\"",
            "  - \"doc: notes/spec.md\"",
            "  - \"search: bilbo\"",
        ]));
        assert!(found.is_empty(), "{found:?}");
        let first = messages(&with_front(&[
            "sources:",
            "  - \"url: x\"",
            "created: 2026-10-02T14:23-03:00",
        ]));
        assert_eq!(first, ["created: given more than once (line 6)"]);
    }

    #[test]
    fn empty_sources_are_invalid() {
        let inline = messages(&with_front(&["sources: []"]));
        assert_eq!(inline.len(), 1);
        assert!(
            inline[0].starts_with("sources: write 'sources:' alone"),
            "{inline:?}"
        );
        let bare = messages(&with_front(&["sources:"]));
        assert_eq!(
            bare,
            ["sources: empty list; add a source or remove the key (line 4)"]
        );
    }

    #[test]
    fn unknown_source_type_is_invalid() {
        let found = messages(&with_front(&[
            "sources:",
            "  - \"web: https://example.org\"",
        ]));
        assert_eq!(
            found,
            ["sources: unknown type 'web'; types: url, code, doc, search (line 5)"]
        );
    }

    #[test]
    fn source_item_forms() {
        for item in [
            "  - 'url: x'",
            "    - \"url: x\"",
            "  - \"url:x\"",
            "  - \"url: a\\b\"",
            "  - \"url: say \"hi\"\"",
        ] {
            let found = messages(&with_front(&["sources:", item]));
            assert_eq!(found.len(), 1, "{item}: {found:?}");
            assert!(
                found[0].starts_with("sources: write each item"),
                "{item}: {found:?}"
            );
        }
        for item in ["  - \"url: \"", "  - \"url:   \""] {
            let empty = messages(&with_front(&["sources:", item]));
            assert_eq!(empty, ["sources: empty value for 'url' (line 5)"]);
        }
        let outside = messages(&with_front(&["  - \"url: x\""]));
        assert!(
            outside[0].starts_with("frontmatter: unexpected line"),
            "{outside:?}"
        );
    }

    #[test]
    fn one_title_is_valid() {
        assert!(messages(&with_body(&["# Note store", "", "## Why", "text"])).is_empty());
    }

    #[test]
    fn heading_inside_fence_does_not_count() {
        for fence in ["```", "~~~"] {
            let body = ["# Note store", fence, "# a shell comment", fence];
            assert!(messages(&with_body(&body)).is_empty(), "{fence}");
        }
        let info = messages(&with_body(&["# One", "``` a`b", "# Two"]));
        assert!(info[0].starts_with("title: found 2"), "{info:?}");
        let only_fenced = messages(&with_body(&["```", "# hidden"]));
        assert!(
            only_fenced[0].starts_with("title: missing"),
            "{only_fenced:?}"
        );
    }

    #[test]
    fn no_title_or_two_titles_is_invalid() {
        let none = messages(&with_body(&["text"]));
        assert_eq!(
            none,
            ["title: missing; add one '# <title>' line after the frontmatter"]
        );
        let two = messages(&with_body(&["# One", "text", "# Two"]));
        assert_eq!(
            two,
            ["title: found 2 '# ' headings outside code fences, expected one (line 8)"]
        );
    }

    #[test]
    fn read_returns_created_and_body_start() {
        let note = read(&with_front(&[]));
        assert_eq!(note.created.as_deref(), Some(CREATED));
        assert_eq!(note.body_start, 5);
    }

    #[test]
    fn invalid_created_is_none() {
        let bad = with_front(&[]).replace(CREATED, "2026-10-02");
        let note = read(&bad);
        assert_eq!(note.created, None);
        assert_eq!(note.body_start, 5);
        let second =
            with_front(&[&format!("created: {CREATED}")]).replacen(CREATED, "2026-10-02", 1);
        assert_eq!(read(&second).created, None);
    }

    #[test]
    fn no_frontmatter_body_starts_at_line_1() {
        let note = read("# Title\n");
        assert_eq!(note.body_start, 1);
        assert_eq!(note.created, None);
    }

    #[test]
    fn unclosed_frontmatter_body_starts_at_line_1() {
        let open = joined(&["---", &format!("created: {CREATED}"), "# Title"]);
        let note = read(&open);
        assert_eq!(note.body_start, 1);
        assert_eq!(note.created, None);
    }

    #[test]
    fn body_start_after_last_line() {
        let text = joined(&[
            "---",
            &format!("id: {ID}"),
            &format!("created: {CREATED}"),
            "---",
        ]);
        assert_eq!(read(&text).body_start, 5);
        assert_eq!(lines(&text).len(), 4);
    }

    #[test]
    fn rendered_note_reads_back_clean() {
        let text = render(&mint_ulid().unwrap(), &now_created(), "Note store");
        let note = read(&text);
        assert!(note.problems.is_empty());
        assert!(note.id.is_some());
    }

    #[test]
    fn render_is_exact() {
        assert_eq!(
            render(ID, CREATED, "Note store"),
            "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02T14:23-03:00\n---\n\n# Note store\n"
        );
    }

    #[test]
    fn default_title_from_topic() {
        assert_eq!(default_title("note-store"), "Note store");
        assert_eq!(default_title("release"), "Release");
    }
}
