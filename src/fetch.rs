use std::io::Read;
use std::time::Duration;

use ureq::ResponseExt;

const ACCEPT: &str =
    "text/html, application/xhtml+xml, text/markdown;q=0.9, text/plain;q=0.8, */*;q=0.1";
const MAX_BODY: u64 = 16 * 1024 * 1024;
const MAX_REDIRECTS: u32 = 10;
const CONNECT: Duration = Duration::from_secs(30);
const TOTAL: Duration = Duration::from_secs(60);

/// What a server answered after redirects: where it landed, its status, its media type essence
/// (lowercased) and the body, gzip-decoded.
pub struct Answer {
    pub final_url: String,
    pub status: u16,
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
    /// True when at least one redirect was followed.
    pub redirected: bool,
}

#[derive(Debug, PartialEq)]
pub enum Kind {
    Html,
    Text,
    Pdf,
    Other(String),
}

/// `GET url`, following up to 10 redirects. Any status outside 2xx, a body over 16 MiB and every
/// transport failure is a message naming `url`.
pub fn get(url: &str) -> Result<Answer, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT))
        .timeout_global(Some(TOTAL))
        .max_redirects(MAX_REDIRECTS)
        .save_redirect_history(true)
        .user_agent(concat!("bilbo/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", ACCEPT)
        .call()
        .map_err(|e| describe(url, e))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("{url} answered {status}"));
    }
    let final_url = response.get_uri().to_string();
    let redirected = response
        .get_redirect_history()
        .is_some_and(|history| history.len() > 1);
    let media_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|v| !v.is_empty());
    let mut bytes = Vec::new();
    response
        .body_mut()
        .with_config()
        .reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| match ureq::Error::from(e) {
            ureq::Error::Io(e) => format!("{url} failed: {e}"),
            e => describe(url, e),
        })?;
    if bytes.len() as u64 > MAX_BODY {
        return Err(over_limit(url));
    }
    Ok(Answer {
        final_url,
        status,
        media_type,
        bytes,
        redirected,
    })
}

fn describe(url: &str, error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => format!("{url} answered {code}"),
        ureq::Error::Timeout(ureq::Timeout::Connect) => {
            format!("{url} did not answer within {} seconds", CONNECT.as_secs())
        }
        ureq::Error::Timeout(_) => {
            format!("{url} did not answer within {} seconds", TOTAL.as_secs())
        }
        ureq::Error::TooManyRedirects => {
            format!("{url} needed more than {MAX_REDIRECTS} redirects")
        }
        ureq::Error::BodyExceedsLimit(_) => over_limit(url),
        ureq::Error::Io(e) => format!("{url} is unreachable: {e}"),
        ureq::Error::HostNotFound => format!("{url} is unreachable: host not found"),
        ureq::Error::ConnectionFailed => format!("{url} is unreachable: connection failed"),
        e => format!("{url} failed: {e}"),
    }
}

fn over_limit(url: &str) -> String {
    format!("{url} is over the {} MiB limit", MAX_BODY / (1024 * 1024))
}

/// How the converter treats an answer: the media type decides; without one the first bytes do.
pub fn kind(media_type: Option<&str>, bytes: &[u8]) -> Kind {
    match media_type {
        Some(m) => {
            let m = m
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            match m.as_str() {
                "text/html" | "application/xhtml+xml" => Kind::Html,
                "application/pdf" => Kind::Pdf,
                _ if m.starts_with("text/") => Kind::Text,
                _ => Kind::Other(m),
            }
        }
        None => {
            let head: Vec<u8> = bytes
                .strip_prefix(b"\xEF\xBB\xBF")
                .unwrap_or(bytes)
                .iter()
                .skip_while(|b| b.is_ascii_whitespace())
                .take(14)
                .map(u8::to_ascii_lowercase)
                .collect();
            if head.starts_with(b"<!doctype html") || head.starts_with(b"<html") {
                Kind::Html
            } else {
                Kind::Text
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn media_type_decides() {
        assert_eq!(kind(Some("text/html"), b"# md"), Kind::Html);
        assert_eq!(kind(Some("application/xhtml+xml"), b""), Kind::Html);
        assert_eq!(kind(Some("TEXT/HTML; charset=utf-8"), b""), Kind::Html);
        assert_eq!(kind(Some("text/markdown"), b"<html>"), Kind::Text);
        assert_eq!(kind(Some("text/csv"), b"a,b"), Kind::Text);
        assert_eq!(kind(Some("application/pdf"), b"%PDF"), Kind::Pdf);
        assert_eq!(
            kind(Some("image/png"), b"\x89PNG"),
            Kind::Other("image/png".into())
        );
    }

    #[test]
    fn sniffs_without_a_type() {
        assert_eq!(kind(None, b"  \n<!DOCTYPE html><p>"), Kind::Html);
        assert_eq!(kind(None, b"<HTML lang=en>"), Kind::Html);
        assert_eq!(kind(None, b"# Title\n\ntext"), Kind::Text);
        assert_eq!(kind(None, b""), Kind::Text);
        assert_eq!(kind(None, b"\xEF\xBB\xBF<!doctype html><p>"), Kind::Html);
    }

    /// Serves `replies` in order, one per connection; returns the base URL.
    fn serve(replies: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(&reply);
            }
        });
        base
    }

    fn reply(head: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn answers_a_page() {
        let base = serve(vec![reply(
            "200 OK\r\nContent-Type: Text/HTML; charset=utf-8",
            b"<p>hi",
        )]);
        let answer = get(&format!("{base}/a")).unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.final_url, format!("{base}/a"));
        assert_eq!(answer.media_type.as_deref(), Some("text/html"));
        assert_eq!(answer.bytes, b"<p>hi");
    }

    #[test]
    fn refuses_a_status() {
        let base = serve(vec![reply("404 Not Found", b"no")]);
        let url = format!("{base}/x");
        assert_eq!(get(&url).err().unwrap(), format!("{url} answered 404"));
    }

    #[test]
    fn follows_redirects_to_the_final_url() {
        let base = serve(vec![
            reply("301 Moved\r\nLocation: /b", b""),
            reply("302 Found\r\nLocation: /c", b""),
            reply("200 OK\r\nContent-Type: text/plain", b"ok"),
        ]);
        let answer = get(&format!("{base}/a")).unwrap();
        assert_eq!(answer.final_url, format!("{base}/c"));
        assert_eq!(answer.bytes, b"ok");
    }

    #[test]
    fn refuses_more_than_ten_redirects() {
        let hop = reply("301 Moved\r\nLocation: /again", b"");
        let base = serve(vec![hop; 11]);
        let url = format!("{base}/a");
        assert_eq!(
            get(&url).err().unwrap(),
            format!("{url} needed more than 10 redirects")
        );
    }

    #[test]
    fn a_body_of_exactly_the_limit_passes() {
        let body = vec![b'a'; MAX_BODY as usize];
        let base = serve(vec![reply("200 OK\r\nContent-Type: text/plain", &body)]);
        assert_eq!(
            get(&format!("{base}/edge")).unwrap().bytes.len(),
            body.len()
        );
    }

    #[test]
    fn a_gzip_body_is_limited_after_decoding() {
        let gz: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/gzip-over-limit.gz"
        ));
        let base = serve(vec![reply(
            "200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip",
            gz,
        )]);
        let url = format!("{base}/gz");
        assert_eq!(
            get(&url).err().unwrap(),
            format!("{url} is over the 16 MiB limit")
        );
    }

    #[test]
    fn ten_redirects_then_a_page_succeeds() {
        let mut replies = vec![reply("301 Moved\r\nLocation: /next", b""); 10];
        replies.push(reply("200 OK\r\nContent-Type: text/plain", b"ok"));
        let base = serve(replies);
        assert_eq!(get(&format!("{base}/a")).unwrap().bytes, b"ok");
    }

    #[test]
    fn a_short_body_is_a_failure_not_unreachable() {
        let base = serve(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".to_vec(),
        ]);
        let url = format!("{base}/cut");
        let message = get(&url).err().unwrap();
        assert!(message.starts_with(&format!("{url} failed: ")), "{message}");
    }

    #[test]
    fn refuses_a_body_over_the_limit() {
        let body = vec![b'a'; MAX_BODY as usize + 1];
        let base = serve(vec![reply("200 OK\r\nContent-Type: text/plain", &body)]);
        let url = format!("{base}/big");
        assert_eq!(
            get(&url).err().unwrap(),
            format!("{url} is over the 16 MiB limit")
        );
    }

    #[test]
    fn unreachable_names_the_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let message = get(&url).err().unwrap();
        assert!(
            message.starts_with(&format!("{url} is unreachable: ")),
            "{message}"
        );
    }
}
