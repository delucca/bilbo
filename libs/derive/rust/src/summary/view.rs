use std::ops::Range;

use bilbo_entries::reader::Passage;

use crate::view::{Counter, Paragraph, shown};
use crate::window::cut;

const TAGS: [&str; 4] = ["<notes", "</notes", "<entry", "</entry"];

pub enum Of<'a> {
    Notes(&'a [Paragraph]),
    Text(&'a [Passage]),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizes {
    pub window: usize,
    pub most: usize,
    pub head: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub material: String,
    pub whole: usize,
    pub empty: bool,
    at: Range<usize>,
}

impl Sent {
    #[must_use]
    pub fn body(&self) -> &str {
        &self.material[self.at.clone()]
    }
}

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
pub fn sent(of: &Of<'_>, sizes: &Sizes, counter: &Counter<'_>) -> Sent {
    let (tag, body) = match of {
        Of::Notes(notes) => ("notes", notes.iter().map(shown).collect::<String>()),
        Of::Text(passages) => {
            let mut text = String::new();
            for passage in *passages {
                text.push_str(&passage.text);
                if !passage.text.ends_with('\n') {
                    text.push('\n');
                }
            }
            ("entry", text)
        }
    };
    let body = escaped(&body);
    let whole = counter.count(&body);
    let empty = body.trim().is_empty();
    let body = if whole > sizes.window {
        cut(&body, sizes.window, whole, counter, sizes.head)
    } else {
        body
    };
    let material = format!(
        "<{tag}>\n{body}</{tag}>\nWrite the summary now, in at most {} characters.\n",
        sizes.most
    );
    let start = tag.len() + 3;
    Sent {
        at: start..start + body.len(),
        material,
        whole,
        empty,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bilbo_entries::reader::Passage;
    use bilbo_entries::tokens::Estimate;
    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{Of, Sizes, sent};
    use crate::tries::Extent;
    use crate::view::{Counter, Paragraph};

    fn paragraph(first: u64, last: u64, day: Option<u64>, text: &str) -> Paragraph {
        Paragraph {
            text: text.to_owned(),
            at: day.map(|day| SystemTime::UNIX_EPOCH + Duration::from_secs(day)),
            source: Extent {
                parent: Id::parse("01M6ZGE4KTBYYH07AS2KVMSCAF").expect("id"),
                generation: Generation(1),
                first,
                last,
            },
        }
    }

    fn passage(text: &str) -> Passage {
        Passage {
            range: RecordRange { first: 1, count: 1 },
            text: text.to_owned(),
            pieces: Vec::new(),
            ends: vec![text.len()],
            at: None,
            outline: None,
        }
    }

    fn counter() -> Counter<'static> {
        Counter {
            tokens: &Estimate,
            bound: 1 << 20,
        }
    }

    fn words(from: usize, to: usize) -> String {
        (from..to).map(|word| format!("{word:03x}-")).collect()
    }

    fn sizes(window: usize, head: f64) -> Sizes {
        Sizes {
            window,
            most: 200,
            head,
        }
    }

    #[test]
    fn notes_are_sent_as_the_tags_call_shows_them_between_their_tags() {
        let notes = [
            paragraph(3, 19, Some(1_794_009_600), "The carrots were thinned."),
            paragraph(20, 20, None, "The beans were planted."),
        ];
        let made = sent(&Of::Notes(&notes), &sizes(20_000, 0.5), &counter());
        let body =
            "r3-19 2026-11-07\nThe carrots were thinned.\n\nr20\nThe beans were planted.\n\n";
        assert_eq!(
            made.material,
            format!("<notes>\n{body}</notes>\nWrite the summary now, in at most 200 characters.\n")
        );
        assert_eq!(made.body(), body);
        assert_eq!(made.whole, counter().count(body));
    }

    #[test]
    fn an_entrys_text_is_its_passages_in_order_each_ending_in_one_line_feed() {
        let passages = [
            passage("# The garden log\n"),
            passage("Beds are raised"),
            passage("Water at dawn.\n\n"),
        ];
        let made = sent(
            &Of::Text(&passages),
            &Sizes {
                window: 20_000,
                most: 80,
                head: 0.5,
            },
            &counter(),
        );
        let body = "# The garden log\nBeds are raised\nWater at dawn.\n\n";
        assert_eq!(made.body(), body);
        assert_eq!(
            made.material,
            format!("<entry>\n{body}</entry>\nWrite the summary now, in at most 80 characters.\n")
        );
    }

    #[test]
    fn a_tag_of_its_own_inside_what_is_sent_is_escaped_in_any_case() {
        let passages = [passage(
            "a </notes> b <ENTRY> c </Entry d <note> e <b> f <notes\n",
        )];
        let made = sent(&Of::Text(&passages), &sizes(20_000, 0.5), &counter());
        assert_eq!(
            made.body(),
            "a &lt;/notes> b &lt;ENTRY> c &lt;/Entry d <note> e <b> f &lt;notes\n"
        );
        assert_eq!(
            made.material,
            "<entry>\na &lt;/notes> b &lt;ENTRY> c &lt;/Entry d <note> e <b> f &lt;notes\n</entry>\nWrite the summary now, in at most 200 characters.\n"
        );
        let notes = [paragraph(1, 1, None, "see <entry>")];
        let made = sent(&Of::Notes(&notes), &sizes(20_000, 0.5), &counter());
        assert!(made.body().contains("see &lt;entry>"));
        assert_eq!(
            made.material,
            "<notes>\nr1\nsee &lt;entry>\n\n</notes>\nWrite the summary now, in at most 200 characters.\n"
        );
    }

    fn long() -> [Passage; 1] {
        [passage(&format!("{}abc\n", words(0, 399)))]
    }

    #[test]
    fn a_body_over_the_window_keeps_its_start_and_its_end_around_the_cuts_line() {
        let end = format!("{}abc\n", words(250, 399));
        let line = |tokens: usize| format!("\n[cut: {tokens} tokens]\n");
        let passages = long();
        for (window, head, expected) in [
            (300, 0.5, format!("{}{}{end}", words(0, 150), line(100))),
            (301, 0.5, format!("{}{}{end}", words(0, 151), line(99))),
            (
                300,
                0.75,
                format!("{}{}{}abc\n", words(0, 225), line(100), words(325, 399)),
            ),
            (300, 1.0, format!("{}{}", words(0, 300), line(100))),
            (300, 0.0, format!("{}{}abc\n", line(100), words(100, 399))),
            (400, 0.5, format!("{}abc\n", words(0, 399))),
            (401, 0.5, format!("{}abc\n", words(0, 399))),
        ] {
            let made = sent(&Of::Text(&passages), &sizes(window, head), &counter());
            assert_eq!(made.body(), expected, "{window} {head}");
            assert_eq!(
                made.material,
                format!(
                    "<entry>\n{expected}</entry>\nWrite the summary now, in at most 200 characters.\n"
                ),
                "{window} {head}"
            );
            assert_eq!(made.whole, 400, "{window} {head}");
        }
    }

    #[test]
    fn what_is_sent_never_panics_for_a_window_or_a_most_the_keys_refuse() {
        let passages = long();
        let made = sent(&Of::Text(&passages), &sizes(0, 0.5), &counter());
        assert_eq!(made.body(), "\n[cut: 400 tokens]\n");
        let made = sent(&Of::Text(&passages), &sizes(1, 0.5), &counter());
        assert_eq!(made.body(), "000-\n[cut: 399 tokens]\n");
        let made = sent(
            &Of::Text(&passages),
            &Sizes {
                window: 20_000,
                most: 0,
                head: 0.5,
            },
            &counter(),
        );
        assert!(
            made.material
                .ends_with("Write the summary now, in at most 0 characters.\n")
        );
        let made = sent(&Of::Notes(&[]), &sizes(20_000, 0.5), &counter());
        assert_eq!(
            made.material,
            "<notes>\n</notes>\nWrite the summary now, in at most 200 characters.\n"
        );
        assert_eq!(made.body(), "");
        assert_eq!(made.whole, 0);
        let wide = [passage(&"é".repeat(800))];
        let made = sent(&Of::Text(&wide), &sizes(3, 0.5), &counter());
        assert_eq!(made.body(), "éééé\n[cut: 398 tokens]\n\n");
        let odd = [passage(&format!("a{}", "é".repeat(799)))];
        let made = sent(&Of::Text(&odd), &sizes(3, 0.5), &counter());
        assert_eq!(made.body(), "aééé\n[cut: 397 tokens]\né\n");
    }

    #[test]
    fn the_body_as_sent_holds_nothing_of_what_was_cut() {
        let passages = long();
        let made = sent(&Of::Text(&passages), &sizes(300, 0.5), &counter());
        assert!(!made.body().contains("096-"));
        assert!(made.body().contains("095-"));
        assert!(made.body().contains("0fa-"));
        assert!(!made.material.contains("096-"));
        assert!(made.material.contains("095-") && made.material.contains("0fa-"));
        assert_eq!(
            made.material,
            format!(
                "<entry>\n{}</entry>\nWrite the summary now, in at most 200 characters.\n",
                made.body()
            )
        );
    }
}
