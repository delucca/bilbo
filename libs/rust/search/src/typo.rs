use crate::lookup::{Edits, Near};

/// The automaton an index walks costs up to 1 KiB a state and 68 states a letter at two edits.
pub const MAX_LETTERS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub terms: usize,
    pub letters: usize,
    pub two_edits: usize,
}

impl Rule {
    pub const OFF: Rule = Rule {
        terms: 0,
        letters: 4,
        two_edits: 8,
    };

    pub fn edits(&self, term: &str) -> Option<Edits> {
        if self.terms == 0
            || !term.bytes().all(|byte| byte.is_ascii_lowercase())
            || term.len() < self.letters
            || term.len() > MAX_LETTERS
        {
            return None;
        }
        Some(if term.len() >= self.two_edits {
            Edits::Two
        } else {
            Edits::One
        })
    }

    pub fn choose(&self, term: &str, near: Vec<Near>) -> Vec<(String, f32)> {
        let mut kept: Vec<(usize, Near)> = near
            .into_iter()
            .filter(|each| each.term.bytes().all(|byte| byte.is_ascii_lowercase()))
            .filter_map(|each| {
                let apart = distance(term, &each.term);
                (apart >= 1).then_some((apart, each))
            })
            .collect();
        kept.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.documents.cmp(&a.1.documents))
                .then_with(|| a.1.term.cmp(&b.1.term))
        });
        kept.truncate(self.terms);
        let total: u64 = kept.iter().map(|(_, each)| each.documents).sum();
        kept.into_iter()
            .map(|(_, each)| {
                let weight = (each.documents as f64 / total as f64) as f32;
                (each.term, weight)
            })
            .collect()
    }
}

/// Restricted: a swapped pair is not edited again, as the index's automaton counts.
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut before = vec![0; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (previous[j] + 1)
                .min(row[j - 1] + 1)
                .min(previous[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(before[j - 2] + 1);
            }
            row[j] = best;
        }
        before = previous;
        previous = row;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use crate::lookup::{Edits, Near};

    use super::{MAX_LETTERS, Rule, distance};

    const RULE: Rule = Rule {
        terms: 3,
        letters: 4,
        two_edits: 8,
    };

    fn near(terms: &[(&str, u64)]) -> Vec<Near> {
        terms
            .iter()
            .map(|(term, documents)| Near {
                term: (*term).to_owned(),
                documents: *documents,
            })
            .collect()
    }

    fn chosen(rule: &Rule, term: &str, terms: &[(&str, u64)]) -> Vec<(String, f32)> {
        rule.choose(term, near(terms))
    }

    #[test]
    fn a_distance_counts_one_for_each_single_edit_and_for_a_swap_of_neighbours() {
        for (a, b, wanted) in [
            ("check", "ceheck", 1),
            ("check", "chck", 1),
            ("check", "chick", 1),
            ("widht", "width", 1),
            ("ab", "ba", 1),
            ("check", "check", 0),
            ("", "", 0),
            ("", "ab", 2),
            ("kitten", "sitting", 3),
        ] {
            assert_eq!(distance(a, b), wanted, "{a} {b}");
            assert_eq!(distance(b, a), wanted, "{b} {a}");
        }
    }

    #[test]
    fn a_swap_with_a_character_put_between_costs_three() {
        assert_eq!(distance("widht", "widtxh"), 3);
        assert_eq!(distance("widtxh", "widht"), 3);
        assert_eq!(distance("ca", "abc"), 3);
        assert_eq!(distance("abc", "ca"), 3);
    }

    #[test]
    fn a_distance_counts_characters_not_bytes() {
        assert_eq!(distance("café", "cafe"), 1);
        assert_eq!(distance("ação", "acao"), 2);
        assert_eq!(distance("a\u{1f600}b", "a\u{1f601}b"), 1);
        assert_eq!(distance("\u{1f600}", ""), 1);
    }

    #[test]
    fn a_word_is_taken_for_misspelt_by_its_letters_and_its_length() {
        let edits = |rule: &Rule, term: &str| rule.edits(term);
        assert_eq!(edits(&RULE, "abc"), None);
        assert_eq!(edits(&RULE, "abcd"), Some(Edits::One));
        assert_eq!(edits(&RULE, "abcdefg"), Some(Edits::One));
        assert_eq!(edits(&RULE, "abcdefgh"), Some(Edits::Two));
        assert_eq!(edits(&RULE, &"a".repeat(MAX_LETTERS)), Some(Edits::Two));
        assert_eq!(edits(&RULE, &"a".repeat(MAX_LETTERS + 1)), None);
        for term in ["carot1", "car_ot", "Carot", "carót", "ca rot", ""] {
            assert_eq!(edits(&RULE, term), None, "{term}");
        }
        let short = Rule { letters: 2, ..RULE };
        assert_eq!(edits(&short, "ab"), Some(Edits::One));
        assert_eq!(edits(&short, "a"), None);
        let early = Rule {
            two_edits: 5,
            ..RULE
        };
        assert_eq!(edits(&early, "abcd"), Some(Edits::One));
        assert_eq!(edits(&early, "abcde"), Some(Edits::Two));
        let never = Rule {
            two_edits: 65,
            ..RULE
        };
        assert_eq!(edits(&never, &"a".repeat(MAX_LETTERS)), Some(Edits::One));
    }

    #[test]
    fn the_off_rule_takes_no_word_for_misspelt() {
        for term in ["carot", "carotte", "carottes", "carrotsoup"] {
            assert!(RULE.edits(term).is_some(), "{term}");
            assert_eq!(Rule::OFF.edits(term), None, "{term}");
        }
    }

    #[test]
    fn the_nearest_are_kept_by_distance_then_more_chunks_then_term() {
        let kept = chosen(
            &RULE,
            "carot",
            &[
                ("carrot", 2),
                ("carat", 9),
                ("caret", 9),
                ("carotid", 50),
                ("cart", 40),
                ("parrot", 80),
            ],
        );
        let names: Vec<&str> = kept.iter().map(|(term, _)| term.as_str()).collect();
        assert_eq!(names, ["cart", "carat", "caret"]);
        let wide = Rule { terms: 6, ..RULE };
        let kept = chosen(
            &wide,
            "carot",
            &[("carrot", 2), ("carat", 9), ("carotid", 50), ("cart", 40)],
        );
        let names: Vec<&str> = kept.iter().map(|(term, _)| term.as_str()).collect();
        assert_eq!(names, ["cart", "carat", "carrot", "carotid"]);
    }

    #[test]
    fn a_kept_terms_weight_is_its_share_of_the_kept_terms_chunks() {
        let kept = chosen(&RULE, "carot", &[("carrot", 3), ("carat", 1)]);
        assert_eq!(kept[0].0, "carrot");
        assert_eq!(kept[0].1.to_bits(), 0.75_f32.to_bits());
        assert_eq!(kept[1].1.to_bits(), 0.25_f32.to_bits());
        let one = chosen(&RULE, "carot", &[("carrot", 7)]);
        assert_eq!(one[0].1.to_bits(), 1.0_f32.to_bits());
        let two = Rule { terms: 2, ..RULE };
        let cut = chosen(
            &two,
            "carot",
            &[("carrot", 3), ("carat", 1), ("caret", 1), ("carotid", 1000)],
        );
        let by_term: Vec<_> = cut.iter().map(|(term, _)| term.as_str()).collect();
        assert_eq!(by_term, ["carrot", "carat"]);
        assert_eq!(cut[0].1.to_bits(), 0.75_f32.to_bits());
        assert_eq!(cut[1].1.to_bits(), 0.25_f32.to_bits());
    }

    #[test]
    fn the_word_itself_and_a_near_term_that_is_not_all_letters_are_never_kept() {
        let kept = chosen(
            &RULE,
            "test",
            &[("test", 5), ("test1", 5), ("te_st", 5), ("tést", 5)],
        );
        assert_eq!(kept, []);
        let kept = chosen(&RULE, "test", &[("test", 5), ("tests", 1), ("test1", 5)]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, "tests");
        assert_eq!(kept[0].1.to_bits(), 1.0_f32.to_bits());
    }
}
