use std::path::PathBuf;

use bilbo_settings::error::Error;
use bilbo_settings::folder;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};

use crate::keys::count;

pub const ENABLED: Key<bool> = Key {
    about: About {
        name: "derive.summary.enabled",
        variable: Some("BILBO_DERIVE_SUMMARY_ENABLED"),
        default: Default::Toml("true"),
        allowed: "true or false",
        doc: "Whether bilbo derive writes a summary of each entry it may send, with one more call an entry. While it is false no summary is asked for.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const INSTRUCTION: Key<Option<PathBuf>> = Key {
    about: About {
        name: "derive.summary.instruction",
        variable: Some("BILBO_DERIVE_SUMMARY_INSTRUCTION"),
        default: Default::Toml(r#""""#),
        allowed: "an absolute path or one starting with ~/, or the empty text for the one bilbo ships",
        doc: "The file whose text bilbo derive sends as the instruction of a call for a summary, in place of the one bilbo ships. Empty: the one bilbo ships. The text must ask for the answer bilbo reads: one paragraph of plain text.",
    },
    parse: |value| match value.os_text() {
        Some(text) if text.is_empty() => Ok(None),
        _ => folder::path(value).map(Some),
    },
    show: |path| path.as_deref().map_or_else(|| quote(""), folder::show),
};

pub const MOST: Key<usize> = Key {
    about: About {
        name: "derive.summary.most",
        variable: Some("BILBO_DERIVE_SUMMARY_MOST"),
        default: Default::Toml("200"),
        allowed: "a whole number from 40 to 1000",
        doc: "The most characters bilbo derive asks a summary to have. A longer answer is kept whole, and bilbo digest and bilbo read print it whole.",
    },
    parse: |value| count(value, 40..=1000),
    show: ToString::to_string,
};

pub const WINDOW: Key<usize> = Key {
    about: About {
        name: "derive.summary.window",
        variable: Some("BILBO_DERIVE_SUMMARY_WINDOW"),
        default: Default::Toml("20000"),
        allowed: "a whole number from 1000 to 500000",
        doc: "The most tokens of an entry a call for a summary is sent, as bilbo counts them: its notes, or its own text where bilbo derive writes no notes for its kind. A longer one keeps its start and its end, shared by derive.passage_head.",
    },
    parse: |value| count(value, 1000..=500_000),
    show: ToString::to_string,
};

pub const LEAST: Key<usize> = Key {
    about: About {
        name: "derive.summary.least",
        variable: Some("BILBO_DERIVE_SUMMARY_LEAST"),
        default: Default::Toml("100"),
        allowed: "a whole number from 0 to 100000",
        doc: "The fewest tokens of notes or text an entry must have for bilbo derive to ask a first summary of it: a shorter entry is read as it is. 0 asks one of every entry that has any.",
    },
    parse: |value| count(value, 0..=100_000),
    show: ToString::to_string,
};

pub const ABOUT: [&About; 5] = [
    &ENABLED.about,
    &INSTRUCTION.about,
    &MOST.about,
    &WINDOW.about,
    &LEAST.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub enabled: Resolved<bool>,
    pub instruction: Resolved<Option<PathBuf>>,
    pub most: Resolved<usize>,
    pub window: Resolved<usize>,
    pub least: Resolved<usize>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        enabled: ENABLED.resolve(source)?,
        instruction: INSTRUCTION.resolve(source)?,
        most: MOST.resolve(source)?,
        window: WINDOW.resolve(source)?,
        least: LEAST.resolve(source)?,
    })
}

pub fn defaults() -> Section {
    let file = bilbo_settings::file::File::default();
    section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("each default is a literal its key's unit test parses")
}

impl Section {
    pub fn shown(&self) -> [Shown; 5] {
        [
            ENABLED.shown(&self.enabled),
            INSTRUCTION.shown(&self.instruction),
            MOST.shown(&self.most),
            WINDOW.shown(&self.window),
            LEAST.shown(&self.least),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, section};

    fn env(variables: &[(&str, &str)], text: &str) -> Result<Section, String> {
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

    fn resolve(text: &str) -> Result<Section, String> {
        env(&[], text)
    }

    fn given(text: &str) -> Section {
        resolve(text).expect("resolves")
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = given("");
        assert!(section.enabled.value);
        assert_eq!(section.instruction.value, None);
        assert_eq!(section.most.value, 200);
        assert_eq!(section.window.value, 20000);
        assert_eq!(section.least.value, 100);
        let shown: Vec<String> = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "derive.summary.enabled = true (default)",
                "derive.summary.instruction = \"\" (default)",
                "derive.summary.most = 200 (default)",
                "derive.summary.window = 20000 (default)",
                "derive.summary.least = 100 (default)",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_DERIVE_SUMMARY_ENABLED"),
                Some("BILBO_DERIVE_SUMMARY_INSTRUCTION"),
                Some("BILBO_DERIVE_SUMMARY_MOST"),
                Some("BILBO_DERIVE_SUMMARY_WINDOW"),
                Some("BILBO_DERIVE_SUMMARY_LEAST"),
            ]
        );
    }

    #[test]
    fn the_switch_is_true_or_false() {
        assert!(!given("derive.summary.enabled = false").enabled.value);
        assert!(given("derive.summary.enabled = true").enabled.value);
        assert_eq!(
            resolve("derive.summary.enabled = \"yes\"").err().as_deref(),
            Some("/c/config.toml:1: derive.summary.enabled must be true or false, got \"yes\"")
        );
        assert_eq!(
            resolve("derive.summary.enabled = 1").err().as_deref(),
            Some("/c/config.toml:1: derive.summary.enabled must be true or false, got 1")
        );
        let section = env(
            &[("BILBO_DERIVE_SUMMARY_ENABLED", "false")],
            "derive.summary.enabled = true",
        )
        .expect("resolves");
        assert!(!section.enabled.value);
        assert_eq!(
            env(&[("BILBO_DERIVE_SUMMARY_ENABLED", "maybe")], "")
                .err()
                .as_deref(),
            Some("BILBO_DERIVE_SUMMARY_ENABLED must be true or false, got \"maybe\"")
        );
    }

    #[test]
    fn the_three_counts_stand_in_their_ranges() {
        type Read = fn(&Section) -> usize;
        type Row<'a> = (&'a str, &'a str, &'a [i64], &'a [i64], Read);
        let table: [Row<'_>; 3] = [
            (
                "most",
                "a whole number from 40 to 1000",
                &[40, 199, 200, 201, 1000],
                &[39, 1001, -1],
                |section| section.most.value,
            ),
            (
                "window",
                "a whole number from 1000 to 500000",
                &[1000, 19999, 20000, 20001, 500_000],
                &[999, 500_001],
                |section| section.window.value,
            ),
            (
                "least",
                "a whole number from 0 to 100000",
                &[0, 99, 100, 101, 100_000],
                &[-1, 100_001],
                |section| section.least.value,
            ),
        ];
        for (name, allowed, taken, refused, read) in table {
            for number in taken {
                let section = given(&format!("derive.summary.{name} = {number}"));
                assert_eq!(
                    i64::try_from(read(&section)).expect("fits"),
                    *number,
                    "{name}"
                );
            }
            for number in refused {
                assert_eq!(
                    resolve(&format!("derive.summary.{name} = {number}"))
                        .err()
                        .as_deref(),
                    Some(
                        format!(
                            "/c/config.toml:1: derive.summary.{name} must be {allowed}, got {number}"
                        )
                        .as_str()
                    ),
                    "{name} {number}"
                );
            }
        }
    }

    #[test]
    fn the_summarys_instruction_is_a_path_or_the_empty_text() {
        let path = |variables: &[(&str, &str)], text: &str| {
            env(variables, text).map(|section| section.instruction.value)
        };
        let home: &[(&str, &str)] = &[("HOME", "/home/me")];
        assert_eq!(path(home, ""), Ok(None));
        assert_eq!(path(home, "derive.summary.instruction = \"\""), Ok(None));
        assert_eq!(
            path(home, "derive.summary.instruction = \"/c/summary.txt\""),
            Ok(Some(PathBuf::from("/c/summary.txt")))
        );
        assert_eq!(
            path(home, "derive.summary.instruction = \"~/notes/summary.txt\""),
            Ok(Some(PathBuf::from("/home/me/notes/summary.txt")))
        );
        let over: &[(&str, &str)] = &[
            ("HOME", "/home/me"),
            ("BILBO_DERIVE_SUMMARY_INSTRUCTION", "/v/summary.txt"),
        ];
        assert_eq!(
            path(over, "derive.summary.instruction = \"/c/summary.txt\""),
            Ok(Some(PathBuf::from("/v/summary.txt")))
        );
        for text in [
            "derive.summary.instruction = \"summary.txt\"",
            "derive.summary.instruction = \"/c/../summary.txt\"",
            "derive.summary.instruction = 3",
        ] {
            let message = path(home, text).expect_err("refused");
            assert!(
                message.starts_with("/c/config.toml:1: derive.summary.instruction"),
                "{message}"
            );
        }
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let section = given(
            "derive.summary.enabled = false\nderive.summary.instruction = \"/c/summary.txt\"\nderive.summary.most = 77\nderive.summary.window = 2000\nderive.summary.least = 0",
        );
        let text: String = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {}\n", shown.name, shown.value))
            .collect();
        let again = given(&text);
        assert!(!again.enabled.value);
        assert_eq!(
            again.instruction.value,
            Some(PathBuf::from("/c/summary.txt"))
        );
        assert_eq!(again.most.value, 77);
        assert_eq!(again.window.value, 2000);
        assert_eq!(again.least.value, 0);
    }
}
