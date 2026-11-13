mod portuguese;
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

/// A language whose function words a query can lose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Portuguese,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::Portuguese => "pt",
        }
    }

    pub fn of(code: &str) -> Option<Language> {
        match code {
            "pt" => Some(Language::Portuguese),
            _ => None,
        }
    }

    fn counts(self, word: &str) -> bool {
        match self {
            Language::Portuguese => portuguese::counts(word),
        }
    }

    fn leaves(self, word: &str) -> bool {
        match self {
            Language::Portuguese => portuguese::leaves(word),
        }
    }
}

pub trait Detect {
    /// `None` for English and for every language bilbo has no function words for.
    fn language(&self, text: &str) -> Option<Language>;
}

#[derive(Clone, Copy, Default)]
pub struct Omit<'a> {
    /// Left out of every query, matched as typed.
    pub words: &'a [String],
    /// The languages whose function words leave a query written in one of them.
    pub languages: &'a [Language],
    /// Asked which language a query is in; `None` leaves every function word in.
    pub detect: Option<&'a dyn Detect>,
}

impl std::fmt::Debug for Omit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Omit")
            .field("words", &self.words)
            .field("languages", &self.languages)
            .finish_non_exhaustive()
    }
}

pub fn query(text: &str, omit: &Omit<'_>) -> Vec<String> {
    let words = words(text);
    let counts = |language: Language| words.iter().any(|word| language.counts(word));
    let language = omit.detect.and_then(|detect| {
        if words.len() < 2 || !omit.languages.iter().any(|language| counts(*language)) {
            return None;
        }
        detect
            .language(text)
            .filter(|language| omit.languages.contains(language) && counts(*language))
    });
    words
        .iter()
        .filter(|word| {
            !omit.words.contains(word) && !language.is_some_and(|language| language.leaves(word))
        })
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
    use crate::testing::Says;

    use super::{
        Language, MAX_TERM_BYTES, Omit, STOPWORDS, is_word, lower, portuguese, query, stem, terms,
        word, words,
    };

    fn portuguese_says() -> Says {
        Says::new(Some(Language::Portuguese))
    }

    fn with(says: &Says) -> Omit<'_> {
        Omit {
            words: &[],
            languages: &[Language::Portuguese],
            detect: Some(says),
        }
    }

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
            query(
                "What did we decide? Decide the index index",
                &Omit {
                    words: &dropped,
                    languages: &[Language::Portuguese],
                    detect: None
                }
            ),
            ["decid", "decid", "index", "index"]
        );
        let omit = Omit {
            words: &dropped,
            languages: &[Language::Portuguese],
            detect: None,
        };
        assert_eq!(query("what did we", &omit), Vec::<String>::new());
    }

    #[test]
    fn a_question_word_stays_a_term_of_a_chunk() {
        assert_eq!(terms("what did we decide"), ["what", "did", "we", "decid"]);
    }

    #[test]
    fn a_dropped_word_is_compared_before_stemming() {
        let dropped = ["anything".to_owned()];
        let omit = Omit {
            words: &dropped,
            languages: &[Language::Portuguese],
            detect: None,
        };
        assert_eq!(query("anything", &omit), Vec::<String>::new());
        assert_eq!(query("anythings", &omit), [stem("anythings")]);
    }

    #[test]
    fn an_empty_list_drops_nothing() {
        assert_eq!(
            query("what did we", &Omit::default()),
            ["what", "did", "we"]
        );
    }

    #[test]
    fn one_word_is_a_word_and_two_or_an_uppercase_one_are_not() {
        assert_eq!(word("decide").as_deref(), Some("decide"));
        assert_eq!(word("x_y").as_deref(), Some("x_y"));
        for text in ["", "a", "Decide", "two words", " decide", "decide!", "the"] {
            assert_eq!(word(text), None, "{text:?}");
        }
    }

    #[test]
    fn every_published_spelling_a_query_can_hold_leaves_a_portuguese_query() {
        let says = portuguese_says();
        let holdable: Vec<_> = portuguese::published()
            .iter()
            .filter(|spelling| word(spelling).is_some())
            .collect();
        assert_eq!(holdable.len(), 199);
        for spelling in holdable {
            assert_eq!(
                query(&format!("{spelling} que pelo"), &with(&says)),
                Vec::<String>::new(),
                "{spelling}"
            );
        }
    }

    #[test]
    fn an_accented_an_unaccented_and_a_capital_spelling_leave_alike() {
        let says = portuguese_says();
        for (accented, plain) in [
            ("há", "ha"),
            ("não", "nao"),
            ("éramos", "eramos"),
            ("você", "voce"),
            ("tínhamos", "tinhamos"),
            ("nós", "nos"),
            ("fôramos", "foramos"),
        ] {
            for spelling in [accented, plain] {
                for typed in [spelling.to_owned(), spelling.to_uppercase()] {
                    assert_eq!(
                        query(&format!("{typed} que pelo"), &with(&says)),
                        Vec::<String>::new(),
                        "{typed}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_query_of_one_word_keeps_it_and_the_detector_is_not_asked() {
        let says = portuguese_says();
        for (text, kept) in [("nas", "nas"), ("the nas", "nas"), ("que", "que")] {
            assert_eq!(query(text, &with(&says)), [stem(kept)], "{text}");
        }
        assert!(says.asked.borrow().is_empty());
        let omit = Omit {
            words: &["nas".to_owned()],
            languages: &[Language::Portuguese],
            detect: Some(&says),
        };
        assert_eq!(query("nas", &omit), Vec::<String>::new());
    }

    #[test]
    fn a_query_the_detector_does_not_call_portuguese_keeps_every_word() {
        let says = Says::new(None);
        for text in ["nas os update", "que pelo"] {
            assert_eq!(query(text, &with(&says)), terms(text), "{text}");
        }
        assert_eq!(says.asked.borrow().len(), 2);
    }

    #[test]
    fn the_detector_is_not_asked_about_a_query_with_no_word_that_counts() {
        let says = portuguese_says();
        for text in [
            "bazel cache after xcode upgrade",
            "do we pin the cargo lock",
            "so do me",
            "me ajuda",
        ] {
            assert_eq!(query(text, &with(&says)), terms(text), "{text}");
        }
        assert!(says.asked.borrow().is_empty());
    }

    #[test]
    fn do_me_and_so_leave_once_a_query_is_portuguese() {
        let says = portuguese_says();
        assert_eq!(
            query("status do deploy que", &with(&says)),
            [stem("status"), stem("deploy")]
        );
        assert_eq!(says.asked.borrow().len(), 1);
    }

    #[test]
    fn the_detector_is_given_the_query_as_typed() {
        let says = portuguese_says();
        let text = "Não, VOCÊ já viu o Deploy?";
        query(text, &with(&says));
        assert_eq!(*says.asked.borrow(), [text]);
    }

    #[test]
    fn a_spelling_that_unaccents_to_an_english_stopword_still_leaves_and_counts() {
        let says = portuguese_says();
        assert_eq!(query("às backup", &with(&says)), [stem("backup")]);
        assert_eq!(says.asked.borrow().len(), 1);
    }

    #[test]
    fn with_no_detector_no_function_word_leaves() {
        assert_eq!(
            query(
                "que pelo nas",
                &Omit {
                    words: &[],
                    languages: &[Language::Portuguese],
                    detect: None,
                }
            ),
            terms("que pelo nas")
        );
    }

    #[test]
    fn a_kept_word_is_stemmed_as_typed() {
        let says = portuguese_says();
        assert_eq!(query("café que", &with(&says)), terms("café"));
        assert_ne!(terms("café"), terms("cafe"));
    }

    #[test]
    fn the_words_the_user_omits_are_matched_as_typed() {
        let omit = Omit {
            words: &["nao".to_owned()],
            languages: &[Language::Portuguese],
            detect: None,
        };
        assert_eq!(query("não", &omit), [stem("não")]);
        assert_eq!(query("nao", &omit), Vec::<String>::new());
    }

    #[test]
    fn a_listed_word_stays_a_term_of_a_chunk() {
        assert_eq!(terms("que pelo nas").len(), 3);
    }

    #[test]
    fn a_language_the_key_does_not_list_leaves_nothing_and_the_detector_is_not_asked() {
        let says = portuguese_says();
        let omit = Omit {
            words: &[],
            languages: &[],
            detect: Some(&says),
        };
        assert_eq!(query("que pelo nas", &omit), terms("que pelo nas"));
        assert_eq!(terms("que pelo nas").len(), 3);
        assert!(says.asked.borrow().is_empty());
    }

    #[test]
    fn an_omit_shows_its_words_and_languages() {
        let words = ["we".to_owned()];
        let omit = Omit {
            words: &words,
            languages: &[Language::Portuguese],
            detect: Some(&Says::new(None)),
        };
        assert_eq!(
            format!("{omit:?}"),
            r#"Omit { words: ["we"], languages: [Portuguese], .. }"#
        );
    }
}
