use bilbo_search::analyze::{Detect, Language};
use whichlang::{Lang, detect_language};

#[derive(Debug, Clone, Copy, Default)]
pub struct Whichlang;

impl Detect for Whichlang {
    fn language(&self, text: &str) -> Option<Language> {
        match detect_language(text) {
            Lang::Por => Some(Language::Portuguese),
            Lang::Ara
            | Lang::Cmn
            | Lang::Deu
            | Lang::Eng
            | Lang::Fra
            | Lang::Hin
            | Lang::Ita
            | Lang::Jpn
            | Lang::Kor
            | Lang::Nld
            | Lang::Rus
            | Lang::Spa
            | Lang::Swe
            | Lang::Tur
            | Lang::Vie => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use bilbo_search::analyze::{Detect, Language};

    use super::Whichlang;

    fn read(text: &str) -> Option<Language> {
        Whichlang.language(text)
    }

    #[test]
    fn only_portuguese_is_a_language_bilbo_has_words_for() {
        assert_eq!(read("Olá feliz contribuinte"), Some(Language::Portuguese));
        for text in [
            "Hello, happy tax payer",
            "Hola feliz contribuyente",
            "Ciao, felice contribuente!",
            "Bonjour joyeux contribuable",
            "Hallo glücklicher Steuerzahler",
        ] {
            assert_eq!(read(text), None, "{text}");
        }
    }

    #[test]
    fn these_queries_read_as_they_did_when_the_gate_was_measured() {
        for (text, language) in [
            ("como configurar o nas", Some(Language::Portuguese)),
            ("pra que serve isso", Some(Language::Portuguese)),
            ("limpa o disco", Some(Language::Portuguese)),
            (
                "tem alguma nota sobre a tarifa de energia",
                Some(Language::Portuguese),
            ),
            ("how do I mount the nas", None),
            ("ha os upgrade", None),
            ("nas backup schedule", None),
            ("os keychain prompt on login", None),
            ("local llama server config", None),
            ("nas os update failed", Some(Language::Portuguese)),
            ("status do deploy?", None),
        ] {
            assert_eq!(read(text), language, "{text}");
        }
    }
}
