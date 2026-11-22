use std::path::Path;
use std::time::{Duration, Instant};

use bilbo_llm::anthropic::api::{self, Anthropic};
use bilbo_llm::model::{Cause, Limits, Model, Outcome, Prompt, Usage, What};
use bilbo_provider::{Provider, Reply, Request};
use bilbo_settings::file::File;
use bilbo_settings::key::Source;

use crate::Scratch;

type Reader = fn(&str) -> Option<String>;

const KEY: &str = "sk-test-PLUM-7391";
const ANSWER: &str = include_str!("../../fixtures/anthropic/api/answer.json");
const CUT: &str = include_str!("../../fixtures/anthropic/api/cut.json");
const REFUSAL: &str = include_str!("../../fixtures/anthropic/api/refusal.json");
const REFUSAL_NULL: &str = include_str!("../../fixtures/anthropic/api/refusal-null.json");
const ERROR_404: &str = include_str!("../../fixtures/anthropic/api/error-404.json");
const ERROR_400: &str = include_str!("../../fixtures/anthropic/api/error-400.json");
const ERROR_429: &str = include_str!("../../fixtures/anthropic/api/error-429.json");
const ERROR_429_SPEND: &str = include_str!("../../fixtures/anthropic/api/error-429-spend.json");
const NO_STOP_REASON: &str = include_str!("../../fixtures/anthropic/api/no-stop-reason.json");
const ERROR_529: &str = include_str!("../../fixtures/anthropic/api/error-529.json");

fn prompt() -> Prompt {
    Prompt {
        instruction: "Write paragraphs.".to_owned(),
        material: "#r1 hello".to_owned(),
    }
}

fn limits() -> Limits {
    Limits {
        answer_tokens: 16000,
        timeout: Duration::from_secs(5),
        response: 1 << 20,
        poll: Duration::from_millis(5),
    }
}

fn config(url: &str, keys: &str, tables: &str) -> String {
    format!(
        "[llm.anthropic]\nmodel = \"test-model-1\"\n\n[llm.anthropic.api]\nendpoint = \"{url}/v1/messages\"\n{keys}\n{tables}"
    )
}

fn variables(name: &str) -> Option<String> {
    (name == "ANTHROPIC_API_KEY").then(|| KEY.to_owned())
}

fn writer_with(text: &str, folder: &Path, variables: fn(&str) -> Option<String>) -> Anthropic {
    let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
    let section = api::section(&Source {
        var: &|_| None,
        file: &file,
    })
    .expect("a section");
    Anthropic::new(
        section,
        Box::new(variables),
        folder.to_path_buf(),
        Duration::from_secs(5),
        1 << 20,
        Duration::from_millis(5),
    )
}

fn ready(text: &str) -> Anthropic {
    let mut writer = writer_with(text, &std::env::temp_dir(), variables);
    writer.prepare().expect("the key is read");
    writer
}

fn answer_of(provider: &Provider, replies: usize) -> Vec<Outcome> {
    let mut writer = ready(&config(&provider.url(), "", ""));
    (0..replies)
        .map(|_| writer.call(&prompt(), &limits()))
        .collect()
}

fn one(reply: Reply) -> Outcome {
    let provider = Provider::start(vec![reply]);
    answer_of(&provider, 1).remove(0)
}

fn ok(body: &str) -> Outcome {
    one(Reply::answer(200, body))
}

fn cause(
    what: What,
    status: i32,
    kind: Option<&str>,
    code: Option<&str>,
    request: Option<&str>,
) -> Cause {
    Cause {
        what,
        status: Some(status),
        kind: kind.map(str::to_owned),
        code: code.map(str::to_owned),
        request: request.map(str::to_owned),
    }
}

fn usage(read: u64, written: u64) -> Usage {
    Usage {
        read: Some(read),
        written: Some(written),
    }
}

fn sent(request: &Request, names: &[&str]) -> Vec<(String, String)> {
    request
        .headers
        .iter()
        .filter(|(name, _)| names.iter().any(|wanted| wanted.eq_ignore_ascii_case(name)))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

#[test]
fn the_request_is_the_documented_one_and_nothing_more() {
    let provider = Provider::start(vec![Reply::answer(200, ANSWER)]);
    answer_of(&provider, 1);
    let seen = provider.requests();
    let request = &seen[0];
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/v1/messages")
    );
    let own = ["content-type", "x-api-key", "anthropic-version"];
    assert_eq!(
        sent(request, &own),
        [
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-api-key".to_owned(), KEY.to_owned()),
            ("anthropic-version".to_owned(), "2026-10-21".to_owned()),
        ]
    );
    let standard = [
        "host",
        "user-agent",
        "accept",
        "accept-encoding",
        "content-length",
    ];
    let other: Vec<_> = request
        .headers
        .iter()
        .filter(|(name, _)| {
            !standard
                .iter()
                .chain(own.iter())
                .any(|known| known.eq_ignore_ascii_case(name))
        })
        .collect();
    assert!(other.is_empty(), "{other:?}");
    assert_eq!(request.header("user-agent"), ["bilbo"]);
    assert_eq!(
        String::from_utf8(request.body.clone()).expect("utf8"),
        r##"{"max_tokens":16000,"messages":[{"content":"#r1 hello","role":"user"}],"model":"test-model-1","system":"Write paragraphs."}"##
    );
}

#[test]
fn the_same_prompt_gives_the_same_bytes() {
    let provider = Provider::start(vec![Reply::answer(200, ANSWER), Reply::answer(200, ANSWER)]);
    answer_of(&provider, 2);
    let seen = provider.requests();
    assert_eq!(seen[0].body, seen[1].body);
}

#[test]
fn extra_fields_and_headers_are_added_as_they_are() {
    let provider = Provider::start(vec![Reply::answer(200, ANSWER)]);
    let text = config(
        &provider.url(),
        "",
        "[llm.anthropic.api.extra]\noutput_config = { effort = \"low\" }\n\n[llm.anthropic.api.headers]\nanthropic-beta = \"beta-1\"\nUser-Agent = \"mine\"\n",
    );
    ready(&text).call(&prompt(), &limits());
    let seen = provider.requests();
    assert_eq!(
        String::from_utf8(seen[0].body.clone()).expect("utf8"),
        r##"{"max_tokens":16000,"messages":[{"content":"#r1 hello","role":"user"}],"model":"test-model-1","output_config":{"effort":"low"},"system":"Write paragraphs."}"##
    );
    assert_eq!(
        sent(
            &seen[0],
            &[
                "content-type",
                "x-api-key",
                "anthropic-version",
                "anthropic-beta",
                "user-agent"
            ]
        )
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>(),
        [
            "content-type",
            "x-api-key",
            "anthropic-version",
            "anthropic-beta",
            "user-agent"
        ]
    );
    assert_eq!(seen[0].header("user-agent"), ["mine"]);
    assert_eq!(seen[0].header("anthropic-beta"), ["beta-1"]);
}

#[test]
fn the_limits_are_sent_and_kept() {
    let provider = Provider::start(vec![
        Reply::answer(200, ANSWER),
        Reply::answer(200, ANSWER),
        Reply::Stall,
    ]);
    let mut writer = ready(&config(&provider.url(), "", ""));
    writer.call(&prompt(), &limits());
    writer.call(
        &prompt(),
        &Limits {
            answer_tokens: 16001,
            ..limits()
        },
    );
    let seen = provider.requests();
    assert!(String::from_utf8_lossy(&seen[0].body).contains("\"max_tokens\":16000,"));
    assert!(String::from_utf8_lossy(&seen[1].body).contains("\"max_tokens\":16001,"));
    let start = Instant::now();
    let outcome = writer.call(
        &prompt(),
        &Limits {
            timeout: Duration::from_secs(1),
            ..limits()
        },
    );
    assert_eq!(
        outcome,
        Outcome::Busy {
            wait: None,
            cause: Cause::new(What::Timeout)
        }
    );
    assert!(start.elapsed() < Duration::from_secs(4));
}

fn blocks(blocks: &str, usage: &str) -> String {
    format!(
        r#"{{"model":"test-model-1","stop_reason":"end_turn","content":[{blocks}],"usage":{usage}}}"#
    )
}

#[test]
fn an_answer_is_its_text_blocks_with_the_providers_counts() {
    assert_eq!(
        ok(ANSWER),
        Outcome::Answer {
            text: "#r1-r2 The user asked for the carrot rows to be thinned.".to_owned(),
            usage: usage(123, 40),
            model: Some("test-model-1".to_owned()),
        }
    );
    let text = |text: &str| format!(r#"{{"type":"text","text":"{text}"}}"#);
    let kind = |kind: &str| format!(r#"{{"type":"{kind}"}}"#);
    let counts = r#"{"input_tokens":5,"output_tokens":2}"#;
    let joined = blocks(
        &[text("one "), kind("thinking"), text("two")].join(","),
        counts,
    );
    assert_eq!(
        ok(&joined),
        Outcome::Answer {
            text: "one two".to_owned(),
            usage: usage(5, 2),
            model: Some("test-model-1".to_owned()),
        }
    );
    let passed = blocks(
        &[
            kind("thinking"),
            kind("redacted_thinking"),
            kind("compaction"),
            text("one "),
            kind("fallback"),
            text("two"),
        ]
        .join(","),
        counts,
    );
    assert!(matches!(ok(&passed), Outcome::Answer { text, .. } if text == "one two"));
    let only = text("x");
    for (counted, read) in [
        (
            r#"{"input_tokens":null,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"output_tokens":1}"#,
            Some(5),
        ),
        (
            r#"{"input_tokens":18446744073709551615,"cache_read_input_tokens":1,"output_tokens":1}"#,
            Some(u64::MAX),
        ),
        (r#"{"input_tokens":null,"output_tokens":1}"#, None),
    ] {
        let outcome = ok(&blocks(&only, counted));
        assert!(
            matches!(&outcome, Outcome::Answer { usage, .. } if usage.read == read && usage.written == Some(1)),
            "{counted}: {outcome:?}"
        );
    }
}

#[test]
fn the_model_that_answered_is_kept_by_the_keys_rule() {
    let answered = |model: &str| {
        ok(&ANSWER.replace(
            "\"model\":\"test-model-1\"",
            &format!("\"model\":\"{model}\""),
        ))
    };
    let long = "é".repeat(200);
    let too_long = "m".repeat(201);
    for (given, kept) in [
        ("test-model-1", "test-model-1"),
        ("vendor/test-model:2026", "vendor/test-model:2026"),
        (long.as_str(), long.as_str()),
        ("a b", "unknown"),
        ("a\\nb", "unknown"),
        ("a\\u0007b", "unknown"),
        (too_long.as_str(), "unknown"),
    ] {
        assert_eq!(
            answered(given),
            Outcome::Answer {
                text: "#r1-r2 The user asked for the carrot rows to be thinned.".to_owned(),
                usage: usage(123, 40),
                model: Some(kept.to_owned()),
            },
            "{given}"
        );
    }
}

#[test]
fn a_block_that_is_no_text_is_a_failed_call() {
    let counts = r#"{"input_tokens":5,"output_tokens":2}"#;
    let failed = |what: What| Outcome::Failed {
        cause: cause(what, 200, None, None, None),
        usage: usage(5, 2),
    };
    for kind in [
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
    ] {
        let body = blocks(
            &format!(r#"{{"type":"text","text":"SECRETTEXT"}},{{"type":"{kind}"}}"#),
            counts,
        );
        let outcome = ok(&body);
        assert_eq!(outcome, failed(What::ToolUsed), "{kind}");
        assert!(!format!("{outcome:?}").contains("SECRETTEXT"));
    }
    for block in [
        r#"{"type":"surprise"}"#,
        r#"{"text":"x"}"#,
        "7",
        r#""text""#,
    ] {
        let body = blocks(&format!(r#"{{"type":"text","text":"x"}},{block}"#), counts);
        assert_eq!(ok(&body), failed(What::Unreadable), "{block}");
    }
    let both = blocks(r#"{"type":"surprise"},{"type":"tool_use"}"#, counts);
    assert_eq!(ok(&both), failed(What::ToolUsed));
}

#[test]
fn no_text_block_is_a_failed_call_and_an_empty_text_is_an_answer() {
    let thinking = r#"{"stop_reason":"end_turn","content":[{"type":"thinking","thinking":""}],"usage":{"input_tokens":5,"output_tokens":2}}"#;
    assert_eq!(
        ok(thinking),
        Outcome::Failed {
            cause: cause(What::Unreadable, 200, None, None, None),
            usage: usage(5, 2)
        }
    );
    let empty = r#"{"model":"m","stop_reason":"end_turn","content":[{"type":"text","text":""}],"usage":{"input_tokens":5,"output_tokens":2}}"#;
    assert_eq!(
        ok(empty),
        Outcome::Answer {
            text: String::new(),
            usage: usage(5, 2),
            model: Some("m".to_owned())
        }
    );
    let no_list = r#"{"stop_reason":"end_turn","content":"text","usage":{"input_tokens":5,"output_tokens":2}}"#;
    assert_eq!(
        ok(no_list),
        Outcome::Failed {
            cause: cause(What::Unreadable, 200, None, None, None),
            usage: usage(5, 2)
        }
    );
}

#[test]
fn a_cut_answer_is_cut_whatever_it_holds() {
    assert_eq!(ok(CUT), Outcome::Cut(usage(100, 40)));
    let with_text = CUT.replace(
        r#""content":[{"type":"thinking","thinking":"","signature":"c2lnbmF0dXJl"}]"#,
        r#""content":[{"type":"text","text":"half a sentence"}]"#,
    );
    assert_eq!(ok(&with_text), Outcome::Cut(usage(100, 40)));
}

#[test]
fn a_refusal_is_read_from_the_stop_reason_and_carries_its_category() {
    assert_eq!(
        ok(REFUSAL),
        Outcome::Refused {
            category: Some("cyber".to_owned()),
            usage: usage(50, 0)
        }
    );
    assert_eq!(
        ok(REFUSAL_NULL),
        Outcome::Refused {
            category: None,
            usage: usage(50, 0)
        }
    );
    assert_eq!(
        ok(&REFUSAL.replace("cyber", "a b")),
        Outcome::Refused {
            category: Some("unknown".to_owned()),
            usage: usage(50, 0)
        }
    );
}

#[test]
fn another_stop_reason_is_a_failed_call_with_its_name() {
    for (reason, code) in [
        ("\"pause_turn\"", Some("pause_turn")),
        ("\"tool_use\"", Some("tool_use")),
        (
            "\"model_context_window_exceeded\"",
            Some("model_context_window_exceeded"),
        ),
    ] {
        let body = format!(
            r#"{{"stop_reason":{reason},"content":[],"usage":{{"input_tokens":5,"output_tokens":2}}}}"#
        );
        assert_eq!(
            ok(&body),
            Outcome::Failed {
                cause: cause(What::StopReason, 200, None, code, None),
                usage: usage(5, 2)
            },
            "{reason}"
        );
    }
}

#[test]
fn a_429_is_busy_for_the_wait_named_and_a_spend_cap_stops() {
    for (header, wait) in [
        (Some("7"), Some(Duration::from_secs(7))),
        (None, None),
        (Some("soon"), None),
    ] {
        let reply = match header {
            Some(value) => Reply::answer(429, ERROR_429).with_header("retry-after", value),
            None => Reply::answer(429, ERROR_429),
        };
        assert_eq!(
            one(reply),
            Outcome::Busy {
                wait,
                cause: cause(
                    What::RateLimit,
                    429,
                    Some("rate_limit_error"),
                    None,
                    Some("req_TESTBODY0429")
                ),
            },
            "{header:?}"
        );
    }
    assert_eq!(
        one(Reply::answer(429, ERROR_429_SPEND)),
        Outcome::Stopped(cause(
            What::Billing,
            429,
            Some("rate_limit_error"),
            Some("enforced_spend_limit_reached"),
            Some("req_TESTBODY0430")
        ))
    );
}

#[test]
fn each_status_is_the_outcome_its_row_says() {
    let bare = |what: What, status: i32| cause(what, status, None, None, None);
    let busy = |what: What, status: i32| Outcome::Busy {
        wait: None,
        cause: bare(what, status),
    };
    let stopped = |what: What, status: i32| Outcome::Stopped(bare(what, status));
    let cases: Vec<(u16, &str, Outcome)> = vec![
        (
            400,
            ERROR_400,
            Outcome::Stopped(cause(
                What::Rejected,
                400,
                Some("invalid_request_error"),
                None,
                Some("req_TESTBODY0400"),
            )),
        ),
        (401, "{}", stopped(What::Unauthorized, 401)),
        (402, "{}", stopped(What::Billing, 402)),
        (403, "{}", stopped(What::Forbidden, 403)),
        (404, "{}", stopped(What::NotFound, 404)),
        (408, "{}", busy(What::Timeout, 408)),
        (409, "{}", busy(What::ServerError, 409)),
        (413, "{}", stopped(What::Rejected, 413)),
        (500, "{}", busy(What::ServerError, 500)),
        (501, "{}", busy(What::ServerError, 501)),
        (502, "{}", busy(What::ServerError, 502)),
        (503, "{}", busy(What::ServerError, 503)),
        (504, "{}", busy(What::Timeout, 504)),
        (520, "{}", busy(What::ServerError, 520)),
        (599, "{}", busy(What::ServerError, 599)),
        (
            529,
            ERROR_529,
            Outcome::Busy {
                wait: None,
                cause: cause(
                    What::Overloaded,
                    529,
                    Some("overloaded_error"),
                    None,
                    Some("req_TESTBODY0529"),
                ),
            },
        ),
        (
            302,
            "",
            Outcome::Failed {
                cause: bare(What::Unreadable, 302),
                usage: Usage::default(),
            },
        ),
    ];
    for (status, body, expected) in cases {
        assert_eq!(one(Reply::answer(status, body)), expected, "{status}");
    }
}

#[test]
fn a_busy_status_takes_the_wait_the_response_names() {
    for (status, header, wait) in [
        (503, Some("7"), Some(Duration::from_secs(7))),
        (503, Some("1.5"), Some(Duration::from_secs(2))),
        (503, None, None),
        (529, Some("3"), Some(Duration::from_secs(3))),
    ] {
        let reply = match header {
            Some(value) => Reply::answer(status, "{}").with_header("retry-after", value),
            None => Reply::answer(status, "{}"),
        };
        let Outcome::Busy { wait: given, .. } = one(reply) else {
            panic!("{status} {header:?}");
        };
        assert_eq!(given, wait, "{status} {header:?}");
    }
}

#[test]
fn an_error_names_its_status_type_and_request_and_never_its_message() {
    let with_header =
        one(Reply::answer(404, ERROR_404).with_header("request-id", "req_HEADER0404"));
    assert_eq!(
        with_header,
        Outcome::Stopped(cause(
            What::NotFound,
            404,
            Some("not_found_error"),
            None,
            Some("req_HEADER0404")
        ))
    );
    let without = one(Reply::answer(404, ERROR_404));
    assert_eq!(
        without,
        Outcome::Stopped(cause(
            What::NotFound,
            404,
            Some("not_found_error"),
            None,
            Some("req_TESTBODY0404")
        ))
    );
    for outcome in [with_header, without] {
        assert!(!format!("{outcome:?}").contains("CANARY"));
    }
    assert_eq!(
        one(Reply::answer(404, ERROR_404).with_header("request-id", "req HEADER")),
        Outcome::Stopped(cause(
            What::NotFound,
            404,
            Some("not_found_error"),
            None,
            Some("req_TESTBODY0404")
        ))
    );
}

#[test]
fn a_response_bilbo_cannot_read_is_a_failed_call() {
    for (body, what) in [
        ("not json", What::Unreadable),
        ("[]", What::Unreadable),
        (r#"{"content":[]}"#, What::Unreadable),
    ] {
        assert_eq!(
            ok(body),
            Outcome::Failed {
                cause: cause(what, 200, None, None, None),
                usage: Usage::default()
            },
            "{body}"
        );
    }
}

#[test]
fn a_response_over_the_limit_is_a_failed_call() {
    let provider = Provider::start(vec![Reply::answer(200, ANSWER), Reply::answer(200, ANSWER)]);
    let mut writer = ready(&config(&provider.url(), "", ""));
    let length = ANSWER.len() as u64;
    assert!(matches!(
        writer.call(
            &prompt(),
            &Limits {
                response: length,
                ..limits()
            }
        ),
        Outcome::Answer { .. }
    ));
    assert_eq!(
        writer.call(
            &prompt(),
            &Limits {
                response: length - 1,
                ..limits()
            }
        ),
        Outcome::Failed {
            cause: Cause::new(What::TooLarge),
            usage: Usage::default()
        }
    );
}

#[test]
fn no_answer_in_time_and_no_connection_are_busy() {
    let stalled = Provider::start(vec![Reply::Stall]);
    let mut writer = ready(&config(&stalled.url(), "", ""));
    assert_eq!(
        writer.call(
            &prompt(),
            &Limits {
                timeout: Duration::from_secs(1),
                ..limits()
            }
        ),
        Outcome::Busy {
            wait: None,
            cause: Cause::new(What::Timeout)
        }
    );
    let gone = Provider::start(Vec::new());
    let mut writer = ready(&config(&gone.url(), "", ""));
    drop(gone);
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Busy {
            wait: None,
            cause: Cause::new(What::NoConnection)
        }
    );
    let mut writer = ready(&config(&crate::untrusted_address(), "", ""));
    assert_eq!(
        writer.call(&prompt(), &limits()),
        Outcome::Stopped(Cause::new(What::Tls))
    );
}

#[test]
fn prepare_reads_the_key_once_and_refuses_what_is_missing() {
    let scratch = Scratch::new("anthropic");
    let url = "http://127.0.0.1:9";
    let no_model = config(url, "", "").replace("model = \"test-model-1\"", "model = \"\"");
    assert_eq!(
        writer_with(&no_model, scratch.path(), variables)
            .prepare()
            .unwrap_err(),
        "llm.anthropic.model is not set"
    );
    assert_eq!(
        writer_with(&config(url, "", ""), scratch.path(), |_| None)
            .prepare()
            .unwrap_err(),
        "ANTHROPIC_API_KEY is not set (llm.anthropic.api.key_env)"
    );
    let counter = scratch.path().join("runs");
    let script = scratch.script(
        "key.sh",
        &format!("echo run >> {}\necho {KEY}", counter.display()),
    );
    let command = format!("key_command = [\"/bin/sh\", \"{}\"]", script.display());
    let provider = Provider::start(vec![Reply::answer(200, ANSWER), Reply::answer(200, ANSWER)]);
    let mut writer = writer_with(
        &config(&provider.url(), &command, ""),
        &scratch.path().join("derive"),
        |_| None,
    );
    let before = writer.call(&prompt(), &limits());
    assert_eq!(before, Outcome::Stopped(Cause::new(What::NotStarted)));
    assert!(provider.requests().is_empty());
    writer.prepare().expect("the program's key");
    writer.call(&prompt(), &limits());
    writer.call(&prompt(), &limits());
    assert_eq!(
        std::fs::read_to_string(&counter)
            .expect("runs")
            .lines()
            .count(),
        1
    );
    let seen = provider.requests();
    assert_eq!(seen[0].header("x-api-key"), [KEY]);
}

#[test]
fn the_key_is_in_one_header_and_in_nothing_else_the_run_gives() {
    let provider = Provider::start(vec![
        Reply::answer(200, ANSWER),
        Reply::answer(404, ERROR_404),
        Reply::answer(200, REFUSAL),
    ]);
    let mut writer = ready(&config(&provider.url(), "", ""));
    let outcomes = [(); 3].map(|()| writer.call(&prompt(), &limits()));
    for request in provider.requests() {
        let values: Vec<_> = request
            .headers
            .iter()
            .filter(|(_, value)| value.contains(KEY))
            .collect();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0.to_ascii_lowercase(), "x-api-key");
        assert!(request.headers.iter().all(|(name, _)| !name.contains(KEY)));
        assert!(!request.path.contains(KEY));
        assert!(!String::from_utf8_lossy(&request.body).contains(KEY));
    }
    for outcome in &outcomes {
        assert!(!format!("{outcome:?}").contains(KEY));
    }
    assert!(writer.describe().iter().all(|line| !line.contains(KEY)));
    assert!(!format!("{writer:?}").contains("PLUM"));
    let spaced = |name: &str| (name == "ANTHROPIC_API_KEY").then(|| format!(" {KEY}\t"));
    let second = Provider::start(vec![Reply::answer(200, ANSWER)]);
    let mut trimmed = writer_with(
        &config(&second.url(), "", ""),
        &std::env::temp_dir(),
        spaced,
    );
    trimmed.prepare().expect("the key is read");
    trimmed.call(&prompt(), &limits());
    let seen = second.requests();
    assert_eq!(
        sent(&seen[0], &["x-api-key"]),
        [("x-api-key".to_owned(), KEY.to_owned())]
    );
    assert_eq!(
        writer.describe(),
        [
            format!(
                "provider: anthropic, via api, model test-model-1, at {}/v1/messages",
                provider.url()
            ),
            "version: 2026-10-21".to_owned(),
            "key: the variable ANTHROPIC_API_KEY, which is set".to_owned(),
        ]
    );
}

fn holds_key(reply: Reply, status: i32) {
    let outcome = one(reply);
    assert_eq!(
        outcome,
        Outcome::Failed {
            cause: cause(What::HoldsKey, status, None, None, None),
            usage: Usage::default()
        }
    );
    let text = format!("{outcome:?}");
    assert!(!text.contains(KEY) && !text.contains("carrot"), "{text}");
}

#[test]
fn a_model_that_is_the_key_is_a_failed_call() {
    holds_key(
        Reply::answer(200, &ANSWER.replace("test-model-1", KEY)),
        200,
    );
}

#[test]
fn a_category_that_is_the_key_is_a_failed_call() {
    holds_key(Reply::answer(200, &REFUSAL.replace("cyber", KEY)), 200);
}

#[test]
fn an_error_type_that_is_the_key_is_a_failed_call() {
    holds_key(
        Reply::answer(400, &ERROR_400.replace("invalid_request_error", KEY)),
        400,
    );
}

#[test]
fn an_error_code_that_is_the_key_is_a_failed_call() {
    let body = format!(
        r#"{{"error":{{"type":"invalid_request_error","details":{{"error_code":"{KEY}"}}}}}}"#
    );
    holds_key(Reply::answer(400, &body), 400);
}

#[test]
fn a_request_id_that_is_the_key_is_a_failed_call() {
    holds_key(
        Reply::answer(400, ERROR_400).with_header("request-id", KEY),
        400,
    );
    holds_key(
        Reply::answer(400, &ERROR_400.replace("req_TESTBODY0400", KEY)),
        400,
    );
}

#[test]
fn a_key_found_in_a_providers_error_keeps_the_providers_cause() {
    assert_eq!(
        one(Reply::answer(401, "{}").with_header("request-id", KEY)),
        Outcome::Stopped(cause(What::Unauthorized, 401, None, None, None))
    );
    assert_eq!(
        one(Reply::answer(503, "{}").with_header("request-id", KEY)),
        Outcome::Busy {
            wait: None,
            cause: cause(What::ServerError, 503, None, None, None)
        }
    );
    let typed = ERROR_404.replace("not_found_error", KEY);
    let outcome = one(Reply::answer(404, &typed));
    assert_eq!(
        outcome,
        Outcome::Stopped(cause(
            What::NotFound,
            404,
            None,
            None,
            Some("req_TESTBODY0404")
        ))
    );
    assert!(!format!("{outcome:?}").contains("PLUM"));
    holds_key(
        Reply::answer(200, &ANSWER.replace("test-model-1", KEY)),
        200,
    );
}

#[test]
fn a_200_with_no_stop_reason_is_unreadable() {
    assert_eq!(
        ok(NO_STOP_REASON),
        Outcome::Failed {
            cause: cause(What::Unreadable, 200, None, None, None),
            usage: usage(123, 40)
        }
    );
}

#[test]
fn a_stop_reason_that_is_the_key_is_a_failed_call() {
    holds_key(Reply::answer(200, &ANSWER.replace("end_turn", KEY)), 200);
}

#[test]
fn an_answer_that_holds_the_key_is_a_failed_call() {
    let beside_a_tool = blocks(
        &format!(r#"{{"type":"text","text":"see {KEY} for the carrot"}},{{"type":"tool_use"}}"#),
        r#"{"input_tokens":5,"output_tokens":2}"#,
    );
    holds_key(Reply::answer(200, &beside_a_tool), 200);
    holds_key(
        Reply::answer(
            200,
            &ANSWER.replace("The user asked", &format!("see {KEY} for the carrot")),
        ),
        200,
    );
}

#[test]
fn a_setting_that_holds_the_key_is_refused_by_its_keys_name_and_nothing_is_sent() {
    let scratch = Scratch::new("anthropic");
    let provider = Provider::start(vec![Reply::answer(200, ANSWER)]);
    let url = provider.url();
    let with_model = |model: &str| config(&url, "", "").replace("test-model-1", model);
    let extra =
        |text: &str| format!("[llm.anthropic.api.extra]\nmetadata = {{ user_id = \"{text}\" }}\n");
    let script = scratch.script("key.sh", &format!("echo {KEY}"));
    let command = format!("key_command = [\"/bin/sh\", \"{}\"]", script.display());
    let cases: Vec<(String, Reader, &str)> = vec![
        (with_model(KEY), variables, "model"),
        (with_model("gw/sk-test-PLUM-7391/x"), variables, "model"),
        (
            config(&url, &format!("version = \"{KEY}\""), ""),
            variables,
            "api.version",
        ),
        (
            config(
                &url,
                "",
                "[llm.anthropic.api.headers]\nx-trace = \"id sk-test-PLUM-7391\"\n",
            ),
            variables,
            "api.headers",
        ),
        (
            config(
                &url,
                "",
                &format!("[llm.anthropic.api.headers]\n{KEY} = \"1\"\n"),
            ),
            variables,
            "api.headers",
        ),
        (config(&url, "", &extra(KEY)), variables, "api.extra"),
        (
            config(
                &url,
                "",
                &format!(
                    "[llm.anthropic.api.headers]\nx-trace = \"{KEY}\"\n{}",
                    extra(KEY)
                ),
            ),
            variables,
            "api.headers",
        ),
        (
            config(
                &url,
                &format!("version = \"{KEY}\""),
                "[llm.anthropic.api.headers]\nx-trace = \"sk-test-PLUM-7391\"\n",
            ),
            variables,
            "api.version",
        ),
        (
            with_model(KEY),
            |name| (name == "ANTHROPIC_API_KEY").then(|| format!(" {KEY}\t")),
            "model",
        ),
        (
            format!("{}{}", with_model(KEY), extra(KEY)),
            variables,
            "model",
        ),
        (
            config(&url, &command, "").replace("test-model-1", KEY),
            |_| None,
            "model",
        ),
    ];
    for (text, reader, key) in cases {
        let mut writer = writer_with(&text, scratch.path(), reader);
        assert_eq!(
            writer.prepare().unwrap_err(),
            format!("llm.anthropic.{key} holds the key")
        );
        let after = writer.call(&prompt(), &limits());
        assert_eq!(after, Outcome::Stopped(Cause::new(What::NotStarted)));
    }
    let near = format!(
        "{}{}",
        with_model("sk-test-PLUM-739"),
        extra("sk-test-PLUM")
    );
    let mut writer = writer_with(&near, scratch.path(), variables);
    writer.prepare().expect("no setting holds the whole key");
    assert!(matches!(
        writer.call(&prompt(), &limits()),
        Outcome::Answer { .. }
    ));
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn a_model_that_holds_the_key_is_said_by_the_key_that_named_it_and_never_by_its_value() {
    let scratch = Scratch::new("anthropic-asked");
    let asked = |text: &str, model: &str| {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let section = bilbo_llm::anthropic::section(&Source {
            var: &|_| None,
            file: &file,
        })
        .expect("a section")
        .asking("derive.tags.model", model);
        Anthropic::new(
            section.api,
            Box::new(variables),
            scratch.path().to_path_buf(),
            Duration::from_secs(5),
            1 << 20,
            Duration::from_millis(5),
        )
    };
    let url = "http://127.0.0.1:9";
    for model in [KEY, "gw/sk-test-PLUM-7391/x"] {
        let said = asked(&config(url, "", ""), model).prepare().unwrap_err();
        assert_eq!(said, "derive.tags.model holds the key");
        assert!(!said.contains(model) && !said.contains("PLUM"), "{said}");
    }
    let header = config(
        url,
        "",
        "[llm.anthropic.api.headers]\nx-trace = \"id sk-test-PLUM-7391\"\n",
    );
    assert_eq!(
        asked(&header, "m2").prepare().unwrap_err(),
        "llm.anthropic.api.headers holds the key"
    );
    assert_eq!(
        asked(&config(url, "", ""), "").prepare().unwrap_err(),
        "derive.tags.model is not set"
    );
}

#[test]
fn describe_says_the_settings_in_force_and_how_the_key_is_read() {
    let tables = "[llm.anthropic.api.headers]\nX-A = \"1\"\nX-B = \"2\"\n\n[llm.anthropic.api.extra]\noutput_config = { effort = \"low\" }\n";
    let url = "http://127.0.0.1:9";
    let writer = writer_with(
        &config(url, "key_command = [\"op\", \"read\"]", tables),
        &std::env::temp_dir(),
        |_| None,
    );
    assert_eq!(writer.id(), "test-model-1");
    assert_eq!(
        writer.describe(),
        [
            "provider: anthropic, via api, model test-model-1, at http://127.0.0.1:9/v1/messages",
            "version: 2026-10-21",
            "headers: { X-A = \"1\", X-B = \"2\" }",
            "extra: { output_config = { effort = \"low\" } }",
            "key: the program [\"op\", \"read\"], not run",
        ]
    );
    let writer = writer_with(
        &config(url, "key_env = \"MY_KEY\"", ""),
        &std::env::temp_dir(),
        |_| None,
    );
    assert_eq!(
        writer.describe().last().map(String::as_str),
        Some("key: the variable MY_KEY, which is not set")
    );
}

#[test]
fn a_key_split_over_two_parts_of_an_answer_is_not_kept() {
    let text = |text: &str| format!(r#"{{"type":"text","text":"{text}"}}"#);
    let body = blocks(
        &[text("see sk-test-"), text("PLUM-7391 here")].join(","),
        r#"{"input_tokens":5,"output_tokens":2}"#,
    );
    holds_key(Reply::answer(200, &body), 200);
}

#[test]
fn the_guard_reads_the_model_before_the_version() {
    let scratch = Scratch::new("anthropic");
    let text = config("http://127.0.0.1:9", &format!("version = \"{KEY}\""), "")
        .replace("test-model-1", KEY);
    let mut writer = writer_with(&text, scratch.path(), variables);
    assert_eq!(
        writer.prepare().unwrap_err(),
        "llm.anthropic.model holds the key"
    );
}
