use std::time::{Duration, Instant};

use bilbo_derive::http::{self, Endpoint, Fault, Response};
use bilbo_provider::{Provider, Reply};

const WAIT: Duration = Duration::from_secs(5);

fn at(provider: &Provider, path: &str) -> Endpoint {
    http::endpoint(&format!("{}{path}", provider.url())).expect("an endpoint")
}

fn plain(provider: &Provider) -> Result<Response, Fault> {
    http::post(&at(provider, "/"), &[], b"{}", &[], WAIT, 1024)
}

#[test]
fn a_status_that_is_no_success_comes_back_with_its_body() {
    let provider = Provider::start(vec![
        Reply::answer(404, "no such model"),
        Reply::answer(500, "boom"),
    ]);
    let first = plain(&provider).unwrap();
    assert_eq!((first.status, first.body), (404, b"no such model".to_vec()));
    let second = plain(&provider).unwrap();
    assert_eq!((second.status, second.body), (500, b"boom".to_vec()));

    let redirect = Provider::start(vec![
        Reply::answer(302, "").with_header("location", "/elsewhere"),
        Reply::answer(200, "followed"),
    ]);
    let response = http::post(&at(&redirect, "/"), &[], b"{}", &["location"], WAIT, 1024).unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(
        response.headers,
        [("location".to_owned(), "/elsewhere".to_owned())]
    );
    assert_eq!(redirect.requests().len(), 1);
}

#[test]
fn the_headers_and_the_body_are_sent_as_given() {
    let provider = Provider::start(vec![Reply::answer(200, "ok"), Reply::answer(200, "ok")]);
    let given = [
        ("x-api-key", "k-123"),
        ("content-type", "application/json"),
        ("anthropic-version", "2026-10-21"),
        ("x-last", "z"),
    ];
    let body = br#"{"model":"m","n":1}"#;
    http::post(
        &at(&provider, "/v1/messages"),
        &given,
        body,
        &[],
        WAIT,
        1024,
    )
    .unwrap();
    http::post(
        &at(&provider, "/v1/messages"),
        &given[..1],
        body,
        &[],
        WAIT,
        1024,
    )
    .unwrap();
    let seen = provider.requests();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/v1/messages");
    let sent: Vec<(String, String)> = seen[0]
        .headers
        .iter()
        .filter(|(name, _)| {
            given
                .iter()
                .any(|(wanted, _)| wanted.eq_ignore_ascii_case(name))
        })
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    let wanted: Vec<(String, String)> = given
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(sent, wanted);
    assert_eq!(seen[0].body, body);
    assert!(seen[1].header("content-type").is_empty());
}

#[test]
fn the_user_agent_is_bilbo_unless_one_is_given() {
    let provider = Provider::start(vec![Reply::answer(200, ""), Reply::answer(200, "")]);
    http::post(&at(&provider, "/"), &[], b"{}", &[], WAIT, 1024).unwrap();
    http::post(
        &at(&provider, "/"),
        &[("User-Agent", "other")],
        b"{}",
        &[],
        WAIT,
        1024,
    )
    .unwrap();
    let seen = provider.requests();
    assert_eq!(seen[0].header("user-agent"), ["bilbo"]);
    assert_eq!(seen[1].header("user-agent"), ["other"]);
}

#[test]
fn one_timeout_bounds_the_whole_call() {
    let stalled = Provider::start(vec![Reply::Stall]);
    let start = Instant::now();
    let fault = http::post(
        &at(&stalled, "/"),
        &[],
        b"{}",
        &[],
        Duration::from_millis(300),
        1024,
    )
    .unwrap_err();
    assert_eq!(fault, Fault::Timeout);
    assert!(start.elapsed() < Duration::from_secs(2));

    let trickle = Provider::start(vec![Reply::trickle(
        &"x".repeat(1000),
        Duration::from_millis(50),
    )]);
    let start = Instant::now();
    let fault = http::post(
        &at(&trickle, "/"),
        &[],
        b"{}",
        &[],
        Duration::from_millis(400),
        4096,
    )
    .unwrap_err();
    assert_eq!(fault, Fault::Timeout);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_body_over_the_limit_is_its_own_fault_and_one_at_the_limit_is_read() {
    let body = "y".repeat(100);
    let provider = Provider::start(vec![
        Reply::answer(200, &body),
        Reply::answer(200, &body),
        Reply::answer(200, &body),
    ]);
    let post = |limit| http::post(&at(&provider, "/"), &[], b"{}", &[], WAIT, limit);
    assert_eq!(post(99).unwrap_err(), Fault::TooLarge);
    assert_eq!(post(100).unwrap().body, body.as_bytes());
    assert_eq!(post(101).unwrap().body, body.as_bytes());
}

#[test]
fn the_headers_asked_for_come_back_whatever_their_case() {
    let provider = Provider::start(vec![Reply::answer(429, "").with_header("Retry-After", "7")]);
    let response = http::post(
        &at(&provider, "/"),
        &[],
        b"{}",
        &["retry-after", "x-none"],
        WAIT,
        1024,
    )
    .unwrap();
    assert_eq!(
        response.headers,
        [("retry-after".to_owned(), "7".to_owned())]
    );
}

#[test]
fn a_port_nobody_listens_on_is_no_connection() {
    let provider = Provider::start(Vec::new());
    let endpoint = at(&provider, "/");
    drop(provider);
    assert_eq!(
        http::post(&endpoint, &[], b"{}", &[], WAIT, 1024).unwrap_err(),
        Fault::NoConnection
    );
}

#[test]
fn a_handshake_the_client_cannot_finish_is_a_connection_that_could_not_be_made_secure() {
    let endpoint = http::endpoint(&crate::untrusted_address()).expect("an https endpoint");
    assert_eq!(
        http::post(&endpoint, &[], b"{}", &[], WAIT, 1024).unwrap_err(),
        Fault::Untrusted
    );
}
