use std::fmt;
use std::path::Path;

use bilbo_settings::error::Error;
use bilbo_settings::key::Value;

use crate::child::Bounds;
use crate::http::{self, Endpoint, Fault};
use crate::model::{Cause, MODEL_REFUSED, Outcome, Usage, What, is_model_name};
use crate::secret::{self, Secret};
use crate::table;

pub const ENDPOINT_ALLOWED: &str = "an https URL, or an http URL at 127.0.0.1 or [::1]";
pub const KEY_ENV_ALLOWED: &str = "the name of a variable: letters, digits and _";
pub const KEY_COMMAND_ALLOWED: &str = "a list of texts, the first a program and not empty";
pub const HEADERS_ALLOWED: &str = "a table of texts, none of them a header bilbo writes itself";
pub const EXTRA_ALLOWED: &str = "a table of fields for the request's body; no field bilbo writes itself, none that offers the model a tool, none that keeps the records at the provider";

pub fn model(value: &Value<'_>) -> Result<String, Error> {
    value
        .text()
        .filter(|id| is_model_name(id))
        .map(str::to_owned)
        .ok_or_else(|| value.refused(MODEL_REFUSED))
}

pub fn endpoint(value: &Value<'_>) -> Result<Endpoint, Error> {
    let text = value
        .text()
        .ok_or_else(|| value.refused(&http::Refusal::Form.to_string()))?;
    http::endpoint(text).map_err(|refusal| value.refused(&refusal.to_string()))
}

pub fn key_env(value: &Value<'_>) -> Result<String, Error> {
    value
        .text()
        .filter(|text| {
            !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .map(str::to_owned)
        .ok_or_else(|| value.refused("must be the name of a variable: letters, digits and _"))
}

pub fn key_command(value: &Value<'_>) -> Result<Vec<String>, Error> {
    let items = value.items().ok_or_else(|| value.refuse())?;
    let texts = items
        .iter()
        .map(|item| item.text().map(str::to_owned).ok_or_else(|| item.refuse()))
        .collect::<Result<Vec<_>, _>>()?;
    match texts.first() {
        Some(first) if first.is_empty() => Err(items[0].refuse()),
        _ => Ok(texts),
    }
}

pub struct Call<'a> {
    pub provider: &'a str,
    pub env: &'a str,
    pub command: &'a [String],
    pub variables: &'a dyn Fn(&str) -> Option<String>,
    pub folder: &'a Path,
    pub bounds: Bounds,
}

impl fmt::Debug for Call<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call")
            .field("provider", &self.provider)
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

fn key(call: &Call<'_>, guarded: &[(&str, Vec<&str>)]) -> Result<Secret, String> {
    let name = if call.command.is_empty() {
        "api.key_env"
    } else {
        "api.key_command"
    };
    let found = secret::read(
        secret::Source::choose(call.env, call.command),
        &format!("derive.{}.{name}", call.provider),
        call.variables,
        call.folder,
        call.bounds,
    )?;
    for (key, texts) in guarded {
        if found.hides(texts) {
            return Err(format!("derive.{}.{key} holds the key", call.provider));
        }
    }
    Ok(found)
}

pub fn prepare(
    call: &Call<'_>,
    model: &str,
    ahead: &[(&str, &str)],
    headers: &[(String, String)],
    extra: &[String],
) -> Result<Secret, String> {
    if model.is_empty() {
        return Err(format!("derive.{}.model is not set", call.provider));
    }
    let named: Vec<&str> = headers
        .iter()
        .flat_map(|(name, text)| [name.as_str(), text.as_str()])
        .collect();
    let mut guarded: Vec<(&str, Vec<&str>)> = vec![("model", vec![model])];
    guarded.extend(ahead.iter().map(|(name, text)| (*name, vec![*text])));
    guarded.push(("api.headers", named));
    guarded.push(("api.extra", extra.iter().map(String::as_str).collect()));
    key(call, &guarded)
}

pub fn key_line(
    env: &str,
    command: &[String],
    variables: &dyn Fn(&str) -> Option<String>,
) -> String {
    if command.is_empty() {
        let set = variables(env).is_some_and(|value| !value.trim_ascii().is_empty());
        let state = if set { "is set" } else { "is not set" };
        format!("key: the variable {env}, which {state}")
    } else {
        format!("key: the program {}, not run", table::show_list(command))
    }
}

pub fn describe(
    mut head: Vec<String>,
    headers: &[(String, String)],
    extra: Option<String>,
    key: String,
) -> Vec<String> {
    if !headers.is_empty() {
        head.push(format!("headers: {}", table::show_texts(headers)));
    }
    head.extend(extra.map(|shown| format!("extra: {shown}")));
    head.push(key);
    head
}

pub fn fault(fault: &Fault) -> Outcome {
    match fault {
        Fault::Timeout => Outcome::Busy {
            wait: None,
            cause: Cause::new(What::Timeout),
        },
        Fault::NoConnection => Outcome::Busy {
            wait: None,
            cause: Cause::new(What::NoConnection),
        },
        Fault::Untrusted => Outcome::Stopped(Cause::new(What::Tls)),
        Fault::TooLarge => Outcome::Failed {
            cause: Cause::new(What::TooLarge),
            usage: Usage::default(),
        },
    }
}

pub fn holds_key(key: &Secret, texts: &[&str], status: u16) -> Option<Outcome> {
    key.hides(texts).then(|| Outcome::Failed {
        cause: Cause {
            status: Some(i32::from(status)),
            ..Cause::new(What::HoldsKey)
        },
        usage: Usage::default(),
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use bilbo_settings::error::Error;
    use bilbo_settings::file::File;
    use bilbo_settings::key::{About, Default, Key, Source, Value};

    use crate::table;

    const SECRET: &str = "sk-test-PLUM-7391";
    const ENDPOINT: &str = "must be an https URL, or an http URL at 127.0.0.1 or [::1]";
    const USER_NAME: &str = "must hold no user name or password, got one";
    const LOCALHOST: &str = "must name the address in place of localhost: 127.0.0.1";
    const MODEL: &str = "must be a model's id of at most 200 characters with no space or control character, or the empty text for none";
    const NAME: &str = "must be the name of a variable: letters, digits and _";

    fn own_headers(value: &Value<'_>) -> Result<Vec<(String, String)>, Error> {
        table::headers(value, &["authorization", "x-api-key"])
    }

    fn own_fields(value: &Value<'_>) -> Result<(), Error> {
        table::fields(value, &["model", "tools"]).map(|_| ())
    }

    fn refusal<T>(
        parse: fn(&Value<'_>) -> Result<T, Error>,
        literal: &str,
        text: Option<&str>,
    ) -> String {
        let key = Key {
            about: About {
                name: "derive.test.key",
                variable: Some("BILBO_TEST_KEY"),
                default: Default::Toml(r#""""#),
                allowed: "anything",
                doc: "A key of the test's own.",
            },
            parse,
            show: |_| String::new(),
        };
        let config = format!("[derive.test]\nkey = {literal}\n");
        let file = File::parse(Path::new("/c/config.toml"), config.as_bytes()).expect("toml");
        let given = text.map(OsString::from);
        let var = |name: &str| given.clone().filter(|_| name == "BILBO_TEST_KEY");
        let source = Source {
            var: &var,
            file: &file,
        };
        match key.resolve(&source) {
            Ok(_) => panic!("{literal} {text:?} was taken"),
            Err(error) => {
                assert!(
                    matches!(error, Error::Refused { .. }),
                    "{literal}: {error:?}"
                );
                error.to_string()
            }
        }
    }

    fn line(origin: &str, reason: &str) -> String {
        format!("{origin} {reason}")
    }

    #[test]
    fn the_keys_that_may_hold_a_secret_are_refused_without_their_value() {
        let at = "/c/config.toml:2: derive.test.key";
        let mut walked: Vec<(String, String)> = Vec::new();
        let mut check = |found: String, reason: &str, origin: &str| {
            assert!(!found.contains("PLUM"), "{found}");
            walked.push((found, line(origin, reason)));
        };
        for literal in [
            format!("\"https://u:{SECRET}@example.com\""),
            format!("\"ftp://u:{SECRET}@example.com\""),
            format!("\"//u:{SECRET}@example.com\""),
        ] {
            check(refusal(super::endpoint, &literal, None), USER_NAME, at);
        }
        check(
            refusal(
                super::endpoint,
                &format!("\"http://localhost:9/?k={SECRET}\""),
                None,
            ),
            LOCALHOST,
            at,
        );
        for literal in [
            format!("[\"{SECRET}\"]"),
            format!("\"ftp://example.com/{SECRET}\""),
            format!("\"http://example.com/{SECRET}\""),
        ] {
            check(refusal(super::endpoint, &literal, None), ENDPOINT, at);
        }
        for literal in [
            "\"https://bob:sk-test?PLUM-7391@gw.example/v1\"".to_owned(),
            "\"https://bob:sk-test/PLUM-7391@gw.example/v1\"".to_owned(),
            format!("\"https:/u:{SECRET}@example.com\""),
            format!("\"u:{SECRET}@example.com/v1?r=https://x\""),
        ] {
            check(refusal(super::endpoint, &literal, None), USER_NAME, at);
        }
        check(
            refusal(
                super::endpoint,
                "\"https://example.com\"",
                Some(&format!("https://u:{SECRET}@example.com")),
            ),
            USER_NAME,
            "BILBO_TEST_KEY",
        );
        check(
            refusal(own_headers, &format!("\"{SECRET}\""), None),
            "must be a table of texts",
            at,
        );
        check(
            refusal(own_headers, &format!("{{ {SECRET} = 7 }}"), None),
            "must hold texts alone",
            at,
        );
        check(
            refusal(own_headers, &format!("{{ \"{SECRET} x\" = \"v\" }}"), None),
            "must name its headers with letters, digits and dashes",
            at,
        );
        for value in [format!("{SECRET}\\nx"), format!("{SECRET}\\u0001x")] {
            check(
                refusal(own_headers, &format!("{{ x-trace = \"{value}\" }}"), None),
                "must hold no line end or control character",
                at,
            );
        }
        for name in ["authorization", "x-api-key"] {
            check(
                refusal(own_headers, &format!("{{ {name} = \"{SECRET}\" }}"), None),
                &format!("must not hold {name}, a header bilbo writes itself"),
                at,
            );
        }
        check(
            refusal(super::key_env, &format!("\"{SECRET}\""), None),
            NAME,
            at,
        );
        check(
            refusal(super::key_env, &format!("[\"{SECRET}\"]"), None),
            NAME,
            at,
        );
        check(
            refusal(super::key_env, "\"A\"", Some(SECRET)),
            NAME,
            "BILBO_TEST_KEY",
        );
        for literal in [
            format!("\"{SECRET} x\""),
            format!("\"{SECRET}\\u0007x\""),
            format!("[\"{SECRET}\"]"),
        ] {
            check(refusal(super::model, &literal, None), MODEL, at);
        }
        check(
            refusal(own_fields, &format!("\"{SECRET}\""), None),
            "must be a table of fields",
            at,
        );
        check(
            refusal(own_fields, &format!("{{ {SECRET} = 1979-05-27 }}"), None),
            "must hold no date, inf or nan",
            at,
        );
        check(
            refusal(own_fields, &format!("{{ model = \"{SECRET}\" }}"), None),
            "must not hold model: a field bilbo writes itself, one that offers the model a tool, or one that keeps the records at the provider",
            "/c/config.toml:2: derive.test.key",
        );
        for (found, wanted) in walked {
            assert_eq!(found, wanted);
        }
    }

    #[test]
    fn a_key_variable_of_white_space_is_not_set() {
        let blank = |_: &str| Some("  \t".to_owned());
        assert_eq!(
            super::key_line("MY_KEY", &[], &blank),
            "key: the variable MY_KEY, which is not set"
        );
        let set = |_: &str| Some(" sk ".to_owned());
        assert_eq!(
            super::key_line("MY_KEY", &[], &set),
            "key: the variable MY_KEY, which is set"
        );
    }

    #[test]
    fn the_debug_of_a_call_and_of_the_tools_adapter_prints_no_secret() {
        let variables = |_: &str| Some(SECRET.to_owned());
        let command = vec!["op".to_owned(), SECRET.to_owned()];
        let folder = std::env::temp_dir();
        let call = super::Call {
            provider: "anthropic",
            env: "MY_KEY",
            command: &command,
            variables: &variables,
            folder: &folder,
            bounds: super::Bounds {
                limit: 10,
                timeout: std::time::Duration::from_secs(1),
                poll: std::time::Duration::from_millis(5),
            },
        };
        let shown = format!("{call:?}");
        assert!(
            shown.contains("anthropic") && !shown.contains("PLUM"),
            "{shown}"
        );
    }

    #[test]
    fn a_key_with_a_control_character_is_refused_where_it_is_read() {
        let folder = std::env::temp_dir();
        let read = |value: &'static str, command: &[String]| {
            let variables = move |_: &str| Some(value.to_owned());
            let call = super::Call {
                provider: "test",
                env: "MY_KEY",
                command,
                variables: &variables,
                folder: &folder,
                bounds: super::Bounds {
                    limit: 100,
                    timeout: std::time::Duration::from_secs(5),
                    poll: std::time::Duration::from_millis(5),
                },
            };
            super::prepare(&call, "m", &[], &[], &[]).unwrap_err()
        };
        for value in ["sk-test-PLUM\nx", "sk-test-PLUM\tx", "sk-test-PLUM\u{1b}x"] {
            assert_eq!(
                read(value, &[]),
                "MY_KEY holds a control character (derive.test.api.key_env)"
            );
        }
        let program = ["/bin/sh", "-c", "printf 'sk-test-PLUM\\tx\\n'"].map(str::to_owned);
        assert_eq!(
            read("", &program),
            "derive.test.api.key_command printed a key with a control character"
        );
    }
}
