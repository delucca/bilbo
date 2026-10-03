use crate::config::{self, Token};
use std::ffi::OsString;
use std::time::{Duration, Instant};

pub const BATCH: usize = 16;

pub struct Client {
    agent: ureq::Agent,
    endpoint: String,
    /// As configured; messages show it.
    url: String,
    model: String,
    token: Option<String>,
    timeout: Duration,
}

#[derive(serde::Serialize)]
struct Request<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(serde::Deserialize)]
struct Response {
    data: Vec<Item>,
}

#[derive(serde::Deserialize)]
struct Item {
    embedding: Vec<f64>,
    #[serde(default)]
    index: Option<usize>,
}

impl Client {
    /// Reads the token (`var` looks a variable up) and builds the agent with `timeout` as its global timeout.
    pub fn new(
        embedder: &config::Embedder,
        var: impl Fn(&str) -> Option<OsString>,
        timeout: Duration,
    ) -> Result<Client, String> {
        let token = match &embedder.token {
            Some(token) => Some(read_token(token, var)?),
            None => None,
        };
        Ok(Client::with_token(embedder, token, timeout))
    }

    /// For a key already in memory, such as one pasted into the wizard.
    pub fn with_token(
        embedder: &config::Embedder,
        token: Option<String>,
        timeout: Duration,
    ) -> Client {
        Client {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .build()
                .into(),
            endpoint: format!("{}/v1/embeddings", embedder.url.trim_end_matches('/')),
            url: embedder.url.clone(),
            model: embedder.model.clone(),
            token,
            timeout,
        }
    }

    /// One request for 1 to `BATCH` inputs: one unit vector per input, all of one length, in input order.
    pub fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        debug_assert!(inputs.len() <= BATCH);
        let mut request = self.agent.post(&self.endpoint);
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request
            .send_json(&Request {
                model: &self.model,
                input: inputs,
            })
            .map_err(|e| self.describe(e))?;
        let answer = response
            .body_mut()
            .read_json::<Response>()
            .map_err(|e| self.describe(e))?;
        self.vectors(answer, inputs.len())
    }

    fn describe(&self, error: ureq::Error) -> String {
        let url = &self.url;
        match error {
            ureq::Error::StatusCode(code) => format!("embedder {url} answered {code}"),
            ureq::Error::Timeout(_) => format!(
                "embedder {url} did not answer within {}",
                seconds(self.timeout)
            ),
            ureq::Error::Io(e) => format!("embedder {url} unreachable: {e}"),
            ureq::Error::HostNotFound => format!("embedder {url} unreachable: host not found"),
            ureq::Error::ConnectionFailed => {
                format!("embedder {url} unreachable: connection failed")
            }
            ureq::Error::Json(_) => {
                format!("embedder {url} answered a body that is not an embeddings list")
            }
            e => format!("embedder {url} failed: {e}"),
        }
    }

    fn vectors(&self, answer: Response, count: usize) -> Result<Vec<Vec<f32>>, String> {
        let url = &self.url;
        if answer.data.len() != count {
            return Err(format!(
                "embedder {url} answered {} vectors for {count} inputs",
                answer.data.len()
            ));
        }
        let mut slots: Vec<Option<Vec<f64>>> = vec![None; count];
        for (i, item) in answer.data.into_iter().enumerate() {
            match slots.get_mut(item.index.unwrap_or(i)) {
                Some(slot @ None) => *slot = Some(item.embedding),
                _ => return Err(format!("embedder {url} answered vectors with bad indexes")),
            }
        }
        let raw: Vec<Vec<f64>> = slots.into_iter().flatten().collect();
        let dims = raw[0].len();
        if dims == 0 {
            return Err(format!("embedder {url} answered an empty vector"));
        }
        if raw.iter().any(|v| v.len() != dims) {
            return Err(format!(
                "embedder {url} answered vectors of different lengths"
            ));
        }
        raw.iter()
            .map(|v| normalize(v))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("embedder {url} answered a vector that cannot be normalized"))
    }

    #[cfg(test)]
    fn timeout(&self) -> Option<Duration> {
        self.agent.config().timeouts().global
    }
}

/// The query's unit vector from one request, bounded by `timeout`; it must have `dims` dimensions,
/// the vector cache's.
pub fn query(
    embedder: &config::Embedder,
    text: &str,
    timeout: Duration,
    dims: usize,
) -> Result<Vec<f32>, String> {
    let client = Client::new(embedder, |name| std::env::var_os(name), timeout)?;
    let q = client.embed(&[text.to_string()])?.remove(0);
    if q.len() == dims {
        Ok(q)
    } else {
        Err(format!(
            "embedder {} answered {} dimensions; the cache holds {dims}",
            embedder.url,
            q.len()
        ))
    }
}

/// `300 ms` under a second, else `5 s` or `1.2 s`.
fn seconds(d: Duration) -> String {
    if d < Duration::from_secs(1) {
        format!("{} ms", d.as_millis())
    } else if d.subsec_millis() == 0 {
        format!("{} s", d.as_secs())
    } else {
        format!("{:.1} s", d.as_secs_f64())
    }
}

#[derive(serde::Deserialize)]
struct Tags {
    models: Vec<Model>,
}

#[derive(serde::Deserialize)]
struct Model {
    name: String,
}

/// The model names of the Ollama at `url` (`GET <url>/api/tags`, `{"models":[{"name":…}]}`), or None when it does not answer in `timeout` or answers something else.
pub fn ollama_models(url: &str, timeout: Duration) -> Option<Vec<String>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut response = agent
        .get(format!("{}/api/tags", url.trim_end_matches('/')))
        .call()
        .ok()?;
    let tags = response.body_mut().read_json::<Tags>().ok()?;
    Some(tags.models.into_iter().map(|m| m.name).collect())
}

const POLL: Duration = Duration::from_millis(500);

/// Polls `GET <url>/health` every 500 ms until it answers 2xx or `deadline` has passed; llama-server answers 503 while it loads the model.
/// The error is `embedder <url> was not ready within <s> s; <last>`, where <last> is `it answered <code>`, `it did not answer` or `it was unreachable: <e>`.
pub fn ready(url: &str, deadline: Duration) -> Result<(), String> {
    let health = format!("{}/health", url.trim_end_matches('/'));
    let start = Instant::now();
    loop {
        let left = deadline.saturating_sub(start.elapsed());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(
                left.min(Duration::from_secs(2))
                    .max(Duration::from_millis(100)),
            ))
            .http_status_as_error(false)
            .build()
            .into();
        let last = match agent.get(&health).call() {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => format!("it answered {}", response.status().as_u16()),
            Err(ureq::Error::Timeout(_)) => "it did not answer".to_string(),
            Err(e) => format!("it was unreachable: {e}"),
        };
        if start.elapsed() + POLL > deadline {
            return Err(format!(
                "embedder {url} was not ready within {} s; {last}",
                deadline.as_secs()
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// The unit vector, or all zeros for a zero vector; `None` when the norm is not finite.
fn normalize(vector: &[f64]) -> Option<Vec<f32>> {
    let norm = vector.iter().map(|x| x * x).sum::<f64>().sqrt();
    if !norm.is_finite() {
        return None;
    }
    Some(
        vector
            .iter()
            .map(|x| if norm == 0.0 { 0.0 } else { (x / norm) as f32 })
            .collect(),
    )
}

fn read_token(token: &Token, var: impl Fn(&str) -> Option<OsString>) -> Result<String, String> {
    let (text, source) = match token {
        Token::Var(name) => {
            let value =
                var(name).ok_or_else(|| format!("embedder token variable {name} is not set"))?;
            let text = value
                .into_string()
                .map_err(|_| format!("embedder token variable {name} is not valid UTF-8"))?;
            if text.trim().is_empty() {
                return Err(format!("embedder token variable {name} is empty"));
            }
            (text, format!("variable {name}"))
        }
        Token::File(path) => {
            let bytes = std::fs::read(path)
                .map_err(|e| format!("cannot read embedder token file {}: {e}", path.display()))?;
            let text = String::from_utf8(bytes).map_err(|_| {
                format!("embedder token file {} is not valid UTF-8", path.display())
            })?;
            if text.trim().is_empty() {
                return Err(format!("embedder token file {} is empty", path.display()));
            }
            (text, format!("file {}", path.display()))
        }
    };
    let text = text.trim();
    if !text.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(format!(
            "embedder token in {source} holds characters an HTTP header cannot carry"
        ));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread::JoinHandle;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-embed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    const TOKEN: &str = "sk-unit-Zq8xW3vK9pL2mN7r";

    fn ok(body: &str) -> String {
        reply("200 OK", body)
    }

    fn reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Serves one request: `response` is written back, or never when it is `None`; the handle yields the raw request.
    fn serve(response: Option<String>) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut raw = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                raw.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            raw.push_str(&String::from_utf8(body).unwrap());
            let mut stream = stream;
            match response {
                Some(response) => {
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.flush().unwrap();
                }
                None => {
                    // The client gives up and closes; reading to the end waits for that.
                    let _ = reader.read_to_end(&mut Vec::new());
                }
            }
            raw
        });
        (url, handle)
    }

    fn serve_once(response: &str) -> (String, JoinHandle<String>) {
        serve(Some(response.to_string()))
    }

    fn embedder(url: &str, token: Option<Token>) -> config::Embedder {
        config::Embedder {
            url: url.to_string(),
            model: "m".to_string(),
            token,
            query_prefix: String::new(),
            min_similarity: config::DEFAULT_MIN_SIMILARITY,
        }
    }

    fn client(url: &str) -> Client {
        Client::new(&embedder(url, None), |_| None, Duration::from_secs(5)).unwrap()
    }

    fn texts(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("text {i}")).collect()
    }

    fn answer(body: &str, inputs: usize) -> Result<Vec<Vec<f32>>, String> {
        let (url, handle) = serve_once(&ok(body));
        let result = client(&url).embed(&texts(inputs));
        handle.join().unwrap();
        result
    }

    /// Answers one connection per reply, in order.
    fn serve_each(replies: Vec<String>) -> (String, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut raw = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    raw.push_str(&line);
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
                requests.push(raw);
            }
            requests
        });
        (url, handle)
    }

    #[test]
    fn ready_at_once() {
        let (url, handle) = serve_each(vec![ok(r#"{"status":"ok"}"#)]);
        ready(&url, Duration::from_secs(5)).unwrap();
        let requests = handle.join().unwrap();
        assert_eq!(requests[0].lines().next(), Some("GET /health HTTP/1.1"));
    }

    #[test]
    fn ready_after_503() {
        let loading = reply("503 Service Unavailable", r#"{"error":{"code":503}}"#);
        let (url, handle) = serve_each(vec![loading.clone(), loading, ok("{}")]);
        let start = Instant::now();
        ready(&url, Duration::from_secs(10)).unwrap();
        assert!(start.elapsed() >= Duration::from_secs(1));
        handle.join().unwrap();
    }

    #[test]
    fn ready_deadline_passes() {
        let url = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let start = Instant::now();
        let error = ready(&url, Duration::from_secs(1)).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(
            error.starts_with(&format!(
                "embedder {url} was not ready within 1 s; it was unreachable:"
            )),
            "{error}"
        );
    }

    #[test]
    fn ready_reports_the_last_status() {
        let loading = reply("503 Service Unavailable", "{}");
        let (url, handle) = serve_each(vec![loading; 6]);
        let error = ready(&url, Duration::from_secs(1)).unwrap_err();
        assert!(error.ends_with("it answered 503"), "{error}");
        drop(handle);
    }

    #[test]
    fn normalizes_and_orders_by_index() {
        let body = r#"{"data":[{"index":1,"embedding":[3,4]},{"index":0,"embedding":[0,2]}]}"#;
        let (url, handle) = serve_once(&ok(body));
        let vectors = client(&url).embed(&texts(2)).unwrap();
        let raw = handle.join().unwrap();
        assert_eq!(vectors, vec![vec![0.0, 1.0], vec![0.6, 0.8]]);
        assert!(raw.starts_with("POST /v1/embeddings HTTP/1.1\r\n"), "{raw}");
        assert!(raw.contains(r#""model": "m""#), "{raw}");
    }

    #[test]
    fn empty_input_sends_no_request() {
        let url = dead_url();
        assert_eq!(client(&url).embed(&[]), Ok(Vec::new()));
    }

    #[test]
    fn missing_index_means_position() {
        let body = r#"{"data":[{"embedding":[0,2]},{"embedding":[3,4]}]}"#;
        assert_eq!(
            answer(body, 2).unwrap(),
            vec![vec![0.0, 1.0], vec![0.6, 0.8]]
        );
    }

    #[test]
    fn zero_vector_stays_zero() {
        let body = r#"{"data":[{"embedding":[0,0]}]}"#;
        assert_eq!(answer(body, 1).unwrap(), vec![vec![0.0, 0.0]]);
    }

    #[test]
    fn trailing_slash_in_the_url_is_dropped() {
        let (url, handle) = serve_once(&ok(r#"{"data":[{"embedding":[1]}]}"#));
        Client::new(
            &embedder(&format!("{url}/"), None),
            |_| None,
            Duration::from_secs(5),
        )
        .unwrap()
        .embed(&texts(1))
        .unwrap();
        assert!(handle.join().unwrap().starts_with("POST /v1/embeddings "));
    }

    #[test]
    fn malformed_answers() {
        let cases: [(&str, usize, &str); 6] = [
            (
                r#"{"data":[{"embedding":[1]},{"embedding":[1]}]}"#,
                3,
                "answered 2 vectors for 3 inputs",
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1]},{"index":0,"embedding":[1]}]}"#,
                2,
                "answered vectors with bad indexes",
            ),
            (
                r#"{"data":[{"index":5,"embedding":[1]},{"index":0,"embedding":[1]}]}"#,
                2,
                "answered vectors with bad indexes",
            ),
            (
                r#"{"data":[{"embedding":[]}]}"#,
                1,
                "answered an empty vector",
            ),
            (
                r#"{"data":[{"embedding":[1,2]},{"embedding":[1,2,3]}]}"#,
                2,
                "answered vectors of different lengths",
            ),
            (
                "{not json",
                1,
                "answered a body that is not an embeddings list",
            ),
        ];
        for (body, inputs, tail) in cases {
            let (url, handle) = serve_once(&ok(body));
            let message = client(&url).embed(&texts(inputs)).unwrap_err();
            handle.join().unwrap();
            assert_eq!(message, format!("embedder {url} {tail}"), "{body}");
        }
    }

    #[test]
    fn unnormalizable_vector() {
        let body = r#"{"data":[{"embedding":[1e200]}]}"#;
        let (url, handle) = serve_once(&ok(body));
        let message = client(&url).embed(&texts(1)).unwrap_err();
        handle.join().unwrap();
        assert_eq!(
            message,
            format!("embedder {url} answered a vector that cannot be normalized")
        );
    }

    #[test]
    fn status_is_named() {
        let (url, handle) = serve_once(&reply("401 Unauthorized", "{}"));
        let message = client(&url).embed(&texts(1)).unwrap_err();
        handle.join().unwrap();
        assert_eq!(message, format!("embedder {url} answered 401"));
    }

    fn dead_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    #[test]
    fn dead_port_is_unreachable() {
        let url = dead_url();
        let message = client(&url).embed(&texts(1)).unwrap_err();
        assert!(
            message.starts_with(&format!("embedder {url} unreachable: ")),
            "{message}"
        );
    }

    #[test]
    fn stall_times_out() {
        let (url, handle) = serve(None);
        let client =
            Client::new(&embedder(&url, None), |_| None, Duration::from_millis(300)).unwrap();
        let message = client.embed(&texts(1)).unwrap_err();
        handle.join().unwrap();
        assert_eq!(
            message,
            format!("embedder {url} did not answer within 300 ms")
        );
    }

    #[test]
    fn query_checks_the_dimensions() {
        let (url, handle) = serve_once(&ok(r#"{"data":[{"embedding":[1,0,0]}]}"#));
        let message = query(&embedder(&url, None), "hello", Duration::from_secs(5), 4).unwrap_err();
        handle.join().unwrap();
        assert_eq!(
            message,
            format!("embedder {url} answered 3 dimensions; the cache holds 4")
        );
    }

    #[test]
    fn seconds_reads_well() {
        assert_eq!(seconds(Duration::from_millis(300)), "300 ms");
        assert_eq!(seconds(Duration::from_secs(5)), "5 s");
        assert_eq!(seconds(Duration::from_millis(1137)), "1.1 s");
    }

    #[test]
    fn agent_has_the_given_timeout() {
        let client = Client::new(
            &embedder("http://127.0.0.1:1", None),
            |_| None,
            Duration::from_secs(600),
        )
        .unwrap();
        assert_eq!(client.timeout(), Some(Duration::from_secs(600)));
    }

    fn token_of(token: Token, var: impl Fn(&str) -> Option<OsString>) -> Result<String, String> {
        read_token(&token, var)
    }

    fn var_token(value: Option<&str>) -> Result<String, String> {
        let value = value.map(OsString::from);
        token_of(Token::Var("EMBED_TOKEN".into()), |_| value.clone())
    }

    #[test]
    fn token_sources() {
        assert_eq!(
            var_token(None).unwrap_err(),
            "embedder token variable EMBED_TOKEN is not set"
        );
        assert_eq!(
            var_token(Some(" \n")).unwrap_err(),
            "embedder token variable EMBED_TOKEN is empty"
        );
        assert_eq!(var_token(Some("  tok \n")).unwrap(), "tok");
        assert_eq!(
            var_token(Some("a b")).unwrap_err(),
            "embedder token in variable EMBED_TOKEN holds characters an HTTP header cannot carry"
        );

        let dir = scratch("token");
        let file = dir.0.join("tok");
        let from_file = || token_of(Token::File(file.clone()), |_| None);
        assert_eq!(
            from_file().unwrap_err(),
            format!(
                "cannot read embedder token file {}: No such file or directory (os error 2)",
                file.display()
            )
        );
        std::fs::write(&file, "\n").unwrap();
        assert_eq!(
            from_file().unwrap_err(),
            format!("embedder token file {} is empty", file.display())
        );
        std::fs::write(&file, "tok\n").unwrap();
        assert_eq!(from_file().unwrap(), "tok");
        std::fs::write(&file, "a\tb").unwrap();
        assert_eq!(
            from_file().unwrap_err(),
            format!(
                "embedder token in file {} holds characters an HTTP header cannot carry",
                file.display()
            )
        );
        std::fs::write(&file, b"\xff").unwrap();
        assert_eq!(
            from_file().unwrap_err(),
            format!("embedder token file {} is not valid UTF-8", file.display())
        );
    }

    #[test]
    fn token_is_sent_as_a_bearer() {
        let (url, handle) = serve_once(&ok(r#"{"data":[{"embedding":[1]}]}"#));
        let client = Client::new(
            &embedder(&url, Some(Token::Var("EMBED_TOKEN".into()))),
            |_| Some(TOKEN.into()),
            Duration::from_secs(5),
        )
        .unwrap();
        client.embed(&texts(1)).unwrap();
        let raw = handle.join().unwrap().to_ascii_lowercase();
        assert!(raw.contains(&format!("authorization: bearer {TOKEN}").to_ascii_lowercase()));
    }

    fn leaks(message: &str) -> bool {
        let secret = &TOKEN[TOKEN.len() - 16..];
        secret
            .as_bytes()
            .windows(4)
            .any(|w| message.contains(std::str::from_utf8(w).unwrap()))
    }

    #[test]
    fn token_never_appears() {
        let secret_client = |url: &str, timeout: Duration| {
            Client::new(
                &embedder(url, Some(Token::Var("EMBED_TOKEN".into()))),
                |_| Some(TOKEN.into()),
                timeout,
            )
            .unwrap()
        };
        let bearer = format!("authorization: bearer {TOKEN}");
        let responses = [
            reply("401 Unauthorized", "{}"),
            reply("500 Internal Server Error", "{}"),
            ok(r#"{"data":[]}"#),
            ok("{not json"),
        ];
        for response in responses {
            let (url, handle) = serve_once(&response);
            let message = secret_client(&url, Duration::from_secs(5))
                .embed(&texts(1))
                .unwrap_err();
            let raw = handle.join().unwrap().to_ascii_lowercase();
            assert!(raw.contains(&bearer.to_ascii_lowercase()), "{raw}");
            assert!(!leaks(&message), "{message}");
        }

        let message = secret_client(&dead_url(), Duration::from_secs(5))
            .embed(&texts(1))
            .unwrap_err();
        assert!(!leaks(&message), "{message}");

        let (url, handle) = serve(None);
        let message = secret_client(&url, Duration::from_millis(300))
            .embed(&texts(1))
            .unwrap_err();
        let raw = handle.join().unwrap().to_ascii_lowercase();
        assert!(raw.contains(&bearer.to_ascii_lowercase()), "{raw}");
        assert!(!leaks(&message), "{message}");

        let dir = scratch("never");
        let file = dir.0.join("tok");
        for content in [format!("{TOKEN} x"), format!("{TOKEN}\u{e9}"), TOKEN.into()] {
            std::fs::write(&file, &content).unwrap();
            let result = read_token(&Token::File(file.clone()), |_| None);
            if let Err(message) = result {
                assert!(!leaks(&message), "{message}");
            }
        }
        std::fs::write(&file, b"\xff").unwrap();
        let message = read_token(&Token::File(file.clone()), |_| None).unwrap_err();
        assert!(!leaks(&message), "{message}");
        let message = read_token(&Token::Var("EMBED_TOKEN".into()), |_| {
            Some(format!("{TOKEN} x").into())
        })
        .unwrap_err();
        assert!(!leaks(&message), "{message}");
    }

    #[test]
    fn with_token_sends_the_bearer() {
        let (url, handle) = serve_once(&ok(r#"{"data":[{"embedding":[1,0]}]}"#));
        let client = Client::with_token(
            &embedder(&url, None),
            Some(TOKEN.to_string()),
            Duration::from_secs(5),
        );
        client.embed(&texts(1)).unwrap();
        let raw = handle.join().unwrap();
        assert!(
            raw.to_ascii_lowercase()
                .contains(&format!("authorization: bearer {TOKEN}").to_ascii_lowercase())
        );
    }

    #[test]
    fn ollama_models_lists_names() {
        let body = r#"{"models":[{"name":"llama3","size":1},{"name":"nomic-embed-text:latest"}]}"#;
        let (url, handle) = serve_once(&ok(body));
        let names = ollama_models(&url, Duration::from_secs(5));
        let raw = handle.join().unwrap();
        assert!(raw.starts_with("GET /api/tags HTTP/1.1\r\n"), "{raw}");
        assert_eq!(
            names,
            Some(vec![
                "llama3".to_string(),
                "nomic-embed-text:latest".to_string()
            ])
        );
    }

    #[test]
    fn ollama_models_none_when_dead() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}");
        assert_eq!(ollama_models(&url, Duration::from_secs(1)), None);
    }

    #[test]
    fn ollama_models_none_on_bad_body() {
        let (url, handle) = serve_once(&ok(r#"{"data":[]}"#));
        assert_eq!(ollama_models(&url, Duration::from_secs(5)), None);
        handle.join().unwrap();
    }
}
