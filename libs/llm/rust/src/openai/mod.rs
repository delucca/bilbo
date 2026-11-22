pub mod api;
pub mod tool;

use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};

use crate::model::{Host, MODEL_ALLOWED, Model, Way};

pub const NAME: &str = "openai";
pub const SUMMARY: &str = "OpenAI's models: via api, with a key; via tool, the tool codex";

pub const MODEL: Key<String> = Key {
    about: About {
        name: "derive.openai.model",
        variable: Some("BILBO_DERIVE_OPENAI_MODEL"),
        default: Default::Toml(r#""""#),
        allowed: MODEL_ALLOWED,
        doc: "The model bilbo derive asks OpenAI for, by either way. bilbo names none.",
    },
    parse: crate::api::model,
    show: |id| quote(id),
};

pub const VIA: Key<Way> = Key {
    about: About {
        name: "derive.openai.via",
        variable: Some("BILBO_DERIVE_OPENAI_VIA"),
        default: Default::Toml(r#""api""#),
        allowed: "api or tool",
        doc: "The way bilbo derive reaches OpenAI's models: api sends a request with a key, tool starts the tool codex with its own login.",
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
    &api::KEY_ENV.about,
    &api::KEY_COMMAND.about,
    &api::HEADERS.about,
    &api::EXTRA.about,
    &tool::PROGRAM.about,
    &tool::EFFORT.about,
    &tool::ARGUMENTS.about,
    &tool::INHERIT.about,
    &tool::ENVIRONMENT.about,
    &tool::HOME.about,
    &tool::SYSTEM_FILES.about,
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
    pub fn reads(&self) -> Vec<String> {
        match self.via.value {
            Way::Api => vec![self.api.key_env.value.clone()],
            Way::Tool => self.tool.reads(),
        }
    }
}

#[must_use]
pub fn model(section: Section, host: Host) -> Box<dyn Model> {
    match section.via.value {
        Way::Api => Box::new(api::OpenAi::new(
            section.api,
            host.variables,
            host.state,
            host.timeout,
            host.response,
            host.poll,
        )),
        Way::Tool => Box::new(tool::Codex::new(
            section.tool,
            host.variables,
            host.work,
            host.timeout,
            host.response,
            host.poll,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use bilbo_settings::error::Error;
    use bilbo_settings::file::File;
    use bilbo_settings::key::{Origin, Source};

    use super::{ABOUT, Section, VIA, model, section, shown};
    use crate::model::{Host, Way};

    const NAMES: [&str; 14] = [
        "derive.openai.model",
        "derive.openai.via",
        "derive.openai.api.endpoint",
        "derive.openai.api.key_env",
        "derive.openai.api.key_command",
        "derive.openai.api.headers",
        "derive.openai.api.extra",
        "derive.openai.tool.program",
        "derive.openai.tool.effort",
        "derive.openai.tool.arguments",
        "derive.openai.tool.inherit",
        "derive.openai.tool.environment",
        "derive.openai.tool.home",
        "derive.openai.tool.system_files",
    ];

    fn resolve_with(text: &str, variable: (&'static str, &str)) -> Result<Section, Error> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let given = OsString::from(variable.1);
        let name = variable.0;
        section(&Source {
            var: &|asked| (asked == name).then(|| given.clone()),
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
    fn the_writer_is_the_api_way() {
        let built = model(resolve("derive.openai.model = \"m\"\n").unwrap(), host());
        assert_eq!(
            built.describe(),
            [
                "provider: openai, via api, model m, at https://api.openai.com/v1/responses",
                "key: the variable OPENAI_API_KEY, which is not set",
            ]
        );
        assert_eq!(built.id(), "m");
    }

    #[test]
    fn via_tool_builds_the_tool_way() {
        let built = model(
            resolve("derive.openai.model = \"m\"\nderive.openai.via = \"tool\"\n").unwrap(),
            host(),
        );
        let lines = built.describe();
        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[0],
            "provider: openai, via tool, model m, effort medium, program codex"
        );
        assert_eq!(built.id(), "m");
    }

    #[test]
    fn via_is_api_or_tool_and_api_when_unset() {
        assert_eq!(resolve("").unwrap().via.value, Way::Api);
        assert_eq!(resolve("").unwrap().via.origin, Origin::Default);
        assert_eq!(
            resolve("derive.openai.via = \"api\"\n").unwrap().via.value,
            Way::Api
        );
        let file = resolve("derive.openai.via = \"tool\"\n").unwrap();
        assert_eq!(file.via.value, Way::Tool);
        assert_eq!(
            file.via.origin,
            Origin::File {
                path: PathBuf::from("/c/config.toml"),
                line: 1
            }
        );
        let parsed = File::parse(
            Path::new("/c/config.toml"),
            b"derive.openai.via = \"tool\"\n",
        )
        .unwrap();
        let shown = VIA.shown(
            &VIA.resolve(&Source {
                var: &|_| None,
                file: &parsed,
            })
            .unwrap(),
        );
        assert_eq!(shown.value, "\"tool\"");
        let variable = resolve_with(
            "derive.openai.via = \"api\"\n",
            ("BILBO_DERIVE_OPENAI_VIA", "tool"),
        )
        .unwrap();
        assert_eq!(variable.via.value, Way::Tool);
        assert_eq!(
            variable.via.origin,
            Origin::Variable("BILBO_DERIVE_OPENAI_VIA")
        );
        for given in ["\"cli\"", "\"\"", "\"API\"", "1"] {
            assert_eq!(
                resolve(&format!("derive.openai.via = {given}\n"))
                    .expect_err("refused")
                    .to_string(),
                format!("/c/config.toml:1: derive.openai.via must be api or tool, got {given}")
            );
        }
        assert_eq!(
            resolve_with("", ("BILBO_DERIVE_OPENAI_VIA", "cli"))
                .expect_err("refused")
                .to_string(),
            "BILBO_DERIVE_OPENAI_VIA must be api or tool, got \"cli\""
        );
        assert_eq!(
            resolve_with("", ("BILBO_DERIVE_OPENAI_VIA", ""))
                .unwrap()
                .via
                .value,
            Way::Api
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
                Some("BILBO_DERIVE_OPENAI_MODEL"),
                Some("BILBO_DERIVE_OPENAI_VIA"),
                Some("BILBO_DERIVE_OPENAI_API_ENDPOINT"),
                Some("BILBO_DERIVE_OPENAI_API_KEY_ENV"),
                None,
                None,
                None,
                Some("BILBO_DERIVE_OPENAI_TOOL_PROGRAM"),
                Some("BILBO_DERIVE_OPENAI_TOOL_EFFORT"),
                None,
                None,
                None,
                Some("BILBO_DERIVE_OPENAI_TOOL_HOME"),
                None,
            ]
        );
    }

    #[test]
    fn the_names_read_are_the_keys_variable_or_the_tools() {
        let reads = |text: &str| resolve(text).unwrap().reads();
        assert_eq!(reads(""), ["OPENAI_API_KEY"]);
        assert_eq!(
            reads("derive.openai.api.key_env = \"MY_KEY\"\n"),
            ["MY_KEY"]
        );
        assert_eq!(
            reads("derive.openai.via = \"tool\"\n"),
            ["PATH", "TMPDIR", "CODEX_HOME", "HOME"]
        );
        assert_eq!(
            reads("derive.openai.via = \"tool\"\n\n[derive.openai.tool]\ninherit = [\"A_B\"]\n"),
            ["A_B", "PATH", "CODEX_HOME", "HOME"]
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
            listed("[derive.openai.tool]\neffort = \"High\"\n")
                .expect_err("refused")
                .to_string(),
            "/c/config.toml:2: derive.openai.tool.effort must be a word of lower-case letters, got \"High\""
        );
        let error =
            listed("derive.openai.via = \"tool\"\nderive.openai.api.endpoint = \"ftp://x\"\n")
                .expect_err("refused")
                .to_string();
        assert!(error.starts_with("/c/config.toml:2: "), "{error}");
        assert!(error.contains("derive.openai.api.endpoint"), "{error}");
        assert!(listed("[derive.openai.tool]\narguments = [\"--bad\"]\n").is_ok());
    }
}
