use std::time::SystemTime;

use bilbo_entries::day;
use bilbo_entries::tokens::{self, Tokens};

use crate::tries::Extent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub text: String,
    pub at: Option<SystemTime>,
    pub source: Extent,
}

pub struct Format<'a> {
    pub kind: &'a str,
    pub write: &'a dyn Fn(&Paragraph) -> Vec<u8>,
    pub read: &'a dyn Fn(&[u8]) -> Option<Paragraph>,
}

impl std::fmt::Debug for Format<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Format")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Counter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Counter")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

pub struct Counter<'a> {
    pub tokens: &'a dyn Tokens,
    pub bound: usize,
}

fn stretches(text: &str, bound: usize) -> Vec<&str> {
    tokens::stretches(text, bound)
        .into_iter()
        .map(|(from, to)| &text[from..to])
        .collect()
}

impl Counter<'_> {
    #[must_use]
    pub fn count(&self, text: &str) -> usize {
        stretches(text, self.bound)
            .into_iter()
            .map(|stretch| self.tokens.count(stretch))
            .sum()
    }

    #[must_use]
    pub fn head(&self, text: &str, budget: usize) -> usize {
        let mut remaining = budget;
        let mut at = 0;
        for stretch in stretches(text, self.bound) {
            let tokens = self.tokens.count(stretch);
            if tokens > remaining {
                return at + self.tokens.prefix(stretch, remaining);
            }
            remaining -= tokens;
            at += stretch.len();
        }
        at
    }

    #[must_use]
    pub fn tail(&self, text: &str, budget: usize) -> usize {
        let mut remaining = budget;
        let mut start = text.len();
        for stretch in stretches(text, self.bound).into_iter().rev() {
            let ends = self.tokens.ends(stretch);
            if ends.len() > remaining {
                let cut = if remaining == 0 {
                    stretch.len()
                } else {
                    stretch.ceil_char_boundary(ends[ends.len() - remaining - 1])
                };
                return start - stretch.len() + cut;
            }
            remaining -= ends.len();
            start -= stretch.len();
        }
        start
    }
}

#[must_use]
fn locator(first: u64, last: u64) -> String {
    if first == last {
        format!("r{first}")
    } else {
        format!("r{first}-{last}")
    }
}

#[must_use]
pub fn line(first: u64, last: u64, text: &str) -> String {
    format!("#{} {text}", locator(first, last))
}

pub(crate) fn shown(paragraph: &Paragraph) -> String {
    let source = &paragraph.source;
    let place = locator(source.first, source.last);
    let head = match paragraph.at {
        Some(at) => format!("{place} {}", day::utc(at)),
        None => place,
    };
    format!("{head}\n{}\n\n", paragraph.text)
}

#[must_use]
pub fn earlier<'a>(
    paragraphs: &'a [Paragraph],
    budget: usize,
    counter: &Counter<'_>,
) -> &'a [Paragraph] {
    let mut used = 0;
    let mut from = paragraphs.len();
    for paragraph in paragraphs.iter().rev() {
        used += counter.count(&shown(paragraph));
        if used > budget {
            break;
        }
        from -= 1;
    }
    &paragraphs[from..]
}

const TAGS: [&str; 4] = ["<records", "</records", "<earlier_notes", "</earlier_notes"];

fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let begins = TAGS.iter().any(|tag| {
            tail.get(..tag.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(tag))
        });
        out.push_str(if begins { "&lt;" } else { "<" });
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

#[must_use]
pub fn grounds(earlier: &[Paragraph], preface: Option<&str>, records: &str) -> String {
    let mut text: String = earlier.iter().map(shown).collect();
    if let Some(preface) = preface {
        text.push_str(preface.trim_end_matches('\n'));
        text.push('\n');
    }
    text.push_str(records);
    if !records.ends_with('\n') {
        text.push('\n');
    }
    text
}

#[must_use]
pub fn material(earlier: &[Paragraph], preface: Option<&str>, records: &str) -> String {
    let mut text = String::from("<earlier_notes>\n");
    for paragraph in earlier {
        text.push_str(&escaped(&shown(paragraph)));
    }
    text.push_str("</earlier_notes>\n");
    if let Some(preface) = preface {
        text.push_str(&escaped(preface.trim_end_matches('\n')));
        text.push('\n');
    }
    text.push_str("<records>\n");
    text.push_str(&escaped(records));
    if !records.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("</records>\nWrite the notes for the records now, each as a block: its records on the first line, then its paragraph.");
    text
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bilbo_entries::tokens::Estimate;
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{Counter, Paragraph, earlier, grounds, line, material, shown};
    use crate::tries::Extent;

    fn paragraph(parent: &str, first: u64, last: u64, text: &str) -> Paragraph {
        Paragraph {
            text: text.to_owned(),
            at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400)),
            source: Extent {
                parent: Id::parse(parent).expect("id"),
                generation: Generation(1),
                first,
                last,
            },
        }
    }

    const ONE: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
    const TWO: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DC";

    #[test]
    fn a_passage_is_shown_after_its_locator() {
        assert_eq!(line(12, 12, "one\ntwo\nthree"), "#r12 one\ntwo\nthree");
        assert_eq!(line(13, 14, "x"), "#r13-14 x");
    }

    const LAST: &str = "Write the notes for the records now, each as a block: its records on the first line, then its paragraph.";

    #[test]
    fn the_material_is_the_earlier_paragraphs_the_preface_and_the_records() {
        let earlier = [
            paragraph(ONE, 3, 19, "First text."),
            paragraph(ONE, 20, 20, "Second text."),
        ];
        assert_eq!(
            material(&earlier, Some("Preface line."), "#r24 a\n#r25-26 b\n"),
            format!(
                "<earlier_notes>\nr3-19 1970-01-02\nFirst text.\n\nr20 1970-01-02\nSecond text.\n\n</earlier_notes>\nPreface line.\n<records>\n#r24 a\n#r25-26 b\n</records>\n{LAST}"
            )
        );
        assert_eq!(
            material(&[], None, "#r1 a\n"),
            format!("<earlier_notes>\n</earlier_notes>\n<records>\n#r1 a\n</records>\n{LAST}")
        );
        assert!(material(&[], None, "#r1 a").ends_with(&format!("a\n</records>\n{LAST}")));
        assert_eq!(
            material(&[], Some("Preface.\n"), "#r1 a\n"),
            format!(
                "<earlier_notes>\n</earlier_notes>\nPreface.\n<records>\n#r1 a\n</records>\n{LAST}"
            )
        );
    }

    #[test]
    fn a_text_that_holds_a_tag_of_the_frame_cannot_close_it() {
        let earlier = [paragraph(ONE, 1, 2, "note <earlier_notes> here")];
        let made = material(
            &earlier,
            Some("preface </RECORDS >"),
            "#r1 x </records>\n#r2 y <b>\n",
        );
        assert!(made.contains("note &lt;earlier_notes> here"));
        assert!(made.contains("preface &lt;/RECORDS >"));
        assert!(made.contains("x &lt;/records>"));
        assert!(made.contains("y <b>"));
        for tag in [
            "<earlier_notes>",
            "</earlier_notes>",
            "<records>",
            "</records>",
        ] {
            assert_eq!(made.matches(tag).count(), 1, "{tag}");
        }
    }

    #[test]
    fn the_earlier_paragraphs_are_the_newest_that_fit_in_the_answers_form() {
        let counter = Counter {
            tokens: &Estimate,
            bound: 1 << 20,
        };
        let all = [
            paragraph(ONE, 1, 2, "abcd"),
            paragraph(ONE, 3, 4, "efgh"),
            paragraph(ONE, 5, 6, "ijkl"),
        ];
        assert_eq!(counter.count("r1-2 1970-01-02\nabcd\n\n"), 6);
        for (budget, shown) in [(18, 3), (17, 2), (12, 2), (11, 1), (6, 1), (5, 0), (0, 0)] {
            assert_eq!(earlier(&all, budget, &counter).len(), shown, "{budget}");
        }
        let two = earlier(&all, 12, &counter);
        assert_eq!(two[0].source.first, 3);
        assert_eq!(two[1].source.first, 5);
        assert!(
            !material(earlier(&all, 0, &counter), None, "").contains("r1-2"),
            "0 shows none"
        );
        assert!(material(two, None, "").contains("r3-4 1970-01-02\nefgh"));

        let long = [
            paragraph(ONE, 1, 2, &"a".repeat(5986)),
            paragraph(ONE, 3, 4, &"a".repeat(5986)),
            paragraph(ONE, 5, 6, &"a".repeat(5982)),
        ];
        assert_eq!(counter.count(&shown(&long[2])), 1500);
        assert_eq!(counter.count(&shown(&long[1])), 1501);
        assert_eq!(earlier(&long, 3001, &counter), &long[1..]);
        assert_eq!(earlier(&long, 3000, &counter), &long[2..]);
        assert_eq!(earlier(&long, 4502, &counter), &long[..]);
    }

    #[test]
    fn the_material_holds_no_id() {
        let one = [paragraph(ONE, 1, 2, "Same words.")];
        let two = [paragraph(TWO, 1, 2, "Same words.")];
        let made = material(&one, None, "#r1 x\n");
        assert_eq!(made, material(&two, None, "#r1 x\n"));
        assert!(!made.contains(ONE));
    }

    #[test]
    fn a_text_over_the_bound_is_counted_in_the_chunkers_stretches() {
        struct Seen(std::sync::Mutex<Vec<String>>);
        impl bilbo_entries::tokens::Tokens for Seen {
            fn ends(&self, text: &str) -> Vec<usize> {
                vec![text.len()]
            }

            fn count(&self, text: &str) -> usize {
                self.0.lock().expect("a list").push(text.to_owned());
                1
            }
        }
        let text = format!("{}\n{}", "a".repeat(10), "b".repeat(29));
        assert_eq!(text.len(), 40);
        let seen = Seen(std::sync::Mutex::new(Vec::new()));
        let counter = Counter {
            tokens: &seen,
            bound: 32,
        };
        assert_eq!(counter.count(&text), 2);
        assert_eq!(*seen.0.lock().expect("a list"), [&text[..32], &text[32..]]);
    }

    #[test]
    fn the_grounds_are_the_frames_texts_before_the_escape() {
        let earlier = [paragraph(ONE, 1, 2, "note <earlier_notes> here")];
        let preface = Some("preface </RECORDS >");
        let records = "#r1 x </records>\n";
        let held = grounds(&earlier, preface, records);
        assert!(held.contains("note <earlier_notes> here"));
        assert!(held.contains("preface </RECORDS >"));
        assert!(held.contains("x </records>"));
        assert!(!held.contains("&lt;"));
        let sent = material(&earlier, preface, records);
        assert!(sent.contains("&lt;earlier_notes>"));
        assert!(sent.contains("&lt;/RECORDS >"));
        assert!(sent.contains("&lt;/records>"));
    }
}
