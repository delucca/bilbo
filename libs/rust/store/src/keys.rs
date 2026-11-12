use std::time::Duration;

use bilbo_settings::duration::{self, DAY, MILLISECOND, MINUTE, SECOND};
use bilbo_settings::error::Error;
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source};
use bilbo_settings::size::{self, GIB, MIB};

use crate::limits::{Compression, TearBudget};
use crate::segments::{Held, Options, Rebuild};

pub const SEAL_SIZE: Key<u64> = Key {
    about: About {
        name: "store.seal_at",
        variable: Some("BILBO_STORE_SEAL_AT"),
        default: Default::Toml(r#""8 MiB""#),
        allowed: "between 1 MiB and 1 GiB",
        doc: "The size at which open.seg is sealed into a segment of the store.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (MIB..=GIB).contains(bytes))
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(*bytes),
};

pub const SEAL_AFTER: Key<Duration> = Key {
    about: About {
        name: "store.seal_after",
        variable: Some("BILBO_STORE_SEAL_AFTER"),
        default: Default::Toml(r#""60m""#),
        allowed: "between 1m and 24h",
        doc: "The age of open.seg's oldest frame at which a run seals it, whatever its size.",
    },
    parse: |value| {
        duration::read(value)
            .filter(|after| (MINUTE..=DAY).contains(after))
            .ok_or_else(|| value.refuse())
    },
    show: |after| duration::show(*after),
};

pub const COMPRESSION: Key<Compression> = Key {
    about: About {
        name: "store.compression",
        variable: Some("BILBO_STORE_COMPRESSION"),
        default: Default::Toml("19"),
        allowed: "a zstd level from 1 to 22",
        doc: "The zstd level of every frame the store writes: higher packs smaller and writes slower.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|level| Compression::try_from(level).ok())
            .ok_or_else(|| value.refuse())
    },
    show: |level| level.level().to_string(),
};

pub const TEAR_BUDGET: Key<TearBudget> = Key {
    about: About {
        name: "store.tear_budget",
        variable: Some("BILBO_STORE_TEAR_BUDGET"),
        default: Default::Toml("4"),
        allowed: "a whole number from 1 to 64",
        doc: "How many times open.seg's size the search after a torn frame may hash before it refuses the file.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|times| usize::try_from(times).ok())
            .and_then(|times| TearBudget::try_from(times).ok())
            .ok_or_else(|| value.refuse())
    },
    show: |times| times.times().to_string(),
};

const LONGEST_WAIT: Duration = Duration::from_secs(10 * 60);

pub const WAIT: Key<Duration> = Key {
    about: About {
        name: "store.wait",
        variable: Some("BILBO_STORE_WAIT"),
        default: Default::Toml(r#""10s""#),
        allowed: "between 0s and 10m",
        doc: "How long a reader of the store, such as bilbo read, waits for a bilbo that is writing it before it gives up.",
    },
    parse: |value| {
        duration::read(value)
            .filter(|wait| *wait <= LONGEST_WAIT)
            .ok_or_else(|| value.refuse())
    },
    show: |wait| duration::show(*wait),
};

pub const POLL: Key<Duration> = Key {
    about: About {
        name: "store.poll",
        variable: Some("BILBO_STORE_POLL"),
        default: Default::Toml(r#""20ms""#),
        allowed: "between 1ms and 1s",
        doc: "How often a reader that waits for a writer tries the lock again: flock has no timed wait.",
    },
    parse: |value| {
        duration::read(value)
            .filter(|poll| (MILLISECOND..=SECOND).contains(poll))
            .ok_or_else(|| value.refuse())
    },
    show: |poll| duration::show(*poll),
};

pub const ABOUT: [&About; 6] = [
    &SEAL_SIZE.about,
    &SEAL_AFTER.about,
    &COMPRESSION.about,
    &TEAR_BUDGET.about,
    &WAIT.about,
    &POLL.about,
];

/// The store's keys as the settings resolved them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub seal_at: Resolved<u64>,
    pub seal_after: Resolved<Duration>,
    pub compression: Resolved<Compression>,
    pub tear_budget: Resolved<TearBudget>,
    pub wait: Resolved<Duration>,
    pub poll: Resolved<Duration>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        seal_at: SEAL_SIZE.resolve(source)?,
        seal_after: SEAL_AFTER.resolve(source)?,
        compression: COMPRESSION.resolve(source)?,
        tear_budget: TEAR_BUDGET.resolve(source)?,
        wait: WAIT.resolve(source)?,
        poll: POLL.resolve(source)?,
    })
}

/// Every key at its default: no variable, no file.
pub fn defaults() -> Section {
    let file = File::default();
    section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("each default is a literal its key's unit test parses")
}

impl std::default::Default for Options {
    fn default() -> Self {
        defaults().options()
    }
}

impl Section {
    pub fn options(&self) -> Options {
        Options {
            seal_at: self.seal_at.value,
            seal_after: self.seal_after.value,
            compression: self.compression.value,
            tear_budget: self.tear_budget.value,
            wait: self.wait.value,
            poll: self.poll.value,
            hook: None,
            held: Held::Wait,
            rebuild: Rebuild::Allowed,
        }
    }

    pub fn shown(&self) -> [Shown; 6] {
        [
            SEAL_SIZE.shown(&self.seal_at),
            SEAL_AFTER.shown(&self.seal_after),
            COMPRESSION.shown(&self.compression),
            TEAR_BUDGET.shown(&self.tear_budget),
            WAIT.shown(&self.wait),
            POLL.shown(&self.poll),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use bilbo_settings::file::File;
    use bilbo_settings::key::{Origin, Resolved, Source};
    use bilbo_settings::size::MIB;

    use super::{ABOUT, Section, defaults, section};
    use crate::limits::{Compression, TearBudget};

    fn resolve(env: &[(&str, &str)], text: &str) -> Result<super::Section, String> {
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
    fn the_defaults_are_8_mib_60_minutes_level_19_four_times_the_file_and_a_10_second_wait() {
        let options = defaults().options();
        assert_eq!(
            (
                options.seal_at,
                options.seal_after,
                options.compression.level(),
                options.tear_budget.times()
            ),
            (8 * MIB, Duration::from_secs(3600), 19, 4)
        );
        let shown: Vec<String> = defaults()
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "store.seal_at = \"8 MiB\" (default)",
                "store.seal_after = \"60m\" (default)",
                "store.compression = 19 (default)",
                "store.tear_budget = 4 (default)",
                "store.wait = \"10s\" (default)",
                "store.poll = \"20ms\" (default)"
            ]
        );
    }

    #[test]
    fn each_key_takes_its_variable_over_the_file() {
        let text = "[store]\nseal_at = \"16 MiB\"\ncompression = 3\ntear_budget = 8\nseal_after = \"2h\"\n";
        let from_file = resolve(&[], text).expect("valid");
        assert_eq!(from_file.options().seal_at, 16 * MIB);
        assert_eq!(
            from_file.options().seal_after,
            Duration::from_secs(2 * 3600)
        );
        assert_eq!(from_file.compression.value.level(), 3);
        assert_eq!(
            from_file.tear_budget.origin,
            Origin::File {
                path: "/c/config.toml".into(),
                line: 4
            }
        );
        let env = [
            ("BILBO_STORE_SEAL_AT", "1GiB"),
            ("BILBO_STORE_COMPRESSION", "22"),
            ("BILBO_STORE_TEAR_BUDGET", "64"),
            ("BILBO_STORE_SEAL_AFTER", "90s"),
        ];
        let options = resolve(&env, text).expect("valid").options();
        assert_eq!(options.seal_after, Duration::from_secs(90));
        assert_eq!(
            (
                options.seal_at,
                options.compression.level(),
                options.tear_budget.times()
            ),
            (1024 * MIB, 22, 64)
        );
    }

    #[test]
    fn each_key_refuses_a_value_out_of_its_range() {
        for (text, message) in [
            (
                "store.seal_at = \"1023 KiB\"",
                "/c/config.toml:1: store.seal_at must be between 1 MiB and 1 GiB, got \"1023 KiB\"",
            ),
            (
                "store.seal_at = \"1025 MiB\"",
                "/c/config.toml:1: store.seal_at must be between 1 MiB and 1 GiB, got \"1025 MiB\"",
            ),
            (
                "store.seal_after = \"59s\"",
                "/c/config.toml:1: store.seal_after must be between 1m and 24h, got \"59s\"",
            ),
            (
                "store.seal_after = \"25h\"",
                "/c/config.toml:1: store.seal_after must be between 1m and 24h, got \"25h\"",
            ),
            (
                "store.seal_after = 60",
                "/c/config.toml:1: store.seal_after must be between 1m and 24h, got 60",
            ),
            (
                "store.compression = 0",
                "/c/config.toml:1: store.compression must be a zstd level from 1 to 22, got 0",
            ),
            (
                "store.compression = 23",
                "/c/config.toml:1: store.compression must be a zstd level from 1 to 22, got 23",
            ),
            (
                "store.tear_budget = 0",
                "/c/config.toml:1: store.tear_budget must be a whole number from 1 to 64, got 0",
            ),
            (
                "store.tear_budget = 65",
                "/c/config.toml:1: store.tear_budget must be a whole number from 1 to 64, got 65",
            ),
        ] {
            assert_eq!(resolve(&[], text).err().as_deref(), Some(message), "{text}");
        }
        assert_eq!(
            resolve(&[("BILBO_STORE_COMPRESSION", "-3")], "")
                .err()
                .as_deref(),
            Some("BILBO_STORE_COMPRESSION must be a zstd level from 1 to 22, got \"-3\"")
        );
    }

    #[test]
    fn a_readers_wait_is_seconds_or_minutes_up_to_ten_minutes() {
        for (text, seconds) in [("0s", 0), ("1s", 1), ("90s", 90), ("10m", 600)] {
            let options = resolve(&[("BILBO_STORE_WAIT", text)], "")
                .expect("a wait")
                .options();
            assert_eq!(options.wait, Duration::from_secs(seconds), "{text}");
        }
        let from_file = resolve(&[], "[store]\nwait = \"2m\"\n").expect("a wait");
        assert_eq!(from_file.options().wait, Duration::from_secs(120));
        assert_eq!(defaults().options().wait, Duration::from_secs(10));
        for text in ["11m", "601s", "1h", "10", "-1s", "soon"] {
            assert_eq!(
                resolve(&[("BILBO_STORE_WAIT", text)], "").err(),
                Some(format!(
                    "BILBO_STORE_WAIT must be between 0s and 10m, got \"{text}\""
                )),
                "{text}"
            );
        }
    }

    #[test]
    fn the_poll_is_milliseconds_up_to_a_second() {
        assert_eq!(defaults().options().poll, Duration::from_millis(20));
        for (text, millis) in [("1ms", 1), ("250ms", 250), ("1s", 1000), ("1000ms", 1000)] {
            let options = resolve(&[("BILBO_STORE_POLL", text)], "")
                .expect("a poll")
                .options();
            assert_eq!(options.poll, Duration::from_millis(millis), "{text}");
        }
        let from_file = resolve(&[], "[store]\npoll = \"50ms\"\n").expect("a poll");
        assert_eq!(from_file.options().poll, Duration::from_millis(50));
        for text in ["0ms", "0s", "1001ms", "2s", "1m", "20", "-1ms", "soon"] {
            assert_eq!(
                resolve(&[("BILBO_STORE_POLL", text)], "").err(),
                Some(format!(
                    "BILBO_STORE_POLL must be between 1ms and 1s, got \"{text}\""
                )),
                "{text}"
            );
        }
        assert_eq!(
            resolve(&[], "store.poll = \"2s\"").err().as_deref(),
            Some("/c/config.toml:1: store.poll must be between 1ms and 1s, got \"2s\"")
        );
    }

    #[test]
    fn a_section_maps_every_key_onto_options() {
        let section = Section {
            seal_at: Resolved {
                value: 3 * MIB,
                origin: Origin::Default,
            },
            seal_after: Resolved {
                value: Duration::from_secs(120),
                origin: Origin::Default,
            },
            compression: Resolved {
                value: Compression::try_from(7).expect("in range"),
                origin: Origin::Default,
            },
            tear_budget: Resolved {
                value: TearBudget::try_from(9).expect("in range"),
                origin: Origin::Default,
            },
            wait: Resolved {
                value: Duration::from_secs(3),
                origin: Origin::Default,
            },
            poll: Resolved {
                value: Duration::from_millis(5),
                origin: Origin::Default,
            },
        };
        let options = section.options();
        assert_eq!(
            (
                options.seal_at,
                options.compression.level(),
                options.tear_budget.times()
            ),
            (3 * MIB, 7, 9)
        );
        assert_eq!(options.seal_after, Duration::from_secs(120));
        assert_eq!(options.wait, Duration::from_secs(3));
        assert_eq!(options.poll, Duration::from_millis(5));
        assert_eq!(
            options.held,
            crate::segments::Held::Wait,
            "only a caller asks a holder to run again"
        );
        assert!(options.hook.is_none());
    }

    #[test]
    fn the_store_declares_six_keys_each_with_its_own_variable() {
        let names: Vec<(&str, Option<&str>)> = ABOUT
            .iter()
            .map(|about| (about.name, about.variable))
            .collect();
        assert_eq!(
            names,
            [
                ("store.seal_at", Some("BILBO_STORE_SEAL_AT")),
                ("store.seal_after", Some("BILBO_STORE_SEAL_AFTER")),
                ("store.compression", Some("BILBO_STORE_COMPRESSION")),
                ("store.tear_budget", Some("BILBO_STORE_TEAR_BUDGET")),
                ("store.wait", Some("BILBO_STORE_WAIT")),
                ("store.poll", Some("BILBO_STORE_POLL")),
            ]
        );
    }
}
