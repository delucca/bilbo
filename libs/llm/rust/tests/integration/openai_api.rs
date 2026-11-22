use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use bilbo_llm::model::{Cause, Limits, Model, Outcome, Prompt, Usage, What};
use bilbo_llm::openai::api::{self, OpenAi};
use bilbo_provider::{Provider, Reply};
use bilbo_settings::file::File;
use bilbo_settings::key::Source;

use crate::Scratch;

type Reader = fn(&str) -> Option<String>;

const KEY: &str = "sk-test-PLUM-7391";
const COMPLETED: &str = include_str!("../../fixtures/openai/api/completed.json");
const REFUSAL: &str = include_str!("../../fixtures/openai/api/refusal.json");
const INCOMPLETE_MAX: &str = include_str!("../../fixtures/openai/api/incomplete-max.json");
const INCOMPLETE_FILTER: &str = include_str!("../../fixtures/openai/api/incomplete-filter.json");
const FAILED_SERVER: &str = include_str!("../../fixtures/openai/api/failed-server.json");
const FAILED_POLICY: &str = include_str!("../../fixtures/openai/api/failed-policy.json");
const FUNCTION_CALL: &str = include_str!("../../fixtures/openai/api/function-call.json");
const ERROR_401: &str = include_str!("../../fixtures/openai/api/error-401.json");
const ERROR_429_SLOW: &str = include_str!("../../fixtures/openai/api/error-429-slow.json");
const ERROR_429_CREDIT: &str = include_str!("../../fixtures/openai/api/error-429-credit.json");
const ERROR_503: &str = include_str!("../../fixtures/openai/api/error-503.json");

const ANSWER_TEXT: &str = "r1-2\nThe user asked about the garden fence.";

fn prompt() -> Prompt {
    Prompt {
        instruction: "Write notes.".to_owned(),
        material: "#r1 hello\n".to_owned(),
    }
}

fn limits() -> Limits {
    Limits {
        answer_tokens: 16_000,
        timeout: Duration::from_secs(5),
        response: 1 << 20,
        poll: Duration::from_millis(5),
    }
}

fn adapter_at(url: &str, toml: &str, scratch: &Scratch) -> OpenAi {
    let text = format!(
        "derive.openai.model = \"gpt-test-1\"\nderive.openai.api.endpoint = \"{url}/v1/responses\"\n{toml}"
    );
    adapter_from(&text, scratch, variable)
}

fn variable(name: &str) -> Option<String> {
    (name == "OPENAI_API_KEY").then(|| KEY.to_owned())
}

fn adapter_from(text: &str, scratch: &Scratch, reader: fn(&str) -> Option<String>) -> OpenAi {
    let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
    let var = |_: &str| None;
    let section = api::section(&Source {
        var: &var,
        file: &file,
    })
    .expect("a section");
    OpenAi::new(
        section,
        Box::new(reader),
        scratch.path().join("derive"),
        Duration::from_secs(5),
        1 << 20,
        Duration::from_millis(5),
    )
}

fn ready(provider: &Provider, toml: &str, scratch: &Scratch) -> OpenAi {
    let mut writer = adapter_at(&provider.url(), toml, scratch);
    writer.prepare().expect("prepared");
    writer
}

fn call(replies: Vec<Reply>) -> Outcome {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(replies);
    ready(&provider, "", &scratch).call(&prompt(), &limits())
}

fn answered(body: &str) -> Outcome {
    call(vec![Reply::answer(200, body)])
}

fn failed(what: What, status: i32) -> Outcome {
    Outcome::Failed {
        cause: Cause {
            status: Some(status),
            ..Cause::new(what)
        },
        usage: Usage::default(),
    }
}

fn usage(read: u64, written: u64) -> Usage {
    Usage {
        read: Some(read),
        written: Some(written),
    }
}

fn with_status(body: &str, status: &str) -> String {
    body.replace(
        "\"status\":\"completed\"",
        &format!("\"status\":\"{status}\""),
    )
}

fn holds_key(status: i32) -> Outcome {
    failed(What::HoldsKey, status)
}

const BODY: &str = "{\"input\":\"#r1 hello\\n\",\"instructions\":\"Write notes.\",\"max_output_tokens\":16000,\"model\":\"gpt-test-1\",\"store\":false}";

#[test]
fn the_request_is_the_documented_one_and_nothing_more() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![Reply::answer(200, COMPLETED)]);
    ready(&provider, "", &scratch).call(&prompt(), &limits());
    let seen = provider.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path.as_str()),
        ("POST", "/v1/responses")
    );
    let ours: Vec<(String, String)> = seen[0]
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .filter(|(name, _)| name == "content-type" || name == "authorization")
        .collect();
    assert_eq!(
        ours,
        [
            ("content-type".to_owned(), "application/json".to_owned()),
            ("authorization".to_owned(), format!("Bearer {KEY}")),
        ]
    );
    assert_eq!(seen[0].header("user-agent"), ["bilbo"]);
    let known = [
        "content-type",
        "authorization",
        "user-agent",
        "host",
        "accept",
        "accept-encoding",
        "content-length",
    ];
    let other: Vec<_> = seen[0]
        .headers
        .iter()
        .filter(|(name, _)| !known.iter().any(|one| one.eq_ignore_ascii_case(name)))
        .collect();
    assert!(other.is_empty(), "{other:?}");
    assert_eq!(String::from_utf8_lossy(&seen[0].body), BODY);
    assert!(!BODY.contains("tools"));
}

#[test]
fn the_same_prompt_gives_the_same_bytes() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![
        Reply::answer(200, COMPLETED),
        Reply::answer(200, COMPLETED),
    ]);
    let mut writer = ready(&provider, "", &scratch);
    writer.call(&prompt(), &limits());
    writer.call(&prompt(), &limits());
    let seen = provider.requests();
    assert_eq!(seen[0].body, seen[1].body);
}

#[test]
fn extra_fields_and_headers_are_added_as_they_are() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![Reply::answer(200, COMPLETED)]);
    let toml = "derive.openai.api.extra = { reasoning = { effort = \"low\" } }\nderive.openai.api.headers = { OpenAI-Project = \"proj_1\", User-Agent = \"agent-x\" }\n";
    ready(&provider, toml, &scratch).call(&prompt(), &limits());
    let seen = provider.requests();
    assert_eq!(
        String::from_utf8_lossy(&seen[0].body),
        "{\"input\":\"#r1 hello\\n\",\"instructions\":\"Write notes.\",\"max_output_tokens\":16000,\"model\":\"gpt-test-1\",\"reasoning\":{\"effort\":\"low\"},\"store\":false}"
    );
    assert_eq!(seen[0].header("openai-project"), ["proj_1"]);
    assert_eq!(seen[0].header("user-agent"), ["agent-x"]);
    let order: Vec<String> = seen[0]
        .headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .filter(|name| {
            [
                "content-type",
                "authorization",
                "openai-project",
                "user-agent",
            ]
            .contains(&name.as_str())
        })
        .collect();
    assert_eq!(
        order,
        [
            "content-type",
            "authorization",
            "openai-project",
            "user-agent"
        ]
    );
}

#[test]
fn the_limits_are_sent_and_kept() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![Reply::answer(200, COMPLETED), Reply::Stall]);
    let mut writer = ready(&provider, "", &scratch);
    writer.call(&prompt(), &limits());
    let more = Limits {
        answer_tokens: 16_001,
        timeout: Duration::from_secs(1),
        ..limits()
    };
    let outcome = writer.call(&prompt(), &more);
    let seen = provider.requests();
    assert!(String::from_utf8_lossy(&seen[0].body).contains("\"max_output_tokens\":16000,"));
    assert!(String::from_utf8_lossy(&seen[1].body).contains("\"max_output_tokens\":16001,"));
    assert!(matches!(outcome, Outcome::Busy { cause, .. } if cause.what == What::Timeout));
}

fn output(items: &str) -> String {
    format!(
        r#"{{"status":"completed","model":"gpt-test-1","output":[{items}],"usage":{{"input_tokens":5,"output_tokens":2}}}}"#
    )
}

fn message(parts: &str) -> String {
    format!(r#"{{"type":"message","role":"assistant","content":[{parts}]}}"#)
}

fn said(text: &str) -> String {
    format!(r#"{{"type":"output_text","text":"{text}"}}"#)
}

#[test]
fn an_answer_is_its_output_text_with_the_providers_counts() {
    assert_eq!(
        answered(COMPLETED),
        Outcome::Answer {
            text: ANSWER_TEXT.to_owned(),
            usage: usage(120, 45),
            model: Some("gpt-test-1".to_owned()),
        }
    );
    let joined = output(
        &[
            message(&[said("one "), said("two ")].join(",")),
            message(&said("three")),
        ]
        .join(","),
    );
    assert_eq!(
        answered(&joined),
        Outcome::Answer {
            text: "one two three".to_owned(),
            usage: usage(5, 2),
            model: Some("gpt-test-1".to_owned()),
        }
    );
    let passed = output(
        &[
            r#"{"type":"reasoning"}"#.to_owned(),
            r#"{"type":"compaction"}"#.to_owned(),
            message(&said("x")),
        ]
        .join(","),
    );
    assert!(matches!(answered(&passed), Outcome::Answer { text, .. } if text == "x"));
}

#[test]
fn an_output_text_that_holds_no_text_is_a_failed_call() {
    for part in [
        r#"{"type":"output_text"}"#,
        r#"{"type":"output_text","text":null}"#,
        r#"{"type":"output_text","text":7}"#,
    ] {
        assert_eq!(
            answered(&output(&message(part))),
            Outcome::Failed {
                cause: Cause {
                    status: Some(200),
                    ..Cause::new(What::Unreadable)
                },
                usage: usage(5, 2),
            },
            "{part}"
        );
    }
}

#[test]
fn the_model_that_answered_is_kept_by_the_keys_rule() {
    let long = "é".repeat(200);
    let too_long = "m".repeat(201);
    for (given, kept) in [
        ("gpt-test-1", "gpt-test-1"),
        ("vendor/test-model:2026", "vendor/test-model:2026"),
        (long.as_str(), long.as_str()),
        ("a b", "unknown"),
        ("a\\nb", "unknown"),
        ("a\\u0007b", "unknown"),
        (too_long.as_str(), "unknown"),
    ] {
        assert_eq!(
            answered(&COMPLETED.replace(
                "\"model\":\"gpt-test-1\"",
                &format!("\"model\":\"{given}\"")
            )),
            Outcome::Answer {
                text: ANSWER_TEXT.to_owned(),
                usage: usage(120, 45),
                model: Some(kept.to_owned()),
            },
            "{given}"
        );
    }
}

#[test]
fn a_cut_answer_is_cut_whatever_it_holds() {
    assert_eq!(answered(INCOMPLETE_MAX), Outcome::Cut(usage(90, 16_000)));
}

#[test]
fn a_refusal_inside_a_message_is_a_refusal_and_its_text_is_not_kept() {
    let outcome = answered(REFUSAL);
    assert_eq!(
        outcome,
        Outcome::Refused {
            category: None,
            usage: usage(80, 9)
        }
    );
    assert!(!format!("{outcome:?}").contains("CANARY"));
    let beside = output(&message(&format!(
        r#"{},{{"type":"refusal","refusal":"no"}}"#,
        said("SECRETTEXT")
    )));
    let outcome = answered(&beside);
    assert_eq!(
        outcome,
        Outcome::Refused {
            category: None,
            usage: usage(5, 2)
        }
    );
    assert!(!format!("{outcome:?}").contains("SECRETTEXT"));
}

#[test]
fn a_content_filter_and_a_policy_failure_are_refusals_with_their_category() {
    assert_eq!(
        answered(INCOMPLETE_FILTER),
        Outcome::Refused {
            category: Some("content_filter".to_owned()),
            usage: usage(70, 3)
        }
    );
    assert_eq!(
        answered(FAILED_POLICY),
        Outcome::Refused {
            category: Some("bio_policy".to_owned()),
            usage: Usage::default()
        }
    );
    assert_eq!(
        answered(&FAILED_POLICY.replace("bio_policy", "misalignment_policy_violation")),
        Outcome::Refused {
            category: Some("misalignment_policy_violation".to_owned()),
            usage: Usage::default()
        }
    );
}

#[test]
fn a_failed_response_under_a_200_is_busy_or_failed_by_its_code() {
    assert!(matches!(
        answered(FAILED_SERVER),
        Outcome::Busy { wait: None, cause } if cause.what == What::ServerError
    ));
    let limited = FAILED_SERVER.replace("server_error", "rate_limit_exceeded");
    assert!(matches!(
        answered(&limited),
        Outcome::Busy { wait: None, cause } if cause.what == What::RateLimit
    ));
    let invalid = FAILED_SERVER.replace("server_error", "invalid_prompt");
    let Outcome::Failed {
        cause,
        usage: counted,
    } = answered(&invalid)
    else {
        panic!("a failed call");
    };
    assert_eq!(cause.what, What::StopReason);
    assert_eq!(cause.code.as_deref(), Some("invalid_prompt"));
    assert_eq!(cause.status, Some(200));
    assert_eq!(counted, usage(60, 0));
    let residency = FAILED_SERVER.replace("server_error", "data_residency_mismatch");
    let Outcome::Stopped(cause) = answered(&residency) else {
        panic!("stopped");
    };
    assert_eq!(cause.what, What::Forbidden);
    assert_eq!(cause.code.as_deref(), Some("data_residency_mismatch"));
}

#[test]
fn another_status_word_is_a_failed_call_with_its_name() {
    for word in ["queued", "in_progress", "cancelled"] {
        let Outcome::Failed { cause, .. } = answered(&with_status(COMPLETED, word)) else {
            panic!("a failed call");
        };
        assert_eq!(
            (cause.what, cause.code.as_deref()),
            (What::StopReason, Some(word))
        );
    }
    let incomplete = COMPLETED.replace(
        "\"incomplete_details\":null",
        "\"incomplete_details\":{\"reason\":\"max_messages\"}",
    );
    let Outcome::Failed { cause, .. } = answered(&with_status(&incomplete, "incomplete")) else {
        panic!("a failed call");
    };
    assert_eq!(
        (cause.what, cause.code.as_deref()),
        (What::StopReason, Some("max_messages"))
    );
}

#[test]
fn an_output_that_is_no_message_is_a_failed_call() {
    let outcome = answered(FUNCTION_CALL);
    let Outcome::Failed {
        cause,
        usage: counted,
    } = &outcome
    else {
        panic!("a failed call");
    };
    assert_eq!(cause.what, What::ToolUsed);
    assert_eq!(*counted, usage(50, 12));
    assert!(!format!("{outcome:?}").contains("lookup"));
    let beside = output(&format!(
        r#"{},{{"type":"function_call"}}"#,
        message(&said("x"))
    ));
    assert!(
        matches!(answered(&beside), Outcome::Failed { cause, .. } if cause.what == What::ToolUsed)
    );
    for kind in [
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
    ] {
        let body = output(&format!(r#"{{"type":"{kind}"}}"#));
        assert!(
            matches!(answered(&body), Outcome::Failed { cause, .. } if cause.what == What::ToolUsed),
            "{kind}"
        );
    }
    let unreadable = |items: String| matches!(answered(&output(&items)), Outcome::Failed { cause, .. } if cause.what == What::Unreadable);
    assert!(unreadable(r#"{"type":"surprise"}"#.to_owned()));
    assert!(unreadable(r#"{"role":"assistant"}"#.to_owned()));
    assert!(unreadable(
        r#"{"type":"message","content":"text"}"#.to_owned()
    ));
    assert!(unreadable(message(&format!(
        r#"{{"type":"surprise"}},{}"#,
        said("x")
    ))));
}

#[test]
fn no_text_is_a_failed_call_and_an_empty_text_is_an_answer() {
    let reasoning = COMPLETED.replace(
        ",{\"type\":\"message\",\"id\":\"msg_fixture_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"r1-2\\nThe user asked about the garden fence.\",\"annotations\":[]}]}",
        "",
    );
    assert_ne!(reasoning, COMPLETED);
    let Outcome::Failed { cause, .. } = answered(&reasoning) else {
        panic!("a failed call");
    };
    assert_eq!(cause.what, What::Unreadable);
    let empty = COMPLETED.replace(ANSWER_TEXT.replace('\n', "\\n").as_str(), "");
    assert_eq!(
        answered(&empty),
        Outcome::Answer {
            text: String::new(),
            usage: usage(120, 45),
            model: Some("gpt-test-1".to_owned()),
        }
    );
}

#[test]
fn a_429_is_busy_for_the_wait_named_and_a_spent_account_stops() {
    let named_wait = call(vec![
        Reply::answer(429, ERROR_429_SLOW).with_header("Retry-After", "7"),
    ]);
    let Outcome::Busy { wait, cause } = named_wait else {
        panic!("busy");
    };
    assert_eq!(wait, Some(Duration::from_secs(7)));
    assert_eq!(
        cause,
        Cause {
            status: Some(429),
            kind: Some("rate_limit_error".to_owned()),
            code: Some("slow_down".to_owned()),
            ..Cause::new(What::RateLimit)
        }
    );
    let Outcome::Busy { wait, .. } = call(vec![Reply::answer(429, ERROR_429_SLOW)]) else {
        panic!("busy");
    };
    assert_eq!(wait, None);
    let Outcome::Busy { wait, .. } = call(vec![
        Reply::answer(429, ERROR_429_SLOW).with_header("Retry-After", "soon"),
    ]) else {
        panic!("busy");
    };
    assert_eq!(wait, None);
    for code in [
        "credit_balance_exhausted",
        "organization_spend_limit_exceeded",
        "project_spend_limit_exceeded",
        "organization_usage_limit_exceeded",
    ] {
        let body = ERROR_429_CREDIT.replace("credit_balance_exhausted", code);
        let Outcome::Stopped(cause) = call(vec![Reply::answer(429, &body)]) else {
            panic!("stopped");
        };
        assert_eq!(
            (cause.what, cause.code.as_deref()),
            (What::Billing, Some(code))
        );
    }
    let by_type = ERROR_429_CREDIT.replace("\"credit_balance_exhausted\"", "null");
    let Outcome::Stopped(cause) = call(vec![Reply::answer(429, &by_type)]) else {
        panic!("stopped");
    };
    assert_eq!(
        (cause.what, cause.kind.as_deref(), cause.code),
        (What::Billing, Some("insufficient_quota"), None)
    );
}

#[test]
fn a_busy_status_takes_the_wait_the_response_names() {
    for (status, header, wait) in [
        (503, Some("7"), Some(Duration::from_secs(7))),
        (503, Some("1.5"), Some(Duration::from_secs(2))),
        (503, None, None),
        (408, Some("3"), Some(Duration::from_secs(3))),
    ] {
        let reply = match header {
            Some(value) => Reply::answer(status, ERROR_503).with_header("retry-after", value),
            None => Reply::answer(status, ERROR_503),
        };
        let Outcome::Busy { wait: given, .. } = call(vec![reply]) else {
            panic!("{status} {header:?}");
        };
        assert_eq!(given, wait, "{status} {header:?}");
    }
}

#[test]
fn each_status_is_the_outcome_its_row_says() {
    let busy = |what| (what, true);
    let stopped = |what| (what, false);
    let rows = [
        (400, stopped(What::Rejected)),
        (401, stopped(What::Unauthorized)),
        (403, stopped(What::Forbidden)),
        (402, stopped(What::Rejected)),
        (404, stopped(What::NotFound)),
        (408, busy(What::Timeout)),
        (409, busy(What::ServerError)),
        (500, busy(What::ServerError)),
        (501, busy(What::ServerError)),
        (502, busy(What::ServerError)),
        (503, busy(What::Overloaded)),
        (504, busy(What::ServerError)),
        (520, busy(What::ServerError)),
        (529, busy(What::ServerError)),
        (599, busy(What::ServerError)),
    ];
    for (status, (what, is_busy)) in rows {
        let outcome = call(vec![Reply::answer(status, "{}")]);
        let expected = Cause {
            status: Some(i32::from(status)),
            ..Cause::new(what)
        };
        match outcome {
            Outcome::Busy { wait: None, cause } if is_busy => {
                assert_eq!(cause, expected, "{status}");
            }
            Outcome::Stopped(cause) if !is_busy => assert_eq!(cause, expected, "{status}"),
            other => panic!("{status}: {other:?}"),
        }
    }
    assert_eq!(
        call(vec![Reply::answer(302, "")]),
        failed(What::Unreadable, 302)
    );
}

#[test]
fn an_error_names_its_status_type_code_and_request_and_never_its_message() {
    let outcome = call(vec![
        Reply::answer(401, ERROR_401).with_header("x-request-id", "req_abc123"),
    ]);
    assert_eq!(
        outcome,
        Outcome::Stopped(Cause {
            what: What::Unauthorized,
            status: Some(401),
            kind: Some("invalid_request_error".to_owned()),
            code: Some("invalid_api_key".to_owned()),
            request: Some("req_abc123".to_owned()),
        })
    );
    assert!(!format!("{outcome:?}").contains("CANARY"));
}

#[test]
fn a_response_bilbo_cannot_read_is_a_failed_call() {
    for body in [
        "not json at all",
        "[1, 2]",
        "{\"id\":\"resp_x\",\"output\":[]}",
    ] {
        assert_eq!(answered(body), failed(What::Unreadable, 200), "{body}");
    }
}

#[test]
fn a_response_over_the_limit_is_a_failed_call() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![
        Reply::answer(200, COMPLETED),
        Reply::answer(200, COMPLETED),
    ]);
    let mut writer = ready(&provider, "", &scratch);
    let at = Limits {
        response: COMPLETED.len() as u64,
        ..limits()
    };
    assert!(matches!(
        writer.call(&prompt(), &at),
        Outcome::Answer { .. }
    ));
    let under = Limits {
        response: COMPLETED.len() as u64 - 1,
        ..limits()
    };
    assert_eq!(
        writer.call(&prompt(), &under),
        Outcome::Failed {
            cause: Cause::new(What::TooLarge),
            usage: Usage::default()
        }
    );
}

#[test]
fn no_answer_in_time_and_no_connection_are_busy() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![Reply::Stall]);
    let short = Limits {
        timeout: Duration::from_millis(300),
        ..limits()
    };
    let started = std::time::Instant::now();
    assert_eq!(
        ready(&provider, "", &scratch).call(&prompt(), &short),
        Outcome::Busy {
            wait: None,
            cause: Cause::new(What::Timeout)
        }
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("a port")
        .local_addr()
        .expect("an address")
        .port();
    let mut writer = adapter_at(&format!("http://127.0.0.1:{port}"), "", &scratch);
    writer.prepare().expect("prepared");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Busy {
            wait: None,
            cause: Cause::new(What::NoConnection)
        }
    );
    let mut writer = adapter_at(&crate::untrusted_address(), "", &scratch);
    writer.prepare().expect("prepared");
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::Tls))
    );
}

#[test]
fn prepare_reads_the_key_once_and_refuses_what_is_missing() {
    let scratch = Scratch::new("openai");
    let file = File::parse(Path::new("/c/config.toml"), b"").expect("toml");
    let var = |_: &str| None;
    let section = api::section(&Source {
        var: &var,
        file: &file,
    })
    .expect("a section");
    let mut bare = OpenAi::new(
        section,
        Box::new(|_| None),
        scratch.path().join("derive"),
        Duration::from_secs(5),
        1 << 20,
        Duration::from_millis(5),
    );
    assert_eq!(
        bare.prepare(),
        Err("derive.openai.model is not set".to_owned())
    );
    let mut no_variable = OpenAi::new(
        api::section(&Source {
            var: &var,
            file: &File::parse(Path::new("/c/config.toml"), b"derive.openai.model = \"m\"")
                .expect("toml"),
        })
        .expect("a section"),
        Box::new(|_| None),
        scratch.path().join("derive"),
        Duration::from_secs(5),
        1 << 20,
        Duration::from_millis(5),
    );
    assert_eq!(
        no_variable.prepare(),
        Err("OPENAI_API_KEY is not set (derive.openai.api.key_env)".to_owned())
    );

    let counter = scratch.path().join("ran");
    let script = scratch.script(
        "key.sh",
        &format!("echo ran >> '{}'\necho {KEY}", counter.display()),
    );
    let toml = format!(
        "derive.openai.api.key_command = [\"/bin/sh\", \"{}\"]\n",
        script.display()
    );
    let provider = Provider::start(vec![
        Reply::answer(200, COMPLETED),
        Reply::answer(200, COMPLETED),
    ]);
    let mut writer = adapter_at(&provider.url(), &toml, &scratch);
    let before = writer.call(&prompt(), &limits());
    assert_eq!(before, Outcome::Stopped(Cause::new(What::NotStarted)));
    assert!(provider.requests().is_empty());
    writer.prepare().expect("prepared");
    writer.prepare().expect("prepared again");
    writer.call(&prompt(), &limits());
    writer.call(&prompt(), &limits());
    assert_eq!(
        std::fs::read_to_string(&counter).expect("a counter"),
        "ran\nran\n"
    );
    let seen = provider.requests();
    assert_eq!(seen[1].header("authorization"), [format!("Bearer {KEY}")]);
}

#[test]
fn the_key_is_in_one_header_and_in_nothing_else_the_run_gives() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![
        Reply::answer(200, COMPLETED),
        Reply::answer(401, ERROR_401),
        Reply::answer(200, REFUSAL),
    ]);
    let mut writer = ready(
        &provider,
        "derive.openai.api.headers = { OpenAI-Project = \"p\" }\n",
        &scratch,
    );
    let outcomes: Vec<String> = (0..3)
        .map(|_| format!("{:?}", writer.call(&prompt(), &limits())))
        .collect();
    for text in &outcomes {
        assert!(!text.contains(KEY), "{text}");
    }
    for line in writer.describe() {
        assert!(!line.contains(KEY), "{line}");
    }
    assert!(!format!("{writer:?}").contains("PLUM"));
    let second = Provider::start(vec![Reply::answer(200, COMPLETED)]);
    let spaced: Reader = |name| (name == "OPENAI_API_KEY").then(|| format!(" {KEY}\t"));
    let text = format!(
        "derive.openai.model = \"gpt-test-1\"\nderive.openai.api.endpoint = \"{}/v1/responses\"\n",
        second.url()
    );
    let mut trimmed = adapter_from(&text, &scratch, spaced);
    trimmed.prepare().expect("prepared");
    trimmed.call(&prompt(), &limits());
    assert_eq!(
        second.requests()[0].header("authorization"),
        [format!("Bearer {KEY}")]
    );
    for request in provider.requests() {
        assert_eq!(request.header("authorization"), [format!("Bearer {KEY}")]);
        assert!(!String::from_utf8_lossy(&request.body).contains(KEY));
        for (name, value) in &request.headers {
            if !name.eq_ignore_ascii_case("authorization") {
                assert!(!value.contains(KEY), "{name}");
            }
        }
    }
    let lines = writer.describe();
    assert_eq!(
        lines[0],
        format!(
            "provider: openai, via api, model gpt-test-1, at {}/v1/responses",
            provider.url()
        )
    );
    assert_eq!(lines[1], "headers: { OpenAI-Project = \"p\" }");
    assert_eq!(lines[2], "key: the variable OPENAI_API_KEY, which is set");
}

fn key_in_response(body: &str) {
    assert_eq!(answered(body), holds_key(200), "{body}");
    let text = format!("{:?}", answered(body));
    assert!(!text.contains(KEY) && !text.contains("garden"), "{text}");
}

#[test]
fn a_model_that_is_the_key_is_a_failed_call() {
    key_in_response(
        &COMPLETED.replace("\"model\":\"gpt-test-1\"", &format!("\"model\":\"{KEY}\"")),
    );
}

#[test]
fn an_error_type_that_is_the_key_is_a_failed_call() {
    let body = ERROR_401.replace("invalid_request_error", KEY);
    assert_eq!(call(vec![Reply::answer(400, &body)]), holds_key(400));
}

#[test]
fn an_error_code_that_is_the_key_is_a_failed_call() {
    let body = ERROR_429_SLOW.replace("slow_down", KEY);
    assert_eq!(call(vec![Reply::answer(400, &body)]), holds_key(400));
    let under_200 = FAILED_SERVER.replace("server_error", KEY);
    assert_eq!(answered(&under_200), holds_key(200));
}

#[test]
fn a_request_id_that_is_the_key_is_a_failed_call() {
    assert_eq!(
        call(vec![
            Reply::answer(200, COMPLETED).with_header("x-request-id", KEY)
        ]),
        holds_key(200)
    );
    assert_eq!(
        call(vec![
            Reply::answer(400, ERROR_503).with_header("x-request-id", KEY)
        ]),
        holds_key(400)
    );
}

#[test]
fn a_key_found_in_a_providers_error_keeps_the_providers_cause() {
    assert_eq!(
        call(vec![
            Reply::answer(401, "{}").with_header("x-request-id", KEY)
        ]),
        Outcome::Stopped(Cause {
            status: Some(401),
            ..Cause::new(What::Unauthorized)
        })
    );
    assert_eq!(
        call(vec![
            Reply::answer(503, "{}").with_header("x-request-id", KEY)
        ]),
        Outcome::Busy {
            wait: None,
            cause: Cause {
                status: Some(503),
                ..Cause::new(What::Overloaded)
            }
        }
    );
    let typed = ERROR_401.replace("invalid_request_error", KEY);
    let outcome = call(vec![Reply::answer(401, &typed)]);
    assert_eq!(
        outcome,
        Outcome::Stopped(Cause {
            status: Some(401),
            code: Some("invalid_api_key".to_owned()),
            ..Cause::new(What::Unauthorized)
        })
    );
    assert!(!format!("{outcome:?}").contains("PLUM"));
    assert_eq!(
        call(vec![
            Reply::answer(200, COMPLETED).with_header("x-request-id", KEY)
        ]),
        holds_key(200)
    );
}

#[test]
fn a_status_or_a_reason_that_is_the_key_is_a_failed_call() {
    key_in_response(&with_status(COMPLETED, KEY));
    let incomplete = COMPLETED.replace(
        "\"incomplete_details\":null",
        &format!("\"incomplete_details\":{{\"reason\":\"{KEY}\"}}"),
    );
    key_in_response(&with_status(&incomplete, "incomplete"));
}

#[test]
fn an_answer_that_holds_the_key_is_a_failed_call() {
    let body = COMPLETED.replace("the garden fence", &format!("the key {KEY} by mistake"));
    assert_ne!(body, COMPLETED);
    assert_eq!(answered(&body), holds_key(200));
    let beside = output(&message(&format!(
        r#"{},{{"type":"refusal","refusal":"no"}}"#,
        said(&format!("see {KEY}"))
    )));
    key_in_response(&beside);
}

#[test]
fn a_setting_that_holds_the_key_is_refused_by_its_keys_name_and_nothing_is_sent() {
    let scratch = Scratch::new("openai");
    let provider = Provider::start(vec![Reply::answer(200, COMPLETED)]);
    let url = provider.url();
    let with_model = |model: &str| {
        format!(
            "derive.openai.model = \"{model}\"\nderive.openai.api.endpoint = \"{url}/v1/responses\"\n"
        )
    };
    let extra = |text: &str| {
        format!("derive.openai.api.extra = {{ metadata = {{ user_id = \"{text}\" }} }}\n")
    };
    let script = scratch.script("key.sh", &format!("echo {KEY}"));
    let command = format!(
        "derive.openai.api.key_command = [\"/bin/sh\", \"{}\"]\n",
        script.display()
    );
    let given: fn(&str) -> Option<String> = variable;
    let none: fn(&str) -> Option<String> = |_| None;
    let cases: Vec<(String, Reader, &str)> = vec![
        (with_model(KEY), given, "model"),
        (with_model("gw/sk-test-PLUM-7391/x"), given, "model"),
        (
            format!(
                "{}derive.openai.api.headers = {{ openai-organization = \"id {KEY}\" }}\n",
                with_model("m")
            ),
            given,
            "api.headers",
        ),
        (
            format!(
                "{}derive.openai.api.headers = {{ {KEY} = \"1\" }}\n",
                with_model("m")
            ),
            given,
            "api.headers",
        ),
        (
            format!("{}{}", with_model("m"), extra(KEY)),
            given,
            "api.extra",
        ),
        (
            format!(
                "{}derive.openai.api.headers = {{ x-trace = \"{KEY}\" }}\n{}",
                with_model("m"),
                extra(KEY)
            ),
            given,
            "api.headers",
        ),
        (
            with_model(KEY),
            |name| (name == "OPENAI_API_KEY").then(|| format!(" {KEY}\t")),
            "model",
        ),
        (format!("{}{}", with_model(KEY), extra(KEY)), given, "model"),
        (format!("{}{command}", with_model(KEY)), none, "model"),
    ];
    for (text, reader, key) in cases {
        let mut writer = adapter_from(&text, &scratch, reader);
        assert_eq!(
            writer.prepare().unwrap_err(),
            format!("derive.openai.{key} holds the key")
        );
        let after = writer.call(&prompt(), &limits());
        assert_eq!(after, Outcome::Stopped(Cause::new(What::NotStarted)));
    }
    let near = format!(
        "{}{}",
        with_model("sk-test-PLUM-739"),
        extra("sk-test-PLUM")
    );
    let mut writer = adapter_from(&near, &scratch, given);
    writer.prepare().expect("no setting holds the whole key");
    assert!(matches!(
        writer.call(&prompt(), &limits()),
        Outcome::Answer { .. }
    ));
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn describe_says_the_settings_in_force_and_how_the_key_is_read() {
    let scratch = Scratch::new("openai");
    let toml = "derive.openai.api.headers = { X-A = \"1\", X-B = \"2\" }\nderive.openai.api.extra = { reasoning = { effort = \"low\" } }\nderive.openai.api.key_command = [\"op\", \"read\"]\n";
    let writer = adapter_at("http://127.0.0.1:9", toml, &scratch);
    assert_eq!(writer.id(), "gpt-test-1");
    assert_eq!(
        writer.describe(),
        [
            "provider: openai, via api, model gpt-test-1, at http://127.0.0.1:9/v1/responses",
            "headers: { X-A = \"1\", X-B = \"2\" }",
            "extra: { reasoning = { effort = \"low\" } }",
            "key: the program [\"op\", \"read\"], not run",
        ]
    );
    let text = "derive.openai.model = \"gpt-test-1\"\nderive.openai.api.key_env = \"MY_KEY\"\n";
    let writer = adapter_from(text, &scratch, |_| None);
    assert_eq!(
        writer.describe().last().map(String::as_str),
        Some("key: the variable MY_KEY, which is not set")
    );
}

#[test]
fn a_key_split_over_two_parts_or_two_messages_is_not_kept() {
    key_in_response(&output(&message(
        &[said("see sk-test-"), said("PLUM-7391 here")].join(","),
    )));
    key_in_response(&output(
        &[
            message(&said("see sk-test-")),
            message(&said("PLUM-7391 here")),
        ]
        .join(","),
    ));
}

#[test]
fn the_guard_reads_the_model_before_the_headers() {
    let scratch = Scratch::new("openai");
    let text = format!(
        "derive.openai.model = \"{KEY}\"\nderive.openai.api.endpoint = \"http://127.0.0.1:9/v1/responses\"\nderive.openai.api.headers = {{ x-trace = \"{KEY}\" }}\n"
    );
    let mut writer = adapter_from(&text, &scratch, variable);
    assert_eq!(
        writer.prepare().unwrap_err(),
        "derive.openai.model holds the key"
    );
}

#[test]
fn a_failed_reply_with_a_busy_code_takes_the_named_wait() {
    for (code, what) in [
        ("server_error", What::ServerError),
        ("rate_limit_exceeded", What::RateLimit),
    ] {
        let body = FAILED_SERVER.replace("server_error", code);
        for (header, wait) in [(Some("7"), Some(Duration::from_secs(7))), (None, None)] {
            let reply = match header {
                Some(value) => Reply::answer(200, &body).with_header("retry-after", value),
                None => Reply::answer(200, &body),
            };
            let Outcome::Busy { wait: given, cause } = call(vec![reply]) else {
                panic!("busy");
            };
            assert_eq!((given, cause.what), (wait, what), "{code} {header:?}");
        }
    }
}
