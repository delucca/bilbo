use std::sync::LazyLock;

use bilbo_language::table::ALL;

use super::{fold, word};

const ENGLISH: &str = "en";

pub(super) struct Table {
    pub(super) code: &'static str,
    words: Vec<String>,
    shared: Vec<String>,
}

impl Table {
    pub(super) fn leaves(&self, word: &str) -> bool {
        self.words
            .binary_search_by(|each| each.as_str().cmp(word))
            .is_ok()
    }

    pub(super) fn counts(&self, word: &str) -> bool {
        self.leaves(word) && !self.shared.iter().any(|each| each == word)
    }
}

pub(super) fn published(code: &str) -> Option<&'static [&'static str]> {
    ALL.iter()
        .find(|language| language.code == code)
        .map(|language| language.function_words)
}

pub(super) fn tables() -> &'static [Table] {
    static TABLES: LazyLock<Vec<Table>> = LazyLock::new(|| {
        let english = published(ENGLISH).expect("the library holds the English list");
        ALL.iter()
            .filter(|language| language.code != ENGLISH)
            .map(|language| {
                let mut words: Vec<String> = language
                    .function_words
                    .iter()
                    .map(|spelling| fold(spelling).into_owned())
                    .filter(|folded| word(folded).is_some())
                    .collect();
                words.sort();
                words.dedup();
                let shared = words
                    .iter()
                    .filter(|each| english.contains(&each.as_str()))
                    .cloned()
                    .collect();
                Table {
                    code: language.code,
                    words,
                    shared,
                }
            })
            .collect()
    });
    &TABLES
}

#[cfg(test)]
mod tests {
    use bilbo_language::table::ALL;

    use super::{Table, tables};
    use crate::analyze::Language;
    use crate::testing::portuguese;

    #[test]
    fn the_table_has_194_words_and_the_english_three_are_do_me_and_so() {
        let table = tables()
            .iter()
            .find(|table| table.code == "pt")
            .expect("a Portuguese table");
        assert_eq!(table.words.len(), 194);
        assert!(table.words.windows(2).all(|pair| pair[0] < pair[1]));
        for word in &table.words {
            assert!(word.chars().all(|c| c.is_ascii_lowercase()), "{word}");
        }
        assert!(!table.words.iter().any(|word| word == "as"));
        assert_eq!(table.shared, ["do", "me", "so"]);
    }

    #[test]
    fn every_language_has_a_query_read_as_it_and_one_that_is_not() {
        for table in tables() {
            let language = ALL
                .iter()
                .find(|language| language.code == table.code)
                .unwrap_or_else(|| panic!("the library has no row for {}", table.code));
            for (key, queries) in [
                ("detected", language.detected),
                ("not_detected", language.not_detected),
            ] {
                assert!(
                    !queries.is_empty(),
                    "libs/language/locales/{}.toml: {key} holds no query",
                    language.code
                );
            }
        }
    }

    #[test]
    fn a_language_equals_itself_and_no_language_of_another_code() {
        static OTHER: Table = Table {
            code: "zxx",
            words: Vec::new(),
            shared: Vec::new(),
        };
        assert_ne!(
            portuguese(),
            Language(&OTHER),
            "two languages with different codes must not be equal"
        );
        let first = portuguese();
        let second = portuguese();
        assert_eq!(first, second, "a language must equal itself");
    }
}
