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
    use super::{Estimate, Tokens};

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
