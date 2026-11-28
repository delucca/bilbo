use std::time::Duration;

use bilbo_settings::duration;
use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, Value, quote};

fn folders(value: &Value<'_>) -> Result<String, Error> {
    let text = value.text().ok_or_else(|| value.refuse())?;
    let plain = |folder: &str| {
        folder.starts_with('/') && folder.split('/').all(|part| part != "." && part != "..")
    };
    if !text.is_empty() && !text.split(':').all(plain) {
        return Err(value.refuse());
    }
    Ok(text.to_owned())
}

fn between(value: &Value<'_>, least: Duration, most: Duration) -> Result<Duration, Error> {
    duration::read(value)
        .filter(|wait| (least..=most).contains(wait))
        .ok_or_else(|| value.refuse())
}

fn minutes(value: &Value<'_>) -> Result<Duration, Error> {
    between(value, duration::MINUTE, duration::DAY).and_then(|wait| {
        wait.as_nanos()
            .is_multiple_of(duration::MINUTE.as_nanos())
            .then_some(wait)
            .ok_or_else(|| value.refuse())
    })
}

pub fn waited(wait: Duration) -> String {
    duration::show(wait).trim_matches('"').to_owned()
}

pub const TIMER_PATH: Key<String> = Key {
    about: About {
        name: "setup.timer.path",
        variable: Some("BILBO_SETUP_TIMER_PATH"),
        default: Default::Toml(r#""/usr/bin:/bin:/usr/sbin:/sbin""#),
        allowed: "absolute folders joined by colons, or the empty text for none",
        doc: "The folders a timer's unit puts on PATH after the folders bilbo setup found the tools' programs in.",
    },
    parse: folders,
    show: |text| quote(text),
};

pub const REFRESH_EVERY: Key<Duration> = Key {
    about: About {
        name: "setup.refresh.every",
        variable: Some("BILBO_SETUP_REFRESH_EVERY"),
        default: Default::Toml(r#""10m""#),
        allowed: "a whole number of minutes from 1m to 24h",
        doc: "How often the timer bilbo setup installs runs bilbo refresh.",
    },
    parse: minutes,
    show: |every| duration::show(*every),
};

pub const DERIVE_EVERY: Key<Duration> = Key {
    about: About {
        name: "setup.derive.every",
        variable: Some("BILBO_SETUP_DERIVE_EVERY"),
        default: Default::Toml(r#""30m""#),
        allowed: "a whole number of minutes from 1m to 24h",
        doc: "How often the timer bilbo setup installs runs bilbo derive.",
    },
    parse: minutes,
    show: |every| duration::show(*every),
};

pub const WAIT: Key<Duration> = Key {
    about: About {
        name: "setup.wait",
        variable: Some("BILBO_SETUP_WAIT"),
        default: Default::Toml(r#""3m""#),
        allowed: "between 1s and 10m",
        doc: "How long bilbo setup waits for one command of a tool or of the scheduler to end and close its output, and for Codex's app-server to take one request and answer it.",
    },
    parse: |value| between(value, duration::SECOND, 10 * duration::MINUTE),
    show: |wait| duration::show(*wait),
};

pub const POLL: Key<Duration> = Key {
    about: About {
        name: "setup.poll",
        variable: Some("BILBO_SETUP_POLL"),
        default: Default::Toml(r#""5ms""#),
        allowed: "between 1ms and 1s",
        doc: "How often bilbo setup looks whether the program it started has ended: a child has no timed wait.",
    },
    parse: |value| between(value, duration::MILLISECOND, duration::SECOND),
    show: |poll| duration::show(*poll),
};

pub const ABOUT: [&About; 5] = [
    &TIMER_PATH.about,
    &REFRESH_EVERY.about,
    &DERIVE_EVERY.about,
    &WAIT.about,
    &POLL.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub timer_path: Resolved<String>,
    pub refresh_every: Resolved<Duration>,
    pub derive_every: Resolved<Duration>,
    pub wait: Resolved<Duration>,
    pub poll: Resolved<Duration>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        timer_path: TIMER_PATH.resolve(source)?,
        refresh_every: REFRESH_EVERY.resolve(source)?,
        derive_every: DERIVE_EVERY.resolve(source)?,
        wait: WAIT.resolve(source)?,
        poll: POLL.resolve(source)?,
    })
}

impl Section {
    pub fn shown(&self) -> [Shown; 5] {
        [
            TIMER_PATH.shown(&self.timer_path),
            REFRESH_EVERY.shown(&self.refresh_every),
            DERIVE_EVERY.shown(&self.derive_every),
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
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, section};

    fn resolve(variables: &[(&str, &str)], text: &str) -> Result<Section, String> {
        let var = |name: &str| {
            variables
                .iter()
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
    fn each_setup_key_of_the_library_takes_its_default_and_refuses_what_is_outside() {
        let defaults = resolve(&[], "").expect("defaults");
        assert_eq!(defaults.timer_path.value, "/usr/bin:/bin:/usr/sbin:/sbin");
        assert_eq!(defaults.refresh_every.value, Duration::from_secs(600));
        assert_eq!(defaults.derive_every.value, Duration::from_secs(1800));
        assert_eq!(defaults.wait.value, Duration::from_secs(180));
        assert_eq!(defaults.poll.value, Duration::from_millis(5));
        let shown: Vec<String> = defaults
            .shown()
            .iter()
            .map(|shown| format!("{} = {}", shown.name, shown.value))
            .collect();
        assert_eq!(
            shown,
            [
                "setup.timer.path = \"/usr/bin:/bin:/usr/sbin:/sbin\"",
                "setup.refresh.every = \"10m\"",
                "setup.derive.every = \"30m\"",
                "setup.wait = \"3m\"",
                "setup.poll = \"5ms\"",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_SETUP_TIMER_PATH"),
                Some("BILBO_SETUP_REFRESH_EVERY"),
                Some("BILBO_SETUP_DERIVE_EVERY"),
                Some("BILBO_SETUP_WAIT"),
                Some("BILBO_SETUP_POLL"),
            ]
        );
        for (key, taken) in [
            ("setup.refresh.every", ["\"1m\"", "\"24h\"", "\"90m\""]),
            ("setup.derive.every", ["\"1m\"", "\"24h\"", "\"61m\""]),
        ] {
            for value in taken {
                let text = format!("{key} = {value}\n");
                assert!(resolve(&[], &text).is_ok(), "{text}");
            }
            for value in ["\"90s\"", "\"0m\"", "\"25h\"", "\"soon\"", "\"60001ms\""] {
                let text = format!("{key} = {value}\n");
                assert!(resolve(&[], &text).is_err(), "{text}");
            }
        }
        for (key, taken, refused) in [
            ("setup.wait", ["\"1s\"", "\"10m\""], ["\"0s\"", "\"11m\""]),
            ("setup.poll", ["\"1ms\"", "\"1s\""], ["\"0ms\"", "\"2s\""]),
        ] {
            for value in taken {
                let text = format!("{key} = {value}\n");
                assert!(resolve(&[], &text).is_ok(), "{text}");
            }
            for value in refused {
                let text = format!("{key} = {value}\n");
                assert!(resolve(&[], &text).is_err(), "{text}");
            }
        }
        for value in ["\"\"", "\"/usr/bin\"", "\"/a:/b\""] {
            let text = format!("setup.timer.path = {value}\n");
            assert!(resolve(&[], &text).is_ok(), "{text}");
        }
        for value in [
            "\"bin\"",
            "\"/a:b\"",
            "\"/a::/b\"",
            "\"~/bin\"",
            "\"/opt/../bin:/usr/bin\"",
            "\"/a/./b\"",
        ] {
            let text = format!("setup.timer.path = {value}\n");
            assert!(resolve(&[], &text).is_err(), "{text}");
        }
        assert_eq!(
            resolve(&[("BILBO_SETUP_WAIT", "2m")], "")
                .expect("a variable")
                .wait
                .value,
            Duration::from_secs(120)
        );
        assert!(resolve(&[("BILBO_SETUP_WAIT", "0s")], "").is_err());
    }
}
