pub trait Tokens: Sync {
    /// Where each token ends in the text's bytes, in order: never falling, the last `text.len()`.
    /// An end may fall inside a character.
    /// A tokenizer that cannot split a text gives no end for it, and only an empty text has no token.
    fn ends(&self, text: &str) -> Vec<usize>;

    fn count(&self, text: &str) -> usize {
        self.ends(text).len()
    }

    /// Bytes of the longest prefix of `text` within `budget` tokens, on a char boundary.
    fn prefix(&self, text: &str, budget: usize) -> usize {
        let ends = self.ends(text);
        if budget >= ends.len() {
            return text.len();
        }
        let end = budget.checked_sub(1).map_or(0, |last| ends[last]);
        text.floor_char_boundary(end.min(text.len()))
    }
}

pub fn stretches(text: &str, bound: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if text.len() - at <= bound {
            found.push((at, text.len()));
            break;
        }
        let hard = text.floor_char_boundary(at + bound);
        let half = at + bound / 2;
        let mut end = text.as_bytes()[half..hard]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(hard, |line_feed| half + line_feed + 1);
        if end == at {
            end = at + text[at..].chars().next().map_or(1, char::len_utf8);
        }
        found.push((at, end));
        at = end;
    }
    found
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Estimate;

const BYTES: usize = 4;

impl Tokens for Estimate {
    fn ends(&self, text: &str) -> Vec<usize> {
        (1..=text.len().div_ceil(BYTES))
            .map(|token| (token * BYTES).min(text.len()))
            .collect()
    }

    fn count(&self, text: &str) -> usize {
        text.len().div_ceil(BYTES)
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        text.floor_char_boundary(budget.saturating_mul(BYTES).min(text.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Estimate, Tokens, stretches};

    #[test]
    fn the_stretches_of_a_text_over_the_bound_end_at_line_feeds() {
        let text: String = (0..10).map(|line| format!("line {line:04}xx\n")).collect();
        assert_eq!(
            stretches(&text, 32),
            (0..5).map(|at| (at * 24, at * 24 + 24)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_stretch_with_no_line_feed_ends_on_a_character() {
        let text = "é".repeat(100);
        let found = stretches(&text, 33);
        assert_eq!(found.first().map(|stretch| stretch.0), Some(0));
        assert_eq!(found.last().map(|stretch| stretch.1), Some(200));
        for (first, last) in &found {
            assert!(text.is_char_boundary(*first) && text.is_char_boundary(*last));
            assert!((1..=33).contains(&(last - first)));
        }
        assert!(found.windows(2).all(|pair| pair[0].1 == pair[1].0));
    }

    #[test]
    fn a_line_feed_in_the_first_half_of_a_stretch_does_not_end_it() {
        let text = format!("aaaa\n{}", "b".repeat(35));
        assert_eq!(stretches(&text, 32), [(0, 32), (32, 40)]);
        let late = format!("{}\n{}", "a".repeat(20), "b".repeat(30));
        assert_eq!(stretches(&late, 32), [(0, 21), (21, 51)]);
    }

    #[test]
    fn a_token_is_four_bytes_rounded_up() {
        for (text, tokens) in [
            ("", 0),
            ("a", 1),
            ("abcd", 1),
            ("abcde", 2),
            ("é", 1),
            ("ééé", 2),
        ] {
            assert_eq!(Estimate.count(text), tokens, "{text:?}");
        }
    }

    #[test]
    fn a_prefix_is_the_most_bytes_within_the_budget_on_a_char_boundary() {
        let text = "abcdéfgh";
        for (budget, bytes) in [(0, 0), (1, 4), (2, 8), (3, 9), (usize::MAX, 9)] {
            assert_eq!(Estimate.prefix(text, budget), bytes, "{budget}");
        }
        // The second byte of `é` is byte 5: a budget that ends inside it stops before it.
        assert_eq!(Estimate.prefix("abcé", 1), 3);
        assert_eq!(Estimate.prefix("", 5), 0);
    }

    #[test]
    fn the_estimate_counts_and_cuts_as_its_ends_say() {
        struct Ends;
        impl Tokens for Ends {
            fn ends(&self, text: &str) -> Vec<usize> {
                Estimate.ends(text)
            }
        }
        for text in [
            "",
            "a",
            "abcd",
            "abcde",
            "abcdéfgh",
            "ééé",
            "one two três quatro 五 six",
        ] {
            let ends = Estimate.ends(text);
            assert_eq!(ends.last().copied().unwrap_or(0), text.len(), "{text:?}");
            assert!(ends.windows(2).all(|pair| pair[0] < pair[1]), "{text:?}");
            assert_eq!(Estimate.count(text), Ends.count(text), "{text:?}");
            for budget in 0..10 {
                assert_eq!(
                    Estimate.prefix(text, budget),
                    Ends.prefix(text, budget),
                    "{text:?} {budget}"
                );
            }
        }
    }

    #[test]
    fn a_prefix_never_counts_more_than_its_budget() {
        let text = "one two três quatro 五 six";
        for budget in 0..10 {
            let prefix = &text[..Estimate.prefix(text, budget)];
            assert!(Estimate.count(prefix) <= budget, "{budget}: {prefix:?}");
        }
    }
}
