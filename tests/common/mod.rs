#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("bilbo-test-{}-{name}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs bilbo with a clean environment plus `env`.
pub fn bilbo(cwd: &Path, env: &[(&str, &str)], args: &[&str]) -> Run {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    bilbo_os(cwd, env, &args)
}

pub fn bilbo_os(cwd: &Path, env: &[(&str, &str)], args: &[&OsStr]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_bilbo"))
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    let run = Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    };
    for line in run.stderr.lines() {
        assert!(
            line.starts_with("bilbo: "),
            "stderr line without the prefix: {line:?}"
        );
    }
    run
}

pub const IDS: [&str; 3] = [
    "01M3YJ7R6HK6NQ30DCDB1P4DYB",
    "01M3YE296FMNXYZS89787DMY0A",
    "01M3YJ7R6HK6NQ30DCDB1P4D00",
];

pub fn note_text(id: &str, title: &str) -> String {
    format!("---\nid: {id}\ncreated: 2026-10-02T14:23-03:00\n---\n\n# {title}\n")
}

/// Makes `<dir>/store/notes` and returns the store root.
pub fn store(dir: &TempDir) -> PathBuf {
    let root = dir.path().join("store");
    std::fs::create_dir_all(root.join("notes")).unwrap();
    root
}

pub fn write(root: &Path, name: &str, text: &str) {
    std::fs::write(root.join("notes").join(name), text).unwrap();
}

pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)> {
    let mut out = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let meta = std::fs::metadata(&path).unwrap();
        let bytes = if meta.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
            None
        } else {
            Some(std::fs::read(&path).unwrap())
        };
        out.insert(path, (bytes, meta.modified().unwrap()));
    }
    out
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Names lowercased, in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub model: String,
    pub inputs: Vec<String>,
}

struct State {
    dims: usize,
    table: Vec<(String, Vec<f32>)>,
    fail_after: Option<usize>,
    status: Option<u16>,
    too_few: bool,
    stall: bool,
    answered: usize,
    requests: Vec<Request>,
}

/// An embedder on 127.0.0.1 answering `POST /v1/embeddings`, one request per connection.
pub struct Fake {
    /// `http://127.0.0.1:<port>`, no trailing slash.
    pub url: String,
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    /// Serves `POST /v1/embeddings` with `dims`-long vectors from a thread.
    pub fn start(dims: usize) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State {
            dims,
            table: Vec::new(),
            fail_after: None,
            status: None,
            too_few: false,
            stall: false,
            answered: 0,
            requests: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        serve(stream, &state, &stop);
                    }
                }
            })
        };
        Fake {
            url: format!("http://{addr}"),
            addr,
            state,
            stop,
            thread: Some(thread),
        }
    }

    /// Inputs holding `substring` get `vector`; the first matching entry wins, in the order added.
    pub fn vector(&self, substring: &str, vector: &[f32]) {
        let mut state = self.state.lock().unwrap();
        state.table.push((substring.to_string(), vector.to_vec()));
    }

    /// After `n` answered requests, answer every request 500.
    pub fn fail_after(&self, n: usize) {
        self.state.lock().unwrap().fail_after = Some(n);
    }

    /// Answer every request with `code`.
    pub fn status(&self, code: u16) {
        self.state.lock().unwrap().status = Some(code);
    }

    /// Answer one vector fewer than there are inputs.
    pub fn too_few(&self) {
        self.state.lock().unwrap().too_few = true;
    }

    /// Read each request and never answer it.
    pub fn stall(&self) {
        self.state.lock().unwrap().stall = true;
    }

    /// Clear every switch and the answered count; keep the table and the log.
    pub fn heal(&self) {
        let mut state = self.state.lock().unwrap();
        state.fail_after = None;
        state.status = None;
        state.too_few = false;
        state.stall = false;
        state.answered = 0;
    }

    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }

    /// Every input of every request, in order.
    pub fn inputs(&self) -> Vec<String> {
        self.requests().into_iter().flat_map(|r| r.inputs).collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: TcpStream, shared: &Mutex<State>, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut bytes = vec![0; length];
    if reader.read_exact(&mut bytes).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let model = json["model"].as_str().unwrap_or("").to_string();
    let inputs: Vec<String> = json["input"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let wrong = method != "POST" || path != "/v1/embeddings";
    let (code, text) = {
        let mut state = shared.lock().unwrap();
        state.requests.push(Request {
            method,
            path,
            headers,
            body,
            model: model.clone(),
            inputs: inputs.clone(),
        });
        if state.stall {
            drop(state);
            let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
            while !stop.load(Ordering::SeqCst) && state_stalls(shared) {
                match reader.read(&mut [0u8; 1]) {
                    Ok(0) => break,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    _ => {}
                }
            }
            return;
        }
        let code = match state.status {
            _ if wrong => 404,
            Some(code) => code,
            None if state.fail_after.is_some_and(|n| state.answered >= n) => 500,
            None => {
                state.answered += 1;
                200
            }
        };
        if code == 200 {
            let mut items: Vec<String> = inputs
                .iter()
                .enumerate()
                .map(|(i, input)| {
                    let vector = vector_for(&state, input)
                        .iter()
                        .map(f32::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    format!(r#"{{"object":"embedding","index":{i},"embedding":[{vector}]}}"#)
                })
                .collect();
            if state.too_few {
                items.pop();
            }
            (
                code,
                format!(
                    r#"{{"object":"list","data":[{}],"model":"{model}"}}"#,
                    items.join(",")
                ),
            )
        } else {
            (code, r#"{"error":{"message":"fake"}}"#.to_string())
        }
    };
    let reason = match code {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.flush();
}

fn state_stalls(state: &Mutex<State>) -> bool {
    state.lock().unwrap().stall
}

/// The first table entry whose substring `input` holds, else zeros with a 1 in the last dimension.
fn vector_for(state: &State, input: &str) -> Vec<f32> {
    match state.table.iter().find(|(s, _)| input.contains(s.as_str())) {
        Some((_, vector)) => vector.clone(),
        None => {
            let mut vector = vec![0.0; state.dims];
            if let Some(last) = vector.last_mut() {
                *last = 1.0;
            }
            vector
        }
    }
}

/// A URL nothing listens on: bind 127.0.0.1:0, keep the port, drop the listener.
pub fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// Writes `<dir>/config` holding `lines`, one per line, and returns its path.
pub fn config(dir: &TempDir, lines: &[&str]) -> PathBuf {
    let path = dir.path().join("config");
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(&path, text).unwrap();
    path
}
