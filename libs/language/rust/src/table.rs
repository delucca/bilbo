#[derive(Debug)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    pub source: &'static str,
    pub function_words: &'static [&'static str],
    pub detected: &'static [&'static str],
    pub not_detected: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/table.rs"));

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn each_list_is_the_one_its_source_publishes_rising_with_no_repeat() {
        for language in ALL {
            let file = format!("{}.toml", language.code);
            assert!(
                language
                    .function_words
                    .windows(2)
                    .all(|pair| pair[0] < pair[1]),
                "{file}: function_words must rise by code point with no repeat"
            );
            assert_eq!(
                language.source, "nltk",
                "{file}: no test knows where `{}` is published",
                language.source
            );
            let mut published = stop_words::lookup(language.code)
                .unwrap_or_else(|| panic!("{file}: the stop-words crate holds no list for it"))
                .to_vec();
            published.sort_unstable();
            published.dedup();
            assert_eq!(
                language.function_words, published,
                "{file}: function_words is not the nltk list"
            );
        }
    }

    #[test]
    fn every_language_has_a_name() {
        for language in ALL {
            assert!(
                !language.name.is_empty() && language.name == language.name.trim(),
                "{}.toml: name is empty or padded",
                language.code
            );
        }
    }

    #[test]
    fn the_languages_rise_by_code() {
        let codes: Vec<&str> = ALL.iter().map(|language| language.code).collect();
        assert!(!codes.is_empty(), "the table holds no language");
        assert!(
            codes.windows(2).all(|pair| pair[0] < pair[1]),
            "the table must hold the languages by rising code, got {codes:?}"
        );
    }
}
