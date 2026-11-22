use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};
use serde_json::{Map, Value as Json};

use crate::api::{self, Call};
use crate::child::Bounds;
use crate::http::{self, Endpoint, Response};
use crate::secret::Secret;
use crate::table;
use crate::writer::{
    Cause, Limits, Outcome, Prompt, Usage, Variables, Way, What, Writer, answered_model,
    identifier, is_identifier,
};

const HEADERS_REFUSED: [&str; 8] = [
    "x-api-key",
    "authorization",
    "anthropic-version",
    "content-type",
    "content-length",
    "host",
    "transfer-encoding",
    "accept-encoding",
];
const EXTRA_REFUSED: [&str; 10] = [
    "model",
    "max_tokens",
    "system",
    "messages",
    "stream",
    "tools",
    "tool_choice",
    "mcp_servers",
    "container",
    "thread",
];
const BLOCKS_PASSED_OVER: [&str; 4] = ["thinking", "redacted_thinking", "fallback", "compaction"];
const BLOCKS_OF_A_TOOL: [&str; 13] = [
    "tool_use",
    "server_tool_use",
    "web_search_tool_result",
    "web_fetch_tool_result",
    "code_execution_tool_result",
    "bash_code_execution_tool_result",
    "text_editor_code_execution_tool_result",
    "tool_search_tool_result",
    "container_upload",
    "mcp_tool_use",
    "mcp_tool_result",
    "mcp_tool_listing",
    "advisor_tool_result",
];
const SPEND_CAP: &str = "enforced_spend_limit_reached";
const VERSION_BYTES: usize = 32;

pub const ENDPOINT: Key<Endpoint> = Key {
    about: About {
        name: "derive.anthropic.api.endpoint",
        variable: Some("BILBO_DERIVE_ANTHROPIC_API_ENDPOINT"),
        default: Default::Toml(r#""https://api.anthropic.com/v1/messages""#),
        allowed: api::ENDPOINT_ALLOWED,
        doc: "Where bilbo derive sends a request for the Anthropic API.",
    },
    parse: api::endpoint,
    show: |endpoint| quote(endpoint.as_str()),
};

pub const VERSION: Key<String> = Key {
    about: About {
        name: "derive.anthropic.api.version",
        variable: Some("BILBO_DERIVE_ANTHROPIC_API_VERSION"),
        default: Default::Toml(r#""2026-10-21""#),
        allowed: "1 to 32 letters, digits and dashes",
        doc: "The value of the anthropic-version header.",
    },
    parse: |value| {
        value
            .text()
            .filter(|text| {
                (1..=VERSION_BYTES).contains(&text.len())
                    && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            .map(str::to_owned)
            .ok_or_else(|| value.refused("must be 1 to 32 letters, digits and dashes"))
    },
    show: |text| quote(text),
};

pub const KEY_ENV: Key<String> = Key {
    about: About {
        name: "derive.anthropic.api.key_env",
        variable: Some("BILBO_DERIVE_ANTHROPIC_API_KEY_ENV"),
        default: Default::Toml(r#""ANTHROPIC_API_KEY""#),
        allowed: api::KEY_ENV_ALLOWED,
        doc: "The variable bilbo derive reads the Anthropic API key from when key_command is empty.",
    },
    parse: api::key_env,
    show: |name| quote(name),
};

pub const KEY_COMMAND: Key<Vec<String>> = Key {
    about: About {
        name: "derive.anthropic.api.key_command",
        variable: None,
        default: Default::Toml("[]"),
        allowed: api::KEY_COMMAND_ALLOWED,
        doc: "A program and its arguments whose first line of standard output is the Anthropic API key. When it is set bilbo derive runs it and reads no variable.",
    },
    parse: api::key_command,
    show: |items| table::show_list(items),
};

pub const HEADERS: Key<Vec<(String, String)>> = Key {
    about: About {
        name: "derive.anthropic.api.headers",
        variable: None,
        default: Default::Toml("{}"),
        allowed: api::HEADERS_ALLOWED,
        doc: "Headers added to each request as they are. A secret never goes here: bilbo config prints every value.",
    },
    parse: |value| table::headers(value, &HEADERS_REFUSED),
    show: |headers| table::show_texts(headers),
};

pub const EXTRA: Key<Map<String, Json>> = Key {
    about: About {
        name: "derive.anthropic.api.extra",
        variable: None,
        default: Default::Toml("{}"),
        allowed: api::EXTRA_ALLOWED,
        doc: "Fields added to the body of each request as they are.",
    },
    parse: |value| table::fields(value, &EXTRA_REFUSED),
    show: |fields| table::show_fields(fields),
};

pub const ABOUT: [&About; 6] = [
    &ENDPOINT.about,
    &VERSION.about,
    &KEY_ENV.about,
    &KEY_COMMAND.about,
    &HEADERS.about,
    &EXTRA.about,
];

#[derive(Debug, Clone)]
pub struct Section {
    pub model: Resolved<String>,
    pub endpoint: Resolved<Endpoint>,
    pub version: Resolved<String>,
    pub key_env: Resolved<String>,
    pub key_command: Resolved<Vec<String>>,
    pub headers: Resolved<Vec<(String, String)>>,
    pub extra: Resolved<Map<String, Json>>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        model: super::MODEL.resolve(source)?,
        endpoint: ENDPOINT.resolve(source)?,
        version: VERSION.resolve(source)?,
        key_env: KEY_ENV.resolve(source)?,
        key_command: KEY_COMMAND.resolve(source)?,
        headers: HEADERS.resolve(source)?,
        extra: EXTRA.resolve(source)?,
    })
}

pub fn shown(source: &Source<'_>) -> Result<Vec<Shown>, Error> {
    let section = section(source)?;
    Ok(vec![
        ENDPOINT.shown(&section.endpoint),
        VERSION.shown(&section.version),
        KEY_ENV.shown(&section.key_env),
        KEY_COMMAND.shown(&section.key_command),
        HEADERS.shown(&section.headers),
        EXTRA.shown(&section.extra),
    ])
}

pub struct Anthropic {
    section: Section,
    variables: Variables,
    folder: PathBuf,
    bounds: Bounds,
    key: Option<Secret>,
}

impl fmt::Debug for Anthropic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Anthropic")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

impl Anthropic {
    pub fn new(
        section: Section,
        variables: Variables,
        folder: PathBuf,
        timeout: Duration,
        response: usize,
        poll: Duration,
    ) -> Anthropic {
        Anthropic {
            section,
            variables,
            folder,
            bounds: Bounds {
                limit: response,
                timeout,
                poll,
            },
            key: None,
        }
    }

    fn request(
        &self,
        key: &Secret,
        prompt: &Prompt,
        limits: &Limits,
    ) -> (Vec<(String, String)>, Vec<u8>) {
        let mut headers = vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-api-key".to_owned(), key.expose().to_owned()),
            (
                "anthropic-version".to_owned(),
                self.section.version.value.clone(),
            ),
        ];
        headers.extend(self.section.headers.value.iter().cloned());
        let mut body = self.section.extra.value.clone();
        body.insert("max_tokens".to_owned(), Json::from(limits.answer_tokens));
        body.insert(
            "messages".to_owned(),
            Json::Array(vec![Json::Object(Map::from_iter([
                ("role".to_owned(), Json::from("user")),
                ("content".to_owned(), Json::from(prompt.material.as_str())),
            ]))]),
        );
        body.insert(
            "model".to_owned(),
            Json::from(self.section.model.value.as_str()),
        );
        body.insert("system".to_owned(), Json::from(prompt.instruction.as_str()));
        (headers, Json::Object(body).to_string().into_bytes())
    }
}

impl Writer for Anthropic {
    fn prepare(&mut self) -> Result<(), String> {
        let section = &self.section;
        let extra = table::texts(&section.extra.value);
        let call = Call {
            provider: super::NAME,
            env: &section.key_env.value,
            command: &section.key_command.value,
            variables: &*self.variables,
            folder: &self.folder,
            bounds: self.bounds,
        };
        let ahead = [("api.version", section.version.value.as_str())];
        let found = api::prepare(
            &call,
            &section.model.value,
            &ahead,
            &section.headers.value,
            &extra,
        )?;
        self.key = Some(found);
        Ok(())
    }

    fn write(&mut self, prompt: &Prompt, limits: &Limits) -> Outcome {
        let Some(key) = &self.key else {
            return Outcome::Stopped(Cause::new(What::NotStarted));
        };
        let (headers, body) = self.request(key, prompt, limits);
        let headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        match http::post(
            &self.section.endpoint.value,
            &headers,
            &body,
            &["retry-after", "request-id"],
            limits.timeout,
            limits.response,
        ) {
            Ok(response) => read(key, &response),
            Err(fault) => api::fault(&fault),
        }
    }

    fn model(&self) -> &str {
        &self.section.model.value
    }

    fn describe(&self) -> Vec<String> {
        let section = &self.section;
        let head = vec![
            format!(
                "provider: {}, via {}, model {}, at {}",
                super::NAME,
                Way::Api.word(),
                section.model.value,
                section.endpoint.value.as_str()
            ),
            format!("version: {}", section.version.value),
        ];
        let extra =
            (!section.extra.value.is_empty()).then(|| table::show_fields(&section.extra.value));
        let key = api::key_line(
            &section.key_env.value,
            &section.key_command.value,
            &*self.variables,
        );
        api::describe(head, &section.headers.value, extra, key)
    }
}

fn text<'a>(value: &'a Json, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |at, name| at.get(name))?
        .as_str()
}

fn usage(body: &Json) -> Usage {
    let Some(counts) = body.get("usage").filter(|counts| counts.is_object()) else {
        return Usage::default();
    };
    let count = |name: &str| counts.get(name).and_then(Json::as_u64);
    let inputs = [
        count("input_tokens"),
        count("cache_creation_input_tokens"),
        count("cache_read_input_tokens"),
    ];
    Usage {
        read: inputs.iter().any(Option::is_some).then(|| {
            inputs
                .iter()
                .flatten()
                .fold(0, |sum, n| u64::saturating_add(sum, *n))
        }),
        written: count("output_tokens"),
    }
}

fn cause(
    key: &Secret,
    what: What,
    status: u16,
    body: Option<&Json>,
    request: Option<&str>,
) -> Cause {
    let found = |path: &[&str]| {
        body.and_then(|body| text(body, path))
            .filter(|said| !key.hides(&[said]))
            .map(identifier)
    };
    Cause {
        what,
        status: Some(i32::from(status)),
        kind: found(&["error", "type"]),
        code: found(&["error", "details", "error_code"]),
        request: request.map(identifier),
    }
}

fn request_of(key: &Secret, response: &Response, body: Option<&Json>) -> Option<String> {
    let given: Vec<&str> = [
        response.header("request-id"),
        body.and_then(|body| text(body, &["request_id"])),
    ]
    .into_iter()
    .flatten()
    .filter(|id| !key.hides(&[id]))
    .collect();
    given
        .iter()
        .find(|id| is_identifier(id))
        .or_else(|| given.first())
        .map(|id| identifier(id))
}

fn read(key: &Secret, response: &Response) -> Outcome {
    let body: Option<Json> = serde_json::from_slice(&response.body)
        .ok()
        .filter(Json::is_object);
    let request = request_of(key, response, body.as_ref());
    let request = request.as_deref();
    let status = response.status;
    let fail = |what: What, usage: Usage| Outcome::Failed {
        cause: cause(key, what, status, body.as_ref(), request),
        usage,
    };
    let mut seen: Vec<&str> = response.header("request-id").into_iter().collect();
    let parts = body.as_ref().map(texts_of).unwrap_or_default();
    let joined = parts.concat();
    if let Some(body) = &body {
        for path in [
            &["request_id"][..],
            &["error", "type"],
            &["error", "details", "error_code"],
            &["model"],
            &["stop_reason"],
            &["stop_details", "category"],
        ] {
            seen.extend(text(body, path));
        }
        if status == 200 {
            seen.extend(&parts);
            seen.push(&joined);
        }
    }
    if status != 200 {
        let outcome = failed_status(key, response, body.as_ref(), request);
        let provider = match &outcome {
            Outcome::Busy { .. } => true,
            Outcome::Stopped(cause) => cause.what != What::Rejected,
            _ => false,
        };
        if provider {
            return outcome;
        }
        return api::holds_key(key, &seen, status).unwrap_or(outcome);
    }
    if let Some(held) = api::holds_key(key, &seen, status) {
        return held;
    }
    let Some(body) = &body else {
        return fail(What::Unreadable, Usage::default());
    };
    let counted = usage(body);
    match text(body, &["stop_reason"]) {
        Some("end_turn") => match answer(body) {
            Ok(said) => Outcome::Answer {
                text: said,
                usage: counted,
                model: text(body, &["model"]).map(answered_model),
            },
            Err(what) => fail(what, counted),
        },
        Some("max_tokens") => Outcome::Cut(counted),
        Some("refusal") => Outcome::Refused {
            category: text(body, &["stop_details", "category"]).map(identifier),
            usage: counted,
        },
        None => fail(What::Unreadable, counted),
        other => Outcome::Failed {
            cause: Cause {
                code: other.map(identifier),
                ..cause(key, What::StopReason, status, Some(body), request)
            },
            usage: counted,
        },
    }
}

fn blocks(body: &Json) -> &[Json] {
    body.get("content")
        .and_then(Json::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn type_of(block: &Json) -> Option<&str> {
    block.get("type").and_then(Json::as_str)
}

fn texts_of(body: &Json) -> Vec<&str> {
    blocks(body)
        .iter()
        .filter(|block| type_of(block) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Json::as_str))
        .collect()
}

fn answer(body: &Json) -> Result<String, What> {
    if !body.get("content").is_some_and(Json::is_array) {
        return Err(What::Unreadable);
    }
    let mut joined = String::new();
    let mut texts = 0;
    let mut tool = false;
    let mut unreadable = false;
    for block in blocks(body) {
        match type_of(block) {
            Some("text") => match block.get("text").and_then(Json::as_str) {
                Some(text) => {
                    joined.push_str(text);
                    texts += 1;
                }
                None => unreadable = true,
            },
            Some(kind) if BLOCKS_PASSED_OVER.contains(&kind) => {}
            Some(kind) if BLOCKS_OF_A_TOOL.contains(&kind) => tool = true,
            _ => unreadable = true,
        }
    }
    if tool {
        Err(What::ToolUsed)
    } else if unreadable || texts == 0 {
        Err(What::Unreadable)
    } else {
        Ok(joined)
    }
}

fn failed_status(
    key: &Secret,
    response: &Response,
    body: Option<&Json>,
    request: Option<&str>,
) -> Outcome {
    let status = response.status;
    let cause = |what| cause(key, what, status, body, request);
    let spent =
        body.and_then(|body| text(body, &["error", "details", "error_code"])) == Some(SPEND_CAP);
    let busy = |what| Outcome::Busy {
        wait: response.wait(),
        cause: cause(what),
    };
    match status {
        429 if spent => Outcome::Stopped(cause(What::Billing)),
        429 => busy(What::RateLimit),
        408 | 504 => busy(What::Timeout),
        409 | 500..=503 | 505..=528 | 530.. => busy(What::ServerError),
        529 => busy(What::Overloaded),
        401 => Outcome::Stopped(cause(What::Unauthorized)),
        402 => Outcome::Stopped(cause(What::Billing)),
        403 => Outcome::Stopped(cause(What::Forbidden)),
        404 => Outcome::Stopped(cause(What::NotFound)),
        400..=499 => Outcome::Stopped(cause(What::Rejected)),
        _ => Outcome::Failed {
            cause: cause(What::Unreadable),
            usage: Usage::default(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bilbo_settings::error::{At, Error};
    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{ABOUT, Section, section, shown};

    const MODEL_REASON: &str = "must be a model's id of at most 200 characters with no space or control character, or the empty text for none";

    fn resolve(text: &str) -> Result<Section, Error> {
        resolve_with(text, ("UNUSED", ""))
    }

    fn refused(text: &str) -> Error {
        resolve(text).expect_err("refused")
    }

    fn resolve_with(text: &str, variable: (&'static str, &str)) -> Result<Section, Error> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let given = variable.1.to_owned();
        let name = variable.0;
        section(&Source {
            var: &|asked| (asked == name && !given.is_empty()).then(|| given.clone().into()),
            file: &file,
        })
    }

    fn at_line(error: &Error) -> usize {
        match error {
            Error::Invalid {
                at: At::Line { line, .. },
                ..
            }
            | Error::Refused {
                at: At::Line { line, .. },
                ..
            } => *line,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = resolve("").expect("defaults");
        assert_eq!(section.model.value, "");
        assert_eq!(
            section.endpoint.value.as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(section.version.value, "2026-10-21");
        assert_eq!(section.key_env.value, "ANTHROPIC_API_KEY");
        assert!(section.key_command.value.is_empty());
        assert!(section.headers.value.is_empty());
        assert!(section.extra.value.is_empty());
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_DERIVE_ANTHROPIC_API_ENDPOINT"),
                Some("BILBO_DERIVE_ANTHROPIC_API_VERSION"),
                Some("BILBO_DERIVE_ANTHROPIC_API_KEY_ENV"),
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn the_model_is_a_text_with_no_space() {
        for good in [
            "",
            "claude-x",
            "vendor/claude-x:2",
            &"m".repeat(200),
            &"é".repeat(200),
        ] {
            let text = format!("[derive.anthropic]\nmodel = \"{good}\"\n");
            assert_eq!(resolve(&text).expect("taken").model.value, good);
        }
        for bad in [
            &"m".repeat(201),
            &"é".repeat(201),
            "a b",
            "a\\tb",
            "a\\u0007b",
        ] {
            let text = format!("[derive.anthropic]\nmodel = \"{bad}\"\n");
            assert_eq!(
                refused(&text).to_string(),
                format!("/c/config.toml:2: derive.anthropic.model {MODEL_REASON}"),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_endpoint_is_https_or_plain_http_at_a_literal_loopback_address() {
        let taken =
            resolve("[derive.anthropic.api]\nendpoint = \"http://127.0.0.1:9/v1/messages\"\n");
        assert_eq!(
            taken.expect("taken").endpoint.value.as_str(),
            "http://127.0.0.1:9/v1/messages"
        );
        assert_eq!(
            refused("[derive.anthropic.api]\nendpoint = \"http://example.com\"\n").to_string(),
            "/c/config.toml:2: derive.anthropic.api.endpoint must be an https URL, or an http URL at 127.0.0.1 or [::1]"
        );
        assert_eq!(
            refused("[derive.anthropic.api]\nendpoint = \"http://localhost:9\"\n").to_string(),
            "/c/config.toml:2: derive.anthropic.api.endpoint must name the address in place of localhost: 127.0.0.1"
        );
    }

    #[test]
    fn the_version_and_the_variables_name_are_short_names() {
        for (key, good, bad) in [
            ("version", ["v", &"v".repeat(32)], [&"v".repeat(33), "a b"]),
            ("key_env", ["A_B1", "X"], ["A-B", ""]),
        ] {
            for text in good {
                let config = format!("[derive.anthropic.api]\n{key} = \"{text}\"\n");
                assert!(resolve(&config).is_ok(), "{key} {text}");
            }
            for text in bad {
                let config = format!("[derive.anthropic.api]\n{key} = \"{text}\"\n");
                let error = refused(&config);
                assert_eq!(at_line(&error), 2, "{key} {text}");
                if key == "key_env" {
                    assert_eq!(
                        error.to_string(),
                        "/c/config.toml:2: derive.anthropic.api.key_env must be the name of a variable: letters, digits and _"
                    );
                }
            }
        }
    }

    #[test]
    fn the_key_command_is_a_program_and_its_arguments() {
        let command =
            |text: &str| resolve(&format!("[derive.anthropic.api]\nkey_command = {text}\n"));
        assert!(command("[]").expect("none").key_command.value.is_empty());
        assert_eq!(command("[\"op\"]").expect("one").key_command.value, ["op"]);
        assert_eq!(
            command("[\"op\", \"read\", \"x\"]")
                .expect("three")
                .key_command
                .value,
            ["op", "read", "x"]
        );
        for bad in ["[\"\"]", "\"op read\"", "[1]"] {
            assert!(command(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_header_bilbo_writes_is_refused_whatever_its_case() {
        for name in [
            "x-api-key",
            "authorization",
            "anthropic-version",
            "content-type",
            "content-length",
            "host",
            "transfer-encoding",
            "accept-encoding",
        ] {
            for written in [name.to_owned(), name.to_ascii_uppercase()] {
                let text = format!("[derive.anthropic.api.headers]\n{written} = \"v\"\n");
                assert_eq!(
                    refused(&text).to_string(),
                    format!(
                        "/c/config.toml:1: derive.anthropic.api.headers must not hold {name}, a header bilbo writes itself"
                    ),
                    "{written}"
                );
            }
        }
        for (bad, line) in [
            (
                "\"X A\" = \"v\"",
                "/c/config.toml:1: derive.anthropic.api.headers must name its headers with letters, digits and dashes",
            ),
            (
                "X-A = \"a\\nb\"",
                "/c/config.toml:1: derive.anthropic.api.headers must hold no line end or control character",
            ),
        ] {
            let text = format!("[derive.anthropic.api.headers]\n{bad}\n");
            assert_eq!(refused(&text).to_string(), line, "{bad}");
        }
        let taken = resolve(
            "[derive.anthropic.api.headers]\nUser-Agent = \"mine\"\nanthropic-beta = \"x\"\n",
        );
        assert_eq!(taken.expect("taken").headers.value.len(), 2);
    }

    #[test]
    fn an_extra_field_bilbo_writes_or_that_offers_a_tool_is_refused() {
        for name in [
            "model",
            "max_tokens",
            "system",
            "messages",
            "stream",
            "tools",
            "tool_choice",
            "mcp_servers",
            "container",
            "thread",
        ] {
            for written in [name.to_owned(), name.to_ascii_uppercase()] {
                let text = format!("[derive.anthropic.api.extra]\nzz = 1\n{written} = 1\n");
                assert_eq!(
                    refused(&text).to_string(),
                    format!(
                        "/c/config.toml:1: derive.anthropic.api.extra must not hold {name}: a field bilbo writes itself, one that offers the model a tool, or one that keeps the records at the provider"
                    ),
                    "{written}"
                );
            }
        }
        assert_eq!(
            refused("[derive.anthropic.api.extra]\nouter = { when = 1979-05-27 }\n").to_string(),
            "/c/config.toml:1: derive.anthropic.api.extra must hold no date, inf or nan"
        );
        assert_eq!(
            refused("[derive.anthropic.api]\nextra = \"x\"\n").to_string(),
            "/c/config.toml:2: derive.anthropic.api.extra must be a table of fields"
        );
        for name in [
            "cache_control",
            "diagnostics",
            "context_management",
            "fallbacks",
            "inference_geo",
            "metadata",
            "output_config",
            "service_tier",
            "speed",
            "stop_sequences",
            "thinking",
        ] {
            let text = format!("[derive.anthropic.api.extra]\n{name} = {{ type = \"x\" }}\n");
            assert!(resolve(&text).is_ok(), "{name}");
        }
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back_and_the_way_lists_six() {
        let text = "[derive.anthropic.api]\nendpoint = \"http://127.0.0.1:9/x\"\nversion = \"2026-10-21\"\nkey_env = \"MY_KEY\"\nkey_command = [\"op\", \"read\"]\n\n[derive.anthropic.api.headers]\nX-A = \"1\"\n\n[derive.anthropic.api.extra]\noutput_config = { effort = \"low\" }\nlist = [1, 2.5, true, \"s\"]\n\"a.b\" = 3\n";
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let source = Source {
            var: &|_| None,
            file: &file,
        };
        let first = shown(&source).expect("shown");
        let names: Vec<_> = first.iter().map(|item| item.name).collect();
        assert_eq!(names, ABOUT.map(|about| about.name));
        let again: String = first
            .iter()
            .map(|item| format!("{} = {}\n", item.name, item.value))
            .collect();
        let reread = File::parse(Path::new("/c/again.toml"), again.as_bytes()).expect("reads back");
        let second = shown(&Source {
            var: &|_| None,
            file: &reread,
        })
        .expect("shown again");
        let values = |items: &[bilbo_settings::key::Shown]| -> Vec<String> {
            items.iter().map(|item| item.value.clone()).collect()
        };
        assert_eq!(values(&first), values(&second));
        assert_eq!(first[4].value, "{ X-A = \"1\" }");
        let literals: Vec<&str> = first.iter().map(|item| item.value.as_str()).collect();
        assert_eq!(
            literals,
            [
                "\"http://127.0.0.1:9/x\"",
                "\"2026-10-21\"",
                "\"MY_KEY\"",
                "[\"op\", \"read\"]",
                "{ X-A = \"1\" }",
                "{ \"a.b\" = 3, list = [1, 2.5, true, \"s\"], output_config = { effort = \"low\" } }",
            ]
        );
    }

    #[test]
    fn a_secret_put_in_the_wrong_key_is_refused_and_never_printed_back() {
        const SECRET: &str = "sk-test-PLUM-7391";
        let name = "must be the name of a variable: letters, digits and _";
        let mut refusals: Vec<(Error, String)> = Vec::new();
        refusals.push((
            refused(&format!(
                "[derive.anthropic.api]\nendpoint = \"https://u:{SECRET}@example.com\"\n"
            )),
            "/c/config.toml:2: derive.anthropic.api.endpoint must hold no user name or password, got one"
                .to_owned(),
        ));
        refusals.push((
            resolve_with(
                "",
                (
                    "BILBO_DERIVE_ANTHROPIC_API_ENDPOINT",
                    &format!("https://u:{SECRET}@example.com"),
                ),
            )
            .expect_err("refused"),
            "BILBO_DERIVE_ANTHROPIC_API_ENDPOINT must hold no user name or password, got one"
                .to_owned(),
        ));
        refusals.push((
            refused(&format!("[derive.anthropic.api.headers]\nx-api-key = \"{SECRET}\"\n")),
            "/c/config.toml:1: derive.anthropic.api.headers must not hold x-api-key, a header bilbo writes itself"
                .to_owned(),
        ));
        refusals.push((
            refused(&format!("[derive.anthropic.api]\nkey_env = \"{SECRET}\"\n")),
            format!("/c/config.toml:2: derive.anthropic.api.key_env {name}"),
        ));
        refusals.push((
            resolve_with("", ("BILBO_DERIVE_ANTHROPIC_API_KEY_ENV", SECRET)).expect_err("refused"),
            format!("BILBO_DERIVE_ANTHROPIC_API_KEY_ENV {name}"),
        ));
        for (table, key) in [
            ("derive.anthropic", "model"),
            ("derive.anthropic.api", "version"),
        ] {
            let reason = if key == "model" {
                MODEL_REASON
            } else {
                "must be 1 to 32 letters, digits and dashes"
            };
            refusals.push((
                refused(&format!("[{table}]\n{key} = \"{SECRET} x\"\n")),
                format!("/c/config.toml:2: {table}.{key} {reason}"),
            ));
        }
        for (error, line) in refusals {
            assert_eq!(error.to_string(), line);
            assert!(!error.to_string().contains("PLUM"));
        }
    }
}
