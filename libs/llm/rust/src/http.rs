use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use ureq::http::Uri;
use ureq::{Agent, Error};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    url: String,
    plain: bool,
}

impl Endpoint {
    pub fn as_str(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    Timeout,
    NoConnection,
    Untrusted,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[must_use]
    pub fn wait(&self) -> Option<Duration> {
        self.header("retry-after").and_then(wait)
    }
}

#[must_use]
pub fn wait(text: &str) -> Option<Duration> {
    let (whole, fraction) = match text.trim().split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text.trim(), None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|part| !digits(part)) {
        return None;
    }
    let seconds = whole.parse::<u64>().unwrap_or(u64::MAX);
    let part = fraction.is_some_and(|part| part.bytes().any(|byte| byte != b'0'));
    Some(Duration::from_secs(seconds.saturating_add(u64::from(part))))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Form,
    UserName,
    Localhost,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Form => "must be an https URL, or an http URL at 127.0.0.1 or [::1]",
            Self::UserName => "must hold no user name or password, got one",
            Self::Localhost => "must name the address in place of localhost: 127.0.0.1",
        })
    }
}

impl std::error::Error for Refusal {}

fn has_user_name(value: &str) -> bool {
    value.contains('@')
}

pub fn endpoint(value: &str) -> Result<Endpoint, Refusal> {
    if has_user_name(value) {
        return Err(Refusal::UserName);
    }
    let uri: Uri = value.parse().map_err(|_| Refusal::Form)?;
    let plain = match uri.scheme_str() {
        Some(scheme) if scheme.eq_ignore_ascii_case("https") => false,
        Some(scheme) if scheme.eq_ignore_ascii_case("http") => true,
        _ => return Err(Refusal::Form),
    };
    let authority = uri.authority().ok_or(Refusal::Form)?;
    let host = authority.host();
    let port = authority.as_str().strip_prefix(host).unwrap_or_default();
    if let Some(port) = port.strip_prefix(':')
        && !port.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Refusal::Form);
    }
    if plain {
        if host.eq_ignore_ascii_case("localhost") {
            return Err(Refusal::Localhost);
        }
        let loopback = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
        if !loopback {
            return Err(Refusal::Form);
        }
    }
    Ok(Endpoint {
        url: value.to_owned(),
        plain,
    })
}

pub fn post(
    endpoint: &Endpoint,
    headers: &[(&str, &str)],
    body: &[u8],
    wanted: &[&str],
    timeout: Duration,
    limit: u64,
) -> Result<Response, Fault> {
    let mut config = Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout));
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        config = config.user_agent("bilbo");
    }
    if endpoint.plain {
        // The key is in the request, and a proxy named by the environment would see it in clear.
        config = config.proxy(None);
    }
    let agent = Agent::new_with_config(config.build());
    let mut request = agent.post(&endpoint.url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request.send(body).map_err(|error| fault(&error))?;
    let status = response.status().as_u16();
    let found = wanted
        .iter()
        .filter_map(|name| {
            let value = response.headers().get(*name)?.to_str().ok()?;
            Some(((*name).to_owned(), value.to_owned()))
        })
        .collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| fault(&error))?;
    Ok(Response {
        status,
        headers: found,
        body,
    })
}

fn fault(error: &Error) -> Fault {
    match error {
        Error::Timeout(_) => Fault::Timeout,
        Error::BodyExceedsLimit(_) => Fault::TooLarge,
        Error::Tls(_) | Error::Rustls(_) | Error::Pem(_) => Fault::Untrusted,
        Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData => Fault::Untrusted,
        _ => Fault::NoConnection,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use bilbo_settings::file::File;
    use bilbo_settings::key::{About, Default, Key, Source};

    use super::{Refusal, endpoint, wait};

    #[test]
    fn an_https_endpoint_is_taken_whatever_its_host() {
        for value in [
            "https://example.com/v1/messages",
            "https://example.com",
            "https://192.168.1.2:8443/x",
            "HTTPS://Example.COM/",
        ] {
            assert_eq!(endpoint(value).unwrap().as_str(), value);
        }
    }

    #[test]
    fn a_plain_http_endpoint_is_taken_only_at_a_literal_loopback_address() {
        for value in [
            "http://127.0.0.1:8080/x",
            "http://127.9.9.9/",
            "http://[::1]:9/",
        ] {
            assert_eq!(endpoint(value).unwrap().as_str(), value);
        }
        for value in [
            "http://example.com",
            "http://0.0.0.0",
            "http://192.168.1.2",
            "http://127.0.0.1.example.com",
            "http://[::2]",
        ] {
            assert_eq!(endpoint(value).unwrap_err(), Refusal::Form);
        }
    }

    #[test]
    fn localhost_is_refused_with_the_address_to_write() {
        let refusal = endpoint("http://localhost:8080/v1").unwrap_err();
        assert_eq!(refusal, Refusal::Localhost);
        assert_eq!(
            refusal.to_string(),
            "must name the address in place of localhost: 127.0.0.1"
        );
    }

    #[test]
    fn a_text_that_is_no_http_url_or_holds_a_password_is_refused() {
        for value in ["ftp://x", "127.0.0.1:80", "", "ftp://127.0.0.1:9/"] {
            let refusal = endpoint(value).unwrap_err();
            assert_eq!(refusal, Refusal::Form, "{value}");
            assert_eq!(
                refusal.to_string(),
                "must be an https URL, or an http URL at 127.0.0.1 or [::1]"
            );
        }
        for value in [
            "https://u:p@example.com",
            "ftp://u:p@example.com",
            "u:p@example.com",
            "http://u@localhost:1",
            "//u:p@example.com",
            "//u@example.com/x",
            "//example.com/a@b",
            "https://example.com/a@b",
            "https://bob:pa/ss@gw.example/v1",
            "https://bob:p?w@gw.example/v1",
        ] {
            let refusal = endpoint(value).unwrap_err();
            assert_eq!(refusal, Refusal::UserName, "{value}");
            assert_eq!(
                refusal.to_string(),
                "must hold no user name or password, got one"
            );
        }
    }

    #[test]
    fn an_endpoint_that_holds_an_at_sign_is_refused() {
        for value in [
            "https://bob:123/x@host/v1",
            "https://:1/x@host",
            "https://example.com/a@b",
            "https://u@h",
            "https://bob:2024?x@127.0.0.1/v1",
            "https://bob:2024#x@127.0.0.1/v1",
            "https://example.com?x=a@b",
            "https://example.com#a@b",
        ] {
            let refusal = endpoint(value).unwrap_err();
            assert_eq!(refusal, Refusal::UserName, "{value}");
            assert_eq!(
                refusal.to_string(),
                "must hold no user name or password, got one"
            );
        }
        let file = File::parse(
            Path::new("/c/config.toml"),
            b"[derive.test]\nkey = \"https://bob:123/SECRETPART@host/v1\"\n",
        )
        .expect("toml");
        let key = Key {
            about: About {
                name: "derive.test.key",
                variable: Some("BILBO_TEST_KEY"),
                default: Default::Toml(r#""https://example.com""#),
                allowed: "anything",
                doc: "A key of the test's own.",
            },
            parse: crate::api::endpoint,
            show: |_| String::new(),
        };
        let none = |_: &str| None;
        let error = key
            .resolve(&Source {
                var: &none,
                file: &file,
            })
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "/c/config.toml:2: derive.test.key must hold no user name or password, got one"
        );
        let variable = |name: &str| {
            (name == "BILBO_TEST_KEY").then(|| OsString::from("https://bob:123/SECRETPART@host/v1"))
        };
        let error = key
            .resolve(&Source {
                var: &variable,
                file: &File::parse(Path::new("/c/config.toml"), b"").expect("toml"),
            })
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "BILBO_TEST_KEY must hold no user name or password, got one"
        );
    }

    #[test]
    fn a_wait_is_whole_seconds_taken_up() {
        let nines = "9".repeat(30);
        for (text, seconds) in [
            ("7", 7),
            (" 7 ", 7),
            ("0", 0),
            ("1.0", 1),
            ("1.5", 2),
            ("0.2", 1),
            (nines.as_str(), u64::MAX),
        ] {
            assert_eq!(wait(text), Some(Duration::from_secs(seconds)), "{text}");
        }
        for text in ["Wed, 21 Oct 2026 07:28:00 GMT", "-1", "1e3", ".5", "", "1."] {
            assert_eq!(wait(text), None, "{text}");
        }
    }
}
