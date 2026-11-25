use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source};

pub const CANDIDATES: Key<usize> = Key {
    about: About {
        name: "digest.candidates",
        variable: Some("BILBO_DIGEST_CANDIDATES"),
        default: Default::Toml("1000"),
        allowed: "a whole number from 1 to 1000",
        doc: "How many of a list's best chunks bilbo digest considers before it lists the entries they belong to.",
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

pub const QUERY: Key<usize> = Key {
    about: About {
        name: "digest.query",
        variable: Some("BILBO_DIGEST_QUERY"),
        default: Default::Toml("2000"),
        allowed: "a whole number from 1 to 1000000",
        doc: "The most characters of a prompt bilbo digest searches by: a longer prompt is searched by its start.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| (1..=1_000_000).contains(count))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ABOUT: [&About; 2] = [&CANDIDATES.about, &QUERY.about];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub candidates: Resolved<usize>,
    pub query: Resolved<usize>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        candidates: CANDIDATES.resolve(source)?,
        query: QUERY.resolve(source)?,
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
    pub fn shown(&self) -> [Shown; 2] {
        [CANDIDATES.shown(&self.candidates), QUERY.shown(&self.query)]
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

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let section = resolve(&[], "digest.candidates = 7\ndigest.query = 99").expect("valid");
        let mut written = String::new();
        for shown in section.shown() {
            written.push_str(&format!("{} = {}\n", shown.name, shown.value));
        }
        let again = resolve(&[], &written).expect("reads back");
        assert_eq!(again.candidates.value, 7);
        assert_eq!(again.query.value, 99);
    }

    #[test]
    fn the_digests_keys_take_their_defaults_and_refuse_what_is_outside() {
        let by_default = defaults();
        assert_eq!(by_default.candidates.value, 1000);
        assert_eq!(by_default.query.value, 2000);
        let shown: Vec<String> = by_default
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "digest.candidates = 1000 (default)",
                "digest.query = 2000 (default)"
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [Some("BILBO_DIGEST_CANDIDATES"), Some("BILBO_DIGEST_QUERY")]
        );

        let low = resolve(&[], "digest.candidates = 1\ndigest.query = 1").expect("valid");
        assert_eq!((low.candidates.value, low.query.value), (1, 1));
        let high = resolve(&[], "digest.candidates = 1000\ndigest.query = 1000000").expect("valid");
        assert_eq!((high.candidates.value, high.query.value), (1000, 1_000_000));

        for (text, message) in [
            (
                "digest.candidates = 0",
                "/c/config.toml:1: digest.candidates must be a whole number from 1 to 1000, got 0",
            ),
            (
                "digest.candidates = 1001",
                "/c/config.toml:1: digest.candidates must be a whole number from 1 to 1000, got 1001",
            ),
            (
                "digest.query = 0",
                "/c/config.toml:1: digest.query must be a whole number from 1 to 1000000, got 0",
            ),
            (
                "digest.query = 1000001",
                "/c/config.toml:1: digest.query must be a whole number from 1 to 1000000, got 1000001",
            ),
            (
                "digest.candidates = \"1000\"",
                "/c/config.toml:1: digest.candidates must be a whole number from 1 to 1000, got \"1000\"",
            ),
            (
                "digest.query = \"2000\"",
                "/c/config.toml:1: digest.query must be a whole number from 1 to 1000000, got \"2000\"",
            ),
        ] {
            assert_eq!(resolve(&[], text).expect_err("refused"), message, "{text}");
        }

        let by_variable = resolve(
            &[
                ("BILBO_DIGEST_CANDIDATES", "7"),
                ("BILBO_DIGEST_QUERY", "9"),
            ],
            "digest.candidates = 1\ndigest.query = 1",
        )
        .expect("valid");
        assert_eq!(
            (by_variable.candidates.value, by_variable.query.value),
            (7, 9)
        );
        for (name, value) in [
            ("BILBO_DIGEST_CANDIDATES", "1001"),
            ("BILBO_DIGEST_QUERY", "0"),
        ] {
            assert!(
                resolve(&[(name, value)], "")
                    .expect_err("refused")
                    .starts_with(&format!("{name} must be a whole number")),
                "{name}"
            );
        }
    }
}
