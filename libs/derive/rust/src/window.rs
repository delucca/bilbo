use std::time::SystemTime;

use bilbo_entries::reader::Passage;
use bilbo_store::entries::RecordRange;

use crate::view::{Counter, line};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizes {
    pub window: usize,
    pub passage: usize,
    pub head: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub range: RecordRange,
    pub at: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub first: u64,
    pub last: u64,
    pub text: String,
    pub sent: usize,
    pub passages: Vec<Sent>,
}

struct Sending {
    sent: Sent,
    text: String,
    tokens: usize,
}

fn end(range: RecordRange) -> u64 {
    range.first + range.count.saturating_sub(1)
}

pub(crate) fn cut(
    text: &str,
    kept: usize,
    total: usize,
    counter: &Counter<'_>,
    share: f64,
) -> String {
    let first = ((kept as f64 * share).ceil() as usize).min(kept);
    let head = counter.head(text, first);
    let tail = counter.tail(text, kept - first).max(head);
    format!(
        "{}\n[cut: {} tokens]\n{}",
        &text[..head],
        total - kept,
        &text[tail..]
    )
}

fn rendered(
    passage: &Passage,
    kept: Option<usize>,
    total: usize,
    counter: &Counter<'_>,
    share: f64,
) -> String {
    let text = match kept {
        None => passage.text.clone(),
        Some(kept) => cut(&passage.text, kept, total, counter, share),
    };
    let mut shown = line(passage.range.first, end(passage.range), &text);
    if !shown.ends_with('\n') {
        shown.push('\n');
    }
    shown
}

fn sending(passage: &Passage, counter: &Counter<'_>, sizes: &Sizes) -> Sending {
    let total = counter.count(&passage.text);
    let mut kept = (sizes.passage > 0 && total > sizes.passage).then_some(sizes.passage);
    let mut text = rendered(passage, kept, total, counter, sizes.head);
    let mut tokens = counter.count(&text);
    if tokens > sizes.window {
        let fits =
            |size: usize| counter.count(&rendered(passage, Some(size), total, counter, sizes.head));
        let mut low = 0;
        let mut high = kept.unwrap_or(total).min(total.saturating_sub(1));
        while low < high {
            let middle = (low + high).div_ceil(2);
            if fits(middle) <= sizes.window {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        kept = Some(low);
        text = rendered(passage, kept, total, counter, sizes.head);
        tokens = counter.count(&text);
    }
    Sending {
        sent: Sent {
            range: passage.range,
            at: passage.at,
        },
        text,
        tokens,
    }
}

#[must_use]
pub fn windows(
    passages: &[Passage],
    frontier: u64,
    last_record: u64,
    counter: &Counter<'_>,
    sizes: &Sizes,
) -> Vec<Window> {
    let mut groups: Vec<Vec<Sending>> = Vec::new();
    let mut sent = 0;
    for passage in passages
        .iter()
        .filter(|passage| end(passage.range) > frontier)
    {
        let next = sending(passage, counter, sizes);
        match groups.last_mut() {
            Some(group) if sent + next.tokens <= sizes.window => {
                sent += next.tokens;
                group.push(next);
            }
            _ => {
                sent = next.tokens;
                groups.push(vec![next]);
            }
        }
    }
    let firsts: Vec<u64> = groups
        .iter()
        .map(|group| group[0].sent.range.first)
        .collect();
    let mut first = frontier + 1;
    groups
        .into_iter()
        .enumerate()
        .map(|(at, group)| {
            let last = firsts.get(at + 1).map_or(last_record, |next| next - 1);
            let window = Window {
                first,
                last,
                sent: group.iter().map(|sending| sending.tokens).sum(),
                text: group.iter().map(|sending| sending.text.as_str()).collect(),
                passages: group.iter().map(|sending| sending.sent).collect(),
            };
            first = last + 1;
            window
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use bilbo_entries::reader::Passage;
    use bilbo_entries::tokens::{Estimate, Tokens};
    use bilbo_store::entries::RecordRange;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Sizes, windows};
    use crate::view::Counter;

    fn passage(first: u64, count: u64, text: &str) -> Passage {
        Passage {
            range: RecordRange { first, count },
            text: text.to_owned(),
            pieces: Vec::new(),
            ends: vec![text.len()],
            at: None,
            outline: None,
        }
    }

    fn sized(first: u64, tokens: usize) -> Passage {
        let locator = format!("#r{first} ").len() + 1;
        passage(first, 1, &"x".repeat(4 * tokens - locator))
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

    const BIG: Sizes = Sizes {
        window: 1_000_000,
        passage: 0,
        head: 0.5,
    };

    #[test]
    fn the_cut_of_a_text_is_its_start_the_line_and_its_end() {
        let text = words(0, 400);
        let end = |from| words(from, 400);
        assert_eq!(
            super::cut(&text, 300, 400, &counter(), 0.5),
            format!("{}\n[cut: 100 tokens]\n{}", words(0, 150), end(250))
        );
        assert_eq!(
            super::cut(&text, 301, 400, &counter(), 0.5),
            format!("{}\n[cut: 99 tokens]\n{}", words(0, 151), end(250))
        );
    }

    #[test]
    fn a_window_is_whole_passages_in_order_up_to_the_window() {
        let mut passages: Vec<Passage> = (1..=19).map(|first| sized(first, 1000)).collect();
        passages.push(sized(20, 1001));
        let sizes = |window| Sizes {
            window,
            passage: 0,
            head: 0.5,
        };
        let apart = windows(&passages, 0, 20, &counter(), &sizes(20_000));
        assert_eq!(
            apart
                .iter()
                .map(|window| (
                    window.first,
                    window.last,
                    window.sent,
                    window.passages.len()
                ))
                .collect::<Vec<_>>(),
            [(1, 19, 19_000, 19), (20, 20, 1001, 1)]
        );
        let together = windows(&passages, 0, 20, &counter(), &sizes(20_001));
        assert_eq!(
            together
                .iter()
                .map(|window| (
                    window.first,
                    window.last,
                    window.sent,
                    window.passages.len()
                ))
                .collect::<Vec<_>>(),
            [(1, 20, 20_001, 20)]
        );
    }

    #[test]
    fn a_passage_is_never_split_between_windows() {
        let passages: Vec<Passage> = (1..=3).map(|first| sized(first, 60)).collect();
        let sizes = Sizes {
            window: 100,
            passage: 0,
            head: 0.5,
        };
        let found = windows(&passages, 0, 3, &counter(), &sizes);
        assert_eq!(found.len(), 3);
        for (window, first) in found.iter().zip(1..) {
            assert_eq!(window.text, format!("#r{first} {}\n", "x".repeat(235)));
            assert_eq!(window.sent, 60);
        }
    }

    #[test]
    fn a_passage_over_the_passage_size_is_sent_as_its_start_and_its_end() {
        let text = words(0, 400);
        assert_eq!(Estimate.count(&text), 400);
        let passages = [passage(1, 1, &text)];
        for (size, share, head, tail, cut) in [
            (300, 0.5, 150, 150, 100),
            (301, 0.5, 151, 150, 99),
            (300, 0.75, 225, 75, 100),
        ] {
            let sizes = Sizes {
                window: 1_000_000,
                passage: size,
                head: share,
            };
            let found = windows(&passages, 0, 1, &counter(), &sizes);
            assert_eq!(
                found[0].text,
                format!(
                    "#r1 {}\n[cut: {cut} tokens]\n{}\n",
                    words(0, head),
                    words(400 - tail, 400)
                ),
                "{size} {share}"
            );
        }
        let whole = windows(
            &passages,
            0,
            1,
            &counter(),
            &Sizes {
                window: 1_000_000,
                passage: 400,
                head: 0.5,
            },
        );
        assert_eq!(whole[0].text, format!("#r1 {text}\n"));
    }

    #[test]
    fn a_passage_size_of_0_sends_every_passage_whole() {
        let text = words(0, 400);
        let found = windows(&[passage(1, 1, &text)], 0, 1, &counter(), &BIG);
        assert_eq!(found[0].text, format!("#r1 {text}\n"));
    }

    #[test]
    fn a_passage_that_alone_passes_the_window_is_cut_to_it() {
        let text = words(0, 4000);
        let sizes = Sizes {
            window: 1000,
            passage: 0,
            head: 0.5,
        };
        let found = windows(&[passage(1, 1, &text)], 0, 1, &counter(), &sizes);
        let tokens = Estimate.count(&text);
        let cut = |kept: usize| {
            let head = 4 * (kept - kept / 2);
            let tail = 4 * (kept / 2);
            format!(
                "#r1 {}\n[cut: {} tokens]\n{}\n",
                &text[..head],
                tokens - kept,
                &text[text.len() - tail..]
            )
        };
        let best = (0..tokens)
            .rev()
            .find(|&kept| Estimate.count(&cut(kept)) <= 1000)
            .expect("a cut fits");
        assert_eq!(found[0].text, cut(best));
        assert!(Estimate.count(&found[0].text) <= 1000);
        assert!(Estimate.count(&cut(best + 1)) > 1000);
        assert_eq!(found[0].sent, Estimate.count(&found[0].text));
    }

    #[test]
    fn sent_counts_the_lines_as_sent_and_nothing_else() {
        let text = words(0, 400);
        let passages = [passage(1, 1, "short"), passage(2, 3, &text)];
        let sizes = Sizes {
            window: 1_000_000,
            passage: 100,
            head: 0.5,
        };
        let found = windows(&passages, 0, 4, &counter(), &sizes);
        assert_eq!(found.len(), 1);
        let lines = ["#r1 short\n".to_owned(), found[0].text[10..].to_owned()];
        assert!(lines[1].starts_with("#r2-4 "));
        assert!(lines[1].contains("[cut: "));
        assert_eq!(
            found[0].sent,
            Estimate.count(&lines[0]) + Estimate.count(&lines[1])
        );
    }

    #[test]
    fn only_passages_that_end_after_the_frontier_are_sent() {
        let passages = [
            passage(1, 1, "one"),
            passage(2, 3, "two to four"),
            passage(5, 1, "five"),
        ];
        let found = windows(&passages, 3, 5, &counter(), &BIG);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "#r2-4 two to four\n#r5 five\n");
        assert_eq!((found[0].first, found[0].last), (4, 5));
        assert_eq!(windows(&passages, 5, 5, &counter(), &BIG), []);
    }

    #[test]
    fn a_window_covers_up_to_the_record_before_the_next_windows_first_passage() {
        let passages = [
            passage(1, 1, "a"),
            passage(2, 1, "b"),
            passage(5, 1, "c"),
            passage(6, 1, "d"),
        ];
        let sizes = Sizes {
            window: 4,
            passage: 0,
            head: 0.5,
        };
        let found = windows(&passages, 0, 8, &counter(), &sizes);
        assert_eq!(
            found
                .iter()
                .map(|window| (window.first, window.last))
                .collect::<Vec<_>>(),
            [(1, 4), (5, 8)]
        );
        let after = windows(&passages, 2, 8, &counter(), &sizes);
        assert_eq!((after[0].first, after[0].last), (3, 8));
    }

    #[test]
    fn the_window_with_the_last_passage_covers_to_the_entrys_last_record() {
        let passages = [passage(1, 1, "a"), passage(2, 1, "b")];
        let found = windows(&passages, 0, 40, &counter(), &BIG);
        assert_eq!((found[0].first, found[0].last), (1, 40));
        assert_eq!(found[0].passages.len(), 2);
    }

    struct Longest(AtomicUsize);

    impl Tokens for Longest {
        fn ends(&self, text: &str) -> Vec<usize> {
            self.0.fetch_max(text.len(), Ordering::Relaxed);
            Estimate.ends(text)
        }

        fn count(&self, text: &str) -> usize {
            self.0.fetch_max(text.len(), Ordering::Relaxed);
            Estimate.count(text)
        }

        fn prefix(&self, text: &str, budget: usize) -> usize {
            self.0.fetch_max(text.len(), Ordering::Relaxed);
            Estimate.prefix(text, budget)
        }
    }

    #[test]
    fn no_text_over_the_bound_is_handed_to_the_tokenizer_at_once() {
        let longest = Longest(AtomicUsize::new(0));
        let counter = Counter {
            tokens: &longest,
            bound: 64,
        };
        let text = format!("{}\n{}", "a".repeat(100), words(0, 200));
        let sizes = Sizes {
            window: 1000,
            passage: 100,
            head: 0.5,
        };
        let found = windows(&[passage(1, 1, &text)], 0, 1, &counter, &sizes);
        let seen = longest.0.load(Ordering::Relaxed);
        assert!(seen <= 64, "{seen}");
        assert!(found[0].text.contains("[cut: "));
    }
}
