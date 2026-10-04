//! Markdown as bilbo reads it: physical lines, code fences, ATX headings and the sections they open.

use crate::text;

/// Physical lines as `read` numbers them: one leading byte order mark dropped, split on '\n', no empty line after a
/// final '\n', one trailing '\r' removed from each.
pub fn lines(text: &str) -> Vec<&str> {
    split_lines(text.strip_prefix('\u{feff}').unwrap_or(text))
}

pub fn split_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for line in &mut lines {
        *line = line.strip_suffix('\r').unwrap_or(line);
    }
    lines
}

/// The fence character, its run length and the rest of the line, when `line` is a fence line.
pub fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let stripped = line.trim_start_matches(' ');
    if line.len() - stripped.len() > 3 {
        return None;
    }
    let ch = stripped.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = stripped.chars().take_while(|c| *c == ch).count();
    let rest = &stripped[len..];
    let is_fence = len >= 3 && !(ch == '`' && rest.contains('`'));
    is_fence.then_some((ch, len, rest))
}

/// An ATX heading outside a fence: its level and its text with the closing sequence removed and whitespace collapsed.
pub fn heading(line: &str) -> Option<(usize, String)> {
    let level = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let mut text = line[level..].strip_prefix(' ')?.trim_end();
    if text.ends_with('#') {
        let stripped = text.trim_end_matches('#');
        if stripped.is_empty() || stripped.ends_with(char::is_whitespace) {
            text = stripped;
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some((level, text))
}

/// Indexes of the lines outside fenced code blocks, fence lines excluded.
pub fn outside_fences(lines: &[&str]) -> Vec<usize> {
    let mut fence: Option<(char, usize)> = None;
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let run = fence_run(line);
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

/// The sections of a file whose `lines` come from `lines`; its body, and so its title, starts on physical
/// line `body_start`.
pub fn outline(lines: &[&str], body_start: usize) -> Vec<Section> {
    let body = lines.get(body_start - 1..).unwrap_or(&[]);
    let mut headings: Vec<(usize, usize, String)> = outside_fences(body)
        .into_iter()
        .filter_map(|i| heading(body[i]).map(|(level, text)| (i, level, text)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_match_read_numbering() {
        assert_eq!(lines("\u{feff}a\r\nb\r\n"), ["a", "b"]);
        assert_eq!(lines("a\n\nb"), ["a", "", "b"]);
        assert_eq!(lines(""), Vec::<&str>::new());
    }

    #[test]
    fn heading_line_rules() {
        for line in ["#", "# ", "#tag", "####### x", " ## x", "#\tx", "## ##"] {
            assert_eq!(heading(line), None, "{line:?}");
        }
        assert_eq!(heading("## Foo ##"), Some((2, "Foo".into())));
        assert_eq!(heading("## C#"), Some((2, "C#".into())));
        assert_eq!(heading("##   a   b"), Some((2, "a b".into())));
        assert_eq!(heading("## Foo ##   "), Some((2, "Foo".into())));
    }

    /// `body` after six frontmatter lines, where `source::read` puts a source's body.
    fn outline_of(body: &str) -> Vec<Section> {
        let text = format!("{}{body}", "-\n".repeat(6));
        outline(&lines(&text), 7)
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
    fn tokens_round_up() {
        assert_eq!(tokens(1000), 400);
        assert_eq!(tokens(1001), 401);
        assert_eq!(tokens(0), 0);
        assert_eq!(tokens(96211), 38485);
        assert_eq!(tokens(1001) * 2, 802);
        assert_eq!(tokens(1001) + tokens(1001), 802);
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
}
