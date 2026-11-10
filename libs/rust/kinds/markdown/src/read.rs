//! A Markdown file as heading sections: the frontmatter block, the text before the first heading,
//! then one passage per heading, down to the line before the next one. Every line is in exactly
//! one passage, so the passages of a file join back to its bytes.

use std::borrow::Cow;

use bilbo_kinds::reader::{Passage, Piece};
use bilbo_store::entries::RecordRange;

use crate::frontmatter;

const FRONTMATTER: &str = "frontmatter";
/// Between the headings of a label: `Title > Section`.
const UNDER: &str = " > ";
/// The characters of a heading a label keeps. A label holds at most six headings, so the labels
/// of a file stay within a small multiple of its size.
const KEPT: usize = 100;
const CLIPPED: char = '…';

fn clip(heading: &str) -> Cow<'_, str> {
    match heading.char_indices().nth(KEPT) {
        Some((end, _)) => Cow::Owned(format!("{}{CLIPPED}", &heading[..end])),
        None => Cow::Borrowed(heading),
    }
}

/// A record's line without its line ending.
fn line(record: &[u8]) -> &str {
    let text = std::str::from_utf8(record).unwrap_or_default();
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.strip_suffix('\r').unwrap_or(text)
}

/// The fence character and its run length when the line opens or closes a fenced code block,
/// and whether anything but spaces follows the run.
fn fence(line: &str) -> Option<(char, usize, bool)> {
    let stripped = line.trim_start_matches(' ');
    if line.len() - stripped.len() > 3 {
        return None;
    }
    let mark = stripped.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = stripped.chars().take_while(|c| *c == mark).count();
    let rest = &stripped[run..];
    (run >= 3 && !(mark == '`' && rest.contains('`'))).then(|| (mark, run, !rest.trim().is_empty()))
}

/// `# Title` to `###### Title`: the level and the text, a closing run of `#` removed.
fn atx(line: &str) -> Option<(usize, String)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
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

/// The level a setext underline gives the paragraph above it: a run of `=` or of `-`.
fn underline(line: &str) -> Option<usize> {
    let stripped = line.trim_start_matches(' ');
    if line.len() - stripped.len() > 3 {
        return None;
    }
    let run = stripped.trim_end_matches([' ', '\t']);
    match run.chars().next()? {
        '=' if run.chars().all(|c| c == '=') => Some(1),
        '-' if run.chars().all(|c| c == '-') => Some(2),
        _ => None,
    }
}

/// Whether the line is indented as code: four spaces or a tab.
fn indented(line: &str) -> bool {
    line.starts_with('\t') || line.len() - line.trim_start_matches(' ').len() > 3
}

/// A thematic break: three or more `*`, `-` or `_`, with nothing but spaces between and around.
fn rule(line: &str) -> bool {
    let marks = line.trim_matches([' ', '\t']);
    let mark = marks
        .chars()
        .next()
        .filter(|c| matches!(c, '*' | '-' | '_'));
    !indented(line)
        && mark.is_some_and(|mark| {
            marks.chars().all(|c| c == mark || c == ' ' || c == '\t')
                && marks.chars().filter(|c| *c == mark).count() >= 3
        })
}

/// A quote or a list item: the lines after it, down to a blank one, continue it. A line indented
/// as code is neither.
fn container(line: &str) -> bool {
    let stripped = line.trim_start_matches(' ');
    let item = |rest: &str| rest.starts_with([' ', '\t']) || rest.is_empty();
    let numbered = || {
        let digits = stripped.trim_start_matches(|c: char| c.is_ascii_digit());
        digits.len() < stripped.len() && digits.strip_prefix(['.', ')']).is_some_and(item)
    };
    !indented(line)
        && (stripped.starts_with('>')
            || stripped.strip_prefix(['-', '*', '+']).is_some_and(item)
            || numbered())
}

/// A line of paragraph text, which a setext underline can make a heading: not blank, not
/// indented as code, and not a quote, a list item, a table row, a thematic break or a heading.
fn paragraph(line: &str) -> bool {
    let stripped = line.trim_start_matches(' ');
    !(stripped.trim().is_empty()
        || indented(line)
        || stripped.starts_with(['|', '#'])
        || container(line)
        || rule(line)
        || underline(line).is_some())
}

struct Heading {
    /// The line the heading starts on, counted from 0: a setext heading starts with its paragraph.
    start: usize,
    level: usize,
    text: String,
}

/// The headings from line `from` on, outside fenced code blocks.
fn headings(lines: &[&str], from: usize) -> Vec<Heading> {
    let mut open: Option<(char, usize)> = None;
    let mut found: Vec<Heading> = Vec::new();
    // The paragraph the last line continues: where it starts.
    let mut text_from: Option<usize> = None;
    // Inside a quote or a list item, whose lines a setext underline makes no heading.
    let mut contained = false;
    for (at, line) in lines.iter().enumerate().skip(from) {
        let run = fence(line);
        if let Some((mark, length)) = open {
            if run.is_some_and(|(c, l, more)| c == mark && l >= length && !more) {
                open = None;
            }
            continue;
        }
        if let (Some(level), Some(start)) = (underline(line), text_from) {
            found.push(Heading {
                start,
                level,
                text: lines[start..at]
                    .iter()
                    .flat_map(|line| line.split_whitespace())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            text_from = None;
            continue;
        }
        if let Some((mark, length, _)) = run {
            open = Some((mark, length));
            text_from = None;
            contained = false;
            continue;
        }
        if let Some((level, text)) = atx(line) {
            found.push(Heading {
                start: at,
                level,
                text,
            });
        }
        // A blank line, a heading and a thematic break end a quote or a list item.
        if line.trim().is_empty() || line.starts_with('#') || rule(line) {
            contained = false;
        } else if container(line) {
            contained = true;
        }
        text_from = if paragraph(line) && !contained {
            text_from.or(Some(at))
        } else {
            None
        };
    }
    found
}

struct Section {
    start: usize,
    label: String,
}

/// Where each passage starts: the frontmatter, the text after it, then each heading with the
/// headings it is under.
fn sections(lines: &[&str]) -> Vec<Section> {
    let mut sections = Vec::new();
    let body = match frontmatter::close(lines.iter().copied()) {
        Some(close) => {
            sections.push(Section {
                start: 0,
                label: FRONTMATTER.to_owned(),
            });
            close + 1
        }
        None => 0,
    };
    let headings = headings(lines, body);
    if body < lines.len() && headings.first().is_none_or(|first| first.start > body) {
        sections.push(Section {
            start: body,
            label: String::new(),
        });
    }
    let mut above: Vec<(usize, Cow<'_, str>)> = Vec::new();
    for heading in &headings {
        while above
            .last()
            .is_some_and(|(level, _)| *level >= heading.level)
        {
            above.pop();
        }
        above.push((heading.level, clip(&heading.text)));
        let path: Vec<&str> = above.iter().map(|(_, text)| text.as_ref()).collect();
        sections.push(Section {
            start: heading.start,
            label: path.join(UNDER),
        });
    }
    sections
}

/// One passage per section, its records the section's lines. Blank lines alone are no passage:
/// they join the one before them, or the one after them at the top of the file.
pub fn passages(records: &[&[u8]]) -> Vec<Passage> {
    let mut lines: Vec<&str> = records.iter().map(|record| line(record)).collect();
    if let Some(first) = lines.first_mut() {
        *first = first.strip_prefix('\u{feff}').unwrap_or(first);
    }
    let found = sections(&lines);
    let mut passages: Vec<Passage> = Vec::new();
    let mut carried: Option<usize> = None;
    for (at, section) in found.iter().enumerate() {
        let end = found.get(at + 1).map_or(lines.len(), |next| next.start);
        let start = carried.take().unwrap_or(section.start);
        let blank = lines[section.start..end]
            .iter()
            .all(|line| line.trim().is_empty());
        if blank && let Some(before) = passages.last_mut() {
            extend(before, &records[start..end]);
            continue;
        }
        if blank && at + 1 < found.len() {
            carried = Some(start);
            continue;
        }
        let mut passage = Passage {
            range: RecordRange {
                first: u64::try_from(start).map_or(u64::MAX, |before| before.saturating_add(1)),
                count: 0,
            },
            text: String::new(),
            pieces: vec![Piece {
                start: 0,
                label: section.label.clone(),
            }],
            ends: Vec::new(),
            at: None,
            outline: None,
        };
        extend(&mut passage, &records[start..end]);
        passage.outline = Some(if section.label.is_empty() {
            let first = lines[start..end]
                .iter()
                .find(|line| !line.trim().is_empty());
            (*first.unwrap_or(&"")).to_owned()
        } else {
            section.label.clone()
        });
        passages.push(passage);
    }
    passages
}

fn extend(passage: &mut Passage, records: &[&[u8]]) {
    for record in records {
        passage.text.push_str(&String::from_utf8_lossy(record));
        passage.ends.push(passage.text.len());
        passage.range.count += 1;
    }
}

#[cfg(test)]
mod tests {
    use bilbo_kinds::reader::Passage;

    use super::{atx, passages, underline};

    fn read(text: &str) -> Vec<Passage> {
        let records: Vec<&[u8]> = text.split_inclusive('\n').map(str::as_bytes).collect();
        passages(&records)
    }

    /// Each passage as its first and last line, counted from 1, and its label.
    fn shape(text: &str) -> Vec<(u64, u64, String)> {
        read(text)
            .iter()
            .map(|passage| {
                (
                    passage.range.first,
                    passage.range.first + passage.range.count - 1,
                    passage.pieces[0].label.clone(),
                )
            })
            .collect()
    }

    fn row(first: u64, last: u64, label: &str) -> (u64, u64, String) {
        (first, last, label.to_owned())
    }

    #[test]
    fn a_note_reads_as_its_frontmatter_then_one_passage_per_heading() {
        let note = include_str!("../fixtures/note.md");
        let found = read(note);
        let labels: Vec<&str> = found.iter().map(|p| p.pieces[0].label.as_str()).collect();
        assert_eq!(labels[0], "frontmatter");
        assert!(labels.len() > 1, "{labels:?}");
        assert!(found.iter().all(|passage| passage.at.is_none()));
        let listed: Vec<Option<&str>> = found.iter().map(|p| p.outline.as_deref()).collect();
        assert_eq!(listed, [Some("frontmatter"), Some("A synthetic note")]);
        let joined: String = found.iter().map(|passage| passage.text.as_str()).collect();
        assert_eq!(joined, note, "the passages join back to the file");
    }

    #[test]
    fn the_sections_of_a_note_are_its_headings_in_both_styles_outside_code() {
        assert_eq!(
            shape(include_str!("../fixtures/sections.md")),
            [
                row(1, 7, "frontmatter"),
                row(8, 11, "The garden log"),
                row(12, 21, "The garden log > Beds"),
                row(22, 27, "The garden log > Watering"),
                row(28, 31, "The garden log > Watering > Tools"),
                row(32, 34, "The garden log > Harvest"),
            ]
        );
    }

    #[test]
    fn each_atx_heading_starts_a_passage_labelled_with_the_headings_above_it() {
        let text = "# Title\n\nintro\n\n## One\n\na\n\n### Deep\n\nb\n\n## Two\n\nc\n# Other\n";
        assert_eq!(
            shape(text),
            [
                row(1, 4, "Title"),
                row(5, 8, "Title > One"),
                row(9, 12, "Title > One > Deep"),
                row(13, 15, "Title > Two"),
                row(16, 16, "Other"),
            ]
        );
    }

    #[test]
    fn a_setext_heading_starts_at_its_paragraph() {
        let text = "Title\n=====\n\nintro\n\nSecond\nline\n------\n\nbody\n";
        assert_eq!(
            shape(text),
            [row(1, 5, "Title"), row(6, 10, "Title > Second line")]
        );
    }

    #[test]
    fn a_rule_a_list_a_quote_or_a_table_above_dashes_is_no_heading() {
        for text in [
            "intro\n\n---\n\nmore\n",
            "- item\n---\n",
            "1. item\n---\n",
            "> quote\n---\n",
            "| a | b |\n---\n",
            "    code\n---\n",
            "# Title\n---\nbody\n",
        ] {
            assert_eq!(read(text).len(), 1, "{text:?}");
        }
        assert_eq!(shape("a\n---\nb\n===\n").len(), 2);
        // A line that continues a list item or a quote is no paragraph of its own.
        for text in [
            "- first point\n  more about it\n---\n",
            "> quote\ncontinued\n---\n",
            "1. item\nlazy line\nand another\n---\n",
        ] {
            assert_eq!(read(text).len(), 1, "{text:?}");
        }
        assert_eq!(
            shape("- item\n  more\n\nTitle\n---\nbody\n"),
            [row(1, 3, ""), row(4, 6, "Title")],
            "a blank line ends the item"
        );
        assert_eq!(
            shape("- item\n---\nTitle\n===\n"),
            [row(1, 2, ""), row(3, 4, "Title")],
            "a rule ends the item"
        );
        assert_eq!(
            shape("    - code\nTitle\n---\nbody\n"),
            [row(1, 1, ""), row(2, 4, "Title")],
            "indented code is no list item"
        );
        assert_eq!(
            shape("\t> code\nTitle\n===\n"),
            [row(1, 1, ""), row(2, 3, "Title")]
        );
        for rule in ["***", "___", "* * *", " - - -", "_ _ _ _"] {
            assert_eq!(
                shape(&format!("- item\n{rule}\nTitle\n---\n")),
                [row(1, 2, ""), row(3, 4, "Title")],
                "{rule} ends the item"
            );
            assert_eq!(
                shape(&format!("{rule}\n---\n")),
                [row(1, 2, "")],
                "{rule} is no paragraph, so no heading"
            );
        }
        assert_eq!(
            read("- item\n**\nlazy\n---\n").len(),
            1,
            "two marks are no rule"
        );
        assert_eq!(
            shape("> quote\n# One\nTitle\n---\n"),
            [row(1, 1, ""), row(2, 2, "One"), row(3, 4, "One > Title")]
        );
        assert_eq!(underline("==="), Some(1));
        assert_eq!(underline("   - "), Some(2));
        assert_eq!(underline("- x"), None);
        assert_eq!(underline("   --\t"), Some(2));
        assert_eq!(underline("    ---"), None);
        assert_eq!(underline("-=-"), None);
        assert_eq!(underline(""), None);
    }

    #[test]
    fn the_labels_of_a_note_stay_within_a_small_multiple_of_its_size() {
        let long = "x".repeat(20_000);
        let note = format!("# {long}\n{}", "## a\n".repeat(2_000));
        let found = read(&note);
        assert_eq!(found.len(), 2_001);
        let held: usize = found
            .iter()
            .map(|passage| {
                passage.pieces[0].label.len() + passage.outline.as_ref().map_or(0, String::len)
            })
            .sum();
        assert!(held <= 20 * note.len(), "{held} bytes of labels");
        assert_eq!(
            found[1].pieces[0].label,
            format!("{}… > a", "x".repeat(100)),
            "a long heading is clipped in the path"
        );
        let setext = format!("{}\n===\n## a\n", "wide ".repeat(100));
        assert_eq!(read(&setext)[1].pieces[0].label.chars().count(), 105);
    }

    /// The reader scans lines; it does not parse Markdown's containers. Inside or right
    /// after a list, a quote or indented code a label can be wrong; no line is lost or doubled.
    #[test]
    fn where_a_label_can_be_wrong_every_line_is_still_in_exactly_one_passage() {
        for text in [
            "- item\n  ***\n  continued\n---\n",
            " \t- code\nTitle\n---\nbody\n",
        ] {
            let found = read(text);
            let lines = text.lines().count() as u64;
            let mut next = 1;
            for passage in &found {
                assert_eq!(passage.range.first, next, "{text:?}");
                assert_eq!(passage.ends.len() as u64, passage.range.count, "{text:?}");
                next += passage.range.count;
            }
            assert_eq!(next - 1, lines, "{text:?}");
            let joined: String = found.iter().map(|passage| passage.text.as_str()).collect();
            assert_eq!(joined, text, "a section's text is its records");
        }
    }

    #[test]
    fn a_heading_inside_a_code_fence_is_text() {
        let text = "# Title\n\n```sh\n# a comment\nTitle\n=====\n```\n\n~~~\n## also text\n```\n~~~\n\n## Real\n";
        assert_eq!(
            shape(text),
            [row(1, 13, "Title"), row(14, 14, "Title > Real")]
        );
        let unclosed = "# Title\n\n````\n```\n# still code\n";
        assert_eq!(shape(unclosed), [row(1, 5, "Title")]);
        let inline = "```not a fence```\n# Heading\n";
        assert_eq!(shape(inline), [row(1, 1, ""), row(2, 2, "Heading")]);
    }

    #[test]
    fn frontmatter_is_its_own_passage_and_its_lines_are_never_headings() {
        let text = "---\nid: x\n# not a heading\ntitle\n---\n\n# Title\n\nbody\n";
        assert_eq!(shape(text), [row(1, 6, "frontmatter"), row(7, 9, "Title")]);
        let only = "---\nid: x\n---\n";
        assert_eq!(shape(only), [row(1, 3, "frontmatter")]);
        let unclosed = "---\nid: x\n\n# Title\n";
        assert_eq!(shape(unclosed), [row(1, 3, ""), row(4, 4, "Title")]);
        let bom = "\u{feff}---\r\nid: x\r\n---\r\n# Title\r\n";
        assert_eq!(shape(bom), [row(1, 3, "frontmatter"), row(4, 4, "Title")]);
    }

    #[test]
    fn text_before_the_first_heading_is_a_passage_with_no_label() {
        assert_eq!(
            shape("intro\nmore\n\n# Title\nbody\n"),
            [row(1, 3, ""), row(4, 5, "Title")]
        );
        assert_eq!(shape("---\na: b\n---\nintro\n# T\n")[1], row(4, 4, ""));
    }

    #[test]
    fn a_passage_is_listed_by_its_headings_or_by_its_first_line_that_is_not_blank() {
        let listed = |text: &str| -> Vec<String> {
            read(text)
                .into_iter()
                .filter_map(|passage| passage.outline)
                .collect()
        };
        assert_eq!(
            listed("\n  intro\nmore\n# Title\n## One\n"),
            ["  intro", "Title", "Title > One"]
        );
        assert_eq!(listed("\n\n"), [""]);
    }

    #[test]
    fn a_file_with_no_heading_is_one_passage() {
        assert_eq!(shape("just\ntext\n"), [row(1, 2, "")]);
        assert_eq!(shape("no final newline"), [row(1, 1, "")]);
        assert_eq!(shape("\n\n\n"), [row(1, 3, "")]);
        assert!(read("").is_empty());
    }

    #[test]
    fn blank_lines_alone_join_the_passage_before_them_or_the_next_at_the_top_of_a_file() {
        assert_eq!(shape("\n\n# Title\nbody\n"), [row(1, 4, "Title")]);
        assert_eq!(
            shape("---\na: b\n---\n\n\n# Title\n"),
            [row(1, 5, "frontmatter"), row(6, 6, "Title")]
        );
    }

    #[test]
    fn every_line_is_in_exactly_one_passage_and_the_ends_are_its_lines() {
        for text in [
            include_str!("../fixtures/note.md"),
            include_str!("../fixtures/crlf.md"),
            include_str!("../fixtures/bom.md"),
            include_str!("../fixtures/no-final-newline.md"),
            include_str!("../fixtures/no-frontmatter.md"),
            include_str!("../fixtures/bad-id.md"),
            include_str!("../fixtures/sections.md"),
            "\n\n",
            "a\n===\n---\n===\n# b\n```\n",
        ] {
            let found = read(text);
            let lines: Vec<&str> = text.split_inclusive('\n').collect();
            let mut next = 1;
            for passage in &found {
                assert_eq!(passage.range.first, next, "{text:?}");
                assert_eq!(passage.ends.len() as u64, passage.range.count);
                let mut start = 0;
                for (at, end) in passage.ends.iter().enumerate() {
                    let line = lines[(passage.range.first - 1) as usize + at];
                    assert_eq!(&passage.text[start..*end], line);
                    start = *end;
                }
                next += passage.range.count;
            }
            assert_eq!(next as usize - 1, lines.len(), "{text:?}");
        }
    }

    #[test]
    fn an_atx_heading_is_one_to_six_hashes_a_space_and_text() {
        for (line, heading) in [
            ("# Title", Some((1, "Title"))),
            ("###### Six", Some((6, "Six"))),
            ("## Two  words ##", Some((2, "Two words"))),
            ("## C# ", Some((2, "C#"))),
            ("####### Seven", None),
            ("#No space", None),
            ("# ", None),
            ("#", None),
            (" # Indented", None),
            ("text # not", None),
        ] {
            assert_eq!(
                atx(line),
                heading.map(|(level, text)| (level, text.to_owned())),
                "{line:?}"
            );
        }
    }

    /// xorshift64*, from a fixed seed: the same cases on every run.
    struct Dice(u64);

    impl Dice {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let drawn = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33;
            usize::try_from(drawn).unwrap_or(0) % bound.max(1)
        }

        fn byte(&mut self) -> u8 {
            const OFTEN: &[u8] = b"{}[]\",:\\ \n\r#-=>`~|0159eEuNI\xff\xc3\x80";
            if self.below(3) == 0 {
                u8::try_from(self.below(256)).unwrap_or(0)
            } else {
                OFTEN[self.below(OFTEN.len())]
            }
        }

        fn bytes(&mut self, most: usize) -> Vec<u8> {
            (0..self.below(most)).map(|_| self.byte()).collect()
        }

        /// A line with a few bytes changed, dropped, added or repeated, or cut short.
        fn spoiled(&mut self, line: &[u8]) -> Vec<u8> {
            let mut line = line.to_vec();
            for _ in 0..=self.below(3) {
                let at = self.below(line.len());
                match self.below(7) {
                    0 if !line.is_empty() => line[at] = self.byte(),
                    1 if !line.is_empty() => {
                        line.remove(at);
                    }
                    2 => line.insert(at, self.byte()),
                    3 => line.truncate(at),
                    4 => {
                        let run = vec![self.byte(); self.below(300)];
                        line.splice(at..at, run);
                    }
                    5 => {
                        const PARTS: [&[u8]; 10] = [
                            b"\\u",
                            b"\\uab\xe2\x82\xac",
                            b"\\ud800",
                            b"NaN",
                            b"1e400",
                            b"[[[[[[[[",
                            b"\"\\",
                            b"\n---\n",
                            b"```",
                            b"> - ",
                        ];
                        let part = PARTS[self.below(PARTS.len())];
                        line.splice(at..at, part.iter().copied());
                    }
                    _ => {
                        let end = (at + self.below(40)).min(line.len());
                        let again = line[at..end].to_vec();
                        line.splice(at..at, again);
                    }
                }
            }
            line
        }
    }

    /// What `Reader` promises of any records: passages in record order, none sharing a record,
    /// all inside the generation, their offsets rising char boundaries of their text.
    fn sound(passages: &[Passage], records: usize, case: usize) {
        let mut next = 1;
        for passage in passages {
            let range = passage.range;
            assert!(
                range.first >= next && range.count > 0,
                "case {case}: {range:?}"
            );
            next = range.first + range.count;
            assert!(
                next - 1 <= records as u64,
                "case {case}: {range:?} of {records}"
            );
            assert_eq!(passage.ends.len() as u64, range.count, "case {case}");
            assert_eq!(
                passage.ends.last(),
                Some(&passage.text.len()),
                "case {case}"
            );
            assert!(passage.ends.is_sorted(), "case {case}");
            let starts = passage.pieces.iter().map(|piece| piece.start);
            assert!(
                passage
                    .ends
                    .iter()
                    .copied()
                    .chain(starts)
                    .all(|at| passage.text.is_char_boundary(at)),
                "case {case}"
            );
            assert!(
                passage.pieces.iter().map(|piece| piece.start).is_sorted(),
                "case {case}"
            );
        }
    }

    #[test]
    fn no_record_makes_the_reader_panic_or_leave_a_line_out() {
        let files = [
            include_str!("../fixtures/sections.md"),
            include_str!("../fixtures/note.md"),
            include_str!("../fixtures/crlf.md"),
            include_str!("../fixtures/bom.md"),
            "- item\n  more\n---\n> quote\nlazy\n===\n```\n# code\n~~~\n",
        ];
        let lines: Vec<&[u8]> = files
            .iter()
            .flat_map(|file| file.as_bytes().split_inclusive(|&byte| byte == b'\n'))
            .collect();
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15);
        let mut labelled = 0;
        for case in 0..4000 {
            let records: Vec<Vec<u8>> = (0..dice.below(12))
                .map(|_| match dice.below(5) {
                    0 => dice.bytes(40),
                    1 | 2 => lines[dice.below(lines.len())].to_vec(),
                    _ => {
                        let line = lines[dice.below(lines.len())];
                        dice.spoiled(line)
                    }
                })
                .collect();
            let records: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
            let found = passages(&records);
            sound(&found, records.len(), case);
            let held: u64 = found.iter().map(|passage| passage.range.count).sum();
            assert_eq!(
                held,
                records.len() as u64,
                "case {case}: every record is in a passage"
            );
            let text: String = found.iter().map(|passage| passage.text.as_str()).collect();
            let whole: String = records
                .iter()
                .map(|record| String::from_utf8_lossy(record))
                .collect();
            assert_eq!(text, whole, "case {case}");
            labelled += found
                .iter()
                .filter(|passage| !passage.pieces[0].label.is_empty())
                .count();
        }
        assert!(
            labelled > 2000,
            "the cases still reach the headings: {labelled}"
        );
    }

    #[test]
    fn a_record_that_is_not_utf8_is_kept_and_never_a_heading() {
        let records: [&[u8]; 3] = [b"# Title\n", b"# \xff\n", b"text\n"];
        let found = passages(&records);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range.count, 3);
        assert_eq!(found[0].text, "# Title\n# \u{fffd}\ntext\n");
    }
}
