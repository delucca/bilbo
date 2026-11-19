use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::Duration;

use bilbo_settings::duration::{self, DAY, HOUR, MILLISECOND, MINUTE, SECOND};
use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::folder;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, Value, quote};
use bilbo_settings::size::{self, MIB};

use crate::{table, tries};

fn count(value: &Value<'_>, range: RangeInclusive<usize>) -> Result<usize, Error> {
    value
        .integer()
        .and_then(|count| usize::try_from(count).ok())
        .filter(|count| range.contains(count))
        .ok_or_else(|| value.refuse())
}

fn within(value: &Value<'_>, range: RangeInclusive<Duration>) -> Result<Duration, Error> {
    duration::read(value)
        .filter(|length| range.contains(length))
        .ok_or_else(|| value.refuse())
}

fn entries(value: &Value<'_>) -> Result<Vec<String>, Error> {
    let items = value.items().ok_or_else(|| value.refuse())?;
    items
        .iter()
        .map(|item| {
            item.text()
                .filter(|text| text.chars().any(char::is_alphanumeric))
                .map(str::to_owned)
                .ok_or_else(|| item.refuse())
        })
        .collect()
}

pub const ENABLED: Key<bool> = Key {
    about: About {
        name: "derive.enabled",
        variable: Some("BILBO_DERIVE_ENABLED"),
        default: Default::Toml("false"),
        allowed: "true or false",
        doc: "Whether bilbo derive may send entries to the provider. While it is false no bilbo verb sends anything.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const PROVIDER: Key<String> = Key {
    about: About {
        name: "derive.provider",
        variable: Some("BILBO_DERIVE_PROVIDER"),
        default: Default::Toml(r#""""#),
        allowed: "the name of a provider, at most 32 lower-case letters, or the empty text for none",
        doc: "The provider bilbo derive reaches a model through. bilbo derive --help lists the providers.",
    },
    parse: |value| {
        value
            .text()
            .filter(|name| name.len() <= 32 && name.bytes().all(|byte| byte.is_ascii_lowercase()))
            .map(str::to_owned)
            .ok_or_else(|| value.refuse())
    },
    show: |name| quote(name),
};

pub const IN: Key<Vec<String>> = Key {
    about: About {
        name: "derive.in",
        variable: None,
        default: Default::Toml("[]"),
        allowed: "a list of texts, each with a letter or a digit",
        doc: "The entries bilbo derive may send, each value as recall --in takes it. Empty: nothing is sent.",
    },
    parse: entries,
    show: |entries| table::show_list(entries),
};

pub const NOT_IN: Key<Vec<String>> = Key {
    about: About {
        name: "derive.not_in",
        variable: None,
        default: Default::Toml("[]"),
        allowed: "a list of texts, each with a letter or a digit",
        doc: "The entries bilbo derive never sends, whatever derive.in names, each value as recall --in takes it.",
    },
    parse: entries,
    show: |entries| table::show_list(entries),
};

pub const INSTRUCTIONS: Key<Vec<(String, PathBuf)>> = Key {
    about: About {
        name: "derive.instructions",
        variable: None,
        default: Default::Toml("{}"),
        allowed: "a table of one path a kind, each path absolute or starting with ~/",
        doc: "The file whose text bilbo derive sends as the instruction for a kind, in place of the one bilbo ships, or for a kind it ships none for.",
    },
    parse: |value| {
        let fields = value.fields().ok_or_else(|| value.refuse())?;
        fields
            .into_iter()
            .map(|(kind, item)| {
                if kind.is_empty() || !kind.bytes().all(|byte| byte.is_ascii_lowercase()) {
                    return Err(item.refuse());
                }
                Ok((kind.to_owned(), folder::path(&item)?))
            })
            .collect()
    },
    show: |table| {
        let paths: Vec<(String, String)> = table
            .iter()
            .map(|(kind, path)| (kind.clone(), path.to_string_lossy().into_owned()))
            .collect();
        table::show_texts(&paths)
    },
};

pub const QUIET: Key<Duration> = Key {
    about: About {
        name: "derive.quiet",
        variable: Some("BILBO_DERIVE_QUIET"),
        default: Default::Toml(r#""30m""#),
        allowed: "between 0s and 24h",
        doc: "How long ago an entry's newest records must have been sealed for bilbo derive to take the entry as finished.",
    },
    parse: |value| within(value, Duration::ZERO..=DAY),
    show: |length| duration::show(*length),
};

pub const WINDOW: Key<usize> = Key {
    about: About {
        name: "derive.window",
        variable: Some("BILBO_DERIVE_WINDOW"),
        default: Default::Toml("20000"),
        allowed: "a whole number from 1000 to 500000",
        doc: "The most tokens of an entry's records one call is sent, as bilbo counts them. The instruction and the earlier notes come on top.",
    },
    parse: |value| count(value, 1000..=500_000),
    show: ToString::to_string,
};

pub const PASSAGE: Key<usize> = Key {
    about: About {
        name: "derive.passage",
        variable: Some("BILBO_DERIVE_PASSAGE"),
        default: Default::Toml("300"),
        allowed: "0, or a whole number from 16 to 500000",
        doc: "The most tokens of one passage a call is sent: a longer one keeps that many from its start and its end, shared by derive.passage_head.",
    },
    parse: |value| {
        count(value, 0..=500_000)
            .ok()
            .filter(|tokens| *tokens == 0 || *tokens >= 16)
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const PASSAGE_HEAD: Key<f64> = Key {
    about: About {
        name: "derive.passage_head",
        variable: Some("BILBO_DERIVE_PASSAGE_HEAD"),
        default: Default::Toml("0.5"),
        allowed: "a number from 0 to 1",
        doc: "The share of the tokens a cut passage keeps that come from its start; the rest come from its end.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|share| (0.0..=1.0).contains(share))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const EARLIER: Key<usize> = Key {
    about: About {
        name: "derive.earlier",
        variable: Some("BILBO_DERIVE_EARLIER"),
        default: Default::Toml("3000"),
        allowed: "a whole number from 0 to 100000",
        doc: "The most tokens of an entry's newest earlier notes a call is shown.",
    },
    parse: |value| count(value, 0..=100_000),
    show: ToString::to_string,
};

pub const NAMES: Key<f64> = Key {
    about: About {
        name: "derive.names",
        variable: Some("BILBO_DERIVE_NAMES"),
        default: Default::Toml("0.5"),
        allowed: "a number from 0 to 1",
        doc: "The share of a note's names that must stand in what its call was sent, or the note is dropped.",
    },
    parse: |value| {
        value
            .decimal()
            .filter(|share| (0.0..=1.0).contains(share))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ANSWER_TOKENS: Key<usize> = Key {
    about: About {
        name: "derive.answer_tokens",
        variable: Some("BILBO_DERIVE_ANSWER_TOKENS"),
        default: Default::Toml("16000"),
        allowed: "a whole number from 256 to 1000000",
        doc: "The most tokens of an answer an API provider may write, its thinking included.",
    },
    parse: |value| count(value, 256..=1_000_000),
    show: ToString::to_string,
};

pub const RESPONSE: Key<u64> = Key {
    about: About {
        name: "derive.response",
        variable: Some("BILBO_DERIVE_RESPONSE"),
        default: Default::Toml(r#""4 MiB""#),
        allowed: "between 1 KiB and 64 MiB",
        doc: "The most bytes bilbo derive reads of a provider's response or of a tool's output.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (1024..=64 * MIB).contains(bytes))
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(*bytes),
};

pub const CALLS: Key<usize> = Key {
    about: About {
        name: "derive.calls",
        variable: Some("BILBO_DERIVE_CALLS"),
        default: Default::Toml("20"),
        allowed: "a whole number from 1 to 100000",
        doc: "The most requests one run sends to an API, or runs of a tool's program it starts, retries included.",
    },
    parse: |value| count(value, 1..=100_000),
    show: ToString::to_string,
};

pub const TIMEOUT: Key<Duration> = Key {
    about: About {
        name: "derive.timeout",
        variable: Some("BILBO_DERIVE_TIMEOUT"),
        default: Default::Toml(r#""5m""#),
        allowed: "between 1s and 10m",
        doc: "How long one call may take, and one run of a program that prints a key.",
    },
    parse: |value| within(value, SECOND..=10 * MINUTE),
    show: |length| duration::show(*length),
};

pub const POLL: Key<Duration> = Key {
    about: About {
        name: "derive.poll",
        variable: Some("BILBO_DERIVE_POLL"),
        default: Default::Toml(r#""5ms""#),
        allowed: "between 1ms and 1s",
        doc: "How often a call looks whether the program it started has ended: a child has no timed wait.",
    },
    parse: |value| within(value, MILLISECOND..=SECOND),
    show: |length| duration::show(*length),
};

pub const RETRIES: Key<usize> = Key {
    about: About {
        name: "derive.retries",
        variable: Some("BILBO_DERIVE_RETRIES"),
        default: Default::Toml("2"),
        allowed: "a whole number from 0 to 10",
        doc: "How many times a call is made again when the provider is busy.",
    },
    parse: |value| count(value, 0..=10),
    show: ToString::to_string,
};

pub const RETRY_WAIT: Key<Duration> = Key {
    about: About {
        name: "derive.retry_wait",
        variable: Some("BILBO_DERIVE_RETRY_WAIT"),
        default: Default::Toml(r#""30s""#),
        allowed: "between 1s and 10m",
        doc: "What the first retry waits. Each later one waits derive.retry_factor times the one before, and a wait the provider names wins.",
    },
    parse: |value| within(value, SECOND..=10 * MINUTE),
    show: |length| duration::show(*length),
};

pub const RETRY_FACTOR: Key<usize> = Key {
    about: About {
        name: "derive.retry_factor",
        variable: Some("BILBO_DERIVE_RETRY_FACTOR"),
        default: Default::Toml("2"),
        allowed: "a whole number from 1 to 10",
        doc: "How many times the wait before it a later retry waits: 1 keeps every wait the same.",
    },
    parse: |value| count(value, 1..=10),
    show: ToString::to_string,
};

pub const RETRY_LIMIT: Key<Duration> = Key {
    about: About {
        name: "derive.retry_limit",
        variable: Some("BILBO_DERIVE_RETRY_LIMIT"),
        default: Default::Toml(r#""5m""#),
        allowed: "between 1s and 1h",
        doc: "The longest wait before a retry: a longer one ends the run.",
    },
    parse: |value| within(value, SECOND..=HOUR),
    show: |length| duration::show(*length),
};

pub const TRIES: Key<usize> = Key {
    about: About {
        name: "derive.tries",
        variable: Some("BILBO_DERIVE_TRIES"),
        default: Default::Toml("3"),
        allowed: "a whole number from 1 to 9",
        doc: "How many refused or failed calls a stretch of records gets, one a run, before bilbo derive passes it over.",
    },
    parse: |value| count(value, 1..=usize::from(tries::MOST)),
    show: ToString::to_string,
};
pub const FAILURES: Key<usize> = Key {
    about: About {
        name: "derive.failures",
        variable: Some("BILBO_DERIVE_FAILURES"),
        default: Default::Toml("3"),
        allowed: "a whole number from 1 to 100",
        doc: "How many failed calls in a row, inside one run, stop the run with nothing charged for them.",
    },
    parse: |value| count(value, 1..=100),
    show: ToString::to_string,
};
pub const ABOUT: [&About; 22] = [
    &ENABLED.about,
    &PROVIDER.about,
    &IN.about,
    &NOT_IN.about,
    &INSTRUCTIONS.about,
    &QUIET.about,
    &WINDOW.about,
    &PASSAGE.about,
    &PASSAGE_HEAD.about,
    &EARLIER.about,
    &NAMES.about,
    &ANSWER_TOKENS.about,
    &RESPONSE.about,
    &CALLS.about,
    &TIMEOUT.about,
    &POLL.about,
    &RETRIES.about,
    &RETRY_WAIT.about,
    &RETRY_FACTOR.about,
    &RETRY_LIMIT.about,
    &TRIES.about,
    &FAILURES.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub enabled: Resolved<bool>,
    pub provider: Resolved<String>,
    pub named: Resolved<Vec<String>>,
    pub left_out: Resolved<Vec<String>>,
    pub instructions: Resolved<Vec<(String, PathBuf)>>,
    pub quiet: Resolved<Duration>,
    pub window: Resolved<usize>,
    pub passage: Resolved<usize>,
    pub passage_head: Resolved<f64>,
    pub earlier: Resolved<usize>,
    pub names: Resolved<f64>,
    pub answer_tokens: Resolved<usize>,
    pub response: Resolved<u64>,
    pub calls: Resolved<usize>,
    pub timeout: Resolved<Duration>,
    pub poll: Resolved<Duration>,
    pub retries: Resolved<usize>,
    pub retry_wait: Resolved<Duration>,
    pub retry_factor: Resolved<usize>,
    pub retry_limit: Resolved<Duration>,
    pub tries: Resolved<usize>,
    pub failures: Resolved<usize>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        enabled: ENABLED.resolve(source)?,
        provider: PROVIDER.resolve(source)?,
        named: IN.resolve(source)?,
        left_out: NOT_IN.resolve(source)?,
        instructions: INSTRUCTIONS.resolve(source)?,
        quiet: QUIET.resolve(source)?,
        window: WINDOW.resolve(source)?,
        passage: PASSAGE.resolve(source)?,
        passage_head: PASSAGE_HEAD.resolve(source)?,
        earlier: EARLIER.resolve(source)?,
        names: NAMES.resolve(source)?,
        answer_tokens: ANSWER_TOKENS.resolve(source)?,
        response: RESPONSE.resolve(source)?,
        calls: CALLS.resolve(source)?,
        timeout: TIMEOUT.resolve(source)?,
        poll: POLL.resolve(source)?,
        retries: RETRIES.resolve(source)?,
        retry_wait: RETRY_WAIT.resolve(source)?,
        retry_factor: RETRY_FACTOR.resolve(source)?,
        retry_limit: RETRY_LIMIT.resolve(source)?,
        tries: TRIES.resolve(source)?,
        failures: FAILURES.resolve(source)?,
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
    pub fn gate(&self) -> Result<(), &'static str> {
        if !self.enabled.value {
            return Err("derive.enabled is false");
        }
        if self.provider.value.is_empty() {
            return Err("derive.provider is not set");
        }
        if self.named.value.is_empty() {
            return Err("derive.in is empty");
        }
        Ok(())
    }

    pub fn shown(&self) -> [Shown; 22] {
        [
            ENABLED.shown(&self.enabled),
            PROVIDER.shown(&self.provider),
            IN.shown(&self.named),
            NOT_IN.shown(&self.left_out),
            INSTRUCTIONS.shown(&self.instructions),
            QUIET.shown(&self.quiet),
            WINDOW.shown(&self.window),
            PASSAGE.shown(&self.passage),
            PASSAGE_HEAD.shown(&self.passage_head),
            EARLIER.shown(&self.earlier),
            NAMES.shown(&self.names),
            ANSWER_TOKENS.shown(&self.answer_tokens),
            RESPONSE.shown(&self.response),
            CALLS.shown(&self.calls),
            TIMEOUT.shown(&self.timeout),
            POLL.shown(&self.poll),
            RETRIES.shown(&self.retries),
            RETRY_WAIT.shown(&self.retry_wait),
            RETRY_FACTOR.shown(&self.retry_factor),
            RETRY_LIMIT.shown(&self.retry_limit),
            TRIES.shown(&self.tries),
            FAILURES.shown(&self.failures),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

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

    fn given(text: &str) -> Section {
        resolve(&[("HOME", "/home/me")], text).expect("resolves")
    }

    fn refused(text: &str) -> String {
        resolve(&[("HOME", "/home/me")], text).expect_err("refused")
    }

    fn seconds(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = defaults();
        assert!(!section.enabled.value);
        assert_eq!(section.provider.value, "");
        assert!(section.named.value.is_empty());
        assert!(section.left_out.value.is_empty());
        assert!(section.instructions.value.is_empty());
        assert_eq!(section.quiet.value, seconds(30 * 60));
        assert_eq!(section.window.value, 20_000);
        assert_eq!(section.passage.value, 300);
        assert!((section.passage_head.value - 0.5).abs() < f64::EPSILON);
        assert_eq!(section.earlier.value, 3000);
        assert!((section.names.value - 0.5).abs() < f64::EPSILON);
        assert_eq!(section.answer_tokens.value, 16_000);
        assert_eq!(section.response.value, 4 << 20);
        assert_eq!(section.calls.value, 20);
        assert_eq!(section.timeout.value, seconds(300));
        assert_eq!(section.poll.value, Duration::from_millis(5));
        assert_eq!(section.retries.value, 2);
        assert_eq!(section.retry_wait.value, seconds(30));
        assert_eq!(section.retry_factor.value, 2);
        assert_eq!(section.retry_limit.value, seconds(300));
        assert_eq!(section.tries.value, 3);
        assert_eq!(section.failures.value, 3);
        let shown: Vec<String> = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "derive.enabled = false (default)",
                "derive.provider = \"\" (default)",
                "derive.in = [] (default)",
                "derive.not_in = [] (default)",
                "derive.instructions = {} (default)",
                "derive.quiet = \"30m\" (default)",
                "derive.window = 20000 (default)",
                "derive.passage = 300 (default)",
                "derive.passage_head = 0.5 (default)",
                "derive.earlier = 3000 (default)",
                "derive.names = 0.5 (default)",
                "derive.answer_tokens = 16000 (default)",
                "derive.response = \"4 MiB\" (default)",
                "derive.calls = 20 (default)",
                "derive.timeout = \"5m\" (default)",
                "derive.poll = \"5ms\" (default)",
                "derive.retries = 2 (default)",
                "derive.retry_wait = \"30s\" (default)",
                "derive.retry_factor = 2 (default)",
                "derive.retry_limit = \"5m\" (default)",
                "derive.tries = 3 (default)",
                "derive.failures = 3 (default)",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_DERIVE_ENABLED"),
                Some("BILBO_DERIVE_PROVIDER"),
                None,
                None,
                None,
                Some("BILBO_DERIVE_QUIET"),
                Some("BILBO_DERIVE_WINDOW"),
                Some("BILBO_DERIVE_PASSAGE"),
                Some("BILBO_DERIVE_PASSAGE_HEAD"),
                Some("BILBO_DERIVE_EARLIER"),
                Some("BILBO_DERIVE_NAMES"),
                Some("BILBO_DERIVE_ANSWER_TOKENS"),
                Some("BILBO_DERIVE_RESPONSE"),
                Some("BILBO_DERIVE_CALLS"),
                Some("BILBO_DERIVE_TIMEOUT"),
                Some("BILBO_DERIVE_POLL"),
                Some("BILBO_DERIVE_RETRIES"),
                Some("BILBO_DERIVE_RETRY_WAIT"),
                Some("BILBO_DERIVE_RETRY_FACTOR"),
                Some("BILBO_DERIVE_RETRY_LIMIT"),
                Some("BILBO_DERIVE_TRIES"),
                Some("BILBO_DERIVE_FAILURES"),
            ]
        );
    }

    #[test]
    fn the_switch_is_true_or_false() {
        assert!(given("derive.enabled = true").enabled.value);
        assert!(!given("derive.enabled = false").enabled.value);
        let var = |value| resolve(&[("BILBO_DERIVE_ENABLED", value)], "");
        assert!(var("true").expect("true").enabled.value);
        assert!(!var("false").expect("false").enabled.value);
        assert_eq!(
            refused("derive.enabled = \"true\""),
            "/c/config.toml:1: derive.enabled must be true or false, got \"true\""
        );
        assert_eq!(
            refused("derive.enabled = 1"),
            "/c/config.toml:1: derive.enabled must be true or false, got 1"
        );
        for value in ["yes", "1", "True"] {
            assert_eq!(
                var(value).expect_err("refused"),
                format!("BILBO_DERIVE_ENABLED must be true or false, got {value:?}")
            );
        }
    }

    #[test]
    fn the_provider_is_lower_case_letters_or_nothing() {
        for name in ["", "anthropic", &"a".repeat(32)] {
            let section = given(&format!("derive.provider = {name:?}"));
            assert_eq!(section.provider.value, name);
        }
        for name in ["Anthropic", "a b", "x1", &"a".repeat(33)] {
            assert!(
                resolve(&[], &format!("derive.provider = {name:?}")).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn the_lists_of_entries_hold_values_recall_in_takes() {
        let section = given(
            "derive.in = [\"01JC5Q3V7XKM4P8R2T6W9YB0DB\", \"garden notes\", \"7\"]\nderive.not_in = [\"x\"]",
        );
        assert_eq!(
            section.named.value,
            ["01JC5Q3V7XKM4P8R2T6W9YB0DB", "garden notes", "7"]
        );
        assert_eq!(section.left_out.value, ["x"]);
        for name in ["derive.in", "derive.not_in"] {
            for (text, bad) in [
                ("[\"\"]", "\"\""),
                ("[\"--\"]", "\"--\""),
                ("[7]", "7"),
                ("[\"ok\", \"--\"]", "\"--\""),
            ] {
                let message = refused(&format!("{name} = {text}"));
                assert!(
                    message.starts_with(&format!("/c/config.toml:1: {name}")),
                    "{message}"
                );
                assert!(message.ends_with(&format!(", got {bad}")), "{message}");
            }
            let message = refused(&format!("{name} = \"alone\""));
            assert!(message.ends_with(", got \"alone\""), "{message}");
        }
    }

    #[test]
    fn the_instructions_are_a_table_of_one_path_a_kind() {
        let section = given(
            "derive.instructions = { plain = \"/a/plain.txt\", folded = \"~/b/folded.txt\" }",
        );
        assert_eq!(
            section.instructions.value,
            [
                ("plain".to_owned(), PathBuf::from("/a/plain.txt")),
                ("folded".to_owned(), PathBuf::from("/home/me/b/folded.txt")),
            ]
        );
        assert!(
            given("derive.instructions = {}")
                .instructions
                .value
                .is_empty()
        );
        for (text, message) in [
            (
                "derive.instructions = { plain = \"a/plain.txt\" }",
                "/c/config.toml:1: derive.instructions.plain",
            ),
            (
                "derive.instructions = { plain = 3 }",
                "/c/config.toml:1: derive.instructions.plain",
            ),
            (
                "derive.instructions = { Plain = \"/a\" }",
                "/c/config.toml:1: derive.instructions.Plain",
            ),
            (
                "derive.instructions = { p1 = \"/a\" }",
                "/c/config.toml:1: derive.instructions.p1",
            ),
            (
                "derive.instructions = \"/a\"",
                "/c/config.toml:1: derive.instructions",
            ),
        ] {
            let found = refused(text);
            assert!(found.starts_with(message), "{text}: {found}");
        }
    }

    fn durations(name: &str, accepted: &[&str], refused_values: &[&str]) {
        for text in accepted {
            assert!(
                resolve(&[], &format!("{name} = \"{text}\"")).is_ok(),
                "{name} {text}"
            );
        }
        for text in refused_values {
            assert!(
                resolve(&[], &format!("{name} = \"{text}\"")).is_err(),
                "{name} {text}"
            );
        }
    }

    #[test]
    fn the_durations_stand_in_their_ranges() {
        durations("derive.quiet", &["0s", "30m", "31m", "24h"], &["25h"]);
        durations("derive.timeout", &["1s", "5m", "6m", "10m"], &["0s", "11m"]);
        durations("derive.poll", &["1ms", "5ms", "6ms", "1s"], &["0ms", "2s"]);
        durations(
            "derive.retry_wait",
            &["1s", "30s", "31s", "10m"],
            &["0s", "11m"],
        );
        durations(
            "derive.retry_limit",
            &["1s", "5m", "6m", "1h"],
            &["0s", "61m"],
        );
    }

    fn integers(name: &str, accepted: &[i64], refused_values: &[i64]) {
        for number in accepted {
            assert!(
                resolve(&[], &format!("{name} = {number}")).is_ok(),
                "{name} {number}"
            );
        }
        for number in refused_values {
            assert!(
                resolve(&[], &format!("{name} = {number}")).is_err(),
                "{name} {number}"
            );
        }
    }

    #[test]
    fn the_token_keys_stand_in_their_ranges() {
        integers(
            "derive.window",
            &[1000, 20_000, 20_001, 500_000],
            &[999, 500_001],
        );
        integers(
            "derive.passage",
            &[0, 16, 300, 301, 500_000],
            &[1, 15, 500_001],
        );
        integers("derive.earlier", &[0, 3000, 3001, 100_000], &[-1, 100_001]);
        integers(
            "derive.answer_tokens",
            &[256, 16_000, 16_001, 1_000_000],
            &[255, 1_000_001],
        );
    }

    #[test]
    fn a_share_is_a_number_from_0_to_1() {
        for name in ["derive.names", "derive.passage_head"] {
            for text in ["0", "0.5", "0.51", "1"] {
                assert!(
                    resolve(&[], &format!("{name} = {text}")).is_ok(),
                    "{name} {text}"
                );
            }
            for text in ["-0.1", "1.01", "\"half\""] {
                assert!(
                    resolve(&[], &format!("{name} = {text}")).is_err(),
                    "{name} {text}"
                );
            }
        }
    }

    #[test]
    fn the_response_size_stands_in_its_range() {
        for text in ["\"1 KiB\"", "\"4 MiB\"", "\"5 MiB\"", "\"64 MiB\"", "1024"] {
            assert!(
                resolve(&[], &format!("derive.response = {text}")).is_ok(),
                "{text}"
            );
        }
        for text in ["1023", "\"65 MiB\""] {
            assert!(
                resolve(&[], &format!("derive.response = {text}")).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn the_counts_stand_in_their_ranges() {
        integers("derive.calls", &[1, 20, 21, 100_000], &[0, 100_001]);
        integers("derive.retries", &[0, 2, 3, 10], &[-1, 11]);
        integers("derive.retry_factor", &[1, 2, 3, 10], &[0, 11]);
        integers("derive.tries", &[1, 3, 4, 9], &[0, 10]);
        integers("derive.failures", &[1, 3, 4, 100], &[0, 101]);
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let text = "derive.enabled = true\nderive.provider = \"anthropic\"\nderive.in = [\"a b\", \"c\\\"d\"]\nderive.not_in = [\"x1\"]\nderive.instructions = { plain = \"/a/b.txt\", folded = \"~/c.txt\" }\nderive.quiet = \"90s\"\nderive.window = 1500\nderive.passage = 0\nderive.passage_head = 0.75\nderive.earlier = 7\nderive.names = 0.25\nderive.answer_tokens = 300\nderive.response = \"2 KiB\"\nderive.calls = 3\nderive.timeout = \"2m\"\nderive.poll = \"20ms\"\nderive.retries = 0\nderive.retry_wait = \"45s\"\nderive.retry_factor = 3\nderive.retry_limit = \"1h\"\nderive.tries = 9\nderive.failures = 2\n";
        let section = given(text);
        let literals = [
            "true",
            "\"anthropic\"",
            "[\"a b\", \"c\\\"d\"]",
            "[\"x1\"]",
            "{ plain = \"/a/b.txt\", folded = \"/home/me/c.txt\" }",
            "\"90s\"",
            "1500",
            "0",
            "0.75",
            "7",
            "0.25",
            "300",
            "\"2 KiB\"",
            "3",
            "\"2m\"",
            "\"20ms\"",
            "0",
            "\"45s\"",
            "3",
            "\"60m\"",
            "9",
            "2",
        ];
        let shown: Vec<String> = section
            .shown()
            .iter()
            .map(|item| item.value.clone())
            .collect();
        assert_eq!(shown, literals);
        let again: String = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {}\n", shown.name, shown.value))
            .collect();
        let section_again = given(&again);
        assert_eq!(section_again.named.value, ["a b", "c\"d"]);
        for (before, after) in section.shown().iter().zip(section_again.shown()) {
            assert_eq!(before.value, after.value, "{}", before.name);
        }
    }

    #[test]
    fn the_gate_names_the_first_key_that_is_unset() {
        let mut section = defaults();
        assert_eq!(section.gate(), Err("derive.enabled is false"));
        section.enabled.value = true;
        assert_eq!(section.gate(), Err("derive.provider is not set"));
        section.provider.value = "anthropic".to_owned();
        assert_eq!(section.gate(), Err("derive.in is empty"));
        section.named.value = vec!["a".to_owned()];
        assert_eq!(section.gate(), Ok(()));
        section.enabled.value = false;
        assert_eq!(section.gate(), Err("derive.enabled is false"));
    }
}
