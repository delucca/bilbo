use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source};
use bilbo_settings::size::{self, GIB, MIB};

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

pub const WORKERS: Key<usize> = Key {
    about: About {
        name: "search.workers",
        variable: Some("BILBO_SEARCH_WORKERS"),
        default: Default::Toml("0"),
        allowed: "a whole number from 0 to 64",
        doc: "The threads that read and chunk entries when the index is built or brought up to date; 0 takes one for each core. It changes no document.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|threads| usize::try_from(threads).ok())
            .filter(|threads| (0..=64).contains(threads))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const INDEX_MEMORY: Key<u64> = Key {
    about: About {
        name: "search.index_memory",
        variable: Some("BILBO_SEARCH_INDEX_MEMORY"),
        default: Default::Toml(r#""64 MiB""#),
        allowed: "between 16 MiB and 2 GiB",
        doc: "The memory the index's writer may hold before it writes a segment. It changes no document.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (16 * MIB..=2 * GIB).contains(bytes))
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(*bytes),
};

pub const MERGE_DELETED_PERCENT: Key<u8> = Key {
    about: About {
        name: "search.merge_deleted_percent",
        variable: Some("BILBO_SEARCH_MERGE_DELETED_PERCENT"),
        default: Default::Toml("20"),
        allowed: "a whole number from 1 to 100",
        doc: "A segment of the index is merged when more than this share of its documents, in percent, was replaced; 100 never merges for that reason. It changes no document and no ranking: only how much space replaced documents hold until a merge.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|percent| u8::try_from(percent).ok())
            .filter(|percent| (1..=100).contains(percent))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ENTRY_TRIES: Key<u8> = Key {
    about: About {
        name: "search.entry_tries",
        variable: Some("BILBO_SEARCH_ENTRY_TRIES"),
        default: Default::Toml("3"),
        allowed: "a whole number from 1 to 9",
        doc: "How many runs in a row try to index an entry that failed, before it waits for a change to it or for bilbo reindex.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|tries| u8::try_from(tries).ok())
            .filter(|tries| (1..=9).contains(tries))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

pub const ABOUT: [&About; 6] = [
    &CHUNK_TOKENS.about,
    &CHUNK_OVERLAP.about,
    &WORKERS.about,
    &INDEX_MEMORY.about,
    &MERGE_DELETED_PERCENT.about,
    &ENTRY_TRIES.about,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub chunk_tokens: Resolved<usize>,
    pub chunk_overlap: Resolved<usize>,
    pub workers: Resolved<usize>,
    pub index_memory: Resolved<u64>,
    pub merge_deleted_percent: Resolved<u8>,
    pub entry_tries: Resolved<u8>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        chunk_tokens: CHUNK_TOKENS.resolve(source)?,
        chunk_overlap: CHUNK_OVERLAP.resolve(source)?,
        workers: WORKERS.resolve(source)?,
        index_memory: INDEX_MEMORY.resolve(source)?,
        merge_deleted_percent: MERGE_DELETED_PERCENT.resolve(source)?,
        entry_tries: ENTRY_TRIES.resolve(source)?,
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
    pub fn chunk(&self) -> Size {
        Size {
            tokens: self.chunk_tokens.value,
            overlap: self.chunk_overlap.value,
        }
    }

    /// The threads to use: `cores` when the key is 0.
    pub fn workers(&self, cores: usize) -> usize {
        match self.workers.value {
            0 => cores,
            threads => threads,
        }
    }

    pub fn memory(&self) -> usize {
        usize::try_from(self.index_memory.value).unwrap_or(usize::MAX)
    }

    pub fn merge_deleted_share(&self) -> f32 {
        f32::from(self.merge_deleted_percent.value) / 100.0
    }

    pub fn tries(&self) -> u8 {
        self.entry_tries.value
    }

    pub fn shown(&self) -> [Shown; 6] {
        [
            CHUNK_TOKENS.shown(&self.chunk_tokens),
            CHUNK_OVERLAP.shown(&self.chunk_overlap),
            WORKERS.shown(&self.workers),
            INDEX_MEMORY.shown(&self.index_memory),
            MERGE_DELETED_PERCENT.shown(&self.merge_deleted_percent),
            ENTRY_TRIES.shown(&self.entry_tries),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;
    use bilbo_settings::size::MIB;

    use super::{ABOUT, Section, defaults, section};
    use crate::chunk::Size;

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
    fn the_defaults_are_the_evals_512_tokens_and_64_of_overlap() {
        assert_eq!(
            defaults().chunk(),
            Size {
                tokens: 512,
                overlap: 64
            }
        );
        assert_eq!(
            shown(&defaults())[..2],
            [
                "search.chunk_tokens = 512 (default)",
                "search.chunk_overlap = 64 (default)"
            ]
        );
    }

    #[test]
    fn the_defaults_of_a_run_are_a_worker_a_core_64_mib_a_fifth_deleted_and_three_tries() {
        let section = defaults();
        assert_eq!(section.workers(8), 8);
        assert_eq!(section.memory(), 64 * 1024 * 1024);
        assert!((section.merge_deleted_share() - 0.2).abs() < f32::EPSILON);
        assert_eq!(section.tries(), 3);
        assert_eq!(
            shown(&section)[2..],
            [
                "search.workers = 0 (default)",
                "search.index_memory = \"64 MiB\" (default)",
                "search.merge_deleted_percent = 20 (default)",
                "search.entry_tries = 3 (default)"
            ]
        );
    }

    #[test]
    fn each_key_takes_its_variable_over_the_file() {
        let text = "[search]\nchunk_tokens = 1024\nchunk_overlap = 0\nworkers = 2\nindex_memory = \"32 MiB\"\nmerge_deleted_percent = 50\nentry_tries = 9\n";
        let size = |tokens, overlap| Size { tokens, overlap };
        let from_file = resolve(&[], text).expect("valid");
        assert_eq!(from_file.chunk(), size(1024, 0));
        assert_eq!(from_file.workers(8), 2);
        assert_eq!(from_file.memory() as u64, 32 * MIB);
        assert!((from_file.merge_deleted_share() - 0.5).abs() < f32::EPSILON);
        assert_eq!(from_file.tries(), 9);
        let env = [
            ("BILBO_SEARCH_CHUNK_TOKENS", "256"),
            ("BILBO_SEARCH_CHUNK_OVERLAP", "128"),
            ("BILBO_SEARCH_WORKERS", "64"),
            ("BILBO_SEARCH_INDEX_MEMORY", "1GiB"),
            ("BILBO_SEARCH_MERGE_DELETED_PERCENT", "100"),
            ("BILBO_SEARCH_ENTRY_TRIES", "1"),
        ];
        let from_env = resolve(&env, text).expect("valid");
        assert_eq!(from_env.chunk(), size(256, 128));
        assert_eq!(from_env.workers(8), 64);
        assert_eq!(from_env.memory() as u64, 1024 * MIB);
        assert!((from_env.merge_deleted_share() - 1.0).abs() < f32::EPSILON);
        assert_eq!(from_env.tries(), 1);
        assert_eq!(
            resolve(&[], "search.chunk_tokens = 8192")
                .expect("valid")
                .chunk(),
            size(8192, 64)
        );
        assert_eq!(
            shown(&from_file)[3],
            "search.index_memory = \"32 MiB\" (/c/config.toml:5)"
        );
    }

    #[test]
    fn the_index_memory_is_a_size_or_a_number_of_bytes() {
        for (text, bytes) in [
            ("\"16 MiB\"", 16 * MIB),
            ("\"32 MiB\"", 32 * MIB),
            ("\"2 GiB\"", 2048 * MIB),
            ("33554432", 32 * MIB),
        ] {
            let section = resolve(&[], &format!("search.index_memory = {text}")).expect("a size");
            assert_eq!(section.memory() as u64, bytes, "{text}");
        }
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
            (
                "search.workers = -1",
                "/c/config.toml:1: search.workers must be a whole number from 0 to 64, got -1",
            ),
            (
                "search.workers = 65",
                "/c/config.toml:1: search.workers must be a whole number from 0 to 64, got 65",
            ),
            (
                "search.index_memory = \"15 MiB\"",
                "/c/config.toml:1: search.index_memory must be between 16 MiB and 2 GiB, got \"15 MiB\"",
            ),
            (
                "search.index_memory = \"2049 MiB\"",
                "/c/config.toml:1: search.index_memory must be between 16 MiB and 2 GiB, got \"2049 MiB\"",
            ),
            (
                "search.merge_deleted_percent = 0",
                "/c/config.toml:1: search.merge_deleted_percent must be a whole number from 1 to 100, got 0",
            ),
            (
                "search.merge_deleted_percent = 101",
                "/c/config.toml:1: search.merge_deleted_percent must be a whole number from 1 to 100, got 101",
            ),
            (
                "search.entry_tries = 0",
                "/c/config.toml:1: search.entry_tries must be a whole number from 1 to 9, got 0",
            ),
            (
                "search.entry_tries = 10",
                "/c/config.toml:1: search.entry_tries must be a whole number from 1 to 9, got 10",
            ),
        ] {
            assert_eq!(resolve(&[], text).expect_err("refused"), message, "{text}");
        }
        for (name, value, message) in [
            (
                "BILBO_SEARCH_CHUNK_TOKENS",
                "12",
                "must be a whole number from 256 to 8192",
            ),
            (
                "BILBO_SEARCH_WORKERS",
                "65",
                "must be a whole number from 0 to 64",
            ),
            (
                "BILBO_SEARCH_INDEX_MEMORY",
                "1 MiB",
                "must be between 16 MiB and 2 GiB",
            ),
            (
                "BILBO_SEARCH_MERGE_DELETED_PERCENT",
                "0",
                "must be a whole number from 1 to 100",
            ),
            (
                "BILBO_SEARCH_ENTRY_TRIES",
                "10",
                "must be a whole number from 1 to 9",
            ),
        ] {
            assert_eq!(
                resolve(&[(name, value)], "").expect_err("refused"),
                format!("{name} {message}, got \"{value}\"")
            );
        }
    }

    #[test]
    fn the_search_section_declares_six_keys_each_with_its_own_variable() {
        let names: Vec<(&str, Option<&str>)> = ABOUT
            .iter()
            .map(|about| (about.name, about.variable))
            .collect();
        assert_eq!(
            names,
            [
                ("search.chunk_tokens", Some("BILBO_SEARCH_CHUNK_TOKENS")),
                ("search.chunk_overlap", Some("BILBO_SEARCH_CHUNK_OVERLAP")),
                ("search.workers", Some("BILBO_SEARCH_WORKERS")),
                ("search.index_memory", Some("BILBO_SEARCH_INDEX_MEMORY")),
                (
                    "search.merge_deleted_percent",
                    Some("BILBO_SEARCH_MERGE_DELETED_PERCENT")
                ),
                ("search.entry_tries", Some("BILBO_SEARCH_ENTRY_TRIES")),
            ]
        );
    }
}
