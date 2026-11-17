use crate::analyze::{Detect, Language};
use whichlang::{Lang, detect_language};

#[derive(Debug, Clone, Copy, Default)]
pub struct Whichlang;

fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::Ara => "ar",
        Lang::Cmn => "zh",
        Lang::Deu => "de",
        Lang::Eng => "en",
        Lang::Fra => "fr",
        Lang::Hin => "hi",
        Lang::Ita => "it",
        Lang::Jpn => "ja",
        Lang::Kor => "ko",
        Lang::Nld => "nl",
        Lang::Por => "pt",
        Lang::Rus => "ru",
        Lang::Spa => "es",
        Lang::Swe => "sv",
        Lang::Tur => "tr",
        Lang::Vie => "vi",
    }
}

impl Detect for Whichlang {
    fn language(&self, text: &str) -> Option<Language> {
        Language::of(code(detect_language(text)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bilbo_language::table::ALL;
    use whichlang::LANGUAGES;

    use crate::analyze::{Detect, Language};

    use super::{Whichlang, code};

    fn read(text: &str) -> Option<Language> {
        Whichlang.language(text)
    }

    #[test]
    fn every_measured_query_reads_as_its_file_records() {
        let mut asked = 0;
        for language in ALL {
            let file = format!("libs/language/locales/{}.toml", language.code);
            for text in language.detected {
                asked += 1;
                assert_eq!(
                    read(text).map(Language::code),
                    Some(language.code),
                    "{file}: detected {text:?}"
                );
            }
            for text in language.not_detected {
                asked += 1;
                assert_ne!(
                    read(text).map(Language::code),
                    Some(language.code),
                    "{file}: not_detected {text:?}"
                );
            }
        }
        assert!(asked > 0, "no language holds a measured query");
    }

    #[test]
    fn the_detector_can_name_every_language() {
        assert!(Language::all().next().is_some());
        for language in Language::all() {
            assert!(
                LANGUAGES.iter().any(|lang| code(*lang) == language.code()),
                "no language whichlang answers has the code {}",
                language.code()
            );
        }
        let codes: HashSet<&str> = LANGUAGES.iter().map(|lang| code(*lang)).collect();
        assert_eq!(codes.len(), LANGUAGES.len(), "two languages share a code");
    }
}
