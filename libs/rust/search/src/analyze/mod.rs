mod word;

use std::sync::LazyLock;

use frostem::{Algorithm, Stemmer};

/// bm25s's English list.
const STOPWORDS: [&str; 33] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into", "is", "it",
    "no", "not", "of", "on", "or", "such", "that", "the", "their", "then", "there", "these",
    "they", "this", "to", "was", "will", "with",
];

/// The longest word or term, in bytes, that an index holds: tantivy drops a longer token without a
/// word. A longer word is in no document and in no query.
pub const MAX_TERM_BYTES: usize = 4096;

/// Whether Python's `\w` takes `c`: a letter, a number or `_`, as the Unicode of `word.rs` has them.
fn is_word(c: char) -> bool {
    let c = u32::from(c);
    word::WORD
        .binary_search_by(|&(first, last)| {
            if last < c {
                std::cmp::Ordering::Less
            } else if first > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Python's `str.lower`, for a text whose words are then split by `is_word`. A character that is
/// no word character there and lowercases into one here is newer than that Unicode: it stays as it is.
fn lower(text: &str) -> String {
    let newer = |c: char| !is_word(c) && c.to_lowercase().any(is_word);
    if text.chars().any(newer) {
        let kept: String = text
            .chars()
            .map(|c| {
                if newer(c) {
                    char::REPLACEMENT_CHARACTER
                } else {
                    c
                }
            })
            .collect();
        kept.to_lowercase()
    } else {
        text.to_lowercase()
    }
}

/// As bm25s splits with `\b\w\w+\b`, lowercased, without its stopwords.
fn words(text: &str) -> Vec<String> {
    let lowered = lower(text);
    let mut words = Vec::new();
    let mut start = None;
    let mut chars = 0;
    for (at, c) in lowered.char_indices().chain([(lowered.len(), ' ')]) {
        if is_word(c) {
            start.get_or_insert(at);
            chars += 1;
            continue;
        }
        if let Some(from) = start.take() {
            let word = &lowered[from..at];
            if chars >= 2 && word.len() <= MAX_TERM_BYTES && !STOPWORDS.contains(&word) {
                words.push(word.to_owned());
            }
        }
        chars = 0;
    }
    words
}

pub fn stem(word: &str) -> String {
    static ENGLISH: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::new(Algorithm::English));
    ENGLISH.stem(word).into_owned()
}

pub fn terms(text: &str) -> Vec<String> {
    words(text)
        .iter()
        .map(|word| stem(word))
        .filter(|term| term.len() <= MAX_TERM_BYTES)
        .collect()
}

pub fn query(text: &str, dropped: &[String]) -> Vec<String> {
    words(text)
        .iter()
        .filter(|word| !dropped.contains(word))
        .map(|word| stem(word))
        .filter(|term| term.len() <= MAX_TERM_BYTES)
        .collect()
}

pub fn word(text: &str) -> Option<String> {
    match words(text).as_slice() {
        [only] if only == text => Some(text.to_owned()),
        _ => None,
    }
}

/// Names these rules in the index's manifest. Change it with any of them, the stemmer's version included.
pub const ID: &str = "bm25s-en/2";

#[cfg(test)]
mod tests {
    use super::{MAX_TERM_BYTES, STOPWORDS, is_word, lower, query, stem, terms, word, words};

    #[test]
    fn a_word_is_two_or_more_letters_numbers_or_underscores() {
        for (text, found) in [
            ("", &[][..]),
            ("x", &[]),
            ("x y z", &[]),
            ("ab", &["ab"]),
            ("ab cd", &["ab", "cd"]),
            ("x_y", &["x_y"]),
            ("__", &["__"]),
            ("_", &[]),
            ("a1 22 3", &["a1", "22"]),
            ("kebab-case", &["kebab", "case"]),
            ("don't", &["don"]),
            ("file.rs:42", &["file", "rs", "42"]),
            ("foo::bar(&baz)?", &["foo", "bar", "baz"]),
            ("日本語 五", &["日本語"]),
            ("x² ½⅓ Ⅷ", &["x²", "½⅓"]),
        ] {
            assert_eq!(words(text), found, "{text:?}");
        }
    }

    #[test]
    fn a_word_is_lowercased_before_it_is_split_or_compared() {
        assert_eq!(
            words("CamelCase ÉCOLE Straße ǅungla"),
            ["camelcase", "école", "straße", "ǆungla"]
        );
        assert_eq!(words("THE The tHe NOT No"), Vec::<String>::new());
    }

    #[test]
    fn a_mark_is_no_word_character_so_a_decomposed_letter_splits_its_word() {
        assert_eq!(words("café"), ["café"]);
        assert_eq!(words("cafe\u{301}s"), ["cafe"]);
        assert_eq!(words("nai\u{308}ve"), ["nai", "ve"]);
        assert_eq!(words("ab\u{200d}cd ab\u{ad}cd"), ["ab", "cd", "ab", "cd"]);
        // Python lowers İ to i and a combining dot, which then ends the word.
        assert_eq!(words("İstanbul"), ["stanbul"]);
        assert_eq!(words("aİb"), ["ai"]);
    }

    #[test]
    fn a_final_sigma_is_lowered_as_python_lowers_it() {
        assert_eq!(words("ΑΣ ΑΣΑ ΌΣΟΣ"), ["ας", "ασα", "όσος"]);
        assert_eq!(lower("ΑΣ'Α Σ"), "ασ'α σ");
    }

    #[test]
    fn a_character_newer_than_the_table_is_no_word_character_whatever_it_lowercases_to() {
        // U+1C89 and U+A7CB came with Unicode 16; the second lowercases to U+0264, a letter since 1.1.
        assert!(!is_word('\u{1c89}') && !is_word('\u{a7cb}') && is_word('\u{264}'));
        assert_eq!(
            words("n\u{1c89} a\u{a7cb} x\u{a7cb}x \u{264}\u{264}"),
            ["\u{264}\u{264}"]
        );
        assert_eq!(lower("A\u{a7cb}"), "a\u{fffd}");
    }

    #[test]
    fn the_table_is_rising_ranges_that_do_not_touch() {
        assert!(word::WORD.iter().all(|(first, last)| first <= last));
        assert!(word::WORD.windows(2).all(|pair| pair[0].1 + 1 < pair[1].0));
        for (c, expected) in [
            ('0', true),
            ('9', true),
            ('A', true),
            ('z', true),
            ('_', true),
            ('/', false),
            (':', false),
            ('@', false),
            ('`', false),
            ('{', false),
            ('\u{aa}', true),
            ('\u{b2}', true),
            ('\u{d7}', false),
            ('\u{300}', false),
            ('\u{2028}', false),
            ('\u{10ffff}', false),
        ] {
            assert_eq!(is_word(c), expected, "{c:?}");
        }
    }

    #[test]
    fn the_stopwords_are_bm25s_33_and_are_dropped_before_stemming() {
        assert_eq!(STOPWORDS.len(), 33);
        assert!(STOPWORDS.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            terms("the and of to in is it was will with"),
            Vec::<String>::new()
        );
        // "ands" is no stopword though it stems to one; "was" is one though "wa" is not.
        assert_eq!(terms("ands was notes"), ["and", "note"]);
    }

    #[test]
    fn a_word_is_stemmed_as_snowball_3_stems_it() {
        for (word, stemmed) in [
            ("running", "run"),
            ("easily", "easili"),
            ("relational", "relat"),
            ("added", "add"),
            ("evening", "evening"),
            ("emergency", "emergenc"),
            ("internal", "internal"),
            ("horologist", "horolog"),
            ("skies", "sky"),
            ("école", "école"),
            ("x_y", "x_i"),
        ] {
            assert_eq!(stem(word), stemmed, "{word}");
        }
    }

    #[test]
    fn a_word_over_the_longest_term_is_dropped_and_one_of_exactly_that_length_is_kept() {
        for (letter, long) in [("x", MAX_TERM_BYTES), ("é", MAX_TERM_BYTES / 2)] {
            let kept = letter.repeat(long);
            let dropped = letter.repeat(long + 1);
            assert_eq!(kept.len(), MAX_TERM_BYTES);
            assert!(dropped.len() > MAX_TERM_BYTES);
            let text = format!("alpha {kept} beta {dropped} gamma");
            assert_eq!(words(&text), ["alpha", kept.as_str(), "beta", "gamma"]);
            let found = terms(&text);
            assert_eq!(found.len(), 4);
            assert!(found.iter().all(|term| term.len() <= MAX_TERM_BYTES));
            assert_eq!(found[1], stem(&kept));
        }
        assert_eq!(words(&"é".repeat(2049)), Vec::<String>::new());
    }

    #[test]
    fn a_text_is_its_words_stemmed_in_order_with_repeats() {
        assert_eq!(
            terms("Running the runners: they ran, running!"),
            ["run", "runner", "ran", "run"]
        );
    }

    #[test]
    fn a_query_loses_its_question_words_and_keeps_its_repeats() {
        let dropped = ["what".to_owned(), "did".to_owned(), "we".to_owned()];
        assert_eq!(
            query("What did we decide? Decide the index index", &dropped),
            ["decid", "decid", "index", "index"]
        );
        assert_eq!(query("what did we", &dropped), Vec::<String>::new());
    }

    #[test]
    fn a_question_word_stays_a_term_of_a_chunk() {
        assert_eq!(terms("what did we decide"), ["what", "did", "we", "decid"]);
    }

    #[test]
    fn a_dropped_word_is_compared_before_stemming() {
        let dropped = ["anything".to_owned()];
        assert_eq!(query("anything", &dropped), Vec::<String>::new());
        assert_eq!(query("anythings", &dropped), [stem("anythings")]);
    }

    #[test]
    fn an_empty_list_drops_nothing() {
        assert_eq!(query("what did we", &[]), ["what", "did", "we"]);
    }

    #[test]
    fn one_word_is_a_word_and_two_or_an_uppercase_one_are_not() {
        assert_eq!(word("decide").as_deref(), Some("decide"));
        assert_eq!(word("x_y").as_deref(), Some("x_y"));
        for text in ["", "a", "Decide", "two words", " decide", "decide!", "the"] {
            assert_eq!(word(text), None, "{text:?}");
        }
    }
}
