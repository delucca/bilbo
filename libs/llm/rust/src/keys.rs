use std::ops::RangeInclusive;
use std::time::Duration;

use bilbo_settings::duration::{self, MILLISECOND, MINUTE, SECOND};
use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, Value, quote};
use bilbo_settings::size::{self, MIB};

use crate::model::Limits;

fn within(value: &Value<'_>, range: RangeInclusive<Duration>) -> Result<Duration, Error> {
    duration::read(value)
        .filter(|length| range.contains(length))
        .ok_or_else(|| value.refuse())
}

pub const PROVIDER: Key<String> = Key {
    about: About {
        name: "llm.provider",
        variable: Some("BILBO_LLM_PROVIDER"),
        default: Default::Toml(r#""""#),
        allowed: "the name of a provider, at most 32 lower-case letters, or the empty text for none",
        doc: "The provider whose models bilbo calls. llm.<provider>.via names the way bilbo reaches them. bilbo derive --help lists the providers.",
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

pub const RESPONSE: Key<u64> = Key {
    about: About {
        name: "llm.response",
        variable: Some("BILBO_LLM_RESPONSE"),
        default: Default::Toml(r#""4 MiB""#),
        allowed: "between 1 KiB and 64 MiB",
        doc: "The most bytes bilbo reads of a provider's response or of a tool's output.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (1024..=64 * MIB).contains(bytes))
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(*bytes),
};

pub const TIMEOUT: Key<Duration> = Key {
    about: About {
        name: "llm.timeout",
        variable: Some("BILBO_LLM_TIMEOUT"),
        default: Default::Toml(r#""5m""#),
        allowed: "between 1s and 10m",
        doc: "How long one call may take, and one run of a program that prints a key.",
    },
    parse: |value| within(value, SECOND..=10 * MINUTE),
    show: |length| duration::show(*length),
};

pub const POLL: Key<Duration> = Key {
    about: About {
        name: "llm.poll",
        variable: Some("BILBO_LLM_POLL"),
        default: Default::Toml(r#""5ms""#),
        allowed: "between 1ms and 1s",
        doc: "How often a call looks whether the program it started has ended: a child has no timed wait.",
    },
    parse: |value| within(value, MILLISECOND..=SECOND),
    show: |length| duration::show(*length),
};

pub const ABOUT: [&About; 4] = [
    &PROVIDER.about,
    &RESPONSE.about,
    &TIMEOUT.about,
    &POLL.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub provider: Resolved<String>,
    pub response: Resolved<u64>,
    pub timeout: Resolved<Duration>,
    pub poll: Resolved<Duration>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        provider: PROVIDER.resolve(source)?,
        response: RESPONSE.resolve(source)?,
        timeout: TIMEOUT.resolve(source)?,
        poll: POLL.resolve(source)?,
    })
}

impl Section {
    pub fn shown(&self) -> [Shown; 4] {
        [
            PROVIDER.shown(&self.provider),
            RESPONSE.shown(&self.response),
            TIMEOUT.shown(&self.timeout),
            POLL.shown(&self.poll),
        ]
    }

    #[must_use]
    pub fn limits(&self, answer_tokens: usize) -> Limits {
        Limits {
            answer_tokens,
            timeout: self.timeout.value,
            response: self.response.value,
            poll: self.poll.value,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, section};

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
        resolve(&[], text).expect("resolves")
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = given("");
        assert_eq!(section.provider.value, "");
        assert_eq!(section.response.value, 4 << 20);
        assert_eq!(section.timeout.value, Duration::from_secs(300));
        assert_eq!(section.poll.value, Duration::from_millis(5));
        let shown: Vec<String> = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "llm.provider = \"\" (default)",
                "llm.response = \"4 MiB\" (default)",
                "llm.timeout = \"5m\" (default)",
                "llm.poll = \"5ms\" (default)",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_LLM_PROVIDER"),
                Some("BILBO_LLM_RESPONSE"),
                Some("BILBO_LLM_TIMEOUT"),
                Some("BILBO_LLM_POLL"),
            ]
        );
    }

    #[test]
    fn the_provider_is_lower_case_letters_or_nothing() {
        for name in ["", "anthropic", &"a".repeat(32)] {
            let section = given(&format!("llm.provider = {name:?}"));
            assert_eq!(section.provider.value, name);
        }
        for name in ["Anthropic", "a b", "x1", &"a".repeat(33)] {
            assert!(
                resolve(&[], &format!("llm.provider = {name:?}")).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn the_durations_stand_in_their_ranges() {
        for (name, accepted, refused) in [
            ("llm.timeout", ["1s", "5m", "6m", "10m"], ["0s", "11m"]),
            ("llm.poll", ["1ms", "5ms", "6ms", "1s"], ["0ms", "2s"]),
        ] {
            for text in accepted {
                assert!(
                    resolve(&[], &format!("{name} = \"{text}\"")).is_ok(),
                    "{name} {text}"
                );
            }
            for text in refused {
                assert!(
                    resolve(&[], &format!("{name} = \"{text}\"")).is_err(),
                    "{name} {text}"
                );
            }
        }
    }

    #[test]
    fn the_response_size_stands_in_its_range() {
        for text in ["\"1 KiB\"", "\"4 MiB\"", "\"5 MiB\"", "\"64 MiB\"", "1024"] {
            assert!(
                resolve(&[], &format!("llm.response = {text}")).is_ok(),
                "{text}"
            );
        }
        for text in ["1023", "\"65 MiB\""] {
            assert!(
                resolve(&[], &format!("llm.response = {text}")).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let text = "llm.provider = \"anthropic\"\nllm.response = \"2 KiB\"\nllm.timeout = \"2m\"\nllm.poll = \"20ms\"\n";
        let section = given(text);
        let shown: Vec<String> = section
            .shown()
            .iter()
            .map(|item| item.value.clone())
            .collect();
        assert_eq!(shown, ["\"anthropic\"", "\"2 KiB\"", "\"2m\"", "\"20ms\""]);
        let again: String = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {}\n", shown.name, shown.value))
            .collect();
        let section_again = given(&again);
        assert_eq!(section_again, section);
    }

    #[test]
    fn the_limits_are_the_three_keys_and_the_answer_length_given() {
        let section = given("llm.response = \"2 KiB\"\nllm.timeout = \"2m\"\nllm.poll = \"20ms\"");
        let limits = section.limits(1234);
        assert_eq!(limits.answer_tokens, 1234);
        assert_eq!(limits.timeout, Duration::from_secs(120));
        assert_eq!(limits.response, 2048);
        assert_eq!(limits.poll, Duration::from_millis(20));
    }
}
