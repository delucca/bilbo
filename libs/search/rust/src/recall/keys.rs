use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};

use crate::analyze::{self, Language};

pub const BUDGET: Key<usize> = Key {
    about: About {
        name: "recall.budget",
        variable: Some("BILBO_RECALL_BUDGET"),
        default: Default::Toml("4000"),
        allowed: "a whole number from 256 to 1000000",
        doc: "The most tokens of chunks bilbo recall prints, as the index's tokenizer counts them, each hit's header included.",
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

pub const NOTES_BUDGET: Key<usize> = Key {
    about: About {
        name: "recall.notes_budget",
        variable: Some("BILBO_RECALL_NOTES_BUDGET"),
        default: Default::Toml("2500"),
        allowed: "0, or a whole number from 256 to 1000000",
        doc: "The most tokens of notes bilbo recall prints after the chunks, as the index's tokenizer counts them, each note's header included.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|tokens| usize::try_from(tokens).ok())
            .filter(|tokens| *tokens == 0 || (256..=1_000_000).contains(tokens))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const CANDIDATES: Key<usize> = Key {
    about: About {
        name: "recall.candidates",
        variable: Some("BILBO_RECALL_CANDIDATES"),
        default: Default::Toml("400"),
        allowed: "a whole number from 1 to 1000",
        doc: "How many of the best chunks bilbo recall considers before it fills the budget. The copies and the further parts of a record that it passes over are among them.",
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
        doc: "The most hits bilbo recall shows from one entry; 0 sets no limit.",
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

pub const RECORD_CAP: Key<usize> = Key {
    about: About {
        name: "recall.record_cap",
        variable: Some("BILBO_RECALL_RECORD_CAP"),
        default: Default::Toml("2"),
        allowed: "a whole number from 0 to 1000",
        doc: "The most hits of an entry that bilbo recall shows with one same record in them, such as the pieces of one long tool output; 0 sets no limit.",
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

pub const COPY_SHARE: Key<f64> = Key {
    about: About {
        name: "recall.copy_share",
        variable: Some("BILBO_RECALL_COPY_SHARE"),
        default: Default::Toml("0.5"),
        allowed: "a number from 0 to 1",
        doc: "A hit is passed over when at least this share of its runs of words is in the hits above it; 0 passes over no copy.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|share| (0.0..=1.0).contains(share))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const COPY_WORDS: Key<usize> = Key {
    about: About {
        name: "recall.copy_words",
        variable: Some("BILBO_RECALL_COPY_WORDS"),
        default: Default::Toml("5"),
        allowed: "a whole number from 1 to 64",
        doc: "How many words in a row make one run when bilbo recall looks for copies.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (1..=64).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const QUESTION_WORDS: Key<Vec<String>> = Key {
    about: About {
        name: "recall.question_words",
        variable: None,
        default: Default::Toml(r#"["what", "did", "we", "should", "how", "anything"]"#),
        allowed: "a list of words as the analyzer reads them: lowercase, with no accent or other mark, no compatibility form it folds, and Hangul as jamo",
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

pub const LANGUAGES: Key<Vec<Language>> = Key {
    about: About {
        name: "recall.languages",
        variable: None,
        default: Default::Toml(r#"["pt"]"#),
        allowed: "a list of language codes bilbo knows, each once",
        doc: "The languages whose function words bilbo recall leaves out of a query written in one of them; they stay searchable in what was kept. An empty list leaves none out.",
    },
    parse: |value| {
        let items = value.items().ok_or_else(|| value.refuse())?;
        let mut languages = Vec::new();
        for item in items {
            let language = item
                .text()
                .and_then(Language::of)
                .filter(|language| !languages.contains(language))
                .ok_or_else(|| item.refuse())?;
            languages.push(language);
        }
        Ok(languages)
    },
    show: |languages| {
        let quoted: Vec<String> = languages
            .iter()
            .map(|language| quote(language.code()))
            .collect();
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

pub const TYPO_TERMS: Key<usize> = Key {
    about: About {
        name: "recall.typo_terms",
        variable: Some("BILBO_RECALL_TYPO_TERMS"),
        default: Default::Toml("3"),
        allowed: "a whole number from 0 to 10",
        doc: "How many of the index's nearest terms bilbo recall searches in place of a query word no chunk holds; 0 turns typo matching off.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (0..=10).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const TYPO_LETTERS: Key<usize> = Key {
    about: About {
        name: "recall.typo_letters",
        variable: Some("BILBO_RECALL_TYPO_LETTERS"),
        default: Default::Toml("4"),
        allowed: "a whole number from 2 to 64",
        doc: "The shortest query word, in letters after stemming, that bilbo recall takes for misspelt when no chunk holds it.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (2..=64).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const TYPO_MAX_LETTERS: Key<usize> = Key {
    about: About {
        name: "recall.typo_max_letters",
        variable: Some("BILBO_RECALL_TYPO_MAX_LETTERS"),
        default: Default::Toml("64"),
        allowed: "a whole number from 2 to 64",
        doc: "The longest query word, in letters after stemming, that bilbo recall takes for misspelt when no chunk holds it. The search for near terms holds memory for each letter of the word, so 64 is the most.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (2..=64).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const TYPO_TWO_EDITS: Key<usize> = Key {
    about: About {
        name: "recall.typo_two_edits",
        variable: Some("BILBO_RECALL_TYPO_TWO_EDITS"),
        default: Default::Toml("8"),
        allowed: "a whole number from 2 to 65",
        doc: "A misspelt word of this many letters or more is matched with terms two edits away, a shorter one with terms one edit away; 65 never allows two.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (2..=65).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const PAIR_WEIGHT: Key<f64> = Key {
    about: About {
        name: "recall.pair_weight",
        variable: Some("BILBO_RECALL_PAIR_WEIGHT"),
        default: Default::Toml("0.25"),
        allowed: "a number from 0 to 10",
        doc: "How much two query words that stand next to each other in a chunk, in the query's order, add to its score, as a share of what one word that rare adds; 0 adds nothing and reads no position.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|weight| (0.0..=10.0).contains(weight))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ABOUT: [&About; 17] = [
    &BUDGET.about,
    &NOTES_BUDGET.about,
    &CANDIDATES.about,
    &CAP.about,
    &RECORD_CAP.about,
    &COPY_SHARE.about,
    &COPY_WORDS.about,
    &QUESTION_WORDS.about,
    &LANGUAGES.about,
    &BM25_K1.about,
    &BM25_B.about,
    &READ_BATCH.about,
    &TYPO_TERMS.about,
    &TYPO_LETTERS.about,
    &TYPO_MAX_LETTERS.about,
    &TYPO_TWO_EDITS.about,
    &PAIR_WEIGHT.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub budget: Resolved<usize>,
    pub notes_budget: Resolved<usize>,
    pub candidates: Resolved<usize>,
    pub cap: Resolved<usize>,
    pub record_cap: Resolved<usize>,
    pub copy_share: Resolved<f64>,
    pub copy_words: Resolved<usize>,
    pub question_words: Resolved<Vec<String>>,
    pub languages: Resolved<Vec<Language>>,
    pub bm25_k1: Resolved<f64>,
    pub bm25_b: Resolved<f64>,
    pub read_batch: Resolved<usize>,
    pub typo_terms: Resolved<usize>,
    pub typo_letters: Resolved<usize>,
    pub typo_max_letters: Resolved<usize>,
    pub typo_two_edits: Resolved<usize>,
    pub pair_weight: Resolved<f64>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        budget: BUDGET.resolve(source)?,
        notes_budget: NOTES_BUDGET.resolve(source)?,
        candidates: CANDIDATES.resolve(source)?,
        cap: CAP.resolve(source)?,
        record_cap: RECORD_CAP.resolve(source)?,
        copy_share: COPY_SHARE.resolve(source)?,
        copy_words: COPY_WORDS.resolve(source)?,
        question_words: QUESTION_WORDS.resolve(source)?,
        languages: LANGUAGES.resolve(source)?,
        bm25_k1: BM25_K1.resolve(source)?,
        bm25_b: BM25_B.resolve(source)?,
        read_batch: READ_BATCH.resolve(source)?,
        typo_terms: TYPO_TERMS.resolve(source)?,
        typo_letters: TYPO_LETTERS.resolve(source)?,
        typo_max_letters: TYPO_MAX_LETTERS.resolve(source)?,
        typo_two_edits: TYPO_TWO_EDITS.resolve(source)?,
        pair_weight: PAIR_WEIGHT.resolve(source)?,
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
    pub fn shown(&self) -> [Shown; 17] {
        [
            BUDGET.shown(&self.budget),
            NOTES_BUDGET.shown(&self.notes_budget),
            CANDIDATES.shown(&self.candidates),
            CAP.shown(&self.cap),
            RECORD_CAP.shown(&self.record_cap),
            COPY_SHARE.shown(&self.copy_share),
            COPY_WORDS.shown(&self.copy_words),
            QUESTION_WORDS.shown(&self.question_words),
            LANGUAGES.shown(&self.languages),
            BM25_K1.shown(&self.bm25_k1),
            BM25_B.shown(&self.bm25_b),
            READ_BATCH.shown(&self.read_batch),
            TYPO_TERMS.shown(&self.typo_terms),
            TYPO_LETTERS.shown(&self.typo_letters),
            TYPO_MAX_LETTERS.shown(&self.typo_max_letters),
            TYPO_TWO_EDITS.shown(&self.typo_two_edits),
            PAIR_WEIGHT.shown(&self.pair_weight),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use crate::testing::portuguese;

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
        assert_eq!(section.notes_budget.value, 2500);
        assert_eq!(section.candidates.value, 400);
        assert_eq!(section.cap.value, 0);
        assert_eq!(section.record_cap.value, 2);
        assert!((section.copy_share.value - 0.5).abs() < f64::EPSILON);
        assert_eq!(section.copy_words.value, 5);
        assert_eq!(
            section.question_words.value,
            ["what", "did", "we", "should", "how", "anything"]
        );
        assert_eq!(section.languages.value, [portuguese()]);
        assert!((section.bm25_k1.value - 1.2).abs() < f64::EPSILON);
        assert!((section.bm25_b.value - 0.75).abs() < f64::EPSILON);
        assert_eq!(section.read_batch.value, 4096);
        assert_eq!(section.typo_terms.value, 3);
        assert_eq!(section.typo_letters.value, 4);
        assert_eq!(section.typo_max_letters.value, 64);
        assert_eq!(section.typo_two_edits.value, 8);
        assert!((section.pair_weight.value - 0.25).abs() < f64::EPSILON);
        assert_eq!(
            shown(&section),
            [
                "recall.budget = 4000 (default)",
                "recall.notes_budget = 2500 (default)",
                "recall.candidates = 400 (default)",
                "recall.cap = 0 (default)",
                "recall.record_cap = 2 (default)",
                "recall.copy_share = 0.5 (default)",
                "recall.copy_words = 5 (default)",
                "recall.question_words = [\"what\", \"did\", \"we\", \"should\", \"how\", \"anything\"] (default)",
                "recall.languages = [\"pt\"] (default)",
                "recall.bm25_k1 = 1.2 (default)",
                "recall.bm25_b = 0.75 (default)",
                "recall.read_batch = 4096 (default)",
                "recall.typo_terms = 3 (default)",
                "recall.typo_letters = 4 (default)",
                "recall.typo_max_letters = 64 (default)",
                "recall.typo_two_edits = 8 (default)",
                "recall.pair_weight = 0.25 (default)",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_RECALL_BUDGET"),
                Some("BILBO_RECALL_NOTES_BUDGET"),
                Some("BILBO_RECALL_CANDIDATES"),
                Some("BILBO_RECALL_CAP"),
                Some("BILBO_RECALL_RECORD_CAP"),
                Some("BILBO_RECALL_COPY_SHARE"),
                Some("BILBO_RECALL_COPY_WORDS"),
                None,
                None,
                Some("BILBO_RECALL_BM25_K1"),
                Some("BILBO_RECALL_BM25_B"),
                Some("BILBO_RECALL_READ_BATCH"),
                Some("BILBO_RECALL_TYPO_TERMS"),
                Some("BILBO_RECALL_TYPO_LETTERS"),
                Some("BILBO_RECALL_TYPO_MAX_LETTERS"),
                Some("BILBO_RECALL_TYPO_TWO_EDITS"),
                Some("BILBO_RECALL_PAIR_WEIGHT"),
            ]
        );
    }

    #[test]
    fn the_notes_budget_is_zero_or_a_whole_number_from_256_to_1000000() {
        for number in [0, 256, 2500, 2501, 1_000_000] {
            let from_file = resolve(&[], &format!("recall.notes_budget = {number}"));
            assert_eq!(from_file.map(|s| s.notes_budget.value), Ok(number));
            let from_variable = resolve(&[("BILBO_RECALL_NOTES_BUDGET", &number.to_string())], "");
            assert_eq!(from_variable.map(|s| s.notes_budget.value), Ok(number));
        }
        let allowed = "recall.notes_budget must be 0, or a whole number from 256 to 1000000";
        for (text, got) in [
            ("1", "1"),
            ("255", "255"),
            ("1000001", "1000001"),
            ("-1", "-1"),
            ("\"2500\"", "\"2500\""),
            ("2.5", "2.5"),
        ] {
            assert_eq!(
                resolve(&[], &format!("recall.notes_budget = {text}")).err(),
                Some(format!("/c/config.toml:1: {allowed}, got {got}")),
                "{text}"
            );
        }
        assert_eq!(
            resolve(&[("BILBO_RECALL_NOTES_BUDGET", "255")], "").err(),
            Some("BILBO_RECALL_NOTES_BUDGET must be 0, or a whole number from 256 to 1000000, got \"255\"".to_owned())
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
    fn a_record_cap_a_copy_share_or_copy_words_outside_its_range_is_refused() {
        for (text, name, shape) in [
            (
                "recall.record_cap = 1001",
                "recall.record_cap",
                "a whole number from 0 to 1000",
            ),
            (
                "recall.record_cap = -1",
                "recall.record_cap",
                "a whole number from 0 to 1000",
            ),
            (
                "recall.copy_share = 1.5",
                "recall.copy_share",
                "a number from 0 to 1",
            ),
            (
                "recall.copy_share = -0.1",
                "recall.copy_share",
                "a number from 0 to 1",
            ),
            (
                "recall.copy_share = nan",
                "recall.copy_share",
                "a number from 0 to 1",
            ),
            (
                "recall.copy_words = 0",
                "recall.copy_words",
                "a whole number from 1 to 64",
            ),
            (
                "recall.copy_words = 65",
                "recall.copy_words",
                "a whole number from 1 to 64",
            ),
        ] {
            let error = resolve(&[], text).expect_err(text);
            assert!(
                error.contains(&format!("{name} must be {shape}")),
                "{text}: {error}"
            );
        }
        let edges = resolve(
            &[],
            "recall.record_cap = 0\nrecall.copy_share = 1\nrecall.copy_words = 64",
        )
        .expect("valid");
        assert_eq!((edges.record_cap.value, edges.copy_words.value), (0, 64));
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
    fn the_question_words_are_words_as_the_analyzer_reads_them() {
        let words = |text: &str| resolve(&[], text).map(|s| s.question_words.value);
        assert_eq!(words("recall.question_words = []"), Ok(Vec::new()));
        assert_eq!(
            words("recall.question_words = [\"nao\"]"),
            Ok(vec!["nao".to_owned()])
        );
        assert_eq!(
            words("recall.question_words = [\"\u{1112}\u{1161}\u{11ab}\"]"),
            Ok(vec!["\u{1112}\u{1161}\u{11ab}".to_owned()])
        );
        assert_eq!(
            words("recall.question_words = [\"2\u{bc}\"]"),
            Ok(vec!["2\u{bc}".to_owned()])
        );
        assert_eq!(
            words("recall.question_words = [\"why\", \"anything\"]"),
            Ok(vec!["why".to_owned(), "anything".to_owned()])
        );
        for text in [
            "recall.question_words = [\"What\"]",
            "recall.question_words = [\"n\u{e3}o\"]",
            "recall.question_words = [\"na\u{303}o\"]",
            "recall.question_words = [\"\u{fb01}x\"]",
            "recall.question_words = [\"\u{ff46}\u{ff58}\"]",
            "recall.question_words = [\"\u{bb34}\u{c5c7}\"]",
            "recall.question_words = [\"\u{3069}\u{3046}\"]",
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
                error.contains(
                    "recall.question_words must be a list of words as the analyzer reads them: lowercase, with no accent or other mark, no compatibility form it folds, and Hangul as jamo"
                ),
                "{text}: {error}"
            );
        }
        assert_eq!(
            resolve(&[], "recall.question_words = [\"what\", \"Did\"]").expect_err("refused"),
            "/c/config.toml:1: recall.question_words must be a list of words as the analyzer reads them: lowercase, with no accent or other mark, no compatibility form it folds, and Hangul as jamo, got \"Did\""
        );
    }

    #[test]
    fn the_languages_are_a_list_of_known_codes_each_once() {
        let languages = |text: &str| resolve(&[], text).map(|s| s.languages.value);
        assert_eq!(
            languages("recall.languages = [\"pt\"]"),
            Ok(vec![portuguese()])
        );
        assert_eq!(languages("recall.languages = []"), Ok(Vec::new()));
        for text in [
            "recall.languages = [\"zxx\"]",
            "recall.languages = [\"en\"]",
            "recall.languages = [\"PT\"]",
            "recall.languages = [\"pt\", \"pt\"]",
            "recall.languages = \"pt\"",
            "recall.languages = [1]",
        ] {
            let error = resolve(&[], text).expect_err(text);
            assert!(
                error.contains("recall.languages must be a list of language codes"),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn the_typo_keys_outside_their_ranges_are_refused() {
        for (text, name, shape) in [
            (
                "recall.typo_terms = 11",
                "recall.typo_terms",
                "a whole number from 0 to 10",
            ),
            (
                "recall.typo_terms = -1",
                "recall.typo_terms",
                "a whole number from 0 to 10",
            ),
            (
                "recall.typo_letters = 1",
                "recall.typo_letters",
                "a whole number from 2 to 64",
            ),
            (
                "recall.typo_letters = 65",
                "recall.typo_letters",
                "a whole number from 2 to 64",
            ),
            (
                "recall.typo_max_letters = 1",
                "recall.typo_max_letters",
                "a whole number from 2 to 64",
            ),
            (
                "recall.typo_max_letters = 65",
                "recall.typo_max_letters",
                "a whole number from 2 to 64",
            ),
            (
                "recall.typo_two_edits = 1",
                "recall.typo_two_edits",
                "a whole number from 2 to 65",
            ),
            (
                "recall.typo_two_edits = 66",
                "recall.typo_two_edits",
                "a whole number from 2 to 65",
            ),
            (
                "recall.typo_terms = \"3\"",
                "recall.typo_terms",
                "a whole number from 0 to 10",
            ),
        ] {
            let error = resolve(&[], text).expect_err(text);
            assert!(
                error.starts_with("/c/config.toml:1: ")
                    && error.contains(&format!("{name} must be {shape}, got ")),
                "{text}: {error}"
            );
        }
        for (name, value) in [
            ("BILBO_RECALL_TYPO_TERMS", "11"),
            ("BILBO_RECALL_TYPO_LETTERS", "1"),
            ("BILBO_RECALL_TYPO_MAX_LETTERS", "65"),
            ("BILBO_RECALL_TYPO_TWO_EDITS", "66"),
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
            "recall.typo_terms = 0\nrecall.typo_letters = 2\nrecall.typo_max_letters = 2\nrecall.typo_two_edits = 2",
        )
        .expect("valid");
        assert_eq!(
            (
                low.typo_terms.value,
                low.typo_letters.value,
                low.typo_max_letters.value,
                low.typo_two_edits.value
            ),
            (0, 2, 2, 2)
        );
        let high = resolve(
            &[
                ("BILBO_RECALL_TYPO_TERMS", "10"),
                ("BILBO_RECALL_TYPO_LETTERS", "64"),
                ("BILBO_RECALL_TYPO_MAX_LETTERS", "64"),
                ("BILBO_RECALL_TYPO_TWO_EDITS", "65"),
            ],
            "recall.typo_terms = 1",
        )
        .expect("valid");
        assert_eq!(
            (
                high.typo_terms.value,
                high.typo_letters.value,
                high.typo_max_letters.value,
                high.typo_two_edits.value
            ),
            (10, 64, 64, 65)
        );
    }

    #[test]
    fn the_pair_weight_is_a_number_from_0_to_10() {
        for text in [
            "recall.pair_weight = -0.1",
            "recall.pair_weight = 10.1",
            "recall.pair_weight = nan",
            "recall.pair_weight = \"0.25\"",
        ] {
            let error = resolve(&[], text).expect_err(text);
            assert!(
                error.starts_with("/c/config.toml:1: ")
                    && error.contains("recall.pair_weight must be a number from 0 to 10, got "),
                "{text}: {error}"
            );
        }
        assert!(
            resolve(&[("BILBO_RECALL_PAIR_WEIGHT", "10.1")], "")
                .expect_err("refused")
                .starts_with("BILBO_RECALL_PAIR_WEIGHT must be a number from 0 to 10")
        );
        for (text, wanted) in [
            ("recall.pair_weight = 0", 0.0),
            ("recall.pair_weight = 0.25", 0.25),
            ("recall.pair_weight = 10", 10.0),
        ] {
            let weight = resolve(&[], text).expect(text).pair_weight.value;
            assert!((weight - wanted).abs() < f64::EPSILON, "{text}");
        }
        let weight = resolve(
            &[("BILBO_RECALL_PAIR_WEIGHT", "0.5")],
            "recall.pair_weight = 1",
        )
        .expect("valid")
        .pair_weight
        .value;
        assert!((weight - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let text = "[recall]\nbudget = 777\nnotes_budget = 888\ncandidates = 9\ncap = 2\nrecord_cap = 3\ncopy_share = 0.25\ncopy_words = 7\nquestion_words = [\"why\", \"who\"]\nlanguages = []\nbm25_k1 = 1.5\nbm25_b = 0.4\nread_batch = 99\ntypo_terms = 5\ntypo_letters = 6\ntypo_max_letters = 12\ntypo_two_edits = 10\npair_weight = 0.25\n";
        for section in [defaults(), resolve(&[], text).expect("valid")] {
            let mut written = String::from("[recall]\n");
            for shown in section.shown() {
                let key = shown.name.strip_prefix("recall.").expect("a recall key");
                written.push_str(&format!("{key} = {}\n", shown.value));
            }
            let again = resolve(&[], &written).expect("reads back");
            assert_eq!(again.budget.value, section.budget.value);
            assert_eq!(again.notes_budget.value, section.notes_budget.value);
            assert_eq!(again.candidates.value, section.candidates.value);
            assert_eq!(again.cap.value, section.cap.value);
            assert_eq!(again.record_cap.value, section.record_cap.value);
            assert!((again.copy_share.value - section.copy_share.value).abs() < f64::EPSILON);
            assert_eq!(again.copy_words.value, section.copy_words.value);
            assert_eq!(again.question_words.value, section.question_words.value);
            assert_eq!(again.languages.value, section.languages.value);
            assert!((again.bm25_k1.value - section.bm25_k1.value).abs() < f64::EPSILON);
            assert!((again.bm25_b.value - section.bm25_b.value).abs() < f64::EPSILON);
            assert_eq!(again.read_batch.value, section.read_batch.value);
            assert_eq!(again.typo_terms.value, section.typo_terms.value);
            assert_eq!(again.typo_letters.value, section.typo_letters.value);
            assert_eq!(again.typo_max_letters.value, section.typo_max_letters.value);
            assert_eq!(again.typo_two_edits.value, section.typo_two_edits.value);
            assert!((again.pair_weight.value - section.pair_weight.value).abs() < f64::EPSILON);
        }
    }
}
