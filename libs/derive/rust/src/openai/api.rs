use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use bilbo_settings::error::Error;
use bilbo_settings::key::{About, Default, Key, Resolved, Shown, Source, quote};
use serde_json::{Map, Value as Json, json};

use crate::api::{self, Call};
use crate::child::Bounds;
use crate::http::{self, Endpoint, Response};
use crate::secret::Secret;
use crate::table;
use crate::writer::{
    Cause, Limits, MODEL_ALLOWED, Outcome, Prompt, Usage, Variables, What, Writer, answered_model,
    identifier,
};

pub const NAME: &str = "openai";
pub const SUMMARY: &str = "the OpenAI API, with a key";

const HEADERS_BILBO_WRITES: [&str; 6] = [
    "authorization",
    "content-type",
    "content-length",
    "host",
    "transfer-encoding",
    "accept-encoding",
];
const FIELDS_BILBO_WRITES: [&str; 14] = [
    "model",
    "instructions",
    "input",
    "max_output_tokens",
    "store",
    "stream",
    "background",
    "background_options",
    "previous_response_id",
    "conversation",
    "prompt",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
];
const SPENT: [&str; 4] = [
    "credit_balance_exhausted",
    "organization_spend_limit_exceeded",
    "project_spend_limit_exceeded",
    "organization_usage_limit_exceeded",
];
const ITEMS_PASSED_OVER: [&str; 2] = ["reasoning", "compaction"];
const ITEMS_OF_A_TOOL: [&str; 25] = [
    "function_call",
    "function_call_output",
    "file_search_call",
    "web_search_call",
    "computer_call",
    "computer_call_output",
    "code_interpreter_call",
    "image_generation_call",
    "local_shell_call",
    "local_shell_call_output",
    "shell_call",
    "shell_call_output",
    "apply_patch_call",
    "apply_patch_call_output",
    "mcp_call",
    "mcp_list_tools",
    "mcp_approval_request",
    "mcp_approval_response",
    "custom_tool_call",
    "custom_tool_call_output",
    "tool_search_call",
    "tool_search_output",
    "additional_tools",
    "program",
    "program_output",
];

pub const MODEL: Key<String> = Key {
    about: About {
        name: "derive.openai.model",
        variable: Some("BILBO_DERIVE_OPENAI_MODEL"),
        default: Default::Toml(r#""""#),
        allowed: MODEL_ALLOWED,
        doc: "The model bilbo derive asks the OpenAI API for. bilbo names none.",
    },
    parse: api::model,
    show: |id| quote(id),
};

pub const ENDPOINT: Key<Endpoint> = Key {
    about: About {
        name: "derive.openai.endpoint",
        variable: Some("BILBO_DERIVE_OPENAI_ENDPOINT"),
        default: Default::Toml(r#""https://api.openai.com/v1/responses""#),
        allowed: api::ENDPOINT_ALLOWED,
        doc: "Where bilbo derive sends a request for the OpenAI API.",
    },
    parse: api::endpoint,
    show: |endpoint| quote(endpoint.as_str()),
};

pub const KEY_ENV: Key<String> = Key {
    about: About {
        name: "derive.openai.key_env",
        variable: Some("BILBO_DERIVE_OPENAI_KEY_ENV"),
        default: Default::Toml(r#""OPENAI_API_KEY""#),
        allowed: api::KEY_ENV_ALLOWED,
        doc: "The variable bilbo derive reads the OpenAI API key from when key_command is empty.",
    },
    parse: api::key_env,
    show: |name| quote(name),
};

pub const KEY_COMMAND: Key<Vec<String>> = Key {
    about: About {
        name: "derive.openai.key_command",
        variable: None,
        default: Default::Toml("[]"),
        allowed: api::KEY_COMMAND_ALLOWED,
        doc: "A program and its arguments whose first line of standard output is the OpenAI API key. When it is set bilbo derive runs it and reads no variable.",
    },
    parse: api::key_command,
    show: |command| table::show_list(command),
};

pub const HEADERS: Key<Vec<(String, String)>> = Key {
    about: About {
        name: "derive.openai.headers",
        variable: None,
        default: Default::Toml("{}"),
        allowed: api::HEADERS_ALLOWED,
        doc: "Headers added to each request as they are. A secret never goes here: bilbo config prints every value.",
    },
    parse: |value| table::headers(value, &HEADERS_BILBO_WRITES),
    show: |headers| table::show_texts(headers),
};

pub const EXTRA: Key<Map<String, Json>> = Key {
    about: About {
        name: "derive.openai.extra",
        variable: None,
        default: Default::Toml("{}"),
        allowed: api::EXTRA_ALLOWED,
        doc: "Fields added to the body of each request as they are.",
    },
    parse: |value| table::fields(value, &FIELDS_BILBO_WRITES),
    show: |map| table::show_fields(map),
};

pub const ABOUT: [&About; 6] = [
    &MODEL.about,
    &ENDPOINT.about,
    &KEY_ENV.about,
    &KEY_COMMAND.about,
    &HEADERS.about,
    &EXTRA.about,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub model: Resolved<String>,
    pub endpoint: Resolved<Endpoint>,
    pub key_env: Resolved<String>,
    pub key_command: Resolved<Vec<String>>,
    pub headers: Resolved<Vec<(String, String)>>,
    pub extra: Resolved<Map<String, Json>>,
}

pub fn section(source: &Source<'_>) -> Result<Section, Error> {
    Ok(Section {
        model: MODEL.resolve(source)?,
        endpoint: ENDPOINT.resolve(source)?,
        key_env: KEY_ENV.resolve(source)?,
        key_command: KEY_COMMAND.resolve(source)?,
        headers: HEADERS.resolve(source)?,
        extra: EXTRA.resolve(source)?,
    })
}

pub fn shown(source: &Source<'_>) -> Result<Vec<Shown>, Error> {
    Ok(section(source)?.shown().into())
}

impl Section {
    pub fn shown(&self) -> [Shown; 6] {
        [
            MODEL.shown(&self.model),
            ENDPOINT.shown(&self.endpoint),
            KEY_ENV.shown(&self.key_env),
            KEY_COMMAND.shown(&self.key_command),
            HEADERS.shown(&self.headers),
            EXTRA.shown(&self.extra),
        ]
    }
}

pub struct OpenAi {
    section: Section,
    variables: Variables,
    folder: PathBuf,
    bounds: Bounds,
    secret: Option<Secret>,
}

impl fmt::Debug for OpenAi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAi")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

impl OpenAi {
    pub fn new(
        section: Section,
        variables: Variables,
        folder: PathBuf,
        timeout: Duration,
        limit: usize,
        poll: Duration,
    ) -> OpenAi {
        OpenAi {
            section,
            variables,
            folder,
            bounds: Bounds {
                limit,
                timeout,
                poll,
            },
            secret: None,
        }
    }

    fn body(&self, prompt: &Prompt, limits: &Limits) -> Vec<u8> {
        let mut fields = self.section.extra.value.clone();
        fields.insert("input".to_owned(), json!(prompt.material));
        fields.insert("instructions".to_owned(), json!(prompt.instruction));
        fields.insert("max_output_tokens".to_owned(), json!(limits.answer_tokens));
        fields.insert("model".to_owned(), json!(self.section.model.value));
        // Left out, the vendor stores the response.
        fields.insert("store".to_owned(), json!(false));
        Json::Object(fields).to_string().into_bytes()
    }
}

fn cause(
    what: What,
    status: u16,
    kind: Option<&str>,
    code: Option<&str>,
    id: Option<&str>,
) -> Cause {
    Cause {
        what,
        status: Some(i32::from(status)),
        kind: kind.map(identifier),
        code: code.map(identifier),
        request: id.map(identifier),
    }
}

struct Read {
    outcome: Outcome,
    texts: Vec<String>,
}

fn clean<'t>(key: &Secret, text: Option<&'t str>) -> Option<&'t str> {
    text.filter(|said| !key.hides(&[said]))
}

fn usage_of(body: &Json) -> Usage {
    let counts = body.get("usage");
    let count = |name: &str| {
        counts
            .and_then(|usage| usage.get(name))
            .and_then(Json::as_u64)
    };
    Usage {
        read: count("input_tokens"),
        written: count("output_tokens"),
    }
}

fn read(key: &Secret, response: &Response) -> Read {
    let id = response.header("x-request-id");
    let body: Option<Json> = serde_json::from_slice(&response.body).ok();
    let error = body.as_ref().and_then(|body| body.get("error"));
    let kind = error
        .and_then(|error| error.get("type"))
        .and_then(Json::as_str);
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(Json::as_str);
    let status = response.status;
    let with = |what: What| {
        cause(
            what,
            status,
            clean(key, kind),
            clean(key, code),
            clean(key, id),
        )
    };
    let busy = |what: What| Outcome::Busy {
        wait: response.wait(),
        cause: with(what),
    };
    let spent =
        code.is_some_and(|code| SPENT.contains(&code)) || kind == Some("insufficient_quota");
    let mut texts: Vec<String> = [kind, code, id]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect();
    let outcome = match status {
        200 => {
            let (outcome, more) = completed(body.as_ref(), response, id);
            texts.extend(more);
            outcome
        }
        429 if spent => Outcome::Stopped(with(What::Billing)),
        429 => busy(What::RateLimit),
        408 => busy(What::Timeout),
        503 => busy(What::Overloaded),
        409 | 500..=502 | 504.. => busy(What::ServerError),
        401 => Outcome::Stopped(with(What::Unauthorized)),
        403 => Outcome::Stopped(with(What::Forbidden)),
        404 => Outcome::Stopped(with(What::NotFound)),
        400..=499 => Outcome::Stopped(with(What::Rejected)),
        _ => Outcome::Failed {
            cause: with(What::Unreadable),
            usage: Usage::default(),
        },
    };
    Read { outcome, texts }
}

fn completed(body: Option<&Json>, response: &Response, id: Option<&str>) -> (Outcome, Vec<String>) {
    let unreadable = |usage: Usage| Outcome::Failed {
        cause: cause(What::Unreadable, 200, None, None, id),
        usage,
    };
    let Some(body) = body.filter(|body| body.is_object()) else {
        return (unreadable(Usage::default()), Vec::new());
    };
    let usage = usage_of(body);
    let Some(status) = body.get("status").and_then(Json::as_str) else {
        return (unreadable(usage), Vec::new());
    };
    let mut texts = vec![status.to_owned()];
    let error = body.get("error");
    let kind = error
        .and_then(|error| error.get("type"))
        .and_then(Json::as_str);
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(Json::as_str);
    let failed_by = |what: What, code: Option<&str>| Outcome::Failed {
        cause: cause(what, response.status, kind, code, id),
        usage,
    };
    let outcome = match status {
        "completed" => {
            let (outcome, more) = answered(body, usage, id);
            texts.extend(more);
            outcome
        }
        "incomplete" => {
            let reason = body
                .get("incomplete_details")
                .and_then(|details| details.get("reason"))
                .and_then(Json::as_str);
            texts.extend(reason.map(str::to_owned));
            match reason {
                Some("max_output_tokens") => Outcome::Cut(usage),
                Some("content_filter") => Outcome::Refused {
                    category: Some("content_filter".to_owned()),
                    usage,
                },
                other => failed_by(What::StopReason, other),
            }
        }
        "failed" => {
            texts.extend(code.map(str::to_owned));
            match code {
                Some("server_error") => Outcome::Busy {
                    wait: response.wait(),
                    cause: cause(What::ServerError, 200, kind, code, id),
                },
                Some("rate_limit_exceeded") => Outcome::Busy {
                    wait: response.wait(),
                    cause: cause(What::RateLimit, 200, kind, code, id),
                },
                Some(policy @ ("bio_policy" | "misalignment_policy_violation")) => {
                    Outcome::Refused {
                        category: Some(identifier(policy)),
                        usage,
                    }
                }
                Some("data_residency_mismatch") => {
                    Outcome::Stopped(cause(What::Forbidden, 200, kind, code, id))
                }
                other => failed_by(What::StopReason, other),
            }
        }
        other => failed_by(What::StopReason, Some(other)),
    };
    (outcome, texts)
}

fn kind_of(item: &Json) -> Option<&str> {
    item.get("type").and_then(Json::as_str)
}

fn answered(body: &Json, usage: Usage, id: Option<&str>) -> (Outcome, Vec<String>) {
    let failed = |what: What| Outcome::Failed {
        cause: cause(what, 200, None, None, id),
        usage,
    };
    let items = body
        .get("output")
        .and_then(Json::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut text = String::new();
    let mut heard = Vec::new();
    let mut spoke = false;
    let mut refused = false;
    let mut tool = false;
    let mut unreadable = false;
    for item in items {
        match kind_of(item) {
            Some("message") => {
                let Some(parts) = item.get("content").and_then(Json::as_array) else {
                    unreadable = true;
                    continue;
                };
                for part in parts {
                    match kind_of(part) {
                        Some("output_text") => match part.get("text").and_then(Json::as_str) {
                            Some(said) => {
                                spoke = true;
                                text.push_str(said);
                                heard.push(said.to_owned());
                            }
                            None => unreadable = true,
                        },
                        Some("refusal") => refused = true,
                        _ => unreadable = true,
                    }
                }
            }
            Some(kind) if ITEMS_PASSED_OVER.contains(&kind) => {}
            Some(kind) if ITEMS_OF_A_TOOL.contains(&kind) => tool = true,
            _ => unreadable = true,
        }
    }
    let model = body.get("model").and_then(Json::as_str);
    heard.extend(model.map(str::to_owned));
    heard.push(text.clone());
    let outcome = if tool {
        failed(What::ToolUsed)
    } else if unreadable {
        failed(What::Unreadable)
    } else if refused {
        Outcome::Refused {
            category: None,
            usage,
        }
    } else if !spoke {
        failed(What::Unreadable)
    } else {
        Outcome::Answer {
            text,
            usage,
            model: model.map(answered_model),
        }
    };
    (outcome, heard)
}

impl Writer for OpenAi {
    fn prepare(&mut self) -> Result<(), String> {
        let section = &self.section;
        let extra = table::texts(&section.extra.value);
        let call = Call {
            provider: NAME,
            env: &section.key_env.value,
            command: &section.key_command.value,
            variables: &*self.variables,
            folder: &self.folder,
            bounds: self.bounds,
        };
        let found = api::prepare(
            &call,
            &section.model.value,
            &[],
            &section.headers.value,
            &extra,
        )?;
        self.secret = Some(found);
        Ok(())
    }

    fn write(&mut self, prompt: &Prompt, limits: &Limits) -> Outcome {
        let Some(secret) = &self.secret else {
            return Outcome::Stopped(Cause::new(What::NotStarted));
        };
        let authorization = format!("Bearer {}", secret.expose());
        let mut headers: Vec<(&str, &str)> = vec![
            ("content-type", "application/json"),
            ("authorization", &authorization),
        ];
        headers.extend(
            self.section
                .headers
                .value
                .iter()
                .map(|(name, text)| (name.as_str(), text.as_str())),
        );
        let sent = http::post(
            &self.section.endpoint.value,
            &headers,
            &self.body(prompt, limits),
            &["x-request-id", "retry-after"],
            limits.timeout,
            limits.response,
        );
        let response = match sent {
            Ok(response) => response,
            Err(fault) => return api::fault(&fault),
        };
        let found = read(secret, &response);
        let texts: Vec<&str> = found.texts.iter().map(String::as_str).collect();
        let provider = match &found.outcome {
            Outcome::Busy { .. } => response.status != 200,
            Outcome::Stopped(cause) => response.status != 200 && cause.what != What::Rejected,
            _ => false,
        };
        if provider {
            return found.outcome;
        }
        api::holds_key(secret, &texts, response.status).unwrap_or(found.outcome)
    }

    fn model(&self) -> &str {
        &self.section.model.value
    }

    fn describe(&self) -> Vec<String> {
        let section = &self.section;
        let head = vec![format!(
            "provider: {NAME}, model {}, at {}",
            section.model.value,
            section.endpoint.value.as_str()
        )];
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;
    use serde_json::json;

    use super::{ABOUT, Section, section, shown};

    const MODEL_REASON: &str = "must be a model's id of at most 200 characters with no space or control character, or the empty text for none";

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

    fn given(text: &str) -> Section {
        resolve(&[], text).expect("resolves")
    }

    fn refused(text: &str) -> String {
        resolve(&[], text).expect_err("refused")
    }

    #[test]
    fn each_default_is_its_documented_value() {
        let section = given("");
        assert_eq!(section.model.value, "");
        assert_eq!(
            section.endpoint.value.as_str(),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(section.key_env.value, "OPENAI_API_KEY");
        assert!(section.key_command.value.is_empty());
        assert!(section.headers.value.is_empty());
        assert!(section.extra.value.is_empty());
        let variables: Vec<_> = ABOUT.iter().map(|about| about.variable).collect();
        assert_eq!(
            variables,
            [
                Some("BILBO_DERIVE_OPENAI_MODEL"),
                Some("BILBO_DERIVE_OPENAI_ENDPOINT"),
                Some("BILBO_DERIVE_OPENAI_KEY_ENV"),
                None,
                None,
                None
            ]
        );
    }

    #[test]
    fn the_model_is_a_text_with_no_space() {
        for model in ["", "gpt-test-1", "vendor/test-model:2", &"é".repeat(200)] {
            assert_eq!(
                given(&format!("derive.openai.model = {model:?}"))
                    .model
                    .value,
                model
            );
        }
        for model in [&"m".repeat(201)[..], &"é".repeat(201), "a b", "a\tb"] {
            assert_eq!(
                refused(&format!("derive.openai.model = {model:?}")),
                format!("/c/config.toml:1: derive.openai.model {MODEL_REASON}")
            );
        }
        assert_eq!(
            refused("derive.openai.model = \"a\\u0007b\""),
            format!("/c/config.toml:1: derive.openai.model {MODEL_REASON}")
        );
    }

    #[test]
    fn the_endpoint_is_https_or_plain_http_at_a_literal_loopback_address() {
        let local = given("derive.openai.endpoint = \"http://127.0.0.1:9/v1/responses\"");
        assert_eq!(
            local.endpoint.value.as_str(),
            "http://127.0.0.1:9/v1/responses"
        );
        assert_eq!(
            refused("derive.openai.endpoint = \"http://example.com\""),
            "/c/config.toml:1: derive.openai.endpoint must be an https URL, or an http URL at 127.0.0.1 or [::1]"
        );
        assert_eq!(
            refused("derive.openai.endpoint = \"http://localhost:9\""),
            "/c/config.toml:1: derive.openai.endpoint must name the address in place of localhost: 127.0.0.1"
        );
    }

    #[test]
    fn the_variables_name_is_a_short_name() {
        assert_eq!(
            given("derive.openai.key_env = \"A_B1\"").key_env.value,
            "A_B1"
        );
        for name in ["A-B", ""] {
            assert_eq!(
                refused(&format!("derive.openai.key_env = {name:?}")),
                "/c/config.toml:1: derive.openai.key_env must be the name of a variable: letters, digits and _"
            );
        }
    }

    #[test]
    fn the_key_command_is_a_program_and_its_arguments() {
        assert!(
            given("derive.openai.key_command = []")
                .key_command
                .value
                .is_empty()
        );
        assert_eq!(
            given("derive.openai.key_command = [\"op\"]")
                .key_command
                .value,
            ["op"]
        );
        assert_eq!(
            given("derive.openai.key_command = [\"op\", \"read\", \"x\"]")
                .key_command
                .value,
            ["op", "read", "x"]
        );
        for text in ["[\"\", \"x\"]", "\"op\"", "[1]"] {
            let message = refused(&format!("derive.openai.key_command = {text}"));
            assert!(message.contains("derive.openai.key_command"), "{message}");
        }
    }

    #[test]
    fn a_header_bilbo_writes_is_refused_whatever_its_case() {
        for name in [
            "authorization",
            "content-type",
            "content-length",
            "host",
            "transfer-encoding",
            "accept-encoding",
        ] {
            for written in [name.to_owned(), name.to_ascii_uppercase()] {
                assert_eq!(
                    refused(&format!("derive.openai.headers = {{ {written} = \"x\" }}")),
                    format!(
                        "/c/config.toml:1: derive.openai.headers must not hold {name}, a header bilbo writes itself"
                    )
                );
            }
        }
        assert_eq!(
            refused("derive.openai.headers = { \"a b\" = \"x\" }"),
            "/c/config.toml:1: derive.openai.headers must name its headers with letters, digits and dashes"
        );
        assert_eq!(
            refused("derive.openai.headers = { A = \"x\\ny\" }"),
            "/c/config.toml:1: derive.openai.headers must hold no line end or control character"
        );
        let taken = given("derive.openai.headers = { User-Agent = \"x\", OpenAI-Project = \"p\" }");
        assert_eq!(
            taken.headers.value,
            [
                ("User-Agent".to_owned(), "x".to_owned()),
                ("OpenAI-Project".to_owned(), "p".to_owned())
            ]
        );
    }

    #[test]
    fn an_extra_field_bilbo_writes_or_that_offers_a_tool_is_refused() {
        for name in [
            "model",
            "instructions",
            "input",
            "max_output_tokens",
            "store",
            "stream",
            "background",
            "background_options",
            "previous_response_id",
            "conversation",
            "prompt",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
        ] {
            for written in [name.to_owned(), name.to_ascii_uppercase()] {
                assert_eq!(
                    refused(&format!("derive.openai.extra = {{ {written} = \"x\" }}")),
                    format!(
                        "/c/config.toml:1: derive.openai.extra must not hold {name}: a field bilbo writes itself, one that offers the model a tool, or one that keeps the records at the provider"
                    )
                );
            }
        }
        assert_eq!(
            refused("derive.openai.extra = { outer = { when = 1979-05-27 } }"),
            "/c/config.toml:1: derive.openai.extra must hold no date, inf or nan"
        );
        assert_eq!(
            refused("derive.openai.extra = \"x\""),
            "/c/config.toml:1: derive.openai.extra must be a table of fields"
        );
        for name in [
            "prompt_cache_key",
            "prompt_cache_options",
            "prompt_cache_retention",
            "access_programs",
            "context_management",
            "image",
            "include",
            "max_tool_calls",
            "metadata",
            "moderation",
            "reasoning",
            "safety_identifier",
            "service_tier",
            "stream_options",
            "temperature",
            "text",
            "top_logprobs",
            "top_p",
            "truncation",
            "user",
        ] {
            assert!(
                resolve(&[], &format!("derive.openai.extra = {{ {name} = 1 }}")).is_ok(),
                "{name}"
            );
        }
        let taken = given("derive.openai.extra = { reasoning = { effort = \"low\" } }");
        assert_eq!(
            serde_json::Value::Object(taken.extra.value),
            json!({"reasoning": {"effort": "low"}})
        );
    }

    #[test]
    fn the_keys_show_as_toml_that_reads_back_and_the_group_lists_six() {
        let text = "derive.openai.model = \"m\"\nderive.openai.endpoint = \"http://127.0.0.1:9/x\"\nderive.openai.key_env = \"K1\"\nderive.openai.key_command = [\"op\", \"read\"]\nderive.openai.headers = { OpenAI-Project = \"p\" }\nderive.openai.extra = { reasoning = { effort = \"low\" }, n = 2, list = [1, \"a\"], \"a.b\" = 1 }\n";
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let var = |_: &str| None;
        let list = shown(&Source {
            var: &var,
            file: &file,
        })
        .expect("shown");
        let names: Vec<&str> = list.iter().map(|shown| shown.name).collect();
        assert_eq!(names, ABOUT.map(|about| about.name));
        let values: Vec<&str> = list.iter().map(|shown| shown.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "\"m\"",
                "\"http://127.0.0.1:9/x\"",
                "\"K1\"",
                "[\"op\", \"read\"]",
                "{ OpenAI-Project = \"p\" }",
                "{ \"a.b\" = 1, list = [1, \"a\"], n = 2, reasoning = { effort = \"low\" } }",
            ]
        );
        let again: String = list
            .iter()
            .map(|shown| format!("{} = {}\n", shown.name, shown.value))
            .collect();
        let before = given(text);
        let after = given(&again);
        assert_eq!(before.extra.value, after.extra.value);
        assert_eq!(before.headers.value, after.headers.value);
        assert_eq!(before.key_command.value, after.key_command.value);
        assert_eq!(before.endpoint.value, after.endpoint.value);
    }

    #[test]
    fn a_secret_put_in_the_wrong_key_is_refused_and_never_printed_back() {
        const SECRET: &str = "sk-test-PLUM-7391";
        let name = "must be the name of a variable: letters, digits and _";
        let endpoint = "must hold no user name or password, got one";
        let mut cases: Vec<(String, String)> = Vec::new();
        cases.push((
            refused(&format!(
                "derive.openai.endpoint = \"https://u:{SECRET}@example.com\""
            )),
            format!("/c/config.toml:1: derive.openai.endpoint {endpoint}"),
        ));
        cases.push((
            resolve(
                &[(
                    "BILBO_DERIVE_OPENAI_ENDPOINT",
                    &format!("https://u:{SECRET}@example.com"),
                )],
                "",
            )
            .expect_err("refused"),
            format!("BILBO_DERIVE_OPENAI_ENDPOINT {endpoint}"),
        ));
        cases.push((
            refused(&format!("derive.openai.endpoint = [\"{SECRET}\"]")),
            "/c/config.toml:1: derive.openai.endpoint must be an https URL, or an http URL at 127.0.0.1 or [::1]"
                .to_owned(),
        ));
        cases.push((
            refused(&format!(
                "derive.openai.headers = {{ authorization = \"{SECRET}\" }}"
            )),
            "/c/config.toml:1: derive.openai.headers must not hold authorization, a header bilbo writes itself"
                .to_owned(),
        ));
        cases.push((
            refused(&format!("derive.openai.key_env = \"{SECRET}\"")),
            format!("/c/config.toml:1: derive.openai.key_env {name}"),
        ));
        cases.push((
            resolve(&[("BILBO_DERIVE_OPENAI_KEY_ENV", SECRET)], "").expect_err("refused"),
            format!("BILBO_DERIVE_OPENAI_KEY_ENV {name}"),
        ));
        cases.push((
            refused(&format!("derive.openai.model = \"{SECRET} x\"")),
            format!("/c/config.toml:1: derive.openai.model {MODEL_REASON}"),
        ));
        for (message, line) in cases {
            assert_eq!(message, line);
            assert!(!message.contains("PLUM"));
        }
    }
}
