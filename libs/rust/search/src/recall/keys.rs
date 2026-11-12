use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};

use crate::analyze;

pub const BUDGET: Key<usize> = Key {
    about: About {
        name: "recall.budget",
        variable: Some("BILBO_RECALL_BUDGET"),
        default: Default::Toml("4000"),
        allowed: "a whole number from 256 to 1000000",
        doc: "The most tokens bilbo recall prints, as the index's tokenizer counts them, each hit's header included.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|tokens| usize::try_from(tokens).ok())
            .filter(|tokens| (256..=1_000_000).contains(tokens))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const CANDIDATES: Key<usize> = Key {
    about: About {
        name: "recall.candidates",
        variable: Some("BILBO_RECALL_CANDIDATES"),
        default: Default::Toml("50"),
        allowed: "a whole number from 1 to 1000",
        doc: "How many of the best chunks bilbo recall considers before it fills the budget.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (1..=1000).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const CAP: Key<usize> = Key {
    about: About {
        name: "recall.cap",
        variable: Some("BILBO_RECALL_CAP"),
        default: Default::Toml("0"),
        allowed: "a whole number from 0 to 1000",
        doc: "The most hits bilbo recall takes from one entry; 0 sets no limit.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (0..=1000).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const QUESTION_WORDS: Key<Vec<String>> = Key {
    about: About {
        name: "recall.question_words",
        variable: None,
        default: Default::Toml(r#"["what", "did", "we", "should", "how", "anything"]"#),
        allowed: "a list of lowercase words",
        doc: "Words left out of a query before it is searched; they stay searchable in what was kept. An empty list leaves none out.",
    },
    parse: |value| {
        let items = value.items().ok_or_else(|| value.refuse())?;
        items
            .iter()
            .map(|item| {
                item.text()
                    .filter(|text| analyze::word(text).as_deref() == Some(*text))
                    .map(str::to_owned)
                    .ok_or_else(|| item.refuse())
            })
            .collect()
    },
    show: |words| {
        let quoted: Vec<String> = words.iter().map(|word| quote(word)).collect();
        format!("[{}]", quoted.join(", "))
    },
};

pub const BM25_K1: Key<f64> = Key {
    about: About {
        name: "recall.bm25_k1",
        variable: Some("BILBO_RECALL_BM25_K1"),
        default: Default::Toml("1.2"),
        allowed: "a number from 0.01 to 10",
        doc: "How fast a word that repeats in a chunk stops adding to its score. At 0 every chunk holding a word would score the same.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|k1| (0.01..=10.0).contains(k1))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const BM25_B: Key<f64> = Key {
    about: About {
        name: "recall.bm25_b",
        variable: Some("BILBO_RECALL_BM25_B"),
        default: Default::Toml("0.75"),
        allowed: "a number from 0 to 1",
        doc: "How much a chunk longer than the average is scored down: 0 not at all, 1 fully.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|b| (0.0..=1.0).contains(b))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const READ_BATCH: Key<usize> = Key {
    about: About {
        name: "recall.read_batch",
        variable: Some("BILBO_RECALL_READ_BATCH"),
        default: Default::Toml("4096"),
        allowed: "a whole number from 1 to 1000000",
        doc: "The most chunks bilbo recall reads from the index at once. By default only a run of equal scores longer than the candidates reaches it. It changes no hit.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|rows| usize::try_from(rows).ok())
            .filter(|rows| (1..=1_000_000).contains(rows))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ABOUT: [&About; 7] = [
    &BUDGET.about,
    &CANDIDATES.about,
    &CAP.about,
    &QUESTION_WORDS.about,
    &BM25_K1.about,
    &BM25_B.about,
    &READ_BATCH.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub budget: Resolved<usize>,
    pub candidates: Resolved<usize>,
    pub cap: Resolved<usize>,
    pub question_words: Resolved<Vec<String>>,
    pub bm25_k1: Resolved<f64>,
    pub bm25_b: Resolved<f64>,
    pub read_batch: Resolved<usize>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        budget: BUDGET.resolve(source)?,
        candidates: CANDIDATES.resolve(source)?,
        cap: CAP.resolve(source)?,
        question_words: QUESTION_WORDS.resolve(source)?,
        bm25_k1: BM25_K1.resolve(source)?,
        bm25_b: BM25_B.resolve(source)?,
        read_batch: READ_BATCH.resolve(source)?,
    })
}

pub fn defaults() -> Section {
    let file = File::default();
    section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("each default is a literal its key's unit test parses")
}

impl Section {
    pub fn shown(&self) -> [Shown; 7] {
        [
            BUDGET.shown(&self.budget),
            CANDIDATES.shown(&self.candidates),
            CAP.shown(&self.cap),
            QUESTION_WORDS.shown(&self.question_words),
            BM25_K1.shown(&self.bm25_k1),
            BM25_B.shown(&self.bm25_b),
            READ_BATCH.shown(&self.read_batch),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, defaults, section};

    fn resolve(env: &[(&str, &str)], text: &str) -> Result<Section, String> {
        let var = |name: &str| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        };
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        section(&Source {
            var: &var,
            file: &file,
        })
        .map_err(|error| error.to_string())
    }

    fn shown(section: &Section) -> Vec<String> {
        section
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect()
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = defaults();
        assert_eq!(section.budget.value, 4000);
        assert_eq!(section.candidates.value, 50);
        assert_eq!(section.cap.value, 0);
        assert_eq!(
            section.question_words.value,
            ["what", "did", "we", "should", "how", "anything"]
        );
        assert!((section.bm25_k1.value - 1.2).abs() < f64::EPSILON);
        assert!((section.bm25_b.value - 0.75).abs() < f64::EPSILON);
        assert_eq!(section.read_batch.value, 4096);
        assert_eq!(
            shown(&section),
            [
                "recall.budget = 4000 (default)",
                "recall.candidates = 50 (default)",
                "recall.cap = 0 (default)",
                "recall.question_words = [\"what\", \"did\", \"we\", \"should\", \"how\", \"anything\"] (default)",
                "recall.bm25_k1 = 1.2 (default)",
                "recall.bm25_b = 0.75 (default)",
                "recall.read_batch = 4096 (default)",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_RECALL_BUDGET"),
                Some("BILBO_RECALL_CANDIDATES"),
                Some("BILBO_RECALL_CAP"),
                None,
                Some("BILBO_RECALL_BM25_K1"),
                Some("BILBO_RECALL_BM25_B"),
                Some("BILBO_RECALL_READ_BATCH"),
            ]
        );
    }

    #[test]
    fn a_budget_candidates_or_cap_outside_its_range_is_refused() {
        for (text, message) in [
            (
                "recall.budget = 255",
                "/c/config.toml:1: recall.budget must be a whole number from 256 to 1000000, got 255",
            ),
            (
                "recall.budget = 1000001",
                "/c/config.toml:1: recall.budget must be a whole number from 256 to 1000000, got 1000001",
            ),
            (
                "recall.candidates = 0",
                "/c/config.toml:1: recall.candidates must be a whole number from 1 to 1000, got 0",
            ),
            (
                "recall.candidates = 1001",
                "/c/config.toml:1: recall.candidates must be a whole number from 1 to 1000, got 1001",
            ),
            (
                "recall.cap = -1",
                "/c/config.toml:1: recall.cap must be a whole number from 0 to 1000, got -1",
            ),
            (
                "recall.cap = 1001",
                "/c/config.toml:1: recall.cap must be a whole number from 0 to 1000, got 1001",
            ),
            (
                "recall.read_batch = 0",
                "/c/config.toml:1: recall.read_batch must be a whole number from 1 to 1000000, got 0",
            ),
            (
                "recall.read_batch = 1000001",
                "/c/config.toml:1: recall.read_batch must be a whole number from 1 to 1000000, got 1000001",
            ),
            (
                "recall.budget = \"4000\"",
                "/c/config.toml:1: recall.budget must be a whole number from 256 to 1000000, got \"4000\"",
            ),
        ] {
            assert_eq!(resolve(&[], text).expect_err("refused"), message, "{text}");
        }
        for (name, value) in [
            ("BILBO_RECALL_BUDGET", "12"),
            ("BILBO_RECALL_CANDIDATES", "1001"),
            ("BILBO_RECALL_CAP", "1001"),
            ("BILBO_RECALL_READ_BATCH", "0"),
        ] {
            assert!(
                resolve(&[(name, value)], "")
                    .expect_err("refused")
                    .starts_with(&format!("{name} must be a whole number")),
                "{name}"
            );
        }
        let low = resolve(
            &[],
            "recall.budget = 256\nrecall.candidates = 1\nrecall.cap = 0",
        )
        .expect("valid");
        assert_eq!(
            (low.budget.value, low.candidates.value, low.cap.value),
            (256, 1, 0)
        );
        let high = resolve(
            &[
                ("BILBO_RECALL_BUDGET", "1000000"),
                ("BILBO_RECALL_CANDIDATES", "1000"),
                ("BILBO_RECALL_CAP", "1000"),
            ],
            "recall.budget = 300",
        )
        .expect("valid");
        assert_eq!(
            (high.budget.value, high.candidates.value, high.cap.value),
            (1_000_000, 1000, 1000)
        );
    }

    #[test]
    fn k1_and_b_are_decimals_in_their_ranges() {
        let valid = |env: &[(&str, &str)], text: &str| {
            let section = resolve(env, text).expect("valid");
            (section.bm25_k1.value, section.bm25_b.value)
        };
        assert_eq!(
            valid(&[], "recall.bm25_k1 = 0.01\nrecall.bm25_b = 0"),
            (0.01, 0.0)
        );
        assert_eq!(
            valid(&[], "recall.bm25_k1 = 10\nrecall.bm25_b = 1"),
            (10.0, 1.0)
        );
        assert_eq!(
            valid(&[], "recall.bm25_k1 = 2.5\nrecall.bm25_b = 0.5"),
            (2.5, 0.5)
        );
        assert_eq!(
            valid(
                &[
                    ("BILBO_RECALL_BM25_K1", "1.5"),
                    ("BILBO_RECALL_BM25_B", "0.25")
                ],
                "recall.bm25_k1 = 3"
            ),
            (1.5, 0.25)
        );
        for (text, name) in [
            ("recall.bm25_k1 = 0", "recall.bm25_k1"),
            ("recall.bm25_k1 = 0.009", "recall.bm25_k1"),
            ("recall.bm25_k1 = 10.1", "recall.bm25_k1"),
            ("recall.bm25_k1 = \"1.2\"", "recall.bm25_k1"),
            ("recall.bm25_b = -0.1", "recall.bm25_b"),
            ("recall.bm25_b = 1.01", "recall.bm25_b"),
            ("recall.bm25_b = inf", "recall.bm25_b"),
            ("recall.bm25_b = nan", "recall.bm25_b"),
        ] {
            let error = resolve(&[], text).expect_err("refused");
            assert!(
                error.contains(&format!("{name} must be a number from")),
                "{text}: {error}"
            );
        }
        assert_eq!(
            resolve(&[], "recall.bm25_k1 = 11").expect_err("refused"),
            "/c/config.toml:1: recall.bm25_k1 must be a number from 0.01 to 10, got 11"
        );
        for (name, value) in [
            ("BILBO_RECALL_BM25_K1", "1e3"),
            ("BILBO_RECALL_BM25_K1", "11"),
            ("BILBO_RECALL_BM25_K1", "0"),
            ("BILBO_RECALL_BM25_B", "1.5"),
            ("BILBO_RECALL_BM25_B", "-1"),
            ("BILBO_RECALL_BM25_B", ".5"),
        ] {
            assert!(resolve(&[(name, value)], "").is_err(), "{name}={value}");
        }
    }

    #[test]
    fn the_question_words_are_a_list_of_single_lowercase_words() {
        let words = |text: &str| resolve(&[], text).map(|s| s.question_words.value);
        assert_eq!(words("recall.question_words = []"), Ok(Vec::new()));
        assert_eq!(
            words("recall.question_words = [\"why\", \"anything\"]"),
            Ok(vec!["why".to_owned(), "anything".to_owned()])
        );
        for text in [
            "recall.question_words = [\"What\"]",
            "recall.question_words = [\"what did\"]",
            "recall.question_words = [\"\"]",
            "recall.question_words = [\"the\"]",
            "recall.question_words = [3]",
            "recall.question_words = \"what\"",
            "recall.question_words = { a = \"what\" }",
            "[recall.question_words]\nwhat = true",
        ] {
            let error = resolve(&[], text).expect_err(text);
            assert!(
                error.contains("recall.question_words must be a list of lowercase words"),
                "{text}: {error}"
            );
        }
        assert_eq!(
            resolve(&[], "recall.question_words = [\"what\", \"Did\"]").expect_err("refused"),
            "/c/config.toml:1: recall.question_words must be a list of lowercase words, got \"Did\""
        );
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let text = "[recall]\nbudget = 777\ncandidates = 9\ncap = 2\nquestion_words = [\"why\", \"who\"]\nbm25_k1 = 1.5\nbm25_b = 0.4\nread_batch = 99\n";
        for section in [defaults(), resolve(&[], text).expect("valid")] {
            let mut written = String::from("[recall]\n");
            for shown in section.shown() {
                let key = shown.name.strip_prefix("recall.").expect("a recall key");
                written.push_str(&format!("{key} = {}\n", shown.value));
            }
            let again = resolve(&[], &written).expect("reads back");
            assert_eq!(again.budget.value, section.budget.value);
            assert_eq!(again.candidates.value, section.candidates.value);
            assert_eq!(again.cap.value, section.cap.value);
            assert_eq!(again.question_words.value, section.question_words.value);
            assert!((again.bm25_k1.value - section.bm25_k1.value).abs() < f64::EPSILON);
            assert!((again.bm25_b.value - section.bm25_b.value).abs() < f64::EPSILON);
            assert_eq!(again.read_batch.value, section.read_batch.value);
        }
    }
}
