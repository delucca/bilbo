use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source};

use crate::chunk::Size;

pub const CHUNK_TOKENS: Key<usize> = Key {
    about: About {
        name: "search.chunk_tokens",
        variable: Some("BILBO_SEARCH_CHUNK_TOKENS"),
        default: Default::Toml("512"),
        allowed: "a whole number from 256 to 8192",
        doc: "The most tokens a chunk of the index holds, as the Qwen3-Embedding tokenizer counts them. A change takes a new index: run bilbo reindex.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|tokens| usize::try_from(tokens).ok())
            .filter(|tokens| (256..=8192).contains(tokens))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const CHUNK_OVERLAP: Key<usize> = Key {
    about: About {
        name: "search.chunk_overlap",
        variable: Some("BILBO_SEARCH_CHUNK_OVERLAP"),
        default: Default::Toml("64"),
        allowed: "a whole number from 0 to 128",
        doc: "The tokens two consecutive chunks share when one record is too long for a chunk and is cut into several. A change takes a new index: run bilbo reindex.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|tokens| usize::try_from(tokens).ok())
            .filter(|tokens| (0..=128).contains(tokens))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ABOUT: [&About; 2] = [&CHUNK_TOKENS.about, &CHUNK_OVERLAP.about];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub chunk_tokens: Resolved<usize>,
    pub chunk_overlap: Resolved<usize>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        chunk_tokens: CHUNK_TOKENS.resolve(source)?,
        chunk_overlap: CHUNK_OVERLAP.resolve(source)?,
    })
}

impl Section {
    pub fn chunk(&self) -> Size {
        Size {
            tokens: self.chunk_tokens.value,
            overlap: self.chunk_overlap.value,
        }
    }

    pub fn shown(&self) -> [Shown; 2] {
        [
            CHUNK_TOKENS.shown(&self.chunk_tokens),
            CHUNK_OVERLAP.shown(&self.chunk_overlap),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, section};
    use crate::chunk::Size;

    fn defaults() -> Section {
        let file = File::default();
        section(&Source {
            var: &|_| None,
            file: &file,
        })
        .expect("each default is a literal its key's unit test parses")
    }

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
    fn the_defaults_are_the_evals_512_tokens_and_64_of_overlap() {
        assert_eq!(
            defaults().chunk(),
            Size {
                tokens: 512,
                overlap: 64
            }
        );
        let shown: Vec<String> = defaults()
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "search.chunk_tokens = 512 (default)",
                "search.chunk_overlap = 64 (default)"
            ]
        );
        assert_eq!(
            ABOUT.map(|about| about.name),
            ["search.chunk_tokens", "search.chunk_overlap"]
        );
    }

    #[test]
    fn each_key_takes_its_variable_over_the_file() {
        let text = "[search]\nchunk_tokens = 1024\nchunk_overlap = 0\n";
        let size = |tokens, overlap| Size { tokens, overlap };
        assert_eq!(resolve(&[], text).expect("valid").chunk(), size(1024, 0));
        let env = [
            ("BILBO_SEARCH_CHUNK_TOKENS", "256"),
            ("BILBO_SEARCH_CHUNK_OVERLAP", "128"),
        ];
        assert_eq!(resolve(&env, text).expect("valid").chunk(), size(256, 128));
        assert_eq!(
            resolve(&[], "search.chunk_tokens = 8192")
                .expect("valid")
                .chunk(),
            size(8192, 64)
        );
    }

    #[test]
    fn each_key_refuses_a_value_out_of_its_range() {
        for (text, message) in [
            (
                "search.chunk_tokens = 255",
                "/c/config.toml:1: search.chunk_tokens must be a whole number from 256 to 8192, got 255",
            ),
            (
                "search.chunk_tokens = 8193",
                "/c/config.toml:1: search.chunk_tokens must be a whole number from 256 to 8192, got 8193",
            ),
            (
                "search.chunk_tokens = \"512\"",
                "/c/config.toml:1: search.chunk_tokens must be a whole number from 256 to 8192, got \"512\"",
            ),
            (
                "search.chunk_overlap = -1",
                "/c/config.toml:1: search.chunk_overlap must be a whole number from 0 to 128, got -1",
            ),
            (
                "search.chunk_overlap = 129",
                "/c/config.toml:1: search.chunk_overlap must be a whole number from 0 to 128, got 129",
            ),
        ] {
            assert_eq!(resolve(&[], text).expect_err("refused"), message, "{text}");
        }
        assert_eq!(
            resolve(&[("BILBO_SEARCH_CHUNK_TOKENS", "12")], "").expect_err("refused"),
            "BILBO_SEARCH_CHUNK_TOKENS must be a whole number from 256 to 8192, got \"12\""
        );
    }
}
