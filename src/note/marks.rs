//! Finds a scope's mark in a note: the topic of its file name, its `sources` items and its body.

use crate::shared::markdown::split_lines;
use crate::shared::text;

/// Where a mark was found.
#[derive(Debug, PartialEq, Eq)]
pub enum Place {
    FileName,
    /// A physical line, counted from 1.
    Line(usize),
}

/// How a mark is compared.
pub enum Mark<'a> {
    /// One folded word, as `text::words` gives it.
    Word(&'a str),
    /// Every form of a path; each matches up to a segment boundary.
    Path(&'a [String]),
}

/// The first place holding `mark`: the file name's `topic`, then each line of `text`, skipping the key lines of a closed
/// frontmatter; its `sources` items are searched.
pub fn find(topic: &str, text: &str, mark: &Mark) -> Option<Place> {
    if holds(topic, mark) {
        return Some(Place::FileName);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = split_lines(text);
    let front_end = (lines.first() == Some(&"---"))
        .then(|| lines[1..].iter().position(|l| *l == "---").map(|i| i + 1))
        .flatten();
    lines.iter().enumerate().find_map(|(i, line)| {
        let in_front = front_end.is_some_and(|end| i > 0 && i < end);
        let skipped = in_front && !line.starts_with([' ', '-']);
        (!skipped && holds(line, mark)).then_some(Place::Line(i + 1))
    })
}

fn holds(line: &str, mark: &Mark) -> bool {
    match mark {
        Mark::Word(word) => text::words(line).iter().any(|w| w == word),
        Mark::Path(forms) => forms.iter().any(|form| path_in(line, form)),
    }
}

/// `form` occurs in `line` with a segment boundary after it.
fn path_in(line: &str, form: &str) -> bool {
    if form.is_empty() {
        return false;
    }
    line.match_indices(form).any(|(at, _)| {
        line[at + form.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONT: &str = "---\nid: 01M3YJ7R6HK6NQ30DCDB1P4DYB\ncreated: 2026-10-02T14:23-03:00\n";

    fn note(extra: &str, body: &str) -> String {
        format!("{FRONT}{extra}---\n\n# T\n\n{body}")
    }

    fn word(text: &str, mark: &str) -> Option<Place> {
        find("topic", text, &Mark::Word(mark))
    }

    fn path(text: &str, forms: &[&str]) -> Option<Place> {
        let forms: Vec<String> = forms.iter().map(|f| f.to_string()).collect();
        find("topic", text, &Mark::Path(&forms))
    }

    #[test]
    fn a_word_in_a_source() {
        let text = note(
            "sources:\n  - \"url: https://wiki.acme.example/deploy\"\n",
            "text\n",
        );
        assert_eq!(word(&text, "acme"), Some(Place::Line(5)));
    }

    #[test]
    fn part_of_a_word_does_not_match() {
        assert_eq!(word(&note("", "acmeish stuff\n"), "acme"), None);
    }

    #[test]
    fn a_word_matches_ignoring_case_and_accents() {
        assert_eq!(
            word(&note("", "see ACMÉ docs\n"), "acme"),
            Some(Place::Line(8))
        );
    }

    #[test]
    fn a_path_in_either_form() {
        let forms = ["~/Developer/acme", "/Users/a/Developer/acme"];
        let text = note("", "see /Users/a/Developer/acme/api/main.go\n");
        assert_eq!(path(&text, &forms), Some(Place::Line(8)));
        let tilde = note("", "see ~/Developer/acme\n");
        assert_eq!(path(&tilde, &forms), Some(Place::Line(8)));
    }

    #[test]
    fn a_sibling_path_does_not_match() {
        let forms = ["~/Developer/acme", "/Users/a/Developer/acme"];
        for body in [
            "~/Developer/acme-tools\n",
            "~/Developer/acme_x\n",
            "~/Developer/acme.old\n",
            "~/Developer/acme2\n",
        ] {
            assert_eq!(path(&note("", body), &forms), None, "{body}");
        }
        assert!(path(&note("", "in ~/Developer/acme, ok\n"), &forms).is_some());
    }

    #[test]
    fn fenced_code_is_searched() {
        let text = note("", "```sh\ncd ~/Developer/acme\n```\n");
        assert_eq!(path(&text, &["~/Developer/acme"]), Some(Place::Line(9)));
    }

    #[test]
    fn the_topic_is_searched_first() {
        let text = note("", "acme again\n");
        assert_eq!(
            find("acme-deploy", &text, &Mark::Word("acme")),
            Some(Place::FileName)
        );
    }

    #[test]
    fn id_created_and_scope_lines_are_skipped() {
        let text = note("scope: acme\n", "text\n");
        assert_eq!(word(&text, "acme"), None);
        assert_eq!(word(&text, "m3yj7r6hk6nq30dcdb1p4dyb"), None);
        let sources = note("sources:\n  - \"url: x\"\n", "text\n");
        assert_eq!(word(&sources, "sources"), None);
        let unknown = note("kind: acme\n", "text\n");
        assert_eq!(word(&unknown, "acme"), None);
        let indented = note("sources:\n  - \"doc: scope: acme\"\n", "text\n");
        assert_eq!(word(&indented, "acme"), Some(Place::Line(5)));
    }

    #[test]
    fn a_body_line_that_looks_like_a_key_is_searched() {
        assert_eq!(
            word(&note("", "scope: acme\n"), "acme"),
            Some(Place::Line(8))
        );
    }
}
