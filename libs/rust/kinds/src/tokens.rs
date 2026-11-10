/// Counts what a budget is spent on.
pub trait Tokens {
    fn count(&self, text: &str) -> usize;
    /// Bytes of the longest prefix of `text` within `budget` tokens, on a char boundary.
    fn prefix(&self, text: &str, budget: usize) -> usize;
}

/// Four bytes a token, rounded up.
#[derive(Debug, Clone, Copy, Default)]
pub struct Estimate;

const BYTES: usize = 4;

impl Tokens for Estimate {
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
    fn a_prefix_never_counts_more_than_its_budget() {
        let text = "one two três quatro 五 six";
        for budget in 0..10 {
            let prefix = &text[..Estimate.prefix(text, budget)];
            assert!(Estimate.count(prefix) <= budget, "{budget}: {prefix:?}");
        }
    }
}
