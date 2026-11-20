pub mod api;

use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};

use crate::writer::{Host, MODEL_ALLOWED, Way, Writer};

pub const NAME: &str = "openai";
pub const SUMMARY: &str = "OpenAI's models: via api, with a key";

pub const MODEL: Key<String> = Key {
    about: About {
        name: "derive.openai.model",
        variable: Some("BILBO_DERIVE_OPENAI_MODEL"),
        default: Default::Toml(r#""""#),
        allowed: MODEL_ALLOWED,
        doc: "The model bilbo derive asks the OpenAI API for. bilbo names none.",
    },
    parse: crate::api::model,
    show: |id| quote(id),
};

pub const VIA: Key<Way> = Key {
    about: About {
        name: "derive.openai.via",
        variable: Some("BILBO_DERIVE_OPENAI_VIA"),
        default: Default::Toml(r#""api""#),
        allowed: "api",
        doc: "The way bilbo derive reaches OpenAI's models: api sends a request with a key. No other way is built.",
    },
    parse: |value| match value.text().and_then(Way::named) {
        Some(Way::Api) => Ok(Way::Api),
        Some(Way::Tool) => Err(value.refused("must be api: tool is not built for openai")),
        None => Err(value.refuse()),
    },
    show: |way| quote(way.word()),
};

pub const ABOUT: [&About; 7] = [
    &MODEL.about,
    &VIA.about,
    &api::ENDPOINT.about,
    &api::KEY_ENV.about,
    &api::KEY_COMMAND.about,
    &api::HEADERS.about,
    &api::EXTRA.about,
];

#[derive(Debug, Clone)]
pub struct Section {
    pub via: Resolved<Way>,
    pub api: api::Section,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        via: VIA.resolve(source)?,
        api: api::section(source)?,
    })
}

pub fn shown(source: &Source<'_>) -> Result<Vec<Shown>, Error> {
    let mut all = vec![
        MODEL.shown(&MODEL.resolve(source)?),
        VIA.shown(&VIA.resolve(source)?),
    ];
    all.extend(api::shown(source)?);
    Ok(all)
}

impl Section {
    #[must_use]
    pub fn reads(&self) -> Vec<String> {
        vec![self.api.key_env.value.clone()]
    }
}

#[must_use]
pub fn writer(section: Section, host: Host) -> Box<dyn Writer> {
    Box::new(api::OpenAi::new(
        section.api,
        host.variables,
        host.state,
        host.timeout,
        host.response,
        host.poll,
    ))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use bilbo_settings::error::Error;
    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, section, shown, writer};
    use crate::writer::{Host, Way};

    const NAMES: [&str; 7] = [
        "derive.openai.model",
        "derive.openai.via",
        "derive.openai.api.endpoint",
        "derive.openai.api.key_env",
        "derive.openai.api.key_command",
        "derive.openai.api.headers",
        "derive.openai.api.extra",
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
        let built = writer(resolve("derive.openai.model = \"m\"\n").unwrap(), host());
        assert_eq!(
            built.describe(),
            [
                "provider: openai, via api, model m, at https://api.openai.com/v1/responses",
                "key: the variable OPENAI_API_KEY, which is not set",
            ]
        );
        assert_eq!(built.model(), "m");
    }

    #[test]
    fn via_is_api_and_tool_is_said_as_not_built() {
        assert_eq!(resolve("").unwrap().via.value, Way::Api);
        assert_eq!(
            resolve("derive.openai.via = \"api\"\n").unwrap().via.value,
            Way::Api
        );
        assert_eq!(
            resolve("derive.openai.via = \"tool\"\n")
                .expect_err("refused")
                .to_string(),
            "/c/config.toml:1: derive.openai.via must be api: tool is not built for openai"
        );
        assert_eq!(
            resolve_with("", ("BILBO_DERIVE_OPENAI_VIA", "tool"))
                .expect_err("refused")
                .to_string(),
            "BILBO_DERIVE_OPENAI_VIA must be api: tool is not built for openai"
        );
        for given in ["cli", ""] {
            assert_eq!(
                resolve(&format!("derive.openai.via = \"{given}\"\n"))
                    .expect_err("refused")
                    .to_string(),
                format!("/c/config.toml:1: derive.openai.via must be api, got \"{given}\"")
            );
        }
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
            ]
        );
    }

    #[test]
    fn the_name_read_is_the_keys_variable() {
        let reads = |text: &str| resolve(text).unwrap().reads();
        assert_eq!(reads(""), ["OPENAI_API_KEY"]);
        assert_eq!(
            reads("derive.openai.api.key_env = \"MY_KEY\"\n"),
            ["MY_KEY"]
        );
    }
}
