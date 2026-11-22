pub mod api;
pub mod tool;

use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};

use crate::model::{Host, MODEL_ALLOWED, Model, Way};

pub const NAME: &str = "anthropic";
pub const SUMMARY: &str = "Anthropic's models: via api, with a key; via tool, the tool claude";

pub const MODEL: Key<String> = Key {
    about: About {
        name: "llm.anthropic.model",
        variable: Some("BILBO_LLM_ANTHROPIC_MODEL"),
        default: Default::Toml(r#""""#),
        allowed: MODEL_ALLOWED,
        doc: "The model bilbo asks Anthropic for, by either way. bilbo names none.",
    },
    parse: crate::api::model,
    show: |id| quote(id),
};

pub const VIA: Key<Way> = Key {
    about: About {
        name: "llm.anthropic.via",
        variable: Some("BILBO_LLM_ANTHROPIC_VIA"),
        default: Default::Toml(r#""api""#),
        allowed: "api or tool",
        doc: "The way bilbo reaches Anthropic's models: api sends a request with a key, tool starts the tool claude with its own login.",
    },
    parse: |value| {
        value
            .text()
            .and_then(Way::named)
            .ok_or_else(|| value.refuse())
    },
    show: |way| quote(way.word()),
};

pub const ABOUT: [&About; 14] = [
    &MODEL.about,
    &VIA.about,
    &api::ENDPOINT.about,
    &api::VERSION.about,
    &api::KEY_ENV.about,
    &api::KEY_COMMAND.about,
    &api::HEADERS.about,
    &api::EXTRA.about,
    &tool::PROGRAM.about,
    &tool::EFFORT.about,
    &tool::ARGUMENTS.about,
    &tool::INHERIT.about,
    &tool::ENVIRONMENT.about,
    &tool::PROJECT_FILES.about,
];

#[derive(Debug, Clone)]
pub struct Section {
    pub via: Resolved<Way>,
    pub api: api::Section,
    pub tool: tool::Section,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        via: VIA.resolve(source)?,
        api: api::section(source)?,
        tool: tool::section(source)?,
    })
}

pub fn shown(source: &Source<'_>) -> Result<Vec<Shown>, Error> {
    let mut all = vec![
        MODEL.shown(&MODEL.resolve(source)?),
        VIA.shown(&VIA.resolve(source)?),
    ];
    all.extend(api::shown(source)?);
    all.extend(tool::shown(source)?);
    Ok(all)
}

impl Section {
    #[must_use]
    pub fn asking(mut self, key: &'static str, model: &str) -> Section {
        self.api.model.value = model.to_owned();
        self.api.model_key = key;
        self.tool.model.value = model.to_owned();
        self.tool.model_key = key;
        self
    }

    #[must_use]
    pub fn reads(&self) -> Vec<String> {
        match self.via.value {
            Way::Api => vec![self.api.key_env.value.clone()],
            Way::Tool => self.tool.inherit.value.clone(),
        }
    }
}

#[must_use]
pub fn model(section: Section, host: Host) -> Box<dyn Model> {
    match section.via.value {
        Way::Api => Box::new(api::Anthropic::new(
            section.api,
            host.variables,
            host.state,
            host.timeout,
            host.response,
            host.poll,
        )),
        Way::Tool => Box::new(tool::Claude::new(section.tool, host.variables, host.work)),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use bilbo_settings::error::Error;
    use bilbo_settings::file::File;
    use bilbo_settings::key::{Origin, Source};

    use super::{ABOUT, Section, VIA, model, section, shown};
    use crate::model::{Host, Way};

    const NAMES: [&str; 14] = [
        "llm.anthropic.model",
        "llm.anthropic.via",
        "llm.anthropic.api.endpoint",
        "llm.anthropic.api.version",
        "llm.anthropic.api.key_env",
        "llm.anthropic.api.key_command",
        "llm.anthropic.api.headers",
        "llm.anthropic.api.extra",
        "llm.anthropic.tool.program",
        "llm.anthropic.tool.effort",
        "llm.anthropic.tool.arguments",
        "llm.anthropic.tool.inherit",
        "llm.anthropic.tool.environment",
        "llm.anthropic.tool.project_files",
    ];

    fn resolve_with(text: &str, variable: (&'static str, &str)) -> Result<Section, Error> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let given = variable.1.to_owned();
        let name = variable.0;
        section(&Source {
            var: &|asked| (asked == name).then(|| given.clone().into()),
            file: &file,
        })
    }

    fn resolve(text: &str) -> Result<Section, Error> {
        resolve_with(text, ("UNUSED", ""))
    }

    fn host() -> Host {
        Host {
            variables: Box::new(|_| None),
            state: PathBuf::from("/s"),
            work: PathBuf::from("/s/derive/work"),
            timeout: Duration::from_secs(1),
            response: 1024,
            poll: Duration::from_millis(5),
        }
    }

    #[test]
    fn the_writer_is_the_way_via_names() {
        let api = model(resolve("[llm.anthropic]\nmodel = \"m\"\n").unwrap(), host());
        assert_eq!(
            api.describe(),
            [
                "provider: anthropic, via api, model m, at https://api.anthropic.com/v1/messages",
                "version: 2026-10-21",
                "key: the variable ANTHROPIC_API_KEY, which is not set",
            ]
        );
        assert_eq!(api.id(), "m");
        let tool = model(
            resolve("[llm.anthropic]\nmodel = \"m\"\nvia = \"tool\"\n").unwrap(),
            host(),
        );
        let lines = tool.describe();
        assert_eq!(lines.len(), 5);
        assert_eq!(
            lines[0],
            "provider: anthropic, via tool, model m, effort medium, program claude"
        );
        assert_eq!(tool.id(), "m");
    }

    #[test]
    fn asking_sets_the_model_of_both_ways_and_the_key_that_named_it() {
        let section = resolve("[llm.anthropic]\nmodel = \"m\"\n").unwrap();
        assert_eq!(section.api.model.value, "m");
        assert_eq!(section.api.model_key, "llm.anthropic.model");
        assert_eq!(section.tool.model.value, "m");
        assert_eq!(section.tool.model_key, "llm.anthropic.model");
        let asked = section.asking("derive.tags.model", "m2");
        assert_eq!(asked.api.model.value, "m2");
        assert_eq!(asked.api.model_key, "derive.tags.model");
        assert_eq!(asked.tool.model.value, "m2");
        assert_eq!(asked.tool.model_key, "derive.tags.model");
    }

    #[test]
    fn the_names_read_are_the_keys_variable_or_the_inherited_ones() {
        let reads = |text: &str| resolve(text).unwrap().reads();
        assert_eq!(reads(""), ["ANTHROPIC_API_KEY"]);
        assert_eq!(
            reads("[llm.anthropic.api]\nkey_env = \"MY_KEY\"\n"),
            ["MY_KEY"]
        );
        assert_eq!(
            reads("[llm.anthropic]\nvia = \"tool\"\n"),
            ["HOME", "USER", "PATH", "TMPDIR"]
        );
        assert_eq!(
            reads("[llm.anthropic]\nvia = \"tool\"\n\n[llm.anthropic.tool]\ninherit = [\"A_B\"]\n"),
            ["A_B"]
        );
    }

    #[test]
    fn via_is_api_or_tool_and_api_when_unset() {
        let none = resolve("").unwrap();
        assert_eq!(none.via.value, Way::Api);
        assert_eq!(none.via.origin, Origin::Default);
        let file = resolve("[llm.anthropic]\nvia = \"tool\"\n").unwrap();
        assert_eq!(file.via.value, Way::Tool);
        assert_eq!(
            file.via.origin,
            Origin::File {
                path: PathBuf::from("/c/config.toml"),
                line: 2
            }
        );
        let source = |text: &str| {
            let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).unwrap();
            let shown = VIA.shown(
                &VIA.resolve(&Source {
                    var: &|_| None,
                    file: &file,
                })
                .unwrap(),
            );
            shown.value
        };
        assert_eq!(source("[llm.anthropic]\nvia = \"tool\"\n"), "\"tool\"");
        let variable = resolve_with(
            "[llm.anthropic]\nvia = \"api\"\n",
            ("BILBO_LLM_ANTHROPIC_VIA", "tool"),
        )
        .unwrap();
        assert_eq!(variable.via.value, Way::Tool);
        assert_eq!(
            variable.via.origin,
            Origin::Variable("BILBO_LLM_ANTHROPIC_VIA")
        );
        let empty_over_file = resolve_with(
            "[llm.anthropic]\nvia = \"tool\"\n",
            ("BILBO_LLM_ANTHROPIC_VIA", ""),
        )
        .unwrap();
        assert_eq!(empty_over_file.via.value, Way::Tool);
        assert_eq!(
            empty_over_file.via.origin,
            Origin::File {
                path: PathBuf::from("/c/config.toml"),
                line: 2
            }
        );
        let empty = resolve_with("", ("BILBO_LLM_ANTHROPIC_VIA", "")).unwrap();
        assert_eq!(empty.via.value, Way::Api);
        for given in ["\"cli\"", "\"\"", "\"API\"", "\"tool \"", "1", "[\"api\"]"] {
            let error = resolve(&format!("[llm.anthropic]\nvia = {given}\n")).expect_err("refused");
            assert_eq!(
                error.to_string(),
                format!("/c/config.toml:2: llm.anthropic.via must be api or tool, got {given}")
            );
        }
        let error = resolve_with("", ("BILBO_LLM_ANTHROPIC_VIA", "cli")).expect_err("refused");
        assert_eq!(
            error.to_string(),
            "BILBO_LLM_ANTHROPIC_VIA must be api or tool, got \"cli\""
        );
    }

    #[test]
    fn the_group_lists_its_keys_in_their_order() {
        let names: Vec<_> = ABOUT.iter().map(|about| about.name).collect();
        assert_eq!(names, NAMES);
        let file = File::parse(Path::new("/c/config.toml"), b"").unwrap();
        let listed = shown(&Source {
            var: &|_| None,
            file: &file,
        })
        .unwrap();
        let listed: Vec<_> = listed.iter().map(|key| key.name).collect();
        assert_eq!(listed, names);
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_LLM_ANTHROPIC_MODEL"),
                Some("BILBO_LLM_ANTHROPIC_VIA"),
                Some("BILBO_LLM_ANTHROPIC_API_ENDPOINT"),
                Some("BILBO_LLM_ANTHROPIC_API_VERSION"),
                Some("BILBO_LLM_ANTHROPIC_API_KEY_ENV"),
                None,
                None,
                None,
                Some("BILBO_LLM_ANTHROPIC_TOOL_PROGRAM"),
                Some("BILBO_LLM_ANTHROPIC_TOOL_EFFORT"),
                None,
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn a_bad_value_in_the_table_of_the_other_way_is_still_refused() {
        let listed = |text: &str| {
            let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).unwrap();
            shown(&Source {
                var: &|_| None,
                file: &file,
            })
        };
        assert_eq!(
            listed("[llm.anthropic.tool]\neffort = \"High\"\n")
                .expect_err("refused")
                .to_string(),
            "/c/config.toml:2: llm.anthropic.tool.effort must be a word of lower-case letters, got \"High\""
        );
        assert_eq!(
            listed("[llm.anthropic]\nvia = \"tool\"\n\n[llm.anthropic.api]\nversion = \"a b\"\n")
                .expect_err("refused")
                .to_string(),
            "/c/config.toml:5: llm.anthropic.api.version must be 1 to 32 letters, digits and dashes"
        );
        assert!(listed("[llm.anthropic.tool]\narguments = [\"--bad\"]\n").is_ok());
    }
}
