use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use bilbo_llm::model::{MODEL_ALLOWED, MODEL_REFUSED, is_model_name};
use bilbo_settings::error::Error;
use bilbo_settings::folder;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, Value, quote, show_list};
use bilbo_store::labels::Tag;

fn count(value: &Value<'_>, range: RangeInclusive<usize>) -> Result<usize, Error> {
    value
        .integer()
        .and_then(|count| usize::try_from(count).ok())
        .filter(|count| range.contains(count))
        .ok_or_else(|| value.refuse())
}

pub const ENABLED: Key<bool> = Key {
    about: About {
        name: "derive.tags.enabled",
        variable: Some("BILBO_DERIVE_TAGS_ENABLED"),
        default: Default::Toml("false"),
        allowed: "true or false",
        doc: "Whether bilbo derive files an entry it wrote notes for under tags, with one more call. While it is false no tag is asked for.",
    },
    parse: |value| value.boolean().ok_or_else(|| value.refuse()),
    show: ToString::to_string,
};

pub const PICK: Key<BTreeSet<Tag>> = Key {
    about: About {
        name: "derive.tags.pick",
        variable: None,
        default: Default::Toml("[]"),
        allowed: "a list of tags, each parts of a to z and digits joined by single hyphens, at most 64 bytes",
        doc: "The tags a model may pick for an entry; it can declare no other. Empty: no call is made. A name an entry was never asked about makes it due again.",
    },
    parse: |value| {
        let items = value.items().ok_or_else(|| value.refuse())?;
        items
            .iter()
            .map(|item| item.text().and_then(Tag::new).ok_or_else(|| item.refuse()))
            .collect()
    },
    show: |tags| {
        let names: Vec<String> = tags.iter().map(|tag| tag.as_str().to_owned()).collect();
        show_list(&names)
    },
};

pub const INSTRUCTION: Key<Option<PathBuf>> = Key {
    about: About {
        name: "derive.tags.instruction",
        variable: Some("BILBO_DERIVE_TAGS_INSTRUCTION"),
        default: Default::Toml(r#""""#),
        allowed: "an absolute path or one starting with ~/, or the empty text for the one bilbo ships",
        doc: "The file whose text bilbo derive sends as the instruction of a call for tags, in place of the one bilbo ships. Empty: the one bilbo ships. The text must ask for the answer bilbo reads: a tag a line, or the one line (none).",
    },
    parse: |value| match value.os_text() {
        Some(text) if text.is_empty() => Ok(None),
        _ => folder::path(value).map(Some),
    },
    show: |path| path.as_deref().map_or_else(|| quote(""), folder::show),
};

pub const MODEL: Key<String> = Key {
    about: About {
        name: "derive.tags.model",
        variable: Some("BILBO_DERIVE_TAGS_MODEL"),
        default: Default::Toml(r#""""#),
        allowed: MODEL_ALLOWED,
        doc: "The model bilbo derive asks for tags, of the provider and the way in force. Empty: the model that writes the notes.",
    },
    parse: |value| {
        value
            .text()
            .filter(|id| is_model_name(id))
            .map(str::to_owned)
            .ok_or_else(|| value.refused(MODEL_REFUSED))
    },
    show: |id| quote(id),
};

pub const MOST: Key<usize> = Key {
    about: About {
        name: "derive.tags.most",
        variable: Some("BILBO_DERIVE_TAGS_MOST"),
        default: Default::Toml("3"),
        allowed: "a whole number from 1 to 64",
        doc: "The most tags one answer may give an entry: the ones after them are dropped.",
    },
    parse: |value| count(value, 1..=64),
    show: ToString::to_string,
};

pub const NOTES: Key<usize> = Key {
    about: About {
        name: "derive.tags.notes",
        variable: Some("BILBO_DERIVE_TAGS_NOTES"),
        default: Default::Toml("6000"),
        allowed: "a whole number from 1000 to 500000",
        doc: "The most tokens of an entry's notes a call for tags is sent, the newest that fit; the newest one is sent whatever its size.",
    },
    parse: |value| count(value, 1000..=500_000),
    show: ToString::to_string,
};

pub const ABOUT: [&About; 6] = [
    &ENABLED.about,
    &PICK.about,
    &INSTRUCTION.about,
    &MODEL.about,
    &MOST.about,
    &NOTES.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub enabled: Resolved<bool>,
    pub pick: Resolved<BTreeSet<Tag>>,
    pub instruction: Resolved<Option<PathBuf>>,
    pub model: Resolved<String>,
    pub most: Resolved<usize>,
    pub notes: Resolved<usize>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        enabled: ENABLED.resolve(source)?,
        pick: PICK.resolve(source)?,
        instruction: INSTRUCTION.resolve(source)?,
        model: MODEL.resolve(source)?,
        most: MOST.resolve(source)?,
        notes: NOTES.resolve(source)?,
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
    pub fn shown(&self) -> [Shown; 6] {
        [
            ENABLED.shown(&self.enabled),
            PICK.shown(&self.pick),
            INSTRUCTION.shown(&self.instruction),
            MODEL.shown(&self.model),
            MOST.shown(&self.most),
            NOTES.shown(&self.notes),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_llm::model::MODEL_REFUSED;
    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;
    use bilbo_store::labels::Tag;

    use super::{ABOUT, PICK, Section, section};

    fn resolve(text: &str) -> Result<Section, String> {
        let var = |_: &str| None::<OsString>;
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        section(&Source {
            var: &var,
            file: &file,
        })
        .map_err(|error| error.to_string())
    }

    fn given(text: &str) -> Section {
        resolve(text).expect("resolves")
    }

    fn refused(text: &str) -> String {
        resolve(text).expect_err("refused")
    }

    fn tags(names: &[&str]) -> BTreeSet<Tag> {
        names
            .iter()
            .map(|name| Tag::new(name).expect("a tag"))
            .collect()
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = given("");
        assert!(!section.enabled.value);
        assert!(section.pick.value.is_empty());
        assert_eq!(section.instruction.value, None);
        assert_eq!(section.model.value, "");
        assert_eq!(section.most.value, 3);
        assert_eq!(section.notes.value, 6000);
        let shown: Vec<String> = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {} ({})", shown.name, shown.value, shown.origin))
            .collect();
        assert_eq!(
            shown,
            [
                "derive.tags.enabled = false (default)",
                "derive.tags.pick = [] (default)",
                "derive.tags.instruction = \"\" (default)",
                "derive.tags.model = \"\" (default)",
                "derive.tags.most = 3 (default)",
                "derive.tags.notes = 6000 (default)",
            ]
        );
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_DERIVE_TAGS_ENABLED"),
                None,
                Some("BILBO_DERIVE_TAGS_INSTRUCTION"),
                Some("BILBO_DERIVE_TAGS_MODEL"),
                Some("BILBO_DERIVE_TAGS_MOST"),
                Some("BILBO_DERIVE_TAGS_NOTES"),
            ]
        );
    }

    #[test]
    fn the_pick_list_is_tags_in_name_order_with_none_twice() {
        let section = given("derive.tags.pick = [\"b\", \"a\", \"b\"]");
        assert_eq!(section.pick.value, tags(&["a", "b"]));
        assert_eq!(section.shown()[1].value, "[\"a\", \"b\"]");
        let long = format!("derive.tags.pick = [\"{}\"]", "a".repeat(65));
        for (text, bad) in [
            ("derive.tags.pick = [\"Rust\"]".to_owned(), "\"Rust\""),
            ("derive.tags.pick = [\"a b\"]".to_owned(), "\"a b\""),
            (long, &format!("\"{}\"", "a".repeat(65))),
        ] {
            let message = refused(&text);
            assert_eq!(
                message,
                format!(
                    "/c/config.toml:1: derive.tags.pick must be {}, got {bad}",
                    PICK.about.allowed
                )
            );
        }
    }

    #[test]
    fn the_tags_model_is_a_models_id_or_the_empty_text() {
        for id in ["", "claude-haiku-5-5"] {
            assert_eq!(
                given(&format!("derive.tags.model = \"{id}\"")).model.value,
                id
            );
        }
        for text in [
            "derive.tags.model = \"a b\"".to_owned(),
            format!("derive.tags.model = \"{}\"", "m".repeat(201)),
        ] {
            let message = refused(&text);
            assert!(message.ends_with(MODEL_REFUSED), "{message}");
        }
    }

    #[test]
    fn the_two_counts_stand_in_their_ranges() {
        for (name, accepted, refused_values) in [
            ("most", [1, 64], [0, 65]),
            ("notes", [1000, 500_000], [999, 500_001]),
        ] {
            for number in accepted {
                assert!(
                    resolve(&format!("derive.tags.{name} = {number}")).is_ok(),
                    "{name} {number}"
                );
            }
            for number in refused_values {
                assert!(
                    resolve(&format!("derive.tags.{name} = {number}")).is_err(),
                    "{name} {number}"
                );
            }
        }
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back() {
        let section = given(
            "derive.tags.enabled = true\nderive.tags.pick = [\"b\", \"a\"]\nderive.tags.instruction = \"/c/tags.txt\"\nderive.tags.model = \"m\"\nderive.tags.most = 7\nderive.tags.notes = 2000",
        );
        let text: String = section
            .shown()
            .iter()
            .map(|shown| format!("{} = {}\n", shown.name, shown.value))
            .collect();
        let again = given(&text);
        assert!(again.enabled.value);
        assert_eq!(again.pick.value, tags(&["a", "b"]));
        assert_eq!(
            again.instruction.value,
            Some(std::path::PathBuf::from("/c/tags.txt"))
        );
        assert_eq!(again.model.value, "m");
        assert_eq!(again.most.value, 7);
        assert_eq!(again.notes.value, 2000);
    }

    #[test]
    fn the_tags_instruction_is_a_path_or_the_empty_text() {
        let env = |variables: &'static [(&'static str, &'static str)], text: &str| {
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
            .map(|section| section.instruction.value)
            .map_err(|error| error.to_string())
        };
        let home: &[(&str, &str)] = &[("HOME", "/home/me")];
        assert_eq!(env(home, ""), Ok(None));
        assert_eq!(env(home, "derive.tags.instruction = \"\""), Ok(None));
        assert_eq!(
            env(home, "derive.tags.instruction = \"/c/tags.txt\""),
            Ok(Some(std::path::PathBuf::from("/c/tags.txt")))
        );
        assert_eq!(
            env(home, "derive.tags.instruction = \"~/notes/tags.txt\""),
            Ok(Some(std::path::PathBuf::from("/home/me/notes/tags.txt")))
        );
        let over: &[(&str, &str)] = &[
            ("HOME", "/home/me"),
            ("BILBO_DERIVE_TAGS_INSTRUCTION", "/v/tags.txt"),
        ];
        assert_eq!(
            env(over, "derive.tags.instruction = \"/c/tags.txt\""),
            Ok(Some(std::path::PathBuf::from("/v/tags.txt")))
        );
        for text in [
            "derive.tags.instruction = \"tags.txt\"",
            "derive.tags.instruction = \"/c/../tags.txt\"",
            "derive.tags.instruction = 3",
        ] {
            let message = env(home, text).expect_err("refused");
            assert!(
                message.starts_with("/c/config.toml:1: derive.tags.instruction"),
                "{message}"
            );
        }
    }
}
