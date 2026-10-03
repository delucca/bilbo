//! The local embedder's pinned model: where it lives and its resumable, verified download.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const FILE: &str = "Qwen3-Embedding-0.6B-Q8_0.gguf";
/// The name the server answers to and the config sets.
pub const NAME: &str = "qwen3-embedding-0.6b";

/// What to fetch and how to recognize it; tests pass their own.
pub struct Pinned<'a> {
    pub url: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}

pub const PINNED: Pinned<'static> = Pinned {
    url: "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/370f27d7550e0def9b39c1f16d3fbaa13aa67728/Qwen3-Embedding-0.6B-Q8_0.gguf",
    size: 639_150_592,
    sha256: "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439",
};

/// How long one request may spend on its body before it is sent again from where it stopped.
/// A request that receives nothing in that time ends the download.
pub const WINDOW: Duration = Duration::from_secs(60);

/// The port the local embedder listens on when `--embedder-port` does not say.
pub const PORT: u16 = 8737;

const CHUNK: usize = 1 << 20;

/// `<cache>/models/<FILE>`, where `cache` is the folder `vectors::dir` resolves.
pub fn path(cache: &Path) -> PathBuf {
    cache.join("models").join(FILE)
}

/// `<path>.part`.
pub fn part(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// A regular file of `size` bytes is at `path`; its bytes are not read.
pub fn kept(path: &Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == size)
}

/// Downloads `pinned.url` into `<path>.part`, continuing a part file left by an earlier run with
/// a range request, then renames it to `path` once its size and SHA-256 match. `progress` gets
/// (bytes done, total) after every chunk. A hash mismatch deletes the part file; any other
/// failure keeps it for the next run.
pub fn download(
    path: &Path,
    pinned: &Pinned,
    window: Duration,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let part = part(path);
    let cannot = |what: &str, e: std::io::Error| format!("cannot {what} {}: {e}", part.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&part)
        .map_err(|e| cannot("open", e))?;
    let mut have = file.metadata().map_err(|e| cannot("read", e))?.len();
    if have > pinned.size {
        file.set_len(0).map_err(|e| cannot("truncate", e))?;
        have = 0;
    }
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0; CHUNK];
    file.rewind().map_err(|e| cannot("read", e))?;
    let mut left = have;
    while left > 0 {
        let n = file.read(&mut buffer).map_err(|e| cannot("read", e))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        left = left.saturating_sub(n as u64);
    }
    progress(have, pinned.size);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(window))
        .timeout_recv_body(Some(window))
        .accept_encoding(ureq::config::AutoHeaderValue::None)
        .build()
        .into();
    let url = pinned.url;
    while have < pinned.size {
        let mut request = agent.get(url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let mut response = request.call().map_err(|e| failed(url, e))?;
        match response.status().as_u16() {
            206 => {
                let range = response
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !range.starts_with(&format!("bytes {have}-")) {
                    return Err(format!(
                        "{url} answered the range from byte {have} with '{range}'"
                    ));
                }
            }
            200 => {
                if have > 0 {
                    file.set_len(0).map_err(|e| cannot("truncate", e))?;
                    hash = ring::digest::Context::new(&ring::digest::SHA256);
                    have = 0;
                    progress(0, pinned.size);
                }
            }
            code => return Err(format!("{url} answered {code}")),
        }
        let mut reader = response.body_mut().as_reader();
        let mut got = 0u64;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if have + n as u64 > pinned.size {
                        drop(file);
                        let _ = std::fs::remove_file(&part);
                        return Err(format!("{url} sent more than {} bytes", pinned.size));
                    }
                    file.write_all(&buffer[..n])
                        .map_err(|e| cannot("write", e))?;
                    hash.update(&buffer[..n]);
                    have += n as u64;
                    got += n as u64;
                    progress(have, pinned.size);
                }
                Err(e) if timed_out(&e) && got > 0 => break,
                Err(e) if timed_out(&e) => {
                    return Err(format!(
                        "{url} sent nothing for {} s; run setup again to continue from byte {have}",
                        window.as_secs_f32()
                    ));
                }
                Err(e) => {
                    return Err(format!(
                        "the download from {url} stopped at byte {have}: {e}; run setup again to continue"
                    ));
                }
            }
        }
        if got == 0 && have < pinned.size {
            return Err(format!(
                "{url} ended the download at byte {have} of {}",
                pinned.size
            ));
        }
    }

    let actual = hex(hash.finish().as_ref());
    if actual != pinned.sha256 {
        drop(file);
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "the download's SHA-256 is {actual}, expected {}",
            pinned.sha256
        ));
    }
    file.sync_all().map_err(|e| cannot("write", e))?;
    drop(file);
    std::fs::rename(&part, path).map_err(|e| {
        format!(
            "cannot rename {} to {}: {e}",
            part.display(),
            path.display()
        )
    })
}

fn timed_out(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::TimedOut
        || e.get_ref()
            .and_then(|inner| inner.downcast_ref::<ureq::Error>())
            .is_some_and(|inner| matches!(inner, ureq::Error::Timeout(_)))
}

fn failed(url: &str, error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => format!("{url} answered {code}"),
        ureq::Error::Timeout(_) => format!("{url} did not answer"),
        e => format!("{url} unreachable: {e}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// llama-server's arguments: bagend's flags with the micro-batch raised to 4096, on loopback only.
pub fn server_args(model: &Path, port: u16) -> Vec<String> {
    let model = model.to_string_lossy().into_owned();
    let port = port.to_string();
    [
        "--model",
        &model,
        "--alias",
        NAME,
        "--embedding",
        "--pooling",
        "last",
        "--host",
        "127.0.0.1",
        "--port",
        &port,
        "--ctx-size",
        "4096",
        "--batch-size",
        "4096",
        "--ubatch-size",
        "4096",
        "--parallel",
        "1",
    ]
    .map(String::from)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_args_are_bagends_with_the_bigger_micro_batch() {
        let args = server_args(Path::new("/c/m.gguf"), 9100);
        assert_eq!(
            args,
            [
                "--model",
                "/c/m.gguf",
                "--alias",
                "qwen3-embedding-0.6b",
                "--embedding",
                "--pooling",
                "last",
                "--host",
                "127.0.0.1",
                "--port",
                "9100",
                "--ctx-size",
                "4096",
                "--batch-size",
                "4096",
                "--ubatch-size",
                "4096",
                "--parallel",
                "1",
            ]
        );
    }
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-model-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// One scripted answer: the status, then the body from `from`, cut after `send` bytes;
    /// `stall` keeps the connection open after that instead of closing it.
    #[derive(Clone)]
    struct Reply {
        status: u16,
        from: usize,
        send: Option<usize>,
        stall: bool,
    }

    fn whole() -> Reply {
        Reply {
            status: 200,
            from: 0,
            send: None,
            stall: false,
        }
    }

    fn rest(from: usize) -> Reply {
        Reply {
            status: 206,
            from,
            send: None,
            stall: false,
        }
    }

    /// Serves `payload` with one reply per connection; the log holds each request's Range header ("" when absent).
    fn serve(payload: Vec<u8>, replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.gguf", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        std::thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range:") {
                        range = v.trim().to_string();
                    }
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                seen.lock().unwrap().push(range);
                let body = &payload[reply.from..];
                let head = match reply.status {
                    206 => format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\n\r\n",
                        reply.from,
                        payload.len() - 1,
                        payload.len(),
                        body.len()
                    ),
                    200 => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()),
                    code => format!("HTTP/1.1 {code} Nope\r\nContent-Length: 0\r\n\r\n"),
                };
                stream.write_all(head.as_bytes()).unwrap();
                if reply.status == 200 || reply.status == 206 {
                    let n = reply.send.unwrap_or(body.len()).min(body.len());
                    let _ = stream.write_all(&body[..n]);
                    let _ = stream.flush();
                    if reply.stall {
                        let _ = reader.read_to_end(&mut Vec::new());
                    }
                }
            }
        });
        (url, log)
    }

    fn payload() -> Vec<u8> {
        (0..3_000_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn sha(bytes: &[u8]) -> String {
        hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
    }

    fn get(
        url: &str,
        data: &[u8],
        path: &Path,
        window: Duration,
    ) -> (Result<(), String>, Vec<(u64, u64)>) {
        let digest = sha(data);
        let pinned = Pinned {
            url,
            size: data.len() as u64,
            sha256: &digest,
        };
        let mut seen = Vec::new();
        let result = download(path, &pinned, window, &mut |done, total| {
            seen.push((done, total))
        });
        (result, seen)
    }

    const SHORT: Duration = Duration::from_millis(400);

    #[test]
    fn pinned_constants() {
        assert!(
            PINNED
                .url
                .contains("/resolve/370f27d7550e0def9b39c1f16d3fbaa13aa67728/")
        );
        assert!(PINNED.url.ends_with(FILE));
        assert_eq!(PINNED.size, 639_150_592);
        assert_eq!(PINNED.sha256.len(), 64);
        assert_eq!(
            path(Path::new("/c/bilbo")),
            Path::new("/c/bilbo/models/Qwen3-Embedding-0.6B-Q8_0.gguf")
        );
        assert_eq!(
            part(Path::new("/c/m/x.gguf")),
            Path::new("/c/m/x.gguf.part")
        );
    }

    #[test]
    fn kept_is_the_size_only() {
        let s = scratch("kept");
        let file = s.0.join("m.gguf");
        assert!(!kept(&file, 10));
        std::fs::File::create(&file).unwrap().set_len(10).unwrap();
        assert!(kept(&file, 10));
        assert!(!kept(&file, 11));
        assert!(!kept(&s.0, 0));
    }

    #[test]
    fn fresh_download() {
        let s = scratch("fresh");
        let data = payload();
        let (url, log) = serve(data.clone(), vec![whole()]);
        let file = s.0.join("models/m.gguf");
        let (result, seen) = get(&url, &data, &file, WINDOW);
        result.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), data);
        assert!(!part(&file).exists());
        assert_eq!(*log.lock().unwrap(), [""]);
        assert_eq!(seen.first(), Some(&(0, data.len() as u64)));
        assert_eq!(seen.last(), Some(&(data.len() as u64, data.len() as u64)));
    }

    #[test]
    fn resume_with_206() {
        let s = scratch("resume");
        let data = payload();
        let file = s.0.join("m.gguf");
        std::fs::write(part(&file), &data[..1_000_000]).unwrap();
        let (url, log) = serve(data.clone(), vec![rest(1_000_000)]);
        let (result, seen) = get(&url, &data, &file, WINDOW);
        result.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), data);
        assert_eq!(*log.lock().unwrap(), ["bytes=1000000-"]);
        assert_eq!(seen[0], (1_000_000, data.len() as u64));
    }

    #[test]
    fn range_answered_with_200_starts_over() {
        let s = scratch("ignored");
        let data = payload();
        let file = s.0.join("m.gguf");
        std::fs::write(part(&file), b"garbage that is not the start").unwrap();
        let (url, log) = serve(data.clone(), vec![whole()]);
        let (result, _) = get(&url, &data, &file, WINDOW);
        result.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), data);
        assert_eq!(log.lock().unwrap()[0], "bytes=29-");
    }

    #[test]
    fn hash_mismatch_leaves_nothing() {
        let s = scratch("mismatch");
        let data = payload();
        let (url, _) = serve(data.clone(), vec![whole()]);
        let file = s.0.join("m.gguf");
        let pinned = Pinned {
            url: &url,
            size: data.len() as u64,
            sha256: "00",
        };
        let err = download(&file, &pinned, WINDOW, &mut |_, _| {}).unwrap_err();
        assert_eq!(
            err,
            format!("the download's SHA-256 is {}, expected 00", sha(&data))
        );
        assert!(!file.exists());
        assert!(!part(&file).exists());
    }

    #[test]
    fn interrupted_body_keeps_the_part_file() {
        let s = scratch("cut");
        let data = payload();
        let cut = Reply {
            send: Some(500_000),
            ..whole()
        };
        let (url, _) = serve(data.clone(), vec![cut]);
        let file = s.0.join("m.gguf");
        let (result, _) = get(&url, &data, &file, WINDOW);
        let err = result.unwrap_err();
        assert!(err.contains("stopped at byte"), "{err}");
        assert!(!file.exists());
        let kept = std::fs::metadata(part(&file)).unwrap().len();
        assert!(kept > 0 && kept <= 500_000, "{kept}");
    }

    #[test]
    fn status_names_the_url() {
        let s = scratch("status");
        let data = payload();
        let (url, _) = serve(
            data.clone(),
            vec![Reply {
                status: 404,
                ..whole()
            }],
        );
        let (result, _) = get(&url, &data, &s.0.join("m.gguf"), WINDOW);
        assert_eq!(result.unwrap_err(), format!("{url} answered 404"));
    }

    #[test]
    fn a_slow_body_is_asked_again_from_where_it_stopped() {
        let s = scratch("window");
        let data = payload();
        let first = Reply {
            send: Some(1_200_000),
            stall: true,
            ..whole()
        };
        let (url, log) = serve(data.clone(), vec![first, rest(1_200_000)]);
        let file = s.0.join("m.gguf");
        let (result, _) = get(&url, &data, &file, SHORT);
        result.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), data);
        assert_eq!(*log.lock().unwrap(), ["", "bytes=1200000-"]);
    }

    #[test]
    fn a_stalled_body_ends_the_download() {
        let s = scratch("stall");
        let data = payload();
        let first = Reply {
            send: Some(1_200_000),
            stall: true,
            ..whole()
        };
        let again = Reply {
            send: Some(0),
            stall: true,
            ..rest(1_200_000)
        };
        let (url, _) = serve(data.clone(), vec![first, again]);
        let file = s.0.join("m.gguf");
        let (result, _) = get(&url, &data, &file, SHORT);
        let err = result.unwrap_err();
        assert!(err.contains("sent nothing for 0.4 s"), "{err}");
        assert_eq!(std::fs::metadata(part(&file)).unwrap().len(), 1_200_000);
    }

    /// Network: the pinned URL redirects to a CDN that honors a range through ureq.
    #[test]
    #[ignore]
    fn pinned_url_honors_a_range() {
        let s = scratch("network");
        let file = s.0.join("m.gguf");
        let from = PINNED.size - 92;
        std::fs::File::create(part(&file))
            .unwrap()
            .set_len(from)
            .unwrap();
        let pinned = Pinned {
            url: PINNED.url,
            size: PINNED.size,
            sha256: "00",
        };
        let mut last = (0, 0);
        let err = download(&file, &pinned, WINDOW, &mut |d, t| last = (d, t)).unwrap_err();
        assert!(err.starts_with("the download's SHA-256 is "), "{err}");
        assert_eq!(last, (PINNED.size, PINNED.size));
    }
}
